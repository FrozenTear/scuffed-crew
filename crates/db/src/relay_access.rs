//! Least-privilege database login for the relay.
//!
//! The compose server image is SurrealDB v3.0. `DEFINE USER` roles on that
//! server are database-wide (`OWNER`, `EDITOR`, `VIEWER`). `VIEWER` can
//! `SELECT` every table, including `nostr_secret_key_encrypted`. Record
//! access is the mechanism that actually limits a login to specific tables.
//! The Rust client and these in-memory tests use the surrealdb 3.2 crate
//! against that same record-access shape. This module does not claim the
//! server image is 3.2.
//!
//! The relay process (`relay-policy` and `relay-read-gate`) only reads:
//!
//! - `member` (`list_relay_member_keys`: `id`, `org_role`, `nostr_pubkey`,
//!   `is_active`; `LIVE SELECT id`; `in.is_active` on roster edges)
//! - `plays_on` (`list_active_roster_pairs`, `LIVE SELECT id`)
//! - `team_channel` (`list_all_team_channels`, `LIVE SELECT id`)
//! - `moderation_action` (`member_id`, `action_type`, `expires_at`,
//!   `is_active` inside `list_active_roster_pairs`)
//!
//! It does not write. Other `member` columns, and `moderation_action.reason`,
//! `issued_by`, and `created_at`, are not selectable by this login. The field
//! permissions are on the migration `DEFINE FIELD` statements, so a later
//! statement failing cannot make those columns readable again. System users
//! (root bootstrap and the EDITOR app user) bypass table and field
//! permissions, so the site keeps its current access.

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

const TABLE_PERMS: &str = "PERMISSIONS \
     FOR select WHERE $access = 'relay' \
     FOR create NONE \
     FOR update NONE \
     FOR delete NONE";

/// Field permissions have no DELETE clause. SurrealDB rejects `FOR delete` on a field.
const FIELD_RELAY: &str = "PERMISSIONS \
     FOR select WHERE $access = 'relay' \
     FOR create NONE \
     FOR update NONE";

const FIELD_HIDDEN: &str = "PERMISSIONS FOR select NONE \
     FOR create NONE \
     FOR update NONE";

/// SurrealQL that grants the relay login its read permissions.
///
/// Table names come from [`RELAY_SELECT_TABLES`], which is a fixed list in
/// this file, not request input. Field clauses restate the migration
/// permissions with `ALTER FIELD` and do not `DEFINE FIELD OVERWRITE`, so
/// they do not change the column type or reopen a secret.
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
        sql.push_str(&format!("ALTER TABLE {table} {TABLE_PERMS};\n"));
    }
    for field in ["org_role", "nostr_pubkey", "is_active"] {
        sql.push_str(&format!("ALTER FIELD {field} ON member {FIELD_RELAY};\n"));
    }
    for field in [
        "user_id",
        "display_name",
        "display_name_key",
        "bio",
        "joined_at",
        "avatar_url",
        "timezone",
        "pronouns",
        "availability_status",
        "nostr_key_mode",
        "nostr_secret_key_encrypted",
        "main_role",
        "twitch",
        "twitter",
    ] {
        sql.push_str(&format!("ALTER FIELD {field} ON member {FIELD_HIDDEN};\n"));
    }
    for field in ["member_id", "action_type", "expires_at", "is_active"] {
        sql.push_str(&format!(
            "ALTER FIELD {field} ON moderation_action {FIELD_RELAY};\n"
        ));
    }
    for field in ["reason", "issued_by", "created_at"] {
        sql.push_str(&format!(
            "ALTER FIELD {field} ON moderation_action {FIELD_HIDDEN};\n"
        ));
    }
    sql
}

