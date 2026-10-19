//! SurrealDB credentials for the relay processes.
//!
//! Prefer `SURREALDB_RELAY_USER` / `SURREALDB_RELAY_PASSWORD`. That login is
//! a record user limited to the tables the relay reads. `SURREALDB_PASSWORD`
//! (and `SURREALDB_PASS`) is only a fallback when the relay secret is missing
//! or blank. `PRODUCTION=1` refuses to sign in as `root`.

use scuffed_db::DbConfig;

const DEV_URL: &str = "ws://127.0.0.1:8000";
const DEV_USER: &str = "root";
const DEV_PASS: &str = "root";

/// How the relay signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayAuthKind {
    /// Record access `relay`. SELECT on the tables the relay reads.
    Record,
    /// System user (`Root` or database role). Dev fallback only when the
    /// relay secret is unset. Production still refuses username `root`.
    System,
}

/// Resolved connection settings. The password is redacted in `Debug`.
pub struct RelayDbConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    pub auth: RelayAuthKind,
    /// True when `SURREALDB_RELAY_PASSWORD` was missing and the process used
    /// `SURREALDB_USER` / `SURREALDB_PASSWORD` instead.
    pub fallback: bool,
}

impl std::fmt::Debug for RelayDbConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayDbConfig")
            .field("url", &self.url)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("auth", &self.auth)
            .field("fallback", &self.fallback)
            .finish()
    }
}

/// Why a production relay must not open a database connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredError {
    /// No relay password and no fallback password.
    MissingPassword,
    /// The username that would be used is `root`.
    RootUser,
    /// `SURREALDB_URL` is unset, blank, or points at a loopback host.
    LocalUrl,
}

impl std::fmt::Display for CredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredError::MissingPassword => write!(
                f,
                "Refusing to start: PRODUCTION is set and SURREALDB_RELAY_PASSWORD is missing \
                 or blank, and SURREALDB_PASSWORD is also missing or blank \
                 (SURREALDB_PASS is only a fallback alias). \
                 The relay will not fall back to root/root."
            ),
            CredError::RootUser => write!(
                f,
                "Refusing to start: PRODUCTION is set and the relay would sign in as the \
                 SurrealDB root user. Set SURREALDB_RELAY_PASSWORD \
                 (scripts/install.sh and scripts/update.sh append one when the key is missing). \
                 The relay does not sign in as root, including root/root."
            ),
            CredError::LocalUrl => write!(
                f,
                "Refusing to start: PRODUCTION is set and SURREALDB_URL is missing, blank, \
                 or a loopback host (localhost, 127.0.0.1, ::1). \
                 The relay will not fall back to ws://127.0.0.1:8000. \
                 Compose sets SURREALDB_URL=ws://surrealdb:8000."
            ),
        }
    }
}

impl std::error::Error for CredError {}

/// Process environment viewed as optional strings. Tests build this directly
/// so they do not mutate process-global env vars.
#[derive(Debug, Clone, Default)]
pub struct DbEnv {
    pub url: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub pass_alias: Option<String>,
    pub relay_user: Option<String>,
    pub relay_password: Option<String>,
}

impl DbEnv {
    pub fn from_process() -> Self {
        Self {
            url: std::env::var("SURREALDB_URL").ok(),
            user: std::env::var("SURREALDB_USER").ok(),
            password: std::env::var("SURREALDB_PASSWORD").ok(),
            pass_alias: std::env::var("SURREALDB_PASS").ok(),
            relay_user: std::env::var("SURREALDB_RELAY_USER").ok(),
            relay_password: std::env::var("SURREALDB_RELAY_PASSWORD").ok(),
        }
    }
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `SURREALDB_PASSWORD`, then `SURREALDB_PASS`. Blank values are skipped.
pub fn password_from_env(env: &DbEnv) -> Option<String> {
    nonempty(env.password.as_deref()).or_else(|| nonempty(env.pass_alias.as_deref()))
}

/// True when the URL host is missing or loopback.
///
/// An unset URL is local because the historical default is `ws://127.0.0.1:8000`.
pub fn url_is_local(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return true;
    }
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let hostport = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        hostport
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(hostport)
    };
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    matches!(
        host.as_str(),
        "" | "localhost" | "127.0.0.1" | "::1" | "0.0.0.0"
    )
}

