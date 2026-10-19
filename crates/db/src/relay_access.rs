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
//! The relay process (`relay-policy` and `relay-read-gate`) reads four
//! projection tables, and nothing else:
//!
//! - `relay_member` (`id`, `org_role`, `nostr_pubkey`, `is_active`)
//! - `relay_roster` (`member_id`, `team_id`, `edge_active`, `member_active`)
//! - `relay_moderation` (`member_id`, `action_type`, `expires_at`, `is_active`)
//! - `relay_team_channel` (the channel columns the directory needs)
//!
//! SurrealDB 3.0.5 serves an indexed `SELECT` with an index scan that checks
//! the table permission and skips field permissions. A relay grant of
//! `SELECT` on `member` or `moderation_action` therefore returns
//! `user_id`, display names, `nostr_secret_key_encrypted`, `reason`, and
//! `issued_by` whenever the planner uses `member_user_idx`,
//! `member_display_name_key_idx`, `member_active_name_idx`,
//! `member_nostr_pubkey_idx`, `mod_member_idx`, or `mod_active_idx`.
//! Field permissions do not close that hole. The base tables are
//! `PERMISSIONS NONE` for this login. The projection tables hold only the
//! columns the relay needs.
//!
//! `DEFINE EVENT` on `member`, `plays_on`, `moderation_action`, and
//! `team_channel` copies those columns in the same write. Startup backfills
//! rows that already exist. `member_active` on `relay_roster` is that copy
//! of `member.is_active`, so roster reads do not follow `in.is_active`.
//! The relay does not write. System users (root bootstrap and the EDITOR
//! app user) bypass table and field permissions, so the site keeps reading
//! the base tables.

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

/// Projection tables the relay login may `SELECT`. Nothing else.
pub const RELAY_SELECT_TABLES: &[&str] = &[
    "relay_member",
    "relay_roster",
    "relay_moderation",
    "relay_team_channel",
];

/// Base tables the relay used to read. Table permission for this login is NONE.
pub const RELAY_BASE_TABLES: &[&str] = &["member", "plays_on", "team_channel", "moderation_action"];

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
    for table in RELAY_BASE_TABLES {
        assert_safe_ident(table, "relay base table").expect("relay base table list");
        sql.push_str(&format!("ALTER TABLE {table} PERMISSIONS NONE;\n"));
    }
    for field in [
        "user_id",
        "org_role",
        "display_name",
        "display_name_key",
        "bio",
        "joined_at",
        "avatar_url",
        "timezone",
        "pronouns",
        "availability_status",
        "nostr_pubkey",
        "nostr_key_mode",
        "nostr_secret_key_encrypted",
        "is_active",
        "main_role",
        "twitch",
        "twitter",
    ] {
        sql.push_str(&format!("ALTER FIELD {field} ON member {FIELD_HIDDEN};\n"));
    }
    for field in [
        "member_id",
        "action_type",
        "reason",
        "issued_by",
        "expires_at",
        "is_active",
        "created_at",
    ] {
        sql.push_str(&format!(
            "ALTER FIELD {field} ON moderation_action {FIELD_HIDDEN};\n"
        ));
    }
    sql
}

/// Projection tables, the events that maintain them, and a backfill.
///
/// Idempotent. `DEFINE EVENT OVERWRITE` replaces the copy logic. The backfill
/// uses `LET` and then `FOR`. On SurrealDB 3.0.5, `FOR $row IN (SELECT ...)`
/// rejects the row. Binding the select first is the form that server accepts.
/// The copied columns are the only ones named here. Secrets, display names,
/// `user_id`, `reason`, and `issued_by` are not on these tables.
const RELAY_PROJECTION_SQL: &str = r#"
DEFINE TABLE OVERWRITE relay_member SCHEMAFULL
    PERMISSIONS FOR select WHERE $access = 'relay'
    FOR create NONE
    FOR update NONE
    FOR delete NONE;
