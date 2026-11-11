//! A game's real start and end: stored on every snapshot of the match,
//! carried to later snapshots, cleared when the daemon clears the end, left
//! out of the upload queue, and absent on older rows.

use super::tests::snap;
use super::*;
use crate::model::MatchTimes;
use surrealdb_types::Datetime as SurrealDatetime;

fn at(rfc3339: &str) -> SurrealDatetime {
    let t: chrono::DateTime<chrono::Utc> = rfc3339.parse().unwrap();
    SurrealDatetime::from(t)
}

fn both() -> MatchTimes {
    MatchTimes {
        started_at: Some(at("2026-11-10T20:00:00Z")),
        ended_at: Some(at("2026-11-10T20:12:34Z")),
    }
}

#[tokio::test]
async fn times_go_on_every_snapshot_and_a_later_one_carries_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    store.insert_match(snap("g1", 2)).await.unwrap();
    assert!(store.attach_match_times("g1", &both()).await.unwrap());
    assert!(!store.attach_match_times("g1", &both()).await.unwrap());
    assert!(!store.attach_match_times("nope", &both()).await.unwrap());
    let g1 = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(g1.len(), 2);
    assert!(g1.iter().all(|r| r.match_times == Some(both())));
    // A later snapshot of g1 carries them; another game does not take them.
    store.insert_match(snap("g2", 1)).await.unwrap();
    store.insert_match(snap("g1", 3)).await.unwrap();
    for row in store.get_all_matches().await.unwrap() {
        let want = (row.session_id == "g1").then(both);
        assert_eq!(row.match_times, want, "game {}", row.session_id);
    }
}

#[tokio::test]
async fn a_start_with_no_end_is_stored_and_a_cleared_end_replaces_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    let start_only = MatchTimes {
        ended_at: None,
        ..both()
    };
    assert!(store.attach_match_times("g1", &start_only).await.unwrap());
    assert!(store.attach_match_times("g1", &both()).await.unwrap());
    // The board showed the result was wrong: the end goes again.
    assert!(store.attach_match_times("g1", &start_only).await.unwrap());
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows[0].match_times, Some(start_only));
}

#[tokio::test]
async fn attaching_times_leaves_a_synced_game_synced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    let rows = store.get_unsynced().await.unwrap();
    store.mark_synced(&SyncClaim::capture(&rows)).await.unwrap();
    assert!(store.attach_match_times("g1", &both()).await.unwrap());
    assert!(
        store.get_unsynced().await.unwrap().is_empty(),
        "the times are local only"
    );
}

#[tokio::test]
async fn a_row_from_before_the_field_reads_as_none_and_takes_times() {
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
    assert_eq!(rows[0].match_times, None);
    assert!(store.attach_match_times("legacy", &both()).await.unwrap());
    let rows = store.get_session_snapshots("legacy").await.unwrap();
    assert_eq!(rows[0].match_times, Some(both()));
}

#[test]
fn none_is_not_written_to_json_and_an_old_line_still_reads() {
    let row = serde_json::to_value(snap("g1", 1)).unwrap();
    assert!(row.get("match_times").is_none(), "omitted when None");
    let back: PersonalMatch = serde_json::from_value(row).unwrap();
    assert_eq!(back.match_times, None);
    let v = serde_json::to_value(MatchTimes::default()).unwrap();
    assert_eq!(v, serde_json::json!({}));
}
