//! The wiring for the bans and picks reader (`cv-v1`,
//! [`stat_tracker::shadow::bans_picks`]): which saved pre-game frames it
//! reads, when, and what the game record and the app status get from it.
//!
//! The pre-game saver ([`super::pregame_dump`]) already finds the three
//! screens: the ban vote, the ban results and the hero select, each as its
//! last frame (the first frame of a screen is mid transition, which the
//! reader must not be given). [`BansPicksFeed`] sees every batch the saver
//! sends, keeps those three frames of the current game, and reads them when
//! the game is first recorded. Nothing here can stop a game from being
//! recorded: every failure is a sentence for the app's Tracker health and
//! an empty result on the game.
//!
//! Frames are held only between the screen and the game's first record.
//! A new game's folder drops the old ones. With the pre-game screen saves
//! off (the default) the saver runs with a writer that writes nothing
//! ([`super::pregame_dump::PregameWriter::memory`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use image::{DynamicImage, RgbImage};
use serde::Serialize;
use stat_tracker::shadow::bans_picks::{
    BanRead, BanResults, BansPicksSetting, FRAME_H, FRAME_W, HeroSelect, Layout, RECOGNIZER_ID,
    Read, Reader, TEMPLATE_DIR,
};

use super::pregame_dump::{BAN_RESULTS_NAME, PregameOp, PregamePhase};

/// How long after the last read screen a game may start and still be the
/// one that was read. Longer, and the game is another one whose own screens
/// were never read (a requeue the saver did not see).
pub(crate) const GAME_WINDOW_MINUTES: i64 = 30;

/// The clock difference allowed between the last screen and the game's
/// first record.
const CLOCK_SLACK_SECONDS: i64 = 5;

/// The longest the pack's error is repeated in the status sentence.
const ERROR_CHARS: usize = 120;

/// The reader, as far as this wiring uses it. A trait so a test can count
/// the calls and make one fail.
pub(crate) trait Engine: Send + Sync {
    fn vote(&self, img: &RgbImage) -> Option<Vec<Read>>;
    fn select(&self, img: &RgbImage) -> Option<HeroSelect>;
    fn bans(&self, img: &RgbImage, ring: Option<[u8; 3]>) -> Option<BanResults>;
}

impl Engine for Reader {
    fn vote(&self, img: &RgbImage) -> Option<Vec<Read>> {
        self.read_ban_vote(img)
    }
    fn select(&self, img: &RgbImage) -> Option<HeroSelect> {
        self.read_hero_select(img)
    }
    fn bans(&self, img: &RgbImage, ring: Option<[u8; 3]>) -> Option<BanResults> {
        self.read_ban_results(img, ring)
    }
}

/// Why there is no reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoadFailure {
    /// The pack's folder is not there (or holds no pictures).
    NotInstalled,
    /// The folder is there and the reader refused it.
    Failed(String),
}

pub(crate) type Loader = Arc<dyn Fn() -> Result<Arc<dyn Engine>, LoadFailure> + Send + Sync>;

/// Whether the pack's folder holds any picture.
fn pack_installed(data_dir: &Path) -> bool {
    std::fs::read_dir(data_dir.join(TEMPLATE_DIR)).is_ok_and(|dir| {
        dir.flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "png"))
    })
}

fn load_reader(data_dir: &Path) -> Result<Arc<dyn Engine>, LoadFailure> {
    if !pack_installed(data_dir) {
        return Err(LoadFailure::NotInstalled);
    }
    match Reader::load(data_dir) {
        Ok(reader) => Ok(Arc::new(reader)),
        Err(e) => Err(LoadFailure::Failed(e.to_string())),
    }
}

/// What is stored on the game (`PersonalMatch::bans_picks`).
#[derive(Debug, Serialize)]
struct Stored {
    layout: Option<Layout>,
    bans: Vec<BanRead>,
    lobby_ban: Option<Read>,
    picks: Vec<Read>,
    /// The six own-team rows of the ban vote screen.
    ban_vote: Vec<Read>,
    bans_picks_recognizer: &'static str,
}

/// The three screens of the current game.
#[derive(Default)]
struct Frames {
    vote: Option<Arc<DynamicImage>>,
    select: Option<Arc<DynamicImage>>,
    results: Option<Arc<DynamicImage>>,
    /// Ban results the saver staged and has not promoted yet.
    staged: Option<Arc<DynamicImage>>,
}

