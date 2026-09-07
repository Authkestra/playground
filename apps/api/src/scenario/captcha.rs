//! Bot-protection scenario: Turnstile, hCaptcha and reCAPTCHA (roadmap P2).
//!
//! One implementation, three configurations — the same shape as the OAuth
//! scenario, and for the same reason. `authkestra_engine::CaptchaVerifier`
//! takes a `CaptchaProvider` and a secret, and the only thing that varies
//! between the three is which `siteverify` endpoint it posts to. Adding a
//! provider is a match arm, not an integration.
//!
//! ## Where the check actually sits
//!
//! A captcha is not a sign-in method, which is why this scenario has no
//! ceremony in the way passkeys and TOTP do. It is a *gate in front of* a
//! sign-in method: the widget runs in the browser, hands back a token, and the
//! server spends that token on the way into the handler it is protecting. The
//! diff and the generated project both say this in as many words, because the
//! commonest way to get a captcha wrong is to verify it somewhere that the
//! caller can skip.
//!
//! ## Why an unreachable provider counts as a failure
//!
//! [`CaptchaVerifier::verify`] returns `Err` both when the provider says no
//! and when the provider could not be reached. This scenario collapses the two
//! into "not verified" rather than telling them apart. That is deliberate on
//! two counts. It fails closed, which is the defensible default for a bot
//! check. And telling them apart would mean matching on the wording of a
//! dependency's error strings, which is the coupling `resource.rs` refuses for
//! the same reason. The engine's own message is passed through as the detail,
//! so nothing is hidden — only the verdict is decided without it.
//!
//! ## Credentials
//!
//! Every provider issues a **site key** (public, rendered in the browser) and
//! a **secret** (server-side, spends the token). Both are needed before this
//! scenario can do anything, and registering them against
//! `play.authkestra.com` is a human's job — roadmap P0, issue #7. Until they
//! exist the scenario reports itself unavailable through
//! [`Scenario::unavailable_reason`] rather than offering a control that leads
//! nowhere, exactly as OAuth does.

use std::collections::BTreeMap;

use authkestra_engine::{CaptchaProvider, CaptchaVerifier};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitEnvVar, KitFragment,
    KitLink, KitOpenApiPath, KitSetup, Scenario, ScenarioContext, ScenarioOption,
};
use crate::error::ApiError;
use crate::events::Step;

/// Providers this scenario knows how to configure.
///
/// The id is the option id, the environment-variable prefix and the key into
/// [`CaptchaKeys`], so a visitor's selection flows straight through to a
/// verifier without a translation table in the middle.
pub const KNOWN_PROVIDERS: [(&str, &str); 3] = [
    ("turnstile", "Cloudflare Turnstile"),
    ("hcaptcha", "hCaptcha"),
    ("recaptcha", "Google reCAPTCHA"),
];

/// The engine's provider for one of our ids.
///
/// Returns `None` rather than defaulting, so an id that is not a provider can
/// never be silently verified against somebody else's endpoint.
fn engine_provider(id: &str) -> Option<CaptchaProvider> {
    match id {
        "turnstile" => Some(CaptchaProvider::Turnstile),
        "hcaptcha" => Some(CaptchaProvider::HCaptcha),
        "recaptcha" => Some(CaptchaProvider::ReCaptcha),
        _ => None,
    }
}

fn label_for(id: &str) -> &str {
    KNOWN_PROVIDERS
        .iter()
        .find(|(pid, _)| *pid == id)
        .map(|(_, label)| *label)
        .unwrap_or(id)
}

/// Site keys and secrets discovered in the environment.
///
/// Absent keys are not an error: the playground runs fine without them and the
/// scenario reports itself unconfigured, which is the same posture
/// [`crate::engine::ProviderCredentials`] takes for OAuth.
///
/// A `BTreeMap` rather than a `HashMap` because [`Self::configured`] feeds the
/// control's option list and the kit's emission order, and both want to be
/// stable across runs.
#[derive(Debug, Clone, Default)]
pub struct CaptchaKeys {
    /// provider id -> (site key, secret).
    keys: BTreeMap<String, (String, String)>,
}

