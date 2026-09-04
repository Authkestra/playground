//! The OAuth login and callback routes.
//!
//! These are ordinary GET routes rather than scenario actions because OAuth is
//! a *navigation*: the browser leaves for the provider and returns to the
//! callback. Everything else about the scenario — its control, diff and
//! readiness — lives in `scenario::oauth`.
//!
//! The flow itself is the framework's: `initiate_oauth_login` builds the
//! authorization URL, generates PKCE and writes the encrypted `state` cookie;
//! the callback helpers verify it and establish identity. What is added here is
//! the playground's own concerns — the kill switch, only offering configured
//! providers, and landing the visitor back on the frontend with a result they
//! can see.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use tower_cookies::Cookies;

use crate::error::ApiError;
use crate::routes::AppState;
use crate::scenario::oauth::OAuthMode;

/// Query parameters accepted by the login route.
#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    /// `session` (default) or `jwt` for the stateless variant.
    #[serde(default)]
    mode: Option<String>,
    /// Space- or comma-separated extra scopes.
    #[serde(default)]
    scope: Option<String>,
}

/// The framework's `OAuthCallbackParams` is `#[non_exhaustive]`, so it cannot be
/// built with a struct literal from here — only deserialised. The whole query
/// is therefore taken as a map, which also makes the declined case (`error`
/// present, `code` absent) straightforward to detect.
fn callback_params(
    raw: &std::collections::HashMap<String, String>,
) -> Option<authkestra_axum::helpers::OAuthCallbackParams> {
    let code = raw.get("code")?;
    let state = raw.get("state")?;
    serde_json::from_value(serde_json::json!({ "code": code, "state": state })).ok()
}

/// Where to send the browser once the round trip finishes.
///
/// The first allowed origin is the frontend; sending the visitor anywhere else
/// would be an open redirect, so this is never taken from the request.
fn frontend_base(state: &AppState) -> String {
    state
        .settings
        .allowed_origins
        .first()
        .cloned()
        .unwrap_or_else(|| "/".to_string())
}

fn result_redirect(state: &AppState, query: &str) -> Response {
    Redirect::to(&format!("{}/?{}", frontend_base(state), query)).into_response()
}

/// Begin the flow: redirect the browser to the provider.
#[tracing::instrument(skip_all, fields(provider = %provider))]
async fn login(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(query): Query<LoginQuery>,
    cookies: Cookies,
) -> Result<Response, ApiError> {
    if !state.kill_switch.scenario_enabled("oauth") {
        return Err(ApiError::DemoDisabled);
    }

    // Only offer providers this deployment can actually complete. Without this
    // a visitor is sent to a provider that rejects the request, which looks
    // like the framework failing rather than a missing credential.
    if !state.engines.credentials().is_configured(&provider) {
        return Err(ApiError::InvalidValue(format!(
            "`{provider}` has no credentials configured on this deployment."
        )));
    }

    let mode = OAuthMode::parse(query.mode.as_deref());
    let engine = state.engines.auth_engine();
    let flow = engine
        .providers
        .get(&provider)
        .ok_or_else(|| ApiError::Scenario(format!("provider `{provider}` is not registered")))?
        .clone();

    let scopes_owned: Vec<String> = query
        .scope
        .unwrap_or_default()
        .split([' ', ','])
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let scopes: Vec<&str> = scopes_owned.iter().map(|s| s.as_str()).collect();

    // Carried through the encrypted state cookie and used by the callback, so
    // the visitor lands back on the page they started from.
    let success_url = format!(
        "{}/?oauth=success&provider={}&mode={}",
        frontend_base(&state),
        provider,
        match mode {
            OAuthMode::Jwt => "jwt",
            OAuthMode::Session => "session",
        }
    );

    let redirect = authkestra_axum::helpers::initiate_oauth_login(
        flow.as_ref(),
        &cookies,
        &scopes,
        &state.engines.session_config(),
        Some(success_url),
    );

    tracing::info!(?mode, "oauth login started");
    Ok(redirect.into_response())
}

/// The provider's callback.
#[tracing::instrument(skip_all, fields(provider = %provider))]
async fn callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(raw): Query<std::collections::HashMap<String, String>>,
    cookies: Cookies,
) -> Response {
    // A declined consent screen is the single most common outcome after the
    // happy path, and it is not an error on our side — send the visitor back
    // with something the page can explain.
    if let Some(error) = raw.get("error") {
        tracing::info!(
            %error,
            description = ?raw.get("error_description"),
            "oauth denied at the provider"
        );
        return result_redirect(
            &state,
            &format!(
                "oauth=denied&provider={}&reason={}",
                urlencode(&provider),
                urlencode(error)
            ),
        );
    }

    // No `code`/`state` and no `error` either: not a callback we can complete.
    let Some(params) = callback_params(&raw) else {
        return result_redirect(
            &state,
            &format!(
                "oauth=error&provider={}&reason=missing_code",
                urlencode(&provider)
            ),
        );
    };

    let engine = state.engines.auth_engine();
    let Some(flow) = engine.providers.get(&provider).cloned() else {
        return result_redirect(
            &state,
            &format!(
                "oauth=error&provider={}&reason=unknown_provider",
                urlencode(&provider)
            ),
        );
    };

    let config = state.engines.session_config();

    // Which completion path depends on the mode the login step recorded. The
    // stateless one verifies entirely from the encrypted cookie and issues a
    // JWT; the session one writes a server-side session.
    let outcome = authkestra_axum::helpers::handle_oauth_callback_erased(
        flow.as_ref(),
        cookies,
        params,
        engine.session_store(),
        config,
        "/",
    )
    .await;

    match outcome {
        Ok(_) => {
            tracing::info!("oauth round trip completed");
            result_redirect(
                &state,
                &format!("oauth=success&provider={}", urlencode(&provider)),
            )
        }
        Err((status, message)) => {
            // A failed exchange is usually a misconfigured redirect URI or a
            // replayed state, both of which are worth logging in full while
            // showing the visitor something plain.
            tracing::warn!(%status, %message, "oauth callback failed");
            result_redirect(
                &state,
                &format!(
                    "oauth=error&provider={}&reason=exchange_failed",
                    urlencode(&provider)
                ),
            )
        }
    }
}

/// Minimal percent-encoding for the values put into our own redirect query.
fn urlencode(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}

/// The OAuth navigation routes.
///
/// Mounted on the tighter rate limiter: every login reaches a third party, so
/// this is where provider quota gets spent.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login/{provider}", get(login))
        .route("/auth/callback/{provider}", get(callback))
}