impl Frames {
    fn any(&self) -> bool {
        self.vote.is_some() || self.select.is_some() || self.results.is_some()
    }
}

/// What one read gave: the JSON for the game and the sentence for the app
/// status (empty when all went well).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) stored: Option<String>,
    pub(crate) notice: String,
}

pub(crate) struct BansPicksFeed {
    data_dir: PathBuf,
    /// This install has a site account, the only way the pack is fetched.
    linked: bool,
    loader: Loader,
    /// The loaded reader. A failed load is tried again at the next game, so
    /// a pack that arrives later is used without a restart.
    engine: Option<Arc<dyn Engine>>,
    folder: Option<String>,
    /// This folder's frames were read already.
    done: bool,
    frames: Frames,
    fed_at: Option<DateTime<Utc>>,
}

impl BansPicksFeed {
    /// `None` when the reader is switched off: the config row "Read bans
    /// and picks" or [`stat_tracker::shadow::bans_picks::ENV`] (off, 0,
    /// false, no).
    pub(crate) fn for_config(
        read_bans_picks: bool,
        setting: BansPicksSetting,
        data_dir: &Path,
        linked: bool,
    ) -> Option<Self> {
        if !read_bans_picks || !setting.is_on() {
            return None;
        }
        let dir = data_dir.to_path_buf();
        let loader: Loader = Arc::new({
            let dir = dir.clone();
            move || load_reader(&dir)
        });
        Some(Self::with_loader(dir, linked, loader))
    }

    pub(crate) fn with_loader(data_dir: PathBuf, linked: bool, loader: Loader) -> Self {
        Self {
            data_dir,
            linked,
            loader,
            engine: None,
            folder: None,
            done: false,
            frames: Frames::default(),
            fed_at: None,
        }
    }

    /// One batch the saver sends. A shot of a new game folder drops the old
    /// game's frames.
    pub(crate) fn observe(&mut self, ops: &[PregameOp<Arc<DynamicImage>>]) {
        for op in ops {
            match op {
                PregameOp::Save(shot) | PregameOp::Stage(shot) => {
                    self.enter(&shot.folder);
                    if self.done {
                        continue;
                    }
                    let staged = matches!(op, PregameOp::Stage(_));
                    let frame = Arc::clone(&shot.frame);
                    if shot.name == PregamePhase::HeroBan.last_name() {
                        self.frames.vote = Some(frame);
                    } else if shot.name == PregamePhase::HeroSelect.last_name() {
                        self.frames.select = Some(frame);
                    } else if shot.name == BAN_RESULTS_NAME && staged {
                        self.frames.staged = Some(frame);
                    } else if shot.name == BAN_RESULTS_NAME {
                        self.frames.results = Some(frame);
                    } else {
                        continue;
                    }
                    self.fed_at = Some(Utc::now());
                }
                PregameOp::Promote { folder, name } => {
                    if self.folder.as_deref() == Some(folder)
                        && *name == BAN_RESULTS_NAME
                        && let Some(frame) = self.frames.staged.take()
                    {
                        self.frames.results = Some(frame);
                    }
                }
                PregameOp::Discard { folder, name } => {
                    if self.folder.as_deref() == Some(folder) && *name == BAN_RESULTS_NAME {
                        self.frames.staged = None;
                    }
                }
                PregameOp::Adopt { .. } => {}
            }
        }
    }

    fn enter(&mut self, folder: &str) {
        if self.folder.as_deref() != Some(folder) {
            self.folder = Some(folder.to_string());
            self.frames = Frames::default();
            self.done = false;
            self.fed_at = None;
        }
    }

    /// There is something to read for a game not read yet.
    pub(crate) fn has_frames(&self) -> bool {
        !self.done && self.frames.any()
    }

    /// Whether a game whose first record was at `started` is the one these
    /// frames belong to: it began after the last of them, and not long
    /// after.
    pub(crate) fn fits(&self, started: DateTime<Utc>) -> bool {
        let Some(fed) = self.fed_at else {
            return false;
        };
        started + Duration::seconds(CLOCK_SLACK_SECONDS) >= fed
            && started - fed <= Duration::minutes(GAME_WINDOW_MINUTES)
    }

