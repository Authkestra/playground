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
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitEnvVar, KitFragment,
    KitLink, KitSetup, Scenario, ScenarioContext,
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

    let mut builder = builder.rp_name(&rp.name);
    for extra in &rp.extra_origins {
        let parsed = url::Url::parse(extra).map_err(|e| {
            ApiError::Scenario(format!(
                "WEBAUTHN_EXTRA_ORIGINS entry `{extra}` is not a valid URL: {e}"
            ))
        })?;
        builder = builder.append_allowed_origin(&parsed);
    }

    Ok(Arc::new(builder.build().map_err(|e| {
        ApiError::Scenario(format!("failed to build relying party: {e}"))
    })?))
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

    fn kit_fragment(&self, value: &ControlValue, _ctx: &KitContext<'_>) -> Option<KitFragment> {
        if !value.is_active() {
            return None;
        }
        Some(KitFragment {
            imports: vec![
                "use authkestra_engine::auth::store::CredentialStore;".to_string(),
                "use authkestra_engine::auth::webauthn::WebAuthnAuthMethod;".to_string(),
                "use authkestra_engine::auth::AuthInput;".to_string(),
                "use authkestra_engine::auth::AuthMethod;".to_string(),
                "use authkestra_engine::auth::WebAuthnStarter;".to_string(),
                "use axum::extract::State;".to_string(),
                "use axum::routing::post;".to_string(),
                "use std::collections::HashMap;".to_string(),
                "use std::sync::Mutex;".to_string(),
                "use std::time::{Duration, Instant};".to_string(),
                "use uuid::Uuid;".to_string(),
                "use webauthn_rs::prelude::Passkey;".to_string(),
                "use webauthn_rs::prelude::PasskeyRegistration;".to_string(),
                "use webauthn_rs::prelude::RegisterPublicKeyCredential;".to_string(),
                "use webauthn_rs::prelude::WebauthnBuilder;".to_string(),
            ],
            prelude: vec![
                r#"    // The relying party must match the origin the browser actually uses.
    // A mismatch fails inside the browser with a deliberately vague error, so
    // it is worth getting right first. The RP ID may be a registrable suffix
    // of the origin (`example.com` for `app.example.com`), which lets you move
    // between subdomains without invalidating every passkey.
    let rp_origin = std::env::var("WEBAUTHN_ORIGIN")
        .unwrap_or_else(|_| format!("http://localhost:{port}"));
    let rp_origin = url::Url::parse(&rp_origin).expect("WEBAUTHN_ORIGIN must be a URL");
    let rp_id = std::env::var("WEBAUTHN_RP_ID")
        .unwrap_or_else(|_| rp_origin.host_str().unwrap_or("localhost").to_string());
    // Built as an `Arc` up front: the engine takes one, and the handlers
    // below need one too.
    let webauthn = std::sync::Arc::new(
        WebauthnBuilder::new(&rp_id, &rp_origin)
        .expect("the RP ID must be a registrable suffix of the origin")
            .rp_name("Authkestra Starter")
            .build()
            .expect("valid WebAuthn configuration"),
    );"#
                .to_string(),
            ],
            builder_calls: vec![
                "        // Passkeys as a first factor. The private key never leaves the
        // authenticator; only its public key is stored here.
        .with_webauthn(webauthn.clone(), SqlxCredentialStore::new(pool.clone()))"
                    .to_string(),
            ],
            routes: vec![
                r#"        // Passkey enrolment and sign-in. The framework wires `/auth/login/*`
        // for OAuth redirects but nothing for WebAuthn, because the ceremony
        // is a conversation with the browser rather than a redirect.
        .route("/auth/passkey/register/start", post(passkey_register_start))
        .route("/auth/passkey/register/finish", post(passkey_register_finish))
        .route("/auth/passkey/login/start", post(passkey_login_start))
        .route("/auth/passkey/login/finish", post(passkey_login_finish))"#
                    .to_string(),
            ],
            handlers: vec![
                r##"/// A WebAuthn ceremony in flight, waiting for the browser's second request.
///
/// Held in memory on purpose: a starter kit should not need Redis to try a
/// passkey. The cost is real and worth knowing — ceremonies do not survive a
/// restart, and do not work at all across more than one instance. Move this
/// into your session store before you scale out.
#[derive(Clone, Default)]
struct Ceremonies(Arc<Mutex<HashMap<String, (Ceremony, Instant)>>>);

#[derive(Clone)]
struct Ceremony {
    /// Captured when the ceremony starts, so the second request cannot switch
    /// identity halfway through by sending a different username.
    user_id: String,
    state: String,
}

impl Ceremonies {
    /// Long enough to find your phone, short enough that an abandoned
    /// challenge is not left lying around.
    const TTL: Duration = Duration::from_secs(300);

    fn put(&self, user_id: String, state: String) -> String {
        let id = Uuid::new_v4().to_string();
        let mut map = self.0.lock().expect("ceremony lock poisoned");
        // Lazy expiry: cheap, and there is no sweeper to forget to start.
        map.retain(|_, (_, at)| at.elapsed() < Self::TTL);
        map.insert(id.clone(), (Ceremony { user_id, state }, Instant::now()));
        id
    }

    /// Removes as it reads. A challenge that can be answered twice is a replay.
    fn take(&self, id: &str) -> Option<Ceremony> {
        let mut map = self.0.lock().expect("ceremony lock poisoned");
        let (ceremony, at) = map.remove(id)?;
        (at.elapsed() < Self::TTL).then_some(ceremony)
    }
}

#[derive(serde::Deserialize)]
struct PasskeyStart {
    username: String,
}

#[derive(serde::Deserialize)]
struct PasskeyFinish {
    ceremony_id: String,
    /// The raw `PublicKeyCredential` from `navigator.credentials`, as JSON.
    credential: serde_json::Value,
}

fn passkey_method(state: &AppState) -> WebAuthnAuthMethod<SqlxCredentialStore<sqlx::Sqlite>> {
    WebAuthnAuthMethod::new(
        state.webauthn.clone(),
        SqlxCredentialStore::new(state.pool.clone()),
    )
}

/// Begin enrolment: hand the browser a challenge to sign.
async fn passkey_register_start(
    State(state): State<AppState>,
    Json(body): Json<PasskeyStart>,
) -> impl IntoResponse {
    let user_id = user_id_for(&body.username);

    match passkey_method(&state).start_register(&user_id, &body.username) {
        Ok((challenge, registration)) => {
            let stored = serde_json::to_string(&registration)
                .expect("webauthn registration state serialises");
            let ceremony_id = state.ceremonies.put(user_id, stored);
            (
                StatusCode::OK,
                Json(json!({ "ceremony_id": ceremony_id, "options": challenge })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not start passkey registration");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "could not start registration" })),
            )
                .into_response()
        }
    }
}

/// Finish enrolment: verify what the authenticator produced and store it.
async fn passkey_register_finish(
    State(state): State<AppState>,
    Json(body): Json<PasskeyFinish>,
) -> impl IntoResponse {
    // Parse before consuming the ceremony. Taking the challenge first would
    // mean a malformed body costs the visitor their challenge and forces them
    // to restart, over a mistake that never reached any cryptography.
    let credential: RegisterPublicKeyCredential = match serde_json::from_value(body.credential) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "malformed credential", "detail": e.to_string() })),
            )
                .into_response();
        }
    };

    let Some(ceremony) = state.ceremonies.take(&body.ceremony_id) else {
        return (
            StatusCode::GONE,
            Json(json!({ "error": "that challenge has expired or was already used" })),
        )
            .into_response();
    };

    let registration: PasskeyRegistration = match serde_json::from_str(&ceremony.state) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "stored ceremony state is unreadable");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "ceremony state is unreadable" })),
            )
                .into_response();
        }
    };

    match passkey_method(&state)
        .finish_register(&ceremony.user_id, credential, registration)
        .await
    {
        Ok(_) => (StatusCode::OK, Json(json!({ "enrolled": true }))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "enrolled": false, "detail": e.to_string() })),
        )
            .into_response(),
    }
}

