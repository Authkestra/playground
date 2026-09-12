//! Push a generated project to the visitor's own GitHub repository (#40).
//!
//! Structured like `oauth_routes.rs` on purpose: `connect` and `callback` are
//! an ordinary navigation — the browser leaves for GitHub and comes back —
//! while `push` is a normal JSON action once a token is on file. This is a
//! **separate** OAuth app from the sign-in scenario's, with its own
//! `GITHUB_KIT_CLIENT_ID` / `GITHUB_KIT_CLIENT_SECRET` and its own, narrower
//! scope. See `docs/decisions/0006-github-push.md` for why the two must not
//! share credentials.
//!
//! ## Never logging the token
//!
//! The access token is handled as a plain, short-lived string that travels
//! from `callback` into `GithubTokenStore` and from there straight into
//! `github_push::push_kit`. It is never placed in a struct that derives
//! `Debug`, never interpolated into a `tracing` field, and never appears on
//! the query string of any redirect this module builds. Every handler here is
//! `#[tracing::instrument(skip_all)]` for exactly that reason — nothing in an
//! argument list is trustworthy to auto-capture as a span field once a token
//! is anywhere nearby.

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_cookies::{Cookie, Cookies};
use ts_rs::TS;
use uuid::Uuid;

use crate::error::ApiError;
use crate::events::Step;
use crate::github_api::GitHubApiError;
use crate::routes::AppState;
use crate::scenario::{DeployTargets, KitOptions};

/// Cookie holding this flow's CSRF `state`, separate from the sign-in
/// scenario's `ak_state` — the two flows must never be able to complete each
/// other's callback by accident.
const STATE_COOKIE: &str = "ak_github_push_state";

/// Kill-switch id. Not a registry scenario — this feature has no diff or
/// control to render — but the switch is a plain string set, so gating a
/// third-party-calling feature by it costs nothing and an operator can
/// disable it independently of everything else with the same admin endpoint.
const SCENARIO_ID: &str = "github_push";

/// The narrowest scope that can create a repository and push to it. Never
/// widened to `repo` here — see the ADR for why private-repo support, if it
/// is ever wanted, is a deliberately separate decision.
const SCOPE: &str = "public_repo";

// ---------------------------------------------------------------- wire types

/// Deploy-target opt-ins for a push, mirroring `scenario::DeployTargets` but
/// with every field optional: an omitted field keeps the kit generator's own
/// default rather than forcing the caller to restate it.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GithubPushDeployTargets {
    #[serde(default)]
    pub docker: Option<bool>,
    #[serde(default)]
    pub render: Option<bool>,
    #[serde(default)]
    pub fly: Option<bool>,
    #[serde(default)]
    pub railway: Option<bool>,
}

fn merge_deploy_targets(requested: Option<GithubPushDeployTargets>) -> DeployTargets {
    let base = DeployTargets::default();
    let Some(r) = requested else { return base };
    DeployTargets {
        docker: r.docker.unwrap_or(base.docker),
        render: r.render.unwrap_or(base.render),
        fly: r.fly.unwrap_or(base.fly),
        railway: r.railway.unwrap_or(base.railway),
    }
}

/// Body of `POST /api/github/push`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitHubPushRequest {
    /// The repository to create under the connected account.
    pub repo_name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Same opt-ins the zip download offers — the push is the same kit, just
    /// delivered a different way.
    #[serde(default)]
    pub openapi: bool,
    #[serde(default)]
    pub ts_client: bool,
    #[serde(default)]
    pub deploy: Option<GithubPushDeployTargets>,
}

/// Response of `POST /api/github/push`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitHubPushResponse {
    pub owner: String,
    pub repo: String,
    pub html_url: String,
    pub default_branch: String,
    pub commit_sha: String,
}

// -------------------------------------------------------------------- utils

/// Percent-encode a value for use in a URL we build ourselves. A local copy of
/// `oauth_routes`'s helper rather than a shared one: the two modules have no
/// other reason to depend on each other, and this is five lines.
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