impl CaptchaKeys {
    /// Read `<PROVIDER>_SITE_KEY` / `<PROVIDER>_SECRET_KEY` for each provider.
    ///
    /// **Both halves or neither.** A site key on its own renders a widget whose
    /// token nothing can spend, which fails at the last step of the flow with
    /// no clue as to why — worse than the control simply not being offered.
    pub fn from_env() -> Self {
        let mut keys = BTreeMap::new();
        for (id, label) in KNOWN_PROVIDERS {
            let prefix = id.to_uppercase();
            let site = std::env::var(format!("{prefix}_SITE_KEY")).unwrap_or_default();
            let secret = std::env::var(format!("{prefix}_SECRET_KEY")).unwrap_or_default();
            match (site.trim().is_empty(), secret.trim().is_empty()) {
                (false, false) => {
                    keys.insert(id.to_string(), (site, secret));
                }
                // Half-configured is worth a word: it is nearly always someone
                // who set one variable and believes they set both.
                (false, true) | (true, false) => tracing::warn!(
                    provider = id,
                    "{label} has only one of {prefix}_SITE_KEY / {prefix}_SECRET_KEY; \
                     both are required, so it will not be offered"
                ),
                (true, true) => {}
            }
        }

        if keys.is_empty() {
            tracing::warn!(
                "no captcha site keys found in the environment; the bot-protection \
                 scenario will report as not configured"
            );
        } else {
            tracing::info!(providers = ?keys.keys().collect::<Vec<_>>(), "captcha keys loaded");
        }

        Self { keys }
    }

    /// Inject keys without touching the environment.
    ///
    /// For tests: everything this scenario offers is driven by which providers
    /// have keys, and that is exactly what needs exercising.
    pub fn insert_for_test(&mut self, provider: &str, site_key: &str, secret: &str) {
        self.keys.insert(
            provider.to_string(),
            (site_key.to_string(), secret.to_string()),
        );
    }

    /// Every provider this deployment could complete a round trip with.
    pub fn configured(&self) -> Vec<String> {
        self.keys.keys().cloned().collect()
    }

    pub fn is_configured(&self, provider: &str) -> bool {
        self.keys.contains_key(provider)
    }

    /// The public half, safe to hand to the browser.
    pub fn site_key(&self, provider: &str) -> Option<&str> {
        self.keys.get(provider).map(|(site, _)| site.as_str())
    }

    /// The private half. Never serialised, never logged, never returned.
    fn secret(&self, provider: &str) -> Option<&str> {
        self.keys.get(provider).map(|(_, secret)| secret.as_str())
    }
}

/// What the browser needs to render one provider's widget.
///
/// Only the site key comes from here. The script URL and the global the script
/// installs stay in the frontend on purpose: they are fixed properties of each
/// provider rather than of this deployment, and keeping them client-side means
/// no server-supplied string is ever interpolated into a `<script src>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CaptchaWidget {
    /// Provider id — `turnstile`, `hcaptcha` or `recaptcha`.
    pub provider: String,
    /// What a visitor sees.
    pub label: String,
    /// The public key the widget is rendered with.
    pub site_key: String,
}

/// The widgets to mount, for the providers the visitor selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CaptchaWidgets {
    pub widgets: Vec<CaptchaWidget>,
}

/// The verdict on one token, as the provider gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CaptchaVerification {
    pub provider: String,
    pub label: String,
    /// True only when the provider affirmatively accepted the token.
    pub verified: bool,
    /// What happened, in the provider's own words when it had any.
    pub detail: String,
}

/// Turn what the verifier returned into a verdict.
///
/// Pure, and separate from the network call, so the fail-closed rule is
/// testable without reaching a third party. See the module docs for why `Err`
/// is not split into "rejected" and "unreachable".
fn verdict(provider: &str, outcome: Result<bool, String>) -> CaptchaVerification {
    let label = label_for(provider).to_string();
    match outcome {
        Ok(true) => CaptchaVerification {
            provider: provider.to_string(),
            label: label.clone(),
            verified: true,
            detail: format!(
                "{label} accepted the token. It was issued for this site key, it had not \
                 expired, and it had not been spent before."
            ),
        },
        // The engine returns `Ok(false)` nowhere today, but the signature
        // allows it and a future release could. Treated as a rejection, which
        // is what `false` means.
        Ok(false) => CaptchaVerification {
            provider: provider.to_string(),
            label: label.clone(),
            verified: false,
            detail: format!("{label} rejected the token."),
        },
        Err(e) => CaptchaVerification {
            provider: provider.to_string(),
            label: label.clone(),
            verified: false,
            detail: format!(
                "Not verified: {e} A check that could not be completed has not passed — \
                 this demo fails closed."
            ),
        },
    }
}

