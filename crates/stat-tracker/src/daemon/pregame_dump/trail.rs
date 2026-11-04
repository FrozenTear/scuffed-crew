//! Why a game ended up with no hero-select frame or no picks: a small record
//! of the pre-game, kept only for one summary line when the game is first
//! recorded ([`PregameTrail::finish`]). It changes nothing the saver or the
//! reader does.
//!
//! The record starts over when a new game folder opens and after each
//! summary, so it covers the stretch from the last game to this one.

use super::{PregameOp, PregamePhase, Seen};
use stat_tracker::model::{BansPicksMissing, BansPicksStatus};

/// Why the picks were not stored, as far as the tracker knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MissReason {
    /// The focus gate paused the poll and the hero select was never kept.
    /// `in_pregame`: a pre-game screen was up (or a game folder open) when
    /// it paused.
    FocusPaused { in_pregame: bool },
    /// No hero select was read: no phase read and no select title.
    SelectNeverDetected,
    /// The hero select was read and no frame of it was kept, and why.
    SelectNotKept(&'static str),
    /// A hero-select frame was kept and the reader stored no picks, and why.
    ReaderSkipped(String),
}

impl std::fmt::Display for MissReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FocusPaused { in_pregame: true } => {
                f.write_str("focus gate paused the poll during the pre-game")
            }
            Self::FocusPaused { in_pregame: false } => f.write_str(
                "focus gate paused the poll since the last game (no pre-game screen up then)",
            ),
            Self::SelectNeverDetected => f.write_str("hero select never detected"),
            Self::SelectNotKept(why) => write!(f, "hero select seen, no frame kept: {why}"),
            Self::ReaderSkipped(why) => write!(f, "reader skipped: {why}"),
        }
    }
}

/// What the reader did for the game, for [`PregameTrail::summary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReaderResult {
    /// The reader ran, or was skipped, and put this status on the game.
    Status(BansPicksStatus),
    /// No frames were held for this game, and why.
    NoFrames(&'static str),
    /// No reader runs (switched off).
    Off,
}

impl ReaderResult {
    /// The hero select's last frame was read, so its picks are on the game.
    fn picks_stored(&self) -> bool {
        match self {
            Self::Status(BansPicksStatus::Ok) => true,
            Self::Status(BansPicksStatus::Partial { missing }) => {
                !missing.contains(&BansPicksMissing::HeroSelect)
            }
            _ => false,
        }
    }

    fn why(&self) -> String {
        match self {
            Self::Off | Self::Status(BansPicksStatus::ReaderOff) => "reader off".into(),
            Self::NoFrames(why) => (*why).into(),
            Self::Status(BansPicksStatus::NoFrames) => "no pre-game frames held".into(),
            Self::Status(BansPicksStatus::FrameSize { width, height }) => {
                format!("wrong resolution {width}x{height}")
            }
            Self::Status(BansPicksStatus::PackMissing { linked }) => {
                format!("pack missing (linked = {linked})")
            }
            Self::Status(BansPicksStatus::PackFailed) => "pack did not load".into(),
            Self::Status(BansPicksStatus::ReaderFailed) => "reader failed".into(),
            Self::Status(BansPicksStatus::Partial { .. }) => {
                "no hero-select last frame (a first frame gives the team ring only)".into()
            }
            Self::Status(BansPicksStatus::Ok) => String::new(),
        }
    }
}

/// The summary line's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PregameSummary {
    /// The hero select was read (a phase read or its title).
    pub(crate) select_seen: bool,
    /// A hero-select frame (first or last) was kept for this game.
    pub(crate) select_frame: bool,
    /// The reader read the hero select's last frame.
    pub(crate) picks_stored: bool,
    /// The focus gate paused the poll at some point since the last game,
    /// shown even when another reason is given.
    pub(crate) focus_paused: bool,
    /// `None` when the picks were stored.
    pub(crate) reason: Option<MissReason>,
}

/// The pre-game record ([`self`]).
#[derive(Debug, Default)]
pub(crate) struct PregameTrail {
    select_seen: bool,
    select_frame: bool,
    /// The last reason a hero-select frame was not kept.
    select_skip: Option<&'static str>,
    focus_paused: bool,
    focus_paused_in_pregame: bool,
    /// The session the last summary was for: one line per game.
    summarized_for: Option<String>,
}

