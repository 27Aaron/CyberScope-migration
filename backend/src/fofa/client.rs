use std::{cmp, future::Future, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{StatusCode, header::RETRY_AFTER};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{
    error::FofaError,
    models::{RawSearchResponse, RetryStats, ReturnField, SearchQuery, SearchResponse},
    validator::QueryValidator,
};

/// Defensive cap for one decoded search response, enforced while chunked bodies
/// are streamed into the per-page buffer.
pub const MAX_SEARCH_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Retries after the initial request.
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
        }
    }
}

impl RetryPolicy {
    fn delay(&self, retry_index: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(retry_after) = retry_after {
            // Honor the upstream value independently of the exponential-backoff
            // ceiling. A safety cap prevents a malicious header from parking the
            // sole global job forever.
            return cmp::min(retry_after, Duration::from_secs(5 * 60));
        }

        let multiplier = 1_u128 << retry_index.min(31);
        let millis = self.base_delay.as_millis().saturating_mul(multiplier);
        let delay = Duration::from_millis(millis.min(u128::from(u64::MAX)) as u64);
        cmp::min(delay, self.max_delay)
    }
}

/// FOFA public search client.
///
/// The injected client should reject cross-origin redirects, so credentials can
/// never be forwarded to another host. Request errors are converted immediately
/// and never retain their credential-bearing URL.
pub struct FofaClient {
    http: reqwest::Client,
    base_url: Url,
    api_key: SecretString,
    retry: RetryPolicy,
}

