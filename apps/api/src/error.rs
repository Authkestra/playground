//! API error type and its wire format.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    /// The kill switch is off. The frontend renders explainer-only mode.
    DemoDisabled,
    /// A scenario id that is not in the registry.
    UnknownScenario(String),
    /// The submitted control value does not fit the scenario's control.
    InvalidValue(String),
    /// The demo session expired or never existed.
    SessionGone,
    Unauthorized,
    /// A ceremony step the scenario does not define.
    UnknownAction {
        scenario: String,
        action: String,
    },
    /// The scenario itself failed. Distinct from a flow *rejecting* input —
    /// a wrong TOTP code is a normal result, not this.
    Scenario(String),
    /// The challenge was never issued, already answered, or timed out.
    CeremonyExpired,
    /// The authenticator's response did not verify.
    CeremonyRejected(String),
    /// The state backend is unreachable. Distinct from a scenario fault: the
    /// service cannot serve anyone until it recovers.
    StateUnavailable(String),
    /// The starter kit could not be packed. Always a bug here, never the
    /// visitor's doing.
    ArchiveFailed(String),
    /// This deployment has no `GITHUB_KIT_CLIENT_ID`/`SECRET` configured, so
    /// pushing to GitHub cannot be offered at all (#40).
    GithubPushNotConfigured,
    /// The visitor tried to push before connecting a GitHub account, or their
    /// connection's short TTL ran out first.
    GithubNotConnected,
    /// GitHub rejected the repository name because one like it already
    /// exists on the connected account.
    GithubRepoNameTaken,
    /// The repository name is not one GitHub will accept.
    GithubInvalidRepoName(String),
    /// The connected token is dead — expired, revoked, or never valid.
    GithubTokenRejected,
    /// The connected token lacks the scope a step needed.
    GithubScopeMissing,
    /// GitHub's own rate limit was hit.
    GithubRateLimited,
    /// The request never reached GitHub, or its response never reached us.
    GithubNetworkError(String),
    /// GitHub refused the request for a reason not covered above.
    GithubPushFailed(String),
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            ApiError::DemoDisabled => (
                StatusCode::SERVICE_UNAVAILABLE,
                "demo_disabled",
                "Live demo flows are temporarily switched off.".to_string(),
            ),
            ApiError::UnknownScenario(id) => (
                StatusCode::NOT_FOUND,
                "unknown_scenario",
                format!("No scenario with id `{id}`."),
            ),
            ApiError::ArchiveFailed(detail) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "archive_failed",
                format!("The starter kit could not be packaged: {detail}"),
            ),
            ApiError::InvalidValue(detail) => {
                (StatusCode::BAD_REQUEST, "invalid_value", detail.clone())
            }
            ApiError::SessionGone => (
                StatusCode::GONE,
                "session_gone",
                "That demo session has expired. Reload to start a new one.".to_string(),
            ),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing or incorrect admin token.".to_string(),
            ),
            ApiError::UnknownAction { scenario, action } => (
                StatusCode::NOT_FOUND,
                "unknown_action",
                format!("Scenario `{scenario}` has no action `{action}`."),
            ),
            ApiError::Scenario(detail) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "scenario_failed",
                detail.clone(),
            ),
            ApiError::CeremonyExpired => (
                StatusCode::GONE,
                "ceremony_expired",
                "That request timed out or was already completed. Start again.".to_string(),
            ),
            ApiError::CeremonyRejected(detail) => {
                (StatusCode::BAD_REQUEST, "ceremony_rejected", detail.clone())
            }
            ApiError::StateUnavailable(detail) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "state_unavailable",
                format!("The playground's state store is unreachable: {detail}"),
            ),
            ApiError::GithubPushNotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "github_push_not_configured",
                "Pushing to GitHub is not configured on this deployment yet.".to_string(),
            ),
            ApiError::GithubNotConnected => (
                StatusCode::BAD_REQUEST,
                "github_not_connected",
                "Connect a GitHub account first: GET /api/github/connect.".to_string(),
            ),
            ApiError::GithubRepoNameTaken => (
                StatusCode::CONFLICT,
                "github_repo_name_taken",
                "A repository with that name already exists on your GitHub account. Choose a \
                 different name."
                    .to_string(),
            ),
            ApiError::GithubInvalidRepoName(detail) => (
                StatusCode::BAD_REQUEST,
                "github_invalid_repo_name",
                format!(
                    "`{detail}` is not a repository name GitHub will accept. Use letters, \
                     digits, hyphens, underscores and periods only."
                ),
            ),
            ApiError::GithubTokenRejected => (
                StatusCode::UNAUTHORIZED,
                "github_token_rejected",
                "Your GitHub connection has expired or was revoked. Connect again.".to_string(),
            ),
            ApiError::GithubScopeMissing => (
                StatusCode::FORBIDDEN,
                "github_scope_missing",
                "The connected GitHub token does not carry the `public_repo` scope this needs."
                    .to_string(),
            ),
            ApiError::GithubRateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "github_rate_limited",
                "GitHub's own rate limit was hit. Wait a few minutes and try again.".to_string(),
            ),
            ApiError::GithubNetworkError(detail) => (
                StatusCode::BAD_GATEWAY,
                "github_network_error",
                format!("Could not reach GitHub: {detail}"),
            ),
            ApiError::GithubPushFailed(detail) => (
                StatusCode::BAD_GATEWAY,
                "github_push_failed",
                format!("GitHub rejected the request: {detail}"),
            ),
        }
    }
}

impl From<crate::github_api::GitHubApiError> for ApiError {
    /// Each of `GitHubApiError`'s variants keeps its own identity rather than
    /// collapsing into one generic failure — a repo-name clash, a dead token
    /// and a rate limit each have an unrelated fix, and a visitor can only act
    /// on the one that actually happened.
    fn from(e: crate::github_api::GitHubApiError) -> Self {
        use crate::github_api::GitHubApiError as E;
        match e {
            E::RepoNameTaken => ApiError::GithubRepoNameTaken,
            E::InvalidRepoName(detail) => ApiError::GithubInvalidRepoName(detail),
            E::TokenRejected => ApiError::GithubTokenRejected,
            E::ScopeMissing => ApiError::GithubScopeMissing,
            E::RateLimited => ApiError::GithubRateLimited,
            E::Network(detail) => ApiError::GithubNetworkError(detail),
            E::Other(detail) => ApiError::GithubPushFailed(detail),
        }
    }
}

impl From<crate::store::StoreError> for ApiError {
    fn from(e: crate::store::StoreError) -> Self {
        ApiError::StateUnavailable(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error, detail) = self.parts();
        if status.is_server_error() {
            tracing::error!(%error, %detail, "request failed");
        } else {
            tracing::debug!(%error, %detail, "request rejected");
        }
        (status, Json(json!({ "error": error, "detail": detail }))).into_response()
    }
}
