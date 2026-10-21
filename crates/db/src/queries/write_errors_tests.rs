//! DB1: write helpers must surface SurrealDB statement errors.
//!
//! The SurrealDB 3 client keeps a failed statement inside the `Response`, so
//! `query(..).await?` alone returns `Ok` when the write itself failed (ASSERT,
//! transaction conflict, a THROW in an event). Each test below makes the
//! write fail inside the database and checks that the helper now returns
//! `Err` instead of reporting success.

use crate::migrations::run_migrations;
use crate::types::{OrgRole, TeamRole};
use crate::{Database, DbError};

async fn test_db() -> Database {
    let db = Database::connect_memory().await.unwrap();
    run_migrations(&db.client).await.unwrap();
    db
}

/// Make every `event` (`"UPDATE"` / `"DELETE"`) on `table` fail with a THROW.
/// `table` and `event` are fixed literals from the tests, not request data.
async fn fail_writes(db: &Database, table: &str, event: &str) {
    db.client
        .query(format!(
            "DEFINE EVENT OVERWRITE db1_fail ON {table} WHEN $event = '{event}' \
             THEN {{ THROW 'db1 injected write failure' }}"
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
}

async fn allow_writes(db: &Database, table: &str) {
    db.client
        .query(format!("REMOVE EVENT db1_fail ON {table}"))
        .await
        .unwrap()
        .check()
        .unwrap();
}

async fn session_count(db: &Database) -> usize {
    let rows: Vec<surrealdb_types::RecordId> = db
        .client
        .query("SELECT VALUE id FROM session")
        .await
        .unwrap()
        .check()
        .unwrap()
        .take(0)
        .unwrap();
    rows.len()
}

#[tokio::test]
async fn session_deletes_surface_statement_errors() {
    let db = test_db().await;
    let user = db.create_local_user("db1-sessions", "hash").await.unwrap();
    db.create_session(&user.id, "live-a", 24).await.unwrap();
    db.create_session(&user.id, "live-b", 24).await.unwrap();
    // Already expired: cleanup_expired_sessions has something to delete.
    db.create_session(&user.id, "expired", -1).await.unwrap();
    assert_eq!(session_count(&db).await, 3);

    fail_writes(&db, "session", "DELETE").await;

    let logout = db.delete_session("live-a").await;
    assert!(logout.is_err(), "logout reported success: {logout:?}");
    let revoke = db.delete_sessions_for_user(&user.id).await;
    assert!(revoke.is_err(), "revoke-all reported success: {revoke:?}");
    let cleanup = db.cleanup_expired_sessions().await;
    assert!(cleanup.is_err(), "cleanup reported success: {cleanup:?}");
    assert_eq!(session_count(&db).await, 3, "nothing was deleted");

    // Same helpers succeed once the injected failure is gone, so the errors
    // above came from the failed DELETE, not from the fixture.
    allow_writes(&db, "session").await;
    db.delete_session("live-a").await.unwrap();
    assert_eq!(db.cleanup_expired_sessions().await.unwrap(), 1);
    assert_eq!(db.delete_sessions_for_user(&user.id).await.unwrap(), 1);
    assert_eq!(session_count(&db).await, 0);
}

#[tokio::test]
async fn password_hash_update_surfaces_assert_failure_and_missing_user() {
    let db = test_db().await;
    let user = db.create_local_user("db1-password", "old").await.unwrap();

    db.client
        .query(
            "DEFINE FIELD OVERWRITE password_hash ON user TYPE option<string> \
             ASSERT $value = NONE OR string::len($value) < 8",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
    let rejected = db
        .set_local_password_hash(&user.id, "a-hash-longer-than-the-assert")
        .await;
    assert!(rejected.is_err(), "ASSERT failure reported success");

    db.set_local_password_hash(&user.id, "short").await.unwrap();

    let missing = db.set_local_password_hash("no-such-user", "short").await;
    assert!(
        matches!(missing, Err(DbError::NotFound(_))),
        "reset of a missing user reported {missing:?}"
    );
}

#[tokio::test]
async fn update_helpers_surface_assert_and_event_failures() {
    let db = test_db().await;

    let ann = db
        .create_announcement("Title", "Body", "author", false)
        .await
        .unwrap();
    db.client
        .query("DEFINE FIELD OVERWRITE is_active ON announcement TYPE bool ASSERT $value = true")
        .await
        .unwrap()
        .check()
        .unwrap();
    let deactivated = db.deactivate_announcement(&ann.id).await;
    assert!(deactivated.is_err(), "ASSERT failure reported success");

    let member = db
        .create_member("u-db1-roster", "Roster", OrgRole::Member)
        .await
        .unwrap();
    let team = db
        .create_team("DB1 Team", "game-1", None, None, None)
        .await
        .unwrap();
    db.add_to_roster(&member.id, &team.id, TeamRole::Player)
        .await
        .unwrap();
    fail_writes(&db, "plays_on", "UPDATE").await;
    let removed = db.remove_from_roster(&member.id, &team.id).await;
    assert!(removed.is_err(), "roster removal reported success");
    let role = db
        .update_roster_role(&member.id, &team.id, TeamRole::Captain)
        .await;
    assert!(role.is_err(), "roster role change reported {role:?}");

    allow_writes(&db, "plays_on").await;
    db.remove_from_roster(&member.id, &team.id).await.unwrap();
}