/// The providers a control value selects, in a stable order.
///
/// Sorted into [`KNOWN_PROVIDERS`] order rather than click order, so the kit
/// emits the same bytes however the visitor arrived at the same selection.
/// `SelectOne` is tolerated so a config stored under an older control shape
/// still reads sensibly instead of silently becoming empty.
fn selected_providers(value: &ControlValue) -> Vec<String> {
    let chosen: Vec<String> = match value {
        ControlValue::SelectMany { selected } => selected.clone(),
        ControlValue::SelectOne {
            selected: Some(one),
        } => vec![one.clone()],
        _ => Vec::new(),
    };

    KNOWN_PROVIDERS
        .iter()
        .filter(|(id, _)| chosen.iter().any(|c| c == id))
        .map(|(id, _)| id.to_string())
        .collect()
}

/// "a", "a and b", "a, b and c".
fn join_human(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The scenario.
///
/// Holds the keys so the control only offers providers that can actually
/// complete a round trip, and so `verify` has a secret to spend without
/// reaching back into the environment per request.
pub struct CaptchaScenario {
    keys: CaptchaKeys,
}

impl CaptchaScenario {
    pub fn new(keys: CaptchaKeys) -> Self {
        Self { keys }
    }

    /// Options for the control: every provider we hold both halves for.
    fn options(&self) -> Vec<ScenarioOption> {
        KNOWN_PROVIDERS
            .iter()
            .filter(|(id, _)| self.keys.is_configured(id))
            .map(|(id, label)| ScenarioOption::new(*id, *label))
            .collect()
    }

    /// The provider a generated project is wired with.
    ///
    /// One, even when the visitor selected several. A real application picks a
    /// provider; offering the visitor a choice of widget is a playground
    /// affordance, not a design to copy. The README says which one was chosen
    /// and how to change it.
    fn kit_provider(value: &ControlValue) -> Option<String> {
        selected_providers(value).into_iter().next()
    }

    /// Where to get the keys. Credentials are the one thing a generator cannot
    /// produce, so the README has to say exactly where they come from.
    fn kit_setup_for(provider: &str) -> Option<KitSetup> {
        let label = label_for(provider);
        let prefix = provider.to_uppercase();

        let (console, create) = match provider {
            "turnstile" => (
                "https://dash.cloudflare.com/?to=/:account/turnstile",
                "**Add site**, then list the hostnames you will serve from",
            ),
            "hcaptcha" => (
                "https://dashboard.hcaptcha.com/sites",
                "**New site**, then add your hostnames",
            ),
            "recaptcha" => (
                "https://www.google.com/recaptcha/admin",
                "**+** to register a site, choosing the **checkbox** (v2) type",
            ),
            // Only configured providers can be selected, so unreachable in
            // practice.
            _ => return None,
        };

        let mut steps = vec![
            format!("Open <{console}> and choose {create}."),
            format!(
                "You get two values. The **site key** is public — it belongs in the \
                 browser, next to the widget. The **secret key** goes in \
                 `{prefix}_SECRET_KEY` in your `.env`, and must never reach the \
                 browser; the generated `.gitignore` already excludes that file."
            ),
            "The hostname list is enforced: a widget served from a host you did not \
             register produces a token that fails verification, which looks exactly \
             like a bot. Add `localhost` while you develop."
                .to_string(),
            format!(
                "{label} publishes test keys that always pass or always fail. Use \
                 them to exercise both paths before your own domain is registered — \
                 and make sure they are gone before you deploy."
            ),
        ];

        // Google has moved reCAPTCHA's console into Google Cloud, where the
        // path it steers you down is reCAPTCHA Enterprise: an assessment call
        // to `recaptchaenterprise.googleapis.com` authenticated with a project
        // and an API key. That is a different protocol, not a different
        // credential — `CaptchaVerifier` speaks only the classic `siteverify`
        // form post, so an Enterprise API key in `RECAPTCHA_SECRET_KEY` fails
        // verification and the error reads like a bad key rather than like the
        // wrong product. Say so here: a generated README that sends a reader to
        // a console for a value it cannot use is exactly the dead end this
        // project keeps refusing to ship.
        if provider == "recaptcha" {
            steps.push(
                "**Take the legacy secret key, not an Enterprise one.** Google's console \
                 now presents reCAPTCHA Enterprise, which verifies through an assessment \
                 call to `recaptchaenterprise.googleapis.com` using a Cloud project and \
                 an API key. `CaptchaVerifier` speaks the classic \
                 `www.google.com/recaptcha/api/siteverify` form post and nothing else, so \
                 an Enterprise API key here will fail with what looks like an invalid \
                 secret. If your project will only issue Enterprise credentials, use \
                 Turnstile or hCaptcha instead — both still verify the way this code \
                 expects."
                    .to_string(),
            );
        }

        Some(KitSetup::new(&format!("Register a {label} site"), &steps))
    }
}

#[async_trait::async_trait]
impl Scenario for CaptchaScenario {
    fn id(&self) -> &'static str {
        "captcha"
    }

    fn name(&self) -> &'static str {
        "Bot protection (captcha)"
    }

    fn summary(&self) -> &'static str {
        "Put a captcha in front of sign-in. The widget runs in the browser, the token is \
         spent once on the server, and a token that does not check out gets nothing."
    }

    fn control(&self) -> ControlShape {
        // SelectMany, matching OAuth: a visitor comparing Turnstile against
        // reCAPTCHA wants both widgets on the page at once. A real application
        // picks one, which the diff and the README both say.
        ControlShape::SelectMany {
            options: self.options(),
        }
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::SelectMany {
            selected: Vec::new(),
        }
    }

    fn actions(&self) -> Vec<&'static str> {
        vec!["widget", "verify"]
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        let selected = selected_providers(value);
        if selected.is_empty() {
            return Consequences::default();
        }

        let labels: Vec<&str> = selected.iter().map(|p| label_for(p)).collect();

        let mut requirements = vec![
            format!(
                "You register a site with {} and hold two values per provider: a public \
                 site key for the browser and a secret the server spends tokens with.",
                join_human(&labels)
            ),
            "The check goes at the top of the handler you want protected — before any \
             lookup, before any work. A route whose only job is to verify a captcha \
             protects nothing, because whatever calls it can decline to call it."
                .to_string(),
            "A token is single-use and short-lived. Verify it once, on the request that \
             carried it, and never store it to check later."
                .to_string(),
            "Decide what an unreachable provider means before it happens. Failing open \
             turns an outage at your captcha vendor into an open door; failing closed \
             turns it into an outage of your own. This demo fails closed."
                .to_string(),
            "The adapter's `captcha` feature is a pure re-export of the engine's — there \
             is no middleware and no extractor. You construct a `CaptchaVerifier` and \
             call it yourself, which is why the check is visible in your handler."
                .to_string(),
        ];

        if selected.len() > 1 {
            requirements.push(
                "Selecting more than one is a playground affordance so the widgets can be \
                 compared side by side. Ship one: two providers means two scripts in \
                 every visitor's browser and two vendors who can take your sign-in down."
                    .to_string(),
            );
        }

        Consequences {
            // The captcha adds no route of its own — it guards one. This is
            // the route the generated project puts behind it, and naming it is
            // how the diff answers "where does the check sit".
            routes: vec!["POST /auth/guarded".to_string()],
            requirements,
            crates: vec![
                CrateRequirement::new("authkestra-engine", &["captcha"]),
                CrateRequirement::new("authkestra-axum", &["captcha"]),
            ],
        }
    }

    async fn action(
        &self,
        action: &str,
        body: Value,
        ctx: &ScenarioContext<'_>,
    ) -> Result<Value, ApiError> {
        let selected = selected_providers(ctx.value);
        if selected.is_empty() {
            // A caller mistake rather than a server fault, so 400 rather than
            // 500: the scenario is switched off, or off for want of keys.
            return Err(ApiError::CeremonyRejected(
                "Choose a captcha provider first.".to_string(),
            ));
        }

        match action {
            // What the browser needs to mount the widgets. Site keys are
            // public by construction — they are rendered into every page that
            // shows a widget — so returning them here gives nothing away.
            "widget" => {
                let widgets: Vec<CaptchaWidget> = selected
                    .iter()
                    .filter_map(|provider| {
                        Some(CaptchaWidget {
                            provider: provider.clone(),
                            label: label_for(provider).to_string(),
                            site_key: self.keys.site_key(provider)?.to_string(),
                        })
                    })
                    .collect();

                Ok(serde_json::to_value(CaptchaWidgets { widgets }).expect("widgets serialise"))
            }

            "verify" => {
                let provider = body
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default();
                // Only a provider the visitor actually turned on. Otherwise a
                // caller could pick which of this deployment's secrets to
                // spend, and burn quota on a provider nobody selected.
                if !selected.iter().any(|p| p == provider) {
                    return Err(ApiError::CeremonyRejected(
                        "That captcha provider is not switched on for this session.".to_string(),
                    ));
                }

                let token = body
                    .get("token")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| {
                        ApiError::CeremonyRejected(
                            "No captcha token. Complete the widget first, or use the \
                             failure button to send a token that cannot pass."
                                .to_string(),
                        )
                    })?;

                let (Some(engine), Some(secret)) =
                    (engine_provider(provider), self.keys.secret(provider))
                else {
                    // Selection is filtered by `options()`, which only offers
                    // configured providers, so this is unreachable in practice.
                    return Err(ApiError::CeremonyRejected(
                        "That captcha provider is not configured on this deployment.".to_string(),
                    ));
                };

                // `remoteip` is deliberately not sent. It is optional at every
                // provider and tightens the check, but the playground sits
                // behind proxies whose client-IP header is deployment-specific
                // (see `ClientIpKeyExtractor`) — sending the wrong address
                // would fail verification in a way indistinguishable from a
                // bot, which is the opposite of what this page is for.
                let outcome = CaptchaVerifier::new(engine, secret)
                    .verify(token, None)
                    .await;
                let result = verdict(provider, outcome);

                let step = if result.verified {
                    Step::success("captcha", "captcha passed")
                } else {
                    Step::rejected("captcha", "captcha failed")
                };
                ctx.record(
                    step.detail(result.detail.clone())
                        .fact("provider", result.label.clone())
                        .fact("verified", result.verified.to_string()),
                )
                .await;

                Ok(serde_json::to_value(result).expect("verification serialises"))
            }

            other => Err(ApiError::UnknownAction {
                scenario: "captcha".to_string(),
                action: other.to_string(),
            }),
        }
    }

    fn kit_fragment(&self, value: &ControlValue, ctx: &KitContext<'_>) -> Option<KitFragment> {
        let selected = selected_providers(value);
        let provider = Self::kit_provider(value)?;
        let label = label_for(&provider);
        let prefix = provider.to_uppercase();
        let variant = match provider.as_str() {
            "turnstile" => "Turnstile",
            "hcaptcha" => "HCaptcha",
            "recaptcha" => "ReCaptcha",
            _ => return None,
        };

        let mut notes = vec![format!(
            "**Bot protection with {label}.** `POST /auth/guarded` will not run its body \
             without a token that {label} accepts. The route is a demonstration of the \
             *shape*: move those first few lines to the top of whichever handler you \
             actually want to protect."
        )];
        notes.push(
            "The secret key spends tokens and must stay on the server. The site key is \
             public and belongs in your frontend — they are not interchangeable, and \
             swapping them fails in a way that reads as a bot rather than as a typo."
                .to_string(),
        );
        // The obvious thing to protect is whatever else was generated, so name
        // it rather than leaving the reader to make the connection.
        let guardable: Vec<&str> = [
            ("totp", "`POST /auth/totp/verify`"),
            ("passkeys", "`POST /auth/passkey/authenticate/start`"),
        ]
        .iter()
        .filter(|(id, _)| ctx.is_active(id))
        .map(|(_, route)| *route)
        .collect();
        if !guardable.is_empty() {
            notes.push(format!(
                "In this project the endpoint worth guarding is {} — it is the one an \
                 attacker can hammer. Move the check there and delete `/auth/guarded`.",
                join_human(&guardable)
            ));
        }
        if selected.len() > 1 {
            let others: Vec<&str> = selected
                .iter()
                .filter(|p| *p != &provider)
                .map(|p| label_for(p))
                .collect();
            notes.push(format!(
                "You selected {} as well. This project is wired with {label} only, \
                 because an application ships one captcha rather than a choice of them. \
                 Switching is one enum variant and one environment variable.",
                join_human(&others)
            ));
        }

        Some(KitFragment {
            imports: vec![
                "use axum::extract::State;".to_string(),
                "use axum::routing::post;".to_string(),
            ],
            prelude: vec![format!(
                r#"    // The captcha secret spends tokens; it is a server-side credential and
    // must never be sent to the browser. Its public counterpart — the site
    // key — is what your frontend renders the widget with.
    let captcha = std::sync::Arc::new(authkestra_engine::CaptchaVerifier::new(
        authkestra_engine::CaptchaProvider::{variant},
        &std::env::var("{prefix}_SECRET_KEY").expect("{prefix}_SECRET_KEY must be set"),
    ));"#
            )],
            // Nothing on the builder: the verifier is not part of the engine's
            // typestate. A captcha guards a handler, it does not authenticate
            // anyone, so it never joins the auth chain.
            builder_calls: Vec::new(),
            routes: vec![
                r#"        // Protected by the captcha check, which is the first thing the
        // handler does — see `guarded` below.
        .route("/auth/guarded", post(guarded))"#
                    .to_string(),
            ],
            handlers: vec![GUARDED_HANDLER.to_string()],
            state_fields: vec![format!(
                "    /// Spends captcha tokens against {label}'s siteverify endpoint.\n\
                 \x20   captcha: std::sync::Arc<authkestra_engine::CaptchaVerifier>,"
            )],
            state_init: vec!["        captcha: captcha.clone(),".to_string()],
            openapi_paths: vec![KitOpenApiPath {
                handler: "guarded".to_string(),
                annotation: r##"#[utoipa::path(
    post,
    path = "/auth/guarded",
    request_body = GuardedRequest,
    responses(
            (status = 200, description = "The captcha checked out and the handler ran"),
            (status = 403, description = "The token was rejected, or could not be checked"),
    ),
    tag = "captcha",
)]"##
                    .to_string(),
            }],
            openapi_schemas: vec!["GuardedRequest".to_string()],
            crates: vec![CrateRequirement::new("serde", &["derive"])],
            env: vec![KitEnvVar::required(
                &format!("{prefix}_SECRET_KEY"),
                &format!(
                    "Secret half of your {label} site keys. Server-side only — the public \
                     site key belongs in your frontend."
                ),
            )],
            notes,
            setup: Self::kit_setup_for(&provider).into_iter().collect(),
            links: vec![KitLink::docs("Bot protection", "advanced/captcha")],
            needs_credential_store: false,
        })
    }

    fn unavailable_reason(&self) -> Option<String> {
        if self.keys.configured().is_empty() {
            Some("No captcha site keys are configured on this deployment yet.".to_string())
        } else {
            None
        }
    }
}

