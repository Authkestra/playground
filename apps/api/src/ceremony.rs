//! Short-lived state for multi-round-trip ceremonies.
//!
//! A WebAuthn ceremony is two requests: the server issues a challenge, the
//! authenticator answers it, and the server must remember what it asked in
//! between. That state is security-relevant — it carries the challenge the
//! signature is verified against — so it lives server-side rather than in a
//! value the client could tamper with.
//!
//! It is deliberately in-memory and short-lived:
//!
//! * A ceremony a visitor abandoned (closed the prompt, walked away, hit a
//!   platform timeout) must not linger. Entries expire on a timer, which is
//!   what stops an abort from leaving orphaned state.
//! * Starting a new ceremony of the same kind replaces the old one, so the
//!   visitor who retries is never verified against a stale challenge.
//!
//! Losing this on a redeploy is harmless: the worst case is a visitor mid-prompt
//! seeing the ceremony fail and retrying, which is already the abort path.

use std::collections::HashMap;
use std::sync::RwLock;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// How long a challenge stays answerable. Platform authenticators typically
/// time out well inside this.
pub const CEREMONY_TTL_SECONDS: i64 = 300;

/// Which ceremony a stored challenge belongs to. Registration and
/// authentication are tracked separately so starting one never disturbs the
/// other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CeremonyKind {
    Registration,
    Authentication,
}

impl CeremonyKind {
    fn as_str(self) -> &'static str {
        match self {
            CeremonyKind::Registration => "registration",
            CeremonyKind::Authentication => "authentication",
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    /// The ceremony state, serialised. Kept as JSON so this store does not need
    /// to know about WebAuthn types.
    state: String,
    expires_at: DateTime<Utc>,
}

/// In-memory ceremony states, keyed by session and kind.
#[derive(Default)]
pub struct CeremonyStore {
    entries: RwLock<HashMap<(Uuid, CeremonyKind), Entry>>,
}

impl CeremonyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the state for a ceremony, replacing any previous one.
    pub fn put(&self, session_id: Uuid, kind: CeremonyKind, state: String) {
        self.put_at(session_id, kind, state, Utc::now())
    }

    pub fn put_at(&self, session_id: Uuid, kind: CeremonyKind, state: String, now: DateTime<Utc>) {
        self.entries
            .write()
            .expect("ceremony store poisoned")
            .insert(
                (session_id, kind),
                Entry {
                    state,
                    expires_at: now + Duration::seconds(CEREMONY_TTL_SECONDS),
                },
            );
        tracing::debug!(%session_id, kind = kind.as_str(), "ceremony started");
    }

    /// Take the state for a ceremony, consuming it.
    ///
    /// Consuming rather than reading is deliberate: a challenge must be
    /// answerable exactly once, or a replayed response could be verified twice.
    pub fn take(&self, session_id: Uuid, kind: CeremonyKind) -> Option<String> {
        self.take_at(session_id, kind, Utc::now())
    }

    pub fn take_at(
        &self,
        session_id: Uuid,
        kind: CeremonyKind,
        now: DateTime<Utc>,
    ) -> Option<String> {
        let entry = self
            .entries
            .write()
            .expect("ceremony store poisoned")
            .remove(&(session_id, kind))?;

        if now >= entry.expires_at {
            tracing::debug!(%session_id, kind = kind.as_str(), "ceremony expired before completion");
            return None;
        }
        Some(entry.state)
    }

    /// Drop every ceremony belonging to a session (called when it expires).
    pub fn clear_session(&self, session_id: Uuid) {
        self.entries
            .write()
            .expect("ceremony store poisoned")
            .retain(|(sid, _), _| *sid != session_id);
    }

    /// Remove expired entries. Returns how many were reclaimed.
    pub fn sweep(&self) -> usize {
        self.sweep_at(Utc::now())
    }

    pub fn sweep_at(&self, now: DateTime<Utc>) -> usize {
        let mut guard = self.entries.write().expect("ceremony store poisoned");
        let before = guard.len();
        guard.retain(|_, e| now < e.expires_at);
        before - guard.len()
    }

    pub fn len(&self) -> usize {
        self.entries.read().expect("ceremony store poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_ceremony_can_be_taken_once() {
        let s = CeremonyStore::new();
        let id = Uuid::new_v4();
        s.put(id, CeremonyKind::Registration, "state".into());

        assert_eq!(
            s.take(id, CeremonyKind::Registration).as_deref(),
            Some("state")
        );
        // A challenge must be answerable exactly once.
        assert!(s.take(id, CeremonyKind::Registration).is_none());
    }

    #[test]
    fn registration_and_authentication_do_not_collide() {
        let s = CeremonyStore::new();
        let id = Uuid::new_v4();
        s.put(id, CeremonyKind::Registration, "reg".into());
        s.put(id, CeremonyKind::Authentication, "auth".into());

        assert_eq!(
            s.take(id, CeremonyKind::Registration).as_deref(),
            Some("reg")
        );
        assert_eq!(
            s.take(id, CeremonyKind::Authentication).as_deref(),
            Some("auth")
        );
    }

    #[test]
    fn restarting_replaces_the_previous_challenge() {
        let s = CeremonyStore::new();
        let id = Uuid::new_v4();
        s.put(id, CeremonyKind::Registration, "first".into());
        s.put(id, CeremonyKind::Registration, "second".into());

        assert_eq!(
            s.take(id, CeremonyKind::Registration).as_deref(),
            Some("second"),
            "a retry must not be verified against the abandoned challenge"
        );
    }

    /// An abandoned ceremony — the visitor dismissed the prompt and left — must
    /// not stay answerable.
    #[test]
    fn an_abandoned_ceremony_expires() {
        let s = CeremonyStore::new();
        let id = Uuid::new_v4();
        let now = Utc::now();
        s.put_at(id, CeremonyKind::Registration, "state".into(), now);

        let later = now + Duration::seconds(CEREMONY_TTL_SECONDS + 1);
        assert!(s.take_at(id, CeremonyKind::Registration, later).is_none());
    }

    #[test]
    fn sweep_reclaims_only_expired_entries() {
        let s = CeremonyStore::new();
        let old = Uuid::new_v4();
        let fresh = Uuid::new_v4();
        let now = Utc::now();
        s.put_at(
            old,
            CeremonyKind::Registration,
            "old".into(),
            now - Duration::seconds(CEREMONY_TTL_SECONDS + 10),
        );
        s.put_at(fresh, CeremonyKind::Registration, "fresh".into(), now);

        assert_eq!(s.sweep_at(now), 1);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn clearing_a_session_drops_its_ceremonies_only() {
        let s = CeremonyStore::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        s.put(a, CeremonyKind::Registration, "a".into());
        s.put(b, CeremonyKind::Registration, "b".into());

        s.clear_session(a);

        assert!(s.take(a, CeremonyKind::Registration).is_none());
        assert!(s.take(b, CeremonyKind::Registration).is_some());
    }
}