DEFINE FIELD OVERWRITE org_role ON relay_member TYPE string;
DEFINE FIELD OVERWRITE nostr_pubkey ON relay_member TYPE option<string>;
DEFINE FIELD OVERWRITE is_active ON relay_member TYPE bool;

DEFINE TABLE OVERWRITE relay_roster SCHEMAFULL
    PERMISSIONS FOR select WHERE $access = 'relay'
    FOR create NONE
    FOR update NONE
    FOR delete NONE;
DEFINE FIELD OVERWRITE member_id ON relay_roster TYPE string;
DEFINE FIELD OVERWRITE team_id ON relay_roster TYPE string;
DEFINE FIELD OVERWRITE edge_active ON relay_roster TYPE bool;
DEFINE FIELD OVERWRITE member_active ON relay_roster TYPE bool;

DEFINE TABLE OVERWRITE relay_moderation SCHEMAFULL
    PERMISSIONS FOR select WHERE $access = 'relay'
    FOR create NONE
    FOR update NONE
    FOR delete NONE;
DEFINE FIELD OVERWRITE member_id ON relay_moderation TYPE string;
DEFINE FIELD OVERWRITE action_type ON relay_moderation TYPE string;
DEFINE FIELD OVERWRITE expires_at ON relay_moderation TYPE option<datetime>;
DEFINE FIELD OVERWRITE is_active ON relay_moderation TYPE bool;

DEFINE TABLE OVERWRITE relay_team_channel SCHEMAFULL
    PERMISSIONS FOR select WHERE $access = 'relay'
    FOR create NONE
    FOR update NONE
    FOR delete NONE;
DEFINE FIELD OVERWRITE team_id ON relay_team_channel TYPE string;
DEFINE FIELD OVERWRITE group_id ON relay_team_channel TYPE string;
DEFINE FIELD OVERWRITE group_type ON relay_team_channel TYPE string;
DEFINE FIELD OVERWRITE relay_url ON relay_team_channel TYPE string;
DEFINE FIELD OVERWRITE is_active ON relay_team_channel TYPE bool;
DEFINE FIELD OVERWRITE created_at ON relay_team_channel TYPE datetime;
DEFINE FIELD OVERWRITE synced_at ON relay_team_channel TYPE option<datetime>;

DEFINE EVENT OVERWRITE relay_member_write ON TABLE member WHEN $event = "CREATE" OR $event = "UPDATE" THEN {
    UPSERT type::record("relay_member", record::id($after.id)) SET org_role = $after.org_role, nostr_pubkey = $after.nostr_pubkey, is_active = $after.is_active;
    UPDATE relay_roster SET member_active = $after.is_active WHERE member_id = <string>record::id($after.id);
};
DEFINE EVENT OVERWRITE relay_member_delete ON TABLE member WHEN $event = "DELETE" THEN {
    DELETE type::record("relay_member", record::id($before.id));
    DELETE relay_roster WHERE member_id = <string>record::id($before.id);
};
DEFINE EVENT OVERWRITE relay_roster_write ON TABLE plays_on WHEN $event = "CREATE" OR $event = "UPDATE" THEN {
    LET $active_rows = (SELECT VALUE is_active FROM type::record("member", record::id($after.in)));
    LET $member_active = $active_rows[0] ?? false;
    UPSERT type::record("relay_roster", record::id($after.id)) SET member_id = <string>record::id($after.in), team_id = <string>record::id($after.out), edge_active = $after.is_active, member_active = $member_active;
};
DEFINE EVENT OVERWRITE relay_roster_delete ON TABLE plays_on WHEN $event = "DELETE" THEN {
    DELETE type::record("relay_roster", record::id($before.id));
};
DEFINE EVENT OVERWRITE relay_moderation_write ON TABLE moderation_action WHEN $event = "CREATE" OR $event = "UPDATE" THEN {
    UPSERT type::record("relay_moderation", record::id($after.id)) SET member_id = $after.member_id, action_type = $after.action_type, expires_at = $after.expires_at, is_active = $after.is_active;
};
DEFINE EVENT OVERWRITE relay_moderation_delete ON TABLE moderation_action WHEN $event = "DELETE" THEN {
    DELETE type::record("relay_moderation", record::id($before.id));
};
DEFINE EVENT OVERWRITE relay_team_channel_write ON TABLE team_channel WHEN $event = "CREATE" OR $event = "UPDATE" THEN {
    UPSERT type::record("relay_team_channel", record::id($after.id)) SET team_id = $after.team_id, group_id = $after.group_id, group_type = $after.group_type, relay_url = $after.relay_url, is_active = $after.is_active, created_at = $after.created_at, synced_at = $after.synced_at;
};
DEFINE EVENT OVERWRITE relay_team_channel_delete ON TABLE team_channel WHEN $event = "DELETE" THEN {
    DELETE type::record("relay_team_channel", record::id($before.id));
};

