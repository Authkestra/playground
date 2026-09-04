//! TOTP scenario: authenticator-app codes (roadmap P2).
//!
//! Wraps `authkestra_engine::auth::totp::TotpAuthMethod`, which is where the
//! real work happens — secret generation, the provisioning URI, verification
//! with clock-skew tolerance and replay protection. The playground supplies the
//! per-visitor identity (the demo session) and the HTTP shape.
//!
//! Two ceremony steps, both through the generic action endpoint:
//!
//! * `provision` — generate a secret, return the `otpauth://` URI for a QR code
//! * `verify`    — check a six-digit code
//!
//! The QR is rendered **client-side** from the URI. Rendering it server-side
//! would mean the secret travelling as an image the browser caches, and would
//! add an image encoder to a service that otherwise only speaks JSON.

use authkestra_engine::auth::totp::TotpAuthMethod;
use authkestra_engine::auth::AuthMethod;
use authkestra_engine::AuthInput;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, Scenario, ScenarioContext,
    TryOutcome, TryResult,
};
use crate::error::ApiError;
use crate::events::Step;

/// Shown in the authenticator app next to the code.
const ISSUER: &str = "Authkestra Playground";

pub struct TotpScenario;

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TotpProvision {
    /// Base32 secret. Shown so a visitor can type it in when they cannot scan.
    pub secret: String,
    /// `otpauth://` URI the frontend renders as a QR code.
    pub uri: String,
}

#[derive(Debug, Deserialize)]
struct VerifyBody {
    code: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TotpVerification {
    pub verified: bool,
    pub detail: String,
}

#[async_trait::async_trait]
impl Scenario for TotpScenario {
    fn id(&self) -> &'static str {
        "totp"
    }

