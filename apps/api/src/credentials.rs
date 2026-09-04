//! Credentials the scenarios create: TOTP secrets and passkeys.
//!
//! These belong to a demo session, not to a person — the "user" is the session
//! id — so they are stored with the session's TTL and expire with it. That is
//! the whole cleanup mechanism: no sweeper, no janitor, nothing to run. Which
//! matters, because a process that is allowed to scale to zero cannot be
//! relied on to run anything on a timer.
//!
//! ## Why not the framework's `SqlxCredentialStore`
//!
//! It works, but it needs a filesystem or a SQL server, and neither survives a
//! container that comes and goes. It also derives a credential's id from
//! `data["credential_id"]`/`data["id"]`, falling back to a random UUID —
//! whereas `update_credential` is called with the id the *browser* used. For a
//! passkey those never match, so the signature-counter update lands on zero
//! rows and is silently lost. This implementation derives the id from the
//! credential itself, so the counter update finds it.

use std::sync::Arc;
use std::time::Duration;

use authkestra_engine::auth::error::AuthError;
use authkestra_engine::auth::store::CredentialStore;
use serde_json::Value;
use uuid::Uuid;

use crate::store::KeyValue;

/// Stable id for the single TOTP secret a session may hold.
///
/// A constant rather than a UUID so that re-enrolling *replaces* the previous
/// secret instead of adding a second one. Enrolment appending was a real bug:
/// verification reads the first stored credential, so a visitor who re-scanned
/// a QR code kept being checked against the secret their app no longer had.
/// A fixed id makes that impossible by construction.
const TOTP_CREDENTIAL_ID: &str = "primary";

/// Credential storage over the shared key-value store.
///
/// `Clone` is cheap (an `Arc` and a `Duration`) and needed because the
/// framework's `TotpAuthMethod<S>` / `WebAuthnAuthMethod<S>` take an owned
/// `S: CredentialStore`.
#[derive(Clone)]
pub struct KvCredentialStore {
    kv: Arc<dyn KeyValue>,
    ttl: Duration,
}

impl KvCredentialStore {
    pub fn new(kv: Arc<dyn KeyValue>, ttl: Duration) -> Self {
        Self { kv, ttl }
    }

    fn credential_key(user_id: &str, cred_type: &str, credential_id: &str) -> String {
        format!("cred:{user_id}:{cred_type}:{credential_id}")
    }

    /// Index from credential id back to its primary key.
    ///
    /// `update_credential` receives only a credential id, so without this there
    /// is no way to find which session's credential to rewrite.
    fn reference_key(credential_id: &str) -> String {
        format!("credref:{credential_id}")
    }

    fn prefix_for(user_id: &str, cred_type: &str) -> String {
        format!("cred:{user_id}:{cred_type}:")
    }

    fn session_prefix(user_id: &str) -> String {
        format!("cred:{user_id}:")
    }

