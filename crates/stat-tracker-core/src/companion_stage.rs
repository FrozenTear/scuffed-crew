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
//! [`SIGHTING_GAP_SECS`] does not clear it: after that long with no known
//! phase, a later repeat is a new sighting (`since` is reset). `seen` is
//! refreshed at most every [`SEEN_REFRESH_SECS`] while the stage is still
//! on screen. A reader should treat the stage as current only while
//! `now - seen` is at most [`seen_current_for_secs`] (that refresh plus
//! two poll intervals plus 1 s, because `seen` is a whole second rounded
//! down, and one poll may be skipped).
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
///
/// A reader should not use this alone as the freshness limit. While the
/// stage is still on screen, `now - seen` can reach [`seen_current_for_secs`].
pub const SEEN_REFRESH_SECS: i64 = 5;

/// The largest `now - seen` a reader should treat as current while the
/// stage is still on screen: [`SEEN_REFRESH_SECS`] plus two poll
/// intervals plus 1 s (`seen` is a whole second, rounded down). The
/// second poll is room for one skipped poll. A poll shorter than 1 s,
/// including zero, counts as 1 s; a fractional poll rounds up.
pub fn seen_current_for_secs(poll_interval: std::time::Duration) -> i64 {
    let poll_secs = i64::try_from(poll_interval.as_secs()).unwrap_or(i64::MAX);
    let poll_secs = if poll_interval.subsec_nanos() > 0 {
        poll_secs.saturating_add(1)
    } else {
        poll_secs
    }
    .max(1);
    SEEN_REFRESH_SECS
        .saturating_add(poll_secs.saturating_mul(2))
        .saturating_add(1)
}

/// Slack for a `seen` slightly ahead of `now` (whole-second rounding
/// or a small clock skew). Further ahead is not current.
const SEEN_FUTURE_SLACK_SECS: i64 = 2;

/// True when `now - seen` is in `-2..=seen_current_for_secs` (`-2` is
/// [`SEEN_FUTURE_SLACK_SECS`]). A `seen` further in the future is not
/// current.
pub fn stage_is_current(seen: i64, now: i64, poll_interval: std::time::Duration) -> bool {
    let age = now.saturating_sub(seen);
    let bound = seen_current_for_secs(poll_interval);
    age >= -SEEN_FUTURE_SLACK_SECS && age <= bound
}

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
    /// Canonical map names (the `MAPS` display names) from the latest
    /// tick's read, deduped, order not meaningful. The list can shrink or
    /// empty within one sighting if a later tick reads fewer names. Empty
    /// on every stage but a vote, and on a vote where no name was read
    /// (including every vote under the default reader).
    pub maps: Vec<String>,
    /// When the daemon first saw this sighting, in Unix seconds. A stage
    /// that repeats within [`SIGHTING_GAP_SECS`] keeps this time, even if
    /// the vote's map names change between ticks. A repeat after a longer
    /// gap is a new sighting.
    pub since: i64,
    /// When the daemon last saw this sighting, in Unix seconds. Refreshed
    /// once [`SEEN_REFRESH_SECS`] have passed since the last `seen`.
    /// While the stage is on screen, `now - seen` is at most
    /// [`seen_current_for_secs`]; treat the stage as current only within
    /// that bound. A missed screen or a stopped daemon leaves `seen`
    /// behind, including across an update restart that keeps the same pid.
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

fn settle_stage(slot: &mut Slot, kind: StageKind, maps: Vec<String>, now: i64) {
    let kept = slot.stage.as_ref().and_then(|old| {
        if old.kind == kind
            && now >= slot.last_known_at
            && now - slot.last_known_at < SIGHTING_GAP_SECS
        {
            Some((old.since, old.seen))
        } else {
            None
        }
    });
    let stage = if let Some((since, old_seen)) = kept {
        let seen = if now - old_seen >= SEEN_REFRESH_SECS {
            now
        } else {
            old_seen
        };
        CompanionStage {
            kind,
            maps,
            since,
            seen,
        }
    } else {
        CompanionStage {
            kind,
            maps,
            since: now,
            seen: now,
        }
    };
    slot.last_known_at = now;
    slot.stage = Some(stage);
}

