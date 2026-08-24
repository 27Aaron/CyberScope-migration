use axum::http::{HeaderMap, HeaderValue, header};

use super::*;

#[test]
fn extracts_the_named_cookie_from_multiple_values() {
    let mut headers = HeaderMap::new();
    headers.append(
        header::COOKIE,
        HeaderValue::from_static("theme=dark; cyberscope_session=abc123"),
    );

    assert_eq!(session_token(&headers), Some("abc123"));
}

#[test]
fn session_cookie_is_http_only_strict_and_scoped_to_root() {
    let cookie = session_cookie(&"a".repeat(64));
    let value = cookie.to_str().unwrap();

    assert!(value.starts_with("cyberscope_session="));
    assert!(value.contains("Path=/"));
    assert!(value.contains("HttpOnly"));
    assert!(value.contains("SameSite=Strict"));
    assert!(value.contains("Max-Age=28800"));
}

#[test]
fn expired_cookie_clears_the_session() {
    let value = expired_session_cookie().to_str().unwrap().to_owned();

    assert!(value.contains("cyberscope_session=;"));
    assert!(value.contains("Max-Age=0"));
}
