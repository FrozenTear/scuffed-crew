//! Tests for the store command queue (`commands`).

use super::*;
use crate::model::{EditForm, Game, GameOcr, Outcome, Role};
use chrono::Utc;

/// A finished game with a website linked: the existing cases.
fn save(dir: &Path, game: &Game, form: &EditForm) -> Result<Saved, String> {
    save_edit(dir, game, form, false, true)
}

fn toast_saved(
    held: bool,
    stats_changed: bool,
    map_changed: bool,
    hero_fixed: bool,
    role_changed: bool,
) -> Saved {
    saved(
        held,
        stats_changed,
        map_changed,
        hero_fixed,
        role_changed,
        None,
        None,
    )
}

fn tmp() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sst-ui-cmd-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample_game() -> Game {
    Game {
        session_id: "sess-t1".into(),
        hero: "Ana".into(),
        map_name: "Ilios".into(),
        role: Role::Support,
        outcome: Outcome::Win,
        elims: 14,
        deaths: 5,
        assists: 16,
        damage: 4200,
        healing: 11200,
        mitigation: 0,
        played_at: Utc::now(),
        edited: false,
        edited_fields: Vec::new(),
        ocr: GameOcr::default(),
        segments: Vec::new(),
        upload_reject: None,
        suspect_fields: Vec::new(),
        synced: false,
        recognizer: String::new(),
        bans_picks: None,
        bans_picks_status: None,
        queue_title: None,
        rank_update: None,
        rank_slot: None,
        team_size: None,
        match_times: None,
    }
}

fn held_game() -> Game {
    let mut g = sample_game();
    g.map_name = String::new();
    g
}

fn queued_edit(dir: &Path) -> MatchEdit {
    match queued(dir).pop() {
        Some(StoreCommand::EditMatch { edit, .. }) => edit,
        other => panic!("{other:?}"),
    }
}

/// A hand-typed map is stored as the reader and the site name it, and
/// a known map releases a game held back for a missing one.
#[test]
fn a_typed_map_is_canonical_and_releases_the_hold() {
    let dir = tmp();
    let g = held_game();
    assert!(g.needs_review());
    let mut form = EditForm::from_game(&g);
    form.map_name = "kings row".into();
    // No stat changed, so no "Stats corrected", and the report is for
    // the map.
    let saved = save(&dir, &g, &form).unwrap();
    assert_eq!(saved.toast, RELEASED);
    assert_eq!(saved.category, Some(GameCategory::WrongMap));
    let edit = queued_edit(&dir);
    assert_eq!(edit.map_name.as_deref(), Some("King's Row"));
    let mut after = g.clone();
    after.map_name = edit.map_name.unwrap();
    assert!(!after.needs_review());
}

/// Setting the missing map or hero on an ended game with no result
/// does not say nothing is missing: the game still waits for its
/// result, as the picker says.
#[test]
fn a_release_with_no_result_says_the_result_is_still_needed() {
    let sets: [&dyn Fn(&mut EditForm); 2] = [&|f| f.map_name = "Ilios".into(), &|f| {
        f.hero = "Mercy".into()
    }];
    for set in sets {
        let dir = tmp();
        let mut g = sample_game();
        g.outcome = Outcome::Unknown;
        g.map_name.clear();
        g.hero.clear();
        let mut form = EditForm::from_game(&g);
        form.map_name = "Ilios".into();
        form.hero = "Mercy".into();
        let saved = save(&dir, &g, &form).unwrap();
        assert_eq!(saved.toast, RELEASED_NO_RESULT);
        assert_eq!(saved.toast, "Saved. It uploads once you set the result.");
        // One field only: still held.
        let mut form = EditForm::from_game(&g);
        set(&mut form);
        let saved = save(&tmp(), &g, &form).unwrap();
        assert!(saved.toast.contains("stays on this computer"));
    }
    let dir = tmp();
    let mut g = held_game();
    g.outcome = Outcome::Unknown;
    let mut form = EditForm::from_game(&g);
    form.map_name = "Ilios".into();
    assert_eq!(save(&dir, &g, &form).unwrap().toast, RELEASED_NO_RESULT);
    g.outcome = Outcome::Loss;
    assert_eq!(save(&tmp(), &g, &form).unwrap().toast, RELEASED);
}

/// Releasing a held live game, or one with sync off, uses the picker's
/// upload line, not the no-result one.
#[test]
fn a_release_on_a_live_or_sync_off_game_does_not_promise_a_result_upload() {
    let mut g = held_game();
    g.outcome = Outcome::Unknown;
    let mut form = EditForm::from_game(&g);
    form.map_name = "Ilios".into();
    assert_eq!(
        save_edit(&tmp(), &g, &form, true, true).unwrap().toast,
        RELEASED_LIVE
    );
    assert_eq!(
        save_edit(&tmp(), &g, &form, true, true).unwrap().toast,
        "Saved. It uploads once it ends."
    );
    assert_eq!(
        save_edit(&tmp(), &g, &form, false, false).unwrap().toast,
        RELEASED_SYNC_OFF
    );
    assert_eq!(
        save_edit(&tmp(), &g, &form, true, false).unwrap().toast,
        RELEASED_SYNC_OFF
    );
    g.outcome = Outcome::Win;
    assert_eq!(
        save_edit(&tmp(), &g, &form, false, false).unwrap().toast,
        RELEASED_SYNC_OFF
    );
}

