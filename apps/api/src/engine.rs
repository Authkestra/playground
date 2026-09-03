//! Per-session `authkestra` engine construction (roadmap P1).
//!
//! The framework uses a typestate builder where methods only exist once their
//! dependencies are supplied. The useful property for us is that
//! `session_store()` is the only call that *moves* the typestate — `provider()`,
//! `with_totp()` and `with_webauthn()` all return `Self`. So the store is
//! supplied once and the visitor's configuration is folded in afterwards with
//! ordinary conditionals, which is what makes runtime composition tractable.
//!
//! Engines are cached by a fingerprint of the configuration that produced them,
//! so N visitors converging on the same config share one engine instead of
//! rebuilding per request.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};

use authkestra_engine::store::memory::MemoryStore;
use authkestra_engine::{AkWebAppEngine, Engine, SessionConfig, SessionStore};

use crate::demo_config::DemoConfig;

/// OAuth credentials discovered in the environment.
///
/// Absent credentials are not an error: the playground runs fine without them,
/// and the affected scenarios report themselves as not configured rather than
/// failing at boot. Registering the real apps is roadmap P0 and needs a human.
#[derive(Debug, Clone, Default)]
pub struct ProviderCredentials {
    creds: HashMap<String, (String, String)>,
    redirect_base: String,
}

impl ProviderCredentials {
    /// Read `<PROVIDER>_CLIENT_ID` / `<PROVIDER>_CLIENT_SECRET` for each known
    /// provider.
    pub fn from_env() -> Self {
        const PROVIDERS: [&str; 3] = ["github", "google", "discord"];
        let redirect_base = std::env::var("OAUTH_REDIRECT_BASE")
            .unwrap_or_else(|_| "http://localhost:8000".to_string());

        let mut creds = HashMap::new();
        for p in PROVIDERS {
            let id = std::env::var(format!("{}_CLIENT_ID", p.to_uppercase()));
            let secret = std::env::var(format!("{}_CLIENT_SECRET", p.to_uppercase()));
            if let (Ok(id), Ok(secret)) = (id, secret) {
                if !id.is_empty() && !secret.is_empty() {
                    creds.insert(p.to_string(), (id, secret));
                }
            }
        }

        if creds.is_empty() {
            tracing::warn!(
                "no OAuth provider credentials found in the environment; \
                 provider scenarios will report as not configured"
            );
        } else {
            tracing::info!(providers = ?creds.keys().collect::<Vec<_>>(), "OAuth credentials loaded");
        }

        Self {
            creds,
            redirect_base,
        }
    }

    pub fn get(&self, provider: &str) -> Option<&(String, String)> {
        self.creds.get(provider)
    }

    pub fn is_configured(&self, provider: &str) -> bool {
        self.creds.contains_key(provider)
    }

    pub fn redirect_uri(&self, provider: &str) -> String {
        format!("{}/auth/{}/callback", self.redirect_base, provider)
    }
}

/// Stable fingerprint of a configuration, used as the engine cache key.
fn fingerprint(config: &DemoConfig) -> u64 {
    // `DemoConfig` is backed by a BTreeMap, so this serialisation is
    // deterministic for equal configs.
    let canonical = serde_json::to_string(config).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    hasher.finish()
}

/// Builds and caches engines keyed by configuration fingerprint.
pub struct EngineFactory {
    cache: RwLock<HashMap<u64, AkWebAppEngine>>,
    credentials: ProviderCredentials,
    cookie_secure: bool,
    /// One key for the whole process.
    ///
    /// The framework keeps OAuth `state`/`nonce` in encrypted cookies rather
    /// than the database. That only works if the key is stable across engine
    /// rebuilds — a per-engine random key would invalidate every in-flight
    /// OAuth round-trip the moment an engine was recreated.
    state_encryption_key: [u8; 32],
}

impl EngineFactory {
    pub fn new(credentials: ProviderCredentials, cookie_secure: bool) -> Self {
        // Prefer an operator-supplied key so restarts don't break in-flight
        // round-trips; fall back to a per-process random one.
        let state_encryption_key = match std::env::var("OAUTH_STATE_KEY") {
            Ok(v) if v.len() >= 32 => {
                let mut k = [0u8; 32];
                k.copy_from_slice(&v.as_bytes()[..32]);
                k
            }
            _ => {
                tracing::warn!(
                    "OAUTH_STATE_KEY not set (or shorter than 32 bytes); using a random \
                     per-process key. In-flight OAuth state will not survive a restart."
                );
                SessionConfig::default().state_encryption_key
            }
        };

        Self {
            cache: RwLock::new(HashMap::new()),
            credentials,
            cookie_secure,
            state_encryption_key,
        }
    }

    pub fn credentials(&self) -> &ProviderCredentials {
        &self.credentials
    }

