//! Deployment manifests for the generated project.
//!
//! Same rule as the rest of the kit (`docs/decisions/0005-starter-kit-model.md`):
//! one source of truth, assembled by concatenation. Here that source is
//! `Plan::all_env_vars()` — the same list `.env.example` renders — so a
//! manifest can never declare a variable the generated project does not read,
//! or omit one it does. Every emitted secret is declared required-but-unset,
//! never with a value: `archive.rs`'s
//! `the_archive_never_carries_a_filled_in_secret` guarantee has to survive
//! whatever lands here too.
//!
//! Files are emitted per target, independently, because the four opt-ins are
//! independent (`scenario::DeployTargets`): someone deploying to Fly should
//! not find a `render.yaml` they never asked for.

use super::{GeneratedFile, Plan, DEFAULT_PORT, PROJECT_NAME};
use crate::scenario::KitEnvVar;

/// The Rust toolchain image the Dockerfile builds with.
///
/// Pinned so a download does not silently pick up a newer, untested
/// toolchain. Matches the version the playground's own `apps/api/Dockerfile`
/// builds with, for the same reason that Dockerfile gives: this is the
/// toolchain actually exercised by this codebase's CI.
const RUST_BUILDER_IMAGE: &str = "rust:1.95-bookworm";

/// The manifests implied by this configuration's deploy options, in a stable
/// order so the archive stays byte-identical run to run.
pub fn deploy_files(plan: &Plan) -> Vec<GeneratedFile> {
    let mut out = Vec::new();
    if plan.options.deploy.docker {
        out.push(GeneratedFile {
            path: "Dockerfile".to_string(),
            contents: dockerfile(),
        });
        out.push(GeneratedFile {
            path: ".dockerignore".to_string(),
            contents: dockerignore(),
        });
    }
    if plan.options.deploy.render {
        out.push(GeneratedFile {
            path: "render.yaml".to_string(),
            contents: render_yaml(plan),
        });
    }
    if plan.options.deploy.fly {
        out.push(GeneratedFile {
            path: "fly.toml".to_string(),
            contents: fly_toml(plan),
        });
    }
    if plan.options.deploy.railway {
        out.push(GeneratedFile {
            path: "railway.json".to_string(),
            contents: railway_json(),
        });
    }
    out
}

fn dockerfile() -> String {
    format!(
        r#"# Multi-stage build: the final image carries one binary and a
# certificate bundle, not a Rust toolchain.
#
# Built from this project's own root:
#   docker build -t {PROJECT_NAME} .
#   docker run -p {port}:{port} --env-file .env {PROJECT_NAME}
FROM {RUST_BUILDER_IMAGE} AS builder

# `rustls-aws-lc-rs` (the TLS backend Cargo.toml pins — see the README) compiles
# C and assembly, so the builder needs a C toolchain even though the rest of
# this project is pure Rust.
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake perl pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml ./
COPY src ./src
# No Cargo.lock is generated with this project, so this resolves one on first
# build. Commit the one it produces if you want a pinned, reproducible build.
RUN cargo build --release

FROM debian:bookworm-slim AS runtime

# ca-certificates: passkeys, OAuth and OIDC all make outbound HTTPS calls, and
# without a trust store every one of them fails at the handshake.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 app

COPY --from=builder /app/target/release/{PROJECT_NAME} /usr/local/bin/{PROJECT_NAME}

# `useradd --create-home` already made this directory writable by `app`. It
# matters when this configuration has a credential store: the default
# `DATABASE_URL` is a relative SQLite path, and root's home is not writable by
# anyone else. Mount a volume here on any host, or a redeploy starts every
# enrolled visitor over — the filesystem is not persistent by default on most
# hosts.
WORKDIR /home/app
USER app

EXPOSE {port}
ENV PORT={port}
CMD ["/usr/local/bin/{PROJECT_NAME}"]
"#,
        port = DEFAULT_PORT,
    )
}

/// Mirrors `.gitignore` deliberately, via the same constant, rather than a
/// second list maintained by hand — the two are answering the same question
/// ("what does not belong in what gets shipped") for two different shippers.
fn dockerignore() -> String {
    format!(
        "# Kept in step with .gitignore: none of this belongs in the build\n\
         # context either.\n\
         {}",
        super::GITIGNORE
    )
}

