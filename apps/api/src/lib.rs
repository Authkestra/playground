//! authkestra playground API.
//!
//! The playground lets a visitor configure auth features, see a real config
//! diff, and try the resulting flows. The framework it demonstrates lives at
//! <https://github.com/marcjazz/authkestra>.
//!
//! Exposed as a library so integration tests can build the same router the
//! binary serves.

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
use tower_governor::key_extractor::SmartIpKeyExtractor;
use tower_governor::GovernorLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::engine::{EngineFactory, ProviderCredentials};

/// Client-IP key extractor that degrades instead of failing.
///
/// `SmartIpKeyExtractor` returns `UnableToExtractKey` — rendered as a 500 —
/// when a request carries no forwarding header *and* no `ConnectInfo`. Fly
/// (and any container platform) puts a proxy in front, and a request that
/// arrives without `X-Forwarded-For` — the platform's own health probe, say —
/// would take the whole endpoint down with it.
///
/// Falling back to a sentinel key means unidentifiable callers share one bucket:
/// they are still rate limited, just collectively. Failing toward *more*
/// limiting is the right direction for a public demo.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClientIpKeyExtractor;

/// The shared bucket for callers we cannot identify.
const UNIDENTIFIED_CLIENT: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);

impl tower_governor::key_extractor::KeyExtractor for ClientIpKeyExtractor {
    type Key = std::net::IpAddr;

    fn extract<T>(
        &self,
        req: &axum::http::Request<T>,
    ) -> Result<Self::Key, tower_governor::GovernorError> {
        Ok(SmartIpKeyExtractor
            .extract(req)
            .unwrap_or(UNIDENTIFIED_CLIENT))
    }
}
use crate::killswitch::KillSwitch;
use crate::routes::AppState;
use crate::scenario::ScenarioRegistry;
use crate::session::{DemoSessionStore, NoopJanitor};
use crate::settings::Settings;

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
pub fn state_from_env() -> AppState {
    let settings = Arc::new(Settings::from_env());
    let kill_switch = Arc::new(KillSwitch::from_env());
    let registry = ScenarioRegistry::with_builtins();

    let sessions = Arc::new(DemoSessionStore::new(
        registry,
        settings.session_ttl_hours,
        Arc::new(NoopJanitor),
    ));
    let engines = Arc::new(EngineFactory::new(
        ProviderCredentials::from_env(),
        settings.cookie_secure,
    ));

    AppState {
        sessions,
        kill_switch,
        engines,
        settings,
    }
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
    let standard = {
        let mut b = GovernorConfigBuilder::default();
        b.per_second(STANDARD_REPLENISH_SECS)
            .burst_size(STANDARD_BURST);
        let mut b = b.key_extractor(ClientIpKeyExtractor);
        b.error_handler(governor_error_response);
        Arc::new(b.finish().expect("valid standard governor config"))
    };
    // Deliberately much tighter: these endpoints create credentials and call
    // third parties, so they are the ones that burn provider quota.
    let sensitive = {
        let mut b = GovernorConfigBuilder::default();
        b.per_second(SENSITIVE_REPLENISH_SECS)
            .burst_size(SENSITIVE_BURST);
        let mut b = b.key_extractor(ClientIpKeyExtractor);
        b.error_handler(governor_error_response);
        Arc::new(b.finish().expect("valid sensitive governor config"))
    };

    let origins: Vec<HeaderValue> = settings
        .allowed_origins
        .iter()
        .filter_map(|o| o.parse().ok())
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
