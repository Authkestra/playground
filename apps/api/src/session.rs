//! Per-visitor demo sessions.
//!
//! Every visitor gets an isolated session keyed by an opaque id in an HttpOnly
//! cookie. Toggles mutate only that visitor's config — there is deliberately no
//! global mutable configuration, which would break the moment two people used
//! the site at once.
//!
//! Sessions live in the shared key-value store rather than process memory, so
//! any instance can serve any visitor and nothing is lost when an instance goes
//! away. **Expiry is the store's TTL.** This used to need a `tokio::interval`
//! sweep plus expiry-on-read; both are gone, because a background sweeper is
//! precisely the thing that stops working once the process is allowed to sleep.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::credentials::KvCredentialStore;
use crate::demo_config::DemoConfig;
use crate::scenario::ScenarioRegistry;
use crate::store::{self, KeyValue, StoreError};

/// Name of the cookie carrying the demo session id.
pub const COOKIE_NAME: &str = "ak_demo";

/// How long a demo session lives.
pub const DEFAULT_TTL_HOURS: i64 = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemoSession {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub config: DemoConfig,
}

impl DemoSession {
    pub fn view(&self) -> DemoSessionView {
        DemoSessionView {
            id: self.id.to_string(),
            created_at: self.created_at.to_rfc3339(),
            expires_at: self.expires_at.to_rfc3339(),
            config: self.config.clone(),
        }
    }
}

/// The serialised shape handed to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DemoSessionView {
    pub id: String,
    /// RFC3339.
    pub created_at: String,
    /// RFC3339.
    pub expires_at: String,
    pub config: DemoConfig,
}

/// Demo sessions, backed by the shared key-value store.
pub struct DemoSessionStore {
    kv: Arc<dyn KeyValue>,
    registry: ScenarioRegistry,
    ttl: Duration,
    credentials: KvCredentialStore,
}

impl DemoSessionStore {
    pub fn new(
        kv: Arc<dyn KeyValue>,
        registry: ScenarioRegistry,
        ttl_hours: i64,
        credentials: KvCredentialStore,
    ) -> Self {
        Self {
            kv,
            registry,
            ttl: Duration::from_secs((ttl_hours.max(1) as u64) * 3600),
            credentials,
        }
    }

