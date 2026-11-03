//! The bans and picks result on a game: stored on every snapshot of the
//! match, carried to later snapshots, kept local.

use super::tests::snap;
use super::*;

const JSON: &str = r#"{"bans_picks_recognizer":"cv-v1"}"#;

#[tokio::test]
async fn the_result_goes_on_every_snapshot_of_the_game_and_only_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    for i in 0..2 {
        store.insert_match(snap("g1", i)).await.unwrap();
    }
    store.insert_match(snap("g2", 0)).await.unwrap();

    assert!(store.attach_bans_picks("g1", JSON).await.unwrap());
    let rows = store.get_all_matches().await.unwrap();
    for row in &rows {
        let want = (row.session_id == "g1").then(|| JSON.to_string());
        assert_eq!(row.bans_picks, want, "game {}", row.session_id);
    }
    assert!(
        !store.attach_bans_picks("g1", "{}").await.unwrap(),
        "a game that has a result keeps it"
    );
    assert!(!store.attach_bans_picks("nope", JSON).await.unwrap());
}

#[tokio::test]
async fn a_later_snapshot_of_the_game_carries_the_result() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    store.attach_bans_picks("g1", JSON).await.unwrap();
    store.insert_match(snap("g1", 2)).await.unwrap();
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.bans_picks.as_deref() == Some(JSON)));
}

#[tokio::test]
async fn the_game_start_is_its_first_snapshot_and_unknown_for_a_stranger() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    assert_eq!(store.session_started("g1").await.unwrap(), None);
    store.insert_match(snap("g1", 1)).await.unwrap();
    let started = store.session_started("g1").await.unwrap().expect("known");
    assert!((chrono::Utc::now() - started).num_seconds().abs() < 60);
}

#[tokio::test]
async fn a_result_with_no_value_in_it_is_not_in_the_upload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let mut m = snap("g1", 1);
    m.hero = "Ana".into();
    m.map_name = "Busan".into();
    m.game_mode = "Control".into();
    m.bans_picks = Some(JSON.to_string());
    store.insert_match(m).await.unwrap();
    let rows = store.get_all_matches().await.unwrap();
    let request = crate::sync::upload_request(&rows, &[]);
    assert_eq!(request.matches.len(), 1, "the row is uploaded");
    let wire = serde_json::to_string(&request).expect("request as json");
    assert!(!wire.contains("cv-v1"), "nothing read, so no key: {wire}");
}
