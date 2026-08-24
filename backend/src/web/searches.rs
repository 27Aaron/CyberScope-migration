use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use csv::WriterBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::{
    export::json_value_to_cell,
    fofa::{FofaError, ReturnField, default_return_fields, supported_return_field_names},
    jobs::{
        JobKind, JobLease, JobSnapshot, JobStartError, JobStatus, RunError, RunSettings, run_single,
    },
    searches::{MAX_RESULTS, SINGLE_USER_ID, SearchRecord, SearchStatus},
    state::AppState,
};

const DEFAULT_PAGE_SIZE: u64 = 100;
const MAX_PAGE_SIZE: u64 = 1_000;

#[derive(Debug, Deserialize)]
pub struct CreateSearchRequest {
    pub query: String,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default = "default_page_size")]
    pub page_size: u64,
    #[serde(default = "default_max_results")]
    pub max_results: u64,
    #[serde(default)]
    pub full: bool,
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    #[serde(default = "default_page")]
    pub page: u64,
    #[serde(default = "default_page_size")]
    pub per_page: u64,
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    #[serde(default = "default_export_format")]
    pub format: String,
}

#[derive(Debug, Serialize)]
struct FieldResponse {
    name: String,
    label: String,
}

#[derive(Debug, Serialize)]
struct SearchTaskResponse {
    id: String,
    status: String,
    query: String,
    fields: Vec<String>,
    max_results: u64,
    full: bool,
    matched_size: Option<u64>,
    written_rows: u64,
    upstream_attempts: u64,
    retries: u64,
    possible_duplicate_charge: bool,
    error_code: Option<String>,
    error_message: Option<String>,
    created_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
    updated_at: String,
}

pub async fn list_fields(State(state): State<std::sync::Arc<AppState>>) -> Response {
    let fields = supported_return_field_names(state.validator.mode())
        .iter()
        .map(|name| FieldResponse {
            name: (*name).to_owned(),
            label: (*name).replace('_', " "),
        })
        .collect::<Vec<_>>();
    Json(json!({ "data": fields })).into_response()
}

pub async fn create_search(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<CreateSearchRequest>,
) -> Response {
    let fields = if request.fields.is_empty() {
        default_return_fields(state.validator.mode())
    } else {
        request
            .fields
            .into_iter()
            .map(ReturnField::from_name)
            .collect::<Vec<_>>()
    };
    let query = request.query.trim().to_owned();
    let validated = match state.validator.validate(query.clone(), fields) {
        Ok(validated) => validated,
        Err(error) => return fofa_error_response(error),
    };

    let max_results = request.max_results.clamp(1, MAX_RESULTS);
    let page_size = request.page_size.clamp(1, MAX_PAGE_SIZE);
    let max_results = max_results.min(page_size);
    let fields = validated.fields().to_vec();
    let id = Uuid::new_v4();
    let now = Utc::now();
    let record = SearchRecord {
        id,
        query: query.clone(),
        fields: fields
            .iter()
            .map(|field| field.output_name.clone())
            .collect(),
        format: "csv".to_owned(),
        max_results,
        full: request.full,
        status: SearchStatus::Queued,
        matched_size: None,
        written_rows: 0,
        upstream_attempts: 0,
        retries: 0,
        possible_duplicate_charge: false,
        error_code: None,
        error_message: None,
        created_at: now,
        started_at: None,
        completed_at: None,
        updated_at: now,
    };

    let lease = match state.jobs.try_start(SINGLE_USER_ID, JobKind::Search) {
        Ok(lease) => lease,
        Err(error) => return job_start_error_response(error),
    };
    if let Err(error) = state.searches.insert(&record).await {
        drop(lease);
        return internal_error_response("无法创建查询任务", error);
    }

    let response = task_response(&record, None);
    let settings = RunSettings {
        fields,
        format: crate::export::ExportFormat::Csv,
        max_results,
        full: request.full,
    };
    let state_for_task = state.clone();
    tokio::spawn(async move {
        execute_search(state_for_task, id, query, settings, lease).await;
    });

    (StatusCode::ACCEPTED, Json(json!({ "data": response }))).into_response()
}

