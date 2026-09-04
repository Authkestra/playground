//! The key-value store behind every piece of playground state.
//!
//! The process holds **no** durable state of its own. Demo sessions, ceremony
//! challenges and scenario credentials all live here, keyed by string and
//! carrying a TTL. That is what makes the service safe to run on infrastructure
//! that scales to zero and recycles instances freely: any instance can serve
//! any visitor, and nothing is lost when one goes away.
//!
//! **Expiry is the store's job, not ours.** Redis drops a key when its TTL
//! lapses, which replaces the `tokio::interval` sweep this used to need — and a
//! sweeper is exactly the kind of thing that silently stops working the moment
//! the process is allowed to sleep.
//!
//! Two implementations: [`RedisKv`] for deployment, and [`MemoryKv`] so tests
//! and local development need no running Redis.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Duration;

use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store backend error: {0}")]
    Backend(String),
    #[error("stored value could not be decoded: {0}")]
    Decode(String),
}

/// A string-keyed store with per-key expiry.
///
/// Deliberately small — five operations — so that swapping the backend, or
/// adding another, stays a contained change.
#[async_trait::async_trait]
pub trait KeyValue: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<String>, StoreError>;

    /// Write a value, replacing any previous one, and (re)set its TTL.
    async fn set(&self, key: &str, value: &str, ttl: Duration) -> Result<(), StoreError>;

    async fn delete(&self, key: &str) -> Result<bool, StoreError>;

    /// Read a value and remove it in one operation.
    ///
    /// Separate from `get` + `delete` so that single-use state — a WebAuthn
    /// challenge, say — is genuinely single-use. Two concurrent requests doing
    /// get-then-delete could both observe the same challenge.
    async fn take(&self, key: &str) -> Result<Option<String>, StoreError>;

    /// Values of every key under `prefix`.
    ///
    /// Used to read a session's credentials, which are stored one key per
    /// credential so that a single credential can be updated in place.
    async fn values_with_prefix(&self, prefix: &str) -> Result<Vec<String>, StoreError>;

    /// Delete every key under `prefix`. Returns how many were removed.
    async fn delete_with_prefix(&self, prefix: &str) -> Result<u64, StoreError>;
}

/// Read a JSON value from the store.
pub async fn get_json<T: serde::de::DeserializeOwned>(
    kv: &dyn KeyValue,
    key: &str,
) -> Result<Option<T>, StoreError> {
    match kv.get(key).await? {
        None => Ok(None),
        Some(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| StoreError::Decode(e.to_string())),
    }
}

/// Write a JSON value with a TTL.
pub async fn set_json<T: serde::Serialize>(
    kv: &dyn KeyValue,
    key: &str,
    value: &T,
    ttl: Duration,
) -> Result<(), StoreError> {
    let raw = serde_json::to_string(value).map_err(|e| StoreError::Decode(e.to_string()))?;
    kv.set(key, &raw, ttl).await
}

// ---------------------------------------------------------------------- redis

/// Redis-backed store. The client is cheap to clone and connects lazily.
#[derive(Clone)]
pub struct RedisKv {
    client: redis::Client,
    /// Namespace, so several deployments can share one Redis instance.
    prefix: String,
}

impl RedisKv {
    pub fn new(client: redis::Client, prefix: impl Into<String>) -> Self {
        Self {
            client,
            prefix: prefix.into(),
        }
    }

    fn full(&self, key: &str) -> String {
        format!("{}:{}", self.prefix, key)
    }

    async fn conn(&self) -> Result<redis::aio::MultiplexedConnection, StoreError> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    /// Every key under a prefix, found with SCAN.
    ///
    /// SCAN rather than KEYS: KEYS blocks the server for the whole scan, which
    /// is a poor thing to do to a shared instance even at demo scale.
    async fn scan(&self, prefix: &str) -> Result<Vec<String>, StoreError> {
        let mut conn = self.conn().await?;
        let pattern = format!("{}*", self.full(prefix));
        let mut cursor: u64 = 0;
        let mut keys = Vec::new();
        loop {
            let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(200)
                .query_async(&mut conn)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            keys.extend(batch);
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        Ok(keys)
    }
}

#[async_trait::async_trait]
impl KeyValue for RedisKv {
    async fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        conn.get(self.full(key))
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    async fn set(&self, key: &str, value: &str, ttl: Duration) -> Result<(), StoreError> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        // SET with EX: the write and the expiry are one round trip, so a key
        // can never be left without a TTL by a failure in between.
        let secs = ttl.as_secs().max(1);
        conn.set_ex::<_, _, ()>(self.full(key), value, secs)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    async fn delete(&self, key: &str) -> Result<bool, StoreError> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        let removed: i64 = conn
            .del(self.full(key))
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(removed > 0)
    }

