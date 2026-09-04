//! End-to-end tests for the TOTP scenario.
//!
//! These generate *real* codes with `totp-rs` and verify them through the HTTP
//! surface, so the enrolment and verification paths are actually exercised
//! rather than only asserting that bad input fails.

use std::sync::Arc;

use api::killswitch::KillSwitch;
use api::routes::AppState;
use api::scenario::ScenarioRegistry;
use api::session::{DemoSessionStore, DEFAULT_TTL_HOURS};
use api::settings::{RelyingParty, Settings};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use totp_rs::{Algorithm, Secret, TOTP};
use tower::ServiceExt;

async fn state() -> AppState {
    let settings = Arc::new(Settings {
        port: 0,
        cookie_secure: false,
        session_ttl_hours: DEFAULT_TTL_HOURS,
        admin_token: None,
        allowed_origins: vec!["http://localhost:3000".to_string()],
        trusted_client_ip_header: None,
        relying_party: RelyingParty {
            id: "localhost".to_string(),
            origin: "http://localhost:3000".to_string(),
            name: "test".to_string(),
        },
    });
    let pool = api::credentials::open_in_memory().await.unwrap();
    AppState {
        sessions: Arc::new(DemoSessionStore::new(
            ScenarioRegistry::with_builtins(),
            settings.session_ttl_hours,
            api::credentials::janitor(pool.clone()),
        )),
        kill_switch: Arc::new(KillSwitch::default()),
        engines: Arc::new(api::engine::EngineFactory::new(
            api::engine::ProviderCredentials::default(),
            false,
        )),
        settings,
        pool,
        ceremonies: Arc::new(api::ceremony::CeremonyStore::new()),
    }
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
async fn expiring_a_session_purges_its_credentials() {
    let st = state().await;
    let pool = st.pool.clone();
    let app = api::build_router(st);

    let cookie = enable_totp(&app).await;
    provision(&app, &cookie).await;

    let session_id: uuid::Uuid = cookie.trim_start_matches("ak_demo=").parse().unwrap();

    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ak_credentials WHERE user_id = ?")
        .bind(session_id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, 1, "a secret should be stored");

    api::credentials::SqliteJanitor::purge(&pool, session_id)
        .await
        .unwrap();

    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ak_credentials WHERE user_id = ?")
        .bind(session_id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, 0, "expiry must leave no credential rows behind");
}
