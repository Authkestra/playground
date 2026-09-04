//! Write a generated starter kit to disk, for compiling it for real.
//!
//! The generator's unit tests assert the *text* it emits; only compiling the
//! result proves the project is valid. This is what the CI compile matrix
//! drives, and what to reach for when changing a fragment:
//!
//! ```sh
//! cargo run --example emit_kit -- /tmp/kit && (cd /tmp/kit && cargo check)
//! ```
//!
//! With no scenario ids it emits the base project; otherwise it turns on each
//! named toggle scenario.

use api::demo_config::DemoConfig;
use api::kit::StarterKit;
use api::scenario::{ControlValue, ScenarioRegistry};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .expect("usage: emit_kit <out-dir> [scenario-id ...]");
    let wanted: Vec<String> = args.collect();

    let registry = ScenarioRegistry::with_providers(vec![
        "github".to_string(),
        "google".to_string(),
        "discord".to_string(),
    ]);
    let mut config = DemoConfig::defaults_for(&registry);

    for id in &wanted {
        let scenario = registry
            .get(id)
            .unwrap_or_else(|| panic!("no scenario `{id}`"));
        // Turn it on in whatever shape its control takes.
        let value = match scenario.control() {
            api::scenario::ControlShape::Toggle => ControlValue::Toggle { enabled: true },
            api::scenario::ControlShape::SelectOne { options } => ControlValue::SelectOne {
                selected: options.first().map(|o| o.id.clone()),
            },
            api::scenario::ControlShape::SelectMany { options } => ControlValue::SelectMany {
                selected: options.first().map(|o| o.id.clone()).into_iter().collect(),
            },
        };
        config.set(id, value);
    }

    let kit = StarterKit::generate(&config, &registry);
    let root = std::path::Path::new(&out);
    for file in &kit.files {
        let path = root.join(&file.path);
        std::fs::create_dir_all(path.parent().expect("file has a parent")).expect("create dir");
        std::fs::write(&path, &file.contents).expect("write file");
        println!("{}", file.path);
    }
}
