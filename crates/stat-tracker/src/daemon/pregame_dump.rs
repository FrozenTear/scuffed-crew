//! Opt-in pre-game screen saves (`debug_save_pregame_screens`): full-size
//! PNGs of the map vote, hero ban, ban results, hero select and loading
//! screens under `{data_dir}/debug/pregame/<YYYYMMDD_HHMMSS>/` (UTC), one
//! folder per game, newest [`PREGAME_KEEP_GAMES`] kept. Training data for
//! later ban and hero reading; local only, nothing reads, uploads or syncs
//! them.
//!
//! The poll tick classifies its frame ([`Seen`]) on the blocking thread and
//! hands it to [`PregameTracker::observe`], which decides what to write.
//! Writes happen only when a phase starts or ends, on one writer thread
//! ([`PregameWriter`]) fed by a bounded queue. The tracker holds at most
//! one frame: the latest of the phase on screen (to write as that phase's
//! last frame when the phase changes), or the latest ban-results frame
//! until the next screen shows. A frame that may still be saved later is
//! staged on disk instead ([`PregameOp::Stage`]: a hidden
//! `.<name>.staged` file in the game's folder, replaced by each later
//! stage), then renamed to its name or deleted. With the switch off no
//! tracker exists.
//!
//! The loading screen: every frame that passes the loading check is
//! staged, replacing the one before, and the last one is saved only when
//! the game starts: a hero ban or hero select read in the same game
//! within [`PHASE_FLICKER`] of it, the screen after loading. So an `APPLYING UPDATE` band that passes the check is replaced
//! by the `ENTERING GAME` screen after it, even with a frame between that
//! fails the check. A loading frame with no game start after it is
//! dropped (the staged file deleted): no start within [`PHASE_FLICKER`]
//! (the menu, a lobby), a release (the game closed, capture stopped,
//! shutdown), or a requeue's map vote opening the next game. Dropped, the
//! loading screen is still looked for, so a later `ENTERING GAME` in the
//! same game is staged and saved.
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

mod post;

/// Game folders kept under `debug/pregame/`; older ones are deleted.
pub(crate) const PREGAME_KEEP_GAMES: usize = 20;

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
/// screen: a requeue's map vote, or the hero select between rounds. The
/// loading screen gone this long with no game start is dropped.
pub(crate) const PHASE_FLICKER: Duration = Duration::from_secs(12);

/// Batches of shots waiting for the writer. A full queue drops the whole
/// batch (logged), so frames pinned by pending writes stay bounded. A
/// dropped save is a screen lost. A dropped stage leaves the file staged
/// before it, if any, so the saver forgets that staged frame
/// ([`PregameTracker::stage_lost`]) and never saves it. A dropped promote
/// or discard leaves its staged file in place, unsaved, until a later
/// stage of the same name replaces it or the sweep at the next start or
/// the next game folder deletes it.
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

    fn first_name(self) -> &'static str {
        match self {
            Self::MapVote => "mapvote_first.png",
            Self::HeroBan => "heroban_first.png",
            Self::HeroSelect => "heroselect_first.png",
        }
    }

    fn last_name(self) -> &'static str {
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
pub(crate) const LOADING_NAME: &str = "loading.png";

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
}

/// Classify a polled frame for the tracker, on the blocking thread, with
/// the process's words-v1 reader for the select title and `examples` for
/// the ban-results title.
pub(crate) fn classify(
    want: PregameWant,
    phase: &detect::GamePhase,
    rgb: &image::RgbImage,
    examples: &super::pregame_ban_title::BanTitleExamples,
) -> Seen {
    classify_with(
        want,
        phase,
        rgb,
        |rgb| {
            shows_select_title(shadow::read_phrase(
                stat_tracker::shadow::result::phrase::Phrase::SelectAPreferredHero,
                rgb,
            ))
        },
        |rgb| examples.matches(rgb),
    )
}

