//! Tests for the passkeys scenario.
//!
//! A full ceremony needs a real authenticator, so what is verified here is
//! everything up to and around the browser: challenge issuance, ceremony-state
//! lifecycle, the failure paths a visitor actually hits (cancelled prompt,
//! timeout, no passkey enrolled), and the relying-party misconfiguration that
//! is the usual cause of an otherwise inexplicable browser-side failure.

use std::sync::Arc;

use api::killswitch::KillSwitch;
use api::routes::AppState;
use api::settings::RelyingParty;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn state_with_rp(rp: RelyingParty) -> AppState {
    let settings = api::settings::Settings {
        relying_party: rp,
        ..api::testing::test_settings(None)
    };
    api::testing::test_state_with_settings(KillSwitch::default(), settings)
}

fn good_rp() -> RelyingParty {
    RelyingParty {
        id: "localhost".to_string(),
        origin: "http://localhost:3000".to_string(),
        name: "Playground Test".to_string(),
        extra_origins: Vec::new(),
    }
}

async fn state() -> AppState {
    state_with_rp(good_rp()).await
}

fn req(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-forwarded-for", "203.0.113.6")
        .header(header::CONTENT_TYPE, "application/json")
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn cookie_of(resp: &axum::response::Response) -> String {
    resp.headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .expect("session cookie")
        .to_string()
}

async fn enable(app: &axum::Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/passkeys/configure")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    cookie_of(&resp)
}

async fn action(
    app: &axum::Router,
    cookie: &str,
    name: &str,
    body: &str,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            req("POST", &format!("/api/scenarios/passkeys/action/{name}"))
                .header(header::COOKIE, cookie)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// A registration response in the browser's own shape.
///
/// Well-formed so it gets past parsing — it cannot verify, which is what these
/// tests want: they are about ceremony state, not cryptography. Note
/// `clientDataJSON`, capitalised as the WebAuthn spec has it.
const WELL_FORMED_REGISTRATION: &str = r#"{
    "id": "Y3JlZC1pZA",
    "rawId": "Y3JlZC1pZA",
    "type": "public-key",
    "response": {
        "clientDataJSON": "eyJ0eXBlIjoid2ViYXV0aG4uY3JlYXRlIn0",
        "attestationObject": "o2NmbXRkbm9uZQ"
    }
}"#;

#[tokio::test]
async fn registration_start_issues_a_usable_challenge() {
    let app = api::build_router(state().await);
    let cookie = enable(&app).await;

    let resp = action(&app, &cookie, "register_start", "{}").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let challenge = body_json(resp).await;
    let pk = &challenge["publicKey"];
    assert!(!pk.is_null(), "expected a publicKey block: {challenge}");
    assert!(
        pk["challenge"].as_str().is_some_and(|c| !c.is_empty()),
        "a challenge must be present"
    );
    // The RP ID is what the browser checks against the page's origin.
    assert_eq!(pk["rp"]["id"], "localhost");
    assert!(pk["user"]["id"].as_str().is_some());
}

/// The usual cause of a ceremony failing with an opaque browser error: the RP
/// ID is not a registrable suffix of the origin's host. It must fail loudly on
/// the server with an explanation rather than producing a broken challenge.
#[tokio::test]
async fn a_mismatched_relying_party_fails_with_an_explanation() {
    let app = api::build_router(
        state_with_rp(RelyingParty {
            id: "example.com".to_string(),
            origin: "http://localhost:3000".to_string(),
            name: "Mismatched".to_string(),
            extra_origins: Vec::new(),
        })
        .await,
    );
    let cookie = enable(&app).await;

    let resp = action(&app, &cookie, "register_start", "{}").await;

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let detail = body_json(resp).await["detail"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        detail.contains("registrable suffix") || detail.contains("relying party"),
        "the error should say what is actually wrong, got: {detail}"
    );
}

#[tokio::test]
async fn finishing_without_starting_is_reported_as_expired() {
    let app = api::build_router(state().await);
    let cookie = enable(&app).await;

    // Well-formed, so it reaches the challenge lookup rather than failing on
    // its shape.
    let resp = action(&app, &cookie, "register_finish", WELL_FORMED_REGISTRATION).await;

    assert_eq!(resp.status(), StatusCode::GONE);
    assert_eq!(body_json(resp).await["error"], "ceremony_expired");
}

/// A challenge must be answerable once. Answering it twice — a replayed
/// response — must not be verified a second time.
#[tokio::test]
async fn a_challenge_cannot_be_answered_twice() {
    let app = api::build_router(state().await);
    let cookie = enable(&app).await;

    action(&app, &cookie, "register_start", "{}").await;

    // First attempt consumes the state; the credential cannot verify, but the
    // state is spent either way.
    let first = action(&app, &cookie, "register_finish", WELL_FORMED_REGISTRATION).await;
    assert_ne!(first.status(), StatusCode::GONE);

    let second = action(&app, &cookie, "register_finish", WELL_FORMED_REGISTRATION).await;
    assert_eq!(
        second.status(),
        StatusCode::GONE,
        "the challenge should have been consumed by the first attempt"
    );
}

#[tokio::test]
async fn a_malformed_registration_response_is_a_400_not_a_panic() {
    let app = api::build_router(state().await);
    let cookie = enable(&app).await;
    action(&app, &cookie, "register_start", "{}").await;

    let resp = action(&app, &cookie, "register_finish", r#"{"id":"nope"}"#).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // And the challenge survives, so a mistake that never reached any
    // cryptography does not cost the visitor the whole ceremony.
    let retry = action(&app, &cookie, "register_finish", WELL_FORMED_REGISTRATION).await;
    assert_ne!(
        retry.status(),
        StatusCode::GONE,
        "a malformed body must not consume the challenge"
    );
}

#[tokio::test]
async fn signing_in_before_registering_explains_itself() {
    let app = api::build_router(state().await);
    let cookie = enable(&app).await;

    let resp = action(&app, &cookie, "authenticate_start", "{}").await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(resp).await["detail"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        detail.to_lowercase().contains("register"),
        "should tell the visitor to register first, got: {detail}"
    );
}

#[tokio::test]
async fn ceremonies_are_refused_until_the_scenario_is_enabled() {
    let app = api::build_router(state().await);

    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/passkeys/action/register_start")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_kill_switch_stops_passkey_ceremonies() {
    use api::killswitch::KillSwitchState;

    let mut st = state().await;
    let mut switch_state = KillSwitchState::default();
    switch_state
        .disabled_scenarios
        .insert("passkeys".to_string());
    let ks = KillSwitch::new(None, switch_state);
    st.kill_switch = Arc::new(ks);
    let app = api::build_router(st);

    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/passkeys/action/register_start")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn two_visitors_do_not_share_ceremony_state() {
    let app = api::build_router(state().await);

    let cookie_a = enable(&app).await;
    let cookie_b = enable(&app).await;
    assert_ne!(cookie_a, cookie_b);

    action(&app, &cookie_a, "register_start", "{}").await;

    // B never started one, so B's finish must not find A's challenge.
    let resp = action(&app, &cookie_b, "register_finish", WELL_FORMED_REGISTRATION).await;
    assert_eq!(resp.status(), StatusCode::GONE);
}

#[tokio::test]
async fn the_diff_names_the_engine_webauthn_feature_not_the_facade() {
    let app = api::build_router(state().await);

    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/passkeys/configure")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    let diff = body_json(resp).await;
    let crates = diff["diff"]["consequences"]["crates"].as_array().unwrap();

    let engine = crates
        .iter()
        .find(|c| c["name"] == "authkestra-engine")
        .expect("must name authkestra-engine");
    let features: Vec<&str> = engine["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(features.contains(&"webauthn"), "features: {features:?}");
    assert!(
        !crates.iter().any(|c| c["name"] == "authkestra"),
        "the facade does not expose `webauthn`; pointing at it is a dead end"
    );

    // The consequences should mention the domain binding, which is the thing
    // that surprises people when they later change domains.
    let requirements = diff["diff"]["consequences"]["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        requirements.contains("domain"),
        "requirements should mention the domain binding: {requirements}"
    );
}

#[tokio::test]
async fn an_unknown_passkey_action_is_a_404() {
    let app = api::build_router(state().await);
    let cookie = enable(&app).await;

    let resp = action(&app, &cookie, "not_a_step", "{}").await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await["error"], "unknown_action");
}

/// A registrable suffix is a valid relying-party ID, which is what lets the
/// playground move between subdomains without invalidating every passkey.
#[tokio::test]
async fn a_parent_domain_is_a_valid_relying_party_for_a_subdomain() {
    let app = api::build_router(
        state_with_rp(RelyingParty {
            id: "authkestra.com".to_string(),
            origin: "https://play.authkestra.com".to_string(),
            name: "Authkestra".to_string(),
            extra_origins: Vec::new(),
        })
        .await,
    );
    let cookie = enable(&app).await;

    let resp = action(&app, &cookie, "register_start", "{}").await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "authkestra.com should be accepted for play.authkestra.com"
    );
    let pk = body_json(resp).await;
    assert_eq!(pk["publicKey"]["rp"]["id"], "authkestra.com");
}

#[tokio::test]
async fn additional_origins_are_accepted() {
    let app = api::build_router(
        state_with_rp(RelyingParty {
            id: "localhost".to_string(),
            origin: "http://localhost:3000".to_string(),
            name: "Playground".to_string(),
            extra_origins: vec!["http://localhost:4000".to_string()],
        })
        .await,
    );
    let cookie = enable(&app).await;

    // The relying party must still build — a bad extra origin is a clear
    // configuration error rather than a broken challenge.
    let resp = action(&app, &cookie, "register_start", "{}").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_malformed_extra_origin_fails_with_an_explanation() {
    let app = api::build_router(
        state_with_rp(RelyingParty {
            id: "localhost".to_string(),
            origin: "http://localhost:3000".to_string(),
            name: "Playground".to_string(),
            extra_origins: vec!["not a url".to_string()],
        })
        .await,
    );
    let cookie = enable(&app).await;

    let resp = action(&app, &cookie, "register_start", "{}").await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let detail = body_json(resp).await["detail"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        detail.contains("WEBAUTHN_EXTRA_ORIGINS"),
        "should name the setting at fault: {detail}"
    );
}

/// Regression test for a real failure: authenticating with a passkey enrolled
/// in Bitwarden returned "not a WebAuthn assertion: missing field
/// `clientDataJson`".
///
/// The spec field is `clientDataJSON`, with `JSON` fully capitalised.
/// `rename_all = "camelCase"` renders it `clientDataJson`, which no browser
/// sends — so every authentication was rejected before any cryptography ran.
/// Registration was unaffected, because that path deserialises into
/// `webauthn-rs`'s own type, which is why enrolment appeared to work.
#[tokio::test]
async fn an_assertion_in_the_browsers_own_shape_is_accepted_for_parsing() {
    let app = api::build_router(state().await);
    let cookie = enable(&app).await;

    // Exactly what `navigator.credentials.get()` produces once serialised —
    // note the capitalisation of `clientDataJSON`.
    let assertion = r#"{
        "id": "Y3JlZC1pZA",
        "rawId": "Y3JlZC1pZA",
        "type": "public-key",
        "response": {
            "clientDataJSON": "eyJ0eXBlIjoid2ViYXV0aG4uZ2V0In0",
            "authenticatorData": "SZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2MBAAAAAQ",
            "signature": "MEUCIQD",
            "userHandle": null
        }
    }"#;

    let resp = action(&app, &cookie, "authenticate_finish", assertion).await;

    // No ceremony was started, so this must fail at the *challenge* stage —
    // which proves the body parsed. A parse failure would be a 400
    // `invalid_value` naming the missing field instead.
    assert_eq!(
        resp.status(),
        StatusCode::GONE,
        "the assertion should parse and fail on the missing challenge, not on its shape"
    );
    assert_eq!(body_json(resp).await["error"], "ceremony_expired");
}

/// And the spelling the old code expected must NOT be accepted, or the bug
/// could quietly return behind a lenient alias.
#[tokio::test]
async fn the_incorrect_camel_case_spelling_is_rejected() {
    let app = api::build_router(state().await);
    let cookie = enable(&app).await;

    let wrong = r#"{
        "id": "Y3JlZC1pZA",
        "response": {
            "clientDataJson": "eyJ0eXBlIjoid2ViYXV0aG4uZ2V0In0",
            "authenticatorData": "SZYN5Y",
            "signature": "MEUCIQD"
        }
    }"#;

    let resp = action(&app, &cookie, "authenticate_finish", wrong).await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "invalid_value");
}
