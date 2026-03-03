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

/// Database client wrapping SurrealDB with optional encryption.
pub struct Database {
    pub client: Surreal<Any>,
    pub crypto: Option<Arc<CryptoService>>,
}

impl Database {
    /// Connect to a remote SurrealDB instance with root credentials.
    pub async fn connect(
        url: &str,
        username: &str,
        password: &str,
        config: DbConfig,
    ) -> DbResult<Self> {
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

        let crypto = CryptoService::from_env()
            .map_err(|e| DbError::Config(format!("Crypto init failed: {e}")))?
            .map(Arc::new);

        Ok(Self { client, crypto })
    }

    /// Connect to a remote SurrealDB instance with database-scoped credentials.
    pub async fn connect_scoped(
        url: &str,
        namespace: &str,
        database: &str,
        username: &str,
        password: &str,
    ) -> DbResult<Self> {
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

        let crypto = CryptoService::from_env()
            .map_err(|e| DbError::Config(format!("Crypto init failed: {e}")))?
            .map(Arc::new);

        Ok(Self { client, crypto })
    }

    /// Connect to an in-memory SurrealDB instance (for dev/testing).
    pub async fn connect_memory() -> DbResult<Self> {
        let config = DbConfig::default();
        let client = any::connect("mem://").await?;
        client
            .use_ns(&config.namespace)
            .use_db(&config.database)
            .await?;

        let crypto = CryptoService::from_env()
            .map_err(|e| DbError::Config(format!("Crypto init failed: {e}")))?
            .map(Arc::new);

        Ok(Self { client, crypto })
    }
}
