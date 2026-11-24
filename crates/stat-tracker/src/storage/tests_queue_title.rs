//! The queue title on a game: stored once on every snapshot of the match,
//! carried to later snapshots, re-queued for upload, and absent on older
//! rows.

use super::tests::snap;
use super::*;
use crate::model::QueueTitle;

fn open_queue() -> QueueTitle {
    QueueTitle {
        value: Some("OPEN QUEUE".into()),
        confidence: 0.93,
        ..QueueTitle::default()
    }
}

fn unsure() -> QueueTitle {
    QueueTitle {
        unsure: true,
        ..QueueTitle::default()
    }
}

#[tokio::test]
async fn the_title_goes_on_every_snapshot_and_a_later_one_carries_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    store.insert_match(snap("g1", 2)).await.unwrap();
    assert!(store.attach_queue_title("g1", &open_queue()).await.unwrap());
    assert!(
        !store
            .attach_queue_title("nope", &open_queue())
            .await
            .unwrap()
    );
    // Every snapshot stored before the attach has it, not only the last.
    let g1 = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(g1.len(), 2);
    assert!(g1.iter().all(|r| r.queue_title == Some(open_queue())));
    // A game inserted after g1 has a title does not take g1's.
    store.insert_match(snap("g2", 1)).await.unwrap();
    store.insert_match(snap("g1", 3)).await.unwrap();
    for row in store.get_all_matches().await.unwrap() {
        let want = (row.session_id == "g1").then(open_queue);
        assert_eq!(row.queue_title, want, "game {}", row.session_id);
    }
}

#[tokio::test]
async fn a_stored_title_is_written_once_and_never_replaced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    assert!(store.attach_queue_title("g1", &open_queue()).await.unwrap());
    // The same value, a warning and another value all leave it as it is.
    let role = QueueTitle {
        value: Some("ROLE QUEUE".into()),
        ..open_queue()
    };
    for later in [open_queue(), unsure(), role] {
        assert!(!store.attach_queue_title("g1", &later).await.unwrap());
        assert_eq!(
            store.session_queue_title("g1").await.unwrap(),
            Some(open_queue())
        );
    }
    // A warning stored first stays too: the game was read once.
    store.insert_match(snap("g2", 1)).await.unwrap();
    assert!(store.attach_queue_title("g2", &unsure()).await.unwrap());
    assert!(!store.attach_queue_title("g2", &open_queue()).await.unwrap());
    assert_eq!(
        store.session_queue_title("g2").await.unwrap(),
        Some(unsure())
    );
}

#[tokio::test]
async fn attaching_a_title_puts_a_synced_game_back_in_the_upload_queue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    let rows = store.get_unsynced().await.unwrap();
    assert_eq!(rows.len(), 1);
    store.mark_synced(&SyncClaim::capture(&rows)).await.unwrap();
    assert!(store.get_unsynced().await.unwrap().is_empty());
    assert!(store.attach_queue_title("g1", &open_queue()).await.unwrap());
    let queued = store.get_unsynced().await.unwrap();
    assert_eq!(queued.len(), 1, "the title re-queues the game");
    assert_eq!(queued[0].queue_title, Some(open_queue()));
    assert!(queued[0].sync_rev > rows[0].sync_rev);
    // An upload claimed before the attach does not mark it synced.
    store.mark_synced(&SyncClaim::capture(&rows)).await.unwrap();
    assert_eq!(store.get_unsynced().await.unwrap().len(), 1);
    // A warning changes no uploaded byte: a synced game stays synced.
    store.insert_match(snap("g2", 1)).await.unwrap();
    let g2: Vec<_> = store
        .get_unsynced()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.session_id == "g2")
        .collect();
    store.mark_synced(&SyncClaim::capture(&g2)).await.unwrap();
    assert!(store.attach_queue_title("g2", &unsure()).await.unwrap());
    let queued = store.get_unsynced().await.unwrap();
    assert!(queued.iter().all(|r| r.session_id != "g2"), "{queued:?}");
}

#[tokio::test]
async fn a_row_from_before_the_field_reads_as_none_and_takes_a_title() {
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
    assert_eq!(rows[0].queue_title, None);
    assert_eq!(store.session_queue_title("legacy").await.unwrap(), None);
    assert!(
        store
            .attach_queue_title("legacy", &open_queue())
            .await
            .unwrap()
    );
    assert_eq!(
        store.session_queue_title("legacy").await.unwrap(),
        Some(open_queue())
    );
    assert_eq!(store.get_unsynced().await.unwrap().len(), 1);
}

#[test]
fn none_is_not_written_to_json() {
    let row = serde_json::to_value(snap("g1", 1)).unwrap();
    assert!(row.get("queue_title").is_none(), "omitted when None");
    // The quiet parts of a title are left out too.
    let v = serde_json::to_value(QueueTitle::default()).unwrap();
    assert_eq!(v, serde_json::json!({ "confidence": 0.0 }));
}