    /// The key a credential is filed under.
    ///
    /// Deliberately **not** the `credential_id` the framework puts in the data:
    /// `register_totp` mints a fresh UUID on every enrolment, so honouring it
    /// would file a second secret rather than replace the first — and
    /// verification reads whichever credential comes back first, so a visitor
    /// who re-scanned a QR code would be checked against the secret they had
    /// just discarded. A session holds at most one TOTP secret, so it gets a
    /// fixed id.
    fn storage_id(cred_type: &str, data: &Value) -> String {
        match cred_type {
            "totp" => TOTP_CREDENTIAL_ID.to_string(),
            // A passkey is referred to by its `cred_id`, which is also what
            // `update_credential` is called with during authentication.
            "webauthn" => data
                .pointer("/cred/cred_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            _ => data
                .get("credential_id")
                .or_else(|| data.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
        }
    }

    /// Every id `update_credential` might later be called with for this
    /// credential.
    ///
    /// The framework updates a credential by the id embedded in the *data* it
    /// stored — a UUID for TOTP, the `cred_id` for a passkey — which is not
    /// necessarily the key the credential is filed under. Each of these gets a
    /// back-reference, or persisting a signature counter or a TOTP
    /// `last_used_step` fails, and the framework treats that as fatal.
    fn reference_ids(cred_type: &str, data: &Value) -> Vec<String> {
        let mut ids = Vec::new();

        if let Some(id) = data
            .get("credential_id")
            .or_else(|| data.get("id"))
            .and_then(|v| v.as_str())
        {
            ids.push(id.to_string());
        }
        if cred_type == "webauthn" {
            if let Some(id) = data.pointer("/cred/cred_id").and_then(|v| v.as_str()) {
                ids.push(id.to_string());
            }
        }
        ids.push(Self::storage_id(cred_type, data));

        ids.sort();
        ids.dedup();
        ids
    }

    /// Remove every credential belonging to a session.
    ///
    /// TTL already handles the ordinary case; this exists for an explicit reset,
    /// where a visitor expects their state gone immediately.
    pub async fn purge_session(&self, user_id: &str) -> Result<u64, AuthError> {
        // Read first so the back-references can go too — they would expire on
        // their own, but leaving them would let a stale id resolve to a key
        // that no longer exists.
        let existing = self
            .kv
            .values_with_prefix(&Self::session_prefix(user_id))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        for raw in &existing {
            if let Ok(value) = serde_json::from_str::<Value>(raw) {
                // The stored blob does not say which kind it is, so clear the
                // references both kinds could have produced. A reference that
                // was never written simply is not there.
                for cred_type in ["webauthn", "totp"] {
                    for id in Self::reference_ids(cred_type, &value) {
                        let _ = self.kv.delete(&Self::reference_key(&id)).await;
                    }
                }
            }
        }

        let removed = self
            .kv
            .delete_with_prefix(&Self::session_prefix(user_id))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        if removed > 0 {
            tracing::info!(user_id, credentials = removed, "purged session credentials");
        }
        Ok(removed)
    }

    /// Remove one kind of credential for a session.
    pub async fn purge_type(&self, user_id: &str, cred_type: &str) -> Result<u64, AuthError> {
        self.kv
            .delete_with_prefix(&Self::prefix_for(user_id, cred_type))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))
    }

    /// How many credentials of a kind a session holds.
    pub async fn count(&self, user_id: &str, cred_type: &str) -> Result<usize, AuthError> {
        Ok(self
            .kv
            .values_with_prefix(&Self::prefix_for(user_id, cred_type))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?
            .len())
    }
}

#[async_trait::async_trait]
impl CredentialStore for KvCredentialStore {
    async fn save_credential(
        &self,
        user_id: &str,
        cred_type: &str,
        data: Value,
    ) -> Result<(), AuthError> {
        let storage_id = Self::storage_id(cred_type, &data);
        let key = Self::credential_key(user_id, cred_type, &storage_id);

        // Re-enrolling replaces the credential at this key, so the previous
        // one's back-references must go. They would expire on their own, but
        // until then a discarded id would still resolve to this key — and an
        // update arriving through it would write stale data over the live
        // credential.
        let new_refs = Self::reference_ids(cred_type, &data);
        if let Ok(Some(previous)) = self.kv.get(&key).await {
            if let Ok(previous) = serde_json::from_str::<Value>(&previous) {
                for id in Self::reference_ids(cred_type, &previous) {
                    if !new_refs.contains(&id) {
                        let _ = self.kv.delete(&Self::reference_key(&id)).await;
                    }
                }
            }
        }

        let raw = serde_json::to_string(&data).map_err(|e| AuthError::Internal(e.to_string()))?;

        self.kv
            .set(&key, &raw, self.ttl)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        // Back-references carry the same TTL, so none can outlive the
        // credential it points at.
        for id in &new_refs {
            self.kv
                .set(&Self::reference_key(id), &key, self.ttl)
                .await
                .map_err(|e| AuthError::Internal(e.to_string()))?;
        }

        tracing::debug!(user_id, cred_type, storage_id, "credential saved");
        Ok(())
    }

