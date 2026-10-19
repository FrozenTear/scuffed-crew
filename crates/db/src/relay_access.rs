//! Least-privilege database login for the relay.
//!
//! SurrealDB 3.2 `DEFINE USER` roles are database-wide. `VIEWER` can `SELECT`
//! every table, including `nostr_secret_key_encrypted`. Record access is the
//! mechanism that actually limits a login to specific tables.
//!
//! The relay process (`relay-policy` and `relay-read-gate`) only reads:
//!
//! - `member` (`list_nostr_identities`, `LIVE SELECT id`, and `in.is_active`
//!   on roster edges)
//! - `plays_on` (`list_active_roster_pairs`, `LIVE SELECT id`)
//! - `team_channel` (`list_all_team_channels`, `LIVE SELECT id`)
//! - `moderation_action` (the active-ban subquery inside
//!   `list_active_roster_pairs`)
//!
//! It does not write. `member.nostr_secret_key_encrypted`,
//! `moderation_action.reason`, and `moderation_action.issued_by` are not
//! selected by those queries. Field permissions hide them from this login
//! anyway. System users (root bootstrap and the EDITOR app user) bypass
//! table and field permissions, so the site keeps its current access.

use surrealdb::engine::any::{self, Any};
use surrealdb::opt::auth::Record;
use surrealdb::Surreal;
use surrealdb_types::SurrealValue;

use crate::{Database, DbError, DbResult};

/// Record access name. Sign-in uses this with `SURREALDB_RELAY_USER`.
pub const RELAY_DB_ACCESS: &str = "relay";

/// Default `SURREALDB_RELAY_USER`. Not `root`.
pub const RELAY_DB_USERNAME_DEFAULT: &str = "scuffed_relay";

/// Table that holds the single relay principal. Record users cannot read it.
pub const RELAY_PRINCIPAL_TABLE: &str = "relay_db_user";

/// Tables the relay login may `SELECT`. Nothing else.
pub const RELAY_SELECT_TABLES: &[&str] =
    &["member", "plays_on", "team_channel", "moderation_action"];

/// Sign-in parameters. Not `Debug`: `pass` is the relay password.
#[derive(SurrealValue)]
struct RelaySigninParams {
    user: String,
    pass: String,
}

fn assert_safe_ident(name: &str, what: &str) -> DbResult<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(DbError::Config(format!(
            "Invalid {what}: use only ASCII alphanumeric and underscore"
        )));
    }
    Ok(())
}

/// SurrealQL that grants the relay login its read permissions.
///
/// Table names come from [`RELAY_SELECT_TABLES`], which is a fixed list in
/// this file, not request input.
pub fn relay_permission_sql() -> String {
    let mut sql = String::from(
        "DEFINE TABLE IF NOT EXISTS relay_db_user SCHEMAFULL PERMISSIONS NONE;\n\
         DEFINE FIELD OVERWRITE username ON relay_db_user TYPE string;\n\
         DEFINE FIELD OVERWRITE pass ON relay_db_user TYPE string\n\
             PERMISSIONS FOR select NONE;\n\
         DEFINE INDEX IF NOT EXISTS relay_db_user_username_idx ON relay_db_user COLUMNS username UNIQUE;\n",
    );
    for table in RELAY_SELECT_TABLES {
        assert_safe_ident(table, "relay select table").expect("relay table list");
        sql.push_str(&format!(
            "ALTER TABLE {table} PERMISSIONS \
             FOR select WHERE meta::tb($auth.id) = '{RELAY_PRINCIPAL_TABLE}' \
             FOR create NONE \
             FOR update NONE \
             FOR delete NONE;\n"
        ));
    }
    sql.push_str(
        "DEFINE FIELD OVERWRITE nostr_secret_key_encrypted ON member TYPE option<object> FLEXIBLE \
             PERMISSIONS FOR select NONE;\n\
         DEFINE FIELD OVERWRITE reason ON moderation_action TYPE string \
             PERMISSIONS FOR select NONE;\n\
         DEFINE FIELD OVERWRITE issued_by ON moderation_action TYPE string \
             PERMISSIONS FOR select NONE;\n",
    );
    sql
}

