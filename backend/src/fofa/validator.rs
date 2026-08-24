use std::collections::HashSet;

use url::Url;

use super::{
    error::FofaError,
    models::{ReturnField, SearchQuery},
};

pub const DEFAULT_REQUEST_SIZE: u32 = 10_000;
pub const LARGE_FIELD_REQUEST_SIZE: u32 = 2_000;
pub const OFFICIAL_BODY_REQUEST_SIZE: u32 = 500;

/// Maximum query size in bytes.
pub const MAX_QUERY_CHARS: usize = 16 * 1024;

/// Fields documented by FOFA for `/api/v1/search/all`.
pub const OFFICIAL_RETURN_FIELD_NAMES: &[&str] = &[
    "ip",
    "port",
    "protocol",
    "country",
    "country_name",
    "region",
    "city",
    "longitude",
    "latitude",
    "asn",
    "org",
    "host",
    "domain",
    "os",
    "server",
    "icp",
    "title",
    "jarm",
    "header",
    "banner",
    "cert",
    "base_protocol",
    "link",
    "cert.issuer.org",
    "cert.issuer.cn",
    "cert.subject.org",
    "cert.subject.cn",
    "tls.ja3s",
    "tls.version",
    "cert.sn",
    "cert.not_before",
    "cert.not_after",
    "cert.domain",
    "status_code",
    "header_hash",
    "banner_hash",
    "banner_fid",
    "cname",
    "lastupdatetime",
    "product",
    "product_category",
    "product.version",
    "icon_hash",
    "cert.is_valid",
    "cname_domain",
    "body",
    "cert.is_match",
    "cert.is_equal",
    "icon",
    "fid",
    "structinfo",
];

/// Fields documented by the FOFA-compatible relay for result display.
pub const RELAY_RETURN_FIELD_NAMES: &[&str] = &[
    "ip",
    "port",
    "protocol",
    "country",
    "country_name",
    "region",
    "city",
    "longitude",
    "latitude",
    "asn",
    "org",
    "host",
    "domain",
    "os",
    "server",
    "icp",
    "title",
    "jarm",
    "header",
    "banner",
    "cert",
    "base_protocol",
    "link",
    "cert.issuer.org",
    "cert.issuer.cn",
    "cert.subject.org",
    "cert.subject.cn",
    "tls.ja3s",
    "tls.version",
    "cert.sn",
    "cert.not_before",
    "cert.not_after",
    "cert.domain",
    "header_hash",
    "banner_hash",
    "banner_fid",
    "cname",
    "lastupdatetime",
    "product",
    "product_category",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiMode {
    Official,
    Relay,
}

impl ApiMode {
    pub fn from_base_url(base_url: &Url) -> Self {
        if base_url
            .host_str()
            .is_some_and(|host| host.trim_end_matches('.').eq_ignore_ascii_case("fofa.info"))
        {
            Self::Official
        } else {
            Self::Relay
        }
    }
}

pub fn supported_return_field_names(mode: ApiMode) -> &'static [&'static str] {
    match mode {
        ApiMode::Official => OFFICIAL_RETURN_FIELD_NAMES,
        ApiMode::Relay => RELAY_RETURN_FIELD_NAMES,
    }
}

