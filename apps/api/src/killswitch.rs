//! Kill switch for live demo flows (roadmap P1).
//!
//! A public demo that creates real credentials and calls third-party providers
//! needs a way to be switched off *now* — not after a redeploy. State lives in
//! atomics read on every request, seeded from the environment at boot and
//! flippable at runtime through the admin endpoint.
//!
//! When flows are off the site degrades to explainer-only mode: controls render
//! as unavailable with an explanation. It should read as intentional, not
//! broken.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

/// Global + per-scenario enablement.
pub struct KillSwitch {
    demo_enabled: AtomicBool,
    disabled_scenarios: RwLock<BTreeSet<String>>,
}

impl KillSwitch {
    pub fn new(demo_enabled: bool, disabled_scenarios: BTreeSet<String>) -> Self {
        Self {
            demo_enabled: AtomicBool::new(demo_enabled),
            disabled_scenarios: RwLock::new(disabled_scenarios),
        }
    }

    /// Read initial state from the environment.
    ///
    /// `DEMO_ENABLED` defaults to true; `DEMO_DISABLED_SCENARIOS` is a
    /// comma-separated list of scenario ids.
    pub fn from_env() -> Self {
        let demo_enabled = std::env::var("DEMO_ENABLED")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no"))
            .unwrap_or(true);
        let disabled = std::env::var("DEMO_DISABLED_SCENARIOS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Self::new(demo_enabled, disabled)
    }

    /// Are live flows enabled at all?
    pub fn demo_enabled(&self) -> bool {
        self.demo_enabled.load(Ordering::Relaxed)
    }

    pub fn set_demo_enabled(&self, enabled: bool) {
        self.demo_enabled.store(enabled, Ordering::Relaxed);
        tracing::warn!(enabled, "demo kill switch flipped");
    }

    /// Is this specific scenario runnable right now?
    pub fn scenario_enabled(&self, id: &str) -> bool {
        self.demo_enabled()
            && !self
                .disabled_scenarios
                .read()
                .expect("kill switch poisoned")
                .contains(id)
    }

    pub fn set_scenario_enabled(&self, id: &str, enabled: bool) {
        let mut guard = self
            .disabled_scenarios
            .write()
            .expect("kill switch poisoned");
        if enabled {
            guard.remove(id);
        } else {
            guard.insert(id.to_string());
        }
        tracing::warn!(scenario = id, enabled, "scenario kill switch flipped");
    }

    pub fn disabled_scenarios(&self) -> Vec<String> {
        self.disabled_scenarios
            .read()
            .expect("kill switch poisoned")
            .iter()
            .cloned()
            .collect()
    }
}

impl Default for KillSwitch {
    fn default() -> Self {
        Self::new(true, BTreeSet::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenarios_follow_the_global_switch() {
        let ks = KillSwitch::default();
        assert!(ks.scenario_enabled("dummy_toggle"));
        ks.set_demo_enabled(false);
        assert!(
            !ks.scenario_enabled("dummy_toggle"),
            "global off must disable every scenario"
        );
    }

    #[test]
    fn a_single_scenario_can_be_disabled_on_its_own() {
        let ks = KillSwitch::default();
        ks.set_scenario_enabled("dummy_toggle", false);
        assert!(!ks.scenario_enabled("dummy_toggle"));
        assert!(
            ks.scenario_enabled("dummy_provider"),
            "disabling one scenario must not affect the others"
        );
        assert!(ks.demo_enabled(), "global switch untouched");
    }

    #[test]
    fn disabling_is_reversible() {
        let ks = KillSwitch::default();
        ks.set_scenario_enabled("dummy_toggle", false);
        ks.set_scenario_enabled("dummy_toggle", true);
        assert!(ks.scenario_enabled("dummy_toggle"));
        assert!(ks.disabled_scenarios().is_empty());
    }
}
