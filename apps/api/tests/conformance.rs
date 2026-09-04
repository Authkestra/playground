//! Scenario conformance harness (roadmap P2).
//!
//! Every scenario exposes the same contract — `configure`, `diff`, `try`, and
//! optionally `action` — so it is tested uniformly rather than ad hoc. Each
//! scenario-specific test file covers that scenario's *behaviour*; this file
//! covers the properties all of them must share.
//!
//! **This harness enumerates the registry.** A scenario added to
//! `ScenarioRegistry::with_builtins` is picked up automatically and must satisfy
//! every property below, so forgetting to wire one up fails CI rather than
//! silently shipping an untested scenario.

use std::collections::BTreeSet;
use std::sync::Arc;

use api::killswitch::KillSwitch;
use api::routes::AppState;
use api::scenario::{ControlShape, ControlValue, ScenarioRegistry};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// State with every OAuth provider configured.
///
/// The harness needs each scenario to have a meaningful "active" value, and a
/// provider-select control offers nothing without credentials — so a deployment
/// missing them would let the OAuth scenario pass vacuously.
async fn state() -> AppState {
    api::testing::test_state_with_providers(
        KillSwitch::default(),
        &[
            ("github", "gh-id", "gh-secret"),
            ("google", "go-id", "go-secret"),
            ("discord", "di-id", "di-secret"),
        ],
    )
}

fn req(method: &str, uri: &str) -> axum::http::request::Builder {
    req_from("203.0.113.77", method, uri)
}

/// A request from a specific client IP.
///
/// The harness walks every scenario, and the endpoints it exercises share the
/// tighter rate-limit bucket — so with enough scenarios the loop throttles
/// itself and a later scenario sees 429 where the test expects its real status.
/// Giving each scenario its own IP keeps the property under test isolated from
/// the limiter, which is genuinely working.
fn req_from(ip: &str, method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-forwarded-for", ip)
        .header(header::CONTENT_TYPE, "application/json")
}