    /// Get (or build) the engine for this configuration.
    #[tracing::instrument(skip_all, fields(fingerprint))]
    pub fn engine_for(&self, config: &DemoConfig) -> AkWebAppEngine {
        let key = fingerprint(config);
        tracing::Span::current().record("fingerprint", key);

        if let Some(engine) = self
            .cache
            .read()
            .expect("engine cache poisoned")
            .get(&key)
            .cloned()
        {
            tracing::debug!("engine cache hit");
            return engine;
        }

        let engine = self.build(config);

        let mut guard = self.cache.write().expect("engine cache poisoned");
        // Another thread may have built the same config while we were working;
        // keep whichever landed first so callers share one engine.
        let entry = guard.entry(key).or_insert(engine);
        tracing::info!("engine built and cached");
        entry.clone()
    }

    fn build(&self, config: &DemoConfig) -> AkWebAppEngine {
        let store: Arc<dyn SessionStore> = Arc::new(MemoryStore::default());

        let mut builder = Engine::builder()
            .session_store(store)
            .session_config(SessionConfig {
                secure: self.cookie_secure,
                state_encryption_key: self.state_encryption_key,
                ..Default::default()
            });

        // Fold the visitor's configuration in. `provider()` returns `Self`, so
        // this stays an ordinary loop rather than a typestate puzzle.
        for provider_id in self.selected_providers(config) {
            let Some((client_id, client_secret)) = self.credentials.get(&provider_id) else {
                tracing::debug!(
                    provider = %provider_id,
                    "selected but no credentials in the environment; skipping"
                );
                continue;
            };
            let redirect_uri = self.credentials.redirect_uri(&provider_id);
            builder = match provider_id.as_str() {
                "github" => builder.provider(authkestra_engine::OAuth2Flow::new(
                    authkestra_providers::github::GithubProvider::new(
                        client_id.clone(),
                        client_secret.clone(),
                        redirect_uri,
                    ),
                )),
                "google" => builder.provider(authkestra_engine::OAuth2Flow::new(
                    authkestra_providers::google::GoogleProvider::new(
                        client_id.clone(),
                        client_secret.clone(),
                        redirect_uri,
                    ),
                )),
                "discord" => builder.provider(authkestra_engine::OAuth2Flow::new(
                    authkestra_providers::discord::DiscordProvider::new(
                        client_id.clone(),
                        client_secret.clone(),
                        redirect_uri,
                    ),
                )),
                other => {
                    tracing::debug!(provider = %other, "unknown provider id; skipping");
                    builder
                }
            };
            tracing::info!(provider = %provider_id, "provider attached to engine");
        }

        builder.build()
    }

    /// Provider ids the visitor's config asks for.
    ///
    /// v0's scenarios are placeholders, so this finds nothing real; P2's OAuth
    /// scenario selects `github` / `google` / `discord` through the same path.
    fn selected_providers(&self, config: &DemoConfig) -> Vec<String> {
        use crate::scenario::ControlValue;
        let mut out = Vec::new();
        for value in config.scenarios.values() {
            match value {
                ControlValue::SelectOne {
                    selected: Some(one),
                } => out.push(one.clone()),
                ControlValue::SelectMany { selected } => out.extend(selected.iter().cloned()),
                _ => {}
            }
        }
        out.retain(|p| self.credentials.is_configured(p));
        out.sort();
        out.dedup();
        out
    }

    #[cfg(test)]
    pub fn cache_len(&self) -> usize {
        self.cache.read().expect("engine cache poisoned").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{ControlValue, ScenarioRegistry};

    fn factory() -> EngineFactory {
        EngineFactory::new(ProviderCredentials::default(), false)
    }

    #[test]
    fn equal_configs_share_one_cached_engine() {
        let f = factory();
        let r = ScenarioRegistry::with_builtins();
        let cfg = DemoConfig::defaults_for(&r);

        let _a = f.engine_for(&cfg);
        let _b = f.engine_for(&cfg.clone());

        assert_eq!(f.cache_len(), 1, "identical configs must not rebuild");
    }

    #[test]
    fn different_configs_get_different_engines() {
        let f = factory();
        let r = ScenarioRegistry::with_builtins();
        let a = DemoConfig::defaults_for(&r);
        let mut b = a.clone();
        b.set("dummy_toggle", ControlValue::Toggle { enabled: true });

        let _ = f.engine_for(&a);
        let _ = f.engine_for(&b);

        assert_eq!(f.cache_len(), 2);
    }

    #[test]
    fn fingerprint_is_stable_and_order_independent() {
        let r = ScenarioRegistry::with_builtins();
        let mut a = DemoConfig::defaults_for(&r);
        let mut b = DemoConfig::default();
        // Insert the same pairs in the opposite order.
        for (k, v) in a.scenarios.clone().into_iter().rev() {
            b.set(&k, v);
        }
        assert_eq!(fingerprint(&a), fingerprint(&b));

        a.set("dummy_toggle", ControlValue::Toggle { enabled: true });
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn providers_without_credentials_are_not_selected() {
        let f = factory();
        let mut cfg = DemoConfig::default();
        cfg.set(
            "dummy_provider",
            ControlValue::SelectOne {
                selected: Some("alpha".into()),
            },
        );
        assert!(
            f.selected_providers(&cfg).is_empty(),
            "an unconfigured provider must never be attached"
        );
    }
}