/// Under `--fixture games` the panel reads as with a website linked, so
/// releasing the held game from the Edit form toasts what the picker
/// line says, never "no website is linked". The app's SaveEdit and the
/// panel read one value ([`crate::games_panel::sync_on`]).
#[test]
fn an_edit_release_under_the_games_fixture_matches_the_picker() {
    use crate::app::Message;
    use crate::fixtures_games::HELD;
    use crate::games_picker::{Field, release_line};
    let dir = tempfile::tempdir().unwrap();
    let mut cli = crate::fixtures::test_cli(dir.path());
    cli.fixture = Some(crate::cli::FixtureKind::Games);
    let mut app = crate::app::TrackerApp::new(cli).0;
    assert!(!app.sync_on(), "the fixture links no website");
    assert!(crate::games_panel::sync_on(&app));
    let g = app
        .games
        .iter()
        .find(|g| g.session_id == HELD)
        .unwrap()
        .clone();
    // The held game misses both its map and its hero, so the picker
    // line reaches the sync check only on the last one: the hero, once
    // the map is set.
    let mut map_set = g.clone();
    map_set.map_name = "Ilios".into();
    let picker = release_line(
        &map_set,
        Field::Hero,
        Some("Ana"),
        false,
        crate::games_panel::sync_on(&app),
    );
    assert!(picker.starts_with("Saving releases this game"), "{picker}");
    assert!(!picker.contains("no website is linked"), "{picker}");
    app.expanded = Some(HELD.into());
    let _ = app.update(Message::ToggleEdit);
    assert!(app.editing);
    app.edit.map_name = "Ilios".into();
    app.edit.hero = "Ana".into();
    let _ = app.update(Message::SaveEdit);
    assert_eq!(app.toast.as_deref(), Some(RELEASED));
    assert_ne!(app.toast.as_deref(), Some(RELEASED_SYNC_OFF));
}

/// Releasing a hold with a stat fixed too still says the stats were
/// corrected.
#[test]
fn a_released_hold_with_a_stat_fix_says_stats_corrected() {
    let dir = tmp();
    let g = held_game();
    let mut form = EditForm::from_game(&g);
    form.map_name = "Ilios".into();
    form.elims = "15".into();
    let saved = save(&dir, &g, &form).unwrap();
    assert_eq!(saved.toast, "Stats corrected");
    assert_eq!(saved.category, Some(GameCategory::WrongStats));
}

/// A hero-only Edit that releases a hold files its report as Wrong
/// hero.
#[test]
fn a_hero_release_reports_a_wrong_hero() {
    let dir = tmp();
    let mut g = sample_game();
    g.hero = String::new();
    assert!(g.needs_review());
    let mut form = EditForm::from_game(&g);
    form.hero = "Mercy".into();
    let saved = save(&dir, &g, &form).unwrap();
    assert_eq!(saved.toast, RELEASED);
    assert_eq!(saved.category, Some(GameCategory::WrongHero));
}

/// A map-only or hero-only Edit on a game that wasn't held names what
/// it fixed, never "Stats corrected", and reports that field. A
/// role-only Edit offers no report, as only the role changed.
#[test]
fn a_map_or_hero_edit_names_what_it_fixed() {
    let g = sample_game();
    assert!(!g.needs_review());
    let save = |set: &dyn Fn(&mut EditForm)| {
        let mut form = EditForm::from_game(&g);
        set(&mut form);
        save(&tmp(), &g, &form).unwrap()
    };
    let map = save(&|f| f.map_name = "Havana".into());
    assert_eq!(map.toast, "Map corrected");
    assert_eq!(map.category, Some(GameCategory::WrongMap));
    let hero = save(&|f| f.hero = "Mercy".into());
    assert_eq!(hero.toast, "Hero corrected");
    assert_eq!(hero.category, Some(GameCategory::WrongHero));
    let both = save(&|f| {
        f.map_name = "Havana".into();
        f.hero = "Mercy".into();
    });
    assert_eq!(both.toast, "Map and hero corrected");
    assert_eq!(both.category, Some(GameCategory::WrongMap));
    let role = save(&|f| f.role = "Tank".into());
    assert_eq!(role.toast, "Role corrected");
    assert_eq!(role.category, None, "a role fix is not a hero misread");
    let map_role = save(&|f| {
        f.map_name = "Havana".into();
        f.role = "Tank".into();
    });
    assert_eq!(map_role.toast, "Map and role corrected");
    assert_eq!(map_role.category, Some(GameCategory::WrongMap));
    let stats = save(&|f| f.elims = "40".into());
    assert_eq!(stats.toast, "Stats corrected");
    assert_eq!(stats.category, Some(GameCategory::WrongStats));
}

/// A hero and a role fixed together are a hero fix: "Hero corrected",
/// filed as Wrong hero.
#[test]
fn a_hero_and_role_edit_reports_a_wrong_hero() {
    let dir = tmp();
    let g = sample_game();
    let mut form = EditForm::from_game(&g);
    form.hero = "Reinhardt".into();
    form.role = "Tank".into();
    let saved = save(&dir, &g, &form).unwrap();
    assert_eq!(saved.toast, "Hero corrected");
    assert_eq!(saved.category, Some(GameCategory::WrongHero));
    let edit = queued_edit(&dir);
    assert_eq!(edit.hero.as_deref(), Some("Reinhardt"));
    assert!(edit.role.is_some());
}

/// Typing the same hero in another case ("ana" for Ana) is no hero
/// change, so with a role change it is still role-only: no report.
#[test]
fn a_retyped_same_hero_with_a_role_change_is_role_only() {
    let dir = tmp();
    let g = sample_game();
    let mut form = EditForm::from_game(&g);
    form.hero = "ana".into();
    form.role = "Tank".into();
    let saved = save(&dir, &g, &form).unwrap();
    assert_eq!(saved.toast, "Role corrected");
    assert_eq!(saved.category, None);
    let edit = queued_edit(&dir);
    assert_eq!(edit.hero, None, "the hero did not change");
    assert!(edit.role.is_some());
}

