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

/// Cookie carrying the identity mode the visitor picked at login.
///
/// The mode is chosen on the login route but only becomes visible on the
/// callback, and the two are separated by a round trip through the provider —
/// so it has to travel with the browser. It rides in its own cookie rather
/// than in the framework's encrypted `state`, which the callback helper
/// consumes before we could read anything out of it.
const MODE_COOKIE: &str = "ak_oauth_mode";

/// The wire spelling of a mode: what the login route stores and the callback
/// hands back to the frontend on the redirect query string.
fn mode_str(mode: OAuthMode) -> &'static str {
    match mode {
        OAuthMode::Jwt => "jwt",
        OAuthMode::Session => "session",
    }
}

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
    let switch = state.kill_switch.snapshot().await;
    if !switch.scenario_enabled("oauth") {
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

    // The mode is picked here but only observable on the callback, on the far
    // side of a round trip through the provider — so it travels with the
    // browser. Same attributes and lifetime as the framework's own state
    // cookie, so the two expire together and a stale mode can never outlive
    // the flow that chose it.
    let mut mode_cookie = Cookie::new(MODE_COOKIE, mode_str(mode));
    mode_cookie.set_http_only(true);
    mode_cookie.set_secure(state.settings.cookie_secure);
    mode_cookie.set_same_site(tower_cookies::cookie::SameSite::Lax);
    mode_cookie.set_path("/");
    mode_cookie.set_max_age(tower_cookies::cookie::time::Duration::seconds(900));
    cookies.add(mode_cookie);

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

    // Handed to the framework, which stores it in the encrypted state cookie as
    // the redirect to follow on success.
    //
    // The callback does not actually use it — it builds its own redirect so it
    // can report denied and failed outcomes in the same shape — but supplying
    // it keeps the state cookie self-describing, and it is what the flow would
    // fall back to if the callback ever stopped overriding it.
    let success_url = format!(
        "{}/?oauth=success&provider={}&mode={}",
        frontend_base(&state),
        urlencode(&provider),
        mode_str(mode)
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
    // The flow log belongs to the visitor's demo session.
    let session_id = cookies
        .get(crate::session::COOKIE_NAME)
        .and_then(|c| uuid::Uuid::parse_str(c.value()).ok())
        .unwrap_or_else(uuid::Uuid::nil);

    // Check the kill switch: if OAuth is disabled mid-flow, redirect back rather
    // than completing the round trip. Same gate as the login step, so flipping it
    // mid-flow does not silently complete an in-flight callback.
    let switch = state.kill_switch.snapshot().await;
    if !switch.scenario_enabled("oauth") {
        return result_redirect(
            &state,
            &format!(
                "oauth=error&provider={}&reason=demo_disabled",
                urlencode(&provider)
            ),
        );
    }

    // Which identity mode the login step recorded. Parsed rather than echoed:
    // the value ends up in a redirect the browser follows, and re-parsing it
    // means only the two strings this service emits can ever appear there,
    // whatever a cookie happens to contain. An absent cookie means `session`,
    // which is also the login route's default.
    let mode_cookie = cookies.get(MODE_COOKIE);
    let identity_mode = OAuthMode::parse(mode_cookie.as_ref().map(|c| c.value()));

    // Removed here rather than left to expire, so a later flow started without
    // an explicit `?mode=` cannot inherit this one's choice.
    let mut remove_mode = Cookie::from(MODE_COOKIE);
    remove_mode.set_path("/");
    cookies.remove(remove_mode);

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

            // Both modes run the same exchange; they differ only in what is
            // kept afterwards, which is the whole reason the toggle is offered.
            // Saying which one ran is what makes the choice mean something to
            // the visitor rather than being an inert control.
            let detail = match identity_mode {
                OAuthMode::Jwt => {
                    "The state cookie verified, the authorization code was exchanged \
                     for a token, and the provider returned an identity. That identity \
                     was signed into a JWT and handed back — nothing about this sign-in \
                     was written server-side, so no store has to be consulted to trust \
                     it later."
                }
                OAuthMode::Session => {
                    "The state cookie verified, the authorization code was exchanged \
                     for a token, and the provider returned an identity. A server-side \
                     session was created and its id put in a cookie. State and nonce \
                     lived in an encrypted cookie throughout, so nothing had to be \
                     written to a database to make the round trip itself work."
                }
            };

            state
                .events
                .record(
                    session_id,
                    Step::success("oauth", "provider round trip completed")
                        .detail(detail)
                        .fact("provider", provider.clone())
                        .fact("identity mode", mode_str(identity_mode))
                        .build(),
                )
                .await;
            result_redirect(
                &state,
                &format!(
                    "oauth=success&provider={}&mode={}",
                    urlencode(&provider),
                    mode_str(identity_mode)
                ),
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

/// Minimal percent-encoding for the values put into our own redirect query.
///
/// Percent-encodes every byte that is not in the unreserved set (A-Za-z0-9-._~).
/// Takes the UTF-8 byte representation, not the Unicode code point, so that
/// multi-byte characters like `é` (UTF-8: 0xC3 0xA9) encode correctly as `%C3%A9`.
fn urlencode(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|&b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_ascii_pass_through() {
        // Unreserved characters should pass through unchanged.
        assert_eq!(
            urlencode("hello-world_2025.test~name"),
            "hello-world_2025.test~name"
        );
        assert_eq!(urlencode("abc123-._~ABC123"), "abc123-._~ABC123");
    }

    #[test]
    fn urlencode_reserved_chars() {
        // Reserved characters like &, =, /, ?, # should be percent-encoded.
        assert_eq!(urlencode("a&b"), "a%26b");
        assert_eq!(urlencode("key=value"), "key%3Dvalue");
        assert_eq!(urlencode("a/b"), "a%2Fb");
        assert_eq!(urlencode("a?b"), "a%3Fb");
        assert_eq!(urlencode("a#b"), "a%23b");
    }

    #[test]
    fn urlencode_two_byte_char() {
        // The character é (U+00E9) encodes in UTF-8 as bytes 0xC3 0xA9.
        // It should produce %C3%A9, not %E9.
        assert_eq!(urlencode("café"), "caf%C3%A9");
        assert_eq!(urlencode("é"), "%C3%A9");
    }

    #[test]
    fn urlencode_four_byte_emoji() {
        // The emoji 😀 (U+1F600) encodes in UTF-8 as bytes 0xF0 0x9F 0x98 0x80.
        // It should produce %F0%9F%98%80.
        assert_eq!(urlencode("😀"), "%F0%9F%98%80");
        assert_eq!(urlencode("hello😀"), "hello%F0%9F%98%80");
    }
}
