//! The Length line, the LENGTH cell and their sort key: the game's real
//! length from the start and end the tracker stored, else the Tab span
//! ([`Tracked`]) as a floor, with a warning that says why. No iced here;
//! the panel draws it. Re-exported from `games_detail`.

use crate::games_detail::{Tracked, mm_ss, tracked};
use crate::model::Game;

/// The game's real start was not seen (a Tab or a stat split opened it).
pub const START_NOT_SEEN: &str = "Game start not seen";
/// The game's real end was not seen: no result read, or none fresh enough
/// to be the end. Not shown while the game is still on: a live game has no
/// end yet.
pub const END_NOT_SEEN: &str = "Game end not seen";
/// Neither the game's real start nor its end was seen. While the game is
/// still on, only the start warns ([`START_NOT_SEEN`]).
pub const START_END_NOT_SEEN: &str = "Game start and end not seen";
/// The stored end is at or before the stored start: the end was read, but
/// the two cannot be a length.
pub const END_NOT_AFTER_START: &str = "Game end read at or before its start";
/// The stored start to end is shorter than the Tab span by more than
/// [`TIMES_SLACK_SECS`]: one of them cannot be right.
pub const SHORTER_THAN_TRACKED: &str = "Game start to end is shorter than your Tab presses";
/// Added to a warning when the Length line shows the Tab span. Not added
/// when there is no span ("Length: not recorded"), so there is no length
/// for it to point at.
pub const NOT_THE_REAL_LENGTH: &str = ", so this is not the real length";

/// How much shorter than the Tab span stored times may be and still be the
/// real length: the daemon's post-match grace (75 s, a Tab in it stays on
/// the game after its result read) plus the oldest result mark it still
/// takes as the end (`MATCH_END_MARK_MAX_AGE`, 120 s before that read). So
/// a fresh end may sit up to 3:15 before the last Tab press.
pub const TIMES_SLACK_SECS: i64 = 75 + 120;

/// Why the Length is a floor on a game that has stored times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthWarning {
    StartNotSeen,
    EndNotSeen,
    StartAndEndNotSeen,
    EndNotAfterStart,
    ShorterThanTracked,
}

impl LengthWarning {
    pub fn text(self) -> &'static str {
        match self {
            LengthWarning::StartNotSeen => START_NOT_SEEN,
            LengthWarning::EndNotSeen => END_NOT_SEEN,
            LengthWarning::StartAndEndNotSeen => START_END_NOT_SEEN,
            LengthWarning::EndNotAfterStart => END_NOT_AFTER_START,
            LengthWarning::ShorterThanTracked => SHORTER_THAN_TRACKED,
        }
    }
}

/// The Length line, the LENGTH cell and their sort key. The real length
/// (end minus start) when the tracker stored both edges of the game and
/// they fit its Tab presses, else the Tab span ([`Tracked`]) as a floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameLength {
    /// End minus start, in seconds.
    Real(i64),
    /// The Tab span, the game ran at least that long. `warning` says why;
    /// `None` on a game recorded before the tracker stored start and end,
    /// which keeps the Tab span quietly.
    Floor {
        tracked: Tracked,
        warning: Option<LengthWarning>,
    },
}

pub fn game_length(g: &Game) -> GameLength {
    let tracked = tracked(g);
    let Some(times) = g.match_times else {
        return GameLength::Floor {
            tracked,
            warning: None,
        };
    };
    let warning = match (times.started_at, times.ended_at) {
        (Some(start), Some(end)) if end > start => {
            let real = (end - start).num_seconds();
            match tracked.span_secs() {
                // A safety net under the daemon's end rule: a stale end
                // that slipped through is not shown as the real length.
                Some(span) if real + TIMES_SLACK_SECS < span => LengthWarning::ShorterThanTracked,
                _ => return GameLength::Real(real),
            }
        }
        (Some(_), Some(_)) => LengthWarning::EndNotAfterStart,
        (Some(_), None) => LengthWarning::EndNotSeen,
        (None, Some(_)) => LengthWarning::StartNotSeen,
        (None, None) => LengthWarning::StartAndEndNotSeen,
    };
    GameLength::Floor {
        tracked,
        warning: Some(warning),
    }
}

