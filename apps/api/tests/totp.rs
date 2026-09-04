//! End-to-end tests for the TOTP scenario.
//!
//! These generate *real* codes with `totp-rs` and verify them through the HTTP
//! surface, so the enrolment and verification paths are actually exercised
//! rather than only asserting that bad input fails.

use std::sync::Arc;

use api::killswitch::KillSwitch;
use api::routes::AppState;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use totp_rs::{Algorithm, Secret, TOTP};
use tower::ServiceExt;

async fn state() -> AppState {
    api::testing::test_state(KillSwitch::default(), None)
}

fn req(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-forwarded-for", "203.0.113.5")
        .header(header::CONTENT_TYPE, "application/json")
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).expect("JSON response")
}

fn cookie_of(resp: &axum::response::Response) -> String {
    resp.headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .expect("session cookie")
        .to_string()
}

/// Enable TOTP and return the session cookie.
async fn enable_totp(app: &axum::Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/totp/configure")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    cookie_of(&resp)
}

async fn provision(app: &axum::Router, cookie: &str) -> (String, String) {
    let resp = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/totp/action/provision")
                .header(header::COOKIE, cookie)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = body_json(resp).await;
    (
        j["secret"].as_str().unwrap().to_string(),
        j["uri"].as_str().unwrap().to_string(),
    )
}

/// Build the same TOTP parameters the engine uses, so generated codes are valid.
fn code_for(secret_b32: &str) -> String {
    let bytes = Secret::Encoded(secret_b32.to_string()).to_bytes().unwrap();
    TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes, None, "demo".to_string())
        .unwrap()
        .generate_current()
        .unwrap()
}

async fn verify(app: &axum::Router, cookie: &str, code: &str) -> Value {
    let resp = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/totp/action/verify")
                .header(header::COOKIE, cookie)
                .body(Body::from(format!(r#"{{"code":"{code}"}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "verify should answer, not error"
    );
    body_json(resp).await
}

#[tokio::test]
async fn provisioning_returns_a_scannable_otpauth_uri() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;

    let (secret, uri) = provision(&app, &cookie).await;

    assert!(
        !secret.is_empty(),
        "a base32 secret is shown for manual entry"
    );
    assert!(uri.starts_with("otpauth://totp/"), "uri was {uri}");
    assert!(uri.contains("secret="), "uri must carry the secret: {uri}");
    assert!(
        uri.contains("issuer=Authkestra") || uri.contains("issuer=Authkestra%20Playground"),
        "uri should name the issuer: {uri}"
    );
}

/// The headline path: a real code from the provisioned secret is accepted.
#[tokio::test]
async fn a_real_code_from_the_provisioned_secret_verifies() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;
    let (secret, _) = provision(&app, &cookie).await;

    let result = verify(&app, &cookie, &code_for(&secret)).await;

    assert_eq!(result["verified"], true, "detail: {}", result["detail"]);
}

#[tokio::test]
async fn a_wrong_code_fails_cleanly_rather_than_erroring() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;
    provision(&app, &cookie).await;

    let result = verify(&app, &cookie, "000000").await;

    assert_eq!(result["verified"], false);
    assert!(
        result["detail"].as_str().is_some_and(|d| !d.is_empty()),
        "a rejection must explain itself"
    );
}

#[tokio::test]
async fn a_malformed_code_is_rejected_without_touching_the_engine() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;
    provision(&app, &cookie).await;

    for bad in ["12345", "abcdef", "1234567", ""] {
        let result = verify(&app, &cookie, bad).await;
        assert_eq!(result["verified"], false, "{bad:?} should not verify");
    }
}

/// The framework tracks the last used step, so a captured code cannot be
/// replayed within its window.
#[tokio::test]
async fn the_same_code_cannot_be_used_twice() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;
    let (secret, _) = provision(&app, &cookie).await;
    let code = code_for(&secret);

    let first = verify(&app, &cookie, &code).await;
    assert_eq!(first["verified"], true, "first use should succeed");

    let second = verify(&app, &cookie, &code).await;
    assert_eq!(
        second["verified"], false,
        "replaying a code must be rejected"
    );
}

