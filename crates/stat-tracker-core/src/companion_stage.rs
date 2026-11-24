//! The pre-game stage for the companion (0.7.1 C3).
//!
//! The daemon writes the last known pre-game stage into the status file as
//! [`crate::ocr_status::Status::companion_stage`]. The companion reads it
//! on its tick to pick a page. Values only: a stage name, canonical map
//! names, a first-seen time, and a last-seen time.
//!
//! Stages the daemon writes today, when detect reports them:
//! - `map_vote`: a map vote. `maps` holds canonical `MAPS` display names
//!   when `reader = "ocr-v1"` (or the words pack has no vote masks). The
//!   default reader (`reader = "new"`) always publishes an empty `maps`
//!   list.
//! - `hero_select`: Select a preferred hero (the map is locked). On
//!   `ocr-v1` the same detector also confirms Assemble Your Team, which
//!   can come after the ban, and that is published as `hero_select` too.
//! - `hero_ban`: the hero ban vote (the map is still locked).
//!
//! `in_game` is in the type for when detect produces a match-start phase.
//! Nothing outside tests produces `GamePhase::InGame` today,
//! so the daemon does not write `in_game` on a live run.
//!
//! An unknown phase publishes nothing, so the last stage stays in the file.
//! After [`SIGHTING_GAP_SECS`] with no known phase, a later repeat is a
//! new sighting (`since` is reset). `seen` is refreshed every few seconds
//! while the stage repeats, so the companion can treat it as current while
//! `now - seen` is small.
//!
//! Read across versions like the rest of the status file: a file from an
//! older build has no stage (`None`), and a stage name a newer daemon adds
//! reads as [`StageKind::Unknown`], so the rest of the file still loads.
//! A `companion_stage` value this build cannot parse is read as `None`
//! and does not make the rest of the status file unreadable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Deserializer, Serialize};

use crate::ocr_status::{read_status, status_write_lock, write_status};

/// A known phase that repeats after this many seconds with no known phase
/// in between is a new sighting (new `since`).
pub const SIGHTING_GAP_SECS: i64 = 60;
/// How often `seen` is rewritten while the same sighting repeats.
const SEEN_REFRESH_SECS: i64 = 5;

/// Which pre-game screen the daemon last saw. Written in snake case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    /// The map vote. [`CompanionStage::maps`] holds the names read there.
    MapVote,
    /// Select a preferred hero: the map is locked. On `ocr-v1` this also
    /// covers Assemble Your Team.
    HeroSelect,
    /// The hero ban vote: the map is still locked.
    HeroBan,
    /// The match is on. Detect does not produce this phase yet, so a live
    /// daemon does not write this kind today.
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
    /// Canonical map names (the `MAPS` display names), deduped, order not
    /// meaningful, empty when none was read. Empty on every stage but a
    /// vote, and on a vote where no name was read (including every vote
    /// under the default reader).
    pub maps: Vec<String>,
    /// When the daemon first saw this sighting, in Unix seconds. A stage
    /// that repeats within [`SIGHTING_GAP_SECS`] keeps this time, even if
    /// the vote's map names change between ticks. A repeat after a longer
    /// gap is a new sighting.
    pub since: i64,
    /// When the daemon last saw this sighting, in Unix seconds. Refreshed
    /// at most every few seconds while the stage repeats. The companion
    /// can treat the stage as current while `now - seen` is small.
    pub seen: i64,
}

impl CompanionStage {
    pub fn new(kind: StageKind, maps: Vec<String>, since: i64) -> Self {
        Self {
            kind,
            maps,
            since,
            seen: since,
        }
    }
}

struct Slot {
    last_known_at: i64,
    stage: Option<CompanionStage>,
    written: Option<CompanionStage>,
    write_failed: bool,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            last_known_at: i64::MIN,
            stage: None,
            written: None,
            write_failed: false,
        }
    }
}

static SLOTS: OnceLock<Mutex<HashMap<PathBuf, Slot>>> = OnceLock::new();

fn slots() -> MutexGuard<'static, HashMap<PathBuf, Slot>> {
    SLOTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

/// The stage last published for this data dir in this process. Status
/// writes copy it, so a later OCR status write does not wipe the stage.
pub fn companion_stage_for(data_dir: &Path) -> Option<CompanionStage> {
    slots().get(data_dir).and_then(|slot| slot.stage.clone())
}

