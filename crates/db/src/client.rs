use std::sync::Arc;

use surrealdb::engine::any::{self, Any};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

use scuffed_auth::crypto::CryptoService;

use crate::{DbError, DbResult};

/// Database client wrapping SurrealDB with optional encryption.
pub struct Database {
    pub client: Surreal<Any>,
    pub crypto: Option<Arc<CryptoService>>,
}

impl Database {
    /// Connect to a remote SurrealDB instance.
    pub async fn connect(url: &str, username: &str, password: &str) -> DbResult<Self> {
        let client = any::connect(url).await?;
        client.signin(Root { username, password }).await?;
        client.use_ns("scuffed_crew").use_db("main").await?;

        let crypto = CryptoService::from_env()
            .map_err(|e| DbError::Config(format!("Crypto init failed: {e}")))?
            .map(Arc::new);

        Ok(Self { client, crypto })
    }

    /// Connect to an in-memory SurrealDB instance (for testing).
    pub async fn connect_memory() -> DbResult<Self> {
        let client = any::connect("mem://").await?;
        client.use_ns("scuffed_crew").use_db("main").await?;

        let crypto = CryptoService::from_env()
            .map_err(|e| DbError::Config(format!("Crypto init failed: {e}")))?
            .map(Arc::new);

        Ok(Self { client, crypto })
    }
}