impl PregameTrail {
    /// Start over (a new game folder): what came before was another game.
    pub(crate) fn new_game(&mut self) {
        let summarized_for = self.summarized_for.take();
        *self = Self {
            summarized_for,
            ..Self::default()
        };
    }

    /// What one poll tick saw.
    pub(crate) fn screen(&mut self, seen: Seen) {
        if matches!(
            seen,
            Seen::Phase(PregamePhase::HeroSelect) | Seen::SelectTitle
        ) {
            self.select_seen = true;
        }
    }

    /// A hero-select frame was not kept, and why.
    pub(crate) fn select_skipped(&mut self, why: &'static str) {
        self.select_skip = Some(why);
    }

    /// A batch the saver sends: a hero-select save is a kept frame.
    pub(crate) fn shots<F>(&mut self, ops: &[PregameOp<F>]) {
        let select = [
            PregamePhase::HeroSelect.first_name(),
            PregamePhase::HeroSelect.last_name(),
        ];
        if ops
            .iter()
            .any(|op| matches!(op, PregameOp::Save(shot) if select.contains(&shot.name)))
        {
            self.select_frame = true;
        }
    }

    /// The focus gate skipped a poll tick; `in_pregame` as at that tick.
    pub(crate) fn focus_paused(&mut self, in_pregame: bool) {
        self.focus_paused = true;
        self.focus_paused_in_pregame |= in_pregame;
    }

    /// The summary for a game the reader handled as `reader`.
    pub(crate) fn summary(&self, reader: &ReaderResult) -> PregameSummary {
        let picks_stored = reader.picks_stored();
        let reason = if picks_stored {
            None
        } else if self.select_frame {
            Some(MissReason::ReaderSkipped(reader.why()))
        } else if self.focus_paused_in_pregame {
            Some(MissReason::FocusPaused { in_pregame: true })
        } else if self.select_seen {
            Some(MissReason::SelectNotKept(
                self.select_skip.unwrap_or("no hero-select save was queued"),
            ))
        } else if self.focus_paused {
            Some(MissReason::FocusPaused { in_pregame: false })
        } else {
            Some(MissReason::SelectNeverDetected)
        };
        PregameSummary {
            select_seen: self.select_seen,
            select_frame: self.select_frame,
            picks_stored,
            focus_paused: self.focus_paused,
            reason,
        }
    }

    /// The game `session_id` was recorded and the reader handled it as
    /// `reader`: the summary, once per session, and the record starts over.
    /// `None` for a later capture of a game already summarized.
    pub(crate) fn finish(
        &mut self,
        session_id: &str,
        reader: &ReaderResult,
    ) -> Option<PregameSummary> {
        if self.summarized_for.as_deref() == Some(session_id) {
            return None;
        }
        let summary = self.summary(reader);
        *self = Self {
            summarized_for: Some(session_id.to_string()),
            ..Self::default()
        };
        Some(summary)
    }
}

/// The local summary file in the data dir: one line per recorded game, the
/// last [`SUMMARY_KEEP`] kept. The journal keeps only minutes; this keeps
/// the reason for days. It is never uploaded.
pub(crate) const SUMMARY_FILE: &str = "pregame-summary.log";

/// Lines kept in [`SUMMARY_FILE`].
pub(crate) const SUMMARY_KEEP: usize = 200;

/// `summary` for `session_id` at `at`, as one file line (no line break).
pub(crate) fn summary_line(
    at: chrono::DateTime<chrono::Utc>,
    session_id: &str,
    summary: &PregameSummary,
) -> String {
    let reason = reason_text(summary);
    let line = format!(
        "{} session={session_id} hero_select_seen={} hero_select_frame={} picks_stored={} focus_paused={} reason={reason}",
        at.format("%Y-%m-%dT%H:%M:%SZ"),
        summary.select_seen,
        summary.select_frame,
        summary.picks_stored,
        summary.focus_paused,
    );
    line.replace(['\n', '\r'], " ")
}

