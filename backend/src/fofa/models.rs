use std::fmt;

use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use super::error::FofaError;

pub type NormalizedRow = Vec<Value>;

/// A requested result column. `output_name` is used in exported files, while
/// `api_name` is sent to FOFA.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReturnField {
    pub output_name: String,
    pub api_name: String,
}

impl ReturnField {
    pub fn new(output_name: impl Into<String>, api_name: impl Into<String>) -> Self {
        Self {
            output_name: output_name.into(),
            api_name: api_name.into(),
        }
    }

    /// Create a field from a UI/API name, applying the documented `location.*`
    /// aliases while preserving the caller's output header.
    pub fn from_name(name: impl Into<String>) -> Self {
        let output_name = name.into();
        let api_name = map_location_field(&output_name).to_owned();
        Self {
            output_name,
            api_name,
        }
    }

    pub fn api(name: impl Into<String>) -> Self {
        let name = name.into();
        Self::new(name.clone(), name)
    }
}

impl From<&str> for ReturnField {
    fn from(value: &str) -> Self {
        Self::from_name(value)
    }
}

impl From<String> for ReturnField {
    fn from(value: String) -> Self {
        Self::from_name(value)
    }
}

pub fn map_location_field(name: &str) -> &str {
    match name {
        "location.country" => "country",
        "location.country_name" => "country_name",
        "location.region" => "region",
        "location.city" => "city",
        "location.longitude" => "longitude",
        "location.latitude" => "latitude",
        _ => name,
    }
}

/// A validated search request shared by `/search/all` and `/search/next`.
///
/// This type intentionally has no `Debug` implementation because the raw query is
/// sensitive operational data.
#[derive(Clone)]
pub struct SearchQuery {
    pub query: String,
    pub fields: Vec<ReturnField>,
    pub size: u32,
    pub full: Option<bool>,
}

impl fmt::Debug for SearchQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchQuery")
            .field("query", &"[REDACTED]")
            .field("field_count", &self.fields.len())
            .field("size", &self.size)
            .field("full", &self.full)
            .finish()
    }
}

impl SearchQuery {
    pub fn new(query: impl Into<String>, fields: Vec<ReturnField>, size: u32) -> Self {
        Self {
            query: query.into(),
            fields,
            size,
            full: None,
        }
    }

    pub fn with_full(mut self, full: bool) -> Self {
        self.full = Some(full);
        self
    }

