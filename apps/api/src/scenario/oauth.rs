//! OAuth scenario: sign in with GitHub, Google or Discord (roadmap P2).
//!
//! One implementation, three configurations. The provider files in
//! `authkestra-providers` are thin endpoint config; the flow itself is the
//! engine's generic `OAuth2Flow`, so adding a provider is credentials plus a
//! feature flag rather than new logic.
//!
//! ## Why this scenario has no `action` steps
//!
//! Unlike TOTP and passkeys, OAuth is not an XHR ceremony. The browser is
//! *navigated* to the provider and comes back to a callback URL, so the flow
//! lives in two ordinary GET routes (`/auth/login/{provider}` and
//! `/auth/callback/{provider}`) rather than the generic action endpoint. The
//! scenario still owns the control, the diff and the readiness check; it just
//! hands the frontend a URL to send the visitor to.
//!
//! ## Stateless vs session
//!
//! Both are offered, because the stateless variant is a genuine selling point:
//! `state` and `nonce` live in an encrypted cookie rather than a database, so
//! the callback verifies itself with no server-side lookup. The login route
//! takes `?mode=session` (default) or `?mode=jwt`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitEnvVar, KitFragment,
    Scenario, ScenarioContext, ScenarioOption, TryOutcome, TryResult,
};
use crate::error::ApiError;

/// Providers this scenario knows how to configure.
///
/// The label is what a visitor sees; the id is both the option id and the
/// provider id the engine registers, which is what lets a visitor's selection
/// flow straight through to engine construction.
pub const KNOWN_PROVIDERS: [(&str, &str); 3] = [
    ("github", "GitHub"),
    ("google", "Google"),
    ("discord", "Discord"),
];

/// How the callback should establish the visitor's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OAuthMode {
    /// Server-side session, id in a cookie.
    Session,
    /// A signed JWT returned to the caller; nothing stored server-side.
    Jwt,
}

impl OAuthMode {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("jwt") | Some("stateless") | Some("token") => OAuthMode::Jwt,
            _ => OAuthMode::Session,
        }
    }
}

/// The providers a control value selects.
///
/// `SelectOne` is still tolerated so a config stored before this control became
/// multi-select still reads sensibly rather than silently becoming empty.
fn selected_providers(value: &ControlValue) -> Vec<String> {
    match value {
        ControlValue::SelectMany { selected } => selected.clone(),
        ControlValue::SelectOne {
            selected: Some(one),
        } => vec![one.clone()],
        _ => Vec::new(),
    }
}

