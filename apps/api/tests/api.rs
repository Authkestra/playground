//! End-to-end tests against the real router.
//!
//! These drive the same stack `main` serves — middleware, cookies, kill switch
//! and all — rather than calling handlers directly, so wiring mistakes show up
//! here rather than in production.

use std::collections::BTreeSet;
use std::sync::Arc;

use api::engine::{EngineFactory, ProviderCredentials};
use api::killswitch::KillSwitch;
use api::routes::AppState;
use api::scenario::ScenarioRegistry;
use api::session::{DemoSessionStore, NoopJanitor, DEFAULT_TTL_HOURS};
use api::settings::Settings;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

fn settings(admin_token: Option<&str>) -> Settings {
    Settings {
        port: 0,
        cookie_secure: false,
        session_ttl_hours: DEFAULT_TTL_HOURS,
        admin_token: admin_token.map(|t| t.to_string()),
        allowed_origins: vec!["http://localhost:3000".to_string()],
        // Tests reach the router directly with no proxy in front, so they
        // identify callers by X-Forwarded-For rather than a trusted header.
        trusted_client_ip_header: None,
        relying_party: api::settings::RelyingParty {
            id: "localhost".to_string(),
            origin: "http://localhost:3000".to_string(),
            name: "test".to_string(),
        },
    }
}

async fn state_with(kill_switch: KillSwitch, admin_token: Option<&str>) -> AppState {
    let settings = Arc::new(settings(admin_token));
    let pool = api::credentials::open_in_memory()
        .await
        .expect("in-memory credential store");
    AppState {
        sessions: Arc::new(DemoSessionStore::new(
            ScenarioRegistry::with_builtins(),
            settings.session_ttl_hours,
            Arc::new(NoopJanitor),
        )),
        kill_switch: Arc::new(kill_switch),
        engines: Arc::new(EngineFactory::new(ProviderCredentials::default(), false)),
        settings,
        pool,
    }
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

#[tokio::test]
async fn try_succeeds_once_the_scenario_is_configured() {
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

    let tried = app
        .oneshot(
            req("POST", "/api/scenarios/dummy_toggle/try")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(tried.status(), StatusCode::OK);
    assert_eq!(body_json(tried).await["outcome"], "ok");
}

#[tokio::test]
async fn try_reports_not_configured_before_the_toggle_is_on() {
    let resp = app(KillSwitch::default(), None)
        .await
        .oneshot(
            req("POST", "/api/scenarios/dummy_toggle/try")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["outcome"], "not_configured");
}

// --------------------------------------------------------------- kill switch

#[tokio::test]
async fn the_kill_switch_stops_try_but_leaves_the_site_readable() {
    let ks = KillSwitch::default();
    ks.set_demo_enabled(false);
    let app = app(ks, None).await;

    let tried = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/dummy_toggle/try")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tried.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_json(tried).await["error"], "demo_disabled");

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

    // And the diff still computes, so the explainer can show consequences.
    let diffed = app
        .oneshot(
            req("GET", "/api/scenarios/dummy_toggle/diff")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(diffed.status(), StatusCode::OK);
}

#[tokio::test]
async fn one_scenario_can_be_disabled_without_touching_the_others() {
    let ks = KillSwitch::default();
    ks.set_scenario_enabled("dummy_toggle", false);
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

    // No redeploy: the very next request already sees flows disabled.
    assert!(!state.kill_switch.demo_enabled());
    let tried = app
        .oneshot(
            req("POST", "/api/scenarios/dummy_toggle/try")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tried.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// -------------------------------------------------------------- rate limiting

#[tokio::test]
async fn abusing_the_try_endpoint_is_throttled_with_a_renderable_429() {
    let app = app(KillSwitch::default(), None).await;

    let mut statuses = Vec::new();
    let mut throttled_body = None;
    for _ in 0..25 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/scenarios/dummy_toggle/try")
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
        "scripted abuse of /try was never throttled: {statuses:?}"
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
