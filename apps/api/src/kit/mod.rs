//! The starter-kit generator: a `DemoConfig` becomes a Cargo project.
//!
//! Design in `docs/decisions/0005-starter-kit-model.md`. The short version:
//! composition in this framework is a linear builder chain, so a project is
//! assembled by concatenating fragments rather than rendered from a templated
//! tree.
//!
//! This module is the **base** — the skeleton every generated project starts
//! from, which must compile and run on its own with nothing selected. The
//! per-scenario fragments that fill the seams are the next piece of work.

use crate::demo_config::DemoConfig;
use crate::scenario::{CrateRequirement, ScenarioRegistry};

/// The framework version every generated project pins.
///
/// Read from one place and asserted against this crate's own dependency, so a
/// download can never advertise a version the playground does not build
/// against. Upstream's README says 0.7 while the crates are 0.8.0, which is
/// exactly why this is not taken from prose.
pub const AUTHKESTRA_VERSION: &str = "0.8.0";

/// The generated project's crate name and directory.
pub const PROJECT_NAME: &str = "authkestra-starter";

/// One file in the generated project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    /// Path relative to the project root, always `/`-separated.
    pub path: String,
    pub contents: String,
}

/// A generated project.
#[derive(Debug, Clone)]
pub struct StarterKit {
    pub files: Vec<GeneratedFile>,
}

impl StarterKit {
    /// Build a project from a visitor's configuration.
    pub fn generate(config: &DemoConfig, registry: &ScenarioRegistry) -> Self {
        let plan = Plan::from(config, registry);
        Self {
            files: vec![
                GeneratedFile {
                    path: "Cargo.toml".to_string(),
                    contents: cargo_toml(&plan),
                },
                GeneratedFile {
                    path: "src/main.rs".to_string(),
                    contents: main_rs(&plan),
                },
                GeneratedFile {
                    path: "README.md".to_string(),
                    contents: readme(&plan),
                },
                GeneratedFile {
                    path: ".env.example".to_string(),
                    contents: env_example(&plan),
                },
                GeneratedFile {
                    path: ".gitignore".to_string(),
                    contents: GITIGNORE.to_string(),
                },
                GeneratedFile {
                    path: "justfile".to_string(),
                    contents: JUSTFILE.to_string(),
                },
            ],
        }
    }

    pub fn file(&self, path: &str) -> Option<&GeneratedFile> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// What the selected configuration implies, resolved once.
///
/// Derived from the same `Scenario::consequences` the diff renders, so the
/// download and the promise cannot drift — see the ADR.
struct Plan {
    /// Scenario ids the visitor turned on, in registry order.
    active: Vec<String>,
    /// Crates and features, unioned across active scenarios.
    crates: Vec<CrateRequirement>,
}

impl Plan {
    fn from(config: &DemoConfig, registry: &ScenarioRegistry) -> Self {
        let mut active = Vec::new();
        let mut consequences = crate::scenario::Consequences::default();

        for scenario in registry.iter() {
            if let Some(value) = config.get(scenario.id()) {
                if value.is_active() {
                    active.push(scenario.id().to_string());
                    consequences.merge(scenario.consequences(value));
                }
            }
        }
        consequences.normalise();

        Self {
            active,
            crates: consequences.crates,
        }
    }

    /// Features requested for one crate by the active scenarios.
    fn features_for(&self, crate_name: &str) -> Vec<String> {
        self.crates
            .iter()
            .find(|c| c.name == crate_name)
            .map(|c| c.features.clone())
            .unwrap_or_default()
    }

    fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}

/// Merge the base features with whatever the scenarios asked for.
fn feature_list(base: &[&str], extra: Vec<String>) -> String {
    let mut all: Vec<String> = base.iter().map(|s| s.to_string()).collect();
    for f in extra {
        if !all.contains(&f) {
            all.push(f);
        }
    }
    all.sort();
    all.iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cargo_toml(plan: &Plan) -> String {
    let engine_features = feature_list(
        &["session", "token", "memory", "rustls-aws-lc-rs"],
        plan.features_for("authkestra-engine"),
    );
    let axum_features = feature_list(
        &["macros", "session", "token", "rustls-aws-lc-rs"],
        plan.features_for("authkestra-axum"),
    );

    let v = AUTHKESTRA_VERSION;
    format!(
        r#"[package]
name = "{PROJECT_NAME}"
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"

[dependencies]
# Direct sub-crate dependencies, NOT the `authkestra` facade.
#
# The facade does not expose `webauthn`, `totp`, `captcha` or the store-backend
# features — it pulls them in only as dev-dependencies for its own examples. An
# application that wants them must depend on the sub-crates directly.
#
# `default-features = false` turns off the default TLS backend so the choice
# below is the one that actually lands in the graph.
authkestra-engine = {{ version = "{v}", default-features = false, features = [{engine_features}] }}
authkestra-axum = {{ version = "{v}", default-features = false, features = [{axum_features}] }}

axum = {{ version = "0.8", features = ["macros"] }}
tokio = {{ version = "1", features = ["full"] }}
tower-cookies = "0.11"
tracing = "0.1"
tracing-subscriber = {{ version = "0.3", features = ["env-filter"] }}
serde_json = "1"
"#
    )
}

fn main_rs(plan: &Plan) -> String {
    let note = if plan.is_empty() {
        "//! Nothing was selected in the playground, so this is the smallest\n\
         //! useful engine: sessions and the framework's `/auth` routes, with no\n\
         //! authentication method registered yet."
    } else {
        "//! Generated from a playground configuration."
    };

    format!(
        r#"{note}
//!
//! Run it with `cargo run`, then:
//!
//! ```sh
//! curl localhost:3000/health
//! curl -i localhost:3000/api/me     # 401 until a session exists
//! ```

use authkestra_axum::{{AuthSession, AxumError, AxumExt, AxumState}};
use authkestra_engine::store::memory::MemoryStore;
use authkestra_engine::{{AkWebAppEngine, Engine, SessionConfig, SessionStore}};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{{Json, Router}};
use serde_json::json;
use std::sync::Arc;
use tower_cookies::CookieManagerLayer;

/// `AkWebAppEngine` is the alias for a session-configured engine. Using the
/// alias rather than spelling out the typestate generics keeps the compile
/// error legible when a required builder call is missing.
#[derive(Clone, AxumState)]
struct AppState {{
    #[authkestra(engine)]
    auth: AkWebAppEngine,
}}

#[tokio::main]
async fn main() {{
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,authkestra=debug".into()),
        )
        .init();

    // Sessions in memory: no infrastructure to run, and nothing survives a
    // restart. `SessionStore` is a trait, so swapping this for Redis is a
    // one-line change.
    let session_store: Arc<dyn SessionStore> = Arc::new(MemoryStore::default());

    // The typestate builder only exposes session APIs once `session_store` has
    // been supplied, so a missing call is a compile error rather than a
    // runtime surprise.
    let engine = Engine::builder()
        .session_store(session_store)
        .session_config(SessionConfig {{
            // Plain HTTP locally. Set this to `true` behind TLS, or the browser
            // will refuse to store the cookie.
            secure: false,
            ..Default::default()
        }})
        .build();

    let state = AppState {{
        auth: engine.clone(),
    }};

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/me", get(me))
        // The engine's own `/auth/*` routes.
        .merge(engine.axum_router())
        // The engine reads and writes cookies, so the cookie layer must wrap
        // the merged routes.
        .layer(CookieManagerLayer::new())
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("failed to bind");
    tracing::info!(%port, "listening on http://localhost:{{port}}");
    axum::serve(listener, app).await.expect("server error");
}}

async fn health() -> impl IntoResponse {{
    Json(json!({{ "status": "ok" }}))
}}

/// The session identity, or 401 when there is no valid session.
///
/// A real 401 rather than a 200 carrying an `error` field, so a caller can tell
/// the two states apart without parsing the body.
async fn me(session: Result<AuthSession, AxumError>) -> impl IntoResponse {{
    match session {{
        Ok(AuthSession(session)) => (
            StatusCode::OK,
            Json(json!({{
                "id": session.identity.external_id,
                "username": session.identity.username,
                "email": session.identity.email,
                "provider": session.identity.provider_id,
            }})),
        ),
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({{ "error": "not authenticated" }})),
        ),
    }}
}}
"#
    )
}