/// [`classify`] with the two title reads as arguments (tests).
///
/// The loading check runs only when `want.loading` and the frame is no
/// phase, and a frame passing the hero-ban pixel gate (a ban screen before
/// its read settles) is never the loading screen. The cheap loading check
/// runs first. The map-vote gate is not used: a vote screen's navy is too
/// bright to pass the loading check's dark bands, so it only turned away a
/// loading screen whose black is tinted navy. The hero-select gate is not
/// used either: its colour spread can pass on the loading screen's glyphs.
pub(crate) fn classify_with(
    want: PregameWant,
    phase: &detect::GamePhase,
    rgb: &image::RgbImage,
    select_title: impl FnOnce(&image::RgbImage) -> bool,
    ban_title: impl FnOnce(&image::RgbImage) -> bool,
) -> Seen {
    if let Some(p) = PregamePhase::of(phase) {
        return Seen::Phase(p);
    }
    if want.loading
        && detect::loading_screen::is_entering_game(rgb)
        && !detect::match_start::passes_ban_gate(rgb)
    {
        return Seen::Loading;
    }
    if want.select_title && select_title(rgb) {
        return Seen::SelectTitle;
    }
    if want.ban_title && ban_title(rgb) {
        return Seen::BanResults;
    }
    Seen::Other
}

/// Whether a words-v1 read of SELECT A PREFERRED HERO says the hero select
/// is on the frame. The mask is the pack's own cut of the select title
/// region (Vision's 0.41,0.165,0.565,0.2 at 1440p), so no new reference
/// crops are needed. Conservative on purpose: a suspect read (the title
/// half drawn while the screen slides in) counts too, since a frame that
/// is turned away here costs at most the results frame, while one let
/// through is a select frame saved as the results. No mask loaded (`None`)
/// is no title.
pub(crate) fn shows_select_title(
    read: Option<stat_tracker::shadow::result::phrase::PhraseRead>,
) -> bool {
    use stat_tracker::shadow::result::phrase::PhraseRead;
    matches!(read, Some(PhraseRead::Sure(_) | PhraseRead::Suspect(_)))
}

