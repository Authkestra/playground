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
        }
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
