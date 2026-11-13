//! The rank update on a game: stored once on every snapshot of the match,
//! carried to later snapshots, re-queued for upload, and absent on older
//! rows.

use super::tests::snap;
use super::*;
use crate::model::{RankNumber, RankText, RankUpdate};

fn rank(sr: i64) -> RankUpdate {
    RankUpdate {
        hero_sr: Some(RankNumber {
            value: sr,
            confidence: 0.85,
        }),
        role: Some(RankText {
            value: "Support".into(),
            confidence: 0.9,
        }),
        ..RankUpdate::default()
    }
}

#[tokio::test]
async fn the_rank_goes_on_every_snapshot_once_and_a_later_one_carries_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    store.insert_match(snap("g1", 2)).await.unwrap();
    assert!(store.attach_rank_update("g1", &rank(3412)).await.unwrap());
    // unknown game, a second read and an empty read: nothing written
    assert!(!store.attach_rank_update("nope", &rank(3412)).await.unwrap());
    assert!(!store.attach_rank_update("g1", &rank(2000)).await.unwrap());
    store.insert_match(snap("g2", 1)).await.unwrap();
    assert!(
        !store
            .attach_rank_update("g2", &RankUpdate::default())
            .await
            .unwrap()
    );
    let g1 = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(g1.len(), 2);
    assert!(g1.iter().all(|r| r.rank_update == Some(rank(3412))));
    // a later snapshot of g1 carries it, another game does not take it
    store.insert_match(snap("g1", 3)).await.unwrap();
    for row in store.get_all_matches().await.unwrap() {
        let want = (row.session_id == "g1").then(|| rank(3412));
        assert_eq!(row.rank_update, want, "game {}", row.session_id);
    }
}

#[tokio::test]
async fn attaching_a_rank_puts_a_synced_game_back_in_the_upload_queue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    let rows = store.get_unsynced().await.unwrap();
    store.mark_synced(&SyncClaim::capture(&rows)).await.unwrap();
    assert!(store.get_unsynced().await.unwrap().is_empty());
    assert!(store.attach_rank_update("g1", &rank(3412)).await.unwrap());
    let queued = store.get_unsynced().await.unwrap();
    assert_eq!(queued.len(), 1, "the rank re-queues the game");
    assert_eq!(queued[0].rank_update, Some(rank(3412)));
    assert!(queued[0].sync_rev > rows[0].sync_rev);
    // the stats themselves are as they were
    assert_eq!(queued[0].elims, rows[0].elims);
    assert_eq!(queued[0].hero, rows[0].hero);
}

#[tokio::test]
async fn a_rank_the_upload_drops_is_stored_but_leaves_the_queue_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    let rows = store.get_unsynced().await.unwrap();
    store.mark_synced(&SyncClaim::capture(&rows)).await.unwrap();
    // an Open role with no open queue title is not sent
    let open = RankUpdate {
        role: Some(RankText {
            value: "Open".into(),
            confidence: 0.5,
        }),
        ..RankUpdate::default()
    };
    assert!(store.attach_rank_update("g1", &open).await.unwrap());
    assert!(
        store.get_unsynced().await.unwrap().is_empty(),
        "no uploaded byte changed: still synced"
    );
    let stored = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(stored[0].rank_update, Some(open));
    assert_eq!(stored[0].sync_rev, rows[0].sync_rev);
}

#[tokio::test]
async fn a_row_from_before_the_field_reads_as_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store
        .db
        .query(
            "CREATE personal_match SET hero = 'Ana', map_name = 'Busan', game_mode = 'x', \
             role = 'Support', outcome = 'victory', elims = 1, deaths = 0, assists = 0, \
             damage = 1, healing = 0, mitigation = 0, played_at = time::now(), \
             synced = true, session_id = 'legacy'",
        )
        .await
        .unwrap();
    let rows = store.get_session_snapshots("legacy").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].rank_update, None);
    assert!(
        store
            .attach_rank_update("legacy", &rank(3412))
            .await
            .unwrap()
    );
    let rows = store.get_session_snapshots("legacy").await.unwrap();
    assert_eq!(rows[0].rank_update, Some(rank(3412)));
}
