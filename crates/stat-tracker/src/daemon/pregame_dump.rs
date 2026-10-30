//! Opt-in pre-game screen saves (`debug_save_pregame_screens`): full-size
//! PNGs of the map vote, hero ban, ban results, hero select and loading
//! screens under `{data_dir}/debug/pregame/<YYYYMMDD_HHMMSS>/`, one folder
//! per game, newest [`PREGAME_KEEP_GAMES`] kept. Training data for later
//! ban and hero reading; local only, nothing reads, uploads or syncs them.
//!
//! The poll tick classifies its frame ([`Seen`]) on the blocking thread and
//! hands it to [`PregameTracker::observe`], which decides what to write.
//! Writes happen only when a phase starts or ends and run on a blocking
//! thread ([`spawn_pregame_saves`]). The tracker holds at most one frame:
//! the latest one of the phase on screen, to write as that phase's last
//! frame when the phase changes, or the ban-results frame until the next
//! screen shows. With the switch off no tracker exists.
//!
//! Ban results are no phase of their own yet: the frame is the first one
//! at least [`BAN_RESULTS_DELAY`] after the last ban-phase frame, kept only
//! if the ban does not come back. Only a map vote or a ban opens a game
//! folder; a hero select alone is the one between rounds.

use crate::*;
use std::time::Duration;

/// Game folders kept under `debug/pregame/`; older ones are deleted.
pub(crate) const PREGAME_KEEP_GAMES: usize = 20;

/// Pre-game events further apart than this belong to different games. The
/// vote, ban and select follow each other within seconds.
pub(crate) const PREGAME_GAME_GAP: Duration = Duration::from_secs(3 * 60);

/// The ban results animate in after the ban vote; the frame is taken at
/// least this long after the last ban-phase frame.
pub(crate) const BAN_RESULTS_DELAY: Duration = Duration::from_millis(1_000);

/// No ban-results frame later than this after the last ban-phase frame.
pub(crate) const BAN_RESULTS_WINDOW: Duration = Duration::from_secs(15);

/// A phase gone for at most this long and back is a detector flicker (the
/// phase read waits for the screen to hold still). Longer, and it is a new
/// screen: a requeue's map vote, or the hero select between rounds.
pub(crate) const PHASE_FLICKER: Duration = Duration::from_secs(12);

/// What a poll tick saw, for the pre-game saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seen {
    Phase(PregamePhase),
    /// The loading screen ([`detect::loading_screen::is_loading_screen`]).
    Loading,
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
    /// Hand over a frame that is no pre-game phase (ban results, loading).
    pub(crate) other_frame: bool,
}

/// Classify a polled frame for the tracker, on the blocking thread. The
/// loading check runs only when `want.loading` and the frame is no phase.
pub(crate) fn classify(
    want: PregameWant,
    phase: &detect::GamePhase,
    rgb: &image::RgbImage,
) -> Seen {
    if let Some(p) = PregamePhase::of(phase) {
        return Seen::Phase(p);
    }
    if want.loading && detect::loading_screen::is_loading_screen(rgb) {
        return Seen::Loading;
    }
    Seen::Other
}

