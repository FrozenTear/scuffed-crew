//! Opt-in post-game screen saves, behind the same switch as the pre-game
//! ones (`debug_save_pregame_screens`): full-size PNGs of the screens after
//! a match ends (the end title, the post-match accolades, the rank update)
//! as `postgame_NN.png` in the game's folder under `debug/pregame/`, with a
//! `postgame.json` sidecar. Training data for the Vision recognizer; local
//! only, nothing reads, uploads or syncs them.
//!
//! No real frames of these screens were at hand, so nothing here looks for
//! a layout. The window opens when the poll confirms an outcome (two reads
//! of the same word, unchanged) or the open game is marked finished. The
//! accolade screen lasts about 20 s and the confirming read lags the first
//! end frame by a poll or more, so while a game runs the tracker keeps a
//! ring of the last few polled frames ([`POSTGAME_RING`] at most, about
//! 5 s plus one poll) and writes the ones still that young when the window
//! opens. Time zero is the first ring frame read as an end (an outcome read
//! or the end-reel hint), or the confirming poll when none was. From there
//! a frame is sampled about once a second for [`POSTGAME_FAST_FOR`], then
//! about every [`POSTGAME_SLOW`], until [`POSTGAME_WINDOW`] after time
//! zero, [`POSTGAME_MAX_FRAMES`] saved, or a map vote, hero ban, hero
//! select or loading screen reads. A sampled frame is saved when its
//! downscaled grey [`Thumb`] differs from the last saved one's, when the
//! last saved frame is [`POSTGAME_STILL_KEEP`] old (a still screen whose
//! numbers change too little for the thumbnail), and always from
//! [`POSTGAME_KEEP_ALL_FROM`] on. The label is the outcome source the
//! detector already read on that frame, if any.
//!
//! Polling: with the switch on, the window's ticks are ordinary polls,
//! brought forward to about one a second ([`sooner_tick`]). A tick is
//! brought forward only after a tick that handed over a frame, and never
//! to less than [`POSTGAME_SLACK`]: a tick whose analysis panicked or
//! kept no frame backs off to the regular poll interval.
//!
//! Memory: the ring is the only place frames are held, while a game runs
//! (or, with no game running, for one frame read as an end until the next
//! tick) and no window is open; the window holds one thumbnail and no
//! frame. Each saved frame is one job for the pre-game writer, whose queue
//! is bounded ([`super::pregame_dump::PREGAME_QUEUE`]). Nothing of the
//! ring is ever written to disk; the PNG and sidecar writes go through
//! hidden `.tmp` files that the staged sweep
//! ([`super::pregame_dump::sweep_staged`]) clears after a crash, and a
//! folder of nothing but leftovers takes no retention slot.

use crate::*;
use std::collections::VecDeque;
use std::time::Duration;

pub(crate) type Frame = Arc<image::DynamicImage>;

/// The most frames the ring holds.
pub(crate) const POSTGAME_RING: usize = 5;

/// How far back the ring reaches when the window opens, plus one poll
/// interval.
pub(crate) const POSTGAME_RING_SPAN: Duration = Duration::from_secs(5);

/// The sampling interval right after the match ends.
pub(crate) const POSTGAME_FAST: Duration = Duration::from_secs(1);

/// How long after time zero the fast sampling lasts.
pub(crate) const POSTGAME_FAST_FOR: Duration = Duration::from_secs(30);

/// The sampling interval after that.
pub(crate) const POSTGAME_SLOW: Duration = Duration::from_millis(2_500);

/// The window ends this long after time zero.
pub(crate) const POSTGAME_WINDOW: Duration = Duration::from_secs(90);

/// The most post-game frames saved per game: more than the sampling can
/// reach in [`POSTGAME_WINDOW`] (about 54 plus the ring), so the rank
/// update at the end of the window is never cut by it.
pub(crate) const POSTGAME_MAX_FRAMES: usize = 60;

/// A still screen is saved again once the last saved frame is this old:
/// a changed number or line of text is too small for the thumbnail.
pub(crate) const POSTGAME_STILL_KEEP: Duration = Duration::from_secs(5);

/// From this long after time zero (the last 30 s of the window, where the
/// rank values settle), every sampled frame is saved, still or not.
pub(crate) const POSTGAME_KEEP_ALL_FROM: Duration = Duration::from_secs(60);

/// A poll this much before a sample is due still takes it (poll jitter).
pub(crate) const POSTGAME_SLACK: Duration = Duration::from_millis(250);

