//! authkestra playground API.
//!
//! The playground lets a visitor configure auth features, see a real config
//! diff, and try the resulting flows. The framework it demonstrates lives at
//! <https://github.com/marcjazz/authkestra>.
//!
//! Exposed as a library so integration tests can build the same router the
//! binary serves.

pub mod credentials;
pub mod demo_config;
pub mod diff;
pub mod engine;
pub mod error;
pub mod killswitch;
pub mod routes;
pub mod scenario;
pub mod session;
pub mod settings;

use std::sync::Arc;
use std::time::Duration;

use axum::http::{header, HeaderValue, Method};
use axum::Router;
use tower_cookies::CookieManagerLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::engine::{EngineFactory, ProviderCredentials};
use crate::killswitch::KillSwitch;
use crate::routes::AppState;
use crate::scenario::ScenarioRegistry;
use crate::session::DemoSessionStore;
use crate::settings::Settings;

/// Client-IP key extractor for the rate limiter.
///
/// Getting this wrong is a rate-limit bypass, so the order matters:
///
/// 1. **A trusted header set by our own proxy.** Fly sets `Fly-Client-IP` and
///    overwrites any client-supplied value, so it cannot be forged. Behind
///    Cloudflare this becomes `CF-Connecting-IP` — hence `TRUSTED_CLIENT_IP_HEADER`.
/// 2. **The rightmost `X-Forwarded-For` entry.** A proxy *appends* the peer it
///    saw, so the rightmost entry is the one written by the hop nearest us and
///    the leftmost is whatever the client sent. Reading the leftmost — which is
///    what `SmartIpKeyExtractor` does — lets any caller mint a fresh rate-limit
///    bucket per request by inventing a header value. Verified against the
///    deployed service before this was changed.
/// 3. **The peer address**, when the app is reached directly.
/// 4. **A shared sentinel**, so an unidentifiable caller is limited collectively
///    rather than 500-ing the endpoint. Some platforms serve a bare `Router`
///    with no `ConnectInfo`, and a health probe without forwarding headers must
///    not take the endpoint down.
#[derive(Clone, Debug, Default)]
pub struct ClientIpKeyExtractor {
    trusted_header: Option<axum::http::HeaderName>,
}

impl ClientIpKeyExtractor {
    pub fn new(trusted_header: Option<axum::http::HeaderName>) -> Self {
        Self { trusted_header }
    }
}

/// The shared bucket for callers we cannot identify.
const UNIDENTIFIED_CLIENT: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);

fn parse_ip(raw: &str) -> Option<std::net::IpAddr> {
    let t = raw.trim();
    // Tolerate `ip:port`, which some proxies emit.
    t.parse()
        .ok()
        .or_else(|| t.rsplit_once(':').and_then(|(h, _)| h.trim().parse().ok()))
}

impl tower_governor::key_extractor::KeyExtractor for ClientIpKeyExtractor {
    type Key = std::net::IpAddr;

    fn extract<T>(
        &self,
        req: &axum::http::Request<T>,
    ) -> Result<Self::Key, tower_governor::GovernorError> {
        let headers = req.headers();

        if let Some(name) = &self.trusted_header {
            if let Some(ip) = headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_ip)
            {
                return Ok(ip);
            }
        }

        // Rightmost entry: written by the nearest proxy, not by the client.
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').find_map(parse_ip))
        {
            return Ok(ip);
        }

        if let Some(peer) = req
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        {
            return Ok(peer.0.ip());
        }

        Ok(UNIDENTIFIED_CLIENT)
    }
}

/// How often expired demo sessions are swept.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(300);

/// Rate limit for ordinary interactive endpoints.
const STANDARD_REPLENISH_SECS: u64 = 2;
const STANDARD_BURST: u32 = 30;

/// Rate limit for endpoints that create credentials or call third parties.
/// Deliberately tighter — these are the ones that burn provider quota.
///
/// Sized so a visitor clicking through every scenario is never throttled, while
/// a script is cut to ~12/min. P5's load test is where these get tuned against
/// real traffic rather than a guess.
const SENSITIVE_REPLENISH_SECS: u64 = 5;
const SENSITIVE_BURST: u32 = 10;

