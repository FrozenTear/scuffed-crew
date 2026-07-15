use std::sync::Arc;

use surrealdb::engine::any::{self, Any};
use surrealdb::opt::auth::{Database as DatabaseAuth, Root};
use surrealdb::Surreal;

use scuffed_auth::crypto::CryptoService;

use crate::{DbError, DbResult};

/// Configuration for database namespace and database selection.
pub struct DbConfig {
    pub namespace: String,
    pub database: String,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            namespace: "scuffed_crew".to_string(),
            database: "main".to_string(),
        }
    }
}

/// Auth mode for remote SurrealDB connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurrealAuthMode {
    /// Namespace/database-scoped user (preferred for production).
    Scoped,
    /// Root credentials (admin). Allowed in non-production only when explicitly chosen.
    Root,
}

impl SurrealAuthMode {
    /// Parse from `SURREALDB_AUTH_MODE` (`scoped` | `root`). Defaults to `root`
    /// for backward compatibility; production deploys should set `scoped`.
    pub fn from_env() -> Self {
        match std::env::var("SURREALDB_AUTH_MODE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "scoped" | "database" | "db" => Self::Scoped,
            _ => Self::Root,
        }
    }
}

/// True when running in a production-hardened configuration.
pub fn is_production_env() -> bool {
    matches!(
        std::env::var("PRODUCTION").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

/// Reject insecure root/root credentials (always in production; warn otherwise).
fn check_credentials(username: &str, password: &str) -> DbResult<()> {
    let weak = username == "root" && (password == "root" || password.is_empty());
    if !weak {
        return Ok(());
    }
    if is_production_env() {
        return Err(DbError::Config(
            "Refusing default root/root SurrealDB credentials when PRODUCTION is set. \
             Set strong SURREALDB_USER/SURREALDB_PASSWORD and prefer SURREALDB_AUTH_MODE=scoped."
                .into(),
        ));
    }
    tracing::warn!(
        "Using default SurrealDB root/root credentials — never use this outside local dev"
    );
    Ok(())
}

/// Database client wrapping SurrealDB with optional encryption.
pub struct Database {
    pub client: Surreal<Any>,
    pub crypto: Option<Arc<CryptoService>>,
}

impl Database {
    fn load_crypto() -> DbResult<Option<Arc<CryptoService>>> {
        let crypto = CryptoService::from_env()
            .map_err(|e| DbError::Config(format!("Crypto init failed: {e}")))?
            .map(Arc::new);

        if crypto.is_none() {
            if is_production_env() {
                return Err(DbError::Config(
                    "ENCRYPTION_KEY is required when PRODUCTION is set \
                     (OAuth provider IDs, Nostr keys, DM content at rest)."
                        .into(),
                ));
            }
            tracing::warn!(
                "ENCRYPTION_KEY not set — OAuth provider IDs stored in cleartext; \
                 Nostr server keys and DM at-rest encryption disabled"
            );
        }

        Ok(crypto)
    }

    /// Connect to a remote SurrealDB instance with root credentials.
    ///
    /// Prefer [`Self::connect_scoped`] or [`Self::connect_from_env`] in production.
    pub async fn connect(
        url: &str,
        username: &str,
        password: &str,
        config: DbConfig,
    ) -> DbResult<Self> {
        check_credentials(username, password)?;
        if is_production_env() {
            tracing::warn!(
                "Connecting with SurrealDB root auth in production — set SURREALDB_AUTH_MODE=scoped"
            );
        }

        let client = any::connect(url).await?;
        client
            .signin(Root {
                username: username.to_string(),
                password: password.to_string(),
            })
            .await?;
        client
            .use_ns(&config.namespace)
            .use_db(&config.database)
            .await?;

        Ok(Self {
            client,
            crypto: Self::load_crypto()?,
        })
    }

    /// Connect to a remote SurrealDB instance with database-scoped credentials.
    pub async fn connect_scoped(
        url: &str,
        namespace: &str,
        database: &str,
        username: &str,
        password: &str,
    ) -> DbResult<Self> {
        check_credentials(username, password)?;

        let client = any::connect(url).await?;
        client
            .signin(DatabaseAuth {
                namespace: namespace.to_string(),
                database: database.to_string(),
                username: username.to_string(),
                password: password.to_string(),
            })
            .await?;
        client.use_ns(namespace).use_db(database).await?;

        Ok(Self {
            client,
            crypto: Self::load_crypto()?,
        })
    }

    /// Connect using environment variables.
    ///
    /// - `SURREALDB_URL` (required)
    /// - `SURREALDB_USER` / `SURREALDB_PASSWORD` (default `root` / `root` — blocked when PRODUCTION)
    /// - `SURREALDB_NS` / `SURREALDB_DB` (defaults `scuffed_crew` / `main`)
    /// - `SURREALDB_AUTH_MODE` = `scoped` | `root` (default `root`)
    pub async fn connect_from_env() -> DbResult<Self> {
        let url = std::env::var("SURREALDB_URL").map_err(|_| {
            DbError::Config("SURREALDB_URL is required for connect_from_env".into())
        })?;
        let user = std::env::var("SURREALDB_USER").unwrap_or_else(|_| "root".to_string());
        let pass = std::env::var("SURREALDB_PASSWORD").unwrap_or_else(|_| "root".to_string());
        let ns = std::env::var("SURREALDB_NS").unwrap_or_else(|_| "scuffed_crew".to_string());
        let db = std::env::var("SURREALDB_DB").unwrap_or_else(|_| "main".to_string());

        match SurrealAuthMode::from_env() {
            SurrealAuthMode::Scoped => Self::connect_scoped(&url, &ns, &db, &user, &pass).await,
            SurrealAuthMode::Root => {
                Self::connect(
                    &url,
                    &user,
                    &pass,
                    DbConfig {
                        namespace: ns,
                        database: db,
                    },
                )
                .await
            }
        }
    }

    /// Connect to an in-memory SurrealDB instance (for dev/testing).
    pub async fn connect_memory() -> DbResult<Self> {
        let config = DbConfig::default();
        let client = any::connect("mem://").await?;
        client
            .use_ns(&config.namespace)
            .use_db(&config.database)
            .await?;

        // Memory/dev: crypto optional (do not force PRODUCTION rules).
        let crypto = CryptoService::from_env()
            .map_err(|e| DbError::Config(format!("Crypto init failed: {e}")))?
            .map(Arc::new);

        Ok(Self { client, crypto })
    }

    /// Whether OAuth provider IDs must be encrypted (production or remote with no override).
    pub fn require_encrypted_provider_ids(&self) -> bool {
        if std::env::var("ALLOW_PLAINTEXT_PROVIDER_IDS").ok().as_deref() == Some("1") {
            return false;
        }
        is_production_env() || self.crypto.is_some()
    }
}
