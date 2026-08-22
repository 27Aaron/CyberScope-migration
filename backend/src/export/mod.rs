//! Streaming, transactional result-file export.
//!
//! `Exporter` owns every part file. Keep it alive while the paths are being
//! uploaded; dropping it removes all of them.

mod csv;
mod text;

use std::{
    fs,
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use serde_json::Value;
use tempfile::{Builder, NamedTempFile};
use thiserror::Error;

/// Keep individual export parts reasonably sized for browser downloads and
/// future delivery adapters.
pub const DEFAULT_MAX_PART_BYTES: u64 = 45 * 1024 * 1024;
/// Prevent one task from filling the disk or producing an unbounded download
/// queue. With the default part size this caps normal output near 450 MiB.
pub const DEFAULT_MAX_PARTS: usize = 10;
/// Backwards-friendly name for callers that think of the limit as a split size.
pub const FILE_SPLIT_BYTES: u64 = DEFAULT_MAX_PART_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Txt,
}

impl ExportFormat {
    fn suffix(self) -> &'static str {
        match self {
            Self::Csv => ".csv",
            Self::Txt => ".txt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// Applied to every CSV part so each part opens independently in Excel.
    /// Ignored for TXT output.
    pub csv_bom: bool,
    /// A part rolls before a record that would make it exceed this size. A
    /// single oversized record is kept intact and is allowed to exceed it.
    pub max_part_bytes: u64,
    pub max_parts: usize,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Csv,
            csv_bom: true,
            max_part_bytes: DEFAULT_MAX_PART_BYTES,
            max_parts: DEFAULT_MAX_PARTS,
        }
    }
}

impl ExportOptions {
    pub fn csv(csv_bom: bool) -> Self {
        Self {
            csv_bom,
            ..Self::default()
        }
    }

    pub fn txt() -> Self {
        Self {
            format: ExportFormat::Txt,
            csv_bom: false,
            ..Self::default()
        }
    }
}

/// One output row. `cells` must already include any local-only columns such as
/// `source_query` or `source_cidr`; those values must never be sent upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRecord {
    pub cells: Vec<String>,
}

impl ExportRecord {
    pub fn new(cells: Vec<String>) -> Self {
        Self { cells }
    }

    /// Convert a normalized FOFA row without coupling the exporter to the HTTP
    /// model. Null is an empty cell, strings are preserved, and scalar values
    /// use their JSON spelling. The compact fallback is defensive; the FOFA
    /// normalizer normally rejects nested cells.
    pub fn from_json_values(values: &[Value]) -> Self {
        Self {
            cells: values.iter().map(json_value_to_cell).collect(),
        }
    }

    pub fn push_local_cell(&mut self, value: impl Into<String>) {
        self.cells.push(value.into());
    }
}

impl From<Vec<String>> for ExportRecord {
    fn from(cells: Vec<String>) -> Self {
        Self::new(cells)
    }
}

impl<const N: usize> From<[&str; N]> for ExportRecord {
    fn from(cells: [&str; N]) -> Self {
        Self::new(cells.into_iter().map(str::to_owned).collect())
    }
}

pub fn json_value_to_cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartInfo {
    pub path: PathBuf,
    pub row_count: u64,
    pub byte_len: u64,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("at least one output field is required")]
    NoFields,
    #[error("TXT field {index} contains a line break")]
    InvalidTextField { index: usize },
    #[error("temporary root is not a directory: {0}")]
    InvalidTempRoot(PathBuf),
    #[error("record {record_index} has {actual} cells, but the export has {expected} fields")]
    CellCountMismatch {
        record_index: usize,
        expected: usize,
        actual: usize,
    },
    #[error("the exporter is unusable because an earlier rollback failed")]
    Poisoned,
    #[error("result reached the configured maximum of {max_parts} file parts")]
    PartLimitExceeded { max_parts: usize },
    #[error("CSV encoding failed: {0}")]
    Csv(#[from] ::csv::Error),
    #[error("file operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("batch write failed ({write_error}); rollback also failed ({rollback_error})")]
    RollbackFailed {
        write_error: Box<ExportError>,
        rollback_error: io::Error,
    },
    #[cfg(test)]
    #[error("injected test write failure")]
    InjectedFailure,
}

struct Part {
    file: NamedTempFile,
    bytes: u64,
    rows: u64,
}

#[derive(Debug, Clone, Copy)]
struct BatchSnapshot {
    part_count: usize,
    current_bytes: u64,
    current_rows: u64,
    total_rows: u64,
}

/// Owns one or more private temporary export files.
pub struct Exporter {
    temp_root: PathBuf,
    fields: Vec<String>,
    options: ExportOptions,
    parts: Vec<Part>,
    row_count: u64,
    poisoned: bool,
    #[cfg(test)]
    fail_after_records: Option<usize>,
}

