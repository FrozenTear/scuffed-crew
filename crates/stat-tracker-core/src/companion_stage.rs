//! The pre-game stage for the companion (0.7.1 C3).
//!
//! The daemon writes the stage it last saw (map vote with the maps it read,
//! hero select, hero ban, match start) into the status file as
//! [`crate::ocr_status::Status::companion_stage`]. The companion reads it on
//! its tick to pick a page. Values only: a stage name, map names as read and
//! a time.
//!
//! Read across versions like the rest of the status file: a file from an
//! older build has no stage (`None`), and a stage name a newer daemon adds
//! reads as [`StageKind::Unknown`], so the rest of the file still loads.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ocr_status::{read_status, status_write_lock, write_status};

/// Which pre-game screen the daemon last saw. Written in snake case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    /// The map vote. [`CompanionStage::maps`] holds the names read there.
    MapVote,
    /// Select a preferred hero: the map is locked.
    HeroSelect,
    /// The hero ban vote: the map is still locked.
    HeroBan,
    /// The match is on.
    InGame,
    /// A name this build does not know.
    #[default]
    #[serde(other)]
    Unknown,
}

/// The stage in [`crate::ocr_status::Status::companion_stage`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CompanionStage {
    pub kind: StageKind,
    /// The map names read on the vote screen, as read. Empty on every other
    /// stage, and on a vote where no name was read.
    pub maps: Vec<String>,
    /// When the daemon first saw this stage, in Unix seconds. A stage that
    /// repeats on later ticks keeps its first time.
    pub since: i64,
}

impl CompanionStage {
    pub fn new(kind: StageKind, maps: Vec<String>, since: i64) -> Self {
        Self { kind, maps, since }
    }

    /// The same stage, whatever time it was first seen.
    pub fn same_as(&self, other: &Self) -> bool {
        self.kind == other.kind && self.maps == other.maps
    }
}

static STAGE: std::sync::Mutex<Option<CompanionStage>> = std::sync::Mutex::new(None);

/// The stage last published in this process. Status writes copy it, so a
/// later OCR status write does not wipe the stage.
pub fn companion_stage() -> Option<CompanionStage> {
    STAGE.lock().unwrap_or_else(|err| err.into_inner()).clone()
}

/// Write the stage into the status file (`None` clears it), keeping
/// everything else there. A stage equal to the one held, apart from its
/// time, keeps the first time and touches no file. Later status writes keep
/// it too.
pub fn publish_companion_stage(data_dir: &Path, stage: Option<CompanionStage>) {
    let _guard = status_write_lock();
    let stage = {
        let mut held = STAGE.lock().unwrap_or_else(|err| err.into_inner());
        match (held.as_ref(), stage) {
            // Already written by this process: no file read on every tick.
            (Some(old), Some(new)) if old.same_as(&new) => return,
            (None, None) => return,
            (_, new) => {
                held.clone_from(&new);
                new
            }
        }
    };
    let mut status = read_status(data_dir).unwrap_or_default();
    if status.companion_stage == stage {
        return;
    }
    status.companion_stage = stage;
    let _ = write_status(data_dir, &status);
}

/// The stage in the status file, or `None` when there is no file, it does
/// not parse, or it holds no stage.
pub fn read_companion_stage(data_dir: &Path) -> Option<CompanionStage> {
    read_status(data_dir)?.companion_stage
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    *STAGE.lock().unwrap_or_else(|err| err.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr_status::Status;

    // The in-memory slot is process wide: one test owns it.
    #[test]
    fn publish_then_read_round_trips_and_a_republish_keeps_the_stage() {
        reset_for_test();
        let dir = tempfile::tempdir().unwrap();
        let vote = CompanionStage::new(
            StageKind::MapVote,
            vec!["Circuit Royale".into(), "Havana".into(), "Ilios".into()],
            100,
        );
        publish_companion_stage(dir.path(), Some(vote.clone()));
        assert_eq!(read_companion_stage(dir.path()), Some(vote.clone()));
        assert_eq!(companion_stage(), Some(vote.clone()));

        // The same stage on a later tick keeps its first time.
        let later = CompanionStage {
            since: 105,
            ..vote.clone()
        };
        publish_companion_stage(dir.path(), Some(later));
        assert_eq!(read_companion_stage(dir.path()), Some(vote.clone()));

        // Another writer of the file (the OCR switch) builds its Status
        // from the held slots: the stage survives that write.
        let republished = Status {
            pid: 9,
            companion_stage: companion_stage(),
            ..Status::default()
        };
        write_status(dir.path(), &republished).unwrap();
        assert_eq!(read_companion_stage(dir.path()), Some(vote));

        let ban = CompanionStage::new(StageKind::HeroBan, Vec::new(), 120);
        publish_companion_stage(dir.path(), Some(ban.clone()));
        assert_eq!(read_companion_stage(dir.path()), Some(ban));
        // Other fields in the file stay.
        assert_eq!(read_status(dir.path()).unwrap().pid, 9);

        publish_companion_stage(dir.path(), None);
        assert_eq!(read_companion_stage(dir.path()), None);
        assert_eq!(companion_stage(), None);
        reset_for_test();
    }

    #[test]
    fn an_old_status_file_without_the_field_loads_with_no_stage() {
        let old: Status = serde_json::from_str("{\"pid\": 3, \"share_open\": true}").unwrap();
        assert_eq!(old.companion_stage, None);
        assert!(old.share_open);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(crate::ocr_status::status_path(dir.path()), b"{\"pid\": 3}").unwrap();
        assert_eq!(read_companion_stage(dir.path()), None);
    }

    #[test]
    fn an_unknown_stage_name_does_not_make_the_file_unreadable() {
        let text = r#"{"pid": 4, "share_open": true,
            "companion_stage": {"kind": "round_end", "maps": [], "since": 7, "extra": 1}}"#;
        let status: Status = serde_json::from_str(text).unwrap();
        assert_eq!(status.pid, 4);
        assert!(status.share_open);
        assert_eq!(
            status.companion_stage,
            Some(CompanionStage::new(StageKind::Unknown, Vec::new(), 7))
        );
    }

    #[test]
    fn every_kind_round_trips_in_snake_case() {
        for (kind, name) in [
            (StageKind::MapVote, "map_vote"),
            (StageKind::HeroSelect, "hero_select"),
            (StageKind::HeroBan, "hero_ban"),
            (StageKind::InGame, "in_game"),
            (StageKind::Unknown, "unknown"),
        ] {
            let stage = CompanionStage::new(kind, Vec::new(), 1);
            let text = serde_json::to_string(&stage).unwrap();
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
            assert_eq!(serde_json::from_str::<CompanionStage>(&text).unwrap(), stage);
        }
    }
}
