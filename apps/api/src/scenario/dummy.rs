//! Placeholder scenarios proving the registry works for both control shapes.
//!
//! P1's acceptance criterion was that a dummy scenario can be registered,
//! configured and diffed without touching shared code paths. These two exist to
//! demonstrate exactly that — one boolean, one provider-select.
//!
//! They are **not** registered for visitors: `ScenarioRegistry::for_tests` adds
//! them, `with_providers` does not. They survive the real scenarios landing
//! because they carry no behaviour of their own, so a test that needs "some
//! toggle" does not end up depending on whatever TOTP happens to do.

use super::{Consequences, ControlShape, ControlValue, CrateRequirement, Scenario, ScenarioOption};

/// A boolean control, shaped like the eventual passkeys / TOTP scenarios.
pub struct DummyToggleScenario;

#[async_trait::async_trait]
impl Scenario for DummyToggleScenario {
    fn id(&self) -> &'static str {
        "dummy_toggle"
    }

    fn name(&self) -> &'static str {
        "Example toggle"
    }

    fn summary(&self) -> &'static str {
        "A stand-in boolean control. Demonstrates the toggle shape until the real \
         passkey and TOTP scenarios land."
    }

    fn control(&self) -> ControlShape {
        ControlShape::Toggle
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::Toggle { enabled: false }
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        if !value.is_active() {
            return Consequences::default();
        }
        Consequences {
            routes: vec!["POST /auth/example".to_string()],
            requirements: vec!["Visitors would be asked for one extra step at sign-in.".to_string()],
            crates: vec![CrateRequirement::new(
                "authkestra-engine",
                &["session", "token"],
            )],
        }
    }
}

/// A provider-select control, shaped like the eventual OAuth / bot-protection
/// scenarios. This is the shape a boolean-only model would have had to be
/// retrofitted for.
pub struct DummyProviderScenario;

impl DummyProviderScenario {
    fn options() -> Vec<ScenarioOption> {
        vec![
            ScenarioOption::new("alpha", "Provider Alpha"),
            ScenarioOption::new("beta", "Provider Beta"),
        ]
    }
}

#[async_trait::async_trait]
impl Scenario for DummyProviderScenario {
    fn id(&self) -> &'static str {
        "dummy_provider"
    }

    fn name(&self) -> &'static str {
        "Example provider"
    }

    fn summary(&self) -> &'static str {
        "A stand-in 'pick a provider' control. Demonstrates the select-one shape \
         until the real OAuth scenario lands."
    }

    fn control(&self) -> ControlShape {
        ControlShape::SelectOne {
            options: Self::options(),
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
        Consequences {
            routes: vec![
                format!("GET /auth/{provider}/login"),
                format!("GET /auth/{provider}/callback"),
            ],
            requirements: vec![format!(
                "A redirect URI would have to be registered with {provider}."
            )],
            crates: vec![CrateRequirement::new("authkestra-providers", &[provider])],
        }
    }
}