/// "a", "a and b", "a, b and c" — read out in prose rather than as a list.
fn join_human(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The scenario.
///
/// Holds the configured provider list so the control only offers providers that
/// can actually complete a round trip.
pub struct OAuthScenario {
    configured: Vec<String>,
}

impl OAuthScenario {
    pub fn new(configured: Vec<String>) -> Self {
        Self { configured }
    }

    /// Options for the control: every provider we have credentials for.
    fn options(&self) -> Vec<ScenarioOption> {
        KNOWN_PROVIDERS
            .iter()
            .filter(|(id, _)| self.configured.iter().any(|c| c == id))
            .map(|(id, label)| ScenarioOption::new(*id, *label))
            .collect()
    }

    fn label_for(id: &str) -> &str {
        KNOWN_PROVIDERS
            .iter()
            .find(|(pid, _)| *pid == id)
            .map(|(_, label)| *label)
            .unwrap_or(id)
    }
}

#[async_trait::async_trait]
impl Scenario for OAuthScenario {
    fn id(&self) -> &'static str {
        "oauth"
    }

    fn name(&self) -> &'static str {
        "Sign in with a provider (OAuth)"
    }

    fn summary(&self) -> &'static str {
        "Hand sign-in to GitHub, Google or Discord. The playground never sees a \
         password, and OAuth state travels in an encrypted cookie rather than a database."
    }

    fn control(&self) -> ControlShape {
        // SelectMany, not SelectOne: real applications offer several providers
        // side by side, and a visitor comparing them wants to see the combined
        // consequences — two providers is not twice one provider's config.
        ControlShape::SelectMany {
            options: self.options(),
        }
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::SelectMany {
            selected: Vec::new(),
        }
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        let selected = selected_providers(value);
        if selected.is_empty() {
            return Consequences::default();
        }

        // One crate, one feature per provider — the useful detail is that
        // adding a second provider is a feature flag, not a second integration.
        let provider_features: Vec<&str> = selected.iter().map(|s| s.as_str()).collect();
        let mut crates = vec![
            CrateRequirement::new("authkestra-providers", &provider_features),
            CrateRequirement::new("authkestra-engine", &["session", "token"]),
        ];

        let labels: Vec<&str> = selected.iter().map(|p| Self::label_for(p)).collect();
        let mut requirements = vec![
            format!(
                "You register an OAuth app with {} and store each client id and secret.",
                join_human(&labels)
            ),
            "Every redirect URI must match exactly, including scheme and path.".to_string(),
            "No password is ever stored, so there is no password reset to build.".to_string(),
        ];

        if selected.len() > 1 {
            // The thing people actually get wrong with several providers.
            requirements.push(
                "With more than one provider you must decide what happens when the same \
                 person arrives from two of them — link the accounts, or treat them as \
                 separate identities. The framework leaves that to you, deliberately: it \
                 owns no user table."
                    .to_string(),
            );
        }

        // Google is OIDC rather than plain OAuth2, so it needs another crate —
        // exactly the kind of thing a diff should surface before someone
        // discovers it mid-implementation.
        if selected.iter().any(|p| p == "google") {
            crates.push(CrateRequirement::new("authkestra-oidc", &[]));
            requirements.push(
                "Google is OpenID Connect, so discovery and ID-token validation come from \
                 `authkestra-oidc`. A public app may also need consent-screen verification."
                    .to_string(),
            );
        }

        let mut routes = Vec::new();
        for provider in &selected {
            routes.push(format!("GET /auth/login/{provider}"));
            routes.push(format!("GET /auth/callback/{provider}"));
        }
        routes.push("GET /auth/logout".to_string());

        Consequences {
            routes,
            requirements,
            crates,
        }
    }

    fn kit_fragment(&self, value: &ControlValue, _ctx: &KitContext<'_>) -> Option<KitFragment> {
        let selected = selected_providers(value);
        if selected.is_empty() {
            return None;
        }

        let mut imports = vec!["use authkestra_engine::OAuth2Flow;".to_string()];
        let mut builder_calls = Vec::new();
        let mut env = vec![KitEnvVar::with_default(
            "OAUTH_REDIRECT_BASE",
            "Base URL the provider redirects back to. Must match what you registered.",
            "http://localhost:3000",
        )];

        for provider in &selected {
            let type_name = match provider.as_str() {
                "github" => "GithubProvider",
                "google" => "GoogleProvider",
                "discord" => "DiscordProvider",
                // Unknown ids cannot be selected — the control only offers
                // configured providers — so this is unreachable in practice.
                _ => continue,
            };
            imports.push(format!(
                "use authkestra_providers::{provider}::{type_name};"
            ));
            let upper = provider.to_uppercase();
            builder_calls.push(format!(
                r#"        .provider(OAuth2Flow::new({type_name}::new(
            std::env::var("{upper}_CLIENT_ID").expect("{upper}_CLIENT_ID must be set"),
            std::env::var("{upper}_CLIENT_SECRET").expect("{upper}_CLIENT_SECRET must be set"),
            format!("{{redirect_base}}/auth/callback/{provider}"),
        )))"#
            ));
            env.push(KitEnvVar::required(
                &format!("{upper}_CLIENT_ID"),
                &format!("Client id of your {} OAuth app.", Self::label_for(provider)),
            ));
            env.push(KitEnvVar::required(
                &format!("{upper}_CLIENT_SECRET"),
                &format!(
                    "Client secret of your {} OAuth app.",
                    Self::label_for(provider)
                ),
            ));
        }

        let labels: Vec<&str> = selected.iter().map(|p| Self::label_for(p)).collect();
        let mut notes = vec![format!(
            "**Sign in with {}.** The routes come from `engine.axum_router()`, already              merged below: `/auth/login/{{provider}}` starts the flow and              `/auth/callback/{{provider}}` completes it. Register the callback URL with each              provider exactly as it appears — including scheme and path.",
            join_human(&labels)
        )];
        notes.push(
            "OAuth `state` and `nonce` travel in an encrypted cookie rather than a database,              so the callback verifies itself with no server-side lookup."
                .to_string(),
        );
        if selected.len() > 1 {
            notes.push(
                "With more than one provider you must decide what happens when the same                  person arrives from two of them — link the accounts, or treat them as                  separate identities. The framework leaves that to you deliberately: it owns                  no user table."
                    .to_string(),
            );
        }

        Some(KitFragment {
            imports,
            prelude: vec![
                r#"    let redirect_base = std::env::var("OAUTH_REDIRECT_BASE")
        .unwrap_or_else(|_| format!("http://localhost:{port}"));"#
                    .to_string(),
            ],
            builder_calls,
            routes: Vec::new(),
            handlers: Vec::new(),
            env,
            notes,
            needs_credential_store: false,
        })
    }

    async fn try_run(&self, ctx: &ScenarioContext<'_>) -> Result<TryResult, ApiError> {
        let selected = selected_providers(ctx.value);

        if selected.is_empty() {
            return Ok(TryResult {
                outcome: TryOutcome::NotConfigured,
                detail: if self.configured.is_empty() {
                    "No provider credentials are configured on this deployment yet.".to_string()
                } else {
                    "Pick at least one provider.".to_string()
                },
            });
        }

        let unconfigured: Vec<&str> = selected
            .iter()
            .filter(|p| !self.configured.iter().any(|c| c == *p))
            .map(|p| p.as_str())
            .collect();
        if !unconfigured.is_empty() {
            return Ok(TryResult {
                outcome: TryOutcome::NotConfigured,
                detail: format!(
                    "No credentials configured for {} on this deployment.",
                    join_human(&unconfigured)
                ),
            });
        }

        let labels: Vec<&str> = selected.iter().map(|p| Self::label_for(p)).collect();
        Ok(TryResult {
            outcome: TryOutcome::Ok,
            detail: format!(
                "Ready. Your sign-in page would offer {}.",
                join_human(&labels)
            ),
        })
    }
}