LET $relay_members = (SELECT id, org_role, nostr_pubkey, is_active FROM member);
FOR $row IN $relay_members {
    UPSERT type::record("relay_member", record::id($row.id)) SET org_role = $row.org_role, nostr_pubkey = $row.nostr_pubkey, is_active = $row.is_active;
};
LET $relay_edges = (SELECT id, in, out, is_active FROM plays_on);
FOR $row IN $relay_edges {
    LET $active_rows = (SELECT VALUE is_active FROM type::record("member", record::id($row.in)));
    LET $member_active = $active_rows[0] ?? false;
    UPSERT type::record("relay_roster", record::id($row.id)) SET member_id = <string>record::id($row.in), team_id = <string>record::id($row.out), edge_active = $row.is_active, member_active = $member_active;
};
LET $relay_actions = (SELECT id, member_id, action_type, expires_at, is_active FROM moderation_action);
FOR $row IN $relay_actions {
    UPSERT type::record("relay_moderation", record::id($row.id)) SET member_id = $row.member_id, action_type = $row.action_type, expires_at = $row.expires_at, is_active = $row.is_active;
};
LET $relay_channels = (SELECT id, team_id, group_id, group_type, relay_url, is_active, created_at, synced_at FROM team_channel);
FOR $row IN $relay_channels {
    UPSERT type::record("relay_team_channel", record::id($row.id)) SET team_id = $row.team_id, group_id = $row.group_id, group_type = $row.group_type, relay_url = $row.relay_url, is_active = $row.is_active, created_at = $row.created_at, synced_at = $row.synced_at;
};
"#;

async fn install_relay_projection(client: &Surreal<Any>) -> DbResult<()> {
    client.query(RELAY_PROJECTION_SQL).await?.check()?;
    Ok(())
}

/// Record access rejects `DURATION FOR TOKEN NONE`. Omitting `DURATION`
/// keeps the server default: token 1 hour, session NONE. An open websocket
/// does not expire. The surrealdb 3.2 client stores the sign-in as a
/// replayable command (`engine/remote/mod.rs`, `record_replayable`) and
/// sends it again after every reconnect (`engine/remote/ws/mod.rs`,
/// `replay_session`). A periodic re-sign-in is not used. It would repeat
/// the argon2 check, and every relay would send that check together after
/// a database restart.
const DEFINE_RELAY_ACCESS: &str = "\
DEFINE ACCESS OVERWRITE relay ON DATABASE TYPE RECORD \
    SIGNIN ( \
        SELECT id, username FROM relay_db_user \
        WHERE username = $user AND crypto::argon2::compare(pass, $pass) \
    )";