    async fn take(&self, key: &str) -> Result<Option<String>, StoreError> {
        let mut conn = self.conn().await?;
        // GETDEL is atomic, so a challenge cannot be observed twice.
        redis::cmd("GETDEL")
            .arg(self.full(key))
            .query_async(&mut conn)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    async fn values_with_prefix(&self, prefix: &str) -> Result<Vec<String>, StoreError> {
        use redis::AsyncCommands;
        let keys = self.scan(prefix).await?;
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.conn().await?;
        // MGET so N credentials cost one round trip rather than N.
        let values: Vec<Option<String>> = conn
            .mget(&keys)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(values.into_iter().flatten().collect())
    }

    async fn delete_with_prefix(&self, prefix: &str) -> Result<u64, StoreError> {
        use redis::AsyncCommands;
        let keys = self.scan(prefix).await?;
        if keys.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn().await?;
        let removed: i64 = conn
            .del(keys)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(removed as u64)
    }
}

// --------------------------------------------------------------------- memory

#[derive(Debug, Clone)]
struct Entry {
    value: String,
    expires_at: DateTime<Utc>,
}

/// In-process store for tests and local development.
///
/// Expiry is enforced on read rather than by a background task, so it behaves
/// like Redis from the caller's point of view without needing a timer.
#[derive(Default)]
pub struct MemoryKv {
    entries: RwLock<HashMap<String, Entry>>,
}

impl MemoryKv {
    pub fn new() -> Self {
        Self::default()
    }

    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[async_trait::async_trait]
impl KeyValue for MemoryKv {
    async fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        let now = self.now();
        let guard = self.entries.read().expect("memory store poisoned");
        Ok(guard
            .get(key)
            .filter(|e| now < e.expires_at)
            .map(|e| e.value.clone()))
    }

