use pretty_assertions::assert_eq;

use super::*;
use crate::fofa::validator::MAX_QUERY_CHARS;

fn options(
    max_upload_bytes: usize,
    max_batch_lines: usize,
    max_query_line_bytes: usize,
) -> BatchParseOptions {
    BatchParseOptions {
        max_upload_bytes,
        max_batch_lines,
        max_query_line_bytes,
        deduplicate: false,
    }
}

#[test]
fn defaults_match_the_documented_exact_limits_and_do_not_deduplicate() {
    assert_eq!(MAX_UPLOAD_BYTES, 10_485_760);
    assert_eq!(MAX_BATCH_LINES, 10_000);
    assert_eq!(MAX_QUERY_LINE_BYTES, 16_384);
    assert_eq!(
        BatchParseOptions::default(),
        BatchParseOptions {
            max_upload_bytes: 10_485_760,
            max_batch_lines: 10_000,
            max_query_line_bytes: 16_384,
            deduplicate: false,
        }
    );
}

#[test]
fn accepts_utf8_bom_crlf_unicode_and_keeps_original_line_numbers() {
    let input = concat!(
        "\u{feff}# 文件说明\r\n",
        "\r\n",
        "  title=\"管理后台\" && country=\"CN\"  \r\n",
        "\t# another comment\r\n",
        "product=\"nginx\"\r\n"
    );
    let document = parse_batch_default(input.as_bytes(), BatchMode::FullQuery).unwrap();

    assert_eq!(document.source_kind, BatchSourceKind::Query);
    assert_eq!(document.effective_lines, 2);
    assert_eq!(document.ignored_lines, 3);
    assert_eq!(
        document.items,
        vec![
            BatchItem {
                line_number: 3,
                source: "title=\"管理后台\" && country=\"CN\"".into(),
                query: "title=\"管理后台\" && country=\"CN\"".into(),
            },
            BatchItem {
                line_number: 5,
                source: "product=\"nginx\"".into(),
                query: "product=\"nginx\"".into(),
            },
        ]
    );
}

#[test]
fn rejects_invalid_utf8_without_exposing_input() {
    assert_eq!(
        parse_batch_default(&[b'a', 0xff, b'b'], BatchMode::FullQuery),
        Err(BatchError::InvalidUtf8 { valid_up_to: 1 })
    );
}

#[test]
fn rejects_nul_and_other_binary_controls_but_allows_tabs() {
    assert_eq!(
        parse_batch_default(b"title=\"a\"\0\n", BatchMode::FullQuery),
        Err(BatchError::BinaryData { offset: 9 })
    );
    assert_eq!(
        parse_batch_default(b"a\rb", BatchMode::FullQuery),
        Err(BatchError::BinaryData { offset: 1 })
    );
    assert_eq!(
        parse_batch_default(b"\tquery\t\n", BatchMode::FullQuery)
            .unwrap()
            .items[0]
            .query,
        "query"
    );
}

#[test]
fn upload_limit_is_inclusive_and_counts_the_bom_and_line_endings() {
    let opts = options(4, 10, 10);
    assert!(parse_batch(b"x\r\n\n", BatchMode::FullQuery, &opts).is_ok());
    assert_eq!(
        parse_batch(b"x\r\n\n!", BatchMode::FullQuery, &opts),
        Err(BatchError::UploadTooLarge { actual: 5, max: 4 })
    );

    let bom_opts = options(4, 10, 10);
    assert!(parse_batch("\u{feff}x".as_bytes(), BatchMode::FullQuery, &bom_opts).is_ok());
}

#[test]
fn effective_line_limit_is_inclusive_and_is_checked_before_deduplication() {
    let opts = options(100, 2, 10);
    assert!(parse_batch(b"a\n# ignored\nb\n", BatchMode::FullQuery, &opts).is_ok());
    assert_eq!(
        parse_batch(b"a\n\nb\nc", BatchMode::FullQuery, &opts),
        Err(BatchError::TooManyLines {
            line_number: 4,
            max: 2
        })
    );

    let dedup = BatchParseOptions {
        deduplicate: true,
        ..opts
    };
    assert_eq!(
        parse_batch(b"same\nsame\nsame", BatchMode::FullQuery, &dedup),
        Err(BatchError::TooManyLines {
            line_number: 3,
            max: 2
        })
    );
}

#[test]
fn per_line_limit_is_inclusive_uses_utf8_bytes_and_excludes_crlf() {
    let opts = options(100, 10, 4);
    assert!(parse_batch(b"1234\r\n", BatchMode::FullQuery, &opts).is_ok());
    assert_eq!(
        parse_batch(b"12345\r\n", BatchMode::FullQuery, &opts),
        Err(BatchError::LineTooLong {
            line_number: 1,
            actual: 5,
            max: 4,
        })
    );
    // A CJK character occupies three UTF-8 bytes.
    assert!(parse_batch("界x".as_bytes(), BatchMode::FullQuery, &opts).is_ok());
    assert_eq!(
        parse_batch("界xx".as_bytes(), BatchMode::FullQuery, &opts),
        Err(BatchError::LineTooLong {
            line_number: 1,
            actual: 5,
            max: 4,
        })
    );
}