#[tokio::test]
async fn re_provisioning_invalidates_the_previous_secret() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;
    let (first_secret, _) = provision(&app, &cookie).await;
    let (second_secret, _) = provision(&app, &cookie).await;
    assert_ne!(first_secret, second_secret);

    // A visitor who re-scanned holds only the newer secret; the older one must
    // no longer work, or they would be verifying against a ghost.
    let stale = verify(&app, &cookie, &code_for(&first_secret)).await;
    assert_eq!(stale["verified"], false);

    let fresh = verify(&app, &cookie, &code_for(&second_secret)).await;
    assert_eq!(fresh["verified"], true);
}

#[tokio::test]
async fn two_visitors_get_independent_secrets() {
    let app = api::build_router(state().await);

    let cookie_a = enable_totp(&app).await;
    let (secret_a, _) = provision(&app, &cookie_a).await;
    let cookie_b = enable_totp(&app).await;
    let (secret_b, _) = provision(&app, &cookie_b).await;

    assert_ne!(cookie_a, cookie_b);
    assert_ne!(secret_a, secret_b, "secrets must not be shared");

    // A's code must not authenticate B.
    let cross = verify(&app, &cookie_b, &code_for(&secret_a)).await;
    assert_eq!(cross["verified"], false, "visitor A's code worked for B");
}

#[tokio::test]
async fn ceremonies_are_refused_until_the_scenario_is_enabled() {
    let app = api::build_router(state().await);

    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/totp/action/provision")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_unknown_action_is_a_404() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;

    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/totp/action/nonsense")
                .header(header::COOKIE, &cookie)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await["error"], "unknown_action");
}

#[tokio::test]
async fn the_kill_switch_stops_ceremonies() {
    let mut st = state().await;
    let ks = KillSwitch::default();
    ks.set_scenario_enabled("totp", false);
    st.kill_switch = Arc::new(ks);
    let app = api::build_router(st);

    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/totp/action/provision")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_json(resp).await["error"], "demo_disabled");
}

