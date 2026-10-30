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
//! select or loading screen reads. A sampled frame is saved only when its
//! downscaled grey [`Thumb`] differs from the last saved one's. The label
//! is the outcome source the detector already read on that frame, if any.
//!
//! Memory: the ring is the only place frames are held, and only while a
//! game runs and no window is open; the window holds one thumbnail and no
//! frame. Each saved frame is one job for the pre-game writer, whose queue
//! is bounded ([`super::pregame_dump::PREGAME_QUEUE`]).

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

/// The most post-game frames saved per game.
pub(crate) const POSTGAME_MAX_FRAMES: usize = 40;

/// A poll this much before a sample is due still takes it (poll jitter).
pub(crate) const POSTGAME_SLACK: Duration = Duration::from_millis(250);

/// After a window ends, a new one waits for a start screen or this long.
pub(crate) const POSTGAME_REARM: Duration = Duration::from_secs(3 * 60);

/// A game's pre-game folder takes its post-game frames when its last
/// pre-game screen showed at most this long before the window opens.
pub(crate) const POSTGAME_FOLDER_GAP: Duration = Duration::from_secs(45 * 60);

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
    /// A window opened: time zero, and the confirming tick `confirmed_s`
    /// seconds after it.
    fn opened(&mut self, now: Instant, confirmed_s: f64);
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
    thumb: Option<Thumb>,
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
    ring_len: usize,
    span: Duration,
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
            ring_len: back.clamp(2, POSTGAME_RING),
            span: POSTGAME_RING_SPAN + poll,
        }
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

    /// While a window is open: how long from `now` until the next sample
    /// is due (the poll loop brings its next tick forward to it, see
    /// [`sooner_tick`]).
    pub(crate) fn next_due_in(&self, now: Instant) -> Option<Duration> {
        match &self.state {
            State::Window(w) => Some(w.last_sample.map_or(Duration::ZERO, |last| {
                (last + Self::interval(w, now)).saturating_duration_since(now)
            })),
            _ => None,
        }
    }

    pub(crate) fn observe(
        &mut self,
        now: Instant,
        tick: PostTick,
        frame: Option<Frame>,
        sink: &mut impl PostSink,
    ) {
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
                    self.open(now, ring, sink);
                } else if tick.running || (tick.end.is_some() && ringed.is_some()) {
                    // A game runs, or no game does and this frame reads as
                    // an end: the confirming read comes a poll later, and
                    // this may be the end title's first frame.
                    ring.reserve_exact(self.ring_len);
                    ring.extend(ringed);
                    self.state = State::Armed(ring, tick.running);
                }
            }
            State::Armed(mut ring, by_game) => {
                if let Some(frame) = frame {
                    while ring.len() >= self.ring_len {
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
                    self.open(now, ring, sink);
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
                    if w.saved >= POSTGAME_MAX_FRAMES {
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

    fn open(&mut self, now: Instant, ring: VecDeque<Ringed>, sink: &mut impl PostSink) {
        let zero = ring
            .iter()
            .find(|r| r.end.is_some() && now.saturating_duration_since(r.at) <= self.span)
            .map_or(now, |r| r.at);
        sink.opened(zero, secs_from(zero, now));
        let mut w = Window {
            zero,
            last_sample: None,
            thumb: None,
            saved: 0,
        };
        for r in ring {
            if now.saturating_duration_since(r.at) > self.span || w.saved >= POSTGAME_MAX_FRAMES {
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
        if w.saved >= POSTGAME_MAX_FRAMES {
            self.end(now, PostEnd::Cap, sink);
        } else {
            self.state = State::Window(w);
        }
    }

    fn save_if_new(
        w: &mut Window,
        at: Instant,
        frame: Frame,
        label: &'static str,
        sink: &mut impl PostSink,
    ) {
        let thumb = Thumb::of(&frame);
        if w.thumb
            .as_ref()
            .is_some_and(|t| t.diff(&thumb) <= THUMB_SAME)
        {
            return;
        }
        let shot = PostShot {
            at_s: secs_from(w.zero, at),
            label,
            frame,
        };
        if sink.shot(shot) {
            w.thumb = Some(thumb);
            w.saved += 1;
        }
    }

    fn end(&mut self, now: Instant, why: PostEnd, sink: &mut impl PostSink) {
        sink.ended(why);
        self.state = State::Done(now);
    }
}

/// When the poll loop should tick next for the post-game window: `due_in`
/// (from [`PostgameTracker::next_due_in`]) when that comes before the
/// regular tick, which is `poll` after the tick that started `since_tick`
/// ago. `None` keeps the regular tick.
pub(crate) fn sooner_tick(
    due_in: Duration,
    since_tick: Duration,
    poll: Duration,
) -> Option<Duration> {
    (due_in < poll.saturating_sub(since_tick)).then_some(due_in)
}

/// `postgame_01.png` to `postgame_40.png`, made once.
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

/// The open (or last) window's sidecar content.
#[derive(Debug, Default)]
pub(crate) struct PostLog {
    pub(crate) folder: Option<String>,
    pub(crate) confirmed_s: f64,
    pub(crate) frames: Vec<PostEntry>,
    pub(crate) ended: Option<PostEnd>,
    /// Every folder a window wrote to this run, newest last, at most
    /// [`super::pregame_dump::PREGAME_KEEP_GAMES`] (older ones are pruned
    /// anyway): a later window never writes into one of them.
    pub(crate) used: Vec<String>,
}

fn round2(s: f64) -> f64 {
    (s * 100.0).round() / 100.0
}

impl PostLog {
    /// `postgame.json`: time zero, the confirming poll, why it ended, and
    /// each frame's file, seconds from time zero and label.
    pub(crate) fn sidecar(&self) -> String {
        let frames: Vec<serde_json::Value> = self
            .frames
            .iter()
            .map(|e| serde_json::json!({ "file": e.file, "t_s": round2(e.t_s), "label": e.label }))
            .collect();
        let doc = serde_json::json!({
            "version": 1,
            "time_zero": "first ring frame read as a match end (outcome read or end-reel hint), else the confirming poll",
            "confirmed_s": round2(self.confirmed_s),
            "ended": self.ended.map(PostEnd::name),
            "frames": frames,
        });
        serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n"
    }
}

/// The poll loop's sink: picks the game's folder when a window opens and
/// queues each frame, then the sidecar, as one writer job.
pub(crate) struct DumpSink<'a> {
    pub(crate) root: &'a std::path::Path,
    /// The newest pre-game folder and when its last screen showed.
    pub(crate) latest: Option<(&'a str, Instant)>,
    pub(crate) log: &'a mut PostLog,
    pub(crate) writer: &'a mut super::pregame_dump::PregameWriter,
}

impl DumpSink<'_> {
    fn note(&self, folder: &str) -> super::pregame_dump::NoteFile {
        super::pregame_dump::NoteFile {
            folder: folder.to_string(),
            name: POSTGAME_SIDECAR,
            text: self.log.sidecar(),
        }
    }
}

/// The folder a window opened at `zero` writes to: the newest pre-game
/// folder when its last screen showed within [`POSTGAME_FOLDER_GAP`] and
/// no earlier window this run wrote to it (`used`), else a new one.
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
    fn opened(&mut self, now: Instant, confirmed_s: f64) {
        let root = self.root;
        let folder = postgame_folder(self.latest, &self.log.used, now, || {
            super::pregame_dump::unique_pregame_folder(root, Utc::now())
        });
        tracing::debug!(target: "scuffed_stat_tracker", folder = %folder, "post-game screens: window opened");
        let mut used = std::mem::take(&mut self.log.used);
        used.push(folder.clone());
        if used.len() > super::pregame_dump::PREGAME_KEEP_GAMES {
            used.remove(0);
        }
        *self.log = PostLog {
            folder: Some(folder),
            confirmed_s,
            frames: Vec::new(),
            ended: None,
            used,
        };
    }

    fn shot(&mut self, shot: PostShot) -> bool {
        let Some(folder) = self.log.folder.clone() else {
            return false;
        };
        let index = self.log.frames.len();
        if index >= POSTGAME_MAX_FRAMES {
            return false;
        }
        let name = postgame_name(index);
        self.log.frames.push(PostEntry {
            file: name,
            t_s: shot.at_s,
            label: shot.label,
        });
        let job = super::pregame_dump::WriterJob {
            shots: vec![super::pregame_dump::PregameShot {
                folder: folder.clone(),
                name,
                frame: shot.frame,
            }],
            note: Some(self.note(&folder)),
        };
        let queued = self.writer.send_job(job);
        if !queued {
            self.log.frames.pop();
        }
        queued
    }

    fn ended(&mut self, why: PostEnd) {
        self.log.ended = Some(why);
        tracing::debug!(target: "scuffed_stat_tracker", why = why.name(), frames = self.log.frames.len(), "post-game screens: window ended");
        if let Some(folder) = self.log.folder.clone() {
            let note = self.note(&folder);
            self.writer.send_job(super::pregame_dump::WriterJob {
                shots: Vec::new(),
                note: Some(note),
            });
        }
    }
}
