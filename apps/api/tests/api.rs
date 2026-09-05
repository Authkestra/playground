//! End-to-end tests against the real router.
//!
//! These drive the same stack `main` serves — middleware, cookies, kill switch
//! and all — rather than calling handlers directly, so wiring mistakes show up
//! here rather than in production.

use std::collections::BTreeSet;
use std::sync::Arc;

use api::killswitch::KillSwitch;
use api::routes::AppState;
use api::settings::Settings;
use api::testing::{test_settings, test_state_with_settings};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

fn settings(admin_token: Option<&str>) -> Settings {
    test_settings(admin_token)
}

async fn state_with(kill_switch: KillSwitch, admin_token: Option<&str>) -> AppState {
    test_state_with_settings(kill_switch, settings(admin_token))
}

async fn app(kill_switch: KillSwitch, admin_token: Option<&str>) -> axum::Router {
    api::build_router(state_with(kill_switch, admin_token).await)
}

/// The rate limiter keys on client IP, so tests must present one.
fn req(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-forwarded-for", "203.0.113.1")
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).expect("response was not JSON")
}

fn session_cookie(resp: &axum::response::Response) -> Option<String> {
    resp.headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(|s| s.to_string())
}

#[tokio::test]
async fn health_reports_status_and_kill_switch_state() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(req("GET", "/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
    assert_eq!(json["demo_enabled"], true);
}

#[tokio::test]
async fn first_request_issues_an_http_only_session_cookie() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(req("GET", "/api/session").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let raw = resp
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie")
        .to_str()
        .unwrap()
        .to_string();
    assert!(raw.contains("ak_demo="), "cookie name");
    assert!(raw.contains("HttpOnly"), "cookie must be HttpOnly: {raw}");
}

#[tokio::test]
async fn scenarios_render_from_data_with_both_control_shapes() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(req("GET", "/api/scenarios").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let json = body_json(resp).await;
    let specs = json.as_array().expect("array of specs");
    assert!(!specs.is_empty(), "the registry should expose scenarios");

    // Asserted by capability rather than count, so registering a new scenario
    // doesn't break an unrelated test.
    let kinds: BTreeSet<&str> = specs
        .iter()
        .map(|s| s["control"]["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains("toggle"));
    assert!(kinds.contains("select_one"));
    assert!(specs.iter().all(|s| s["available"] == true));

    // The real TOTP scenario is registered and advertises its ceremony steps,
    // so the frontend can drive it from data.
    let totp = specs
        .iter()
        .find(|s| s["id"] == "totp")
        .expect("totp scenario registered");
    let actions: BTreeSet<&str> = totp["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert!(actions.contains("provision"), "actions: {actions:?}");
    assert!(actions.contains("verify"), "actions: {actions:?}");
}

/// P1's headline acceptance: configure a scenario, get back a real diff.
#[tokio::test]
async fn configuring_a_scenario_returns_config_and_a_real_diff() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(
            req("POST", "/api/scenarios/dummy_toggle/configure")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;

    assert_eq!(json["config"]["scenarios"]["dummy_toggle"]["enabled"], true);

    let entries = json["diff"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"], "changed");
    assert_eq!(entries[0]["before"], "false");
    assert_eq!(entries[0]["after"], "true");

    // The diff must name concrete consequences, not just the flipped bool.
    let routes = json["diff"]["consequences"]["routes"].as_array().unwrap();
    assert!(routes.iter().any(|r| r == "POST /auth/example"));
    let crates = json["diff"]["consequences"]["crates"].as_array().unwrap();
    assert!(crates.iter().any(|c| c["name"] == "authkestra-engine"));
}

#[tokio::test]
async fn configuration_persists_across_requests_on_the_same_cookie() {
    let app = app(KillSwitch::default(), None).await;

    let first = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/dummy_toggle/configure")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = session_cookie(&first).expect("cookie");

    let second = app
        .oneshot(
            req("GET", "/api/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let json = body_json(second).await;
    assert_eq!(
        json["config"]["scenarios"]["dummy_toggle"]["enabled"], true,
        "the visitor's configuration did not survive the round-trip"
    );
}

/// Two visitors must never see each other's configuration.
#[tokio::test]
async fn a_second_visitor_gets_an_independent_configuration() {
    let app = app(KillSwitch::default(), None).await;

    let first = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/dummy_toggle/configure")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie_a = session_cookie(&first).expect("cookie");

    // A visitor with no cookie at all.
    let second = app
        .oneshot(req("GET", "/api/session").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let cookie_b = session_cookie(&second).expect("cookie");
    assert_ne!(cookie_a, cookie_b);

    let json = body_json(second).await;
    assert_eq!(
        json["config"]["scenarios"]["dummy_toggle"]["enabled"], false,
        "visitor B saw visitor A's configuration"
    );
}

#[tokio::test]
async fn reset_returns_the_visitor_to_defaults() {
    let app = app(KillSwitch::default(), None).await;

    let configured = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/dummy_toggle/configure")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = session_cookie(&configured).expect("cookie");

    let reset = app
        .oneshot(
            req("POST", "/api/session/reset")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let json = body_json(reset).await;
    assert_eq!(
        json["config"]["scenarios"]["dummy_toggle"]["enabled"],
        false
    );
}

#[tokio::test]
async fn an_unknown_scenario_is_a_404_not_a_panic() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(
            req("POST", "/api/scenarios/nope/configure")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await["error"], "unknown_scenario");
}

#[tokio::test]
async fn a_value_of_the_wrong_shape_is_rejected() {
    // A select_one value posted at a toggle control.
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(
            req("POST", "/api/scenarios/dummy_toggle/configure")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"value":{"kind":"select_one","selected":"alpha"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "invalid_value");
}

#[tokio::test]
async fn an_unknown_option_is_rejected() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(
            req("POST", "/api/scenarios/dummy_provider/configure")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"value":{"kind":"select_one","selected":"nonexistent"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// --------------------------------------------------------------- kill switch

#[tokio::test]
async fn the_kill_switch_leaves_the_site_readable() {
    use api::killswitch::KillSwitchState;

    let switch_state = KillSwitchState {
        demo_enabled: false,
        ..Default::default()
    };
    let ks = KillSwitch::new(None, switch_state);
    let app = app(ks, None).await;

    // Explainer-only mode still needs real content, so listing scenarios must
    // keep working — flagged unavailable rather than withheld.
    let listed = app
        .clone()
        .oneshot(req("GET", "/api/scenarios").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let specs = body_json(listed).await;
    assert!(specs
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["available"] == false));
}

#[tokio::test]
async fn one_scenario_can_be_disabled_without_touching_the_others() {
    use api::killswitch::KillSwitchState;

    let mut switch_state = KillSwitchState::default();
    switch_state
        .disabled_scenarios
        .insert("dummy_toggle".to_string());
    let ks = KillSwitch::new(None, switch_state);
    let app = app(ks, None).await;

    let specs = body_json(
        app.oneshot(req("GET", "/api/scenarios").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;

    let by_id: std::collections::HashMap<&str, bool> = specs
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["id"].as_str().unwrap(), s["available"].as_bool().unwrap()))
        .collect();

    assert!(
        !by_id["dummy_toggle"],
        "the disabled scenario should be unavailable"
    );
    assert!(
        by_id["dummy_provider"],
        "the other scenario should be untouched"
    );
}

#[tokio::test]
async fn admin_routes_are_absent_when_no_token_is_configured() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(
            req("POST", "/admin/kill-switch")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"demo_enabled":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Not mounted at all — a missing secret must never mean an open switch.
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_requires_the_bearer_token() {
    let resp = app(KillSwitch::default(), Some("s3cret"))
        .await
        .oneshot(
            req("POST", "/admin/kill-switch")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"demo_enabled":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_can_flip_the_switch_at_runtime() {
    let state = state_with(KillSwitch::default(), Some("s3cret")).await;
    let app = api::build_router(state.clone());

    let resp = app
        .clone()
        .oneshot(
            req("POST", "/admin/kill-switch")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .body(Body::from(r#"{"demo_enabled":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // No redeploy: the very next request already sees the kill switch.
    let snap = state.kill_switch.snapshot().await;
    assert!(!snap.demo_enabled());
}

// -------------------------------------------------------------- rate limiting

#[tokio::test]
async fn abusing_expensive_endpoints_is_throttled_with_a_renderable_429() {
    let app = app(KillSwitch::default(), None).await;

    let mut statuses = Vec::new();
    let mut throttled_body = None;
    for _ in 0..25 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/scenarios/dummy_toggle/action/does_not_matter")
                    // A single abusive client IP.
                    .header("x-forwarded-for", "198.51.100.7")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS && throttled_body.is_none() {
            throttled_body = Some(body_json(resp).await);
        }
        statuses.push(status);
    }

    assert!(
        statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
        "scripted abuse of expensive endpoints was never throttled: {statuses:?}"
    );

    // The UI renders this, so it must be JSON with the documented shape.
    let body = throttled_body.expect("a 429 body");
    assert_eq!(body["error"], "rate_limited");
    assert!(
        body["detail"].as_str().is_some_and(|d| !d.is_empty()),
        "429 must carry a renderable detail message"
    );
}

#[tokio::test]
async fn normal_interactive_use_is_not_throttled() {
    let app = app(KillSwitch::default(), None).await;

    // A handful of ordinary reads from one visitor must all succeed.
    for i in 0..10 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/scenarios")
                    .header("x-forwarded-for", "198.51.100.42")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "request {i} was throttled");
    }
}

/// Regression test for the deployed path.
///
/// `oneshot` on a bare `Router` has no `ConnectInfo`, which is the same
/// condition as a platform health probe. With a plain `SmartIpKeyExtractor`, a request that
/// also lacks `X-Forwarded-For` — an internal health probe, say — fails key
/// extraction and is rendered as a 500, taking the endpoint down with it.
///
/// It must be rate limited under a shared bucket instead, never rejected.
#[tokio::test]
async fn a_request_with_no_forwarding_header_and_no_connect_info_still_works() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "an unidentifiable caller must not 500 the endpoint"
    );
    assert_eq!(body_json(resp).await["status"], "ok");
}

/// CORS here is credentialed (the session rides in a cookie), so an origin that
/// fails to match produces no `access-control-allow-origin` header and the
/// browser blocks the request — with no server-side error. These pin the two
/// mistakes that cause it.
#[tokio::test]
async fn an_allowed_origin_is_echoed_back() {
    let settings = Settings {
        allowed_origins: vec!["https://example.test".to_string()],
        ..settings(None)
    };
    let mut st = state_with(KillSwitch::default(), None).await;
    st.settings = Arc::new(settings);

    let resp = api::build_router(st)
        .oneshot(
            req("GET", "/api/session")
                .header(header::ORIGIN, "https://example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok()),
        Some("https://example.test"),
        "an allowed origin must be echoed, or the browser blocks the request"
    );
}

#[tokio::test]
async fn a_trailing_slash_in_the_allow_list_still_matches() {
    // `Origin` headers never carry a trailing slash, so an entry written as
    // "https://example.test/" would otherwise match nothing at all.
    std::env::set_var("ALLOWED_ORIGINS", "https://example.test/");
    let parsed = Settings::from_env();
    std::env::remove_var("ALLOWED_ORIGINS");

    assert_eq!(
        parsed.allowed_origins,
        vec!["https://example.test".to_string()]
    );
}

#[tokio::test]
async fn an_origin_not_on_the_list_is_not_echoed() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(
            req("GET", "/api/session")
                .header(header::ORIGIN, "https://evil.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "an unlisted origin must not be granted access"
    );
}

/// Regression test for a cross-site session that silently never persisted.
///
/// `SameSite=Lax` is not sent on cross-site fetches, so with the frontend and
/// API on different registrable domains every request arrived without a cookie,
/// got a fresh session, and the visitor's toggles appeared to do nothing —
/// `try` kept reporting "not configured" however many times they clicked.
#[tokio::test]
async fn a_secure_deployment_defaults_to_a_cross_site_capable_cookie() {
    let settings = Settings {
        cookie_secure: true,
        cookie_same_site: api::settings::CookieSameSite::from_env(true),
        ..test_settings(None)
    };
    let app = api::build_router(test_state_with_settings(KillSwitch::default(), settings));

    let resp = app
        .oneshot(req("GET", "/api/session").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let raw = resp
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie")
        .to_str()
        .unwrap()
        .to_string();

    assert!(
        raw.contains("SameSite=None"),
        "a cross-site deployment needs SameSite=None or the cookie is never sent back: {raw}"
    );
    assert!(
        raw.contains("Secure"),
        "browsers reject SameSite=None without Secure: {raw}"
    );
    assert!(raw.contains("HttpOnly"), "{raw}");
}

#[tokio::test]
async fn local_development_keeps_the_stricter_lax_policy() {
    // Not secure => same-site localhost => Lax is correct and stricter.
    let policy = api::settings::CookieSameSite::from_env(false);
    assert_eq!(policy, api::settings::CookieSameSite::Lax);
}

/// The property the bug actually broke: a returning cookie must resume the
/// same session rather than minting a new one.
#[tokio::test]
async fn a_returned_cookie_resumes_the_same_session() {
    let app = app(KillSwitch::default(), None).await;

    let first = app
        .clone()
        .oneshot(req("GET", "/api/session").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let cookie = session_cookie(&first).expect("cookie");
    let first_id = body_json(first).await["id"].as_str().unwrap().to_string();

    let second = app
        .oneshot(
            req("GET", "/api/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let second_id = body_json(second).await["id"].as_str().unwrap().to_string();

    assert_eq!(
        first_id, second_id,
        "the same cookie must resume the same session, not create another"
    );
}

// ------------------------------------------------- P4 #31: the download

/// Read a zip response back into (path, contents) pairs.
async fn archive_entries(resp: axum::response::Response) -> Vec<(String, String)> {
    use std::io::{Cursor, Read};

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).expect("a valid zip");
    (0..archive.len())
        .map(|i| {
            let mut entry = archive.by_index(i).expect("entry");
            let name = entry.name().to_string();
            let mut contents = String::new();
            entry.read_to_string(&mut contents).expect("utf-8");
            (name, contents)
        })
        .collect()
}

#[tokio::test]
async fn the_starter_kit_downloads_as_a_named_zip() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(req("GET", "/api/starter-kit").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "application/zip");

    let disposition = resp.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        disposition.starts_with("attachment; filename=\"authkestra-starter-"),
        "{disposition}"
    );
    assert!(disposition.ends_with(".zip\""), "{disposition}");

    let entries = archive_entries(resp).await;
    for expected in ["Cargo.toml", "src/main.rs", "README.md", ".env.example"] {
        assert!(
            entries.iter().any(|(name, _)| name.ends_with(expected)),
            "{expected} missing from {:?}",
            entries.iter().map(|(n, _)| n).collect::<Vec<_>>()
        );
    }
}

/// The acceptance criterion: the download matches what the visitor configured,
/// not some default. Configure on one request, download on the next, same
/// cookie.
#[tokio::test]
async fn the_download_reflects_the_session_configuration() {
    let app = app(KillSwitch::default(), None).await;

    let configured = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/passkeys/configure")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(configured.status(), StatusCode::OK);
    let cookie = session_cookie(&configured).expect("cookie");

    let resp = app
        .oneshot(
            req("GET", "/api/starter-kit")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let disposition = resp.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        disposition.contains("passkeys"),
        "the name should reflect the selection: {disposition}"
    );

    let entries = archive_entries(resp).await;
    let manifest = entries
        .iter()
        .find(|(name, _)| name.ends_with("Cargo.toml"))
        .expect("a manifest");
    assert!(
        manifest.1.contains("webauthn"),
        "passkeys were selected but the manifest lacks the feature:\n{}",
        manifest.1
    );
}

/// A fresh visitor and a configured one must not receive the same archive.
#[tokio::test]
async fn an_unconfigured_visitor_gets_the_base_project() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(req("GET", "/api/starter-kit").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let disposition = resp.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .to_string();
    assert!(disposition.contains("base"), "{disposition}");
}

/// Generating a project is the most expensive request this service serves, so
/// it must sit behind the tighter bucket rather than the default one.
#[tokio::test]
async fn the_download_is_rate_limited() {
    let app = app(KillSwitch::default(), None).await;
    let mut limited = false;

    // The sensitive bucket allows a burst of 10; the standard one allows 30.
    // Exceeding the smaller budget proves which one is in front of this route.
    for _ in 0..15 {
        let resp = app
            .clone()
            .oneshot(
                req("GET", "/api/starter-kit")
                    .header("x-forwarded-for", "198.51.100.77")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
    }

    assert!(limited, "the download endpoint is not rate limited");
}

// --------------------------------------------------------- kill switch durability

/// Prove that kill switch state survives a restart.
///
/// This is the core durability requirement: flip the switch off, then simulate
/// a cold start by building a fresh AppState with the same store, and verify
/// the switch is still off. Without this, the service would silently reset to
/// DEMO_ENABLED on restart (as it did before moving state to the store).
#[tokio::test]
async fn kill_switch_state_survives_a_cold_start() {
    use api::killswitch::KillSwitchState;

    // Build an initial state with a shared store.
    let shared_store = api::testing::shared_store();
    let init_ks = api::testing::test_state_with_shared_store(
        shared_store.clone(),
        KillSwitchState::default(),
    );

    // Check the initial state: should be enabled (the seed value).
    let snap = init_ks.kill_switch.snapshot().await;
    assert!(
        snap.demo_enabled(),
        "fresh state should seed with DEMO_ENABLED=true from env"
    );

    // Flip it off by calling set_state directly on the kill switch.
    let off_state = KillSwitchState {
        demo_enabled: false,
        ..Default::default()
    };
    init_ks.kill_switch.set_state(off_state).await;

    // Build a fresh state from the same store (simulating a cold start).
    // The kill switch will read from the store and should find it still off.
    let restarted_ks =
        api::testing::test_state_with_shared_store(shared_store, KillSwitchState::default());

    // Now the switch should still be off, because the store persisted it.
    let snap_after_restart = restarted_ks.kill_switch.snapshot().await;
    assert!(
        !snap_after_restart.demo_enabled(),
        "kill switch state should survive a cold start; was resurrected from environment seed"
    );
}

/// Prove that per-scenario disables also survive restarts.
#[tokio::test]
async fn per_scenario_disable_survives_a_cold_start() {
    use api::killswitch::KillSwitchState;

    let shared_store = api::testing::shared_store();

    // Build initial state.
    let init_ks = api::testing::test_state_with_shared_store(
        shared_store.clone(),
        KillSwitchState::default(),
    );

    // Disable a specific scenario.
    let mut disabled_state = KillSwitchState::default();
    disabled_state
        .disabled_scenarios
        .insert("oauth".to_string());
    init_ks.kill_switch.set_state(disabled_state).await;

    // Restart: build fresh state with the same store.
    let restarted_ks =
        api::testing::test_state_with_shared_store(shared_store, KillSwitchState::default());

    // Verify the scenario is still disabled.
    let snap = restarted_ks.kill_switch.snapshot().await;
    assert!(
        !snap.scenario_enabled("oauth"),
        "per-scenario disable should survive a cold start"
    );
    assert!(
        snap.scenario_enabled("dummy_toggle"),
        "other scenarios should remain enabled"
    );
}

/// The same durability property, but driven the way an operator actually
/// drives it: over HTTP, through `/admin/kill-switch`.
///
/// The direct-`set_state` tests above would still pass if the admin handler
/// mutated a local copy and never persisted it, which is exactly the regression
/// worth guarding — the endpoint is the only way the switch is ever flipped in
/// production.
#[tokio::test]
async fn a_switch_flipped_over_http_survives_a_cold_start() {
    use api::killswitch::KillSwitchState;

    let store = api::testing::shared_store();
    let before = api::testing::test_state_with_shared_store_and_admin(
        store.clone(),
        KillSwitchState::default(),
        Some("s3cret"),
    );

    let resp = api::build_router(before)
        .oneshot(
            req("POST", "/admin/kill-switch")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .body(Body::from(r#"{"demo_enabled":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // A fresh AppState over the same store: what a Render cold start produces,
    // seeded from an environment that still says enabled.
    let after = api::testing::test_state_with_shared_store_and_admin(
        store,
        KillSwitchState::default(),
        Some("s3cret"),
    );
    assert!(
        !after.kill_switch.snapshot().await.demo_enabled(),
        "a switch flipped through the admin endpoint must outlive the process that served it"
    );
}
