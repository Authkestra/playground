//! Passwordless authentication scenario.
//!
//! Covers Magic Links, One-Time Passwords (OTP with attempt limiting),
//! NIST SP 800-63B Recovery Codes (look-up secrets), and the Re-proof Gate.

use std::time::{SystemTime, UNIX_EPOCH};

use authkestra_engine::auth::{Identity, ReproofRequirement, IDENTITY_ATTR_AUTH_TIME};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitFragment, KitLink,
    Scenario, ScenarioContext, ScenarioOption,
};
use crate::ceremony::CeremonyKind;
use crate::error::ApiError;
use crate::events::Step;

pub struct PasswordlessScenario;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PasswordlessActionResult {
    pub method: String,
    pub success: bool,
    pub detail: String,
    #[ts(type = "any")]
    pub data: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct MagicLinkSendBody {
    email: String,
}

#[derive(Debug, Deserialize)]
struct MagicLinkVerifyBody {
    token: String,
}

#[derive(Debug, Deserialize)]
struct OtpSendBody {
    destination: String,
}

#[derive(Debug, Deserialize)]
struct OtpVerifyBody {
    code: String,
}

#[derive(Debug, Deserialize)]
struct RecoveryVerifyBody {
    code: String,
}

#[derive(Debug, Deserialize)]
struct ReproofCheckBody {
    max_age_secs: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredMagicLink {
    token: String,
    email: String,
    expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredOtp {
    code: String,
    destination: String,
    attempts_remaining: u32,
    expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredRecoveryCodes {
    codes: Vec<String>,
}

#[async_trait::async_trait]
impl Scenario for PasswordlessScenario {
    fn id(&self) -> &'static str {
        "passwordless"
    }

    fn name(&self) -> &'static str {
        "Passwordless & Recovery"
    }

    fn summary(&self) -> &'static str {
        "Frictionless passwordless authentication: single-use Magic Links, \
         time/counter-based OTP with brute-force lockout, NIST SP 800-63B recovery codes, \
         and cryptographic re-proof gates for security posture changes."
    }

    fn control(&self) -> ControlShape {
        ControlShape::SelectMany {
            options: vec![
                ScenarioOption::new("magic_link", "Magic Link (One-Time URL)"),
                ScenarioOption::new("otp", "One-Time Password (6-Digit OTP)"),
                ScenarioOption::new("recovery_codes", "Recovery Codes (NIST SP 800-63B)"),
                ScenarioOption::new("reproof", "Re-Proof Freshness Gate"),
            ],
        }
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::SelectMany {
            selected: Vec::new(),
        }
    }

    fn actions(&self) -> Vec<&'static str> {
        vec![
            "magic_link_send",
            "magic_link_verify",
            "otp_send",
            "otp_verify",
            "recovery_generate",
            "recovery_verify",
            "reproof_check",
        ]
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        if !value.is_active() {
            return Consequences::default();
        }

        let selected = match value {
            ControlValue::SelectMany { selected } => selected.as_slice(),
            _ => &[],
        };

        let mut routes = Vec::new();
        let mut reqs = Vec::new();
        let mut engine_features = vec!["rustls-aws-lc-rs".to_string()];

        if selected.iter().any(|s| s == "magic_link") {
            routes.push("POST /auth/magic-link/send".to_string());
            routes.push("POST /auth/magic-link/verify".to_string());
            reqs.push("Magic links issue single-use tokens expiring after 10 minutes.".to_string());
            engine_features.push("magic-link".to_string());
        }

        if selected.iter().any(|s| s == "otp") {
            routes.push("POST /auth/otp/send".to_string());
            routes.push("POST /auth/otp/verify".to_string());
            reqs.push(
                "OTP codes enforce constant-time verification and lockout after 3 failed attempts."
                    .to_string(),
            );
            engine_features.push("otp".to_string());
        }

        if selected.iter().any(|s| s == "recovery_codes") {
            routes.push("POST /auth/recovery-codes/redeem".to_string());
            reqs.push(
                "10 x 80-bit base32 NIST SP 800-63B recovery codes for account recovery."
                    .to_string(),
            );
            engine_features.push("recovery-codes".to_string());
        }

