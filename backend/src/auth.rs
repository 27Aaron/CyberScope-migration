use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

pub const SESSION_TTL: Duration = Duration::from_secs(8 * 60 * 60);
const MAX_ACTIVE_SESSIONS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub username: String,
}

#[derive(Clone)]
struct Session {
    user: AuthenticatedUser,
    expires_at: Instant,
}

pub struct AuthManager {
    username: String,
    password: SecretString,
    sessions: Mutex<HashMap<String, Session>>,
}

impl AuthManager {
    pub fn new(username: String, password: SecretString) -> Self {
        Self {
            username,
            password,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn login(&self, username: &str, password: &str) -> Option<(String, AuthenticatedUser)> {
        if username != self.username
            || !constant_time_eq(
                password.as_bytes(),
                self.password.expose_secret().as_bytes(),
            )
        {
            return None;
        }

        let user = AuthenticatedUser {
            username: self.username.clone(),
        };
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let now = Instant::now();
        let mut sessions = self.sessions.lock().expect("auth sessions mutex poisoned");
        sessions.retain(|_, session| session.expires_at > now);
        if sessions.len() >= MAX_ACTIVE_SESSIONS
            && let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, session)| session.expires_at)
                .map(|(token, _)| token.clone())
        {
            sessions.remove(&oldest);
        }
        sessions.insert(
            token.clone(),
            Session {
                user: user.clone(),
                expires_at: now + SESSION_TTL,
            },
        );
        Some((token, user))
    }

    pub fn authenticate(&self, token: &str) -> Option<AuthenticatedUser> {
        if !is_valid_token(token) {
            return None;
        }

        let now = Instant::now();
        let mut sessions = self.sessions.lock().expect("auth sessions mutex poisoned");
        let session = sessions.get(token)?.clone();
        if session.expires_at <= now {
            sessions.remove(token);
            return None;
        }
        Some(session.user)
    }

    pub fn logout(&self, token: &str) {
        self.sessions
            .lock()
            .expect("auth sessions mutex poisoned")
            .remove(token);
    }
}

fn is_valid_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or_default();
        let right_byte = right.get(index).copied().unwrap_or_default();
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

#[cfg(test)]
#[path = "../tests/unit/auth.rs"]
mod tests;