/// Build application state from the environment.
///
/// Async because the credential store is opened and migrated here: a scenario
/// that cannot persist a credential is broken, so failing at boot is better
/// than failing on a visitor's first ceremony.
pub async fn state_from_env() -> Result<AppState, sqlx::Error> {
    let settings = Arc::new(Settings::from_env());
    let kill_switch = Arc::new(KillSwitch::from_env());
    let registry = ScenarioRegistry::with_builtins();

    let pool = crate::credentials::open().await?;

    let sessions = Arc::new(DemoSessionStore::new(
        registry,
        settings.session_ttl_hours,
        crate::credentials::janitor(pool.clone()),
    ));
    let engines = Arc::new(EngineFactory::new(
        ProviderCredentials::from_env(),
        settings.cookie_secure,
    ));

    Ok(AppState {
        sessions,
        kill_switch,
        engines,
        settings,
        pool,
    })
}

/// The 429 body, matching the shape documented in `docs/api-contract.md` so the
/// UI can render a friendly message rather than parsing prose.
fn rate_limited_response(wait_secs: u64) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        axum::Json(serde_json::json!({
            "error": "rate_limited",
            "detail": format!("Too many requests. Try again in about {wait_secs}s."),
        })),
    )
        .into_response()
}

/// Render any governor rejection. Only the rate-limit case gets the documented
/// JSON shape; the rest fall back to the library's own rendering.
fn governor_error_response(error: tower_governor::GovernorError) -> axum::response::Response {
    match error {
        tower_governor::GovernorError::TooManyRequests { wait_time, .. } => {
            rate_limited_response(wait_time)
        }
        mut other => other.as_response(),
    }
}

/// Assemble the router, its rate limiters and middleware.
pub fn build_router(state: AppState) -> Router {
    let settings = state.settings.clone();

    // Two buckets, built inline: naming `GovernorConfig`'s middleware generic
    // would mean depending on `governor` directly and pinning it to whatever
    // major `tower_governor` happens to use.
    //
    // `SmartIpKeyExtractor` reads `X-Forwarded-For` / `X-Real-IP` before falling
    // back to the peer address, which is what we want behind Cloudflare.
    let key_extractor = ClientIpKeyExtractor::new(settings.trusted_client_ip_header.clone());

    let standard = {
        let mut b = GovernorConfigBuilder::default();
        b.per_second(STANDARD_REPLENISH_SECS)
            .burst_size(STANDARD_BURST);
        let mut b = b.key_extractor(key_extractor.clone());
        b.error_handler(governor_error_response);
        Arc::new(b.finish().expect("valid standard governor config"))
    };
    // Deliberately much tighter: these endpoints create credentials and call
    // third parties, so they are the ones that burn provider quota.
    let sensitive = {
        let mut b = GovernorConfigBuilder::default();
        b.per_second(SENSITIVE_REPLENISH_SECS)
            .burst_size(SENSITIVE_BURST);
        let mut b = b.key_extractor(key_extractor.clone());
        b.error_handler(governor_error_response);
        Arc::new(b.finish().expect("valid sensitive governor config"))
    };

    let origins: Vec<HeaderValue> = settings
        .allowed_origins
        .iter()
        .filter_map(|o| match o.parse::<HeaderValue>() {
            Ok(v) => Some(v),
            Err(_) => {
                // Dropping this silently would block the origin in a browser
                // with no server-side trace of why.
                tracing::error!(origin = %o, "ALLOWED_ORIGINS entry is not a valid header value; ignoring it");
                None
            }
        })
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        // The demo session rides in a cookie, so credentials must be allowed.
        .allow_credentials(true);

    let mut app = Router::new()
        .merge(routes::sensitive_router().layer(GovernorLayer { config: sensitive }))
        .merge(routes::standard_router().layer(GovernorLayer { config: standard }));

    // A missing admin token must mean "no admin surface", never "open switch".
    if settings.admin_token.is_some() {
        tracing::info!("admin kill-switch endpoint mounted");
        app = app.merge(routes::admin_router());
    } else {
        tracing::warn!("ADMIN_TOKEN unset; admin kill-switch endpoint not mounted");
    }

    app.layer(cors)
        .layer(CookieManagerLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Spawn the background sweep of expired demo sessions.
///
/// Reads already treat expired sessions as absent, so this is about reclaiming
/// memory and triggering credential cleanup rather than correctness. The
/// service runs as a long-lived process, so no external cron is needed.
pub fn spawn_session_sweeper(sessions: Arc<DemoSessionStore>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        // The first tick fires immediately; skip it so boot isn't a no-op sweep.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let swept = sessions.sweep();
            tracing::debug!(swept, live = sessions.len(), "sweep complete");
        }
    })
}

