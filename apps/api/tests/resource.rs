//! The resource server, end to end over a real socket (roadmap #52).
//!
//! Every other test in this repository drives the router with `oneshot`, which
//! never opens a port. That is not enough here: the whole claim of this
//! scenario is that the validating side **discovers** the signing key over
//! HTTP, and a validator pointed at a URL nothing is listening on can only
//! ever demonstrate the fail-closed path.
//!
//! So this suite binds a real listener, serves the real router on it, and
//! points the deployment's issuer at its own address. The key set is then
//! genuinely fetched over the loopback interface by
//! `authkestra-resource`'s `JwksCache` — the same code a deployment runs
//! against somebody else's issuer.
//!
//! The pure classification of every failure lives in the scenario's own unit
//! tests. What is asserted here is that the pieces are wired to each other:
//! that a token this deployment signed validates against the key set this
//! deployment publishes, and that each forgery fails for the reason it was
//! built to fail for.

use std::net::SocketAddr;

use api::killswitch::KillSwitch;
use api::testing::{test_settings, test_state_with_settings};

/// A running API, and where to reach it.
struct Server {
    base: String,
    client: reqwest::Client,
    /// The demo-session cookie, carried by hand.
    ///
    /// `reqwest`'s own cookie store is behind a feature that pulls another
    /// crate, and this workspace is deliberately careful about what it adds to
    /// `reqwest` — see the dev-dependency comment in `Cargo.toml`. One header
    /// is cheaper than a dependency.
    session: std::sync::Mutex<Option<String>>,
}

impl Server {
    /// Bind, serve, and point the issuer at ourselves.
    ///
    /// The issuer has to be the address we are actually listening on, because
    /// it is also the JWKS prefix — the validator resolves the token's `iss`
    /// to a key-set URL and fetches it. Getting these out of step is the
    /// commonest real misconfiguration, and here it would simply not work.
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback port");
        let addr = listener.local_addr().expect("local address");
        let base = format!("http://{addr}");

        let mut settings = test_settings(None);
        settings.public_base_url = base.clone();

        let state = test_state_with_settings(KillSwitch::default(), settings);
        let app = api::build_router(state);

        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("server runs");
        });

        Self {
            base,
            client: reqwest::Client::new(),
            session: std::sync::Mutex::new(None),
        }
    }

    fn cookie(&self) -> Option<String> {
        self.session.lock().expect("session lock").clone()
    }

    /// Remember the session cookie the API just set, if it set one.
    fn remember(&self, resp: &reqwest::Response) {
        if let Some(pair) = resp
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
        {
            *self.session.lock().expect("session lock") = Some(pair.to_string());
        }
    }

    /// POST JSON, from a caller-chosen client IP.
    ///
    /// The IP varies per logical step because these endpoints share the
    /// tighter rate-limit bucket (10 in a burst), and one run here makes more
    /// than that. The limiter is working; this test is not about it.
    async fn post(&self, path: &str, ip: &str, body: serde_json::Value) -> serde_json::Value {
        let mut request = self
            .client
            .post(format!("{}{path}", self.base))
            .header("x-forwarded-for", ip)
            .json(&body);
        if let Some(cookie) = self.cookie() {
            request = request.header(reqwest::header::COOKIE, cookie);
        }

        let resp = request
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e}"));
        self.remember(&resp);

        let status = resp.status();
        let text = resp.text().await.expect("a body");
        assert!(status.is_success(), "POST {path} answered {status}: {text}");
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("POST {path} sent {text}: {e}"))
    }

    async fn get_json(&self, path: &str) -> serde_json::Value {
        self.client
            .get(format!("{}{path}", self.base))
            .header("x-forwarded-for", "198.18.9.9")
            .send()
            .await
            .expect("GET")
            .json()
            .await
            .expect("JSON")
    }

    /// Switch the scenario on. Establishes the session every later call rides.
    async fn enable(&self) {
        self.post(
            "/api/scenarios/resource/configure",
            "198.18.0.1",
            serde_json::json!({ "value": { "kind": "toggle", "enabled": true } }),
        )
        .await;
    }

    async fn issue(&self, ip: &str) -> serde_json::Value {
        self.post(
            "/api/scenarios/resource/action/issue",
            ip,
            serde_json::json!({}),
        )
        .await
    }

    async fn forge(&self, ip: &str, kind: &str) -> serde_json::Value {
        self.post(
            "/api/scenarios/resource/action/forge",
            ip,
            serde_json::json!({ "kind": kind }),
        )
        .await
    }

    async fn call(&self, ip: &str, token: Option<&str>) -> serde_json::Value {
        self.post(
            "/api/scenarios/resource/action/call",
            ip,
            match token {
                Some(t) => serde_json::json!({ "token": t }),
                None => serde_json::json!({}),
            },
        )
        .await
    }
}

