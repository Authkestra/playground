//! authkestra playground API.
//!
//! The playground lets a visitor configure auth features, see a real config
//! diff, and try the resulting flows. The framework it demonstrates lives at
//! <https://github.com/marcjazz/authkestra>.
//!
//! Exposed as a library so integration tests can build the same router the
//! binary serves.

pub mod ceremony;
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
pub mod store;
pub mod testing;

use std::sync::Arc;

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

/// Open the state backend named by `REDIS_URL`.
///
/// Falls back to an in-process store when unset, so `cargo run` works with no
/// infrastructure. That fallback is only safe for a single instance — it is
/// logged loudly, because using it in a deployment that scales would hand
/// visitors inconsistent state with no other symptom.
pub async fn open_state_store() -> Result<Arc<dyn crate::store::KeyValue>, StateError> {
    match std::env::var("REDIS_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
    {
        Some(url) => {
            let client = redis::Client::open(url.trim())
                .map_err(|e| StateError::Redis(format!("invalid REDIS_URL: {e}")))?;

            // Connect eagerly: a broken Redis should fail at boot, not on a
            // visitor's first request.
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| StateError::Redis(format!("could not reach Redis: {e}")))?;
            redis::cmd("PING")
                .query_async::<String>(&mut conn)
                .await
                .map_err(|e| StateError::Redis(format!("Redis did not answer PING: {e}")))?;

            let scheme = if url.starts_with("rediss://") {
                "TLS"
            } else {
                "plaintext"
            };
            tracing::info!(transport = scheme, "state store: Redis");
            Ok(Arc::new(crate::store::RedisKv::new(
                client,
                std::env::var("REDIS_PREFIX").unwrap_or_else(|_| "ak_playground".to_string()),
            )))
        }
        None => {
            tracing::warn!(
                "REDIS_URL is not set; using an in-process state store. Sessions will not \
                 survive a restart and MUST NOT be relied on with more than one instance."
            );
            Ok(Arc::new(crate::store::MemoryKv::new()))
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("state store unavailable: {0}")]
    Redis(String),
}

/// Build application state from the environment.
pub async fn state_from_env() -> Result<AppState, StateError> {
    let settings = Arc::new(Settings::from_env());
    let kill_switch = Arc::new(KillSwitch::from_env());
    let registry = ScenarioRegistry::with_builtins();

    let kv = open_state_store().await?;

    // Credentials share the session's lifetime, so they expire with it. That is
    // the entire cleanup story — nothing runs on a timer.
    let credentials = crate::credentials::KvCredentialStore::new(
        kv.clone(),
        std::time::Duration::from_secs((settings.session_ttl_hours.max(1) as u64) * 3600),
    );

    let sessions = Arc::new(DemoSessionStore::new(
        kv.clone(),
        registry,
        settings.session_ttl_hours,
        credentials.clone(),
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
        credentials: Arc::new(credentials),
        ceremonies: Arc::new(crate::ceremony::CeremonyStore::new(kv)),
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
