//! Write `StoreCommand` files under `<data_dir>/commands/` — same contract
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

pub fn pick_hero(data_dir: &Path, session_id: &str, hero: &str) -> Result<(), String> {
    if !stat_tracker::parse::hero_is_known(hero) {
        return Err("Pick a hero from the list".into());
    }
    let mut edit = MatchEdit {
        hero: Some(hero.to_string()),
        ..MatchEdit::default()
    };
    if let Some(role) = scuffed_types::role_for_hero_name(hero) {
        edit.role = Some(role.to_string());
    }
    edit_match(data_dir, session_id, edit)
}

pub fn retry_upload(data_dir: &Path, session_id: &str) -> Result<(), String> {
    queue(
        data_dir,
        &StoreCommand::RetryUpload {
            session_id: session_id.to_string(),
        },
    )
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

pub fn save_edit(
    data_dir: &Path,
    game: &Game,
    form: &crate::model::EditForm,
) -> Result<(), String> {
    edit_match(data_dir, &game.session_id, form.diff(game))
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
        }
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
    fn pick_map_and_hero_queue_known_names_only() {
        let dir = tmp();
        let err = pick_map(&dir, "sess-t1", "Paraíso").unwrap_err();
        assert!(err.contains("list"));
        assert!(queued(&dir).is_empty());
        pick_map(&dir, "sess-t1", "King's Row").unwrap();
        pick_hero(&dir, "sess-t1", "Reinhardt").unwrap();
        let cmds = queued(&dir);
        assert!(matches!(
            &cmds[0],
            StoreCommand::EditMatch { session_id, edit }
                if session_id == "sess-t1" && edit.map_name.as_deref() == Some("King's Row")
        ));
        assert!(matches!(
            &cmds[1],
            StoreCommand::EditMatch { session_id, edit }
                if session_id == "sess-t1"
                    && edit.hero.as_deref() == Some("Reinhardt")
                    && edit.role.as_deref() == Some("Tank")
        ));
        let mut game = sample_game();
        assert!(!game.needs_review());
        game.map_name.clear();
        assert!(game.map_needs_pick());
        assert_eq!(game.map_label(), "Map needed");
        game.map_name = "King's Row".into();
        game.hero = "Unknown".into();
        assert!(!game.map_needs_pick());
        assert!(game.hero_needs_pick());
        assert!(game.needs_review());
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
