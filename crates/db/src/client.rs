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
    /// Namespace/database-scoped app user (required default for production).
    Scoped,
    /// Root credentials. Dev-only unless explicitly forced.
    Root,
}

impl SurrealAuthMode {
    /// `SURREALDB_AUTH_MODE`: `scoped` | `root`.
    /// Defaults to **scoped** when `PRODUCTION` is on, otherwise **root** (local convenience).
    pub fn from_env() -> Self {
        match std::env::var("SURREALDB_AUTH_MODE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "scoped" | "database" | "db" => Self::Scoped,
            "root" => Self::Root,
            "" if scuffed_auth::is_production_env() => Self::Scoped,
            "" => Self::Root,
            other => {
                tracing::warn!(
                    mode = other,
                    "Unknown SURREALDB_AUTH_MODE — defaulting to scoped"
                );
                Self::Scoped
            }
        }
    }
}

/// Reject insecure root/root credentials (always in production; warn otherwise).
fn check_credentials(username: &str, password: &str) -> DbResult<()> {
    let weak = username == "root" && (password == "root" || password.is_empty());
    if !weak {
        return Ok(());
    }
    if scuffed_auth::is_production_env() {
        return Err(DbError::Config(
            "Refusing default root/root SurrealDB credentials when PRODUCTION is set. \
             Set strong SURREALDB_PASSWORD and prefer SURREALDB_AUTH_MODE=scoped."
                .into(),
        ));
    }
    tracing::warn!(
        "Using default SurrealDB root/root credentials — never use this outside local dev"
    );
    Ok(())
}

fn assert_safe_sql_ident(name: &str, what: &str) -> DbResult<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(DbError::Config(format!(
            "Invalid {what}: use only ASCII alphanumeric and underscore"
        )));
    }
    Ok(())
}

fn escape_surreal_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Remote DB boot policy: PRODUCTION + ENCRYPTION_KEY required.
fn assert_remote_production_policy() -> DbResult<()> {
    if !scuffed_auth::is_production_env() {
        return Err(DbError::Config(
            "Remote SurrealDB requires PRODUCTION=1 (set by install.sh for real deploys). \
             In-memory dev: leave SURREALDB_URL unset."
                .into(),
        ));
    }
    match std::env::var("ENCRYPTION_KEY") {
        Ok(k) if !k.trim().is_empty() => Ok(()),
        _ => Err(DbError::Config(
            "Remote SurrealDB requires ENCRYPTION_KEY when PRODUCTION is set \
             (OAuth IDs, Nostr keys, DM content at rest)."
                .into(),
        )),
    }
}

/// Whether to run root bootstrap (migrations + ensure app user) on connect.
///
/// `SURREALDB_BOOTSTRAP=0` (or `false`/`no`/`off`) skips root entirely so the
/// process only connects as the app user — for app-only containers after init.
/// Default: bootstrap (backwards compatible with single-container install.sh).
fn should_bootstrap_from_env() -> bool {
    match std::env::var("SURREALDB_BOOTSTRAP") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "no" || v == "off")
        }
        Err(_) => true,
    }
}

/// Resolve database-scoped app credentials.
///
/// **Production:** `SURREALDB_APP_PASSWORD` is required, non-empty, and must not
/// equal the root password (no silent fallback).
///
/// **Non-production:** falls back to `root_pass` with a warning for local convenience.
fn resolve_app_credentials(root_pass: &str) -> DbResult<(String, String)> {
    let app_user =
        std::env::var("SURREALDB_APP_USER").unwrap_or_else(|_| "scuffed_app".to_string());
    if app_user == "root" {
        return Err(DbError::Config(
            "SURREALDB_APP_USER must not be 'root' in scoped mode".into(),
        ));
    }

    let app_pass_set = std::env::var("SURREALDB_APP_PASSWORD")
        .ok()
        .filter(|s| !s.is_empty());

    if scuffed_auth::is_production_env() {
        let app_pass = app_pass_set.ok_or_else(|| {
            DbError::Config(
                "SURREALDB_APP_PASSWORD is required in production scoped mode \
                 (must be set, non-empty, and different from SURREALDB_PASSWORD)."
                    .into(),
            )
        })?;
        if app_pass == root_pass {
            return Err(DbError::Config(
                "SURREALDB_APP_PASSWORD must not equal SURREALDB_PASSWORD (root) in production. \
                 Use a distinct app password (install.sh generates both)."
                    .into(),
            ));
        }
        return Ok((app_user, app_pass));
    }

    // Non-production: allow fallback / same password for local convenience.
    match app_pass_set {
        Some(p) => {
            if p == root_pass {
                tracing::warn!(
                    "SURREALDB_APP_PASSWORD equals root password — ok for local only; \
                     production requires a distinct app password"
                );
            }
            Ok((app_user, p))
        }
        None => {
            tracing::warn!(
                "SURREALDB_APP_PASSWORD unset — falling back to SURREALDB_PASSWORD (dev only)"
            );
            Ok((app_user, root_pass.to_string()))
        }
    }
}

