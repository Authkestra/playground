//! HTTP surface. Every scenario is driven through the registry, so adding one
//! never means adding a handler.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_cookies::{Cookie, Cookies};
use ts_rs::TS;
use uuid::Uuid;

use crate::demo_config::DemoConfig;
use crate::diff::{self, ConfigDiff};
use crate::engine::EngineFactory;
use crate::error::ApiError;
use crate::killswitch::KillSwitch;
use crate::kit::StarterKit;
use crate::scenario::{ControlValue, ScenarioContext, ScenarioSpec};
use crate::session::{DemoSession, DemoSessionStore, DemoSessionView, COOKIE_NAME};
use crate::settings::Settings;

#[derive(Clone)]
pub struct AppState {
    pub sessions: Arc<DemoSessionStore>,
    pub kill_switch: Arc<KillSwitch>,
    pub engines: Arc<EngineFactory>,
    pub settings: Arc<Settings>,
    /// Where scenarios enrol credentials.
    pub credentials: Arc<crate::credentials::KvCredentialStore>,
    /// In-flight ceremony state (WebAuthn challenges).
    pub ceremonies: Arc<crate::ceremony::CeremonyStore>,
    /// The visitor-facing flow log.
    pub events: Arc<crate::events::EventLog>,
}

// ---------------------------------------------------------------- wire types

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub demo_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConfigureBody {
    pub value: ControlValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConfigureResponse {
    pub config: DemoConfig,
    pub diff: ConfigDiff,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminKillSwitchBody {
    /// Global switch. Omit to leave unchanged.
    pub demo_enabled: Option<bool>,
    /// Per-scenario override, `{ "<id>": true|false }`.
    #[serde(default)]
    pub scenarios: std::collections::BTreeMap<String, bool>,
}

// ------------------------------------------------------------------- helpers

/// Resolve the caller's session from the cookie, creating one when needed, and
/// write the cookie back if it changed.
async fn resolve_session(state: &AppState, cookies: &Cookies) -> Result<DemoSession, ApiError> {
    let existing = cookies
        .get(COOKIE_NAME)
        .and_then(|c| Uuid::parse_str(c.value()).ok());

    let session = state.sessions.get_or_create(existing).await?;

    if existing != Some(session.id) {
        let mut cookie = Cookie::new(COOKIE_NAME, session.id.to_string());
        cookie.set_http_only(true);
        cookie.set_path("/");
        cookie.set_secure(state.settings.cookie_secure);
        // Cross-site deployments need `None`, or the browser never sends this
        // back and every request looks like a new visitor. See CookieSameSite.
        cookie.set_same_site(match state.settings.cookie_same_site {
            crate::settings::CookieSameSite::Strict => tower_cookies::cookie::SameSite::Strict,
            crate::settings::CookieSameSite::Lax => tower_cookies::cookie::SameSite::Lax,
            crate::settings::CookieSameSite::None => tower_cookies::cookie::SameSite::None,
        });
        cookie.set_max_age(tower_cookies::cookie::time::Duration::hours(
            state.settings.session_ttl_hours,
        ));
        cookies.add(cookie);
    }

    Ok(session)
}

/// Attach the kill switch's current view of availability to each spec.
/// When the kill switch has disabled the scenario, the reason is that live flows
/// are switched off; otherwise fall through to the scenario's unavailable_reason.
fn specs_for(state: &AppState) -> Vec<ScenarioSpec> {
    state
        .sessions
        .registry()
        .iter()
        .map(|s| {
            let unavailable_reason = if state.kill_switch.scenario_enabled(s.id()) {
                s.unavailable_reason()
            } else {
                Some("Live flows are switched off.".to_string())
            };
            ScenarioSpec {
                id: s.id().to_string(),
                name: s.name().to_string(),
                summary: s.summary().to_string(),
                control: s.control(),
                depends_on: s.depends_on(),
                available: state.kill_switch.scenario_enabled(s.id()),
                actions: s.actions().iter().map(|a| a.to_string()).collect(),
                unavailable_reason,
            }
        })
        .collect()
}

// ------------------------------------------------------------------ handlers

#[tracing::instrument(skip_all)]
async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        demo_enabled: state.kill_switch.demo_enabled(),
    })
}