/// Compare two strings without letting an attacker learn *how much* of a
/// guess was right from how quickly the comparison returns.
///
/// A `==` on two `&str` stops at the first differing byte, and while that
/// difference is nanoseconds here, this exists specifically because "it's a
/// state token, not a password" is exactly the reasoning that lets a timing
/// side-channel slip through review. Length is compared up front — that alone
/// leaks nothing about *content*, only about the length of a value the caller
/// already controls the format of (both sides are always a UUID). What must
/// not happen is stopping partway through the byte comparison once the
/// lengths match, so the loop below always walks every byte and only inspects
/// the accumulated result at the end.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn frontend_base(state: &AppState) -> String {
    state
        .settings
        .allowed_origins
        .first()
        .cloned()
        .unwrap_or_else(|| "/".to_string())
}

/// Send the browser back to the frontend with a result on the query string —
/// never a page rendered here, so the frontend is the only thing that decides
/// how a visitor sees the outcome. The first allowed origin is used, and
/// never anything taken from the request, so this can never become an open
/// redirect.
fn result_redirect(state: &AppState, query: &str) -> Response {
    Redirect::to(&format!("{}/?{}", frontend_base(state), query)).into_response()
}

/// This deployment's own callback URL, which must match what is registered
/// with the GitHub OAuth App byte for byte.
fn redirect_uri(state: &AppState) -> String {
    format!("{}/api/github/callback", state.settings.public_base_url)
}

fn session_id_from_cookies(cookies: &Cookies) -> Uuid {
    cookies
        .get(crate::session::COOKIE_NAME)
        .and_then(|c| Uuid::parse_str(c.value()).ok())
        .unwrap_or_else(Uuid::nil)
}

/// GitHub's own repository-naming rule, checked before spending a call on it:
/// letters, digits, hyphens, underscores and periods, and not empty. GitHub
/// itself is the final authority — this only catches the common, cheap cases
/// before a round trip, so its own `422` handling in `github_api` stays the
/// source of truth for anything subtler.
fn is_plausible_repo_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The query-string reason a connect failure redirects with. Classified from
/// the error rather than its `Display` text, so the frontend gets a stable
/// vocabulary rather than prose that could be reworded later.
fn connect_error_reason(e: &GitHubApiError) -> &'static str {
    match e {
        GitHubApiError::TokenRejected => "token_rejected",
        GitHubApiError::RateLimited => "rate_limited",
        GitHubApiError::Network(_) => "network_error",
        GitHubApiError::RepoNameTaken
        | GitHubApiError::InvalidRepoName(_)
        | GitHubApiError::ScopeMissing
        | GitHubApiError::Other(_) => "exchange_failed",
    }
}

// ------------------------------------------------------------------ handlers

/// Begin the flow: redirect the browser to GitHub's own consent screen.
#[tracing::instrument(skip_all)]
async fn connect(State(state): State<AppState>, cookies: Cookies) -> Result<Response, ApiError> {
    let switch = state.kill_switch.snapshot().await;
    if !switch.scenario_enabled(SCENARIO_ID) {
        return Err(ApiError::DemoDisabled);
    }

    let Some((client_id, _)) = state.settings.github_kit.credentials() else {
        return Err(ApiError::GithubPushNotConfigured);
    };

    // A fresh, unguessable value per attempt. 122 bits from `Uuid::new_v4`,
    // the same primitive this codebase already trusts for session and
    // ceremony ids, is ample for a token that lives at most fifteen minutes.
    let csrf_state = Uuid::new_v4().to_string();

    let mut cookie = Cookie::new(STATE_COOKIE, csrf_state.clone());
    cookie.set_http_only(true);
    cookie.set_secure(state.settings.cookie_secure);
    cookie.set_same_site(tower_cookies::cookie::SameSite::Lax);
    cookie.set_path("/");
    // Fifteen minutes: long enough to sit on GitHub's consent screen, short
    // enough that an abandoned flow's state cannot be replayed much later.
    cookie.set_max_age(tower_cookies::cookie::time::Duration::seconds(900));
    cookies.add(cookie);

    let url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&scope={}&state={}&redirect_uri={}",
        urlencode(client_id),
        urlencode(SCOPE),
        urlencode(&csrf_state),
        urlencode(&redirect_uri(&state)),
    );

    tracing::info!("github push: connect started");
    Ok(Redirect::to(&url).into_response())
}