/// The generated handler for the guarded route.
const GUARDED_HANDLER: &str = r##"#[derive(serde::Deserialize)]
struct GuardedRequest {
    /// The token the widget produced in the browser. Single-use, short-lived,
    /// and worthless once spent.
    captcha_token: String,
}

/// An endpoint that will not do its work without a token the provider accepts.
///
/// The shape matters more than the route. The check is the *first* thing the
/// handler does — before any database lookup, before any credential is read,
/// before anything an attacker could make expensive. Move these lines to the
/// top of whichever handler you actually want to protect: sign-in, sign-up,
/// password reset, "email me a code".
///
/// What not to do instead: expose a route whose only job is to verify a
/// captcha and answer "yes". That protects nothing, because whatever was
/// supposed to call it can simply not call it, and the endpoint you cared
/// about never learns the difference.
async fn guarded(
    State(state): State<AppState>,
    Json(body): Json<GuardedRequest>,
) -> impl IntoResponse {
    // `verify` returns `Err` both when the provider rejects the token and when
    // the provider could not be reached. They are collapsed here on purpose: a
    // check that did not complete has not passed. If you would rather stay
    // open while your captcha vendor is down, make that an explicit branch on
    // the error — it is a real trade-off, not a detail to leave implicit.
    if let Err(e) = state.captcha.verify(&body.captcha_token, None).await {
        // Worth logging, never worth returning: the provider's error codes
        // tell an attacker which part of their forgery to fix.
        tracing::warn!(error = %e, "captcha token rejected");
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "captcha_failed" })),
        )
            .into_response();
    }

    // Past this point the caller has cleared the bot check. Do the real work.
    Json(json!({ "ok": true })).into_response()
}"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> CaptchaKeys {
        let mut k = CaptchaKeys::default();
        k.insert_for_test("turnstile", "ts-site", "ts-secret");
        k.insert_for_test("hcaptcha", "hc-site", "hc-secret");
        k.insert_for_test("recaptcha", "rc-site", "rc-secret");
        k
    }

    fn scenario() -> CaptchaScenario {
        CaptchaScenario::new(keys())
    }

    fn select(ids: &[&str]) -> ControlValue {
        ControlValue::SelectMany {
            selected: ids.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn every_known_provider_maps_to_an_engine_provider() {
        for (id, _) in KNOWN_PROVIDERS {
            assert!(
                engine_provider(id).is_some(),
                "{id} is offered but has no engine provider"
            );
        }
        assert!(engine_provider("definitely-not-a-provider").is_none());
    }

    /// The control must never offer something the deployment cannot finish.
    #[test]
    fn only_providers_with_both_halves_are_offered() {
        let mut k = CaptchaKeys::default();
        k.insert_for_test("turnstile", "ts-site", "ts-secret");
        let s = CaptchaScenario::new(k);

        let ids: Vec<String> = s.options().into_iter().map(|o| o.id).collect();
        assert_eq!(ids, vec!["turnstile".to_string()]);
        assert!(s.unavailable_reason().is_none());
    }

    #[test]
    fn a_deployment_with_no_keys_says_so_rather_than_offering_a_dead_end() {
        let s = CaptchaScenario::new(CaptchaKeys::default());
        assert!(s.options().is_empty());
        let reason = s
            .unavailable_reason()
            .expect("an empty control needs a reason");
        assert!(
            reason.to_lowercase().contains("configured"),
            "the reason should name the cause: {reason}"
        );
    }

    /// A site key with no secret renders a widget whose token nothing can
    /// spend. Half-configured must mean not configured.
    #[test]
    fn half_a_key_pair_is_not_configured() {
        let mut k = CaptchaKeys::default();
        k.insert_for_test("turnstile", "ts-site", "ts-secret");
        assert!(k.is_configured("turnstile"));
        assert!(!k.is_configured("hcaptcha"));
        assert_eq!(k.site_key("hcaptcha"), None);
        assert_eq!(k.secret("hcaptcha"), None);
    }

    /// Two visitors clicking the same providers in different orders must get
    /// byte-identical projects, so the selection is normalised, not preserved.
    #[test]
    fn selection_is_normalised_to_a_stable_order() {
        assert_eq!(
            selected_providers(&select(&["recaptcha", "turnstile"])),
            selected_providers(&select(&["turnstile", "recaptcha"])),
        );
        assert_eq!(
            selected_providers(&select(&["recaptcha", "turnstile"])),
            vec!["turnstile".to_string(), "recaptcha".to_string()],
            "KNOWN_PROVIDERS order, not click order"
        );
    }

    /// A config stored under an older single-select control must still read.
    #[test]
    fn a_select_one_value_is_still_understood() {
        let value = ControlValue::SelectOne {
            selected: Some("hcaptcha".to_string()),
        };
        assert_eq!(selected_providers(&value), vec!["hcaptcha".to_string()]);
    }

    #[test]
    fn switched_off_it_contributes_nothing() {
        let s = scenario();
        let c = s.consequences(&s.default_value());
        assert!(c.routes.is_empty() && c.requirements.is_empty() && c.crates.is_empty());
        assert!(s
            .kit_fragment(
                &s.default_value(),
                &KitContext {
                    active: &[],
                    options: Default::default()
                }
            )
            .is_none());
    }

    #[test]
    fn switched_on_it_names_the_route_it_guards_and_the_engine_feature() {
        let c = scenario().consequences(&select(&["turnstile"]));
        assert_eq!(c.routes, vec!["POST /auth/guarded".to_string()]);

        let engine = c
            .crates
            .iter()
            .find(|c| c.name == "authkestra-engine")
            .expect("the engine is where the verifier lives");
        assert!(engine.features.contains(&"captcha".to_string()));

        // The facade deliberately does not expose `captcha`; pointing at it
        // would send a reader down a dead end.
        assert!(!c.crates.iter().any(|c| c.name == "authkestra"));
    }

    /// The honest half of a select-many control: say that shipping two is not
    /// the design being demonstrated.
    #[test]
    fn selecting_several_says_to_ship_one() {
        let one = scenario().consequences(&select(&["turnstile"]));
        let two = scenario().consequences(&select(&["turnstile", "hcaptcha"]));

        assert!(two.requirements.len() > one.requirements.len());
        assert!(
            two.requirements.iter().any(|r| r.contains("Ship one")),
            "{:?}",
            two.requirements
        );
    }

    /// The fail-closed rule, without a network.
    #[test]
    fn a_check_that_could_not_be_completed_has_not_passed() {
        let unreachable = verdict(
            "turnstile",
            Err("Network request to CAPTCHA API failed: dns error".to_string()),
        );
        assert!(!unreachable.verified);
        assert!(
            unreachable.detail.contains("fails closed"),
            "the visitor should be told which way it failed: {}",
            unreachable.detail
        );

        let rejected = verdict(
            "hcaptcha",
            Err("CAPTCHA verification failed: invalid-input-response".to_string()),
        );
        assert!(!rejected.verified);
        // The provider's own words survive; only the verdict is ours.
        assert!(rejected.detail.contains("invalid-input-response"));

        assert!(!verdict("recaptcha", Ok(false)).verified);
        assert!(verdict("turnstile", Ok(true)).verified);
    }

    /// Every verdict names the provider it came from, since a page can be
    /// showing three widgets at once.
    #[test]
    fn a_verdict_names_its_provider() {
        let v = verdict("recaptcha", Ok(true));
        assert_eq!(v.provider, "recaptcha");
        assert_eq!(v.label, "Google reCAPTCHA");
    }

    #[test]
    fn the_kit_wires_exactly_one_provider_and_says_which() {
        let ctx = KitContext {
            active: &["captcha".to_string()],
            options: Default::default(),
        };
        let f = scenario()
            .kit_fragment(&select(&["hcaptcha", "recaptcha"]), &ctx)
            .expect("a selection produces a fragment");

        let prelude = f.prelude.join("\n");
        assert!(prelude.contains("CaptchaProvider::HCaptcha"), "{prelude}");
        assert!(
            !prelude.contains("ReCaptcha"),
            "only one verifier should be constructed: {prelude}"
        );
        assert_eq!(f.env.len(), 1);
        assert_eq!(f.env[0].name, "HCAPTCHA_SECRET_KEY");
        assert!(
            f.env[0].default.is_none(),
            "a secret has no default, so a project without it fails loudly"
        );
        assert!(
            f.notes.iter().any(|n| n.contains("Google reCAPTCHA")),
            "the unused selection should be accounted for: {:?}",
            f.notes
        );
    }

    /// The site key is public, but it must not leak into the generated
    /// project's server code — that is where the *secret* goes, and confusing
    /// the two is the mistake the setup steps warn about.
    #[test]
    fn the_kit_never_emits_a_key_value_of_its_own() {
        let ctx = KitContext {
            active: &["captcha".to_string()],
            options: Default::default(),
        };
        let f = scenario()
            .kit_fragment(&select(&["turnstile"]), &ctx)
            .expect("fragment");

        let emitted = format!("{}{}", f.prelude.join("\n"), f.handlers.join("\n"));
        for leaked in ["ts-site", "ts-secret"] {
            assert!(
                !emitted.contains(leaked),
                "a deployment's own key reached the generated project: {leaked}"
            );
        }
        assert!(emitted.contains("TURNSTILE_SECRET_KEY"));
    }

    /// The point of the fragment is the placement of the check, so the
    /// generated project has to say what to guard when it knows.
    #[test]
    fn the_kit_names_the_endpoint_worth_guarding_when_there_is_one() {
        let alone = KitContext {
            active: &["captcha".to_string()],
            options: Default::default(),
        };
        let with_totp = KitContext {
            active: &["totp".to_string(), "captcha".to_string()],
            options: Default::default(),
        };
        let s = scenario();

        let solo = s.kit_fragment(&select(&["turnstile"]), &alone).expect("f");
        assert!(!solo.notes.iter().any(|n| n.contains("totp")));

        let paired = s
            .kit_fragment(&select(&["turnstile"]), &with_totp)
            .expect("f");
        assert!(
            paired.notes.iter().any(|n| n.contains("/auth/totp/verify")),
            "{:?}",
            paired.notes
        );
    }

    #[test]
    fn every_provider_has_setup_steps_naming_its_console() {
        for (id, _) in KNOWN_PROVIDERS {
            let setup = CaptchaScenario::kit_setup_for(id)
                .unwrap_or_else(|| panic!("{id} has no setup steps"));
            assert!(setup.title.contains(label_for(id)));
            assert!(
                setup.steps.iter().any(|s| s.contains("https://")),
                "{id}: the reader needs somewhere to go"
            );
            assert!(
                setup
                    .steps
                    .iter()
                    .any(|s| s.contains(&format!("{}_SECRET_KEY", id.to_uppercase()))),
                "{id}: the steps should name the variable they fill"
            );
        }
    }

    /// The reCAPTCHA step has to name the protocol split, or the generated
    /// README sends a reader to a console for a credential this code cannot
    /// spend. `CaptchaVerifier` posts to `siteverify` and nothing else.
    #[test]
    fn the_recaptcha_steps_warn_that_an_enterprise_key_will_not_work() {
        let setup = CaptchaScenario::kit_setup_for("recaptcha").expect("setup steps");
        let joined = setup.steps.join(" ");
        assert!(joined.contains("Enterprise"), "{joined}");
        assert!(joined.contains("legacy secret key"), "{joined}");
        // And it must offer a way forward rather than only a warning.
        assert!(
            joined.contains("Turnstile") || joined.contains("hCaptcha"),
            "a caveat with no alternative is a dead end: {joined}"
        );

        // The other two verify the way the engine expects, so the caveat would
        // only be noise there.
        for id in ["turnstile", "hcaptcha"] {
            let other = CaptchaScenario::kit_setup_for(id).expect("setup steps");
            assert!(
                !other.steps.join(" ").contains("Enterprise"),
                "{id} carries a caveat that is not its problem"
            );
        }
    }

    #[test]
    fn validate_rejects_an_option_that_was_never_offered() {
        let s = scenario();
        assert!(s.validate(&select(&["turnstile"])).is_ok());
        assert!(s.validate(&select(&["mystery-captcha"])).is_err());
        assert!(s.validate(&ControlValue::Toggle { enabled: true }).is_err());
    }
}
