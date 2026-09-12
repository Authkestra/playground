//! Emit a generated starter project to disk.
//!
//! Exists so CI can build what visitors download, using the same generator the
//! service runs rather than a reimplementation of it. The combination list
//! lives in `kit::matrix` and is read from here, so the workflow never repeats
//! it in YAML and cannot drift from the code.
//!
//! ```sh
//! generate-kit --list                    # names, one per line
//! generate-kit --list-json --exhaustive  # a GitHub Actions matrix
//! generate-kit --name all --out ./out
//! generate-kit --spec "passkeys,oauth=github" --out ./out
//! generate-kit --spec passkeys --deploy docker,render --out ./out
//! ```

use std::process::ExitCode;

use api::kit::matrix;
use api::kit::StarterKit;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // Plain stderr, not tracing: this is a CLI, and the message is the
            // whole point of the process.
            eprintln!("generate-kit: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Read `--deploy docker,render` into a target set.
///
/// Absent means none, which is the opposite of the HTTP route's default. That
/// asymmetry is deliberate: a visitor downloading a project wants a Dockerfile
/// without having to ask, whereas a CI leg that did not ask for manifests
/// should not have to explain why its output contains them.
fn deploy_targets(args: &[String]) -> api::scenario::DeployTargets {
    let mut targets = api::scenario::DeployTargets {
        docker: false,
        render: false,
        fly: false,
        railway: false,
    };

    let Some(list) = args
        .iter()
        .position(|a| a == "--deploy")
        .and_then(|i| args.get(i + 1))
    else {
        return targets;
    };

    for name in list.split(',').map(str::trim) {
        match name.to_ascii_lowercase().as_str() {
            "docker" => targets.docker = true,
            "render" => targets.render = true,
            "fly" | "fly.io" | "flyio" => targets.fly = true,
            "railway" => targets.railway = true,
            other if !other.is_empty() => {
                eprintln!("generate-kit: ignoring unknown deploy target `{other}`");
            }
            _ => {}
        }
    }

    targets
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }

    let exhaustive = args.iter().any(|a| a == "--exhaustive");
    let set = if exhaustive {
        matrix::exhaustive()
    } else {
        matrix::representative()
    };

    if args.iter().any(|a| a == "--list") {
        for c in set {
            println!("{}", c.name);
        }
        return Ok(());
    }

    if args.iter().any(|a| a == "--list-json") {
        let entries: Vec<String> = set
            .iter()
            .map(|c| format!(r#"{{"name":"{}","spec":"{}"}}"#, c.name, c.spec))
            .collect();
        println!("[{}]", entries.join(","));
        return Ok(());
    }

    let name = value_of(&args, "--name");
    let named_lookup = name.clone();
    let spec = value_of(&args, "--spec");
    let out = value_of(&args, "--out").ok_or("--out is required")?;

    let spec = match (name, spec) {
        (Some(_), Some(_)) => return Err("pass --name or --spec, not both".to_string()),
        (Some(name), None) => set
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| {
                let known: Vec<&str> = set.iter().map(|c| c.name.as_str()).collect();
                format!("no combination named `{name}`. Known: {}", known.join(", "))
            })?
            .spec
            .clone(),
        (None, Some(spec)) => spec,
        (None, None) => return Err("pass --name or --spec".to_string()),
    };

    // A named combination carries its own opt-ins, so CI can name one leg
    // rather than repeating flags in YAML. Explicit flags still win.
    let named = named_lookup
        .as_ref()
        .and_then(|n| set.iter().find(|c| &c.name == n))
        .map(|c| c.options)
        .unwrap_or_default();

    let options = api::scenario::KitOptions {
        openapi: named.openapi || args.iter().any(|a| a == "--openapi"),
        ts_client: named.ts_client || args.iter().any(|a| a == "--ts-client"),
        // None by default: this binary's main job is compiling the generated
        // Rust project in CI, and a manifest is not Rust. `--deploy` opts in,
        // so the manifests can be generated for inspection and so a workflow
        // that wants to build the Dockerfile can ask for one — without every
        // matrix leg carrying files `cargo check` will never look at.
        deploy: deploy_targets(&args),
    };

    let registry = matrix::ci_registry();
    let config = matrix::config_from_spec(&spec, &registry)?;
    let kit = StarterKit::generate_with(&config, &registry, options);

    let root = std::path::Path::new(&out);
    for file in &kit.files {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, &file.contents)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }

    println!(
        "generated {} file(s) for `{}` into {}",
        kit.files.len(),
        if spec.is_empty() { "base" } else { &spec },
        root.display()
    );
    Ok(())
}

fn value_of(args: &[String], flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).cloned()
}

const USAGE: &str = "\
Emit a generated starter project to disk.

  --list                 print combination names, one per line
  --list-json            print the set as JSON, for a CI matrix
  --exhaustive           use the full product rather than the PR set
  --name <name>          generate a named combination
  --spec <spec>          generate an ad-hoc one, e.g. \"passkeys,oauth=github\"
  --out <dir>            where to write it (required to generate)
  --openapi              annotate the handlers and serve an OpenAPI document
  --ts-client            emit a typed TypeScript client for the ceremonies\n  --deploy <list>        deployment manifests to emit: docker,render,fly,railway";
