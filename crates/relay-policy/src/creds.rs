//! SurrealDB credentials for the relay processes.
//!
//! `SURREALDB_PASSWORD` is the primary name (the same one Compose and
//! `scripts/install.sh` write). `SURREALDB_PASS` is accepted only as a
//! fallback alias when the primary value is missing or blank.

use scuffed_db::DbConfig;

const DEV_URL: &str = "ws://127.0.0.1:8000";
const DEV_USER: &str = "root";
const DEV_PASS: &str = "root";

/// Resolved connection settings. The password is redacted in `Debug`.
pub struct RelayDbConfig {
    pub url: String,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for RelayDbConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayDbConfig")
            .field("url", &self.url)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Why a production relay must not open a database connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredError {
    /// `SURREALDB_PASSWORD` and `SURREALDB_PASS` are both unset or blank.
    MissingPassword,
    /// Username and password are both `root`.
    DefaultRoot,
    /// `SURREALDB_URL` is unset, blank, or points at a loopback host.
    LocalUrl,
}

impl std::fmt::Display for CredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredError::MissingPassword => write!(
                f,
                "Refusing to start: PRODUCTION is set and SURREALDB_PASSWORD is missing \
                 or blank (SURREALDB_PASS is only a fallback alias). \
                 The relay will not fall back to root/root."
            ),
            CredError::DefaultRoot => write!(
                f,
                "Refusing to start: PRODUCTION is set and SurrealDB credentials are root/root. \
                 Set a strong SURREALDB_PASSWORD (scripts/install.sh generates one)."
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
}

impl DbEnv {
    pub fn from_process() -> Self {
        Self {
            url: std::env::var("SURREALDB_URL").ok(),
            user: std::env::var("SURREALDB_USER").ok(),
            password: std::env::var("SURREALDB_PASSWORD").ok(),
            pass_alias: std::env::var("SURREALDB_PASS").ok(),
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

/// Resolve the database URL, user, and password.
///
/// Outside production, missing values keep the local defaults (`root` / `root`
/// and `ws://127.0.0.1:8000`) so a developer can run the plugin without Compose.
/// When `production` is true, a missing password, root/root, or a loopback URL
/// is an error. The error is returned before any default password is applied.
pub fn resolve_relay_db_config(production: bool, env: &DbEnv) -> Result<RelayDbConfig, CredError> {
    let username = nonempty(env.user.as_deref()).unwrap_or_else(|| DEV_USER.to_string());
    let password = password_from_env(env);

    if production {
        let password = password.ok_or(CredError::MissingPassword)?;
        if username == DEV_USER && password == DEV_PASS {
            return Err(CredError::DefaultRoot);
        }
        let url = nonempty(env.url.as_deref()).ok_or(CredError::LocalUrl)?;
        if url_is_local(&url) {
            return Err(CredError::LocalUrl);
        }
        return Ok(RelayDbConfig {
            url,
            username,
            password,
        });
    }

    Ok(RelayDbConfig {
        url: nonempty(env.url.as_deref()).unwrap_or_else(|| DEV_URL.to_string()),
        username,
        password: password.unwrap_or_else(|| DEV_PASS.to_string()),
    })
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
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::error!("{err}");
            eprintln!("error: {err}");
            std::process::exit(1);
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
        }
    }

    #[test]
    fn production_missing_password_rejects_instead_of_root_root() {
        let env = DbEnv {
            url: Some("ws://surrealdb:8000".into()),
            user: None,
            password: None,
            pass_alias: None,
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
        };
        assert_eq!(
            resolve_relay_db_config(true, &env).unwrap_err(),
            CredError::MissingPassword
        );
    }

    #[test]
    fn production_pass_alias_used_when_password_missing() {
        let env = DbEnv {
            url: Some("ws://surrealdb:8000".into()),
            user: Some("root".into()),
            password: None,
            pass_alias: Some("alias-secret".into()),
        };
        let cfg = resolve_relay_db_config(true, &env).unwrap();
        assert_eq!(cfg.password, "alias-secret");
        assert_ne!(cfg.password, "root");
        assert_eq!(cfg.username, "root");
    }

    #[test]
    fn production_password_wins_over_pass_alias() {
        let env = DbEnv {
            password: Some("primary-secret".into()),
            pass_alias: Some("alias-secret".into()),
            ..prod_base()
        };
        let cfg = resolve_relay_db_config(true, &env).unwrap();
        assert_eq!(cfg.password, "primary-secret");
    }

    #[test]
    fn production_blank_password_falls_through_to_alias() {
        let env = DbEnv {
            password: Some("   ".into()),
            pass_alias: Some("from-alias".into()),
            ..prod_base()
        };
        let cfg = resolve_relay_db_config(true, &env).unwrap();
        assert_eq!(cfg.password, "from-alias");
    }

    #[test]
    fn production_root_root_rejected() {
        let env = DbEnv {
            password: Some("root".into()),
            pass_alias: Some("not-used".into()),
            ..prod_base()
        };
        assert_eq!(
            resolve_relay_db_config(true, &env).unwrap_err(),
            CredError::DefaultRoot
        );
    }

    #[test]
    fn production_unset_user_with_password_root_is_root_root() {
        let env = DbEnv {
            url: Some("ws://surrealdb:8000".into()),
            user: None,
            password: Some("root".into()),
            pass_alias: None,
        };
        assert_eq!(
            resolve_relay_db_config(true, &env).unwrap_err(),
            CredError::DefaultRoot
        );
    }

    #[test]
    fn production_missing_url_rejected() {
        let env = DbEnv {
            url: None,
            ..prod_base()
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
                ..prod_base()
            };
            assert_eq!(
                resolve_relay_db_config(true, &env).unwrap_err(),
                CredError::LocalUrl,
                "{url}"
            );
        }
    }

    #[test]
    fn production_compose_url_and_password_accepted() {
        let cfg = resolve_relay_db_config(true, &prod_base()).unwrap();
        assert_eq!(cfg.url, "ws://surrealdb:8000");
        assert_eq!(cfg.username, "root");
        assert_eq!(cfg.password, "generated-secret");
    }

    #[test]
    fn dev_missing_values_keep_local_defaults() {
        let cfg = resolve_relay_db_config(false, &DbEnv::default()).unwrap();
        assert_eq!(cfg.url, "ws://127.0.0.1:8000");
        assert_eq!(cfg.username, "root");
        assert_eq!(cfg.password, "root");
    }
}
