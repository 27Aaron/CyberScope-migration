use futures_util::StreamExt;
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{error::FofaError, models::UserInfoEnvelope};

pub const MAX_AUTH_RESPONSE_BYTES: u64 = 1024 * 1024;

/// In-memory authentication for an optional FOFA-compatible relay quota API.
///
/// Neither the API key nor the temporary token is persisted or exposed through
/// `Debug`. Concurrent refreshes are coalesced by `refresh_lock`.
pub struct RelayQuotaAuthManager {
    http: reqwest::Client,
    base_url: Url,
    api_key: SecretString,
    token: RwLock<Option<SecretString>>,
    refresh_lock: Mutex<()>,
}

impl RelayQuotaAuthManager {
    pub fn new(
        http: reqwest::Client,
        base_url: Url,
        api_key: SecretString,
    ) -> Result<Self, FofaError> {
        if !matches!(base_url.scheme(), "http" | "https") || base_url.host_str().is_none() {
            return Err(FofaError::invalid("中转 API 基础地址必须是 HTTP(S) URL"));
        }
        Ok(Self {
            http,
            base_url,
            api_key,
            token: RwLock::new(None),
            refresh_lock: Mutex::new(()),
        })
    }

    /// Get current quota data. A 401 clears/refreshes the token and retries
    /// `/userinfo` exactly once.
    pub async fn userinfo(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<UserInfoEnvelope, FofaError> {
        let token = self.token_or_login(cancellation).await?;
        match self.fetch_userinfo(&token, cancellation).await? {
            UserInfoAttempt::Success(envelope) => Ok(envelope),
            UserInfoAttempt::Unauthorized => {
                let refreshed = self
                    .refresh_after_unauthorized(&token, cancellation)
                    .await?;
                match self.fetch_userinfo(&refreshed, cancellation).await? {
                    UserInfoAttempt::Success(envelope) => Ok(envelope),
                    UserInfoAttempt::Unauthorized => {
                        self.clear_if_current(&refreshed).await;
                        Err(FofaError::AuthenticationExpired)
                    }
                }
            }
        }
    }

    pub async fn clear_cached_token(&self) {
        *self.token.write().await = None;
    }

    pub async fn has_cached_token(&self) -> bool {
        self.token.read().await.is_some()
    }

    async fn token_or_login(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<SecretString, FofaError> {
        if let Some(token) = self.token.read().await.clone() {
            return Ok(token);
        }

        let _refresh_guard = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FofaError::Cancelled),
            guard = self.refresh_lock.lock() => guard,
        };
        if let Some(token) = self.token.read().await.clone() {
            return Ok(token);
        }
        let token = self.login(cancellation).await?;
        *self.token.write().await = Some(token.clone());
        Ok(token)
    }