    pub fn api_fields(&self) -> String {
        self.fields
            .iter()
            .map(|field| field.api_name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// A normalized FOFA batch. Rows always follow the requested field order.
///
/// This type intentionally has no `Debug` implementation because it contains
/// queries and returned asset data.
pub struct SearchResponse {
    pub query: Option<String>,
    pub mode: Option<String>,
    pub matched_size: Option<u64>,
    pub page: Option<u64>,
    pub rows: Vec<NormalizedRow>,
    pub next: Option<String>,
    pub tip: Option<String>,
    pub retry: RetryStats,
}

impl fmt::Debug for SearchResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchResponse")
            .field("query", &self.query.as_ref().map(|_| "[REDACTED]"))
            .field("mode", &self.mode)
            .field("matched_size", &self.matched_size)
            .field("page", &self.page)
            .field("row_count", &self.rows.len())
            .field("has_next", &self.next.is_some())
            .field("has_tip", &self.tip.is_some())
            .field("retry", &self.retry)
            .finish()
    }
}

impl SearchResponse {
    pub fn batch_size(&self) -> usize {
        self.rows.len()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetryStats {
    /// Number of HTTP attempts, including the successful/final attempt.
    pub attempts: u32,
    /// Number of attempts made after the initial request.
    pub retries: u32,
    /// A 502/503 retry can duplicate an upstream quota charge even if this batch
    /// eventually succeeds.
    pub possible_duplicate_charge: bool,
}

#[derive(Deserialize)]
pub(crate) struct RawSearchResponse {
    #[serde(default)]
    pub error: bool,
    #[serde(default)]
    pub errmsg: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub page: Option<u64>,
    #[serde(default)]
    pub results: Option<Value>,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub tip: Option<String>,
}

impl RawSearchResponse {
    pub(crate) fn normalize(self, fields: &[ReturnField]) -> Result<SearchResponse, FofaError> {
        if self.error {
            return Err(FofaError::protocol("业务错误必须在结果标准化之前处理"));
        }

        let results = self
            .results
            .ok_or_else(|| FofaError::protocol("成功响应缺少 results 字段"))?;
        let rows = normalize_results(results, fields)?;
        let next = self.next.filter(|cursor| !cursor.is_empty());

        Ok(SearchResponse {
            query: self.query,
            mode: self.mode,
            matched_size: self.size,
            page: self.page,
            rows,
            next,
            tip: self.tip,
            retry: RetryStats::default(),
        })
    }
}

pub(crate) fn normalize_results(
    results: Value,
    fields: &[ReturnField],
) -> Result<Vec<NormalizedRow>, FofaError> {
    if fields.is_empty() {
        return Err(FofaError::protocol("无法在没有返回字段时标准化结果"));
    }

    let values = results
        .as_array()
        .ok_or_else(|| FofaError::protocol("results 不是数组"))?;
    if values.is_empty() {
        return Ok(Vec::new());
    }

    let array_rows = values.iter().all(Value::is_array);
    let object_rows = values.iter().all(Value::is_object);
    let scalar_rows = values.iter().all(is_scalar);

    if array_rows {
        values
            .iter()
            .map(|row| normalize_array_row(row, fields.len()))
            .collect()
    } else if object_rows {
        values
            .iter()
            .map(|row| normalize_object_row(row, fields))
            .collect()
    } else if scalar_rows {
        if fields.len() != 1 {
            return Err(FofaError::protocol("标量 results 仅能用于单个返回字段"));
        }
        Ok(values.iter().cloned().map(|value| vec![value]).collect())
    } else {
        Err(FofaError::protocol("results 混合了多种行结构"))
    }
}

fn normalize_array_row(value: &Value, field_count: usize) -> Result<NormalizedRow, FofaError> {
    let row = value
        .as_array()
        .ok_or_else(|| FofaError::protocol("结果行不是数组"))?;
    if row.len() != field_count {
        return Err(FofaError::protocol(format!(
            "结果列数为 {}，请求字段数为 {field_count}",
            row.len()
        )));
    }
    if !row.iter().all(is_scalar) {
        return Err(FofaError::protocol("二维 results 中出现嵌套值"));
    }
    Ok(row.clone())
}

fn normalize_object_row(value: &Value, fields: &[ReturnField]) -> Result<NormalizedRow, FofaError> {
    let object: &Map<String, Value> = value
        .as_object()
        .ok_or_else(|| FofaError::protocol("结果行不是对象"))?;

    fields
        .iter()
        .map(|field| {
            let value = object.get(&field.api_name).cloned().unwrap_or(Value::Null);
            if is_scalar(&value) {
                Ok(value)
            } else {
                Err(FofaError::protocol(format!(
                    "对象结果字段 `{}` 包含嵌套值",
                    field.api_name
                )))
            }
        })
        .collect()
}

fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

/// A relay time field may be a display string or Unix seconds.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum FlexibleTime {
    Text(String),
    Unix(i64),
}

/// Preserves the important difference between an absent field and JSON `null`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FieldState<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> FieldState<T> {
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    pub fn as_ref(&self) -> FieldState<&T> {
        match self {
            Self::Missing => FieldState::Missing,
            Self::Null => FieldState::Null,
            Self::Value(value) => FieldState::Value(value),
        }
    }
}

impl<'de, T> Deserialize<'de> for FieldState<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map(|value| match value {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct UserInfoEnvelope {
    #[serde(default)]
    pub success: Option<bool>,
    #[serde(default)]
    pub key_type: Option<String>,
    #[serde(default)]
    pub data: Option<QuotaInfo>,
}

impl UserInfoEnvelope {
    pub fn protocol_compatible(&self) -> bool {
        self.data
            .as_ref()
            .is_some_and(QuotaInfo::protocol_compatible)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct QuotaInfo {
    #[serde(default)]
    pub status: FieldState<i32>,
    #[serde(default)]
    pub status_text: Option<String>,
    #[serde(default)]
    pub activate_time: FieldState<FlexibleTime>,
    #[serde(default)]
    pub expire_at: FieldState<FlexibleTime>,
    #[serde(default)]
    pub expire_seconds: FieldState<i64>,
    #[serde(default)]
    pub remaining_api_calls: FieldState<i64>,
    #[serde(default)]
    pub remaining_item_count: FieldState<i64>,
}

impl QuotaInfo {
    /// Field-state names missing from the response. The UI can render these as
    /// “未知” and mark the optional quota module protocol-incompatible.
    pub fn missing_fields(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.status.is_missing() {
            missing.push("status");
        }
        if self.activate_time.is_missing() {
            missing.push("activate_time");
        }
        if self.expire_at.is_missing() {
            missing.push("expire_at");
        }
        if self.expire_seconds.is_missing() {
            missing.push("expire_seconds");
        }
        if self.remaining_api_calls.is_missing() {
            missing.push("remaining_api_calls");
        }
        if self.remaining_item_count.is_missing() {
            missing.push("remaining_item_count");
        }
        missing
    }

    pub fn protocol_compatible(&self) -> bool {
        self.missing_fields().is_empty()
    }
}

impl fmt::Display for FlexibleTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(value) => formatter.write_str(value),
            Self::Unix(value) => write!(formatter, "{value}"),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/fofa/models.rs"]
mod tests;