fn remote_url_ns_db_from_env() -> DbResult<(String, String, String, String, String)> {
    let url = std::env::var("SURREALDB_URL").map_err(|_| {
        DbError::Config("SURREALDB_URL is required for remote database operations".into())
    })?;
    let root_user = std::env::var("SURREALDB_USER").unwrap_or_else(|_| "root".to_string());
    let root_pass = std::env::var("SURREALDB_PASSWORD").unwrap_or_else(|_| "root".to_string());
    let ns = std::env::var("SURREALDB_NS").unwrap_or_else(|_| "scuffed_crew".to_string());
    let db = std::env::var("SURREALDB_DB").unwrap_or_else(|_| "main".to_string());
    Ok((url, root_user, root_pass, ns, db))
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
            if scuffed_auth::is_production_env() {
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
    /// Prefer [`Self::connect_from_env`] which uses a scoped app user in production.
    pub async fn connect(
        url: &str,
        username: &str,
        password: &str,
        config: DbConfig,
    ) -> DbResult<Self> {
        check_credentials(username, password)?;
        if scuffed_auth::is_production_env() {
            tracing::warn!(
                "Connecting with SurrealDB root auth in production — prefer SURREALDB_AUTH_MODE=scoped"
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
        assert_safe_sql_ident(username, "database username")?;

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

    /// Ensure a database-level app user exists (must be connected as root in NS/DB).
    ///
    /// Uses **EDITOR** role (CRUD only). Schema migrations run as root before
    /// the app reconnects as this user — the app never holds OWNER/root for DEFINE.
    pub async fn ensure_database_app_user(
        client: &Surreal<Any>,
        username: &str,
        password: &str,
    ) -> DbResult<()> {
        assert_safe_sql_ident(username, "app database username")?;
        if password.is_empty() {
            return Err(DbError::Config(
                "App database password must not be empty".into(),
            ));
        }
        let pass = escape_surreal_string(password);
        // DEFINE USER does not accept bound password parameters reliably; username
        // is restricted to [A-Za-z0-9_], password is escaped.
        // OVERWRITE required: SurrealDB v3 plain DEFINE USER is create-only and
        // errors when the user exists (crash-looped prod on redeploy). OVERWRITE
        // also re-aligns the password to the env value on every boot.
        let q =
            format!("DEFINE USER OVERWRITE {username} ON DATABASE PASSWORD '{pass}' ROLES EDITOR");
        client.query(q).await?.check()?;
        tracing::info!(
            username,
            "Ensured database-scoped SurrealDB app user (EDITOR)"
        );
        Ok(())
    }

    /// Bootstrap remote DB as root: run schema migrations and ensure the EDITOR app user.
    ///
    /// Does **not** return a long-lived root client — drop the root session after init.
    ///
    /// Requires `SURREALDB_URL`, remote production policy (`PRODUCTION` + `ENCRYPTION_KEY`),
    /// root credentials (`SURREALDB_USER` / `SURREALDB_PASSWORD`), and app credentials
    /// (`SURREALDB_APP_USER` / `SURREALDB_APP_PASSWORD`).
    ///
    /// Use with `SURREALDB_MIGRATE_ONLY=1` on the server binary for init-only jobs, or via
    /// [`Self::connect_from_env`] when `SURREALDB_BOOTSTRAP` is not `0`.
    pub async fn bootstrap_from_env() -> DbResult<()> {
        assert_remote_production_policy()?;
        let (url, root_user, root_pass, ns, db_name) = remote_url_ns_db_from_env()?;
        let (app_user, app_pass) = resolve_app_credentials(&root_pass)?;
        let config = DbConfig {
            namespace: ns,
            database: db_name,
        };

        // Root session is intentionally scoped to this block and dropped after bootstrap.
        {
            let bootstrap = Self::connect(&url, &root_user, &root_pass, config).await?;
            crate::migrations::run_migrations(&bootstrap.client).await?;
            Self::ensure_database_app_user(&bootstrap.client, &app_user, &app_pass).await?;
        }

        tracing::info!(
            username = %app_user,
            "Database bootstrap complete (migrations + EDITOR app user); root session dropped"
        );
        Ok(())
    }

    /// Connect using environment variables.
    ///
    /// **Remote DB policy:** requires `PRODUCTION=1` and `ENCRYPTION_KEY`.
    ///
    /// **Scoped mode (default in production):**
    /// 1. Optionally bootstrap as root (migrations + ensure EDITOR app user) unless
    ///    `SURREALDB_BOOTSTRAP=0` (app-only containers after init).
    /// 2. Connect as the app user only (`SURREALDB_APP_USER` / `SURREALDB_APP_PASSWORD`).
    ///    The returned client never holds root credentials.
    ///
    /// **Root mode:** single root connection + migrations (dev only; warned in production).
    ///
    /// Callers should **not** re-run migrations after this for remote connections
    /// (app EDITOR cannot DEFINE). In-memory dev still runs migrations in main.
    pub async fn connect_from_env() -> DbResult<Self> {
        assert_remote_production_policy()?;
        let (url, root_user, root_pass, ns, db) = remote_url_ns_db_from_env()?;
        let config = DbConfig {
            namespace: ns.clone(),
            database: db.clone(),
        };

        match SurrealAuthMode::from_env() {
            SurrealAuthMode::Root => {
                if scuffed_auth::is_production_env() {
                    tracing::warn!(
                        "SURREALDB_AUTH_MODE=root in production — app holds full root credentials"
                    );
                }
                let conn = Self::connect(&url, &root_user, &root_pass, config).await?;
                crate::migrations::run_migrations(&conn.client).await?;
                Ok(conn)
            }
            SurrealAuthMode::Scoped => {
                let (app_user, app_pass) = resolve_app_credentials(&root_pass)?;

                if should_bootstrap_from_env() {
                    // Root migrate + ensure user; root client is not retained.
                    Self::bootstrap_from_env().await?;
                } else {
                    tracing::info!(
                        "SURREALDB_BOOTSTRAP=0 — skipping root bootstrap; \
                         connecting as app user only"
                    );
                }

                tracing::info!(
                    username = %app_user,
                    "Connecting to SurrealDB as database-scoped EDITOR app user (not root)"
                );
                Self::connect_scoped(&url, &ns, &db, &app_user, &app_pass).await
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

        let crypto = CryptoService::from_env()
            .map_err(|e| DbError::Config(format!("Crypto init failed: {e}")))?
            .map(Arc::new);

        Ok(Self { client, crypto })
    }
}

#[cfg(test)]
mod app_user_tests {
    use super::*;

    /// Redeploys hit an existing user: DEFINE USER must be idempotent
    /// (SurrealDB v3 requires OVERWRITE — plain DEFINE errors "already exists"
    /// and crash-looped production).
    #[tokio::test]
    async fn ensure_app_user_is_idempotent() {
        let db = Database::connect_memory().await.expect("mem db");
        Database::ensure_database_app_user(&db.client, "scuffed_app", "test-pass-1")
            .await
            .expect("first define");
        Database::ensure_database_app_user(&db.client, "scuffed_app", "test-pass-2")
            .await
            .expect("re-define with new password must not error");
    }
}
