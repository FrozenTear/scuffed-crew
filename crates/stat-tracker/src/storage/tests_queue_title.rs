//! The queue title on a game: stored on every snapshot of the match,
//! carried to later snapshots, and absent on older rows.

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

#[tokio::test]
async fn the_title_goes_on_every_snapshot_and_a_later_one_carries_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    store.insert_match(snap("g2", 1)).await.unwrap();
    assert!(store.attach_queue_title("g1", &open_queue()).await.unwrap());
    assert!(
        !store.attach_queue_title("g1", &open_queue()).await.unwrap(),
        "the same value is not written again"
    );
    assert!(
        !store
            .attach_queue_title("nope", &open_queue())
            .await
            .unwrap()
    );
    store.insert_match(snap("g1", 2)).await.unwrap();
    for row in store.get_all_matches().await.unwrap() {
        let want = (row.session_id == "g1").then(open_queue);
        assert_eq!(row.queue_title, want, "game {}", row.session_id);
    }
    // A weak read replaces it with the warning, value kept.
    let warned = QueueTitle {
        unsure: true,
        ..open_queue()
    };
    assert!(store.attach_queue_title("g1", &warned).await.unwrap());
    assert_eq!(store.session_queue_title("g1").await.unwrap(), Some(warned));
}

#[test]
fn a_row_from_before_the_field_reads_as_none_and_none_is_not_written() {
    let mut old = serde_json::to_value(snap("g1", 1)).unwrap();
    assert!(old.get("queue_title").is_none(), "omitted when None");
    old.as_object_mut().unwrap().remove("queue_title");
    let row: PersonalMatch = serde_json::from_value(old).unwrap();
    assert_eq!(row.queue_title, None);
    // The quiet parts of a title are left out too.
    let v = serde_json::to_value(QueueTitle::default()).unwrap();
    assert_eq!(v, serde_json::json!({ "confidence": 0.0 }));
}
