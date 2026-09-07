//! Resource-server scenario: validating a token at an API edge (roadmap P2).
//!
//! The other scenarios answer "how does someone sign in". This one answers the
//! question that comes immediately after, and that most auth libraries leave
//! entirely to the application: **how does an API decide whether to serve a
//! request?**
//!
//! It demos on its own terms with no second application involved — issue a
//! token, call a protected route with it, call the same route without it, and
//! watch the difference. That is why it comes before the OP server, which
//! needs a client to complete a flow against.
//!
//! ## On distinguishing failures
//!
//! A flat 401 teaches nothing. "Absent", "malformed", "expired" and "wrong
//! audience" are four different mistakes with four different fixes, and an API
//! that collapses them costs its callers an afternoon each.
//!
//! They are classified structurally rather than by matching on error strings:
//! `validate_token` returns `AuthError::Token(e.to_string())`, so a string
//! match would bind this scenario to the wording of a dependency's errors.
//! Instead the claims are read unverified first — only to *classify*, never to
//! decide — and the verdict always comes from `validate_token`.

use authkestra_engine::{Identity, TokenManager};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitEnvVar, KitFragment,
    KitLink, Scenario, ScenarioContext,
};
use crate::error::ApiError;
use crate::events::Step;

/// The audience every demo token is issued for, and the one the protected
/// route demands. A token for a different audience is a valid token being
/// presented to the wrong service, which is its own failure worth showing.
pub const DEMO_AUDIENCE: &str = "playground-api";

/// Short on purpose, so "expired" is reachable rather than theoretical.
///
/// Note what "reachable" costs: `validate_token` builds a `Validation` without
/// overriding `leeway`, and `jsonwebtoken` defaults that to **60 seconds** of
/// clock skew. So a token keeps being accepted for roughly a minute past its
/// `exp`, and the real wait is this plus that. Sixty rather than five so the
/// happy path is comfortable, and the flow log says what the wait is instead
/// of leaving someone clicking.
const TOKEN_TTL_SECS: u64 = 60;

/// `jsonwebtoken`'s default clock-skew allowance, which the engine does not
/// override. Recorded here because it is the difference between "expired" and
/// "still accepted", and it is invisible in the API.
pub const VALIDATION_LEEWAY_SECS: u64 = 60;

pub struct ResourceScenario;

/// What the flow log says about a freshly issued token.
///
/// Built from the constants rather than written out, because it *was* written
/// out and went stale: it claimed a five-minute life while `TOKEN_TTL_SECS`
/// said sixty seconds, with the contradicting `expires_in` fact attached to the
/// very same event. Wrong prose about expiry, in the one scenario whose subject
/// is expiry. Prose that restates a constant has to be generated from it.
fn issued_token_detail() -> String {
    format!(
        "Signed for this session only, with an `aud` of `{DEMO_AUDIENCE}` and a \
         {TOKEN_TTL_SECS}-second life. `jsonwebtoken` allows \
         {VALIDATION_LEEWAY_SECS}s of clock skew by default and the engine does not \
         override it, so it keeps being accepted for about {total}s in total — that \
         is the wait before `expired` shows.",
        total = TOKEN_TTL_SECS + VALIDATION_LEEWAY_SECS
    )
}

/// A token minted for the visitor's demo identity.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IssuedToken {
    pub token: String,
    /// Seconds until its `exp` passes.
    ///
    /// Not the same as when it stops being accepted: the validator allows 60
    /// seconds of clock skew on top. See `VALIDATION_LEEWAY_SECS`.
    pub expires_in: u32,
    /// What the protected route will demand.
    pub audience: String,
}

/// Why a request to the protected route was answered the way it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum TokenVerdict {
    /// Valid, in date, for this audience.
    Accepted,
    /// No `Authorization: Bearer` header at all.
    Absent,
    /// Present but not a JWT this service can even read.
    Malformed,
    /// Well-formed and correctly signed, but past its `exp`.
    Expired,
    /// Valid for a different service.
    WrongAudience,
    /// Signature does not verify — the usual cause is a different signing key.
    BadSignature,
}

