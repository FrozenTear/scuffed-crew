//! Tests of the upload queue: the `sync_rev` CAS in `mark_synced`, upload
//! quarantine and retry.

use super::tests::{reopen_store, snap};
use super::*;
use chrono::Utc;

#[tokio::test]
async fn mark_synced_skips_row_revised_during_upload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("s1", 10)).await.unwrap();

    let before = store.get_unsynced().await.unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].sync_rev, 0);
    let claims = SyncClaim::capture(&before);

    // Outcome back-fill while the HTTP upload of `claims` is in flight.
    store.set_session_outcome("s1", "defeat").await.unwrap();

    store.mark_synced(&claims).await.unwrap();

    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "defeat");
    assert!(
        !rows[0].synced,
        "a row revised during upload must stay queued"
    );
    assert!(rows[0].sync_rev > claims[0].sync_rev);

    // The next sync, capturing the new revision, is allowed to commit.
    let again = SyncClaim::capture(&store.get_unsynced().await.unwrap());
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].sync_rev, rows[0].sync_rev);
    store.mark_synced(&again).await.unwrap();
    let committed = store.get_all_matches().await.unwrap();
    assert!(committed[0].synced);
    assert_eq!(committed[0].outcome, "defeat");
    assert!(store.get_unsynced().await.unwrap().is_empty());
}

#[tokio::test]
async fn quarantined_row_stays_out_of_the_queue_until_retry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.unwrap();
    store.insert_match(snap("keep", 1)).await.unwrap();
    store.insert_match(snap("bad", 2)).await.unwrap();
    let rows = store.get_unsynced().await.unwrap();
    let bad: Vec<_> = rows
        .iter()
        .filter(|row| row.session_id == "bad")
        .cloned()
        .collect();
    store
        .quarantine_upload(
            &SyncClaim::capture(&bad),
            None,
            "matches[0]: hero is not allowed",
        )
        .await
        .unwrap();
    let pending = store.get_unsynced().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].session_id, "keep");
    let stored = store.get_all_matches().await.unwrap();
    let bad_row = stored.iter().find(|row| row.session_id == "bad").unwrap();
    assert!(!bad_row.synced);
    assert_eq!(
        bad_row.upload_rejection(),
        Some("Upload rejected by server: hero is not allowed")
    );
    store
        .apply_command(&StoreCommand::RetryUpload {
            session_id: "bad".into(),
        })
        .await
        .unwrap();
    let again = store.get_unsynced().await.unwrap();
    assert_eq!(again.len(), 2);
    assert!(again.iter().all(|row| row.upload_rejection().is_none()));
}

#[tokio::test]
async fn retry_rejected_uploads_clears_every_quarantine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.unwrap();
    store.insert_match(snap("keep", 1)).await.unwrap();
    store.insert_match(snap("bad-a", 2)).await.unwrap();
    store.insert_match(snap("bad-b", 3)).await.unwrap();
    // Same session: one snapshot already uploaded, one still quarantined.
    // Clearing the flag must not mark the uploaded snapshot unsynced.
    store.insert_match(snap("shared", 10)).await.unwrap();
    store.insert_match(snap("shared", 11)).await.unwrap();
    let shared = store.get_all_matches().await.unwrap();
    let already = shared.iter().find(|row| row.elims == 10).unwrap().clone();
    store
        .mark_synced(&SyncClaim::capture(std::slice::from_ref(&already)))
        .await
        .unwrap();
    let shared = store.get_all_matches().await.unwrap();
    let held = shared.iter().find(|row| row.elims == 11).unwrap().clone();
    assert!(!held.synced);
    let rows = store.get_unsynced().await.unwrap();
    for sid in ["bad-a", "bad-b"] {
        let bad: Vec<_> = rows
            .iter()
            .filter(|row| row.session_id == sid)
            .cloned()
            .collect();
        store
            .quarantine_upload(
                &SyncClaim::capture(&bad),
                None,
                "matches[0]: hero is not allowed",
            )
            .await
            .unwrap();
    }
    store
        .quarantine_upload(
            &SyncClaim::capture(std::slice::from_ref(&held)),
            None,
            "matches[0]: hero is not allowed",
        )
        .await
        .unwrap();
    assert_eq!(store.get_unsynced().await.unwrap().len(), 1);
    store
        .apply_command(&StoreCommand::RetryRejectedUploads)
        .await
        .unwrap();
    let again = store.get_unsynced().await.unwrap();
    assert_eq!(again.len(), 4);
    assert!(again.iter().all(|row| row.upload_rejection().is_none()));
    assert!(again.iter().any(|row| row.elims == 11));
    let stored = store.get_all_matches().await.unwrap();
    let synced_sibling = stored.iter().find(|row| row.elims == 10).unwrap();
    let cleared = stored.iter().find(|row| row.elims == 11).unwrap();
    assert!(
        synced_sibling.synced,
        "clearing a quarantine leaves the synced snapshot synced"
    );
    assert!(synced_sibling.upload_rejection().is_none());
    assert!(!cleared.synced);
    assert!(cleared.upload_rejection().is_none());
    assert!(
        stored
            .iter()
            .filter(|row| row.session_id != "shared" || row.elims != 10)
            .all(|row| !row.synced)
    );
}

