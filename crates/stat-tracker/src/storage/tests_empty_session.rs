//! Tests of the session-keyed writers given an empty session id. Imported
//! games carry no session id, so `WHERE session_id = $sid` with an empty id
//! matched every one of them: one edit of one game changed them all.

use super::tests::{mark_current, snap};
use super::*;

/// Two imported games with no session id, both already synced.
async fn two_imported_games(dir: &std::path::Path) -> LocalStore {
    let store = LocalStore::open(dir).await.expect("open store");
    let mut first = snap("", 10);
    first.hero = "Ana".into();
    first.map_name = "Ilios".into();
    let mut second = snap("", 4);
    second.hero = "Reinhardt".into();
    second.map_name = "Busan".into();
    store.insert_match(first).await.unwrap();
    store.insert_match(second).await.unwrap();
    mark_current(&store).await;
    store
}

/// How many rows no longer look as inserted by [`two_imported_games`].
async fn changed_rows(store: &LocalStore) -> usize {
    let rows = store.get_all_matches().await.unwrap();
    let missing = 2usize.saturating_sub(rows.len());
    missing
        + rows
            .iter()
            .filter(|m| {
                let (hero, map) = if m.elims == 10 {
                    ("Ana", "Ilios")
                } else {
                    ("Reinhardt", "Busan")
                };
                m.is_edited()
                    || !m.synced
                    || m.display_hero() != hero
                    || m.map_name != map
                    || m.display_elims() != m.elims
                    || m.outcome != "victory"
            })
            .count()
}

fn an_edit() -> MatchEdit {
    MatchEdit {
        elims: Some(99),
        hero: Some("Mercy".into()),
        ..Default::default()
    }
}

/// The reported bug: editing one imported game rewrote every imported game.
#[tokio::test]
async fn an_edit_with_an_empty_session_id_changes_no_game() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = two_imported_games(dir.path()).await;

    let result = store.edit_match("", &an_edit()).await;

    let changed = changed_rows(&store).await;
    assert_eq!(
        changed, 0,
        "an edit with an empty session id changed {changed} imported games"
    );
    assert!(result.is_err(), "an empty session id must be refused");
}

/// The GUI queues the edit as a command file; the daemon applies it here.
#[tokio::test]
async fn a_queued_edit_with_an_empty_session_id_changes_no_game() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = two_imported_games(dir.path()).await;

    let result = store
        .apply_command(&StoreCommand::EditMatch {
            session_id: String::new(),
            edit: an_edit(),
        })
        .await;

    let changed = changed_rows(&store).await;
    assert_eq!(
        changed, 0,
        "a queued edit with an empty session id changed {changed} imported games"
    );
    assert!(result.is_err(), "an empty session id must be refused");
}

/// Outcome, map, hero, hero swap and retry are keyed the same way.
#[tokio::test]
async fn the_other_session_writers_refuse_an_empty_session_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = two_imported_games(dir.path()).await;

    let results = [
        (
            "set_session_outcome",
            store.set_session_outcome("", "defeat").await,
        ),
        ("set_session_map", store.set_session_map("", "Nepal").await),
        (
            "set_session_map_keeping_member_choices",
            store
                .set_session_map_keeping_member_choices("", "Nepal")
                .await
                .map(|_| ()),
        ),
        (
            "set_session_hero",
            store.set_session_hero("", "Mercy", "Support").await,
        ),
        (
            "resolve_hero_segment",
            store.resolve_hero_segment("", 0, SEGMENT_DISMISS).await,
        ),
        (
            "refresh_session_hero_timeline",
            store.refresh_session_hero_timeline("").await,
        ),
        ("clear_upload_reject", store.clear_upload_reject("").await),
    ];

    let changed = changed_rows(&store).await;
    assert_eq!(
        changed, 0,
        "session writers with an empty id changed {changed} imported games"
    );
    for (name, result) in results {
        assert!(result.is_err(), "{name} must refuse an empty session id");
    }
}

/// A delete with an empty id removed every imported game and queued a
/// server delete for the empty id.
#[tokio::test]
async fn a_delete_with_an_empty_session_id_removes_no_game() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = two_imported_games(dir.path()).await;

    let result = store.delete_session("").await;

    assert_eq!(
        store.match_count().await.unwrap(),
        2,
        "a delete with an empty session id removed imported games"
    );
    assert!(
        store.get_pending_tombstones().await.unwrap().is_empty(),
        "an empty session id must not queue a server delete"
    );
    assert!(result.is_err(), "an empty session id must be refused");
}

/// An edit for a session id no row has changes no row, and does not spill
/// over onto the imported games either.
#[tokio::test]
async fn an_edit_of_a_game_that_is_gone_changes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = two_imported_games(dir.path()).await;

    let _ = store.edit_match("no-such-game", &an_edit()).await;

    assert_eq!(changed_rows(&store).await, 0);
}

/// The refusal is typed, so the daemon can drop the command file instead
/// of retrying it on every tick.
#[tokio::test]
async fn the_refusal_is_a_refused_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = two_imported_games(dir.path()).await;

    let err = store.edit_match("", &an_edit()).await.unwrap_err();
    assert!(err.downcast_ref::<Refused>().is_some(), "{err}");
}

/// The bans and picks writers are keyed the same way. The capture path
/// logs their error and records the game anyway.
#[tokio::test]
async fn the_bans_and_picks_writers_refuse_an_empty_session_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = two_imported_games(dir.path()).await;

    let result = store
        .attach_bans_picks("", r#"{"bans_picks_recognizer":"cv-v1"}"#)
        .await;
    let status = store
        .attach_bans_picks_status("", &crate::model::BansPicksStatus::ReaderOff)
        .await;

    let rows = store.get_all_matches().await.unwrap();
    let touched = rows
        .iter()
        .filter(|m| m.bans_picks.is_some() || m.bans_picks_status.is_some())
        .count();
    assert_eq!(
        touched, 0,
        "bans and picks writers with an empty id changed {touched} imported games"
    );
    for (name, result) in [
        ("attach_bans_picks", result.map(|_| ())),
        ("attach_bans_picks_status", status.map(|_| ())),
    ] {
        let err = result.expect_err(name);
        assert!(err.downcast_ref::<Refused>().is_some(), "{name}: {err}");
    }
}

/// Two session rows with an empty id: the hero migration renames one by
/// `WHERE session_id = $sid`, which would rename the other too. Empty ids
/// are skipped, so the other row keeps its own hero.
#[tokio::test]
async fn the_hero_migration_skips_session_rows_with_an_empty_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let mut pack_key = super::tests::session_row("", "victory");
    pack_key.hero = "wrecking-ball".into();
    pack_key.role = "Damage".into();
    let mut ana = super::tests::session_row("", "victory");
    ana.hero = "Ana".into();
    ana.role = "Support".into();
    for row in [pack_key, ana] {
        store
            .db
            .query("CREATE match_session CONTENT $s")
            .bind(("s", row))
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    store.migrate_pack_key_heroes().await.unwrap();

    let mut heroes: Vec<(String, String)> = store
        .get_all_sessions()
        .await
        .unwrap()
        .into_iter()
        .map(|s| (s.hero, s.role))
        .collect();
    heroes.sort();
    assert_eq!(
        heroes,
        vec![
            ("Ana".to_string(), "Support".to_string()),
            ("wrecking-ball".to_string(), "Damage".to_string()),
        ],
        "a session row with an empty id was renamed with another one"
    );
}
