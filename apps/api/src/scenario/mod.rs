//! The scenario abstraction (roadmap P1).
//!
//! Not every playground control is an on/off switch: passkeys and TOTP are
//! boolean, OAuth and bot-protection are "pick a provider". [`ControlShape`]
//! models that difference up front so that adding a provider-select scenario
//! later is a new module rather than a retrofit of every call site.
//!
//! Adding a scenario means writing one module that implements [`Scenario`] and
//! registering it in [`ScenarioRegistry::with_builtins`]. No shared code path
//! needs a new `match` arm.

pub mod dummy;

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A selectable option for `SelectOne` / `SelectMany` controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScenarioOption {
    pub id: String,
    pub label: String,
}

impl ScenarioOption {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// The shape of the control the frontend should render for a scenario.
///
/// The frontend renders from this data — it never hardcodes a scenario list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ControlShape {
    Toggle,
    SelectOne { options: Vec<ScenarioOption> },
    SelectMany { options: Vec<ScenarioOption> },
}

/// A visitor's chosen value for one scenario's control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ControlValue {
    Toggle { enabled: bool },
    SelectOne { selected: Option<String> },
    SelectMany { selected: Vec<String> },
}

impl ControlValue {
    /// True when the visitor has actually turned this scenario on.
    pub fn is_active(&self) -> bool {
        match self {
            ControlValue::Toggle { enabled } => *enabled,
            ControlValue::SelectOne { selected } => selected.is_some(),
            ControlValue::SelectMany { selected } => !selected.is_empty(),
        }
    }

    /// Stable, human-readable rendering used in diff output.
    pub fn render(&self) -> String {
        match self {
            ControlValue::Toggle { enabled } => enabled.to_string(),
            ControlValue::SelectOne { selected } => {
                selected.clone().unwrap_or_else(|| "none".to_string())
            }
            ControlValue::SelectMany { selected } => {
                if selected.is_empty() {
                    "none".to_string()
                } else {
                    // Sorted so diff output does not depend on click order.
                    let mut s = selected.clone();
                    s.sort();
                    s.join(", ")
                }
            }
        }
    }

    /// Whether this value matches the control shape it is being applied to.
    pub fn matches_shape(&self, shape: &ControlShape) -> bool {
        matches!(
            (self, shape),
            (ControlValue::Toggle { .. }, ControlShape::Toggle)
                | (
                    ControlValue::SelectOne { .. },
                    ControlShape::SelectOne { .. }
                )
                | (
                    ControlValue::SelectMany { .. },
                    ControlShape::SelectMany { .. }
                )
        )
    }
}

/// The public description of a scenario, serialised for the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScenarioSpec {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub control: ControlShape,
    /// Ids of scenarios that must be active for this one to be meaningful.
    pub depends_on: Vec<String>,
    /// False when the kill switch has disabled this scenario. The UI renders
    /// the control as unavailable rather than hiding it, so the page still
    /// reads as intentional.
    pub available: bool,
}

/// A crate + feature set a real project would need for the current config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CrateRequirement {
    pub name: String,
    pub features: Vec<String>,
}

impl CrateRequirement {
    pub fn new(name: &str, features: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            features: features.iter().map(|f| f.to_string()).collect(),
        }
    }
}

/// The human-meaningful consequences of a configuration.
///
/// This is what makes the diff worth looking at: not just "a bool flipped" but
/// which routes appear, what a user is now required to do, and which crates and
/// features a real project would have to add.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Consequences {
    pub routes: Vec<String>,
    pub requirements: Vec<String>,
    pub crates: Vec<CrateRequirement>,
}