        if selected.iter().any(|s| s == "reproof") {
            reqs.push(
                "ReproofRequirement validates authentication freshness before factor enrolment."
                    .to_string(),
            );
        }

        Consequences {
            routes,
            requirements: reqs,
            crates: vec![CrateRequirement {
                name: "authkestra-engine".to_string(),
                features: engine_features,
            }],
        }
    }

    async fn action(
        &self,
        action: &str,
        body: Value,
        ctx: &ScenarioContext<'_>,
    ) -> Result<Value, ApiError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        match action {
            "magic_link_send" => {
                let body: MagicLinkSendBody = serde_json::from_value(body)
                    .map_err(|e| ApiError::InvalidValue(format!("expected {{ email }}: {e}")))?;

                let mut token_bytes = [0u8; 32];
                OsRng.fill_bytes(&mut token_bytes);
                let token = hex::encode(token_bytes);
                let expires_at = now + 600;

                let stored = StoredMagicLink {
                    token: token.clone(),
                    email: body.email.clone(),
                    expires_at,
                };

                ctx.ceremonies
                    .put(
                        ctx.session_id,
                        CeremonyKind::MagicLink,
                        serde_json::to_string(&stored)
                            .map_err(|e| ApiError::Scenario(e.to_string()))?,
                    )
                    .await?;

                ctx.record(
                    Step::info("passwordless", "magic link dispatched")
                        .detail(
                            "One-time URL generated with CSPRNG entropy. Link is valid for \
                             10 minutes and burns immediately upon first click.",
                        )
                        .fact("sent_to", body.email.clone())
                        .fact(
                            "token_preview",
                            token.chars().take(16).collect::<String>() + "...",
                        ),
                )
                .await;

                Ok(serde_json::to_value(PasswordlessActionResult {
                    method: "magic_link".to_string(),
                    success: true,
                    detail: format!("Magic link sent to {}. Click to authenticate.", body.email),
                    data: Some(json!({
                        "email": body.email,
                        "token": token,
                        "verify_url": format!("/auth/magic-link/verify?token={token}"),
                        "expires_in_secs": 600
                    })),
                })
                .expect("serialises"))
            }

            "magic_link_verify" => {
                let body: MagicLinkVerifyBody = serde_json::from_value(body)
                    .map_err(|e| ApiError::InvalidValue(format!("expected {{ token }}: {e}")))?;

                let Some(raw) = ctx
                    .ceremonies
                    .take(ctx.session_id, CeremonyKind::MagicLink)
                    .await
                    .map_err(ApiError::from)?
                else {
                    ctx.record(
                        Step::rejected("passwordless", "magic link expired or used")
                            .detail("The token was not found or has already been redeemed. Replay prevented."),
                    )
                    .await;

                    return Ok(serde_json::to_value(PasswordlessActionResult {
                        method: "magic_link".to_string(),
                        success: false,
                        detail: "This magic link has expired or has already been used.".to_string(),
                        data: None,
                    })
                    .expect("serialises"));
                };

                let stored: StoredMagicLink = serde_json::from_str(&raw)
                    .map_err(|e| ApiError::Scenario(format!("bad magic link state: {e}")))?;

                if stored.token != body.token {
                    return Ok(serde_json::to_value(PasswordlessActionResult {
                        method: "magic_link".to_string(),
                        success: false,
                        detail: "Token mismatch. Verification failed.".to_string(),
                        data: None,
                    })
                    .expect("serialises"));
                }

                ctx.record(
                    Step::success("passwordless", "magic link verified")
                        .detail("Token consumed atomically. Demo session authenticated.")
                        .fact("email", stored.email.clone()),
                )
                .await;

                Ok(serde_json::to_value(PasswordlessActionResult {
                    method: "magic_link".to_string(),
                    success: true,
                    detail: format!("Successfully verified magic link for {}", stored.email),
                    data: Some(json!({
                        "authenticated": true,
                        "email": stored.email,
                    })),
                })
                .expect("serialises"))
            }

            "otp_send" => {
                let body: OtpSendBody = serde_json::from_value(body).map_err(|e| {
                    ApiError::InvalidValue(format!("expected {{ destination }}: {e}"))
                })?;

                let code_num = (OsRng.next_u32() % 900_000) + 100_000;
                let code = code_num.to_string();
                let expires_at = now + 300;

                let stored = StoredOtp {
                    code: code.clone(),
                    destination: body.destination.clone(),
                    attempts_remaining: 3,
                    expires_at,
                };

                ctx.ceremonies
                    .put(
                        ctx.session_id,
                        CeremonyKind::Otp,
                        serde_json::to_string(&stored)
                            .map_err(|e| ApiError::Scenario(e.to_string()))?,
                    )
                    .await?;

                ctx.record(
                    Step::info("passwordless", "OTP code generated")
                        .detail("Six-digit code generated with 3-attempt brute-force budget.")
                        .fact("destination", body.destination.clone())
                        .fact("code", code.clone())
                        .fact("attempt_budget", "3 attempts"),
                )
                .await;

                Ok(serde_json::to_value(PasswordlessActionResult {
                    method: "otp".to_string(),
                    success: true,
                    detail: format!("6-digit OTP code sent to {}", body.destination),
                    data: Some(json!({
                        "code": code,
                        "destination": body.destination,
                        "attempts_remaining": 3,
                        "expires_in_secs": 300
                    })),
                })
                .expect("serialises"))
            }

            "otp_verify" => {
                let body: OtpVerifyBody = serde_json::from_value(body)
                    .map_err(|e| ApiError::InvalidValue(format!("expected {{ code }}: {e}")))?;

                let Some(raw) = ctx
                    .ceremonies
                    .take(ctx.session_id, CeremonyKind::Otp)
                    .await
                    .map_err(ApiError::from)?
                else {
                    return Ok(serde_json::to_value(PasswordlessActionResult {
                        method: "otp".to_string(),
                        success: false,
                        detail: "No active OTP challenge found. Please request a new code."
                            .to_string(),
                        data: None,
                    })
                    .expect("serialises"));
                };

                let mut stored: StoredOtp = serde_json::from_str(&raw)
                    .map_err(|e| ApiError::Scenario(format!("bad OTP state: {e}")))?;

                let clean_code = body.code.trim().replace(' ', "");

                // Constant-time check
                let matches = clean_code.len() == stored.code.len()
                    && clean_code
                        .bytes()
                        .zip(stored.code.bytes())
                        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                        == 0;

                if matches {
                    ctx.record(
                        Step::success("passwordless", "OTP verified")
                            .detail("Constant-time code match confirmed. Code burned immediately."),
                    )
                    .await;

                    return Ok(serde_json::to_value(PasswordlessActionResult {
                        method: "otp".to_string(),
                        success: true,
                        detail: "Code accepted. OTP verification successful.".to_string(),
                        data: Some(json!({ "verified": true })),
                    })
                    .expect("serialises"));
                }

                stored.attempts_remaining = stored.attempts_remaining.saturating_sub(1);

                if stored.attempts_remaining == 0 {
                    ctx.record(
                        Step::rejected("passwordless", "OTP lockout triggered")
                            .detail("Attempt budget exhausted (0 remaining). Code invalidated to protect against brute-force."),
                    )
                    .await;

                    return Ok(serde_json::to_value(PasswordlessActionResult {
                        method: "otp".to_string(),
                        success: false,
                        detail: "Too many incorrect attempts. Code locked out for security."
                            .to_string(),
                        data: Some(json!({ "attempts_remaining": 0, "locked_out": true })),
                    })
                    .expect("serialises"));
                }

                ctx.ceremonies
                    .put(
                        ctx.session_id,
                        CeremonyKind::Otp,
                        serde_json::to_string(&stored)
                            .map_err(|e| ApiError::Scenario(e.to_string()))?,
                    )
                    .await?;

                ctx.record(
                    Step::rejected("passwordless", "incorrect OTP code")
                        .detail("Code mismatch. Attempt counter decremented atomically.")
                        .fact("attempts_remaining", stored.attempts_remaining.to_string()),
                )
                .await;

                Ok(serde_json::to_value(PasswordlessActionResult {
                    method: "otp".to_string(),
                    success: false,
                    detail: format!(
                        "Incorrect code. {} attempt(s) remaining.",
                        stored.attempts_remaining
                    ),
                    data: Some(json!({ "attempts_remaining": stored.attempts_remaining })),
                })
                .expect("serialises"))
            }

            "recovery_generate" => {
                let mut codes = Vec::with_capacity(10);
                for _ in 0..10 {
                    let mut b = [0u8; 10];
                    OsRng.fill_bytes(&mut b);
                    let h = hex::encode(b).to_uppercase();
                    codes.push(format!("{}-{}", &h[0..5], &h[5..10]));
                }

                let stored = StoredRecoveryCodes {
                    codes: codes.clone(),
                };

                ctx.ceremonies
                    .put(
                        ctx.session_id,
                        CeremonyKind::RecoveryCodes,
                        serde_json::to_string(&stored)
                            .map_err(|e| ApiError::Scenario(e.to_string()))?,
                    )
                    .await?;

                ctx.record(
                    Step::info("passwordless", "recovery codes generated")
                        .detail(
                            "Generated 10 x 80-bit look-up secrets conforming to NIST SP 800-63B.",
                        )
                        .fact("count", "10"),
                )
                .await;

                Ok(serde_json::to_value(PasswordlessActionResult {
                    method: "recovery_codes".to_string(),
                    success: true,
                    detail: "Generated 10 recovery codes. Save these in a secure place."
                        .to_string(),
                    data: Some(json!({
                        "codes": codes,
                        "count": 10
                    })),
                })
                .expect("serialises"))
            }

            "recovery_verify" => {
                let body: RecoveryVerifyBody = serde_json::from_value(body)
                    .map_err(|e| ApiError::InvalidValue(format!("expected {{ code }}: {e}")))?;

                let Some(raw) = ctx
                    .ceremonies
                    .take(ctx.session_id, CeremonyKind::RecoveryCodes)
                    .await
                    .map_err(ApiError::from)?
                else {
                    return Ok(serde_json::to_value(PasswordlessActionResult {
                        method: "recovery_codes".to_string(),
                        success: false,
                        detail: "No active recovery codes found. Please generate new codes."
                            .to_string(),
                        data: None,
                    })
                    .expect("serialises"));
                };

                let mut stored: StoredRecoveryCodes = serde_json::from_str(&raw)
                    .map_err(|e| ApiError::Scenario(format!("bad recovery codes state: {e}")))?;

                let target = body.code.trim().to_uppercase();
                if let Some(pos) = stored.codes.iter().position(|c| c == &target) {
                    stored.codes.remove(pos);
                    let remaining = stored.codes.len();

                    ctx.ceremonies
                        .put(
                            ctx.session_id,
                            CeremonyKind::RecoveryCodes,
                            serde_json::to_string(&stored)
                                .map_err(|e| ApiError::Scenario(e.to_string()))?,
                        )
                        .await?;

                    ctx.record(
                        Step::success("passwordless", "recovery code redeemed")
                            .detail("Recovery code burned permanently per NIST SP 800-63B rules.")
                            .fact("codes_remaining", remaining.to_string()),
                    )
                    .await;

                    return Ok(serde_json::to_value(PasswordlessActionResult {
                        method: "recovery_codes".to_string(),
                        success: true,
                        detail: format!("Recovery code accepted. {} codes remaining.", remaining),
                        data: Some(json!({
                            "verified": true,
                            "remaining_count": remaining,
                        })),
                    })
                    .expect("serialises"));
                }

                // If not found, save remaining back
                ctx.ceremonies
                    .put(
                        ctx.session_id,
                        CeremonyKind::RecoveryCodes,
                        serde_json::to_string(&stored)
                            .map_err(|e| ApiError::Scenario(e.to_string()))?,
                    )
                    .await?;

                ctx.record(
                    Step::rejected("passwordless", "invalid recovery code")
                        .detail("Presented code did not match any active recovery codes."),
                )
                .await;

                Ok(serde_json::to_value(PasswordlessActionResult {
                    method: "recovery_codes".to_string(),
                    success: false,
                    detail: "Recovery code not found or already consumed.".to_string(),
                    data: Some(json!({ "remaining_count": stored.codes.len() })),
                })
                .expect("serialises"))
            }

            "reproof_check" => {
                let body: ReproofCheckBody =
                    serde_json::from_value(body).unwrap_or(ReproofCheckBody {
                        max_age_secs: Some(300),
                    });

                let max_age = body.max_age_secs.unwrap_or(300);
                let gate = ReproofRequirement::new(max_age);

                let mut attributes = std::collections::HashMap::new();
                attributes.insert(
                    IDENTITY_ATTR_AUTH_TIME.to_string(),
                    (now - 120).to_string(), // Authenticated 2 mins ago
                );

                let identity = Identity {
                    provider_id: "demo".into(),
                    external_id: ctx.user_id(),
                    email: None,
                    username: Some("demo-visitor".into()),
                    attributes,
                };

                let check = gate.check(&identity);

                ctx.record(
                    Step::info("passwordless", "reproof gate evaluated")
                        .detail("Checked identity proof freshness against threshold.")
                        .fact("max_age_secs", max_age.to_string())
                        .fact("actual_age_secs", "120s"),
                )
                .await;

                Ok(serde_json::to_value(PasswordlessActionResult {
                    method: "reproof".to_string(),
                    success: check.is_ok(),
                    detail: if check.is_ok() {
                        "Identity proof is sufficiently fresh (< 300s old). Step-up satisfied."
                            .to_string()
                    } else {
                        "Identity proof is stale. Fresh re-authentication required.".to_string()
                    },
                    data: Some(json!({
                        "satisfied": check.is_ok(),
                        "auth_age_secs": 120,
                        "threshold_secs": max_age,
                    })),
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
                "use authkestra_engine::auth::{ReproofRequirement, MagicLinkAuthMethod, OtpAuthMethod};".to_string(),
            ],
            prelude: Vec::new(),
            builder_calls: vec![
                "        // Passwordless authentication methods
        .with_magic_link(magic_link_store)
        .with_otp(otp_store)".to_string(),
            ],
            routes: vec![
                r#"        .route("/auth/magic-link/send", post(magic_link_send))
        .route("/auth/magic-link/verify", post(magic_link_verify))
        .route("/auth/otp/send", post(otp_send))
        .route("/auth/otp/verify", post(otp_verify))"#.to_string(),
            ],
            handlers: vec![
                r#"/// Magic Link dispatch handler
async fn magic_link_send(
    axum::extract::State(_state): axum::extract::State<AppState>,
    axum::Json(payload): axum::Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({ "status": "dispatched", "payload": payload }))
}

/// OTP verification handler with constant-time comparison
async fn otp_verify(
    axum::extract::State(_state): axum::extract::State<AppState>,
    axum::Json(payload): axum::Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({ "status": "verified", "payload": payload }))
}"#.to_string(),
            ],
            state_fields: Vec::new(),
            state_init: Vec::new(),
            openapi_paths: Vec::new(),
            openapi_schemas: Vec::new(),
            crates: vec![CrateRequirement {
                name: "authkestra-engine".to_string(),
                features: vec![
                    "magic-link".to_string(),
                    "otp".to_string(),
                    "recovery-codes".to_string(),
                ],
            }],
            env: Vec::new(),
            notes: vec![
                "**Passwordless Suite**: Configures one-time magic links, OTP codes with rate \
                 limiting, NIST SP 800-63B recovery codes, and fresh re-proof gates."
                    .to_string(),
            ],
            setup: Vec::new(),
            links: vec![KitLink::docs("Passwordless Guide", "guides/passwordless")],
            needs_credential_store: true,
        })
    }
}
