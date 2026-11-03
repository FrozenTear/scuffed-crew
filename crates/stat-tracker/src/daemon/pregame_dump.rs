//! Opt-in pre-game screen saves (`debug_save_pregame_screens`): full-size
//! PNGs of the map vote, hero ban, ban results, roster, hero select and
//! loading screens under `{data_dir}/debug/pregame/<YYYYMMDD_HHMMSS>/` (UTC), one
//! folder per game, newest [`PREGAME_KEEP_GAMES`] kept. Training data for
//! later ban and hero reading; local only, nothing reads, uploads or syncs
//! them.
//!
//! The poll tick classifies its frame ([`Seen`]) on the blocking thread and
//! hands it to [`PregameTracker::observe`], which decides what to write.
//! Writes happen only when a phase starts or ends, on one writer thread
//! ([`PregameWriter`]) fed by a bounded queue. The tracker holds at most
//! one frame: the latest of the phase on screen (to write as that phase's
//! last frame when the phase changes), the latest ban-results frame until
//! the next screen shows, or the latest loading frame until the game
//! starts. A ban-results frame that may still be saved later is staged on
//! disk instead ([`PregameOp::Stage`]: a hidden `.<name>.staged` file in
//! the game's folder, replaced by each later stage), then renamed to its
//! name or deleted. With the switch off no tracker exists.
//!
//! The loading screen: the latest frame that passes the loading check is
//! held in memory, replacing the one before (nothing is written for it
//! yet), and saved once, as an ordinary save, at the next phase read in an
//! open game with no loading frame yet ([`pending`]): so an `APPLYING
//! UPDATE` band that passes the check is replaced by the `ENTERING GAME`
//! screen after it, even with a frame between that fails the check. It is
//! held with no game open too, and survives a map vote after it and a slow
//! game start. With no game folder within [`PENDING_WINDOW`], or at a
//! release (the game closed, capture stopped, a clean shutdown), it is
//! dropped and nothing is written. A tracker killed before the next phase
//! read loses it; the game's other screens are not affected.
//!
//! The roster screen ([`roster`]) is saved apart from the others: its first
//! frame as [`ROSTER_NAME`], its last as [`ROSTER_LAST_NAME`] (staged on
//! disk, nothing held), so the loading and ban-results frames never stand
//! in for it, nor it for them. It is accepted after [`roster_samples`]
//! roster samples in a row, and one seen before any game folder is staged
//! in the pending slot ([`pending`]) until the next folder opens. Each check's turned-down candidates are
//! logged and the last one kept as `rejected_<label>.png` ([`rejects`]).
//!
//! Ban results are no phase of their own yet. The candidate is the first
//! frame at least [`BAN_RESULTS_DELAY`] after the last ban-phase frame (in
//! practice the next poll) that shows the ban-results title
//! ([`super::pregame_ban_title`]: a shape rule, or example screenshots on
//! this PC when there are any, which replace it). The ban icons and scores
//! fill in below the title while the screen stays up, so each later frame
//! with the title replaces the held one. That later frame confirms the
//! held one, which is staged as the frame owed a write (replacing an
//! earlier staged one); the newest frame waits for its own confirmation.
//! Only the newest frame that is confirmed when the results end is saved,
//! once per game: the held one when it is confirmed, the staged one
//! otherwise. Either is only saved within [`BAN_RESULTS_WINDOW`] plus one
//! poll interval of the first candidate frame.
//!
//! What confirms the held frame: a frame that shows the hero select's
//! title on the very next tick, with no plain frame or skipped tick
//! between. For a first candidate, also any later non-phase frame other
//! than a select title (it is not the next screen's first frame read one
//! poll early). For a replacement, also a later frame with the results
//! title, but no plain frame: the replacement may be the select sliding
//! in under a results title, and the plain frame the select's own first
//! frame. So on the roster path (results, roster, select) the frame
//! before the last results frame is the one saved. A release (no next
//! screen) confirms nothing, so a lone first candidate is not saved then.
//!
//! The phase read lags a new screen by one poll, so a held frame right
//! before the hero select read is the select's first frame and is not
//! saved (the staged one is, if any): a lone first candidate right before
//! the select read is dropped. A frame that shows the hero select's title
//! (the words-v1 mask, read without the stability wait, and only with
//! `reader = "new"`, which loads the mask) is the select too: it never
//! becomes a candidate, and it ends the wait, which covers a select read
//! that lags two polls or more. It confirms the held frame when it comes
//! on the very next tick: the title is read with no lag, and the held
//! frame did not show it, so that frame was still the results. Every
//! ban-results frame is dropped (a staged one deleted) when the ban comes
//! back, and when that window runs out before the next screen reads. Only
//! a map vote or a ban opens a game folder; a hero select alone is the one
//! between rounds.

use crate::*;
use std::time::Duration;

mod bans;
mod classify;
mod folders;
mod pending;
mod post;
mod rejects;
mod roster;
#[cfg(test)]
mod tests_bans;
mod writer;

pub(crate) use classify::*;
pub(crate) use folders::*;
pub(crate) use pending::*;
pub(crate) use rejects::*;
use roster::RosterOnScreen;
pub(crate) use writer::PregameWriter;

