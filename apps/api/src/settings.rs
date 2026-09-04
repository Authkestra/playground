//! Process settings read from the environment at boot.

use crate::session::DEFAULT_TTL_HOURS;

/// WebAuthn relying-party identity.
///
/// The RP ID must be the site's registrable domain and the origin must match
/// exactly what the browser sends, or every ceremony fails with an opaque
/// browser-side error. Preview deployments therefore need their own origin.
#[derive(Debug, Clone)]
pub struct RelyingParty {
    pub id: String,
    pub origin: String,
    pub name: String,
}

impl RelyingParty {
    pub fn from_env() -> Self {
        // Defaults suit local development; production sets both explicitly.
        let origin = std::env::var("WEBAUTHN_ORIGIN")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());
        let id = std::env::var("WEBAUTHN_RP_ID").unwrap_or_else(|_| {
            // Derive the host from the origin so the two cannot drift apart by
            // accident, which is the usual cause of a silent ceremony failure.
            origin
                .split("://")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .and_then(|host| host.split(':').next())
                .unwrap_or("localhost")
                .to_string()
        });
        let name = std::env::var("WEBAUTHN_RP_NAME")
            .unwrap_or_else(|_| "Authkestra Playground".to_string());

        tracing::info!(rp_id = %id, rp_origin = %origin, "WebAuthn relying party");
        Self { id, origin, name }
    }
}

/// `SameSite` policy for the demo-session cookie.
///
/// This is load-bearing for a cross-site deployment. `Lax` cookies are **not
/// sent on cross-site fetches** — only on same-site requests and top-level
/// navigations — so if the frontend and API are on different registrable
/// domains (`*.vercel.app` and `*.onrender.com`, say), every API call arrives
/// without a session and the visitor's configuration silently never persists.
///
/// `None` is required in that case, and browsers only accept it alongside
/// `Secure`. Once both sides share a domain (`play.authkestra.com` and
/// `api.play.authkestra.com` are same-site), `Lax` becomes correct again and is
/// the stricter choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSameSite {
    Strict,
    Lax,
    None,
}

impl CookieSameSite {
    /// Read from `COOKIE_SAMESITE`, defaulting by deployment shape.
    ///
    /// A `Secure` cookie implies a real deployment, which today means the API
    /// is on a different site from the frontend — so the default there is
    /// `None`. Locally both sides are `localhost` (ports do not affect
    /// same-site), so `Lax` is both correct and stricter.
    pub fn from_env(secure: bool) -> Self {
        let chosen = match std::env::var("COOKIE_SAMESITE")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "strict" => Some(CookieSameSite::Strict),
            "lax" => Some(CookieSameSite::Lax),
            "none" => Some(CookieSameSite::None),
            _ => None,
        };

        let value = chosen.unwrap_or(if secure {
            CookieSameSite::None
        } else {
            CookieSameSite::Lax
        });

        if value == CookieSameSite::None && !secure {
            tracing::error!(
                "COOKIE_SAMESITE=none requires a Secure cookie; browsers reject the \
                 combination and the session will not persist. Set COOKIE_SECURE=true."
            );
        }
        tracing::info!(same_site = ?value, secure, "session cookie policy");
        value
    }
}

/// Which `X-Forwarded-For` entry holds the client IP.
///
/// This is a property of the proxy in front, and getting it wrong has real
/// consequences either way, so it is configuration rather than a guess:
///
/// * `Rightmost` (default) — safe by construction. A proxy *appends* the peer
///   it saw, so the last entry is the one it wrote and the client cannot forge
///   it. If several proxies are chained, this is the nearest one's view, which
///   may be an internal address — every caller then shares one rate-limit
///   bucket. Coarse, never a bypass.
/// * `Leftmost` — correct only where the edge proxy *overwrites* the header
///   rather than appending to it (Render documents doing this). Where it does
///   not, any caller can mint a fresh bucket per request by sending their own
///   header, which is a complete rate-limit bypass.
///
/// Prefer setting `TRUSTED_CLIENT_IP_HEADER` over relying on either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XffPosition {
    Rightmost,
    Leftmost,
}

impl XffPosition {
    pub fn from_env() -> Self {
        match std::env::var("CLIENT_IP_XFF_POSITION")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "leftmost" | "left" | "first" => {
                tracing::warn!(
                    "CLIENT_IP_XFF_POSITION=leftmost: correct only if the proxy in front \
                     OVERWRITES X-Forwarded-For. If it appends, callers can bypass rate \
                     limiting by sending the header themselves."
                );
                XffPosition::Leftmost
            }
            _ => XffPosition::Rightmost,
        }
    }
}

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
    /// WebAuthn relying-party identity.
    pub relying_party: RelyingParty,
    /// Which end of `X-Forwarded-For` to trust when no trusted header is set.
    pub xff_position: XffPosition,
    /// `SameSite` for the demo-session cookie.
    pub cookie_same_site: CookieSameSite,
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

        // A malformed entry here disables CORS silently: the browser simply
        // blocks every request and the frontend looks like the API is down. So
        // normalise the easy mistakes and be loud about the rest.
        let raw_origins = std::env::var("ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());
        let allowed_origins: Vec<String> = raw_origins
            .split(',')
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if allowed_origins.is_empty() {
            tracing::error!(
                raw = %raw_origins,
                "ALLOWED_ORIGINS resolved to an empty list; every cross-origin request \
                 will be blocked by the browser and the frontend will look like the API \
                 is down"
            );
        } else {
            tracing::info!(origins = ?allowed_origins, "CORS allow-list");
        }

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
            relying_party: RelyingParty::from_env(),
            xff_position: XffPosition::from_env(),
            cookie_same_site: CookieSameSite::from_env(cookie_secure),
        }
    }
}
