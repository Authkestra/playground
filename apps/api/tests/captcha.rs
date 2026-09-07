//! End-to-end tests for the bot-protection scenario.
//!
//! **Nothing here reaches a captcha provider.** The unit tests in
//! `scenario::captcha` cover the verdict rules against every shape of answer a
//! provider can give; what needs an HTTP surface is the layer in front of them
//! — which requests are refused before a secret is ever spent, and what a
//! deployment without keys offers a visitor.
//!
//! That boundary is the point of this file. Every test below asserts a
//! rejection that happens *before* the network call, so the suite stays
//! offline and deterministic while still covering the paths that decide
//! whether a third party gets called at all.

use std::sync::Arc;

use api::killswitch::KillSwitch;
use api::routes::AppState;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// Every provider configured, with fictional keys.
const KEYS: &[(&str, &str, &str)] = &[
    ("turnstile", "ts-site", "ts-secret"),
    ("hcaptcha", "hc-site", "hc-secret"),
    ("recaptcha", "rc-site", "rc-secret"),
];

async fn state() -> AppState {
    api::testing::test_state_with_all_credentials(KillSwitch::default(), &[], KEYS)
}

/// A deployment that has not registered anything yet — the live case until
/// roadmap #7 is done.
async fn state_without_keys() -> AppState {
    api::testing::test_state(KillSwitch::default(), None)
}