/// In the app, a role-only save shows "Role corrected" with no toast
/// action, so no training report is offered.
#[test]
fn a_role_only_save_in_the_app_offers_no_report() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let g = sample_game();
    app.games = vec![g.clone()];
    app.expanded = Some(g.session_id.clone());
    let _ = app.update(crate::app::Message::ToggleEdit);
    app.edit.role = "Tank".into();
    let _ = app.update(crate::app::Message::SaveEdit);
    assert_eq!(app.toast.as_deref(), Some("Role corrected"));
    assert!(app.toast_action.is_none(), "{:?}", app.toast_action);
}

/// A role-only save on a game held for a missing map still says the
/// game stays local, and offers no report.
#[test]
fn a_role_only_edit_on_a_held_game_offers_no_report() {
    let dir = tmp();
    let g = held_game();
    let mut form = EditForm::from_game(&g);
    form.role = "Tank".into();
    let saved = save(&dir, &g, &form).unwrap();
    assert!(
        saved.toast.contains("stays on this computer"),
        "{}",
        saved.toast
    );
    assert_eq!(saved.category, None);
    assert!(!offers_report(saved));
}

#[test]
fn an_unknown_map_is_refused_and_nothing_is_queued() {
    let dir = tmp();
    let g = held_game();
    let mut form = EditForm::from_game(&g);
    form.map_name = "Not A Real Place".into();
    let err = save(&dir, &g, &form).unwrap_err();
    assert_eq!(
        err,
        "Map not found: Not A Real Place. Type a map name like King's Row"
    );
    assert!(queued(&dir).is_empty());
}

#[test]
fn a_hero_matches_the_list_in_any_case() {
    let dir = tmp();
    let g = sample_game();
    let mut form = EditForm::from_game(&g);
    form.hero = "kiriko".into();
    save(&dir, &g, &form).unwrap();
    assert_eq!(queued_edit(&dir).hero.as_deref(), Some("Kiriko"));
    form.hero = "Nobody".into();
    assert!(
        save(&dir, &g, &form)
            .unwrap_err()
            .starts_with("Hero not found")
    );
}

/// A refused map keeps the Edit form open with what was typed, so the
/// stat fixes typed with it are not lost; a good save closes it.
#[test]
fn a_refused_edit_keeps_the_form_and_what_was_typed() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let mut g = sample_game();
    g.session_id = "sess-edit".into();
    app.games = vec![g.clone()];
    app.expanded = Some(g.session_id.clone());
    let _ = app.update(crate::app::Message::ToggleEdit);
    assert!(app.editing);
    app.edit.map_name = "Not A Real Place".into();
    app.edit.elims = "21".into();
    let _ = app.update(crate::app::Message::SaveEdit);
    assert!(app.editing, "the form stays open on a refused map");
    assert_eq!(app.edit.map_name, "Not A Real Place");
    assert_eq!(app.edit.elims, "21");
    app.edit.map_name = "Ilios".into();
    let _ = app.update(crate::app::Message::SaveEdit);
    assert!(!app.editing, "a good save closes the form");
}

/// Whether `saved` shown in an app offers a training report.
fn offers_report(saved: Saved) -> bool {
    let (_dir, mut app) = crate::fixtures::test_app();
    let g = sample_game();
    app.games = vec![g.clone()];
    show_saved(&mut app, &g.session_id, &saved);
    assert_eq!(app.toast.as_deref(), Some(saved.toast.as_str()));
    matches!(
        app.toast_action,
        Some(crate::app::Message::Report(
            crate::report::ReportMessage::OpenGame { training: true, .. }
        ))
    )
}

/// A save that changed nothing says "Saved" and offers no training
/// report; a fix still offers one. (The form never sends an empty
/// edit, "No changes to save", which offers none either.)
#[test]
fn an_unchanged_save_offers_no_report() {
    let unchanged = toast_saved(false, false, false, false, false);
    assert_eq!(unchanged.toast, "Saved");
    assert_eq!(unchanged.category, None);
    assert!(!offers_report(unchanged));
    assert!(offers_report(toast_saved(false, false, true, false, false)));
    let app_save = |set: &dyn Fn(&mut EditForm)| {
        let (_dir, mut app) = crate::fixtures::test_app();
        let g = sample_game();
        app.games = vec![g.clone()];
        app.expanded = Some(g.session_id.clone());
        let _ = app.update(crate::app::Message::ToggleEdit);
        set(&mut app.edit);
        let _ = app.update(crate::app::Message::SaveEdit);
        (app.toast.clone(), app.toast_action.is_some())
    };
    assert_eq!(
        app_save(&|_| {}),
        (Some("No changes to save".to_string()), false)
    );
    assert_eq!(
        app_save(&|f| f.elims = "40".into()),
        (Some("Stats corrected".to_string()), true)
    );
}

/// The same on a game held for a missing map or hero: a save that
/// changed nothing offers no report; a stat fix still offers one.
#[test]
fn an_unchanged_save_on_a_held_game_offers_no_report() {
    let held = "Saved. A map or hero is still missing, so this game stays on this computer.";
    let unchanged = toast_saved(true, false, false, false, false);
    assert_eq!(unchanged.toast, held);
    assert_eq!(unchanged.category, None);
    assert!(!offers_report(unchanged));
    let fixed = toast_saved(true, true, false, false, false);
    assert_eq!(fixed.category, Some(GameCategory::WrongStats));
    assert!(offers_report(fixed));
}