impl GameLength {
    /// The real length, drawn bold like Tracked; a floor stays grey.
    pub fn is_real(&self) -> bool {
        matches!(self, GameLength::Real(_))
    }

    /// The length in seconds the LENGTH column sorts by.
    pub fn secs(&self) -> Option<i64> {
        match self {
            GameLength::Real(s) => Some(*s),
            GameLength::Floor { tracked, .. } => tracked.span_secs(),
        }
    }

    /// "Length: 12:34", or the floor: "Length: at least 9:14" or
    /// "Length: not recorded".
    pub fn length_line(&self) -> String {
        match self {
            GameLength::Real(s) => format!("Length: {}", mm_ss(*s)),
            GameLength::Floor { tracked, .. } => tracked.length_line(),
        }
    }

    /// The LENGTH cell: "12:34", "9:14+" (at least), or `None` (drawn "-").
    pub fn length_cell(&self) -> Option<String> {
        match self {
            GameLength::Real(s) => Some(mm_ss(*s)),
            GameLength::Floor { tracked, .. } => tracked.length_cell(),
        }
    }

    /// The warning under the Length line, or `None`. While the game is
    /// still on (`in_progress`) its end is not due yet, so the end not
    /// seen is not a warning; a missed start still is.
    pub fn warning(&self, in_progress: bool) -> Option<String> {
        let GameLength::Floor {
            tracked,
            warning: Some(warning),
        } = self
        else {
            return None;
        };
        let warning = match (warning, in_progress) {
            (LengthWarning::EndNotSeen, true) => return None,
            (LengthWarning::StartAndEndNotSeen, true) => LengthWarning::StartNotSeen,
            (w, _) => *w,
        };
        Some(match tracked.span_secs() {
            Some(_) => format!("{}{NOT_THE_REAL_LENGTH}", warning.text()),
            None => warning.text().to_string(),
        })
    }