/// Append `line` to the file at `path` and keep only its last `keep`
/// lines. Written to a temporary file next to it, then renamed over it.
pub(crate) fn append_capped(
    path: &std::path::Path,
    line: &str,
    keep: usize,
) -> std::io::Result<()> {
    let old = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let mut lines: Vec<&str> = old.lines().filter(|l| !l.is_empty()).collect();
    lines.push(line);
    let skip = lines.len().saturating_sub(keep);
    let mut text = lines[skip..].join("\n");
    text.push('\n');
    let tmp = path.with_extension("log.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Append the summary line to `path` ([`append_capped`]). A failure is a
/// warn line only: the game is recorded either way.
pub(crate) fn write_summary(path: &std::path::Path, session_id: &str, summary: &PregameSummary) {
    let line = summary_line(chrono::Utc::now(), session_id, summary);
    if let Err(e) = append_capped(path, &line, SUMMARY_KEEP) {
        tracing::warn!(
            target: "scuffed_stat_tracker",
            error = %e,
            path = %path.display(),
            "pre-game summary not written to the local file"
        );
    }
}

fn reason_text(summary: &PregameSummary) -> String {
    summary
        .reason
        .as_ref()
        .map_or_else(|| "none".to_string(), ToString::to_string)
}

/// Log `summary` for `session_id`, one info line.
pub(crate) fn log_summary(session_id: &str, summary: &PregameSummary) {
    let reason = reason_text(summary);
    tracing::info!(
        target: "scuffed_stat_tracker",
        session = session_id,
        hero_select_seen = summary.select_seen,
        hero_select_frame = summary.select_frame,
        picks_stored = summary.picks_stored,
        focus_paused = summary.focus_paused,
        reason = %reason,
        "pre-game summary for the recorded game"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::pregame_dump::PregameShot;

    fn select_save(name: &'static str) -> Vec<PregameOp<()>> {
        vec![PregameOp::Save(PregameShot {
            folder: "20261104_200000".into(),
            name,
            frame: (),
        })]
    }

    fn ok() -> ReaderResult {
        ReaderResult::Status(BansPicksStatus::Ok)
    }

    #[test]
    fn focus_paused_through_hero_select() {
        let mut trail = PregameTrail::default();
        trail.screen(Seen::Phase(PregamePhase::MapVote));
        trail.screen(Seen::Phase(PregamePhase::HeroBan));
        // Tabbed out: no more ticks are read, the pre-game was up.
        trail.focus_paused(true);
        trail.focus_paused(true);
        let reader = ReaderResult::Status(BansPicksStatus::Partial {
            missing: vec![BansPicksMissing::HeroSelect],
        });
        let summary = trail.finish("s1", &reader).expect("first record");
        assert!(!summary.select_seen);
        assert!(!summary.select_frame);
        assert!(!summary.picks_stored);
        assert_eq!(
            summary.reason,
            Some(MissReason::FocusPaused { in_pregame: true })
        );
    }

    #[test]
    fn focus_paused_before_any_pregame_screen() {
        let mut trail = PregameTrail::default();
        trail.focus_paused(false);
        let summary = trail.summary(&ReaderResult::NoFrames("no frames"));
        assert_eq!(
            summary.reason,
            Some(MissReason::FocusPaused { in_pregame: false })
        );
    }

    #[test]
    fn hero_select_never_detected() {
        let mut trail = PregameTrail::default();
        trail.screen(Seen::Phase(PregamePhase::HeroBan));
        trail.screen(Seen::BanResults);
        trail.screen(Seen::Loading);
        let summary = trail.summary(&ReaderResult::Status(BansPicksStatus::Partial {
            missing: vec![BansPicksMissing::HeroSelect],
        }));
        assert!(!summary.select_seen);
        assert!(!summary.picks_stored);
        assert_eq!(summary.reason, Some(MissReason::SelectNeverDetected));
    }

    #[test]
    fn detected_and_stored() {
        let mut trail = PregameTrail::default();
        trail.screen(Seen::Phase(PregamePhase::HeroSelect));
        trail.shots(&select_save(PregamePhase::HeroSelect.first_name()));
        trail.shots(&select_save(PregamePhase::HeroSelect.last_name()));
        let summary = trail.finish("s1", &ok()).expect("first record");
        assert_eq!(
            summary,
            PregameSummary {
                select_seen: true,
                select_frame: true,
                picks_stored: true,
                focus_paused: false,
                reason: None,
            }
        );
    }

    #[test]
    fn a_focus_pause_does_not_hide_stored_picks() {
        let mut trail = PregameTrail::default();
        trail.focus_paused(true);
        trail.screen(Seen::SelectTitle);
        trail.shots(&select_save(PregamePhase::HeroSelect.last_name()));
        let summary = trail.summary(&ok());
        assert_eq!(summary.reason, None);
        assert!(summary.focus_paused);
    }

    #[test]
    fn select_seen_with_no_frame_kept_gives_the_skip_reason() {
        let mut trail = PregameTrail::default();
        trail.screen(Seen::Phase(PregamePhase::HeroSelect));
        trail.select_skipped("no open game folder");
        let summary = trail.summary(&ReaderResult::NoFrames("no frames"));
        assert_eq!(
            summary.reason,
            Some(MissReason::SelectNotKept("no open game folder"))
        );
    }

    #[test]
    fn a_kept_frame_the_reader_skipped_gives_the_reader_reason() {
        let mut trail = PregameTrail::default();
        trail.screen(Seen::Phase(PregamePhase::HeroSelect));
        trail.shots(&select_save(PregamePhase::HeroSelect.last_name()));
        let summary = trail.summary(&ReaderResult::Status(BansPicksStatus::FrameSize {
            width: 1920,
            height: 1080,
        }));
        assert_eq!(
            summary.reason,
            Some(MissReason::ReaderSkipped(
                "wrong resolution 1920x1080".into()
            ))
        );
        let off = trail.summary(&ReaderResult::Off);
        assert_eq!(
            off.reason,
            Some(MissReason::ReaderSkipped("reader off".into()))
        );
    }

    #[test]
    fn one_summary_per_session_and_the_record_starts_over() {
        let mut trail = PregameTrail::default();
        trail.screen(Seen::Phase(PregamePhase::HeroSelect));
        assert!(trail.finish("s1", &ok()).is_some());
        assert!(trail.finish("s1", &ok()).is_none());
        let next = trail.finish("s2", &ReaderResult::Off).expect("next game");
        assert!(!next.select_seen);
        assert_eq!(next.reason, Some(MissReason::SelectNeverDetected));
    }

    #[test]
    fn a_new_game_folder_drops_the_last_attempt() {
        let mut trail = PregameTrail::default();
        trail.focus_paused(true);
        trail.screen(Seen::Phase(PregamePhase::HeroSelect));
        trail.new_game();
        let summary = trail.summary(&ReaderResult::Off);
        assert!(!summary.select_seen);
        assert_eq!(summary.reason, Some(MissReason::SelectNeverDetected));
    }

    #[test]
    fn the_summary_file_keeps_the_last_lines_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SUMMARY_FILE);
        for i in 0..7 {
            append_capped(&path, &format!("line {i}"), 5).expect("append");
        }
        let text = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines, ["line 2", "line 3", "line 4", "line 5", "line 6"]);
        assert!(!path.with_extension("log.tmp").exists());
    }

    #[test]
    fn the_summary_file_is_capped_at_its_keep() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SUMMARY_FILE);
        for i in 0..SUMMARY_KEEP + 3 {
            append_capped(&path, &format!("line {i}"), SUMMARY_KEEP).expect("append");
        }
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(text.lines().count(), SUMMARY_KEEP);
        assert_eq!(text.lines().next(), Some("line 3"));
    }

    #[test]
    fn a_summary_write_that_fails_does_not_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A directory where the file should be: the read fails.
        let path = dir.path().join(SUMMARY_FILE);
        std::fs::create_dir(&path).expect("mkdir");
        let summary = PregameTrail::default().summary(&ReaderResult::Off);
        assert!(append_capped(&path, "x", SUMMARY_KEEP).is_err());
        write_summary(&path, "s1", &summary);
    }

    #[test]
    fn the_summary_line_is_one_line() {
        let mut trail = PregameTrail::default();
        trail.focus_paused(true);
        let summary = trail.summary(&ReaderResult::Off);
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-04T20:00:00Z")
            .expect("time")
            .with_timezone(&chrono::Utc);
        assert_eq!(
            summary_line(at, "s1", &summary),
            "2026-11-04T20:00:00Z session=s1 hero_select_seen=false hero_select_frame=false \
             picks_stored=false focus_paused=true reason=focus gate paused the poll during the pre-game"
        );
    }

    #[test]
    fn reasons_read_without_em_dashes() {
        for reason in [
            MissReason::FocusPaused { in_pregame: true },
            MissReason::FocusPaused { in_pregame: false },
            MissReason::SelectNeverDetected,
            MissReason::SelectNotKept("x"),
            MissReason::ReaderSkipped("y".into()),
        ] {
            assert!(!reason.to_string().contains('\u{2014}'));
        }
    }
}
