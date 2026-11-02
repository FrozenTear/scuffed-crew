//! The pending slot of the pre-game saves: the latest loading frame and the
//! latest roster frame seen before the game folder they belong to opens,
//! attached to the next game folder that opens instead of dropped.
//!
//! The loading screen and the roster can both show before the map vote or
//! the ban that opens a game folder, and the game start (a hero ban or
//! hero select read) can come long after the loading screen. So:
//!
//! * the latest loading frame is held in memory (the one frame the saver
//!   already held for it, replaced by each later pass), with or without an
//!   open game, and saved as [`LOADING_NAME`] at the next pre-game phase
//!   read in an open game that has none yet: a map vote after it (one that
//!   opens a game folder too), the ban, or the select, but not a phase back
//!   long after it ended (the hero select between rounds). Saved at a map
//!   vote, it is still looked for until the game start, and a later pass
//!   is saved over it then, so a pass before the vote (an update band, the
//!   last match's screens) never shuts out the game's own ENTERING GAME. A
//!   map vote coming after it and a slow game start no longer drop it;
//! * the latest roster frame seen with no open game is staged on disk
//!   under [`PENDING_DIR`] (nothing held in memory), each later one
//!   replacing it, and moved into the next game folder as [`ROSTER_NAME`]
//!   when that folder opens ([`PregameOp::Adopt`]). A roster read later in
//!   that game is saved over it.
//!
//! The two are separate: neither replaces nor stands in for the other.
//! Both are dropped (the staged roster deleted) when no game folder opens
//! within [`PENDING_WINDOW`] of the frame, and at a release (the game
//! closed, capture stopped, a clean shutdown). The startup sweep deletes a
//! staged roster a run left behind. Once attached they are ordinary files
//! of that game folder, so they count in the folder cap
//! ([`PREGAME_KEEP_GAMES`]) like any other screen. A batch dropped by a
//! full queue that staged the roster empties the slot, so a stale staged
//! file is never attached.
//!
//! A roster is accepted only after [`ROSTER_SAMPLES`] roster samples in a
//! row at polls under [`ROSTER_ONE_SAMPLE_POLL`] apart (the new reader's
//! 1.5 s); any other sample between them starts the count again. The
//! roster stays up about 7 s, several such polls, so this costs at most
//! the first frame. At slower polls (ocr-v1's 4 s) one sample is enough:
//! two in a row would need the roster up for two polls, which it may not
//! be. The poll interval is the one actually measured, from the tick fed
//! before this one, not the configured one, so a route that polls slower
//! than configured (a screenshot route's slower floor) takes one sample.
//! With no tick before (the first after start or a release) the
//! configured interval is used.
//!
//! Neither the loading nor the roster check runs with no game folder open
//! while a match is in progress ([`PregameTracker::set_match_read`]):
//! those screens never show mid-match, and the checks would only cost time
//! and risk a false pass on gameplay. The skip is short and ends early, so
//! a missed result never costs the next game's screens: it is on only
//! within [`MATCH_SKIP_IDLE`] of the open game's last scoreboard read, and
//! it ends at once, until the next scoreboard read, on an end-of-match
//! signal ([`PregameTracker::note_match_end`]: an outcome read, a sure end
//! title, the end-reel hint) and on any pre-game phase read.

use super::*;

/// A loading or roster frame held this long with no game folder to attach
/// it to is dropped. The loading screen and the roster come within seconds
/// of the map vote; this also covers a slow vote read.
pub(crate) const PENDING_WINDOW: Duration = Duration::from_secs(3 * 60);

/// Where a roster frame seen before any game folder is staged, under the
/// pre-game root: a hidden folder, never a game folder, so the retention
/// and the per-game sweep leave it alone.
pub(crate) const PENDING_DIR: &str = ".pending";

/// The no-folder checks stay off only this long after the open game's last
/// scoreboard read with no result yet: a match in progress reads its
/// scoreboard far more often, and a result missed after the match does not
/// hold them off for long.
pub(crate) const MATCH_SKIP_IDLE: Duration = Duration::from_secs(180);