/// The happy path, and the only one that proves discovery works at all: a
/// token signed by the deployment's key, validated by fetching that key over
/// HTTP and matching on `kid`.
#[tokio::test]
async fn a_token_validates_against_the_key_set_the_issuer_publishes() {
    let server = Server::start().await;
    server.enable().await;

    let issued = server.issue("198.18.1.1").await;
    let token = issued["token"].as_str().expect("a token");
    let kid = issued["kid"].as_str().expect("a kid");

    // The `kid` really is in the document the validator will fetch. This is
    // the check a visitor is invited to do by hand, so it is worth asserting.
    let jwks = server.get_json("/.well-known/jwks.json").await;
    let published: Vec<&str> = jwks["keys"]
        .as_array()
        .expect("keys")
        .iter()
        .filter_map(|k| k["kid"].as_str())
        .collect();
    assert!(
        published.contains(&kid),
        "the issued token names {kid}, which is not published: {published:?}"
    );

    let result = server.call("198.18.1.2", Some(token)).await;
    assert_eq!(result["verdict"], "accepted", "{result}");
    assert_eq!(result["status"], 200);
    assert!(
        result["subject"].is_string(),
        "an accepted token should name its subject: {result}"
    );
}

/// The token the issuer hands out advertises everything needed to check it
/// independently. Without these the "go and look" invitation is empty.
#[tokio::test]
async fn an_issued_token_says_where_its_key_is_published() {
    let server = Server::start().await;
    server.enable().await;

    let issued = server.issue("198.18.2.1").await;
    assert_eq!(issued["audience"], "playground-api");
    assert!(issued["forged_as"].is_null(), "an honest token: {issued}");

    let jwks_url = issued["jwks_url"].as_str().expect("a jwks_url");
    assert!(
        jwks_url.ends_with("/.well-known/jwks.json"),
        "{jwks_url} is not the well-known path"
    );
    assert_eq!(
        issued["issuer"]
            .as_str()
            .map(|i| format!("{i}/.well-known/jwks.json")),
        Some(jwks_url.to_string()),
        "the key set must live under the issuer the token names"
    );
}

/// The heart of it. Each forgery is a real, correctly signed token that fails
/// one specific check, and each has to fail for *its own* reason — a scenario
/// that collapsed these into one 401 would teach nothing, which is the whole
/// reason it was rebuilt.
#[tokio::test]
async fn every_forgery_fails_for_the_reason_it_was_built_to_fail_for() {
    let server = Server::start().await;
    server.enable().await;

    // (wire name, expected verdict, expected status)
    let cases = [
        ("unknown_kid", "unknown_kid", 401),
        ("bad_signature", "bad_signature", 401),
        ("untrusted_issuer", "untrusted_issuer", 401),
        ("wrong_audience", "wrong_audience", 401),
        ("expired", "expired", 401),
        ("missing_kid", "missing_kid", 401),
    ];

    for (i, (kind, expected, status)) in cases.iter().enumerate() {
        let ip = format!("198.19.{}.1", i + 1);
        let forged = server.forge(&ip, kind).await;
        assert!(
            forged["forged_as"].is_string(),
            "{kind} should say what it is: {forged}"
        );

        let token = forged["token"].as_str().expect("a forged token");
        let result = server
            .call(&format!("198.19.{}.2", i + 1), Some(token))
            .await;

        assert_eq!(
            result["verdict"], *expected,
            "{kind} produced {} rather than {expected}: {}",
            result["verdict"], result["detail"]
        );
        assert_eq!(result["status"], *status, "{kind}: {result}");
    }
}

/// `unknown_kid` is the one a visitor can disprove for themselves, so the
/// property that makes that possible is worth its own assertion: the forged
/// token names a key the published set does not contain.
#[tokio::test]
async fn a_forged_tokens_key_is_genuinely_absent_from_the_published_set() {
    let server = Server::start().await;
    server.enable().await;

    let forged = server.forge("198.20.1.1", "unknown_kid").await;
    let forged_kid = forged["kid"].as_str().expect("a kid");

    let jwks = server.get_json("/.well-known/jwks.json").await;
    let published: Vec<&str> = jwks["keys"]
        .as_array()
        .expect("keys")
        .iter()
        .filter_map(|k| k["kid"].as_str())
        .collect();

    assert!(
        !published.contains(&forged_kid),
        "the forgery claims {forged_kid}, which IS published — then it is not a forgery"
    );
}