#[test]
fn oversized_comments_and_blank_lines_are_ignored_as_non_query_lines() {
    let opts = options(100, 1, 1);
    let document = parse_batch(b"#####\n     \nx", BatchMode::FullQuery, &opts).unwrap();
    assert_eq!(document.effective_lines, 1);
    assert_eq!(document.ignored_lines, 2);
    assert_eq!(document.items[0].line_number, 3);
}

#[test]
fn duplicates_are_preserved_by_default_with_their_own_line_numbers() {
    let document = parse_batch_default(b"q\nq\n", BatchMode::FullQuery).unwrap();
    assert_eq!(document.items.len(), 2);
    assert_eq!(document.items[0].line_number, 1);
    assert_eq!(document.items[1].line_number, 2);
    assert_eq!(document.deduplicated_lines, 0);
}

#[test]
fn optional_deduplication_is_stable_and_keeps_the_first_occurrence() {
    let opts = BatchParseOptions::default().with_deduplication(true);
    let document = parse_batch(b"a\n b \na\nb\n", BatchMode::FullQuery, &opts).unwrap();
    assert_eq!(
        document.items,
        vec![
            BatchItem {
                line_number: 1,
                source: "a".into(),
                query: "a".into(),
            },
            BatchItem {
                line_number: 2,
                source: "b".into(),
                query: "b".into(),
            },
        ]
    );
    assert_eq!(document.effective_lines, 4);
    assert_eq!(document.deduplicated_lines, 2);
}

#[test]
fn cidr_mode_accepts_only_strict_canonical_ipv4_and_ipv6_networks() {
    let document = parse_batch_default(
        b"192.0.2.0/24\n2001:db8::/32",
        BatchMode::cidr("port=\"443\""),
    )
    .unwrap();
    assert_eq!(document.source_kind, BatchSourceKind::Cidr);
    assert_eq!(document.items[0].source, "192.0.2.0/24");
    assert_eq!(document.items[1].source, "2001:db8::/32");

    assert!(matches!(
        parse_batch_default(b"192.0.2.7/24", BatchMode::cidr("")),
        Err(BatchError::NonCanonicalCidr { canonical, .. })
            if canonical == "192.0.2.0/24"
    ));
    assert!(matches!(
        parse_batch_default(b"2001:0db8::/32", BatchMode::cidr("")),
        Err(BatchError::NonCanonicalCidr { canonical, .. })
            if canonical == "2001:db8::/32"
    ));
    assert!(matches!(
        parse_batch_default(b"192.0.2.1", BatchMode::cidr("")),
        Err(BatchError::InvalidCidr { line_number: 1, .. })
    ));
}

#[test]
fn cidr_query_composition_is_exact_for_empty_and_nonempty_base_queries() {
    assert_eq!(compose_cidr_query("", "1.2.3.0/24"), r#"ip="1.2.3.0/24""#);
    assert_eq!(
        compose_cidr_query("   ", "2001:db8::/32"),
        r#"ip="2001:db8::/32""#
    );
    assert_eq!(
        compose_cidr_query("  title=\"login\" || product=\"nginx\"  ", "1.2.3.0/24"),
        r#"(title="login" || product="nginx") && ip="1.2.3.0/24""#
    );

    let document = parse_batch_default(
        b"1.2.3.0/24",
        BatchMode::cidr(" title=\"login\" || product=\"nginx\" "),
    )
    .unwrap();
    assert_eq!(
        document.items[0].query,
        r#"(title="login" || product="nginx") && ip="1.2.3.0/24""#
    );
}

#[test]
fn cidr_mode_rejects_queries_that_would_exceed_the_validator_cap() {
    // The CIDR line fits, but the composed query exceeds the validator cap.
    let base_len = MAX_QUERY_CHARS - "1.2.3.0/24".len() - r#"() && ip="""#.len() + 1;
    let base_query = "a".repeat(base_len);
    let error = parse_batch_default(b"1.2.3.0/24", BatchMode::cidr(base_query)).unwrap_err();
    assert!(matches!(error, BatchError::LineTooLong { .. }));

    // A composed query at the cap remains valid.
    let base_len = MAX_QUERY_CHARS - "1.2.3.0/24".len() - r#"() && ip="""#.len();
    let base_query = "a".repeat(base_len);
    let document = parse_batch_default(b"1.2.3.0/24", BatchMode::cidr(base_query)).unwrap();
    assert_eq!(document.items[0].query.len(), MAX_QUERY_CHARS);
}

#[test]
fn empty_or_comment_only_upload_is_a_valid_empty_batch() {
    let document = parse_batch_default(b"\n # only a comment\n", BatchMode::FullQuery).unwrap();
    assert!(document.is_empty());
    assert_eq!(document.entries(), &[]);
    assert_eq!(document.effective_lines, 0);
    assert_eq!(document.ignored_lines, 2);
}
