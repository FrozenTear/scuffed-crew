//! Role Queue 6v6 (#654) on the store: a title that changes no uploaded
//! byte does not re-queue a game, and a store written by 0.6 (rows with no
//! team size) opens with its rank history unchanged and files each rank
//! under its own queue.

use super::tests::{reopen_store, snap};
use super::*;
use crate::model::{QueueTitle, RankNumber, RankText, RankUpdate};
use scuffed_types::api::{RankQueue, RankRole};

fn title(value: &str) -> QueueTitle {
    QueueTitle {
        value: Some(value.into()),
        confidence: 0.93,
        ..QueueTitle::default()
    }
}

/// Insert `session_id` and mark it synced.
async fn synced_game(store: &LocalStore, session_id: &str) {
    store.insert_match(snap(session_id, 1)).await.unwrap();
    let rows: Vec<_> = store
        .get_unsynced()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.session_id == session_id)
        .collect();
    store.mark_synced(&SyncClaim::capture(&rows)).await.unwrap();
}

fn queued(rows: &[PersonalMatch], session_id: &str) -> bool {
    rows.iter().any(|r| r.session_id == session_id)
}

/// A confident title the upload does not send as a `rank` object (a mode
/// that is not Competitive, or one naming neither ROLE nor OPEN with no
/// rank screen read) is stored but leaves a synced game synced. A title
/// naming ROLE still re-queues it, as before.
#[tokio::test]
async fn a_title_that_changes_no_uploaded_byte_does_not_requeue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let cases = [
        ("qp", "ROLE QUEUE: QUICK PLAY", false),
        ("qp6", "OPEN QUEUE: 6V6 QUICK PLAY", false),
        ("arcade", "ARCADE: OPEN QUEUE", false),
        ("comp", "COMPETITIVE", false),
        ("role", "ROLE QUEUE", true),
        ("role6", "ROLE QUEUE 6V6", true),
    ];
    for (session, _, _) in cases {
        synced_game(&store, session).await;
    }
    assert!(store.get_unsynced().await.unwrap().is_empty());
    for (session, value, requeued) in cases {
        assert!(
            store
                .attach_queue_title(session, &title(value))
                .await
                .unwrap()
        );
        assert_eq!(
            store.session_queue_title(session).await.unwrap(),
            Some(title(value)),
            "{value}: the title is stored either way"
        );
        let rows = store.get_unsynced().await.unwrap();
        assert_eq!(queued(&rows, session), requeued, "{value}");
    }
}

/// A game the site set aside keeps its reason when a title that changes no
/// uploaded byte is stored, so it is not retried for nothing.
#[tokio::test]
async fn a_set_aside_game_is_not_retried_for_a_title_that_changes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("qp", 1)).await.unwrap();
    let rows = store.get_unsynced().await.unwrap();
    store
        .quarantine_upload(&SyncClaim::capture(&rows), None, "matches[0]: refused")
        .await
        .unwrap();
    assert!(store.get_unsynced().await.unwrap().is_empty());
    let quick = title("ROLE QUEUE: QUICK PLAY");
    assert!(store.attach_queue_title("qp", &quick).await.unwrap());
    assert!(store.get_unsynced().await.unwrap().is_empty());
    let row = &store.get_session_snapshots("qp").await.unwrap()[0];
    assert!(row.upload_rejection().is_some(), "the reason is kept");
}

