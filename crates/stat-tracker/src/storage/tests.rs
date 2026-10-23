//! Shared fixtures for the `storage` tests (`snap`, `mark_current`,
//! `reopen_store`, `session_row`, `final_outcome`), and the tests of the
//! store handle: the directory flock and the writer lock held until exit.

use super::*;
use chrono::Utc;

pub(super) fn snap(session_id: &str, elims: u32) -> PersonalMatch {
    PersonalMatch {
        id: None,
        hero: "Test".into(),
        map_name: String::new(),
        game_mode: String::new(),
        role: "Tank".into(),
        outcome: "victory".into(),
        elims,
        deaths: 0,
        assists: 0,
        damage: 0,
        healing: 0,
        mitigation: 0,
        played_at: SurrealDatetime::from(Utc::now()),
        synced: false,
        sync_rev: 0,
        upload_reject: None,
        session_id: session_id.into(),
        corrected_hero: None,
        corrected_role: None,
        corrected_map_name: None,
        corrected_outcome: None,
        corrected_elims: None,
        corrected_deaths: None,
        corrected_assists: None,
        corrected_damage: None,
        corrected_healing: None,
        corrected_mitigation: None,
        edited_fields: Vec::new(),
        edited_at: None,
        heroes_played: Vec::new(),
        segment_resolutions: Vec::new(),
        recognizer: scuffed_types::RECOGNIZER_OCR_V1.to_string(),
        suspect_fields: Vec::new(),
        name_recognizer: None,
    }
}

/// Mark every stored row at its current revision. Test setup for "already
/// synced" — nothing else writes between the read and the CAS.
pub(super) async fn mark_current(store: &LocalStore) {
    let rows = store.get_all_matches().await.unwrap();
    store.mark_synced(&SyncClaim::capture(&rows)).await.unwrap();
}

pub(super) async fn reopen_store(path: &std::path::Path) -> LocalStore {
    let mut last = None;
    for _ in 0..20 {
        match LocalStore::open(path).await {
            Ok(store) => return store,
            Err(err) => {
                last = Some(err.to_string());
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
    panic!("store did not reopen: {}", last.unwrap_or_default());
}

pub(super) fn session_row(session_id: &str, outcome: &str) -> MatchSession {
    MatchSession {
        session_id: session_id.into(),
        hero: "Test".into(),
        map_name: String::new(),
        role: "Tank".into(),
        started_at: SurrealDatetime::from(Utc::now()),
        last_capture_at: SurrealDatetime::from(Utc::now()),
        capture_count: 1,
        final_outcome: outcome.into(),
    }
}

pub(super) async fn final_outcome(store: &LocalStore, session_id: &str) -> Option<String> {
    store
        .get_all_sessions()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.session_id == session_id)
        .map(|s| s.final_outcome)
}

fn expect_busy<T>(result: Result<T, Box<dyn std::error::Error + Send + Sync>>, what: &str) {
    match result {
        Err(err) => assert!(is_store_busy(err.as_ref()), "{what}: {err}"),
        Ok(_) => panic!("{what}"),
    }
}

#[test]
fn store_dir_lock_is_exclusive_until_dropped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = lock_store_dir(dir.path()).expect("first lock");
    expect_busy(lock_store_dir(dir.path()), "second lock must fail");
    drop(first);
    let _second = lock_store_dir(dir.path()).expect("lock after release");
}

#[tokio::test]
async fn second_store_open_fails_until_the_first_drops() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = LocalStore::open(dir.path()).await.expect("open");
    expect_busy(LocalStore::open(dir.path()).await, "second open must fail");
    drop(first);
    // Our directory flock drops with the handle. SurrealKV's own LOCK file
    // can stay busy while its engine thread shuts down, so this checks the
    // directory inode directly instead of opening the store again.
    let db_path = dir.path().join("stats.surrealkv");
    let _released = lock_store_dir(&db_path).expect("directory flock released with the handle");
}

#[tokio::test]
async fn writer_lock_survives_store_drop_until_process_exit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open");
    store.hold_writer_lock_until_exit();
    drop(store);
    expect_busy(
        LocalStore::open(dir.path()).await,
        "leaked dup must still hold the flock",
    );
}
