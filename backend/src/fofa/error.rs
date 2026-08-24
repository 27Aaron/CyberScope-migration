use std::time::Duration;

use thiserror::Error;

/// Errors from FOFA transport, validation, and relay quota operations.
///
/// Response bodies and request URLs are excluded to avoid retaining sensitive data.
#[derive(Debug, Error)]
pub enum FofaError {
    #[error("查询无效：{reason}")]
    InvalidQuery { reason: String },

    #[error("不支持字段 `{field}`：{reason}")]
    UnsupportedField { field: String, reason: String },

    #[error("FOFA API Key 被拒绝")]
    AuthenticationRejected,

    #[error("中转额度认证已失效，刷新后仍无法使用")]
    AuthenticationExpired,

    #[error("FOFA 查询额度已用尽")]
    QuotaExhausted,

    #[error("FOFA 请求受到限流")]
    RateLimited { retry_after: Option<Duration> },

    #[error("上游服务暂不可用")]
    UpstreamUnavailable {
        status: Option<u16>,
        possible_duplicate_charge: bool,
    },

    #[error("上游返回业务错误（{code}）")]
    UpstreamBusiness { code: String },

    #[error("上游响应不符合协议：{reason}")]
    UpstreamProtocol { reason: String },

    #[error("任务已取消")]
    Cancelled,
}

impl FofaError {
    pub(crate) fn invalid(reason: impl Into<String>) -> Self {
        Self::InvalidQuery {
            reason: reason.into(),
        }
    }

    pub(crate) fn protocol(reason: impl Into<String>) -> Self {
        Self::UpstreamProtocol {
            reason: reason.into(),
        }
    }

    /// Whether the failed request may have consumed upstream quota.
    pub fn possible_duplicate_charge(&self) -> bool {
        matches!(
            self,
            Self::UpstreamUnavailable {
                possible_duplicate_charge: true,
                ..
            }
        )
    }
}
