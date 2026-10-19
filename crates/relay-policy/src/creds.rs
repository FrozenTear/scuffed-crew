//! SurrealDB credentials for the relay processes.
//!
//! Prefer `SURREALDB_RELAY_USER` / `SURREALDB_RELAY_PASSWORD`. That login is
//! a record user limited to the tables the relay reads. Outside production,
//! a missing relay password falls back to `SURREALDB_USER` /
//! `SURREALDB_PASSWORD` (`SURREALDB_PASS` is an alias). When `PRODUCTION=1`,
//! a missing or blank relay password is an error. There is no fallback to
//! `SURREALDB_USER` or `SURREALDB_PASSWORD`.

use scuffed_db::DbConfig;

const DEV_URL: &str = "ws://127.0.0.1:8000";
const DEV_USER: &str = "root";
const DEV_PASS: &str = "root";

/// How the relay signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayAuthKind {
    /// Record access `relay`. SELECT on the tables the relay reads.
    Record,
    /// Dev-only system user. `Database::connect_plaintext` signs in with
    /// `surrealdb::opt::auth::Root`. This is not a database role. Production
    /// never reaches this variant: it requires `SURREALDB_RELAY_PASSWORD`.
    System,
}

/// Resolved connection settings. The password is redacted in `Debug`.
#[derive(Clone)]
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
                 or blank. The relay does not fall back to SURREALDB_USER or SURREALDB_PASSWORD."
            ),
            CredError::RootUser => write!(
                f,
                "Refusing to start: PRODUCTION is set and SURREALDB_RELAY_USER is root. \
                 Set a non-root SURREALDB_RELAY_USER (the default is scuffed_relay)."
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
/// `SURREALDB_RELAY_USER` is set). Whitespace is trimmed on the user and the
/// password. Outside production, a missing relay password uses
/// `SURREALDB_USER` / `SURREALDB_PASSWORD` and [`RelayDbConfig::fallback`] is
/// true. A fully unset environment keeps the local defaults (`root` / `root`
/// and `ws://127.0.0.1:8000`). When `production` is true, the relay password
/// must be non-blank. There is no fallback, including a non-root system user.
/// Username `root` or a loopback URL is still an error.
pub fn resolve_relay_db_config(production: bool, env: &DbEnv) -> Result<RelayDbConfig, CredError> {
    if production {
        let password = nonempty(env.relay_password.as_deref()).ok_or(CredError::MissingPassword)?;
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

/// Startup warning when the dev fallback is still the legacy root password.
///
/// The message names that condition. It does not include the password value.
pub(crate) fn legacy_root_password_warning(cfg: &RelayDbConfig) -> Option<&'static str> {
    if cfg.fallback && cfg.password == DEV_PASS {
        Some(
            "relay database login is still the legacy root password. \
             Set SURREALDB_RELAY_PASSWORD. This process is using the dev fallback.",
        )
    } else {
        None
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
            if let Some(message) = legacy_root_password_warning(&cfg) {
                tracing::warn!("{message}");
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

const CONNECT_ATTEMPTS: u32 = 8;

/// Backoff after a failed attempt. Attempt 1 sleeps 500ms, then doubles, capped at 15s.
pub(crate) fn connect_delay(attempt: u32) -> std::time::Duration {
    let shift = attempt.saturating_sub(1).min(5);
    let millis = 500u64.saturating_mul(1u64 << shift);
    std::time::Duration::from_millis(millis.min(15_000))
}

/// Open the database, retrying with backoff. Exit 1 after the last failure
/// so the container restart policy connects again. Does not log the password.
pub async fn connect_or_retry(cfg: &RelayDbConfig) -> scuffed_db::Database {
    let mut attempt = 1u32;
    loop {
        match connect_configured(cfg).await {
            Ok(db) => return db,
            Err(err) => {
                tracing::warn!(attempt, error = %err, "database connect failed");
                if attempt >= CONNECT_ATTEMPTS {
                    eprintln!(
                        "error: database connect failed after {CONNECT_ATTEMPTS} attempts: {err}"
                    );
                    std::process::exit(1);
                }
                tokio::time::sleep(connect_delay(attempt)).await;
                attempt += 1;
            }
        }
    }
}

/// Open the database with the resolved login. Does not load `ENCRYPTION_KEY`.
///
/// The surrealdb 3.2 client stores this sign-in as a replayable command
/// (`engine/remote/mod.rs`, `record_replayable`) and sends it again after
/// every reconnect (`engine/remote/ws/mod.rs`, `replay_session`). The record
/// access omits `DURATION`, so the server default applies: token 1 hour,
/// session NONE. An open websocket does not expire. There is no periodic
/// re-sign-in. That timer repeated the argon2 check, and after a database
/// restart every relay sent the check at the same time.
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
        assert!(rendered.contains("SURREALDB_RELAY_PASSWORD"));
        assert!(rendered.contains("does not fall back to SURREALDB_USER"));
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
            CredError::MissingPassword
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
        assert_eq!(err, CredError::MissingPassword);
        assert!(err.to_string().contains("SURREALDB_RELAY_PASSWORD"));
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
            CredError::MissingPassword
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
    fn production_non_root_fallback_is_refused() {
        let env = DbEnv {
            user: Some("scuffed_legacy".into()),
            password: Some("legacy-secret".into()),
            relay_password: None,
            ..prod_base()
        };
        assert_eq!(
            resolve_relay_db_config(true, &env).unwrap_err(),
            CredError::MissingPassword
        );
    }

    #[test]
    fn production_only_relay_password_no_root_password() {
        let env = DbEnv {
            url: Some("ws://surrealdb:8000".into()),
            user: None,
            password: None,
            pass_alias: None,
            relay_user: None,
            relay_password: Some("only-relay".into()),
        };
        let cfg = resolve_relay_db_config(true, &env).unwrap();
        assert_eq!(cfg.auth, RelayAuthKind::Record);
        assert!(!cfg.fallback);
        assert_eq!(cfg.username, "scuffed_relay");
        assert_eq!(cfg.password, "only-relay");
        assert_ne!(cfg.password, "root");
    }

    #[test]
    fn production_trims_relay_password_and_rejects_blank() {
        let trimmed = DbEnv {
            relay_password: Some("  relay-secret  ".into()),
            relay_user: Some("  scuffed_relay  ".into()),
            ..prod_relay()
        };
        let cfg = resolve_relay_db_config(true, &trimmed).unwrap();
        assert_eq!(cfg.password, "relay-secret");
        assert_eq!(cfg.username, "scuffed_relay");
        let blank = DbEnv {
            relay_password: Some("   ".into()),
            password: Some("generated-secret".into()),
            ..prod_base()
        };
        assert_eq!(
            resolve_relay_db_config(true, &blank).unwrap_err(),
            CredError::MissingPassword
        );
    }

    #[test]
    fn connect_delay_backs_off_and_caps() {
        assert_eq!(connect_delay(1), std::time::Duration::from_millis(500));
        assert_eq!(connect_delay(2), std::time::Duration::from_secs(1));
        assert_eq!(connect_delay(3), std::time::Duration::from_secs(2));
        assert_eq!(connect_delay(6), std::time::Duration::from_secs(15));
        assert_eq!(connect_delay(8), std::time::Duration::from_secs(15));
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
    fn legacy_root_fallback_warns_without_the_password_value() {
        let cfg = resolve_relay_db_config(false, &DbEnv::default()).unwrap();
        let message = legacy_root_password_warning(&cfg).expect("root fallback warns");
        assert!(message.contains("legacy root password"));
        assert!(message.contains("SURREALDB_RELAY_PASSWORD"));
        assert!(
            !message.contains("not-the-default"),
            "the warning must be a fixed sentence, not the configured password"
        );

        let custom = DbEnv {
            url: Some("ws://127.0.0.1:8000".into()),
            user: Some("editor".into()),
            password: Some("not-the-default".into()),
            ..DbEnv::default()
        };
        let custom = resolve_relay_db_config(false, &custom).unwrap();
        assert!(custom.fallback);
        assert!(legacy_root_password_warning(&custom).is_none());

        let record = DbEnv {
            relay_password: Some("relay-secret".into()),
            ..DbEnv::default()
        };
        let record = resolve_relay_db_config(false, &record).unwrap();
        assert!(legacy_root_password_warning(&record).is_none());
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
