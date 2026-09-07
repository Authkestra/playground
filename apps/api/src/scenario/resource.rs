//! Resource-server scenario: validating a token by **key discovery** (#52).
//!
//! The question this answers is the one that comes immediately after "how does
//! someone sign in": how does an API decide whether to serve a request, for a
//! token it did not issue and holds no secret for?
//!
//! ## What changed, and why the previous version was not enough
//!
//! This scenario used to sign with a per-session HMAC secret and validate with
//! the same secret in the same process. That proves a 401 can be produced. It
//! does not demonstrate a resource server, because nothing is discovered and
//! the validating side holds the key that minted the token — so the only thing
//! being tested is that a function agrees with itself.
//!
//! Now the deployment signs **EdDSA** and publishes its public key at
//! `/.well-known/jwks.json` (see [`crate::signing`]). Validation goes through
//! `authkestra-resource`: the token's `kid` is looked up in a JWKS fetched over
//! HTTP, against an `IssuerTrustMap` that decides which issuers are allowed a
//! key at all. The validating half holds **no secret** — only a URL and a name
//! it trusts. That is the capability, and it is what the original issue asked
//! for.
//!
//! ## Verdicts come from the type system, not from error strings
//!
//! `ValidationError` is an enum and `jsonwebtoken::ErrorKind` is matchable, so
//! every verdict below is decided structurally. The previous version had to
//! validate twice — once with an audience, once without — to tell "wrong
//! audience" from "expired", because all it had was a stringified error. That
//! trick is gone.
//!
//! ## No clock-skew wait
//!
//! `jsonwebtoken` allows 60 seconds of skew by default, which meant an expired
//! token kept being accepted for a minute and "expired" took two minutes to
//! reach. The `Validation` here sets `leeway` to zero, deliberately: skew
//! tolerance is right for a real deployment and wrong for a demonstration,
//! where the whole point is that the boundary is observable.

use std::sync::Arc;
use std::time::Duration;

use authkestra_engine::token::Claims;
use authkestra_engine::Identity;
use authkestra_resource::jwt::{
    validate_jwt_with_resolver, IssuerTrustMap, JwksCache, ValidationError,
};
use jsonwebtoken::{Algorithm, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitEnvVar, KitFragment,
    KitLink, Scenario, ScenarioContext,
};
use crate::error::ApiError;
use crate::events::Step;
use crate::signing::SigningKeys;

/// The audience every demo token is issued for, and the one the protected route
/// demands. A token for a different audience is a valid token presented to the
/// wrong service, which is its own failure worth showing.
pub const DEMO_AUDIENCE: &str = "playground-api";

/// An audience belonging to nobody, for the wrong-audience demonstration.
const OTHER_AUDIENCE: &str = "somebody-elses-api";

/// An issuer this deployment does not trust, for the untrusted-issuer
/// demonstration. Deliberately plausible-looking: the lesson is that a
/// well-formed, correctly signed token from an unlisted issuer gets nothing.
const FOREIGN_ISSUER: &str = "https://issuer.example.com";

/// Short, so "expired" is reachable rather than theoretical. With `leeway` at
/// zero this is the whole wait, not the first half of it.
const TOKEN_TTL_SECS: u64 = 60;

/// How far in the past the expired demonstration's `exp` is placed.
///
/// Comfortably more than a second, so the outcome never depends on how long
/// the round trip took.
const EXPIRED_BY_SECS: i64 = 120;

/// How long a fetched key set is reused before the validator re-fetches.
///
/// Short for a demo, because a visitor who restarts the API and watches the
/// resource server pick up the new key is seeing the actual point of
/// discovery. A real deployment would use minutes.
const JWKS_REFRESH: Duration = Duration::from_secs(30);

/// What the flow log says about a freshly issued token.
///
/// Built from the constants rather than written out, because it *was* written
/// out and went stale: it claimed a five-minute life while the TTL said sixty
/// seconds. Prose that restates a constant has to be generated from it.
fn issued_token_detail(kid: &str, issuer: &str) -> String {
    format!(
        "Signed with `{kid}` for `{issuer}`, audience `{DEMO_AUDIENCE}`, \
         {TOKEN_TTL_SECS}-second life. The protected route will look that `kid` up in \
         the published key set — it holds no secret of its own."
    )
}

// ------------------------------------------------------------------- verdicts

/// Why a request to the protected route was answered the way it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum TokenVerdict {
    /// Valid, in date, for this audience, signed by a discoverable key.
    Accepted,
    /// No `Authorization: Bearer` header at all.
    Absent,
    /// Not a JWT this service can even read.
    Malformed,
    /// No `kid` header, which the strict policy here requires.
    MissingKid,
    /// A `kid` the issuer's published key set does not contain.
    UnknownKid,
    /// An `iss` this deployment trusts no keys for.
    UntrustedIssuer,
    /// A trusted issuer's key, but an `iss` claim the validator rejects.
    WrongIssuer,
    /// Valid for a different service.
    WrongAudience,
    /// Correctly signed, past its expiry.
    Expired,
    /// The named key exists, but it did not sign this token.
    BadSignature,
    /// The key set could not be fetched, so nothing could be decided.
    KeysUnreachable,
    /// Refused for a reason this scenario has no specific name for.
    Rejected,
}