/// Write this tick's stage into the status file (`None` clears it), keeping
/// everything else there. `tick` is `(kind, maps, now)`: `now` is this
/// tick's Unix time, used as `since` on a new sighting and to refresh
/// `seen`. Pass that tick time; do not pass a `since` copied from a
/// `CompanionStage` that was read back. A repeat of
/// the same kind within [`SIGHTING_GAP_SECS`] keeps the first `since`
/// (map names may change). The held slot is keyed by `data_dir` and is
/// marked written only after the file write succeeds. Later status writes
/// keep the stage too.
pub fn publish_companion_stage(data_dir: &Path, tick: Option<(StageKind, Vec<String>, i64)>) {
    let _guard = status_write_lock();
    let to_write = {
        let mut slots = slots();
        let slot = slots.entry(data_dir.to_path_buf()).or_default();
        match tick {
            None => slot.stage = None,
            Some((kind, maps, now)) => settle_stage(slot, kind, maps, now),
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

/// Deserialize [`crate::ocr_status::Status::companion_stage`] so a shape
/// this build cannot parse becomes `None` instead of failing the whole
/// status file.
pub(crate) fn deserialize_lenient<'de, D>(
    deserializer: D,
) -> Result<Option<CompanionStage>, D::Error>
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
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    fn publish_tick(dir: &Path, kind: StageKind, maps: Vec<String>, now: i64) {
        publish_companion_stage(dir, Some((kind, maps, now)));
    }

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
        publish_tick(dir.path(), vote.kind, vote.maps.clone(), vote.since);
        assert_eq!(read_companion_stage(dir.path(), 0), Some(vote.clone()));
        assert_eq!(companion_stage_for(dir.path()), Some(vote.clone()));

        // The same stage two seconds later keeps its first time and does
        // not rewrite the file (`seen` is not due yet).
        let path = crate::ocr_status::status_path(dir.path());
        #[cfg(unix)]
        let inode = std::fs::metadata(&path).unwrap().ino();
        let later = CompanionStage::new(StageKind::MapVote, vote.maps.clone(), 102);
        publish_tick(dir.path(), later.kind, later.maps.clone(), later.since);
        assert_eq!(read_companion_stage(dir.path(), 0), Some(vote.clone()));
        #[cfg(unix)]
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);

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
        publish_tick(dir.path(), ban.kind, ban.maps.clone(), ban.since);
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
        publish_tick(dir.path(), first.kind, first.maps.clone(), first.since);
        assert_eq!(written_stage(dir.path()).unwrap().since, 1_000);
        let path = crate::ocr_status::status_path(dir.path());
        #[cfg(unix)]
        let inode = std::fs::metadata(&path).unwrap().ino();

        // t + 2: same since, no write.
        let soon = CompanionStage::new(StageKind::MapVote, Vec::new(), 1_002);
        publish_tick(dir.path(), soon.kind, soon.maps.clone(), soon.since);
        let held = written_stage(dir.path()).unwrap();
        assert_eq!(held.since, 1_000);
        assert_eq!(held.seen, 1_000);
        #[cfg(unix)]
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);

        // t + 600: new since, file rewritten.
        let later = CompanionStage::new(StageKind::MapVote, Vec::new(), 1_600);
        publish_tick(dir.path(), later.kind, later.maps.clone(), later.since);
        let held = written_stage(dir.path()).unwrap();
        assert_eq!(held.since, 1_600);
        assert_eq!(held.seen, 1_600);
    }

    #[test]
    fn map_names_changing_inside_the_gap_keep_since() {
        let dir = tempfile::tempdir().unwrap();
        let first = CompanionStage::new(StageKind::MapVote, vec!["Havana".into()], 50);
        publish_tick(dir.path(), first.kind, first.maps.clone(), first.since);
        let richer = CompanionStage::new(
            StageKind::MapVote,
            vec!["Havana".into(), "Ilios".into()],
            52,
        );
        publish_tick(dir.path(), richer.kind, richer.maps.clone(), richer.since);
        let held = written_stage(dir.path()).unwrap();
        assert_eq!(held.since, 50);
        assert_eq!(held.maps, vec!["Havana", "Ilios"]);
    }

    #[test]
    fn the_held_slot_is_keyed_by_data_dir() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let vote = CompanionStage::new(StageKind::MapVote, vec!["Ilios".into()], 10);
        publish_tick(a.path(), vote.kind, vote.maps.clone(), vote.since);
        publish_tick(b.path(), vote.kind, vote.maps.clone(), vote.since);
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
    #[test]
    fn freshness_helpers_cover_poll_edges_and_a_future_seen() {
        use std::time::Duration;
        let cases: &[(Duration, i64)] = &[
            (Duration::ZERO, 8),
            (Duration::from_millis(1), 8),
            (Duration::from_secs(1), 8),
            (Duration::from_millis(1500), 10),
            (Duration::from_secs(4), 14),
        ];
        let now = 1_000;
        for &(poll, bound) in cases {
            assert_eq!(seen_current_for_secs(poll), bound, "{poll:?}");
            assert!(stage_is_current(now, now, poll), "age 0 {poll:?}");
            assert!(
                stage_is_current(now - bound, now, poll),
                "age == bound {poll:?}"
            );
            assert!(
                !stage_is_current(now - bound - 1, now, poll),
                "age == bound+1 {poll:?}"
            );
            assert!(
                stage_is_current(now + SEEN_FUTURE_SLACK_SECS, now, poll),
                "2 s ahead {poll:?}"
            );
            assert!(
                !stage_is_current(now + SEEN_FUTURE_SLACK_SECS + 1, now, poll),
                "3 s ahead {poll:?}"
            );
            assert!(
                !stage_is_current(now + 3_600, now, poll),
                "1 h ahead {poll:?}"
            );
            assert!(
                !stage_is_current(i64::MAX, now, poll),
                "i64::MAX seen {poll:?}"
            );
        }
        // A huge poll must not overflow or panic.
        let huge = Duration::from_secs(u64::MAX);
        assert_eq!(seen_current_for_secs(huge), i64::MAX);
        assert!(stage_is_current(now, now, huge));
    }

    fn seen_is_refreshed_after_the_interval_and_keeps_the_inode_before() {
        let dir = tempfile::tempdir().unwrap();
        let maps = vec![
            "Circuit Royal".into(),
            "Esperanca".into(),
            "Throne of Anubis".into(),
        ];
        publish_tick(dir.path(), StageKind::MapVote, maps.clone(), 1_000);
        let path = crate::ocr_status::status_path(dir.path());
        #[cfg(unix)]
        let inode = std::fs::metadata(&path).unwrap().ino();
        let held = written_stage(dir.path()).unwrap();
        assert_eq!(held.since, 1_000);
        assert_eq!(held.seen, 1_000);

        // t + 4: same inode, seen stays t (refresh is 5 s).
        publish_tick(dir.path(), StageKind::MapVote, maps.clone(), 1_004);
        let held = written_stage(dir.path()).unwrap();
        assert_eq!(held.since, 1_000);
        assert_eq!(held.seen, 1_000);
        #[cfg(unix)]
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);

        // t + 5: rewritten, same since, seen is t + 5.
        publish_tick(dir.path(), StageKind::MapVote, maps, 1_005);
        let held = written_stage(dir.path()).unwrap();
        assert_eq!(held.since, 1_000);
        assert_eq!(held.seen, 1_005);
    }

    #[test]
    fn a_failed_write_is_retried_and_keeps_the_first_since() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("no_status_dir");
        publish_tick(missing.as_path(), StageKind::HeroSelect, Vec::new(), 100);
        assert!(!crate::ocr_status::status_path(&missing).exists());

        std::fs::create_dir(&missing).unwrap();
        // Same kind 1 s later: the first tick's since is kept, and the
        // write is retried now that the dir exists.
        publish_tick(missing.as_path(), StageKind::HeroSelect, Vec::new(), 101);
        let held = written_stage(&missing).unwrap();
        assert_eq!(held.kind, StageKind::HeroSelect);
        assert_eq!(held.since, 100);
        assert_eq!(held.seen, 100);
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
