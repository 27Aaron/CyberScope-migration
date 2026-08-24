//! Parsing and normalization for `/batch` TXT uploads.
//!
//! The parser deliberately operates on bytes so that upload-size, encoding, and
//! binary-file checks happen before any query text is retained in memory.

use std::collections::HashSet;

use ipnet::IpNet;
use thiserror::Error;

/// Maximum accepted size of a batch upload (10 MiB).
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Maximum number of non-blank, non-comment lines in one upload.
pub const MAX_BATCH_LINES: usize = 10_000;
/// Maximum byte length of one effective input line (16 KiB).
pub const MAX_QUERY_LINE_BYTES: usize = 16 * 1024;

/// The explicitly selected interpretation of a batch file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchMode {
    /// Each effective line is a canonical IPv4 or IPv6 network.
    Cidr { base_query: String },
    /// Each effective line is already a complete FOFA query.
    FullQuery,
}

impl BatchMode {
    /// Construct CIDR mode. An empty or whitespace-only base query is allowed.
    pub fn cidr(base_query: impl Into<String>) -> Self {
        Self::Cidr {
            base_query: base_query.into(),
        }
    }

    /// Construct complete-query mode.
    pub const fn full_query() -> Self {
        Self::FullQuery
    }
}

/// Resource limits and optional normalization behavior for a batch upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchParseOptions {
    pub max_upload_bytes: usize,
    pub max_batch_lines: usize,
    pub max_query_line_bytes: usize,
    /// Remove later occurrences of the same normalized input line.
    ///
    /// This is intentionally disabled by default. Duplicate lines still count
    /// toward `max_batch_lines`, even when they are removed from the result.
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
    /// Return these options with duplicate removal enabled or disabled.
    pub const fn with_deduplication(mut self, deduplicate: bool) -> Self {
        self.deduplicate = deduplicate;
        self
    }
}

/// Which source column a batch result should use when it is exported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchSourceKind {
    Cidr,
    Query,
}

impl BatchSourceKind {
    /// Header to append to an exported result.
    pub const fn export_field(self) -> &'static str {
        match self {
            Self::Cidr => "source_cidr",
            Self::Query => "source_query",
        }
    }
}

/// One normalized, executable line from a batch upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchItem {
    /// One-based line number in the original decoded TXT file.
    pub line_number: usize,
    /// Trimmed input line (a CIDR or a complete query, depending on the mode).
    pub source: String,
    /// Complete FOFA query to execute.
    pub query: String,
}

/// Backwards-friendly synonym for callers that use "entry" terminology.
pub type BatchEntry = BatchItem;

/// Successfully parsed batch contents and summary counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchDocument {
    pub source_kind: BatchSourceKind,
    pub items: Vec<BatchItem>,
    /// Number of non-blank, non-comment input lines before deduplication.
    pub effective_lines: usize,
    /// Number of blank and comment lines ignored by the parser.
    pub ignored_lines: usize,
    /// Number of later duplicate lines removed from `items`.
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

/// Parse an uploaded TXT file according to an explicitly selected mode.
///
/// UTF-8 BOM and CRLF are accepted. Surrounding whitespace is removed from
/// effective lines; blank lines and lines whose first non-whitespace character
/// is `#` are ignored. The original one-based line number is retained.
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

    // Check the encoding before examining Unicode controls. Invalid UTF-8 is a
    // distinct user-facing problem and its byte offset is useful diagnostics.
    let text = std::str::from_utf8(bytes).map_err(|error| BatchError::InvalidUtf8 {
        valid_up_to: error.valid_up_to(),
    })?;
    reject_binary_controls(text)?;

    // A BOM is metadata only at the very beginning of a UTF-8 text file. It
    // must not become part of the first query or its per-line byte count.
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

        // Count bytes in the logical input line after BOM/line-ending removal,
        // but before trimming. Whitespace is still upload data and therefore
        // consumes the same per-line resource budget as query text.
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
                compose_cidr_query(base_query, &network.to_string())
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

/// Parse with the documented default limits and with deduplication disabled.
pub fn parse_batch_default(bytes: &[u8], mode: BatchMode) -> Result<BatchDocument, BatchError> {
    parse_batch(bytes, mode, &BatchParseOptions::default())
}

/// Compose the exact FOFA query used by CIDR mode.
///
/// `cidr` is expected to have already passed canonical CIDR validation. This
/// function only composes text, which also makes it useful to presentation and
/// confirmation layers without repeating parser internals.
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
            // A carriage return is text only as the first half of CRLF.
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
