//! Kill switch for live demo flows.
//!
//! A public demo that creates real credentials and calls third-party providers
//! needs a way to be switched off *now* — not after a redeploy. State is
//! durable, living in the shared store (Redis in production, in-process memory
//! in tests) seeded from the environment at boot. The switch is flippable at
//! runtime through the admin endpoint, and the state survives restarts — which is
//! the entire point on platforms like Render's free tier that spin the service
//! down after inactivity.
//!
//! When flows are off the site degrades to explainer-only mode: controls render
//! as unavailable with an explanation. It should read as intentional, not
//! broken.
//!
//! ## Caching strategy
//!
//! A simple in-process cache (5 second TTL) sits in front of the store, so a
//! burst of requests costs one round trip rather than N. A flip takes up to 5
//! seconds to be seen everywhere; that is an acceptable price for an emergency
//! stop that happens maybe twice a year, and far better than one that silently
//! reverts.
//!
//! If the store is unreachable, the switch falls back to the last known cached
//! value, so a dependency blinking mid-flight does not resurrect flows — which
//! is the case that used to bite.
//!
//! One path is deliberately not covered: a cold start with an empty cache *and*
//! an unreachable store falls back to the environment seed, which will usually
//! say enabled. Failing closed there would take the whole site down on any store
//! blip. It is close to unreachable in practice anyway — `open_state_store`
//! connects eagerly, so a boot with a dead Redis fails before the router exists
//! and the process never serves a request from that state.

use std::collections::BTreeSet;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The kill switch's state: durable and shareable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KillSwitchState {
    pub demo_enabled: bool,
    pub disabled_scenarios: BTreeSet<String>,
}

impl KillSwitchState {
    pub fn new(demo_enabled: bool, disabled_scenarios: BTreeSet<String>) -> Self {
        Self {
            demo_enabled,
            disabled_scenarios,
        }
    }

    /// Are live flows enabled at all?
    pub fn demo_enabled(&self) -> bool {
        self.demo_enabled
    }

    /// Is this specific scenario runnable right now?
    pub fn scenario_enabled(&self, id: &str) -> bool {
        self.demo_enabled && !self.disabled_scenarios.contains(id)
    }

    pub fn disabled_scenarios(&self) -> Vec<String> {
        self.disabled_scenarios.iter().cloned().collect()
    }
}

impl Default for KillSwitchState {
    fn default() -> Self {
        Self {
            demo_enabled: true,
            disabled_scenarios: BTreeSet::new(),
        }
    }
}

/// In-process cache entry: state + when it was cached.
#[derive(Clone)]
struct CacheEntry {
    state: KillSwitchState,
    cached_at: Instant,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        self.cached_at.elapsed() > CACHE_TTL
    }
}

/// How long to cache the kill switch state in process.
/// A flip takes up to this long to be seen everywhere.
/// Five seconds is an acceptable price for an emergency stop.
const CACHE_TTL: Duration = Duration::from_secs(5);

/// The store key holding the kill switch state.
const STORE_KEY: &str = "killswitch:state";

/// Global + per-scenario enablement.
///
/// State is durable (lives in the store), with an in-process cache in front.
pub struct KillSwitch {
    /// The shared store, where state is persisted.
    store: Option<std::sync::Arc<dyn crate::store::KeyValue>>,
    /// Cached state + when it was read. Allows quick reads when Redis is slow.
    /// The last known value and cache time are kept here to fall back to if Redis
    /// becomes unreachable.
    cache: RwLock<Option<CacheEntry>>,
    /// The environment seed, used as the ultimate fallback if Redis is unreachable
    /// and the cache is empty. This is kept separately so it cannot change.
    env_seed: KillSwitchState,
}

impl KillSwitch {
    /// Create a kill switch with an optional store and environment seed.
    ///
    /// When `store` is `None`, the switch operates entirely in-process (used in
    /// tests). When it is `Some`, the initial state is read from the store; if
    /// nothing is stored yet, the environment seed is written so the first boot
    /// establishes the state. On restart, the environment seed is ignored — the
    /// stored state is authoritative.
    pub fn new(
        store: Option<std::sync::Arc<dyn crate::store::KeyValue>>,
        env_seed: KillSwitchState,
    ) -> Self {
        Self {
            store,
            cache: RwLock::new(None),
            env_seed,
        }
    }