/// The reader's own spellings find the hero, stored as the reader
/// stores it, so the Heroes tab never splits one hero in two.
#[test]
fn a_typed_hero_folds_case_punctuation_and_accents() {
    for (typed, want) in [
        ("soldier 76", "Soldier: 76"),
        ("dva", "D.Va"),
        ("Lúcio", "Lúcio"),
        ("lucio", "Lúcio"),
        ("Torbjorn", "Torbjörn"),
        ("wrecking-ball", "Wrecking Ball"),
    ] {
        let stored = hero_from_typed(typed);
        assert_eq!(stored, Some(want), "{typed}");
        assert_eq!(stat_tracker::parse::canonical_hero(want), want, "{want}");
    }
    assert_eq!(hero_from_typed("Nobody"), None);
}

/// A picked hero is stored by the reader's name, whichever spelling
/// the list gave.
#[test]
fn a_picked_hero_is_stored_as_the_reader_stores_it() {
    for picked in ["Lucio", "Lúcio"] {
        let dir = tmp();
        pick_hero(&dir, "sess-h", picked).unwrap();
        assert_eq!(queued_edit(&dir).hero.as_deref(), Some("Lúcio"));
    }
    assert!(pick_hero(&tmp(), "sess-h", "Nobody").is_err());
}

/// Undo on a dismissed run queues a clear, not a Confirm, and says so.
#[test]
fn undo_queues_a_clear_not_a_confirm() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let _ = app.update(crate::app::Message::ResolveSegment {
        session_id: "sess-u".into(),
        segment: 2,
        step: SegmentStep::Undo,
    });
    assert!(matches!(
        queued(&app.data_dir).as_slice(),
        [StoreCommand::ResolveSegment { session_id, segment: 2, action }]
            if session_id == "sess-u" && action == SEGMENT_CLEAR
    ));
    assert_eq!(app.toast.as_deref(), Some("Hero swap undone"));
}

/// A stat edit on a held game saves, and says the game stays local.
#[test]
fn a_stat_edit_on_a_held_game_says_it_stays_local() {
    let dir = tmp();
    let g = held_game();
    let mut form = EditForm::from_game(&g);
    form.elims = "20".into();
    let msg = save(&dir, &g, &form).unwrap().toast;
    assert!(msg.contains("stays on this computer"), "{msg}");
    assert_eq!(queued_edit(&dir).elims, Some(20));
}