/// Begin sign-in: challenge the passkeys this user has already enrolled.
async fn passkey_login_start(
    State(state): State<AppState>,
    Json(body): Json<PasskeyStart>,
) -> impl IntoResponse {
    let user_id = user_id_for(&body.username);

    let stored = match SqlxCredentialStore::new(state.pool.clone())
        .get_credentials(&user_id, "webauthn")
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "could not read stored passkeys");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "could not read credentials" })),
            )
                .into_response();
        }
    };

    let passkeys: Vec<Passkey> = stored
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();

    if passkeys.is_empty() {
        // Deliberately explicit. A real deployment may prefer to answer
        // identically whether or not the account exists, to avoid confirming
        // which usernames are enrolled.
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no passkey is enrolled for that username" })),
        )
            .into_response();
    }

    match passkey_method(&state).start_authentication(&passkeys) {
        Ok((challenge, authentication)) => {
            let stored = serde_json::to_string(&authentication)
                .expect("webauthn authentication state serialises");
            let ceremony_id = state.ceremonies.put(user_id, stored);
            (
                StatusCode::OK,
                Json(json!({ "ceremony_id": ceremony_id, "options": challenge })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not start passkey authentication");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "could not start authentication" })),
            )
                .into_response()
        }
    }
}

/// The browser's assertion, in the shape `navigator.credentials.get()` produces
/// once serialised. Deserialised explicitly so a malformed body is a 400 rather
/// than a panic deep in the engine, and because `AuthInput` wants base64url
/// strings rather than `webauthn-rs`'s decoded types.
#[derive(serde::Deserialize)]
struct Assertion {
    id: String,
    response: AssertionResponse,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssertionResponse {
    /// Spelled `clientDataJSON`, with `JSON` fully capitalised.
    ///
    /// This is a genuine quirk of the WebAuthn spec and the one field
    /// `rename_all = "camelCase"` gets wrong — it would produce
    /// `clientDataJson`, which no browser ever sends. Registration is
    /// unaffected because it deserialises into `webauthn-rs`'s own type, so
    /// only sign-in breaks, and it breaks on shape before any cryptography
    /// runs. Do not "tidy" this rename away.
    #[serde(rename = "clientDataJSON")]
    client_data_json: String,
    authenticator_data: String,
    signature: String,
    #[serde(default)]
    user_handle: Option<String>,
}

/// Finish sign-in: verify the signature and open a session.
async fn passkey_login_finish(
    State(state): State<AppState>,
    Json(body): Json<PasskeyFinish>,
) -> impl IntoResponse {
    let assertion: Assertion = match serde_json::from_value(body.credential) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "malformed credential", "detail": e.to_string() })),
            )
                .into_response();
        }
    };

    let Some(ceremony) = state.ceremonies.take(&body.ceremony_id) else {
        return (
            StatusCode::GONE,
            Json(json!({ "error": "that challenge has expired or was already used" })),
        )
            .into_response();
    };

    // Goes through `AuthMethod::authenticate` rather than
    // `finish_authentication` directly, because only this path advances and
    // persists the signature counter. A counter that fails to advance is how a
    // cloned authenticator is detected, so skipping it silently gives up the
    // guarantee.
    let result = passkey_method(&state)
        .authenticate(AuthInput::WebAuthnAuthentication {
            user_id: ceremony.user_id,
            credential_id: assertion.id,
            client_data_json: assertion.response.client_data_json,
            authenticator_data: assertion.response.authenticator_data,
            signature: assertion.response.signature,
            user_handle: assertion.response.user_handle,
            auth_state_json: Some(ceremony.state),
        })
        .await;

    let identity = match result {
        Ok(identity) => identity,
        Err(e) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "verified": false, "detail": e.to_string() })),
            )
                .into_response();
        }
    };

    match state.auth.create_session(identity).await {
        Ok(session) => (
            StatusCode::OK,
            Json(json!({ "verified": true, "session_id": session.id })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "signature verified but the session could not be created");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "could not create a session" })),
            )
                .into_response()
        }
    }
}"##
                .to_string(),
            ],
            state_fields: vec![r#"    /// The relying party, shared by every ceremony.
    webauthn: Arc<webauthn_rs::Webauthn>,
    ceremonies: Ceremonies,"#
                .to_string()],
            state_init: vec![r#"        webauthn: webauthn.clone(),
        ceremonies: Ceremonies::default(),"#
                .to_string()],
            crates: vec![
                CrateRequirement::new("uuid", &["v4", "v5"]),
                CrateRequirement::new("serde", &["derive"]),
            ],
            env: vec![
                KitEnvVar::with_default(
                    "WEBAUTHN_ORIGIN",
                    "The origin the browser loads your app from, exactly.",
                    "http://localhost:3000",
                ),
                KitEnvVar::with_default(
                    "WEBAUTHN_RP_ID",
                    "Relying-party ID. May be a registrable suffix of the origin.",
                    "localhost",
                ),
            ],
            notes: vec![
                "**Passkeys.** A passkey is bound to the relying-party ID that created it, so \
                 changing `WEBAUTHN_RP_ID` invalidates every existing registration. Prefer a \
                 registrable suffix (`example.com` over `app.example.com`) so you can move \
                 between subdomains later."
                    .to_string(),
                "The signature counter is stored alongside the credential and must be \
                 persisted: a counter that fails to advance is how a cloned authenticator is \
                 detected."
                    .to_string(),
                "The four ceremony routes are generated for you: \
                 `POST /auth/passkey/register/start` and `/register/finish` to enrol, \
                 `/login/start` and `/login/finish` to sign in. The framework wires \
                 `/auth/login/{provider}` for OAuth redirects but nothing for WebAuthn, \
                 because a passkey ceremony is a conversation with the browser rather \
                 than a redirect — so these are yours, and yours to change."
                    .to_string(),
                "Each pair parses the request body *before* consuming the stored \
                 challenge. Consuming first means a typo costs the visitor their \
                 challenge and forces them to restart, over a mistake that never \
                 reached any cryptography."
                    .to_string(),
            ],
            setup: vec![KitSetup::new(
                "Point WebAuthn at the origin you actually serve",
                &[
                    "Browsers only allow WebAuthn from `http://localhost` or an HTTPS \
                     origin. There is nothing to register with a third party — the \
                     origin is the trust anchor."
                        .to_string(),
                    "Set `WEBAUTHN_ORIGIN` to the origin exactly as the browser shows \
                     it, scheme and port included."
                        .to_string(),
                    "Set `WEBAUTHN_RP_ID` to that host, or to a registrable suffix of \
                     it. Every passkey already enrolled stops working if you change \
                     it later."
                        .to_string(),
                ],
            )],
            links: vec![
                KitLink::docs("Passkeys", "providers/passkeys"),
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
                // Parsed before the challenge is consumed. Taking it first
                // meant a malformed body burned the challenge and forced the
                // visitor to restart the whole ceremony for a mistake that
                // never reached any cryptography.
                let credential = serde_json::from_value(body).map_err(|e| {
                    ApiError::InvalidValue(format!("not a WebAuthn registration response: {e}"))
                })?;

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
                // Parsed before the challenge is consumed, so a malformed body
                // does not cost the visitor the ceremony.
                let assertion: Assertion = serde_json::from_value(body).map_err(|e| {
                    ApiError::InvalidValue(format!("not a WebAuthn assertion: {e}"))
                })?;

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
    /// Spelled `clientDataJSON`, with `JSON` fully capitalised.
    ///
    /// This is a genuine quirk of the WebAuthn spec and the one field
    /// `rename_all = "camelCase"` gets wrong — it would produce
    /// `clientDataJson`, which no browser ever sends. Registration was
    /// unaffected because it deserialises into `webauthn-rs`'s own type, so
    /// only authentication broke.
    #[serde(rename = "clientDataJSON")]
    client_data_json: String,
    authenticator_data: String,
    signature: String,
    #[serde(default)]
    user_handle: Option<String>,
}