/// GitHub's callback: verify `state`, exchange `code`, store the token.
#[tracing::instrument(skip_all)]
async fn callback(
    State(state): State<AppState>,
    Query(raw): Query<std::collections::HashMap<String, String>>,
    cookies: Cookies,
) -> Response {
    let session_id = session_id_from_cookies(&cookies);

    let switch = state.kill_switch.snapshot().await;
    if !switch.scenario_enabled(SCENARIO_ID) {
        return result_redirect(&state, "github_push=error&reason=demo_disabled");
    }

    let Some((client_id, client_secret)) = state.settings.github_kit.credentials() else {
        return result_redirect(&state, "github_push=error&reason=not_configured");
    };

    // A declined consent screen is an ordinary outcome, not a failure on our
    // side — same treatment as the sign-in OAuth callback.
    if let Some(error) = raw.get("error") {
        tracing::info!(%error, "github push: declined at GitHub");
        state
            .events
            .record(
                session_id,
                Step::rejected("github_push", "declined at GitHub")
                    .detail("GitHub reported that consent was not granted.")
                    .fact("reason", error.clone())
                    .build(),
            )
            .await;
        return result_redirect(
            &state,
            &format!("github_push=denied&reason={}", urlencode(error)),
        );
    }

    let (Some(code), Some(provided_state)) = (raw.get("code"), raw.get("state")) else {
        return result_redirect(&state, "github_push=error&reason=missing_code");
    };

    // Read and clear the cookie together: whatever happens next, a stale
    // state token must not be usable a second time.
    let cookie_state = cookies.get(STATE_COOKIE).map(|c| c.value().to_string());
    let mut remove = Cookie::from(STATE_COOKIE);
    remove.set_path("/");
    cookies.remove(remove);

    let Some(cookie_state) = cookie_state else {
        tracing::warn!("github push callback arrived without the state cookie");
        return result_redirect(&state, "github_push=error&reason=state_missing");
    };

    if !constant_time_eq(&cookie_state, provided_state) {
        tracing::warn!("github push callback's state did not match the cookie");
        state
            .events
            .record(
                session_id,
                Step::failed("github_push", "state mismatch")
                    .detail(
                        "The state returned by GitHub did not match the one this flow started \
                         with, so the callback was refused rather than trusted.",
                    )
                    .build(),
            )
            .await;
        return result_redirect(&state, "github_push=error&reason=state_invalid");
    }

    match state
        .github_push
        .api
        .exchange_code(client_id, client_secret, code, &redirect_uri(&state))
        .await
    {
        Ok(token) => {
            if let Err(e) = state.github_push.tokens.store(session_id, &token).await {
                tracing::error!(error = %e, "github push: could not store the exchanged token");
                return result_redirect(&state, "github_push=error&reason=store_failed");
            }
            state
                .events
                .record(
                    session_id,
                    Step::success("github_push", "connected")
                        .detail(
                            "GitHub state verified and the authorization code was exchanged for \
                             a token, scoped to public_repo only. It is held server-side for up \
                             to fifteen minutes — just long enough to push once.",
                        )
                        .build(),
                )
                .await;
            tracing::info!("github push: connected");
            result_redirect(&state, "github_push=connected")
        }
        Err(e) => {
            let reason = connect_error_reason(&e);
            tracing::warn!(%reason, "github push: token exchange failed");
            state
                .events
                .record(
                    session_id,
                    Step::failed("github_push", "connect failed")
                        .detail(format!("Exchanging the authorization code failed: {e}"))
                        .fact("reason", reason)
                        .build(),
                )
                .await;
            result_redirect(&state, &format!("github_push=error&reason={reason}"))
        }
    }
}