impl Consequences {
    /// Merge another scenario's consequences into this one, de-duplicating.
    ///
    /// Crate requirements union their feature lists rather than appearing
    /// twice, which mirrors how Cargo actually resolves additive features.
    pub fn merge(&mut self, other: Consequences) {
        for r in other.routes {
            if !self.routes.contains(&r) {
                self.routes.push(r);
            }
        }
        for r in other.requirements {
            if !self.requirements.contains(&r) {
                self.requirements.push(r);
            }
        }
        for c in other.crates {
            match self.crates.iter_mut().find(|e| e.name == c.name) {
                Some(existing) => {
                    for f in c.features {
                        if !existing.features.contains(&f) {
                            existing.features.push(f);
                        }
                    }
                    existing.features.sort();
                }
                None => self.crates.push(c),
            }
        }
    }

    /// Sort every collection so snapshot tests are order-independent.
    pub fn normalise(&mut self) {
        self.routes.sort();
        self.requirements.sort();
        self.crates.sort_by(|a, b| a.name.cmp(&b.name));
        for c in &mut self.crates {
            c.features.sort();
        }
    }
}

/// Outcome of a `try` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TryOutcome {
    Ok,
    Disabled,
    NotConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TryResult {
    pub outcome: TryOutcome,
    pub detail: String,
}

/// One playground scenario.
///
/// Implementors are registered once and driven entirely through this trait, so
/// the HTTP layer never grows a per-scenario branch.
pub trait Scenario: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn summary(&self) -> &'static str;
    fn control(&self) -> ControlShape;

    /// Scenarios that must be active for this one to be meaningful.
    fn depends_on(&self) -> Vec<String> {
        Vec::new()
    }

    /// The value a fresh session starts with.
    fn default_value(&self) -> ControlValue;

    /// Reject values that do not fit the control (e.g. an unknown option id).
    fn validate(&self, value: &ControlValue) -> Result<(), String> {
        if !value.matches_shape(&self.control()) {
            return Err(format!(
                "value shape does not match control shape for scenario `{}`",
                self.id()
            ));
        }
        match (value, self.control()) {
            (
                ControlValue::SelectOne { selected: Some(s) },
                ControlShape::SelectOne { options },
            ) if !options.iter().any(|o| &o.id == s) => {
                return Err(format!("unknown option `{s}` for scenario `{}`", self.id()));
            }
            (ControlValue::SelectMany { selected }, ControlShape::SelectMany { options }) => {
                for s in selected {
                    if !options.iter().any(|o| &o.id == s) {
                        return Err(format!("unknown option `{s}` for scenario `{}`", self.id()));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// What this scenario implies for a real project, given its current value.
    /// Returns empty consequences when the scenario is not active.
    fn consequences(&self, value: &ControlValue) -> Consequences;

    /// Exercise the scenario. In v0 this reports configuration state; live
    /// flows land with the individual scenarios in P2.
    fn try_run(&self, value: &ControlValue) -> TryResult;
}

/// Registry of every known scenario.
#[derive(Clone)]
pub struct ScenarioRegistry {
    order: Vec<String>,
    by_id: HashMap<String, Arc<dyn Scenario>>,
}

impl ScenarioRegistry {
    pub fn new() -> Self {
        Self {
            order: Vec::new(),
            by_id: HashMap::new(),
        }
    }

    /// Every scenario shipped in this build.
    ///
    /// P2 adds passkeys / TOTP / OAuth / bot-protection here; nothing else in
    /// the codebase has to change to accommodate them.
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        r.register(Arc::new(dummy::DummyToggleScenario));
        r.register(Arc::new(dummy::DummyProviderScenario));
        r
    }

    pub fn register(&mut self, scenario: Arc<dyn Scenario>) {
        let id = scenario.id().to_string();
        if self.by_id.insert(id.clone(), scenario).is_none() {
            self.order.push(id);
        }
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Scenario>> {
        self.by_id.get(id)
    }

    /// Registration order, which is also display order.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Scenario>> {
        self.order.iter().filter_map(|id| self.by_id.get(id))
    }

    pub fn ids(&self) -> &[String] {
        &self.order
    }
}

impl Default for ScenarioRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}
