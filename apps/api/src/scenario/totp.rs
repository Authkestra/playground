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
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitFragment, KitLink,
    Scenario, ScenarioContext,
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

    fn kit_fragment(&self, value: &ControlValue, ctx: &KitContext<'_>) -> Option<KitFragment> {
        if !value.is_active() {
            return None;
        }

        // TOTP changes role by company, following the framework's own MFA
        // example. Alone it is the only way in, so it must be a first factor.
        // Alongside another method it is registered as step-up, which is the
        // stronger design and almost certainly what was intended.
        let alone = !ctx.has_company("totp");
        let (call, note) = if alone {
            (
                "        // TOTP as a *first factor* — it is the only method registered.
        .with_totp(SqlxCredentialStore::new(pool.clone()))",
                "**TOTP is registered as a first factor**, because it is the only method \
                 you selected. That means a six-digit code is the whole of authentication. \
                 If you add another method later, move this to `.with_mfa_method(...)` so \
                 TOTP becomes a second factor instead.",
            )
        } else {
            (
                "        // TOTP as *step-up only*: it cannot start a session on its own,
        // only answer an `MfaRequired` challenge from another method.
        .with_mfa_method(authkestra_engine::auth::totp::TotpAuthMethod::new(\n            SqlxCredentialStore::new(pool.clone()),\n        ))",
                "**TOTP is registered as step-up only**, because another method is also \
                 enabled. It cannot start a session by itself — it answers an `MfaRequired` \
                 challenge. Swap it to `.with_totp(...)` to make it a first factor.",
            )
        };

        Some(KitFragment {
            imports: vec![
                "use authkestra_engine::auth::totp::TotpAuthMethod;".to_string(),
                "use authkestra_engine::auth::store::CredentialStore;".to_string(),
                "use authkestra_engine::auth::AuthInput;".to_string(),
                "use authkestra_engine::auth::AuthMethod;".to_string(),
                "use axum::extract::State;".to_string(),
                "use axum::routing::post;".to_string(),
                "use uuid::Uuid;".to_string(),
            ],
            prelude: Vec::new(),
            builder_calls: vec![call.to_string()],
            routes: vec![
                r#"        // TOTP enrolment and verification. Like WebAuthn, the framework wires
        // no routes for this: the ceremony is yours to expose.
        .route("/auth/totp/enroll", post(totp_enrol))
        .route("/auth/totp/verify", post(totp_verify))"#
                    .to_string(),
            ],
            handlers: vec![r##"#[derive(serde::Deserialize)]
struct TotpEnrol {
    username: String,
}

#[derive(serde::Deserialize)]
struct TotpVerify {
    username: String,
    code: String,
}

/// Enrol an authenticator app.
///
/// Returns the secret and an `otpauth://` URI. Render the URI as a QR code;
/// show the secret only as the manual fallback. It is a credential, not a
/// profile field — do not log it, and do not return it again afterwards.
async fn totp_enrol(
    State(state): State<AppState>,
    Json(body): Json<TotpEnrol>,
) -> impl IntoResponse {
    let method = TotpAuthMethod::new(SqlxCredentialStore::new(state.pool.clone()));
    let user_id = user_id_for(&body.username);

    // Refuse a second enrolment rather than silently creating one that cannot
    // work.
    //
    // `register_totp` saves under a fresh credential id every time, and
    // `CredentialStore` exposes no way to remove one — only save, get and
    // update. So a second enrolment leaves two secrets, and verification
    // matches the *first*: the QR code just handed over is dead while the old
    // authenticator, possibly on the phone being replaced, still works.
    //
    // Deleting the row directly would mean reaching past the trait into the
    // store's schema. Refusing is honest and keeps the failure in front of the
    // person who can act on it.
    match SqlxCredentialStore::new(state.pool.clone())
        .get_credentials(&user_id, "totp")
        .await
    {
        Ok(existing) if !existing.is_empty() => {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "already enrolled",
                    "detail": "An authenticator is already enrolled for this user. \
                               Remove the stored credential before enrolling another — \
                               the framework's CredentialStore has no delete, so that \
                               is your application's job.",
                })),
            )
                .into_response();
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, "could not check for an existing authenticator");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "could not read credentials" })),
            )
                .into_response();
        }
    }

    match method
        .register_totp(&user_id, "Authkestra Starter", &body.username)
        .await
    {
        Ok((secret, uri)) => (StatusCode::OK, Json(json!({ "secret": secret, "uri": uri })))
            .into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "could not provision TOTP");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "could not provision an authenticator" })),
            )
                .into_response()
        }
    }
}

/// Check a six-digit code.
///
/// A wrong code is an ordinary outcome, not a server fault, so it answers 401
/// with `verified: false` rather than an error shape the caller has to
/// special-case.
async fn totp_verify(
    State(state): State<AppState>,
    Json(body): Json<TotpVerify>,
) -> impl IntoResponse {
    let method = TotpAuthMethod::new(SqlxCredentialStore::new(state.pool.clone()));
    let user_id = user_id_for(&body.username);

    match method
        .authenticate(AuthInput::Totp {
            user_id,
            code: body.code,
        })
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({ "verified": true, "detail": "code accepted" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "verified": false, "detail": e.to_string() })),
        )
            .into_response(),
    }
}"##
            .to_string()],
            state_fields: Vec::new(),
            state_init: Vec::new(),
            crates: vec![
                CrateRequirement::new("uuid", &["v4", "v5"]),
                CrateRequirement::new("serde", &["derive"]),
            ],
            env: Vec::new(),
            notes: vec![
                note.to_string(),
                "Enrolment generates one secret per user and returns an `otpauth://` URI for \
                 a QR code. Treat the secret as a credential, not a profile field."
                    .to_string(),
                "Two routes are generated for you: `POST /auth/totp/enroll` returns the \
                 secret and URI, and `POST /auth/totp/verify` checks a code. Verification \
                 goes through `AuthMethod::authenticate`, which is what advances the \
                 replay window — a code accepted twice would otherwise be a valid replay."
                    .to_string(),
            ],
            // Nothing to register anywhere: enrolment happens in your own app,
            // and the authenticator is whatever the user already has.
            setup: Vec::new(),
            links: vec![
                KitLink::docs("TOTP", "providers/totp"),
                KitLink::example("crates/authkestra-engine/examples/totp_webauthn.rs"),
            ],
            needs_credential_store: true,
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
