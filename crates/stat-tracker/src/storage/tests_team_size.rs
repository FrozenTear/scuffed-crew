//! The board's team size on a game: carried onto a later snapshot that
//! has none, and used with a carried title to decide `rank.queue` when a
//! later Tab re-queues the game.

use super::tests::{mark_current, snap};
use super::*;
use crate::model::QueueTitle;
use crate::sync::wire::upload_request;
use scuffed_types::api::RankQueue;
use serde_json::Value;

fn role_queue() -> QueueTitle {
    QueueTitle {
        value: Some("ROLE QUEUE".into()),
        confidence: 0.93,
        ..QueueTitle::default()
    }
}

fn sent_queue(m: &PersonalMatch) -> Value {
    let body = serde_json::to_value(upload_request(std::slice::from_ref(m), &[])).unwrap();
    body["matches"][0]["rank"]["queue"].clone()
}

/// A later snapshot that did not read a size takes the one already stored
/// on the game (`carry_game_fields`).
#[tokio::test]
async fn a_later_snapshot_without_a_size_takes_the_stored_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let mut first = snap("g1", 1);
    first.team_size = Some(6);
    store.insert_match(first).await.unwrap();
    store.insert_match(snap("g1", 2)).await.unwrap();
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|r| r.team_size == Some(6)),
        "{:?}",
        rows.iter().map(|r| r.team_size).collect::<Vec<_>>()
    );
}

/// A game stored with a role-queue title and no size, already synced, then
/// a later Tab that has the size and no title: the new row carries the
/// title, is unsynced, and uploads as `role_5v5`.
#[tokio::test]
async fn a_later_tab_requeues_the_game_with_the_decided_queue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let mut first = snap("g1", 1);
    first.queue_title = Some(role_queue());
    store.insert_match(first).await.unwrap();
    mark_current(&store).await;
    assert!(store.get_unsynced().await.unwrap().is_empty());

    let mut later = snap("g1", 2);
    later.team_size = Some(5);
    store.insert_match(later).await.unwrap();
    let queued = store.get_unsynced().await.unwrap();
    assert_eq!(queued.len(), 1, "the later Tab re-queues the game");
    assert_eq!(queued[0].queue_title, Some(role_queue()));
    assert_eq!(queued[0].team_size, Some(5));
    assert_eq!(sent_queue(&queued[0])["value"], "role_5v5");
    let decided = crate::rank_queue::decide_rank_queue(
        queued[0]
            .queue_title
            .as_ref()
            .and_then(|t| t.confident_value()),
        Some(0.93),
        queued[0].team_size,
        None,
        None,
    );
    assert_eq!(decided.value, RankQueue::Role5v5);
}