impl Exporter {
    pub fn new(
        temp_root: impl AsRef<Path>,
        fields: Vec<String>,
        options: ExportOptions,
    ) -> Result<Self, ExportError> {
        if fields.is_empty() {
            return Err(ExportError::NoFields);
        }
        if options.max_parts == 0 {
            return Err(ExportError::PartLimitExceeded { max_parts: 0 });
        }
        if options.format == ExportFormat::Txt
            && let Some(index) = fields.iter().position(|field| field.contains(['\r', '\n']))
        {
            return Err(ExportError::InvalidTextField { index });
        }

        let input_root = temp_root.as_ref();
        let metadata = fs::metadata(input_root)?;
        if !metadata.is_dir() {
            return Err(ExportError::InvalidTempRoot(input_root.to_path_buf()));
        }
        let temp_root = fs::canonicalize(input_root)?;

        let mut exporter = Self {
            temp_root,
            fields,
            options,
            parts: Vec::new(),
            row_count: 0,
            poisoned: false,
            #[cfg(test)]
            fail_after_records: None,
        };
        let first = exporter.create_part()?;
        exporter.parts.push(first);
        Ok(exporter)
    }

    pub fn fields(&self) -> &[String] {
        &self.fields
    }

    pub fn format(&self) -> ExportFormat {
        self.options.format
    }

    pub fn row_count(&self) -> u64 {
        self.row_count
    }

    pub fn part_count(&self) -> usize {
        self.parts.len()
    }

