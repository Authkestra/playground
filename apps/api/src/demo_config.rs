//! The per-visitor demo configuration: which scenarios are on, and how.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::scenario::{ControlValue, ScenarioRegistry};

/// A visitor's full playground configuration.
///
/// Backed by a `BTreeMap` so serialisation is deterministic — the diff engine
/// and its snapshot tests depend on stable key ordering.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DemoConfig {
    pub scenarios: BTreeMap<String, ControlValue>,
}

impl DemoConfig {
    /// The configuration a fresh session starts with: every scenario at its
    /// registered default.
    pub fn defaults_for(registry: &ScenarioRegistry) -> Self {
        let scenarios = registry
            .iter()
            .map(|s| (s.id().to_string(), s.default_value()))
            .collect();
        Self { scenarios }
    }

    pub fn get(&self, id: &str) -> Option<&ControlValue> {
        self.scenarios.get(id)
    }

    pub fn set(&mut self, id: &str, value: ControlValue) {
        self.scenarios.insert(id.to_string(), value);
    }

    /// Ids of every scenario the visitor has actually turned on.
    pub fn active_ids(&self) -> Vec<&str> {
        self.scenarios
            .iter()
            .filter(|(_, v)| v.is_active())
            .map(|(k, _)| k.as_str())
            .collect()
    }
}