impl TokenVerdict {
    /// The status a real API would answer with.
    pub fn status(self) -> u16 {
        match self {
            TokenVerdict::Accepted => 200,
            // Not 401: nothing was decided about the credential, so claiming it
            // was rejected would be a lie. A resource server that cannot reach
            // its issuer's keys is broken, not being attacked — and a client
            // that retries on 503 and gives up on 401 will then do the right
            // thing with a token that was fine all along.
            TokenVerdict::KeysUnreachable => 503,
            _ => 401,
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            TokenVerdict::Accepted => {
                "The `kid` was found in the issuer's published key set, the signature \
                 verified against that key, and the token is in date and for this audience."
            }
            TokenVerdict::Absent => {
                "No `Authorization: Bearer` header. The route is protected, so there is \
                 nothing to check."
            }
            TokenVerdict::Malformed => {
                "That is not a token this service can read. A JWT is three base64url \
                 segments separated by dots."
            }
            TokenVerdict::MissingKid => {
                "No `kid` in the header, so there is no way to say which key should have \
                 signed this. Accepting it would mean trying every key the issuer \
                 publishes, which is how a retired key keeps working."
            }
            TokenVerdict::UnknownKid => {
                "The `kid` is not in the issuer's key set — and the validator re-fetched \
                 to be sure, in case of a rotation. Fetch the key set yourself and look: \
                 this is a signature by a key nobody published."
            }
            TokenVerdict::UntrustedIssuer => {
                "The `iss` is not in the trust map, so no key set is trusted for it. \
                 Correctly signed by somebody — just not by anybody this service was told \
                 to believe. There is deliberately no fallback to a default key."
            }
            TokenVerdict::WrongIssuer => {
                "A key this service trusts, but an `iss` claim it will not accept. The \
                 resolver picks the key; the validation rules decide which names are \
                 allowed, and both have to agree."
            }
            TokenVerdict::WrongAudience => {
                "A valid token for a different service. The `aud` claim is what stops a \
                 token issued for one API being replayed against another."
            }
            TokenVerdict::Expired => {
                "Correctly signed, but past its expiry. Issue a new one — do not extend \
                 the lifetime to make this go away."
            }
            TokenVerdict::BadSignature => {
                "The named key was found, and it did not sign this. Either the token was \
                 tampered with after signing, or something signed it while claiming \
                 somebody else's `kid`."
            }
            TokenVerdict::KeysUnreachable => {
                "The issuer's key set could not be fetched, so nothing could be decided \
                 about this token. Refused rather than accepted: a check that did not \
                 complete has not passed."
            }
            TokenVerdict::Rejected => {
                "The validator refused it. The reason it gave is below — this scenario \
                 has no shorter name for that one."
            }
        }
    }
}

/// Turn what the validator returned into a verdict.
///
/// Pure, and separate from the fetch, so every branch is testable without a
/// network. The classification is structural throughout: a `ValidationError`
/// variant or a `jsonwebtoken::ErrorKind`, never a parsed message.
fn classify(outcome: Result<Claims, ValidationError>) -> (TokenVerdict, Option<String>, String) {
    use jsonwebtoken::errors::ErrorKind;

    match outcome {
        Ok(claims) => (
            TokenVerdict::Accepted,
            Some(claims.sub),
            TokenVerdict::Accepted.detail().to_string(),
        ),
        Err(e) => {
            let verdict = match &e {
                ValidationError::MissingKid => TokenVerdict::MissingKid,
                ValidationError::KeyNotFound => TokenVerdict::UnknownKid,
                ValidationError::UntrustedIssuer(_) | ValidationError::MissingIssuer => {
                    TokenVerdict::UntrustedIssuer
                }
                // Fail closed, and say which way. An unreachable key set is
                // never "the credential is bad" — it is "we could not tell".
                ValidationError::Http(_) => TokenVerdict::KeysUnreachable,
                ValidationError::InvalidToken(_) => TokenVerdict::Malformed,
                ValidationError::Jwt(jwt) => match jwt.kind() {
                    ErrorKind::ExpiredSignature => TokenVerdict::Expired,
                    ErrorKind::InvalidAudience => TokenVerdict::WrongAudience,
                    ErrorKind::InvalidIssuer => TokenVerdict::WrongIssuer,
                    ErrorKind::InvalidSignature => TokenVerdict::BadSignature,
                    ErrorKind::InvalidToken
                    | ErrorKind::InvalidKeyFormat
                    | ErrorKind::Base64(_)
                    | ErrorKind::Json(_)
                    | ErrorKind::Utf8(_) => TokenVerdict::Malformed,
                    // `ErrorKind` is non-exhaustive, and mapping an unknown
                    // kind onto a specific verdict would mislabel it.
                    _ => TokenVerdict::Rejected,
                },
                // `ValidationError` is non-exhaustive too.
                _ => TokenVerdict::Rejected,
            };

            // The validator's own words, kept alongside our name for the
            // outcome. Neither replaces the other: the verdict is what we
            // decided, the message is what we decided it from.
            let detail = format!("{}\n\n{e}", verdict.detail());
            (verdict, None, detail)
        }
    }
}

