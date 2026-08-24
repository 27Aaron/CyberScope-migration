use std::fmt::Write as _;

pub(crate) fn encode_record(fields: &[String], cells: &[String], number: u64) -> Vec<u8> {
    let mut output = String::new();
    // The width is a minimum, not a hard row limit.
    let _ = writeln!(output, "========== RECORD {number:06} ==========");

    for (field, cell) in fields.iter().zip(cells) {
        if cell.contains(['\r', '\n']) {
            let _ = writeln!(output, "{field}:");
            output.push_str(cell);
            if !cell.ends_with('\n') {
                output.push('\n');
            }
        } else {
            let _ = writeln!(output, "{field}: {cell}");
        }
    }
    output.push('\n');
    output.into_bytes()
}
