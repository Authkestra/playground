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
use crate::scenario::{ControlValue, ScenarioContext, ScenarioSpec, TryResult};
use crate::session::{DemoSession, DemoSessionStore, DemoSessionView, COOKIE_NAME};
use crate::settings::Settings;

#[derive(Clone)]
pub struct AppState {
    pub sessions: Arc<DemoSessionStore>,
    pub kill_switch: Arc<KillSwitch>,
    pub engines: Arc<EngineFactory>,
    pub settings: Arc<Settings>,
    /// Pool backing the credential store scenarios enrol into.
    pub pool: sqlx::SqlitePool,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TryBody {}

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
fn resolve_session(state: &AppState, cookies: &Cookies) -> DemoSession {
    let existing = cookies
        .get(COOKIE_NAME)
        .and_then(|c| Uuid::parse_str(c.value()).ok());

    let session = state.sessions.get_or_create(existing);

    if existing != Some(session.id) {
        let mut cookie = Cookie::new(COOKIE_NAME, session.id.to_string());
        cookie.set_http_only(true);
        cookie.set_path("/");
        cookie.set_secure(state.settings.cookie_secure);
        cookie.set_same_site(tower_cookies::cookie::SameSite::Lax);
        cookie.set_max_age(tower_cookies::cookie::time::Duration::hours(
            state.settings.session_ttl_hours,
        ));
        cookies.add(cookie);
    }

    session
}

/// Attach the kill switch's current view of availability to each spec.
fn specs_for(state: &AppState) -> Vec<ScenarioSpec> {
    state
        .sessions
        .registry()
        .iter()
        .map(|s| ScenarioSpec {
            id: s.id().to_string(),
            name: s.name().to_string(),
            summary: s.summary().to_string(),
            control: s.control(),
            depends_on: s.depends_on(),
            available: state.kill_switch.scenario_enabled(s.id()),
            actions: s.actions().iter().map(|a| a.to_string()).collect(),
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
async fn get_session(State(state): State<AppState>, cookies: Cookies) -> Json<DemoSessionView> {
    let session = resolve_session(&state, &cookies);
    Json(session.view())
}

#[tracing::instrument(skip_all)]
async fn reset_session(State(state): State<AppState>, cookies: Cookies) -> Json<DemoSessionView> {
    let session = resolve_session(&state, &cookies);
    let fresh = state.sessions.reset(session.id);
    Json(fresh.view())
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

    let session = resolve_session(&state, &cookies);
    let before = session.config.clone();
    let mut after = before.clone();
    after.set(&id, body.value);

    let updated = state
        .sessions
        .update_config(session.id, after.clone())
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

#[tracing::instrument(skip_all, fields(scenario = %id))]
async fn scenario_diff(
    State(state): State<AppState>,
    cookies: Cookies,
    Path(id): Path<String>,
) -> Result<Json<ConfigDiff>, ApiError> {
    let registry = state.sessions.registry();
    let scenario = registry
        .get(&id)
        .ok_or_else(|| ApiError::UnknownScenario(id.clone()))?;

    let session = resolve_session(&state, &cookies);

    // Isolate this scenario's contribution: the current config against the same
    // config with this one scenario back at its default.
    let after = session.config.clone();
    let mut baseline = after.clone();
    baseline.set(&id, scenario.default_value());

    Ok(Json(diff::diff(&baseline, &after, registry)))
}

#[tracing::instrument(skip_all, fields(scenario = %id))]
async fn try_scenario(
    State(state): State<AppState>,
    cookies: Cookies,
    Path(id): Path<String>,
) -> Result<Json<TryResult>, ApiError> {
    let registry = state.sessions.registry();
    let scenario = registry
        .get(&id)
        .ok_or_else(|| ApiError::UnknownScenario(id.clone()))?;

    if !state.kill_switch.scenario_enabled(&id) {
        return Err(ApiError::DemoDisabled);
    }

    let session = resolve_session(&state, &cookies);
    let value = session
        .config
        .get(&id)
        .cloned()
        .unwrap_or_else(|| scenario.default_value());

    // The engine this visitor's config implies.
    let _engine = state.engines.engine_for(&session.config);

    let ctx = ScenarioContext {
        session_id: session.id,
        value: &value,
        pool: &state.pool,
        relying_party: &state.settings.relying_party,
    };
    Ok(Json(scenario.try_run(&ctx).await?))
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

    let session = resolve_session(&state, &cookies);
    let value = session
        .config
        .get(&id)
        .cloned()
        .unwrap_or_else(|| scenario.default_value());

    let ctx = ScenarioContext {
        session_id: session.id,
        value: &value,
        pool: &state.pool,
        relying_party: &state.settings.relying_party,
    };

    let payload = body.map(|Json(v)| v).unwrap_or(serde_json::Value::Null);
    Ok(Json(scenario.action(&action, payload, &ctx).await?))
}

#[tracing::instrument(skip_all)]
async fn admin_kill_switch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AdminKillSwitchBody>,
) -> Result<impl IntoResponse, ApiError> {
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

    // Length-independent comparison is overkill here, but the token guards the
    // one control that can take the demo down.
    if presented.is_empty() || presented.as_bytes() != expected.as_bytes() {
        return Err(ApiError::Unauthorized);
    }

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

// -------------------------------------------------------------------- router

/// Routes that hit third parties or create credentials, and so carry the
/// tighter rate limit.
pub fn sensitive_router() -> Router<AppState> {
    Router::new()
        .route("/api/scenarios/{id}/try", post(try_scenario))
        // Ceremony steps create credentials and call third parties, so they
        // belong on the tighter bucket alongside `try`.
        .route("/api/scenarios/{id}/action/{action}", post(scenario_action))
}

/// Everything else.
pub fn standard_router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/api/session", get(get_session))
        .route("/api/session/reset", post(reset_session))
        .route("/api/scenarios", get(list_scenarios))
        .route("/api/scenarios/{id}/configure", post(configure_scenario))
        .route("/api/scenarios/{id}/diff", get(scenario_diff))
}

/// Admin routes, mounted only when an admin token is configured.
pub fn admin_router() -> Router<AppState> {
    Router::new().route("/admin/kill-switch", post(admin_kill_switch))
}