/// A stable per-scenario client IP, so buckets never overlap.
fn ip_for(scenario: &str) -> String {
    let n = scenario.bytes().map(|b| b as u32).sum::<u32>() % 250 + 1;
    format!("198.18.{}.{}", n / 250, n % 250 + 1)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn cookie_of(resp: &axum::response::Response) -> Option<String> {
    resp.headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(|s| s.to_string())
}

/// A value that turns a scenario on, whatever its control shape.
fn active_value(shape: &ControlShape) -> ControlValue {
    match shape {
        ControlShape::Toggle => ControlValue::Toggle { enabled: true },
        ControlShape::SelectOne { options } => ControlValue::SelectOne {
            selected: options.first().map(|o| o.id.clone()),
        },
        ControlShape::SelectMany { options } => ControlValue::SelectMany {
            selected: options.first().map(|o| o.id.clone()).into_iter().collect(),
        },
    }
}

/// Every registered scenario, as (id, control shape, actions).
fn registered() -> Vec<(String, ControlShape, Vec<String>)> {
    test_registry()
        .iter()
        .map(|s| {
            (
                s.id().to_string(),
                s.control(),
                s.actions().iter().map(|a| a.to_string()).collect(),
            )
        })
        .collect()
}

/// The same registry the test state is built with, so control options line up.
fn test_registry() -> ScenarioRegistry {
    ScenarioRegistry::for_tests(vec![
        "discord".to_string(),
        "github".to_string(),
        "google".to_string(),
    ])
}

#[tokio::test]
async fn the_registry_is_not_empty() {
    assert!(
        !registered().is_empty(),
        "the harness would vacuously pass with no scenarios"
    );
}

/// Ids are the routing key and the frontend's React key, so duplicates would
/// silently shadow a scenario.
#[tokio::test]
async fn every_scenario_has_a_unique_id_and_human_facing_text() {
    let mut seen = BTreeSet::new();
    for s in test_registry().iter() {
        assert!(
            seen.insert(s.id().to_string()),
            "duplicate scenario id `{}`",
            s.id()
        );
        assert!(!s.name().trim().is_empty(), "{} has no name", s.id());
        assert!(
            s.summary().trim().len() > 20,
            "{} needs a summary a visitor can actually read",
            s.id()
        );
        assert!(
            !s.id().contains(' ') && s.id() == s.id().to_lowercase(),
            "`{}` should be a lowercase, space-free id — it appears in URLs",
            s.id()
        );
    }
}

/// A scenario's default must fit its own control, or the first render of a
/// fresh session is already inconsistent.
#[tokio::test]
async fn every_default_value_matches_its_control_shape() {
    for s in test_registry().iter() {
        let default = s.default_value();
        assert!(
            default.matches_shape(&s.control()),
            "{}: default {:?} does not fit control {:?}",
            s.id(),
            default,
            s.control()
        );
        assert!(
            !default.is_active(),
            "{} starts switched on; a fresh visitor should opt in",
            s.id()
        );
        assert!(
            s.validate(&default).is_ok(),
            "{} rejects its own default value",
            s.id()
        );
    }
}

/// An inactive scenario must contribute nothing to the diff, or the diff claims
/// a project needs crates it does not.
#[tokio::test]
async fn an_inactive_scenario_has_no_consequences() {
    for s in test_registry().iter() {
        let c = s.consequences(&s.default_value());
        assert!(
            c.routes.is_empty() && c.requirements.is_empty() && c.crates.is_empty(),
            "{} reports consequences while switched off: {c:?}",
            s.id()
        );
    }
}

/// The whole promise of the diff is that it names real, actionable changes.
#[tokio::test]
async fn an_active_scenario_names_real_consequences() {
    for s in test_registry().iter() {
        // Placeholders exist only to cover control shapes; they are exempt.
        if s.id().starts_with("dummy_") {
            continue;
        }
        let c = s.consequences(&active_value(&s.control()));
        assert!(
            !c.routes.is_empty(),
            "{} should name the routes it adds",
            s.id()
        );
        assert!(
            !c.requirements.is_empty(),
            "{} should say what changes for users",
            s.id()
        );
        assert!(
            !c.crates.is_empty(),
            "{} should name the crates a real project needs",
            s.id()
        );

        // The facade deliberately does not expose webauthn/totp/captcha, so a
        // scenario pointing at it would send a reader down a dead end.
        // See docs/decisions/0001-dependency-and-tls-baseline.md.
        assert!(
            !c.crates.iter().any(|cr| cr.name == "authkestra"),
            "{} names the `authkestra` facade; depend on the sub-crate instead",
            s.id()
        );
    }
}

#[tokio::test]
async fn every_scenario_rejects_a_value_of_the_wrong_shape() {
    for s in test_registry().iter() {
        // A shape this control is definitely not.
        let wrong = match s.control() {
            ControlShape::Toggle => ControlValue::SelectOne {
                selected: Some("nope".to_string()),
            },
            _ => ControlValue::Toggle { enabled: true },
        };
        assert!(
            s.validate(&wrong).is_err(),
            "{} accepted a value of the wrong shape",
            s.id()
        );
    }
}

#[tokio::test]
async fn every_scenario_rejects_an_unknown_option() {
    for s in test_registry().iter() {
        let bogus = match s.control() {
            ControlShape::Toggle => continue,
            ControlShape::SelectOne { .. } => ControlValue::SelectOne {
                selected: Some("definitely-not-an-option".to_string()),
            },
            ControlShape::SelectMany { .. } => ControlValue::SelectMany {
                selected: vec!["definitely-not-an-option".to_string()],
            },
        };
        assert!(
            s.validate(&bogus).is_err(),
            "{} accepted an option id it never offered",
            s.id()
        );
    }
}

/// The uniform contract, driven over HTTP for every scenario:
/// configure → diff → try.
#[tokio::test]
async fn every_scenario_supports_configure_diff_and_try() {
    let app = api::build_router(state().await);

    for (id, shape, _) in registered() {
        let value = serde_json::to_string(&active_value(&shape)).unwrap();

        let configured = app
            .clone()
            .oneshot(
                req_from(
                    &ip_for(&id),
                    "POST",
                    &format!("/api/scenarios/{id}/configure"),
                )
                .body(Body::from(format!(r#"{{"value":{value}}}"#)))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            configured.status(),
            StatusCode::OK,
            "{id}: configure failed"
        );
        let cookie = cookie_of(&configured).expect("configure issues a session");
        let payload = body_json(configured).await;
        assert!(
            payload["config"]["scenarios"][&id].is_object(),
            "{id}: configure did not persist the value"
        );

        let diffed = app
            .clone()
            .oneshot(
                req_from(&ip_for(&id), "GET", &format!("/api/scenarios/{id}/diff"))
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(diffed.status(), StatusCode::OK, "{id}: diff failed");
        let diff = body_json(diffed).await;
        assert!(
            diff["entries"].as_array().is_some_and(|e| !e.is_empty()),
            "{id}: turning the scenario on produced no diff entries"
        );

        let tried = app
            .clone()
            .oneshot(
                req_from(&ip_for(&id), "POST", &format!("/api/scenarios/{id}/try"))
                    .header(header::COOKIE, &cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(tried.status(), StatusCode::OK, "{id}: try failed");
        let result = body_json(tried).await;
        assert!(
            ["ok", "disabled", "not_configured"]
                .contains(&result["outcome"].as_str().unwrap_or("")),
            "{id}: unexpected try outcome {:?}",
            result["outcome"]
        );
        assert!(
            result["detail"].as_str().is_some_and(|d| !d.is_empty()),
            "{id}: try must explain its outcome"
        );
    }
}

/// Every declared action must actually be dispatchable — a step advertised in
/// `ScenarioSpec.actions` that returns `unknown_action` would strand the UI.
#[tokio::test]
async fn every_declared_action_is_dispatchable() {
    let app = api::build_router(state().await);

    for (id, shape, actions) in registered() {
        if actions.is_empty() {
            continue;
        }
        let value = serde_json::to_string(&active_value(&shape)).unwrap();
        let configured = app
            .clone()
            .oneshot(
                req_from(
                    &ip_for(&id),
                    "POST",
                    &format!("/api/scenarios/{id}/configure"),
                )
                .body(Body::from(format!(r#"{{"value":{value}}}"#)))
                .unwrap(),
            )
            .await
            .unwrap();
        let cookie = cookie_of(&configured).unwrap();

        for action in actions {
            let resp = app
                .clone()
                .oneshot(
                    req_from(
                        &ip_for(&id),
                        "POST",
                        &format!("/api/scenarios/{id}/action/{action}"),
                    )
                    .header(header::COOKIE, &cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
                )
                .await
                .unwrap();

            // An empty body legitimately fails most steps; what must never
            // happen is the scenario disowning a step it advertised.
            assert_ne!(
                resp.status(),
                StatusCode::NOT_FOUND,
                "{id}: advertises action `{action}` but does not handle it"
            );
            let status = resp.status();
            assert!(
                status.is_success() || status.is_client_error(),
                "{id}/{action}: returned {status}, which suggests an unhandled server fault"
            );
        }
    }
}

/// A step no scenario declares must 404 rather than doing something surprising.
#[tokio::test]
async fn an_undeclared_action_is_rejected_everywhere() {
    let app = api::build_router(state().await);

    for (id, shape, _) in registered() {
        let value = serde_json::to_string(&active_value(&shape)).unwrap();
        let configured = app
            .clone()
            .oneshot(
                req_from(
                    &ip_for(&id),
                    "POST",
                    &format!("/api/scenarios/{id}/configure"),
                )
                .body(Body::from(format!(r#"{{"value":{value}}}"#)))
                .unwrap(),
            )
            .await
            .unwrap();
        let cookie = cookie_of(&configured).unwrap();

        let resp = app
            .clone()
            .oneshot(
                req_from(
                    &ip_for(&id),
                    "POST",
                    &format!("/api/scenarios/{id}/action/__nope__"),
                )
                .header(header::COOKIE, &cookie)
                .body(Body::from("{}"))
                .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{id}: an undeclared action should 404"
        );
    }
}

/// The property that breaks the moment two people use the site at once.
#[tokio::test]
async fn conflicting_configurations_do_not_interfere() {
    let app = api::build_router(state().await);

    for (id, shape, _) in registered() {
        let active = serde_json::to_string(&active_value(&shape)).unwrap();

        // Visitor A turns the scenario on.
        let a = app
            .clone()
            .oneshot(
                req_from(
                    &ip_for(&id),
                    "POST",
                    &format!("/api/scenarios/{id}/configure"),
                )
                .body(Body::from(format!(r#"{{"value":{active}}}"#)))
                .unwrap(),
            )
            .await
            .unwrap();
        let cookie_a = cookie_of(&a).unwrap();

        // Visitor B arrives with no cookie and must see defaults.
        let b = app
            .clone()
            .oneshot(req("GET", "/api/session").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let cookie_b = cookie_of(&b).unwrap();
        assert_ne!(cookie_a, cookie_b);

        // B must hold exactly the registered default — comparing against that
        // is shape-agnostic, so this works for toggles and selects alike.
        let default = test_registry()
            .get(&id)
            .expect("registered")
            .default_value();
        let expected = serde_json::to_value(&default).unwrap();
        let b_config = body_json(b).await;
        assert_eq!(
            b_config["config"]["scenarios"][&id], expected,
            "{id}: visitor B saw visitor A's configuration"
        );

        // And A still holds theirs.
        let a_again = app
            .clone()
            .oneshot(
                req("GET", "/api/session")
                    .header(header::COOKIE, &cookie_a)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let a_config = body_json(a_again).await;
        assert!(
            a_config["config"]["scenarios"][&id].is_object(),
            "{id}: visitor A lost their configuration"
        );
    }
}

/// With the kill switch off, no scenario may run a flow — but the site must
/// still be readable, so listing and diffing keep working.
#[tokio::test]
async fn the_kill_switch_stops_every_scenario_uniformly() {
    let mut st = state().await;
    let ks = KillSwitch::default();
    ks.set_demo_enabled(false);
    st.kill_switch = Arc::new(ks);
    let app = api::build_router(st);

    for (id, shape, actions) in registered() {
        let value = serde_json::to_string(&active_value(&shape)).unwrap();
        let configured = app
            .clone()
            .oneshot(
                req_from(
                    &ip_for(&id),
                    "POST",
                    &format!("/api/scenarios/{id}/configure"),
                )
                .body(Body::from(format!(r#"{{"value":{value}}}"#)))
                .unwrap(),
            )
            .await
            .unwrap();
        // Configuring stays allowed: a visitor can still explore the diff.
        assert_eq!(configured.status(), StatusCode::OK, "{id}");
        let cookie = cookie_of(&configured).unwrap();

        let tried = app
            .clone()
            .oneshot(
                req_from(&ip_for(&id), "POST", &format!("/api/scenarios/{id}/try"))
                    .header(header::COOKIE, &cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            tried.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{id}: try ran while the demo was disabled"
        );

        for action in actions {
            let resp = app
                .clone()
                .oneshot(
                    req_from(
                        &ip_for(&id),
                        "POST",
                        &format!("/api/scenarios/{id}/action/{action}"),
                    )
                    .header(header::COOKIE, &cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{id}/{action}: ceremony ran while the demo was disabled"
            );
        }
    }
}

/// Expiry must take a session's credentials with it, whichever scenario made
/// them.
#[tokio::test]
async fn resetting_leaves_no_credentials_behind_for_any_scenario() {
    let st = state().await;
    let credentials = st.credentials.clone();
    let sessions = st.sessions.clone();
    let app = api::build_router(st);

    // Enrol a TOTP secret, the one credential creatable without a browser.
    let configured = app
        .clone()
        .oneshot(
            req("POST", "/api/scenarios/totp/configure")
                .body(Body::from(r#"{"value":{"kind":"toggle","enabled":true}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = cookie_of(&configured).unwrap();
    app.clone()
        .oneshot(
            req("POST", "/api/scenarios/totp/action/provision")
                .header(header::COOKIE, &cookie)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    let session_id: uuid::Uuid = cookie.trim_start_matches("ak_demo=").parse().unwrap();
    let user_id = session_id.to_string();
    assert_eq!(credentials.count(&user_id, "totp").await.unwrap(), 1);

    sessions.reset(session_id).await.unwrap();

    for cred_type in ["totp", "webauthn"] {
        assert_eq!(
            credentials.count(&user_id, cred_type).await.unwrap(),
            0,
            "{cred_type} credentials survived a session reset"
        );
    }
}

/// The placeholders are test fixtures, not product.
///
/// With passkeys, OAuth and TOTP all real, "Example toggle" appearing as a
/// selectable sign-in method is just confusing — and the wizard renders its
/// method list generically from the registry, so anything registered shows up.
#[tokio::test]
async fn the_shipped_registry_offers_no_placeholder_scenarios() {
    let shipped = ScenarioRegistry::with_providers(vec!["github".to_string()]);
    let ids: Vec<&str> = shipped.iter().map(|s| s.id()).collect();

    assert!(
        !ids.iter().any(|id| id.starts_with("dummy_")),
        "placeholder scenarios must not be offered to visitors: {ids:?}"
    );
    for expected in ["passkeys", "oauth", "totp"] {
        assert!(
            ids.contains(&expected),
            "{expected} should be shipped: {ids:?}"
        );
    }
}
