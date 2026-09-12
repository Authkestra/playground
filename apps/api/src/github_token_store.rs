//! Short-lived, server-side storage for a visitor's GitHub push token (#40).
//!
//! Mirrors `credentials.rs`'s reasoning: this belongs to a demo session, not a
//! person, so it is stored under the session's own key in the shared
//! key-value store and never touches the browser. It is deliberately **not**
//! folded into `KvCredentialStore` — that store exists for the framework's
//! `CredentialStore` trait (TOTP secrets, passkeys), which this is not; a
//! GitHub access token is a bearer credential for a *different* service, kept
//! only long enough for the visitor to click one button.
//!
//! ## Why fifteen minutes
//!
//! The token only has to survive the round trip from "I clicked Connect" to
//! "I clicked Push" — one visitor, one button, one browser tab. Anything
//! longer is exposure with no corresponding benefit; anything shorter risks
//! expiring while someone is still reading the repository-name field.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::store::{self, KeyValue, StoreError};

/// How long a connected GitHub token is kept before it must be re-earned by
/// going through `/api/github/connect` again.
pub const TOKEN_TTL: Duration = Duration::from_secs(15 * 60);

/// The token, wrapped so nothing prints it by accident.
///
/// `Serialize`/`Deserialize` are derived — the wire format is exactly the
/// token — but `Debug` is written by hand so a stray `{:?}` of this value (in
/// a log line, a panic message, a test failure) can never show it.
#[derive(Clone, Serialize, Deserialize)]
struct StoredToken {
    access_token: String,
}

impl std::fmt::Debug for StoredToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredToken")
            .field("access_token", &"<redacted>")
            .finish()
    }
}

/// The store itself: a thin, TTL'd wrapper over the shared key-value backend.
#[derive(Clone)]
pub struct GithubTokenStore {
    kv: Arc<dyn KeyValue>,
    ttl: Duration,
}

impl GithubTokenStore {
    pub fn new(kv: Arc<dyn KeyValue>, ttl: Duration) -> Self {
        Self { kv, ttl }
    }

    fn key(session_id: Uuid) -> String {
        format!("github_push_token:{session_id}")
    }

    /// Record a token for a session, superseding whatever was stored before
    /// (a visitor who reconnects gets the newer token, not a merge of both).
    #[tracing::instrument(skip_all, fields(%session_id))]
    pub async fn store(&self, session_id: Uuid, access_token: &str) -> Result<(), StoreError> {
        let value = StoredToken {
            access_token: access_token.to_string(),
        };
        store::set_json(&*self.kv, &Self::key(session_id), &value, self.ttl).await
    }

    /// The token for a session, if one is on file and has not expired.
    #[tracing::instrument(skip_all, fields(%session_id))]
    pub async fn load(&self, session_id: Uuid) -> Result<Option<String>, StoreError> {
        let stored: Option<StoredToken> = store::get_json(&*self.kv, &Self::key(session_id)).await?;
        Ok(stored.map(|t| t.access_token))
    }

    /// Drop a session's token. Called unconditionally once a push attempt has
    /// finished, whatever the outcome — the token has done its one job either
    /// way, and there is no reason to let it sit until the TTL catches up.
    #[tracing::instrument(skip_all, fields(%session_id))]
    pub async fn clear(&self, session_id: Uuid) -> Result<(), StoreError> {
        self.kv.delete(&Self::key(session_id)).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryKv;

    fn store() -> GithubTokenStore {
        GithubTokenStore::new(Arc::new(MemoryKv::new()), Duration::from_secs(3600))
    }

    #[tokio::test]
    async fn a_stored_token_can_be_loaded_back() {
        let s = store();
        let id = Uuid::new_v4();
        s.store(id, "the-token").await.unwrap();
        assert_eq!(s.load(id).await.unwrap().as_deref(), Some("the-token"));
    }

    #[tokio::test]
    async fn an_unconnected_session_has_no_token() {
        let s = store();
        assert_eq!(s.load(Uuid::new_v4()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn clearing_removes_it() {
        let s = store();
        let id = Uuid::new_v4();
        s.store(id, "tok").await.unwrap();
        s.clear(id).await.unwrap();
        assert_eq!(s.load(id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn reconnecting_replaces_the_previous_token() {
        let s = store();
        let id = Uuid::new_v4();
        s.store(id, "first").await.unwrap();
        s.store(id, "second").await.unwrap();
        assert_eq!(s.load(id).await.unwrap().as_deref(), Some("second"));
    }

    #[tokio::test]
    async fn one_sessions_token_is_invisible_to_another() {
        let s = store();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        s.store(a, "a-token").await.unwrap();
        assert_eq!(s.load(b).await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_expired_token_is_unreachable() {
        let s = GithubTokenStore::new(Arc::new(MemoryKv::new()), Duration::from_millis(30));
        let id = Uuid::new_v4();
        s.store(id, "tok").await.unwrap();
        assert!(s.load(id).await.unwrap().is_some());
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(
            s.load(id).await.unwrap(),
            None,
            "the store's own TTL must be what expires this, with nothing running on a timer"
        );
    }

    #[test]
    fn debug_output_never_carries_the_token() {
        let t = StoredToken {
            access_token: "gho_super_secret_value".to_string(),
        };
        assert!(!format!("{t:?}").contains("gho_super_secret_value"));
    }
}
