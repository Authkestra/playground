//! Short-lived state for multi-round-trip ceremonies.
//!
//! A WebAuthn ceremony is two requests: the server issues a challenge, the
//! authenticator answers it, and the server must remember what it asked in
//! between. That state is security-relevant — it carries the challenge the
//! signature is verified against — so it lives server-side rather than in a
//! value the client could tamper with.
//!
//! It lives in the shared key-value store with a short TTL:
//!
//! * A ceremony a visitor abandoned (closed the prompt, walked away, hit a
//!   platform timeout) must not linger. Entries expire on a timer, which is
//!   what stops an abort from leaving orphaned state.
//! * Starting a new ceremony of the same kind replaces the old one, so the
//!   visitor who retries is never verified against a stale challenge.
//!
//! Losing this on a redeploy is harmless: the worst case is a visitor mid-prompt
//! seeing the ceremony fail and retrying, which is already the abort path.

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::store::{KeyValue, StoreError};

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

/// Ceremony state, keyed by session and kind.
pub struct CeremonyStore {
    kv: Arc<dyn KeyValue>,
    ttl: Duration,
}

impl CeremonyStore {
    pub fn new(kv: Arc<dyn KeyValue>) -> Self {
        Self {
            kv,
            ttl: Duration::from_secs(CEREMONY_TTL_SECONDS as u64),
        }
    }

    fn key(session_id: Uuid, kind: CeremonyKind) -> String {
        format!("ceremony:{session_id}:{}", kind.as_str())
    }

    /// Record the state for a ceremony, replacing any previous one.
    ///
    /// Replacing matters: a visitor who restarts must not then be verified
    /// against the challenge they abandoned.
    pub async fn put(
        &self,
        session_id: Uuid,
        kind: CeremonyKind,
        state: String,
    ) -> Result<(), StoreError> {
        self.kv
            .set(&Self::key(session_id, kind), &state, self.ttl)
            .await?;
        tracing::debug!(%session_id, kind = kind.as_str(), "ceremony started");
        Ok(())
    }

    /// Take the state for a ceremony, consuming it.
    ///
    /// A challenge must be answerable exactly once, or a replayed response
    /// could be verified twice — so this is an atomic take, not a read.
    pub async fn take(
        &self,
        session_id: Uuid,
        kind: CeremonyKind,
    ) -> Result<Option<String>, StoreError> {
        let taken = self.kv.take(&Self::key(session_id, kind)).await?;
        if taken.is_none() {
            tracing::debug!(%session_id, kind = kind.as_str(), "no live ceremony to complete");
        }
        Ok(taken)
    }

    /// Drop every ceremony belonging to a session.
    pub async fn clear_session(&self, session_id: Uuid) -> Result<u64, StoreError> {
        self.kv
            .delete_with_prefix(&format!("ceremony:{session_id}:"))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryKv;

    fn store() -> CeremonyStore {
        CeremonyStore::new(Arc::new(MemoryKv::new()))
    }

    #[tokio::test]
    async fn a_stored_ceremony_can_be_taken_once() {
        let s = store();
        let id = Uuid::new_v4();
        s.put(id, CeremonyKind::Registration, "state".into())
            .await
            .unwrap();

        assert_eq!(
            s.take(id, CeremonyKind::Registration)
                .await
                .unwrap()
                .as_deref(),
            Some("state")
        );
        // A challenge must be answerable exactly once.
        assert!(s
            .take(id, CeremonyKind::Registration)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn registration_and_authentication_do_not_collide() {
        let s = store();
        let id = Uuid::new_v4();
        s.put(id, CeremonyKind::Registration, "reg".into())
            .await
            .unwrap();
        s.put(id, CeremonyKind::Authentication, "auth".into())
            .await
            .unwrap();

        assert_eq!(
            s.take(id, CeremonyKind::Registration)
                .await
                .unwrap()
                .as_deref(),
            Some("reg")
        );
        assert_eq!(
            s.take(id, CeremonyKind::Authentication)
                .await
                .unwrap()
                .as_deref(),
            Some("auth")
        );
    }

    #[tokio::test]
    async fn restarting_replaces_the_previous_challenge() {
        let s = store();
        let id = Uuid::new_v4();
        s.put(id, CeremonyKind::Registration, "first".into())
            .await
            .unwrap();
        s.put(id, CeremonyKind::Registration, "second".into())
            .await
            .unwrap();

        assert_eq!(
            s.take(id, CeremonyKind::Registration)
                .await
                .unwrap()
                .as_deref(),
            Some("second"),
            "a retry must not be verified against the abandoned challenge"
        );
    }

    /// An abandoned ceremony — the visitor dismissed the prompt and left — must
    /// not stay answerable. The store's TTL handles that.
    #[tokio::test]
    async fn an_abandoned_ceremony_expires() {
        let kv: Arc<dyn KeyValue> = Arc::new(MemoryKv::new());
        let s = CeremonyStore {
            kv,
            ttl: Duration::from_millis(30),
        };
        let id = Uuid::new_v4();
        s.put(id, CeremonyKind::Registration, "state".into())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(s
            .take(id, CeremonyKind::Registration)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn clearing_a_session_drops_its_ceremonies_only() {
        let s = store();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        s.put(a, CeremonyKind::Registration, "a".into())
            .await
            .unwrap();
        s.put(b, CeremonyKind::Registration, "b".into())
            .await
            .unwrap();

        s.clear_session(a).await.unwrap();

        assert!(s
            .take(a, CeremonyKind::Registration)
            .await
            .unwrap()
            .is_none());
        assert!(s
            .take(b, CeremonyKind::Registration)
            .await
            .unwrap()
            .is_some());
    }
}