/// Record access cannot use `DURATION FOR TOKEN NONE`. The relay signs in
/// once when the process starts and keeps that token. Ten years is long
/// enough that a normal process lifetime does not expire it. A restart
/// signs in again. `FOR SESSION NONE` keeps the server session from expiring.
const DEFINE_RELAY_ACCESS: &str = "\
DEFINE ACCESS OVERWRITE relay ON DATABASE TYPE RECORD \
    SIGNIN ( \
        SELECT id, username FROM relay_db_user \
        WHERE username = $user AND crypto::argon2::compare(pass, $pass) \
    ) \
    DURATION FOR TOKEN 3650d, FOR SESSION NONE";

/// Install table and field permissions. Idempotent. Does not create the login.
pub async fn apply_relay_read_permissions(client: &Surreal<Any>) -> DbResult<()> {
    client.query(relay_permission_sql()).await?.check()?;
    Ok(())
}

/// Create or refresh the relay record login.
///
/// When `force_sync` is false and the principal already exists, the stored
/// password is left alone. `DEFINE ACCESS` still runs so the sign-in query
/// stays current. Set `force_sync` (env `FORCE_RELAY_PASSWORD_SYNC=1` on the
/// server) only when rotating `SURREALDB_RELAY_PASSWORD`.
///
/// The password is a bound parameter. It is not interpolated and it is not logged.
pub async fn ensure_relay_access(
    client: &Surreal<Any>,
    username: &str,
    password: &str,
    force_sync: bool,
) -> DbResult<()> {
    assert_safe_ident(username, "relay database username")?;
    if username == "root" {
        return Err(DbError::Config(
            "SURREALDB_RELAY_USER must not be root".into(),
        ));
    }
    if password.is_empty() {
        return Err(DbError::Config(
            "Relay database password must not be empty".into(),
        ));
    }

    apply_relay_read_permissions(client).await?;
    client.query(DEFINE_RELAY_ACCESS).await?.check()?;

    let exists = relay_principal_exists(client, username).await?;
    if exists && !force_sync {
        tracing::info!(
            username,
            access = RELAY_DB_ACCESS,
            "Relay database user already exists; leaving password untouched \
             (set FORCE_RELAY_PASSWORD_SYNC=1 to rotate)"
        );
        return Ok(());
    }

    let statement = if exists {
        "UPDATE relay_db_user SET pass = crypto::argon2::generate($pass) WHERE username = $user"
    } else {
        "CREATE relay_db_user SET username = $user, pass = crypto::argon2::generate($pass)"
    };
    client
        .query(statement)
        .bind(("user", username.to_string()))
        .bind(("pass", password.to_string()))
        .await?
        .check()?;
    tracing::info!(
        username,
        access = RELAY_DB_ACCESS,
        force_sync,
        "Defined relay database user (SELECT on member, plays_on, team_channel, moderation_action)"
    );
    Ok(())
}

async fn relay_principal_exists(client: &Surreal<Any>, username: &str) -> DbResult<bool> {
    #[derive(serde::Deserialize, SurrealValue)]
    struct NameRow {
        username: String,
    }

    let mut response = client
        .query("SELECT username FROM relay_db_user WHERE username = $user LIMIT 1")
        .bind(("user", username.to_string()))
        .await?
        .check()?;
    let rows: Vec<NameRow> = response.take(0)?;
    Ok(!rows.is_empty())
}