#[cfg(test)]
mod key_extractor_tests {
    use super::*;
    use axum::http::{HeaderName, Request};
    use std::net::{IpAddr, SocketAddr};
    use tower_governor::key_extractor::KeyExtractor;

    fn fly() -> ClientIpKeyExtractor {
        ClientIpKeyExtractor::new(Some(HeaderName::from_static("fly-client-ip")))
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// The bug this type exists to prevent: a proxy *appends* the peer it saw,
    /// so a client that sends its own `X-Forwarded-For` controls the leftmost
    /// entry. Keying on that would let anyone mint a fresh rate-limit bucket
    /// per request just by varying a header.
    #[test]
    fn a_spoofed_forwarded_for_cannot_change_the_key() {
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.99, 198.51.100.7")
            .header("fly-client-ip", "198.51.100.7")
            .body(())
            .unwrap();
        assert_eq!(fly().extract(&req).unwrap(), ip("198.51.100.7"));
    }

    #[test]
    fn two_requests_spoofing_different_values_share_one_bucket() {
        let a = Request::builder()
            .header("x-forwarded-for", "203.0.113.1, 198.51.100.7")
            .header("fly-client-ip", "198.51.100.7")
            .body(())
            .unwrap();
        let b = Request::builder()
            .header("x-forwarded-for", "192.0.2.55, 198.51.100.7")
            .header("fly-client-ip", "198.51.100.7")
            .body(())
            .unwrap();
        let e = fly();
        assert_eq!(
            e.extract(&a).unwrap(),
            e.extract(&b).unwrap(),
            "a caller varying X-Forwarded-For must not escape its bucket"
        );
    }

    /// Without a trusted header we fall back to XFF — but the *rightmost*
    /// entry, which the nearest proxy wrote, not the client-supplied leftmost.
    #[test]
    fn falls_back_to_the_rightmost_forwarded_for_entry() {
        let e = ClientIpKeyExtractor::new(None);
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.99, 198.51.100.7")
            .body(())
            .unwrap();
        assert_eq!(e.extract(&req).unwrap(), ip("198.51.100.7"));
    }

    #[test]
    fn uses_the_peer_address_when_reached_directly() {
        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(SocketAddr::from((
                [198, 51, 100, 9],
                1234,
            ))));
        assert_eq!(
            ClientIpKeyExtractor::new(None).extract(&req).unwrap(),
            ip("198.51.100.9")
        );
    }

    /// A health probe with no forwarding headers and no ConnectInfo must be
    /// limited collectively, never rejected — erroring here 500s the endpoint.
    #[test]
    fn an_unidentifiable_caller_gets_the_shared_bucket_not_an_error() {
        let req = Request::builder().body(()).unwrap();
        assert_eq!(fly().extract(&req).unwrap(), UNIDENTIFIED_CLIENT);
    }

    #[test]
    fn the_trusted_header_wins_over_forwarded_for() {
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.99")
            .header("fly-client-ip", "192.0.2.1")
            .body(())
            .unwrap();
        assert_eq!(fly().extract(&req).unwrap(), ip("192.0.2.1"));
    }

    #[test]
    fn tolerates_an_ip_port_pair() {
        let e = ClientIpKeyExtractor::new(None);
        let req = Request::builder()
            .header("x-forwarded-for", "198.51.100.7:51234")
            .body(())
            .unwrap();
        assert_eq!(e.extract(&req).unwrap(), ip("198.51.100.7"));
    }
}
