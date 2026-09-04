//! Passkeys scenario: WebAuthn registration and authentication (roadmap P2).
//!
//! The flagship demo. `authkestra_engine::auth::webauthn::WebAuthnAuthMethod`
//! does the cryptography; the playground supplies the per-visitor identity, the
//! ceremony state, and the HTTP shape.
//!
//! Four ceremony steps, all through the generic action endpoint:
//!
//! * `register_start`     → challenge for `navigator.credentials.create()`
//! * `register_finish`    → verify the attestation, store the passkey
//! * `authenticate_start` → challenge for `navigator.credentials.get()`
//! * `authenticate_finish`→ verify the assertion, advance the signature counter
//!
//! **Relying-party identity is the frontend's domain, not the API's.** The
//! ceremony runs in the browser at the page's origin, so `WEBAUTHN_RP_ID` must
//! be the site the visitor is looking at, and `WEBAUTHN_ORIGIN` must match it
//! exactly. Getting this wrong fails inside the browser with a deliberately
//! vague error, so it is worth checking first when a ceremony misbehaves.
//! A passkey is also bound to the RP ID that created it: changing domains means
//! visitors re-register.

use std::sync::Arc;

use authkestra_engine::auth::webauthn::WebAuthnAuthMethod;
use authkestra_engine::auth::{AuthMethod, WebAuthnStarter};
use authkestra_engine::AuthInput;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use webauthn_rs::prelude::{Passkey, PasskeyRegistration, Webauthn};
use webauthn_rs::WebauthnBuilder;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, Scenario, ScenarioContext,
    TryOutcome, TryResult,
};
use crate::ceremony::CeremonyKind;
use crate::error::ApiError;
use crate::events::Step;
use crate::settings::RelyingParty;

pub struct PasskeysScenario;

/// Result of a completed authentication, surfaced so the demo can show that
/// the signature counter actually moved.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PasskeyAuthResult {
    pub verified: bool,
    pub detail: String,
    /// The authenticator's signature counter after this ceremony. A counter
    /// that fails to advance is how cloned authenticators are detected.
    pub counter: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PasskeyEnrolment {
    pub enrolled: bool,
    pub count: u32,
}

/// Build the relying party for a ceremony.
fn webauthn(rp: &RelyingParty) -> Result<Arc<Webauthn>, ApiError> {
    let origin = url::Url::parse(&rp.origin).map_err(|e| {
        ApiError::Scenario(format!(
            "WEBAUTHN_ORIGIN `{}` is not a valid URL: {e}",
            rp.origin
        ))
    })?;

    let builder = WebauthnBuilder::new(&rp.id, &origin).map_err(|e| {
        ApiError::Scenario(format!(
            "WebAuthn relying party is misconfigured (rp_id `{}`, origin `{}`): {e}. \
             The RP ID must be a registrable suffix of the origin's host.",
            rp.id, rp.origin
        ))
    })?;

    Ok(Arc::new(builder.rp_name(&rp.name).build().map_err(
        |e| ApiError::Scenario(format!("failed to build relying party: {e}")),
    )?))
}

/// Passkeys already enrolled for this session.
async fn stored_passkeys(ctx: &ScenarioContext<'_>) -> Result<Vec<Passkey>, ApiError> {
    use authkestra_engine::auth::store::CredentialStore;
    let creds = ctx
        .credentials()
        .get_credentials(&ctx.user_id(), "webauthn")
        .await
        .map_err(|e| ApiError::Scenario(e.to_string()))?;

    creds
        .into_iter()
        .map(|v| {
            serde_json::from_value(v)
                .map_err(|e| ApiError::Scenario(format!("stored passkey is unreadable: {e}")))
        })
        .collect()
}