/// Whether a tick seeing `seen` should hand its frame to the tracker.
pub(crate) fn keeps_frame(want: PregameWant, seen: Seen) -> bool {
    match seen {
        Seen::Phase(_) => true,
        Seen::Loading | Seen::BanResults => want.other_frame,
        Seen::Other | Seen::SelectTitle => false,
    }
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
    /// hold it.
    Stage(PregameShot<F>),
    /// Rename the staged copy of `name` to `name`.
    Promote { folder: String, name: &'static str },
    /// Delete the staged copy of `name`.
    Discard { folder: String, name: &'static str },
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
    /// When the last loading-check pass was staged, until it is saved.
    loading_staged_at: Option<Instant>,
    ban_results_saved: bool,
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
/// use plain values. It holds the latest frame of the phase on screen, or
/// the ban-results candidate (never both: no phase is on screen while a
/// candidate waits).
pub(crate) struct PregameTracker<F> {
    game: Option<PregameGame>,
    on_screen: Option<OnScreen<F>>,
    /// When the ban phase was last seen, while a ban-results frame may
    /// still come.
    ban_ended_at: Option<Instant>,
    /// Settled when the next screen shows (the held frame if confirmed,
    /// the staged one otherwise); dropped when the ban comes back (it was
    /// a flicker, not the results) or the window runs out.
    ban_results: Option<BanResults<F>>,
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
            ban_ended_at: None,
            ban_results: None,
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
        self.on_screen.is_some() || self.ban_results.is_some()
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
            return PregameWant::default();
        };
        let loading = !game.loading_saved;
        // From the last ban tick on: the tick after it may be the results.
        let ban_on_screen = self
            .on_screen
            .as_ref()
            .is_some_and(|s| s.phase == PregamePhase::HeroBan);
        let waiting = !game.ban_results_saved && (ban_on_screen || self.ban_ended_at.is_some());
        PregameWant {
            loading,
            other_frame: loading || waiting,
            select_title: waiting,
            ban_title: waiting,
        }
    }

    /// Feed one poll tick. `frame` is `None` when [`keeps_frame`] said the
    /// tick need not keep it. `new_folder` names a new game's folder.
    pub(crate) fn observe(
        &mut self,
        now: Instant,
        seen: Seen,
        frame: Option<F>,
        new_folder: impl FnOnce() -> String,
    ) -> Vec<PregameOp<F>> {
        let mut shots = Vec::new();
        // The select title is no start: it is only looked for while the
        // ban results may come, and a loading frame ends that wait.
        let started = matches!(
            seen,
            Seen::Phase(PregamePhase::HeroBan | PregamePhase::HeroSelect)
        );
        self.settle_loading(now, started, &mut shots);
        match seen {
            Seen::Phase(phase) => self.observe_phase(now, phase, frame, new_folder, &mut shots),
            Seen::Loading | Seen::Other | Seen::BanResults => {
                self.observe_other(now, seen, frame, &mut shots)
            }
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
    /// staged frame before it is saved, if any), and hold nothing. A
    /// staged loading frame is deleted: the game did not start after it.
    pub(crate) fn release(&mut self, now: Instant) -> Vec<PregameOp<F>> {
        let mut shots = Vec::new();
        self.end_phase(now, &mut shots);
        self.settle_ban_results(now, &mut shots);
        self.drop_loading(&mut shots);
        shots
    }

    /// Save the staged loading frame when the game starts (`started`: a
    /// hero ban or hero select read, the screen after loading) in the game
    /// open at `now`, within [`PHASE_FLICKER`] of the last pass; saved, the
    /// loading screen is no longer looked for in this game. Delete it when
    /// no start came within [`PHASE_FLICKER`] (a start after that too), or
    /// a start came after the game closed (a hero ban then opens the next
    /// game; a lone hero select opens none). A map vote is no start: it
    /// comes before the loading screen.
    fn settle_loading(&mut self, now: Instant, started: bool, shots: &mut Vec<PregameOp<F>>) {
        let open = self.game_open(now).is_some();
        let Some(at) = self.game.as_ref().and_then(|g| g.loading_staged_at) else {
            return;
        };
        let over = now.saturating_duration_since(at) > PHASE_FLICKER;
        if started && open && !over {
            if let Some(game) = self.game.as_mut() {
                game.loading_staged_at = None;
                game.loading_saved = true;
                shots.push(PregameOp::Promote {
                    folder: game.folder.clone(),
                    name: LOADING_NAME,
                });
            }
        } else if started || over {
            self.drop_loading(shots);
        }
    }

    /// A batch that staged `name` was dropped (the writer's queue was
    /// full), so the staged file on disk, if any, is an older frame. A
    /// loading one is never saved (it may be an update band before the
    /// `ENTERING GAME` screen): nothing is written for it, and the next
    /// stage replaces it or the sweep deletes it. The loading screen is
    /// still looked for. For the ban results, the owed frame goes back to
    /// what it was before that stage: the earlier confirmed frame staged
    /// for this candidate, still a results frame, or none.
    pub(crate) fn stage_lost(&mut self, name: &str) {
        if name == LOADING_NAME {
            if let Some(game) = self.game.as_mut() {
                game.loading_staged_at = None;
            }
        } else if name == BAN_RESULTS_NAME
            && let Some(c) = self.ban_results.as_mut()
        {
            c.owed = c.owed_before.take();
        }
    }

    /// Delete the staged loading frame, if any. The loading screen is still
    /// looked for, so a later pass in this game is staged again.
    fn drop_loading(&mut self, shots: &mut Vec<PregameOp<F>>) {
        if let Some(game) = self.game.as_mut()
            && game.loading_staged_at.take().is_some()
        {
            shots.push(PregameOp::Discard {
                folder: game.folder.clone(),
                name: LOADING_NAME,
            });
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
            self.drop_ban_results(shots);
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
            // A requeue: the last game never started after its loading
            // frame.
            self.drop_loading(shots);
            self.game = Some(PregameGame {
                folder: new_folder(),
                last_event: now,
                firsts: 0,
                last_seen: [None; 3],
                loading_saved: false,
                loading_staged_at: None,
                ban_results_saved: false,
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
            self.drop_ban_results(shots);
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
        if self.game_open(now).is_none() {
            return;
        }
        let Some(game) = self.game.as_mut() else {
            return;
        };
        // Staged, replacing an earlier pass: an update band before the
        // ENTERING GAME screen can pass the check too.
        if seen == Seen::Loading
            && !game.loading_saved
            && let Some(frame) = frame
        {
            game.loading_staged_at = Some(now);
            shots.push(PregameOp::Stage(PregameShot {
                folder: game.folder.clone(),
                name: LOADING_NAME,
                frame,
            }));
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
        match (game, c.confirmed, c.owed) {
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
            }
            (Some(game), false, Some(folder)) => {
                game.ban_results_saved = true;
                shots.push(PregameOp::Promote {
                    folder,
                    name: BAN_RESULTS_NAME,
                });
            }
            (_, _, Some(folder)) => shots.push(PregameOp::Discard {
                folder,
                name: BAN_RESULTS_NAME,
            }),
            _ => {}
        }
    }

    /// Drop the ban-results candidate (deleting its staged frame) and stop
    /// waiting for one.
    fn drop_ban_results(&mut self, shots: &mut Vec<PregameOp<F>>) {
        self.ban_ended_at = None;
        if let Some(folder) = self.ban_results.take().and_then(|c| c.owed) {
            shots.push(PregameOp::Discard {
                folder,
                name: BAN_RESULTS_NAME,
            });
        }
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

/// The pre-game folder under the data dir.
pub(crate) fn pregame_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("debug").join("pregame")
}

/// The folder name for a game that starts at `at`.
pub(crate) fn pregame_folder_name(at: chrono::DateTime<Utc>) -> String {
    at.format(FOLDER_FORMAT).to_string()
}

/// A new game's folder name under `root`: the time now, a second later per
/// name already there (a clock set back can land on an old folder).
pub(crate) fn unique_pregame_folder(root: &std::path::Path, at: chrono::DateTime<Utc>) -> String {
    (0..60)
        .map(|s| pregame_folder_name(at + chrono::Duration::seconds(s)))
        .find(|name| std::fs::symlink_metadata(root.join(name)).is_err())
        .unwrap_or_else(|| pregame_folder_name(at))
}

/// Exactly a name [`pregame_folder_name`] makes: a real date and time in
/// `YYYYMMDD_HHMMSS`, nothing more. Only these are pruned; anything else in
/// `debug/pregame/` is left alone.
pub(crate) fn is_game_folder(name: &str) -> bool {
    name.len() == 15
        && chrono::NaiveDateTime::parse_from_str(name, FOLDER_FORMAT)
            .is_ok_and(|t| t.format(FOLDER_FORMAT).to_string() == name)
}

/// Game folders in the order they were made, oldest first, one per line.
/// Kept across runs so a clock set back (which gives a new folder an old
/// name) does not make the next run prune its newest games first.
const ORDER_FILE: &str = ".order";

/// The folders [`ORDER_FILE`] lists, oldest first, each once (a repeat
/// keeps its first place); empty when it is missing.
pub(crate) fn load_pregame_order(root: &std::path::Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join(ORDER_FILE)).unwrap_or_default();
    let mut made: Vec<String> = Vec::new();
    for name in text.lines().filter(|name| is_game_folder(name)) {
        if !made.iter().any(|m| m == name) {
            made.push(name.to_string());
        }
    }
    made
}

/// Write [`ORDER_FILE`] through a temporary file and a rename.
fn save_pregame_order(root: &std::path::Path, made: &[String]) {
    let tmp = root.join(format!("{ORDER_FILE}.tmp"));
    let text: String = made.iter().map(|name| format!("{name}\n")).collect();
    let saved =
        std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, root.join(ORDER_FILE)));
    if let Err(e) = saved {
        let _ = std::fs::remove_file(&tmp);
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game folder order not saved");
    }
}

/// Delete the oldest game folders under `root` so at most `keep` are left.
/// Only real directories with an exact game-folder name count: no file, no
/// symlink, never `root` itself. Age order is name order (UTC timestamps)
/// for folders not in `made`, then `made` (the folders in the order they
/// were made, oldest first) after them all: a clock set back gives a new
/// folder an old name, and it still goes last. The newest of `made` is
/// never deleted. A folder with only staged or temporary files in it
/// ([`only_leftovers`]) does not count.
pub(crate) fn prune_pregame_games(root: &std::path::Path, keep: usize, made: &[String]) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let games: Vec<String> = entries
        .flatten()
        // `DirEntry::file_type` does not follow symlinks.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| is_game_folder(name))
        .collect();
    // The game in progress: never deleted, even with nothing saved yet.
    let newest = made.iter().rev().find(|name| games.contains(name));
    // A folder holding only staged or temporary files is no saved game: it
    // takes no slot and pushes no game out ([`sweep_staged`] deletes it).
    let mut older: Vec<String> = games
        .into_iter()
        .filter(|name| !only_leftovers(&root.join(name)))
        .collect();
    // Each folder once, even if `made` repeats it.
    let mut ours: Vec<String> = Vec::new();
    for name in made {
        if older.contains(name) && !ours.contains(name) {
            ours.push(name.clone());
        }
    }
    older.retain(|name| !made.contains(name));
    older.sort();
    let total = older.len() + ours.len();
    if total <= keep {
        return;
    }
    for name in older
        .iter()
        .chain(ours.iter())
        .filter(|name| Some(*name) != newest)
        .take(total - keep)
    {
        if let Err(e) = std::fs::remove_dir_all(root.join(name)) {
            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game folder not deleted");
        }
    }
}

/// A staged frame ([`staged_name`]) or a temporary file of a write (a
/// dot name ending `.tmp`): never a saved screen.
fn is_leftover(name: &str) -> bool {
    name.starts_with('.') && (name.ends_with(".staged") || name.ends_with(".tmp"))
}

/// Whether `dir` holds files and all of them are leftovers
/// ([`is_leftover`]). An empty or unreadable folder, or one with anything
/// else in it (a saved screen, a subfolder), is not.
fn only_leftovers(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut any = false;
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        let leftover = entry.file_name().to_str().is_some_and(is_leftover)
            && entry.file_type().is_ok_and(|t| t.is_file());
        if !leftover {
            return false;
        }
        any = true;
    }
    any
}

/// Delete what a write left half done under `root`: staged frames
/// ([`staged_name`]) and temporary files, in `root` itself and in every
/// game folder but `except`, and then each of those folders it left
/// empty. Run at start and when a new game folder is made, since a staged
/// frame belongs to the game in progress only. A saved screen is never
/// touched, and a folder with one in it is never deleted.
pub(crate) fn sweep_staged(root: &std::path::Path, except: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() && is_leftover(&name) {
            // The order file's temporary copy.
            let _ = std::fs::remove_file(entry.path());
            continue;
        }
        if !kind.is_dir() || !is_game_folder(&name) || except == Some(name.as_str()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        let mut swept = false;
        for file in files.flatten() {
            let leftover = file.file_name().to_str().is_some_and(is_leftover);
            if leftover && file.file_type().is_ok_and(|t| t.is_file()) {
                swept |= std::fs::remove_file(file.path()).is_ok();
            }
        }
        if swept {
            // Only an empty folder goes: `remove_dir` fails on any other.
            let _ = std::fs::remove_dir(entry.path());
        }
    }
}

/// Write `frame` under `root` at full size as `<folder>/<file>`, to a
/// temporary file then renamed, so a crash leaves no half PNG under the
/// final name. After a new game folder (added to `made`), keep the newest
/// `keep`, save the order and sweep staged frames left in the others.
fn write_frame(
    root: &std::path::Path,
    folder: &str,
    file: &str,
    frame: &image::DynamicImage,
    keep: usize,
    made: &mut Vec<String>,
) {
    let dir = root.join(folder);
    let new_folder = !dir.is_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game folder not made");
        return;
    }
    let tmp = dir.join(format!(".{file}.tmp"));
    let saved = frame
        .save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(|e| e.to_string())
        .and_then(|()| std::fs::rename(&tmp, dir.join(file)).map_err(|e| e.to_string()));
    if let Err(e) = saved {
        let _ = std::fs::remove_file(&tmp);
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game frame not saved");
    }
    if new_folder {
        note_new_folder(root, folder, keep, made);
    }
}

/// A new game folder was made (by a pre-game or a post-game write): add it
/// to `made` as the newest, keep the newest `keep`, save the order and
/// sweep staged frames left in the others.
fn note_new_folder(root: &std::path::Path, folder: &str, keep: usize, made: &mut Vec<String>) {
    made.retain(|name| name != folder);
    made.push(folder.to_string());
    prune_pregame_games(root, keep, made);
    made.retain(|name| root.join(name).is_dir());
    save_pregame_order(root, made);
    sweep_staged(root, Some(folder));
}

/// The post-game sidecar to write: `<root>/<folder>/<name>`, replaced
/// whole each time.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NoteFile {
    pub(crate) folder: String,
    pub(crate) name: &'static str,
    pub(crate) doc: super::postgame_dump::PostSidecar,
}