/// Install the projection tables and the relay read permissions. Idempotent.
/// Does not create the login.
///
/// Tables outside [`RELAY_SELECT_TABLES`] are set to `PERMISSIONS NONE` so a
/// record user cannot select or create them. That includes [`RELAY_BASE_TABLES`].
/// System users bypass that.
pub async fn apply_relay_read_permissions(client: &Surreal<Any>) -> DbResult<()> {
    install_relay_projection(client).await?;
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
        "Defined relay database user (SELECT on relay_member, relay_roster, relay_moderation, relay_team_channel)"
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
        assert!(sql.contains("ALTER FIELD org_role ON member PERMISSIONS FOR select NONE"));
        assert!(sql.contains("ALTER FIELD nostr_pubkey ON member PERMISSIONS FOR select NONE"));
        assert!(sql.contains("ALTER FIELD is_active ON member PERMISSIONS FOR select NONE"));
        assert!(
            sql.contains("ALTER FIELD member_id ON moderation_action PERMISSIONS FOR select NONE")
        );
        assert!(sql
            .contains("ALTER FIELD action_type ON moderation_action PERMISSIONS FOR select NONE"));
        for table in RELAY_BASE_TABLES {
            assert!(
                sql.contains(&format!("ALTER TABLE {table} PERMISSIONS NONE")),
                "{table} must not be selectable by the relay: {sql}"
            );
            assert!(
                !sql.contains(&format!("ALTER TABLE {table} PERMISSIONS FOR select")),
                "{table} still has a relay SELECT grant: {sql}"
            );
        }
        assert!(RELAY_PROJECTION_SQL.contains("DEFINE EVENT OVERWRITE relay_member_write"));
        assert!(RELAY_PROJECTION_SQL.contains("UPDATE relay_roster SET member_active"));
        assert!(!RELAY_PROJECTION_SQL.contains("nostr_secret_key_encrypted"));
        assert!(!RELAY_PROJECTION_SQL.contains("display_name"));
        assert!(!RELAY_PROJECTION_SQL.contains("issued_by"));
        assert!(!RELAY_PROJECTION_SQL.contains("user_id"));
        let rollback = include_str!("../../../scripts/relay-access-down.sh");
        for statement in [
            "REMOVE EVENT IF EXISTS relay_member_write ON TABLE member;",
            "REMOVE EVENT IF EXISTS relay_member_delete ON TABLE member;",
            "REMOVE EVENT IF EXISTS relay_roster_write ON TABLE plays_on;",
            "REMOVE EVENT IF EXISTS relay_roster_delete ON TABLE plays_on;",
            "REMOVE EVENT IF EXISTS relay_moderation_write ON TABLE moderation_action;",
            "REMOVE EVENT IF EXISTS relay_moderation_delete ON TABLE moderation_action;",
            "REMOVE EVENT IF EXISTS relay_team_channel_write ON TABLE team_channel;",
            "REMOVE EVENT IF EXISTS relay_team_channel_delete ON TABLE team_channel;",
            "REMOVE TABLE IF EXISTS relay_member;",
            "REMOVE TABLE IF EXISTS relay_roster;",
            "REMOVE TABLE IF EXISTS relay_moderation;",
            "REMOVE TABLE IF EXISTS relay_team_channel;",
        ] {
            assert!(
                rollback.contains(statement),
                "rollback script is missing {statement}"
            );
            assert!(
                RELAY_PROJECTION_SQL.contains(
                    statement
                        .trim_start_matches("REMOVE EVENT IF EXISTS ")
                        .trim_start_matches("REMOVE TABLE IF EXISTS ")
                        .trim_end_matches(';')
                        .split_once(" ON TABLE")
                        .map(|(name, _)| name)
                        .unwrap_or(statement)
                ) || statement.starts_with("REMOVE TABLE"),
                "projection SQL does not define {statement}"
            );
        }
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
                "SELECT VALUE member_id FROM relay_moderation \
                 WHERE is_active = true AND action_type = 'ban'",
            )
            .await
            .expect("ban subquery");
        let banned = format!("{banned:?}");
        assert!(
            banned.contains("probe"),
            "ban subquery must still see member_id on the projection: {banned}"
        );
        let base_ban = relay
            .client
            .query(
                "SELECT VALUE member_id FROM moderation_action \
                 WHERE is_active = true AND action_type = 'ban'",
            )
            .await
            .expect("base ban query");
        let base_ban_dbg = format!("{base_ban:?}");
        assert!(
            base_ban.check().is_err() || !base_ban_dbg.contains("probe"),
            "relay read moderation_action through the ban predicate: {base_ban_dbg}"
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
        for required in [
            "audit_log",
            "user",
            "relay_db_user",
            "session",
            "member",
            "plays_on",
            "team_channel",
            "moderation_action",
        ] {
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
            .query("LIVE SELECT id FROM relay_member")
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

    #[tokio::test]
    async fn relay_projection_follows_member_active_and_bans() {
        let owner = owner("relay-projection-roster").await;
        crate::migrations::run_migrations(&owner.client)
            .await
            .expect("migrations");
        ensure_relay_access(&owner.client, "scuffed_relay", "relay-pass-1", false)
            .await
            .expect("relay user");

        let member = owner
            .create_member("proj-user", "ProjName", crate::OrgRole::Member)
            .await
            .expect("member");
        let pubkey = "ab";
        owner
            .update_member_nostr_keys(&member.id, Some(pubkey), Some("external"), None)
            .await
            .expect("pubkey");
        let secret = "proj-secret-marker";
        owner
            .client
            .query("UPDATE $rid SET nostr_secret_key_encrypted = { marker: $secret }")
            .bind((
                "rid",
                surrealdb_types::RecordId::new("member", member.id.clone()),
            ))
            .bind(("secret", secret))
            .await
            .expect("secret update")
            .check()
            .expect("secret update check");
        let team = owner
            .create_team("Proj Squad", "ow2", None, None, None)
            .await
            .expect("team");
        owner
            .add_to_roster(&member.id, &team.id, crate::TeamRole::Player)
            .await
            .expect("roster");
        owner
            .create_team_channel(&team.id, "proj-public", crate::GroupType::Public, "")
            .await
            .expect("channel");

        let relay = relay_session(&owner, "relay-pass-1")
            .await
            .expect("relay sign-in");
        let keys = relay.list_relay_member_keys().await.expect("keys");
        assert!(
            keys.iter()
                .any(|row| row.id == member.id && row.nostr_pubkey.as_deref() == Some(pubkey)),
            "projection missed the pubkey: {keys:?}"
        );
        let pairs = relay.list_active_roster_pairs().await.expect("pairs");
        assert!(
            pairs
                .iter()
                .any(|(member_id, team_id)| member_id == &member.id && team_id == &team.id),
            "projection missed the roster pair: {pairs:?}"
        );
        let channels = relay.list_all_team_channels().await.expect("channels");
        assert!(
            channels
                .iter()
                .any(|channel| channel.group_id == "proj-public" && channel.team_id == team.id),
            "projection missed the channel: {channels:?}"
        );
        let copied = owner
            .client
            .query("SELECT * FROM relay_member")
            .await
            .expect("owner projection");
        let copied = format!("{copied:?}");
        assert!(
            !copied.contains(secret) && !copied.contains("ProjName"),
            "projection stored a hidden column: {copied}"
        );

        let indexed = relay
            .client
            .query("SELECT * FROM member WHERE nostr_pubkey = 'ab'")
            .await
            .expect("indexed member select");
        let indexed_dbg = format!("{indexed:?}");
        assert!(
            indexed.check().is_err() || !indexed_dbg.contains(secret),
            "indexed member select returned the secret: {indexed_dbg}"
        );
        assert!(
            !indexed_dbg.contains("ProjName"),
            "indexed member select returned the display name: {indexed_dbg}"
        );

        owner
            .client
            .query("UPDATE $rid SET is_active = false")
            .bind((
                "rid",
                surrealdb_types::RecordId::new("member", member.id.clone()),
            ))
            .await
            .expect("deactivate")
            .check()
            .expect("deactivate check");
        let pairs = relay.list_active_roster_pairs().await.expect("pairs off");
        assert!(
            pairs.iter().all(|(member_id, _)| member_id != &member.id),
            "inactive member stayed on the relay roster: {pairs:?}"
        );
        let site = owner.get_team_roster(&team.id).await.expect("site roster");
        assert!(
            site.is_empty(),
            "site roster still lists the inactive member: {site:?}"
        );

        owner
            .client
            .query("UPDATE $rid SET is_active = true")
            .bind((
                "rid",
                surrealdb_types::RecordId::new("member", member.id.clone()),
            ))
            .await
            .expect("reactivate")
            .check()
            .expect("reactivate check");
        let pairs = relay.list_active_roster_pairs().await.expect("pairs on");
        assert!(
            pairs
                .iter()
                .any(|(member_id, team_id)| member_id == &member.id && team_id == &team.id),
            "reactivated member did not return to the relay roster: {pairs:?}"
        );

        let reason = "proj-reason-marker";
        owner
            .client
            .query(
                "CREATE moderation_action SET member_id = $id, action_type = 'ban', \
                 reason = $reason, issued_by = 'officer-proj', is_active = true",
            )
            .bind(("id", member.id.clone()))
            .bind(("reason", reason))
            .await
            .expect("ban")
            .check()
            .expect("ban check");
        let pairs = relay
            .list_active_roster_pairs()
            .await
            .expect("pairs banned");
        assert!(
            pairs.iter().all(|(member_id, _)| member_id != &member.id),
            "banned member stayed on the relay roster: {pairs:?}"
        );
        let banned_base = relay
            .client
            .query("SELECT * FROM moderation_action")
            .await
            .expect("base moderation");
        let banned_base_dbg = format!("{banned_base:?}");
        assert!(
            !banned_base_dbg.contains(reason),
            "relay read the ban reason from moderation_action: {banned_base_dbg}"
        );

        owner
            .client
            .query("DELETE relay_member")
            .await
            .expect("drop projection")
            .check()
            .expect("drop projection check");
        apply_relay_read_permissions(&owner.client)
            .await
            .expect("backfill");
        let keys = relay
            .list_relay_member_keys()
            .await
            .expect("keys after backfill");
        assert!(
            keys.iter()
                .any(|row| row.id == member.id && row.nostr_pubkey.as_deref() == Some(pubkey)),
            "backfill did not restore the pubkey: {keys:?}"
        );
    }

    /// The text of one SurrealQL statement in `migrations.rs`, not a copy.
    fn migration_statement(marker: &str) -> String {
        let src = include_str!("migrations.rs");
        let start = src
            .find(marker)
            .unwrap_or_else(|| panic!("migrations.rs is missing {marker}"));
        let rest = &src[start..];
        let end = rest
            .find(';')
            .unwrap_or_else(|| panic!("migrations.rs statement has no semicolon after {marker}"));
        rest[..=end].trim().to_string()
    }

    /// DEFINE FIELD OVERWRITE on the secret columns, and the audit_log table
    /// define, used to omit permissions. A later failure then left those
    /// columns readable. Re-running the statements from `migrations.rs` must
    /// keep the relay from seeing them.
    #[tokio::test]
    async fn define_field_overwrite_does_not_reopen_hidden_columns() {
        let secret_define =
            migration_statement("DEFINE FIELD OVERWRITE nostr_secret_key_encrypted ON member");
        let reason_define =
            migration_statement("DEFINE FIELD OVERWRITE reason ON moderation_action");
        let issued_define =
            migration_statement("DEFINE FIELD OVERWRITE issued_by ON moderation_action");
        let audit_define = migration_statement("DEFINE TABLE OVERWRITE audit_log");
        for statement in [&secret_define, &reason_define, &issued_define] {
            assert!(
                statement.contains("PERMISSIONS FOR select NONE"),
                "hidden field define lost its permissions: {statement}"
            );
        }
        assert!(
            audit_define.contains("PERMISSIONS NONE"),
            "audit_log define lost PERMISSIONS NONE: {audit_define}"
        );
        let src = include_str!("migrations.rs");
        let apply_at = src
            .find("apply_relay_read_permissions")
            .expect("migrations call apply_relay_read_permissions");
        for marker in [
            "DEFINE FIELD OVERWRITE nostr_secret_key_encrypted ON member",
            "DEFINE FIELD OVERWRITE reason ON moderation_action",
            "DEFINE FIELD OVERWRITE issued_by ON moderation_action",
            "DEFINE TABLE OVERWRITE audit_log",
        ] {
            let at = src.find(marker).expect(marker);
            assert!(
                at < apply_at,
                "{marker} is defined after apply_relay_read_permissions"
            );
        }

        let owner = owner("relay-access-redefine").await;
        crate::migrations::run_migrations(&owner.client)
            .await
            .expect("migrations");
        ensure_relay_access(&owner.client, "scuffed_relay", "relay-pass-1", false)
            .await
            .expect("relay user");
        let secret = "redefine-secret-marker";
        let reason = "redefine-reason-marker";
        owner
            .client
            .query(
                "CREATE member:redefine SET user_id = 'user:redefine', org_role = 'member', \
                 display_name = 'Redefine', is_active = true, nostr_pubkey = 'ef', \
                 nostr_secret_key_encrypted = { marker: $secret };
                 CREATE moderation_action:redefine SET member_id = 'redefine', \
                 action_type = 'ban', reason = $reason, issued_by = 'officer-redefine', \
                 is_active = true;
                 CREATE audit_log SET actor_id = 'officer-redefine', \
                 action = 'redefine-audit-marker', target_type = 'member', target_id = 'redefine'",
            )
            .bind(("secret", secret))
            .bind(("reason", reason))
            .await
            .expect("seed")
            .check()
            .expect("seed check");

        owner
            .client
            .query(format!(
                "{secret_define}\n{reason_define}\n{issued_define}\n{audit_define}"
            ))
            .await
            .expect("replay defines")
            .check()
            .expect("replay defines check");

        let relay = relay_session(&owner, "relay-pass-1")
            .await
            .expect("relay sign-in");
        let member = relay
            .client
            .query("SELECT * FROM member:redefine")
            .await
            .expect("relay member");
        let member_dbg = format!("{member:?}");
        assert!(
            !member_dbg.contains(secret),
            "replaying the field define reopened nostr_secret_key_encrypted: {member_dbg}"
        );
        let action = relay
            .client
            .query("SELECT * FROM moderation_action:redefine")
            .await
            .expect("relay moderation");
        let action_dbg = format!("{action:?}");
        assert!(
            !action_dbg.contains(reason),
            "replaying the field define reopened reason: {action_dbg}"
        );
        assert!(
            !action_dbg.contains("officer-redefine"),
            "replaying the field define reopened issued_by: {action_dbg}"
        );
        let audit = relay
            .client
            .query("SELECT * FROM audit_log")
            .await
            .expect("relay audit");
        let audit_dbg = format!("{audit:?}");
        assert!(
            audit.check().is_err() || !audit_dbg.contains("redefine-audit-marker"),
            "replaying the table define reopened audit_log: {audit_dbg}"
        );

        let mut info = owner.client.query("INFO FOR DB").await.expect("info");
        let info_value: surrealdb_types::Value = info.take(0).expect("info value");
        let definition = info_value
            .into_json_value()
            .get("tables")
            .and_then(|value| value.get("audit_log"))
            .map(|value| value.to_string())
            .unwrap_or_default();
        assert!(
            definition.contains("PERMISSIONS NONE"),
            "audit_log is not PERMISSIONS NONE after the real define: {definition}"
        );
    }
}
