//! Process settings read from the environment at boot.

use crate::session::DEFAULT_TTL_HOURS;

#[derive(Debug, Clone)]
pub struct Settings {
    pub port: u16,
    /// Marks the session cookie `Secure`. False for plain-HTTP local dev.
    pub cookie_secure: bool,
    pub session_ttl_hours: i64,
    /// Shared secret guarding the admin (kill-switch) endpoints. When unset the
    /// admin routes are not mounted at all — a missing secret must never mean
    /// an open switch.
    pub admin_token: Option<String>,
    /// Origins allowed to call the API with credentials.
    pub allowed_origins: Vec<String>,
}

impl Settings {
    pub fn from_env() -> Self {
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8000);

        let cookie_secure = std::env::var("COOKIE_SECURE")
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);

        let session_ttl_hours = std::env::var("SESSION_TTL_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TTL_HOURS);

        let admin_token = std::env::var("ADMIN_TOKEN").ok().filter(|t| !t.is_empty());

        let allowed_origins = std::env::var("ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "http://localhost:3000".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Self {
            port,
            cookie_secure,
            session_ttl_hours,
            admin_token,
            allowed_origins,
        }
    }
}
