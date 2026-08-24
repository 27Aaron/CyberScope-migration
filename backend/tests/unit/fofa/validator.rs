use super::*;

fn fields(names: &[&str]) -> Vec<ReturnField> {
    names.iter().copied().map(ReturnField::from_name).collect()
}

#[test]
fn api_mode_treats_only_fofa_info_as_official() {
    assert_eq!(
        ApiMode::from_base_url(&Url::parse("https://fofa.info").unwrap()),
        ApiMode::Official
    );
    assert_eq!(
        ApiMode::from_base_url(&Url::parse("https://fofa.info./").unwrap()),
        ApiMode::Official
    );
    assert_eq!(
        ApiMode::from_base_url(&Url::parse("https://api.fofa.info").unwrap()),
        ApiMode::Relay
    );
    assert_eq!(
        ApiMode::from_base_url(&Url::parse("https://fofa.info.evil.example").unwrap()),
        ApiMode::Relay
    );
    assert_eq!(
        ApiMode::from_base_url(&Url::parse("https://relay.example.invalid").unwrap()),
        ApiMode::Relay
    );
    assert_eq!(
        ApiMode::from_base_url(&Url::parse("https://relay.example.com").unwrap()),
        ApiMode::Relay
    );
}

#[test]
fn return_field_catalogs_match_the_documented_capabilities() {
    assert_eq!(OFFICIAL_RETURN_FIELD_NAMES.len(), 51);
    assert_eq!(RELAY_RETURN_FIELD_NAMES.len(), 40);
    assert_eq!(
        OFFICIAL_RETURN_FIELD_NAMES
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len(),
        OFFICIAL_RETURN_FIELD_NAMES.len()
    );
    assert_eq!(
        RELAY_RETURN_FIELD_NAMES
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len(),
        RELAY_RETURN_FIELD_NAMES.len()
    );
    assert!(
        RELAY_RETURN_FIELD_NAMES
            .iter()
            .all(|field| OFFICIAL_RETURN_FIELD_NAMES.contains(field))
    );
    assert_eq!(
        default_return_fields(ApiMode::Official)
            .iter()
            .map(|field| field.api_name.as_str())
            .collect::<Vec<_>>(),
        OFFICIAL_RETURN_FIELD_NAMES
    );
    assert_eq!(
        default_return_fields(ApiMode::Relay)
            .iter()
            .map(|field| field.api_name.as_str())
            .collect::<Vec<_>>(),
        RELAY_RETURN_FIELD_NAMES
    );
    for mode in [ApiMode::Official, ApiMode::Relay] {
        assert!(
            QueryValidator::new(mode)
                .validate(r#"ip="192.0.2.1""#, default_return_fields(mode))
                .is_ok()
        );
    }
    assert_eq!(
        field_limit(ApiMode::Official, &default_return_fields(ApiMode::Official)),
        OFFICIAL_BODY_REQUEST_SIZE
    );
    assert_eq!(
        field_limit(ApiMode::Relay, &default_return_fields(ApiMode::Relay)),
        LARGE_FIELD_REQUEST_SIZE
    );

    let official_only = OFFICIAL_RETURN_FIELD_NAMES
        .iter()
        .copied()
        .filter(|field| !RELAY_RETURN_FIELD_NAMES.contains(field))
        .collect::<Vec<_>>();
    assert_eq!(
        official_only,
        vec![
            "status_code",
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
        ]
    );
}

#[test]
fn relay_rejects_only_operators_outside_quoted_values() {
    let validator = QueryValidator::new(ApiMode::Relay);
    assert!(
        validator
            .validate(r#"banner*="mysql""#, fields(&["ip"]))
            .is_err()
    );
    assert!(
        validator
            .validate(r#"title="literal *= text""#, fields(&["ip"]))
            .is_ok()
    );
}

#[test]
fn relay_rejects_unsupported_conflicting_and_time_range_fields() {
    let validator = QueryValidator::new(ApiMode::Relay);
    assert!(
        validator
            .validate(r#"sdk_hash="x""#, fields(&["ip"]))
            .is_err()
    );
    assert!(
        validator
            .validate(r#"is_honeypot=true"#, fields(&["ip"]))
            .is_err()
    );
    assert!(
        validator
            .validate(
                r#"after="2026-01-01" && before="2026-08-01""#,
                fields(&["ip"]),
            )
            .is_err()
    );
}

#[test]
fn official_mode_does_not_apply_relay_capabilities() {
    let validated = QueryValidator::new(ApiMode::Official)
        .validate(r#"sdk_hash="x""#, fields(&["body"]))
        .unwrap();
    assert_eq!(validated.max_request_size(), 500);
}

#[test]
fn return_fields_are_bounded_by_each_mode_catalog() {
    let official = QueryValidator::new(ApiMode::Official);
    assert!(
        official
            .validate(r#"ip="192.0.2.1""#, fields(&["status_code", "body"]))
            .is_ok()
    );
    assert!(
        official
            .validate(r#"ip="192.0.2.1""#, fields(&["cloud_name"]))
            .is_err()
    );
    assert!(
        official
            .validate(r#"ip="192.0.2.1""#, fields(&["totally_unknown"]))
            .is_err()
    );

    let relay = QueryValidator::new(ApiMode::Relay);
    assert!(
        relay
            .validate(r#"ip="192.0.2.1""#, fields(&["product_category"]))
            .is_ok()
    );
    assert!(
        relay
            .validate(r#"ip="192.0.2.1""#, fields(&["status_code"]))
            .is_err()
    );
}

#[test]
fn relay_maps_location_fields_and_enforces_large_field_limit() {
    let validated = QueryValidator::new(ApiMode::Relay)
        .validate(
            r#"country="CN""#,
            fields(&["location.country_name", "header"]),
        )
        .unwrap();
    assert_eq!(validated.fields()[0].api_name, "country_name");
    assert_eq!(validated.max_request_size(), 2_000);
}

#[test]
fn mixed_boolean_operators_without_parentheses_are_a_warning() {
    let validated = QueryValidator::new(ApiMode::Relay)
        .validate(
            r#"country="CN" && title="login" || server="nginx""#,
            fields(&["ip"]),
        )
        .unwrap();
    assert_eq!(
        validated.warnings(),
        &[ValidationWarning::AmbiguousBooleanPrecedence]
    );

    let parenthesized = QueryValidator::new(ApiMode::Relay)
        .validate(
            r#"country="CN" && (title="login" || server="nginx")"#,
            fields(&["ip"]),
        )
        .unwrap();
    assert!(parenthesized.warnings().is_empty());
}