fn finish(
    production: bool,
    env: &DbEnv,
    username: String,
    password: String,
    auth: RelayAuthKind,
    fallback: bool,
) -> Result<RelayDbConfig, CredError> {
    if production {
        if username == DEV_USER {
            return Err(CredError::RootUser);
        }
        let url = nonempty(env.url.as_deref()).ok_or(CredError::LocalUrl)?;
        if url_is_local(&url) {
            return Err(CredError::LocalUrl);
        }
        return Ok(RelayDbConfig {
            url,
            username,
            password,
            auth,
            fallback,
        });
    }

    Ok(RelayDbConfig {
        url: nonempty(env.url.as_deref()).unwrap_or_else(|| DEV_URL.to_string()),
        username,
        password,
        auth,
        fallback,
    })
}

/// Resolve the database URL, user, password, and sign-in kind.
///
/// `SURREALDB_RELAY_PASSWORD` selects the record login (`scuffed_relay` unless
/// `SURREALDB_RELAY_USER` is set). When that secret is missing, the relay uses
/// `SURREALDB_USER` / `SURREALDB_PASSWORD` and [`RelayDbConfig::fallback`] is
/// true. Outside production, missing values keep the local defaults (`root` /
/// `root` and `ws://127.0.0.1:8000`). When `production` is true, a missing
/// password, username `root`, or a loopback URL is an error before any default
/// password is applied.
pub fn resolve_relay_db_config(production: bool, env: &DbEnv) -> Result<RelayDbConfig, CredError> {
    if let Some(password) = nonempty(env.relay_password.as_deref()) {
        let username = nonempty(env.relay_user.as_deref())
            .unwrap_or_else(|| scuffed_db::RELAY_DB_USERNAME_DEFAULT.to_string());
        return finish(
            production,
            env,
            username,
            password,
            RelayAuthKind::Record,
            false,
        );
    }

    let username = nonempty(env.user.as_deref()).unwrap_or_else(|| DEV_USER.to_string());
    let Some(password) = password_from_env(env) else {
        if production {
            return Err(CredError::MissingPassword);
        }
        return finish(
            production,
            env,
            username,
            DEV_PASS.to_string(),
            RelayAuthKind::System,
            true,
        );
    };
    finish(
        production,
        env,
        username,
        password,
        RelayAuthKind::System,
        true,
    )
}

/// Namespace and database name from the process environment.
pub fn namespace_from_env() -> DbConfig {
    DbConfig {
        namespace: nonempty(std::env::var("SURREALDB_NS").ok().as_deref())
            .unwrap_or_else(|| "scuffed_crew".to_string()),
        database: nonempty(std::env::var("SURREALDB_DB").ok().as_deref())
            .unwrap_or_else(|| "main".to_string()),
    }
}

