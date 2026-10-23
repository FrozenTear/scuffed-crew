//! Tests of the capture write path and stored rows: decided outcomes, rows
//! landing after a delete, and fields that must survive a reopen.

use super::tests::{reopen_store, snap};
use super::*;
use chrono::Utc;

#[test]
fn old_rows_without_edit_fields_deserialize_unchanged() {
    // A pre-feature jsonl line has none of the corrected_*/edited_* keys.
    let line = r#"{"hero":"Genji","map_name":"Hanamura","game_mode":"assault","role":"Damage","outcome":"victory","elims":20,"deaths":4,"assists":6,"damage":8000,"healing":0,"mitigation":0,"played_at":"2026-01-01T00:00:00Z","synced":true,"session_id":"legacy1"}"#;
    let m: PersonalMatch = serde_json::from_str(line).expect("legacy row parses");
    assert!(!m.is_edited());
    assert!(m.edited_fields.is_empty());
    assert_eq!(m.edited_at, None);
    // Display falls back to the OCR value when no correction exists.
    assert_eq!(m.display_elims(), 20);
    assert_eq!(m.display_hero(), "Genji");
    assert_eq!(m.corrected_elims, None);
}

fn session_row(session_id: &str, outcome: &str) -> MatchSession {
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

async fn final_outcome(store: &LocalStore, session_id: &str) -> Option<String> {
    store
        .get_all_sessions()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.session_id == session_id)
        .map(|s| s.final_outcome)
}

/// MAIN-1: a capture requested before the result was known writes
/// `unknown`. A result decided while it ran stays on the session, and
/// the capture's row takes it.
#[tokio::test]
async fn an_in_flight_unknown_capture_keeps_a_decided_outcome() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store
        .create_session(&session_row("s1", "unknown"))
        .await
        .unwrap();
    let mut early = snap("s1", 3);
    early.outcome = "unknown".into();
    store.insert_match(early).await.unwrap();
    // The end-reel poll decides the game while the Tab capture runs.
    store.set_session_outcome("s1", "victory").await.unwrap();
    let now = SurrealDatetime::from(Utc::now());
    store.append_capture("s1", now, "unknown").await.unwrap();
    assert_eq!(
        final_outcome(&store, "s1").await.as_deref(),
        Some("victory")
    );
    let mut late = snap("s1", 9);
    late.outcome = "unknown".into();
    store.insert_match(late).await.unwrap();
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|row| row.outcome == "victory"),
        "{:?}",
        rows.iter()
            .map(|r| (r.elims, &r.outcome))
            .collect::<Vec<_>>()
    );
    // A decided capture still writes its result.
    store.append_capture("s1", now, "defeat").await.unwrap();
    assert_eq!(final_outcome(&store, "s1").await.as_deref(), Some("defeat"));
    // An undecided session leaves an unknown row unknown.
    store
        .create_session(&session_row("s2", "unknown"))
        .await
        .unwrap();
    let mut open = snap("s2", 1);
    open.outcome = "unknown".into();
    store.insert_match(open).await.unwrap();
    store.append_capture("s2", now, "unknown").await.unwrap();
    assert_eq!(
        final_outcome(&store, "s2").await.as_deref(),
        Some("unknown")
    );
    let rows = store.get_session_snapshots("s2").await.unwrap();
    assert_eq!(rows[0].outcome, "unknown");
}

/// MAIN-1 (3): a capture that was in flight when its game was deleted
/// does not bring the game back, neither the session row nor the
/// snapshot nor the `matches.jsonl` line.
#[tokio::test]
async fn a_capture_landing_after_a_delete_does_not_resurrect_the_game() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store
        .create_session(&session_row("gone", "unknown"))
        .await
        .unwrap();
    store.insert_match(snap("gone", 1)).await.unwrap();
    store.delete_session("gone").await.unwrap();
    // The in-flight capture's writes, in the order the capture runs them.
    store
        .create_session(&session_row("gone", "unknown"))
        .await
        .unwrap();
    store
        .append_capture("gone", SurrealDatetime::from(Utc::now()), "victory")
        .await
        .unwrap();
    append_match_log(dir.path(), &snap("gone", 2));
    store.insert_match(snap("gone", 2)).await.unwrap();
    assert_eq!(store.match_count().await.unwrap(), 0);
    assert_eq!(final_outcome(&store, "gone").await, None);
    assert!(read_match_log(dir.path()).is_empty());
    assert_eq!(
        store.get_pending_tombstones().await.unwrap(),
        vec!["gone".to_string()],
        "the tombstone still goes to the server"
    );
    // Other games are stored as before.
    store.insert_match(snap("kept", 1)).await.unwrap();
    assert_eq!(store.match_count().await.unwrap(), 1);
}

/// A pending tombstone is not a delete. A game whose server copy goes
/// (re-read as an untracked map or mode) keeps its local rows and can be
/// the open game: its new rows are stored and its `matches.jsonl` lines
/// stay. Only a delete in this process blocks a late capture.
#[tokio::test]
async fn a_pending_tombstone_without_a_delete_does_not_block_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store
        .create_session(&session_row("dm", "unknown"))
        .await
        .unwrap();
    append_match_log(dir.path(), &snap("dm", 1));
    store.insert_match(snap("dm", 1)).await.unwrap();
    // Stands in for a server-side removal queued for a game kept here.
    store.insert_tombstone("dm").await.unwrap();
    append_match_log(dir.path(), &snap("dm", 2));
    store.insert_match(snap("dm", 2)).await.unwrap();
    store
        .create_session(&session_row("dm", "victory"))
        .await
        .unwrap();
    assert_eq!(store.get_session_snapshots("dm").await.unwrap().len(), 2);
    assert_eq!(
        final_outcome(&store, "dm").await.as_deref(),
        Some("victory")
    );
    assert_eq!(read_match_log(dir.path()).len(), 2);
}

