//! Binary entrypoint for the playground API.
//!
//! Plain Axum over a TCP listener, identical locally and in production — the
//! container image runs exactly this. Configuration comes from the environment,
//! so there is one code path for dev, tests and deployment.

use std::net::SocketAddr;

use api::{build_router, state_from_env};

#[tokio::main]
async fn main() {
    // Before anything that might build a TLS client — the engine's provider
    // clients and the Redis connection both do. See the function's docs for
    // why this cannot be settled by Cargo features alone.
    api::install_crypto_provider();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,api=debug,authkestra=debug".into()),
        )
        .init();

    let state = match state_from_env().await {
        Ok(s) => s,
        Err(e) => panic!("failed to initialise application state: {e}"),
    };

    // Snapshot the kill switch state for the startup log. The snapshot reads
    // from the store, establishing the initial state and seeding it if needed.
    let switch = state.kill_switch.snapshot().await;

    tracing::info!(
        scenarios = ?state.sessions.registry().ids(),
        demo_enabled = switch.demo_enabled(),
        "starting playground api"
    );

    let port = state.settings.port;
    let app = build_router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    tracing::info!(%addr, "listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("server error");
}
