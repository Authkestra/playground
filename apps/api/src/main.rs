//! Binary entrypoint for the playground API.

use std::net::SocketAddr;

use api::{build_router, spawn_session_sweeper, state_from_env};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,api=debug,authkestra=debug".into()),
        )
        .init();

    let state = state_from_env();

    tracing::info!(
        scenarios = ?state.sessions.registry().ids(),
        demo_enabled = state.kill_switch.demo_enabled(),
        "starting playground api"
    );

    spawn_session_sweeper(state.sessions.clone());

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
