//! Process settings read from the environment at boot.

use crate::session::DEFAULT_TTL_HOURS;

/// Credentials for the GitHub OAuth App used to push a generated project to a
/// visitor's own repository (#40) — deliberately **separate** from the
/// sign-in scenario's own GitHub credentials (`GITHUB_CLIENT_ID` /
/// `GITHUB_CLIENT_SECRET`, read in `engine::ProviderCredentials`).
///
/// The sign-in scenario exists to demonstrate identity, and the honest way to
/// demonstrate it is to ask for the narrowest scope that proves who someone
/// is. This feature needs `public_repo`, a scope with nothing to do with
/// identity. Folding the two together would mean the identity demo started
/// silently asking for repo-write access on every visitor's behalf — exactly
/// the kind of scope creep a playground teaching OAuth should model as
/// abnormal, not ship as its own default.
///
/// Like every other provider in this deployment, absent credentials are not a
/// startup error: the feature reports itself unavailable at the point a
/// visitor would use it, the same way `ProviderCredentials` degrades.
#[derive(Clone, Default)]
pub struct GithubKitCredentials {
    client_id: Option<String>,
    client_secret: Option<String>,
}

impl std::fmt::Debug for GithubKitCredentials {
    /// The secret is redacted by hand — a derived `Debug` would print it in
    /// any log line or panic message that ever formats `Settings`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubKitCredentials")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl GithubKitCredentials {
    pub fn from_env() -> Self {
        let client_id = std::env::var("GITHUB_KIT_CLIENT_ID")
            .ok()
            .filter(|v| !v.is_empty());
        let client_secret = std::env::var("GITHUB_KIT_CLIENT_SECRET")
            .ok()
            .filter(|v| !v.is_empty());

        if client_id.is_none() || client_secret.is_none() {
            tracing::warn!(
                "GITHUB_KIT_CLIENT_ID/GITHUB_KIT_CLIENT_SECRET not set; pushing a project to \
                 GitHub will report as not configured"
            );
        } else {
            tracing::info!("GitHub push credentials loaded");
        }

        Self {
            client_id,
            client_secret,
        }
    }

    /// Inject credentials without touching the environment, for tests.
    pub fn for_test(client_id: &str, client_secret: &str) -> Self {
        Self {
            client_id: Some(client_id.to_string()),
            client_secret: Some(client_secret.to_string()),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }

    /// The client id and secret, only when both are present — a deployment
    /// with just one of the two is exactly as unusable as one with neither,
    /// and treating it otherwise would send a visitor to GitHub with a
    /// request that can only fail there instead of here.
    pub fn credentials(&self) -> Option<(&str, &str)> {
        match (&self.client_id, &self.client_secret) {
            (Some(id), Some(secret)) => Some((id.as_str(), secret.as_str())),
            _ => None,
        }
    }
}

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
    /// Additional origins accepted for a ceremony.
    ///
    /// Useful for running a local frontend against a deployed API, or for
    /// several subdomains under one relying party. **It cannot bridge
    /// different registrable domains**: the browser requires the RP ID to be a
    /// suffix of the page's own origin, so adding `*.vercel.app` here while the
    /// RP ID is `authkestra.com` would still be refused client-side.
    pub extra_origins: Vec<String>,
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

        let extra_origins: Vec<String> = std::env::var("WEBAUTHN_EXTRA_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect();

        tracing::info!(
            rp_id = %id,
            rp_origin = %origin,
            extra_origins = ?extra_origins,
            "WebAuthn relying party"
        );
        Self {
            id,
            origin,
            name,
            extra_origins,
        }
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
    /// There is no portable default — the correct value is a property of the
    /// proxy sitting in front of this service, and that proxy must *overwrite*
    /// the header rather than appending to it, or the rate limiter can be
    /// bypassed by forging it.
    ///
    /// To find the right value, set `ADMIN_TOKEN` and call:
    ///   `GET /admin/client-ip`
    /// It returns every candidate header that actually arrived, what each
    /// strategy would select, and which one the rate limiter is using. Then
    /// set this variable to that header name. Examples: `cf-connecting-ip`
    /// behind Cloudflare, `x-forwarded-for` on some CDNs (risky; see below).
    ///
    /// Set to an empty string (the safe default) to disable and fall back to
    /// the rightmost `X-Forwarded-For` entry. When disabled, every visitor
    /// shares one coarse rate-limit bucket based on the proxy's address — a
    /// weaker limiter, but never a bypass. Until this is properly set, that is
    /// the only option that does not risk rate-limiting the service's own IP
    /// address or making the limiter bypassable.
    pub trusted_client_ip_header: Option<axum::http::HeaderName>,
    /// WebAuthn relying-party identity.
    pub relying_party: RelyingParty,
    /// Which end of `X-Forwarded-For` to trust when no trusted header is set.
    pub xff_position: XffPosition,
    /// `SameSite` for the demo-session cookie.
    pub cookie_same_site: CookieSameSite,
    /// This API's own externally reachable base URL, with no trailing slash.
    ///
    /// Load-bearing for the resource-server scenario: it is the `iss` of every
    /// token this deployment signs, and the prefix of the JWKS URL a validator
    /// fetches. Point it at something a validator cannot reach and tokens are
    /// issued fine and then fail to validate, which is a confusing way round.
    pub public_base_url: String,
    /// Credentials for the separate "push to GitHub" OAuth app (#40).
    pub github_kit: GithubKitCredentials,
}

/// Whether an origin points at the machine the process is running on.
///
/// Compared against the host only, so a port never changes the answer.
fn is_loopback_origin(origin: &str) -> bool {
    let host = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or("");
    // Strip the port without cutting an IPv6 literal in half. A bracketed
    // literal delimits itself; a bare one has several colons and no port,
    // which is why "exactly one colon" is the test rather than "ends in
    // digits" alone.
    let host = if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map(|(inner, _)| inner).unwrap_or(rest)
    } else {
        match host.rsplit_once(':') {
            Some((before, after))
                if !before.contains(':')
                    && !after.is_empty()
                    && after.chars().all(|c| c.is_ascii_digit()) =>
            {
                before
            }
            _ => host,
        }
    };
    matches!(
        host.trim().to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1"
    )
}