/// After a window ends, a new one waits for a start screen or this long.
pub(crate) const POSTGAME_REARM: Duration = Duration::from_secs(3 * 60);

/// A game's pre-game folder takes its post-game frames when its last
/// pre-game screen showed at most this long before the window opens, and
/// only when it is that game's own folder ([`FolderOwner`]).
pub(crate) const POSTGAME_FOLDER_GAP: Duration = Duration::from_secs(45 * 60);

/// A pre-game folder belongs to the first game that opened no earlier than
/// this before the folder was first seen.
pub(crate) const POSTGAME_OWNER_GAP: Duration = Duration::from_secs(3 * 60);

pub(crate) const POSTGAME_SIDECAR: &str = "postgame.json";

/// Thumbnail size: 32 by 18 cells, 16:9.
pub(crate) const THUMB_W: u32 = 32;
pub(crate) const THUMB_H: u32 = 18;

/// Mean grey difference (0 to 255) at or below which two thumbnails are
/// the same picture.
pub(crate) const THUMB_SAME: f32 = 2.0;

/// A downscaled grey copy of a frame: each cell the mean of 4 by 4 points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Thumb(Vec<u8>);

impl Thumb {
    pub(crate) fn of(img: &image::DynamicImage) -> Self {
        use image::GenericImageView;
        let (w, h) = img.dimensions();
        let mut cells = Vec::with_capacity((THUMB_W * THUMB_H) as usize);
        for cy in 0..THUMB_H {
            for cx in 0..THUMB_W {
                let (x0, x1) = (cx * w / THUMB_W, (cx + 1) * w / THUMB_W);
                let (y0, y1) = (cy * h / THUMB_H, (cy + 1) * h / THUMB_H);
                let mut sum = 0u32;
                let mut n = 0u32;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let x = x0 + (2 * sx + 1) * (x1 - x0) / 8;
                        let y = y0 + (2 * sy + 1) * (y1 - y0) / 8;
                        if x < w && y < h {
                            let [r, g, b, _] = img.get_pixel(x, y).0;
                            sum += (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114)
                                / 1000;
                            n += 1;
                        }
                    }
                }
                cells.push((sum / n.max(1)) as u8);
            }
        }
        Self(cells)
    }

    /// Mean absolute grey difference per cell.
    pub(crate) fn diff(&self, other: &Self) -> f32 {
        let total: u32 = self
            .0
            .iter()
            .zip(&other.0)
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .sum();
        total as f32 / self.0.len().max(1) as f32
    }
}

/// What a tick's frame showed of the match end, for the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndSeen {
    /// An outcome read from the end title.
    Title,
    /// An outcome read from the accolade screen's result word.
    Accolades,
    /// An outcome read from the rank update screen.
    Rank,
    /// No outcome read, but the end-reel hint fired.
    Reel,
}

impl EndSeen {
    pub(crate) fn of(
        source: Option<detect::match_end::OutcomeSource>,
        end_reel: bool,
    ) -> Option<Self> {
        use detect::match_end::OutcomeSource;
        match source {
            Some(OutcomeSource::EndTitle) => Some(Self::Title),
            Some(OutcomeSource::ResultWord) => Some(Self::Accolades),
            Some(OutcomeSource::RankScreen) => Some(Self::Rank),
            None => end_reel.then_some(Self::Reel),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Title => "end_title",
            Self::Accolades => "accolades",
            Self::Rank => "rank_update",
            Self::Reel => "end_reel",
        }
    }
}

/// One poll tick, as the post-game saves see it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PostTick {
    pub(crate) end: Option<EndSeen>,
    /// The poll confirmed an outcome on this tick ([`super::resolve_word_tick`]).
    pub(crate) confirmed: bool,
    /// A game is open and has no outcome yet.
    pub(crate) running: bool,
    /// A game is open and has its outcome.
    pub(crate) finished: bool,
    /// A map vote, hero ban, hero select or loading screen read.
    pub(crate) new_screen: bool,
    /// When the open game (finished or not) was opened: which game this
    /// is, for its folder ([`FolderOwner`]).
    pub(crate) game_opened: Option<Instant>,
}

/// Which frames the next tick should hand over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PostWant {
    /// Every frame (the ring is on, or a sample is due).
    pub(crate) every: bool,
    /// A frame read as an end (it may confirm with no game open).
    pub(crate) on_end: bool,
}