// ---------------------------------------------------------------- wire types

/// A token minted for the visitor's demo identity.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IssuedToken {
    pub token: String,
    /// Seconds until its `exp` passes. There is no skew allowance here, so
    /// this is the whole life.
    pub expires_in: u32,
    /// What the protected route demands in `aud`.
    pub audience: String,
    /// The `iss` it carries, and the name the validator has to trust.
    pub issuer: String,
    /// The key that signed it. Look for this in the published key set.
    pub kid: Option<String>,
    /// Where the key set is published, so the `kid` can be checked by hand.
    pub jwks_url: String,
    /// Whether this token was minted to fail, and how.
    ///
    /// `None` for an honest token. The panel shows it so a rejection is never
    /// mistaken for a bug in the happy path.
    pub forged_as: Option<String>,
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

/// A deliberately-invalid token a visitor can ask for.
///
/// Each is a *real* token, signed for real, that fails for one specific
/// structural reason — not a canned string. A visitor can fetch the key set and
/// confirm for themselves why each one gets nothing, which is the difference
/// between a demonstration and an assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Forgery {
    /// Signed by a key that was never published, under its own `kid`.
    UnknownKid,
    /// Signed by an unpublished key that *claims* the real `kid`.
    BadSignature,
    /// Correctly signed, by an issuer the trust map does not list.
    UntrustedIssuer,
    /// Valid, for another service's audience.
    WrongAudience,
    /// Valid, already expired.
    Expired,
    /// Signed with no `kid` header at all.
    MissingKid,
}

impl Forgery {
    /// What this forgery is meant to prove, in the visitor's terms.
    pub fn label(self) -> &'static str {
        match self {
            Forgery::UnknownKid => "signed by an unpublished key",
            Forgery::BadSignature => "claims a real `kid` it did not sign with",
            Forgery::UntrustedIssuer => "from an issuer that is not trusted",
            Forgery::WrongAudience => "issued for another service",
            Forgery::Expired => "already expired",
            Forgery::MissingKid => "carries no `kid` header",
        }
    }

    /// The verdict it should produce, which is also what the tests assert.
    pub fn expected(self) -> TokenVerdict {
        match self {
            Forgery::UnknownKid => TokenVerdict::UnknownKid,
            Forgery::BadSignature => TokenVerdict::BadSignature,
            Forgery::UntrustedIssuer => TokenVerdict::UntrustedIssuer,
            Forgery::WrongAudience => TokenVerdict::WrongAudience,
            Forgery::Expired => TokenVerdict::Expired,
            Forgery::MissingKid => TokenVerdict::MissingKid,
        }
    }

    pub const ALL: [Forgery; 6] = [
        Forgery::UnknownKid,
        Forgery::BadSignature,
        Forgery::UntrustedIssuer,
        Forgery::WrongAudience,
        Forgery::Expired,
        Forgery::MissingKid,
    ];
}

/// A throwaway key for minting tokens that are *meant* to fail.
///
/// Not a [`SigningKeys`]: these are not signing identities, they are never
/// published, and nothing should be able to mistake one for the deployment's
/// own. Kept here rather than in `signing.rs` for that reason — forging is the
/// scenario's business.
///
/// It signs with `jsonwebtoken` directly rather than through `TokenManager`,
/// because `TokenManager` always puts a `kid` in the header (inventing one when
/// not given it) and the missing-`kid` case is precisely a header without one.
struct RogueKey {
    encoding: jsonwebtoken::EncodingKey,
    /// What the header will claim. `None` emits no `kid` at all.
    kid: Option<String>,
}

impl RogueKey {
    /// A fresh key claiming `kid`.
    fn generate(kid: Option<String>) -> Result<Self, ApiError> {
        let pem = crate::signing::generate_ed25519_pem();
        let encoding = jsonwebtoken::EncodingKey::from_ed_pem(pem.as_bytes())
            .map_err(|e| ApiError::Scenario(format!("could not build a forging key: {e}")))?;
        Ok(Self { encoding, kid })
    }

    fn sign(&self, claims: &Value) -> Result<String, ApiError> {
        let mut header = jsonwebtoken::Header::new(Algorithm::EdDSA);
        header.kid = self.kid.clone();
        jsonwebtoken::encode(&header, claims, &self.encoding)
            .map_err(|e| ApiError::Scenario(format!("could not sign a forged token: {e}")))
    }
}

/// Claims for a forged token: well-formed, plausible, and in date.
///
/// Everything about these is correct except the one thing under test. A forgery
/// that failed for two reasons at once would prove neither.
///
/// Built as JSON rather than as `Claims`, which is `#[non_exhaustive]` and so
/// cannot be constructed from outside the engine. That is no loss: an attacker
/// writes a claim set, not somebody else's struct, and `jsonwebtoken::encode`
/// signs anything `Serialize`.
fn forged_claims(subject: &str, issuer: &str, audience: &str, ttl_secs: i64) -> Value {
    let now = chrono::Utc::now().timestamp();
    serde_json::json!({
        "iss": issuer,
        "sub": subject,
        "aud": audience,
        "exp": (now + ttl_secs).max(0),
        "iat": now,
        "nbf": now,
        "jti": uuid::Uuid::new_v4().to_string(),
    })
}

