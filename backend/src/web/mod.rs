use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use mime_guess::from_path;
use rust_embed::Embed;
use serde::Serialize;

use crate::state::AppState;

mod searches;

#[derive(Embed)]
#[folder = "../frontend/dist/"]
#[allow_missing = true]
struct Assets;

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/v1/fields", get(searches::list_fields))
        .route(
            "/api/v1/searches",
            axum::routing::post(searches::create_search),
        )
        .route("/api/v1/searches/{id}", get(searches::get_search))
        .route(
            "/api/v1/searches/{id}/cancel",
            axum::routing::post(searches::cancel_search),
        )
        .route("/api/v1/searches/{id}/results", get(searches::get_results))
        .route("/api/v1/searches/{id}/export", get(searches::export_search))
        .fallback(static_or_not_found)
        .with_state(state)
}

async fn health(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    axum::Json(HealthResponse {
        status: "ok",
        service: "cyberscope",
    })
}

async fn static_or_not_found(method: Method, uri: Uri) -> Response {
    if !matches!(method, Method::GET | Method::HEAD) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    if uri.path().starts_with("/api/") {
        return api_not_found().into_response();
    }

    let requested = uri.path().trim_start_matches('/');
    let asset_path = if requested.is_empty() {
        Some("index.html")
    } else if Assets::get(requested).is_some() {
        Some(requested)
    } else if is_spa_route(requested) {
        Some("index.html")
    } else {
        None
    };

    let Some(asset_path) = asset_path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(asset) = Assets::get(asset_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let content_type = from_path(asset_path)
        .first_or_octet_stream()
        .as_ref()
        .parse::<HeaderValue>()
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    let cache_control = if asset_path == "index.html" {
        HeaderValue::from_static("no-cache")
    } else {
        HeaderValue::from_static("public, max-age=31536000, immutable")
    };
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(asset.data.into_owned())
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache_control)
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn is_spa_route(path: &str) -> bool {
    !path.starts_with("assets/") && !path.rsplit('/').next().unwrap_or_default().contains('.')
}

fn api_not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "error": {
                "code": "not_found",
                "message": "API route not found"
            }
        })),
    )
}