impl PostWant {
    pub(crate) fn keeps(self, end_seen: bool) -> bool {
        self.every || (self.on_end && end_seen)
    }
}

/// Why a window ended, as the sidecar says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostEnd {
    Time,
    Cap,
    NewScreen,
    GameClosed,
    Shutdown,
}

impl PostEnd {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Time => "window_time",
            Self::Cap => "frame_cap",
            Self::NewScreen => "new_game_screen",
            Self::GameClosed => "game_closed",
            Self::Shutdown => "shutdown",
        }
    }
}

/// One frame to save, `at_s` seconds from time zero (negative before it).
pub(crate) struct PostShot {
    pub(crate) at_s: f64,
    pub(crate) label: &'static str,
    pub(crate) frame: Frame,
}

/// Where the tracker's decisions go.
pub(crate) trait PostSink {
    /// A window opened: time zero, the confirming tick `confirmed_s`
    /// seconds after it, and when its game was opened ([`PostTick::game_opened`]).
    fn opened(&mut self, now: Instant, confirmed_s: f64, game_opened: Option<Instant>);
    /// Save `shot`; false when it was not queued.
    fn shot(&mut self, shot: PostShot) -> bool;
    fn ended(&mut self, why: PostEnd);
}

struct Ringed {
    at: Instant,
    frame: Frame,
    end: Option<EndSeen>,
}

struct Window {
    zero: Instant,
    last_sample: Option<Instant>,
    /// The last saved frame's thumbnail and time.
    thumb: Option<(Thumb, Instant)>,
    saved: usize,
}

enum State {
    Idle,
    /// The ring. `true` while a game runs; `false` for a frame read as an
    /// end with no game running, held only until the next tick confirms
    /// it or reads no end.
    Armed(VecDeque<Ringed>, bool),
    Window(Window),
    Done(Instant),
}

/// Decides which post-game frames to save.
pub(crate) struct PostgameTracker {
    state: State,
    ring_cap: usize,
    span: Duration,
    /// The latest tick handed over a frame: only then is the next tick
    /// brought forward ([`Self::next_due_in`]).
    handed: bool,
    cap: usize,
}

/// Seconds from `zero` to `at`, negative before it.
fn secs_from(zero: Instant, at: Instant) -> f64 {
    match at.checked_duration_since(zero) {
        Some(d) => d.as_secs_f64(),
        None => -zero.saturating_duration_since(at).as_secs_f64(),
    }
}

impl PostgameTracker {
    /// For a poll loop that polls every `poll`: the ring holds enough
    /// frames for [`POSTGAME_RING_SPAN`] plus one poll, 2 to
    /// [`POSTGAME_RING`] of them.
    pub(crate) fn new(poll: Duration) -> Self {
        let poll_ms = poll.as_millis().max(1);
        let back = POSTGAME_RING_SPAN.as_millis().div_ceil(poll_ms) as usize + 1;
        Self {
            state: State::Idle,
            ring_cap: back.clamp(2, POSTGAME_RING),
            span: POSTGAME_RING_SPAN + poll,
            handed: false,
            cap: POSTGAME_MAX_FRAMES,
        }
    }

    /// [`Self::new`] with a lower frame cap (tests: the sampling alone
    /// cannot reach [`POSTGAME_MAX_FRAMES`]).
    #[cfg(test)]
    pub(crate) fn capped(poll: Duration, cap: usize) -> Self {
        Self {
            cap: cap.min(POSTGAME_MAX_FRAMES),
            ..Self::new(poll)
        }
    }

    /// The most frames the ring may hold for this poll (tests).
    #[cfg(test)]
    pub(crate) fn ring_cap(&self) -> usize {
        self.ring_cap
    }

    /// Frames the ring holds (tests).
    #[cfg(test)]
    pub(crate) fn ring_len(&self) -> usize {
        match &self.state {
            State::Armed(ring, _) => ring.len(),
            _ => 0,
        }
    }

    /// Whether a window is open (tests).
    #[cfg(test)]
    pub(crate) fn window_open(&self) -> bool {
        matches!(self.state, State::Window(_))
    }

    fn interval(w: &Window, now: Instant) -> Duration {
        if now.saturating_duration_since(w.zero) < POSTGAME_FAST_FOR {
            POSTGAME_FAST
        } else {
            POSTGAME_SLOW
        }
    }

