//! Tests for pushing a generated project to the visitor's own GitHub
//! repository (#40).
//!
//! No test here makes a network call: `AppState.github_push.api` is always a
//! `FakeGitHubApi` (see `api::testing`), which lets every distinct GitHub-side
//! failure be exercised deterministically. What *is* exercised for real is
//! everything on our side — CSRF state verification (including a forged
//! mismatch), the "not configured" degradation, the connect/callback/push
//! round trip through the real router, and each failure mode mapping to its
//! own HTTP status and error code.

use std::sync::Arc;

use api::github_api::{GitHubApiError, GitHubRepoInfo};
use api::killswitch::{KillSwitch, KillSwitchState};
use api::routes::AppState;
use api::testing::{test_state, test_state_with_github, FakeGitHubApi};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const CLIENT_ID: &str = "kit-client-id";
const CLIENT_SECRET: &str = "kit-client-secret";

fn configured_state() -> (AppState, Arc<FakeGitHubApi>) {
    let fake = Arc::new(FakeGitHubApi::default());
    let state = test_state_with_github(KillSwitch::default(), CLIENT_ID, CLIENT_SECRET, fake.clone());
    (state, fake)
}

fn req(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-forwarded-for", "203.0.113.42")
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn location(resp: &axum::response::Response) -> String {
    resp.headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// Every `name=value` pair this response's `Set-Cookie` headers carry, one
/// request can set several (the session cookie and the CSRF state cookie
/// both, on a fresh `connect`).
fn set_cookies(resp: &axum::response::Response) -> Vec<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(|s| s.to_string())
        .collect()
}

fn cookie_named<'a>(cookies: &'a [String], name: &str) -> Option<&'a str> {
    cookies
        .iter()
        .find(|c| c.starts_with(&format!("{name}=")))
        .map(|s| s.as_str())
}

fn cookie_header(cookies: &[&str]) -> String {
    cookies.join("; ")
}