/// Game folders kept under `debug/pregame/`; older ones are deleted.
pub(crate) const PREGAME_KEEP_GAMES: usize = 20;

/// Game folders kept that hold only turned-down candidates
/// (`rejected_<label>.png`): they take no slot of [`PREGAME_KEEP_GAMES`],
/// so they never push a real game out, and are capped on their own.
pub(crate) const PREGAME_KEEP_REJECTED_ONLY: usize = 5;

/// Pre-game events further apart than this belong to different games. The
/// vote, ban and select follow each other within seconds.
pub(crate) const PREGAME_GAME_GAP: Duration = Duration::from_secs(3 * 60);

/// The ban results animate in after the ban vote; the candidate is taken at
/// least this long after the last ban-phase frame. Polls are 1.5 s apart
/// (4 s with ocr-v1), so this is the next poll.
pub(crate) const BAN_RESULTS_DELAY: Duration = Duration::from_millis(1_000);

/// The wait for a candidate ends this long plus one poll interval after the
/// last ban-phase frame, and a candidate not settled this long plus one
/// poll interval after the first candidate frame is dropped (the ban read
/// was lost: ESC, alt-tab). A replacement does not extend it. Measured
/// from the first candidate, and with the poll added so a next screen
/// read 13 to 14 s after the results at 4 s polls (19 s then) still
/// settles it.
pub(crate) const BAN_RESULTS_WINDOW: Duration = Duration::from_secs(15);

/// A phase gone for at most this long and back is a detector flicker (the
/// phase read waits for the screen to hold still). Longer, and it is a new
/// screen: a requeue's map vote, or the hero select between rounds.
pub(crate) const PHASE_FLICKER: Duration = Duration::from_secs(12);

/// Batches of shots waiting for the writer. A full queue drops the whole
/// batch (logged), so frames pinned by pending writes stay bounded. A
/// dropped save is a screen lost, and still counts as saved (the loading
/// screen too: it is not looked for again in that game). A dropped stage
/// (ban results only) leaves the file staged before it, if any, so the
/// saver goes back to what that file is ([`PregameTracker::stage_lost`]).
/// A dropped promote or discard leaves its staged file in place, unsaved,
/// until a later stage of the same name replaces it or the sweep at the
/// next start or the next game folder deletes it.
pub(crate) const PREGAME_QUEUE: usize = 8;

/// The most the shutdown waits for the writer to finish.
pub(crate) const PREGAME_SHUTDOWN_CAP: Duration = Duration::from_secs(5);

/// Folder name format, in UTC so a daylight-saving change keeps name order
/// as age order.
const FOLDER_FORMAT: &str = "%Y%m%d_%H%M%S";

/// What a poll tick saw, for the pre-game saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seen {
    Phase(PregamePhase),
    /// The loading screen ([`detect::loading_screen::is_entering_game`]:
    /// the loading check plus where the `ENTERING GAME` text sits).
    Loading,
    /// No phase read yet, but the hero select's title is on the frame: the
    /// select before its read settles.
    SelectTitle,
    /// No phase, and the ban-results title is on the frame.
    BanResults,
    /// No phase, and the roster screen's layout is on the frame
    /// ([`detect::roster_screen`]).
    Roster,
    /// Anything else.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PregamePhase {
    MapVote,
    HeroBan,
    HeroSelect,
}

impl PregamePhase {
    fn of(phase: &detect::GamePhase) -> Option<Self> {
        match phase {
            detect::GamePhase::MapVote { .. } => Some(Self::MapVote),
            detect::GamePhase::HeroBan => Some(Self::HeroBan),
            detect::GamePhase::HeroSelect => Some(Self::HeroSelect),
            _ => None,
        }
    }

    pub(crate) fn first_name(self) -> &'static str {
        match self {
            Self::MapVote => "mapvote_first.png",
            Self::HeroBan => "heroban_first.png",
            Self::HeroSelect => "heroselect_first.png",
        }
    }

    pub(crate) fn last_name(self) -> &'static str {
        match self {
            Self::MapVote => "mapvote_last.png",
            Self::HeroBan => "heroban_last.png",
            Self::HeroSelect => "heroselect_last.png",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::MapVote => 0,
            Self::HeroBan => 1,
            Self::HeroSelect => 2,
        }
    }

    fn bit(self) -> u8 {
        1 << self.index()
    }
}

pub(crate) const BAN_RESULTS_NAME: &str = "ban_results.png";
/// Saved once, when the game starts; until then the frame is held in
/// memory only (never staged on disk).
pub(crate) const LOADING_NAME: &str = "loading.png";
/// The roster screen's first frame, saved when it shows.
pub(crate) const ROSTER_NAME: &str = "roster.png";
/// The roster screen's latest frame: staged on disk while the screen stays
/// up, saved (renamed) when it ends.
pub(crate) const ROSTER_LAST_NAME: &str = "roster_last.png";

/// A roster frame this long after the last one, with none between, ends
/// the screen (a phase read ends it at once). Shorter, a frame that is no
/// roster is a flicker of the layout check.
pub(crate) const ROSTER_GONE: Duration = Duration::from_secs(4);

