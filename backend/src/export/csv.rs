use ::csv::WriterBuilder;

use super::ExportError;

pub(crate) const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

pub(crate) fn preamble(fields: &[String], with_bom: bool) -> Result<Vec<u8>, ExportError> {
    let mut output = Vec::new();
    if with_bom {
        output.extend_from_slice(UTF8_BOM);
    }
    output.extend_from_slice(&encode_record(fields)?);
    Ok(output)
}

pub(crate) fn encode_record(cells: &[String]) -> Result<Vec<u8>, ExportError> {
    // Encode into memory so the exporter can roll only at record boundaries.
    let mut writer = WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());
    writer.write_record(cells)?;
    writer
        .into_inner()
        .map_err(|error| ExportError::Io(error.into_error()))
}