// --------------------------------------------------------------- the scenario

/// Validates tokens by discovery, holding no signing key.
///
/// Built once and shared: `JwksCache` is the thing that makes discovery
/// affordable, and rebuilding it per request would fetch the key set every
/// time — turning a cache into a stampede against the issuer.
pub struct TokenValidator {
    resolver: IssuerTrustMap,
    trusted_issuer: String,
}

impl TokenValidator {
    pub fn new(trusted_issuer: String, jwks_url: String) -> Self {
        let cache = Arc::new(JwksCache::new(jwks_url, JWKS_REFRESH).require_kid(true));
        Self {
            resolver: IssuerTrustMap::new().with_issuer(trusted_issuer.clone(), cache),
            trusted_issuer,
        }
    }

    /// The rules a token is held to, quite apart from which key signed it.
    ///
    /// The resolver decides *which key*; this decides *which names are
    /// acceptable*. Both have to agree, which is why the issuer appears twice.
    fn validation(&self) -> Validation {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[DEMO_AUDIENCE]);
        validation.set_issuer(&[&self.trusted_issuer]);
        // Zero, deliberately — see the module docs. Skew tolerance is right
        // for a deployment and wrong for a demonstration.
        validation.leeway = 0;
        validation
    }

    pub async fn validate(&self, token: &str) -> Result<Claims, ValidationError> {
        validate_jwt_with_resolver::<Claims>(token, &self.resolver, &self.validation()).await
    }
}

pub struct ResourceScenario {
    signing: Arc<SigningKeys>,
    validator: TokenValidator,
}

impl ResourceScenario {
    pub fn new(signing: Arc<SigningKeys>) -> Self {
        let validator =
            TokenValidator::new(signing.issuer().to_string(), signing.jwks_url().to_string());
        Self { signing, validator }
    }

    fn identity(&self, user_id: String) -> Identity {
        Identity {
            provider_id: "playground".to_string(),
            external_id: user_id,
            email: None,
            username: Some("demo-visitor".to_string()),
            attributes: std::collections::HashMap::new(),
        }
    }

    fn describe(
        &self,
        token: String,
        expires_in: u64,
        audience: &str,
        issuer: &str,
        kid: Option<String>,
        forged_as: Option<&'static str>,
    ) -> IssuedToken {
        IssuedToken {
            token,
            expires_in: expires_in as u32,
            audience: audience.to_string(),
            issuer: issuer.to_string(),
            kid,
            jwks_url: self.signing.jwks_url().to_string(),
            forged_as: forged_as.map(str::to_string),
        }
    }

    /// Mint an honest token with the deployment's published key.
    fn issue_honest(&self, user_id: String) -> Result<IssuedToken, ApiError> {
        let token = self
            .signing
            .manager()
            .issue_user_token(
                self.identity(user_id),
                TOKEN_TTL_SECS,
                Some("demo".to_string()),
                Some(DEMO_AUDIENCE.to_string()),
            )
            .map_err(|e| ApiError::Scenario(e.to_string()))?;

        Ok(self.describe(
            token,
            TOKEN_TTL_SECS,
            DEMO_AUDIENCE,
            self.signing.issuer(),
            self.signing.kid().map(str::to_string),
            None,
        ))
    }