/// One `envVars` entry per configured variable: a value for anything with a
/// usable default, `sync: false` — required, but never filled in — for
/// anything without one. This is the one place Render's blueprint schema
/// distinguishes the two; every other target's equivalent is built the same
/// way from the same list.
fn render_env_vars(vars: &[KitEnvVar]) -> String {
    vars.iter()
        .map(|var| match &var.default {
            // `must_be_supplied_on_a_host` rather than `default.is_none()`:
            // a default of `http://localhost:3000` is not a value this host
            // can use, and writing it here would hand Render a service whose
            // passkey flow fails at the first ceremony.
            Some(value) if !var.must_be_supplied_on_a_host() => format!(
                "      # {}\n      - key: {}\n        value: \"{value}\"",
                var.comment, var.name
            ),
            _ => format!(
                "      # {}\n      - key: {}\n        sync: false",
                var.comment, var.name
            ),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_yaml(plan: &Plan) -> String {
    format!(
        r#"# Render blueprint for {PROJECT_NAME}.
#
# Connect this repo in the Render dashboard ("New +" -> "Blueprint") and Render
# builds the Dockerfile above and redeploys on every push — no CI secrets
# stored here. See the README's "Deploy it" section for the one-click button
# and what to fill in once the service exists.
services:
  - type: web
    name: {PROJECT_NAME}
    runtime: docker
    dockerfilePath: ./Dockerfile
    dockerContext: .
    plan: free
    healthCheckPath: /health
    envVars:
{env_vars}
"#,
        env_vars = render_env_vars(&plan.all_env_vars()),
    )
}

fn fly_toml(plan: &Plan) -> String {
    // Fly's `[env]` block is plaintext committed to the repo, so — unlike
    // Render's `envVars`, which has `sync: false` for exactly this case —
    // there is no way to *declare* a required variable here without also
    // filling it in. Only variables with a usable default get a line under
    // `[env]`; the rest are named in a comment telling you which
    // `fly secrets set` to run instead, never assigned a value.
    let env_lines = plan
        .all_env_vars()
        .iter()
        .filter_map(|var| {
            if var.must_be_supplied_on_a_host() {
                return None;
            }
            var.default
                .as_ref()
                .map(|value| format!("  {} = \"{value}\"", var.name))
        })
        .collect::<Vec<_>>()
        .join("\n");

    let required = plan.required_env_vars();
    let secrets_hint = if required.is_empty() {
        String::new()
    } else {
        let set_cmd = required
            .iter()
            .map(|v| format!("{}=...", v.name))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "\n# Required, and never plaintext here — set each with:\n#   fly secrets set {set_cmd}\n"
        )
    };

    // Variables whose default only holds locally are neither assignable here
    // nor secret, so they fall through both of the cases above. Left at that,
    // they would simply vanish from this file — and a `fly.toml` that never
    // mentions `WEBAUTHN_ORIGIN` is how someone discovers, after deploying,
    // that passkey registration fails for a reason nothing told them about.
    // They cannot be filled in now because the value is the app's own URL,
    // which `fly launch` assigns; so they are written out ready to uncomment.
    let host_bound: Vec<KitEnvVar> = plan
        .all_env_vars()
        .into_iter()
        .filter(|v| v.local_only)
        .collect();
    let host_hint = if host_bound.is_empty() {
        String::new()
    } else {
        let lines = host_bound
            .iter()
            .map(|v| format!("#   {} = \"...\"   # {}", v.name, v.comment))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n# These depend on the URL Fly gives this app, so they cannot be filled in\n\
             # before `fly launch` has run. Once you know the hostname, add them under\n\
             # `[env]` above — an app deployed without them will start and then fail\n\
             # the first ceremony that checks an origin:\n{lines}\n"
        )
    };

    format!(
        r#"# Fly.io configuration for {PROJECT_NAME}.
#
# `fly launch` reads this file and asks you to confirm (or rename) the app and
# region rather than deploying blind — the app name below is only a starting
# guess, since this project has not been pushed anywhere yet.
#
# See https://fly.io/docs/reference/configuration/ for the schema.

app = "{PROJECT_NAME}"

[build]

[env]
{env_lines}
{secrets_hint}{host_hint}
[http_service]
  internal_port = {port}
  force_https = true
  auto_stop_machines = "stop"
  auto_start_machines = true
  min_machines_running = 0

  [[http_service.checks]]
    interval = "30s"
    timeout = "5s"
    grace_period = "10s"
    method = "GET"
    path = "/health"
"#,
        port = DEFAULT_PORT,
    )
}

fn railway_json() -> String {
    // Railway has no analogue of Render's `sync: false` in its config file —
    // service variables live in the dashboard (or `railway variables set`),
    // not in version control — so required variables are named in the
    // README's Railway section instead of invented here as a value this file
    // does not actually have a slot for.
    r#"{
  "$schema": "https://railway.app/railway.schema.json",
  "build": {
    "builder": "DOCKERFILE",
    "dockerfilePath": "Dockerfile"
  },
  "deploy": {
    "healthcheckPath": "/health",
    "healthcheckTimeout": 100,
    "restartPolicyType": "ON_FAILURE",
    "restartPolicyMaxRetries": 3
  }
}
"#
    .to_string()
}

