use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::json;
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path, query_param},
};

use super::*;

fn test_client(server: &MockServer, max_retries: u32) -> FofaClient {
    FofaClient::new(
        reqwest::Client::new(),
        Url::parse(&server.uri()).unwrap(),
        SecretString::from("test-secret"),
        RetryPolicy {
            max_retries,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        },
    )
    .unwrap()
}

#[test]
fn retry_after_is_honored_separately_from_backoff_ceiling() {
    let policy = RetryPolicy {
        max_retries: 3,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_secs(1),
    };
    assert_eq!(
        policy.delay(0, Some(Duration::from_secs(60))),
        Duration::from_secs(60)
    );
    assert_eq!(
        policy.delay(0, Some(Duration::from_secs(600))),
        Duration::from_secs(300)
    );
}

#[tokio::test]
async fn search_all_uses_encoded_query_parameters_and_normalizes_rows() {
    let server = MockServer::start().await;
    let raw_query = r#"domain="example.com" && port="443""#;
    Mock::given(method("GET"))
        .and(path("/api/v1/search/all"))
        .and(query_param("key", "test-secret"))
        .and(query_param("qbase64", STANDARD.encode(raw_query)))
        .and(query_param("fields", "ip,port"))
        .and(query_param("size", "10000"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": false,
            "size": 123,
            "results": [["1.1.1.1", 443]]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let search = SearchQuery::new(
        raw_query,
        vec![ReturnField::api("ip"), ReturnField::api("port")],
        10_000,
    );
    let response = test_client(&server, 0)
        .search_all(&search, Some(2), &CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(response.matched_size, Some(123));
    assert_eq!(response.rows, vec![vec![json!("1.1.1.1"), json!(443)]]);
    assert_eq!(response.retry.attempts, 1);
}

struct FailOnce {
    calls: Arc<AtomicUsize>,
}

impl Respond for FailOnce {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            ResponseTemplate::new(503)
        } else {
            ResponseTemplate::new(200).set_body_json(json!({
                "error": false,
                "results": ["1.1.1.1"]
            }))
        }
    }
}

#[tokio::test]
async fn retry_is_counted_and_marks_possible_duplicate_charge() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/api/v1/search/all"))
        .respond_with(FailOnce {
            calls: calls.clone(),
        })
        .mount(&server)
        .await;

    let search = SearchQuery::new("example", vec![ReturnField::api("ip")], 10_000);
    let response = test_client(&server, 1)
        .search_all(&search, None, &CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(response.retry.attempts, 2);
    assert_eq!(response.retry.retries, 1);
    assert!(response.retry.possible_duplicate_charge);
}

#[tokio::test]
async fn next_rejects_a_stalled_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/search/next"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": false,
            "results": ["1.1.1.1"],
            "next": "same"
        })))
        .mount(&server)
        .await;

    let search = SearchQuery::new("example", vec![ReturnField::api("ip")], 10_000);
    let error = test_client(&server, 0)
        .search_next(&search, Some("same"), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, FofaError::UpstreamProtocol { .. }));
}

#[tokio::test]
async fn pre_cancelled_request_never_reaches_the_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let search = SearchQuery::new("example", vec![ReturnField::api("ip")], 10_000);
    let error = test_client(&server, 0)
        .search_all(&search, None, &cancellation)
        .await
        .unwrap_err();
    assert!(matches!(error, FofaError::Cancelled));
}
