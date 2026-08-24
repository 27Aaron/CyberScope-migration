//! Parse and normalize `/batch` TXT uploads.

use std::collections::HashSet;

use ipnet::IpNet;
use thiserror::Error;

/// Maximum upload size.
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Maximum number of effective input lines.
pub const MAX_BATCH_LINES: usize = 10_000;
/// Maximum byte length of one input line.
pub const MAX_QUERY_LINE_BYTES: usize = 16 * 1024;

/// How to interpret each effective input line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchMode {
    /// Each line is a canonical IPv4 or IPv6 network.
    Cidr { base_query: String },
    /// Each line is a complete FOFA query.
    FullQuery,
}

impl BatchMode {
    /// Create CIDR mode. An empty base query is allowed.
    pub fn cidr(base_query: impl Into<String>) -> Self {
        Self::Cidr {
            base_query: base_query.into(),
        }
    }

    /// Create complete-query mode.
    pub const fn full_query() -> Self {
        Self::FullQuery
    }
}

/// Batch parser limits and normalization options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchParseOptions {
    pub max_upload_bytes: usize,
    pub max_batch_lines: usize,
    pub max_query_line_bytes: usize,
    /// Remove later duplicates after normalization.
    pub deduplicate: bool,
}

impl Default for BatchParseOptions {
    fn default() -> Self {
        Self {
            max_upload_bytes: MAX_UPLOAD_BYTES,
            max_batch_lines: MAX_BATCH_LINES,
            max_query_line_bytes: MAX_QUERY_LINE_BYTES,
            deduplicate: false,
        }
    }
}

impl BatchParseOptions {
    /// Toggle duplicate removal.
    pub const fn with_deduplication(mut self, deduplicate: bool) -> Self {
        self.deduplicate = deduplicate;
        self
    }
}

/// Source column appended to batch exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchSourceKind {
    Cidr,
    Query,
}

impl BatchSourceKind {
    /// Export header.
    pub const fn export_field(self) -> &'static str {
        match self {
            Self::Cidr => "source_cidr",
            Self::Query => "source_query",
        }
    }
}

/// One normalized, executable batch line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchItem {
    /// One-based line number in the decoded input.
    pub line_number: usize,
    /// Trimmed input line.
    pub source: String,
    /// Query sent to FOFA.
    pub query: String,
}

/// Compatibility alias for callers using "entry" terminology.
pub type BatchEntry = BatchItem;

/// Parsed batch contents and counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchDocument {
    pub source_kind: BatchSourceKind,
    pub items: Vec<BatchItem>,
    /// Effective input lines before deduplication.
    pub effective_lines: usize,
    /// Ignored blank and comment lines.
    pub ignored_lines: usize,
    /// Duplicates removed from `items`.
    pub deduplicated_lines: usize,
}

impl BatchDocument {
    pub fn items(&self) -> &[BatchItem] {
        &self.items
    }

    pub fn entries(&self) -> &[BatchItem] {
        &self.items
    }