    fn due(w: &Window, now: Instant) -> bool {
        w.last_sample
            .is_none_or(|last| now + POSTGAME_SLACK >= last + Self::interval(w, now))
    }

    pub(crate) fn want(&self, now: Instant) -> PostWant {
        match &self.state {
            State::Idle => PostWant {
                every: false,
                on_end: true,
            },
            State::Armed(..) => PostWant {
                every: true,
                on_end: true,
            },
            State::Window(w) => PostWant {
                every: Self::due(w, now),
                on_end: false,
            },
            State::Done(_) => PostWant::default(),
        }
    }

    /// While a window is open and the latest tick handed over a frame: how
    /// long from `now` until the next sample is due, at least
    /// [`POSTGAME_SLACK`] (the poll loop brings its next tick forward to
    /// it, see [`sooner_tick`]). A sample's time is its tick's poll time,
    /// not the end of the tick's work, so the samples do not drift by the
    /// grab and analysis time. `None` after a tick that handed over no
    /// frame (its analysis panicked, or nothing was kept): the next tick is
    /// the regular one, so a failing tick never polls again at once.
    pub(crate) fn next_due_in(&self, now: Instant) -> Option<Duration> {
        match &self.state {
            State::Window(w) if self.handed => {
                // The interval at the next sample: the step that lands on
                // the end of the fast part is already a slow one.
                let due = w.last_sample.map_or(Duration::ZERO, |last| {
                    (last + Self::interval(w, last + POSTGAME_FAST)).saturating_duration_since(now)
                });
                Some(due.max(POSTGAME_SLACK))
            }
            _ => None,
        }
    }

    /// Drop the ring and any window with no sidecar note: the writer is
    /// gone, so nothing could be saved.
    pub(crate) fn stop(&mut self) {
        self.state = State::Idle;
        self.handed = false;
    }

