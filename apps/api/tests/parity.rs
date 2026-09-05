//! The download must behave like the demo.
//!
//! The promise of the starter kit is "this is what you just used". If the two
//! diverge, the demo becomes a lie — and the divergence would be invisible,
//! because each side has its own passing tests.
//!
//! So the assertions live here once, expressed against a `Target` trait, and
//! are run twice: against the playground's own router in process, and against
//! a generated project over HTTP.
//!
//! Routes differ between the two on purpose, and so does how a ceremony is
//! addressed — see `DELIBERATE_DIFFERENCES` at the bottom, which is asserted
//! to appear in the generated README rather than left as folklore.
//!
//! ```sh
//! # the playground half runs with the rest of the suite
//! cargo test --test parity
//!
//! # the generated half needs a project running, and is not skipped quietly
//! PARITY_TARGET=http://127.0.0.1:8300 cargo test --test parity -- --ignored
//! ```

use api::killswitch::KillSwitch;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use totp_rs::{Algorithm, Secret, TOTP};
use tower::ServiceExt;

/// One side of the comparison. Both must answer the same questions, however
/// differently they are wired underneath.
#[allow(async_fn_in_trait)]
trait Target {
    fn name(&self) -> &'static str;
    /// Start again as somebody new.
    ///
    /// TOTP codes are only unique per 30-second step, so two checks sharing an
    /// identity inside one step would see each other's code as a replay — and
    /// the failure would look like a bug in the code under test rather than in
    /// the test.
    async fn use_fresh_identity(&mut self);
    async fn totp_provision(&self) -> (u16, Value);
    async fn totp_verify(&self, code: &str) -> (u16, Value);
    async fn passkey_register_start(&self) -> (u16, Value);
    async fn passkey_register_finish_malformed(&self) -> (u16, Value);
}

/// Build the same TOTP parameters the engine uses, so generated codes verify.
fn code_for(secret_b32: &str) -> String {
    let bytes = Secret::Encoded(secret_b32.to_string()).to_bytes().unwrap();
    TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes, None, "parity".to_string())
        .unwrap()
        .generate_current()
        .unwrap()
}

// ------------------------------------------------------------ the contract

/// Enrolling hands back a secret and a URI that actually carries it.
///
/// A URI missing the secret is the failure that looks fine in a screenshot and
/// produces an authenticator that can never generate a valid code.
async fn enrolment_returns_a_usable_secret<T: Target>(t: &mut T) -> String {
    let (status, body) = t.totp_provision().await;
    assert_eq!(status, 200, "{}: provisioning failed: {body}", t.name());

    let secret = body["secret"].as_str().unwrap_or_default().to_string();
    let uri = body["uri"].as_str().unwrap_or_default();

    assert!(!secret.is_empty(), "{}: no secret returned", t.name());
    assert!(
        uri.starts_with("otpauth://totp/"),
        "{}: not an otpauth URI: {uri}",
        t.name()
    );
    assert!(
        uri.contains(&secret),
        "{}: the URI does not carry the secret it just issued",
        t.name()
    );
    secret
}

async fn a_correct_code_is_accepted<T: Target>(t: &mut T, secret: &str) {
    let (status, body) = t.totp_verify(&code_for(secret)).await;
    assert_eq!(
        status,
        200,
        "{}: a valid code was refused: {body}",
        t.name()
    );
    assert_eq!(
        body["verified"],
        true,
        "{}: a valid code was not verified: {body}",
        t.name()
    );
}

/// A wrong code is an ordinary outcome, not a fault. Answering 5xx would tell
/// a caller to retry, and would page somebody.
async fn a_wrong_code_is_rejected_not_errored<T: Target>(t: &mut T) {
    let (status, body) = t.totp_verify("000000").await;
    assert!(
        (400..500).contains(&status) || status == 200,
        "{}: a wrong code produced {status}, which reads as a server fault: {body}",
        t.name()
    );
    assert_eq!(
        body["verified"],
        false,
        "{}: a wrong code was treated as verified: {body}",
        t.name()
    );
}

/// The same correct code twice is a replay, and the second must fail. Without
/// this, an observed code stays usable for the rest of its window.
async fn a_replayed_code_is_rejected<T: Target>(t: &mut T, secret: &str) {
    let code = code_for(secret);
    let (_, first) = t.totp_verify(&code).await;
    assert_eq!(first["verified"], true, "{}: setup failed", t.name());

    let (_, second) = t.totp_verify(&code).await;
    assert_eq!(
        second["verified"],
        false,
        "{}: the same code verified twice — replay protection is not working",
        t.name()
    );
}