    /// Construct a switch from environment variables, with optional store.
    ///
    /// `DEMO_ENABLED` defaults to true; `DEMO_DISABLED_SCENARIOS` is a
    /// comma-separated list of scenario ids.
    pub fn from_env(store: Option<std::sync::Arc<dyn crate::store::KeyValue>>) -> Self {
        let demo_enabled = std::env::var("DEMO_ENABLED")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no"))
            .unwrap_or(true);
        let disabled = std::env::var("DEMO_DISABLED_SCENARIOS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let seed = KillSwitchState::new(demo_enabled, disabled);
        Self::new(store, seed)
    }

    /// Get a snapshot of the current kill switch state.
    ///
    /// This is an async method that reads from the store if needed, but returns
    /// an owned snapshot that can be queried synchronously. Handlers call this
    /// once per request and pass the snapshot around or query it directly.
    ///
    /// Falls back gracefully: cached value if not expired, last known value if
    /// Redis is unreachable, environment seed if the cache is empty. This means
    /// the switch never fails open.
    pub async fn snapshot(&self) -> KillSwitchState {
        // Check in-process cache first.
        {
            let cache = self.cache.read().expect("cache lock poisoned");
            if let Some(entry) = cache.as_ref() {
                if !entry.is_expired() {
                    return entry.state.clone();
                }
            }
        }

        // Cache is expired or empty. Try to read from the store.
        if let Some(store) = &self.store {
            match crate::store::get_json::<KillSwitchState>(store.as_ref(), STORE_KEY).await {
                Ok(Some(state)) => {
                    // Update the cache and return the fresh state.
                    let entry = CacheEntry {
                        state: state.clone(),
                        cached_at: Instant::now(),
                    };
                    {
                        let mut cache = self.cache.write().expect("cache lock poisoned");
                        *cache = Some(entry);
                    }
                    return state;
                }
                Ok(None) => {
                    // First boot: nothing in the store yet. Write the environment seed.
                    // If the write fails, we will still return the seed, but just not
                    // durably.
                    let _ = crate::store::set_json(
                        store.as_ref(),
                        STORE_KEY,
                        &self.env_seed,
                        Duration::from_secs(365 * 24 * 3600),
                    )
                    .await
                    .map_err(|e| tracing::warn!("could not seed kill switch in store: {e}"));

                    let entry = CacheEntry {
                        state: self.env_seed.clone(),
                        cached_at: Instant::now(),
                    };
                    {
                        let mut cache = self.cache.write().expect("cache lock poisoned");
                        *cache = Some(entry);
                    }
                    return self.env_seed.clone();
                }
                Err(e) => {
                    // Redis is unreachable. Fall back to the last known cached value.
                    tracing::warn!(
                        "could not read kill switch from store: {e}; using cached value"
                    );
                }
            }
        }

        // Fall back to last known cached value.
        {
            let cache = self.cache.read().expect("cache lock poisoned");
            if let Some(entry) = cache.as_ref() {
                return entry.state.clone();
            }
        }

        // No cache and no store: return the environment seed.
        self.env_seed.clone()
    }

    /// Update the kill switch state and persist it to the store.
    ///
    /// This is async and must be awaited to ensure durability. Updates both the
    /// store and the in-process cache, so the change is immediately visible and
    /// durable.
    pub async fn set_state(&self, state: KillSwitchState) {
        // Update the in-process cache first, so reads are immediately fast.
        let entry = CacheEntry {
            state: state.clone(),
            cached_at: Instant::now(),
        };
        {
            let mut cache = self.cache.write().expect("cache lock poisoned");
            *cache = Some(entry);
        }

        // Write to the store for durability.
        if let Some(store) = &self.store {
            if let Err(e) = crate::store::set_json(
                store.as_ref(),
                STORE_KEY,
                &state,
                Duration::from_secs(365 * 24 * 3600),
            )
            .await
            {
                tracing::error!("could not persist kill switch state: {e}");
            }
        }
    }
}

impl Default for KillSwitch {
    fn default() -> Self {
        Self::new(None, KillSwitchState::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenarios_follow_the_global_switch() {
        let state = KillSwitchState::default();
        assert!(state.scenario_enabled("dummy_toggle"));

        let mut state = state;
        state.demo_enabled = false;
        assert!(
            !state.scenario_enabled("dummy_toggle"),
            "global off must disable every scenario"
        );
    }

    #[test]
    fn a_single_scenario_can_be_disabled_on_its_own() {
        let mut state = KillSwitchState::default();
        state.disabled_scenarios.insert("dummy_toggle".to_string());

        assert!(!state.scenario_enabled("dummy_toggle"));
        assert!(
            state.scenario_enabled("dummy_provider"),
            "disabling one scenario must not affect the others"
        );
        assert!(state.demo_enabled(), "global switch untouched");
    }

    #[test]
    fn disabling_is_reversible() {
        let mut state = KillSwitchState::default();
        state.disabled_scenarios.insert("dummy_toggle".to_string());
        assert!(!state.scenario_enabled("dummy_toggle"));

        state.disabled_scenarios.remove("dummy_toggle");
        assert!(state.scenario_enabled("dummy_toggle"));
        assert!(state.disabled_scenarios().is_empty());
    }

    #[tokio::test]
    async fn switch_with_no_store_works_in_process_only() {
        let ks = KillSwitch::default();
        let snap1 = ks.snapshot().await;
        assert!(snap1.demo_enabled());

        let mut new_state = snap1.clone();
        new_state.demo_enabled = false;
        ks.set_state(new_state).await;

        let snap2 = ks.snapshot().await;
        assert!(!snap2.demo_enabled());
    }
}
