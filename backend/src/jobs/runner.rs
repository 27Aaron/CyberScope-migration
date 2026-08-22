use std::{path::Path, time::Duration};

use thiserror::Error;
use tokio::time::Instant;

use crate::{
    batch::BatchDocument,
    export::{ExportError, ExportFormat, ExportOptions, Exporter},
    fofa::{FofaClient, FofaError, QueryValidator, ReturnField},
};

use super::{JobLease, JobStatus};

pub const MIN_UPSTREAM_REQUEST_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct RunSettings {
    pub fields: Vec<ReturnField>,
    pub format: ExportFormat,
    pub max_results: u64,
    pub full: bool,
}

pub struct SingleOutcome {
    pub exporter: Exporter,
    pub matched_size: Option<u64>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchCompletion {
    Completed,
    Cancelled,
    SafetyLimit,
    AuthenticationStopped,
    QuotaStopped,
    RateLimitStopped,
    UpstreamStopped,
    FileWriteStopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchFailure {
    pub line_number: usize,
    pub category: &'static str,
}

pub struct BatchOutcome {
    pub exporter: Exporter,
    pub failures: Vec<BatchFailure>,
    pub completion: BatchCompletion,
}

pub async fn run_single(
    client: &FofaClient,
    validator: QueryValidator,
    temp_root: &Path,
    query: String,
    settings: &RunSettings,
    job: &JobLease,
) -> Result<SingleOutcome, RunError> {
    validate_result_limit(settings.max_results)?;
    let validated = validator.validate(query, settings.fields.clone())?;
    let search = validated
        .to_search_query(Some(settings.max_results))
        .with_full(settings.full);
    let mut exporter = create_exporter(temp_root, settings, None)?;

    job.set_status(JobStatus::Running);
    job.update(|snapshot| snapshot.upstream_attempts += 1);
    let response = match client
        .search_all(&search, Some(1), &job.cancellation_token())
        .await
    {
        Ok(response) => response,
        Err(error) => {
            if error.possible_duplicate_charge() {
                job.update(|snapshot| snapshot.possible_duplicate_charge = true);
            }
            return Err(error.into());
        }
    };
    record_retry_stats(job, response.retry);

    if response.rows.len() as u64 > settings.max_results {
        return Err(FofaError::UpstreamProtocol {
            reason: "上游返回条数超过本次查询上限".to_owned(),
        }
        .into());
    }

    if job.cancellation_token().is_cancelled() {
        return Err(FofaError::Cancelled.into());
    }

    exporter.write_json_batch(&response.rows, None)?;
    let written_rows = response.rows.len() as u64;
    job.update(|snapshot| {
        snapshot.committed_batches += 1;
        snapshot.written_rows += written_rows;
    });

    Ok(SingleOutcome {
        exporter,
        matched_size: response.matched_size,
        rows: response.rows,
    })
}

pub async fn run_batch(
    client: &FofaClient,
    validator: QueryValidator,
    temp_root: &Path,
    document: &BatchDocument,
    settings: &RunSettings,
    max_batches: u64,
    job: &JobLease,
) -> Result<BatchOutcome, RunError> {
    validate_result_limit(settings.max_results)?;
    let mut exporter = create_exporter(
        temp_root,
        settings,
        Some(document.source_kind.export_field()),
    )?;
    let mut failures = Vec::new();
    let mut last_request_started = None;
    job.set_status(JobStatus::Running);

    'queries: for item in &document.items {
        if job.cancellation_token().is_cancelled() {
            return Ok(BatchOutcome {
                exporter,
                failures,
                completion: BatchCompletion::Cancelled,
            });
        }

        let validated = match validator.validate(item.query.clone(), settings.fields.clone()) {
            Ok(validated) => validated,
            Err(error) => {
                record_batch_failure(job, &mut failures, item.line_number, &error);
                continue;
            }
        };
        let mut cursor: Option<String> = None;
        let mut written_for_query = 0_u64;

        loop {
            let remaining_results = settings.max_results - written_for_query;
            if remaining_results == 0 {
                break;
            }
            if job.snapshot().upstream_attempts >= max_batches {
                return Ok(BatchOutcome {
                    exporter,
                    failures,
                    completion: BatchCompletion::SafetyLimit,
                });
            }
            if job.cancellation_token().is_cancelled() {
                return Ok(BatchOutcome {
                    exporter,
                    failures,
                    completion: BatchCompletion::Cancelled,
                });
            }

            if wait_for_request_slot(&mut last_request_started, &job.cancellation_token())
                .await
                .is_err()
            {
                return Ok(BatchOutcome {
                    exporter,
                    failures,
                    completion: BatchCompletion::Cancelled,
                });
            }

            job.update(|snapshot| snapshot.upstream_attempts += 1);
            let search = validated
                .to_search_query(Some(remaining_results))
                .with_full(settings.full);
            let response = match client
                .search_next(&search, cursor.as_deref(), &job.cancellation_token())
                .await
            {
                Ok(response) => response,
                Err(FofaError::Cancelled) => {
                    return Ok(BatchOutcome {
                        exporter,
                        failures,
                        completion: BatchCompletion::Cancelled,
                    });
                }
                Err(error) => {
                    if error.possible_duplicate_charge() {
                        job.update(|snapshot| snapshot.possible_duplicate_charge = true);
                    }
                    let completion = terminal_completion(&error);
                    record_batch_failure(job, &mut failures, item.line_number, &error);
                    if let Some(completion) = completion {
                        return Ok(BatchOutcome {
                            exporter,
                            failures,
                            completion,
                        });
                    }
                    continue 'queries;
                }
            };
            record_retry_stats(job, response.retry);

            if response.rows.len() as u64 > remaining_results {
                let error = FofaError::UpstreamProtocol {
                    reason: "上游返回条数超过本次查询的剩余上限".to_owned(),
                };
                record_batch_failure(job, &mut failures, item.line_number, &error);
                continue 'queries;
            }

            if let Err(_error) = exporter.write_json_batch(&response.rows, Some(&item.source)) {
                job.update(|snapshot| snapshot.failed_queries += 1);
                failures.push(BatchFailure {
                    line_number: item.line_number,
                    category: "文件写入失败",
                });
                return Ok(BatchOutcome {
                    exporter,
                    failures,
                    completion: BatchCompletion::FileWriteStopped,
                });
            }

            let written_rows = response.rows.len() as u64;
            written_for_query += written_rows;
            job.update(|snapshot| {
                snapshot.committed_batches += 1;
                snapshot.written_rows += written_rows;
            });

            if response.rows.is_empty() || written_for_query >= settings.max_results {
                break;
            }
            match response.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
    }

    Ok(BatchOutcome {
        exporter,
        failures,
        completion: BatchCompletion::Completed,
    })
}

fn validate_result_limit(max_results: u64) -> Result<(), FofaError> {
    if max_results == 0 {
        return Err(FofaError::InvalidQuery {
            reason: "查询结果上限必须大于 0".to_owned(),
        });
    }
    Ok(())
}

async fn wait_for_request_slot(
    last_request_started: &mut Option<Instant>,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<(), FofaError> {
    if let Some(previous) = *last_request_started {
        let deadline = previous + MIN_UPSTREAM_REQUEST_INTERVAL;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FofaError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => {}
        }
    }
    *last_request_started = Some(Instant::now());
    Ok(())
}

fn create_exporter(
    temp_root: &Path,
    settings: &RunSettings,
    local_field: Option<&str>,
) -> Result<Exporter, ExportError> {
    let mut fields: Vec<_> = settings
        .fields
        .iter()
        .map(|field| field.output_name.clone())
        .collect();
    if let Some(local_field) = local_field {
        fields.push(local_field.to_owned());
    }
    let options = match settings.format {
        ExportFormat::Csv => ExportOptions::csv(true),
        ExportFormat::Txt => ExportOptions::txt(),
    };
    Exporter::new(temp_root, fields, options)
}

fn record_retry_stats(job: &JobLease, retry: crate::fofa::RetryStats) {
    job.update(|snapshot| {
        snapshot.upstream_attempts += u64::from(retry.attempts.saturating_sub(1));
        snapshot.retries += u64::from(retry.retries);
        snapshot.possible_duplicate_charge |= retry.possible_duplicate_charge;
    });
}

fn record_batch_failure(
    job: &JobLease,
    failures: &mut Vec<BatchFailure>,
    line_number: usize,
    error: &FofaError,
) {
    job.update(|snapshot| snapshot.failed_queries += 1);
    failures.push(BatchFailure {
        line_number,
        category: error_category(error),
    });
}

fn error_category(error: &FofaError) -> &'static str {
    match error {
        FofaError::InvalidQuery { .. } => "查询无效",
        FofaError::UnsupportedField { .. } => "字段不支持",
        FofaError::AuthenticationRejected | FofaError::AuthenticationExpired => "认证失败",
        FofaError::QuotaExhausted => "额度已用尽",
        FofaError::RateLimited { .. } => "请求限流",
        FofaError::UpstreamUnavailable { .. } => "上游不可用",
        FofaError::UpstreamBusiness { .. } => "上游业务错误",
        FofaError::UpstreamProtocol { .. } => "上游协议错误",
        FofaError::Cancelled => "任务已取消",
    }
}

fn terminal_completion(error: &FofaError) -> Option<BatchCompletion> {
    match error {
        FofaError::AuthenticationRejected | FofaError::AuthenticationExpired => {
            Some(BatchCompletion::AuthenticationStopped)
        }
        FofaError::QuotaExhausted => Some(BatchCompletion::QuotaStopped),
        FofaError::RateLimited { .. } => Some(BatchCompletion::RateLimitStopped),
        FofaError::UpstreamUnavailable { .. } => Some(BatchCompletion::UpstreamStopped),
        FofaError::Cancelled => Some(BatchCompletion::Cancelled),
        FofaError::InvalidQuery { .. }
        | FofaError::UnsupportedField { .. }
        | FofaError::UpstreamBusiness { .. }
        | FofaError::UpstreamProtocol { .. } => None,
    }
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error(transparent)]
    Fofa(#[from] FofaError),
    #[error(transparent)]
    Export(#[from] ExportError),
}

#[cfg(test)]
#[path = "../../tests/unit/jobs/runner.rs"]
mod tests;