async fn registration_starts_with_a_real_challenge<T: Target>(t: &mut T) {
    let (status, body) = t.passkey_register_start().await;
    assert_eq!(status, 200, "{}: {body}", t.name());

    let challenge = body
        .pointer("/publicKey/challenge")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let rp = body
        .pointer("/publicKey/rp/id")
        .and_then(Value::as_str)
        .unwrap_or_default();

    assert!(
        challenge.len() > 16,
        "{}: challenge is too short to be real: {body}",
        t.name()
    );
    assert!(!rp.is_empty(), "{}: no relying-party id: {body}", t.name());
}

/// A malformed body must be a client error, and must *not* consume the
/// challenge. Consuming first means a typo costs the visitor their ceremony
/// over a mistake that never reached any cryptography — a bug this repository
/// shipped once already.
async fn a_malformed_credential_is_a_client_error<T: Target>(t: &mut T) {
    let (status, body) = t.passkey_register_finish_malformed().await;
    assert_eq!(
        status,
        400,
        "{}: expected 400 for a malformed credential, got {status}: {body}",
        t.name()
    );
}

/// Whatever secret you were handed last is the one that works.
///
/// The two reach this differently and both are defensible: the playground's
/// store replaces the credential, so re-enrolling issues a new working secret;
/// the generated project refuses a second enrolment, because the framework's
/// `SqlxCredentialStore` appends and `CredentialStore` has no delete, so a
/// second secret would be dead on arrival while the old one kept working.
///
/// What neither may do is hand over a QR code that cannot verify. This test
/// caught exactly that in the generated project.
async fn the_last_issued_secret_is_the_one_that_works<T: Target>(t: &mut T) {
    let first = enrolment_returns_a_usable_secret(t).await;

    let (status, body) = t.totp_provision().await;
    let usable = if status == 200 {
        // Re-enrolment was allowed, so the new secret must be the live one.
        body["secret"].as_str().expect("a secret").to_string()
    } else {
        assert!(
            (400..500).contains(&status),
            "{}: re-enrolling gave {status}, which reads as a fault: {body}",
            t.name()
        );
        first
    };

    let (_, verified) = t.totp_verify(&code_for(&usable)).await;
    assert_eq!(
        verified["verified"],
        true,
        "{}: the secret the caller was last given does not verify",
        t.name()
    );
}

async fn run_contract<T: Target>(t: &mut T) {
    // Every check starts as a new identity, for the reason on
    // `use_fresh_identity`.
    t.use_fresh_identity().await;
    let secret = enrolment_returns_a_usable_secret(t).await;
    a_correct_code_is_accepted(t, &secret).await;

    t.use_fresh_identity().await;
    let secret = enrolment_returns_a_usable_secret(t).await;
    a_replayed_code_is_rejected(t, &secret).await;

    t.use_fresh_identity().await;
    a_wrong_code_is_rejected_not_errored(t).await;

    t.use_fresh_identity().await;
    the_last_issued_secret_is_the_one_that_works(t).await;

    t.use_fresh_identity().await;
    registration_starts_with_a_real_challenge(t).await;
    a_malformed_credential_is_a_client_error(t).await;
}

// ------------------------------------------------------- the playground side

struct Playground {
    app: axum::Router,
    cookie: String,
}

impl Playground {
    async fn start() -> Self {
        let app = api::build_router(api::testing::test_state(KillSwitch::default(), None));

        let mut cookie = String::new();
        for id in ["totp", "passkeys"] {
            let resp = app
                .clone()
                .oneshot(
                    Self::req("POST", &format!("/api/scenarios/{id}/configure"), &cookie)
                        .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "could not enable {id}");
            if cookie.is_empty() {
                cookie = resp
                    .headers()
                    .get(header::SET_COOKIE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.split(';').next())
                    .expect("session cookie")
                    .to_string();
            }
        }
        Self { app, cookie }
    }

    fn req(method: &str, uri: &str, cookie: &str) -> axum::http::request::Builder {
        let b = Request::builder()
            .method(method)
            .uri(uri)
            // The rate limiter keys on client IP, so tests must present one.
            .header("x-forwarded-for", "203.0.113.9")
            .header(header::CONTENT_TYPE, "application/json");
        if cookie.is_empty() {
            b
        } else {
            b.header(header::COOKIE, cookie)
        }
    }

