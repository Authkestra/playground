//! Which generated projects CI actually builds.
//!
//! The combinatorial space is passkeys x TOTP x three OAuth providers, and it
//! grows every time a scenario is added. Building all of it on every push
//! would be slow enough that people start skipping it, so there are two sets:
//! a small representative one for pull requests, and the full product for the
//! scheduled run.
//!
//! This is the single source of truth. CI reads the list from the binary
//! rather than repeating it in YAML, so the two cannot drift.

use crate::demo_config::DemoConfig;
use crate::scenario::{ControlValue, ScenarioRegistry};

/// Every provider the generator knows how to emit.
pub const PROVIDERS: &[&str] = &["github", "google", "discord"];

/// One generated project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combination {
    /// Stable identifier — the CI job name, and the directory it builds in.
    pub name: String,
    /// `scenario[=opt+opt],scenario`, or empty for the base project.
    pub spec: String,
}

impl Combination {
    fn new(name: &str, spec: &str) -> Self {
        Self {
            name: name.to_string(),
            spec: spec.to_string(),
        }
    }
}

/// What every pull request builds.
///
/// The shape the issue asked for: nothing on, each method alone, one per
/// provider, and everything at once. Between them these cover each fragment in
/// isolation and all of them composed, which is where the composition bugs
/// live — TOTP alone is `with_totp`, TOTP with company is `with_mfa_method`,
/// and only the "all" case exercises the second form.
pub fn representative() -> Vec<Combination> {
    let mut out = vec![
        Combination::new("base", ""),
        Combination::new("passkeys", "passkeys"),
        Combination::new("totp", "totp"),
    ];
    for p in PROVIDERS {
        out.push(Combination::new(
            &format!("oauth-{p}"),
            &format!("oauth={p}"),
        ));
    }
    out.push(Combination::new("totp-passkeys", "passkeys,totp"));
    out.push(Combination::new(
        "all",
        &format!("passkeys,totp,oauth={}", PROVIDERS.join("+")),
    ));
    out
}

/// The full product, for the scheduled run: every subset of the two toggles
/// against every subset of the providers.
pub fn exhaustive() -> Vec<Combination> {
    let mut out = Vec::new();
    for toggles in 0..4u8 {
        for providers in 0..(1 << PROVIDERS.len()) {
            let mut parts = Vec::new();
            let mut name = Vec::new();
            if toggles & 1 != 0 {
                parts.push("passkeys".to_string());
                name.push("passkeys");
            }
            if toggles & 2 != 0 {
                parts.push("totp".to_string());
                name.push("totp");
            }
            let chosen: Vec<&str> = PROVIDERS
                .iter()
                .enumerate()
                .filter(|(i, _)| providers & (1 << i) != 0)
                .map(|(_, p)| *p)
                .collect();
            if !chosen.is_empty() {
                parts.push(format!("oauth={}", chosen.join("+")));
                name.push("oauth");
                name.extend(chosen.iter().copied());
            }
            out.push(Combination::new(
                &if name.is_empty() {
                    "base".to_string()
                } else {
                    name.join("-")
                },
                &parts.join(","),
            ));
        }
    }
    out
}

/// Turn a spec into a configuration.
///
/// Returns the unrecognised scenario or option rather than ignoring it: a
/// silently dropped name would mean CI cheerfully building the wrong project
/// and reporting success.
pub fn config_from_spec(spec: &str, registry: &ScenarioRegistry) -> Result<DemoConfig, String> {
    let mut config = DemoConfig::defaults_for(registry);

    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (id, options) = match part.split_once('=') {
            Some((id, opts)) => (
                id.trim(),
                opts.split('+')
                    .map(str::trim)
                    .filter(|o| !o.is_empty())
                    .map(str::to_string)
                    .collect(),
            ),
            None => (part, Vec::new()),
        };

        let scenario = registry
            .get(id)
            .ok_or_else(|| format!("no scenario named `{id}`"))?;

        let value = if options.is_empty() {
            ControlValue::Toggle { enabled: true }
        } else {
            for option in &options {
                if !PROVIDERS.contains(&option.as_str()) {
                    return Err(format!("`{id}` has no option `{option}`"));
                }
            }
            ControlValue::SelectMany { selected: options }
        };

        // The control decides the shape; a toggle spec against a select-many
        // control would otherwise be accepted and quietly do nothing.
        let value = match (scenario.control(), value) {
            (crate::scenario::ControlShape::SelectMany { .. }, ControlValue::Toggle { .. }) => {
                return Err(format!(
                    "`{id}` needs options, e.g. `{id}=github` — it is not a toggle"
                ));
            }
            (crate::scenario::ControlShape::Toggle, ControlValue::SelectMany { .. }) => {
                return Err(format!("`{id}` is a toggle and takes no options"));
            }
            (_, v) => v,
        };

        config.set(id, value);
    }

    Ok(config)
}

/// The registry the generator runs against in CI: every provider available, so
/// a combination naming one is never silently dropped for want of credentials.
pub fn ci_registry() -> ScenarioRegistry {
    ScenarioRegistry::with_providers(PROVIDERS.iter().map(|p| p.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_representative_set_covers_what_the_matrix_promises() {
        let names: Vec<String> = representative().into_iter().map(|c| c.name).collect();

        for expected in ["base", "passkeys", "totp", "all"] {
            assert!(names.iter().any(|n| n == expected), "missing {expected}");
        }
        for p in PROVIDERS {
            assert!(
                names.iter().any(|n| n == &format!("oauth-{p}")),
                "no combination builds {p} on its own"
            );
        }
    }

    #[test]
    fn every_name_is_unique_and_safe_as_a_directory() {
        for set in [representative(), exhaustive()] {
            let mut seen: Vec<&str> = Vec::new();
            for c in &set {
                assert!(
                    c.name
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
                    "{} is not a safe directory name",
                    c.name
                );
                assert!(!seen.contains(&c.name.as_str()), "duplicate: {}", c.name);
                seen.push(&c.name);
            }
        }
    }

    #[test]
    fn the_exhaustive_set_is_the_whole_product() {
        // two toggles x every subset of three providers
        assert_eq!(exhaustive().len(), 4 * 8);
        // and it contains everything the pull-request set builds
        let all: Vec<String> = exhaustive().into_iter().map(|c| c.spec).collect();
        for c in representative() {
            assert!(all.contains(&c.spec), "exhaustive is missing `{}`", c.spec);
        }
    }

    #[test]
    fn every_representative_spec_parses_and_activates_what_it_names() {
        let registry = ci_registry();
        for c in representative() {
            let config =
                config_from_spec(&c.spec, &registry).unwrap_or_else(|e| panic!("{}: {e}", c.name));

            let active = registry
                .iter()
                .filter(|s| config.get(s.id()).is_some_and(|v| v.is_active()))
                .count();
            let expected = c.spec.split(',').filter(|p| !p.trim().is_empty()).count();
            assert_eq!(active, expected, "{} activated the wrong count", c.name);
        }
    }

    #[test]
    fn a_typo_is_an_error_rather_than_a_silently_smaller_project() {
        let registry = ci_registry();
        assert!(config_from_spec("passkyes", &registry).is_err());
        assert!(config_from_spec("oauth=gihtub", &registry).is_err());
        assert!(config_from_spec("oauth", &registry).is_err());
        assert!(config_from_spec("passkeys=github", &registry).is_err());
    }
}
