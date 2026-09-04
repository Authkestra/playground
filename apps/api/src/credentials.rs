//! Storage for credentials the scenarios create (TOTP secrets, passkeys).
//!
//! These belong to a demo session, not to a person: the "user" is the session
//! id. When the session expires the credentials must go with it, which is why
//! [`SqliteJanitor`] exists.
//!
//! SQLite on the container filesystem is deliberately ephemeral — see
//! `docs/decisions/0002-session-store.md`. A visitor loses their passkey on a
//! redeploy exactly as they lose the session that owned it, so the two have
//! consistent lifetimes.

use std::sync::Arc;

use authkestra_engine::store::sql::credential::SqlxCredentialStore;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Sqlite, SqlitePool};
use uuid::Uuid;

use crate::session::CredentialJanitor;

/// The concrete credential store the scenarios use.
///
/// A concrete type rather than `Arc<dyn CredentialStore>` because the
/// framework's `TotpAuthMethod<S>` / `WebAuthnAuthMethod<S>` are generic over
/// `S: CredentialStore`, and a trait object does not satisfy that bound.
///
/// It is built on demand from the pool rather than stored: its derived `Clone`
/// is over-constrained (it requires `DB: Clone`, which `Sqlite` is not), so it
/// cannot live in a `Clone` application state. Construction is just wrapping an
/// already-`Arc`ed pool, so this costs nothing.
pub type Credentials = SqlxCredentialStore<Sqlite>;

/// Build a credential store from the pool.
pub fn store(pool: &SqlitePool) -> Credentials {
    SqlxCredentialStore::new(pool.clone())
}

/// Table the framework's SQLx credential store reads and writes.
const TABLE: &str = "ak_credentials";

/// Open the credential database and run the framework's migration.
///
/// `DATABASE_URL` defaults to an on-disk file so credentials survive a restart
/// within the same container; the file itself does not survive a redeploy.
pub async fn open() -> Result<SqlitePool, sqlx::Error> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://data.db".to_string());

    let options: SqliteConnectOptions =
        url.parse::<SqliteConnectOptions>()?.create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    store(&pool)
        .migrate()
        .await
        .map_err(|e| sqlx::Error::Protocol(format!("credential migration failed: {e}")))?;

    tracing::info!(%url, "credential store ready");
    Ok(pool)
}

/// An in-memory credential database, migrated and ready.
///
/// For tests. Capped at one connection deliberately: each SQLite in-memory
/// connection gets its *own* database, so a larger pool would hand different
/// queries different (empty) databases and fail in a thoroughly confusing way.
pub async fn open_in_memory() -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    store(&pool)
        .migrate()
        .await
        .map_err(|e| sqlx::Error::Protocol(format!("credential migration failed: {e}")))?;
    Ok(pool)
}

/// Deletes a session's credentials when the session goes away.
///
/// `CredentialStore` deliberately exposes no delete — the framework does not
/// own your data lifecycle — so this reaches the table directly. That is the
/// playground's own table, created by the framework's migration.
pub struct SqliteJanitor {
    pool: SqlitePool,
    handle: tokio::runtime::Handle,
}

impl SqliteJanitor {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            // Captured so the synchronous janitor hook can dispatch async work.
            handle: tokio::runtime::Handle::current(),
        }
    }

    /// Delete one kind of credential for a session. Returns rows removed.
    ///
    /// Needed because enrolment *appends*: the framework's `save_credential`
    /// upserts on credential id, so enrolling twice leaves two rows, and
    /// verification reads `creds.first()` — the stale one. A visitor who
    /// re-scanned a QR code would then be verifying against a secret their app
    /// no longer has, and every code would be rejected.
    pub async fn purge_type(
        pool: &SqlitePool,
        session_id: Uuid,
        cred_type: &str,
    ) -> Result<u64, sqlx::Error> {
        let sql = format!("DELETE FROM {TABLE} WHERE user_id = ? AND cred_type = ?");
        let result = sqlx::query(&sql)
            .bind(session_id.to_string())
            .bind(cred_type)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Delete every credential belonging to a session. Returns rows removed.
    pub async fn purge(pool: &SqlitePool, session_id: Uuid) -> Result<u64, sqlx::Error> {
        let sql = format!("DELETE FROM {TABLE} WHERE user_id = ?");
        let result = sqlx::query(&sql)
            .bind(session_id.to_string())
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

impl CredentialJanitor for SqliteJanitor {
    fn purge_session(&self, session_id: Uuid) {
        // Expiry is detected on a read path and by the sweeper, neither of
        // which can await. Spawn rather than block so a slow delete never
        // stalls a request; a failure is logged and retried on the next sweep.
        let pool = self.pool.clone();
        self.handle.spawn(async move {
            match SqliteJanitor::purge(&pool, session_id).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(%session_id, credentials = n, "purged session credentials"),
                Err(e) => {
                    tracing::error!(%session_id, error = %e, "failed to purge session credentials")
                }
            }
        });
    }
}

/// Build a janitor for the session store.
pub fn janitor(pool: SqlitePool) -> Arc<dyn CredentialJanitor> {
    Arc::new(SqliteJanitor::new(pool))
}