    pub fn note(&self) -> Option<&'static str> {
        match self {
            GameLength::Real(_) => Some("From the game's start screen to its result"),
            GameLength::Floor { tracked, .. } => tracked.note(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games_detail::tests::{at, game, seg};
    use crate::model::GameTimes;
    use chrono::{DateTime, Utc};

    fn times(start: Option<DateTime<Utc>>, end: Option<DateTime<Utc>>) -> Option<GameTimes> {
        Some(GameTimes {
            started_at: start,
            ended_at: end,
        })
    }

    /// A game tracked 20:00:10 to 20:09:24 (Tracked 9:14).
    fn tracked_game() -> Game {
        let mut g = game("g", "Mei", 24, 1, at(20, 12, 0));
        g.segments = vec![seg(0, "Mei", 4, true, at(20, 0, 10), at(20, 9, 24))];
        g
    }

    /// Item 12: with the game's real start and end stored, the Length line
    /// and the LENGTH cell are the real length, not a floor, with no
    /// warning, live or not. Tracked stays the Tab span.
    #[test]
    fn a_stored_start_and_end_show_the_real_length() {
        let mut g = game("g", "Mei", 24, 1, at(20, 12, 0));
        g.segments = vec![seg(0, "Mei", 4, true, at(20, 2, 10), at(20, 11, 24))];
        g.match_times = times(Some(at(20, 0, 0)), Some(at(20, 12, 34)));
        let len = game_length(&g);
        assert_eq!(len, GameLength::Real(754));
        assert!(len.is_real());
        assert_eq!(len.length_line(), "Length: 12:34");
        assert_eq!(len.length_cell().as_deref(), Some("12:34"));
        assert_eq!(len.secs(), Some(754));
        assert_eq!(len.warning(false), None);
        assert_eq!(len.warning(true), None);
        assert_eq!(
            len.note(),
            Some("From the game's start screen to its result")
        );
        for shown in [len.length_line(), len.length_cell().unwrap()] {
            assert!(
                !shown.contains("at least") && !shown.contains('+'),
                "{shown}"
            );
        }
        assert_eq!(tracked(&g).line(), "Tracked 9:14");
    }

    /// A game a Tab opened (start not seen) still shows the Tab span as a
    /// floor, and says the start was not seen, live or not.
    #[test]
    fn a_missed_start_falls_back_to_at_least_and_warns() {
        let mut g = tracked_game();
        g.match_times = times(None, Some(at(20, 12, 34)));
        let len = game_length(&g);
        assert!(!len.is_real());
        assert_eq!(len.length_line(), "Length: at least 9:14");
        assert_eq!(len.length_cell().as_deref(), Some("9:14+"));
        assert_eq!(len.secs(), Some(554));
        let want = "Game start not seen, so this is not the real length";
        assert_eq!(len.warning(false).as_deref(), Some(want));
        assert_eq!(len.warning(true).as_deref(), Some(want));
        assert_eq!(
            len.note(),
            Some("From your first to last Tab press, shorter than the game")
        );
    }

    /// A finished game with no end read: the floor and the end warning. A
    /// live game has no end yet, so it does not warn about one; with no
    /// start either, only the start warns.
    #[test]
    fn a_missed_end_warns_on_a_finished_game_and_not_on_a_live_one() {
        let mut g = tracked_game();
        g.match_times = times(Some(at(20, 0, 0)), None);
        let len = game_length(&g);
        assert_eq!(len.length_line(), "Length: at least 9:14");
        assert_eq!(len.length_cell().as_deref(), Some("9:14+"));
        assert_eq!(
            len.warning(false).as_deref(),
            Some("Game end not seen, so this is not the real length")
        );
        assert_eq!(len.warning(true), None);
        g.match_times = times(None, None);
        let len = game_length(&g);
        assert_eq!(
            len.warning(false).as_deref(),
            Some("Game start and end not seen, so this is not the real length")
        );
        assert_eq!(
            len.warning(true).as_deref(),
            Some("Game start not seen, so this is not the real length")
        );
        assert_eq!(len.length_line(), "Length: at least 9:14");
    }

    /// With no span the Length line says "not
    /// recorded", so the warning does not say "this is not the real
    /// length": there is no length for "this" to point at.
    #[test]
    fn with_no_span_the_warning_has_no_length_to_point_at() {
        let mut g = game("g", "Mei", 24, 1, at(20, 0, 0));
        g.segments = vec![seg(0, "Mei", 1, true, at(20, 0, 0), at(20, 0, 0))];
        g.match_times = times(Some(at(20, 0, 0)), None);
        let len = game_length(&g);
        assert_eq!(len.length_line(), "Length: not recorded");
        assert_eq!(len.length_cell(), None);
        assert_eq!(len.warning(false).as_deref(), Some(END_NOT_SEEN));
        g.match_times = times(None, None);
        assert_eq!(
            game_length(&g).warning(false).as_deref(),
            Some(START_END_NOT_SEEN)
        );
    }

    /// An end at or before the start was seen, so it is not
    /// "end not seen". It has its own line, and still falls back.
    #[test]
    fn an_end_not_after_the_start_has_its_own_warning() {
        let mut g = tracked_game();
        for end in [at(20, 0, 0), at(19, 59, 0)] {
            g.match_times = times(Some(at(20, 0, 0)), Some(end));
            let len = game_length(&g);
            assert!(!len.is_real());
            assert_eq!(len.length_line(), "Length: at least 9:14");
            let want = "Game end read at or before its start, so this is not the real length";
            assert_eq!(len.warning(false).as_deref(), Some(want));
            assert_eq!(len.warning(true).as_deref(), Some(want));
        }
    }

    /// The safety net under the daemon's end rule: a misread VICTORY at 20:05 stored as the
    /// end of a game that started at 20:00 and was tracked to 20:10 is not
    /// shown as "Length: 5:00" with no warning. It falls back to the Tab
    /// span with a warning. Inside the slack it is still the real length.
    #[test]
    fn a_real_length_far_shorter_than_tracked_falls_back_with_a_warning() {
        let mut g = game("g", "Mei", 24, 1, at(20, 12, 0));
        g.segments = vec![seg(0, "Mei", 6, true, at(20, 1, 0), at(20, 11, 0))];
        g.match_times = times(Some(at(20, 0, 0)), Some(at(20, 5, 0)));
        let len = game_length(&g);
        assert!(!len.is_real());
        assert_ne!(len.length_line(), "Length: 5:00");
        assert_eq!(len.length_line(), "Length: at least 10:00");
        assert_eq!(len.length_cell().as_deref(), Some("10:00+"));
        assert_eq!(len.secs(), Some(600));
        let want =
            "Game start to end is shorter than your Tab presses, so this is not the real length";
        assert_eq!(len.warning(false).as_deref(), Some(want));
        assert_eq!(len.warning(true).as_deref(), Some(want));
        // 6:45 + 3:15 = 10:00: inside the slack, a real length.
        g.match_times = times(Some(at(20, 0, 0)), Some(at(20, 6, 45)));
        assert_eq!(game_length(&g), GameLength::Real(405));
        // One second shorter: outside it.
        g.match_times = times(Some(at(20, 0, 0)), Some(at(20, 6, 44)));
        assert!(!game_length(&g).is_real());
        // A fresh end the daemon takes: VICTORY first seen at 20:10:00 and
        // confirmed 115 s later, then a Tab inside the grace at 20:13:08.
        // The span is 12:18, the real length 10:00, and it stays real.
        g.segments = vec![seg(0, "Mei", 6, true, at(20, 0, 50), at(20, 13, 8))];
        g.match_times = times(Some(at(20, 0, 0)), Some(at(20, 10, 0)));
        assert_eq!(game_length(&g), GameLength::Real(600));
    }

    /// A game recorded before start and end were stored keeps today's
    /// length: the Tab span floor, or not recorded, and no warning.
    #[test]
    fn an_older_row_keeps_the_tab_span_and_no_warning() {
        let mut g = game("g", "Mei", 24, 1, at(20, 0, 0));
        assert_eq!(g.match_times, None);
        assert_eq!(game_length(&g).length_line(), "Length: not recorded");
        assert_eq!(game_length(&g).length_cell(), None);
        assert_eq!(game_length(&g).warning(false), None);
        g.segments = vec![seg(0, "Mei", 1, true, at(20, 0, 0), at(20, 0, 0))];
        assert_eq!(game_length(&g).length_line(), "Length: not recorded");
        assert_eq!(game_length(&g).length_cell(), None);
        assert_eq!(game_length(&g).warning(false), None);
        g.segments = vec![seg(0, "Mei", 4, true, at(20, 0, 10), at(20, 9, 24))];
        assert_eq!(game_length(&g).length_line(), "Length: at least 9:14");
        assert_eq!(game_length(&g).length_cell().as_deref(), Some("9:14+"));
        assert_eq!(game_length(&g).warning(false), None);
        assert!(!game_length(&g).is_real());
    }

    #[test]
    fn the_length_copy_has_no_dashes() {
        for w in [
            START_NOT_SEEN,
            END_NOT_SEEN,
            START_END_NOT_SEEN,
            END_NOT_AFTER_START,
            SHORTER_THAN_TRACKED,
            NOT_THE_REAL_LENGTH,
        ] {
            assert!(!w.contains('\u{2014}') && !w.contains('\u{2013}'), "{w}");
        }
    }
}