/// A fresh demo-session cookie, established the same way every other visitor
/// gets one: an ordinary `GET`.
async fn session_cookie(app: &axum::Router) -> String {
    let resp = app
        .clone()
        .oneshot(req("GET", "/api/session").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let cookies = set_cookies(&resp);
    cookie_named(&cookies, "ak_demo")
        .expect("a session cookie")
        .to_string()
}

/// Drive `connect` for real (to obtain a genuine state cookie) and then
/// `callback` with a matching `state`, leaving the session connected. Returns
/// the session cookie used throughout, so a caller can immediately push.
async fn connect_and_complete(app: &axum::Router) -> String {
    let session = session_cookie(app).await;

    let connect_resp = app
        .clone()
        .oneshot(
            req("GET", "/api/github/connect")
                .header(header::COOKIE, &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(connect_resp.status().is_redirection());
    let cookies = set_cookies(&connect_resp);
    let state_cookie = cookie_named(&cookies, "ak_github_push_state")
        .expect("a CSRF state cookie")
        .to_string();
    let csrf_state = state_cookie.split('=').nth(1).unwrap().to_string();

    let callback_resp = app
        .clone()
        .oneshot(
            req(
                "GET",
                &format!("/api/github/callback?code=abc123&state={csrf_state}"),
            )
            .header(header::COOKIE, cookie_header(&[&session, &state_cookie]))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(callback_resp.status().is_redirection());
    assert!(
        location(&callback_resp).contains("github_push=connected"),
        "{}",
        location(&callback_resp)
    );

    session
}

fn push_body() -> Body {
    Body::from(
        serde_json::to_vec(&json!({ "repo_name": "my-new-project" }))
            .unwrap(),
    )
}

fn push_request(session: &str) -> Request<Body> {
    req("POST", "/api/github/push")
        .header(header::COOKIE, session)
        .header(header::CONTENT_TYPE, "application/json")
        .body(push_body())
        .unwrap()
}

// ---------------------------------------------------------- not configured

#[tokio::test]
async fn connect_is_refused_cleanly_when_no_credentials_are_configured() {
    let app = api::build_router(test_state(KillSwitch::default(), None));
    let resp = app
        .oneshot(req("GET", "/api/github/connect").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let detail = body_json(resp).await["detail"].as_str().unwrap().to_string();
    assert!(detail.to_lowercase().contains("not configured"), "{detail}");
}

#[tokio::test]
async fn push_is_refused_cleanly_when_no_credentials_are_configured() {
    let app = api::build_router(test_state(KillSwitch::default(), None));
    let session = session_cookie(&app).await;
    let resp = app.oneshot(push_request(&session)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "github_push_not_configured");
}

#[tokio::test]
async fn callback_degrades_cleanly_when_credentials_disappear_mid_flight() {
    // Simulates credentials being present at `connect` time but not at
    // `callback` time by hitting the callback directly on an unconfigured
    // deployment — the important property is a clean redirect, never a panic.
    let app = api::build_router(test_state(KillSwitch::default(), None));
    let session = session_cookie(&app).await;
    let resp = app
        .oneshot(
            req("GET", "/api/github/callback?code=abc&state=xyz")
                .header(header::COOKIE, &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_redirection());
    assert!(location(&resp).contains("reason=not_configured"));
}

// -------------------------------------------------------------- kill switch

#[tokio::test]
async fn the_kill_switch_stops_the_whole_feature() {
    let (state, _fake) = configured_state();
    let mut switch_state = KillSwitchState::default();
    switch_state.disabled_scenarios.insert("github_push".to_string());
    let mut state = state;
    state.kill_switch = Arc::new(KillSwitch::new(None, switch_state));
    let app = api::build_router(state);

    let session = session_cookie(&app).await;
    let resp = app
        .clone()
        .oneshot(
            req("GET", "/api/github/connect")
                .header(header::COOKIE, &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let resp = app.oneshot(push_request(&session)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// --------------------------------------------------------------- csrf state

#[tokio::test]
async fn connect_sets_an_http_only_state_cookie_and_redirects_to_github() {
    let (state, _fake) = configured_state();
    let app = api::build_router(state);
    let session = session_cookie(&app).await;

    let resp = app
        .oneshot(
            req("GET", "/api/github/connect")
                .header(header::COOKIE, &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(resp.status().is_redirection());
    let url = location(&resp);
    assert!(url.starts_with("https://github.com/login/oauth/authorize"), "{url}");
    assert!(url.contains(&format!("client_id={CLIENT_ID}")), "{url}");
    assert!(url.contains("scope=public_repo"), "{url}");
    assert!(!url.contains("scope=repo&"), "must not ask for the wider `repo` scope: {url}");

    let raw = resp
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(raw.contains("ak_github_push_state="));
    assert!(raw.contains("HttpOnly"), "{raw}");
}

#[tokio::test]
async fn a_callback_whose_state_does_not_match_the_cookie_is_refused() {
    let (state, _fake) = configured_state();
    let app = api::build_router(state);
    let session = session_cookie(&app).await;

    let connect_resp = app
        .clone()
        .oneshot(
            req("GET", "/api/github/connect")
                .header(header::COOKIE, &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let cookies = set_cookies(&connect_resp);
    let state_cookie = cookie_named(&cookies, "ak_github_push_state").unwrap().to_string();

    // A forged `state` — the CSRF value an attacker would have to guess.
    let resp = app
        .oneshot(
            req("GET", "/api/github/callback?code=abc&state=forged-value")
                .header(header::COOKIE, cookie_header(&[&session, &state_cookie]))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(resp.status().is_redirection());
    let url = location(&resp);
    assert!(url.contains("github_push=error"), "{url}");
    assert!(url.contains("reason=state_invalid"), "{url}");
}

#[tokio::test]
async fn a_callback_with_no_state_cookie_at_all_is_refused() {
    let (state, _fake) = configured_state();
    let app = api::build_router(state);
    let session = session_cookie(&app).await;

    let resp = app
        .oneshot(
            req("GET", "/api/github/callback?code=abc&state=whatever")
                .header(header::COOKIE, &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(resp.status().is_redirection());
    assert!(location(&resp).contains("reason=state_missing"));
}

#[tokio::test]
async fn a_callback_with_no_code_is_reported_not_crashed() {
    let (state, _fake) = configured_state();
    let app = api::build_router(state);
    let resp = app
        .oneshot(
            req("GET", "/api/github/callback")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_redirection());
    assert!(location(&resp).contains("reason=missing_code"));
}

#[tokio::test]
async fn a_declined_consent_screen_is_reported_as_denied_not_an_error() {
    let (state, _fake) = configured_state();
    let app = api::build_router(state);
    let resp = app
        .oneshot(
            req("GET", "/api/github/callback?error=access_denied")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_redirection());
    let url = location(&resp);
    assert!(url.contains("github_push=denied"), "{url}");
    assert!(url.contains("reason=access_denied"), "{url}");
}

// -------------------------------------------------------------- push flow

#[tokio::test]
async fn the_full_connect_then_push_round_trip_succeeds() {
    let (state, fake) = configured_state();
    let app = api::build_router(state);

    let session = connect_and_complete(&app).await;

    let resp = app.clone().oneshot(push_request(&session)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{:?}", body_json(resp).await);

    assert!(
        !fake.blobs_created.lock().unwrap().is_empty(),
        "the kit's files must have been pushed as blobs"
    );
    assert_eq!(*fake.commit_calls.lock().unwrap(), 1, "one commit for the whole kit");
    assert_eq!(*fake.ref_calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn pushing_without_ever_connecting_is_refused_clearly() {
    let (state, _fake) = configured_state();
    let app = api::build_router(state);
    let session = session_cookie(&app).await;

    let resp = app.oneshot(push_request(&session)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "github_not_connected");
}

#[tokio::test]
async fn the_token_is_consumed_by_one_push_attempt() {
    let (state, _fake) = configured_state();
    let app = api::build_router(state);
    let session = connect_and_complete(&app).await;

    let first = app.clone().oneshot(push_request(&session)).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app.oneshot(push_request(&session)).await.unwrap();
    assert_eq!(
        second.status(),
        StatusCode::BAD_REQUEST,
        "the token must not still be usable after one push attempt"
    );
    assert_eq!(body_json(second).await["error"], "github_not_connected");
}

#[tokio::test]
async fn the_token_is_dropped_even_when_the_push_fails() {
    let (state, fake) = configured_state();
    *fake.create_repo_result.lock().unwrap() = Some(Err(GitHubApiError::RepoNameTaken));
    let app = api::build_router(state);
    let session = connect_and_complete(&app).await;

    let first = app.clone().oneshot(push_request(&session)).await.unwrap();
    assert_eq!(first.status(), StatusCode::CONFLICT);

    let second = app.oneshot(push_request(&session)).await.unwrap();
    assert_eq!(
        body_json(second).await["error"],
        "github_not_connected",
        "the token must be dropped on failure too, not just on success"
    );
}

#[tokio::test]
async fn an_implausible_repo_name_is_rejected_before_any_github_call() {
    let (state, fake) = configured_state();
    let app = api::build_router(state);
    let session = connect_and_complete(&app).await;

    let resp = app
        .oneshot(
            req("POST", "/api/github/push")
                .header(header::COOKIE, &session)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "repo_name": "has a space" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "github_invalid_repo_name");
    assert!(
        fake.blobs_created.lock().unwrap().is_empty(),
        "an obviously bad name must never spend a GitHub call"
    );
}

/// Each distinct GitHub-side failure must map to its own status and code —
/// a flat 500 for any of these is exactly what the issue calls a bug.
#[tokio::test]
async fn each_github_failure_mode_maps_to_its_own_error() {
    let cases: [(GitHubApiError, StatusCode, &str); 6] = [
        (GitHubApiError::RepoNameTaken, StatusCode::CONFLICT, "github_repo_name_taken"),
        (
            GitHubApiError::InvalidRepoName("bad name".into()),
            StatusCode::BAD_REQUEST,
            "github_invalid_repo_name",
        ),
        (GitHubApiError::TokenRejected, StatusCode::UNAUTHORIZED, "github_token_rejected"),
        (GitHubApiError::ScopeMissing, StatusCode::FORBIDDEN, "github_scope_missing"),
        (
            GitHubApiError::RateLimited,
            StatusCode::TOO_MANY_REQUESTS,
            "github_rate_limited",
        ),
        (
            GitHubApiError::Network("connection refused".into()),
            StatusCode::BAD_GATEWAY,
            "github_network_error",
        ),
    ];

    for (err, expected_status, expected_code) in cases {
        let (state, fake) = configured_state();
        let app = api::build_router(state);
        let session = connect_and_complete(&app).await;
        *fake.create_repo_result.lock().unwrap() = Some(Err(err));

        let resp = app.oneshot(push_request(&session)).await.unwrap();
        let status = resp.status();
        let json = body_json(resp).await;
        assert_eq!(status, expected_status, "for {expected_code}: {json:?}");
        assert_eq!(json["error"], expected_code, "{json:?}");
    }
}

#[tokio::test]
async fn a_successful_push_reports_the_repository_it_created() {
    let (state, fake) = configured_state();
    *fake.create_repo_result.lock().unwrap() = Some(Ok(GitHubRepoInfo {
        name: "my-new-project".to_string(),
        html_url: "https://github.com/fake-user/my-new-project".to_string(),
        default_branch: "main".to_string(),
    }));
    let app = api::build_router(state);
    let session = connect_and_complete(&app).await;

    let resp = app.oneshot(push_request(&session)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["repo"], "my-new-project");
    assert_eq!(json["html_url"], "https://github.com/fake-user/my-new-project");
    assert_eq!(json["default_branch"], "main");
    assert!(json["commit_sha"].as_str().is_some());
}
