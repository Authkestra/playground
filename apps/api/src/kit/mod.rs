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
mod archive;
mod client;
pub mod matrix;

pub use archive::ArchiveError;

use crate::scenario::{
    ControlValue, CrateRequirement, KitContext, KitEnvVar, KitFragment, KitLink, KitOptions,
    KitSetup, ScenarioRegistry,
};

/// The framework version every generated project pins.
///
/// Read from one place and asserted against this crate's own dependency, so a
/// download can never advertise a version the playground does not build
/// against. Upstream's README says 0.7 while the crates are 0.8.0, which is
/// exactly why this is not taken from prose.
pub const AUTHKESTRA_VERSION: &str = "0.9.2";

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
    /// A stable, readable summary of the selection, for the archive name.
    /// Two different selections never produce the same one.
    pub slug: String,
}

impl StarterKit {
    /// Build a project from a visitor's configuration, with nothing extra.
    pub fn generate(config: &DemoConfig, registry: &ScenarioRegistry) -> Self {
        Self::generate_with(config, registry, KitOptions::default())
    }

    /// Build a project, including whatever was asked of the download itself.
    pub fn generate_with(
        config: &DemoConfig,
        registry: &ScenarioRegistry,
        options: KitOptions,
    ) -> Self {
        let plan = Plan::from(config, registry, options);
        Self {
            slug: plan.slug(),
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
            ]
            .into_iter()
            .chain(client_file(&plan))
            .collect(),
        }
    }

    pub fn file(&self, path: &str) -> Option<&GeneratedFile> {
        self.files.iter().find(|f| f.path == path)
    }

    /// The name the archive is offered under, extension included.
    pub fn archive_name(&self) -> String {
        format!("{PROJECT_NAME}-{}.zip", self.slug)
    }

    /// The directory every file sits under inside the archive, so unzipping in
    /// a downloads folder does not scatter six files across it.
    pub fn archive_root(&self) -> String {
        format!("{PROJECT_NAME}-{}", self.slug)
    }
}

/// What the selected configuration implies, resolved once.
///
/// Derived from the same `Scenario::consequences` the diff renders, so the
/// download and the promise cannot drift — see the ADR.
struct Plan {
    /// What was asked of the download itself, as opposed to of the auth.
    options: KitOptions,
    /// Scenario ids the visitor turned on, in registry order.
    active: Vec<String>,
    /// The same scenarios by their display name, for prose. The README is read
    /// by a person, so it says "Passkeys", not `passkeys`.
    labels: Vec<String>,
    /// Each active scenario's id with whatever options it had chosen, in
    /// registry order. Only the archive name needs this much detail.
    selection: Vec<(String, Vec<String>)>,
    /// Crates and features, unioned across active scenarios.
    crates: Vec<CrateRequirement>,
    /// What each active scenario contributes, in registry order — which is the
    /// emission order, deliberately, so the same selection always produces
    /// byte-identical output.
    fragments: Vec<KitFragment>,
}

impl Plan {
    fn from(config: &DemoConfig, registry: &ScenarioRegistry, options: KitOptions) -> Self {
        let mut active = Vec::new();
        let mut labels = Vec::new();
        let mut selection: Vec<(String, Vec<String>)> = Vec::new();
        let mut consequences = crate::scenario::Consequences::default();

        for scenario in registry.iter() {
            if let Some(value) = config.get(scenario.id()) {
                if value.is_active() {
                    active.push(scenario.id().to_string());
                    labels.push(scenario.name().to_string());
                    selection.push((scenario.id().to_string(), chosen_options(value)));
                    consequences.merge(scenario.consequences(value));
                }
            }
        }
        consequences.normalise();

        // Fragments are gathered in a second pass so each one can see the full
        // set of active scenarios — TOTP's role depends on whether it has
        // company.
        let ctx = KitContext {
            active: &active,
            options,
        };
        let mut fragments = Vec::new();
        for scenario in registry.iter() {
            if let Some(value) = config.get(scenario.id()) {
                if let Some(fragment) = scenario.kit_fragment(value, &ctx) {
                    fragments.push(fragment);
                }
            }
        }

        // Fragment crates are merged in after the framework's own, so a
        // generated handler's dependency is emitted without appearing in the
        // diff as something authkestra asked for.
        let mut crates = consequences.crates;
        if options.openapi && fragments.iter().any(|f| !f.openapi_paths.is_empty()) {
            crates.push(CrateRequirement::new("utoipa", &["axum_extras"]));
        }
        for fragment in &fragments {
            for req in &fragment.crates {
                match crates.iter_mut().find(|c| c.name == req.name) {
                    Some(existing) => {
                        for f in &req.features {
                            if !existing.features.contains(f) {
                                existing.features.push(f.clone());
                            }
                        }
                        existing.features.sort();
                    }
                    None => crates.push(req.clone()),
                }
            }
        }
        crates.sort_by(|a, b| a.name.cmp(&b.name));

        Self {
            options,
            active,
            labels,
            selection,
            crates,
            fragments,
        }
    }

    /// Any credential-backed method needs the shared store, emitted once.
    fn needs_credential_store(&self) -> bool {
        self.fragments.iter().any(|f| f.needs_credential_store)
    }

    fn collect<'a, T: 'a>(
        &'a self,
        pick: impl Fn(&'a KitFragment) -> &'a Vec<T>,
    ) -> impl Iterator<Item = &'a T> {
        self.fragments.iter().flat_map(pick)
    }

    fn env_vars(&self) -> Vec<&KitEnvVar> {
        let mut seen: Vec<&str> = Vec::new();
        let mut out = Vec::new();
        // Two scenarios can want the same variable; it is listed once.
        for var in self.collect(|f| &f.env) {
            if seen.contains(&var.name.as_str()) {
                continue;
            }
            seen.push(&var.name);
            out.push(var);
        }
        out
    }

    /// Variables with no usable default. These are what stands between an
    /// unzipped project and a running one.
    fn required_env_vars(&self) -> Vec<&KitEnvVar> {
        self.env_vars()
            .into_iter()
            .filter(|v| v.default.is_none())
            .collect()
    }

    /// Every documented endpoint, in emission order.
    fn openapi_paths(&self) -> Vec<&crate::scenario::KitOpenApiPath> {
        self.collect(|f| &f.openapi_paths).collect()
    }

