//! Publish the pre-game stage for the companion (0.7.1 C3).
//!
//! The poll calls [`publish`] with each tick's [`GamePhase`]. A known phase
//! becomes the status file's stage
//! ([`stat_tracker::companion_stage::CompanionStage`]); an unknown one
//! publishes nothing, so the last stage stays in the file. The sighting gap
//! ([`stat_tracker::companion_stage::SIGHTING_GAP_SECS`]) does not clear
//! it: a later known phase after that long is a new sighting (new `since`).
//! The write keeps everything else in the file, and later OCR status writes
//! keep the stage (`ocr_switch` copies the held slot).
//!
//! Vote `maps` are canonical `MAPS` display names from the latest tick,
//! deduped, in first-seen order (order is not meaningful). The list can
//! shrink within a sighting. They are filled only on the ocr-v1 route; the
//! default reader always publishes an empty list.

use std::path::Path;

use stat_tracker::companion_stage::{CompanionStage, StageKind, publish_companion_stage};
use stat_tracker::detect::GamePhase;

/// Canonical `MAPS` display names, deduped in first-seen order. Raw
/// detector keys that do not map to a name are dropped.
fn canonical_maps(maps: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for raw in maps {
        let Some(name) = stat_tracker::parse::canonical_map(raw) else {
            continue;
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The stage for a phase, first seen at `now` (Unix seconds), or `None`
/// for [`GamePhase::Unknown`]. Map vote names are canonical `MAPS`
/// display names from this tick's read, deduped, empty when none was read.
pub(crate) fn stage_of(phase: &GamePhase, now: i64) -> Option<CompanionStage> {
    let (kind, maps) = match phase {
        GamePhase::MapVote { maps } => (StageKind::MapVote, canonical_maps(maps)),
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
        publish_companion_stage(data_dir, Some((stage.kind, stage.maps, stage.since)));
    }
}
