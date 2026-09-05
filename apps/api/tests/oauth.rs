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

    // An empty control on its own reads as broken, so the spec has to carry the
    // reason. This is the live case: the deployment has no provider credentials.
    let reason = s["unavailable_reason"]
        .as_str()
        .expect("an offerless control must explain itself");
    assert!(
        reason.contains("credentials"),
        "the reason should say credentials are missing: {reason}"
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
    use api::killswitch::KillSwitchState;

    let mut st = state(&ALL);
    let mut switch_state = KillSwitchState::default();
    switch_state.disabled_scenarios.insert("oauth".to_string());
    let ks = KillSwitch::new(None, switch_state);
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
                    r#"{"value":{"kind":"select_many","selected":["github"]}}"#,
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
                    r#"{"value":{"kind":"select_many","selected":["google"]}}"#,
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
                    r#"{"value":{"kind":"select_many","selected":["myspace"]}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// The point of multi-select: someone can offer several providers at once, and
/// the diff must describe the combination rather than one of them.
#[tokio::test]
async fn several_providers_can_be_selected_together() {
    let resp = app(&ALL)
        .oneshot(
            req("POST", "/api/scenarios/oauth/configure")
                .body(Body::from(
                    r#"{"value":{"kind":"select_many","selected":["github","google"]}}"#,
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
        .expect("providers crate");
    let features: Vec<&str> = providers["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(features.contains(&"github"), "{features:?}");
    assert!(features.contains(&"google"), "{features:?}");

    // Google in the mix still pulls in the OIDC crate.
    assert!(crates.iter().any(|c| c["name"] == "authkestra-oidc"));

    // Both providers' routes appear.
    let routes = diff["diff"]["consequences"]["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(routes.iter().any(|r| r.contains("/auth/callback/github")));
    assert!(routes.iter().any(|r| r.contains("/auth/callback/google")));
}

/// Offering two providers raises a question the framework deliberately does not
/// answer for you, so the diff should say so.
#[tokio::test]
async fn multiple_providers_surface_the_account_linking_decision() {
    let resp = app(&ALL)
        .oneshot(
            req("POST", "/api/scenarios/oauth/configure")
                .body(Body::from(
                    r#"{"value":{"kind":"select_many","selected":["github","discord"]}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let diff = body_json(resp).await;
    let requirements = diff["diff"]["consequences"]["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        requirements.contains("two of them") || requirements.contains("link"),
        "should raise account linking: {requirements}"
    );
}

/// Three unrelated causes used to collapse into one `exchange_failed`, which
/// made a real failure undiagnosable. They must be distinguishable.
#[tokio::test]
async fn a_callback_with_no_state_cookie_says_so_specifically() {
    let resp = app(&ALL)
        .oneshot(
            req("GET", "/auth/callback/github?code=abc&state=xyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(resp.status().is_redirection());
    let url = location(&resp);
    assert!(
        url.contains("reason=state_missing"),
        "a missing state cookie must not be reported as a failed exchange: {url}"
    );
}

#[tokio::test]
async fn a_callback_with_an_undecryptable_state_cookie_is_distinguished() {
    let resp = app(&ALL)
        .oneshot(
            req("GET", "/auth/callback/github?code=abc&state=xyz")
                .header(header::COOKIE, "ak_state=not-a-valid-encrypted-blob")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(resp.status().is_redirection());
    let url = location(&resp);
    assert!(
        url.contains("reason=state_invalid") || url.contains("reason=callback_failed"),
        "a corrupt state cookie should not read as a failed exchange: {url}"
    );
    assert!(
        !url.contains("reason=state_missing"),
        "the cookie was present: {url}"
    );
}

/// Whatever the outcome, the visitor's own flow log should explain it — that is
/// the only diagnosis available to someone without server access.
#[tokio::test]
async fn a_failed_callback_is_narrated_in_the_flow_log() {
    let st = state(&ALL);
    let events = st.events.clone();
    let app = api::build_router(st);

    // Establish a demo session so the log has an owner.
    let session_resp = app
        .clone()
        .oneshot(req("GET", "/api/session").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let cookie = session_resp
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .unwrap()
        .to_string();
    let session_id: uuid::Uuid = cookie.trim_start_matches("ak_demo=").parse().unwrap();

    app.oneshot(
        req("GET", "/auth/callback/github?error=access_denied")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    let log = events.read(session_id).await.unwrap();
    let entry = log
        .iter()
        .find(|e| e.scenario == "oauth")
        .expect("the OAuth outcome should be narrated");
    assert_eq!(
        entry.level,
        api::events::EventLevel::Rejected,
        "declining consent is an ordinary outcome, not a failure"
    );
    assert!(entry.detail.len() > 20, "{entry:?}");
}

/// Regression test for the upstream nonce bug, inverted.
///
/// In 0.8.0, `OAuth2Flow::initiate_login` set a nonce unconditionally and
/// `finalize_login` then demanded a matching one back — but the shipped plain
/// OAuth2 providers never return one, so every round trip failed with "Nonce
/// mismatch". This service carried a workaround that stripped the nonce back
/// out of the state cookie.
///
/// 0.8.1 fixed it the other way round, which is the better fix: the nonce is
/// still generated, and the *enforcement* is gated on
/// `OAuthProvider::validates_nonce()`. A nonce sent to a provider that ignores
/// it is harmless. So the workaround is gone, and what is worth pinning is the
/// upstream behaviour this now depends on — if a future version enforced the
/// nonce for a provider that cannot echo one, OAuth would break for every
/// visitor again, and this is what would say so.
#[tokio::test]
async fn no_shipped_provider_demands_a_nonce_back() {
    use authkestra_engine::OAuthProvider;

    let redirect = "http://localhost/auth/callback/x".to_string();
    let id = "id".to_string();
    let secret = "secret".to_string();

    // Every provider this service constructs, not just one. They inherit the
    // trait default today; a provider that started overriding it would break
    // only its own sign-in, which is exactly the kind of partial outage that
    // takes a while to notice.
    let providers: Vec<(&str, bool)> = vec![
        (
            "github",
            authkestra_providers::github::GithubProvider::new(
                id.clone(),
                secret.clone(),
                redirect.clone(),
            )
            .validates_nonce(),
        ),
        (
            "google",
            authkestra_providers::google::GoogleProvider::new(
                id.clone(),
                secret.clone(),
                redirect.clone(),
            )
            .validates_nonce(),
        ),
        (
            "discord",
            authkestra_providers::discord::DiscordProvider::new(id, secret, redirect)
                .validates_nonce(),
        ),
    ];

    for (name, validates) in providers {
        assert!(
            !validates,
            "{name} claims to validate a nonce, but the shipped providers build \
             their identity with no `nonce` attribute — the engine would hold it \
             to one it can never return and every {name} callback would fail"
        );
    }
}

/// The nonce is allowed to be there now. What must survive is everything that
/// actually protects the flow.
#[tokio::test]
async fn the_state_cookie_keeps_its_csrf_state_and_pkce_verifier() {
    use authkestra_engine::state::OAuth2State;

    let st = state(&ALL);
    let key = st.engines.session_config().state_encryption_key;
    let app = api::build_router(st);

    let resp = app
        .oneshot(
            req("GET", "/auth/login/github")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let raw = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("ak_state="))
        .expect("the state cookie should be set")
        .to_string();

    let value = raw
        .trim_start_matches("ak_state=")
        .split(';')
        .next()
        .unwrap();
    let decoded = OAuth2State::decrypt(value, &key).expect("state should decrypt");

    assert!(!decoded.state.is_empty(), "CSRF state must remain");
    assert!(decoded.code_verifier.is_some(), "PKCE verifier must remain");
}
