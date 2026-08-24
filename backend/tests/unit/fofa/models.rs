use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;

fn fields(names: &[&str]) -> Vec<ReturnField> {
    names.iter().copied().map(ReturnField::api).collect()
}

#[test]
fn normalizes_all_three_documented_result_shapes() {
    assert_eq!(
        normalize_results(json!(["1.1.1.1", "8.8.8.8"]), &fields(&["ip"])).unwrap(),
        vec![vec![json!("1.1.1.1")], vec![json!("8.8.8.8")]]
    );
    assert_eq!(
        normalize_results(
            json!([["1.1.1.1", 443], ["8.8.8.8", 53]]),
            &fields(&["ip", "port"]),
        )
        .unwrap(),
        vec![
            vec![json!("1.1.1.1"), json!(443)],
            vec![json!("8.8.8.8"), json!(53)]
        ]
    );
    assert_eq!(
        normalize_results(
            json!([{"port": 443, "extra": true, "ip": "1.1.1.1"}, {"ip": "2.2.2.2"}]),
            &fields(&["ip", "port"]),
        )
        .unwrap(),
        vec![
            vec![json!("1.1.1.1"), json!(443)],
            vec![json!("2.2.2.2"), Value::Null]
        ]
    );
}

#[test]
fn rejects_mixed_and_wrong_width_results() {
    assert!(normalize_results(json!(["ip", ["ip"]]), &fields(&["ip"])).is_err());
    assert!(normalize_results(json!([["ip"]]), &fields(&["ip", "port"])).is_err());
    assert!(normalize_results(json!(["ip"]), &fields(&["ip", "port"])).is_err());
}

#[test]
fn field_state_distinguishes_missing_null_and_value() {
    let envelope: UserInfoEnvelope = serde_json::from_value(json!({
        "data": {
            "status": 1,
            "expire_at": null,
            "activate_time": 1_800_000_000,
            "remaining_api_calls": -1
        }
    }))
    .unwrap();
    let quota = envelope.data.unwrap();

    assert_eq!(quota.status, FieldState::Value(1));
    assert_eq!(quota.expire_at, FieldState::Null);
    assert_eq!(
        quota.activate_time,
        FieldState::Value(FlexibleTime::Unix(1_800_000_000))
    );
    assert_eq!(quota.remaining_api_calls, FieldState::Value(-1));
    assert_eq!(quota.remaining_item_count, FieldState::Missing);
    assert!(!quota.protocol_compatible());
}

#[test]
fn maps_location_display_names_to_api_names() {
    let field = ReturnField::from_name("location.country_name");
    assert_eq!(field.output_name, "location.country_name");
    assert_eq!(field.api_name, "country_name");
}