fn req(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-forwarded-for", "203.0.113.42")
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

/// Select the given providers and return the session cookie.
async fn select(app: &axum::Router, providers: &[&str]) -> String {
    let selected = serde_json::to_string(providers).unwrap();
    let resp = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/captcha/configure")
                .body(Body::from(format!(
                    r#"{{"value":{{"kind":"select_many","selected":{selected}}}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    cookie_of(&resp)
}

async fn action(app: &axum::Router, cookie: &str, name: &str, body: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            req("POST", &format!("/api/scenarios/captcha/action/{name}"))
                .header(header::COOKIE, cookie)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    (status, body_json(resp).await)
}

// ------------------------------------------------------- what a visitor sees

#[tokio::test]
async fn a_deployment_with_keys_offers_every_configured_provider() {
    let app = api::build_router(state().await);
    let resp = app
        .clone()
        .oneshot(req("GET", "/api/scenarios").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let specs = body_json(resp).await;

    let captcha = specs
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "captcha")
        .expect("the captcha scenario is published");

    assert_eq!(captcha["control"]["kind"], "select_many");
    let ids: Vec<&str> = captcha["control"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["turnstile", "hcaptcha", "recaptcha"]);
    assert!(
        captcha["unavailable_reason"].is_null(),
        "a configured deployment has nothing to apologise for"
    );
}

/// The state this deployment is actually in until someone registers the keys
/// (roadmap #7). It must read as "not set up here" rather than as a bug: an
/// empty control with no explanation is the dead end `unavailable_reason`
/// exists to prevent.
#[tokio::test]
async fn a_deployment_without_keys_explains_the_empty_control() {
    let app = api::build_router(state_without_keys().await);
    let resp = app
        .clone()
        .oneshot(req("GET", "/api/scenarios").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let specs = body_json(resp).await;

    let captcha = specs
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "captcha")
        .expect("the scenario is still published, not hidden");

    assert!(
        captcha["control"]["options"].as_array().unwrap().is_empty(),
        "a provider with no keys must never be offered"
    );
    let reason = captcha["unavailable_reason"]
        .as_str()
        .expect("an empty control needs a reason");
    assert!(reason.contains("configured"), "{reason}");
    // Still `available`: the kill switch is a different thing, and conflating
    // them would hide whichever one is not the cause.
    assert_eq!(captcha["available"], true);
}

// ------------------------------------------------------------- the widget step

#[tokio::test]
async fn the_widget_step_returns_site_keys_for_the_selected_providers_only() {
    let app = api::build_router(state().await);
    let cookie = select(&app, &["hcaptcha"]).await;

    let (status, body) = action(&app, &cookie, "widget", "{}").await;
    assert_eq!(status, StatusCode::OK);

    let widgets = body["widgets"].as_array().unwrap();
    assert_eq!(widgets.len(), 1, "{body}");
    assert_eq!(widgets[0]["provider"], "hcaptcha");
    assert_eq!(widgets[0]["label"], "hCaptcha");
    assert_eq!(widgets[0]["site_key"], "hc-site");
}

/// The site key is public and the secret is not. The one place both exist side
/// by side is this response, so it is worth asserting the secret is not in it.
#[tokio::test]
async fn the_widget_step_never_returns_a_secret() {
    let app = api::build_router(state().await);
    let cookie = select(&app, &["turnstile", "hcaptcha", "recaptcha"]).await;

    let (_, body) = action(&app, &cookie, "widget", "{}").await;
    let serialised = body.to_string();
    for (_, _, secret) in KEYS {
        assert!(
            !serialised.contains(secret),
            "a secret reached the browser: {serialised}"
        );
    }
}

#[tokio::test]
async fn nothing_works_until_a_provider_is_chosen() {
    let app = api::build_router(state().await);
    let cookie = select(&app, &[]).await;

    for (name, body) in [("widget", "{}"), ("verify", r#"{"provider":"turnstile"}"#)] {
        let (status, payload) = action(&app, &cookie, name, body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{name}: a scenario that is switched off is a caller mistake, not a server fault"
        );
        assert_eq!(payload["error"], "ceremony_rejected");
    }
}

// ------------------------------------------------------------- the verify step

/// The guard that matters. Without it a caller could name any provider and
/// choose which of this deployment's secrets to spend — burning quota on a
/// provider nobody selected, and against an endpoint they picked.
#[tokio::test]
async fn a_provider_the_visitor_did_not_select_is_refused_before_any_secret_is_spent() {
    let app = api::build_router(state().await);
    let cookie = select(&app, &["turnstile"]).await;

    let (status, body) = action(
        &app,
        &cookie,
        "verify",
        r#"{"provider":"recaptcha","token":"anything"}"#,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "ceremony_rejected");
    assert!(
        body["detail"].as_str().unwrap().contains("not switched on"),
        "{body}"
    );
}

#[tokio::test]
async fn an_unknown_provider_is_refused_rather_than_guessed_at() {
    let app = api::build_router(state().await);
    let cookie = select(&app, &["turnstile"]).await;

    for provider in ["", "TURNSTILE ", "friendly-captcha", "../turnstile"] {
        let payload = serde_json::json!({ "provider": provider, "token": "t" }).to_string();
        let (status, _) = action(&app, &cookie, "verify", &payload).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`{provider}` should not resolve to a provider"
        );
    }
}

/// A blank token cannot pass, so asking the provider about it is a wasted
/// round trip and a wasted rate-limit slot at a third party.
#[tokio::test]
async fn a_missing_or_blank_token_is_refused_without_a_round_trip() {
    let app = api::build_router(state().await);
    let cookie = select(&app, &["turnstile"]).await;

    for body in [
        r#"{"provider":"turnstile"}"#,
        r#"{"provider":"turnstile","token":""}"#,
        r#"{"provider":"turnstile","token":"   "}"#,
        r#"{"provider":"turnstile","token":null}"#,
    ] {
        let (status, payload) = action(&app, &cookie, "verify", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        // The message has to point at the fix, since this is what a visitor
        // hits by clicking verify before completing the widget.
        assert!(
            payload["detail"]
                .as_str()
                .unwrap()
                .contains("Complete the widget"),
            "{payload}"
        );
    }
}

#[tokio::test]
async fn an_unknown_action_is_a_404() {
    let app = api::build_router(state().await);
    let cookie = select(&app, &["turnstile"]).await;

    let (status, _) = action(&app, &cookie, "solve", "{}").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Ceremonies reach third parties, so the kill switch has to stop them — and
/// stop them before the argument checks, or a disabled demo would still answer
/// questions about which providers a session has selected.
#[tokio::test]
async fn the_kill_switch_stops_verification() {
    use api::killswitch::KillSwitchState;

    let mut st = state().await;
    let app_enabled = api::build_router(st.clone());
    let cookie = select(&app_enabled, &["turnstile"]).await;

    st.kill_switch = Arc::new(KillSwitch::new(
        None,
        KillSwitchState {
            demo_enabled: false,
            ..Default::default()
        },
    ));
    let app = api::build_router(st);

    for (name, body) in [("widget", "{}"), ("verify", r#"{"provider":"turnstile"}"#)] {
        let (status, payload) = action(&app, &cookie, name, body).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{name}");
        assert_eq!(payload["error"], "demo_disabled");
    }
}

// -------------------------------------------------------------------- the diff

#[tokio::test]
async fn the_diff_names_the_engine_captcha_feature_not_the_facade() {
    let app = api::build_router(state().await);
    let resp = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/captcha/configure")
                .body(Body::from(
                    r#"{"value":{"kind":"select_many","selected":["turnstile"]}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;

    let rendered = body["diff"].to_string();
    assert!(rendered.contains("authkestra-engine"), "{rendered}");
    assert!(rendered.contains("captcha"), "{rendered}");
    // The facade deliberately does not expose `captcha` — see
    // docs/decisions/0001-dependency-and-tls-baseline.md.
    assert!(
        !rendered.contains("\"authkestra\""),
        "the diff points at the facade, which cannot give you captcha: {rendered}"
    );
    // The whole question a bot-protection diff has to answer.
    assert!(
        rendered.contains("/auth/guarded"),
        "the diff should say which route the check guards: {rendered}"
    );
}
