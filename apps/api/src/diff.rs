//! The config-diff engine (roadmap P1).
//!
//! "See the diff" is the playground's core promise, so the diff is *derived*
//! from two real [`DemoConfig`] values rather than written by hand per
//! scenario. Alongside the raw before/after it carries the human-meaningful
//! consequences of the resulting config — routes, requirements, and the crates
//! and features a real project would need.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::demo_config::DemoConfig;
use crate::scenario::{Consequences, ScenarioRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiffKind {
    Added,
    Removed,
    Changed,
}

/// One line of the raw configuration diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiffEntry {
    pub kind: DiffKind,
    /// Dotted path into the config, e.g. `scenarios.dummy_toggle`.
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// A full diff: what literally changed, and what it would mean.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConfigDiff {
    pub entries: Vec<DiffEntry>,
    pub consequences: Consequences,
}

impl ConfigDiff {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Compute the diff between two configurations.
///
/// `consequences` describes the *after* state — what a real project built from
/// the new configuration would need — which is the question a visitor is
/// actually asking when they flip a switch.
pub fn diff(before: &DemoConfig, after: &DemoConfig, registry: &ScenarioRegistry) -> ConfigDiff {
    let mut entries = Vec::new();

    // BTreeMap iteration is ordered, so entry order is deterministic.
    for (id, after_value) in &after.scenarios {
        match before.get(id) {
            None => entries.push(DiffEntry {
                kind: DiffKind::Added,
                path: format!("scenarios.{id}"),
                before: None,
                after: Some(after_value.render()),
            }),
            Some(before_value) if before_value != after_value => entries.push(DiffEntry {
                kind: DiffKind::Changed,
                path: format!("scenarios.{id}"),
                before: Some(before_value.render()),
                after: Some(after_value.render()),
            }),
            Some(_) => {}
        }
    }

    for (id, before_value) in &before.scenarios {
        if !after.scenarios.contains_key(id) {
            entries.push(DiffEntry {
                kind: DiffKind::Removed,
                path: format!("scenarios.{id}"),
                before: Some(before_value.render()),
                after: None,
            });
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    ConfigDiff {
        entries,
        consequences: consequences_of(after, registry),
    }
}

/// The consequences of a single configuration, merged across active scenarios.
pub fn consequences_of(config: &DemoConfig, registry: &ScenarioRegistry) -> Consequences {
    let mut merged = Consequences::default();
    for scenario in registry.iter() {
        if let Some(value) = config.get(scenario.id()) {
            merged.merge(scenario.consequences(value));
        }
    }
    merged.normalise();
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::ControlValue;

    fn registry() -> ScenarioRegistry {
        ScenarioRegistry::with_builtins()
    }

    #[test]
    fn identical_configs_produce_no_entries() {
        let r = registry();
        let c = DemoConfig::defaults_for(&r);
        let d = diff(&c, &c, &r);
        assert!(d.is_empty(), "expected no entries, got {:?}", d.entries);
    }

    #[test]
    fn toggling_a_scenario_reports_a_change_and_its_consequences() {
        let r = registry();
        let before = DemoConfig::defaults_for(&r);
        let mut after = before.clone();
        after.set("dummy_toggle", ControlValue::Toggle { enabled: true });

        let d = diff(&before, &after, &r);

        assert_eq!(d.entries.len(), 1);
        let e = &d.entries[0];
        assert_eq!(e.kind, DiffKind::Changed);
        assert_eq!(e.path, "scenarios.dummy_toggle");
        assert_eq!(e.before.as_deref(), Some("false"));
        assert_eq!(e.after.as_deref(), Some("true"));

        // The diff must name concrete consequences, not just the flipped bool.
        assert!(d
            .consequences
            .routes
            .contains(&"POST /auth/example".to_string()));
        assert!(d
            .consequences
            .crates
            .iter()
            .any(|c| c.name == "authkestra-engine"));
    }

    #[test]
    fn selecting_a_provider_names_that_providers_routes_and_crate_feature() {
        let r = registry();
        let before = DemoConfig::defaults_for(&r);
        let mut after = before.clone();
        after.set(
            "dummy_provider",
            ControlValue::SelectOne {
                selected: Some("alpha".to_string()),
            },
        );

        let d = diff(&before, &after, &r);

        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].after.as_deref(), Some("alpha"));
        assert!(d
            .consequences
            .routes
            .contains(&"GET /auth/alpha/callback".to_string()));
        let providers = d
            .consequences
            .crates
            .iter()
            .find(|c| c.name == "authkestra-providers")
            .expect("providers crate requirement");
        assert_eq!(providers.features, vec!["alpha".to_string()]);
    }

    #[test]
    fn added_and_removed_keys_are_reported() {
        let r = registry();
        let mut before = DemoConfig::default();
        before.set("dummy_toggle", ControlValue::Toggle { enabled: true });
        let mut after = DemoConfig::default();
        after.set("dummy_provider", ControlValue::SelectOne { selected: None });

        let d = diff(&before, &after, &r);

        let kinds: Vec<_> = d
            .entries
            .iter()
            .map(|e| (e.path.as_str(), e.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("scenarios.dummy_provider", DiffKind::Added),
                ("scenarios.dummy_toggle", DiffKind::Removed),
            ]
        );
    }

    #[test]
    fn select_many_rendering_is_click_order_independent() {
        let a = ControlValue::SelectMany {
            selected: vec!["b".into(), "a".into()],
        };
        let b = ControlValue::SelectMany {
            selected: vec!["a".into(), "b".into()],
        };
        assert_eq!(a.render(), b.render());
    }

    /// Snapshot of the full diff payload. If this output changes, the frontend's
    /// rendering assumptions change with it — so the drift has to be deliberate.
    #[test]
    fn diff_output_snapshot_does_not_drift() {
        let r = registry();
        let before = DemoConfig::defaults_for(&r);
        let mut after = before.clone();
        after.set("dummy_toggle", ControlValue::Toggle { enabled: true });
        after.set(
            "dummy_provider",
            ControlValue::SelectOne {
                selected: Some("beta".to_string()),
            },
        );

        let d = diff(&before, &after, &r);
        let actual = serde_json::to_string_pretty(&d).expect("serialise diff");

        let expected = r#"{
  "entries": [
    {
      "kind": "changed",
      "path": "scenarios.dummy_provider",
      "before": "none",
      "after": "beta"
    },
    {
      "kind": "changed",
      "path": "scenarios.dummy_toggle",
      "before": "false",
      "after": "true"
    }
  ],
  "consequences": {
    "routes": [
      "GET /auth/beta/callback",
      "GET /auth/beta/login",
      "POST /auth/example"
    ],
    "requirements": [
      "A redirect URI would have to be registered with beta.",
      "Visitors would be asked for one extra step at sign-in."
    ],
    "crates": [
      {
        "name": "authkestra-engine",
        "features": [
          "session",
          "token"
        ]
      },
      {
        "name": "authkestra-providers",
        "features": [
          "beta"
        ]
      }
    ]
  }
}"#;

        assert_eq!(actual, expected, "diff snapshot drifted");
    }
}
