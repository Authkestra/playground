//! Per-visitor demo sessions (roadmap P1).
//!
//! Every visitor gets an isolated session keyed by an opaque id in an HttpOnly
//! cookie. Toggles mutate only that visitor's config — there is deliberately no
//! global mutable configuration, which would break the moment two people used
//! the site at once.
//!
//! Expiry is belt-and-braces: reads treat a stale session as absent (so an
//! expired session is unreachable even if the sweeper has not run yet), and a
//! background `tokio` interval sweep reclaims the memory and triggers
//! credential cleanup. The service is a long-lived process, so no external cron
//! is needed.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::demo_config::DemoConfig;
use crate::scenario::ScenarioRegistry;

/// Name of the cookie carrying the demo session id.
pub const COOKIE_NAME: &str = "ak_demo";

/// How long a demo session lives.
pub const DEFAULT_TTL_HOURS: i64 = 12;

#[derive(Debug, Clone)]
pub struct DemoSession {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub config: DemoConfig,
}

impl DemoSession {
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }

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

/// Cleans up artefacts a session created outside the session map itself.
///
/// WebAuthn credentials and TOTP secrets are created per demo session (P2), and
/// must not outlive it. The seam exists now so that expiry is already wired
/// when those scenarios land; v0 has no credentials to remove.
pub trait CredentialJanitor: Send + Sync {
    fn purge_session(&self, session_id: Uuid);
}

/// v0 janitor: nothing is stored yet, so there is nothing to purge.
pub struct NoopJanitor;

impl CredentialJanitor for NoopJanitor {
    fn purge_session(&self, session_id: Uuid) {
        tracing::debug!(%session_id, "no credential store wired yet; nothing to purge");
    }
}

/// In-memory store of demo sessions.
///
/// Memory rather than Redis is a deliberate v0 choice — see
/// `docs/decisions/0002-session-store.md`. Sessions do not survive a redeploy.
pub struct DemoSessionStore {
    sessions: RwLock<HashMap<Uuid, DemoSession>>,
    registry: ScenarioRegistry,
    ttl: Duration,
    janitor: Arc<dyn CredentialJanitor>,
}