    /// Read the held frames, off the poll loop's thread, and publish the
    /// sentence (empty clears it). The frames are dropped either way.
    pub(crate) async fn read_now(&mut self) -> Outcome {
        let frames = std::mem::take(&mut self.frames);
        self.done = true;
        let loader = Arc::clone(&self.loader);
        let engine = self.engine.clone();
        let linked = self.linked;
        let joined =
            tokio::task::spawn_blocking(move || read_frames(engine, &loader, frames, linked)).await;
        let outcome = match joined {
            Ok((engine, outcome)) => {
                if engine.is_some() {
                    self.engine = engine;
                }
                outcome
            }
            Err(_) => Outcome {
                stored: None,
                notice: not_read("the reader failed on this game's screens"),
            },
        };
        stat_tracker::ocr_status::publish_bans_picks_notice(&self.data_dir, &outcome.notice);
        outcome
    }
}

fn not_read(why: &str) -> String {
    format!("Bans and picks were not read: {why}.")
}

fn short(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= ERROR_CHARS {
        return flat;
    }
    let cut: String = flat.chars().take(ERROR_CHARS).collect();
    format!("{cut}...")
}

/// The sentence for a pack that is not there, with the way to get it.
pub(crate) fn missing_notice(linked: bool) -> String {
    if linked {
        not_read("the pack is not installed yet, the tracker fetches it in the background")
    } else {
        not_read(
            "the pack is not installed, and this install is not linked to a site account, \
             which is what fetches it. Link one in Settings and the tracker downloads the pack",
        )
    }
}

fn load_notice(error: &str) -> String {
    not_read(&format!(
        "the pack did not load ({}). Reinstall the pack or turn off Read bans and picks in \
         Settings > Troubleshooting",
        short(error)
    ))
}

fn resolution_notice(w: u32, h: u32) -> String {
    not_read(&format!(
        "the game ran at {w}x{h} and the reader needs {FRAME_W}x{FRAME_H}"
    ))
}

/// The read itself, on a blocking thread. Returns the reader when it
/// loaded, to be kept.
fn read_frames(
    cached: Option<Arc<dyn Engine>>,
    loader: &Loader,
    frames: Frames,
    linked: bool,
) -> (Option<Arc<dyn Engine>>, Outcome) {
    let fail = |engine, notice| {
        (
            engine,
            Outcome {
                stored: None,
                notice,
            },
        )
    };
    let shown = [&frames.vote, &frames.select, &frames.results];
    // The reader refuses any other size, so ask before loading it.
    if let Some(odd) = shown
        .iter()
        .copied()
        .flatten()
        .find(|f| (f.width(), f.height()) != (FRAME_W, FRAME_H))
    {
        return fail(cached, resolution_notice(odd.width(), odd.height()));
    }
    let engine = match cached {
        Some(engine) => engine,
        None => match loader() {
            Ok(engine) => engine,
            Err(LoadFailure::NotInstalled) => return fail(None, missing_notice(linked)),
            Err(LoadFailure::Failed(e)) => return fail(None, load_notice(&e)),
        },
    };
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        read_with(engine.as_ref(), &frames)
    }));
    match read {
        Ok(Some(stored)) => {
            let json = serde_json::to_string(&stored).ok();
            let notice = if json.is_some() {
                String::new()
            } else {
                not_read("the result could not be written")
            };
            (
                Some(engine),
                Outcome {
                    stored: json,
                    notice,
                },
            )
        }
        _ => fail(
            Some(engine),
            not_read("the reader failed on this game's screens"),
        ),
    }
}

/// `None` when a frame that was there could not be read.
fn read_with(engine: &dyn Engine, frames: &Frames) -> Option<Stored> {
    let rgb = |f: &Arc<DynamicImage>| f.to_rgb8();
    let select = match &frames.select {
        Some(f) => Some(engine.select(&rgb(f))?),
        None => None,
    };
    let results = match &frames.results {
        Some(f) => Some(engine.bans(&rgb(f), select.as_ref().map(|s| s.ring))?),
        None => None,
    };
    let ban_vote = match &frames.vote {
        Some(f) => engine.vote(&rgb(f))?,
        None => Vec::new(),
    };
    Some(Stored {
        layout: results.as_ref().map(|r| r.layout),
        bans: results.as_ref().map(|r| r.bans.clone()).unwrap_or_default(),
        lobby_ban: results.and_then(|r| r.lobby_ban),
        picks: select.map(|s| s.picks).unwrap_or_default(),
        ban_vote,
        bans_picks_recognizer: RECOGNIZER_ID,
    })
}
