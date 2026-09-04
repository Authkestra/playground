//! Test fixtures, shared by the integration suites.
//!
//! Compiled into the library rather than gated behind `#[cfg(test)]` because
//! integration tests link this crate as an external dependency and so cannot
//! see its test-only items. It pulls in no extra dependencies.
//!
//! Everything here uses [`MemoryKv`], so the suite runs with no Redis. The
//! Redis backend is covered separately by the tests in [`crate::store`], which
//! skip unless `REDIS_URL` is set.

use std::sync::Arc;
use std::time::Duration;

use crate::credentials::KvCredentialStore;
use crate::engine::{EngineFactory, ProviderCredentials};
use crate::killswitch::KillSwitch;
use crate::routes::AppState;
use crate::scenario::ScenarioRegistry;
use crate::session::{DemoSessionStore, DEFAULT_TTL_HOURS};
use crate::settings::{RelyingParty, Settings, XffPosition};
use crate::store::{KeyValue, MemoryKv};

/// Settings suitable for tests: plain HTTP, localhost relying party.
pub fn test_settings(admin_token: Option<&str>) -> Settings {
    Settings {
        port: 0,
        cookie_secure: false,
        session_ttl_hours: DEFAULT_TTL_HOURS,
        admin_token: admin_token.map(|t| t.to_string()),
        allowed_origins: vec!["http://localhost:3000".to_string()],
        // Tests reach the router directly with no proxy in front, so they
        // identify callers by X-Forwarded-For rather than a trusted header.
        trusted_client_ip_header: None,
        relying_party: RelyingParty {
            id: "localhost".to_string(),
            origin: "http://localhost:3000".to_string(),
            name: "test".to_string(),
        },
        xff_position: XffPosition::Rightmost,
    }
}

/// Application state backed by an in-process store.
pub fn test_state(kill_switch: KillSwitch, admin_token: Option<&str>) -> AppState {
    test_state_with_settings(kill_switch, test_settings(admin_token))
}

/// As [`test_state`], with settings supplied by the caller.
pub fn test_state_with_settings(kill_switch: KillSwitch, settings: Settings) -> AppState {
    let kv: Arc<dyn KeyValue> = Arc::new(MemoryKv::new());
    let ttl = Duration::from_secs((settings.session_ttl_hours.max(1) as u64) * 3600);
    let credentials = KvCredentialStore::new(kv.clone(), ttl);
    let settings = Arc::new(settings);

    AppState {
        sessions: Arc::new(DemoSessionStore::new(
            kv.clone(),
            ScenarioRegistry::with_builtins(),
            settings.session_ttl_hours,
            credentials.clone(),
        )),
        kill_switch: Arc::new(kill_switch),
        engines: Arc::new(EngineFactory::new(ProviderCredentials::default(), false)),
        settings,
        credentials: Arc::new(credentials),
        ceremonies: Arc::new(crate::ceremony::CeremonyStore::new(kv)),
    }
}

/// A router over freshly built test state.
pub fn test_app(kill_switch: KillSwitch, admin_token: Option<&str>) -> axum::Router {
    crate::build_router(test_state(kill_switch, admin_token))
}