    pub(crate) fn observe(
        &mut self,
        now: Instant,
        tick: PostTick,
        frame: Option<Frame>,
        sink: &mut impl PostSink,
    ) {
        self.handed = frame.is_some();
        if let State::Done(at) = self.state
            && (tick.new_screen || now.saturating_duration_since(at) >= POSTGAME_REARM)
        {
            self.state = State::Idle;
        }
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Done(at) => self.state = State::Done(at),
            State::Idle => {
                let mut ring = VecDeque::new();
                let ringed = frame.map(|frame| Ringed {
                    at: now,
                    frame,
                    end: tick.end,
                });
                if tick.confirmed {
                    ring.extend(ringed);
                    self.open(now, ring, tick.game_opened, sink);
                } else if tick.running || (tick.end.is_some() && ringed.is_some()) {
                    // A game runs, or no game does and this frame reads as
                    // an end: the confirming read comes a poll later, and
                    // this may be the end title's first frame.
                    ring.reserve_exact(self.ring_cap);
                    ring.extend(ringed);
                    self.state = State::Armed(ring, tick.running);
                }
            }
            State::Armed(mut ring, by_game) => {
                if let Some(frame) = frame {
                    while ring.len() >= self.ring_cap {
                        ring.pop_front();
                    }
                    ring.push_back(Ringed {
                        at: now,
                        frame,
                        end: tick.end,
                    });
                }
                // A ring held for a lone end read opens only on a
                // confirmation: a finished game then is an older one.
                if tick.confirmed || (by_game && tick.finished) {
                    self.open(now, ring, tick.game_opened, sink);
                } else if tick.running || (!by_game && tick.end.is_some()) {
                    self.state = State::Armed(ring, tick.running);
                }
                // No game running and no end read: the ring is dropped here.
            }
            State::Window(mut w) => {
                if tick.new_screen {
                    // The next game is here: its ticks arm the ring again.
                    self.end(now, PostEnd::NewScreen, sink);
                    self.state = State::Idle;
                } else if now.saturating_duration_since(w.zero) >= POSTGAME_WINDOW {
                    self.end(now, PostEnd::Time, sink);
                } else {
                    if let Some(frame) = frame.filter(|_| Self::due(&w, now)) {
                        w.last_sample = Some(now);
                        let label = tick.end.map_or("after_end", EndSeen::label);
                        Self::save_if_new(&mut w, now, frame, label, sink);
                    }
                    if w.saved >= self.cap {
                        self.end(now, PostEnd::Cap, sink);
                    } else {
                        self.state = State::Window(w);
                    }
                }
            }
        }
    }

    /// The game closed, or shutdown: drop the ring, end a window. A failed
    /// capture does not come here: the window goes on, and its time limit
    /// still ends it.
    pub(crate) fn release(&mut self, now: Instant, why: PostEnd, sink: &mut impl PostSink) {
        match self.state {
            State::Armed(..) => self.state = State::Idle,
            State::Window(_) => self.end(now, why, sink),
            State::Idle | State::Done(_) => {}
        }
    }

    fn open(
        &mut self,
        now: Instant,
        ring: VecDeque<Ringed>,
        game_opened: Option<Instant>,
        sink: &mut impl PostSink,
    ) {
        let zero = ring
            .iter()
            .find(|r| r.end.is_some() && now.saturating_duration_since(r.at) <= self.span)
            .map_or(now, |r| r.at);
        sink.opened(zero, secs_from(zero, now), game_opened);
        let mut w = Window {
            zero,
            last_sample: None,
            thumb: None,
            saved: 0,
        };
        for r in ring {
            if now.saturating_duration_since(r.at) > self.span || w.saved >= self.cap {
                continue;
            }
            let label = r.end.map_or(
                if r.at < zero {
                    "before_end"
                } else {
                    "after_end"
                },
                EndSeen::label,
            );
            w.last_sample = Some(r.at);
            Self::save_if_new(&mut w, r.at, r.frame, label, sink);
        }
        if w.saved >= self.cap {
            self.end(now, PostEnd::Cap, sink);
        } else {
            self.state = State::Window(w);
        }
    }

    /// Save `frame` unless it is the last saved picture again: a near
    /// duplicate (mean grey change at most [`THUMB_SAME`]) is skipped,
    /// except once the last saved frame is [`POSTGAME_STILL_KEEP`] old and
    /// from [`POSTGAME_KEEP_ALL_FROM`] on.
    fn save_if_new(
        w: &mut Window,
        at: Instant,
        frame: Frame,
        label: &'static str,
        sink: &mut impl PostSink,
    ) {
        let thumb = Thumb::of(&frame);
        let keep_all = at.saturating_duration_since(w.zero) >= POSTGAME_KEEP_ALL_FROM;
        if !keep_all
            && w.thumb.as_ref().is_some_and(|(t, saved_at)| {
                t.diff(&thumb) <= THUMB_SAME
                    && at.saturating_duration_since(*saved_at) < POSTGAME_STILL_KEEP
            })
        {
            return;
        }
        let shot = PostShot {
            at_s: secs_from(w.zero, at),
            label,
            frame,
        };
        if sink.shot(shot) {
            w.thumb = Some((thumb, at));
            w.saved += 1;
        }
    }

    fn end(&mut self, now: Instant, why: PostEnd, sink: &mut impl PostSink) {
        sink.ended(why);
        self.state = State::Done(now);
    }
}

/// When the poll loop should tick next for the post-game window: `due_in`
/// (from [`PostgameTracker::next_due_in`]), never less than
/// [`POSTGAME_SLACK`], when that comes before the regular tick, which is
/// `poll` after the tick that started `since_tick` ago. `None` keeps the
/// regular tick.
pub(crate) fn sooner_tick(
    due_in: Duration,
    since_tick: Duration,
    poll: Duration,
) -> Option<Duration> {
    let due_in = due_in.max(POSTGAME_SLACK);
    (due_in < poll.saturating_sub(since_tick)).then_some(due_in)
}

/// Which game the newest pre-game folder belongs to: the first game seen
/// open that was opened no earlier than [`POSTGAME_OWNER_GAP`] before the
/// folder was first seen. A game that started with no pre-game read (no
/// map vote or ban folder of its own) then never takes an older game's
/// folder, however recent.
#[derive(Debug, Default)]
pub(crate) struct FolderOwner {
    folder: String,
    seen_at: Option<Instant>,
    game: Option<Instant>,
}

impl FolderOwner {
    /// One tick: `latest` is the newest pre-game folder, `game_opened` the
    /// open game's open time.
    pub(crate) fn note(
        &mut self,
        now: Instant,
        latest: Option<&str>,
        game_opened: Option<Instant>,
    ) {
        let Some(latest) = latest else {
            return;
        };
        if self.seen_at.is_none() || self.folder != latest {
            latest.clone_into(&mut self.folder);
            self.seen_at = Some(now);
            self.game = None;
        }
        if self.game.is_none()
            && let (Some(opened), Some(seen)) = (game_opened, self.seen_at)
            && opened + POSTGAME_OWNER_GAP >= seen
        {
            self.game = Some(opened);
        }
    }