/// The fallback log follows a row that took the session's result.
#[tokio::test]
async fn an_adopted_outcome_is_written_to_the_match_log_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    store
        .create_session(&session_row("s1", "defeat"))
        .await
        .unwrap();
    let mut late = snap("s1", 4);
    late.outcome = "unknown".into();
    append_match_log(dir.path(), &late);
    store.insert_match(late).await.unwrap();
    let log = read_match_log(dir.path());
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, "defeat");
}

#[tokio::test]
async fn recognizer_and_suspects_survive_reopen_and_an_edit_clears_one_field() {
    let dir = tempfile::tempdir().expect("tempdir");
    let recognizer = crate::model::RECOGNIZER_ID.to_string();
    {
        let store = LocalStore::open(dir.path()).await.unwrap();
        let mut row = snap("kept", 4);
        row.map_name = "Busan".into();
        row.game_mode = "Control".into();
        row.recognizer = recognizer.clone();
        row.suspect_fields = vec!["r3.dmg".into(), "hero".into(), "e".into()];
        store.insert_match(row).await.unwrap();
    }
    let store = reopen_store(dir.path()).await;
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows[0].recognizer, recognizer);
    assert_eq!(
        rows[0].suspect_fields,
        vec!["r3.dmg".to_string(), "hero".into(), "e".into()]
    );
    let body = crate::sync::upload_request(&rows, &[]);
    let json = serde_json::to_string(&body).unwrap();
    assert!(
        !json.contains("r3.dmg"),
        "an upload payload must not contain a dotted name: {json}"
    );
    assert!(json.contains(&format!("\"recognizer\":\"{recognizer}\"")));
    assert!(
        body.matches[0].name_recognizer.is_none(),
        "this row stored no name recognizer"
    );
    assert_eq!(
        body.matches[0].suspect_fields,
        vec!["hero".to_string(), "e".into(), "dmg".into()]
    );
    store
        .edit_match(
            "kept",
            &MatchEdit {
                damage: Some(10),
                ..MatchEdit::default()
            },
        )
        .await
        .unwrap();
    let edited = store.get_all_matches().await.unwrap();
    assert_eq!(edited[0].recognizer, recognizer);
    assert_eq!(
        edited[0].suspect_fields,
        vec!["hero".to_string(), "e".into()]
    );
    let again = crate::sync::upload_request(&edited, &[]);
    let again_json = serde_json::to_string(&again).unwrap();
    assert!(!again_json.contains("r3.dmg"), "{again_json}");
    assert!(!again_json.contains("\"dmg\""));
}

/// The recognizer that found the player's row is stored on the match.
/// A restart and a later snapshot that did not read a name both keep it,
/// and a resend still sends it. A missing value stays omitted.
#[tokio::test]
async fn name_recognizer_survives_restart_and_is_resent() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = LocalStore::open(dir.path()).await.unwrap();
        let mut row = snap("kept", 4);
        row.hero = "Ana".into();
        row.map_name = "Busan".into();
        row.game_mode = "Control".into();
        row.name_recognizer = Some("names-v1".into());
        row.played_at = SurrealDatetime::from(Utc::now() - chrono::Duration::seconds(30));
        store.insert_match(row).await.unwrap();
    }
    let store = reopen_store(dir.path()).await;
    let opened = store.get_all_matches().await.unwrap();
    assert_eq!(opened[0].name_recognizer.as_deref(), Some("names-v1"));
    let first = crate::sync::upload_request(&opened, &[]);
    assert_eq!(
        first.matches[0].name_recognizer.as_deref(),
        Some("names-v1")
    );

    let mut later = snap("kept", 11);
    later.hero = "Ana".into();
    later.map_name = "Busan".into();
    later.game_mode = "Control".into();
    later.name_recognizer = None;
    later.played_at = SurrealDatetime::from(Utc::now());
    store.insert_match(later).await.unwrap();
    let stored = store.get_session_snapshots("kept").await.unwrap();
    assert!(
        stored
            .iter()
            .all(|row| row.name_recognizer.as_deref() == Some("names-v1")),
        "a snapshot with no name read keeps the match recognizer: {stored:?}"
    );
    drop(store);

    let store = reopen_store(dir.path()).await;
    let mut newest_first = store.get_unsynced().await.unwrap();
    newest_first.reverse();
    let games = latest_per_game(newest_first);
    let body = crate::sync::upload_request(&games, &[]);
    assert_eq!(body.matches.len(), 1);
    assert_eq!(body.matches[0].name_recognizer.as_deref(), Some("names-v1"));
    let raw = serde_json::to_string(&body).unwrap();
    assert!(raw.contains("\"name_recognizer\":\"names-v1\""), "{raw}");

    store
        .mark_synced(&SyncClaim::capture(&store.get_all_matches().await.unwrap()))
        .await
        .unwrap();
    store.set_session_map("kept", "Ilios").await.unwrap();
    let mut resent = store.get_unsynced().await.unwrap();
    assert!(!resent.is_empty(), "the map change queues a resend");
    resent.reverse();
    let again = crate::sync::upload_request(&latest_per_game(resent), &[]);
    assert_eq!(again.matches[0].map_name, "Ilios");
    assert_eq!(
        again.matches[0].name_recognizer.as_deref(),
        Some("names-v1"),
        "a resend still sends the stored recognizer"
    );
}
