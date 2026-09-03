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
