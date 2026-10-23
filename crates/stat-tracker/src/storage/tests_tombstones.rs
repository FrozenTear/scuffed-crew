//! Tests of `delete_session` and the server-delete tombstones.

use super::tests::snap;
use super::*;

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
