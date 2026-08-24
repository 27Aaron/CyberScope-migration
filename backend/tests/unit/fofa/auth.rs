use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::json;
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{header, method, path},
};

use super::*;
use crate::fofa::models::FieldState;

fn manager(server: &MockServer) -> RelayQuotaAuthManager {
    RelayQuotaAuthManager::new(
        reqwest::Client::new(),
        Url::parse(&server.uri()).unwrap(),
        SecretString::from("relay-key"),
    )
    .unwrap()
}

#[tokio::test]
async fn caches_login_token_for_later_quota_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "cached"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/auth/userinfo"))
        .and(header("authorization", "Bearer cached"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {
                "status": 1,
                "activate_time": null,
                "expire_at": null,
                "expire_seconds": 1,
                "remaining_api_calls": 497,
                "remaining_item_count": 19997990
            }
        })))
        .expect(2)
        .mount(&server)
        .await;

    let manager = manager(&server);
    let first = manager.userinfo(&CancellationToken::new()).await.unwrap();
    let second = manager.userinfo(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        first.data.unwrap().remaining_api_calls,
        FieldState::Value(497)
    );
    assert!(second.protocol_compatible());
    assert!(manager.has_cached_token().await);
}

struct LoginSequence {
    calls: Arc<AtomicUsize>,
}

impl Respond for LoginSequence {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let token = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            "expired"
        } else {
            "fresh"
        };
        ResponseTemplate::new(200).set_body_json(json!({"token": token}))
    }
}

#[tokio::test]
async fn refreshes_once_after_401() {
    let server = MockServer::start().await;
    let logins = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/api/auth/login"))
        .respond_with(LoginSequence {
            calls: logins.clone(),
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/auth/userinfo"))
        .and(header("authorization", "Bearer expired"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/auth/userinfo"))
        .and(header("authorization", "Bearer fresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {
                "status": 1,
                "activate_time": null,
                "expire_at": null,
                "expire_seconds": 1,
                "remaining_api_calls": 10,
                "remaining_item_count": 20
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let envelope = manager(&server)
        .userinfo(&CancellationToken::new())
        .await
        .unwrap();
    assert!(envelope.protocol_compatible());
    assert_eq!(logins.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_second_401_stops_without_an_infinite_refresh_loop() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "bad"})))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/auth/userinfo"))
        .respond_with(ResponseTemplate::new(401))
        .expect(2)
        .mount(&server)
        .await;

    let manager = manager(&server);
    let error = manager
        .userinfo(&CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, FofaError::AuthenticationExpired));
    assert!(!manager.has_cached_token().await);
}