    fn name(&self) -> &'static str {
        "Authenticator app (TOTP)"
    }

    fn summary(&self) -> &'static str {
        "Six-digit codes from an authenticator app. Scan the QR with any TOTP app, \
         then verify a code against the running engine."
    }

    fn control(&self) -> ControlShape {
        ControlShape::Toggle
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::Toggle { enabled: false }
    }

    fn actions(&self) -> Vec<&'static str> {
        vec!["provision", "verify"]
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        if !value.is_active() {
            return Consequences::default();
        }
        Consequences {
            routes: vec![
                "POST /auth/totp/enroll".to_string(),
                "POST /auth/totp/verify".to_string(),
            ],
            requirements: vec![
                "Users enrol an authenticator app once, then supply a 6-digit code at sign-in."
                    .to_string(),
                "You store one TOTP secret per user — treat it as a credential, not a profile field."
                    .to_string(),
            ],
            // The facade does NOT expose `totp`; it lives on the engine. Naming
            // the wrong crate here would send someone down a dead end, which is
            // the exact trap docs/decisions/0001 exists to prevent.
            crates: vec![
                CrateRequirement::new("authkestra-engine", &["totp", "sql-sqlite"]),
                CrateRequirement::new("sqlx", &["sqlite"]),
            ],
        }
    }

    async fn try_run(&self, ctx: &ScenarioContext<'_>) -> Result<TryResult, ApiError> {
        if !ctx.value.is_active() {
            return Ok(TryResult {
                outcome: TryOutcome::NotConfigured,
                detail: "Turn TOTP on first.".to_string(),
            });
        }

        let method = TotpAuthMethod::new(ctx.credentials());
        let enrolled = method
            .has_enrolled(&ctx.user_id())
            .await
            .map_err(|e| ApiError::Scenario(e.to_string()))?;

        Ok(if enrolled {
            TryResult {
                outcome: TryOutcome::Ok,
                detail: "An authenticator is enrolled for this session. Enter a code to verify it."
                    .to_string(),
            }
        } else {
            TryResult {
                outcome: TryOutcome::NotConfigured,
                detail: "Scan the QR code with an authenticator app to enrol first.".to_string(),
            }
        })
    }

    async fn action(
        &self,
        action: &str,
        body: Value,
        ctx: &ScenarioContext<'_>,
    ) -> Result<Value, ApiError> {
        if !ctx.value.is_active() {
            return Err(ApiError::InvalidValue(
                "Turn TOTP on before running its flows.".to_string(),
            ));
        }

        let method = TotpAuthMethod::new(ctx.credentials());
        let user_id = ctx.user_id();

        match action {
            // Re-enrolling replaces the previous secret rather than adding a
            // second one, because the credential store files a session's TOTP
            // secret under a fixed id. That used to need an explicit delete
            // here; it is now impossible to get wrong.
            "provision" => {
                let (secret, uri) = method
                    .register_totp(&user_id, ISSUER, "demo-visitor")
                    .await
                    .map_err(|e| ApiError::Scenario(e.to_string()))?;

                tracing::info!(session_id = %ctx.session_id, "TOTP secret provisioned");
                ctx.record(
                    Step::info("totp", "secret generated")
                        .detail(
                            "A shared secret was generated for this session and stored as a \
                             credential. The QR code encodes it as an otpauth:// URI — the \
                             secret itself never leaves your browser and this server.",
                        )
                        .fact("algorithm", "SHA1")
                        .fact("digits", "6")
                        .fact("period", "30s"),
                )
                .await;
                Ok(serde_json::to_value(TotpProvision { secret, uri })
                    .expect("provision serialises"))
            }

            "verify" => {
                let body: VerifyBody = serde_json::from_value(body).map_err(|_| {
                    ApiError::InvalidValue("expected { \"code\": \"123456\" }".into())
                })?;
                let code = body.code.trim().replace(' ', "");

                if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
                    ctx.record(
                        Step::rejected("totp", "malformed code")
                            .detail(
                                "Rejected before any cryptography: a TOTP code is always six \
                                 digits, so there is nothing to verify.",
                            )
                            .fact("received length", code.len().to_string()),
                    )
                    .await;
                    return Ok(serde_json::to_value(TotpVerification {
                        verified: false,
                        detail: "A TOTP code is six digits.".to_string(),
                    })
                    .expect("verification serialises"));
                }

                let result = method
                    .authenticate(AuthInput::Totp {
                        user_id: user_id.clone(),
                        code,
                    })
                    .await;

                Ok(match result {
                    Ok(_) => {
                        tracing::info!(session_id = %ctx.session_id, "TOTP code verified");
                        ctx.record(
                            Step::success("totp", "code verified")
                                .detail(
                                    "The engine derived the expected code from the stored \
                                     secret and the current time step, and it matched. The \
                                     step was then recorded so the same code cannot be \
                                     replayed.",
                                )
                                .fact("clock skew allowed", "±1 step (30s)"),
                        )
                        .await;
                        serde_json::to_value(TotpVerification {
                            verified: true,
                            detail: "Code accepted. The engine verified it against the enrolled secret."
                                .to_string(),
                        })
                    }
                    // A wrong code is an ordinary outcome of this demo, not a
                    // server error — it must render as a normal result.
                    Err(e) => {
                        tracing::debug!(session_id = %ctx.session_id, error = %e, "TOTP verification failed");
                        ctx.record(
                            Step::rejected("totp", "code rejected")
                                .detail(
                                    "The code did not match the expected value for any \
                                     allowed time step — or it was already used. Reusing a \
                                     code is refused on purpose: that is what stops someone \
                                     replaying one they observed.",
                                ),
                        )
                        .await;
                        serde_json::to_value(TotpVerification {
                            verified: false,
                            detail: "That code was rejected. Codes expire every 30 seconds, and \
                                     each one can only be used once."
                                .to_string(),
                        })
                    }
                }
                .expect("verification serialises"))
            }

            other => Err(ApiError::UnknownAction {
                scenario: self.id().to_string(),
                action: other.to_string(),
            }),
        }
    }
}

/// Convenience for tests and the frontend: the shape `verify` accepts.
pub fn verify_body(code: &str) -> Value {
    json!({ "code": code })
}