#[tracing::instrument(skip_all)]
async fn get_session(
    State(state): State<AppState>,
    cookies: Cookies,
) -> Result<Json<DemoSessionView>, ApiError> {
    let session = resolve_session(&state, &cookies).await?;
    Ok(Json(session.view()))
}

#[tracing::instrument(skip_all)]
async fn reset_session(
    State(state): State<AppState>,
    cookies: Cookies,
) -> Result<Json<DemoSessionView>, ApiError> {
    let session = resolve_session(&state, &cookies).await?;
    let fresh = state.sessions.reset(session.id).await?;
    // Ceremonies and the flow log belong to the state the visitor discarded.
    let _ = state.ceremonies.clear_session(session.id).await;
    let _ = state.events.clear(session.id).await;
    Ok(Json(fresh.view()))
}

/// The visitor's flow log: what the engine actually did, in order.
#[tracing::instrument(skip_all)]
async fn session_events(
    State(state): State<AppState>,
    cookies: Cookies,
) -> Result<Json<Vec<crate::events::FlowEvent>>, ApiError> {
    let session = resolve_session(&state, &cookies).await?;
    Ok(Json(state.events.read(session.id).await?))
}

#[tracing::instrument(skip_all)]
async fn list_scenarios(State(state): State<AppState>) -> Json<Vec<ScenarioSpec>> {
    Json(specs_for(&state))
}

#[tracing::instrument(skip_all, fields(scenario = %id))]
async fn configure_scenario(
    State(state): State<AppState>,
    cookies: Cookies,
    Path(id): Path<String>,
    Json(body): Json<ConfigureBody>,
) -> Result<Json<ConfigureResponse>, ApiError> {
    let registry = state.sessions.registry();
    let scenario = registry
        .get(&id)
        .ok_or_else(|| ApiError::UnknownScenario(id.clone()))?;

    // Configuring is allowed while flows are off — a visitor can still explore
    // the diff in explainer-only mode. Only `try` needs live flows.
    scenario
        .validate(&body.value)
        .map_err(ApiError::InvalidValue)?;

    let session = resolve_session(&state, &cookies).await?;
    let before = session.config.clone();
    let mut after = before.clone();
    after.set(&id, body.value);

    let updated = state
        .sessions
        .update_config(session.id, after.clone())
        .await?
        .ok_or(ApiError::SessionGone)?;

    let d = diff::diff(&before, &after, registry);
    tracing::info!(entries = d.entries.len(), "configuration changed");

    // Warm the engine for the new config so the first `try` isn't the one
    // paying for construction.
    let _ = state.engines.engine_for(&updated.config);

    Ok(Json(ConfigureResponse {
        config: updated.config,
        diff: d,
    }))
}

/// One step of a scenario's ceremony.
///
/// Generic on purpose: registration and verification are multi-round-trip, and
/// giving each scenario its own routes would put a per-scenario branch back
/// into the HTTP layer. Scenarios declare the steps they accept via
/// `ScenarioSpec::actions`, so the frontend discovers them from data.
#[tracing::instrument(skip_all, fields(scenario = %id, action = %action))]
async fn scenario_action(
    State(state): State<AppState>,
    cookies: Cookies,
    Path((id, action)): Path<(String, String)>,
    body: Option<Json<serde_json::Value>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let registry = state.sessions.registry();
    let scenario = registry
        .get(&id)
        .ok_or_else(|| ApiError::UnknownScenario(id.clone()))?;

    // Ceremonies create credentials and can reach third parties, so they are
    // gated by the kill switch exactly like `try`.
    if !state.kill_switch.scenario_enabled(&id) {
        return Err(ApiError::DemoDisabled);
    }

    let session = resolve_session(&state, &cookies).await?;
    let value = session
        .config
        .get(&id)
        .cloned()
        .unwrap_or_else(|| scenario.default_value());

    let ctx = ScenarioContext {
        session_id: session.id,
        value: &value,
        credentials: &state.credentials,
        relying_party: &state.settings.relying_party,
        ceremonies: &state.ceremonies,
        events: &state.events,
    };

    let payload = body.map(|Json(v)| v).unwrap_or(serde_json::Value::Null);
    Ok(Json(scenario.action(&action, payload, &ctx).await?))
}