/// Create the relay login from process env when the secret is present.
///
/// A missing `SURREALDB_RELAY_PASSWORD` logs a warning and returns. Site boot
/// must keep working for a deployment that has not received the new secret yet.
/// `scripts/install.sh` and `scripts/update.sh` append the secret when the key
/// is missing, so the next boot creates the user.
pub async fn ensure_relay_from_env(client: &Surreal<Any>, root_pass: &str) -> DbResult<()> {
    let username = std::env::var("SURREALDB_RELAY_USER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| RELAY_DB_USERNAME_DEFAULT.to_string());
    let password = std::env::var("SURREALDB_RELAY_PASSWORD")
        .ok()
        .filter(|value| !value.is_empty());
    let Some(password) = password else {
        tracing::warn!(
            "SURREALDB_RELAY_PASSWORD is unset. The relay database user was not created. \
             The relay falls back to its configured database user until this secret is set \
             and site-server starts again. scripts/install.sh and scripts/update.sh append \
             the key when it is missing."
        );
        return Ok(());
    };
    if crate::is_production_env() && password == root_pass {
        return Err(DbError::Config(
            "SURREALDB_RELAY_PASSWORD must not equal SURREALDB_PASSWORD in production. \
             Use a distinct password (install.sh generates one)."
                .into(),
        ));
    }
    if let Ok(app_pass) = std::env::var("SURREALDB_APP_PASSWORD") {
        if crate::is_production_env() && !app_pass.is_empty() && password == app_pass {
            return Err(DbError::Config(
                "SURREALDB_RELAY_PASSWORD must not equal SURREALDB_APP_PASSWORD in production. \
                 Use a distinct password (install.sh generates one)."
                    .into(),
            ));
        }
    }
    if !crate::is_production_env() && password == root_pass {
        tracing::warn!(
            "SURREALDB_RELAY_PASSWORD equals the root password. Production refuses that."
        );
    }
    let force_sync = std::env::var("FORCE_RELAY_PASSWORD_SYNC").ok().as_deref() == Some("1");
    ensure_relay_access(client, &username, &password, force_sync).await
}

async fn sign_in_relay_client(
    client: &Surreal<Any>,
    namespace: &str,
    database: &str,
    username: &str,
    password: &str,
) -> DbResult<()> {
    assert_safe_ident(username, "relay database username")?;
    if password.is_empty() {
        return Err(DbError::Config(
            "Relay database password must not be empty".into(),
        ));
    }
    client
        .signin(Record {
            namespace: namespace.to_string(),
            database: database.to_string(),
            access: RELAY_DB_ACCESS.to_string(),
            params: RelaySigninParams {
                user: username.to_string(),
                pass: password.to_string(),
            },
        })
        .await?;
    client.use_ns(namespace).use_db(database).await?;
    Ok(())
}

impl Database {
    /// Sign in as the relay record user. Does not load `ENCRYPTION_KEY`.
    pub async fn connect_relay_record(
        url: &str,
        namespace: &str,
        database: &str,
        username: &str,
        password: &str,
    ) -> DbResult<Self> {
        let client = any::connect(url).await?;
        sign_in_relay_client(&client, namespace, database, username, password).await?;
        Ok(Self {
            client,
            crypto: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::opt::auth::Database as DatabaseAuth;

    fn mem_url(name: &str) -> String {
        format!("mem://{name}")
    }

    async fn owner(name: &str) -> Database {
        let client = any::connect(mem_url(name)).await.expect("mem connect");
        client
            .use_ns("scuffed_crew")
            .use_db("main")
            .await
            .expect("ns");
        Database {
            client,
            crypto: None,
        }
    }

    /// Sign in on a cloned session. `mem://` opens a fresh store per connection,
    /// so a second `connect` cannot see the namespace this owner just created.
    async fn relay_session(owner: &Database, password: &str) -> DbResult<Database> {
        let client = owner.client.clone();
        sign_in_relay_client(&client, "scuffed_crew", "main", "scuffed_relay", password).await?;
        Ok(Database {
            client,
            crypto: None,
        })
    }

    #[test]
    fn permission_sql_names_only_the_tables_the_relay_reads() {
        let sql = relay_permission_sql();
        for table in RELAY_SELECT_TABLES {
            assert!(
                sql.contains(&format!("ALTER TABLE {table} PERMISSIONS")),
                "{table} missing from {sql}"
            );
            assert!(sql.contains("FOR create NONE"), "{sql}");
            assert!(sql.contains("FOR update NONE"), "{sql}");
            assert!(sql.contains("FOR delete NONE"), "{sql}");
        }
        for table in [
            "user",
            "session",
            "dm_message",
            "audit_log",
            "strategy",
            "personal_match",
            "site_settings",
            "daemon_token",
        ] {
            assert!(
                !sql.contains(&format!("ALTER TABLE {table} ")),
                "relay must not be granted {table}: {sql}"
            );
        }
        assert!(sql.contains("nostr_secret_key_encrypted"));
        assert!(sql.contains("PERMISSIONS FOR select NONE"));
        assert!(!sql.contains("ROLES VIEWER"));
        assert!(!sql.contains("ROLES EDITOR"));
        assert!(!sql.contains("ROLES OWNER"));
    }

    #[tokio::test]
    async fn relay_login_can_read_only_its_tables_and_not_secrets() {
        let name = "relay-access-perms";
        let owner = owner(name).await;
        crate::migrations::run_migrations(&owner.client)
            .await
            .expect("migrations");
        ensure_relay_access(&owner.client, "scuffed_relay", "relay-pass-1", false)
            .await
            .expect("create relay user");

        let secret = "sekrit-nostr-blob-do-not-leak";
        let reason = "moderation-reason-do-not-leak";
        owner
            .client
            .query(
                "CREATE user:leak SET username = 'leak-marker-user', provider = 'local';
                 CREATE member:probe SET user_id = 'user:leak', org_role = 'member', \
                     display_name = 'Probe', is_active = true, \
                     nostr_pubkey = 'ab', nostr_secret_key_encrypted = { marker: $secret };
                 CREATE moderation_action:probe SET member_id = 'probe', action_type = 'ban', \
                     reason = $reason, issued_by = 'officer-1', is_active = true",
            )
            .bind(("secret", secret))
            .bind(("reason", reason))
            .await
            .expect("seed")
            .check()
            .expect("seed check");

        let stored = owner
            .client
            .query("SELECT * FROM member:probe")
            .await
            .expect("owner select");
        let stored = format!("{stored:?}");
        assert!(
            stored.contains(secret),
            "owner session must still read the secret so this fixture is real: {stored}"
        );

        let relay = relay_session(&owner, "relay-pass-1")
            .await
            .expect("relay sign-in");

        let members = relay.list_nostr_identities().await.expect("identities");
        assert!(members.iter().any(|member| member.id == "probe"));

        let member_star = relay
            .client
            .query("SELECT * FROM member")
            .await
            .expect("relay member select");
        let member_star = format!("{member_star:?}");
        assert!(
            !member_star.contains(secret),
            "relay SELECT * member leaked the secret: {member_star}"
        );

        let users = relay
            .client
            .query("SELECT * FROM user")
            .await
            .expect("relay user select");
        let users = format!("{users:?}");
        assert!(
            !users.contains("leak-marker-user"),
            "relay read the user table: {users}"
        );

        let actions = relay
            .client
            .query("SELECT * FROM moderation_action")
            .await
            .expect("relay moderation select");
        let actions = format!("{actions:?}");
        assert!(
            !actions.contains(reason),
            "relay read moderation reason: {actions}"
        );
        assert!(
            !actions.contains("officer-1"),
            "relay read moderation issued_by: {actions}"
        );

        let banned = relay
            .client
            .query(
                "SELECT VALUE member_id FROM moderation_action \
                 WHERE is_active = true AND action_type = 'ban'",
            )
            .await
            .expect("ban subquery");
        let banned = format!("{banned:?}");
        assert!(
            banned.contains("probe"),
            "ban subquery must still see member_id: {banned}"
        );

        let write = relay
            .client
            .query("CREATE member:nope SET display_name = 'nope', org_role = 'member', user_id = 'x', is_active = true")
            .await
            .expect("create query");
        let write_dbg = format!("{write:?}");
        assert!(
            write.check().is_err() || !write_dbg.contains("nope"),
            "relay created a member: {write_dbg}"
        );

        let editor_client = owner.client.clone();
        editor_client
            .query("DEFINE USER OVERWRITE scuffed_app ON DATABASE PASSWORD 'app-pass-test' ROLES EDITOR")
            .await
            .expect("define editor")
            .check()
            .expect("define editor check");
        editor_client
            .signin(DatabaseAuth {
                namespace: "scuffed_crew".into(),
                database: "main".into(),
                username: "scuffed_app".into(),
                password: "app-pass-test".into(),
            })
            .await
            .expect("editor sign-in");
        editor_client
            .use_ns("scuffed_crew")
            .use_db("main")
            .await
            .expect("editor ns");
        let editor_row = editor_client
            .query("SELECT nostr_secret_key_encrypted, display_name FROM member:probe")
            .await
            .expect("editor select");
        let editor_row = format!("{editor_row:?}");
        assert!(
            editor_row.contains(secret),
            "EDITOR app user must still read the secret: {editor_row}"
        );

        let info = owner.client.query("INFO FOR DB").await.expect("info");
        let info = format!("{info:?}");
        assert!(
            info.to_ascii_lowercase().contains("plays_on")
                && info.to_ascii_lowercase().contains("relation"),
            "ALTER TABLE must leave plays_on a relation: {info}"
        );
    }

    #[tokio::test]
    async fn relay_password_is_not_reset_unless_forced() {
        let name = "relay-access-rotate";
        let owner = owner(name).await;
        crate::migrations::run_migrations(&owner.client)
            .await
            .expect("migrations");
        ensure_relay_access(&owner.client, "scuffed_relay", "relay-pass-1", false)
            .await
            .expect("first");
        ensure_relay_access(&owner.client, "scuffed_relay", "relay-pass-2", false)
            .await
            .expect("second without force");

        relay_session(&owner, "relay-pass-1")
            .await
            .expect("original password still signs in");
        assert!(
            relay_session(&owner, "relay-pass-2").await.is_err(),
            "unstated rotation must not replace the password"
        );

        ensure_relay_access(&owner.client, "scuffed_relay", "relay-pass-2", true)
            .await
            .expect("forced rotation");
        relay_session(&owner, "relay-pass-2")
            .await
            .expect("rotated password signs in");
    }

    #[tokio::test]
    async fn relay_live_select_omits_the_secret() {
        use futures_util::StreamExt;

        let name = "relay-access-live";
        let owner = owner(name).await;
        crate::migrations::run_migrations(&owner.client)
            .await
            .expect("migrations");
        ensure_relay_access(&owner.client, "scuffed_relay", "relay-pass-1", false)
            .await
            .expect("relay user");
        let relay = relay_session(&owner, "relay-pass-1")
            .await
            .expect("sign-in");
        let secret = "sekrit-live-do-not-leak";
        let mut response = relay
            .client
            .query("LIVE SELECT id FROM member")
            .await
            .expect("live query");
        let mut stream = response
            .stream::<surrealdb::Notification<surrealdb::types::Value>>(0)
            .expect("live stream");
        owner
            .client
            .query(
                "CREATE member:liveprobe SET user_id = 'user:live', org_role = 'member', \
                 display_name = 'Live', is_active = true, nostr_secret_key_encrypted = { marker: $secret }",
            )
            .bind(("secret", secret))
            .await
            .expect("create")
            .check()
            .expect("create check");
        let notice = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("live timeout")
            .expect("live ended")
            .expect("live notice");
        let rendered = format!("{:?}", notice.data);
        assert!(
            !rendered.contains(secret),
            "live notice carried the secret: {rendered}"
        );
        assert!(
            rendered.contains("liveprobe"),
            "live notice should name the record: {rendered}"
        );
    }
}