#[test]
fn writes_same_ops_as_dioxus_gui() {
    let dir = tmp();
    set_outcome(&dir, "sess-t1", Outcome::Loss).unwrap();
    delete_session(&dir, "sess-t1").unwrap();
    edit_match(
        &dir,
        "sess-t1",
        MatchEdit {
            elims: Some(21),
            ..MatchEdit::default()
        },
    )
    .unwrap();
    resolve_segment(&dir, "sess-t1", 1, SegmentStep::Confirm).unwrap();
    resolve_segment(&dir, "sess-t1", 0, SegmentStep::Dismiss).unwrap();

    let cmds = queued(&dir);
    assert_eq!(cmds.len(), 5);
    assert!(matches!(
        &cmds[0],
        StoreCommand::SetOutcome { session_id, outcome }
            if session_id == "sess-t1" && outcome == "defeat"
    ));
    assert!(matches!(
        &cmds[1],
        StoreCommand::DeleteSession { session_id } if session_id == "sess-t1"
    ));
    assert!(matches!(
        &cmds[2],
        StoreCommand::EditMatch { session_id, edit }
            if session_id == "sess-t1" && edit.elims == Some(21)
    ));
    assert!(matches!(
        &cmds[3],
        StoreCommand::ResolveSegment { session_id, segment, action }
            if session_id == "sess-t1" && *segment == 1 && action == SEGMENT_CONFIRM
    ));
    assert!(matches!(
        &cmds[4],
        StoreCommand::ResolveSegment { session_id, segment, action }
            if session_id == "sess-t1" && *segment == 0 && action == SEGMENT_DISMISS
    ));

    let bytes = std::fs::read_dir(dir.join("commands"))
        .unwrap()
        .flatten()
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .unwrap();
    assert!(
        bytes.contains("\"op\":\"set_outcome\"")
            || read_commands(&dir)
                .iter()
                .any(|(_, c)| matches!(c, StoreCommand::SetOutcome { .. })),
        "command files must deserialize as tagged StoreCommand: {bytes}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn retry_upload_queues_a_retry_command() {
    let dir = tmp();
    retry_upload(&dir, "sess-t1").unwrap();
    let cmds = queued(&dir);
    assert!(matches!(
        &cmds[0],
        StoreCommand::RetryUpload { session_id } if session_id == "sess-t1"
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn retry_rejected_uploads_queues_a_bulk_command() {
    let dir = tmp();
    retry_rejected_uploads(&dir).unwrap();
    let raw = std::fs::read_dir(dir.join("commands"))
        .unwrap()
        .flatten()
        .find(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .unwrap();
    let parsed: StoreCommand = serde_json::from_str(&raw).unwrap();
    assert!(matches!(parsed, StoreCommand::RetryRejectedUploads));
    assert!(raw.contains("\"op\":\"retry_rejected_uploads\""));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_edit_is_rejected() {
    let dir = tmp();
    let g = sample_game();
    let form = EditForm::from_game(&g);
    let err = save(&dir, &g, &form).unwrap_err();
    assert!(err.contains("No changes"));
    assert!(queued(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pick_map_and_hero_queue_the_canonical_names() {
    let dir = tmp();
    let map = stat_tracker::parse::known_map_names()
        .into_iter()
        .find(|name| *name == "Paraiso")
        .expect("Paraiso");
    assert_eq!(map, "Paraiso");
    pick_map(&dir, "sess-t1", map).unwrap();
    pick_hero(&dir, "sess-t1", "Ana").unwrap();
    assert!(pick_map(&dir, "sess-t1", "Paraíso").is_err());
    assert!(pick_hero(&dir, "sess-t1", "Unknown").is_err());
    let cmds = queued(&dir);
    assert_eq!(cmds.len(), 2);
    match &cmds[0] {
        StoreCommand::EditMatch { session_id, edit } => {
            assert_eq!(session_id, "sess-t1");
            assert_eq!(edit.map_name.as_deref(), Some("Paraiso"));
            assert!(edit.hero.is_none());
        }
        other => panic!("map pick: {other:?}"),
    }
    match &cmds[1] {
        StoreCommand::EditMatch { edit, .. } => {
            assert_eq!(edit.hero.as_deref(), Some("Ana"));
            assert!(edit.map_name.is_none());
        }
        other => panic!("hero pick: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn edit_form_diffs_only_changed_fields() {
    let g = sample_game();
    let mut form = EditForm::from_game(&g);
    form.elims = "21".into();
    form.hero = "Kiriko".into();
    let edit = form.diff(&g, false);
    assert_eq!(edit.elims, Some(21));
    assert_eq!(edit.hero.as_deref(), Some("Kiriko"));
    assert!(edit.deaths.is_none());
    assert!(edit.map_name.is_none());
}

/// The game as a reload shows it once the queued commands in `dir`
/// are overlaid on its stored row ([`crate::snapshot::overlay_queued`]).
fn after_queued(dir: &Path, g: &Game) -> Game {
    let mut m = crate::fixtures::pm(
        &g.session_id,
        &g.hero,
        &g.map_name,
        g.role.label(),
        "victory",
        g.elims,
        g.deaths,
        g.assists,
        g.damage,
        g.healing,
        g.mitigation,
        crate::fixtures::dt(2026, 9, 2, 21, 14),
    );
    m.suspect_fields = g.suspect_fields.clone();
    let mut snap = stat_tracker::storage::Snapshot {
        matches: vec![m],
        ..Default::default()
    };
    crate::snapshot::overlay_queued(&mut snap, &queued(dir));
    Game::from_match(&snap.matches[0])
}

/// An unsure stat ("Check first") saved at its read value, with nothing
/// else changed, is sent, so the store marks it edited and the mark
/// clears; the toast says it was confirmed, not corrected, and offers
/// no report. (The store side, `apply_match_edit`, marks any sent stat
/// edited and clears its suspect whatever the value.)
#[test]
fn an_unsure_stat_saved_at_its_read_value_is_confirmed() {
    let mut g = sample_game();
    g.suspect_fields = vec!["e".into()];
    assert!(g.unsure_now("elims"));
    let form = EditForm::from_game(&g);
    assert!(form.diff(&g, false).is_empty());
    let edit = form.diff(&g, true);
    assert_eq!(edit.elims, Some(14));
    assert!(edit.deaths.is_none() && edit.damage.is_none());
    assert!(edit.hero.is_none() && edit.role.is_none() && edit.map_name.is_none());

    let dir = tmp();
    let saved = save(&dir, &g, &form).unwrap();
    assert_eq!(saved.toast, "Elims confirmed");
    assert_eq!(saved.category, None, "a confirm fixed nothing");
    assert_eq!(queued_edit(&dir).elims, Some(14));
    let after = after_queued(&dir, &g);
    assert_eq!(after.elims, 14);
    assert_eq!(after.edited_fields, vec!["elims".to_string()]);
    assert!(!after.unsure_now("elims"), "Check first is gone");
    assert_eq!(after.unsure_stats(), 0);
    let _ = std::fs::remove_dir_all(&dir);

    // Two unsure stats confirmed at once: the toast names both.
    g.suspect_fields.push("dmg".into());
    let dir = tmp();
    let both = save(&dir, &g, &EditForm::from_game(&g)).unwrap();
    assert_eq!(both.toast, "Elims and damage confirmed");
    assert_eq!(both.category, None);
    assert!(after_queued(&dir, &g).suspect_fields.is_empty());
    let _ = std::fs::remove_dir_all(&dir);

    // The same through the app's Save.
    let (_dir, mut app) = crate::fixtures::test_app();
    g.suspect_fields = vec!["e".into()];
    app.games = vec![g.clone()];
    app.expanded = Some(g.session_id.clone());
    let _ = app.update(crate::app::Message::ToggleEdit);
    let _ = app.update(crate::app::Message::SaveEdit);
    assert_eq!(app.toast.as_deref(), Some("Elims confirmed"));
    assert!(app.toast_action.is_none(), "no report for a confirm");
}

/// Each unsure stat is looked up under its own name and confirmed at
/// its own value: a swapped lookup (deaths for assists) sends nothing
/// or the wrong stat, and a swapped toast names the wrong one.
#[test]
fn each_unsure_stat_is_confirmed_under_its_own_name() {
    let mut g = sample_game();
    g.mitigation = 3100;
    type Set = fn(&mut MatchEdit, u32);
    let cases: [(&str, &str, &str, u32, Set); 6] = [
        ("e", "elims", "Elims confirmed", 14, |e, v| {
            e.elims = Some(v)
        }),
        ("d", "deaths", "Deaths confirmed", 5, |e, v| {
            e.deaths = Some(v)
        }),
        ("a", "assists", "Assists confirmed", 16, |e, v| {
            e.assists = Some(v)
        }),
        ("dmg", "damage", "Damage confirmed", 4200, |e, v| {
            e.damage = Some(v)
        }),
        ("h", "healing", "Healing confirmed", 11200, |e, v| {
            e.healing = Some(v)
        }),
        ("mit", "mitigation", "Mitigation confirmed", 3100, |e, v| {
            e.mitigation = Some(v)
        }),
    ];
    for (suspect, field, toast, value, set) in cases {
        g.suspect_fields = vec![suspect.into()];
        let dir = tmp();
        let saved = save(&dir, &g, &EditForm::from_game(&g)).unwrap();
        assert_eq!(saved.toast, toast, "{field}");
        assert_eq!(saved.category, None, "{field}");
        let mut want = MatchEdit::default();
        set(&mut want, value);
        assert_eq!(queued_edit(&dir), want, "{field}");
        let after = after_queued(&dir, &g);
        assert!(!after.unsure_now(field), "{field}");
        assert!(after.suspect_fields.is_empty(), "{field}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    g.suspect_fields = vec!["e".into(), "d".into(), "a".into()];
    let dir = tmp();
    let saved = save(&dir, &g, &EditForm::from_game(&g)).unwrap();
    assert_eq!(saved.toast, "Elims, deaths and assists confirmed");
    let edit = queued_edit(&dir);
    assert_eq!(
        (edit.elims, edit.deaths, edit.assists),
        (Some(14), Some(5), Some(16))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stat the reader was sure of, saved at its value, is still left
/// out, so an untouched form says "No changes to save".
#[test]
fn a_sure_stat_saved_at_its_value_is_still_dropped() {
    let mut g = sample_game();
    g.suspect_fields = vec!["hero".into()];
    assert!(!g.unsure_now("elims"));
    let form = EditForm::from_game(&g);
    assert!(form.diff(&g, true).is_empty());
    let dir = tmp();
    assert_eq!(save(&dir, &g, &form).unwrap_err(), "No changes to save");
    assert!(queued(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// One stat changed and one unsure stat left at its value: only the
/// changed stat is sent, so the unsure one stays Check first.
#[test]
fn a_changed_stat_does_not_confirm_an_unchanged_unsure_stat() {
    let mut g = sample_game();
    g.suspect_fields = vec!["dmg".into()];
    let mut form = EditForm::from_game(&g);
    form.elims = "20".into();
    let dir = tmp();
    let saved = save(&dir, &g, &form).unwrap();
    assert_eq!(saved.toast, "Stats corrected");
    assert_eq!(saved.category, Some(GameCategory::WrongStats));
    let edit = queued_edit(&dir);
    assert_eq!(edit.elims, Some(20));
    assert!(edit.damage.is_none(), "unsure damage is not confirmed");
    assert!(edit.deaths.is_none() && edit.healing.is_none());
    let after = after_queued(&dir, &g);
    assert!(after.unsure_now("damage"));
    assert_eq!(after.suspect_fields, vec!["dmg".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A hero, map or role fix on a game with unsure stats sends no stat,
/// so the stats stay Check first. A typed map or hero that names the
/// current one sends nothing, so that save confirms them instead.
#[test]
fn a_hero_map_or_role_fix_leaves_unsure_stats_unsure() {
    let mut g = sample_game();
    g.suspect_fields = vec!["dmg".into(), "h".into()];
    type Fix = fn(&mut EditForm);
    let fixes: [(Fix, &str); 3] = [
        (|f| f.hero = "Kiriko".into(), "Hero corrected"),
        (|f| f.map_name = "kings row".into(), "Map corrected"),
        (|f| f.role = "Tank".into(), "Role corrected"),
    ];
    for (fix, toast) in fixes {
        let mut form = EditForm::from_game(&g);
        fix(&mut form);
        let dir = tmp();
        let saved = save(&dir, &g, &form).unwrap();
        assert_eq!(saved.toast, toast);
        let edit = queued_edit(&dir);
        assert!(edit.damage.is_none() && edit.healing.is_none(), "{toast}");
        let after = after_queued(&dir, &g);
        assert_eq!(
            after.suspect_fields,
            vec!["dmg".to_string(), "h".into()],
            "{toast}"
        );
        assert!(after.unsure_now("damage") && after.unsure_now("healing"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    let mut form = EditForm::from_game(&g);
    form.map_name = "ilios".into();
    form.hero = "ana".into();
    let dir = tmp();
    let saved = save(&dir, &g, &form).unwrap();
    assert_eq!(saved.toast, "Damage and healing confirmed");
    let edit = queued_edit(&dir);
    assert!(edit.map_name.is_none() && edit.hero.is_none());
    assert_eq!((edit.damage, edit.healing), (Some(4200), Some(11200)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// On a game held for a missing map, an untouched save confirms an
/// unsure stat (the toast says the game is still held), and a save
/// that sets the map leaves it unsure.
#[test]
fn a_held_game_confirms_only_on_an_untouched_save() {
    let mut g = held_game();
    g.suspect_fields = vec!["e".into()];
    let dir = tmp();
    let saved = save(&dir, &g, &EditForm::from_game(&g)).unwrap();
    assert_eq!(
        saved.toast,
        "Elims confirmed. A map or hero is still missing, so this game stays on this computer."
    );
    assert_eq!(saved.category, None);
    assert_eq!(
        queued_edit(&dir),
        MatchEdit {
            elims: Some(14),
            ..MatchEdit::default()
        }
    );
    let _ = std::fs::remove_dir_all(&dir);

    let mut form = EditForm::from_game(&g);
    form.map_name = "Ilios".into();
    let dir = tmp();
    let saved = save(&dir, &g, &form).unwrap();
    assert_eq!(saved.toast, RELEASED);
    let edit = queued_edit(&dir);
    assert_eq!(edit.map_name.as_deref(), Some("Ilios"));
    assert!(edit.elims.is_none());
    assert!(after_queued(&dir, &g).unsure_now("elims"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stat typed as something other than a number ("12,345", as
/// Overwatch prints damage) is not "nothing changed": the save does not
/// confirm the unsure elims, says "No changes to save" as before, and
/// the form stays open with what was typed. With another change it is
/// left out as before, and the unsure stat still is not confirmed.
#[test]
fn an_unparsed_stat_blocks_the_confirm() {
    let mut g = sample_game();
    g.suspect_fields = vec!["e".into()];
    let mut form = EditForm::from_game(&g);
    form.damage = "12,345".into();
    assert!(form.has_unparsed_stat());
    let dir = tmp();
    assert_eq!(save(&dir, &g, &form).unwrap_err(), "No changes to save");
    assert!(queued(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);

    form.deaths = "6".into();
    let dir = tmp();
    assert_eq!(save(&dir, &g, &form).unwrap().toast, "Stats corrected");
    assert_eq!(
        queued_edit(&dir),
        MatchEdit {
            deaths: Some(6),
            ..MatchEdit::default()
        }
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Blank still means keep the read.
    let mut form = EditForm::from_game(&g);
    form.damage = "  ".into();
    assert!(!form.has_unparsed_stat());

    let (_dir, mut app) = crate::fixtures::test_app();
    app.games = vec![g.clone()];
    app.expanded = Some(g.session_id.clone());
    let _ = app.update(crate::app::Message::ToggleEdit);
    app.edit.damage = "12,345".into();
    let _ = app.update(crate::app::Message::SaveEdit);
    assert_eq!(toast(&app), "No changes to save");
    assert!(app.editing, "the form stays open");
    assert_eq!(app.edit.damage, "12,345");
    assert!(queued(&app.data_dir).is_empty());
}

/// Correct damage, then save the form again untouched before the
/// tracker rewrites `live_snapshot.json`: the open game already shows
/// the correction, so the second save confirms only the unsure elims
/// and never re-sends the old damage misread.
#[test]
fn a_second_save_does_not_resend_a_corrected_misread() {
    use crate::app::Message;
    let (_dir, mut app) = crate::fixtures::test_app();
    let g = sample_game();
    let mut m = crate::fixtures::pm(
        &g.session_id,
        &g.hero,
        &g.map_name,
        g.role.label(),
        "victory",
        g.elims,
        g.deaths,
        g.assists,
        g.damage,
        g.healing,
        g.mitigation,
        crate::fixtures::dt(2026, 9, 2, 21, 14),
    );
    m.suspect_fields = vec!["e".into(), "dmg".into()];
    let snap = stat_tracker::storage::Snapshot {
        matches: vec![m],
        ..Default::default()
    };
    std::fs::write(
        app.data_dir.join("live_snapshot.json"),
        serde_json::to_vec(&snap).unwrap(),
    )
    .unwrap();
    app.games =
        crate::snapshot::games_from_snapshot(&crate::snapshot::load_snapshot(&app.data_dir));
    assert_eq!(app.games.len(), 1);
    assert!(app.games[0].unsure_now("elims") && app.games[0].unsure_now("damage"));
    app.expanded = Some(g.session_id.clone());

    let _ = app.update(Message::ToggleEdit);
    app.edit.damage = "4800".into();
    let _ = app.update(Message::SaveEdit);
    assert_eq!(toast(&app), "Stats corrected");
    assert!(!app.editing);
    assert_eq!(app.games[0].damage, 4800, "the open game shows the fix");
    assert!(!app.games[0].unsure_now("damage"));

    let _ = app.update(Message::ToggleEdit);
    assert_eq!(app.edit.damage, "4800");
    let _ = app.update(Message::SaveEdit);
    assert_eq!(toast(&app), "Elims confirmed");
    let cmds = queued(&app.data_dir);
    assert_eq!(cmds.len(), 2);
    let StoreCommand::EditMatch { edit, .. } = &cmds[1] else {
        panic!("{cmds:?}")
    };
    assert_eq!(
        *edit,
        MatchEdit {
            elims: Some(14),
            ..MatchEdit::default()
        },
        "the old damage read 4200 is not sent again"
    );
    assert_eq!(app.games[0].damage, 4800);
    assert_eq!(app.games[0].unsure_stats(), 0);
}

/// A retyped role that names the current one, in any case, is no
/// change, so the untouched save still confirms the unsure stat.
#[test]
fn a_retyped_current_role_does_not_block_the_confirm() {
    let mut g = sample_game();
    g.suspect_fields = vec!["e".into()];
    for typed in ["support", " SUPPORT "] {
        let mut form = EditForm::from_game(&g);
        form.role = typed.into();
        let dir = tmp();
        let saved = save(&dir, &g, &form).unwrap();
        assert_eq!(saved.toast, "Elims confirmed", "{typed}");
        assert_eq!(
            queued_edit(&dir),
            MatchEdit {
                elims: Some(14),
                ..MatchEdit::default()
            },
            "{typed}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// An untouched save does not confirm a stat whose tile says "No read"
/// (the rule is unsure and no read read the shown 0): only the unsure
/// stat with a real read is sent. With only the "No read" stat unsure
/// there is nothing to send.
#[test]
fn an_untouched_save_does_not_confirm_a_no_read_stat() {
    use stat_tracker::games::stat_reads;
    use stat_tracker::unsure_rule::StatRead;
    let mut g = sample_game();
    g.damage = 0;
    g.suspect_fields = vec!["e".into(), "dmg".into()];
    let dir = tmp();
    stat_reads::update(&dir, |l| {
        for _ in 0..3 {
            let mut tab: stat_reads::TabReads = [None; 6];
            tab[3] = Some(StatRead::unsure(None, 0.0));
            l.push(&g.session_id, false, &tab);
        }
    });
    assert!(crate::unsure_reads::no_read(&dir, &g.session_id, "dmg", 0));
    assert!(!crate::unsure_reads::no_read(&dir, &g.session_id, "e", 14));
    let saved = save(&dir, &g, &EditForm::from_game(&g)).unwrap();
    assert_eq!(saved.toast, "Elims confirmed");
    assert_eq!(
        queued_edit(&dir),
        MatchEdit {
            elims: Some(14),
            ..MatchEdit::default()
        }
    );
    let _ = std::fs::remove_dir_all(dir.join("commands"));

    g.suspect_fields = vec!["dmg".into()];
    assert_eq!(
        save(&dir, &g, &EditForm::from_game(&g)).unwrap_err(),
        "No changes to save"
    );
    assert!(queued(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A latest-game unsure stat with a real read is not "No read", so an
/// untouched save confirms it at the shown value. That holds when the
/// read matches the shown value (tile "Read at 40%") and when it differs
/// (tile "Read 14 at 40%").
#[test]
fn an_untouched_save_confirms_a_latest_game_stat_with_a_real_read() {
    use stat_tracker::games::stat_reads;
    use stat_tracker::unsure_rule::{StatRead, Verdict};
    for shown in [14, 15] {
        let mut g = sample_game();
        g.elims = shown;
        g.suspect_fields = vec!["e".into()];
        let dir = tmp();
        stat_reads::update(&dir, |l| {
            for _ in 0..3 {
                let mut tab: stat_reads::TabReads = [None; 6];
                tab[0] = Some(StatRead::unsure(Some(14), 0.4));
                l.push(&g.session_id, false, &tab);
            }
        });
        let elims = stat_reads::load(&dir).confirm(&g.session_id, true)[0].unwrap();
        assert_eq!(elims.verdict, Verdict::Unsure);
        let tile = if shown == 14 {
            "Read at 40%"
        } else {
            "Read 14 at 40%"
        };
        assert_eq!(stat_reads::confidence_line(&elims, shown), tile);
        assert!(!crate::unsure_reads::no_read(
            &dir,
            &g.session_id,
            "e",
            shown
        ));
        let saved = save(&dir, &g, &EditForm::from_game(&g)).unwrap();
        assert_eq!(saved.toast, "Elims confirmed", "{shown}");
        assert_eq!(
            queued_edit(&dir),
            MatchEdit {
                elims: Some(shown),
                ..MatchEdit::default()
            },
            "{shown}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// What the toasts below must say.
const SAYS_NO_SESSION_ID: &str = "no session id";
const SAYS_GAME_GONE: &str = "no longer in the list";

/// Two imported games with no session id, the first one open.
fn app_with_imported_games() -> (tempfile::TempDir, crate::app::TrackerApp) {
    let (dir, mut app) = crate::fixtures::test_app();
    let mut first = sample_game();
    first.session_id = String::new();
    let mut second = sample_game();
    second.session_id = String::new();
    second.hero = "Reinhardt".into();
    app.games = vec![first, second];
    app.expanded = Some(String::new());
    (dir, app)
}

fn toast(app: &crate::app::TrackerApp) -> String {
    app.toast.clone().unwrap_or_default()
}

/// Saving an edit on an imported game queues nothing (the daemon would
/// apply it to every imported game) and says why in a toast.
#[test]
fn an_edit_on_an_imported_game_queues_nothing_and_says_why() {
    let (_dir, mut app) = app_with_imported_games();
    let _ = app.update(crate::app::Message::ToggleEdit);
    assert!(app.editing);
    app.edit.elims = "21".into();
    let _ = app.update(crate::app::Message::SaveEdit);
    assert!(queued(&app.data_dir).is_empty(), "nothing is queued");
    assert!(
        toast(&app).contains(SAYS_NO_SESSION_ID),
        "the toast says why: {:?}",
        app.toast
    );
    assert!(app.editing, "the form stays open with what was typed");
    assert_eq!(app.edit.elims, "21");
}

/// Outcome, map, hero, retry and delete on an imported game: nothing
/// is queued and each one shows why.
#[test]
fn other_changes_to_an_imported_game_queue_nothing_and_say_why() {
    use crate::app::Message;
    let messages = [
        Message::SetOutcome {
            session_id: String::new(),
            outcome: Outcome::Loss,
        },
        Message::PickMap {
            session_id: String::new(),
            map: "Ilios".into(),
        },
        Message::PickHero {
            session_id: String::new(),
            hero: "Ana".into(),
        },
        Message::RetryUpload(String::new()),
        Message::DeleteSession(String::new()),
    ];
    for msg in messages {
        let (_dir, mut app) = app_with_imported_games();
        let shown = format!("{msg:?}");
        let _ = app.update(msg);
        assert!(queued(&app.data_dir).is_empty(), "{shown} queued a command");
        assert!(
            toast(&app).contains(SAYS_NO_SESSION_ID),
            "{shown} toast: {:?}",
            app.toast
        );
        assert_eq!(app.games.len(), 2, "{shown} kept both games");
    }
}

/// An edit whose game left the list says so instead of closing quietly.
#[test]
fn an_edit_whose_game_is_gone_says_so() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let g = sample_game();
    app.games = vec![g.clone()];
    app.expanded = Some(g.session_id.clone());
    let _ = app.update(crate::app::Message::ToggleEdit);
    app.edit.elims = "21".into();
    app.games.clear();
    let _ = app.update(crate::app::Message::SaveEdit);
    assert!(queued(&app.data_dir).is_empty());
    assert!(
        toast(&app).contains(SAYS_GAME_GONE),
        "the toast says the game is gone: {:?}",
        app.toast
    );
}