    /// Mint a token designed to fail one specific check.
    ///
    /// Every one is really signed, and every one is well-formed and in date
    /// except where that is the point. Four use a key generated on the spot and
    /// never published, which is what makes "fetch the key set and look" a real
    /// answer rather than a claim.
    fn issue_forged(&self, user_id: String, forgery: Forgery) -> Result<IssuedToken, ApiError> {
        let real_kid = self.signing.kid().map(str::to_string);
        let issuer = self.signing.issuer();
        let ttl = TOKEN_TTL_SECS as i64;

        let (token, claimed_issuer, audience, kid, expires_in) = match forgery {
            // Signed by a key nobody published, under its own name. The issuer
            // is trusted, so the key set really is fetched and really does not
            // contain this `kid`.
            Forgery::UnknownKid => {
                let kid = format!("unpublished-{}", uuid::Uuid::new_v4());
                let rogue = RogueKey::generate(Some(kid.clone()))?;
                let claims = forged_claims(&user_id, issuer, DEMO_AUDIENCE, ttl);
                (rogue.sign(&claims)?, issuer, DEMO_AUDIENCE, Some(kid), ttl)
            }

            // Claims the *published* `kid`, so the lookup succeeds and the
            // signature is what fails. A different lesson from a key nobody
            // has heard of, and the one people conflate.
            Forgery::BadSignature => {
                let rogue = RogueKey::generate(real_kid.clone())?;
                let claims = forged_claims(&user_id, issuer, DEMO_AUDIENCE, ttl);
                (
                    rogue.sign(&claims)?,
                    issuer,
                    DEMO_AUDIENCE,
                    real_kid.clone(),
                    ttl,
                )
            }

            // Refused before any key is looked up: the trust map has no entry
            // for this `iss`, and deliberately no fallback.
            Forgery::UntrustedIssuer => {
                let kid = format!("foreign-{}", uuid::Uuid::new_v4());
                let rogue = RogueKey::generate(Some(kid.clone()))?;
                let claims = forged_claims(&user_id, FOREIGN_ISSUER, DEMO_AUDIENCE, ttl);
                (
                    rogue.sign(&claims)?,
                    FOREIGN_ISSUER,
                    DEMO_AUDIENCE,
                    Some(kid),
                    ttl,
                )
            }

            // No `kid` header at all — which `TokenManager` cannot produce,
            // hence `RogueKey`. Under `require_kid` this is refused without
            // trying any key, rather than tried against all of them.
            Forgery::MissingKid => {
                let rogue = RogueKey::generate(None)?;
                let claims = forged_claims(&user_id, issuer, DEMO_AUDIENCE, ttl);
                (rogue.sign(&claims)?, issuer, DEMO_AUDIENCE, None, ttl)
            }

            // These two need no second key: the deployment's own published key
            // signs something it should nonetheless refuse to accept back,
            // which is the more instructive version of both failures.
            Forgery::WrongAudience => {
                let token = self
                    .signing
                    .manager()
                    .issue_user_token(
                        self.identity(user_id),
                        TOKEN_TTL_SECS,
                        None,
                        Some(OTHER_AUDIENCE.to_string()),
                    )
                    .map_err(|e| ApiError::Scenario(e.to_string()))?;
                (token, issuer, OTHER_AUDIENCE, real_kid.clone(), ttl)
            }
            Forgery::Expired => {
                // Past-dated, not zero-TTL. A zero lifetime puts `exp` at
                // *now*, which is not yet past — so with `leeway` at zero the
                // token stays valid for the remainder of the current second,
                // and the demonstration only works if you are slow. Signed by
                // the real published key, so expiry is the single thing wrong
                // with it.
                let claims = forged_claims(&user_id, issuer, DEMO_AUDIENCE, -EXPIRED_BY_SECS);
                let token = self
                    .signing
                    .sign(&claims)
                    .map_err(|e| ApiError::Scenario(e.to_string()))?;
                (token, issuer, DEMO_AUDIENCE, real_kid.clone(), 0)
            }
        };

        Ok(self.describe(
            token,
            expires_in.max(0) as u64,
            audience,
            claimed_issuer,
            kid,
            Some(forgery.label()),
        ))
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
        "A route that validates a token it did not issue: the `kid` is looked up in a \
         published key set, so the route holds no secret at all."
    }

    fn control(&self) -> ControlShape {
        ControlShape::Toggle
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::Toggle { enabled: false }
    }

    fn actions(&self) -> Vec<&'static str> {
        vec!["issue", "forge", "call"]
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        if !value.is_active() {
            return Consequences::default();
        }