#[tracing::instrument(skip_all)]
/// Check the admin bearer token.
fn authorize_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = state
        .settings
        .admin_token
        .as_deref()
        .ok_or(ApiError::Unauthorized)?;

    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();

    if presented.is_empty() || presented.as_bytes() != expected.as_bytes() {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}

async fn admin_kill_switch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AdminKillSwitchBody>,
) -> Result<impl IntoResponse, ApiError> {
    authorize_admin(&state, &headers)?;

    if let Some(enabled) = body.demo_enabled {
        state.kill_switch.set_demo_enabled(enabled);
    }
    for (id, enabled) in body.scenarios {
        state.kill_switch.set_scenario_enabled(&id, enabled);
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "demo_enabled": state.kill_switch.demo_enabled(),
            "disabled_scenarios": state.kill_switch.disabled_scenarios(),
        })),
    ))
}

/// What the rate limiter sees for this request.
///
/// Which header carries the real client IP is a property of the proxy in front,
/// and getting it wrong either lets callers bypass rate limiting or lumps every
/// visitor into one bucket. Vendor documentation and reality do not always
/// agree, so this reports what actually arrived — configure from the answer
/// rather than from an assumption.
///
/// Behind `ADMIN_TOKEN`: it echoes request headers, which is not something to
/// expose publicly.
#[tracing::instrument(skip_all)]
async fn admin_client_ip(
    State(state): State<AppState>,
    headers: HeaderMap,
    req_headers: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    authorize_admin(&state, &headers)?;

    let extractor = crate::ClientIpKeyExtractor::new(
        state.settings.trusted_client_ip_header.clone(),
        state.settings.xff_position,
    );
    Ok(Json(extractor.explain(&req_headers)))
}

/// Download the visitor's configuration as a runnable project.
///
/// Generated from the same session config the diff and the try buttons read,
/// so what arrives is what the page has been describing. On the tighter rate
/// limit: it is the most expensive thing this service does.
#[tracing::instrument(skip_all)]
async fn download_starter_kit(
    State(state): State<AppState>,
    cookies: Cookies,
) -> Result<impl IntoResponse, ApiError> {
    let session = resolve_session(&state, &cookies).await?;
    let kit = StarterKit::generate(&session.config, state.sessions.registry());

    let name = kit.archive_name();
    let bytes = kit
        .to_zip()
        .map_err(|e| ApiError::ArchiveFailed(e.to_string()))?;

    tracing::info!(archive = %name, bytes = bytes.len(), "starter kit downloaded");

    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            // The name is asserted to need no quoting, so this stays a plain
            // token — no encoding games, nothing for a browser to mis-parse.
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        bytes,
    ))
}

// -------------------------------------------------------------------- router

/// Routes that hit third parties or create credentials, and so carry the
/// tighter rate limit.
pub fn sensitive_router() -> Router<AppState> {
    Router::new()
        // Ceremony steps create credentials and call third parties.
        .route("/api/scenarios/{id}/action/{action}", post(scenario_action))
        // Generating and compressing a project is the most expensive request
        // this service serves, so it shares the tighter bucket.
        .route("/api/starter-kit", get(download_starter_kit))
}

/// Everything else.
pub fn standard_router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/api/session", get(get_session))
        .route("/api/session/reset", post(reset_session))
        .route("/api/session/events", get(session_events))
        .route("/api/scenarios", get(list_scenarios))
        .route("/api/scenarios/{id}/configure", post(configure_scenario))
}

/// Admin routes, mounted only when an admin token is configured.
pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/admin/kill-switch", post(admin_kill_switch))
        .route("/admin/client-ip", get(admin_client_ip))
}
