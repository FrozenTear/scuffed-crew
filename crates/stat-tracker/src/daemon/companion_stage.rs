//! Publish the pre-game stage for the companion (0.7.1 C3).
//!
//! The poll calls [`publish`] with each tick's [`GamePhase`]. A known phase
//! becomes the status file's stage
//! ([`stat_tracker::companion_stage::CompanionStage`]); an unknown one
//! publishes nothing, so the last stage seen stays until the next one. The
//! write keeps everything else in the file, and later OCR status writes keep
//! the stage (`ocr_switch` copies the held slot).

use std::path::Path;

use stat_tracker::companion_stage::{CompanionStage, StageKind, publish_companion_stage};
use stat_tracker::detect::GamePhase;

/// The stage for a phase, first seen at `now` (Unix seconds), or `None`
/// for [`GamePhase::Unknown`]. Map vote names are kept as read.
pub(crate) fn stage_of(phase: &GamePhase, now: i64) -> Option<CompanionStage> {
    let (kind, maps) = match phase {
        GamePhase::MapVote { maps } => (StageKind::MapVote, maps.clone()),
        GamePhase::HeroSelect => (StageKind::HeroSelect, Vec::new()),
        GamePhase::HeroBan => (StageKind::HeroBan, Vec::new()),
        GamePhase::InGame => (StageKind::InGame, Vec::new()),
        GamePhase::Unknown => return None,
    };
    Some(CompanionStage::new(kind, maps, now))
}

/// Write this tick's stage into the status file when the phase is known.
pub(crate) fn publish(data_dir: &Path, phase: &GamePhase) {
    if let Some(stage) = stage_of(phase, chrono::Utc::now().timestamp()) {
        publish_companion_stage(data_dir, Some(stage));
    }
}

#[cfg(test)]
mod tests_companion_stage {
    use super::*;

    #[test]
    fn each_game_phase_maps_to_its_stage() {
        let maps = vec![
            "Circuit Royale".to_string(),
            "Havana".into(),
            "Ilios".into(),
        ];
        let vote = stage_of(&GamePhase::MapVote { maps: maps.clone() }, 10).unwrap();
        assert_eq!(vote, CompanionStage::new(StageKind::MapVote, maps, 10));
        // A vote read with no names is still the vote, with no maps.
        let blank = stage_of(&GamePhase::MapVote { maps: Vec::new() }, 11).unwrap();
        assert_eq!(blank.kind, StageKind::MapVote);
        assert!(blank.maps.is_empty());
        for (phase, kind) in [
            (GamePhase::HeroSelect, StageKind::HeroSelect),
            (GamePhase::HeroBan, StageKind::HeroBan),
            (GamePhase::InGame, StageKind::InGame),
        ] {
            assert_eq!(
                stage_of(&phase, 12),
                Some(CompanionStage::new(kind, Vec::new(), 12))
            );
        }
        assert_eq!(stage_of(&GamePhase::Unknown, 13), None);
    }

    #[test]
    fn an_unknown_phase_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        publish(dir.path(), &GamePhase::Unknown);
        assert!(!stat_tracker::ocr_status::status_path(dir.path()).exists());
    }
}