impl TokenVerdict {
    /// The status a real API would answer with.
    pub fn status(self) -> u16 {
        match self {
            TokenVerdict::Accepted => 200,
            _ => 401,
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            TokenVerdict::Accepted => "The token is valid, in date, and for this audience.",
            TokenVerdict::Absent => {
                "No `Authorization: Bearer` header. The route is protected, so there is \
                 nothing to check."
            }
            TokenVerdict::Malformed => {
                "That is not a token this service can read. A JWT is three base64url \
                 segments separated by dots."
            }
            TokenVerdict::Expired => {
                "Correctly signed, but past its expiry. Issue a new one — do not extend \
                 the lifetime to make this go away."
            }
            TokenVerdict::WrongAudience => {
                "A valid token for a different service. The `aud` claim is what stops a \
                 token issued for one API being replayed against another."
            }
            TokenVerdict::BadSignature => {
                "The signature does not verify. Usually a token signed with a different \
                 key than this service validates with."
            }
        }
    }
}

/// The outcome of calling the protected route.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProtectedCall {
    pub verdict: TokenVerdict,
    pub status: u16,
    pub detail: String,
    /// The subject the token carried, when it was good enough to read one.
    pub subject: Option<String>,
}

/// Classify a bearer token without deciding anything on the classification.
///
/// The verdict for a *good* token always comes from `validate_token`. The
/// unverified read below exists only to tell four failures apart, and it is
/// never allowed to turn a rejection into an acceptance.
pub fn classify(manager: &TokenManager, bearer: Option<&str>) -> (TokenVerdict, Option<String>) {
    let Some(token) = bearer.map(str::trim).filter(|t| !t.is_empty()) else {
        return (TokenVerdict::Absent, None);
    };

    // Shape first. Anything that is not three segments is not a JWT, and the
    // decoder's message for that is not worth showing anyone.
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return (TokenVerdict::Malformed, None);
    }
    let Some(claims) = unverified_claims(parts[1]) else {
        return (TokenVerdict::Malformed, None);
    };
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .map(str::to_string);

    // The real check. Everything below only explains a failure it already had.
    if manager.validate_token(token, Some(DEMO_AUDIENCE)).is_ok() {
        return (TokenVerdict::Accepted, subject);
    }

    // Audience before expiry, deliberately. This branch means signature *and*
    // dates verified, so "valid for another service" is certain rather than
    // inferred, and it cannot be confused with a token that is merely stale.
    if manager.validate_token(token, None).is_ok() {
        return (TokenVerdict::WrongAudience, subject);
    }

    // Past here the token failed for its signature, its dates, or both. A
    // token can fail more than one check and only one answer can be given;
    // expiry wins because it is much the commoner cause and the cheapest to
    // act on. A forged token that is *also* stale reports as expired, which is
    // a fair trade for not telling someone to audit their signing keys when
    // their token was simply old.
    let past_expiry = claims
        .get("exp")
        .and_then(Value::as_u64)
        .is_some_and(|exp| exp + VALIDATION_LEEWAY_SECS < now_secs());
    if past_expiry {
        return (TokenVerdict::Expired, subject);
    }

    (TokenVerdict::BadSignature, subject)
}

/// Read a JWT payload without verifying it.
///
/// Only ever used to classify a token that has *already* been rejected, or to
/// name the subject on one that has already been accepted. Never to admit one.
fn unverified_claims(payload: &str) -> Option<Value> {
    let bytes = base64url_decode(payload)?;
    serde_json::from_slice(&bytes).ok()
}

fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let value = TABLE.iter().position(|c| *c == byte)? as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl ResourceScenario {
    /// The signing key for demo tokens.
    ///
    /// Per-session, so one visitor's token is worthless to another and the
    /// "bad signature" case is reachable by pasting someone else's token.
    pub fn token_manager(session_id: uuid::Uuid) -> TokenManager {
        TokenManager::new(
            session_id.as_bytes(),
            Some("https://play.authkestra.com".to_string()),
        )
    }
}

