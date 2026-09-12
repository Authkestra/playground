//! Which generated projects CI actually builds.
//!
//! The combinatorial space is passkeys x TOTP x resource x captcha x three
//! OAuth providers, and it grows every time a scenario is added. Building all of it on every push
//! would be slow enough that people start skipping it, so there are two sets:
//! a small representative one for pull requests, and the full product for the
//! scheduled run.
//!
//! This is the single source of truth. CI reads the list from the binary
//! rather than repeating it in YAML, so the two cannot drift.

use crate::demo_config::DemoConfig;
use crate::scenario::captcha::{CaptchaKeys, KNOWN_PROVIDERS as CAPTCHA_PROVIDERS};
use crate::scenario::{ControlShape, ControlValue, KitOptions, ScenarioRegistry};

/// Every OAuth provider the generator knows how to emit.
pub const PROVIDERS: &[&str] = &["github", "google", "discord"];

/// The captcha provider the exhaustive run crosses everything else with.
///
/// One rather than all three, deliberately. The three fragments differ by an
/// enum variant, an environment-variable name and some prose — the *composition*
/// with every other fragment is identical, which is what a cross-product is for.
/// All three are compiled in the representative set below, so none goes
/// unbuilt; crossing all three would put the nightly matrix at GitHub's 256-job
/// ceiling to prove the same thing three times.
const EXHAUSTIVE_CAPTCHA: &str = "turnstile";

/// One generated project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combination {
    /// Stable identifier — the CI job name, and the directory it builds in.
    pub name: String,
    /// `scenario[=opt+opt],scenario`, or empty for the base project.
    pub spec: String,
    /// What was asked of the download itself.
    pub options: KitOptions,
}

impl Combination {
    fn new(name: &str, spec: &str) -> Self {
        Self {
            name: name.to_string(),
            spec: spec.to_string(),
            options: KitOptions::default(),
        }
    }

    fn with_options(name: &str, spec: &str, options: KitOptions) -> Self {
        Self {
            name: name.to_string(),
            spec: spec.to_string(),
            options,
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
        Combination::new("resource", "resource"),
    ];
    for p in PROVIDERS {
        out.push(Combination::new(
            &format!("oauth-{p}"),
            &format!("oauth={p}"),
        ));
    }
    // One leg per captcha provider: the enum variant and the secret's name are
    // the only things that vary, and both are things a typo would break
    // silently in prose but loudly in a build.
    for (p, _) in CAPTCHA_PROVIDERS {
        out.push(Combination::new(
            &format!("captcha-{p}"),
            &format!("captcha={p}"),
        ));
    }
    out.push(Combination::new("totp-passkeys", "passkeys,totp"));
    // Captcha alongside TOTP is the composition worth pinning: the fragment
    // adapts its README to name the endpoint worth guarding.
    out.push(Combination::new("totp-captcha", "totp,captcha=turnstile"));
    out.push(Combination::new(
        "all",
        &format!(
            "passkeys,totp,resource,captcha={EXHAUSTIVE_CAPTCHA},oauth={}",
            PROVIDERS.join("+")
        ),
    ));
    // The opt-ins, on. Every other leg covers them off. Only OpenAPI changes
    // what the compiler sees — the TypeScript client is a file the Rust build
    // never looks at — but the two ship together here so a leg exists where
    // both are on at once.
    out.push(Combination::with_options(
        "all-extras",
        "passkeys,totp",
        KitOptions {
            openapi: true,
            ts_client: true,
            ..KitOptions::default()
        },
    ));
    out
}

/// The full product, for the scheduled run: every subset of the toggles
/// against every subset of the OAuth providers, with and without a captcha.
pub fn exhaustive() -> Vec<Combination> {
    let mut out = Vec::new();
    for toggles in 0..8u8 {
        for providers in 0..(1 << PROVIDERS.len()) {
            // Captcha off, then captcha on. See EXHAUSTIVE_CAPTCHA for why one
            // provider stands in for three here.
            for captcha in [false, true] {
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
                if toggles & 4 != 0 {
                    parts.push("resource".to_string());
                    name.push("resource");
                }
                if captcha {
                    parts.push(format!("captcha={EXHAUSTIVE_CAPTCHA}"));
                    name.push("captcha");
                    name.push(EXHAUSTIVE_CAPTCHA);
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
            // Against the scenario's own control, not a global provider list.
            // There are two select-many controls now with disjoint options, and
            // one shared list would have let `captcha=github` through.
            let offered: Vec<String> = match scenario.control() {
                ControlShape::SelectOne { options } | ControlShape::SelectMany { options } => {
                    options.into_iter().map(|o| o.id).collect()
                }
                ControlShape::Toggle => Vec::new(),
            };
            for option in &options {
                if !offered.contains(option) {
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
///
/// The captcha keys are placeholders. Generation only ever reads which
/// providers are *offered*; the keys themselves never reach a generated
/// project, which reads its secret from the environment at runtime.
pub fn ci_registry() -> ScenarioRegistry {
    let mut captcha = CaptchaKeys::default();
    for (id, _) in CAPTCHA_PROVIDERS {
        captcha.insert_for_test(id, "ci-site-key", "ci-secret-key");
    }
    ScenarioRegistry::with_credentials(PROVIDERS.iter().map(|p| p.to_string()).collect(), captcha)
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
        // Every captcha provider is compiled somewhere. The exhaustive run
        // crosses only one of them, so if these legs go the other two stop
        // being built at all.
        for (p, _) in CAPTCHA_PROVIDERS {
            assert!(
                names.iter().any(|n| n == &format!("captcha-{p}")),
                "no combination builds the {p} captcha fragment"
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
        // three toggles x every subset of three providers x captcha off/on
        assert_eq!(exhaustive().len(), 8 * 8 * 2);
        // and it contains every *selection* the pull-request set builds; the
        // opt-in legs differ by options rather than by scenarios.
        //
        // The captcha providers the exhaustive run does not cross are the one
        // exception, and a deliberate one — see EXHAUSTIVE_CAPTCHA. They are
        // built on every pull request instead, which the test above pins.
        let all: Vec<String> = exhaustive().into_iter().map(|c| c.spec).collect();
        for c in representative() {
            if c.spec.contains("captcha=") && !c.spec.contains(EXHAUSTIVE_CAPTCHA) {
                continue;
            }
            assert!(all.contains(&c.spec), "exhaustive is missing `{}`", c.spec);
        }
    }

    /// A matrix that only ever builds the default would let the opt-ins rot.
    #[test]
    fn the_matrix_builds_the_opt_ins_as_well_as_without_them() {
        let set = representative();
        assert!(
            set.iter().any(|c| c.options.openapi && c.options.ts_client),
            "no combination exercises the download opt-ins"
        );
        assert!(
            set.iter().any(|c| c.options == KitOptions::default()),
            "no combination exercises a plain download"
        );
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
        assert!(config_from_spec("captcha=turnstyle", &registry).is_err());
        // The two select-many controls have disjoint options, and neither may
        // borrow the other's.
        assert!(config_from_spec("captcha=github", &registry).is_err());
        assert!(config_from_spec("oauth=turnstile", &registry).is_err());
    }
}