/// Record access rejects `DURATION FOR TOKEN NONE`. Omitting `DURATION`
/// keeps the server default: token 1 hour, session none. The relay process
/// signs in again every 20 minutes and exits if that refresh fails, so the
/// container restarts with a new token.
const DEFINE_RELAY_ACCESS: &str = "\
DEFINE ACCESS OVERWRITE relay ON DATABASE TYPE RECORD \
    SIGNIN ( \
        SELECT id, username FROM relay_db_user \
        WHERE username = $user AND crypto::argon2::compare(pass, $pass) \
    )";

/// Install table and field permissions. Idempotent. Does not create the login.
///
/// Tables outside [`RELAY_SELECT_TABLES`] are set to `PERMISSIONS NONE` so a
/// record user cannot select or create them. System users bypass that.
pub async fn apply_relay_read_permissions(client: &Surreal<Any>) -> DbResult<()> {
    client.query(relay_permission_sql()).await?.check()?;
    deny_unlisted_tables(client).await?;
    Ok(())
}

async fn deny_unlisted_tables(client: &Surreal<Any>) -> DbResult<()> {
    let mut response = client.query("INFO FOR DB").await?.check()?;
    let info: surrealdb_types::Value = response.take(0)?;
    let json = info.into_json_value();
    let Some(tables) = json.get("tables").and_then(|value| value.as_object()) else {
        return Err(DbError::Config("INFO FOR DB did not list tables".into()));
    };
    let mut sql = String::new();
    for name in tables.keys() {
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(DbError::Config(format!(
                "Refusing to alter unexpected table name {name}"
            )));
        }
        if RELAY_SELECT_TABLES.contains(&name.as_str()) {
            continue;
        }
        sql.push_str(&format!("ALTER TABLE {name} PERMISSIONS NONE;\n"));
    }
    if !sql.is_empty() {
        client.query(sql).await?.check()?;
    }
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
        .and_then(|value| normalize_relay_password(&value));
    let Some(password) = password else {
        tracing::warn!(
            "SURREALDB_RELAY_PASSWORD is unset or blank. The relay database user was not \
             created. The relay container does not receive the root password. In production \
             the relay process refuses to start until this secret is non-blank and \
             site-server has started again. scripts/install.sh and scripts/update.sh \
             replace a missing or blank key."
        );
        return Ok(());
    };
    if crate::is_production_env() && password == root_pass.trim() {
        return Err(DbError::Config(
            "SURREALDB_RELAY_PASSWORD must not equal SURREALDB_PASSWORD in production. \
             Use a distinct password (install.sh generates one)."
                .into(),
        ));
    }
    if let Ok(app_pass) = std::env::var("SURREALDB_APP_PASSWORD") {
        let app_pass = app_pass.trim();
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

/// Trim the relay password the same way the relay process does.
/// Whitespace-only is missing.
fn normalize_relay_password(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
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

    /// Sign in again on this session. The record token defaults to 1 hour.
    pub async fn refresh_relay_signin(
        &self,
        namespace: &str,
        database: &str,
        username: &str,
        password: &str,
    ) -> DbResult<()> {
        let password = normalize_relay_password(password)
            .ok_or_else(|| DbError::Config("Relay database password must not be empty".into()))?;
        sign_in_relay_client(&self.client, namespace, database, username, &password).await
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
        assert!(sql.contains("WHERE $access = 'relay'"));
        assert!(!sql.contains("meta::tb($auth.id)"));
        assert!(sql.contains(
            "ALTER FIELD nostr_secret_key_encrypted ON member PERMISSIONS FOR select NONE"
        ));
        assert!(sql.contains("ALTER FIELD reason ON moderation_action PERMISSIONS FOR select NONE"));
        assert!(
            sql.contains("ALTER FIELD issued_by ON moderation_action PERMISSIONS FOR select NONE")
        );
        assert!(sql.contains(
            "ALTER FIELD org_role ON member PERMISSIONS FOR select WHERE $access = 'relay'"
        ));
        assert!(sql.contains(
            "ALTER FIELD member_id ON moderation_action PERMISSIONS FOR select WHERE $access = 'relay'"
        ));
        assert!(!sql.contains("DEFINE FIELD OVERWRITE nostr_secret_key_encrypted"));
        assert!(!sql.contains("DEFINE FIELD OVERWRITE reason"));
        assert!(!sql.contains("DEFINE FIELD OVERWRITE issued_by"));
        assert!(!sql.contains("3650"));
        assert!(!DEFINE_RELAY_ACCESS.contains("3650"));
        assert!(!DEFINE_RELAY_ACCESS.contains("DURATION"));
        assert!(!sql.contains("ROLES VIEWER"));
        assert!(!sql.contains("ROLES EDITOR"));
        assert!(!sql.contains("ROLES OWNER"));
    }

    #[test]
    fn blank_relay_password_is_missing() {
        assert_eq!(normalize_relay_password("   "), None);
        assert_eq!(normalize_relay_password("\n"), None);
        assert_eq!(
            normalize_relay_password("  relay-secret  ").as_deref(),
            Some("relay-secret")
        );
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

        let members = relay.list_relay_member_keys().await.expect("identities");
        assert!(members.iter().any(|member| member.id == "probe"));

        let hidden = relay
            .client
            .query("SELECT display_name, nostr_secret_key_encrypted FROM member:probe")
            .await
            .expect("hidden field query");
        let hidden_dbg = format!("{hidden:?}");
        assert!(
            hidden.check().is_err() || !hidden_dbg.contains(secret),
            "relay read nostr_secret_key_encrypted: {hidden_dbg}"
        );
        assert!(
            !hidden_dbg.contains("Probe"),
            "relay read display_name: {hidden_dbg}"
        );

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

        let updated = relay
            .client
            .query("UPDATE member:probe SET display_name = 'changed'")
            .await
            .expect("update query");
        let update_dbg = format!("{updated:?}");
        let update_denied = updated.check().is_err();
        assert!(
            update_denied || !update_dbg.contains("changed"),
            "relay UPDATE member returned a row: {update_dbg}"
        );
        let deleted = relay
            .client
            .query("DELETE member:probe")
            .await
            .expect("delete query");
        let deleted_dbg = format!("{deleted:?}");
        let delete_denied = deleted.check().is_err();
        assert!(
            delete_denied || deleted_dbg.contains("[]"),
            "relay DELETE member returned a row: {deleted_dbg}"
        );
        owner
            .client
            .query("CREATE team:nope SET name = 'Nope', game_id = 'game:x', is_active = true")
            .await
            .expect("team")
            .check()
            .expect("team check");
        let related = relay
            .client
            .query(
                "RELATE member:probe->plays_on->team:nope SET team_role = 'player', is_active = true",
            )
            .await
            .expect("relate query");
        let related_dbg = format!("{related:?}");
        let relate_denied = related.check().is_err();
        assert!(
            relate_denied || related_dbg.contains("[]"),
            "relay RELATE plays_on returned a row: {related_dbg}"
        );
        let still = owner
            .client
            .query("SELECT display_name FROM member:probe")
            .await
            .expect("owner still");
        let still = format!("{still:?}");
        assert!(
            still.contains("Probe"),
            "relay mutation changed the member: {still}"
        );
        let mut edges = owner
            .client
            .query("SELECT VALUE id FROM plays_on WHERE out = team:nope")
            .await
            .expect("edges");
        let edge_ids: Vec<surrealdb_types::Value> = edges.take(0).expect("edge rows");
        assert!(
            edge_ids.is_empty(),
            "relay created a plays_on edge: {edge_ids:?}"
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
    async fn relay_cannot_select_or_create_outside_its_tables() {
        let name = "relay-access-other-tables";
        let owner = owner(name).await;
        crate::migrations::run_migrations(&owner.client)
            .await
            .expect("migrations");
        ensure_relay_access(&owner.client, "scuffed_relay", "relay-pass-1", false)
            .await
            .expect("relay user");
        owner
            .client
            .query(
                "CREATE audit_log SET actor_id = 'officer-1', action = 'relay-audit-marker', \
                 target_type = 'member', target_id = 'probe'",
            )
            .await
            .expect("audit seed")
            .check()
            .expect("audit seed check");

        let mut info = owner.client.query("INFO FOR DB").await.expect("info");
        let info_value: surrealdb_types::Value = info.take(0).expect("info value");
        let json = info_value.into_json_value();
        let tables = json
            .get("tables")
            .and_then(|value| value.as_object())
            .expect("INFO FOR DB tables");
        let mut denied = Vec::new();
        for (name, definition) in tables {
            assert!(
                name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "unexpected table name {name}"
            );
            if RELAY_SELECT_TABLES.contains(&name.as_str()) {
                continue;
            }
            let definition = definition.to_string();
            assert!(
                definition.contains("PERMISSIONS NONE"),
                "{name} is not PERMISSIONS NONE: {definition}"
            );
            denied.push(name.clone());
        }
        for required in ["audit_log", "user", "relay_db_user", "session"] {
            assert!(
                denied.iter().any(|table| table == required),
                "INFO FOR DB did not list {required}: {denied:?}"
            );
        }

        let relay = relay_session(&owner, "relay-pass-1")
            .await
            .expect("relay sign-in");
        for table in &denied {
            let selected = relay
                .client
                .query(format!("SELECT * FROM {table} LIMIT 5"))
                .await
                .unwrap_or_else(|err| panic!("select {table} transport: {err}"));
            let selected_dbg = format!("{selected:?}");
            let select_denied = selected.check().is_err();
            assert!(
                select_denied || !selected_dbg.contains("relay-audit-marker"),
                "relay SELECT {table} leaked the audit marker: {selected_dbg}"
            );
            assert!(
                select_denied || selected_dbg.contains("[]"),
                "relay SELECT {table} returned rows: {selected_dbg}"
            );
            let created = relay
                .client
                .query(format!(
                    "CREATE {table} SET action = 'relay-should-not-create'"
                ))
                .await
                .unwrap_or_else(|err| panic!("create {table} transport: {err}"));
            let created_dbg = format!("{created:?}");
            let create_denied = created.check().is_err();
            assert!(
                create_denied || created_dbg.contains("[]"),
                "relay CREATE {table} returned a row: {created_dbg}"
            );
            let stored = owner
                .client
                .query(format!("SELECT * FROM {table}"))
                .await
                .unwrap_or_else(|err| panic!("owner select {table}: {err}"));
            let stored_dbg = format!("{stored:?}");
            assert!(
                !stored_dbg.contains("relay-should-not-create"),
                "relay CREATE {table} was stored: {stored_dbg}"
            );
        }
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

        let restricted = relay
            .client
            .query("LIVE SELECT nostr_secret_key_encrypted FROM member")
            .await;
        match restricted {
            Err(err) => {
                let rendered = err.to_string();
                assert!(
                    !rendered.contains(secret),
                    "live error leaked the secret: {rendered}"
                );
            }
            Ok(mut response) => {
                match response.stream::<surrealdb::Notification<surrealdb::types::Value>>(0) {
                    Err(err) => {
                        let rendered = format!("{err}");
                        assert!(
                            !rendered.contains(secret),
                            "live stream error leaked the secret: {rendered}"
                        );
                    }
                    Ok(mut restricted_stream) => {
                        owner
                        .client
                        .query(
                            "UPDATE member:liveprobe SET nostr_secret_key_encrypted = { marker: $secret }",
                        )
                        .bind(("secret", secret))
                        .await
                        .expect("update secret")
                        .check()
                        .expect("update secret check");
                        let notice = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            restricted_stream.next(),
                        )
                        .await;
                        if let Ok(Some(Ok(notice))) = notice {
                            let rendered = format!("{:?}", notice.data);
                            assert!(
                                !rendered.contains(secret),
                                "restricted live notice carried the secret: {rendered}"
                            );
                        }
                    }
                }
            }
        }
    }
}