/// Push the visitor's current configuration to a new repository on their
/// account.
#[tracing::instrument(skip_all, fields(repo = %body.repo_name))]
async fn push(
    State(state): State<AppState>,
    cookies: Cookies,
    Json(body): Json<GitHubPushRequest>,
) -> Result<Json<GitHubPushResponse>, ApiError> {
    let switch = state.kill_switch.snapshot().await;
    if !switch.scenario_enabled(SCENARIO_ID) {
        return Err(ApiError::DemoDisabled);
    }
    if state.settings.github_kit.credentials().is_none() {
        return Err(ApiError::GithubPushNotConfigured);
    }
    if !is_plausible_repo_name(&body.repo_name) {
        return Err(ApiError::GithubInvalidRepoName(body.repo_name.clone()));
    }

    let session = crate::routes::resolve_session(&state, &cookies).await?;

    let Some(token) = state.github_push.tokens.load(session.id).await? else {
        return Err(ApiError::GithubNotConnected);
    };

    let options = KitOptions {
        openapi: body.openapi,
        ts_client: body.ts_client,
        deploy: merge_deploy_targets(body.deploy),
    };
    let kit =
        crate::kit::StarterKit::generate_with(&session.config, state.sessions.registry(), options);

    let result = crate::github_push::push_kit(
        state.github_push.api.as_ref(),
        &token,
        &body.repo_name,
        body.description.as_deref(),
        &kit,
    )
    .await;

    // The token has done its one job either way — dropping it here, rather
    // than waiting for the TTL, is the whole point of keeping it server-side
    // for as short a time as possible.
    if let Err(e) = state.github_push.tokens.clear(session.id).await {
        tracing::error!(error = %e, "github push: could not clear the token after a push attempt");
    }

    match result {
        Ok(outcome) => {
            state
                .events
                .record(
                    session.id,
                    Step::success("github_push", "pushed")
                        .detail(format!(
                            "Created {}/{} and pushed the generated project as one commit.",
                            outcome.owner, outcome.repo
                        ))
                        .fact("repository", outcome.html_url.clone())
                        .fact("commit", outcome.commit_sha.clone())
                        .build(),
                )
                .await;
            tracing::info!(repo = %outcome.repo, "github push: succeeded");
            Ok(Json(GitHubPushResponse {
                owner: outcome.owner,
                repo: outcome.repo,
                html_url: outcome.html_url,
                default_branch: outcome.default_branch,
                commit_sha: outcome.commit_sha,
            }))
        }
        Err(e) => {
            tracing::warn!(error = %e, "github push: failed");
            state
                .events
                .record(
                    session.id,
                    Step::failed("github_push", "push failed")
                        .detail(format!("{e}"))
                        .build(),
                )
                .await;
            Err(ApiError::from(e))
        }
    }
}

// -------------------------------------------------------------------- router

/// The connect/callback navigation. Merged into the sensitive bucket in
/// `lib.rs`'s `build_router`, alongside the sign-in OAuth routes — both reach
/// a third party on every call.
pub fn navigation_router() -> Router<AppState> {
    Router::new()
        .route("/api/github/connect", get(connect))
        .route("/api/github/callback", get(callback))
}

/// The push action itself. Mounted alongside `/api/starter-kit` in
/// `routes::sensitive_router` — see `lib.rs`.
pub fn action_router() -> Router<AppState> {
    Router::new().route("/api/github/push", post(push))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_accepts_equal_strings() {
        assert!(constant_time_eq("abc123", "abc123"));
    }

    #[test]
    fn constant_time_eq_rejects_a_single_differing_byte() {
        assert!(!constant_time_eq("abc123", "abc124"));
    }

    #[test]
    fn constant_time_eq_rejects_different_lengths() {
        assert!(!constant_time_eq("short", "much-longer-value"));
    }

    #[test]
    fn constant_time_eq_rejects_empty_against_nonempty() {
        assert!(!constant_time_eq("", "x"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn repo_name_validation_accepts_ordinary_names() {
        for name in ["my-repo", "my_repo", "repo.name", "Repo123"] {
            assert!(is_plausible_repo_name(name), "{name} should be valid");
        }
    }

    #[test]
    fn repo_name_validation_rejects_the_obviously_invalid() {
        for name in ["", ".", "..", "has a space", "has/slash", "has:colon"] {
            assert!(!is_plausible_repo_name(name), "{name} should be invalid");
        }
    }

    #[test]
    fn deploy_targets_default_when_nothing_is_requested() {
        let merged = merge_deploy_targets(None);
        assert_eq!(merged, DeployTargets::default());
    }

    #[test]
    fn deploy_targets_only_overrides_what_was_specified() {
        let merged = merge_deploy_targets(Some(GithubPushDeployTargets {
            fly: Some(true),
            ..Default::default()
        }));
        let default = DeployTargets::default();
        assert!(merged.fly);
        assert_eq!(merged.docker, default.docker);
        assert_eq!(merged.render, default.render);
        assert_eq!(merged.railway, default.railway);
    }

    #[test]
    fn urlencode_handles_reserved_characters() {
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
    }
}
