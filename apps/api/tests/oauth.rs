//! Tests for the OAuth scenario and its navigation routes.
//!
//! A real round trip needs registered OAuth apps, which this deployment does
//! not have yet, so what is verified here is everything on our side of the
//! redirect: which providers are offered, the authorization URL we build, the
//! declined path, the gating, and the diff. The provider round trip itself is
//! noted on the issue as browser-verified work.

use std::sync::Arc;

use api::killswitch::KillSwitch;
use api::routes::AppState;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const ALL: [(&str, &str, &str); 3] = [
    ("github", "gh-id", "gh-secret"),
    ("google", "go-id", "go-secret"),
    ("discord", "di-id", "di-secret"),
];

fn state(providers: &[(&str, &str, &str)]) -> AppState {
    api::testing::test_state_with_providers(KillSwitch::default(), providers)
}

fn app(providers: &[(&str, &str, &str)]) -> axum::Router {
    api::build_router(state(providers))
}

fn req(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-forwarded-for", "203.0.113.8")
        .header(header::CONTENT_TYPE, "application/json")
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

async fn spec(app: &axum::Router, id: &str) -> Value {
    let resp = app
        .clone()
        .oneshot(req("GET", "/api/scenarios").body(Body::empty()).unwrap())
        .await
        .unwrap();
    body_json(resp)
        .await
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .cloned()
        .expect("scenario present")
}

/// Offering a provider with no credentials is a dead end that looks like the
/// framework failing, so the control must only list configured ones.
#[tokio::test]
async fn only_configured_providers_are_offered() {
    let app = app(&[("github", "id", "secret")]);
    let s = spec(&app, "oauth").await;

    let options: Vec<&str> = s["control"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(options, vec!["github"], "only github has credentials");
}

#[tokio::test]
async fn with_no_credentials_the_control_offers_nothing() {
    let app = app(&[]);
    let s = spec(&app, "oauth").await;
    assert!(s["control"]["options"].as_array().unwrap().is_empty());

    // And `try` explains why rather than looking broken.
    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/oauth/try")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let result = body_json(resp).await;
    assert_eq!(result["outcome"], "not_configured");
    assert!(
        result["detail"].as_str().unwrap().contains("credentials"),
        "detail should say credentials are missing: {}",
        result["detail"]
    );
}

#[tokio::test]
async fn all_three_providers_are_offered_when_configured() {
    let app = app(&ALL);
    let s = spec(&app, "oauth").await;
    let mut options: Vec<&str> = s["control"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    options.sort();
    assert_eq!(options, vec!["discord", "github", "google"]);
}

/// The login route must send the browser to the provider, carrying PKCE and
/// the state the callback will verify.
#[tokio::test]
async fn login_redirects_to_the_provider_with_pkce_and_state() {
    let resp = app(&ALL)
        .oneshot(
            req("GET", "/auth/login/github")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.status().is_redirection(),
        "expected a redirect, got {}",
        resp.status()
    );
    let url = location(&resp);
    assert!(
        url.starts_with("https://github.com/login/oauth/authorize"),
        "should go to GitHub: {url}"
    );
    assert!(url.contains("client_id=gh-id"), "{url}");
    assert!(url.contains("state="), "state is required: {url}");
    assert!(
        url.contains("code_challenge=") && url.contains("code_challenge_method=S256"),
        "PKCE should be applied: {url}"
    );
    // The state must travel in a cookie, not a server-side table — that is the
    // stateless property this scenario exists to show.
    assert!(
        resp.headers().get(header::SET_COOKIE).is_some(),
        "the encrypted state cookie should be set"
    );
}

#[tokio::test]
async fn login_for_an_unconfigured_provider_is_refused_clearly() {
    let resp = app(&[("github", "id", "secret")])
        .oneshot(
            req("GET", "/auth/login/google")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(resp).await["detail"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        detail.contains("google"),
        "should name the provider: {detail}"
    );
}

#[tokio::test]
async fn the_kill_switch_stops_oauth_logins() {
    let mut st = state(&ALL);
    let ks = KillSwitch::default();
    ks.set_scenario_enabled("oauth", false);
    st.kill_switch = Arc::new(ks);

    let resp = api::build_router(st)
        .oneshot(
            req("GET", "/auth/login/github")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// Declining the consent screen is the most common non-happy outcome, and it
/// is not an error on our side.
#[tokio::test]
async fn a_declined_consent_screen_lands_back_on_the_frontend() {
    let resp = app(&ALL)
        .oneshot(
            req(
                "GET",
                "/auth/callback/github?error=access_denied&error_description=The+user+denied",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();

    assert!(resp.status().is_redirection());
    let url = location(&resp);
    assert!(
        url.starts_with("http://localhost:3000/"),
        "must return to the frontend, not show a server page: {url}"
    );
    assert!(url.contains("oauth=denied"), "{url}");
    assert!(url.contains("provider=github"), "{url}");
    assert!(url.contains("reason=access_denied"), "{url}");
}

#[tokio::test]
async fn a_callback_without_a_code_is_reported_not_crashed() {
    let resp = app(&ALL)
        .oneshot(
            req("GET", "/auth/callback/github")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(resp.status().is_redirection());
    assert!(location(&resp).contains("oauth=error"));
    assert!(location(&resp).contains("reason=missing_code"));
}

/// A forged callback must not be accepted. Without the matching encrypted
/// state cookie the exchange cannot be verified.
#[tokio::test]
async fn a_callback_with_no_state_cookie_is_rejected() {
    let resp = app(&ALL)
        .oneshot(
            req("GET", "/auth/callback/github?code=abc&state=forged")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(resp.status().is_redirection());
    let url = location(&resp);
    assert!(
        url.contains("oauth=error"),
        "a forged state must not succeed: {url}"
    );
}

#[tokio::test]
async fn the_diff_names_the_provider_feature_flag() {
    let app = app(&ALL);
    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/oauth/configure")
                .body(Body::from(
                    r#"{"value":{"kind":"select_one","selected":"github"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let diff = body_json(resp).await;
    let crates = diff["diff"]["consequences"]["crates"].as_array().unwrap();

    let providers = crates
        .iter()
        .find(|c| c["name"] == "authkestra-providers")
        .expect("must name authkestra-providers");
    assert_eq!(
        providers["features"].as_array().unwrap(),
        &vec![Value::from("github")],
        "the feature flag is per provider — that is the point"
    );

    let routes = diff["diff"]["consequences"]["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        routes.iter().any(|r| r.contains("/auth/callback/github")),
        "routes should match what is actually mounted: {routes:?}"
    );
}

/// Google is OIDC, not plain OAuth2, so it needs a crate the others do not —
/// exactly the kind of thing the diff should surface up front.
#[tokio::test]
async fn choosing_google_surfaces_the_oidc_crate() {
    let app = app(&ALL);
    let resp = app
        .oneshot(
            req("POST", "/api/scenarios/oauth/configure")
                .body(Body::from(
                    r#"{"value":{"kind":"select_one","selected":"google"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let diff = body_json(resp).await;
    let crates = diff["diff"]["consequences"]["crates"].as_array().unwrap();
    assert!(
        crates.iter().any(|c| c["name"] == "authkestra-oidc"),
        "Google needs authkestra-oidc: {crates:?}"
    );
}

#[tokio::test]
async fn an_unknown_option_is_rejected() {
    let resp = app(&ALL)
        .oneshot(
            req("POST", "/api/scenarios/oauth/configure")
                .body(Body::from(
                    r#"{"value":{"kind":"select_one","selected":"myspace"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
