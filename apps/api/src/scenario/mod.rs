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
pub mod oauth;
pub mod passkeys;
pub mod totp;

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;

use crate::credentials::KvCredentialStore;
use crate::error::ApiError;

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
    /// Ceremony steps this scenario accepts at
    /// `POST /api/scenarios/:id/action/:action`.
    pub actions: Vec<String>,
    /// Why this scenario cannot be used on this deployment right now, when it cannot.
    /// The kill switch and a missing credential are different reasons; the UI renders
    /// whichever applies. None means usable.
    pub unavailable_reason: Option<String>,
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

/// Everything a scenario needs to do real work for one visitor.
///
/// The "user" a scenario enrols credentials against is the demo session itself,
/// so the session id is the user id. That is what makes expiry cleanup simple:
/// deleting the session's credentials is a single delete by user id.
pub struct ScenarioContext<'a> {
    pub session_id: Uuid,
    /// The visitor's current value for this scenario.
    pub value: &'a ControlValue,
    pub credentials: &'a KvCredentialStore,
    /// Relying-party settings for WebAuthn ceremonies.
    pub relying_party: &'a crate::settings::RelyingParty,
    /// Short-lived state for multi-round-trip ceremonies.
    pub ceremonies: &'a crate::ceremony::CeremonyStore,
    /// The visitor-facing flow log. Scenarios narrate their steps here.
    pub events: &'a crate::events::EventLog,
}

impl ScenarioContext<'_> {
    /// Narrate a step of this flow for the visitor.
    pub async fn record(&self, step: crate::events::Step) {
        self.events.record(self.session_id, step.build()).await;
    }

    /// The credential owner id. Scoped to the session, never to a person.
    pub fn user_id(&self) -> String {
        self.session_id.to_string()
    }

    /// An owned credential store for this request.
    ///
    /// Owned because the framework's `TotpAuthMethod<S>` /
    /// `WebAuthnAuthMethod<S>` take `S: CredentialStore` by value. Cloning is
    /// an `Arc` bump.
    pub fn credentials(&self) -> KvCredentialStore {
        self.credentials.clone()
    }
}

/// What a scenario contributes to a generated project.
///
/// Assembled by concatenation rather than templated, because composition in
/// this framework is a linear builder chain — see
/// `docs/decisions/0005-starter-kit-model.md`.
///
/// The crates and features are deliberately **not** here: they come from
/// [`Scenario::consequences`], which is what the diff already renders. Two
/// descriptions of the same thing would drift, and the failure mode is the
/// worst available — the playground promising one dependency set while the
/// download ships another.
#[derive(Debug, Clone, Default)]
pub struct KitFragment {
    /// `use` lines.
    pub imports: Vec<String>,
    /// Setup that must run before the builder chain.
    pub prelude: Vec<String>,
    /// Lines appended to the `Engine::builder()` chain, in order.
    pub builder_calls: Vec<String>,
    /// `.route(...)` lines.
    pub routes: Vec<String>,
    /// Whole `fn` definitions appended after `main`.
    pub handlers: Vec<String>,
    /// Environment variables this scenario reads, for `.env.example`.
    pub env: Vec<KitEnvVar>,
    /// Paragraphs for the generated README.
    pub notes: Vec<String>,
    /// What the developer must register or create themselves before this
    /// works — an OAuth app, a callback URL. Rendered as the README's setup
    /// steps, because a project you cannot configure is not a starting point.
    pub setup: Vec<KitSetup>,
    /// Where to read more: the framework's docs, and the upstream example
    /// this fragment derives from.
    pub links: Vec<KitLink>,
    /// Whether this scenario needs the shared credential store.
    ///
    /// Emitted once however many scenarios ask for it, as in the framework's
    /// own MFA example.
    pub needs_credential_store: bool,
}

/// Something the developer must do outside the project before it will run.
#[derive(Debug, Clone)]
pub struct KitSetup {
    pub title: String,
    /// Ordered steps, rendered as a numbered list.
    pub steps: Vec<String>,
}

impl KitSetup {
    pub fn new(title: &str, steps: &[String]) -> Self {
        Self {
            title: title.to_string(),
            steps: steps.to_vec(),
        }
    }
}

/// A pointer into the framework's documentation or examples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitLink {
    pub label: String,
    pub url: String,
}