impl DemoSessionStore {
    pub fn new(
        registry: ScenarioRegistry,
        ttl_hours: i64,
        janitor: Arc<dyn CredentialJanitor>,
    ) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            registry,
            ttl: Duration::hours(ttl_hours),
            janitor,
        }
    }

    pub fn registry(&self) -> &ScenarioRegistry {
        &self.registry
    }

    fn new_session_at(&self, now: DateTime<Utc>) -> DemoSession {
        DemoSession {
            id: Uuid::new_v4(),
            created_at: now,
            expires_at: now + self.ttl,
            config: DemoConfig::defaults_for(&self.registry),
        }
    }

    /// Create a fresh session.
    pub fn create(&self) -> DemoSession {
        self.create_at(Utc::now())
    }

    pub fn create_at(&self, now: DateTime<Utc>) -> DemoSession {
        let session = self.new_session_at(now);
        self.sessions
            .write()
            .expect("session store poisoned")
            .insert(session.id, session.clone());
        tracing::info!(session_id = %session.id, expires_at = %session.expires_at, "demo session created");
        session
    }

    /// Fetch a session, treating an expired one as absent and reclaiming it.
    pub fn get(&self, id: Uuid) -> Option<DemoSession> {
        self.get_at(id, Utc::now())
    }

    pub fn get_at(&self, id: Uuid, now: DateTime<Utc>) -> Option<DemoSession> {
        // Fast path under a read lock.
        {
            let guard = self.sessions.read().expect("session store poisoned");
            match guard.get(&id) {
                None => return None,
                Some(s) if !s.is_expired_at(now) => return Some(s.clone()),
                Some(_) => { /* expired — fall through to remove it */ }
            }
        }
        // Lazy expiry: an expired session is unreachable immediately, whether or
        // not the sweeper has run.
        let removed = self
            .sessions
            .write()
            .expect("session store poisoned")
            .remove(&id)
            .is_some();
        if removed {
            tracing::debug!(session_id = %id, "expired demo session reclaimed on read");
            self.janitor.purge_session(id);
        }
        None
    }

    /// Return the session for `id`, or a brand-new one if it is missing/stale.
    pub fn get_or_create(&self, id: Option<Uuid>) -> DemoSession {
        match id.and_then(|i| self.get(i)) {
            Some(s) => s,
            None => self.create(),
        }
    }

    /// Replace a session's config. Returns `None` if the session has gone.
    pub fn update_config(&self, id: Uuid, config: DemoConfig) -> Option<DemoSession> {
        let now = Utc::now();
        let mut guard = self.sessions.write().expect("session store poisoned");
        let session = guard.get_mut(&id)?;
        if session.is_expired_at(now) {
            guard.remove(&id);
            self.janitor.purge_session(id);
            return None;
        }
        session.config = config;
        Some(session.clone())
    }

    /// Drop a session's state and hand back a clean one, reusing the same id so
    /// the visitor's cookie stays valid.
    pub fn reset(&self, id: Uuid) -> DemoSession {
        let now = Utc::now();
        let mut guard = self.sessions.write().expect("session store poisoned");
        self.janitor.purge_session(id);
        let fresh = DemoSession {
            id,
            created_at: now,
            expires_at: now + self.ttl,
            config: DemoConfig::defaults_for(&self.registry),
        };
        guard.insert(id, fresh.clone());
        tracing::info!(session_id = %id, "demo session reset");
        fresh
    }

    /// Remove every expired session. Returns how many were reclaimed.
    pub fn sweep(&self) -> usize {
        self.sweep_at(Utc::now())
    }

    pub fn sweep_at(&self, now: DateTime<Utc>) -> usize {
        let expired: Vec<Uuid> = {
            let guard = self.sessions.read().expect("session store poisoned");
            guard
                .values()
                .filter(|s| s.is_expired_at(now))
                .map(|s| s.id)
                .collect()
        };
        if expired.is_empty() {
            return 0;
        }
        let mut guard = self.sessions.write().expect("session store poisoned");
        for id in &expired {
            guard.remove(id);
            self.janitor.purge_session(*id);
        }
        tracing::info!(count = expired.len(), "swept expired demo sessions");
        expired.len()
    }

    pub fn len(&self) -> usize {
        self.sessions.read().expect("session store poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::ControlValue;

    fn store() -> DemoSessionStore {
        DemoSessionStore::new(
            ScenarioRegistry::with_builtins(),
            DEFAULT_TTL_HOURS,
            Arc::new(NoopJanitor),
        )
    }

    #[test]
    fn a_new_session_starts_at_scenario_defaults() {
        let s = store();
        let session = s.create();
        assert_eq!(
            session.config.get("dummy_toggle"),
            Some(&ControlValue::Toggle { enabled: false })
        );
        assert!(session.config.active_ids().is_empty());
    }

    #[test]
    fn ttl_is_twelve_hours() {
        let s = store();
        let session = s.create();
        assert_eq!(session.expires_at - session.created_at, Duration::hours(12));
    }

    /// The property that matters most: two visitors must never share config.
    #[test]
    fn two_concurrent_visitors_hold_different_configurations() {
        let s = store();
        let a = s.create();
        let b = s.create();
        assert_ne!(a.id, b.id);

        let mut cfg = a.config.clone();
        cfg.set("dummy_toggle", ControlValue::Toggle { enabled: true });
        s.update_config(a.id, cfg).expect("a still live");

        let a_after = s.get(a.id).expect("a still live");
        let b_after = s.get(b.id).expect("b still live");

        assert!(a_after.config.get("dummy_toggle").unwrap().is_active());
        assert!(
            !b_after.config.get("dummy_toggle").unwrap().is_active(),
            "visitor B's config leaked from visitor A"
        );
    }

    #[test]
    fn an_expired_session_is_unreachable_on_read() {
        let s = store();
        let session = s.create();
        let later = session.expires_at + Duration::seconds(1);
        assert!(s.get_at(session.id, later).is_none());
        // and it was reclaimed rather than left to rot
        assert!(s.is_empty());
    }

    #[test]
    fn sweep_reclaims_only_expired_sessions() {
        let s = store();
        let old = s.create_at(Utc::now() - Duration::hours(13));
        let fresh = s.create();

        let swept = s.sweep();

        assert_eq!(swept, 1);
        assert!(s.get(old.id).is_none());
        assert!(s.get(fresh.id).is_some());
    }

    #[test]
    fn reset_clears_config_but_keeps_the_id() {
        let s = store();
        let session = s.create();
        let mut cfg = session.config.clone();
        cfg.set("dummy_toggle", ControlValue::Toggle { enabled: true });
        s.update_config(session.id, cfg).unwrap();

        let fresh = s.reset(session.id);

        assert_eq!(fresh.id, session.id, "cookie should stay valid");
        assert!(fresh.config.active_ids().is_empty());
    }

    #[test]
    fn get_or_create_makes_a_session_for_an_unknown_id() {
        let s = store();
        let made = s.get_or_create(Some(Uuid::new_v4()));
        assert!(s.get(made.id).is_some());
        assert_eq!(s.len(), 1);
    }
}