/// Consecutive roster samples before a roster frame is accepted, at polls
/// under [`ROSTER_ONE_SAMPLE_POLL`] apart.
pub(crate) const ROSTER_SAMPLES: u8 = 2;

/// At polls this far apart or more, one roster sample is accepted: the
/// roster is up only about 7 s.
pub(crate) const ROSTER_ONE_SAMPLE_POLL: Duration = Duration::from_secs(3);

/// Roster samples in a row needed at polls `poll` apart.
pub(crate) fn roster_samples(poll: Duration) -> u8 {
    if poll < ROSTER_ONE_SAMPLE_POLL {
        ROSTER_SAMPLES
    } else {
        1
    }
}

/// The latest loading-check pass, held until a game folder takes it or it
/// is dropped. Replaced by each later pass.
pub(super) struct HeldLoading<F> {
    pub(super) frame: F,
    /// When `frame` was taken.
    pub(super) at: Instant,
}

impl<F: Clone> PregameTracker<F> {
    /// Count this tick's roster sample; true when the roster is accepted
    /// ([`roster_samples`] in a row, this one included, at the interval
    /// measured since the tick fed before it, or the configured poll with
    /// none). A sample more than one and a half configured poll intervals
    /// after the last one (a capture miss or a tick not fed between them)
    /// starts the count again.
    pub(super) fn roster_sample(&mut self, now: Instant, seen: Seen) -> bool {
        let interval = self
            .last_tick_at
            .map_or(self.poll, |at| now.saturating_duration_since(at));
        self.last_tick_at = Some(now);
        if seen != Seen::Roster {
            self.roster_streak = 0;
            self.roster_sample_at = None;
            return false;
        }
        let next = self
            .roster_sample_at
            .is_some_and(|at| now.saturating_duration_since(at) <= self.next_tick);
        self.roster_streak = if next {
            self.roster_streak.saturating_add(1)
        } else {
            1
        };
        self.roster_sample_at = Some(now);
        self.roster_streak >= roster_samples(interval)
    }

    /// Drop what the pending slot held longer than [`PENDING_WINDOW`].
    pub(super) fn expire_pending(&mut self, now: Instant, shots: &mut Vec<PregameOp<F>>) {
        let old = |at: Instant| now.saturating_duration_since(at) > PENDING_WINDOW;
        if self.loading.as_ref().is_some_and(|l| old(l.at)) {
            self.drop_loading(now, "no game folder within the pending window", shots);
        }
        if self.pending_roster.is_some_and(old) {
            self.drop_pending_roster("no game folder within the pending window", shots);
        }
    }

    /// Hold an accepted loading frame: with no open game, or in an open
    /// game with no loading frame saved yet.
    pub(super) fn hold_loading(&mut self, now: Instant, frame: F) {
        let saved = self.game_open(now).is_some_and(|g| g.loading_saved);
        if !saved {
            self.loading = Some(HeldLoading { frame, at: now });
        }
    }

    /// Stage an accepted roster frame seen with no open game, replacing
    /// the one staged before.
    pub(super) fn stage_pending_roster(
        &mut self,
        now: Instant,
        frame: F,
        shots: &mut Vec<PregameOp<F>>,
    ) {
        shots.push(PregameOp::Stage(PregameShot {
            folder: PENDING_DIR.to_string(),
            name: ROSTER_NAME,
            frame,
        }));
        self.pending_roster = Some(now);
    }