    pub fn registry(&self) -> &ScenarioRegistry {
        &self.registry
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    fn key(id: Uuid) -> String {
        format!("session:{id}")
    }

    fn new_session(&self) -> DemoSession {
        let now = Utc::now();
        DemoSession {
            id: Uuid::new_v4(),
            created_at: now,
            expires_at: now
                + chrono::Duration::from_std(self.ttl)
                    .unwrap_or_else(|_| chrono::Duration::hours(12)),
            config: DemoConfig::defaults_for(&self.registry),
        }
    }

    async fn write(&self, session: &DemoSession) -> Result<(), StoreError> {
        store::set_json(&*self.kv, &Self::key(session.id), session, self.ttl).await
    }

    /// Create a fresh session.
    pub async fn create(&self) -> Result<DemoSession, StoreError> {
        let session = self.new_session();
        self.write(&session).await?;
        tracing::info!(session_id = %session.id, expires_at = %session.expires_at, "demo session created");
        Ok(session)
    }

    /// Fetch a session. An expired one is simply absent — the store dropped it.
    pub async fn get(&self, id: Uuid) -> Result<Option<DemoSession>, StoreError> {
        store::get_json(&*self.kv, &Self::key(id)).await
    }

    /// Return the session for `id`, or a brand-new one if it is missing or gone.
    pub async fn get_or_create(&self, id: Option<Uuid>) -> Result<DemoSession, StoreError> {
        if let Some(id) = id {
            if let Some(session) = self.get(id).await? {
                return Ok(session);
            }
        }
        self.create().await
    }

    /// Replace a session's config. `None` if the session has gone.
    pub async fn update_config(
        &self,
        id: Uuid,
        config: DemoConfig,
    ) -> Result<Option<DemoSession>, StoreError> {
        let Some(mut session) = self.get(id).await? else {
            return Ok(None);
        };
        session.config = config;
        // Writing refreshes the TTL, so an actively used session does not
        // expire out from under a visitor mid-interaction.
        self.write(&session).await?;
        Ok(Some(session))
    }

    /// Drop a session's state and hand back a clean one, reusing the id so the
    /// visitor's cookie stays valid.
    ///
    /// Credentials are removed explicitly here: TTL would get them eventually,
    /// but a visitor who asked for a reset expects their passkey and TOTP
    /// secret gone *now*.
    pub async fn reset(&self, id: Uuid) -> Result<DemoSession, StoreError> {
        if let Err(e) = self.credentials.purge_session(&id.to_string()).await {
            tracing::error!(session_id = %id, error = %e, "failed to purge credentials on reset");
        }

        let now = Utc::now();
        let fresh = DemoSession {
            id,
            created_at: now,
            expires_at: now
                + chrono::Duration::from_std(self.ttl)
                    .unwrap_or_else(|_| chrono::Duration::hours(12)),
            config: DemoConfig::defaults_for(&self.registry),
        };
        self.write(&fresh).await?;
        tracing::info!(session_id = %id, "demo session reset");
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::ControlValue;
    use crate::store::MemoryKv;

    fn store() -> DemoSessionStore {
        let kv: Arc<dyn KeyValue> = Arc::new(MemoryKv::new());
        let credentials = KvCredentialStore::new(kv.clone(), Duration::from_secs(3600));
        DemoSessionStore::new(
            kv,
            ScenarioRegistry::with_builtins(),
            DEFAULT_TTL_HOURS,
            credentials,
        )
    }

    #[tokio::test]
    async fn a_new_session_starts_at_scenario_defaults() {
        let s = store();
        let session = s.create().await.unwrap();
        assert_eq!(
            session.config.get("totp"),
            Some(&ControlValue::Toggle { enabled: false })
        );
        assert!(session.config.active_ids().is_empty());
    }

    #[tokio::test]
    async fn ttl_is_twelve_hours() {
        let s = store();
        let session = s.create().await.unwrap();
        assert_eq!(
            session.expires_at - session.created_at,
            chrono::Duration::hours(12)
        );
    }

    /// The property that matters most: two visitors must never share config.
    #[tokio::test]
    async fn two_concurrent_visitors_hold_different_configurations() {
        let s = store();
        let a = s.create().await.unwrap();
        let b = s.create().await.unwrap();
        assert_ne!(a.id, b.id);

        let mut cfg = a.config.clone();
        cfg.set("totp", ControlValue::Toggle { enabled: true });
        s.update_config(a.id, cfg).await.unwrap().expect("a live");

        let a_after = s.get(a.id).await.unwrap().expect("a live");
        let b_after = s.get(b.id).await.unwrap().expect("b live");

        assert!(a_after.config.get("totp").unwrap().is_active());
        assert!(
            !b_after.config.get("totp").unwrap().is_active(),
            "visitor B's config leaked from visitor A"
        );
    }

    #[tokio::test]
    async fn an_unknown_session_is_absent() {
        let s = store();
        assert!(s.get(Uuid::new_v4()).await.unwrap().is_none());
    }

    /// Expiry is the store's TTL now, so an expired session simply is not there.
    #[tokio::test]
    async fn an_expired_session_is_unreachable() {
        let kv: Arc<dyn KeyValue> = Arc::new(MemoryKv::new());
        let credentials = KvCredentialStore::new(kv.clone(), Duration::from_millis(30));
        // A one-hour TTL rounded from a sub-second duration is not expressible
        // through the hour-based constructor, so write directly.
        let s = DemoSessionStore {
            kv: kv.clone(),
            registry: ScenarioRegistry::with_builtins(),
            ttl: Duration::from_millis(30),
            credentials,
        };
        let session = s.create().await.unwrap();
        assert!(s.get(session.id).await.unwrap().is_some());
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            s.get(session.id).await.unwrap().is_none(),
            "the store's TTL should have dropped it"
        );
    }

    #[tokio::test]
    async fn reset_clears_config_but_keeps_the_id() {
        let s = store();
        let session = s.create().await.unwrap();
        let mut cfg = session.config.clone();
        cfg.set("totp", ControlValue::Toggle { enabled: true });
        s.update_config(session.id, cfg).await.unwrap();

        let fresh = s.reset(session.id).await.unwrap();

        assert_eq!(fresh.id, session.id, "cookie should stay valid");
        assert!(fresh.config.active_ids().is_empty());
    }

    #[tokio::test]
    async fn get_or_create_makes_a_session_for_an_unknown_id() {
        let s = store();
        let made = s.get_or_create(Some(Uuid::new_v4())).await.unwrap();
        assert!(s.get(made.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn updating_refreshes_the_expiry() {
        let s = store();
        let session = s.create().await.unwrap();
        let updated = s
            .update_config(session.id, session.config.clone())
            .await
            .unwrap()
            .unwrap();
        assert!(updated.expires_at >= session.expires_at);
    }
}