pub fn default_return_fields(mode: ApiMode) -> Vec<ReturnField> {
    supported_return_field_names(mode)
        .iter()
        .copied()
        .map(ReturnField::api)
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationWarning {
    AmbiguousBooleanPrecedence,
}

/// A validated query and field list for the selected endpoint.
pub struct ValidatedSearch {
    query: String,
    fields: Vec<ReturnField>,
    max_request_size: u32,
    warnings: Vec<ValidationWarning>,
}

impl ValidatedSearch {
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn fields(&self) -> &[ReturnField] {
        &self.fields
    }

    pub fn max_request_size(&self) -> u32 {
        self.max_request_size
    }

    pub fn warnings(&self) -> &[ValidationWarning] {
        &self.warnings
    }

    /// Cap the request by both the upstream limit and remaining rows.
    pub fn request_size(&self, remaining_rows: Option<u64>) -> u32 {
        remaining_rows
            .map(|remaining| remaining.min(u64::from(self.max_request_size)) as u32)
            .unwrap_or(self.max_request_size)
    }

    pub fn to_search_query(&self, remaining_rows: Option<u64>) -> SearchQuery {
        SearchQuery::new(
            self.query.clone(),
            self.fields.clone(),
            self.request_size(remaining_rows),
        )
    }

    pub fn into_search_query(self) -> SearchQuery {
        SearchQuery::new(self.query, self.fields, self.max_request_size)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct QueryValidator {
    mode: ApiMode,
}

impl QueryValidator {
    pub fn new(mode: ApiMode) -> Self {
        Self { mode }
    }

    pub fn for_base_url(base_url: &Url) -> Self {
        Self::new(ApiMode::from_base_url(base_url))
    }

    pub fn mode(&self) -> ApiMode {
        self.mode
    }

    pub fn validate(
        &self,
        query: impl Into<String>,
        fields: Vec<ReturnField>,
    ) -> Result<ValidatedSearch, FofaError> {
        let query = query.into();
        if query.trim().is_empty() {
            return Err(FofaError::invalid("查询语句不能为空"));
        }
        // Enforce the same cap used by batch query composition.
        if query.len() > MAX_QUERY_CHARS {
            return Err(FofaError::invalid("查询语句过长，最大 16384 字节"));
        }
        validate_return_field_shape(&fields)?;

        let scan = scan_query(&query)?;
        if self.mode == ApiMode::Relay {
            validate_relay_query(&scan)?;
        }
        validate_supported_return_fields(self.mode, &fields)?;

        let max_request_size = field_limit(self.mode, &fields);
        let warnings = if scan.has_and && scan.has_or && !scan.has_parenthesis {
            vec![ValidationWarning::AmbiguousBooleanPrecedence]
        } else {
            Vec::new()
        };

        Ok(ValidatedSearch {
            query,
            fields,
            max_request_size,
            warnings,
        })
    }
}

pub fn field_limit(mode: ApiMode, fields: &[ReturnField]) -> u32 {
    match mode {
        ApiMode::Official => {
            if fields.iter().any(|field| field.api_name == "body") {
                OFFICIAL_BODY_REQUEST_SIZE
            } else if fields
                .iter()
                .any(|field| matches!(field.api_name.as_str(), "cert" | "banner"))
            {
                LARGE_FIELD_REQUEST_SIZE
            } else {
                DEFAULT_REQUEST_SIZE
            }
        }
        ApiMode::Relay => {
            if fields
                .iter()
                .any(|field| matches!(field.api_name.as_str(), "header" | "banner" | "cert"))
            {
                LARGE_FIELD_REQUEST_SIZE
            } else {
                DEFAULT_REQUEST_SIZE
            }
        }
    }
}

fn validate_return_field_shape(fields: &[ReturnField]) -> Result<(), FofaError> {
    if fields.is_empty() {
        return Err(FofaError::invalid("返回字段不能为空"));
    }

    let mut seen = HashSet::with_capacity(fields.len());
    for field in fields {
        if field.output_name.trim().is_empty() || field.api_name.trim().is_empty() {
            return Err(FofaError::invalid("返回字段名不能为空"));
        }
        if !field
            .api_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
        {
            return Err(FofaError::UnsupportedField {
                field: field.api_name.clone(),
                reason: "API 字段名只能包含字母、数字、下划线和点".to_owned(),
            });
        }
        if !seen.insert(field.api_name.as_str()) {
            return Err(FofaError::UnsupportedField {
                field: field.api_name.clone(),
                reason: "返回字段重复".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_relay_query(scan: &QueryScan) -> Result<(), FofaError> {
    if scan.has_wildcard_operator {
        return Err(FofaError::invalid("中转端点不支持 `*=` 运算符"));
    }

    for field in &scan.fields {
        if RELAY_CONFLICT_QUERY_FIELDS.contains(&field.as_str()) {
            return Err(FofaError::UnsupportedField {
                field: field.clone(),
                reason: "中转平台支持状态存在冲突，保守模式下禁用".to_owned(),
            });
        }
        if !RELAY_QUERY_FIELDS.contains(&field.as_str()) {
            return Err(FofaError::UnsupportedField {
                field: field.clone(),
                reason: "中转端点不支持该查询字段".to_owned(),
            });
        }
    }

    if scan.fields.iter().any(|field| field == "after")
        && scan.fields.iter().any(|field| field == "before")
    {
        return Err(FofaError::invalid(
            "中转端点不支持在同一查询中组合 after 与 before",
        ));
    }
    Ok(())
}

fn validate_supported_return_fields(
    mode: ApiMode,
    fields: &[ReturnField],
) -> Result<(), FofaError> {
    let catalog = supported_return_field_names(mode);
    let reason = match mode {
        ApiMode::Official => "FOFA 官方文档未列出该返回字段",
        ApiMode::Relay => "中转端点不支持该返回字段",
    };
    for field in fields {
        if !catalog.contains(&field.api_name.as_str()) {
            return Err(FofaError::UnsupportedField {
                field: field.api_name.clone(),
                reason: reason.to_owned(),
            });
        }
    }
    Ok(())
}

struct QueryScan {
    fields: Vec<String>,
    has_wildcard_operator: bool,
    has_and: bool,
    has_or: bool,
    has_parenthesis: bool,
}

fn scan_query(query: &str) -> Result<QueryScan, FofaError> {
    let bytes = query.as_bytes();
    let mut index = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut parenthesis_depth = 0_u32;
    let mut fields = Vec::new();
    let mut has_wildcard_operator = false;
    let mut has_and = false;
    let mut has_or = false;
    let mut has_parenthesis = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            index += utf8_char_len(byte);
            continue;
        }

        match byte {
            b'"' => {
                quoted = true;
                index += 1;
            }
            b'(' => {
                has_parenthesis = true;
                parenthesis_depth = parenthesis_depth.saturating_add(1);
                index += 1;
            }
            b')' => {
                has_parenthesis = true;
                if parenthesis_depth == 0 {
                    return Err(FofaError::invalid("查询语句括号不匹配"));
                }
                parenthesis_depth -= 1;
                index += 1;
            }
            b'&' if bytes.get(index + 1) == Some(&b'&') => {
                has_and = true;
                index += 2;
            }
            b'|' if bytes.get(index + 1) == Some(&b'|') => {
                has_or = true;
                index += 2;
            }
            b'*' if bytes.get(index + 1) == Some(&b'=') => {
                has_wildcard_operator = true;
                index += 2;
            }
            byte if is_identifier_start(byte) => {
                let start = index;
                index += 1;
                while index < bytes.len() && is_identifier_continue(bytes[index]) {
                    index += 1;
                }
                let end = index;
                while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                    index += 1;
                }

                let operator_len = if bytes.get(index..index + 2) == Some(b"*=") {
                    has_wildcard_operator = true;
                    2
                } else if matches!(bytes.get(index..index + 2), Some(b"==" | b"!=")) {
                    2
                } else if bytes.get(index) == Some(&b'=') {
                    1
                } else {
                    0
                };
                if operator_len > 0 {
                    fields.push(query[start..end].to_owned());
                }
            }
            _ => index += utf8_char_len(byte),
        }
    }

    if quoted {
        return Err(FofaError::invalid("查询语句包含未闭合的双引号"));
    }
    if parenthesis_depth != 0 {
        return Err(FofaError::invalid("查询语句括号不匹配"));
    }

    Ok(QueryScan {
        fields,
        has_wildcard_operator,
        has_and,
        has_or,
        has_parenthesis,
    })
}

fn utf8_char_len(first_byte: u8) -> usize {
    if first_byte < 0x80 {
        1
    } else if first_byte < 0xe0 {
        2
    } else if first_byte < 0xf0 {
        3
    } else {
        4
    }
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.')
}

const RELAY_CONFLICT_QUERY_FIELDS: &[&str] = &["is_fraud", "is_honeypot"];

const RELAY_QUERY_FIELDS: &[&str] = &[
    "ip",
    "port",
    "domain",
    "host",
    "os",
    "server",
    "asn",
    "org",
    "is_domain",
    "is_ipv6",
    "app",
    "fid",
    "product",
    "category",
    "type",
    "cloud_name",
    "is_cloud",
    "is_fraud",
    "is_honeypot",
    "protocol",
    "banner",
    "base_protocol",
    "title",
    "header",
    "header_hash",
    "body",
    "body_hash",
    "js_name",
    "js_md5",
    "cname",
    "cname_domain",
    "icon_hash",
    "status_code",
    "icp",
    "country",
    "region",
    "city",
    "cert",
    "cert.subject",
    "cert.issuer",
    "cert.subject.org",
    "cert.subject.cn",
    "cert.issuer.org",
    "cert.issuer.cn",
    "cert.domain",
    "cert.is_equal",
    "cert.is_valid",
    "cert.is_match",
    "cert.is_expired",
    "jarm",
    "tls.version",
    "tls.ja3s",
    "after",
    "before",
];

#[cfg(test)]
#[path = "../../tests/unit/fofa/validator.rs"]
mod tests;