        Consequences {
            routes: vec![
                "GET /.well-known/jwks.json".to_string(),
                "GET /api/protected".to_string(),
            ],
            requirements: vec![
                "You sign with an asymmetric key and publish the public half as a JWKS. \
                 The validating side then needs no secret — which is what lets it be a \
                 different service, or somebody else's."
                    .to_string(),
                "Every token carries a `kid`, and the validator refuses one that does \
                 not. Falling back to \"try every published key\" is how a retired key \
                 keeps working for as long as it stays published."
                    .to_string(),
                "You keep a trust map of issuer to key set. An `iss` that is not in it \
                 is refused outright, with no fallback to a default key — otherwise any \
                 issuer you ever trusted can mint tokens for every service you own."
                    .to_string(),
                "The key set is cached and re-fetched on a miss, so a rotation is picked \
                 up without a restart. Decide the refresh interval deliberately: it is \
                 the window in which a retired key still works."
                    .to_string(),
                "Decide what an unreachable key set means before it happens. This demo \
                 answers 503 rather than 401 — nothing was decided about the credential, \
                 so calling it invalid would be a lie."
                    .to_string(),
            ],
            crates: vec![
                CrateRequirement::new("authkestra-engine", &["token"]),
                CrateRequirement::new("authkestra-resource", &[]),
                CrateRequirement::new("authkestra-axum", &["resource", "token"]),
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
            return Err(ApiError::CeremonyRejected(
                "Turn the protected route on first.".to_string(),
            ));
        }

        match action {
            "issue" => {
                let issued = self.issue_honest(ctx.user_id())?;
                ctx.record(
                    Step::success("resource", "token issued")
                        .detail(issued_token_detail(
                            issued.kid.as_deref().unwrap_or("no kid"),
                            &issued.issuer,
                        ))
                        .fact("kid", issued.kid.clone().unwrap_or_default())
                        .fact("audience", issued.audience.clone()),
                )
                .await;
                Ok(serde_json::to_value(issued).expect("token serialises"))
            }

            "forge" => {
                let forgery: Forgery = body
                    .get("kind")
                    .and_then(|k| serde_json::from_value(k.clone()).ok())
                    .ok_or_else(|| {
                        ApiError::CeremonyRejected(format!(
                            "Choose what to forge. One of: {}.",
                            Forgery::ALL
                                .iter()
                                .filter_map(|f| serde_json::to_string(f).ok())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    })?;

                let issued = self.issue_forged(ctx.user_id(), forgery)?;
                ctx.record(
                    Step::info("resource", "forged token minted")
                        .detail(format!(
                            "A real, correctly signed token that {}. Present it to the \
                             protected route and watch it get nothing.",
                            forgery.label()
                        ))
                        .fact("forgery", forgery.label()),
                )
                .await;
                Ok(serde_json::to_value(issued).expect("token serialises"))
            }

            "call" => {
                let bearer = body
                    .get("token")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|t| !t.is_empty());

                let (verdict, subject, detail) = match bearer {
                    None => (
                        TokenVerdict::Absent,
                        None,
                        TokenVerdict::Absent.detail().to_string(),
                    ),
                    // Structurally not a JWT. Checked here so obvious junk
                    // never causes a key-set fetch, and so it is reported as
                    // malformed rather than as whatever the resolver makes of
                    // a token it cannot read an `iss` out of.
                    Some(token) if !looks_like_a_jwt(token) => (
                        TokenVerdict::Malformed,
                        None,
                        TokenVerdict::Malformed.detail().to_string(),
                    ),
                    Some(token) => classify(self.validator.validate(token).await),
                };

                let step = match verdict {
                    TokenVerdict::Accepted => Step::success("resource", "request served"),
                    TokenVerdict::KeysUnreachable => Step::failed("resource", "could not decide"),
                    _ => Step::rejected("resource", "request refused"),
                };
                ctx.record(
                    step.detail(detail.clone())
                        .fact("verdict", format!("{verdict:?}"))
                        .fact("status", verdict.status().to_string()),
                )
                .await;

                Ok(serde_json::to_value(ProtectedCall {
                    verdict,
                    status: verdict.status(),
                    detail,
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
                "use authkestra_resource::jwt::JwksCache;".to_string(),
                "use axum::extract::State;".to_string(),
                "use axum::http::HeaderMap;".to_string(),
            ],
            prelude: vec![
                r#"    // The validating side of a resource server. Note what is NOT here: any
    // signing key, any shared secret, anything that could mint a token. It
    // holds a URL and the name of an issuer it trusts, and that is the whole
    // of its trust configuration — which is what lets the issuer be a
    // different service, or somebody else's.
    //
    // `require_kid(true)` is the strict policy and worth defaulting to: a
    // token with no `kid` would otherwise be tried against every published
    // key, which is how a retired key keeps working.
    let trusted_issuer = std::env::var("TOKEN_ISSUER").expect("TOKEN_ISSUER must be set");
    let jwks = std::sync::Arc::new(
        JwksCache::new(
            std::env::var("JWKS_URL").expect("JWKS_URL must be set"),
            // The window in which a retired key still works. Deliberate, not
            // incidental.
            std::time::Duration::from_secs(300),
        )
        .require_kid(true),
    );
    let audience = std::env::var("TOKEN_AUDIENCE").unwrap_or_else(|_| "my-api".to_string());"#
                    .to_string(),
            ],
            builder_calls: Vec::new(),
            routes: vec![
                r#"        // Validated by key discovery — see `protected` below.
        .route("/api/protected", get(protected))"#
                    .to_string(),
            ],
            handlers: vec![PROTECTED_HANDLER.to_string()],
            state_fields: vec![
                "    /// The issuer's published keys, cached. No secret here.\n\
                 \x20   jwks: std::sync::Arc<authkestra_resource::jwt::JwksCache>,\n\
                 \x20   /// The `iss` this service accepts, and the `aud` it demands.\n\
                 \x20   trusted_issuer: String,\n\
                 \x20   audience: String,"
                    .to_string(),
            ],
            state_init: vec![
                "        jwks: jwks.clone(),".to_string(),
                "        trusted_issuer: trusted_issuer.clone(),".to_string(),
                "        audience: audience.clone(),".to_string(),
            ],
            openapi_paths: Vec::new(),
            openapi_schemas: Vec::new(),
            crates: vec![CrateRequirement::new("serde", &["derive"])],
            env: vec![
                KitEnvVar::required(
                    "JWKS_URL",
                    "Where the issuer publishes its public keys, e.g. \
                     https://issuer.example.com/.well-known/jwks.json.",
                ),
                KitEnvVar::required(
                    "TOKEN_ISSUER",
                    "The `iss` this service accepts. An `iss` not named here is refused \
                     outright, with no fallback to a default key.",
                ),
                KitEnvVar::with_default(
                    "TOKEN_AUDIENCE",
                    "What this service demands in a token's `aud` claim.",
                    "my-api",
                ),
            ],
            notes: vec![
                "**Protected route.** `GET /api/protected` validates a bearer token by \
                 looking its `kid` up in the issuer's published key set. This project \
                 holds no signing key and no shared secret, which is what makes it a \
                 resource server rather than a second copy of the issuer."
                    .to_string(),
                "The `aud` claim is checked, not just the signature. Without it a token \
                 issued for one of your services is accepted by all of them, which is \
                 the whole reason the claim exists."
                    .to_string(),
                "There is no fallback in the trust map. An `iss` you have not listed is \
                 refused even if its signature verifies against a key you happen to \
                 hold — otherwise any issuer you ever trusted can mint tokens for \
                 everything you own."
                    .to_string(),
                "Decide what an unreachable key set means. This handler answers 503 \
                 rather than 401: nothing was decided about the credential, so calling \
                 it invalid would be a lie, and a client that retries on 503 and gives \
                 up on 401 will then do the right thing."
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

/// Three dot-separated segments whose payload decodes. Cheap, and enough to
/// keep obvious junk from causing a key-set fetch.
fn looks_like_a_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3 && base64url_decode(parts[1]).is_some_and(|p| !p.is_empty())
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

/// The generated handler for the protected route.
const PROTECTED_HANDLER: &str = r##"/// A route that validates a token it did not issue.
///
/// What makes this a *resource server* rather than a second copy of the issuer
/// is what it does not have: no signing key, no shared secret. It fetches the
/// issuer's public keys, finds the one named by the token's `kid`, and checks
/// the signature against that.
///
/// Move these lines to the top of whichever handler you want protected. A route
/// whose only job is to validate a token protects nothing, because whatever was
/// supposed to call it can decline to.
async fn protected(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "no_token" })),
        )
            .into_response();
    };

    // The resolver decides which key; this decides which names are acceptable.
    // Both have to agree, which is why the issuer is configured twice.
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::EdDSA);
    validation.set_issuer(&[state.trusted_issuer.as_str()]);
    validation.set_audience(&[state.audience.as_str()]);

    match authkestra_resource::jwt::validate_jwt(token, &state.jwks, &validation).await {
        Ok(claims) => {
            Json(json!({ "subject": claims.sub, "expires_at": claims.exp })).into_response()
        }
        // Two failures, two statuses. `Http` means the key set could not be
        // fetched, so nothing was decided about the credential — answering 401
        // would tell the client its token is bad when we do not know that, and
        // a client that retries on 503 and gives up on 401 would then give up
        // on a token that was fine.
        Err(authkestra_resource::jwt::ValidationError::Http(e)) => {
            tracing::error!(error = %e, "could not fetch the issuer's key set");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "keys_unreachable" })),
            )
                .into_response()
        }
        Err(e) => {
            // Worth logging in full, never worth returning: the reason a
            // forgery failed tells its author which part to fix next.
            tracing::warn!(error = %e, "token rejected");
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "invalid_token" })),
            )
                .into_response()
        }
    }
}"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt_error(kind: jsonwebtoken::errors::ErrorKind) -> ValidationError {
        ValidationError::Jwt(kind.into())
    }

    fn verdict_of(outcome: Result<Claims, ValidationError>) -> TokenVerdict {
        classify(outcome).0
    }

    /// `Claims` is `#[non_exhaustive]`, so it is deserialised rather than
    /// constructed — which is how it arrives in production anyway.
    fn claims() -> Claims {
        serde_json::from_value(serde_json::json!({
            "iss": "https://issuer.test",
            "sub": "visitor-1",
            "exp": 0,
            "iat": 0,
        }))
        .expect("claims deserialise")
    }

    #[test]
    fn a_valid_token_is_accepted_and_names_its_subject() {
        let (verdict, subject, _) = classify(Ok(claims()));
        assert_eq!(verdict, TokenVerdict::Accepted);
        assert_eq!(subject.as_deref(), Some("visitor-1"));
        assert_eq!(verdict.status(), 200);
    }

    /// Each of these is a different mistake with a different fix, and the whole
    /// reason this scenario exists is not to collapse them into one 401.
    #[test]
    fn every_validation_error_maps_to_its_own_verdict() {
        use jsonwebtoken::errors::ErrorKind;

        let cases: Vec<(ValidationError, TokenVerdict)> = vec![
            (ValidationError::MissingKid, TokenVerdict::MissingKid),
            (ValidationError::KeyNotFound, TokenVerdict::UnknownKid),
            (
                ValidationError::UntrustedIssuer("https://elsewhere".into()),
                TokenVerdict::UntrustedIssuer,
            ),
            (
                ValidationError::MissingIssuer,
                TokenVerdict::UntrustedIssuer,
            ),
            (
                ValidationError::InvalidToken("junk".into()),
                TokenVerdict::Malformed,
            ),
            (
                jwt_error(ErrorKind::ExpiredSignature),
                TokenVerdict::Expired,
            ),
            (
                jwt_error(ErrorKind::InvalidAudience),
                TokenVerdict::WrongAudience,
            ),
            (
                jwt_error(ErrorKind::InvalidIssuer),
                TokenVerdict::WrongIssuer,
            ),
            (
                jwt_error(ErrorKind::InvalidSignature),
                TokenVerdict::BadSignature,
            ),
            (jwt_error(ErrorKind::InvalidToken), TokenVerdict::Malformed),
        ];

        for (error, expected) in cases {
            let got = verdict_of(Err(error));
            assert_eq!(got, expected, "expected {expected:?}, got {got:?}");
        }
    }

    /// An unknown error kind must not be dressed up as a specific verdict. Both
    /// enums are `#[non_exhaustive]`, so this is a live risk on every upgrade.
    #[test]
    fn an_unrecognised_failure_is_reported_as_unnamed_rather_than_guessed() {
        let verdict = verdict_of(Err(jwt_error(
            jsonwebtoken::errors::ErrorKind::ImmatureSignature,
        )));
        assert_eq!(verdict, TokenVerdict::Rejected);
        assert_eq!(verdict.status(), 401);
    }

    /// The validator's own message has to survive, or a verdict this scenario
    /// has no name for tells the visitor nothing at all.
    #[test]
    fn the_detail_carries_both_our_verdict_and_the_validators_words() {
        let (_, _, detail) = classify(Err(ValidationError::UntrustedIssuer(
            "https://elsewhere.example".into(),
        )));
        assert!(detail.contains("trust map"), "{detail}");
        assert!(
            detail.contains("https://elsewhere.example"),
            "the issuer that was refused should appear: {detail}"
        );
    }

    #[test]
    fn every_verdict_explains_itself_distinctly() {
        let all = [
            TokenVerdict::Accepted,
            TokenVerdict::Absent,
            TokenVerdict::Malformed,
            TokenVerdict::MissingKid,
            TokenVerdict::UnknownKid,
            TokenVerdict::UntrustedIssuer,
            TokenVerdict::WrongIssuer,
            TokenVerdict::WrongAudience,
            TokenVerdict::Expired,
            TokenVerdict::BadSignature,
            TokenVerdict::KeysUnreachable,
            TokenVerdict::Rejected,
        ];

        for (i, a) in all.iter().enumerate() {
            assert!(
                a.detail().len() > 40,
                "a flat message teaches nothing: {a:?}"
            );
            for b in all.iter().skip(i + 1) {
                assert_ne!(a.detail(), b.detail(), "{a:?} and {b:?} share a message");
            }
        }
    }

    /// Only two statuses are meaningful here, and which failure gets which is
    /// the distinction the scenario is teaching.
    #[test]
    fn only_an_undecidable_check_answers_503() {
        assert_eq!(TokenVerdict::Accepted.status(), 200);
        assert_eq!(TokenVerdict::KeysUnreachable.status(), 503);
        for rejected in [
            TokenVerdict::Absent,
            TokenVerdict::Malformed,
            TokenVerdict::MissingKid,
            TokenVerdict::UnknownKid,
            TokenVerdict::UntrustedIssuer,
            TokenVerdict::WrongIssuer,
            TokenVerdict::WrongAudience,
            TokenVerdict::Expired,
            TokenVerdict::BadSignature,
            TokenVerdict::Rejected,
        ] {
            assert_eq!(rejected.status(), 401, "{rejected:?}");
        }
    }

    #[test]
    fn obvious_junk_is_malformed_without_a_key_set_fetch() {
        for junk in ["hello", "a.b", "a.b.c.d", "..", "a..c"] {
            assert!(!looks_like_a_jwt(junk), "{junk} should not look like a JWT");
        }
        // A real token's shape, whoever signed it.
        assert!(looks_like_a_jwt(
            "eyJhbGciOiJFZERTQSJ9.eyJzdWIiOiJhIn0.c2ln"
        ));
    }

    /// Each forgery has to state what it should prove, or the panel cannot
    /// explain itself and the tests have nothing to assert against.
    #[test]
    fn every_forgery_names_a_distinct_expected_verdict() {
        let mut seen = Vec::new();
        for forgery in Forgery::ALL {
            assert!(
                !forgery.label().is_empty(),
                "{forgery:?} has no explanation"
            );
            let expected = forgery.expected();
            assert!(
                !seen.contains(&expected),
                "{forgery:?} duplicates the verdict {expected:?}"
            );
            assert_ne!(
                expected,
                TokenVerdict::Accepted,
                "{forgery:?} is meant to fail"
            );
            seen.push(expected);
        }
        assert_eq!(seen.len(), Forgery::ALL.len());
    }

    #[test]
    fn a_forgery_kind_round_trips_through_its_wire_name() {
        for forgery in Forgery::ALL {
            let wire = serde_json::to_value(forgery).expect("serialises");
            let back: Forgery = serde_json::from_value(wire.clone()).expect("deserialises");
            assert_eq!(back, forgery, "{wire} did not round trip");
        }
    }

    #[test]
    fn the_issued_token_detail_agrees_with_the_constants() {
        let detail = issued_token_detail("key-1", "https://issuer.test");
        assert!(detail.contains(&TOKEN_TTL_SECS.to_string()), "{detail}");
        assert!(detail.contains("key-1"), "{detail}");
        assert!(detail.contains(DEMO_AUDIENCE), "{detail}");
        assert!(
            !detail.contains("minute"),
            "the TTL is in seconds; saying minutes is how this went wrong before: {detail}"
        );
    }
}