pub async fn get_search(
    State(state): State<std::sync::Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Response {
    let record = match get_record(&state, id).await {
        Ok(Some(record)) => record,
        Ok(None) => return not_found_response("查询任务不存在"),
        Err(error) => return internal_error_response("读取查询任务失败", error),
    };
    let snapshot = state.jobs.snapshot_by_id(id);
    Json(json!({ "data": task_response(&record, snapshot.as_ref()) })).into_response()
}

pub async fn cancel_search(
    State(state): State<std::sync::Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Response {
    let record = match get_record(&state, id).await {
        Ok(Some(record)) => record,
        Ok(None) => return not_found_response("查询任务不存在"),
        Err(error) => return internal_error_response("读取查询任务失败", error),
    };
    if matches!(
        record.status,
        SearchStatus::Completed | SearchStatus::Failed | SearchStatus::Cancelled
    ) {
        return conflict_response("当前任务已经结束");
    }

    match state
        .searches
        .mark_cancelling(id)
        .await
        .map_err(|error| error.to_string())
    {
        Ok(true) => {
            state.jobs.cancel_by_id(id);
            match state.searches.get(id).await.map_err(|error| error.to_string()) {
                Ok(Some(record)) => {
                    Json(json!({ "data": task_response(&record, state.jobs.snapshot_by_id(id).as_ref()) }))
                        .into_response()
                }
                Ok(None) => not_found_response("查询任务不存在"),
                Err(error) => internal_error_response("读取取消后的任务状态失败", error),
            }
        }
        Ok(false) => conflict_response("当前任务无法取消"),
        Err(error) => internal_error_response("取消查询任务失败", error),
    }
}

pub async fn get_results(
    State(state): State<std::sync::Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Response {
    let record = match get_record(&state, id).await {
        Ok(Some(record)) => record,
        Ok(None) => return not_found_response("查询任务不存在"),
        Err(error) => return internal_error_response("读取查询任务失败", error),
    };
    let per_page = page.per_page.clamp(1, MAX_PAGE_SIZE) as usize;
    let page_number = page.page.max(1);
    let offset = (page_number.saturating_sub(1) as usize).saturating_mul(per_page);
    let rows = match state
        .searches
        .results(id)
        .await
        .map_err(|error| error.to_string())
    {
        Ok(rows) => rows,
        Err(error) => return internal_error_response("读取查询结果失败", error),
    };
    let total = rows.len();
    let page_rows = rows
        .iter()
        .skip(offset)
        .take(per_page)
        .map(|row| row_to_object(&record.fields, row))
        .collect::<Vec<_>>();

    Json(json!({
        "data": {
            "rows": page_rows,
            "fields": record.fields,
            "total": total,
            "page": page_number,
            "per_page": per_page,
            "status": record.status.as_str(),
        }
    }))
    .into_response()
}

pub async fn export_search(
    State(state): State<std::sync::Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(query): Query<ExportQuery>,
) -> Response {
    let record = match get_record(&state, id).await {
        Ok(Some(record)) => record,
        Ok(None) => return not_found_response("查询任务不存在"),
        Err(error) => return internal_error_response("读取查询任务失败", error),
    };
    let rows = match state
        .searches
        .results(id)
        .await
        .map_err(|error| error.to_string())
    {
        Ok(rows) => rows,
        Err(error) => return internal_error_response("读取导出结果失败", error),
    };
    let format = query.format.to_ascii_lowercase();
    let (content_type, extension, bytes) = match format.as_str() {
        "csv" => match export_csv(&record.fields, &rows) {
            Ok(bytes) => ("text/csv; charset=utf-8", "csv", bytes),
            Err(error) => return internal_error_response("生成 CSV 失败", error),
        },
        "json" => match export_json(&record.fields, &rows) {
            Ok(bytes) => ("application/json; charset=utf-8", "json", bytes),
            Err(error) => return internal_error_response("生成 JSON 失败", error),
        },
        "txt" => (
            "text/plain; charset=utf-8",
            "txt",
            export_txt(&record.fields, &rows),
        ),
        _ => return bad_request_response("format 必须是 csv、json 或 txt"),
    };

    let filename = format!("fofa-search-{}.{}", id, extension);
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

async fn execute_search(
    state: std::sync::Arc<AppState>,
    id: Uuid,
    query: String,
    settings: RunSettings,
    lease: JobLease,
) {
    if let Err(error) = state.searches.mark_running(id).await {
        tracing::error!(task_id = %id, error = %error, "无法更新查询任务为运行中");
        return;
    }

    let result = run_single(
        &state.fofa,
        state.validator,
        state.temp_root(),
        query,
        &settings,
        &lease,
    )
    .await;

    let snapshot = lease.snapshot();
    match result {
        Ok(outcome) => {
            if let Err(error) = state
                .searches
                .finish_success(
                    id,
                    outcome.matched_size,
                    &outcome.rows,
                    snapshot.upstream_attempts,
                    snapshot.retries,
                    snapshot.possible_duplicate_charge,
                )
                .await
            {
                tracing::error!(task_id = %id, error = %error, "保存查询结果失败");
            }
        }
        Err(RunError::Fofa(FofaError::Cancelled)) => {
            if let Err(error) = state.searches.finish_cancelled(id).await {
                tracing::error!(task_id = %id, error = %error, "保存取消状态失败");
            }
        }
        Err(error) => {
            let (code, message) = run_error_details(&error);
            if let Err(storage_error) = state.searches.finish_failed(id, code, &message).await {
                tracing::error!(task_id = %id, error = %storage_error, "保存查询失败状态失败");
            }
        }
    }
}

async fn get_record(
    state: &std::sync::Arc<AppState>,
    id: Uuid,
) -> Result<Option<SearchRecord>, String> {
    state
        .searches
        .get(id)
        .await
        .map_err(|error| error.to_string())
}

fn task_response(record: &SearchRecord, snapshot: Option<&JobSnapshot>) -> SearchTaskResponse {
    let (status, written_rows, upstream_attempts, retries, duplicate_charge) = snapshot
        .map(|snapshot| {
            (
                job_status_to_search_status(snapshot.status)
                    .as_str()
                    .to_owned(),
                snapshot.written_rows,
                snapshot.upstream_attempts,
                snapshot.retries,
                snapshot.possible_duplicate_charge,
            )
        })
        .unwrap_or_else(|| {
            (
                record.status.as_str().to_owned(),
                record.written_rows,
                record.upstream_attempts,
                record.retries,
                record.possible_duplicate_charge,
            )
        });
    SearchTaskResponse {
        id: record.id.to_string(),
        status,
        query: record.query.clone(),
        fields: record.fields.clone(),
        max_results: record.max_results,
        full: record.full,
        matched_size: record.matched_size,
        written_rows,
        upstream_attempts,
        retries,
        possible_duplicate_charge: duplicate_charge,
        error_code: record.error_code.clone(),
        error_message: record.error_message.clone(),
        created_at: record.created_at.to_rfc3339(),
        started_at: record.started_at.map(|value| value.to_rfc3339()),
        completed_at: record.completed_at.map(|value| value.to_rfc3339()),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

fn job_status_to_search_status(status: JobStatus) -> SearchStatus {
    match status {
        JobStatus::Queued => SearchStatus::Queued,
        JobStatus::Running => SearchStatus::Running,
        JobStatus::Cancelling => SearchStatus::Cancelling,
        JobStatus::Completed => SearchStatus::Completed,
        JobStatus::Failed => SearchStatus::Failed,
        JobStatus::Cancelled => SearchStatus::Cancelled,
    }
}

fn row_to_object(fields: &[String], row: &[Value]) -> Value {
    let mut object = Map::new();
    for (index, field) in fields.iter().enumerate() {
        object.insert(
            field.clone(),
            row.get(index).cloned().unwrap_or(Value::Null),
        );
    }
    Value::Object(object)
}

fn export_csv(fields: &[String], rows: &[Vec<Value>]) -> Result<Vec<u8>, csv::Error> {
    let mut output = Vec::new();
    output.extend_from_slice(b"\xEF\xBB\xBF");
    let mut writer = WriterBuilder::new().has_headers(false).from_writer(output);
    writer.write_record(fields)?;
    for row in rows {
        writer.write_record(row.iter().map(json_value_to_cell))?;
    }
    writer
        .into_inner()
        .map_err(|error| error.into_error().into())
}

fn export_json(fields: &[String], rows: &[Vec<Value>]) -> Result<Vec<u8>, serde_json::Error> {
    let objects = rows
        .iter()
        .map(|row| row_to_object(fields, row))
        .collect::<Vec<_>>();
    serde_json::to_vec_pretty(&objects)
}

fn export_txt(fields: &[String], rows: &[Vec<Value>]) -> Vec<u8> {
    let mut output = String::new();
    for (index, row) in rows.iter().enumerate() {
        output.push_str(&format!("========== RECORD {:06} ==========\n", index + 1));
        for (field, value) in fields.iter().zip(row) {
            output.push_str(field);
            output.push_str(": ");
            output.push_str(&json_value_to_cell(value));
            output.push('\n');
        }
        output.push('\n');
    }
    output.into_bytes()
}

fn run_error_details(error: &RunError) -> (&'static str, String) {
    match error {
        RunError::Fofa(error) => (fofa_error_code(error), error.to_string()),
        RunError::Export(error) => ("export_error", error.to_string()),
    }
}

fn fofa_error_code(error: &FofaError) -> &'static str {
    match error {
        FofaError::InvalidQuery { .. } => "invalid_query",
        FofaError::UnsupportedField { .. } => "unsupported_field",
        FofaError::AuthenticationRejected | FofaError::AuthenticationExpired => {
            "authentication_error"
        }
        FofaError::QuotaExhausted => "quota_exhausted",
        FofaError::RateLimited { .. } => "rate_limited",
        FofaError::UpstreamUnavailable { .. } => "upstream_unavailable",
        FofaError::UpstreamBusiness { .. } => "upstream_business_error",
        FofaError::UpstreamProtocol { .. } => "upstream_protocol_error",
        FofaError::Cancelled => "cancelled",
    }
}

fn fofa_error_response(error: FofaError) -> Response {
    let status = match &error {
        FofaError::InvalidQuery { .. } | FofaError::UnsupportedField { .. } => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        FofaError::AuthenticationRejected | FofaError::AuthenticationExpired => {
            StatusCode::UNAUTHORIZED
        }
        FofaError::QuotaExhausted => StatusCode::PAYMENT_REQUIRED,
        FofaError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_GATEWAY,
    };
    let code = fofa_error_code(&error);
    api_error(status, code, error.to_string())
}

fn job_start_error_response(error: JobStartError) -> Response {
    match error {
        JobStartError::UserBusy | JobStartError::GloballyBusy => {
            api_error(StatusCode::CONFLICT, "job_busy", error.to_string())
        }
    }
}

fn api_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "code": code,
                "message": message.into(),
            }
        })),
    )
        .into_response()
}

fn bad_request_response(message: &str) -> Response {
    api_error(StatusCode::BAD_REQUEST, "bad_request", message)
}

fn conflict_response(message: &str) -> Response {
    api_error(StatusCode::CONFLICT, "conflict", message)
}

fn not_found_response(message: &str) -> Response {
    api_error(StatusCode::NOT_FOUND, "not_found", message)
}

fn internal_error_response<E: std::fmt::Display>(message: &str, error: E) -> Response {
    tracing::error!(error = %error, "{message}");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
}

const fn default_page() -> u64 {
    1
}

const fn default_page_size() -> u64 {
    DEFAULT_PAGE_SIZE
}

const fn default_max_results() -> u64 {
    DEFAULT_PAGE_SIZE
}

fn default_export_format() -> String {
    "csv".to_owned()
}