/// A token minted for another audience is signed by the *published* key, so
/// this is the case that separates "is the signature good" from "is this token
/// for me". Both have to be checked, and only one of them is cryptography.
#[tokio::test]
async fn a_correctly_signed_token_for_another_service_is_still_refused() {
    let server = Server::start().await;
    server.enable().await;

    let forged = server.forge("198.20.2.1", "wrong_audience").await;
    assert_eq!(
        forged["kid"].as_str(),
        server.get_json("/.well-known/jwks.json").await["keys"][0]["kid"].as_str(),
        "this forgery is meant to use the real, published key"
    );
    assert_eq!(forged["audience"], "somebody-elses-api");

    let result = server.call("198.20.2.2", forged["token"].as_str()).await;
    assert_eq!(result["verdict"], "wrong_audience", "{result}");
}

#[tokio::test]
async fn no_token_and_junk_are_told_apart() {
    let server = Server::start().await;
    server.enable().await;

    let absent = server.call("198.20.3.1", None).await;
    assert_eq!(absent["verdict"], "absent", "{absent}");

    for junk in ["hello", "a.b", "not.a.jwt.at.all"] {
        let result = server.call("198.20.3.2", Some(junk)).await;
        assert_eq!(result["verdict"], "malformed", "{junk}: {result}");
    }
}

/// Tampering after signing has to be distinguishable from a key nobody
/// published — same outcome, different cause, different thing to go and fix.
///
/// The **signature** segment is what gets edited, not the payload. Editing a
/// compact JWS payload almost always breaks its base64 or its JSON too, and
/// the validator then reports it as malformed — which is correct, and is what
/// the first version of this test tripped over. A tampered signature leaves a
/// perfectly readable token whose only fault is that it does not verify, which
/// is the case worth telling apart.
#[tokio::test]
async fn a_token_whose_signature_was_altered_fails_on_the_signature() {
    let server = Server::start().await;
    server.enable().await;

    let issued = server.issue("198.20.4.1").await;
    let token = issued["token"].as_str().expect("a token");

    // Edit the *first* character of the signature, not the last. A 64-byte ES256
    // signature encodes to 86 base64url characters whose final character carries
    // only two significant bits, so `A`, `Q`, `g` and `w` are the only legal values
    // there. ECDSA signatures are randomised, so the old last-character flip drew a
    // fresh final character every run and produced invalid base64 — reported, quite
    // correctly, as `malformed` — the one run in four that it landed on `A`. The
    // first character is unconstrained, so any substitution stays decodable and the
    // token fails on the signature, which is what this test is about.
    let (signed_part, signature) = token.rsplit_once('.').expect("a signature");
    let mut chars = signature.chars();
    let first = chars.next().expect("a non-empty signature");
    let altered: String =
        std::iter::once(if first == 'A' { 'B' } else { 'A' }).chain(chars).collect();

    let result = server
        .call("198.20.4.2", Some(&format!("{signed_part}.{altered}")))
        .await;
    assert_eq!(
        result["verdict"], "bad_signature",
        "the header and payload are intact, so only the signature can be at fault: {result}"
    );
}

/// The companion case, and the reason the test above edits the signature: an
/// edited *payload* is malformed rather than badly signed, because it stops
/// being readable JSON before anyone gets to check a signature.
#[tokio::test]
async fn an_edited_payload_is_malformed_rather_than_badly_signed() {
    let server = Server::start().await;
    server.enable().await;

    let issued = server.issue("198.20.6.1").await;
    let token = issued["token"].as_str().expect("a token");

    let (head, rest) = token.split_once('.').expect("a header");
    let (payload, signature) = rest.split_once('.').expect("a payload");
    let mut edited = payload.to_string();
    let last = edited.pop().expect("a non-empty payload");
    edited.push(if last == 'A' { 'B' } else { 'A' });

    let result = server
        .call("198.20.6.2", Some(&format!("{head}.{edited}.{signature}")))
        .await;
    assert_eq!(result["verdict"], "malformed", "{result}");
}

/// The diff has to name what actually changed, and after the rebuild that is
/// a different crate from before.
#[tokio::test]
async fn turning_it_on_names_the_resource_crate_and_the_key_set_route() {
    let server = Server::start().await;
    let response = server
        .post(
            "/api/scenarios/resource/configure",
            "198.20.5.1",
            serde_json::json!({ "value": { "kind": "toggle", "enabled": true } }),
        )
        .await;

    let diff = response["diff"].to_string();
    assert!(diff.contains("authkestra-resource"), "{diff}");
    assert!(diff.contains("/.well-known/jwks.json"), "{diff}");
    assert!(diff.contains("/api/protected"), "{diff}");
    // The facade does not expose this, and neither does the engine's `token`
    // feature on its own any more.
    assert!(!diff.contains("\"authkestra\""), "{diff}");
}
