//! Write `StoreCommand` files under `<data_dir>/commands/`, same contract
//! as the Dioxus GUI. The daemon applies them; this crate never opens the store.

use std::path::Path;

use stat_tracker::storage::{
    self, MatchEdit, SEGMENT_CLEAR, SEGMENT_CONFIRM, SEGMENT_DISMISS, StoreCommand, read_commands,
};

use crate::model::{Game, Outcome};
use crate::report::GameCategory;

pub fn queue(data_dir: &Path, cmd: &StoreCommand) -> Result<(), String> {
    storage::queue_command(data_dir, cmd).map_err(|e| e.to_string())
}

/// Why a change to an imported game is refused. Its commands are keyed by
/// session id, and the empty one would match every imported game.
pub(crate) const NO_SESSION_ID: &str =
    "This imported game has no session id, so it can't be changed in the app";

/// A save whose game is no longer in the list.
pub(crate) const GAME_GONE: &str = "Could not save: this game is no longer in the list";

fn require_session(session_id: &str) -> Result<(), String> {
    if session_id.is_empty() {
        return Err(NO_SESSION_ID.into());
    }
    Ok(())
}

pub fn set_outcome(data_dir: &Path, session_id: &str, outcome: Outcome) -> Result<(), String> {
    require_session(session_id)?;
    queue(
        data_dir,
        &StoreCommand::SetOutcome {
            session_id: session_id.to_string(),
            outcome: outcome.store_label().to_string(),
        },
    )
}

pub fn delete_session(data_dir: &Path, session_id: &str) -> Result<(), String> {
    require_session(session_id)?;
    queue(
        data_dir,
        &StoreCommand::DeleteSession {
            session_id: session_id.to_string(),
        },
    )
}

pub fn edit_match(data_dir: &Path, session_id: &str, edit: MatchEdit) -> Result<(), String> {
    if edit.is_empty() {
        return Err("No changes to save".into());
    }
    require_session(session_id)?;
    queue(
        data_dir,
        &StoreCommand::EditMatch {
            session_id: session_id.to_string(),
            edit,
        },
    )
}

pub fn retry_upload(data_dir: &Path, session_id: &str) -> Result<(), String> {
    require_session(session_id)?;
    queue(
        data_dir,
        &StoreCommand::RetryUpload {
            session_id: session_id.to_string(),
        },
    )
}

pub fn retry_rejected_uploads(data_dir: &Path) -> Result<(), String> {
    queue(data_dir, &StoreCommand::RetryRejectedUploads)
}

/// What a hero swap button does to its run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStep {
    Confirm,
    Dismiss,
    /// Take back the run's Confirm or Dismiss: the run goes back to what
    /// the reader made of it (a one-snapshot swap asks again).
    Undo,
}

impl SegmentStep {
    fn action(self) -> &'static str {
        match self {
            SegmentStep::Confirm => SEGMENT_CONFIRM,
            SegmentStep::Dismiss => SEGMENT_DISMISS,
            SegmentStep::Undo => SEGMENT_CLEAR,
        }
    }

    /// The toast once it is queued.
    pub fn toast(self) -> &'static str {
        match self {
            SegmentStep::Confirm => "Hero swap confirmed",
            SegmentStep::Dismiss => "Hero swap dismissed",
            SegmentStep::Undo => "Hero swap undone",
        }
    }
}

pub fn resolve_segment(
    data_dir: &Path,
    session_id: &str,
    segment: u32,
    step: SegmentStep,
) -> Result<(), String> {
    require_session(session_id)?;
    queue(
        data_dir,
        &StoreCommand::ResolveSegment {
            session_id: session_id.to_string(),
            segment,
            action: step.action().to_string(),
        },
    )
}