    fn openapi_schemas(&self) -> Vec<&String> {
        self.collect(|f| &f.openapi_schemas).collect()
    }

    /// Which engine alias the generated `AppState` should name.
    ///
    /// Only `session_store` and `token_manager` move the typestate, so the
    /// alias follows whether a token manager was supplied — and no fragment
    /// supplies one any more. The resource scenario used to: it minted tokens
    /// as well as validating them, which was the thing wrong with it (#52). A
    /// resource server *validates* tokens somebody else issued, so it needs no
    /// signing key and no `TokenManager`, and the engine stays
    /// `AkWebAppEngine`.
    ///
    /// Left as a function rather than inlined because the OP-server scenario
    /// will genuinely need `AkEngine` — it issues — and this is where that
    /// belongs when it lands.
    fn engine_alias(&self) -> &'static str {
        "AkWebAppEngine"
    }

    fn wants_openapi(&self) -> bool {
        self.options.openapi && !self.openapi_paths().is_empty()
    }

    fn setup_steps(&self) -> Vec<&KitSetup> {
        self.collect(|f| &f.setup).collect()
    }

    /// Every link worth following, deduplicated and in emission order: the
    /// ones that apply to any generated project, then the ones each selected
    /// scenario contributed.
    fn links(&self) -> Vec<KitLink> {
        let mut out = vec![
            KitLink::docs("Quickstart", "guides/quickstart"),
            KitLink::docs(
                "The endpoints the engine wires for you",
                "guides/wired-endpoints",
            ),
            KitLink::docs(
                "Why the builder is a typestate",
                "concepts/typestate-builder",
            ),
            KitLink::example("crates/authkestra/examples/axum_basic_setup.rs"),
        ];
        if self.needs_credential_store() {
            out.push(KitLink::docs("The SQL store", "storage/sql-store"));
        }
        for link in self.collect(|f| &f.links) {
            if !out.contains(link) {
                out.push(link.clone());
            }
        }
        out
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

    fn is_active(&self, id: &str) -> bool {
        self.active.iter().any(|a| a == id)
    }

    /// A filename-safe summary of the selection.
    ///
    /// Scenario ids alone would collide: picking GitHub and picking Google are
    /// both "oauth", and two downloads that differ would arrive under one name.
    /// So the chosen options are folded in too.
    fn slug(&self) -> String {
        // Options belong in the name too: two downloads that differ only by an
        // opt-in would otherwise arrive under one filename and overwrite each
        // other in a downloads folder.
        let mut extras = String::new();
        if self.options.openapi {
            extras.push_str("-openapi");
        }
        if self.options.ts_client {
            extras.push_str("-client");
        }

        if self.selection.is_empty() {
            return format!("base{extras}");
        }
        let methods = self
            .selection
            .iter()
            .map(|(id, options)| {
                if options.is_empty() {
                    id.clone()
                } else {
                    format!("{id}-{}", options.join("-"))
                }
            })
            .collect::<Vec<_>>()
            .join("-");
        format!("{methods}{extras}")
    }
}

/// The options a control has selected, if it is the kind of control that has
/// any. A toggle contributes nothing beyond its own id.
fn chosen_options(value: &ControlValue) -> Vec<String> {
    match value {
        ControlValue::Toggle { .. } => Vec::new(),
        ControlValue::SelectOne { selected } => selected.iter().cloned().collect(),
        ControlValue::SelectMany { selected } => selected.clone(),
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

/// The TypeScript client, when it was asked for and there is something for it
/// to talk to.
fn client_file(plan: &Plan) -> Option<GeneratedFile> {
    if !plan.options.ts_client || !(plan.is_active("passkeys") || plan.is_active("totp")) {
        return None;
    }
    Some(GeneratedFile {
        path: client::CLIENT_PATH.to_string(),
        contents: client::client_ts(plan),
    })
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

# Stands alone, deliberately.
#
# Without this, unzipping anywhere inside an existing Cargo workspace — a
# monorepo, or a checkout you happened to be standing in — fails with "current
# package believes it's in a workspace when it's not", before a line of this
# project is compiled. An empty table opts out.
[workspace]

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
{extra}"#,
        extra = third_party_deps(plan),
    )
}

/// Dependency lines for the non-authkestra crates the selection implies.
///
/// The *names and features* come from `Scenario::consequences` — the same list
/// the diff renders — so this only supplies the versions.
fn third_party_deps(plan: &Plan) -> String {
    let mut lines = Vec::new();
    for req in &plan.crates {
        let version = match req.name.as_str() {
            // Emitted in the base block above, pinned together.
            "authkestra-engine" | "authkestra-axum" => continue,
            // Every other authkestra crate is pinned to the same version.
            n if n.starts_with("authkestra-") => AUTHKESTRA_VERSION,
            "webauthn-rs" => "0.5",
            "uuid" => "1",
            "serde" => "1",
            "utoipa" => "5",
            // The resource fragment builds a `Validation` directly. Kept in
            // step with the engine's own requirement, and with `aws_lc_rs`
            // deliberately unset: a second crypto backend in one crate is how
            // this workspace got a boot panic once already.
            "jsonwebtoken" => "11",
            "sqlx" => "0.8",
            "url" => "2.5",
            other => {
                // A scenario naming a crate the kit has no version for would
                // otherwise emit a manifest that does not resolve.
                debug_assert!(false, "no pinned version for `{other}`");
                continue;
            }
        };
        let mut features = req.features.clone();
        // sqlx needs an async runtime, which no scenario names because it is a
        // property of how the generated project drives it, not of the scenario.
        if req.name == "sqlx" && !features.iter().any(|f| f.starts_with("runtime-")) {
            features.push("runtime-tokio-rustls".to_string());
        }
        features.sort();

        if features.is_empty() {
            lines.push(format!("{} = \"{version}\"", req.name));
        } else {
            let feats = features
                .iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "{} = {{ version = \"{version}\", features = [{feats}] }}",
                req.name
            ));
        }
    }
    // The credential store needs a runtime and a driver beyond what a scenario
    // names, and `url` is used by the WebAuthn prelude.
    if plan.needs_credential_store() && !lines.iter().any(|l| l.starts_with("sqlx =")) {
        lines.push(
            "sqlx = { version = \"0.8\", features = [\"runtime-tokio-rustls\", \"sqlite\"] }"
                .to_string(),
        );
    }
    if plan.is_active("passkeys") && !lines.iter().any(|l| l.starts_with("url =")) {
        lines.push("url = \"2.5\"".to_string());
    }
    if lines.is_empty() {
        String::new()
    } else {
        lines.sort();
        format!("{}\n", lines.join("\n"))
    }
}

