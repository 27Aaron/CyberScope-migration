use std::sync::Arc;

use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::{auth::SESSION_TTL, state::AppState};

const SESSION_COOKIE_NAME: &str = "cyberscope_session";

#[derive(Deserialize)]
pub(super) struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct UserResponse {
    username: String,
}

#[derive(Serialize)]
struct SessionResponse {
    user: UserResponse,
}

pub(super) async fn login(
    State(state): State<Arc<AppState>>,
    Json(input): Json<LoginRequest>,
) -> Response {
    if input.username.len() > 64 || input.password.len() > 1_024 {
        return unauthorized();
    }
    let Some((token, user)) = state.auth.login(&input.username, &input.password) else {
        return unauthorized();
    };

    let mut response = Json(SessionResponse {
        user: UserResponse {
            username: user.username,
        },
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, session_cookie(&token));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(token) = session_token(&headers) {
        state.auth.logout(token);
    }

    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, expired_session_cookie());
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn me(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Some(user) = session_token(&headers).and_then(|token| state.auth.authenticate(token))
    else {
        return unauthorized();
    };

    let mut response = Json(SessionResponse {
        user: UserResponse {
            username: user.username,
        },
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn require_auth(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let authenticated = session_token(request.headers())
        .and_then(|token| state.auth.authenticate(token))
        .is_some();
    if !authenticated {
        return unauthorized();
    }
    next.run(request).await
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    for value in headers.get_all(header::COOKIE) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for cookie in value.split(';') {
            let Some((name, value)) = cookie.trim().split_once('=') else {
                continue;
            };
            if name == SESSION_COOKIE_NAME {
                return Some(value);
            }
        }
    }
    None
}

fn session_cookie(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        SESSION_TTL.as_secs()
    ))
    .expect("session cookie contains only static ASCII and a hexadecimal token")
}

fn expired_session_cookie() -> HeaderValue {
    HeaderValue::from_static("cyberscope_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": {
                "code": "unauthorized",
                "message": "用户名或密码错误，或登录会话已失效"
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
#[path = "../../tests/unit/web/auth.rs"]
mod tests;