#[async_trait::async_trait]
impl Scenario for ResourceScenario {
    fn id(&self) -> &'static str {
        "resource"
    }

    fn name(&self) -> &'static str {
        "Protected API route (resource server)"
    }

    fn summary(&self) -> &'static str {
        "Validate a bearer token at your API edge, and tell the four ways it can fail apart."
    }

    fn control(&self) -> ControlShape {
        ControlShape::Toggle
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::Toggle { enabled: false }
    }

    fn actions(&self) -> Vec<&'static str> {
        vec!["issue", "call"]
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        if !value.is_active() {
            return Consequences::default();
        }

        Consequences {
            routes: vec![
                "POST /api/protected".to_string(),
                "GET /api/protected".to_string(),
            ],
            requirements: vec![
                "Issue tokens with an `aud` claim naming the service that should accept them."
                    .to_string(),
                "Decide the lifetime deliberately: short enough to limit a leak, long \
                 enough that clients are not refreshing constantly."
                    .to_string(),
            ],
            crates: vec![
                CrateRequirement::new("authkestra-engine", &["token"]),
                CrateRequirement::new("authkestra-axum", &["token"]),
            ],
        }
    }

    async fn action(
        &self,
        action: &str,
        body: Value,
        ctx: &ScenarioContext<'_>,
    ) -> Result<Value, ApiError> {
        if !ctx.value.is_active() {
            return Err(ApiError::Scenario(
                "Turn the protected route on first.".to_string(),
            ));
        }

        let manager = Self::token_manager(ctx.session_id);

        match action {
            "issue" => {
                let identity = Identity {
                    provider_id: "playground".to_string(),
                    external_id: ctx.user_id(),
                    email: None,
                    username: Some("demo-visitor".to_string()),
                    attributes: std::collections::HashMap::new(),
                };

                let token = manager
                    .issue_user_token(
                        identity,
                        TOKEN_TTL_SECS,
                        Some("demo".to_string()),
                        Some(DEMO_AUDIENCE.to_string()),
                    )
                    .map_err(|e| ApiError::Scenario(e.to_string()))?;

                ctx.record(
                    Step::success("resource", "token issued")
                        .detail(issued_token_detail())
                        .fact("audience", DEMO_AUDIENCE)
                        .fact("expires_in", TOKEN_TTL_SECS.to_string()),
                )
                .await;

                Ok(serde_json::to_value(IssuedToken {
                    token,
                    expires_in: TOKEN_TTL_SECS as u32,
                    audience: DEMO_AUDIENCE.to_string(),
                })
                .expect("token serialises"))
            }

            "call" => {
                let bearer = body.get("token").and_then(Value::as_str);
                let (verdict, subject) = classify(&manager, bearer);

                let step = match verdict {
                    TokenVerdict::Accepted => Step::success("resource", "request served"),
                    _ => Step::rejected("resource", "request refused"),
                };
                ctx.record(
                    step.detail(verdict.detail())
                        .fact("verdict", format!("{verdict:?}"))
                        .fact("status", verdict.status().to_string()),
                )
                .await;

                Ok(serde_json::to_value(ProtectedCall {
                    verdict,
                    status: verdict.status(),
                    detail: verdict.detail().to_string(),
                    subject,
                })
                .expect("outcome serialises"))
            }

            other => Err(ApiError::UnknownAction {
                scenario: "resource".to_string(),
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
                "use authkestra_axum::AuthToken;".to_string(),
                "use axum::extract::State;".to_string(),
                "use axum::routing::post;".to_string(),
            ],
            prelude: vec![
                r#"    // The signing key for API tokens. A fixed secret in an environment
    // variable is the simplest thing that works; rotate it by rotating the
    // variable, and every token issued under the old one stops validating.
    let token_secret = std::env::var("TOKEN_SECRET")
        .unwrap_or_else(|_| "change-me-in-production".to_string());
    let token_manager = std::sync::Arc::new(authkestra_engine::TokenManager::new(
        token_secret.as_bytes(),
        Some(std::env::var("TOKEN_ISSUER").unwrap_or_else(|_| format!("http://localhost:{port}"))),
    ));"#
                    .to_string(),
            ],
            builder_calls: vec![
                "        // A token manager moves the typestate, like the session store:
        // token methods do not exist on the engine until it is supplied.
        .token_manager(token_manager.clone())"
                    .to_string(),
            ],
            routes: vec![
                r#"        // Issue a token, then present it to the protected route.
        .route("/api/token", post(issue_token))
        .route("/api/protected", get(protected))"#
                    .to_string(),
            ],
            handlers: vec![PROTECTED_HANDLERS.to_string()],
            state_fields: vec!["    /// Issues the tokens the protected route validates.\n\
                 \x20   tokens: std::sync::Arc<authkestra_engine::TokenManager>,"
                .to_string()],
            state_init: vec!["        tokens: token_manager.clone(),".to_string()],
            openapi_paths: Vec::new(),
            openapi_schemas: Vec::new(),
            crates: vec![CrateRequirement::new("serde", &["derive"])],
            env: vec![
                KitEnvVar::with_default(
                    "TOKEN_SECRET",
                    "Signing key for API tokens. Rotating it invalidates every issued token.",
                    "change-me-in-production",
                ),
                KitEnvVar::with_default(
                    "TOKEN_AUDIENCE",
                    "What the protected route demands in a token's `aud` claim.",
                    "my-api",
                ),
            ],
            notes: vec![
                "**Protected route.** `AuthToken` is an extractor: reaching for it in a \
                 handler's arguments is what makes the route require a valid bearer token, \
                 and a request without one never reaches your code."
                    .to_string(),
                "The `aud` claim is checked, not just the signature. Without it a token \
                 issued for one of your services is accepted by all of them, which is the \
                 whole reason the claim exists."
                    .to_string(),
            ],
            setup: Vec::new(),
            links: vec![
                KitLink::docs("Resource server", "advanced/resource-server"),
                KitLink::example("crates/authkestra/examples/axum_resource_server.rs"),
            ],
            needs_credential_store: false,
        })
    }
}