/// A row as 0.6.0 wrote it: a queue title and a rank screen read, but no
/// `team_size` (the field is new in 0.7), already uploaded.
async fn insert_06_row(store: &LocalStore, session_id: &str, elims: u32) {
    store
        .db
        .query(
            "CREATE personal_match SET hero = 'Reinhardt', map_name = 'Busan', \
             game_mode = 'Control', role = 'Tank', outcome = 'victory', elims = $e, \
             deaths = 0, assists = 0, damage = 1, healing = 0, mitigation = 0, \
             played_at = time::now(), synced = true, sync_rev = 3, session_id = $sid, \
             recognizer = 'cv-v1', \
             queue_title = { value: 'ROLE QUEUE', confidence: 0.93f }, \
             rank_update = { role: { value: 'Tank', confidence: 0.9f }, \
               role_rank: { value: 'DIAMOND', confidence: 0.81f }, \
               division: { value: 2, confidence: 0.9f } }",
        )
        .bind(("e", elims))
        .bind(("sid", session_id.to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
}

fn rank_06() -> RankUpdate {
    RankUpdate {
        role: Some(RankText {
            value: "Tank".into(),
            confidence: 0.9,
        }),
        role_rank: Some(RankText {
            value: "DIAMOND".into(),
            confidence: 0.81,
        }),
        division: Some(RankNumber {
            value: 2,
            confidence: 0.9,
        }),
        ..RankUpdate::default()
    }
}

/// Opening a 0.6 store: the rows read back with no team size, the same
/// rank and title, still synced (nothing re-queued on open), and their
/// rank is filed under the unknown queue, never a known one. A 0.6
/// `matches.jsonl` line still reads. A later Tab on 0.7 that reads a
/// 12-player board files the game under role queue 6v6 and leaves the
/// stored rank as it was.
#[tokio::test]
async fn a_06_store_opens_with_its_rank_history_and_an_unknown_queue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    insert_06_row(&store, "s06", 4).await;
    drop(store);
    std::fs::write(
        dir.path().join("matches.jsonl"),
        concat!(
            r#"{"hero":"Reinhardt","map_name":"Busan","game_mode":"Control","role":"Tank","#,
            r#""outcome":"victory","elims":4,"deaths":0,"assists":0,"damage":1,"healing":0,"#,
            r#""mitigation":0,"played_at":"2026-11-20T20:00:00Z","synced":true,"#,
            r#""session_id":"s06","queue_title":{"value":"ROLE QUEUE","confidence":0.93},"#,
            r#""rank_update":{"role":{"value":"Tank","confidence":0.9}}}"#,
            "\n"
        ),
    )
    .unwrap();

    let store = reopen_store(dir.path()).await;
    let rows = store.get_session_snapshots("s06").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].team_size, None);
    assert_eq!(rows[0].rank_update, Some(rank_06()));
    assert_eq!(rows[0].queue_title, Some(title("ROLE QUEUE")));
    assert!(rows[0].synced, "opening does not re-queue a 0.6 row");
    assert!(store.get_unsynced().await.unwrap().is_empty());
    let slot = crate::sync::rank_slot(&rows[0]).expect("a rank slot");
    assert_eq!(slot.queue, RankQueue::Unknown);
    assert_eq!(slot.role, Some(RankRole::Tank));

    let log = read_match_log(dir.path());
    assert_eq!(log.len(), 1, "the 0.6 log line reads");
    assert_eq!(log[0].team_size, None);
    assert_eq!(log[0].session_id, "s06");

    // A later 0.7 Tab of the same game reads a 12-player board.
    let mut later = snap("s06", 5);
    later.team_size = Some(6);
    store.insert_match(later).await.unwrap();
    let rows = store.get_session_snapshots("s06").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.rank_update == Some(rank_06())));
    assert_eq!(rows[0].team_size, None, "the 0.6 row is not rewritten");
    let newest = rows.iter().find(|r| r.team_size == Some(6)).unwrap();
    let slot = crate::sync::rank_slot(newest).unwrap();
    assert_eq!(slot.queue, RankQueue::Role6v6);
    assert_eq!(slot.role, Some(RankRole::Tank));
}

fn tank_gold() -> RankUpdate {
    RankUpdate {
        role: Some(RankText {
            value: "Tank".into(),
            confidence: 0.9,
        }),
        role_rank: Some(RankText {
            value: "GOLD".into(),
            confidence: 0.9,
        }),
        ..RankUpdate::default()
    }
}