#[async_trait::async_trait]
impl Scenario for PasskeysScenario {
    fn id(&self) -> &'static str {
        "passkeys"
    }

    fn name(&self) -> &'static str {
        "Passkeys (WebAuthn)"
    }

    fn summary(&self) -> &'static str {
        "Register a passkey with this device's authenticator, then sign in with it. \
         No password is ever created or sent."
    }

    fn control(&self) -> ControlShape {
        ControlShape::Toggle
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::Toggle { enabled: false }
    }

    fn actions(&self) -> Vec<&'static str> {
        vec![
            "register_start",
            "register_finish",
            "authenticate_start",
            "authenticate_finish",
        ]
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        if !value.is_active() {
            return Consequences::default();
        }
        Consequences {
            routes: vec![
                "POST /auth/passkey/register/start".to_string(),
                "POST /auth/passkey/register/finish".to_string(),
                "POST /auth/passkey/login/start".to_string(),
                "POST /auth/passkey/login/finish".to_string(),
            ],
            requirements: vec![
                "Users register a passkey once per device; there is no password to store or reset."
                    .to_string(),
                "You must serve over HTTPS and pin a relying-party ID to your domain — a passkey \
                 is bound to the domain that created it."
                    .to_string(),
                "You store one credential per passkey and persist its signature counter, which is \
                 how a cloned authenticator is detected."
                    .to_string(),
            ],
            // `webauthn` is an engine feature; the facade does not expose it.
            crates: vec![
                CrateRequirement::new("authkestra-engine", &["webauthn", "sql-sqlite"]),
                CrateRequirement::new("webauthn-rs", &[]),
                CrateRequirement::new("sqlx", &["sqlite"]),
            ],
        }
    }

    async fn try_run(&self, ctx: &ScenarioContext<'_>) -> Result<TryResult, ApiError> {
        if !ctx.value.is_active() {
            return Ok(TryResult {
                outcome: TryOutcome::NotConfigured,
                detail: "Turn passkeys on first.".to_string(),
            });
        }

        let count = stored_passkeys(ctx).await?.len();
        Ok(if count > 0 {
            TryResult {
                outcome: TryOutcome::Ok,
                detail: format!(
                    "{count} passkey{} registered for this session. Try signing in with it.",
                    if count == 1 { "" } else { "s" }
                ),
            }
        } else {
            TryResult {
                outcome: TryOutcome::NotConfigured,
                detail: "Register a passkey first — your device will prompt you.".to_string(),
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
                "Turn passkeys on before running its flows.".to_string(),
            ));
        }

        let rp = webauthn(ctx.relying_party)?;
        let method = WebAuthnAuthMethod::new(rp, ctx.credentials());
        let user_id = ctx.user_id();

        match action {
            "register_start" => {
                let (challenge, state) = method
                    .start_register(&user_id, "demo-visitor")
                    .map_err(|e| ApiError::Scenario(e.to_string()))?;

                ctx.ceremonies
                    .put(
                        ctx.session_id,
                        CeremonyKind::Registration,
                        serde_json::to_string(&state)
                            .map_err(|e| ApiError::Scenario(e.to_string()))?,
                    )
                    .await?;

                tracing::info!(session_id = %ctx.session_id, "passkey registration started");
                ctx.record(
                    Step::info("passkeys", "challenge issued")
                        .detail(
                            "A random challenge was generated and held server-side. Your \
                             authenticator must sign exactly this value, which is what stops \
                             a captured response being replayed later.",
                        )
                        .fact("relying party", ctx.relying_party.id.clone())
                        .fact("answerable for", "5 minutes, once"),
                )
                .await;
                serde_json::to_value(challenge).map_err(|e| ApiError::Scenario(e.to_string()))
            }

            "register_finish" => {
                // Taking the state consumes it, so a challenge is answerable
                // exactly once and an abandoned ceremony simply expires.
                let Some(raw) = ctx
                    .ceremonies
                    .take(ctx.session_id, CeremonyKind::Registration)
                    .await
                    .map_err(ApiError::from)?
                else {
                    return Err(ApiError::CeremonyExpired);
                };
                let state: PasskeyRegistration = serde_json::from_str(&raw)
                    .map_err(|e| ApiError::Scenario(format!("bad ceremony state: {e}")))?;

                let credential = serde_json::from_value(body).map_err(|e| {
                    ApiError::InvalidValue(format!("not a WebAuthn registration response: {e}"))
                })?;

                match method.finish_register(&user_id, credential, state).await {
                    Ok(_) => {
                        tracing::info!(session_id = %ctx.session_id, "passkey registered");
                        ctx.record(
                            Step::success("passkeys", "passkey registered")
                                .detail(
                                    "The attestation verified against the challenge, and the \
                                     public key was stored. The private key never left your \
                                     device — this server could not use it even if it wanted \
                                     to.",
                                )
                                .fact("stored", "public key + signature counter"),
                        )
                        .await;
                        let count = stored_passkeys(ctx).await?.len() as u32;
                        serde_json::to_value(PasskeyEnrolment {
                            enrolled: true,
                            count,
                        })
                        .map_err(|e| ApiError::Scenario(e.to_string()))
                    }
                    // A rejected attestation is a normal outcome of a public
                    // demo (wrong device, cancelled prompt), not a server fault.
                    Err(e) => {
                        tracing::debug!(session_id = %ctx.session_id, error = %e, "passkey registration rejected");
                        ctx.record(Step::rejected("passkeys", "registration rejected").detail(
                            "The attestation did not verify against the issued challenge. \
                             That happens if the prompt was cancelled, the challenge expired, \
                             or the response was for a different origin.",
                        ))
                        .await;
                        Err(ApiError::CeremonyRejected(
                            "That registration could not be verified. Try again.".to_string(),
                        ))
                    }
                }
            }

            "authenticate_start" => {
                let passkeys = stored_passkeys(ctx).await?;
                if passkeys.is_empty() {
                    return Err(ApiError::InvalidValue(
                        "Register a passkey before trying to sign in with one.".to_string(),
                    ));
                }

                let (challenge, state) = method
                    .start_authentication(&passkeys)
                    .map_err(|e| ApiError::Scenario(e.to_string()))?;

                ctx.ceremonies
                    .put(
                        ctx.session_id,
                        CeremonyKind::Authentication,
                        serde_json::to_string(&state)
                            .map_err(|e| ApiError::Scenario(e.to_string()))?,
                    )
                    .await?;

                tracing::info!(session_id = %ctx.session_id, "passkey authentication started");
                ctx.record(
                    Step::info("passkeys", "sign-in challenge issued")
                        .detail(
                            "A fresh challenge was issued, listing the credentials enrolled \
                             for this session so your device knows which key to use.",
                        )
                        .fact("credentials offered", passkeys.len().to_string()),
                )
                .await;
                serde_json::to_value(challenge).map_err(|e| ApiError::Scenario(e.to_string()))
            }

            "authenticate_finish" => {
                let Some(auth_state_json) = ctx
                    .ceremonies
                    .take(ctx.session_id, CeremonyKind::Authentication)
                    .await
                    .map_err(ApiError::from)?
                else {
                    return Err(ApiError::CeremonyExpired);
                };

                // Routed through the engine's `authenticate` rather than
                // `finish_authentication`, because that path also advances and
                // persists the signature counter — the clone-detection signal
                // this scenario is meant to demonstrate.
                let assertion: Assertion = serde_json::from_value(body).map_err(|e| {
                    ApiError::InvalidValue(format!("not a WebAuthn assertion: {e}"))
                })?;

                let result = method
                    .authenticate(AuthInput::WebAuthnAuthentication {
                        user_id: user_id.clone(),
                        credential_id: assertion.id,
                        client_data_json: assertion.response.client_data_json,
                        authenticator_data: assertion.response.authenticator_data,
                        signature: assertion.response.signature,
                        user_handle: assertion.response.user_handle,
                        auth_state_json: Some(auth_state_json),
                    })
                    .await;

                match result {
                    Ok(_) => {
                        // Read the counter back from storage rather than from
                        // the ceremony result: the persisted value is the one
                        // that will be compared on the next sign-in, so it is
                        // the honest number to show.
                        let counter = persisted_counter(ctx).await;
                        tracing::info!(session_id = %ctx.session_id, "passkey authentication verified");
                        let mut step = Step::success("passkeys", "signature verified").detail(
                            "Your authenticator signed the challenge with the private key, and \
                             the signature checked out against the stored public key. The \
                             engine then advanced the signature counter — a counter that fails \
                             to move is how a cloned authenticator is spotted.",
                        );
                        if let Some(counter) = counter {
                            step = step.fact("signature counter", counter.to_string());
                        }
                        ctx.record(step).await;
                        serde_json::to_value(PasskeyAuthResult {
                            verified: true,
                            detail: "Signature verified. The engine also advanced this \
                                     authenticator's counter, which is how cloned devices are \
                                     spotted."
                                .to_string(),
                            counter,
                        })
                        .map_err(|e| ApiError::Scenario(e.to_string()))
                    }
                    Err(e) => {
                        tracing::debug!(session_id = %ctx.session_id, error = %e, "passkey authentication rejected");
                        ctx.record(Step::rejected("passkeys", "signature rejected").detail(
                            "The signature did not verify. Either the challenge had already \
                             been answered, or the response did not come from a key enrolled \
                             for this session.",
                        ))
                        .await;
                        serde_json::to_value(PasskeyAuthResult {
                            verified: false,
                            detail: "That passkey could not be verified.".to_string(),
                            counter: None,
                        })
                        .map_err(|e| ApiError::Scenario(e.to_string()))
                    }
                }
            }

            other => Err(ApiError::UnknownAction {
                scenario: self.id().to_string(),
                action: other.to_string(),
            }),
        }
    }
}

/// The signature counter currently stored for this session's first passkey.
///
/// Read out of the serialised credential rather than through an accessor, so
/// this does not depend on `webauthn-rs` internals.
async fn persisted_counter(ctx: &ScenarioContext<'_>) -> Option<u32> {
    use authkestra_engine::auth::store::CredentialStore;
    let creds = ctx
        .credentials()
        .get_credentials(&ctx.user_id(), "webauthn")
        .await
        .ok()?;
    creds.first().and_then(|v| {
        v.pointer("/cred/counter")
            .or_else(|| v.pointer("/counter"))
            .and_then(|c| c.as_u64())
            .map(|c| c as u32)
    })
}

/// The browser's assertion, in the shape `navigator.credentials.get()` produces
/// once serialised. Deserialised explicitly so a malformed body is a 400 rather
/// than a panic deep in the engine.
#[derive(Debug, Deserialize)]
struct Assertion {
    id: String,
    response: AssertionResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssertionResponse {
    client_data_json: String,
    authenticator_data: String,
    signature: String,
    #[serde(default)]
    user_handle: Option<String>,
}
