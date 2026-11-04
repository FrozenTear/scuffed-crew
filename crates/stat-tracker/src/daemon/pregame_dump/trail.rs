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
#[path = "tests_trail.rs"]
mod tests;
