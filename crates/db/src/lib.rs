// Query functions in this crate take many primitive columns by value, mirroring
// their SurrealDB table shape. Refactoring each into a parameter struct would add
// indirection without improving clarity, so we allow the lint crate-wide.
#![allow(clippy::too_many_arguments)]

mod client;
pub mod migrations;
pub mod queries;
pub mod types;

pub use client::{Database, DbConfig};
pub use types::*;

use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database error: {0}")]
    Surreal(#[from] surrealdb::Error),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Operation timed out")]
    Timeout,
    #[error("Encryption error: {0}")]
    Crypto(#[from] scuffed_auth::crypto::CryptoError),
}

pub type DbResult<T> = Result<T, DbError>;

const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Extract a string from a `RecordIdKey` enum (SurrealDB v3).
pub(crate) fn record_id_key_to_string(key: surrealdb_types::RecordIdKey) -> String {
    match key {
        surrealdb_types::RecordIdKey::String(s) => s,
        surrealdb_types::RecordIdKey::Number(n) => n.to_string(),
        other => format!("{:?}", other),
    }
}

pub(crate) async fn with_timeout<T, F>(future: F) -> DbResult<T>
where
    F: std::future::Future<Output = DbResult<T>>,
{
    tokio::time::timeout(DEFAULT_QUERY_TIMEOUT, future)
        .await
        .map_err(|_| DbError::Timeout)?
}
