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
//! one frame of the phase on screen (its latest, to write as that phase's
//! last frame when the phase changes), or at most two ban-results frames
//! until the next screen shows: the latest, and the confirmed one before
//! it, owed a write. With the switch off no tracker exists.
//!
//! Ban results are no phase of their own yet. The candidate is the first
//! frame at least [`BAN_RESULTS_DELAY`] after the last ban-phase frame (in
//! practice the next poll) that shows the ban-results title
//! ([`super::pregame_ban_title`]: a shape rule, or example screenshots on
//! this PC when there are any, which replace it). The ban icons and scores
//! fill in below the title while the screen stays up, so each later frame
//! with the title replaces the held one. That later frame confirms the
//! held one, which becomes the frame owed a write (replacing an earlier
//! owed one); the newest frame waits for its own confirmation. Only the
//! newest frame that is confirmed when the results end is written, once
//! per game: the held one when it is confirmed, the owed one otherwise.
//! Either is only written within [`BAN_RESULTS_WINDOW`] plus one poll
//! interval of the first candidate frame.
//!
//! What confirms the held frame: for a first candidate, any later
//! non-phase frame other than a select title (it is not the next screen's
//! first frame read one poll early). For a replacement, only a later frame
//! with the results title, or a frame that shows the hero select's title
//! on the very next tick. A plain frame after a replacement does not, nor
//! does a select title after that plain frame or after a skipped tick:
//! the replacement may be the select sliding in under a results title,
//! and the plain frame the select's own first frame. So on the roster
//! path (results, roster, select) the frame before the last results frame
//! is the one written. A release (no next screen) confirms nothing.
//!
//! The phase read lags a new screen by one poll, so a held frame right
//! before the hero select read is the select's first frame and is not
//! written (the owed one is, if any). A frame that shows the hero select's
//! title (the words-v1 mask, read without the stability wait, and only
//! with `reader = "new"`, which loads the mask) is the select too: it
//! never becomes a candidate or confirms a first one, and it ends the
//! wait, which covers a select read that lags two polls or more. It does
//! confirm a held replacement: the title is read with no lag, and the
//! title was on the frames before, so the frame before it was still the
//! results, with the bans filled in. Every ban-results frame is dropped
//! when the ban comes back, and when that window runs out before the next
//! screen reads. Only a map vote or a ban opens a game folder; a hero
//! select alone is the one between rounds.

use crate::*;
use std::time::Duration;

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
/// from the first candidate, and with the poll added so
/// a next screen read 13 to 14 s after the results at 4 s polls (19 s
/// then) still settles it.
pub(crate) const BAN_RESULTS_WINDOW: Duration = Duration::from_secs(15);

/// A phase gone for at most this long and back is a detector flicker (the
/// phase read waits for the screen to hold still). Longer, and it is a new
/// screen: a requeue's map vote, or the hero select between rounds.
pub(crate) const PHASE_FLICKER: Duration = Duration::from_secs(12);

/// Batches of shots waiting for the writer. A full queue drops the batch
/// (logged), so frames pinned by pending writes stay bounded.
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

struct PregameGame {
    folder: String,
    last_event: Instant,
    /// [`PregamePhase::bit`]s whose first frame is written.
    firsts: u8,
    /// When each phase was last on screen, by [`PregamePhase::index`].
    last_seen: [Option<Instant>; 3],
    loading_saved: bool,
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

/// The ban-results candidate: the latest frame with the title, and the
/// confirmed frame before it, so at most two are held.
struct BanResults<F> {
    frame: F,
    /// When the first candidate frame was taken: the window runs from
    /// there, and a replacement keeps it.
    taken_at: Instant,
    /// `frame` is not the select's first frame: a later non-phase frame
    /// other than a select title came after a first candidate, or a select
    /// title right after a replacement.
    confirmed: bool,
    /// `frame` replaced an earlier candidate, so a plain later frame does
    /// not confirm it.
    replaced: bool,
    /// A plain frame came right after the replacement `frame`: a select
    /// title after that no longer confirms it.
    passed: bool,
    /// When `frame` was taken, for a replacement: a select title confirms
    /// it only on the next tick.
    replaced_at: Instant,
    /// The frame before `frame`, confirmed by `frame` (a later frame with
    /// the title): written when `frame` is not confirmed by then.
    owed: Option<F>,
}

/// Decides which pre-game frames to write. Generic over the frame so tests
/// use plain values. It holds the latest frame of the phase on screen, or
/// the ban-results candidate and its owed frame (never both: no phase is
/// on screen while a candidate waits).
pub(crate) struct PregameTracker<F> {
    game: Option<PregameGame>,
    on_screen: Option<OnScreen<F>>,
    /// When the ban phase was last seen, while a ban-results frame may
    /// still come.
    ban_ended_at: Option<Instant>,
    /// Settled when the next screen shows (the held frame if confirmed,
    /// the owed one otherwise); dropped when the ban comes back (it was a
    /// flicker, not the results) or the window runs out.
    ban_results: Option<BanResults<F>>,
    /// [`BAN_RESULTS_WINDOW`] plus one poll interval.
    window: Duration,
    /// Longest gap after a replacement for a select title to count as the
    /// very next tick: one and a half poll intervals, so a skipped tick
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