#[tokio::test]
async fn try_reports_enrolment_state() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;

    let before = body_json(
        app.clone()
            .oneshot(
                req("POST", "/api/scenarios/totp/try")
                    .header(header::COOKIE, &cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(before["outcome"], "not_configured");

    provision(&app, &cookie).await;

    let after = body_json(
        app.oneshot(
            req("POST", "/api/scenarios/totp/try")
                .header(header::COOKIE, &cookie)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(after["outcome"], "ok");
}

#[tokio::test]
async fn the_diff_names_the_engine_crate_not_the_facade() {
    let app = api::build_router(state().await);

    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/totp/configure")
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
    assert!(features.contains(&"totp"), "features: {features:?}");

    // The facade does not expose `totp`; naming it would send a reader down a
    // dead end. See docs/decisions/0001.
    assert!(
        !crates.iter().any(|c| c["name"] == "authkestra"),
        "the diff must not point at the facade for a TOTP project"
    );
}

/// Expiry must take the session's credentials with it — a demo that leaves
/// TOTP secrets behind for every visitor who ever passed through is a liability.
#[tokio::test]
async fn resetting_a_session_purges_its_credentials() {
    let st = state().await;
    let credentials = st.credentials.clone();
    let sessions = st.sessions.clone();
    let app = api::build_router(st);

    let cookie = enable_totp(&app).await;
    provision(&app, &cookie).await;

    let session_id: uuid::Uuid = cookie.trim_start_matches("ak_demo=").parse().unwrap();
    let user_id = session_id.to_string();

    assert_eq!(
        credentials.count(&user_id, "totp").await.unwrap(),
        1,
        "a secret should be stored"
    );

    sessions.reset(session_id).await.unwrap();

    assert_eq!(
        credentials.count(&user_id, "totp").await.unwrap(),
        0,
        "a reset must leave no credentials behind"
    );
}

/// The mechanism that actually runs in production: nothing sweeps, the store's
/// TTL drops the key. A visitor's secret must not outlive their session.
#[tokio::test]
async fn credentials_expire_on_their_own_without_anything_sweeping() {
    use api::store::{KeyValue, MemoryKv};
    use std::sync::Arc;
    use std::time::Duration;

    let kv: Arc<dyn KeyValue> = Arc::new(MemoryKv::new());
    let credentials =
        api::credentials::KvCredentialStore::new(kv.clone(), Duration::from_millis(40));

    use authkestra_engine::auth::store::CredentialStore;
    credentials
        .save_credential("session-x", "totp", serde_json::json!({ "secret": "S" }))
        .await
        .unwrap();
    assert_eq!(credentials.count("session-x", "totp").await.unwrap(), 1);

    tokio::time::sleep(Duration::from_millis(80)).await;

    assert_eq!(
        credentials.count("session-x", "totp").await.unwrap(),
        0,
        "the store's TTL should have dropped it with no sweeper involved"
    );
}

/// The flow log is the thing a playground can show that docs cannot, so it has
/// to actually narrate what happened — in order, and readably.
#[tokio::test]
async fn the_flow_log_narrates_a_real_totp_attempt() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;

    // Nothing has happened yet.
    let empty = events(&app, &cookie).await;
    assert!(empty.is_empty(), "a fresh session has no flow log");

    let (secret, _) = provision(&app, &cookie).await;
    verify(&app, &cookie, "12345").await; // malformed
    verify(&app, &cookie, "000000").await; // wrong
    let code = code_for(&secret);
    verify(&app, &cookie, &code).await; // correct
    verify(&app, &cookie, &code).await; // replayed

    let log = events(&app, &cookie).await;
    let steps: Vec<&str> = log.iter().map(|e| e["step"].as_str().unwrap()).collect();

    assert_eq!(
        steps,
        vec![
            "secret generated",
            "malformed code",
            "code rejected",
            "code verified",
            "code rejected",
        ],
        "the log should read as the sequence of what happened"
    );

    let levels: Vec<&str> = log.iter().map(|e| e["level"].as_str().unwrap()).collect();
    assert_eq!(
        levels,
        vec!["info", "rejected", "rejected", "success", "rejected"],
        "a wrong code is `rejected`, not `failed` — it is a normal outcome"
    );

    // Every entry explains itself, and the useful facts are surfaced.
    for e in &log {
        assert!(
            e["detail"].as_str().is_some_and(|d| d.len() > 20),
            "each step needs a readable explanation: {e}"
        );
        assert!(e["scenario"] == "totp");
        assert!(e["at"].as_str().is_some());
    }

    let generated = &log[0];
    let facts: Vec<&str> = generated["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert!(facts.contains(&"algorithm"), "{facts:?}");
    assert!(facts.contains(&"period"), "{facts:?}");
}

/// A secret must never appear in something shown to the visitor.
#[tokio::test]
async fn the_flow_log_never_contains_the_secret() {
    let app = api::build_router(state().await);
    let cookie = enable_totp(&app).await;
    let (secret, _) = provision(&app, &cookie).await;
    verify(&app, &cookie, &code_for(&secret)).await;

    let raw = serde_json::to_string(&events(&app, &cookie).await).unwrap();
    assert!(
        !raw.contains(&secret),
        "the shared secret leaked into the visitor-facing flow log"
    );
}

#[tokio::test]
async fn resetting_clears_the_flow_log() {
    let st = state().await;
    let sessions = st.sessions.clone();
    let app = api::build_router(st);
    let cookie = enable_totp(&app).await;
    provision(&app, &cookie).await;
    assert!(!events(&app, &cookie).await.is_empty());

    let session_id: uuid::Uuid = cookie.trim_start_matches("ak_demo=").parse().unwrap();
    sessions.reset(session_id).await.unwrap();
    // Reset through the store clears credentials; the route also clears events.
    let resp = app
        .clone()
        .oneshot(
            req("POST", "/api/session/reset")
                .header(header::COOKIE, &cookie)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    assert!(
        events(&app, &cookie).await.is_empty(),
        "a reset should leave a clean slate"
    );
}

/// One visitor must never read another's flow log.
#[tokio::test]
async fn flow_logs_are_per_visitor() {
    let app = api::build_router(state().await);
    let cookie_a = enable_totp(&app).await;
    provision(&app, &cookie_a).await;

    let cookie_b = enable_totp(&app).await;
    assert_ne!(cookie_a, cookie_b);

    assert!(!events(&app, &cookie_a).await.is_empty());
    assert!(events(&app, &cookie_b).await.is_empty());
}

async fn events(app: &axum::Router, cookie: &str) -> Vec<Value> {
    let resp = app
        .clone()
        .oneshot(
            req("GET", "/api/session/events")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp)
        .await
        .as_array()
        .cloned()
        .unwrap_or_default()
}
