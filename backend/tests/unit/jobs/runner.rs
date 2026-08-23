use secrecy::SecretString;
use serde_json::json;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

use super::*;
use crate::{
    batch::{BatchItem, BatchSourceKind},
    fofa::{ApiMode, RetryPolicy},
    jobs::{JobKind, JobManager},
};

fn test_client(server: &MockServer) -> FofaClient {
    FofaClient::new(
        reqwest::Client::new(),
        Url::parse(&server.uri()).unwrap(),
        SecretString::from("test-key"),
        RetryPolicy {
            max_retries: 0,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        },
    )
    .unwrap()
}

fn run_settings(max_results: u64) -> RunSettings {
    RunSettings {
        fields: vec![ReturnField::api("ip")],
        format: ExportFormat::Csv,
        max_results,
        full: false,
    }
}

fn batch_document() -> BatchDocument {
    BatchDocument {
        source_kind: BatchSourceKind::Query,
        items: vec![BatchItem {
            line_number: 1,
            source: r#"ip="192.0.2.1""#.to_owned(),
            query: r#"ip="192.0.2.1""#.to_owned(),
        }],
        effective_lines: 1,
        ignored_lines: 0,
        deduplicated_lines: 0,
    }
}

#[test]
fn only_systemic_fofa_errors_stop_a_batch() {
    assert_eq!(
        terminal_completion(&FofaError::QuotaExhausted),
        Some(BatchCompletion::QuotaStopped)
    );
    assert_eq!(
        terminal_completion(&FofaError::InvalidQuery {
            reason: "bad".to_owned()
        }),
        None
    );
    assert_eq!(
        terminal_completion(&FofaError::UpstreamProtocol {
            reason: "cursor".to_owned()
        }),
        None
    );
}

#[tokio::test]
async fn single_query_uses_the_configured_result_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/search/all"))
        .and(query_param("size", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": false,
            "size": 5000,
            "results": ["192.0.2.1"]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let temp = tempfile::tempdir().unwrap();
    let manager = JobManager::new(1);
    let job = manager.try_start(7, JobKind::Search).unwrap();

    let outcome = run_single(
        &test_client(&server),
        QueryValidator::new(ApiMode::Official),
        temp.path(),
        r#"country="CN""#.to_owned(),
        &run_settings(100),
        &job,
    )
    .await
    .unwrap();

    assert_eq!(outcome.exporter.row_count(), 1);
    assert_eq!(job.snapshot().written_rows, 1);
}

#[tokio::test]
async fn batch_stops_at_the_limit_even_when_the_upstream_returns_a_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/search/next"))
        .and(query_param("size", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": false,
            "results": [["192.0.2.1"], ["192.0.2.2"]],
            "next": "must-not-be-used"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let temp = tempfile::tempdir().unwrap();
    let manager = JobManager::new(1);
    let job = manager.try_start(7, JobKind::Batch).unwrap();

    let outcome = run_batch(
        &test_client(&server),
        QueryValidator::new(ApiMode::Official),
        temp.path(),
        &batch_document(),
        &run_settings(2),
        10,
        &job,
    )
    .await
    .unwrap();

    assert_eq!(outcome.completion, BatchCompletion::Completed);
    assert_eq!(outcome.exporter.row_count(), 2);
    assert_eq!(job.snapshot().upstream_attempts, 1);
}

#[tokio::test]
async fn batch_reduces_the_next_request_to_the_remaining_result_count() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/search/next"))
        .and(query_param("size", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": false,
            "results": [["192.0.2.1"], ["192.0.2.2"]],
            "next": "cursor-1"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/search/next"))
        .and(query_param("size", "1"))
        .and(query_param("next", "cursor-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": false,
            "results": [["192.0.2.3"]],
            "next": "must-not-be-used"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let temp = tempfile::tempdir().unwrap();
    let manager = JobManager::new(1);
    let job = manager.try_start(7, JobKind::Batch).unwrap();

    let outcome = run_batch(
        &test_client(&server),
        QueryValidator::new(ApiMode::Official),
        temp.path(),
        &batch_document(),
        &run_settings(3),
        10,
        &job,
    )
    .await
    .unwrap();

    assert_eq!(outcome.completion, BatchCompletion::Completed);
    assert_eq!(outcome.exporter.row_count(), 3);
    assert_eq!(job.snapshot().upstream_attempts, 2);
}

#[tokio::test]
async fn failed_logical_requests_count_toward_the_hard_batch_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/search/next"))
        .respond_with(ResponseTemplate::new(400))
        .expect(1)
        .mount(&server)
        .await;
    let client = test_client(&server);
    let document = BatchDocument {
        source_kind: BatchSourceKind::Query,
        items: vec![
            BatchItem {
                line_number: 1,
                source: r#"ip="192.0.2.1""#.to_owned(),
                query: r#"ip="192.0.2.1""#.to_owned(),
            },
            BatchItem {
                line_number: 2,
                source: r#"ip="192.0.2.2""#.to_owned(),
                query: r#"ip="192.0.2.2""#.to_owned(),
            },
        ],
        effective_lines: 2,
        ignored_lines: 0,
        deduplicated_lines: 0,
    };
    let settings = RunSettings {
        fields: vec![ReturnField::api("ip")],
        format: ExportFormat::Csv,
        max_results: 100,
        full: false,
    };
    let temp = tempfile::tempdir().unwrap();
    let manager = JobManager::new(1);
    let job = manager.try_start(7, JobKind::Batch).unwrap();

    let outcome = run_batch(
        &client,
        QueryValidator::new(ApiMode::Official),
        temp.path(),
        &document,
        &settings,
        1,
        &job,
    )
    .await
    .unwrap();

    assert_eq!(outcome.completion, BatchCompletion::SafetyLimit);
    assert_eq!(outcome.failures.len(), 1);
    assert_eq!(job.snapshot().upstream_attempts, 1);
}

#[tokio::test]
async fn request_gate_is_immediately_cancellable() {
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let mut last = Some(Instant::now());
    assert!(matches!(
        wait_for_request_slot(&mut last, &token).await,
        Err(FofaError::Cancelled)
    ));
}