    async fn post(&self, uri: &str, body: Value) -> (u16, Value) {
        let resp = self
            .app
            .clone()
            .oneshot(
                Self::req("POST", uri, &self.cookie)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

impl Target for Playground {
    fn name(&self) -> &'static str {
        "playground"
    }

    async fn use_fresh_identity(&mut self) {
        // A demo session *is* the identity here, so a new cookie is a new
        // person, with the scenarios re-enabled for them.
        *self = Playground::start().await;
    }

    async fn totp_provision(&self) -> (u16, Value) {
        self.post("/api/scenarios/totp/action/provision", json!({}))
            .await
    }

    async fn totp_verify(&self, code: &str) -> (u16, Value) {
        self.post("/api/scenarios/totp/action/verify", json!({ "code": code }))
            .await
    }

    async fn passkey_register_start(&self) -> (u16, Value) {
        self.post("/api/scenarios/passkeys/action/register_start", json!({}))
            .await
    }

    async fn passkey_register_finish_malformed(&self) -> (u16, Value) {
        self.post(
            "/api/scenarios/passkeys/action/register_finish",
            json!({ "credential": { "not": "a credential" } }),
        )
        .await
    }
}

#[tokio::test]
async fn the_playground_satisfies_the_parity_contract() {
    run_contract(&mut Playground::start().await).await;
}

// ------------------------------------------------ the generated-project side

struct Generated {
    base: String,
    client: reqwest::Client,
    username: String,
}

impl Generated {
    /// Deliberately not `Option`. A parity test that skips itself when the
    /// target is missing would report success for a comparison it never made.
    fn from_env() -> Self {
        let base = std::env::var("PARITY_TARGET").expect(
            "PARITY_TARGET must point at a running generated project. \
             This test compares two implementations; without the second there \
             is nothing to compare, so it fails rather than passing quietly.",
        );
        Self {
            base: base.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            // Fresh per run: the generated project keeps credentials in a file
            // that survives restarts, and an already-enrolled user would make
            // the replay check meaningless.
            username: format!("parity-{}", uuid::Uuid::new_v4()),
        }
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .expect("the generated project should answer");
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }
}

impl Target for Generated {
    fn name(&self) -> &'static str {
        "generated project"
    }

    async fn use_fresh_identity(&mut self) {
        self.username = format!("parity-{}", uuid::Uuid::new_v4());
    }

    async fn totp_provision(&self) -> (u16, Value) {
        self.post("/auth/totp/enroll", json!({ "username": self.username }))
            .await
    }

    async fn totp_verify(&self, code: &str) -> (u16, Value) {
        self.post(
            "/auth/totp/verify",
            json!({ "username": self.username, "code": code }),
        )
        .await
    }

    async fn passkey_register_start(&self) -> (u16, Value) {
        self.post(
            "/auth/passkey/register/start",
            json!({ "username": self.username }),
        )
        .await
    }

    async fn passkey_register_finish_malformed(&self) -> (u16, Value) {
        self.post(
            "/auth/passkey/register/finish",
            json!({ "ceremony_id": "unused", "credential": { "not": "a credential" } }),
        )
        .await
    }
}

#[tokio::test]
#[ignore = "needs PARITY_TARGET pointing at a running generated project"]
async fn the_generated_project_satisfies_the_parity_contract() {
    run_contract(&mut Generated::from_env()).await;
}

// ------------------------------------------------------ documented differences

/// Where the two legitimately differ. Asserted to be in the generated README,
/// so a difference is something a reader is told about rather than something
/// they discover.
const DELIBERATE_DIFFERENCES: &[&str] = &[
    // The playground scopes a ceremony to the demo session cookie; a generated
    // project has no session before sign-in, so it hands back an explicit id.
    "ceremony",
    // The playground's routes are namespaced under /api/scenarios/ because it
    // drives many scenarios from one surface.
    "/auth/passkey/",
];

#[test]
fn the_generated_readme_explains_where_the_two_differ() {
    use api::demo_config::DemoConfig;
    use api::kit::StarterKit;
    use api::scenario::{ControlValue, ScenarioRegistry};

    let registry = ScenarioRegistry::with_providers(Vec::new());
    let mut config = DemoConfig::defaults_for(&registry);
    config.set("passkeys", ControlValue::Toggle { enabled: true });
    config.set("totp", ControlValue::Toggle { enabled: true });

    let kit = StarterKit::generate(&config, &registry);
    let readme = &kit.file("README.md").expect("a README").contents;

    for needle in DELIBERATE_DIFFERENCES {
        assert!(
            readme.contains(needle),
            "the README never mentions {needle}, so the difference is folklore"
        );
    }
}
