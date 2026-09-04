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
    /// Header carrying the true client IP, set by the proxy in front of us.
    ///
    /// Must name a header the proxy *overwrites*, or the rate limiter can be
    /// bypassed by forging it. `Fly-Client-IP` on Fly; `CF-Connecting-IP` once
    /// Cloudflare sits in front. Set to an empty string to disable and fall
    /// back to the rightmost `X-Forwarded-For` entry.
    pub trusted_client_ip_header: Option<axum::http::HeaderName>,
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

        let trusted_client_ip_header = std::env::var("TRUSTED_CLIENT_IP_HEADER")
            .unwrap_or_else(|_| "fly-client-ip".to_string());
        let trusted_client_ip_header = if trusted_client_ip_header.trim().is_empty() {
            None
        } else {
            match axum::http::HeaderName::try_from(trusted_client_ip_header.trim().to_lowercase()) {
                Ok(h) => Some(h),
                Err(_) => {
                    tracing::error!(
                        "TRUSTED_CLIENT_IP_HEADER is not a valid header name; falling back to \
                         X-Forwarded-For, which is weaker. Fix the value."
                    );
                    None
                }
            }
        };

        Self {
            port,
            cookie_secure,
            session_ttl_hours,
            admin_token,
            allowed_origins,
            trusted_client_ip_header,
        }
    }
}