/// Whether a tick seeing `seen` should hand its frame to the tracker.
pub(crate) fn keeps_frame(want: PregameWant, seen: Seen) -> bool {
    match seen {
        Seen::Phase(_) => true,
        Seen::Loading | Seen::Other => want.other_frame,
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

/// Decides which pre-game frames to write. Generic over the frame so tests
/// use plain values. It holds at most one frame: the latest of the phase on
/// screen, or the ban-results candidate (never both, no phase is on screen
/// while a candidate waits).
pub(crate) struct PregameTracker<F> {
    game: Option<PregameGame>,
    on_screen: Option<OnScreen<F>>,
    /// When the ban phase was last seen, while a ban-results frame may
    /// still come.
    ban_ended_at: Option<Instant>,
    /// The first frame at least [`BAN_RESULTS_DELAY`] after the ban. Written
    /// when the next screen shows; dropped when the ban comes back (it was a
    /// flicker, not the results).
    ban_results: Option<F>,
}

impl<F> Default for PregameTracker<F> {
    fn default() -> Self {
        Self {
            game: None,
            on_screen: None,
            ban_ended_at: None,
            ban_results: None,
        }
    }
}

impl<F: Clone> PregameTracker<F> {
    fn game_open(&self, now: Instant) -> Option<&PregameGame> {
        self.game
            .as_ref()
            .filter(|g| now.saturating_duration_since(g.last_event) <= PREGAME_GAME_GAP)
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
        let ban_results = !game.ban_results_saved
            && self.ban_results.is_none()
            && (ban_on_screen || self.ban_ended_at.is_some());
        PregameWant {
            loading,
            other_frame: loading || ban_results,
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
            Seen::Loading | Seen::Other => self.observe_other(now, seen, frame, &mut shots),
        }
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
        if self.on_screen.as_ref().is_some_and(|s| s.phase != phase) {
            self.end_phase(now, shots);
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
            self.settle_ban_results(shots);
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
        if phase == PregamePhase::HeroBan {
            // The ban is back: the candidate was a flicker inside it.
            self.ban_results = None;
            self.ban_ended_at = None;
        } else {
            // The next screen: the ban results, if any, end here.
            self.settle_ban_results(shots);
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
        if self.game_open(now).is_none() {
            self.settle_ban_results(shots);
            return;
        }
        if seen == Seen::Loading {
            self.settle_ban_results(shots);
        } else if let Some(ended) = self.ban_ended_at {
            let since = now.saturating_duration_since(ended);
            if since > BAN_RESULTS_WINDOW {
                self.settle_ban_results(shots);
            } else if since >= BAN_RESULTS_DELAY && self.ban_results.is_none() {
                self.ban_results = frame.clone();
            }
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

    /// Write the waiting ban-results frame, and stop waiting for one.
    fn settle_ban_results(&mut self, shots: &mut Vec<PregameShot<F>>) {
        self.ban_ended_at = None;
        let Some(frame) = self.ban_results.take() else {
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

/// A new game's folder name, local time.
pub(crate) fn pregame_folder_name() -> String {
    chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
}

/// A folder [`pregame_folder_name`] made: `YYYYMMDD_HHMMSS`. Only these are
/// pruned; anything else in `debug/pregame/` is left alone.
fn is_game_folder(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 15
        && b.iter().enumerate().all(|(i, c)| {
            if i == 8 {
                *c == b'_'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// Delete the oldest game folders under `root` so at most `keep` are left.
/// The names are timestamps, so name order is age order.
pub(crate) fn prune_pregame_games(root: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut games: Vec<(String, std::path::PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            is_game_folder(&name).then(|| (name, e.path()))
        })
        .collect();
    if games.len() <= keep {
        return;
    }
    games.sort();
    for (_, old) in &games[..games.len() - keep] {
        if let Err(e) = std::fs::remove_dir_all(old) {
            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game folder not deleted");
        }
    }
}

/// Write `shots` under `root` at full size on a blocking thread, then keep
/// the newest [`PREGAME_KEEP_GAMES`] game folders. Fire-and-forget; the
/// handle is for tests. Nothing to write spawns nothing.
pub(crate) fn spawn_pregame_saves(
    root: std::path::PathBuf,
    shots: Vec<PregameShot<Arc<image::DynamicImage>>>,
) -> Option<tokio::task::JoinHandle<()>> {
    if shots.is_empty() {
        return None;
    }
    Some(tokio::task::spawn_blocking(move || {
        write_pregame_shots(&root, &shots, PREGAME_KEEP_GAMES)
    }))
}

pub(crate) fn write_pregame_shots(
    root: &std::path::Path,
    shots: &[PregameShot<Arc<image::DynamicImage>>],
    keep: usize,
) {
    for shot in shots {
        let dir = root.join(&shot.folder);
        let new_folder = !dir.is_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game folder not made");
            continue;
        }
        if let Err(e) = shot.frame.save(dir.join(shot.name)) {
            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game frame not saved");
        }
        if new_folder {
            prune_pregame_games(root, keep);
        }
    }
}

/// The poll loop's pre-game saver: `Some` only with the switch on.
pub(crate) struct PregameDump {
    root: std::path::PathBuf,
    tracker: PregameTracker<Arc<image::DynamicImage>>,
}

impl PregameDump {
    /// `None` when `on` is false: no tracker, no checks, no files.
    pub(crate) fn new(on: bool, data_dir: &std::path::Path) -> Option<Self> {
        on.then(|| Self {
            root: pregame_dir(data_dir),
            tracker: PregameTracker::default(),
        })
    }

    pub(crate) fn want(&self, now: Instant) -> PregameWant {
        self.tracker.want(now)
    }

    /// Feed one poll tick and start writing what it earned.
    pub(crate) fn observe(
        &mut self,
        now: Instant,
        seen: Seen,
        frame: Option<Arc<image::DynamicImage>>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let shots = self.tracker.observe(now, seen, frame, pregame_folder_name);
        spawn_pregame_saves(self.root.clone(), shots)
    }
}
