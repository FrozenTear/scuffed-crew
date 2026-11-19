//! Write `StoreCommand` files under `<data_dir>/commands/`, same contract
//! as the Dioxus GUI. The daemon applies them; this crate never opens the store.

use std::path::Path;

use stat_tracker::storage::{
    self, MatchEdit, SEGMENT_CLEAR, SEGMENT_CONFIRM, SEGMENT_DISMISS, StoreCommand, read_commands,
};

use crate::games_detail::Stat;
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
    let mut edit = form.diff(game, false);
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
    // A typed role that names the current one ("support" on a Support
    // game) is no change. A role that names none is sent as typed.
    if let Some(typed) = edit.role.take() {
        let role = crate::model::Role::parse(&typed);
        edit.role = (role == crate::model::Role::Unknown || role != game.role).then_some(typed);
    }
    if edit.is_empty() && !form.has_unparsed_stat() {
        // Nothing to correct: an unsure stat left at its read value is sent
        // to confirm it. Only then, so a fix to anything else never
        // confirms a Check first stat the member did not look at, and a
        // stat typed as something other than a number ("12,345") still
        // gets "No changes to save". A typed map, hero or role that names
        // the current one stays unsent. A "No read" stat has no read to
        // confirm, so it stays unsent too.
        edit = MatchEdit {
            hero: None,
            role: None,
            map_name: None,
            ..form.diff(game, true)
        };
        for (sent, stat) in [
            (&mut edit.elims, Stat::E),
            (&mut edit.deaths, Stat::D),
            (&mut edit.assists, Stat::A),
            (&mut edit.damage, Stat::Dmg),
            (&mut edit.healing, Stat::Heal),
            (&mut edit.mitigation, Stat::Mit),
        ] {
            let shown = stat.value(game);
            if crate::unsure_reads::no_read(data_dir, &game.session_id, stat.flat(), shown) {
                *sent = None;
            }
        }
    }
    let map = edit.map_name.as_deref().unwrap_or(&game.map_name);
    let hero = edit.hero.as_deref().unwrap_or(&game.hero);
    let held = !stat_tracker::parse::review_suspect_fields(map, "", hero).is_empty();
    let was_held =
        !stat_tracker::parse::review_suspect_fields(&game.map_name, "", &game.hero).is_empty();
    // An unsure stat saved at its shown value is sent to confirm it (see
    // above): that confirms the read, it does not correct it.
    let stats = [
        (edit.elims, game.elims, "Elims"),
        (edit.deaths, game.deaths, "Deaths"),
        (edit.assists, game.assists, "Assists"),
        (edit.damage, game.damage, "Damage"),
        (edit.healing, game.healing, "Healing"),
        (edit.mitigation, game.mitigation, "Mitigation"),
    ];
    let stats_changed = stats
        .iter()
        .any(|(v, shown, _)| v.is_some_and(|v| v != *shown));
    let confirmed: Vec<&str> = stats
        .iter()
        .filter(|(v, shown, _)| *v == Some(*shown))
        .map(|(_, _, name)| *name)
        .collect();
    let confirmed =
        crate::games_rows::names_list(&confirmed).map(|list| format!("{list} confirmed"));
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
        confirmed,
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

/// What a queued Edit says, from whether the game is still held, which
/// kinds of field changed, and the release line when the save lifted a
/// hold (`None` when the game was not held). `confirmed` is the line for
/// unsure stats saved at their read value ("Elims confirmed", "Elims and
/// deaths confirmed"), sent only when nothing else changed, and it leads
/// the held line too; a confirm fixed nothing, so it offers no report. A role-only change reads
/// "Role corrected" and offers no training report, as there is no role
/// category. A release takes its line from [`upload`], as the
/// picker does: a live game uploads once it ends, and with no website
/// linked it stays on this computer.
fn saved(
    held: bool,
    stats_changed: bool,
    map_changed: bool,
    hero_fixed: bool,
    role_changed: bool,
    confirmed: Option<String>,
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
    let toast: String = if held {
        // A held game names its confirm too: "Elims confirmed. A map or
        // hero is still missing, ...".
        let lead = confirmed.as_deref().unwrap_or("Saved");
        format!("{lead}. A map or hero is still missing, so this game stays on this computer.")
    } else if stats_changed {
        "Stats corrected".into()
    } else if let Some(line) = release {
        line.into()
    } else {
        match (map_changed, hero_changed, role_only) {
            (true, true, false) => "Map and hero corrected".into(),
            (true, true, true) => "Map and role corrected".into(),
            (true, false, _) => "Map corrected".into(),
            (false, true, false) => "Hero corrected".into(),
            (false, true, true) => "Role corrected".into(),
            (false, false, _) => confirmed.unwrap_or_else(|| "Saved".into()),
        }
    };
    Saved { toast, category }
}

/// What an Edit save says, and the training report category its toast
/// offers (`None`: the save changed nothing, so no report is offered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub toast: String,
    pub category: Option<GameCategory>,
}

/// Show an Edit save's toast, with the training report offer when the
/// save fixed something.
pub(crate) fn show_saved(app: &mut crate::app::TrackerApp, sid: &str, saved: &Saved) {
    app.replace_toast(saved.toast.clone());
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
#[path = "commands_tests.rs"]
mod tests;
