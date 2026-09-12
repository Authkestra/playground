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
pub mod events;
pub mod github_api;
pub mod github_push;
pub mod github_routes;
pub mod github_token_store;
pub mod killswitch;
pub mod kit;
pub mod oauth_routes;
pub mod routes;
pub mod scenario;
pub mod session;
pub mod settings;
pub mod signing;
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

/// Log directives used when `RUST_LOG` is unset.
///
/// `authkestra=debug` rather than `=info` is the load-bearing part. The
/// framework emits about 34 `debug!` events to 6 `info!` ones, so at INFO it
/// says almost nothing and the service looks like it is not using it — which
/// is exactly how the deployment looked until `render.yaml` was corrected.
///
/// One `authkestra` entry covers `authkestra_engine`, `authkestra_axum`,
/// `authkestra_providers` and `authkestra_resource`, because `EnvFilter`
/// matches a directive against the event target by **prefix**. That is an
/// assumption worth not making silently, so there is a test for it.
pub const DEFAULT_LOG_DIRECTIVES: &str = "info,api=debug,authkestra=debug";

/// The filter this process logs under.
///
/// `RUST_LOG` wins when set, which is what a deployment wants — but note that
/// it wins *completely*: a deployment setting `RUST_LOG=info` silences the
/// framework no matter what [`DEFAULT_LOG_DIRECTIVES`] says.
pub fn log_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| DEFAULT_LOG_DIRECTIVES.into())
}

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
#[derive(Clone, Debug)]
pub struct ClientIpKeyExtractor {
    trusted_header: Option<axum::http::HeaderName>,
    xff_position: crate::settings::XffPosition,
}

impl Default for ClientIpKeyExtractor {
    fn default() -> Self {
        Self {
            trusted_header: None,
            xff_position: crate::settings::XffPosition::Rightmost,
        }
    }
}

impl ClientIpKeyExtractor {
    pub fn new(
        trusted_header: Option<axum::http::HeaderName>,
        xff_position: crate::settings::XffPosition,
    ) -> Self {
        Self {
            trusted_header,
            xff_position,
        }
    }

    /// The client IP this extractor would choose, and how it got there.
    ///
    /// Exposed so a deployment can be checked against reality rather than
    /// against assumptions about the proxy in front — see the admin
    /// `client-ip` endpoint.
    pub fn explain<T>(&self, req: &axum::http::Request<T>) -> ClientIpExplanation {
        let headers = req.headers();
        let read = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        };
        let xff = read("x-forwarded-for");

        ClientIpExplanation {
            trusted_header: self.trusted_header.as_ref().map(|h| h.as_str().to_string()),
            trusted_header_value: self.trusted_header.as_ref().and_then(|h| read(h.as_str())),
            x_forwarded_for: xff.clone(),
            xff_leftmost: xff
                .as_deref()
                .and_then(|v| v.split(',').find_map(parse_ip))
                .map(|ip| ip.to_string()),
            xff_rightmost: xff
                .as_deref()
                .and_then(|v| v.rsplit(',').find_map(parse_ip))
                .map(|ip| ip.to_string()),
            cf_connecting_ip: read("cf-connecting-ip"),
            true_client_ip: read("true-client-ip"),
            x_real_ip: read("x-real-ip"),
            peer: req
                .extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|c| c.0.ip().to_string()),
            xff_position: match self.xff_position {
                crate::settings::XffPosition::Leftmost => "leftmost",
                crate::settings::XffPosition::Rightmost => "rightmost",
            }
            .to_string(),
            selected: {
                use tower_governor::key_extractor::KeyExtractor as _;
                self.extract(req)
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|_| "<none>".to_string())
            },
        }
    }
}

/// What the rate limiter sees, for diagnosing a real deployment.
#[derive(Debug, serde::Serialize)]
pub struct ClientIpExplanation {
    pub trusted_header: Option<String>,
    pub trusted_header_value: Option<String>,
    pub x_forwarded_for: Option<String>,
    pub xff_leftmost: Option<String>,
    pub xff_rightmost: Option<String>,
    pub cf_connecting_ip: Option<String>,
    pub true_client_ip: Option<String>,
    pub x_real_ip: Option<String>,
    pub peer: Option<String>,
    pub xff_position: String,
    /// The key the limiter would actually bucket this request under.
    pub selected: String,
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