impl FofaClient {
    pub fn new(
        http: reqwest::Client,
        base_url: Url,
        api_key: SecretString,
        retry: RetryPolicy,
    ) -> Result<Self, FofaError> {
        if !matches!(base_url.scheme(), "http" | "https") || base_url.host_str().is_none() {
            return Err(FofaError::invalid("FOFA API 基础地址必须是 HTTP(S) URL"));
        }
        Ok(Self {
            http,
            base_url,
            api_key,
            retry,
        })
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Execute one ordinary FOFA page.
    pub async fn search_all(
        &self,
        search: &SearchQuery,
        page: Option<u64>,
        cancellation: &CancellationToken,
    ) -> Result<SearchResponse, FofaError> {
        self.validate_search_shape(search)?;
        let url = self.endpoint("/api/v1/search/all")?;
        let qbase64 = STANDARD.encode(search.query.as_bytes());
        let api_fields = search.api_fields();

        self.execute_search(
            || {
                self.http.get(url.clone()).query(&AllQueryParams {
                    key: self.api_key.expose_secret(),
                    qbase64: &qbase64,
                    fields: &api_fields,
                    size: search.size,
                    page,
                    full: search.full,
                })
            },
            &search.fields,
            None,
            cancellation,
        )
        .await
    }

    /// Execute the first or a subsequent cursor page. The first call passes `None`;
    /// every later call passes the last returned cursor.
    pub async fn search_next(
        &self,
        search: &SearchQuery,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<SearchResponse, FofaError> {
        self.validate_search_shape(search)?;
        if cursor.is_some_and(str::is_empty) {
            return Err(FofaError::invalid("连续分页游标不能为空"));
        }

        let url = self.endpoint("/api/v1/search/next")?;
        let qbase64 = STANDARD.encode(search.query.as_bytes());
        let api_fields = search.api_fields();

        self.execute_search(
            || {
                self.http.get(url.clone()).query(&NextQueryParams {
                    key: self.api_key.expose_secret(),
                    qbase64: &qbase64,
                    fields: &api_fields,
                    size: search.size,
                    next: cursor,
                    full: search.full,
                })
            },
            &search.fields,
            cursor,
            cancellation,
        )
        .await
    }

    fn endpoint(&self, path: &str) -> Result<Url, FofaError> {
        self.base_url
            .join(path)
            .map_err(|_| FofaError::invalid("无法构造 FOFA API 地址"))
    }

    fn validate_search_shape(&self, search: &SearchQuery) -> Result<(), FofaError> {
        let validated = QueryValidator::for_base_url(&self.base_url)
            .validate(search.query.clone(), search.fields.clone())?;
        if search.size == 0 || search.size > validated.max_request_size() {
            return Err(FofaError::invalid(format!(
                "当前返回字段的单次请求 size 必须在 1..={} 范围内",
                validated.max_request_size()
            )));
        }
        Ok(())
    }

    async fn execute_search<F>(
        &self,
        build_request: F,
        fields: &[ReturnField],
        requested_cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<SearchResponse, FofaError>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut retries = 0_u32;
        let mut possible_duplicate_charge = false;

        loop {
            let response = match cancellable(cancellation, build_request().send()).await {
                Ok(response) => response,
                Err(CancellableError::Cancelled) => return Err(FofaError::Cancelled),
                Err(CancellableError::Inner(error)) => {
                    if error.is_builder() {
                        return Err(FofaError::protocol("无法编码 FOFA 请求参数"));
                    }
                    // Connect errors happen before a request is sent, so they are the
                    // only transport errors retried by default.
                    if error.is_connect() && retries < self.retry.max_retries {
                        self.wait_before_retry(retries, None, cancellation).await?;
                        retries += 1;
                        continue;
                    }
                    return Err(FofaError::UpstreamUnavailable {
                        status: error.status().map(|status| status.as_u16()),
                        possible_duplicate_charge: !error.is_connect(),
                    });
                }
            };

            let status = response.status();
            let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
            if status == StatusCode::TOO_MANY_REQUESTS {
                if retries < self.retry.max_retries {
                    self.wait_before_retry(retries, retry_after, cancellation)
                        .await?;
                    retries += 1;
                    continue;
                }
                return Err(FofaError::RateLimited { retry_after });
            }

            if matches!(
                status,
                StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE
            ) {
                possible_duplicate_charge = true;
                if retries < self.retry.max_retries {
                    self.wait_before_retry(retries, retry_after, cancellation)
                        .await?;
                    retries += 1;
                    continue;
                }
                return Err(FofaError::UpstreamUnavailable {
                    status: Some(status.as_u16()),
                    possible_duplicate_charge: true,
                });
            }

            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                return Err(FofaError::AuthenticationRejected);
            }
            if status == StatusCode::PAYMENT_REQUIRED {
                return Err(FofaError::QuotaExhausted);
            }
            if !status.is_success() {
                return Err(
                    if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
                        FofaError::UpstreamUnavailable {
                            status: Some(status.as_u16()),
                            possible_duplicate_charge: true,
                        }
                    } else {
                        FofaError::protocol(format!("上游返回 HTTP {}", status.as_u16()))
                    },
                );
            }

            if response
                .content_length()
                .is_some_and(|length| length > MAX_SEARCH_RESPONSE_BYTES)
            {
                return Err(FofaError::protocol("上游搜索响应超过 64 MiB 安全上限"));
            }

            let body = read_limited_body(response, cancellation).await?;

            let raw: RawSearchResponse = serde_json::from_slice(&body)
                .map_err(|_| FofaError::protocol("上游响应不是有效的搜索 JSON"))?;

            if raw.error {
                let business = classify_business_error(raw.errmsg.as_deref());
                if matches!(business, BusinessError::RateLimited)
                    && retries < self.retry.max_retries
                {
                    self.wait_before_retry(retries, None, cancellation).await?;
                    retries += 1;
                    continue;
                }
                return Err(business.into_fofa_error());
            }

            if raw.tip.as_deref().is_some_and(is_quota_exhausted) {
                return Err(FofaError::QuotaExhausted);
            }

            let mut normalized = raw.normalize(fields)?;
            if let (Some(old), Some(new)) = (requested_cursor, normalized.next.as_deref())
                && old == new
            {
                return Err(FofaError::protocol("连续分页游标未推进"));
            }
            normalized.retry = RetryStats {
                attempts: retries.saturating_add(1),
                retries,
                possible_duplicate_charge,
            };
            return Ok(normalized);
        }
    }

    async fn wait_before_retry(
        &self,
        retry_index: u32,
        retry_after: Option<Duration>,
        cancellation: &CancellationToken,
    ) -> Result<(), FofaError> {
        let delay = self.retry.delay(retry_index, retry_after);
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(FofaError::Cancelled),
            _ = tokio::time::sleep(delay) => Ok(()),
        }
    }
}