fn readme(plan: &Plan) -> String {
    let selected = if plan.is_empty() {
        "Nothing was selected, so this is the smallest useful engine: sessions \
         and the framework's `/auth` routes, with no authentication method \
         registered yet."
            .to_string()
    } else {
        format!(
            "Selected in the playground: **{}**.",
            plan.active.join("**, **")
        )
    };

    let deps = plan
        .crates
        .iter()
        .map(|c| {
            if c.features.is_empty() {
                format!("- `{}`", c.name)
            } else {
                format!("- `{}` with `{}`", c.name, c.features.join("`, `"))
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let deps_section = if deps.is_empty() {
        String::new()
    } else {
        format!("\n## What your selection added\n\n{deps}\n")
    };

    format!(
        r#"# {PROJECT_NAME}

Generated by the [authkestra playground](https://play.authkestra.com).

{selected}

## Run it

```sh
cargo run
curl localhost:3000/health
```

`PORT` overrides the bind port.
{deps_section}
## About the dependencies

This project depends on `authkestra-engine` and `authkestra-axum` **directly**,
not on the `authkestra` facade. The facade does not expose the `webauthn`,
`totp`, `captcha` or store-backend features — it pulls them in only as
dev-dependencies for its own examples — so anything beyond sessions and OAuth
needs the sub-crates.

Versions are pinned to `{AUTHKESTRA_VERSION}`.

### TLS backend

`rustls-aws-lc-rs` is enabled, which compiles C and assembly and therefore needs
a C toolchain. If you build for musl, or your policy bans `aws-lc-rs`, switch to
`rustls-no-provider` and install a provider yourself **before any HTTPS client
is constructed**, or `reqwest` panics at construction:

```rust,ignore
rustls::crypto::ring::default_provider()
    .install_default()
    .expect("failed to install rustls crypto provider");
```

Cargo features are additive, so check what actually landed:

```sh
cargo tree -i aws-lc-rs -e features
```

## What this does not include

The framework deliberately owns no user or account table — there is no
`UserStore` trait. Your application owns that data, so this project stores no
users and invents no schema. Wire the identity you get from a session into
whatever persistence you already have.

## Licence

MIT OR Apache-2.0, matching the framework.
"#
    )
}

fn env_example(plan: &Plan) -> String {
    let mut out = String::from(
        "# Copy to .env and fill in. Only the variables this configuration\n\
         # actually reads are listed.\n\n\
         # Port the server binds to.\n\
         PORT=3000\n\n\
         # Log filter, e.g. `info`, or `debug,authkestra=trace`.\n\
         RUST_LOG=info,authkestra=debug\n",
    );
    if !plan.is_empty() {
        out.push_str(
            "\n# Your selection may need more; see README.md for what each scenario\n\
             # requires.\n",
        );
    }
    out
}

const GITIGNORE: &str = "/target\n**/*.rs.bk\n.env\n*.db\n*.db-journal\n";

const JUSTFILE: &str = r#"# Common commands. Install with `cargo install just`, or run them by hand.

# Run the server.
run:
    cargo run

# Compile without producing a binary — the fast feedback loop.
check:
    cargo check

test:
    cargo test

fmt:
    cargo fmt --all

# What the CI bar would be.
lint:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings

# Which TLS backend actually landed in the dependency graph.
tls:
    cargo tree -i aws-lc-rs -e features
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::ScenarioRegistry;

    fn base() -> StarterKit {
        let registry = ScenarioRegistry::with_providers(Vec::new());
        let config = DemoConfig::defaults_for(&registry);
        StarterKit::generate(&config, &registry)
    }

    fn contents(kit: &StarterKit, path: &str) -> String {
        kit.file(path)
            .unwrap_or_else(|| panic!("{path} should be generated"))
            .contents
            .clone()
    }

    #[test]
    fn the_base_project_has_everything_needed_to_run() {
        let kit = base();
        let paths: Vec<&str> = kit.files.iter().map(|f| f.path.as_str()).collect();
        for expected in [
            "Cargo.toml",
            "src/main.rs",
            "README.md",
            ".env.example",
            ".gitignore",
            "justfile",
        ] {
            assert!(paths.contains(&expected), "missing {expected}: {paths:?}");
        }
    }

    /// The pin must match what the playground itself builds against, or a
    /// download advertises a version we never tested.
    ///
    /// Read from the *workspace* manifest, which is where the version is
    /// declared — `apps/api` inherits it with `workspace = true`.
    #[test]
    fn the_pinned_version_matches_the_playgrounds_own_dependency() {
        let workspace = include_str!("../../../../Cargo.toml");
        let line = workspace
            .lines()
            .find(|l| l.trim_start().starts_with("authkestra-engine"))
            .expect("the workspace declares authkestra-engine");

        assert!(
            line.contains(AUTHKESTRA_VERSION),
            "the kit pins {AUTHKESTRA_VERSION} but the workspace declares: {line}"
        );

        // And the member really does inherit it, or the check above is
        // measuring something the build does not use.
        let member = include_str!("../../Cargo.toml");
        let member_line = member
            .lines()
            .find(|l| l.trim_start().starts_with("authkestra-engine"))
            .expect("apps/api depends on authkestra-engine");
        assert!(
            member_line.contains("workspace = true"),
            "expected apps/api to inherit the workspace version: {member_line}"
        );
    }

    /// The facade does not expose webauthn/totp/captcha or the store backends,
    /// so pointing a generated project at it is a dead end.
    #[test]
    fn the_generated_manifest_never_depends_on_the_facade() {
        let cargo = contents(&base(), "Cargo.toml");
        for line in cargo.lines() {
            let trimmed = line.trim_start();
            assert!(
                !trimmed.starts_with("authkestra ="),
                "generated projects must depend on the sub-crates: {line}"
            );
        }
        assert!(cargo.contains("authkestra-engine = "));
        assert!(cargo.contains("authkestra-axum = "));
    }

    #[test]
    fn the_tls_backend_is_pinned_and_its_caveat_documented() {
        let cargo = contents(&base(), "Cargo.toml");
        assert!(cargo.contains("rustls-aws-lc-rs"));
        assert!(
            cargo.contains("default-features = false"),
            "without this the TLS choice is not the one that lands"
        );

        let readme = contents(&base(), "README.md");
        assert!(readme.contains("rustls-no-provider"), "musl caveat missing");
        assert!(
            readme.contains("install_default"),
            "the caveat must say what to actually do"
        );
    }

    /// The framework owns no user table, and the generated project should keep
    /// that boundary rather than inventing a schema.
    #[test]
    fn the_readme_is_explicit_about_owning_no_user_table() {
        let readme = contents(&base(), "README.md");
        assert!(
            readme.contains("no user or account table"),
            "the reader should not have to discover this"
        );
    }

    #[test]
    fn the_base_project_carries_no_dependency_it_does_not_use() {
        let cargo = contents(&base(), "Cargo.toml");
        // Nothing is selected, so no credential store is needed. Shipping sqlx
        // in a session-only project would be dead weight.
        assert!(
            !cargo.contains("sqlx"),
            "unused dependency in the base: sqlx"
        );
        assert!(!cargo.contains("webauthn-rs"), "unused dependency");
        assert!(!cargo.contains("authkestra-providers"), "unused dependency");
    }

    #[test]
    fn features_are_sorted_and_deduplicated() {
        // Deterministic output keeps the download diffable and the zip cacheable.
        let merged = feature_list(&["session", "token"], vec!["session".into(), "totp".into()]);
        assert_eq!(merged, "\"session\", \"token\", \"totp\"");
    }

    #[test]
    fn an_empty_configuration_still_produces_a_runnable_project() {
        let main = contents(&base(), "src/main.rs");
        assert!(main.contains("fn main()"));
        assert!(
            main.contains(".session_store("),
            "the typestate builder needs a session store to reach `build()`"
        );
        assert!(main.contains("axum::serve"));

        let readme = contents(&base(), "README.md");
        assert!(
            readme.contains("smallest useful engine"),
            "an empty selection should explain itself rather than look broken"
        );
    }

    #[test]
    fn the_env_example_only_lists_what_this_configuration_reads() {
        let env = contents(&base(), ".env.example");
        assert!(env.contains("PORT="));
        assert!(env.contains("RUST_LOG="));
        // Nothing is selected, so no provider or WebAuthn variables belong here.
        assert!(!env.contains("CLIENT_ID"), "not read by this configuration");
        assert!(!env.contains("WEBAUTHN"), "not read by this configuration");
    }
}