/// Queue the Edit form's changes and say what they did. A typed map goes
/// through [`stat_tracker::parse::canonical_map`] ("kings row" is King's
/// Row) and must be a known map, and a typed hero must be on the hero list
/// in any case, so the store holds the names the reader and the website
/// use. A game held back for a missing map or hero uploads once both are
/// set; the message says when one is still missing. `in_progress` and
/// `sync_on` pick the release toast the way the picker does.
pub fn save_edit(
    data_dir: &Path,
    game: &Game,
    form: &crate::model::EditForm,
    in_progress: bool,
    sync_on: bool,
) -> Result<Saved, String> {
    let mut edit = form.diff(game);
    if let Some(typed) = edit.map_name.take() {
        let map = stat_tracker::parse::canonical_map(&typed)
            .filter(|m| stat_tracker::parse::map_is_known(m))
            .ok_or_else(|| format!("Map not found: {typed}. Type a map name like King's Row"))?;
        edit.map_name = (map != game.map_name).then_some(map);
    }
    if let Some(typed) = edit.hero.take() {
        let hero = hero_from_typed(&typed)
            .ok_or_else(|| format!("Hero not found: {typed}. Type a hero name like Ana"))?;
        edit.hero = (*hero != game.hero).then(|| hero.to_string());
    }
    let map = edit.map_name.as_deref().unwrap_or(&game.map_name);
    let hero = edit.hero.as_deref().unwrap_or(&game.hero);
    let held = !stat_tracker::parse::review_suspect_fields(map, "", hero).is_empty();
    let was_held =
        !stat_tracker::parse::review_suspect_fields(&game.map_name, "", &game.hero).is_empty();
    let stats = [
        edit.elims,
        edit.deaths,
        edit.assists,
        edit.damage,
        edit.healing,
        edit.mitigation,
    ];
    let stats_changed = stats.iter().any(Option::is_some);
    let map_changed = edit.map_name.is_some();
    let hero_fixed = edit.hero.is_some();
    let role_changed = edit.role.is_some();
    let map_after = edit.map_name.clone();
    let hero_after = edit.hero.clone();
    edit_match(data_dir, &game.session_id, edit)?;
    let mut after = game.clone();
    if let Some(map) = map_after {
        after.map_name = map;
    }
    if let Some(hero) = hero_after {
        after.hero = hero;
    }
    let release = was_held.then(|| release_toast(&after, in_progress, sync_on));
    Ok(saved(
        held,
        stats_changed,
        map_changed,
        hero_fixed,
        role_changed,
        release,
    ))
}

/// [`save_edit`] for the open Edit form: the toast follows whether this
/// game is still being played and whether a website is linked, read as
/// the panel reads it ([`crate::games_panel::sync_on`]), so the toast and
/// the picker's line never disagree.
pub fn save_edit_for(
    app: &crate::app::TrackerApp,
    game: &Game,
    form: &crate::model::EditForm,
) -> Result<Saved, String> {
    save_edit(
        &app.data_dir,
        game,
        form,
        crate::model::game_card_in_progress(
            &game.session_id,
            game.outcome,
            app.live_game_session.as_deref(),
        ),
        crate::games_panel::sync_on(app),
    )
}

/// What a queued Edit says, from whether the game is still held, was held,
/// which kinds of field changed, and the game after the save. A role-only
/// change reads "Role corrected" and offers no training report, as there
/// is no role category. A release takes its line from [`upload`], as the
/// picker does: a live game uploads once it ends, and with no website
/// linked it stays on this computer.
fn saved(
    held: bool,
    stats_changed: bool,
    map_changed: bool,
    hero_fixed: bool,
    role_changed: bool,
    release: Option<&'static str>,
) -> Saved {
    let hero_changed = hero_fixed || role_changed;
    let role_only = !hero_fixed && role_changed;
    // The training report files the fix under what was fixed: a stat fix
    // first, as the stats are what the reader reads most. A save that
    // changed nothing fixed nothing, so it offers no report. A role-only
    // fix offers none either: there is no role category, and filed as
    // Wrong hero it would say the hero was misread when only the role was
    // changed. With a map fix too, the report is for the map.
    let category = if stats_changed {
        Some(GameCategory::WrongStats)
    } else if map_changed {
        Some(GameCategory::WrongMap)
    } else if hero_fixed {
        Some(GameCategory::WrongHero)
    } else {
        None
    };
    let toast = if held {
        "Saved. A map or hero is still missing, so this game stays on this computer."
    } else if stats_changed {
        "Stats corrected"
    } else if let Some(line) = release {
        line
    } else {
        match (map_changed, hero_changed, role_only) {
            (true, true, false) => "Map and hero corrected",
            (true, true, true) => "Map and role corrected",
            (true, false, _) => "Map corrected",
            (false, true, false) => "Hero corrected",
            (false, true, true) => "Role corrected",
            (false, false, _) => "Saved",
        }
    };
    Saved { toast, category }
}

/// What an Edit save says, and the training report category its toast
/// offers (`None`: the save changed nothing, so no report is offered).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
    pub toast: &'static str,
    pub category: Option<GameCategory>,
}