    /// Cloned paths are intentional: they remain usable by upload code without
    /// borrowing the exporter, but are valid only while this exporter lives.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.parts
            .iter()
            .map(|part| part.file.path().to_path_buf())
            .collect()
    }

    pub fn part_paths(&self) -> impl ExactSizeIterator<Item = &Path> {
        self.parts.iter().map(|part| part.file.path())
    }

    pub fn parts(&self) -> Vec<PartInfo> {
        self.parts
            .iter()
            .map(|part| PartInfo {
                path: part.file.path().to_path_buf(),
                row_count: part.rows,
                byte_len: part.bytes,
            })
            .collect()
    }

    /// Atomically append a complete upstream page. On any encoding, creation,
    /// write, or flush failure, all rows and newly-created parts from this call
    /// are removed.
    pub fn write_batch(&mut self, records: &[ExportRecord]) -> Result<(), ExportError> {
        if self.poisoned {
            return Err(ExportError::Poisoned);
        }
        for (index, record) in records.iter().enumerate() {
            if record.cells.len() != self.fields.len() {
                return Err(ExportError::CellCountMismatch {
                    record_index: index + 1,
                    expected: self.fields.len(),
                    actual: record.cells.len(),
                });
            }
        }

        let snapshot = self.begin_batch()?;

        let result = self.write_batch_inner(records);
        self.finish_batch(snapshot, result)
    }

    /// Transactionally append normalized JSON rows while converting only one
    /// row at a time. This avoids duplicating an entire large FOFA page in an
    /// intermediate `Vec<ExportRecord>`.
    pub fn write_json_batch(
        &mut self,
        rows: &[Vec<Value>],
        local_cell: Option<&str>,
    ) -> Result<(), ExportError> {
        let local_count = usize::from(local_cell.is_some());
        let remote_fields = self.fields.len().saturating_sub(local_count);
        for (index, row) in rows.iter().enumerate() {
            if row.len() != remote_fields {
                return Err(ExportError::CellCountMismatch {
                    record_index: index + 1,
                    expected: self.fields.len(),
                    actual: row.len() + local_count,
                });
            }
        }

        let snapshot = self.begin_batch()?;
        let result = self.write_json_batch_inner(rows, local_cell);
        self.finish_batch(snapshot, result)
    }

    /// Convenience for callers that already hold rows as vectors.
    pub fn write_cells_batch(&mut self, rows: &[Vec<String>]) -> Result<(), ExportError> {
        let records: Vec<_> = rows.iter().cloned().map(ExportRecord::new).collect();
        self.write_batch(&records)
    }

    /// Flush every part. `write_batch` already flushes on successful commit;
    /// this is useful immediately before handing paths to another process.
    pub fn flush(&mut self) -> Result<(), ExportError> {
        if self.poisoned {
            return Err(ExportError::Poisoned);
        }
        for part in &mut self.parts {
            part.file.as_file_mut().flush()?;
        }
        Ok(())
    }

    fn write_batch_inner(&mut self, records: &[ExportRecord]) -> Result<(), ExportError> {
        for (batch_index, record) in records.iter().enumerate() {
            self.maybe_inject_failure(batch_index)?;

            self.write_record_cells(&record.cells)?;
        }
        self.flush_current_part()?;
        Ok(())
    }

    fn write_json_batch_inner(
        &mut self,
        rows: &[Vec<Value>],
        local_cell: Option<&str>,
    ) -> Result<(), ExportError> {
        for row in rows {
            let mut cells: Vec<_> = row.iter().map(json_value_to_cell).collect();
            if let Some(local_cell) = local_cell {
                cells.push(local_cell.to_owned());
            }
            self.write_record_cells(&cells)?;
        }
        self.flush_current_part()?;
        Ok(())
    }

    fn write_record_cells(&mut self, cells: &[String]) -> Result<(), ExportError> {
        let encoded = match self.options.format {
            ExportFormat::Csv => csv::encode_record(cells)?,
            ExportFormat::Txt => text::encode_record(&self.fields, cells, self.row_count + 1),
        };
        self.roll_if_needed(encoded.len() as u64)?;

        let current = self.parts.last_mut().expect("exporter always has one part");
        current.file.as_file_mut().write_all(&encoded)?;
        current.bytes = current.bytes.saturating_add(encoded.len() as u64);
        current.rows += 1;
        self.row_count += 1;
        Ok(())
    }

    fn flush_current_part(&mut self) -> Result<(), ExportError> {
        self.parts
            .last_mut()
            .expect("exporter always has one part")
            .file
            .as_file_mut()
            .flush()?;
        Ok(())
    }

    fn begin_batch(&mut self) -> Result<BatchSnapshot, ExportError> {
        self.flush()?;
        let current = self.parts.last().expect("exporter always has one part");
        Ok(BatchSnapshot {
            part_count: self.parts.len(),
            current_bytes: current.bytes,
            current_rows: current.rows,
            total_rows: self.row_count,
        })
    }

    fn finish_batch(
        &mut self,
        snapshot: BatchSnapshot,
        result: Result<(), ExportError>,
    ) -> Result<(), ExportError> {
        if let Err(write_error) = result {
            if let Err(rollback_error) = self.rollback(snapshot) {
                self.poisoned = true;
                return Err(ExportError::RollbackFailed {
                    write_error: Box::new(write_error),
                    rollback_error,
                });
            }
            return Err(write_error);
        }
        Ok(())
    }

    fn roll_if_needed(&mut self, next_record_bytes: u64) -> Result<(), ExportError> {
        let current = self.parts.last().expect("exporter always has one part");
        let should_roll = current.rows > 0
            && current.bytes.saturating_add(next_record_bytes) > self.options.max_part_bytes;
        if should_roll {
            if self.parts.len() >= self.options.max_parts {
                return Err(ExportError::PartLimitExceeded {
                    max_parts: self.options.max_parts,
                });
            }
            self.parts
                .last_mut()
                .expect("exporter always has one part")
                .file
                .as_file_mut()
                .flush()?;
            let part = self.create_part()?;
            self.parts.push(part);
        }
        Ok(())
    }

    fn create_part(&self) -> Result<Part, ExportError> {
        let mut file = Builder::new()
            .prefix(".fofa-export-")
            .suffix(self.options.format.suffix())
            .tempfile_in(&self.temp_root)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file_mut()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }

        let preamble = match self.options.format {
            ExportFormat::Csv => csv::preamble(&self.fields, self.options.csv_bom)?,
            ExportFormat::Txt => Vec::new(),
        };
        file.as_file_mut().write_all(&preamble)?;
        file.as_file_mut().flush()?;
        Ok(Part {
            file,
            bytes: preamble.len() as u64,
            rows: 0,
        })
    }

    fn rollback(&mut self, snapshot: BatchSnapshot) -> io::Result<()> {
        while self.parts.len() > snapshot.part_count {
            // Close explicitly so a failed unlink is observable rather than
            // being swallowed by NamedTempFile's best-effort Drop cleanup.
            let part = self.parts.pop().expect("length checked above");
            part.file.close()?;
        }

        let current = self
            .parts
            .get_mut(snapshot.part_count - 1)
            .expect("snapshot refers to an existing part");
        let file = current.file.as_file_mut();
        file.set_len(snapshot.current_bytes)?;
        file.seek(SeekFrom::Start(snapshot.current_bytes))?;
        file.flush()?;
        current.bytes = snapshot.current_bytes;
        current.rows = snapshot.current_rows;
        self.row_count = snapshot.total_rows;
        Ok(())
    }

    #[cfg(test)]
    fn inject_failure_after_records(&mut self, count: usize) {
        self.fail_after_records = Some(count);
    }

    #[cfg(test)]
    fn maybe_inject_failure(&self, batch_index: usize) -> Result<(), ExportError> {
        if self.fail_after_records == Some(batch_index) {
            Err(ExportError::InjectedFailure)
        } else {
            Ok(())
        }
    }

    #[cfg(not(test))]
    fn maybe_inject_failure(&self, _batch_index: usize) -> Result<(), ExportError> {
        Ok(())
    }
}

impl Drop for Exporter {
    fn drop(&mut self) {
        // Being explicit documents the lifecycle guarantee. NamedTempFile's
        // drop unlinks each part, including paths cloned by `paths()`.
        self.parts.clear();
    }
}

#[cfg(test)]
#[path = "../../tests/unit/export.rs"]
mod tests;
