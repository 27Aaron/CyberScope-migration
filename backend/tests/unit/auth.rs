use secrecy::SecretString;

use super::*;

fn manager() -> AuthManager {
    AuthManager::new(
        "admin".to_owned(),
        SecretString::from("correct horse battery staple".to_owned()),
    )
}

#[test]
fn valid_credentials_create_an_authenticatable_session() {
    let auth = manager();
    let (token, user) = auth
        .login("admin", "correct horse battery staple")
        .expect("credentials should be accepted");

    assert_eq!(token.len(), 64);
    assert_eq!(user.username, "admin");
    assert_eq!(auth.authenticate(&token), Some(user));
}

#[test]
fn invalid_credentials_do_not_create_a_session() {
    let auth = manager();

    assert!(
        auth.login("other", "correct horse battery staple")
            .is_none()
    );
    assert!(auth.login("admin", "wrong password").is_none());
}

#[test]
fn logout_revokes_the_session() {
    let auth = manager();
    let (token, _) = auth
        .login("admin", "correct horse battery staple")
        .expect("credentials should be accepted");

    auth.logout(&token);

    assert_eq!(auth.authenticate(&token), None);
}

#[test]
fn malformed_tokens_are_rejected() {
    let auth = manager();

    assert_eq!(auth.authenticate("not-a-session"), None);
    assert!(!constant_time_eq(b"secret", b"different"));
    assert!(constant_time_eq(b"same", b"same"));
}