    /// A pre-game `phase` read in the open game: give it what the slot
    /// holds. The loading frame is saved when the game has none yet; at a
    /// map vote the loading screen is still looked for after it, and a
    /// later pass is saved over it at the game start (a hero ban or hero
    /// select), so the frame that game keeps is the latest pass before its
    /// start. The staged roster is moved in.
    pub(super) fn attach_pending(&mut self, phase: PregamePhase, shots: &mut Vec<PregameOp<F>>) {
        let Some(game) = self.game.as_mut() else {
            return;
        };
        if !game.loading_saved
            && let Some(held) = self.loading.take()
        {
            game.loading_saved = phase != PregamePhase::MapVote;
            shots.push(PregameOp::Save(PregameShot {
                folder: game.folder.clone(),
                name: LOADING_NAME,
                frame: held.frame,
            }));
        }
        if self.pending_roster.take().is_some() {
            shots.push(PregameOp::Adopt {
                folder: game.folder.clone(),
                name: ROSTER_NAME,
            });
        }
    }

    /// Drop the held loading frame, if any; nothing is on disk for it but
    /// its turned-down copy ([`Self::turned_down`], in an open game only).
    /// The loading screen is still looked for, so a later pass is held
    /// again.
    pub(super) fn drop_loading(&mut self, now: Instant, why: &str, shots: &mut Vec<PregameOp<F>>) {
        let Some(held) = self.loading.take() else {
            return;
        };
        let detail = format!(
            "held frame dropped: {why}; held {:.1} s, a game folder must open within {} s",
            now.saturating_duration_since(held.at).as_secs_f32(),
            PENDING_WINDOW.as_secs()
        );
        self.turned_down(
            now,
            RejectLabel::Loading,
            &detail,
            Some(held.frame),
            true,
            shots,
        );
    }

    /// Delete the staged pending roster, if any.
    pub(super) fn drop_pending_roster(&mut self, why: &str, shots: &mut Vec<PregameOp<F>>) {
        if self.pending_roster.take().is_none() {
            return;
        }
        tracing::debug!(
            target: "scuffed_stat_tracker",
            why,
            "pre-game pending roster frame dropped"
        );
        shots.push(PregameOp::Discard {
            folder: PENDING_DIR.to_string(),
            name: ROSTER_NAME,
        });
    }

    /// The open game's last scoreboard read while its match may be in
    /// progress ([`match_board_read`]), `None` with none. With no game
    /// folder open the loading and roster checks are off from that read
    /// until an end signal or a pre-game phase read after it
    /// ([`Self::match_running`]).
    pub(crate) fn set_match_read(&mut self, read: Option<Instant>) {
        self.match_read_at = read;
    }

    /// An end-of-match signal at `now` (an outcome read, a sure end title,
    /// the end-reel hint): the skip ends until the next scoreboard read.
    pub(crate) fn note_match_end(&mut self, now: Instant) {
        self.match_end_at = Some(now);
    }

    /// The skip is on: a scoreboard read with no end signal or pre-game
    /// phase read since.
    pub(super) fn match_running(&self) -> bool {
        self.match_read_at
            .is_some_and(|read| self.match_end_at.is_none_or(|end| read > end))
    }

    /// Whether a roster frame is staged in the pending slot (tests).
    #[cfg(test)]
    pub(crate) fn holds_pending_roster(&self) -> bool {
        self.pending_roster.is_some()
    }
}

/// The last scoreboard read of `game` (the session's open game) when its
/// match may be in progress at `now`: no result yet, its scoreboard read
/// and stored at least once, and that read (its last activity) within
/// [`MATCH_SKIP_IDLE`]. Before its first scoreboard read a game is still in
/// its pre-game; past the window a missed result holds nothing off.
pub(crate) fn match_board_read(game: Option<&crate::ActiveGame>, now: Instant) -> Option<Instant> {
    game.filter(|g| {
        !g.finished()
            && g.session_created
            && now.saturating_duration_since(g.last_activity) <= MATCH_SKIP_IDLE
    })
    .map(|g| g.last_activity)
}

impl PregameDump {
    /// [`PregameTracker::set_match_read`], from the session state before
    /// each tick's want ([`match_board_read`]).
    pub(crate) fn set_match_read(&mut self, read: Option<Instant>) {
        self.tracker.set_match_read(read);
    }

    /// [`PregameTracker::note_match_end`].
    pub(crate) fn note_match_end(&mut self, now: Instant) {
        self.tracker.note_match_end(now);
    }
}
