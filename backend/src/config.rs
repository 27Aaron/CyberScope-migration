use std::{env, net::SocketAddr, path::PathBuf};

use secrecy::SecretString;
use thiserror::Error;
use url::Url;

use crate::fofa::ApiMode;

pub const MAX_CONCURRENT_JOBS: usize = 1;
pub const MAX_BATCHES_PER_JOB: usize = 1_000;
pub const DEFAULT_WEB_BIND_ADDRESS: &str = "127.0.0.1:3000";
pub const DEFAULT_DATABASE_PATH: &str = "data";
pub const DEFAULT_WEB_ADMIN_USERNAME: &str = "admin";
pub const MIN_WEB_ADMIN_PASSWORD_LENGTH: usize = 8;

#[derive(Clone, Debug)]
pub struct Config {
    pub fofa_api_key: SecretString,
    pub fofa_api_base_url: Url,
    pub relay_quota_enabled: bool,
    pub web_bind_address: SocketAddr,
    pub web_admin_username: String,
    pub web_admin_password: SecretString,
    pub database_path: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_source(|name| env::var(name).ok())
    }

    fn from_source(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let fofa_api_key = required(&get, "FOFA_API_KEY")?;
        let base_url = get("FOFA_API_BASE_URL").unwrap_or_else(|| "https://fofa.info".to_owned());
        let fofa_api_base_url = validate_base_url(&base_url)?;
        let relay_quota_enabled = parse_bool(
            "FOFA_RELAY_QUOTA_ENABLED",
            get("FOFA_RELAY_QUOTA_ENABLED")
                .as_deref()
                .unwrap_or("false"),
        )?;

        if relay_quota_enabled && ApiMode::from_base_url(&fofa_api_base_url) == ApiMode::Official {
            return Err(ConfigError::RelayQuotaBaseUrl);
        }

        let web_bind_address = get("WEB_BIND_ADDRESS")
            .as_deref()
            .unwrap_or(DEFAULT_WEB_BIND_ADDRESS)
            .parse()
            .map_err(|_| ConfigError::InvalidWebBindAddress)?;
        let web_admin_username = get("WEB_ADMIN_USERNAME")
            .unwrap_or_else(|| DEFAULT_WEB_ADMIN_USERNAME.to_owned())
            .trim()
            .to_owned();
        if web_admin_username.is_empty()
            || web_admin_username.len() > 64
            || web_admin_username.chars().any(char::is_control)
        {
            return Err(ConfigError::InvalidAdminUsername);
        }
        let web_admin_password = required(&get, "WEB_ADMIN_PASSWORD")?;
        if web_admin_password.chars().count() < MIN_WEB_ADMIN_PASSWORD_LENGTH {
            return Err(ConfigError::WeakAdminPassword(
                MIN_WEB_ADMIN_PASSWORD_LENGTH,
            ));
        }
        let database_path =
            get("DATABASE_PATH").unwrap_or_else(|| DEFAULT_DATABASE_PATH.to_owned());
        if database_path.trim().is_empty() {
            return Err(ConfigError::InvalidDatabasePath);
        }

        Ok(Self {
            fofa_api_key: SecretString::from(fofa_api_key),
            fofa_api_base_url,
            relay_quota_enabled,
            web_bind_address,
            web_admin_username,
            web_admin_password: SecretString::from(web_admin_password),
            database_path: PathBuf::from(database_path),
        })
    }
}

fn required(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<String, ConfigError> {
    get(name)
        .filter(|value| !value.trim().is_empty())
        .ok_or(ConfigError::Missing(name))
}

fn parse_bool(name: &'static str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::InvalidBoolean(name)),
    }
}

fn validate_base_url(value: &str) -> Result<Url, ConfigError> {
    let mut url = Url::parse(value).map_err(|_| ConfigError::InvalidBaseUrl)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(ConfigError::InvalidBaseUrl);
    }

    url.set_path("");
    Ok(url)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("缺少必填环境变量 {0}")]
    Missing(&'static str),
    #[error("环境变量 {0} 必须是 true 或 false")]
    InvalidBoolean(&'static str),
    #[error("FOFA_API_BASE_URL 必须是无凭据、查询、片段和额外路径的 HTTPS 地址")]
    InvalidBaseUrl,
    #[error("启用中转额度时，FOFA_API_BASE_URL 不能是官方 API 地址")]
    RelayQuotaBaseUrl,
    #[error("WEB_BIND_ADDRESS 不是有效的监听地址")]
    InvalidWebBindAddress,
    #[error("WEB_ADMIN_USERNAME 必须是 1 到 64 个非控制字符")]
    InvalidAdminUsername,
    #[error("WEB_ADMIN_PASSWORD 至少需要 {0} 个字符")]
    WeakAdminPassword(usize),
    #[error("DATABASE_PATH 不能为空")]
    InvalidDatabasePath,
}

#[cfg(test)]
#[path = "../tests/unit/config.rs"]
mod tests;