        // Which end to trust depends on whether the proxy in front appends or
        // overwrites; see `XffPosition`.
        if let Some(raw) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            let found = match self.xff_position {
                crate::settings::XffPosition::Rightmost => raw.rsplit(',').find_map(parse_ip),
                crate::settings::XffPosition::Leftmost => raw.split(',').find_map(parse_ip),
            };
            if let Some(ip) = found {
                return Ok(ip);
            }
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

/// Choose the rustls crypto provider for this process.
///
/// Two dependencies disagree about which provider to use, and both are out of
/// our hands: `redis`'s `tls-rustls` feature hardcodes `rustls/ring`, while
/// `authkestra-engine`'s `rustls-aws-lc-rs` pulls `rustls/aws-lc-rs` through
/// reqwest. Cargo features being additive, the one `rustls` in the graph gets
/// both — and rustls will not guess between them. Without this call the process
/// panics the first time it builds a TLS connection, which in practice means
/// the first `rediss://` connection at boot.
///
/// `aws-lc-rs` is chosen to match the framework's own default
/// (`docs/decisions/0001-dependency-and-tls-baseline.md`), so only one provider
/// is actually exercised at runtime.
///
/// Idempotent: installing twice is not an error worth surfacing, so a second
/// call is ignored. **Must run before any TLS client is constructed** — that
/// includes the engine's provider clients, so it belongs at the very top of
/// `main`.
pub fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .is_err()
        {
            // Already installed by something else; that is fine, and any
            // provider is better than the ambiguity that panics.
            tracing::debug!("a rustls crypto provider was already installed");
        } else {
            tracing::debug!("installed the aws-lc-rs rustls crypto provider");
        }
    });
}

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

    // Both provider-select controls must only offer providers this deployment
    // can actually complete, so credentials are read before the registry is
    // built. Neither is required: a scenario with no credentials reports itself
    // unavailable rather than failing at boot.
    let provider_credentials = ProviderCredentials::from_env();
    let captcha_keys = crate::scenario::captcha::CaptchaKeys::from_env();

    // Signing comes first: the resource scenario issues with this key and
    // validates against the key set it publishes, so it cannot be built
    // without it. The issuer is this API's own base URL, so the key set lands
    // at `<iss>/.well-known/jwks.json` — where a resource server pointed at
    // this issuer will look for it.
    let signing = Arc::new(crate::signing::SigningKeys::from_env(
        settings.public_base_url.clone(),
    ));

    let registry = ScenarioRegistry::from_config(crate::scenario::RegistryConfig {
        oauth_providers: provider_credentials.configured(),
        captcha: captcha_keys,
        signing: signing.clone(),
    });

    let kv = open_state_store().await?;

    // Initialize the kill switch with the store, so its state is durable.
    // The environment variables seed it only on the first boot; after that,
    // the stored state is authoritative and survives restarts.
    let kill_switch = Arc::new(KillSwitch::from_env(Some(kv.clone())));

    // Credentials and the flow log share the session's lifetime, so they expire
    // with it. That is the entire cleanup story — nothing runs on a timer.
    let session_ttl =
        std::time::Duration::from_secs((settings.session_ttl_hours.max(1) as u64) * 3600);
    let credentials = crate::credentials::KvCredentialStore::new(kv.clone(), session_ttl);

    let sessions = Arc::new(DemoSessionStore::new(
        kv.clone(),
        registry,
        settings.session_ttl_hours,
        credentials.clone(),
    ));
    let engines = Arc::new(EngineFactory::new(
        provider_credentials,
        settings.cookie_secure,
    ));

    // The push token only has to survive one visitor clicking Connect and then
    // Push, so it gets its own short TTL rather than the session's twelve
    // hours — see `github_token_store`'s module docs.
    let github_push = Arc::new(crate::github_push::GithubPushState::new(
        Arc::new(crate::github_api::HttpGitHubApi::new()),
        crate::github_token_store::GithubTokenStore::new(
            kv.clone(),
            crate::github_token_store::TOKEN_TTL,
        ),
    ));

    Ok(AppState {
        sessions,
        kill_switch,
        engines,
        settings,
        credentials: Arc::new(credentials),
        ceremonies: Arc::new(crate::ceremony::CeremonyStore::new(kv.clone())),
        events: Arc::new(crate::events::EventLog::new(kv, session_ttl)),
        signing,
        github_push,
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
    let key_extractor = ClientIpKeyExtractor::new(
        settings.trusted_client_ip_header.clone(),
        settings.xff_position,
    );

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
        // The download is fetched cross-site, and a cross-origin response's
        // headers are invisible to script unless they are exposed. Without
        // this the frontend cannot read the filename the server chose.
        .expose_headers([header::CONTENT_DISPOSITION])
        // The demo session rides in a cookie, so credentials must be allowed.
        .allow_credentials(true);

    let mut app = Router::new()
        .merge(
            routes::sensitive_router()
                // Every OAuth login reaches a third party, so it shares the
                // tighter bucket that protects provider quota.
                .merge(crate::oauth_routes::router())
                // The GitHub push feature reaches a third party on every one
                // of its three routes — the navigation pair and the push
                // itself — so all three share the same tighter bucket.
                .merge(crate::github_routes::navigation_router())
                .merge(crate::github_routes::action_router())
                .layer(GovernorLayer { config: sensitive }),
        )
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

#[cfg(test)]
mod key_extractor_tests {
    use super::*;
    use axum::http::{HeaderName, Request};
    use std::net::{IpAddr, SocketAddr};
    use tower_governor::key_extractor::KeyExtractor;

    fn fly() -> ClientIpKeyExtractor {
        ClientIpKeyExtractor::new(
            Some(HeaderName::from_static("fly-client-ip")),
            crate::settings::XffPosition::Rightmost,
        )
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
        let e = ClientIpKeyExtractor::new(None, crate::settings::XffPosition::Rightmost);
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
            ClientIpKeyExtractor::new(None, crate::settings::XffPosition::Rightmost)
                .extract(&req)
                .unwrap(),
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
        let e = ClientIpKeyExtractor::new(None, crate::settings::XffPosition::Rightmost);
        let req = Request::builder()
            .header("x-forwarded-for", "198.51.100.7:51234")
            .body(())
            .unwrap();
        assert_eq!(e.extract(&req).unwrap(), ip("198.51.100.7"));
    }

    /// Leftmost is correct only where the edge proxy overwrites the header.
    /// Where it appends, reading it is a bypass — which is why it is opt-in.
    #[test]
    fn leftmost_position_reads_the_first_entry() {
        let e = ClientIpKeyExtractor::new(None, crate::settings::XffPosition::Leftmost);
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.99, 198.51.100.7")
            .body(())
            .unwrap();
        assert_eq!(e.extract(&req).unwrap(), ip("203.0.113.99"));
    }

    #[test]
    fn the_default_position_is_the_unforgeable_one() {
        let e = ClientIpKeyExtractor::default();
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.99, 198.51.100.7")
            .body(())
            .unwrap();
        assert_eq!(
            e.extract(&req).unwrap(),
            ip("198.51.100.7"),
            "the default must be the entry a client cannot forge"
        );
    }

    /// The diagnostic has to report every candidate, or it cannot settle which
    /// header a real deployment should trust.
    #[test]
    fn explain_reports_each_candidate_and_the_selection() {
        let e = fly();
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.99, 198.51.100.7")
            .header("fly-client-ip", "192.0.2.1")
            .header("cf-connecting-ip", "192.0.2.9")
            .body(())
            .unwrap();

        let x = e.explain(&req);
        assert_eq!(x.xff_leftmost.as_deref(), Some("203.0.113.99"));
        assert_eq!(x.xff_rightmost.as_deref(), Some("198.51.100.7"));
        assert_eq!(x.cf_connecting_ip.as_deref(), Some("192.0.2.9"));
        assert_eq!(x.trusted_header_value.as_deref(), Some("192.0.2.1"));
        assert_eq!(
            x.selected, "192.0.2.1",
            "the trusted header must win over anything client-supplied"
        );
    }
}

#[cfg(test)]
mod log_filter_tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::EnvFilter;

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("capture poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    /// Emit one event per framework target under `directive`, and return what
    /// the subscriber actually printed.
    fn emitted_under(directive: &str) -> String {
        let capture = Capture(Arc::new(Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(directive))
            .with_writer(capture.clone())
            .finish();

        // Targets are set explicitly so this tests the *filter*, not whether
        // some dependency happens to log during the test.
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "authkestra_engine::token", "ENGINE_DEBUG");
            tracing::debug!(target: "authkestra_axum::helpers", "AXUM_DEBUG");
            tracing::debug!(target: "authkestra_resource::jwt", "RESOURCE_DEBUG");
            tracing::debug!(target: "api::routes", "API_DEBUG");
        });

        let bytes = capture.0.lock().expect("capture poisoned").clone();
        String::from_utf8(bytes).expect("log output is UTF-8")
    }

    /// The point of the default: the framework's own events reach the log.
    ///
    /// This is not hypothetical. The deployment ran with `authkestra=info` for
    /// its whole life, which suppressed every `debug!` the framework emits —
    /// and since it emits far more of those than `info!` ones, the service
    /// looked like it was not using the framework at all.
    #[test]
    fn the_shipped_log_filter_lets_the_frameworks_own_events_through() {
        let out = emitted_under(super::DEFAULT_LOG_DIRECTIVES);

        for expected in ["ENGINE_DEBUG", "AXUM_DEBUG", "RESOURCE_DEBUG", "API_DEBUG"] {
            assert!(
                out.contains(expected),
                "{expected} was filtered out by `{}`:\n{out}",
                super::DEFAULT_LOG_DIRECTIVES
            );
        }
    }

    /// One `authkestra` entry covering four crates relies on `EnvFilter`
    /// matching targets by prefix. If that ever stopped being true the
    /// directive above would silently cover nothing.
    #[test]
    fn a_directive_matches_framework_targets_by_prefix() {
        let out = emitted_under("warn,authkestra=debug");
        assert!(out.contains("ENGINE_DEBUG"), "{out}");
        assert!(out.contains("AXUM_DEBUG"), "{out}");
        assert!(
            !out.contains("API_DEBUG"),
            "`authkestra` should not match `api`: {out}"
        );
    }

    /// The failure mode this all exists to prevent, asserted directly so the
    /// difference between the two settings is visible in the suite.
    #[test]
    fn info_level_hides_almost_everything_the_framework_says() {
        let out = emitted_under("info,api=debug,authkestra=info");
        assert!(
            !out.contains("ENGINE_DEBUG"),
            "this directive is meant to suppress engine debug — if it no longer \
             does, the deployment note in render.yaml is wrong: {out}"
        );
    }
}