/// Show an Edit save's toast, with the training report offer when the
/// save fixed something.
pub(crate) fn show_saved(app: &mut crate::app::TrackerApp, sid: &str, saved: &Saved) {
    app.replace_toast(saved.toast);
    if let Some(category) = saved.category {
        app.arm_training_toast(sid, category);
    }
}

/// A save that set the missing map or hero and changed no stat.
pub const RELEASED: &str = "Saved. Nothing is missing now.";

/// [`RELEASED`] on a game with no result yet: it still does not upload,
/// in the picker's words ("it uploads once you set the result").
pub const RELEASED_NO_RESULT: &str = "Saved. It uploads once you set the result.";

/// [`RELEASED`] on a game still being played.
pub const RELEASED_LIVE: &str = "Saved. It uploads once it ends.";

/// [`RELEASED`] with no website linked: the picker says so too.
pub const RELEASED_SYNC_OFF: &str = "Saved. It stays on this computer: no website is linked.";

/// The release toast for the game after the save, from [`upload`].
fn release_toast(after: &Game, in_progress: bool, sync_on: bool) -> &'static str {
    use crate::games_detail::{Upload, upload};
    match upload(after, in_progress, sync_on) {
        Upload::InProgress => RELEASED_LIVE,
        Upload::SyncOff => RELEASED_SYNC_OFF,
        Upload::Undecided => RELEASED_NO_RESULT,
        Upload::Held(_) | Upload::Waiting | Upload::Uploaded | Upload::Rejected(_) => RELEASED,
    }
}

/// The stored name of the hero a typed name means. Case, punctuation and
/// accents fold the way the reader's keys do
/// ([`scuffed_types::hero_key_to_name`]), so "soldier 76", "dva", "Lucio"
/// and "Torbjörn" all find their hero. The result is the name the reader
/// stores (Lúcio, Torbjörn), so one hero never gets two rows.
fn hero_from_typed(typed: &str) -> Option<&'static str> {
    let typed = typed.trim();
    let want = scuffed_types::hero_key_to_name(typed);
    scuffed_types::HEROES
        .iter()
        .copied()
        .find(|h| {
            h.eq_ignore_ascii_case(typed)
                || want.is_some_and(|w| scuffed_types::hero_key_to_name(h) == Some(w))
        })
        .map(stored_hero)
}

/// The name the reader stores for a HEROES name.
fn stored_hero(h: &'static str) -> &'static str {
    scuffed_types::hero_key_to_name(h).unwrap_or(h)
}

/// Queue a map picked from [`stat_tracker::parse::known_map_names`].
/// The daemon derives the mode from that name.
pub fn pick_map(data_dir: &Path, session_id: &str, map: &str) -> Result<(), String> {
    if !stat_tracker::parse::map_is_known(map) {
        return Err("Pick a map from the list".into());
    }
    edit_match(
        data_dir,
        session_id,
        MatchEdit {
            map_name: Some(map.to_string()),
            ..MatchEdit::default()
        },
    )
}

/// Queue a hero picked from the canonical hero list.
/// The daemon derives the role. `Unknown` is not in the list.
pub fn pick_hero(data_dir: &Path, session_id: &str, hero: &str) -> Result<(), String> {
    let Some(hero) = scuffed_types::HEROES
        .iter()
        .copied()
        .find(|h| *h == hero || stored_hero(h) == hero)
        .map(stored_hero)
    else {
        return Err("Pick a hero from the list".into());
    };
    edit_match(
        data_dir,
        session_id,
        MatchEdit {
            hero: Some(hero.to_string()),
            ..MatchEdit::default()
        },
    )
}

/// Newest queued command (tests / docs). Empty if the directory is missing.
pub fn queued(data_dir: &Path) -> Vec<StoreCommand> {
    read_commands(data_dir)
        .into_iter()
        .map(|(_, c)| c)
        .collect()
}

#[cfg(test)]
mod tests {
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
        let picker = release_line(&g, Field::Map, Some("Ilios"), false, true);
        assert_eq!(
            release_line(
                &g,
                Field::Map,
                Some("Ilios"),
                false,
                crate::games_panel::sync_on(&app)
            ),
            picker
        );
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
        assert_eq!(app.toast.as_deref(), Some(saved.toast));
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
        let edit = form.diff(&g);
        assert_eq!(edit.elims, Some(21));
        assert_eq!(edit.hero.as_deref(), Some("Kiriko"));
        assert!(edit.deaths.is_none());
        assert!(edit.map_name.is_none());
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
}
