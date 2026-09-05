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
use tower_cookies::{Cookie, Cookies};

use crate::error::ApiError;
use crate::events::Step;
use crate::routes::AppState;
use crate::scenario::oauth::OAuthMode;

/// Cookie the framework keeps the encrypted OAuth state in.
const STATE_COOKIE: &str = "ak_state";

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

    // ---------------------------------------------------------------------
    // Work around an upstream bug in authkestra 0.8.0.
    //
    // `OAuth2Flow::initiate_login` sets a nonce unconditionally
    // (engine/src/flow/oauth2.rs:128), and `finalize_login` then requires the
    // returned identity to carry a matching one. But a nonce is an OIDC
    // ID-token concept: the shipped providers take `_nonce` as an unused
    // parameter and build their identity with an empty attribute map, so it
    // can never come back. The result is that every plain-OAuth2 round trip
    // fails with "Nonce mismatch" — which is exactly what happened here for
    // Google, GitHub and Discord alike.
    //
    // Clearing it is the correct semantics rather than a fudge: with no ID
    // token there is nothing for a nonce to bind to. CSRF protection is the
    // `state` parameter and PKCE, both of which are untouched.
    //
    // Fixed upstream in marcjazz/authkestra#318, which gates the *enforcement*
    // check rather than the generation — merged 2026-09-04.
    //
    // That is not yet enough to remove this. We depend on `0.8.0` from
    // crates.io, published 2026-09-03, so the fix is on `main` and in no
    // release. Removing this on the strength of the merge alone would restore
    // the bug for every visitor.
    //
    // The trigger is a *published* version carrying the fix: bump the
    // workspace pin past 0.8.0, then delete this and `strip_unusable_nonce`.
    strip_unusable_nonce(&state, &cookies);

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
    // The flow log belongs to the visitor's demo session.
    let session_id = cookies
        .get(crate::session::COOKIE_NAME)
        .and_then(|c| uuid::Uuid::parse_str(c.value()).ok())
        .unwrap_or_else(uuid::Uuid::nil);

    // A declined consent screen is the single most common outcome after the
    // happy path, and it is not an error on our side — send the visitor back
    // with something the page can explain.
    if let Some(error) = raw.get("error") {
        tracing::info!(
            %error,
            description = ?raw.get("error_description"),
            "oauth denied at the provider"
        );
        state
            .events
            .record(
                session_id,
                Step::rejected("oauth", "declined at the provider")
                    .detail(
                        "The provider reported that consent was not granted. An ordinary \
                         outcome, not a failure of the integration.",
                    )
                    .fact("provider", provider.clone())
                    .fact("reason", error.clone())
                    .build(),
            )
            .await;
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

    // Checked here rather than inferred from the helper's error string: a
    // missing state cookie and a failed token exchange are unrelated problems,
    // and collapsing them into one reason made a real failure undiagnosable.
    if cookies.get(STATE_COOKIE).is_none() {
        tracing::warn!("oauth callback arrived without the state cookie");
        state
            .events
            .record(
                session_id,
                Step::failed("oauth", "state cookie missing")
                    .detail(
                        "The browser did not send back the short-lived cookie holding this \
                         flow's state, so the callback could not be verified. Usually that \
                         means more than 15 minutes passed, the cookie was blocked, or the \
                         flow was started in a different browser.",
                    )
                    .build(),
            )
            .await;
        return result_redirect(
            &state,
            &format!(
                "oauth=error&provider={}&reason=state_missing",
                urlencode(&provider)
            ),
        );
    }

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
            state
                .events
                .record(
                    session_id,
                    Step::success("oauth", "provider round trip completed")
                        .detail(
                            "The state cookie verified, the authorization code was exchanged \
                             for a token, and the provider returned an identity. State and \
                             nonce lived in an encrypted cookie throughout — nothing was \
                             written to a database to make this work.",
                        )
                        .fact("provider", provider.clone())
                        .build(),
                )
                .await;
            result_redirect(
                &state,
                &format!("oauth=success&provider={}", urlencode(&provider)),
            )
        }
        Err((status, message)) => {
            tracing::warn!(%status, %message, "oauth callback failed");

            // Classified from the helper's message because it exposes no typed
            // error. Brittle if upstream rewords these, hence the fallback —
            // but one opaque reason for three unrelated causes is worse.
            let reason = if message.contains("Invalid state cookie") {
                "state_invalid"
            } else if message.contains("Authentication failed") {
                "exchange_failed"
            } else {
                "callback_failed"
            };

            // The real detail goes to the visitor's own flow log, which is
            // scoped to their session. Without it, a failure here cannot be
            // diagnosed without server access.
            state
                .events
                .record(
                    session_id,
                    Step::failed("oauth", "callback failed")
                        .detail(format!(
                            "The provider redirected back, but completing the flow failed: {message}"
                        ))
                        .fact("provider", provider.clone())
                        .fact("stage", reason)
                        .build(),
                )
                .await;

            result_redirect(
                &state,
                &format!(
                    "oauth=error&provider={}&reason={}",
                    urlencode(&provider),
                    reason
                ),
            )
        }
    }
}

/// Remove the nonce from the freshly written OAuth state cookie.
///
/// See the call site for why. Failures here are logged and ignored: leaving the
/// nonce in place means the callback fails later with a clear message, which is
/// strictly better than refusing to start the flow.
fn strip_unusable_nonce(state: &AppState, cookies: &Cookies) {
    use authkestra_engine::state::OAuth2State;

    let key = state.engines.session_config().state_encryption_key;
    let Some(existing) = cookies.get(STATE_COOKIE) else {
        return;
    };

    let mut decoded = match OAuth2State::decrypt(existing.value(), &key) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "could not read back the state cookie to clear its nonce");
            return;
        }
    };
    if decoded.nonce.take().is_none() {
        return; // nothing to do
    }

    match decoded.encrypt(&key) {
        Ok(encoded) => {
            // Rebuilt with the same attributes the framework used, so the
            // cookie's lifetime and scope are unchanged.
            let mut cookie = Cookie::new(STATE_COOKIE, encoded);
            cookie.set_http_only(true);
            cookie.set_secure(state.settings.cookie_secure);
            cookie.set_same_site(tower_cookies::cookie::SameSite::Lax);
            cookie.set_path("/");
            cookie.set_max_age(tower_cookies::cookie::time::Duration::seconds(900));
            cookies.add(cookie);
            tracing::debug!("cleared the unusable OIDC nonce from the OAuth state");
        }
        Err(e) => tracing::warn!(error = %e, "could not re-encrypt the state cookie"),
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