    async fn refresh_after_unauthorized(
        &self,
        rejected: &SecretString,
        cancellation: &CancellationToken,
    ) -> Result<SecretString, FofaError> {
        let _refresh_guard = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FofaError::Cancelled),
            guard = self.refresh_lock.lock() => guard,
        };

        // Another caller may already have replaced the rejected token while this
        // task waited for the refresh lock.
        if let Some(current) = self.token.read().await.clone()
            && current.expose_secret() != rejected.expose_secret()
        {
            return Ok(current);
        }

        *self.token.write().await = None;
        let token = self.login(cancellation).await?;
        *self.token.write().await = Some(token.clone());
        Ok(token)
    }

    async fn clear_if_current(&self, rejected: &SecretString) {
        let mut token = self.token.write().await;
        if token
            .as_ref()
            .is_some_and(|current| current.expose_secret() == rejected.expose_secret())
        {
            *token = None;
        }
    }

    async fn login(&self, cancellation: &CancellationToken) -> Result<SecretString, FofaError> {
        let url = self.endpoint("/api/auth/login")?;
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FofaError::Cancelled),
            result = self.http.post(url).json(&LoginRequest {
                key_value: self.api_key.expose_secret(),
            }).send() => result.map_err(sanitize_transport_error)?,
        };
        let status = response.status();
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
                return Err(FofaError::AuthenticationRejected);
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(FofaError::RateLimited { retry_after: None });
            }
            StatusCode::UNPROCESSABLE_ENTITY => {
                return Err(FofaError::protocol("中转登录请求被拒绝（HTTP 422）"));
            }
            _ if status.is_server_error() => {
                return Err(FofaError::UpstreamUnavailable {
                    status: Some(status.as_u16()),
                    possible_duplicate_charge: false,
                });
            }
            _ if !status.is_success() => {
                return Err(FofaError::protocol(format!(
                    "中转登录返回 HTTP {}",
                    status.as_u16()
                )));
            }
            _ => {}
        }

        let body = read_auth_body(response, cancellation).await?;
        let response: LoginResponse = serde_json::from_slice(&body)
            .map_err(|_| FofaError::protocol("中转登录响应不是有效 JSON"))?;
        if response.token.trim().is_empty() {
            return Err(FofaError::protocol("中转登录响应缺少 Token"));
        }
        Ok(SecretString::from(response.token))
    }

    async fn fetch_userinfo(
        &self,
        token: &SecretString,
        cancellation: &CancellationToken,
    ) -> Result<UserInfoAttempt, FofaError> {
        let url = self.endpoint("/api/auth/userinfo")?;
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FofaError::Cancelled),
            result = self.http.get(url).bearer_auth(token.expose_secret()).send() => {
                result.map_err(sanitize_transport_error)?
            },
        };
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            return Ok(UserInfoAttempt::Unauthorized);
        }
        if status == StatusCode::FORBIDDEN {
            return Err(FofaError::AuthenticationRejected);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(FofaError::RateLimited { retry_after: None });
        }
        if status.is_server_error() {
            return Err(FofaError::UpstreamUnavailable {
                status: Some(status.as_u16()),
                possible_duplicate_charge: false,
            });
        }
        if !status.is_success() {
            return Err(FofaError::protocol(format!(
                "中转额度接口返回 HTTP {}",
                status.as_u16()
            )));
        }

        let body = read_auth_body(response, cancellation).await?;
        let envelope: UserInfoEnvelope = serde_json::from_slice(&body)
            .map_err(|_| FofaError::protocol("中转额度响应不是有效 JSON"))?;
        if envelope.success == Some(false) {
            return Err(FofaError::protocol("中转额度响应标记 success=false"));
        }
        Ok(UserInfoAttempt::Success(envelope))
    }

    fn endpoint(&self, path: &str) -> Result<Url, FofaError> {
        self.base_url
            .join(path)
            .map_err(|_| FofaError::invalid("无法构造中转 API 地址"))
    }
}

fn sanitize_transport_error(error: reqwest::Error) -> FofaError {
    if error.is_builder() {
        return FofaError::protocol("无法编码中转认证请求");
    }
    FofaError::UpstreamUnavailable {
        status: error.status().map(|status| status.as_u16()),
        possible_duplicate_charge: !error.is_connect(),
    }
}

async fn read_auth_body(
    response: reqwest::Response,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, FofaError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_AUTH_RESPONSE_BYTES)
    {
        return Err(FofaError::protocol("中转认证响应超过 1 MiB 安全上限"));
    }

    let mut body = Vec::with_capacity(response.content_length().unwrap_or_default() as usize);
    let mut stream = response.bytes_stream();
    loop {
        let next = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FofaError::Cancelled),
            next = stream.next() => next,
        };
        let Some(chunk) = next else {
            return Ok(body);
        };
        let chunk = chunk.map_err(sanitize_transport_error)?;
        let new_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| FofaError::protocol("中转认证响应大小溢出"))?;
        if new_len as u64 > MAX_AUTH_RESPONSE_BYTES {
            return Err(FofaError::protocol("中转认证响应超过 1 MiB 安全上限"));
        }
        body.extend_from_slice(&chunk);
    }
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    key_value: &'a str,
}

#[derive(Deserialize)]
struct LoginResponse {
    token: String,
}

enum UserInfoAttempt {
    Success(UserInfoEnvelope),
    Unauthorized,
}

#[cfg(test)]
#[path = "../../tests/unit/fofa/auth.rs"]
mod tests;