/// A synced or set-aside game with a Tank GOLD read and `size` a side.
async fn ranked_game(store: &LocalStore, session_id: &str, size: Option<u8>, set_aside: bool) {
    let mut row = snap(session_id, 1);
    row.rank_update = Some(tank_gold());
    row.team_size = size;
    store.insert_match(row).await.unwrap();
    let rows: Vec<_> = store
        .get_unsynced()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.session_id == session_id)
        .collect();
    let claims = SyncClaim::capture(&rows);
    if set_aside {
        store
            .quarantine_upload(&claims, None, "matches[0]: refused")
            .await
            .unwrap();
    } else {
        store.mark_synced(&claims).await.unwrap();
    }
}

/// R1 and R4 of the #926 review: on a game with a rank screen read, a
/// title re-queues it whenever the uploaded `rank` object changes, even
/// when the slot does not. A bare `COMPETITIVE` title leaves the slot at
/// (unknown, Tank) but adds `rank.queue_title`; a title naming a mode that
/// is not Competitive drops the whole object. Both re-queue a synced game
/// and retry a set-aside one.
#[tokio::test]
async fn a_title_that_changes_the_rank_object_requeues_even_with_the_same_slot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let cases = [
        ("COMPETITIVE", None, true),
        ("COMPETITIVE", Some(5), true),
        ("ROLE QUEUE: QUICK PLAY", None, false),
        ("ARCADE: OPEN QUEUE", Some(6), false),
    ];
    for (i, (value, size, same_slot)) in cases.into_iter().enumerate() {
        for set_aside in [false, true] {
            let session = format!("r1-{i}-{set_aside}");
            ranked_game(&store, &session, size, set_aside).await;
            let before = store.get_session_snapshots(&session).await.unwrap();
            let slot_before = crate::sync::rank_slot(&before[0]);
            assert!(
                store
                    .get_unsynced()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.session_id != session)
            );
            assert!(
                store
                    .attach_queue_title(&session, &title(value))
                    .await
                    .unwrap()
            );
            let after = store.get_session_snapshots(&session).await.unwrap();
            let slot_after = crate::sync::rank_slot(&after[0]);
            assert_eq!(slot_before == slot_after, same_slot, "{value} {size:?}");
            let rows = store.get_unsynced().await.unwrap();
            assert!(
                queued(&rows, &session),
                "{value} {size:?} set_aside={set_aside}"
            );
            assert!(after[0].upload_rejection().is_none(), "{value}: retried");
        }
    }
}

/// Q2: the newest row decides. Tabs read 5, then 6, then a Tab with no
/// size: it carries 6 (the newest read), so its slot is role queue 6v6, the
/// same queue the upload sends for it.
#[tokio::test]
async fn a_tab_with_no_size_carries_the_newest_size() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    for (elims, size) in [(1, Some(5)), (2, Some(6)), (3, None)] {
        let mut row = snap("q2", elims);
        row.hero = "Ana".into();
        row.map_name = "Busan".into();
        row.game_mode = "Control".into();
        row.role = "Support".into();
        row.team_size = size;
        row.queue_title = Some(title("ROLE QUEUE"));
        row.rank_update = Some(tank_gold());
        store.insert_match(row).await.unwrap();
    }
    let rows = store.get_session_snapshots("q2").await.unwrap();
    let newest = rows.iter().find(|r| r.elims == 3).unwrap();
    assert_eq!(newest.team_size, Some(6));
    let slot = crate::sync::rank_slot(newest).unwrap();
    assert_eq!(slot.queue, RankQueue::Role6v6);
    let body = serde_json::to_value(crate::sync::upload_request(
        std::slice::from_ref(newest),
        &[],
    ))
    .unwrap();
    assert_eq!(body["matches"][0]["rank"]["queue"]["value"], "role_6v6");
}
