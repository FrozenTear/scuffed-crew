//! Write `StoreCommand` files under `<data_dir>/commands/`, same contract
//! as the Dioxus GUI. The daemon applies them; this crate never opens the store.

use std::path::Path;

use stat_tracker::storage::{
    self, MatchEdit, SEGMENT_CONFIRM, SEGMENT_DISMISS, StoreCommand, read_commands,
};

use crate::model::{Game, Outcome};

pub fn queue(data_dir: &Path, cmd: &StoreCommand) -> Result<(), String> {
    storage::queue_command(data_dir, cmd).map_err(|e| e.to_string())
}

pub fn set_outcome(data_dir: &Path, session_id: &str, outcome: Outcome) -> Result<(), String> {
    queue(
        data_dir,
        &StoreCommand::SetOutcome {
            session_id: session_id.to_string(),
            outcome: outcome.store_label().to_string(),
        },
    )
}

pub fn delete_session(data_dir: &Path, session_id: &str) -> Result<(), String> {
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
    queue(
        data_dir,
        &StoreCommand::EditMatch {
            session_id: session_id.to_string(),
            edit,
        },
    )
}

pub fn retry_upload(data_dir: &Path, session_id: &str) -> Result<(), String> {
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

pub fn resolve_segment(
    data_dir: &Path,
    session_id: &str,
    segment: u32,
    confirm: bool,
) -> Result<(), String> {
    queue(
        data_dir,
        &StoreCommand::ResolveSegment {
            session_id: session_id.to_string(),
            segment,
            action: if confirm {
                SEGMENT_CONFIRM.to_string()
            } else {
                SEGMENT_DISMISS.to_string()
            },
        },
    )
}

/// Queue the Edit form's changes and say what they did. A typed map goes
/// through [`stat_tracker::parse::canonical_map`] ("kings row" is King's
/// Row) and must be a known map, and a typed hero must be on the hero list
/// in any case, so the store holds the names the reader and the website
/// use. A game held back for a missing map or hero uploads once both are
/// set; the message says when one is still missing.
pub fn save_edit(
    data_dir: &Path,
    game: &Game,
    form: &crate::model::EditForm,
) -> Result<&'static str, String> {
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
    edit_match(data_dir, &game.session_id, edit)?;
    Ok(if held {
        "Saved. A map or hero is still missing, so this game stays on this computer."
    } else {
        "Stats corrected"
    })
}

/// The HEROES name a typed hero means. Case, punctuation and accents fold
/// the way the reader's keys do ([`scuffed_types::hero_key_to_name`]), so
/// "soldier 76", "dva", "Lúcio" and "Torbjörn" all find their hero.
fn hero_from_typed(typed: &str) -> Option<&'static str> {
    let typed = typed.trim();
    let want = scuffed_types::hero_key_to_name(typed);
    scuffed_types::HEROES.iter().copied().find(|h| {
        h.eq_ignore_ascii_case(typed)
            || want.is_some_and(|w| scuffed_types::hero_key_to_name(h) == Some(w))
    })
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
    if !scuffed_types::HEROES.contains(&hero) {
        return Err("Pick a hero from the list".into());
    }
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
        assert_eq!(save_edit(&dir, &g, &form).unwrap(), "Stats corrected");
        let edit = queued_edit(&dir);
        assert_eq!(edit.map_name.as_deref(), Some("King's Row"));
        let mut after = g.clone();
        after.map_name = edit.map_name.unwrap();
        assert!(!after.needs_review());
    }

    #[test]
    fn an_unknown_map_is_refused_and_nothing_is_queued() {
        let dir = tmp();
        let g = held_game();
        let mut form = EditForm::from_game(&g);
        form.map_name = "Not A Real Place".into();
        let err = save_edit(&dir, &g, &form).unwrap_err();
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
        save_edit(&dir, &g, &form).unwrap();
        assert_eq!(queued_edit(&dir).hero.as_deref(), Some("Kiriko"));
        form.hero = "Nobody".into();
        assert!(
            save_edit(&dir, &g, &form)
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

    /// The reader's own spellings find the hero, stored as HEROES has it.
    #[test]
    fn a_typed_hero_folds_case_punctuation_and_accents() {
        for (typed, want) in [
            ("soldier 76", "Soldier: 76"),
            ("dva", "D.Va"),
            ("Lúcio", "Lucio"),
            ("Torbjörn", "Torbjorn"),
            ("wrecking-ball", "Wrecking Ball"),
        ] {
            let stored = hero_from_typed(typed);
            assert_eq!(stored, Some(want), "{typed}");
            assert!(scuffed_types::HEROES.contains(&want), "{want}");
        }
        assert_eq!(hero_from_typed("Nobody"), None);
    }

    /// A stat edit on a held game saves, and says the game stays local.
    #[test]
    fn a_stat_edit_on_a_held_game_says_it_stays_local() {
        let dir = tmp();
        let g = held_game();
        let mut form = EditForm::from_game(&g);
        form.elims = "20".into();
        let msg = save_edit(&dir, &g, &form).unwrap();
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
        resolve_segment(&dir, "sess-t1", 1, true).unwrap();
        resolve_segment(&dir, "sess-t1", 0, false).unwrap();

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
        let err = save_edit(&dir, &g, &form).unwrap_err();
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
}