impl KitLink {
    /// A page on the docs site. `path` is the route, without slashes at
    /// either end — `providers/passkeys`.
    pub fn docs(label: &str, path: &str) -> Self {
        Self {
            label: label.to_string(),
            url: format!("https://authkestra.com/{path}/"),
        }
    }

    /// An example in the framework repository, by path from its root.
    pub fn example(path: &str) -> Self {
        let name = path.rsplit('/').next().unwrap_or(path);
        Self {
            label: format!("upstream example: `{name}`"),
            url: format!("https://github.com/marcjazz/authkestra/blob/main/{path}"),
        }
    }
}

/// An environment variable a generated project reads.
#[derive(Debug, Clone)]
pub struct KitEnvVar {
    pub name: String,
    pub comment: String,
    /// A usable default, or `None` when the value must be supplied.
    pub default: Option<String>,
}

impl KitEnvVar {
    pub fn required(name: &str, comment: &str) -> Self {
        Self {
            name: name.to_string(),
            comment: comment.to_string(),
            default: None,
        }
    }

    pub fn with_default(name: &str, comment: &str, default: &str) -> Self {
        Self {
            name: name.to_string(),
            comment: comment.to_string(),
            default: Some(default.to_string()),
        }
    }
}

/// What else is switched on, so a fragment can adapt to its company.
pub struct KitContext<'a> {
    /// Ids of every active scenario, in registry order.
    pub active: &'a [String],
}

impl KitContext<'_> {
    pub fn is_active(&self, id: &str) -> bool {
        self.active.iter().any(|a| a == id)
    }

    /// True when something other than `id` is also switched on.
    pub fn has_company(&self, id: &str) -> bool {
        self.active.iter().any(|a| a != id)
    }
}

/// One playground scenario.
///
/// Implementors are registered once and driven entirely through this trait, so
/// the HTTP layer never grows a per-scenario branch.
#[async_trait::async_trait]
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

    /// Why this scenario cannot be used on this deployment right now, when it cannot.
    /// `None` means usable. It exists so the UI can explain an empty or inert control
    /// instead of rendering a dead end.
    fn unavailable_reason(&self) -> Option<String> {
        None
    }

    /// Handle one step of a multi-step ceremony.
    ///
    /// Registration and authentication are several round-trips, which the
    /// uniform configure/diff/try contract does not cover. Rather than giving
    /// each scenario its own routes — which would put a per-scenario branch
    /// back in the HTTP layer — every step arrives here through
    /// `POST /api/scenarios/:id/action/:action`.
    ///
    /// The default rejects everything, so a scenario without ceremonies need
    /// not implement it.
    async fn action(
        &self,
        _action: &str,
        _body: Value,
        _ctx: &ScenarioContext<'_>,
    ) -> Result<Value, ApiError> {
        Err(ApiError::UnknownAction {
            scenario: self.id().to_string(),
            action: _action.to_string(),
        })
    }

    /// Actions this scenario understands, advertised to the frontend so the UI
    /// can be built from data rather than hardcoded per scenario.
    fn actions(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// What this scenario contributes to a downloaded project.
    ///
    /// `None` means it contributes nothing — a placeholder, or a scenario that
    /// is switched off. The default suits a scenario with no generated
    /// counterpart.
    fn kit_fragment(&self, _value: &ControlValue, _ctx: &KitContext<'_>) -> Option<KitFragment> {
        None
    }
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
    /// The registry with no OAuth providers configured.
    pub fn with_builtins() -> Self {
        Self::with_providers(Vec::new())
    }

    /// The scenarios a visitor is offered.
    ///
    /// The OAuth control only lists providers this deployment has credentials
    /// for, so it can never offer a dead end.
    pub fn with_providers(configured_providers: Vec<String>) -> Self {
        let mut r = Self::new();
        r.register(Arc::new(passkeys::PasskeysScenario));
        r.register(Arc::new(oauth::OAuthScenario::new(configured_providers)));
        r.register(Arc::new(totp::TotpScenario));
        r
    }

    /// As [`Self::with_providers`], plus the placeholder scenarios.
    ///
    /// The placeholders are deliberately **not** shipped: with passkeys, OAuth
    /// and TOTP all real, "Example toggle" showing up as a selectable sign-in
    /// method is just confusing. They stay useful in tests precisely because
    /// they carry no real behaviour — a test that needs "some toggle" should not
    /// depend on what TOTP happens to do.
    pub fn for_tests(configured_providers: Vec<String>) -> Self {
        let mut r = Self::with_providers(configured_providers);
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