    async fn set(&self, key: &str, value: &str, ttl: Duration) -> Result<(), StoreError> {
        let expires_at = self.now()
            + chrono::Duration::from_std(ttl)
                .map_err(|e| StoreError::Backend(format!("bad ttl: {e}")))?;
        self.entries.write().expect("memory store poisoned").insert(
            key.to_string(),
            Entry {
                value: value.to_string(),
                expires_at,
            },
        );
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<bool, StoreError> {
        Ok(self
            .entries
            .write()
            .expect("memory store poisoned")
            .remove(key)
            .is_some())
    }

    async fn take(&self, key: &str) -> Result<Option<String>, StoreError> {
        let now = self.now();
        // One lock, so this is atomic with respect to other callers.
        Ok(self
            .entries
            .write()
            .expect("memory store poisoned")
            .remove(key)
            .filter(|e| now < e.expires_at)
            .map(|e| e.value))
    }

    async fn values_with_prefix(&self, prefix: &str) -> Result<Vec<String>, StoreError> {
        let now = self.now();
        let guard = self.entries.read().expect("memory store poisoned");
        Ok(guard
            .iter()
            .filter(|(k, e)| k.starts_with(prefix) && now < e.expires_at)
            .map(|(_, e)| e.value.clone())
            .collect())
    }

    async fn delete_with_prefix(&self, prefix: &str) -> Result<u64, StoreError> {
        let mut guard = self.entries.write().expect("memory store poisoned");
        let before = guard.len();
        guard.retain(|k, _| !k.starts_with(prefix));
        Ok((before - guard.len()) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Properties both backends must share. `MemoryKv` is what the rest of the
    /// test suite runs against, so it has to behave like Redis or the tests are
    /// measuring the wrong thing.
    async fn assert_kv_contract(kv: &dyn KeyValue) {
        let ttl = Duration::from_secs(60);

        assert_eq!(kv.get("missing").await.unwrap(), None);

        kv.set("a", "one", ttl).await.unwrap();
        assert_eq!(kv.get("a").await.unwrap().as_deref(), Some("one"));

        // set replaces rather than appends
        kv.set("a", "two", ttl).await.unwrap();
        assert_eq!(kv.get("a").await.unwrap().as_deref(), Some("two"));

        // take reads and removes atomically
        kv.set("t1", "once", ttl).await.unwrap();
        assert_eq!(kv.take("t1").await.unwrap().as_deref(), Some("once"));
        assert_eq!(
            kv.take("t1").await.unwrap(),
            None,
            "take must be single-use"
        );

        kv.set("a", "two", ttl).await.unwrap();
        assert!(kv.delete("a").await.unwrap());
        assert!(!kv.delete("a").await.unwrap(), "second delete is a no-op");
        assert_eq!(kv.get("a").await.unwrap(), None);

        // prefix operations must not touch neighbours
        kv.set("cred:s1:totp:x", "1", ttl).await.unwrap();
        kv.set("cred:s1:totp:y", "2", ttl).await.unwrap();
        kv.set("cred:s2:totp:z", "3", ttl).await.unwrap();

        let mut mine = kv.values_with_prefix("cred:s1:").await.unwrap();
        mine.sort();
        assert_eq!(mine, vec!["1".to_string(), "2".to_string()]);

        assert_eq!(kv.delete_with_prefix("cred:s1:").await.unwrap(), 2);
        assert!(kv.values_with_prefix("cred:s1:").await.unwrap().is_empty());
        assert_eq!(
            kv.values_with_prefix("cred:s2:").await.unwrap(),
            vec!["3".to_string()],
            "deleting one session's credentials must not touch another's"
        );
    }

    #[tokio::test]
    async fn memory_backend_satisfies_the_contract() {
        assert_kv_contract(&MemoryKv::new()).await;
    }

    /// An expired key must read as absent — this is what replaces the sweeper.
    #[tokio::test]
    async fn memory_backend_expires_keys() {
        let kv = MemoryKv::new();
        kv.set("k", "v", Duration::from_millis(30)).await.unwrap();
        assert!(kv.get("k").await.unwrap().is_some());
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(
            kv.get("k").await.unwrap(),
            None,
            "expiry must be enforced without a background task"
        );
    }

    #[tokio::test]
    async fn expired_keys_are_excluded_from_prefix_reads() {
        let kv = MemoryKv::new();
        kv.set("p:live", "1", Duration::from_secs(60))
            .await
            .unwrap();
        kv.set("p:dead", "2", Duration::from_millis(30))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(
            kv.values_with_prefix("p:").await.unwrap(),
            vec!["1".to_string()]
        );
    }

    #[tokio::test]
    async fn json_helpers_round_trip() {
        let kv = MemoryKv::new();
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Thing {
            a: u32,
            b: String,
        }
        let thing = Thing {
            a: 7,
            b: "x".into(),
        };
        set_json(&kv, "t", &thing, Duration::from_secs(60))
            .await
            .unwrap();
        let back: Option<Thing> = get_json(&kv, "t").await.unwrap();
        assert_eq!(back, Some(thing));
    }

    /// Only runs when a Redis is reachable, so the suite stays runnable
    /// without one. CI provides it as a service container.
    #[tokio::test]
    async fn redis_backend_satisfies_the_same_contract() {
        let Ok(url) = std::env::var("REDIS_URL") else {
            eprintln!("skipping: REDIS_URL not set");
            return;
        };
        let client = redis::Client::open(url).expect("redis client");
        let kv = RedisKv::new(client, format!("test-{}", uuid::Uuid::new_v4()));
        assert_kv_contract(&kv).await;
        // leave nothing behind
        kv.delete_with_prefix("").await.unwrap();
    }

    #[tokio::test]
    async fn redis_backend_expires_keys() {
        let Ok(url) = std::env::var("REDIS_URL") else {
            eprintln!("skipping: REDIS_URL not set");
            return;
        };
        let client = redis::Client::open(url).expect("redis client");
        let kv = RedisKv::new(client, format!("test-{}", uuid::Uuid::new_v4()));
        // Redis TTL granularity is one second, so this is the shortest
        // meaningful check.
        kv.set("k", "v", Duration::from_secs(1)).await.unwrap();
        assert!(kv.get("k").await.unwrap().is_some());
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(kv.get("k").await.unwrap(), None);
    }
}
