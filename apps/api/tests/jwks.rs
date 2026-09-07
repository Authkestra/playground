//! The published key set (roadmap #52).
//!
//! A resource server that validates by discovery is only demonstrable if the
//! discovery endpoint is real: fetchable without a session, at the path the
//! RFCs name, carrying the `kid` that appears in issued tokens and nothing
//! else. These are the properties a visitor checking the demo by hand relies
//! on, so they are asserted over HTTP rather than against the module.

use api::killswitch::KillSwitch;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const PATH: &str = "/.well-known/jwks.json";

fn app() -> axum::Router {
    api::build_router(api::testing::test_state(KillSwitch::default(), None))
}

fn req(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("x-forwarded-for", "203.0.113.61")
        .body(Body::empty())
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).expect("JSON response")
}

#[tokio::test]
async fn the_key_set_is_served_at_the_well_known_path() {
    let resp = app().oneshot(req(PATH)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let doc = body_json(resp).await;
    let keys = doc["keys"].as_array().expect("an RFC 7517 `keys` array");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["kty"], "OKP");
    assert_eq!(keys[0]["crv"], "Ed25519");
    assert!(
        keys[0]["kid"].is_string(),
        "a key with no kid is unlookupable"
    );
}

/// No cookie, no session. A validator fetching keys is not a visitor, and
/// requiring a session would make the endpoint useless to the thing it exists
/// for — as well as unverifiable by hand.
#[tokio::test]
async fn fetching_keys_needs_no_session_and_creates_none() {
    let resp = app().oneshot(req(PATH)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers().get(header::SET_COOKIE).is_none(),
        "a key fetch must not mint a demo session"
    );
}

/// The one property worth a test of its own: the endpoint publishes a *public*
/// key set. A leak here would hand every visitor the ability to mint tokens
/// this deployment accepts.
#[tokio::test]
async fn the_key_set_carries_nothing_that_could_sign() {
    let resp = app().oneshot(req(PATH)).await.unwrap();
    let serialised = body_json(resp).await.to_string();

    // `d` is the Ed25519 private scalar; the rest are RSA private fields,
    // checked because the Jwk type carries both shapes.
    for private in ["\"d\"", "\"p\"", "\"q\"", "\"dp\"", "\"dq\"", "\"qi\""] {
        assert!(
            !serialised.contains(private),
            "a private field {private} reached the key set: {serialised}"
        );
    }
    assert!(!serialised.to_uppercase().contains("PRIVATE"));
}

/// A validator caches by this header, so an absent one means a fetch per
/// request against a third party — or, here, against ourselves.
#[tokio::test]
async fn the_key_set_is_cacheable() {
    let resp = app().oneshot(req(PATH)).await.unwrap();
    let cache = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(cache.contains("max-age"), "no cache directive: {cache:?}");
}

/// The kill switch governs live flows, not key publication. A resource server
/// mid-request must not start failing validation because the demo was paused —
/// that would look like a signing compromise rather than a maintenance window.
#[tokio::test]
async fn the_kill_switch_does_not_withdraw_the_key_set() {
    use api::killswitch::KillSwitchState;

    let state = api::testing::test_state_with_shared_store(
        api::testing::shared_store(),
        KillSwitchState {
            demo_enabled: false,
            ..Default::default()
        },
    );
    let resp = api::build_router(state).oneshot(req(PATH)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