/// Whether the configuration around a base URL says this is plainly not a
/// developer's laptop.
///
/// Used to decide how loudly to complain about a localhost fallback. Two
/// signals, either of which is sufficient:
///
/// - `COOKIE_SECURE=true`, which pins the session cookie to HTTPS. A plain
///   `http://localhost` run cannot use such a cookie at all, so setting it
///   means someone was configuring a deployment.
/// - a non-loopback entry in `ALLOWED_ORIGINS`, which means the browser
///   talking to this API is served from a real hostname.
///
/// Deliberately not "is there a PORT set" or "is this Linux": both are true on
/// a laptop. The point is to catch a *contradiction*, not to guess at an
/// environment.
fn looks_like_a_deployment(cookie_secure: bool, allowed_origins: &[String]) -> bool {
    cookie_secure
        || allowed_origins
            .iter()
            .any(|o| !o.trim().is_empty() && !is_loopback_origin(o))
}

impl Settings {
    /// Whether this configuration plainly is not a developer's laptop.
    ///
    /// See [`looks_like_a_deployment`]. Exposed so the OAuth redirect base,
    /// which is read elsewhere and from a different variable, can be judged
    /// against the same signals rather than growing its own opinion.
    pub fn looks_like_a_deployment(&self) -> bool {
        looks_like_a_deployment(self.cookie_secure, &self.allowed_origins)
    }

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

        // Defaults to the local address so `cargo run` needs no configuration.
        // A deployment must set it: a JWKS published at a URL nothing can reach
        // is a resource server that rejects every token.
        let configured_base_url = std::env::var("PUBLIC_BASE_URL")
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .ok()
            .filter(|v| !v.is_empty());
        let public_base_url_defaulted = configured_base_url.is_none();
        let public_base_url =
            configured_base_url.unwrap_or_else(|| format!("http://localhost:{port}"));

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

        // `info!` is the wrong level for a value that silently makes a
        // deployment issue tokens under a hostname nothing can reach. Every
        // comparable misconfiguration in this file is an `error!`; this one
        // used to be the quietest line here.
        //
        // But shouting on every `cargo run` trains people to ignore it, so
        // the level is decided by whether the fallback *contradicts* the rest
        // of the configuration rather than by the fallback alone.
        if public_base_url_defaulted && looks_like_a_deployment(cookie_secure, &allowed_origins) {
            tracing::error!(
                base_url = %public_base_url,
                "PUBLIC_BASE_URL is unset, so it fell back to localhost — but the rest of \
                 this configuration is not a local run. Tokens will be issued with a \
                 localhost `iss` and a JWKS URL no validator can reach, and the GitHub push \
                 callback will not match what is registered with the OAuth App. Set \
                 PUBLIC_BASE_URL to this deployment's externally reachable origin."
            );
        } else {
            tracing::info!(base_url = %public_base_url, "public base URL (token issuer)");
        }

        let trusted_client_ip_header =
            std::env::var("TRUSTED_CLIENT_IP_HEADER").unwrap_or_default();
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
            public_base_url,
            github_kit: GithubKitCredentials::from_env(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_local_run_is_not_mistaken_for_a_deployment() {
        // The default `cargo run` shape: insecure cookie, localhost frontend.
        assert!(!looks_like_a_deployment(
            false,
            &["http://localhost:3000".to_string()]
        ));
        assert!(!looks_like_a_deployment(false, &[]));
        // Ports and loopback spellings must not change the answer, or the
        // warning fires on laptops and stops being read.
        for origin in [
            "http://127.0.0.1:3000",
            "http://localhost",
            "http://[::1]:8080",
            "http://0.0.0.0:3000",
            "HTTP://LocalHost:3000",
        ] {
            assert!(
                !looks_like_a_deployment(false, &[origin.to_string()]),
                "{origin} was read as a deployment"
            );
        }
    }

    #[test]
    fn either_signal_alone_is_enough() {
        // A Secure cookie cannot work over plain-HTTP localhost, so its
        // presence means someone was configuring a deployment.
        assert!(looks_like_a_deployment(true, &[]));
        assert!(looks_like_a_deployment(
            true,
            &["http://localhost:3000".to_string()]
        ));
        // A real frontend hostname, on its own.
        assert!(looks_like_a_deployment(
            false,
            &["https://play.authkestra.com".to_string()]
        ));
    }

    #[test]
    fn one_real_origin_among_local_ones_still_counts() {
        // The mixed list a staging deployment tends to accumulate.
        assert!(looks_like_a_deployment(
            false,
            &[
                "http://localhost:3000".to_string(),
                "https://playground-web.vercel.app".to_string(),
            ]
        ));
    }

    #[test]
    fn a_host_that_merely_contains_localhost_is_not_loopback() {
        // `notlocalhost.com` and friends must not get the quiet treatment.
        assert!(!is_loopback_origin("https://notlocalhost.com"));
        assert!(!is_loopback_origin("https://localhost.evil.example"));
        assert!(!is_loopback_origin("https://127.0.0.1.example.com"));
    }
}