/// Write `note` the way [`write_frame`] writes a PNG: a temporary file then
/// a rename, and a new game folder counts for the retention. It lists only
/// the frames on disk: those whose write failed are left out.
pub(crate) fn write_note(
    root: &std::path::Path,
    note: &NoteFile,
    keep: usize,
    made: &mut Vec<String>,
) {
    let dir = root.join(&note.folder);
    let new_folder = !dir.is_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "post-game folder not made");
        return;
    }
    let tmp = dir.join(format!(".{}.tmp", note.name));
    let text = note.doc.render(|file| dir.join(file).is_file());
    let saved =
        std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, dir.join(note.name)));
    if let Err(e) = saved {
        let _ = std::fs::remove_file(&tmp);
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "post-game sidecar not saved");
    }
    if new_folder {
        note_new_folder(root, &note.folder, keep, made);
    }
}

/// Carry out `ops` under `root`, in order: saves and stages through
/// [`write_frame`], a promote as one rename of the staged frame, a discard
/// as its delete. A stage first deletes the staged frame it replaces, so a
/// stage whose write fails leaves none, and a promote or discard with no
/// staged frame changes nothing: an older frame is never saved in place of
/// a newer one that failed to write.
pub(crate) fn write_pregame_ops(
    root: &std::path::Path,
    ops: &[PregameOp<Arc<image::DynamicImage>>],
    keep: usize,
    made: &mut Vec<String>,
) {
    for op in ops {
        match op {
            PregameOp::Save(shot) => {
                write_frame(root, &shot.folder, shot.name, &shot.frame, keep, made)
            }
            PregameOp::Stage(shot) => {
                let staged = staged_name(shot.name);
                let _ = std::fs::remove_file(root.join(&shot.folder).join(&staged));
                write_frame(root, &shot.folder, &staged, &shot.frame, keep, made)
            }
            PregameOp::Promote { folder, name } => {
                let dir = root.join(folder);
                if let Err(e) = std::fs::rename(dir.join(staged_name(name)), dir.join(name)) {
                    tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game staged frame not saved");
                }
            }
            PregameOp::Discard { folder, name } => {
                let _ = std::fs::remove_file(root.join(folder).join(staged_name(name)));
            }
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

/// One thread writes every batch in order, so two writes of the same
/// `*_last.png` never overlap. The queue is bounded by [`PREGAME_QUEUE`].
pub(crate) struct PregameWriter {
    tx: Option<std::sync::mpsc::SyncSender<WriterJob>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The thread's end was logged (once).
    dead_logged: bool,
}

impl PregameWriter {
    pub(crate) fn start(root: std::path::PathBuf) -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<WriterJob>(PREGAME_QUEUE);
        let thread = std::thread::Builder::new()
            .name("pregame-writer".into())
            .spawn(move || {
                // Game folders in the order they were made, oldest first.
                let mut made = load_pregame_order(&root);
                // Staged frames of a run that ended mid-game.
                sweep_staged(&root, None);
                for job in rx {
                    write_pregame_ops(&root, &job.shots, PREGAME_KEEP_GAMES, &mut made);
                    if let Some(note) = &job.note {
                        write_note(&root, note, PREGAME_KEEP_GAMES, &mut made);
                    }
                }
            })?;
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
            dead_logged: false,
        })
    }

    /// Queue `shots`; false when the queue is full or the writer is gone,
    /// and then none of the batch is written. A full queue is logged at
    /// debug each time; a writer thread that ended (a panic) is logged
    /// once at warn, and no later batch is sent.
    pub(crate) fn send(&mut self, shots: ShotBatch) -> bool {
        self.send_job(WriterJob { shots, note: None })
    }

    /// [`Self::send`] for shots plus a text file written after them.
    pub(crate) fn send_job(&mut self, job: WriterJob) -> bool {
        if job.shots.is_empty() && job.note.is_none() {
            return true;
        }
        let Some(tx) = self.tx.as_ref() else {
            return false;
        };
        match tx.try_send(job) {
            Ok(()) => true,
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                tracing::debug!(target: "scuffed_stat_tracker", "saved screens dropped (pre- or post-game): writer busy");
                false
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                if !self.dead_logged {
                    self.dead_logged = true;
                    tracing::warn!(target: "scuffed_stat_tracker", "screen writer thread ended: the screens around each game are no longer saved this run");
                }
                false
            }
        }
    }

    /// Whether the writer thread still runs: not when it ended (a panic),
    /// whether or not a send found that out yet.
    pub(crate) fn alive(&self) -> bool {
        !self.dead_logged
            && self.tx.is_some()
            && self.thread.as_ref().is_none_or(|t| !t.is_finished())
    }

    /// [`Self::send_job`], retried every 20 ms while the queue is full, up
    /// to `within`: the post-game window's final note at shutdown.
    pub(crate) async fn send_wait(&mut self, mut job: WriterJob, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            let Some(tx) = self.tx.as_ref() else {
                return false;
            };
            match tx.try_send(job) {
                Ok(()) => return true,
                Err(std::sync::mpsc::TrySendError::Full(back)) if Instant::now() < deadline => {
                    job = back;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(_) => return false,
            }
        }
    }

    /// Whether the writer thread ended this run is logged (tests).
    #[cfg(test)]
    pub(crate) fn dead_logged(&self) -> bool {
        self.dead_logged
    }

    /// A writer whose queue nobody reads, so it fills (tests). Keep the
    /// receiver alive.
    #[cfg(test)]
    pub(crate) fn stalled() -> (Self, std::sync::mpsc::Receiver<WriterJob>) {
        let (tx, rx) = std::sync::mpsc::sync_channel::<WriterJob>(PREGAME_QUEUE);
        let writer = Self {
            tx: Some(tx),
            thread: None,
            dead_logged: false,
        };
        (writer, rx)
    }

    /// A writer whose thread is already gone (tests).
    #[cfg(test)]
    pub(crate) fn dead() -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<WriterJob>(PREGAME_QUEUE);
        let thread = std::thread::spawn(move || drop(rx));
        while !thread.is_finished() {
            std::thread::yield_now();
        }
        Self {
            tx: Some(tx),
            thread: Some(thread),
            dead_logged: false,
        }
    }

    /// Close the queue and wait up to `within` for the queued writes.
    /// True when they all finished.
    pub(crate) async fn finish(mut self, within: Duration) -> bool {
        drop(self.tx.take());
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let deadline = Instant::now() + within;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                tracing::debug!(target: "scuffed_stat_tracker", "pre-game writes still running at shutdown");
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        thread.join().is_ok()
    }
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
}

impl PregameDump {
    /// `None` when `on` is false: no tracker, no thread, no checks, no
    /// files. Also `None`, logged, when the writer thread cannot start.
    /// `poll` is the poll loop's interval, added to the ban-results window.
    pub(crate) fn new(on: bool, data_dir: &std::path::Path, poll: Duration) -> Option<Self> {
        if !on {
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
        match PregameWriter::start(root.clone()) {
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
        }
    }

    pub(crate) fn want(&self, now: Instant) -> PregameWant {
        self.tracker.want(now)
    }

    /// The ban-results title examples, for the poll's blocking read.
    pub(crate) fn examples(&self) -> Arc<super::pregame_ban_title::BanTitleExamples> {
        Arc::clone(&self.examples)
    }

    /// Feed one poll tick and queue what it earned.
    pub(crate) fn observe(
        &mut self,
        now: Instant,
        seen: Seen,
        frame: Option<Arc<image::DynamicImage>>,
    ) {
        let root = &self.root;
        self.saw_loading = seen == Seen::Loading;
        let shots = self
            .tracker
            .observe(now, seen, frame, || unique_pregame_folder(root, Utc::now()));
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
