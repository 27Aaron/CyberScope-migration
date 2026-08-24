use std::{fs, io::Read};

use pretty_assertions::assert_eq;
use tempfile::tempdir;

use super::*;

fn record(cells: &[&str]) -> ExportRecord {
    ExportRecord::new(cells.iter().map(|cell| (*cell).to_owned()).collect())
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

#[test]
fn csv_uses_declared_order_bom_and_standard_escaping() {
    let root = tempdir().unwrap();
    let mut exporter = Exporter::new(
        root.path(),
        vec!["title".into(), "source_query".into(), "header".into()],
        ExportOptions::default(),
    )
    .unwrap();
    exporter
        .write_batch(&[record(&[
            "hello, \"FOFA\"",
            "title=\"管理后台\"",
            "line one\nline two",
        ])])
        .unwrap();

    let bytes = read(&exporter.paths()[0]);
    assert!(bytes.starts_with(csv::UTF8_BOM));
    let body = std::str::from_utf8(&bytes[csv::UTF8_BOM.len()..]).unwrap();
    assert_eq!(
        body,
        "title,source_query,header\n\"hello, \"\"FOFA\"\"\",\"title=\"\"管理后台\"\"\",\"line one\nline two\"\n"
    );
    assert_eq!(exporter.row_count(), 1);
}

#[test]
fn csv_bom_can_be_disabled() {
    let root = tempdir().unwrap();
    let mut exporter =
        Exporter::new(root.path(), vec!["ip".into()], ExportOptions::csv(false)).unwrap();
    exporter.write_batch(&[record(&["1.2.3.4"])]).unwrap();
    assert_eq!(read(&exporter.paths()[0]), b"ip\n1.2.3.4\n");
}

#[test]
fn txt_preserves_multiline_cells_and_global_record_numbers() {
    let root = tempdir().unwrap();
    let mut exporter = Exporter::new(
        root.path(),
        vec!["ip".into(), "banner".into(), "source_cidr".into()],
        ExportOptions::txt(),
    )
    .unwrap();
    exporter
        .write_batch(&[
            record(&["1.2.3.4", "SSH-2.0\nready", "1.2.3.0/24"]),
            record(&["5.6.7.8", "HTTP", "5.6.7.0/24"]),
        ])
        .unwrap();

    let text = String::from_utf8(read(&exporter.paths()[0])).unwrap();
    assert_eq!(
        text,
        concat!(
            "========== RECORD 000001 ==========\n",
            "ip: 1.2.3.4\n",
            "banner:\nSSH-2.0\nready\n",
            "source_cidr: 1.2.3.0/24\n\n",
            "========== RECORD 000002 ==========\n",
            "ip: 5.6.7.8\n",
            "banner: HTTP\n",
            "source_cidr: 5.6.7.0/24\n\n",
        )
    );
}

#[test]
fn exact_limit_stays_in_one_part_and_next_byte_rolls() {
    let root = tempdir().unwrap();
    // Header and each row are two bytes.
    let options = ExportOptions {
        csv_bom: false,
        max_part_bytes: 6,
        ..ExportOptions::default()
    };
    let mut exporter = Exporter::new(root.path(), vec!["v".into()], options).unwrap();
    exporter
        .write_batch(&[record(&["x"]), record(&["x"]), record(&["x"])])
        .unwrap();

    assert_eq!(exporter.part_count(), 2);
    assert_eq!(read(&exporter.paths()[0]), b"v\nx\nx\n");
    assert_eq!(read(&exporter.paths()[1]), b"v\nx\n");
    assert_eq!(
        exporter
            .parts()
            .iter()
            .map(|part| part.row_count)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
}

#[test]
fn oversized_record_is_never_truncated_or_preceded_by_empty_part() {
    let root = tempdir().unwrap();
    let options = ExportOptions {
        csv_bom: false,
        max_part_bytes: 4,
        ..ExportOptions::default()
    };
    let huge = "a".repeat(100);
    let mut exporter = Exporter::new(root.path(), vec!["v".into()], options).unwrap();
    exporter
        .write_batch(&[ExportRecord::new(vec![huge.clone()])])
        .unwrap();

    assert_eq!(exporter.part_count(), 1);
    let bytes = read(&exporter.paths()[0]);
    assert_eq!(bytes, format!("v\n{huge}\n").into_bytes());
    assert!(exporter.parts()[0].byte_len > options.max_part_bytes);

    exporter.write_batch(&[record(&["b"])]).unwrap();
    assert_eq!(exporter.part_count(), 2);
    assert_eq!(read(&exporter.paths()[1]), b"v\nb\n");
}

#[test]
fn mismatch_is_rejected_before_any_batch_cell_is_written() {
    let root = tempdir().unwrap();
    let mut exporter = Exporter::new(
        root.path(),
        vec!["ip".into(), "source_query".into()],
        ExportOptions::csv(false),
    )
    .unwrap();
    let before = read(&exporter.paths()[0]);

    let error = exporter
        .write_batch(&[record(&["ok", "q"]), record(&["missing"])])
        .unwrap_err();
    assert!(matches!(
        error,
        ExportError::CellCountMismatch {
            record_index: 2,
            expected: 2,
            actual: 1
        }
    ));
    assert_eq!(read(&exporter.paths()[0]), before);
    assert_eq!(exporter.row_count(), 0);
}

#[test]
fn failed_batch_rolls_back_rows_and_all_new_parts() {
    let root = tempdir().unwrap();
    let options = ExportOptions {
        csv_bom: false,
        max_part_bytes: 6,
        ..ExportOptions::default()
    };
    let mut exporter = Exporter::new(root.path(), vec!["v".into()], options).unwrap();
    exporter.write_batch(&[record(&["a"])]).unwrap();
    let original_path = exporter.paths()[0].clone();
    let before = read(&original_path);

    // Force a part rollover before injecting a write failure.
    exporter.inject_failure_after_records(2);
    let error = exporter
        .write_batch(&[record(&["b"]), record(&["c"]), record(&["d"])])
        .unwrap_err();
    assert!(matches!(error, ExportError::InjectedFailure));
    assert_eq!(exporter.paths(), vec![original_path.clone()]);
    assert_eq!(read(&original_path), before);
    assert_eq!(exporter.row_count(), 1);
    assert_eq!(exporter.parts()[0].row_count, 1);

    exporter.fail_after_records = None;
    exporter.write_batch(&[record(&["z"])]).unwrap();
    assert_eq!(exporter.row_count(), 2);
}

#[test]
fn paths_are_random_private_and_beneath_caller_root() {
    let root = tempdir().unwrap();
    let exporter_a =
        Exporter::new(root.path(), vec!["ip".into()], ExportOptions::default()).unwrap();
    let exporter_b =
        Exporter::new(root.path(), vec!["ip".into()], ExportOptions::default()).unwrap();
    let path_a = exporter_a.paths()[0].clone();
    let path_b = exporter_b.paths()[0].clone();

    assert_ne!(path_a, path_b);
    assert_eq!(
        path_a.parent(),
        Some(fs::canonicalize(root.path()).unwrap().as_path())
    );
    assert!(
        path_a
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".fofa-export-")
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path_a).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn dropping_exporter_removes_every_part() {
    let root = tempdir().unwrap();
    let paths = {
        let options = ExportOptions {
            csv_bom: false,
            max_part_bytes: 4,
            ..ExportOptions::default()
        };
        let mut exporter = Exporter::new(root.path(), vec!["v".into()], options).unwrap();
        exporter
            .write_batch(&[record(&["one"]), record(&["two"])])
            .unwrap();
        let paths = exporter.paths();
        assert!(paths.iter().all(|path| path.exists()));
        paths
    };
    assert!(paths.iter().all(|path| !path.exists()));
}

#[test]
fn json_cells_and_local_cells_are_explicit() {
    let mut record = ExportRecord::from_json_values(&[
        Value::Null,
        Value::String("原样".into()),
        Value::from(443),
        Value::Bool(true),
    ]);
    record.push_local_cell("source");
    assert_eq!(record.cells, ["", "原样", "443", "true", "source"]);
}

#[test]
fn csv_injection_cells_are_neutralized_with_a_tab_prefix() {
    // Formula-like values receive a neutral tab prefix.
    for hostile in ["=cmd", "+1", "-2", "@x", "\n=1", "\r=2"] {
        let sanitized = sanitize_cell(hostile);
        assert!(sanitized.starts_with('\t'), "未消毒: {hostile:?}");
        assert_ne!(sanitized.as_str(), hostile);
    }
    // Existing prefixes are preserved.
    assert_eq!(sanitize_cell("\t=1"), "\t=1");
    // Ordinary values are unchanged.
    for benign in ["1.2.3.4", "host", "管理后台", "", "a=b", "443"] {
        assert_eq!(sanitize_cell(benign), benign);
    }
}

#[test]
fn json_string_cells_pass_through_csv_injection_sanitization() {
    let record = ExportRecord::from_json_values(&[Value::String("=SUM(A1)".into())]);
    assert_eq!(record.cells, ["\t=SUM(A1)"]);
    let nested = ExportRecord::from_json_values(&[serde_json::json!([1, "ok"])]);
    assert_eq!(nested.cells, ["[1,\"ok\"]"]);
}

#[test]
fn json_batch_streams_conversion_and_appends_one_local_cell() {
    let root = tempdir().unwrap();
    let mut exporter = Exporter::new(
        root.path(),
        vec!["ip".into(), "port".into(), "source_query".into()],
        ExportOptions::csv(false),
    )
    .unwrap();
    exporter
        .write_json_batch(
            &[vec![Value::String("192.0.2.1".into()), Value::from(443)]],
            Some(r#"port="443""#),
        )
        .unwrap();

    assert_eq!(
        read(&exporter.paths()[0]),
        b"ip,port,source_query\n192.0.2.1,443,\"port=\"\"443\"\"\"\n"
    );
}

#[test]
fn write_json_batch_neutralizes_formula_cells() {
    let root = tempdir().unwrap();
    let mut exporter =
        Exporter::new(root.path(), vec!["title".into()], ExportOptions::csv(false)).unwrap();
    exporter
        .write_json_batch(
            &[vec![Value::String("=HYPERLINK(\"http://evil\")".into())]],
            None,
        )
        .unwrap();

    let bytes = read(&exporter.paths()[0]);
    // The CSV writer quotes the formula; the tab neutralizes it.
    assert_eq!(bytes, b"title\n\"\t=HYPERLINK(\"\"http://evil\"\")\"\n");
}

#[test]
fn write_batch_neutralizes_raw_cells() {
    let root = tempdir().unwrap();
    let mut exporter =
        Exporter::new(root.path(), vec!["title".into()], ExportOptions::csv(false)).unwrap();
    exporter
        .write_batch(&[record(&["=HYPERLINK(\"http://evil\")"])])
        .unwrap();

    assert_eq!(
        read(&exporter.paths()[0]),
        b"title\n\"\t=HYPERLINK(\"\"http://evil\"\")\"\n"
    );
}

#[test]
fn hostile_local_cells_are_also_neutralized() {
    // Local columns are user-controlled and require the same protection.
    let root = tempdir().unwrap();
    let mut exporter = Exporter::new(
        root.path(),
        vec!["ip".into(), "source_query".into()],
        ExportOptions::csv(false),
    )
    .unwrap();
    exporter
        .write_json_batch(
            &[vec![Value::String("192.0.2.1".into())]],
            Some("=cmd|' /C calc'!A0"),
        )
        .unwrap();

    let bytes = read(&exporter.paths()[0]);
    // A tab prefix is not quoted; verify it precedes the formula marker.
    assert_eq!(bytes, b"ip,source_query\n192.0.2.1,\t=cmd|' /C calc'!A0\n");
}

#[test]
fn part_limit_rolls_back_only_the_rejected_batch() {
    let root = tempdir().unwrap();
    let options = ExportOptions {
        csv_bom: false,
        max_part_bytes: 4,
        max_parts: 1,
        ..ExportOptions::default()
    };
    let mut exporter = Exporter::new(root.path(), vec!["v".into()], options).unwrap();
    exporter.write_batch(&[record(&["a"])]).unwrap();
    let before = read(&exporter.paths()[0]);

    let error = exporter.write_batch(&[record(&["b"])]).unwrap_err();
    assert!(matches!(
        error,
        ExportError::PartLimitExceeded { max_parts: 1 }
    ));
    assert_eq!(exporter.row_count(), 1);
    assert_eq!(read(&exporter.paths()[0]), before);
}

#[test]
fn constructor_rejects_invalid_schema_and_root() {
    let root = tempdir().unwrap();
    assert!(matches!(
        Exporter::new(root.path(), vec![], ExportOptions::default()),
        Err(ExportError::NoFields)
    ));
    assert!(matches!(
        Exporter::new(root.path(), vec!["bad\nfield".into()], ExportOptions::txt()),
        Err(ExportError::InvalidTextField { index: 0 })
    ));

    let file_path = root.path().join("not-a-directory");
    fs::write(&file_path, b"x").unwrap();
    assert!(matches!(
        Exporter::new(&file_path, vec!["ip".into()], ExportOptions::default()),
        Err(ExportError::InvalidTempRoot(path)) if path == file_path
    ));
}

#[test]
fn readable_while_alive_and_open_file_position_survives_rollback() {
    let root = tempdir().unwrap();
    let mut exporter =
        Exporter::new(root.path(), vec!["v".into()], ExportOptions::csv(false)).unwrap();
    exporter.write_batch(&[record(&["a"])]).unwrap();
    exporter.inject_failure_after_records(1);
    let _ = exporter.write_batch(&[record(&["b"]), record(&["c"])]);
    exporter.fail_after_records = None;
    exporter.write_batch(&[record(&["d"])]).unwrap();

    let mut contents = String::new();
    fs::File::open(&exporter.paths()[0])
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents, "v\na\nd\n");
}
