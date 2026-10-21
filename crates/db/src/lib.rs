// Query functions in this crate take many primitive columns by value, mirroring
// their SurrealDB table shape. Refactoring each into a parameter struct would add
// indirection without improving clarity, so we allow the lint crate-wide.
#![allow(clippy::too_many_arguments)]

mod client;
mod display_name;
pub mod migrations;
pub mod queries;
mod relay_access;
pub mod rewrap;
pub mod types;

pub use client::{
    database_boot_mode_or_exit, in_memory_dev_from_env, normalize_surrealdb_url,
    resolve_database_boot_mode, resolve_database_boot_mode_from_env, surrealdb_url_from_env,
    Database, DatabaseBootMode, DbConfig, SurrealAuthMode, PRODUCTION_WITHOUT_DATABASE_URL,
};
pub use display_name::{display_name_key, DisplayNameError};
pub use queries::daemon_tokens::DaemonTokenAuth;
pub use queries::stat_reports::{retention_deadline, utc_day_start};
pub use relay_access::{
    ensure_relay_access, ensure_relay_from_env, relay_permission_sql, RELAY_DB_ACCESS,
    RELAY_DB_USERNAME_DEFAULT, RELAY_PRINCIPAL_TABLE, RELAY_SELECT_TABLES,
};
pub use rewrap::{rewrap_all_encrypted_fields, RewrapStats};
pub use scuffed_auth::is_production_env;
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
    /// Display name is empty after normalization or contains a control character.
    /// HTTP 400 `{"error":"display_name_invalid"}`.
    #[error("display_name_invalid")]
    DisplayNameInvalid,
    /// Another member already owns this display name key.
    /// HTTP 409 `{"error":"display_name_taken"}`.
    #[error("display_name_taken")]
    DisplayNameTaken,
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
