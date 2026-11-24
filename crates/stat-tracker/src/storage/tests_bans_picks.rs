//! The bans and picks result on a game: stored on every snapshot of the
//! match, carried to later snapshots, kept local.

use super::tests::snap;
use super::*;
use crate::model::{BansPicksMissing, BansPicksStatus};

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

fn every_status() -> Vec<BansPicksStatus> {
    vec![
        BansPicksStatus::Ok,
        BansPicksStatus::Partial {
            missing: vec![BansPicksMissing::BanVote, BansPicksMissing::HeroSelect],
        },
        BansPicksStatus::NoFrames,
        BansPicksStatus::FrameSize {
            width: 1920,
            height: 1080,
        },
        BansPicksStatus::PackMissing { linked: false },
        BansPicksStatus::PackFailed,
        BansPicksStatus::ReaderFailed,
        BansPicksStatus::ReaderOff,
    ]
}

#[test]
fn the_status_is_snake_case_json_and_an_old_row_has_none() {
    let json = |s: &BansPicksStatus| serde_json::to_string(s).unwrap();
    assert_eq!(json(&BansPicksStatus::NoFrames), r#""no_frames""#);
    assert_eq!(
        json(&BansPicksStatus::Partial {
            missing: vec![BansPicksMissing::HeroSelect]
        }),
        r#"{"partial":{"missing":["hero_select"]}}"#
    );
    assert_eq!(
        json(&BansPicksStatus::FrameSize {
            width: 1,
            height: 2
        }),
        r#"{"frame_size":{"width":1,"height":2}}"#
    );
    assert_eq!(
        json(&BansPicksStatus::PackMissing { linked: true }),
        r#"{"pack_missing":{"linked":true}}"#
    );
    // A match log line from before the field existed.
    let mut old = serde_json::to_value(snap("g1", 1)).unwrap();
    assert!(old.get("bans_picks_status").is_none(), "omitted when None");
    old.as_object_mut().unwrap().remove("bans_picks_status");
    let row: PersonalMatch = serde_json::from_value(old).unwrap();
    assert_eq!(row.bans_picks_status, None);
}

#[tokio::test]
async fn every_status_survives_the_store_and_a_later_snapshot_carries_it() {
    for (i, status) in every_status().into_iter().enumerate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(dir.path()).await.expect("open store");
        let sid = format!("g{i}");
        store.insert_match(snap(&sid, 1)).await.unwrap();
        assert!(store.attach_bans_picks_status(&sid, &status).await.unwrap());
        assert!(
            !store.attach_bans_picks_status(&sid, &status).await.unwrap(),
            "the same status is not written twice"
        );
        store.insert_match(snap(&sid, 2)).await.unwrap();
        let rows = store.get_all_matches().await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r.bans_picks_status.as_ref() == Some(&status)),
            "{status:?}"
        );
    }
}

#[tokio::test]
async fn a_later_status_replaces_no_frames_but_a_stored_result_keeps_its_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store.insert_match(snap("g1", 1)).await.unwrap();
    assert!(
        !store
            .attach_bans_picks_status("nope", &BansPicksStatus::Ok)
            .await
            .unwrap()
    );
    store
        .attach_bans_picks_status("g1", &BansPicksStatus::NoFrames)
        .await
        .unwrap();
    assert!(
        store
            .attach_bans_picks_status("g1", &BansPicksStatus::Ok)
            .await
            .unwrap()
    );
    store.attach_bans_picks("g1", JSON).await.unwrap();
    assert!(
        !store
            .attach_bans_picks_status("g1", &BansPicksStatus::ReaderOff)
            .await
            .unwrap()
    );
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows[0].bans_picks_status, Some(BansPicksStatus::Ok));
}
