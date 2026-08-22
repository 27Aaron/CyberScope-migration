use std::collections::HashMap;

use secrecy::ExposeSecret;

use crate::config::{Config, ConfigError};

fn config(values: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let values: HashMap<_, _> = values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    Config::from_source(|name| values.get(name).cloned())
}

#[test]
fn parses_web_configuration() {
    let config = config(&[
        ("FOFA_API_KEY", "fofa-key"),
        ("WEB_BIND_ADDRESS", "0.0.0.0:8080"),
        ("DATABASE_PATH", "data"),
    ])
    .unwrap();

    assert_eq!(config.fofa_api_key.expose_secret(), "fofa-key");
    assert_eq!(config.web_bind_address.to_string(), "0.0.0.0:8080");
    assert_eq!(config.database_path, std::path::Path::new("data"));
    assert!(!config.relay_quota_enabled);
}

#[test]
fn missing_key_is_rejected() {
    assert_eq!(
        config(&[]).unwrap_err(),
        ConfigError::Missing("FOFA_API_KEY")
    );
}

#[test]
fn relay_quota_requires_custom_endpoint() {
    assert_eq!(
        config(&[
            ("FOFA_API_KEY", "key"),
            ("FOFA_RELAY_QUOTA_ENABLED", "true"),
        ])
        .unwrap_err(),
        ConfigError::RelayQuotaBaseUrl
    );

    let config = config(&[
        ("FOFA_API_KEY", "key"),
        ("FOFA_API_BASE_URL", "https://relay.example.invalid"),
        ("FOFA_RELAY_QUOTA_ENABLED", "true"),
    ])
    .unwrap();
    assert!(config.relay_quota_enabled);
}

#[test]
fn invalid_web_bind_address_is_rejected() {
    assert_eq!(
        config(&[("FOFA_API_KEY", "key"), ("WEB_BIND_ADDRESS", "bad")]).unwrap_err(),
        ConfigError::InvalidWebBindAddress
    );
}

#[test]
fn empty_database_path_is_rejected() {
    assert_eq!(
        config(&[("FOFA_API_KEY", "key"), ("DATABASE_PATH", " ")]).unwrap_err(),
        ConfigError::InvalidDatabasePath
    );
}
