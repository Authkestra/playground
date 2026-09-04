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
    Consequences, ControlShape, ControlValue, CrateRequirement, Scenario, ScenarioContext,
    ScenarioOption, TryOutcome, TryResult,
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
        ControlShape::SelectOne {
            options: self.options(),
        }
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::SelectOne { selected: None }
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        let ControlValue::SelectOne {
            selected: Some(provider),
        } = value
        else {
            return Consequences::default();
        };

        // The exact feature flag differs per provider, which is the useful
        // detail: it is one crate with three features, not three integrations.
        let mut crates = vec![
            CrateRequirement::new("authkestra-providers", &[provider.as_str()]),
            CrateRequirement::new("authkestra-engine", &["session", "token"]),
        ];
        let mut requirements = vec![
            format!(
                "You register an OAuth app with {} and store its client id and secret.",
                Self::label_for(provider)
            ),
            "The redirect URI must match exactly, including scheme and path.".to_string(),
            "No password is ever stored, so there is no password reset to build.".to_string(),
        ];

        // Google is OIDC rather than plain OAuth2, so it needs another crate —
        // exactly the kind of thing a diff should surface before someone
        // discovers it mid-implementation.
        if provider == "google" {
            crates.push(CrateRequirement::new("authkestra-oidc", &[]));
            requirements.push(
                "Google is OpenID Connect, so discovery and ID-token validation come from \
                 `authkestra-oidc`. A public app may also need consent-screen verification."
                    .to_string(),
            );
        }

        Consequences {
            routes: vec![
                format!("GET /auth/login/{provider}"),
                format!("GET /auth/callback/{provider}"),
                "GET /auth/logout".to_string(),
            ],
            requirements,
            crates,
        }
    }

    async fn try_run(&self, ctx: &ScenarioContext<'_>) -> Result<TryResult, ApiError> {
        let ControlValue::SelectOne {
            selected: Some(provider),
        } = ctx.value
        else {
            return Ok(TryResult {
                outcome: TryOutcome::NotConfigured,
                detail: if self.configured.is_empty() {
                    "No provider credentials are configured on this deployment yet.".to_string()
                } else {
                    "Pick a provider first.".to_string()
                },
            });
        };

        if !self.configured.iter().any(|c| c == provider) {
            return Ok(TryResult {
                outcome: TryOutcome::NotConfigured,
                detail: format!("`{provider}` has no credentials configured on this deployment."),
            });
        }

        Ok(TryResult {
            outcome: TryOutcome::Ok,
            detail: format!(
                "Ready. Starting the flow will send you to {} and back.",
                Self::label_for(provider)
            ),
        })
    }
}