/// Log a clear refusal and exit 1. Call this after tracing is initialized.
pub fn enforce_or_exit() -> RelayDbConfig {
    match resolve_relay_db_config(scuffed_db::is_production_env(), &DbEnv::from_process()) {
        Ok(cfg) => {
            if cfg.fallback {
                tracing::warn!(
                    user = %cfg.username,
                    "SURREALDB_RELAY_PASSWORD is unset. Using the fallback database user. \
                     PRODUCTION refuses this when that user is root. \
                     scripts/install.sh and scripts/update.sh append SURREALDB_RELAY_PASSWORD \
                     when the key is missing."
                );
            }
            cfg
        }
        Err(err) => {
            tracing::error!("{err}");
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}

/// Open the database with the resolved login. Does not load `ENCRYPTION_KEY`.
pub async fn connect_configured(cfg: &RelayDbConfig) -> scuffed_db::DbResult<scuffed_db::Database> {
    let surreal = namespace_from_env();
    match cfg.auth {
        RelayAuthKind::Record => {
            scuffed_db::Database::connect_relay_record(
                &cfg.url,
                &surreal.namespace,
                &surreal.database,
                &cfg.username,
                &cfg.password,
            )
            .await
        }
        RelayAuthKind::System => {
            scuffed_db::Database::connect_plaintext(&cfg.url, &cfg.username, &cfg.password, surreal)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prod_base() -> DbEnv {
        DbEnv {
            url: Some("ws://surrealdb:8000".into()),
            user: Some("root".into()),
            password: Some("generated-secret".into()),
            pass_alias: None,
            relay_user: None,
            relay_password: None,
        }
    }

    fn prod_relay() -> DbEnv {
        DbEnv {
            relay_user: Some("scuffed_relay".into()),
            relay_password: Some("relay-secret".into()),
            user: Some("root".into()),
            password: Some("generated-secret".into()),
            ..prod_base()
        }
    }

    #[test]
    fn production_missing_password_rejects_instead_of_root_root() {
        let env = DbEnv {
            url: Some("ws://surrealdb:8000".into()),
            user: None,
            password: None,
            pass_alias: None,
            relay_user: None,
            relay_password: None,
        };
        let err = resolve_relay_db_config(true, &env).expect_err("missing password must reject");
        assert_eq!(err, CredError::MissingPassword);
        let rendered = err.to_string();
        assert!(rendered.contains("SURREALDB_PASSWORD"));
        assert!(rendered.contains("will not fall back to root/root"));
        // The old plugin did `SURREALDB_PASS` default `"root"` plus user `root`.
        // That pair must not come out of the production resolver.
        assert!(resolve_relay_db_config(true, &env).ok().is_none());
    }

    #[test]
    fn production_blank_password_and_blank_alias_reject() {
        let env = DbEnv {
            url: Some("ws://surrealdb:8000".into()),
            user: Some("root".into()),
            password: Some("  ".into()),
            pass_alias: Some("".into()),
            relay_user: None,
            relay_password: None,
        };
        assert_eq!(
            resolve_relay_db_config(true, &env).unwrap_err(),
            CredError::MissingPassword
        );
    }

    #[test]
    fn production_pass_alias_does_not_sign_in_as_root() {
        let env = DbEnv {
            url: Some("ws://surrealdb:8000".into()),
            user: Some("root".into()),
            password: None,
            pass_alias: Some("alias-secret".into()),
            relay_user: None,
            relay_password: None,
        };
        assert_eq!(
            resolve_relay_db_config(true, &env).unwrap_err(),
            CredError::RootUser
        );
    }

    #[test]
    fn dev_password_wins_over_pass_alias_on_fallback() {
        let env = DbEnv {
            password: Some("primary-secret".into()),
            pass_alias: Some("alias-secret".into()),
            user: Some("scuffed_legacy".into()),
            ..prod_base()
        };
        let cfg = resolve_relay_db_config(false, &env).unwrap();
        assert_eq!(cfg.password, "primary-secret");
        assert!(cfg.fallback);
        assert_eq!(cfg.auth, RelayAuthKind::System);
    }

    #[test]
    fn dev_blank_password_falls_through_to_alias() {
        let env = DbEnv {
            password: Some("   ".into()),
            pass_alias: Some("from-alias".into()),
            user: Some("scuffed_legacy".into()),
            ..prod_base()
        };
        let cfg = resolve_relay_db_config(false, &env).unwrap();
        assert_eq!(cfg.password, "from-alias");
        assert!(cfg.fallback);
    }

    #[test]
    fn production_root_with_strong_password_rejected() {
        let env = DbEnv {
            password: Some("generated-secret".into()),
            pass_alias: Some("not-used".into()),
            ..prod_base()
        };
        let err = resolve_relay_db_config(true, &env).unwrap_err();
        assert_eq!(err, CredError::RootUser);
        assert!(err.to_string().contains("root/root"));
    }

    #[test]
    fn production_unset_user_with_password_root_is_root() {
        let env = DbEnv {
            url: Some("ws://surrealdb:8000".into()),
            user: None,
            password: Some("root".into()),
            pass_alias: None,
            relay_user: None,
            relay_password: None,
        };
        assert_eq!(
            resolve_relay_db_config(true, &env).unwrap_err(),
            CredError::RootUser
        );
    }

    #[test]
    fn production_relay_password_uses_record_login() {
        let cfg = resolve_relay_db_config(true, &prod_relay()).unwrap();
        assert_eq!(cfg.auth, RelayAuthKind::Record);
        assert!(!cfg.fallback);
        assert_eq!(cfg.username, "scuffed_relay");
        assert_eq!(cfg.password, "relay-secret");
        assert_ne!(cfg.password, "generated-secret");
        let rendered = format!("{cfg:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("relay-secret"));
        assert!(!rendered.contains("generated-secret"));
    }

    #[test]
    fn production_relay_user_named_root_rejected() {
        let env = DbEnv {
            relay_user: Some("root".into()),
            relay_password: Some("relay-secret".into()),
            ..prod_relay()
        };
        assert_eq!(
            resolve_relay_db_config(true, &env).unwrap_err(),
            CredError::RootUser
        );
    }

    #[test]
    fn production_non_root_fallback_keeps_working() {
        let env = DbEnv {
            user: Some("scuffed_legacy".into()),
            password: Some("legacy-secret".into()),
            relay_password: None,
            ..prod_base()
        };
        let cfg = resolve_relay_db_config(true, &env).unwrap();
        assert!(cfg.fallback);
        assert_eq!(cfg.auth, RelayAuthKind::System);
        assert_eq!(cfg.username, "scuffed_legacy");
        assert_eq!(cfg.password, "legacy-secret");
    }

    #[test]
    fn production_missing_url_rejected() {
        let env = DbEnv {
            url: None,
            ..prod_relay()
        };
        assert_eq!(
            resolve_relay_db_config(true, &env).unwrap_err(),
            CredError::LocalUrl
        );
    }

    #[test]
    fn production_loopback_urls_rejected() {
        for url in [
            "ws://127.0.0.1:8000",
            "ws://localhost:8000",
            "ws://LOCALHOST:8000",
            "http://[::1]:8000",
            "ws://0.0.0.0:8000",
            "   ",
        ] {
            let env = DbEnv {
                url: Some(url.into()),
                ..prod_relay()
            };
            assert_eq!(
                resolve_relay_db_config(true, &env).unwrap_err(),
                CredError::LocalUrl,
                "{url}"
            );
        }
    }

    #[test]
    fn production_compose_relay_secret_accepted() {
        let cfg = resolve_relay_db_config(true, &prod_relay()).unwrap();
        assert_eq!(cfg.url, "ws://surrealdb:8000");
        assert_eq!(cfg.username, "scuffed_relay");
        assert_eq!(cfg.password, "relay-secret");
        assert_eq!(cfg.auth, RelayAuthKind::Record);
    }

    #[test]
    fn dev_missing_values_keep_local_defaults() {
        let cfg = resolve_relay_db_config(false, &DbEnv::default()).unwrap();
        assert_eq!(cfg.url, "ws://127.0.0.1:8000");
        assert_eq!(cfg.username, "root");
        assert_eq!(cfg.password, "root");
        assert!(cfg.fallback);
        assert_eq!(cfg.auth, RelayAuthKind::System);
    }

    #[test]
    fn dev_relay_password_prefers_record_login() {
        let env = DbEnv {
            relay_password: Some("relay-secret".into()),
            ..DbEnv::default()
        };
        let cfg = resolve_relay_db_config(false, &env).unwrap();
        assert_eq!(cfg.auth, RelayAuthKind::Record);
        assert!(!cfg.fallback);
        assert_eq!(cfg.username, "scuffed_relay");
        assert_eq!(cfg.password, "relay-secret");
    }
}