    async fn get_credentials(
        &self,
        user_id: &str,
        cred_type: &str,
    ) -> Result<Vec<Value>, AuthError> {
        let raws = self
            .kv
            .values_with_prefix(&Self::prefix_for(user_id, cred_type))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        raws.into_iter()
            .map(|raw| {
                serde_json::from_str(&raw).map_err(|e| {
                    AuthError::Internal(format!("stored credential is unreadable: {e}"))
                })
            })
            .collect()
    }

    async fn update_credential(&self, credential_id: &str, data: Value) -> Result<(), AuthError> {
        // Resolve the id the caller has back to the key it lives under.
        let key = self
            .kv
            .get(&Self::reference_key(credential_id))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?
            .ok_or_else(|| {
                // Returning an error rather than quietly succeeding: the caller
                // updates a signature counter here, and losing it would weaken
                // clone detection with no sign that anything went wrong.
                AuthError::Internal(format!("no credential is filed under id `{credential_id}`"))
            })?;

        let raw = serde_json::to_string(&data).map_err(|e| AuthError::Internal(e.to_string()))?;

        self.kv
            .set(&key, &raw, self.ttl)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        tracing::debug!(credential_id, "credential updated");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryKv;
    use serde_json::json;

    fn store() -> (Arc<dyn KeyValue>, KvCredentialStore) {
        let kv: Arc<dyn KeyValue> = Arc::new(MemoryKv::new());
        let creds = KvCredentialStore::new(kv.clone(), Duration::from_secs(3600));
        (kv, creds)
    }

    /// `register_totp` mints a fresh `credential_id` on every enrolment. Filing
    /// by that id would accumulate secrets, and verification reads whichever
    /// comes back first — so a visitor who re-scanned would be checked against
    /// the secret they had just replaced.
    #[test]
    fn totp_is_filed_under_a_stable_id_regardless_of_the_supplied_one() {
        let a = json!({ "credential_id": "uuid-one", "secret": "AAA" });
        let b = json!({ "credential_id": "uuid-two", "secret": "BBB" });
        assert_eq!(
            KvCredentialStore::storage_id("totp", &a),
            KvCredentialStore::storage_id("totp", &b),
        );
    }

    #[test]
    fn a_passkey_is_filed_under_its_cred_id() {
        let passkey = json!({ "cred": { "cred_id": "Y2lk", "counter": 0 } });
        assert_eq!(
            KvCredentialStore::storage_id("webauthn", &passkey),
            "Y2lk",
            "the browser refers to a passkey by cred_id, and so does update_credential"
        );
    }

    /// The framework updates by the id in the *data*, which for TOTP is not the
    /// key we filed it under. Both must resolve.
    #[test]
    fn reference_ids_cover_both_the_supplied_id_and_the_storage_id() {
        let data = json!({ "credential_id": "uuid-one", "secret": "AAA" });
        let ids = KvCredentialStore::reference_ids("totp", &data);
        assert!(ids.contains(&"uuid-one".to_string()), "{ids:?}");
        assert!(ids.contains(&TOTP_CREDENTIAL_ID.to_string()), "{ids:?}");
    }

    #[tokio::test]
    async fn re_enrolling_totp_replaces_rather_than_accumulates() {
        let (_kv, creds) = store();
        creds
            .save_credential(
                "s1",
                "totp",
                json!({ "credential_id": "u1", "secret": "AAA" }),
            )
            .await
            .unwrap();
        creds
            .save_credential(
                "s1",
                "totp",
                json!({ "credential_id": "u2", "secret": "BBB" }),
            )
            .await
            .unwrap();

        let stored = creds.get_credentials("s1", "totp").await.unwrap();
        assert_eq!(stored.len(), 1, "a session holds one TOTP secret");
        assert_eq!(stored[0]["secret"], "BBB", "the newer secret must win");
    }

    /// Replay protection depends on this: the framework persists
    /// `last_used_step` through `update_credential` and treats a failure as
    /// fatal, so the id it holds has to resolve.
    #[tokio::test]
    async fn a_credential_can_be_updated_by_the_id_the_framework_holds() {
        let (_kv, creds) = store();
        creds
            .save_credential(
                "s1",
                "totp",
                json!({ "credential_id": "u1", "secret": "AAA", "last_used_step": 0 }),
            )
            .await
            .unwrap();

        creds
            .update_credential(
                "u1",
                json!({ "credential_id": "u1", "secret": "AAA", "last_used_step": 42 }),
            )
            .await
            .expect("updating by the supplied credential_id must work");

        let stored = creds.get_credentials("s1", "totp").await.unwrap();
        assert_eq!(stored[0]["last_used_step"], 42);
        assert_eq!(stored.len(), 1, "an update must not create a second row");
    }

    #[tokio::test]
    async fn a_passkey_counter_update_resolves_by_cred_id() {
        let (_kv, creds) = store();
        creds
            .save_credential(
                "s1",
                "webauthn",
                json!({ "cred": { "cred_id": "Y2lk", "counter": 0 } }),
            )
            .await
            .unwrap();

        creds
            .update_credential(
                "Y2lk",
                json!({ "cred": { "cred_id": "Y2lk", "counter": 7 } }),
            )
            .await
            .expect("counter update must find the passkey");

        let stored = creds.get_credentials("s1", "webauthn").await.unwrap();
        assert_eq!(
            stored[0]["cred"]["counter"], 7,
            "a counter that cannot be persisted silently weakens clone detection"
        );
    }

    /// Better a loud error than a silently-lost security counter.
    #[tokio::test]
    async fn updating_an_unknown_credential_is_an_error() {
        let (_kv, creds) = store();
        assert!(creds
            .update_credential("never-saved", json!({}))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn credentials_are_scoped_to_their_session() {
        let (_kv, creds) = store();
        creds
            .save_credential(
                "s1",
                "totp",
                json!({ "credential_id": "u1", "secret": "AAA" }),
            )
            .await
            .unwrap();
        creds
            .save_credential(
                "s2",
                "totp",
                json!({ "credential_id": "u2", "secret": "BBB" }),
            )
            .await
            .unwrap();

        assert_eq!(creds.count("s1", "totp").await.unwrap(), 1);
        assert_eq!(creds.count("s2", "totp").await.unwrap(), 1);

        creds.purge_session("s1").await.unwrap();

        assert_eq!(creds.count("s1", "totp").await.unwrap(), 0);
        assert_eq!(
            creds.count("s2", "totp").await.unwrap(),
            1,
            "purging one session must not touch another's credentials"
        );
    }

    #[tokio::test]
    async fn purging_also_clears_the_back_references() {
        let (kv, creds) = store();
        creds
            .save_credential(
                "s1",
                "totp",
                json!({ "credential_id": "u1", "secret": "AAA" }),
            )
            .await
            .unwrap();
        creds.purge_session("s1").await.unwrap();

        assert!(
            kv.get("credref:u1").await.unwrap().is_none(),
            "a dangling reference would resolve to a key that no longer exists"
        );
    }
}

#[cfg(test)]
mod stale_reference_tests {
    use super::*;
    use crate::store::MemoryKv;
    use serde_json::json;

    /// A discarded credential id must stop resolving, or an update arriving
    /// through it would write stale data over the live credential.
    #[tokio::test]
    async fn re_enrolling_drops_the_previous_credentials_references() {
        let kv: Arc<dyn KeyValue> = Arc::new(MemoryKv::new());
        let creds = KvCredentialStore::new(kv.clone(), Duration::from_secs(3600));

        creds
            .save_credential(
                "s1",
                "totp",
                json!({ "credential_id": "old", "secret": "AAA" }),
            )
            .await
            .unwrap();
        creds
            .save_credential(
                "s1",
                "totp",
                json!({ "credential_id": "new", "secret": "BBB" }),
            )
            .await
            .unwrap();

        assert!(
            kv.get("credref:old").await.unwrap().is_none(),
            "the discarded id should no longer resolve"
        );
        assert!(kv.get("credref:new").await.unwrap().is_some());

        // And an update through the current id still works.
        creds
            .update_credential(
                "new",
                json!({ "credential_id": "new", "secret": "BBB", "last_used_step": 9 }),
            )
            .await
            .unwrap();
        let stored = creds.get_credentials("s1", "totp").await.unwrap();
        assert_eq!(stored[0]["last_used_step"], 9);
    }
}
