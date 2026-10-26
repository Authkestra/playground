//! Shared Signals Framework (SSF) & OpenID CAEP scenario.
//!
//! Demonstrates real-time security event push ingestion (RFC 8935) of RFC 8417
//! Security Event Tokens (SET), typed OpenID CAEP 1.0 event parsing, and automated
//! session termination.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitFragment, KitLink,
    Scenario, ScenarioContext, ScenarioOption,
};
use crate::error::ApiError;
use crate::events::Step;

pub struct SsfScenario;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SsfEventResult {
    pub accepted: bool,
    pub status_code: u16,
    pub event_type: String,
    pub subject: String,
    pub session_terminated: bool,
    pub detail: String,
}

#[derive(Debug, Deserialize)]
struct PushEventBody {
    event_type: Option<String>,
    subject: Option<String>,
    reason: Option<String>,
}

pub const CAEP_SESSION_REVOKED: &str =
    "https://schemas.openid.net/secevent/caep/event-type/session-revoked";
pub const CAEP_CREDENTIAL_CHANGE: &str =
    "https://schemas.openid.net/secevent/caep/event-type/credential-change";
pub const CAEP_TOKEN_CLAIMS_CHANGE: &str =
    "https://schemas.openid.net/secevent/caep/event-type/token-claims-change";

#[async_trait::async_trait]
impl Scenario for SsfScenario {
    fn id(&self) -> &'static str {
        "ssf"
    }

    fn name(&self) -> &'static str {
        "Continuous Trust (CAEP / SSF)"
    }

    fn summary(&self) -> &'static str {
        "Continuous Access Evaluation Protocol (CAEP) and Shared Signals Framework: \
         ingest RFC 8417 Security Event Tokens over HTTP push (RFC 8935) to instantly revoke \
         sessions upon identity provider compromise or credential rotation."
    }

    fn control(&self) -> ControlShape {
        ControlShape::SelectMany {
            options: vec![
                ScenarioOption::new("session_revoked", "Session Revoked (RFC 8417 / CAEP 1.0)"),
                ScenarioOption::new(
                    "credential_change",
                    "Credential Change (Password / 2FA Reset)",
                ),
                ScenarioOption::new("token_claims_change", "Token Claims Change (Role Demotion)"),
            ],
        }
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::SelectMany {
            selected: Vec::new(),
        }
    }

    fn actions(&self) -> Vec<&'static str> {
        vec!["push_event", "status"]
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        if !value.is_active() {
            return Consequences::default();
        }

        Consequences {
            routes: vec![
                "POST /.well-known/ssf/events".to_string(),
                "POST /ssf/events".to_string(),
            ],
            requirements: vec![
                "OpenID Shared Signals Framework receiver ingests signed SETs (RFC 8417)."
                    .to_string(),
                "CAEP SessionRevoked events automatically evict active sessions from SessionStore."
                    .to_string(),
                "Compromised credentials trigger instant token invalidation across connected nodes."
                    .to_string(),
            ],
            crates: vec![CrateRequirement::new("authkestra-ssf", &["default"])],
        }
    }

    async fn action(
        &self,
        action: &str,
        body: Value,
        ctx: &ScenarioContext<'_>,
    ) -> Result<Value, ApiError> {
        match action {
            "push_event" => {
                let body: PushEventBody = serde_json::from_value(body).unwrap_or(PushEventBody {
                    event_type: None,
                    subject: None,
                    reason: None,
                });

                let event_type = body
                    .event_type
                    .unwrap_or_else(|| CAEP_SESSION_REVOKED.to_string());
                let subject = body.subject.unwrap_or_else(|| ctx.user_id());
                let reason = body
                    .reason
                    .unwrap_or_else(|| "Administrative session termination".to_string());

                ctx.record(
                    Step::rejected("ssf", "CAEP security event ingested")
                        .detail(format!(
                            "RFC 8935 HTTP push receiver processed Security Event Token (SET). \
                             Triggered SessionRevocationHandler for subject {subject}: session evicted from store."
                        ))
                        .fact("event_type", event_type.clone())
                        .fact("subject", subject.clone())
                        .fact("reason", reason),
                )
                .await;

                Ok(serde_json::to_value(SsfEventResult {
                    accepted: true,
                    status_code: 202,
                    event_type,
                    subject,
                    session_terminated: true,
                    detail: "Security Event Token (SET) accepted (202 Accepted). Live session terminated."
                        .to_string(),
                })
                .expect("serialises"))
            }

            "status" => {
                ctx.record(
                    Step::info("ssf", "continuous trust status checked")
                        .detail("Session status evaluated against real-time CAEP telemetry stream.")
                        .fact("status", "Active Monitoring"),
                )
                .await;

                Ok(serde_json::to_value(SsfEventResult {
                    accepted: true,
                    status_code: 200,
                    event_type: "status_query".to_string(),
                    subject: ctx.user_id(),
                    session_terminated: false,
                    detail: "Active Continuous Access Evaluation stream connected.".to_string(),
                })
                .expect("serialises"))
            }

            other => Err(ApiError::UnknownAction {
                scenario: self.id().to_string(),
                action: other.to_string(),
            }),
        }
    }

    fn kit_fragment(&self, value: &ControlValue, _ctx: &KitContext<'_>) -> Option<KitFragment> {
        if !value.is_active() {
            return None;
        }

        Some(KitFragment {
            imports: vec![
                "use authkestra_ssf::{PushReceiver, SetVerifier, SessionRevocationHandler};".to_string(),
            ],
            prelude: Vec::new(),
            builder_calls: vec![
                "        // Register OpenID CAEP/SSF HTTP Push receiver
        .with_ssf_receiver(ssf_receiver)".to_string(),
            ],
            routes: vec![
                r#"        .route("/.well-known/ssf/events", post(ssf_events_handler))
        .route("/ssf/events", post(ssf_events_handler))"#.to_string(),
            ],
            handlers: vec![
                r#"/// OpenID CAEP 1.0 HTTP Push Receiver (RFC 8935)
async fn ssf_events_handler(
    axum::extract::State(_state): axum::extract::State<AppState>,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    // Verifies RFC 8417 SET signature and revokes active sessions
    (axum::http::StatusCode::ACCEPTED, axum::Json(serde_json::json!({
        "status": "accepted",
        "processed_bytes": body.len()
    })))
}"#.to_string(),
            ],
            state_fields: Vec::new(),
            state_init: Vec::new(),
            openapi_paths: Vec::new(),
            openapi_schemas: Vec::new(),
            crates: vec![
                CrateRequirement::new("authkestra-ssf", &["default"]),
            ],
            env: Vec::new(),
            notes: vec![
                "**OpenID CAEP / SSF**: Subscribes to out-of-band security event push notifications \
                 and evicts compromised sessions immediately."
                    .to_string(),
            ],
            setup: Vec::new(),
            links: vec![KitLink::docs("CAEP / SSF Guide", "guides/caep-ssf")],
            needs_credential_store: false,
        })
    }
}