/// Which frames the next tick should hand over, so a tick the tracker has
/// no use for runs no loading check and keeps no frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PregameWant {
    /// Run the loading-screen check on a frame that is no pre-game phase.
    pub(crate) loading: bool,
    /// Hand over a loading-screen or ban-results frame.
    pub(crate) other_frame: bool,
    /// Look for the hero select's title on a frame that is no phase (the
    /// ban-results wait is on).
    pub(crate) select_title: bool,
    /// Look for the ban-results title on a frame that is no phase (the
    /// wait is on; a frame showing it replaces a held candidate).
    pub(crate) ban_title: bool,
    /// Run the roster layout check on a frame that is no phase, and hand
    /// over a frame that passes it.
    pub(crate) roster: bool,
    /// [`RejectLabel::bit`]s whose `rejected_<label>.png` may be written
    /// now: a tick that turns down a candidate of one hands its frame over.
    pub(crate) rejected: u8,
}

/// One file to write: `<root>/<folder>/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PregameShot<F> {
    pub(crate) folder: String,
    pub(crate) name: &'static str,
    pub(crate) frame: F,
}

/// One step for the writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PregameOp<F> {
    /// Write the frame as its name.
    Save(PregameShot<F>),
    /// Write the frame as the staged copy of its name ([`staged_name`]),
    /// replacing an earlier one: not saved yet, so the tracker need not
    /// hold it. Ban results only: the loading frame is held in memory and
    /// saved with [`PregameOp::Save`].
    Stage(PregameShot<F>),
    /// Rename the staged copy of `name` to `name`.
    Promote { folder: String, name: &'static str },
    /// Delete the staged copy of `name`.
    Discard { folder: String, name: &'static str },
    /// Move the staged copy of `name` in [`PENDING_DIR`] into `folder` as
    /// `name` (the pending slot's roster frame).
    Adopt { folder: String, name: &'static str },
}

/// The hidden name a frame of `name` is staged under: no game folder, no
/// `.png`, and swept away at start and when a new game folder is made.
pub(crate) fn staged_name(name: &str) -> String {
    format!(".{name}.staged")
}

struct PregameGame {
    folder: String,
    last_event: Instant,
    /// [`PregamePhase::bit`]s whose first frame is written.
    firsts: u8,
    /// When each phase was last on screen, by [`PregamePhase::index`].
    last_seen: [Option<Instant>; 3],
    loading_saved: bool,
    ban_results_saved: bool,
    /// The roster screen showed and ended: not looked for again.
    roster_done: bool,
}

impl PregameGame {
    /// A phase seen again this long after it went away is no flicker: a
    /// requeue's map vote, or the hero select between rounds.
    fn came_back_late(&self, phase: PregamePhase, now: Instant) -> bool {
        self.last_seen[phase.index()]
            .is_some_and(|at| now.saturating_duration_since(at) > PHASE_FLICKER)
    }
}

struct OnScreen<F> {
    phase: PregamePhase,
    /// The latest frame of the phase.
    last: F,
    /// `last` is not the frame already written as the phase's first.
    last_is_new: bool,
}

/// The ban-results candidate: the latest frame with the title (the one
/// frame held), and where the confirmed frame before it is staged.
struct BanResults<F> {
    frame: F,
    /// When the first candidate frame was taken: the window runs from
    /// there, and a replacement keeps it.
    taken_at: Instant,
    /// `frame` is not the select's first frame: a later non-phase frame
    /// other than a select title came after a first candidate, or a select
    /// title on the very next tick.
    confirmed: bool,
    /// `frame` replaced an earlier candidate, so a plain later frame does
    /// not confirm it.
    replaced: bool,
    /// A plain frame came right after the replacement `frame`: a select
    /// title after that no longer confirms it.
    passed: bool,
    /// When `frame` was taken: a select title confirms it only on the
    /// next tick.
    frame_at: Instant,
    /// The game folder where the frame before `frame`, confirmed by
    /// `frame` (a later frame with the title), is staged: saved when
    /// `frame` is not confirmed by then, deleted otherwise.
    owed: Option<String>,
    /// `owed` before the latest stage: what is on disk when that stage's
    /// batch is dropped (an earlier confirmed frame, or nothing of this
    /// candidate).
    owed_before: Option<String>,
}

/// Decides which pre-game frames to write. Generic over the frame so tests
/// use plain values. It holds the latest frame of the phase on screen, the
/// ban-results candidate, or the loading frame (never two: no phase is on
/// screen while a candidate waits, a loading pass ends the results wait
/// and the phase on screen, and the next phase read saves or drops the
/// loading frame before it is held).
pub(crate) struct PregameTracker<F> {
    game: Option<PregameGame>,
    on_screen: Option<OnScreen<F>>,
    /// The loading frame waiting for a game folder ([`pending`]).
    loading: Option<HeldLoading<F>>,
    /// When the roster frame staged in [`PENDING_DIR`] was taken.
    pending_roster: Option<Instant>,
    /// Roster samples in a row ([`roster_samples`]), and when the last
    /// one was taken.
    roster_streak: u8,
    roster_sample_at: Option<Instant>,
    /// The configured poll interval, which fills the gaps not measured yet,
    /// and the last few gaps between fed ticks: their median decides
    /// [`roster_samples`] ([`TickGaps`]).
    poll: Duration,
    ticks: TickGaps,
    /// The last scoreboard read of a match that may be in progress, and
    /// the last end signal or pre-game phase read ([`pending`]).
    match_read_at: Option<Instant>,
    match_end_at: Option<Instant>,
    /// When the ban phase was last seen, while a ban-results frame may
    /// still come.
    ban_ended_at: Option<Instant>,
    /// Settled when the next screen shows (the held frame if confirmed,
    /// the staged one otherwise); dropped when the ban comes back (it was
    /// a flicker, not the results) or the window runs out.
    ban_results: Option<BanResults<F>>,
    /// The roster screen, while it shows.
    roster: Option<RosterOnScreen>,
    /// Rate limits for the turned-down candidates' lines and frames.
    rejects: RejectLimiter,
    /// [`BAN_RESULTS_WINDOW`] plus one poll interval.
    window: Duration,
    /// Longest gap after a candidate frame for a select title to count as
    /// the very next tick: one and a half poll intervals, so a skipped tick
    /// (Tab busy, a capture miss) is not mistaken for none.
    next_tick: Duration,
}

impl<F> PregameTracker<F> {
    /// A tracker for a poll loop that polls every `poll`.
    pub(crate) fn new(poll: Duration) -> Self {
        Self {
            game: None,
            on_screen: None,
            loading: None,
            pending_roster: None,
            roster_streak: 0,
            roster_sample_at: None,
            poll,
            ticks: TickGaps::default(),
            match_read_at: None,
            match_end_at: None,
            ban_ended_at: None,
            ban_results: None,
            roster: None,
            rejects: RejectLimiter::default(),
            window: BAN_RESULTS_WINDOW + poll,
            next_tick: poll + poll / 2,
        }
    }
}

impl<F: Clone> PregameTracker<F> {
    fn game_open(&self, now: Instant) -> Option<&PregameGame> {
        self.game
            .as_ref()
            .filter(|g| now.saturating_duration_since(g.last_event) <= PREGAME_GAME_GAP)
    }

    /// Whether the tracker holds a frame (for tests of the frame bound).
    #[cfg(test)]
    pub(crate) fn holds_frame(&self) -> bool {
        self.on_screen.is_some() || self.ban_results.is_some() || self.loading.is_some()
    }

    /// Whether a loading frame is held, waiting for the game to start
    /// (tests).
    #[cfg(test)]
    pub(crate) fn holds_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// The newest game's folder and when its last pre-game screen showed,
    /// for the post-game saves.
    pub(crate) fn latest_game(&self) -> Option<(&str, Instant)> {
        self.game
            .as_ref()
            .map(|g| (g.folder.as_str(), g.last_event))
    }

    /// Whether a ban-results frame is staged, owed a write (tests).
    #[cfg(test)]
    pub(crate) fn owes_frame(&self) -> bool {
        self.ban_results.as_ref().is_some_and(|c| c.owed.is_some())
    }

    /// What the tick at `now` should hand over.
    pub(crate) fn want(&self, now: Instant) -> PregameWant {
        let Some(game) = self.game_open(now) else {
            // No game folder: the loading screen and the roster may come
            // before the one that opens, for the pending slot; not while a
            // match is in progress.
            if self.match_running() {
                return PregameWant::default();
            }
            return PregameWant {
                loading: true,
                other_frame: true,
                roster: true,
                ..PregameWant::default()
            };
        };
        let loading = !game.loading_saved;
        // From the last ban tick on: the tick after it may be the results.
        let ban_on_screen = self
            .on_screen
            .as_ref()
            .is_some_and(|s| s.phase == PregamePhase::HeroBan);
        let waiting = !game.ban_results_saved && (ban_on_screen || self.ban_ended_at.is_some());
        // The roster comes before the hero select: once the select is read
        // in this game it is not looked for.
        let roster =
            !game.roster_done && game.last_seen[PregamePhase::HeroSelect.index()].is_none();
        PregameWant {
            loading,
            other_frame: loading || waiting,
            select_title: waiting,
            ban_title: waiting,
            roster,
            // Only for the checks that run on the tick.
            rejected: self.rejects.due_bits(now)
                & (if loading {
                    RejectLabel::Loading.bit()
                } else {
                    0
                } | if waiting {
                    RejectLabel::BanResults.bit()
                } else {
                    0
                } | if roster { RejectLabel::Roster.bit() } else { 0 }),
        }
    }

    /// Feed one poll tick. `frame` is `None` when [`keeps_frame`] said the
    /// tick need not keep it. `new_folder` names a new game's folder.
    #[cfg(test)]
    pub(crate) fn observe(
        &mut self,
        now: Instant,
        seen: Seen,
        frame: Option<F>,
        new_folder: impl FnOnce() -> String,
    ) -> Vec<PregameOp<F>> {
        self.observe_checked(now, seen.into(), frame, new_folder)
    }

    /// [`Self::observe`] with the checks the tick turned down: each is
    /// logged at debug and its frame kept as `rejected_<label>.png`, both
    /// rate limited ([`RejectLimiter`]).
    pub(crate) fn observe_checked(
        &mut self,
        now: Instant,
        checked: Classified,
        frame: Option<F>,
        new_folder: impl FnOnce() -> String,
    ) -> Vec<PregameOp<F>> {
        let seen = checked.seen;
        let rejected_frame = if checked.rejects.labels().next().is_some() {
            frame.clone()
        } else {
            None
        };
        // A roster frame goes to the roster only: to the rest it is a plain
        // frame, as before the roster was looked for.
        let (frame, roster_frame) = if seen == Seen::Roster {
            (None, frame)
        } else {
            (frame, None)
        };
        let mut shots = self.observe_seen(now, seen, frame, roster_frame, new_folder);
        self.note_rejects(now, &checked.rejects, rejected_frame, &mut shots);
        shots
    }

    fn observe_seen(
        &mut self,
        now: Instant,
        seen: Seen,
        frame: Option<F>,
        roster_frame: Option<F>,
        new_folder: impl FnOnce() -> String,
    ) -> Vec<PregameOp<F>> {
        let mut shots = Vec::new();
        if matches!(seen, Seen::Phase(_)) {
            // A pre-game read: no match is in progress now.
            self.match_end_at = Some(now);
        }
        self.expire_pending(now, &mut shots);
        let accepted = self.roster_sample(now, seen);
        self.observe_roster(now, seen, accepted, roster_frame, &mut shots);
        match seen {
            Seen::Phase(phase) => self.observe_phase(now, phase, frame, new_folder, &mut shots),
            Seen::Loading | Seen::Other | Seen::BanResults => {
                self.observe_other(now, seen, frame, &mut shots)
            }
            Seen::Roster => self.observe_other(now, Seen::Other, None, &mut shots),
            Seen::SelectTitle => {
                // The select is on screen: the results, if any, end here.
                // A candidate on the tick right before it (no plain frame
                // between, no tick skipped), first or replacement, is
                // confirmed: the title is read with no lag, so that frame
                // still showed the results.
                let next_tick = self.next_tick;
                if let Some(c) = self
                    .ban_results
                    .as_mut()
                    .filter(|c| !c.passed && now.saturating_duration_since(c.frame_at) <= next_tick)
                {
                    c.confirmed = true;
                }
                tracing::debug!(
                    target: "scuffed_stat_tracker",
                    candidate = self.ban_results.is_some(),
                    "hero-select title: the ban-results wait ends"
                );
                self.end_phase(now, &mut shots);
                self.settle_ban_results(now, &mut shots);
            }
        }
        shots
    }

    /// No frames for now (the game closed, capture stopped, shutdown):
    /// write the phase's last frame and a confirmed ban-results frame (an
    /// unconfirmed one is not: it may be the select sliding in, so the
    /// staged frame before it is saved, if any), and hold nothing. A held
    /// loading frame is dropped, nothing written: the game did not start
    /// after it.
    pub(crate) fn release(&mut self, now: Instant) -> Vec<PregameOp<F>> {
        let mut shots = Vec::new();
        self.end_phase(now, &mut shots);
        self.settle_ban_results(now, &mut shots);
        self.end_roster(&mut shots);
        let why = "no frames (game closed, capture stopped or shutdown)";
        self.drop_loading(now, why, &mut shots);
        self.drop_pending_roster(why, &mut shots);
        self.roster_streak = 0;
        self.roster_sample_at = None;
        self.ticks.clear();
        shots
    }

    /// A batch that staged `name` was dropped (the writer's queue was
    /// full), so the staged file on disk, if any, is an older frame. For
    /// the ban results, the owed frame goes back
    /// to what it was before that stage: the earlier confirmed frame
    /// staged for this candidate, still a results frame, or none.
    pub(crate) fn stage_lost(&mut self, name: &str) {
        if name == BAN_RESULTS_NAME
            && let Some(c) = self.ban_results.as_mut()
        {
            c.owed = c.owed_before.take();
        }
        // The pending slot's roster (the only `ROSTER_NAME` stage): what is
        // on disk may be no frame of this stretch, so nothing is attached.
        if name == ROSTER_NAME {
            self.pending_roster = None;
        }
    }

    fn observe_phase(
        &mut self,
        now: Instant,
        phase: PregamePhase,
        frame: Option<F>,
        new_folder: impl FnOnce() -> String,
        shots: &mut Vec<PregameOp<F>>,
    ) {
        if phase == PregamePhase::HeroBan {
            // The ban is back, late or not: the candidate was a frame
            // inside it, not the results.
            self.drop_ban_results(now, "the hero ban came back", shots);
        }
        if self.on_screen.as_ref().is_some_and(|s| s.phase != phase) {
            self.end_phase(now, shots);
        }
        if phase != PregamePhase::HeroBan {
            // The next screen: the ban results, if any, end here.
            self.settle_ban_results(now, shots);
        }
        let continues = self.on_screen.is_some();
        let new_game = match self.game_open(now) {
            // Only the vote or the ban starts a game: a lone hero select is
            // the one between rounds.
            None => phase != PregamePhase::HeroSelect,
            // A vote after a later screen, or back after a while (a
            // cancelled match and a requeue), is the next game.
            Some(g) => {
                phase == PregamePhase::MapVote
                    && !continues
                    && (g.firsts & !PregamePhase::MapVote.bit() != 0
                        || g.came_back_late(phase, now))
            }
        };
        if new_game {
            self.end_phase(now, shots);
            self.game = Some(PregameGame {
                folder: new_folder(),
                last_event: now,
                firsts: 0,
                last_seen: [None; 3],
                loading_saved: false,
                ban_results_saved: false,
                roster_done: false,
            });
        }
        let Some(game) = self.game.as_ref().filter(|_| self.game_open(now).is_some()) else {
            return;
        };
        // A phase back long after it ended (the hero select between
        // rounds, a late ban screen) belongs to no pre-game.
        if !continues && game.came_back_late(phase, now) {
            return;
        }
        // A pre-game read: a loading or roster frame seen before it
        // belongs to this game ([`pending`]).
        self.attach_pending(phase, shots);
        let Some(game) = self.game.as_mut() else {
            return;
        };
        game.last_event = now;
        game.last_seen[phase.index()] = Some(now);
        let Some(frame) = frame else {
            return;
        };
        match self.on_screen.as_mut() {
            Some(s) => {
                s.last = frame;
                s.last_is_new = true;
            }
            None if game.firsts & phase.bit() == 0 => {
                game.firsts |= phase.bit();
                shots.push(PregameOp::Save(PregameShot {
                    folder: game.folder.clone(),
                    name: phase.first_name(),
                    frame: frame.clone(),
                }));
                // Same frame: a one-tick phase writes no last.
                self.on_screen = Some(OnScreen {
                    phase,
                    last: frame,
                    last_is_new: false,
                });
            }
            // Back after a flicker: its first frame is on disk, this one
            // is a candidate last frame.
            None => {
                self.on_screen = Some(OnScreen {
                    phase,
                    last: frame,
                    last_is_new: true,
                });
            }
        }
    }

    fn observe_other(
        &mut self,
        now: Instant,
        seen: Seen,
        frame: Option<F>,
        shots: &mut Vec<PregameOp<F>>,
    ) {
        self.end_phase(now, shots);
        // This frame is no phase, so a first candidate was not the next
        // phase's first frame. A replacement may be the select sliding in
        // under the results title, and this frame the select's own first
        // frame: only a later results frame or a select title confirms it.
        if let Some(c) = self.ban_results.as_mut() {
            if c.replaced {
                c.passed = seen != Seen::BanResults;
            } else {
                c.confirmed = true;
            }
        }
        if self.game_open(now).is_none() || seen == Seen::Loading {
            self.settle_ban_results(now, shots);
        } else if self
            .ban_results
            .as_ref()
            .is_some_and(|c| now.saturating_duration_since(c.taken_at) > self.window)
        {
            // No next screen in time: the ban read was lost (ESC,
            // alt-tab), and the candidate is a frame from that gap.
            self.drop_ban_results(now, "no next screen within the window", shots);
        } else if let Some(ended) = self.ban_ended_at {
            let since = now.saturating_duration_since(ended);
            if since > self.window && self.ban_results.is_none() {
                // No results title in time: stop looking.
                self.ban_ended_at = None;
            } else if seen == Seen::BanResults
                && let Some(frame) = frame.clone()
            {
                match (self.ban_results.as_mut(), self.game.as_ref()) {
                    // Still the results, more of the bans shown: the held
                    // frame is confirmed and staged, owed a write (over an
                    // earlier staged one), and this one waits for its own
                    // confirmation.
                    (Some(c), Some(game)) => {
                        shots.push(PregameOp::Stage(PregameShot {
                            folder: game.folder.clone(),
                            name: BAN_RESULTS_NAME,
                            frame: std::mem::replace(&mut c.frame, frame),
                        }));
                        c.owed_before = c.owed.replace(game.folder.clone());
                        c.confirmed = false;
                        c.replaced = true;
                        c.passed = false;
                        c.frame_at = now;
                    }
                    (None, _) if since >= BAN_RESULTS_DELAY => {
                        self.ban_results = Some(BanResults {
                            frame,
                            taken_at: now,
                            confirmed: false,
                            replaced: false,
                            passed: false,
                            frame_at: now,
                            owed: None,
                            owed_before: None,
                        });
                    }
                    _ => {}
                }
            }
        }
        // Held in memory, replacing an earlier pass (an update band before
        // the ENTERING GAME screen can pass the check too); nothing is
        // written until a phase read in an open game ([`pending`]).
        if seen == Seen::Loading
            && let Some(frame) = frame
        {
            self.hold_loading(now, frame);
        }
    }

    /// Save the held ban-results frame when it is confirmed, the staged one
    /// otherwise, while inside the window (15 s plus one poll from the
    /// first candidate) at `now`; drop them otherwise (the staged one
    /// deleted), and stop waiting for one. Nothing else ever saves them,
    /// so a dropped candidate is never saved later (game end, shutdown).
    fn settle_ban_results(&mut self, now: Instant, shots: &mut Vec<PregameOp<F>>) {
        self.ban_ended_at = None;
        let Some(c) = self.ban_results.take() else {
            return;
        };
        let inside = now.saturating_duration_since(c.taken_at) <= self.window;
        let game = self
            .game
            .as_mut()
            .filter(|g| inside && !g.ban_results_saved);
        // The held frame, when it is not the one saved, and why.
        let turned = match (game, c.confirmed, c.owed) {
            (Some(game), true, owed) => {
                game.ban_results_saved = true;
                shots.push(PregameOp::Save(PregameShot {
                    folder: game.folder.clone(),
                    name: BAN_RESULTS_NAME,
                    frame: c.frame,
                }));
                if let Some(folder) = owed {
                    shots.push(PregameOp::Discard {
                        folder,
                        name: BAN_RESULTS_NAME,
                    });
                }
                None
            }
            (Some(game), false, Some(folder)) => {
                game.ban_results_saved = true;
                shots.push(PregameOp::Promote {
                    folder,
                    name: BAN_RESULTS_NAME,
                });
                Some((
                    c.frame,
                    "not confirmed, the staged frame before it is saved",
                ))
            }
            (Some(_), false, None) => {
                Some((c.frame, "not confirmed (may be the hero select sliding in)"))
            }
            (None, _, owed) => {
                if let Some(folder) = owed {
                    shots.push(PregameOp::Discard {
                        folder,
                        name: BAN_RESULTS_NAME,
                    });
                }
                Some((
                    c.frame,
                    if inside {
                        "already saved in this game"
                    } else {
                        "outside the window"
                    },
                ))
            }
        };
        if let Some((frame, why)) = turned {
            let detail = format!(
                "held frame not saved: {why}; {:.1} s after the first candidate",
                now.saturating_duration_since(c.taken_at).as_secs_f32()
            );
            self.turned_down(
                now,
                RejectLabel::BanResults,
                &detail,
                Some(frame),
                true,
                shots,
            );
        }
    }

    /// Drop the ban-results candidate (deleting its staged frame) and stop
    /// waiting for one. The held frame is a turned-down candidate (`why`).
    fn drop_ban_results(&mut self, now: Instant, why: &str, shots: &mut Vec<PregameOp<F>>) {
        self.ban_ended_at = None;
        let Some(c) = self.ban_results.take() else {
            return;
        };
        if let Some(folder) = c.owed {
            shots.push(PregameOp::Discard {
                folder,
                name: BAN_RESULTS_NAME,
            });
        }
        let detail = format!(
            "held frame dropped: {why}; {:.1} s after the first candidate",
            now.saturating_duration_since(c.taken_at).as_secs_f32()
        );
        self.turned_down(
            now,
            RejectLabel::BanResults,
            &detail,
            Some(c.frame),
            true,
            shots,
        );
    }

    /// The phase on screen ended: write its last frame when that is not
    /// the first one, and start the ban-results wait after a ban.
    fn end_phase(&mut self, now: Instant, shots: &mut Vec<PregameOp<F>>) {
        let Some(s) = self.on_screen.take() else {
            return;
        };
        let Some(game) = self.game.as_ref() else {
            return;
        };
        if s.phase == PregamePhase::HeroBan {
            self.ban_ended_at = game.last_seen[s.phase.index()].or(Some(now));
        }
        if s.last_is_new {
            shots.push(PregameOp::Save(PregameShot {
                folder: game.folder.clone(),
                name: s.phase.last_name(),
                frame: s.last,
            }));
        }
    }
}

pub(crate) type ShotBatch = Vec<PregameOp<Arc<image::DynamicImage>>>;

/// What the writer thread takes: shots, then an optional text file, written
/// in that order.
pub(crate) struct WriterJob {
    pub(crate) shots: ShotBatch,
    pub(crate) note: Option<NoteFile>,
}

/// The poll loop's pre-game saver: `Some` only with the switch on.
pub(crate) struct PregameDump {
    root: std::path::PathBuf,
    tracker: PregameTracker<Arc<image::DynamicImage>>,
    writer: PregameWriter,
    /// Ban-results title examples, read at start; shared with the poll's
    /// blocking read.
    examples: Arc<super::pregame_ban_title::BanTitleExamples>,
    /// The post-game saves, same switch, same folders and writer.
    post: super::postgame_dump::PostgameTracker,
    post_log: super::postgame_dump::PostLog,
    post_owner: super::postgame_dump::FolderOwner,
    /// The last pre-game tick read the loading screen.
    saw_loading: bool,
    /// When the coming tick was polled ([`Self::post_want`]).
    post_polled_at: Option<Instant>,
    /// When the latest fed tick was polled ([`Self::post_sooner_tick`]).
    post_ticked_at: Option<Instant>,
    /// The bans and picks reader's feed: sees every batch, `None` with the
    /// reader off.
    bans: Option<super::bans_picks::BansPicksFeed>,
}

impl PregameDump {
    /// `None` when `on` is false: no tracker, no thread, no checks, no
    /// files. Also `None`, logged, when the writer thread cannot start.
    /// `poll` is the poll loop's interval, added to the ban-results window.
    #[cfg(test)]
    pub(crate) fn new(on: bool, data_dir: &std::path::Path, poll: Duration) -> Option<Self> {
        Self::with_bans_picks(on, None, data_dir, poll)
    }

    /// [`Self::new`], also running for the bans and picks reader: with `bans`
    /// the saver exists with `on` false too, and then writes nothing and
    /// keeps no post-game screens.
    pub(crate) fn with_bans_picks(
        on: bool,
        bans: Option<super::bans_picks::BansPicksFeed>,
        data_dir: &std::path::Path,
        poll: Duration,
    ) -> Option<Self> {
        if !on && bans.is_none() {
            return None;
        }
        let root = pregame_dir(data_dir);
        let examples_dir = super::pregame_ban_title::ban_title_dir(data_dir);
        let examples = super::pregame_ban_title::BanTitleExamples::load_dir(&examples_dir);
        if examples.len() == 0 {
            tracing::info!(
                target: "scuffed_stat_tracker",
                dir = %examples_dir.display(),
                "pre-game screens: no ban-results examples, the ban-results title is told by its shape"
            );
        } else {
            tracing::info!(
                target: "scuffed_stat_tracker",
                examples = examples.len(),
                "pre-game screens: ban-results examples loaded"
            );
        }
        let writer = if on {
            PregameWriter::start(root.clone())
        } else {
            PregameWriter::memory()
        };
        match writer {
            Ok(writer) => Some(Self {
                root,
                tracker: PregameTracker::new(poll),
                writer,
                examples: Arc::new(examples),
                post: super::postgame_dump::PostgameTracker::new(poll),
                post_log: super::postgame_dump::PostLog::default(),
                post_owner: super::postgame_dump::FolderOwner::default(),
                saw_loading: false,
                post_polled_at: None,
                post_ticked_at: None,
                bans,
            }),
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "pre-game screens not saved: no writer thread");
                None
            }
        }
    }

    /// A saver writing through `writer` with no examples (tests).
    #[cfg(test)]
    pub(crate) fn with_writer(
        root: std::path::PathBuf,
        poll: Duration,
        writer: PregameWriter,
    ) -> Self {
        Self {
            root,
            tracker: PregameTracker::new(poll),
            writer,
            examples: Arc::default(),
            post: super::postgame_dump::PostgameTracker::new(poll),
            post_log: super::postgame_dump::PostLog::default(),
            post_owner: super::postgame_dump::FolderOwner::default(),
            saw_loading: false,
            post_polled_at: None,
            post_ticked_at: None,
            bans: None,
        }
    }

    pub(crate) fn want(&self, now: Instant) -> PregameWant {
        self.with_post_loading(self.tracker.want(now))
    }

    /// The tracker (tests).
    #[cfg(test)]
    pub(crate) fn tracker(&self) -> &PregameTracker<Arc<image::DynamicImage>> {
        &self.tracker
    }

    /// The ban-results title examples, for the poll's blocking read.
    pub(crate) fn examples(&self) -> Arc<super::pregame_ban_title::BanTitleExamples> {
        Arc::clone(&self.examples)
    }

    /// Feed one poll tick (what it saw, and the checks it turned down) and
    /// queue what it earned.
    pub(crate) fn observe(
        &mut self,
        now: Instant,
        seen: impl Into<Classified>,
        frame: Option<Arc<image::DynamicImage>>,
    ) {
        let mut checked: Classified = seen.into();
        self.saw_loading = checked.seen == Seen::Loading;
        if !self.tracker.want(now).loading {
            // The post-game window's own loading check: no pre-game
            // candidate.
            checked.rejects.loading = None;
        }
        checked.seen = self.pre_seen(now, checked.seen);
        let root = &self.root;
        let shots = self.tracker.observe_checked(now, checked, frame, || {
            unique_pregame_folder(root, Utc::now())
        });
        self.send(shots);
    }

    /// The game closed or capture stopped: write what is owed and hold no
    /// frame. The post-game saves end only when the game closed
    /// ([`Self::game_closed`]): a failed capture may be one poll, and the
    /// window's own time limit still ends it.
    pub(crate) fn release(&mut self, now: Instant) {
        let shots = self.tracker.release(now);
        self.send(shots);
        self.saw_loading = false;
    }

    /// Queue `shots`; when the batch is dropped, the tracker forgets each
    /// frame it staged.
    fn send(&mut self, shots: ShotBatch) {
        let staged: Vec<&'static str> = shots
            .iter()
            .filter_map(|op| match op {
                PregameOp::Stage(shot) => Some(shot.name),
                _ => None,
            })
            .collect();
        if let Some(bans) = self.bans.as_mut() {
            bans.observe(&shots);
        }
        if !self.writer.send(shots) {
            for name in staged {
                self.tracker.stage_lost(name);
            }
        }
    }

    /// Shutdown: write what is owed and wait up to `within` for the queue.
    pub(crate) async fn finish(mut self, within: Duration) -> bool {
        let now = Instant::now();
        let (post, mut sink) = self.post_sink();
        post.release(now, super::postgame_dump::PostEnd::Shutdown, &mut sink);
        self.release(now);
        // The final sidecar note waits for room, within the same deadline.
        if let Some(note) = self.post_log.pending.take() {
            let job = WriterJob {
                shots: Vec::new(),
                note: Some(note),
            };
            self.writer.send_wait(job, within).await;
        }
        self.writer
            .finish(within.saturating_sub(now.elapsed()))
            .await
    }
}

/// `debug/pregame/` left on disk while the switch is off: never deleted
/// (the user's files); startup only says it is no longer written.
pub(crate) fn stale_pregame_dir(
    on: bool,
    data_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let dir = pregame_dir(data_dir);
    (!on && dir.is_dir()).then_some(dir)
}
