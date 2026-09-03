//! Shuttle entrypoint for the playground API.
//!
//! This is the crate's primary binary and the only one carrying
//! `#[shuttle_runtime::main]`, which is how Shuttle picks its target — it looks
//! for the macro across the workspace's binary targets. Keeping it here rather
//! than on a secondary binary removes any ambiguity about which one gets
//! deployed.
//!
//! For local development use `cargo run -p api --bin dev`, which serves the
//! same router over a plain TCP listener without needing the Shuttle CLI.

use api::{build_router, spawn_session_sweeper, state_from_env};

#[shuttle_runtime::main]
async fn axum(
    #[shuttle_runtime::Secrets] secrets: shuttle_runtime::SecretStore,
) -> shuttle_axum::ShuttleAxum {
    // Shuttle delivers secrets through its own store rather than the process
    // environment. Everything in this crate reads configuration from env vars
    // (so local dev, tests and Shuttle all share one code path), so bridge them
    // across before any of that configuration is read.
    for (key, value) in secrets.into_iter() {
        std::env::set_var(key, value);
    }

    // Deployed behind Shuttle's proxy, so the session cookie must be `Secure`
    // unless the operator has deliberately said otherwise.
    if std::env::var("COOKIE_SECURE").is_err() {
        std::env::set_var("COOKIE_SECURE", "true");
    }

    // No `tracing_subscriber` init here: shuttle-runtime installs its own
    // subscriber, and a second one would panic on startup.

    let state = state_from_env();

    tracing::info!(
        scenarios = ?state.sessions.registry().ids(),
        demo_enabled = state.kill_switch.demo_enabled(),
        "starting playground api on shuttle"
    );

    spawn_session_sweeper(state.sessions.clone());

    Ok(build_router(state).into())
}