    /// Whether `folder` is the folder of the game opened at `game_opened`.
    pub(crate) fn owns(&self, folder: &str, game_opened: Option<Instant>) -> bool {
        self.seen_at.is_some()
            && self.folder == folder
            && game_opened.is_some()
            && self.game == game_opened
    }
}

/// `postgame_01.png` to `postgame_60.png`, made once.
pub(crate) fn postgame_name(index: usize) -> &'static str {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let names = NAMES.get_or_init(|| {
        (1..=POSTGAME_MAX_FRAMES)
            .map(|n| format!("postgame_{n:02}.png"))
            .collect()
    });
    names[index.min(POSTGAME_MAX_FRAMES - 1)].as_str()
}

/// One saved frame in the sidecar.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PostEntry {
    pub(crate) file: &'static str,
    pub(crate) t_s: f64,
    pub(crate) label: &'static str,
}

/// What `postgame.json` says. Rendered on the writer thread
/// ([`super::pregame_dump::write_note`]), after the frames queued before
/// it, so it lists only the PNGs that were written.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PostSidecar {
    pub(crate) confirmed_s: f64,
    /// Time zero as a UTC time, so a window in the wrong folder shows.
    pub(crate) zero_utc: Option<chrono::DateTime<Utc>>,
    pub(crate) frames: Vec<PostEntry>,
    pub(crate) ended: Option<PostEnd>,
}

fn round2(s: f64) -> f64 {
    (s * 100.0).round() / 100.0
}

impl PostSidecar {
    /// `postgame.json`: time zero (how it was chosen, and as UTC), the
    /// confirming poll, why it ended, and each frame for which `written`
    /// holds, with its seconds from time zero and label.
    pub(crate) fn render(&self, written: impl Fn(&str) -> bool) -> String {
        let frames: Vec<serde_json::Value> = self
            .frames
            .iter()
            .filter(|e| written(e.file))
            .map(|e| serde_json::json!({ "file": e.file, "t_s": round2(e.t_s), "label": e.label }))
            .collect();
        let doc = serde_json::json!({
            "version": 1,
            "time_zero": "first ring frame read as a match end (outcome read or end-reel hint) within about 5 s plus one poll, else the confirming poll",
            "time_zero_utc": self
                .zero_utc
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
            "confirmed_s": round2(self.confirmed_s),
            "ended": self.ended.map(PostEnd::name),
            "frames": frames,
        });
        serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n"
    }
}

/// The open (or last) window's sidecar and folder.
#[derive(Debug, Default)]
pub(crate) struct PostLog {
    pub(crate) folder: Option<String>,
    pub(crate) doc: PostSidecar,
    /// Every folder a window wrote to this run, newest last, at most
    /// [`super::pregame_dump::PREGAME_KEEP_GAMES`] (older ones are pruned
    /// anyway): a later window never writes into one of them.
    pub(crate) used: Vec<String>,
    /// The final sidecar note a full queue refused: sent again on each
    /// tick and, waiting briefly, at shutdown ([`super::pregame_dump::PregameDump::finish`]).
    pub(crate) pending: Option<super::pregame_dump::NoteFile>,
}

/// The poll loop's sink: picks the game's folder when a window opens and
/// queues each frame, then the sidecar, as one writer job.
pub(crate) struct DumpSink<'a> {
    pub(crate) root: &'a std::path::Path,
    /// The newest pre-game folder and when its last screen showed.
    pub(crate) latest: Option<(&'a str, Instant)>,
    /// Which game that folder belongs to.
    pub(crate) owner: &'a FolderOwner,
    pub(crate) log: &'a mut PostLog,
    pub(crate) writer: &'a mut super::pregame_dump::PregameWriter,
}

impl DumpSink<'_> {
    fn note(&self, folder: &str) -> super::pregame_dump::NoteFile {
        super::pregame_dump::NoteFile {
            folder: folder.to_string(),
            name: POSTGAME_SIDECAR,
            doc: self.log.doc.clone(),
        }
    }
}

