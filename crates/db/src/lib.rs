mod client;
pub mod migrations;
pub mod queries;
pub mod types;

pub use client::Database;
pub use types::*;

use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database error: {0}")]
    Surreal(#[from] surrealdb::Error),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Operation timed out")]
    Timeout,
    #[error("Encryption error: {0}")]
    Crypto(#[from] scuffed_auth::crypto::CryptoError),
}

pub type DbResult<T> = Result<T, DbError>;

const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn with_timeout<T, F>(future: F) -> DbResult<T>
where
    F: std::future::Future<Output = DbResult<T>>,
{
    tokio::time::timeout(DEFAULT_QUERY_TIMEOUT, future)
        .await
        .map_err(|_| DbError::Timeout)?
}