/// The README's "Deploy it" section: what was selected, and what to actually
/// do with each — concrete commands, not just a manifest sitting there.
pub fn readme_section(plan: &Plan) -> String {
    let targets = &plan.options.deploy;
    if !(targets.docker || targets.render || targets.fly || targets.railway) {
        return String::new();
    }

    let mut parts = Vec::new();

    if targets.docker {
        parts.push(format!(
            "### Docker\n\n\
             ```sh\n\
             docker build -t {PROJECT_NAME} .\n\
             docker run -p {port}:{port} --env-file .env {PROJECT_NAME}\n\
             ```\n\n\
             This is the same Dockerfile every other target here builds from — a \
             plain VPS running Docker, or any host that accepts an image, works \
             the same way.",
            port = DEFAULT_PORT
        ));
    }

    if targets.render {
        parts.push(
            "### Render\n\n\
             [![Deploy to Render](https://render.com/images/deploy-to-render-button.svg)](https://render.com/deploy?repo=REPLACE_WITH_YOUR_REPO_URL)\n\n\
             The button needs a real repository URL, which this zip cannot know \
             before you push it somewhere — replace `REPLACE_WITH_YOUR_REPO_URL` \
             once you have, or skip the button and use \"New +\" -> \"Blueprint\" \
             in the Render dashboard instead. Either way it reads `render.yaml`.\n\n\
             `render.yaml` leaves every required variable unset \
             (`sync: false`) rather than guessing a value — set them in the \
             dashboard once the service exists."
                .to_string(),
        );
    }

    if targets.fly {
        parts.push(
            "### Fly.io\n\n\
             Fly has no deploy-from-a-repo-URL button to link to here, so this is \
             `fly launch` instead:\n\n\
             ```sh\n\
             fly launch --no-deploy   # confirm the app name and region in fly.toml\n\
             fly secrets set WEBAUTHN_ORIGIN=... # every variable fly.toml names as required\n\
             fly deploy\n\
             ```"
            .to_string(),
        );
    }

    if targets.railway {
        parts.push(
            "### Railway\n\n\
             [![Deploy on Railway](https://railway.app/button.svg)](https://railway.app/new/template?template=REPLACE_WITH_YOUR_REPO_URL)\n\n\
             Same caveat as Render: `REPLACE_WITH_YOUR_REPO_URL` is a placeholder \
             this zip cannot fill in for you. Replace it once you have pushed \
             this project somewhere, or use \"New Project\" -> \"Deploy from GitHub \
             repo\" in the Railway dashboard.\n\n\
             `railway.json` only carries the build and health-check \
             configuration — Railway has no config-file equivalent of \
             \"required, but unset\", so set the variables below under the \
             service's Variables tab."
                .to_string(),
        );
    }

    let required = plan.required_env_vars();
    let required_list = if required.is_empty() {
        String::new()
    } else {
        format!(
            "\nWhatever the target, these have no default and every one of them \
             leaves it unset rather than guessing:\n\n{}\n",
            required
                .iter()
                .map(|v| format!("- `{}` — {}", v.name, v.comment))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    format!("\n## Deploy it\n\n{}\n{required_list}", parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo_config::DemoConfig;
    use crate::scenario::{ControlValue, DeployTargets, KitOptions, ScenarioRegistry};

    fn kit_with(
        deploy: DeployTargets,
        scenarios: &[(&str, ControlValue)],
    ) -> super::super::StarterKit {
        let registry = ScenarioRegistry::with_providers(vec!["github".to_string()]);
        let mut config = DemoConfig::defaults_for(&registry);
        for (id, value) in scenarios {
            config.set(id, value.clone());
        }
        super::super::StarterKit::generate_with(
            &config,
            &registry,
            KitOptions {
                deploy,
                ..KitOptions::default()
            },
        )
    }

    fn none() -> DeployTargets {
        DeployTargets {
            docker: false,
            render: false,
            fly: false,
            railway: false,
        }
    }

    fn only(pick: impl Fn(&mut DeployTargets)) -> DeployTargets {
        let mut t = none();
        pick(&mut t);
        t
    }

    #[test]
    fn docker_is_on_by_default_and_produces_a_dockerfile_and_dockerignore() {
        let kit = kit_with(DeployTargets::default(), &[]);
        assert!(kit.file("Dockerfile").is_some());
        assert!(kit.file(".dockerignore").is_some());
        // The other three are opt-in, so a default download carries none of
        // them.
        assert!(kit.file("render.yaml").is_none());
        assert!(kit.file("fly.toml").is_none());
        assert!(kit.file("railway.json").is_none());
    }

    #[test]
    fn turning_docker_off_removes_it_and_nothing_else_appears_uninvited() {
        let kit = kit_with(none(), &[]);
        assert!(kit.file("Dockerfile").is_none());
        assert!(kit.file(".dockerignore").is_none());
    }

    #[test]
    fn each_target_emits_only_when_selected() {
        let render_only = kit_with(only(|t| t.render = true), &[]);
        assert!(render_only.file("render.yaml").is_some());
        assert!(render_only.file("fly.toml").is_none());
        assert!(render_only.file("railway.json").is_none());

        let fly_only = kit_with(only(|t| t.fly = true), &[]);
        assert!(fly_only.file("fly.toml").is_some());
        assert!(fly_only.file("render.yaml").is_none());

        let railway_only = kit_with(only(|t| t.railway = true), &[]);
        assert!(railway_only.file("railway.json").is_some());
        assert!(railway_only.file("render.yaml").is_none());
    }

    /// The whole point of building manifests from `Plan` rather than from a
    /// guess: a scenario that needs `WEBAUTHN_ORIGIN` gets it declared on
    /// every host that has somewhere to declare it.
    #[test]
    fn env_vars_propagate_from_the_plan_to_every_manifest() {
        let kit = kit_with(
            DeployTargets {
                docker: true,
                render: true,
                fly: true,
                railway: true,
            },
            &[("passkeys", ControlValue::Toggle { enabled: true })],
        );

        let render = kit.file("render.yaml").unwrap().contents.clone();
        assert!(render.contains("WEBAUTHN_ORIGIN"), "{render}");
        assert!(render.contains("WEBAUTHN_RP_ID"), "{render}");

        let fly = kit.file("fly.toml").unwrap().contents.clone();
        assert!(fly.contains("WEBAUTHN_ORIGIN"), "{fly}");

        // Railway's manifest has no slot for env vars at all (see
        // `railway_json`'s comment), so the requirement lands in the README
        // instead — which is where it must actually appear.
        let readme = kit.file("README.md").unwrap().contents.clone();
        assert!(readme.contains("WEBAUTHN_ORIGIN"), "{readme}");
    }

    /// The acceptance criterion from `archive.rs`, re-checked here because a
    /// deployment manifest is exactly the kind of file that would be tempting
    /// to fill in "for convenience".
    #[test]
    fn no_manifest_ever_carries_a_filled_in_secret() {
        let kit = kit_with(
            DeployTargets {
                docker: true,
                render: true,
                fly: true,
                railway: true,
            },
            &[(
                "oauth",
                ControlValue::SelectMany {
                    selected: vec!["github".to_string()],
                },
            )],
        );

        // Render is the one format with a way to *declare* a required
        // variable at all, and it must do so as `sync: false` — required, but
        // never filled in — never paired with a `value:` line.
        let render = kit.file("render.yaml").unwrap().contents.clone();
        assert!(
            render.contains("- key: GITHUB_CLIENT_SECRET\n        sync: false"),
            "{render}"
        );
        assert!(
            !render.contains("key: GITHUB_CLIENT_SECRET\n        value:"),
            "{render}"
        );

        // Fly's `[env]` is plaintext with no "declared but unset" syntax, so
        // the only safe move is to leave the variable out of it entirely —
        // never a real `GITHUB_CLIENT_SECRET = "..."` assignment. The comment
        // reminding you to run `fly secrets set GITHUB_CLIENT_SECRET=...` is
        // fine — the ellipsis there is prose, not this file assigning a
        // value — so this checks for the assignment specifically, not for
        // the name appearing at all.
        let fly = kit.file("fly.toml").unwrap().contents.clone();
        assert!(
            !fly.contains("GITHUB_CLIENT_SECRET = \""),
            "fly.toml assigns a value to a required secret: {fly}"
        );

        // Railway and the Dockerfile have no per-scenario env-var syntax at
        // all, so a secret appearing in either would mean one leaked in by
        // some other path.
        for path in ["railway.json", "Dockerfile"] {
            let contents = kit.file(path).unwrap().contents.clone();
            assert!(
                !contents.contains("GITHUB_CLIENT_SECRET"),
                "{path} should not mention this variable at all: {contents}"
            );
        }
    }

    #[test]
    fn the_port_matches_what_main_rs_binds() {
        let kit = kit_with(
            DeployTargets {
                docker: true,
                render: true,
                fly: true,
                railway: false,
            },
            &[],
        );
        let main = kit.file("src/main.rs").unwrap().contents.clone();
        assert!(
            main.contains(&format!("unwrap_or({DEFAULT_PORT})")),
            "{main}"
        );

        let dockerfile = kit.file("Dockerfile").unwrap().contents.clone();
        assert!(
            dockerfile.contains(&format!("EXPOSE {DEFAULT_PORT}")),
            "{dockerfile}"
        );

        let render = kit.file("render.yaml").unwrap().contents.clone();
        assert!(
            render.contains(&format!("value: \"{DEFAULT_PORT}\"")),
            "{render}"
        );

        let fly = kit.file("fly.toml").unwrap().contents.clone();
        assert!(
            fly.contains(&format!("internal_port = {DEFAULT_PORT}")),
            "{fly}"
        );
    }

    #[test]
    fn the_same_configuration_generates_identical_deploy_files() {
        let a = kit_with(
            DeployTargets::default(),
            &[("totp", ControlValue::Toggle { enabled: true })],
        );
        let b = kit_with(
            DeployTargets::default(),
            &[("totp", ControlValue::Toggle { enabled: true })],
        );
        assert_eq!(a.files, b.files);
    }

    #[test]
    fn the_readme_lists_next_steps_for_every_selected_target() {
        let kit = kit_with(
            DeployTargets {
                docker: true,
                render: true,
                fly: true,
                railway: true,
            },
            &[],
        );
        let readme = kit.file("README.md").unwrap().contents.clone();
        assert!(readme.contains("## Deploy it"), "{readme}");
        assert!(readme.contains("### Docker"), "{readme}");
        assert!(readme.contains("### Render"), "{readme}");
        assert!(readme.contains("### Fly.io"), "{readme}");
        assert!(readme.contains("### Railway"), "{readme}");
        // Fly gets `fly launch`, not an invented button — it has none.
        assert!(readme.contains("fly launch"), "{readme}");
        assert!(!readme.contains("fly.io/deploy"), "{readme}");
    }

    #[test]
    fn no_deploy_section_when_nothing_was_selected() {
        let kit = kit_with(none(), &[]);
        let readme = kit.file("README.md").unwrap().contents.clone();
        assert!(!readme.contains("## Deploy it"), "{readme}");
    }
}

#[cfg(test)]
mod host_bound_env_tests {
    use crate::kit::matrix::ci_registry;
    use crate::scenario::{DeployTargets, KitOptions};

    fn kit_with(spec: &str, deploy: DeployTargets) -> crate::kit::StarterKit {
        let registry = ci_registry();
        let config = crate::kit::matrix::config_from_spec(spec, &registry).expect("spec parses");
        crate::kit::StarterKit::generate_with(
            &config,
            &registry,
            KitOptions {
                openapi: false,
                ts_client: false,
                deploy,
            },
        )
    }

    fn file<'a>(kit: &'a crate::kit::StarterKit, path: &str) -> &'a str {
        &kit.files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} should have been emitted"))
            .contents
    }

    fn only(target: fn(&mut DeployTargets)) -> DeployTargets {
        let mut t = DeployTargets {
            docker: false,
            render: false,
            fly: false,
            railway: false,
        };
        target(&mut t);
        t
    }

    /// The bug this guards: `WEBAUTHN_ORIGIN` defaults to `http://localhost:3000`
    /// because that is right for `cargo run`. Copied into a deploy manifest it
    /// produces a service whose passkey registration can never succeed — the
    /// origin the browser reports will never be localhost — while looking
    /// entirely configured. Unset is recoverable; wrong-and-confident is not.
    #[test]
    fn render_never_assigns_a_localhost_default_as_a_value() {
        let kit = kit_with("passkeys", only(|t| t.render = true));
        let render = file(&kit, "render.yaml");

        assert!(
            !render.contains("localhost"),
            "a deploy manifest must not carry a localhost value:\n{render}"
        );
        assert!(
            render.contains("- key: WEBAUTHN_ORIGIN\n        sync: false"),
            "WEBAUTHN_ORIGIN should be declared required-but-unset:\n{render}"
        );
    }

    #[test]
    fn fly_neither_assigns_nor_silently_drops_a_host_bound_variable() {
        let kit = kit_with("passkeys", only(|t| t.fly = true));
        let fly = file(&kit, "fly.toml");

        // Not assigned...
        assert!(
            !fly.contains("  WEBAUTHN_ORIGIN = \"http://localhost:3000\""),
            "fly.toml must not assign the local default:\n{fly}"
        );
        // ...but not vanished either. Absent-and-unmentioned is how someone
        // finds out after deploying.
        assert!(
            fly.contains("WEBAUTHN_ORIGIN"),
            "fly.toml must still name the variable:\n{fly}"
        );
    }

    #[test]
    fn oauth_redirect_base_is_host_bound_too() {
        let kit = kit_with("oauth=github", only(|t| t.render = true));
        let render = file(&kit, "render.yaml");
        assert!(
            render.contains("- key: OAUTH_REDIRECT_BASE\n        sync: false"),
            "OAUTH_REDIRECT_BASE depends on the deployed URL:\n{render}"
        );
    }

    /// The counterpart: a default that holds everywhere must still be written,
    /// or this fix would have turned every manifest into a list of blanks.
    #[test]
    fn a_universally_valid_default_is_still_assigned() {
        let kit = kit_with("passkeys", only(|t| t.render = true));
        let render = file(&kit, "render.yaml");
        assert!(
            render.contains("- key: RUST_LOG\n        value:"),
            "RUST_LOG's default is correct on any host:\n{render}"
        );
    }

    /// `.env.example` is for local development, where localhost is the right
    /// answer — so this must NOT have been fixed by blanking the default.
    #[test]
    fn the_env_example_keeps_the_local_default() {
        let kit = kit_with("passkeys", DeployTargets::default());
        assert!(
            file(&kit, ".env.example").contains("WEBAUTHN_ORIGIN=http://localhost:3000"),
            ".env.example should still be ready to run locally"
        );
    }

    #[test]
    fn a_config_needing_no_host_bound_variable_gets_no_hint() {
        // Nothing to warn about should mean no warning — an unconditional
        // block teaches people to skim past it.
        let kit = kit_with("", only(|t| t.fly = true));
        let fly = file(&kit, "fly.toml");
        assert!(
            !fly.contains("depend on the URL Fly gives"),
            "the hint should only appear when it applies:\n{fly}"
        );
    }
}