    /// Whether a ban-results frame is owed a write, the one case with a
    /// second frame held (for tests of the frame bound).
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
    ) -> Vec<PregameShot<F>> {
        let mut shots = Vec::new();
        match seen {
            Seen::Phase(phase) => self.observe_phase(now, phase, frame, new_folder, &mut shots),
            Seen::Loading | Seen::Other | Seen::BanResults => {
                self.observe_other(now, seen, frame, &mut shots)
            }
            Seen::SelectTitle => {
                // The select is on screen: the results, if any, end here,
                // and this frame confirms no first candidate. A replacement
                // on the tick right before it (no plain frame between, no
                // tick skipped) is confirmed: the title is read with no
                // lag, so that frame still showed the results.
                let next_tick = self.next_tick;
                if let Some(c) = self.ban_results.as_mut().filter(|c| {
                    c.replaced
                        && !c.passed
                        && now.saturating_duration_since(c.replaced_at) <= next_tick
                }) {
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
    /// write the phase's last frame and a confirmed ban-results frame (a
    /// held replacement is not: it may be the select sliding in, so the
    /// owed frame before it is written), and hold nothing.
    pub(crate) fn release(&mut self, now: Instant) -> Vec<PregameShot<F>> {
        let mut shots = Vec::new();
        self.end_phase(now, &mut shots);
        self.settle_ban_results(now, &mut shots);
        shots
    }

    fn observe_phase(
        &mut self,
        now: Instant,
        phase: PregamePhase,
        frame: Option<F>,
        new_folder: impl FnOnce() -> String,
        shots: &mut Vec<PregameShot<F>>,
    ) {
        if phase == PregamePhase::HeroBan {
            // The ban is back, late or not: the candidate was a frame
            // inside it, not the results.
            self.ban_results = None;
            self.ban_ended_at = None;
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
                shots.push(PregameShot {
                    folder: game.folder.clone(),
                    name: phase.first_name(),
                    frame: frame.clone(),
                });
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
        shots: &mut Vec<PregameShot<F>>,
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
            self.ban_results = None;
            self.ban_ended_at = None;
        } else if let Some(ended) = self.ban_ended_at {
            let since = now.saturating_duration_since(ended);
            if since > self.window && self.ban_results.is_none() {
                // No results title in time: stop looking.
                self.ban_ended_at = None;
            } else if seen == Seen::BanResults
                && let Some(frame) = frame.clone()
            {
                match self.ban_results.as_mut() {
                    // Still the results, more of the bans shown: the held
                    // frame is confirmed and owed a write (dropping an
                    // earlier owed one), and this one waits for its own
                    // confirmation.
                    Some(c) => {
                        c.owed = Some(std::mem::replace(&mut c.frame, frame));
                        c.confirmed = false;
                        c.replaced = true;
                        c.passed = false;
                        c.replaced_at = now;
                    }
                    None if since >= BAN_RESULTS_DELAY => {
                        self.ban_results = Some(BanResults {
                            frame,
                            taken_at: now,
                            confirmed: false,
                            replaced: false,
                            passed: false,
                            replaced_at: now,
                            owed: None,
                        });
                    }
                    None => {}
                }
            }
        }
        if self.game_open(now).is_none() {
            return;
        }
        let Some(game) = self.game.as_mut() else {
            return;
        };
        if seen == Seen::Loading
            && !game.loading_saved
            && let Some(frame) = frame
        {
            game.loading_saved = true;
            shots.push(PregameShot {
                folder: game.folder.clone(),
                name: LOADING_NAME,
                frame,
            });
        }
    }

    /// Write the held ban-results frame when it is confirmed, the owed one
    /// otherwise, while inside the window (15 s plus one poll from the
    /// first candidate) at `now`; drop them otherwise, and stop waiting for
    /// one. Nothing else ever writes them, so a dropped candidate is never
    /// saved later (game end, shutdown).
    fn settle_ban_results(&mut self, now: Instant, shots: &mut Vec<PregameShot<F>>) {
        self.ban_ended_at = None;
        let Some(frame) = self
            .ban_results
            .take()
            .filter(|c| now.saturating_duration_since(c.taken_at) <= self.window)
            .and_then(|c| if c.confirmed { Some(c.frame) } else { c.owed })
        else {
            return;
        };
        let Some(game) = self.game.as_mut() else {
            return;
        };
        if !game.ban_results_saved {
            game.ban_results_saved = true;
            shots.push(PregameShot {
                folder: game.folder.clone(),
                name: BAN_RESULTS_NAME,
                frame,
            });
        }
    }

    /// The phase on screen ended: write its last frame when that is not
    /// the first one, and start the ban-results wait after a ban.
    fn end_phase(&mut self, now: Instant, shots: &mut Vec<PregameShot<F>>) {
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
            shots.push(PregameShot {
                folder: game.folder.clone(),
                name: s.phase.last_name(),
                frame: s.last,
            });
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
/// never deleted.
pub(crate) fn prune_pregame_games(root: &std::path::Path, keep: usize, made: &[String]) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut older: Vec<String> = entries
        .flatten()
        // `DirEntry::file_type` does not follow symlinks.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| is_game_folder(name))
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
    let newest = ours.last();
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

/// Write `shots` under `root` at full size, each to a temporary file then
/// renamed, so a crash leaves no half PNG under the final name. After a
/// new game folder (added to `made`), keep the newest `keep` and save the
/// order.
pub(crate) fn write_pregame_shots(
    root: &std::path::Path,
    shots: &[PregameShot<Arc<image::DynamicImage>>],
    keep: usize,
    made: &mut Vec<String>,
) {
    for shot in shots {
        let dir = root.join(&shot.folder);
        let new_folder = !dir.is_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game folder not made");
            continue;
        }
        let tmp = dir.join(format!(".{}.tmp", shot.name));
        let saved = shot
            .frame
            .save_with_format(&tmp, image::ImageFormat::Png)
            .map_err(|e| e.to_string())
            .and_then(|()| std::fs::rename(&tmp, dir.join(shot.name)).map_err(|e| e.to_string()));
        if let Err(e) = saved {
            let _ = std::fs::remove_file(&tmp);
            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game frame not saved");
        }
        if new_folder {
            made.retain(|name| name != &shot.folder);
            made.push(shot.folder.clone());
            prune_pregame_games(root, keep, made);
            made.retain(|name| root.join(name).is_dir());
            save_pregame_order(root, made);
        }
    }
}

pub(crate) type ShotBatch = Vec<PregameShot<Arc<image::DynamicImage>>>;

/// One thread writes every batch in order, so two writes of the same
/// `*_last.png` never overlap. The queue is bounded by [`PREGAME_QUEUE`].
pub(crate) struct PregameWriter {
    tx: Option<std::sync::mpsc::SyncSender<ShotBatch>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The thread's end was logged (once).
    dead_logged: bool,
}

impl PregameWriter {
    pub(crate) fn start(root: std::path::PathBuf) -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<ShotBatch>(PREGAME_QUEUE);
        let thread = std::thread::Builder::new()
            .name("pregame-writer".into())
            .spawn(move || {
                // Game folders in the order they were made, oldest first.
                let mut made = load_pregame_order(&root);
                for batch in rx {
                    write_pregame_shots(&root, &batch, PREGAME_KEEP_GAMES, &mut made);
                }
            })?;
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
            dead_logged: false,
        })
    }

    /// Queue `shots`; false when the queue is full or the writer is gone.
    /// A full queue is logged at debug each time; a writer thread that
    /// ended (a panic) is logged once at warn, and no later batch is sent.
    pub(crate) fn send(&mut self, shots: ShotBatch) -> bool {
        if shots.is_empty() {
            return true;
        }
        let Some(tx) = self.tx.as_ref() else {
            return false;
        };
        match tx.try_send(shots) {
            Ok(()) => true,
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                tracing::debug!(target: "scuffed_stat_tracker", "pre-game frames dropped: writer busy");
                false
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                if !self.dead_logged {
                    self.dead_logged = true;
                    tracing::warn!(target: "scuffed_stat_tracker", "pre-game writer thread ended: pre-game screens are no longer saved this run");
                }
                false
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
    pub(crate) fn stalled() -> (Self, std::sync::mpsc::Receiver<ShotBatch>) {
        let (tx, rx) = std::sync::mpsc::sync_channel::<ShotBatch>(PREGAME_QUEUE);
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
        let (tx, rx) = std::sync::mpsc::sync_channel::<ShotBatch>(PREGAME_QUEUE);
        drop(rx);
        Self {
            tx: Some(tx),
            thread: None,
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
            }),
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "pre-game screens not saved: no writer thread");
                None
            }
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
        let shots = self
            .tracker
            .observe(now, seen, frame, || unique_pregame_folder(root, Utc::now()));
        self.writer.send(shots);
    }

    /// The game closed or capture stopped: write what is owed and hold no
    /// frame.
    pub(crate) fn release(&mut self, now: Instant) {
        let shots = self.tracker.release(now);
        self.writer.send(shots);
    }

    /// Shutdown: write what is owed and wait up to `within` for the queue.
    pub(crate) async fn finish(mut self, within: Duration) -> bool {
        self.release(Instant::now());
        self.writer.finish(within).await
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
