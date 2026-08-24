use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use secrecy::SecretString;
use tower::ServiceExt;
use url::Url;

use crate::{config::Config, state::AppState};

use super::*;

#[tokio::test]
async fn login_cookie_protects_api_routes_and_logout_revokes_it() {
    let database = tempfile::tempdir().unwrap();
    let config = Arc::new(Config {
        fofa_api_key: SecretString::from("fofa-key".to_owned()),
        fofa_api_base_url: Url::parse("https://fofa.info").unwrap(),
        relay_quota_enabled: false,
        web_bind_address: "127.0.0.1:3000".parse().unwrap(),
        web_admin_username: "admin".to_owned(),
        web_admin_password: SecretString::from("correct horse battery staple".to_owned()),
        database_path: database.path().to_path_buf(),
    });
    let app = router(Arc::new(AppState::new(config).await.unwrap()));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/fields")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"username":"admin","password":"correct horse battery staple"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();

    for uri in ["/api/v1/me", "/api/v1/fields"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/fields")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