/// The user id a generated project stores credentials against.
///
/// authkestra deliberately owns no user table, so there is nothing to look
/// this up in — which is exactly the decision the README explains. Deriving it
/// from the username keeps a starter self-contained and stable across
/// restarts.
const USER_ID_HELPER: &str = r#"/// The user id your application would supply from its own users table.
///
/// authkestra deliberately owns no users, so there is nothing to look this up
/// in. Deriving it from the username keeps this project self-contained and
/// stable across restarts. **Replace it with your own primary key** — every
/// credential is stored against whatever this returns, so changing it later
/// orphans everything already enrolled.
fn user_id_for(username: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, username.as_bytes()).to_string()
}"#;

fn main_rs(plan: &Plan) -> String {
    let note = if plan.is_empty() {
        "//! Nothing was selected in the playground, so this is the smallest\n         //! useful engine: sessions and the framework's `/auth` routes, with no\n         //! authentication method registered yet."
            .to_string()
    } else {
        format!(
            "//! Generated from a playground configuration: {}.",
            plan.active.join(", ")
        )
    };

    // Imports: the base set, plus whatever the fragments need, deduplicated and
    // sorted so the same selection always yields identical output.
    let mut imports: Vec<String> = vec![
        "use authkestra_axum::{AuthSession, AxumError, AxumExt, AxumState};".to_string(),
        "use authkestra_engine::store::memory::MemoryStore;".to_string(),
        format!(
            "use authkestra_engine::{{{}, Engine, SessionConfig, SessionStore}};",
            plan.engine_alias()
        ),
        "use axum::http::StatusCode;".to_string(),
        "use axum::response::IntoResponse;".to_string(),
        "use axum::routing::get;".to_string(),
        "use axum::{Json, Router};".to_string(),
        "use serde_json::json;".to_string(),
        "use std::sync::Arc;".to_string(),
        "use tower_cookies::CookieManagerLayer;".to_string(),
    ];
    if plan.needs_credential_store() {
        imports.push(
            "use authkestra_engine::store::sql::credential::SqlxCredentialStore;".to_string(),
        );
        imports.push("use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};".to_string());
    }
    imports.extend(plan.collect(|f| &f.imports).cloned());
    imports.sort();
    imports.dedup();
    let imports = imports.join("\n");

    // The shared credential store, emitted once however many methods want it.
    let credential_store = if plan.needs_credential_store() {
        r#"
    // One credential store, shared by every method that enrols something.
    // SQLite keeps this to a single file; the store is a trait, so Postgres or
    // MySQL is a feature flag and a URL away.
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://credentials.db".to_string());
    let options = database_url
        .parse::<SqliteConnectOptions>()
        .expect("DATABASE_URL must be a SQLite URL")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .expect("could not open the credential database");
    // `SqlxCredentialStore`'s derived `Clone` is over-constrained — it requires
    // `DB: Clone`, which `Sqlite` is not — so the pool is what gets cloned and
    // a store is built per use. Construction just wraps an already-`Arc`ed
    // pool, so this costs nothing.
    SqlxCredentialStore::new(pool.clone())
        .migrate()
        .await
        .expect("could not create the credentials table");
"#
    } else {
        ""
    };

    let prelude = {
        let lines: Vec<String> = plan.collect(|f| &f.prelude).cloned().collect();
        if lines.is_empty() {
            String::new()
        } else {
            format!("\n{}\n", lines.join("\n\n"))
        }
    };

    let builder_calls = {
        let lines: Vec<String> = plan.collect(|f| &f.builder_calls).cloned().collect();
        if lines.is_empty() {
            String::new()
        } else {
            format!("\n{}", lines.join("\n"))
        }
    };

    let extra_routes = {
        let lines: Vec<String> = plan.collect(|f| &f.routes).cloned().collect();
        if lines.is_empty() {
            String::new()
        } else {
            format!("\n{}", lines.join("\n"))
        }
    };

    let extra_handlers = {
        let mut fns: Vec<String> = Vec::new();
        // Any method that stores a credential needs a user id to store it
        // against, so this belongs to the store rather than to one method.
        if plan.needs_credential_store() {
            fns.push(USER_ID_HELPER.to_string());
        }
        fns.extend(plan.collect(|f| &f.handlers).cloned());
        let fns: Vec<String> = fns;
        if fns.is_empty() {
            String::new()
        } else {
            format!("\n{}\n", fns.join("\n\n"))
        }
    };

    // Annotations are woven in here rather than baked into the fragments, so a
    // project that did not ask for a spec carries no trace of one: no unused
    // attribute, no dependency, and no `cfg` naming a feature that does not
    // exist in its manifest.
    let extra_handlers = if plan.wants_openapi() {
        let mut annotated = extra_handlers;
        for path in plan.openapi_paths() {
            let needle = format!("async fn {}(", path.handler);
            let Some(at) = annotated.find(&needle) else {
                // A handler named in `openapi_paths` that no fragment emits is
                // a bug in the fragment, not something to paper over.
                panic!("no generated handler named `{}`", path.handler);
            };
            // Back up over the handler's doc comment so the attribute sits
            // above it, where rustfmt and rustdoc both expect it.
            let start = annotated[..at].rfind("\n\n").map(|i| i + 2).unwrap_or(at);
            annotated.insert_str(start, &format!("{}\n", path.annotation));
        }
        for schema in plan.openapi_schemas() {
            let needle = format!("struct {schema} {{");
            if let Some(at) = annotated.find(&needle) {
                annotated.insert_str(at, "#[derive(utoipa::ToSchema)]\n");
            }
        }
        annotated
    } else {
        extra_handlers
    };

    let openapi_block = if plan.wants_openapi() {
        let paths = plan
            .openapi_paths()
            .iter()
            .map(|p| p.handler.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let schemas = plan
            .openapi_schemas()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            r#"
/// The generated OpenAPI document.
///
/// Descriptions come from the handlers' own doc comments, so the spec and the
/// code cannot drift: there is only one place to write them.
#[derive(utoipa::OpenApi)]
#[openapi(paths({paths}), components(schemas({schemas})))]
struct ApiDoc;

async fn openapi_spec() -> impl IntoResponse {{
    use utoipa::OpenApi as _;
    Json(ApiDoc::openapi())
}}
"#
        )
    } else {
        String::new()
    };

    let openapi_route = if plan.wants_openapi() {
        "\n        .route(\"/openapi.json\", get(openapi_spec))".to_string()
    } else {
        String::new()
    };

    let engine_alias = plan.engine_alias();

    let state_fields = {
        let mut lines: Vec<String> = Vec::new();
        // Emitted once, by whoever needs credentials at all, rather than by
        // each method — two methods sharing one store must not declare the
        // field twice.
        if plan.needs_credential_store() {
            lines.push(
                "    /// The credential pool. `SqlxCredentialStore` is not `Clone`, so\n\
                 \x20   /// handlers build one per use; construction just wraps an\n\
                 \x20   /// already-`Arc`ed pool.\n\
                 \x20   pool: sqlx::SqlitePool,"
                    .to_string(),
            );
        }
        lines.extend(plan.collect(|f| &f.state_fields).cloned());
        let lines: Vec<String> = lines;
        if lines.is_empty() {
            String::new()
        } else {
            format!("\n{}", lines.join("\n"))
        }
    };

    let state_init = {
        let mut lines: Vec<String> = Vec::new();
        if plan.needs_credential_store() {
            lines.push("        pool: pool.clone(),".to_string());
        }
        lines.extend(plan.collect(|f| &f.state_init).cloned());
        let lines: Vec<String> = lines;
        if lines.is_empty() {
            String::new()
        } else {
            format!("\n{}", lines.join("\n"))
        }
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

{imports}

/// `{engine_alias}` is the alias for the engine this project builds. Using the
/// alias rather than spelling out the typestate generics keeps the compile
/// error legible when a required builder call is missing.
#[derive(Clone, AxumState)]
struct AppState {{
    #[authkestra(engine)]
    auth: {engine_alias},{state_fields}
}}

#[tokio::main]
async fn main() {{
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,authkestra=debug".into()),
        )
        .init();

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
{credential_store}{prelude}
    // Sessions in memory: no infrastructure to run, and nothing survives a
    // restart. `SessionStore` is a trait, so swapping this for Redis is a
    // one-line change.
    let session_store: Arc<dyn SessionStore> = Arc::new(MemoryStore::default());

    // The typestate builder only exposes session APIs once `session_store` has
    // been supplied, so a missing call is a compile error rather than a
    // runtime surprise. Everything after it returns `Self`, which is why
    // composing methods and providers is just a longer chain.
    let engine = Engine::builder()
        .session_store(session_store)
        .session_config(SessionConfig {{
            // Plain HTTP locally. Set this to `true` behind TLS, or the browser
            // will refuse to store the cookie.
            secure: false,
            ..Default::default()
        }}){builder_calls}
        .build();

    let state = AppState {{
        auth: engine.clone(),{state_init}
    }};

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/me", get(me)){extra_routes}{openapi_route}
        // The engine's own `/auth/*` routes.
        .merge(engine.axum_router())
        // The engine reads and writes cookies, so the cookie layer must wrap
        // the merged routes.
        .layer(CookieManagerLayer::new())
        .with_state(state);

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
{extra_handlers}{openapi_block}"#
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
            plan.labels.join("**, **")
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

    let notes: Vec<String> = plan.collect(|f| &f.notes).cloned().collect();
    let notes_section = if notes.is_empty() {
        String::new()
    } else {
        format!("\n## About what you selected\n\n{}\n", notes.join("\n\n"))
    };

    format!(
        r#"# {PROJECT_NAME}

Generated by the [authkestra playground](https://play.authkestra.com).

{selected}
{configure}{run}{deps_section}{notes_section}{extras}{differences}
## Read more

{links}

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
"#,
        configure = configure_section(plan),
        differences = differences_section(plan),
        extras = extras_section(plan),
        run = run_section(plan),
        links = link_list(&plan.links()),
    )
}

/// What the developer must do before `cargo run` produces anything useful.
///
/// Empty when there is genuinely nothing to configure, rather than a section
/// that says "no configuration required" and wastes the reader's attention.
/// Where this project deliberately differs from the playground it came from.
///
/// The promise of a download is "this is what you just used", so a difference
/// the reader has to discover is a small betrayal of that. A parity test keeps
/// this list honest: the assertions both sides must satisfy live in
/// `apps/api/tests/parity.rs`, and anything not on that list is written here.
/// What the two download opt-ins added, when they were asked for.
fn extras_section(plan: &Plan) -> String {
    let mut parts = Vec::new();

    if plan.wants_openapi() {
        parts.push(
            "### OpenAPI\n\nThe service serves its own document at `GET /openapi.json`. \
             Descriptions come from the handlers' doc comments, so there is one place to \
             write them and the spec cannot drift from the code.\n\n`utoipa` is a normal \
             dependency here because you asked for it; remove the annotations and the \
             `/openapi.json` route to drop it."
                .to_string(),
        );
    }

    if plan.options.ts_client && (plan.is_active("passkeys") || plan.is_active("totp")) {
        parts.push(format!(
            "### TypeScript client\n\n`{}` is a dependency-free client for the endpoints \
             above. It is hand-written rather than generated from the OpenAPI document, \
             deliberately: a schema describes the wire shapes, but not the base64url \
             conversion `navigator.credentials` needs on both sides, which is where most \
             passkey integrations break.\n\nCopy it, or import it as-is. It has no build \
             step and no dependencies.",
            client::CLIENT_PATH
        ));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("\n## What you also asked for\n\n{}\n", parts.join("\n\n"))
    }
}

fn differences_section(plan: &Plan) -> String {
    if !plan.needs_credential_store() {
        return String::new();
    }

    let mut items = vec![
        "**Routes are yours, not the playground's.** The playground serves its \
         ceremonies under `/api/scenarios/{id}/action/{action}` because it drives \
         many scenarios from one surface. Here they are `/auth/passkey/...` and \
         `/auth/totp/...`, which is what you would write by hand. Rename them \
         freely — nothing in the framework depends on these paths."
            .to_string(),
        "**A ceremony is addressed explicitly.** The playground scopes an \
         in-flight ceremony to its demo session cookie. This project has no \
         session before sign-in, so `register/start` and `login/start` return a \
         `ceremony_id` that the matching `finish` call must send back."
            .to_string(),
    ];

    items.push(
        "**Sessions and credentials outlive a restart differently.** The \
         playground keeps demo state in Redis with a twelve-hour expiry, because \
         it is a demo. Here, credentials are a SQLite file that persists and \
         sessions are in memory and do not."
            .to_string(),
    );

    format!(
        "\n## How this differs from the playground\n\n{}\n",
        items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn configure_section(plan: &Plan) -> String {
    let required = plan.required_env_vars();
    let steps = plan.setup_steps();
    if required.is_empty() && steps.is_empty() {
        return String::new();
    }

    let mut out = String::from("\n## Configure it\n\nStart from the generated example:\n\n```sh\ncp .env.example .env\n```\n");

    if !required.is_empty() {
        let list = required
            .iter()
            .map(|v| format!("- `{}` — {}", v.name, v.comment))
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&format!(
            "\nThese have no default and the server exits at startup without them:\n\n{list}\n\nEverything else in `.env.example` already has a working local value.\n"
        ));
    }

    for step in steps {
        let numbered = step
            .steps
            .iter()
            .enumerate()
            .map(|(i, text)| format!("{}. {text}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&format!("\n### {}\n\n{numbered}\n", step.title));
    }

    out
}

fn run_section(plan: &Plan) -> String {
    // With nothing required, copying the env file is a convenience rather than
    // a step, so it is mentioned here instead of given a section of its own.
    let env_hint = if plan.required_env_vars().is_empty() && plan.setup_steps().is_empty() {
        "\n`PORT` overrides the bind port. Copy `.env.example` to `.env` to change\nthat or the log filter.\n"
    } else {
        "\nThe server reads `.env` on startup.\n"
    };

    format!(
        r#"
## Run it

```sh
cargo run
curl localhost:3000/health
```
{env_hint}"#
    )
}

fn link_list(links: &[KitLink]) -> String {
    links
        .iter()
        .map(|l| format!("- [{}]({})", l.label, l.url))
        .collect::<Vec<_>>()
        .join("\n")
}

fn env_example(plan: &Plan) -> String {
    let mut out = String::from(
        "# Copy to .env and fill in. Only the variables this configuration\n\
         # actually reads are listed.\n\
         \n\
         # Port the server binds to.\n\
         PORT=3000\n\
         \n\
         # Log filter, e.g. `info`, or `debug,authkestra=trace`.\n\
         RUST_LOG=info,authkestra=debug\n",
    );

    if plan.needs_credential_store() {
        out.push_str(
            "\n\
             # Where enrolled credentials (passkeys, TOTP secrets) are stored.\n\
             DATABASE_URL=sqlite://credentials.db\n",
        );
    }

    for var in plan.env_vars() {
        out.push_str(&format!("\n# {}\n", var.comment));
        match &var.default {
            Some(value) => out.push_str(&format!("{}={}\n", var.name, value)),
            // No default: left blank on purpose, so starting without it fails
            // loudly rather than silently using something wrong.
            None => out.push_str(&format!("{}=\n", var.name)),
        }
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

    fn kit_with(scenarios: &[(&str, ControlValue)]) -> StarterKit {
        let registry = ScenarioRegistry::with_providers(vec![
            "github".to_string(),
            "google".to_string(),
            "discord".to_string(),
        ]);
        let mut config = DemoConfig::defaults_for(&registry);
        for (id, value) in scenarios {
            config.set(id, value.clone());
        }
        StarterKit::generate(&config, &registry)
    }

    fn on() -> ControlValue {
        ControlValue::Toggle { enabled: true }
    }

    /// The rule from the spec: TOTP alone is the only way in, so it must be a
    /// first factor. Alongside another method it is step-up, which is the
    /// stronger design and almost certainly what was intended.
    #[test]
    fn totp_alone_is_a_first_factor() {
        let kit = kit_with(&[("totp", on())]);
        let main = contents(&kit, "src/main.rs");
        assert!(main.contains(".with_totp("), "{main}");
        assert!(!main.contains(".with_mfa_method("));

        let readme = contents(&kit, "README.md");
        assert!(
            readme.contains("first factor"),
            "the README must say which role was chosen"
        );
    }

    #[test]
    fn totp_beside_another_method_becomes_step_up() {
        let kit = kit_with(&[("totp", on()), ("passkeys", on())]);
        let main = contents(&kit, "src/main.rs");
        assert!(main.contains(".with_mfa_method("), "{main}");
        assert!(
            !main.contains(".with_totp("),
            "it should not also be registered as a first factor"
        );

        let readme = contents(&kit, "README.md");
        assert!(readme.contains("step-up"), "the change must be explained");
    }

    /// Two credential-backed methods share one store, as in the framework's own
    /// MFA example.
    #[test]
    fn the_credential_store_is_emitted_once_however_many_methods_want_it() {
        let main = contents(
            &kit_with(&[("totp", on()), ("passkeys", on())]),
            "src/main.rs",
        );
        assert_eq!(
            main.matches("SqlitePoolOptions::new()").count(),
            1,
            "the store should be built once and shared"
        );
        assert_eq!(main.matches(".migrate()").count(), 1);
    }

    #[test]
    fn a_session_only_project_has_no_credential_store() {
        let main = contents(&kit_with(&[]), "src/main.rs");
        assert!(!main.contains("SqlxCredentialStore"));
        assert!(!main.contains("DATABASE_URL"));
    }

    #[test]
    fn selecting_providers_emits_their_dependency_and_secrets() {
        let kit = kit_with(&[(
            "oauth",
            ControlValue::SelectMany {
                selected: vec!["github".to_string(), "google".to_string()],
            },
        )]);
        let cargo = contents(&kit, "Cargo.toml");
        assert!(cargo.contains("authkestra-providers"), "{cargo}");
        // Google is OIDC, so it needs a crate the others do not.
        assert!(cargo.contains("authkestra-oidc"), "{cargo}");

        let main = contents(&kit, "src/main.rs");
        assert!(main.contains("GithubProvider::new("));
        assert!(main.contains("GoogleProvider::new("));

        let env = contents(&kit, ".env.example");
        for expected in [
            "GITHUB_CLIENT_ID",
            "GITHUB_CLIENT_SECRET",
            "GOOGLE_CLIENT_ID",
        ] {
            assert!(
                env.contains(expected),
                "{expected} missing from .env.example"
            );
        }
        // A secret has no default, so starting without it fails loudly.
        assert!(env.contains("GITHUB_CLIENT_SECRET=\n"), "{env}");
    }

    /// The inverse of the test this replaces.
    ///
    /// Until 0.9.0, `authkestra-providers` declared `urlencoding` behind the
    /// `discord` feature while using it for every provider, so a project
    /// selecting only GitHub failed to compile inside the dependency and the
    /// generator had to enable `discord` as a workaround. Fixed upstream in
    /// marcjazz/authkestra#322, so a selection now asks for exactly what it
    /// selected — and quietly reacquiring the extra provider would be a
    /// regression nobody would notice.
    #[test]
    fn a_selection_asks_for_only_the_providers_it_chose() {
        let kit = kit_with(&[(
            "oauth",
            ControlValue::SelectMany {
                selected: vec!["github".to_string()],
            },
        )]);
        let cargo = contents(&kit, "Cargo.toml");
        let line = cargo
            .lines()
            .find(|l| l.starts_with("authkestra-providers"))
            .expect("providers dependency");
        assert!(line.contains("\"github\""), "{line}");
        assert!(
            !line.contains("\"discord\""),
            "the discord workaround is back: {line}"
        );
    }

    /// Deterministic output keeps the download diffable and the zip cacheable.
    #[test]
    fn the_same_configuration_generates_identical_bytes() {
        let a = kit_with(&[("totp", on()), ("passkeys", on())]);
        let b = kit_with(&[("passkeys", on()), ("totp", on())]);
        assert_eq!(
            a.files, b.files,
            "emission order must follow the registry, not the order of selection"
        );
    }

    // ---- P4 #30: the generated project has to be configurable from its own
    // README, or the download is a puzzle rather than a starting point. ----

    fn full_kit() -> StarterKit {
        kit_with(&[
            ("passkeys", on()),
            ("totp", on()),
            (
                "oauth",
                ControlValue::SelectMany {
                    selected: vec!["github".to_string(), "google".to_string()],
                },
            ),
        ])
    }

    /// The acceptance criterion, stated directly: every variable the project
    /// refuses to start without is named in the README. Anything missing here
    /// is a value the reader has to guess.
    #[test]
    fn the_readme_names_every_variable_that_has_no_default() {
        let kit = full_kit();
        let readme = contents(&kit, "README.md");

        let required: Vec<String> = contents(&kit, ".env.example")
            .lines()
            .filter(|l| l.ends_with('=') && !l.starts_with('#'))
            .map(|l| l.trim_end_matches('=').to_string())
            .collect();

        assert!(
            !required.is_empty(),
            "this configuration should require secrets"
        );
        for name in required {
            assert!(
                readme.contains(&name),
                "{name} has no default but the README never mentions it"
            );
        }
    }

    #[test]
    fn the_readme_says_where_to_register_each_provider() {
        let readme = contents(&full_kit(), "README.md");

        for (console, callback) in [
            (
                "https://github.com/settings/developers",
                "http://localhost:3000/auth/callback/github",
            ),
            (
                "https://console.cloud.google.com/apis/credentials",
                "http://localhost:3000/auth/callback/google",
            ),
        ] {
            assert!(readme.contains(console), "no link to {console}");
            // The exact callback URL, because "nearly right" fails at the
            // provider with an error that does not say why.
            assert!(readme.contains(callback), "{callback} is not spelled out");
        }
    }

    #[test]
    fn a_configuration_with_secrets_tells_you_to_create_the_env_file() {
        let readme = contents(&full_kit(), "README.md");
        assert!(readme.contains("## Configure it"));
        assert!(readme.contains("cp .env.example .env"));
    }

    /// A section saying "nothing to configure" costs the reader attention and
    /// tells them nothing, so it is not emitted at all.
    #[test]
    fn a_project_with_nothing_to_configure_has_no_configure_section() {
        let readme = contents(&base(), "README.md");
        assert!(!readme.contains("## Configure it"), "{readme}");
        // Copying the file is still worth mentioning, just not as a step.
        assert!(readme.contains(".env.example"));
    }

    #[test]
    fn the_readme_links_to_the_docs_and_examples_behind_what_was_selected() {
        let readme = contents(&full_kit(), "README.md");
        for url in [
            "https://authkestra.com/guides/quickstart/",
            "https://authkestra.com/providers/passkeys/",
            "https://authkestra.com/providers/totp/",
            "https://authkestra.com/providers/oauth2/",
            "https://authkestra.com/providers/oidc/",
            "https://github.com/marcjazz/authkestra/blob/main/crates/authkestra/examples/axum_oauth2_github.rs",
        ] {
            assert!(readme.contains(url), "missing {url}");
        }
    }

    /// Passkeys and TOTP both point at `totp_webauthn.rs`. Selecting both must
    /// not list it twice.
    #[test]
    fn links_are_listed_once_however_many_scenarios_want_them() {
        let readme = contents(&full_kit(), "README.md");
        let url = "crates/authkestra-engine/examples/totp_webauthn.rs";
        assert_eq!(readme.matches(url).count(), 1, "{url} is duplicated");
    }

    #[test]
    fn a_selection_only_links_to_what_it_actually_uses() {
        let readme = contents(&base(), "README.md");
        for url in [
            "https://authkestra.com/providers/passkeys/",
            "https://authkestra.com/providers/oauth2/",
            "https://authkestra.com/storage/sql-store/",
        ] {
            assert!(!readme.contains(url), "base project should not link {url}");
        }
    }

    /// Prose assembled from Rust string literals picks up the source file's own
    /// indentation whenever a continuation `\` is forgotten. It surfaces two
    /// ways — a run of spaces mid-sentence, or a wrapped line that arrives
    /// indented — and the second is worse than untidy: four leading spaces make
    /// Markdown render the paragraph as a code block.
    ///
    /// Both have shipped in this generator before. Nothing else catches them.
    #[test]
    fn generated_prose_carries_no_stray_indentation() {
        for kit in [base(), full_kit()] {
            for path in ["README.md", ".env.example"] {
                let text = contents(&kit, path);
                let mut fenced = false;
                for (n, line) in text.lines().enumerate() {
                    let at = format!("{path}:{}", n + 1);
                    if line.starts_with("```") {
                        fenced = !fenced;
                        continue;
                    }
                    if fenced {
                        continue;
                    }
                    assert!(
                        !line.trim_start().contains("  "),
                        "{at} has a run of spaces mid-line: {line:?}"
                    );
                    assert!(
                        !line.starts_with(' '),
                        "{at} is indented, which Markdown may render as code: {line:?}"
                    );
                }
                assert!(!fenced, "{path} has an unclosed code fence");
            }
        }
    }

    /// Scenario ids are an implementation detail. The README is read by a
    /// person.
    #[test]
    fn the_readme_names_scenarios_the_way_the_playground_does() {
        let readme = contents(&full_kit(), "README.md");
        let summary = readme
            .lines()
            .find(|l| l.starts_with("Selected in the playground:"))
            .expect("the README summarises the selection");
        assert!(summary.contains("Passkeys"), "{summary}");
        assert!(!summary.contains("**passkeys**"), "{summary}");
    }

    // ---- P4 #48: the generated project has an HTTP surface for its
    // ceremonies, not just a wired engine. ----

    #[test]
    fn selecting_passkeys_generates_the_four_ceremony_routes() {
        let main = contents(&kit_with(&[("passkeys", on())]), "src/main.rs");
        for route in [
            "/auth/passkey/register/start",
            "/auth/passkey/register/finish",
            "/auth/passkey/login/start",
            "/auth/passkey/login/finish",
        ] {
            assert!(main.contains(route), "missing {route}");
        }
        // A route with no handler behind it is a compile error, but asserting
        // the pair keeps the failure legible when only one is dropped.
        for handler in [
            "async fn passkey_register_start",
            "async fn passkey_register_finish",
            "async fn passkey_login_start",
            "async fn passkey_login_finish",
        ] {
            assert!(main.contains(handler), "missing {handler}");
        }
    }

    #[test]
    fn selecting_totp_generates_its_two_routes() {
        let main = contents(&kit_with(&[("totp", on())]), "src/main.rs");
        for expected in [
            "/auth/totp/enroll",
            "/auth/totp/verify",
            "async fn totp_enrol",
            "async fn totp_verify",
        ] {
            assert!(main.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn a_project_without_a_method_has_no_ceremony_routes() {
        let main = contents(&base(), "src/main.rs");
        assert!(!main.contains("/auth/passkey/"), "{main}");
        assert!(!main.contains("/auth/totp/"), "{main}");
        // And nothing that only exists to serve them.
        assert!(!main.contains("fn user_id_for"), "{main}");
        assert!(!main.contains("struct Ceremonies"), "{main}");
    }

    /// Both methods store credentials against a user id and share one pool.
    /// Emitting either twice is a compile error in the generated project, and
    /// the compile matrix would catch it — but only after a full build.
    #[test]
    fn shared_scaffolding_is_emitted_once_for_two_methods() {
        let main = contents(
            &kit_with(&[("passkeys", on()), ("totp", on())]),
            "src/main.rs",
        );
        assert_eq!(main.matches("fn user_id_for").count(), 1);
        assert_eq!(main.matches("pool: sqlx::SqlitePool,").count(), 1);
        assert_eq!(main.matches("pool: pool.clone(),").count(), 1);
    }

    /// The one field `rename_all = "camelCase"` gets wrong. It cost this
    /// playground a silent sign-in outage; a generated project must not
    /// inherit the same bug.
    #[test]
    fn the_generated_assertion_spells_client_data_json_correctly() {
        let main = contents(&kit_with(&[("passkeys", on())]), "src/main.rs");
        assert!(
            main.contains(r#"#[serde(rename = "clientDataJSON")]"#),
            "the generated assertion type would reject every real sign-in"
        );
    }

    /// Handlers need `uuid` and `serde` derive; the framework does not. The
    /// distinction matters because the playground's diff answers "what does
    /// authkestra require", and these are not part of that answer.
    #[test]
    fn handler_dependencies_are_emitted_without_entering_the_diff() {
        use crate::scenario::ScenarioRegistry;

        let kit = kit_with(&[("passkeys", on())]);
        let manifest = contents(&kit, "Cargo.toml");
        assert!(manifest.contains("uuid"), "{manifest}");
        assert!(manifest.contains("serde = "), "{manifest}");

        let registry = ScenarioRegistry::with_providers(Vec::new());
        let scenario = registry.get("passkeys").expect("passkeys is registered");
        let consequences = scenario.consequences(&on());
        assert!(
            !consequences.crates.iter().any(|c| c.name == "uuid"),
            "uuid is a handler dependency, not something authkestra asks for"
        );
    }

    /// A download unzipped inside someone's monorepo must still build. Without
    /// an empty `[workspace]`, cargo refuses before compiling anything, with an
    /// error about a workspace the reader never asked to be part of.
    #[test]
    fn the_generated_project_stands_outside_any_workspace() {
        for kit in [base(), full_kit()] {
            let manifest = contents(&kit, "Cargo.toml");
            assert!(
                manifest.contains("\n[workspace]\n"),
                "the manifest would inherit a surrounding workspace:\n{manifest}"
            );
        }
    }

    // ---- P4 #49: two independent opt-ins ----

    fn kit_with_options(scenarios: &[(&str, ControlValue)], options: KitOptions) -> StarterKit {
        let registry = ScenarioRegistry::with_providers(vec!["github".to_string()]);
        let mut config = DemoConfig::defaults_for(&registry);
        for (id, value) in scenarios {
            config.set(id, value.clone());
        }
        StarterKit::generate_with(&config, &registry, options)
    }

    const BOTH: KitOptions = KitOptions {
        openapi: true,
        ts_client: true,
    };

    /// The default must leave no trace of either opt-in. Someone who wants a
    /// Rust service should not find TypeScript in their download, and should
    /// not read `utoipa` attributes they did not ask for.
    #[test]
    fn a_download_that_asked_for_nothing_extra_contains_nothing_extra() {
        let kit = kit_with(&[("passkeys", on()), ("totp", on())]);

        assert!(
            kit.file(client::CLIENT_PATH).is_none(),
            "a TS client appeared"
        );
        let main = contents(&kit, "src/main.rs");
        assert!(!main.contains("utoipa"), "utoipa annotations appeared");
        assert!(!main.contains("openapi"), "an openapi route appeared");
        assert!(
            !contents(&kit, "Cargo.toml").contains("utoipa"),
            "utoipa is in the manifest"
        );
    }

    #[test]
    fn the_two_opt_ins_are_independent() {
        let spec_only = kit_with_options(
            &[("totp", on())],
            KitOptions {
                openapi: true,
                ts_client: false,
            },
        );
        assert!(contents(&spec_only, "src/main.rs").contains("utoipa::path"));
        assert!(
            spec_only.file(client::CLIENT_PATH).is_none(),
            "asking for a spec should not produce TypeScript"
        );

        let client_only = kit_with_options(
            &[("totp", on())],
            KitOptions {
                openapi: false,
                ts_client: true,
            },
        );
        assert!(client_only.file(client::CLIENT_PATH).is_some());
        assert!(
            !contents(&client_only, "Cargo.toml").contains("utoipa"),
            "asking for a client should not add a Rust dependency"
        );
    }

    /// Every generated handler must appear in the spec. A documented endpoint
    /// that silently stops being documented is worse than none, because the
    /// spec still looks complete.
    #[test]
    fn every_ceremony_endpoint_is_documented() {
        let main = contents(
            &kit_with_options(&[("passkeys", on()), ("totp", on())], BOTH),
            "src/main.rs",
        );
        for route in [
            "/auth/passkey/register/start",
            "/auth/passkey/register/finish",
            "/auth/passkey/login/start",
            "/auth/passkey/login/finish",
            "/auth/totp/enroll",
            "/auth/totp/verify",
        ] {
            let documented = main.match_indices(route).any(|(i, _)| {
                main[..i].rfind("#[utoipa::path(").is_some_and(|a| {
                    // the annotation that mentions this route is the nearest one above it
                    main[a..i].matches("async fn").count() == 0
                })
            });
            assert!(documented, "{route} is served but not documented");
        }
        assert_eq!(main.matches("#[utoipa::path(").count(), 6);
    }

    /// The client is only worth emitting when there is something to call.
    #[test]
    fn no_client_is_emitted_for_a_project_with_no_ceremonies() {
        let kit = kit_with_options(&[], BOTH);
        assert!(kit.file(client::CLIENT_PATH).is_none());
        // And with nothing to document, no spec route either.
        assert!(!contents(&kit, "src/main.rs").contains("openapi.json"));
    }

    /// The client only covers what the project actually serves.
    #[test]
    fn the_client_covers_the_selected_methods_and_no_others() {
        let totp_only = kit_with_options(&[("totp", on())], BOTH);
        let ts = &totp_only.file(client::CLIENT_PATH).unwrap().contents;
        assert!(ts.contains("export async function enrolTotp"));
        assert!(
            !ts.contains("enrolPasskey"),
            "the client offers a passkey call the project cannot answer"
        );

        let passkeys_only = kit_with_options(&[("passkeys", on())], BOTH);
        let ts = &passkeys_only.file(client::CLIENT_PATH).unwrap().contents;
        assert!(ts.contains("export async function enrolPasskey"));
        assert!(!ts.contains("enrolTotp"));
    }

    /// The one field a blanket camelCase rule gets wrong, on the browser side
    /// this time. Getting it wrong here fails every sign-in on shape.
    #[test]
    fn the_client_spells_client_data_json_correctly() {
        let kit = kit_with_options(&[("passkeys", on())], BOTH);
        let ts = &kit.file(client::CLIENT_PATH).unwrap().contents;
        assert!(ts.contains("clientDataJSON:"), "{ts}");
        assert!(
            // The wrong spelling appears once, in the comment warning against it.
            // What must not exist is a field written that way.
            !ts.contains("clientDataJson:"),
            "no browser sends that spelling"
        );
    }

    /// Two downloads that differ only by an opt-in must not share a filename.
    #[test]
    fn the_opt_ins_reach_the_archive_name() {
        let plain = kit_with(&[("totp", on())]).archive_name();
        let with_spec = kit_with_options(
            &[("totp", on())],
            KitOptions {
                openapi: true,
                ts_client: false,
            },
        )
        .archive_name();
        let with_both = kit_with_options(&[("totp", on())], BOTH).archive_name();

        assert_ne!(plain, with_spec);
        assert_ne!(with_spec, with_both);
        assert!(
            with_both.contains("openapi") && with_both.contains("client"),
            "{with_both}"
        );
    }

    #[test]
    fn the_readme_explains_what_the_opt_ins_added() {
        let readme = contents(&kit_with_options(&[("totp", on())], BOTH), "README.md");
        assert!(readme.contains("GET /openapi.json"), "{readme}");
        assert!(readme.contains(client::CLIENT_PATH), "{readme}");
    }

    /// Re-enrolment must clear the previous secret before issuing one.
    ///
    /// Without the delete, `register_totp` appends and verification matches
    /// the oldest credential: the QR code just handed over is dead while the
    /// device being replaced keeps working. That shipped once, and the order
    /// matters as much as the call.
    #[test]
    fn totp_enrolment_clears_the_previous_authenticator_first() {
        let main = contents(&kit_with(&[("totp", on())]), "src/main.rs");

        // Matched as calls, not as words: both names appear in the comment
        // above them explaining why the order is what it is, and a substring
        // search would compare the positions of the prose instead.
        let delete = main
            .find(".delete_credentials(")
            .expect("enrolment should clear the previous credential");
        let register = main
            .find(".register_totp(")
            .expect("enrolment should register a new credential");

        assert!(
            delete < register,
            "the old credential must be cleared before the new one is saved"
        );
    }

    #[test]
    fn sqlx_gets_an_async_runtime() {
        let cargo = contents(&kit_with(&[("totp", on())]), "Cargo.toml");
        let line = cargo
            .lines()
            .find(|l| l.starts_with("sqlx"))
            .expect("sqlx dependency");
        assert!(
            line.contains("runtime-"),
            "sqlx without a runtime does not build: {line}"
        );
    }
}