fn settle_stage(slot: &mut Slot, new: CompanionStage) -> Option<CompanionStage> {
    let now = new.since;
    let same_sighting = slot.stage.as_ref().is_some_and(|old| old.kind == new.kind)
        && now >= slot.last_known_at
        && now - slot.last_known_at < SIGHTING_GAP_SECS;
    let stage = if same_sighting {
        let old = slot.stage.as_ref().expect("same_sighting implies a stage");
        let seen = if now - old.seen >= SEEN_REFRESH_SECS {
            now
        } else {
            old.seen
        };
        CompanionStage {
            kind: new.kind,
            maps: new.maps,
            since: old.since,
            seen,
        }
    } else {
        CompanionStage {
            kind: new.kind,
            maps: new.maps,
            since: now,
            seen: now,
        }
    };
    slot.last_known_at = now;
    slot.stage = Some(stage.clone());
    Some(stage)
}

/// Write the stage into the status file (`None` clears it), keeping
/// everything else there. A repeat of the same kind within
/// [`SIGHTING_GAP_SECS`] keeps the first `since` (map names may change).
/// `seen` is refreshed at most every few seconds. The held slot is keyed
/// by `data_dir` and is marked written only after the file write succeeds.
/// Later status writes keep the stage too.
pub fn publish_companion_stage(data_dir: &Path, stage: Option<CompanionStage>) {
    let _guard = status_write_lock();
    let to_write = {
        let mut slots = slots();
        let slot = slots.entry(data_dir.to_path_buf()).or_default();
        match stage {
            None => slot.stage = None,
            Some(new) => {
                settle_stage(slot, new);
            }
        }
        if slot.written == slot.stage {
            return;
        }
        slot.stage.clone()
    };
    let mut status = read_status(data_dir).unwrap_or_default();
    if status.companion_stage == to_write {
        let mut slots = slots();
        if let Some(slot) = slots.get_mut(data_dir) {
            slot.written.clone_from(&to_write);
            slot.write_failed = false;
        }
        return;
    }
    status.companion_stage = to_write.clone();
    match write_status(data_dir, &status) {
        Ok(()) => {
            let mut slots = slots();
            if let Some(slot) = slots.get_mut(data_dir) {
                slot.written = to_write;
                slot.write_failed = false;
            }
        }
        Err(e) => {
            let mut slots = slots();
            if let Some(slot) = slots.get_mut(data_dir) {
                if !slot.write_failed {
                    slot.write_failed = true;
                    tracing::warn!(
                        error = %e,
                        "could not write the companion stage; will retry on a later tick"
                    );
                } else {
                    tracing::debug!(error = %e, "could not write the companion stage");
                }
            }
        }
    }
}

/// The stage in the status file, or `None` when there is no file, it does
/// not parse, it holds no stage, or `status.pid` is not `live_pid`.
pub fn read_companion_stage(data_dir: &Path, live_pid: u32) -> Option<CompanionStage> {
    let status = read_status(data_dir)?;
    if status.pid != live_pid {
        return None;
    }
    status.companion_stage
}