/// The generated handlers for the protected route.
const PROTECTED_HANDLERS: &str = r##"#[derive(serde::Deserialize)]
struct IssueRequest {
    /// Who the token is for. Your application decides this; here it is taken
    /// at face value because there is nobody to check it against.
    subject: String,
}

/// Mint a token for a subject.
///
/// A real service would do this only after authenticating the caller — this
/// route is the part you replace first.
async fn issue_token(
    State(state): State<AppState>,
    Json(body): Json<IssueRequest>,
) -> impl IntoResponse {
    let audience = std::env::var("TOKEN_AUDIENCE").unwrap_or_else(|_| "my-api".to_string());

    let identity = authkestra_engine::Identity {
        provider_id: "local".to_string(),
        external_id: body.subject,
        email: None,
        username: None,
        attributes: std::collections::HashMap::new(),
    };

    match state.tokens.issue_user_token(identity, 300, None, Some(audience)) {
        Ok(token) => (
            StatusCode::OK,
            Json(json!({ "token": token, "expires_in": 300 })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "could not issue a token");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "could not issue a token" })),
            )
                .into_response()
        }
    }
}

/// A route that requires a valid bearer token.
///
/// The protection is the `AuthToken` extractor in the arguments: a request
/// without a valid token is rejected before this function runs, so there is no
/// check to forget to write.
async fn protected(AuthToken(claims): AuthToken) -> impl IntoResponse {
    Json(json!({
        "subject": claims.sub,
        "audience": claims.aud,
        "expires_at": claims.exp,
    }))
}"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> TokenManager {
        TokenManager::new(b"a-test-signing-key", Some("test-issuer".to_string()))
    }

    fn identity() -> Identity {
        Identity {
            provider_id: "playground".to_string(),
            external_id: "visitor-1".to_string(),
            email: None,
            username: Some("demo-visitor".to_string()),
            attributes: std::collections::HashMap::new(),
        }
    }

    fn issue(m: &TokenManager, ttl: u64, aud: Option<&str>) -> String {
        m.issue_user_token(identity(), ttl, None, aud.map(str::to_string))
            .expect("token issues")
    }

    #[test]
    fn a_good_token_is_accepted_and_names_its_subject() {
        let m = manager();
        let token = issue(&m, 300, Some(DEMO_AUDIENCE));

        let (verdict, subject) = classify(&m, Some(&token));
        assert_eq!(verdict, TokenVerdict::Accepted);
        assert_eq!(subject.as_deref(), Some("visitor-1"));
        assert_eq!(verdict.status(), 200);
    }

    #[test]
    fn no_header_is_absent_rather_than_malformed() {
        let m = manager();
        assert_eq!(classify(&m, None).0, TokenVerdict::Absent);
        // An empty or whitespace-only header is the same mistake, not a
        // different one.
        assert_eq!(classify(&m, Some("")).0, TokenVerdict::Absent);
        assert_eq!(classify(&m, Some("   ")).0, TokenVerdict::Absent);
    }

    #[test]
    fn something_that_is_not_a_jwt_is_malformed() {
        let m = manager();
        for junk in ["hello", "a.b", "a.b.c.d", "not.a.jwt"] {
            assert_eq!(
                classify(&m, Some(junk)).0,
                TokenVerdict::Malformed,
                "{junk} should be malformed"
            );
        }
    }

    /// A token one second past `exp` is still accepted, and that is correct.
    ///
    /// `jsonwebtoken` allows 60 seconds of clock skew by default and the
    /// engine does not override it. Worth pinning: it is invisible in the API,
    /// it surprised this scenario during development, and anyone timing a
    /// token-expiry test against this service needs to know.
    #[test]
    fn a_token_just_past_expiry_is_still_within_the_clock_skew_allowance() {
        let m = manager();
        let token = issue(&m, 0, Some(DEMO_AUDIENCE));
        std::thread::sleep(std::time::Duration::from_millis(1100));

        assert_eq!(
            classify(&m, Some(&token)).0,
            TokenVerdict::Accepted,
            "within {VALIDATION_LEEWAY_SECS}s of expiry a token is still valid"
        );
    }

    /// The real expiry path. Ignored because it can only be observed after the
    /// leeway elapses, and a minute is too long for the default suite — but it
    /// is a real assertion, not a placeholder.
    ///
    ///     cargo test -p api --lib expired -- --ignored
    #[test]
    #[ignore = "waits out the 60s clock-skew allowance"]
    fn an_expired_token_is_reported_as_expired() {
        let m = manager();
        let token = issue(&m, 0, Some(DEMO_AUDIENCE));
        std::thread::sleep(std::time::Duration::from_secs(VALIDATION_LEEWAY_SECS + 2));

        let (verdict, subject) = classify(&m, Some(&token));
        assert_eq!(verdict, TokenVerdict::Expired);
        assert_eq!(subject.as_deref(), Some("visitor-1"));
    }

    #[test]
    fn a_token_for_another_service_is_a_wrong_audience() {
        let m = manager();
        let token = issue(&m, 300, Some("somebody-elses-api"));

        assert_eq!(classify(&m, Some(&token)).0, TokenVerdict::WrongAudience);
    }

    /// A token signed by someone else's key. Reachable in the demo by pasting
    /// another session's token, since the signing key is per-session.
    #[test]
    fn a_token_from_a_different_key_fails_on_signature() {
        let mine = manager();
        let theirs = TokenManager::new(b"a-different-signing-key", Some("test-issuer".to_string()));
        let token = issue(&theirs, 300, Some(DEMO_AUDIENCE));

        assert_eq!(classify(&mine, Some(&token)).0, TokenVerdict::BadSignature);
    }

    /// Classification reads the claims without verifying them, which would be
    /// a hole if it could ever admit a token. It must not: every path to
    /// `Accepted` goes through `validate_token`.
    #[test]
    fn the_unverified_read_can_never_admit_a_token() {
        let mine = manager();
        let theirs = TokenManager::new(b"a-different-signing-key", None);

        // A forged token whose claims look perfect: right audience, far future
        // expiry, plausible subject.
        let forged = theirs
            .issue_user_token(identity(), 86_400, None, Some(DEMO_AUDIENCE.to_string()))
            .expect("token issues");

        let (verdict, _) = classify(&mine, Some(&forged));
        assert_ne!(verdict, TokenVerdict::Accepted);
        assert_eq!(verdict.status(), 401);
    }

    /// The regression. The detail is visitor-facing text about expiry in the
    /// scenario about expiry, so it must agree with the constants that decide
    /// it — and it did not.
    #[test]
    fn the_issued_token_detail_agrees_with_the_constants() {
        let detail = issued_token_detail();

        for expected in [
            TOKEN_TTL_SECS.to_string(),
            VALIDATION_LEEWAY_SECS.to_string(),
            (TOKEN_TTL_SECS + VALIDATION_LEEWAY_SECS).to_string(),
        ] {
            assert!(
                detail.contains(&expected),
                "the detail should name {expected}: {detail}"
            );
        }
        assert!(detail.contains(DEMO_AUDIENCE), "{detail}");
        // The stale wording, and any other unit that is not what the constants
        // are measured in.
        assert!(
            !detail.contains("minute"),
            "the TTL is in seconds; saying minutes is how this went wrong: {detail}"
        );
    }

    #[test]
    fn every_rejection_explains_itself_distinctly() {
        let details: Vec<&str> = [
            TokenVerdict::Absent,
            TokenVerdict::Malformed,
            TokenVerdict::Expired,
            TokenVerdict::WrongAudience,
            TokenVerdict::BadSignature,
        ]
        .iter()
        .map(|v| v.detail())
        .collect();

        for (i, a) in details.iter().enumerate() {
            assert!(a.len() > 40, "a flat message teaches nothing: {a}");
            for b in details.iter().skip(i + 1) {
                assert_ne!(a, b, "two failures share an explanation");
            }
        }
    }
}
