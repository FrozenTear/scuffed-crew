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
//! one frame: the latest one of the phase on screen, to write as that
//! phase's last frame when the phase changes, or the ban-results frame
//! until the next screen shows. With the switch off no tracker exists.
//!
//! Ban results are no phase of their own yet. The candidate is the first
//! frame at least [`BAN_RESULTS_DELAY`] after the last ban-phase frame (in
//! practice the next poll), kept only when one more non-phase frame
//! follows it before the next phase reads. The phase read lags a new
//! screen by one poll, so a candidate right before the hero select read is
//! the hero select's first frame and is dropped. It is also dropped when
//! the ban comes back. Only a map vote or a ban opens a game folder; a
//! hero select alone is the one between rounds.

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

/// No ban-results frame later than this after the last ban-phase frame.
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
/// A frame that passes the map-vote or hero-ban pixel gate is such a
/// screen before its read settles, never the loading screen. The
/// hero-select gate is left out: its colour spread can pass on the
/// loading screen's white glyphs.
pub(crate) fn classify(
    want: PregameWant,
    phase: &detect::GamePhase,
    rgb: &image::RgbImage,
) -> Seen {
    if let Some(p) = PregamePhase::of(phase) {
        return Seen::Phase(p);
    }
    if want.loading
        && !passes_vote_or_ban_gate(rgb)
        && detect::loading_screen::is_loading_screen(rgb)
    {
        return Seen::Loading;
    }
    Seen::Other
}

/// The map-vote or hero-ban pixel gate of [`detect::match_start`], with the
/// thresholds of its module table.
fn passes_vote_or_ban_gate(rgb: &image::RgbImage) -> bool {
    let m = detect::match_start::gate_metrics(rgb);
    m.navy_ratio >= 0.40 || (m.ban_red_ratio >= 0.003 && m.ban_dark_ratio >= 0.30)
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

/// The ban-results candidate frame.
struct BanResults<F> {
    frame: F,
    /// Another non-phase frame came after it, so it is not the next
    /// screen's first frame read one poll early.
    confirmed: bool,
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
    /// Written when the next screen shows, if confirmed; dropped when the
    /// ban comes back (it was a flicker, not the results).
    ban_results: Option<BanResults<F>>,
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

    /// Whether the tracker holds a frame (for tests of the one-frame bound).
    #[cfg(test)]
    pub(crate) fn holds_frame(&self) -> bool {
        self.on_screen.is_some() || self.ban_results.is_some()
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

    /// No frames for now (the game closed, capture stopped, shutdown):
    /// write the phase's last frame and a confirmed ban-results frame, and
    /// hold nothing.
    pub(crate) fn release(&mut self, now: Instant) -> Vec<PregameShot<F>> {
        let mut shots = Vec::new();
        self.end_phase(now, &mut shots);
        self.settle_ban_results(&mut shots);
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
            self.settle_ban_results(shots);
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
        // This frame is no phase, so a waiting candidate was not the next
        // phase's first frame.
        if let Some(c) = self.ban_results.as_mut() {
            c.confirmed = true;
        }
        if self.game_open(now).is_none() || seen == Seen::Loading {
            self.settle_ban_results(shots);
        } else if let Some(ended) = self.ban_ended_at {
            let since = now.saturating_duration_since(ended);
            if since > BAN_RESULTS_WINDOW {
                self.settle_ban_results(shots);
            } else if since >= BAN_RESULTS_DELAY
                && self.ban_results.is_none()
                && let Some(frame) = frame.clone()
            {
                self.ban_results = Some(BanResults {
                    frame,
                    confirmed: false,
                });
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

    /// Write the waiting ban-results frame when it is confirmed, drop it
    /// otherwise, and stop waiting for one.
    fn settle_ban_results(&mut self, shots: &mut Vec<PregameShot<F>>) {
        self.ban_ended_at = None;
        let Some(c) = self.ban_results.take().filter(|c| c.confirmed) else {
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
                frame: c.frame,
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

type ShotBatch = Vec<PregameShot<Arc<image::DynamicImage>>>;

/// One thread writes every batch in order, so two writes of the same
/// `*_last.png` never overlap. The queue is bounded by [`PREGAME_QUEUE`].
pub(crate) struct PregameWriter {
    tx: Option<std::sync::mpsc::SyncSender<ShotBatch>>,
    thread: Option<std::thread::JoinHandle<()>>,
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
        })
    }

    /// Queue `shots`; false when the queue is full or the writer is gone.
    pub(crate) fn send(&self, shots: ShotBatch) -> bool {
        if shots.is_empty() {
            return true;
        }
        let Some(tx) = self.tx.as_ref() else {
            return false;
        };
        match tx.try_send(shots) {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game frames dropped: writer busy");
                false
            }
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
}

impl PregameDump {
    /// `None` when `on` is false: no tracker, no thread, no checks, no
    /// files. Also `None`, logged, when the writer thread cannot start.
    pub(crate) fn new(on: bool, data_dir: &std::path::Path) -> Option<Self> {
        if !on {
            return None;
        }
        let root = pregame_dir(data_dir);
        match PregameWriter::start(root.clone()) {
            Ok(writer) => Some(Self {
                root,
                tracker: PregameTracker::default(),
                writer,
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