    pub fn into_items(self) -> Vec<BatchItem> {
        self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BatchError {
    #[error("batch upload is {actual} bytes; the limit is {max} bytes")]
    UploadTooLarge { actual: usize, max: usize },
    #[error("batch upload is not valid UTF-8 (valid through byte {valid_up_to})")]
    InvalidUtf8 { valid_up_to: usize },
    #[error("batch upload contains binary/control data at byte {offset}")]
    BinaryData { offset: usize },
    #[error("line {line_number} is {actual} bytes; the limit is {max} bytes")]
    LineTooLong {
        line_number: usize,
        actual: usize,
        max: usize,
    },
    #[error("line {line_number} exceeds the limit of {max} effective lines")]
    TooManyLines { line_number: usize, max: usize },
    #[error("line {line_number} is not a valid CIDR: {value}")]
    InvalidCidr { line_number: usize, value: String },
    #[error("line {line_number} is not a canonical network CIDR: {value} (canonical: {canonical})")]
    NonCanonicalCidr {
        line_number: usize,
        value: String,
        canonical: String,
    },
}

/// Parse an uploaded TXT file using the selected mode.
///
/// Accepts UTF-8 BOM/CRLF, trims effective lines, ignores blanks and comments,
/// and preserves original line numbers.
pub fn parse_batch(
    bytes: &[u8],
    mode: BatchMode,
    options: &BatchParseOptions,
) -> Result<BatchDocument, BatchError> {
    if bytes.len() > options.max_upload_bytes {
        return Err(BatchError::UploadTooLarge {
            actual: bytes.len(),
            max: options.max_upload_bytes,
        });
    }

    // Validate UTF-8 first so invalid input reports its byte offset.
    let text = std::str::from_utf8(bytes).map_err(|error| BatchError::InvalidUtf8 {
        valid_up_to: error.valid_up_to(),
    })?;
    reject_binary_controls(text)?;

    // Strip only a leading BOM; it is file metadata, not query data.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let source_kind = match mode {
        BatchMode::Cidr { .. } => BatchSourceKind::Cidr,
        BatchMode::FullQuery => BatchSourceKind::Query,
    };
    let base_query = match &mode {
        BatchMode::Cidr { base_query } => Some(base_query.trim()),
        BatchMode::FullQuery => None,
    };

    let mut items = Vec::new();
    let mut effective_lines = 0usize;
    let mut ignored_lines = 0usize;
    let mut deduplicated_lines = 0usize;
    let mut seen = options.deduplicate.then(HashSet::<String>::new);

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let source = raw_line.trim();

        if source.is_empty() || source.starts_with('#') {
            ignored_lines += 1;
            continue;
        }

        // Enforce the raw line budget before trimming.
        let line_bytes = raw_line.len();
        if line_bytes > options.max_query_line_bytes {
            return Err(BatchError::LineTooLong {
                line_number,
                actual: line_bytes,
                max: options.max_query_line_bytes,
            });
        }

        effective_lines += 1;
        if effective_lines > options.max_batch_lines {
            return Err(BatchError::TooManyLines {
                line_number,
                max: options.max_batch_lines,
            });
        }

        if let Some(seen) = &mut seen
            && !seen.insert(source.to_owned())
        {
            deduplicated_lines += 1;
            continue;
        }

        let query = match base_query {
            Some(base_query) => {
                let network = parse_canonical_cidr(source, line_number)?;
                let query = compose_cidr_query(base_query, &network.to_string());
                // Validate the composed query before execution.
                if query.len() > crate::fofa::validator::MAX_QUERY_CHARS {
                    return Err(BatchError::LineTooLong {
                        line_number,
                        actual: query.len(),
                        max: crate::fofa::validator::MAX_QUERY_CHARS,
                    });
                }
                query
            }
            None => source.to_owned(),
        };

        items.push(BatchItem {
            line_number,
            source: source.to_owned(),
            query,
        });
    }

    Ok(BatchDocument {
        source_kind,
        items,
        effective_lines,
        ignored_lines,
        deduplicated_lines,
    })
}

/// Parse with default limits and deduplication disabled.
pub fn parse_batch_default(bytes: &[u8], mode: BatchMode) -> Result<BatchDocument, BatchError> {
    parse_batch(bytes, mode, &BatchParseOptions::default())
}

/// Compose the query used by CIDR mode.
///
/// `cidr` must already be canonical; this function only formats the query.
pub fn compose_cidr_query(base_query: &str, cidr: &str) -> String {
    let base_query = base_query.trim();
    if base_query.is_empty() {
        format!(r#"ip="{cidr}""#)
    } else {
        format!(r#"({base_query}) && ip="{cidr}""#)
    }
}

fn parse_canonical_cidr(source: &str, line_number: usize) -> Result<IpNet, BatchError> {
    let network = source
        .parse::<IpNet>()
        .map_err(|_| BatchError::InvalidCidr {
            line_number,
            value: source.to_owned(),
        })?;

    let truncated = network.trunc();
    let canonical = truncated.to_string();
    if network != truncated || source != canonical {
        return Err(BatchError::NonCanonicalCidr {
            line_number,
            value: source.to_owned(),
            canonical,
        });
    }

    Ok(network)
}

fn reject_binary_controls(text: &str) -> Result<(), BatchError> {
    for (offset, character) in text.char_indices() {
        if !character.is_control() {
            continue;
        }

        let allowed = match character {
            '\n' | '\t' => true,
            // Accept CR only as part of CRLF.
            '\r' => text.as_bytes().get(offset + 1) == Some(&b'\n'),
            _ => false,
        };
        if !allowed {
            return Err(BatchError::BinaryData { offset });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/batch.rs"]
mod tests;
