//! Tests of `delete_session` and the server-delete tombstones, and of a
//! capture that lands after its game was deleted.

use super::tests::{final_outcome, session_row, snap};
use super::*;
use chrono::Utc;

#[tokio::test]
async fn delete_session_tombstones_until_acknowledged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("s1", 1)).await.unwrap();
    append_match_log(dir.path(), &snap("s1", 1));

    store.delete_session("s1").await.unwrap();
    // Local rows gone, jsonl fallback cleaned, tombstone pending for sync.
    assert_eq!(store.match_count().await.unwrap(), 0);
    assert!(read_match_log(dir.path()).is_empty());
    assert_eq!(
        store.get_pending_tombstones().await.unwrap(),
        vec!["s1".to_string()]
    );

    // Server acknowledged — tombstone cleared, nothing left to send.
    store.clear_tombstones(vec!["s1".into()]).await.unwrap();
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
    // Clearing all data never leaks tombstones either.
    store.clear_all_data().await.unwrap();
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
}

/// O3: a tombstone write that fails must fail the delete and leave the
/// game in place. Before the fix the three statements ran unchecked and
/// outside a transaction, so the local rows went, no tombstone was
/// stored, the call returned Ok, and the site copy was never deleted.
#[tokio::test]
async fn delete_session_is_atomic_when_the_tombstone_write_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("s1", 1)).await.unwrap();
    store.insert_match(snap("s1", 2)).await.unwrap();
    // Make the tombstone CREATE fail for this one session.
    store
        .db
        .query("DEFINE FIELD OVERWRITE session_id ON deleted_session ASSERT $value != 's1'")
        .await
        .unwrap()
        .check()
        .unwrap();

    assert!(
        store.delete_session("s1").await.is_err(),
        "a failed tombstone write must surface as an error"
    );
    assert_eq!(
        store.match_count().await.unwrap(),
        2,
        "the local rows stay when the tombstone could not be written"
    );
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
}

/// A delete whose checked transaction fails must clear its in-process
/// mark. Otherwise every later row of that game would be dropped for the
/// rest of the process.
#[tokio::test]
async fn a_failed_checked_delete_does_not_block_later_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("s1", 1)).await.unwrap();
    store
        .db
        .query("DEFINE FIELD OVERWRITE session_id ON deleted_session ASSERT $value != 's1'")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(store.delete_session("s1").await.is_err());
    assert_eq!(store.match_count().await.unwrap(), 1);
    store.insert_match(snap("s1", 2)).await.unwrap();
    assert_eq!(
        store.get_session_snapshots("s1").await.unwrap().len(),
        2,
        "a later row of the game is still stored"
    );
}

/// A committed transactional delete still blocks a late capture.
#[tokio::test]
async fn a_checked_delete_still_blocks_a_late_capture() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("s1", 1)).await.unwrap();
    store.delete_session("s1").await.unwrap();
    let late = snap("s1", 2);
    append_match_log(dir.path(), &late);
    store.insert_match(late).await.unwrap();
    assert!(store.get_session_snapshots("s1").await.unwrap().is_empty());
    assert!(read_match_log(dir.path()).is_empty());
}

/// A row written after its game was marked deleted (the delete landed
/// during insert_match's await) is removed again; other games' rows are
/// untouched.
#[tokio::test]
async fn a_row_written_while_its_game_was_deleted_is_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("keep", 1)).await.unwrap();
    let created: Option<PersonalMatch> = store
        .db
        .create("personal_match")
        .content(snap("raced", 1))
        .await
        .unwrap();
    // The delete marked the session after insert_match's first check.
    store
        .deleted_here
        .lock()
        .unwrap()
        .insert("raced".to_string());
    store.drop_if_deleted_meanwhile(created).await.unwrap();
    assert!(
        store
            .get_session_snapshots("raced")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.get_session_snapshots("keep").await.unwrap().len(), 1);
}

/// DATA-4: tombstones queued for sessions that turn local-only are
/// stored once per session and ride the next sync like a GUI delete.
#[tokio::test]
async fn queue_tombstones_dedups_and_skips_pending_and_blank_ids() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("gone", 1)).await.unwrap();
    store.delete_session("gone").await.unwrap();

    store
        .queue_tombstones(&[
            "dm".to_string(),
            "dm".to_string(),
            String::new(),
            "gone".to_string(),
        ])
        .await
        .unwrap();
    let mut pending = store.get_pending_tombstones().await.unwrap();
    pending.sort();
    assert_eq!(pending, vec!["dm".to_string(), "gone".to_string()]);

    store.queue_tombstones(&["dm".to_string()]).await.unwrap();
    assert_eq!(store.get_pending_tombstones().await.unwrap().len(), 2);
}

/// MAIN-1 (3): a capture that was in flight when its game was deleted
/// does not bring the game back, neither the session row nor the
/// snapshot nor the `matches.jsonl` line.
#[tokio::test]
async fn a_capture_landing_after_a_delete_does_not_resurrect_the_game() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store
        .create_session(&session_row("gone", "unknown"))
        .await
        .unwrap();
    store.insert_match(snap("gone", 1)).await.unwrap();
    store.delete_session("gone").await.unwrap();
    // The in-flight capture's writes, in the order the capture runs them.
    store
        .create_session(&session_row("gone", "unknown"))
        .await
        .unwrap();
    store
        .append_capture("gone", SurrealDatetime::from(Utc::now()), "victory")
        .await
        .unwrap();
    append_match_log(dir.path(), &snap("gone", 2));
    store.insert_match(snap("gone", 2)).await.unwrap();
    assert_eq!(store.match_count().await.unwrap(), 0);
    assert_eq!(final_outcome(&store, "gone").await, None);
    assert!(read_match_log(dir.path()).is_empty());
    assert_eq!(
        store.get_pending_tombstones().await.unwrap(),
        vec!["gone".to_string()],
        "the tombstone still goes to the server"
    );
    // Other games are stored as before.
    store.insert_match(snap("kept", 1)).await.unwrap();
    assert_eq!(store.match_count().await.unwrap(), 1);
}

/// A pending tombstone is not a delete. A game whose server copy goes
/// (re-read as an untracked map or mode) keeps its local rows and can be
/// the open game: its new rows are stored and its `matches.jsonl` lines
/// stay. Only a delete in this process blocks a late capture.
#[tokio::test]
async fn a_pending_tombstone_without_a_delete_does_not_block_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store
        .create_session(&session_row("dm", "unknown"))
        .await
        .unwrap();
    append_match_log(dir.path(), &snap("dm", 1));
    store.insert_match(snap("dm", 1)).await.unwrap();
    // Stands in for a server-side removal queued for a game kept here.
    store.insert_tombstone("dm").await.unwrap();
    append_match_log(dir.path(), &snap("dm", 2));
    store.insert_match(snap("dm", 2)).await.unwrap();
    store
        .create_session(&session_row("dm", "victory"))
        .await
        .unwrap();
    assert_eq!(store.get_session_snapshots("dm").await.unwrap().len(), 2);
    assert_eq!(
        final_outcome(&store, "dm").await.as_deref(),
        Some("victory")
    );
    assert_eq!(read_match_log(dir.path()).len(), 2);
}