#[tokio::test]
async fn editing_a_quarantined_row_queues_it_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.unwrap();
    store.insert_match(snap("bad", 2)).await.unwrap();
    let rows = store.get_unsynced().await.unwrap();
    store
        .quarantine_upload(
            &SyncClaim::capture(&rows),
            None,
            "matches[0]: hero is not allowed",
        )
        .await
        .unwrap();
    assert!(store.get_unsynced().await.unwrap().is_empty());
    store
        .edit_match(
            "bad",
            &MatchEdit {
                elims: Some(9),
                ..MatchEdit::default()
            },
        )
        .await
        .unwrap();
    let pending = store.get_unsynced().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].upload_rejection().is_none());
    assert_eq!(pending[0].corrected_elims, Some(9));
}

#[tokio::test]
async fn reject_message_drops_the_batch_index() {
    let cases = [
        (
            "a",
            "matches[0]: hero is not allowed",
            "Upload rejected by server: hero is not allowed",
        ),
        (
            "b",
            "matches[12]: map is unknown",
            "Upload rejected by server: map is unknown",
        ),
        (
            "c",
            "hero is not allowed",
            "Upload rejected by server: hero is not allowed",
        ),
        ("d", "matches[0]: ", "Upload rejected by server"),
    ];
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.unwrap();
    for (index, (sid, raw, _)) in cases.iter().enumerate() {
        store.insert_match(snap(sid, index as u32)).await.unwrap();
        let rows = store.get_unsynced().await.unwrap();
        let row = rows
            .into_iter()
            .find(|row| row.session_id == *sid)
            .expect("row");
        store
            .quarantine_upload(&SyncClaim::capture(std::slice::from_ref(&row)), None, raw)
            .await
            .unwrap();
    }
    let stored = store.get_all_matches().await.unwrap();
    for (sid, _, expect) in cases {
        let row = stored.iter().find(|row| row.session_id == sid).unwrap();
        assert_eq!(row.upload_rejection(), Some(expect), "{sid}");
    }
}

#[tokio::test]
async fn upload_reject_survives_store_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = LocalStore::open(dir.path()).await.unwrap();
        store.insert_match(snap("bad", 2)).await.unwrap();
        let rows = store.get_unsynced().await.unwrap();
        store
            .quarantine_upload(
                &SyncClaim::capture(&rows),
                Some("invalid_hero"),
                "matches[3]: hero is not allowed",
            )
            .await
            .unwrap();
    }
    let store = reopen_store(dir.path()).await;
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].synced);
    assert_eq!(
        rows[0].upload_rejection(),
        Some("Upload rejected by server: invalid_hero: hero is not allowed")
    );
    assert!(store.get_unsynced().await.unwrap().is_empty());
}

#[tokio::test]
async fn mark_synced_commits_unchanged_revision() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("s1", 4)).await.unwrap();
    let claims = SyncClaim::capture(&store.get_unsynced().await.unwrap());
    store.mark_synced(&claims).await.unwrap();
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].synced);
    assert!(store.get_unsynced().await.unwrap().is_empty());
}

#[tokio::test]
async fn revised_sibling_keeps_whole_session_unsynced() {
    // Two snapshots, one session. Map back-fill touches only the older
    // row (the newer already has the map). Marking the unchanged newer
    // row synced would let the next upload send the older snapshot alone
    // and last-write its lower stats over the server row.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let mut older = snap("s1", 3);
    older.map_name = String::new();
    older.played_at = SurrealDatetime::from(Utc::now() - chrono::Duration::seconds(30));
    let mut newer = snap("s1", 20);
    newer.map_name = "Ilios".into();
    newer.played_at = SurrealDatetime::from(Utc::now());
    // A second session that did not change must still commit.
    let mut other = snap("s2", 7);
    other.map_name = "Busan".into();
    store.insert_match(older).await.unwrap();
    store.insert_match(newer).await.unwrap();
    store.insert_match(other).await.unwrap();

    let claims = SyncClaim::capture(&store.get_unsynced().await.unwrap());
    assert_eq!(claims.len(), 3);
    store.set_session_map("s1", "Ilios").await.unwrap();
    store.mark_synced(&claims).await.unwrap();

    let rows = store.get_all_matches().await.unwrap();
    let s1: Vec<_> = rows.iter().filter(|m| m.session_id == "s1").collect();
    let s2: Vec<_> = rows.iter().filter(|m| m.session_id == "s2").collect();
    assert_eq!(s1.len(), 2);
    assert!(
        s1.iter().all(|m| !m.synced && m.map_name == "Ilios"),
        "changed session stays fully unsynced: {s1:?}"
    );
    assert_eq!(s2.len(), 1);
    assert!(s2[0].synced, "untouched session still commits");
}

#[tokio::test]
async fn legacy_row_without_sync_rev_marks_at_revision_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store
        .db
        .query(
            "CREATE personal_match SET hero = 'Ana', map_name = 'Busan', game_mode = 'x', \
             role = 'Support', outcome = 'victory', elims = 1, deaths = 0, assists = 0, \
             damage = 1, healing = 0, mitigation = 0, played_at = time::now(), \
             synced = false, session_id = 'legacy'",
        )
        .await
        .unwrap();
    let rows = store.get_unsynced().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].sync_rev, 0, "missing sync_rev reads as 0");
    assert_eq!(
        rows[0].stored_recognizer(),
        scuffed_types::RECOGNIZER_OCR_V1,
        "a row written before recognizer existed still uploads as ocr-v1"
    );
    assert!(rows[0].suspect_fields.is_empty());
    assert_eq!(
        rows[0].name_recognizer, None,
        "a row written before name_recognizer existed uploads with the field omitted"
    );
    store.mark_synced(&SyncClaim::capture(&rows)).await.unwrap();
    let after = store.get_all_matches().await.unwrap();
    assert!(after[0].synced);
}