#[derive(Serialize)]
struct AllQueryParams<'a> {
    key: &'a str,
    qbase64: &'a str,
    fields: &'a str,
    size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    page: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full: Option<bool>,
}

#[derive(Serialize)]
struct NextQueryParams<'a> {
    key: &'a str,
    qbase64: &'a str,
    fields: &'a str,
    size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full: Option<bool>,
}

enum CancellableError<E> {
    Cancelled,
    Inner(E),
}

async fn cancellable<T, E>(
    cancellation: &CancellationToken,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, CancellableError<E>> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(CancellableError::Cancelled),
        result = future => result.map_err(CancellableError::Inner),
    }
}

async fn read_limited_body(
    response: reqwest::Response,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, FofaError> {
    let capacity = response
        .content_length()
        .unwrap_or_default()
        .min(8 * 1024 * 1024) as usize;
    let mut body = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();

    loop {
        let next = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FofaError::Cancelled),
            next = stream.next() => next,
        };
        let Some(chunk) = next else {
            return Ok(body);
        };
        let chunk = chunk.map_err(|_| FofaError::UpstreamUnavailable {
            status: None,
            possible_duplicate_charge: true,
        })?;
        let new_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| FofaError::protocol("上游搜索响应大小溢出"))?;
        if new_len as u64 > MAX_SEARCH_RESPONSE_BYTES {
            return Err(FofaError::protocol("上游搜索响应超过 64 MiB 安全上限"));
        }
        body.extend_from_slice(&chunk);
    }
}

fn parse_retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let value = value?.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let deadline = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    deadline.signed_duration_since(Utc::now()).to_std().ok()
}

enum BusinessError {
    Authentication,
    QuotaExhausted,
    RateLimited,
    InvalidQuery,
    Other(String),
}

impl BusinessError {
    fn into_fofa_error(self) -> FofaError {
        match self {
            Self::Authentication => FofaError::AuthenticationRejected,
            Self::QuotaExhausted => FofaError::QuotaExhausted,
            Self::RateLimited => FofaError::RateLimited { retry_after: None },
            Self::InvalidQuery => FofaError::invalid("上游拒绝了查询语法或字段"),
            Self::Other(code) => FofaError::UpstreamBusiness { code },
        }
    }
}

fn classify_business_error(message: Option<&str>) -> BusinessError {
    let Some(message) = message else {
        return BusinessError::Other("unknown".to_owned());
    };
    let lowercase = message.to_ascii_lowercase();
    if message.contains("[-100]")
        || message.contains("[-101]")
        || message.contains("Key 不存在")
        || message.contains("Token 或 Key")
        || lowercase.contains("invalid key")
    {
        BusinessError::Authentication
    } else if message.contains("[45012]") || lowercase.contains("rate limit") {
        BusinessError::RateLimited
    } else if is_quota_exhausted(message) {
        BusinessError::QuotaExhausted
    } else if message.contains("语法")
        || message.contains("查询字段")
        || lowercase.contains("syntax")
    {
        BusinessError::InvalidQuery
    } else {
        BusinessError::Other(extract_business_code(message))
    }
}

fn is_quota_exhausted(message: &str) -> bool {
    let lowercase = message.to_ascii_lowercase();
    message.contains("额度已用尽")
        || message.contains("额度已用完")
        || message.contains("已用尽")
        || message.contains("已用完")
        || lowercase.contains("quota exhausted")
        || lowercase.contains("insufficient quota")
}

fn extract_business_code(message: &str) -> String {
    let Some(start) = message.find('[') else {
        return "unknown".to_owned();
    };
    let Some(relative_end) = message[start + 1..].find(']') else {
        return "unknown".to_owned();
    };
    let code = &message[start + 1..start + 1 + relative_end];
    if !code.is_empty()
        && code.len() <= 16
        && code
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-'))
    {
        code.to_owned()
    } else {
        "unknown".to_owned()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/fofa/client.rs"]
mod tests;