/// Send the pending final note, if any, without waiting. Dropped when the
/// writer is gone.
pub(crate) fn retry_pending(log: &mut PostLog, writer: &mut super::pregame_dump::PregameWriter) {
    let Some(note) = log.pending.take() else {
        return;
    };
    let job = super::pregame_dump::WriterJob {
        shots: Vec::new(),
        note: Some(note.clone()),
    };
    if !writer.send_job(job) && writer.alive() {
        log.pending = Some(note);
    }
}

/// The folder a window opened at `zero` writes to: the newest pre-game
/// folder (`latest`, already only when it is the game's own, see
/// [`FolderOwner`]) when its last screen showed within
/// [`POSTGAME_FOLDER_GAP`] and no earlier window this run wrote to it
/// (`used`), else a new one.
pub(crate) fn postgame_folder(
    latest: Option<(&str, Instant)>,
    used: &[String],
    zero: Instant,
    new_folder: impl FnOnce() -> String,
) -> String {
    latest
        .filter(|(folder, at)| {
            zero.saturating_duration_since(*at) <= POSTGAME_FOLDER_GAP
                && !used.iter().any(|u| u == folder)
        })
        .map_or_else(new_folder, |(folder, _)| folder.to_string())
}

impl PostSink for DumpSink<'_> {
    fn opened(&mut self, now: Instant, confirmed_s: f64, game_opened: Option<Instant>) {
        let root = self.root;
        let owner = self.owner;
        let latest = self
            .latest
            .filter(|(folder, _)| owner.owns(folder, game_opened));
        let folder = postgame_folder(latest, &self.log.used, now, || {
            super::pregame_dump::unique_pregame_folder(root, Utc::now())
        });
        // `now` is time zero on the poll's clock; the tick's work since
        // then is not added to it.
        let since_zero = Instant::now().saturating_duration_since(now);
        let zero_utc = Utc::now() - chrono::Duration::from_std(since_zero).unwrap_or_default();
        tracing::info!(
            target: "scuffed_stat_tracker",
            folder = %folder,
            time_zero_utc = %zero_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            reused = latest.is_some_and(|(f, _)| f == folder),
            "post-game screens: window opened"
        );
        let mut used = std::mem::take(&mut self.log.used);
        // A newest pre-game folder passed over here belongs to a game that
        // has ended (or never got its own window): no later game takes it.
        if let Some((passed, _)) = self.latest.filter(|(f, _)| *f != folder)
            && !used.iter().any(|u| u == passed)
        {
            used.push(passed.to_string());
        }
        used.push(folder.clone());
        while used.len() > super::pregame_dump::PREGAME_KEEP_GAMES {
            used.remove(0);
        }
        let pending = self.log.pending.take();
        *self.log = PostLog {
            folder: Some(folder),
            doc: PostSidecar {
                confirmed_s,
                zero_utc: Some(zero_utc),
                frames: Vec::new(),
                ended: None,
            },
            used,
            pending,
        };
    }

    fn shot(&mut self, shot: PostShot) -> bool {
        let Some(folder) = self.log.folder.clone() else {
            return false;
        };
        let index = self.log.doc.frames.len();
        if index >= POSTGAME_MAX_FRAMES {
            return false;
        }
        let name = postgame_name(index);
        self.log.doc.frames.push(PostEntry {
            file: name,
            t_s: shot.at_s,
            label: shot.label,
        });
        let job = super::pregame_dump::WriterJob {
            shots: vec![super::pregame_dump::PregameOp::Save(
                super::pregame_dump::PregameShot {
                    folder: folder.clone(),
                    name,
                    frame: shot.frame,
                },
            )],
            note: Some(self.note(&folder)),
        };
        let queued = self.writer.send_job(job);
        if !queued {
            self.log.doc.frames.pop();
        }
        queued
    }

    /// The final note always goes: a full queue keeps it pending, sent
    /// again on the next ticks and at shutdown.
    fn ended(&mut self, why: PostEnd) {
        self.log.doc.ended = Some(why);
        let Some(folder) = self.log.folder.clone() else {
            return;
        };
        tracing::info!(
            target: "scuffed_stat_tracker",
            folder = %folder,
            frames = self.log.doc.frames.len(),
            why = why.name(),
            "post-game screens: window ended"
        );
        // An older window's note first: one pending note at a time.
        retry_pending(self.log, self.writer);
        if self.log.pending.is_some() {
            tracing::debug!(target: "scuffed_stat_tracker", "post-game sidecar: an older final note is replaced");
        }
        self.log.pending = Some(self.note(&folder));
        retry_pending(self.log, self.writer);
    }
}