/// Deserialize [`Status::companion_stage`] so a shape this build cannot
/// parse becomes `None` instead of failing the whole status file.
pub fn deserialize_lenient<'de, D>(deserializer: D) -> Result<Option<CompanionStage>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr_status::Status;

    fn written_stage(dir: &Path) -> Option<CompanionStage> {
        let status = read_status(dir).unwrap();
        assert_eq!(status.pid, 0);
        status.companion_stage
    }

    #[test]
    fn publish_then_read_round_trips_and_a_republish_keeps_the_stage() {
        let dir = tempfile::tempdir().unwrap();
        let vote = CompanionStage::new(
            StageKind::MapVote,
            vec!["Circuit Royal".into(), "Havana".into(), "Ilios".into()],
            100,
        );
        publish_companion_stage(dir.path(), Some(vote.clone()));
        assert_eq!(read_companion_stage(dir.path(), 0), Some(vote.clone()));
        assert_eq!(companion_stage_for(dir.path()), Some(vote.clone()));

        // The same stage two seconds later keeps its first time and does
        // not rewrite the file (`seen` is not due yet).
        let mtime = std::fs::metadata(crate::ocr_status::status_path(dir.path()))
            .unwrap()
            .modified()
            .unwrap();
        let later = CompanionStage::new(StageKind::MapVote, vote.maps.clone(), 102);
        publish_companion_stage(dir.path(), Some(later));
        assert_eq!(read_companion_stage(dir.path(), 0), Some(vote.clone()));
        let mtime_later = std::fs::metadata(crate::ocr_status::status_path(dir.path()))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(mtime, mtime_later);

        // Another writer of the file (the OCR switch) builds its Status
        // from the held slots: the stage survives that write.
        let republished = Status {
            pid: 9,
            companion_stage: companion_stage_for(dir.path()),
            ..Status::default()
        };
        write_status(dir.path(), &republished).unwrap();
        assert_eq!(read_companion_stage(dir.path(), 9), Some(vote.clone()));
        assert_eq!(read_companion_stage(dir.path(), 0), None);

        let ban = CompanionStage::new(StageKind::HeroBan, Vec::new(), 120);
        publish_companion_stage(dir.path(), Some(ban.clone()));
        assert_eq!(read_companion_stage(dir.path(), 9), Some(ban));
        // Other fields in the file stay.
        assert_eq!(read_status(dir.path()).unwrap().pid, 9);

        publish_companion_stage(dir.path(), None);
        assert_eq!(read_companion_stage(dir.path(), 9), None);
        assert_eq!(companion_stage_for(dir.path()), None);
    }

    #[test]
    fn a_repeat_after_the_gap_is_a_new_sighting() {
        let dir = tempfile::tempdir().unwrap();
        let first = CompanionStage::new(StageKind::MapVote, Vec::new(), 1_000);
        publish_companion_stage(dir.path(), Some(first.clone()));
        assert_eq!(written_stage(dir.path()).unwrap().since, 1_000);

        // t + 2: same since, no write.
        let soon = CompanionStage::new(StageKind::MapVote, Vec::new(), 1_002);
        publish_companion_stage(dir.path(), Some(soon));
        let held = written_stage(dir.path()).unwrap();
        assert_eq!(held.since, 1_000);
        assert_eq!(held.seen, 1_000);

        // t + 600: new since, file rewritten.
        let later = CompanionStage::new(StageKind::MapVote, Vec::new(), 1_600);
        publish_companion_stage(dir.path(), Some(later));
        let held = written_stage(dir.path()).unwrap();
        assert_eq!(held.since, 1_600);
        assert_eq!(held.seen, 1_600);
    }

    #[test]
    fn map_names_changing_inside_the_gap_keep_since() {
        let dir = tempfile::tempdir().unwrap();
        let first = CompanionStage::new(StageKind::MapVote, vec!["Havana".into()], 50);
        publish_companion_stage(dir.path(), Some(first));
        let richer = CompanionStage::new(
            StageKind::MapVote,
            vec!["Havana".into(), "Ilios".into()],
            52,
        );
        publish_companion_stage(dir.path(), Some(richer));
        let held = written_stage(dir.path()).unwrap();
        assert_eq!(held.since, 50);
        assert_eq!(held.maps, vec!["Havana", "Ilios"]);
    }

    #[test]
    fn the_held_slot_is_keyed_by_data_dir() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let vote = CompanionStage::new(StageKind::MapVote, vec!["Ilios".into()], 10);
        publish_companion_stage(a.path(), Some(vote.clone()));
        publish_companion_stage(b.path(), Some(vote.clone()));
        assert_eq!(read_companion_stage(a.path(), 0), Some(vote.clone()));
        assert_eq!(read_companion_stage(b.path(), 0), Some(vote));
    }

    #[test]
    fn an_old_status_file_without_the_field_loads_with_no_stage() {
        let old: Status = serde_json::from_str("{\"pid\": 3, \"share_open\": true}").unwrap();
        assert_eq!(old.companion_stage, None);
        assert!(old.share_open);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(crate::ocr_status::status_path(dir.path()), b"{\"pid\": 3}").unwrap();
        assert_eq!(read_companion_stage(dir.path(), 3), None);
    }

    #[test]
    fn an_unknown_stage_name_does_not_make_the_file_unreadable() {
        let text = r#"{"pid": 4, "share_open": true,
            "companion_stage": {"kind": "round_end", "maps": [], "since": 7, "seen": 7, "extra": 1}}"#;
        let status: Status = serde_json::from_str(text).unwrap();
        assert_eq!(status.pid, 4);
        assert!(status.share_open);
        assert_eq!(
            status.companion_stage,
            Some(CompanionStage::new(StageKind::Unknown, Vec::new(), 7))
        );
    }

    #[test]
    fn a_bad_stage_shape_does_not_make_the_file_unreadable() {
        let text = r#"{"pid": 5, "share_open": true,
            "companion_stage": {"kind": {"name": "map_vote"}, "since": "now"}}"#;
        let status: Status = serde_json::from_str(text).unwrap();
        assert_eq!(status.pid, 5);
        assert!(status.share_open);
        assert_eq!(status.companion_stage, None);
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
            assert!(text.contains("\"seen\""), "{text}");
            assert_eq!(
                serde_json::from_str::<CompanionStage>(&text).unwrap(),
                stage
            );
        }
    }
}
