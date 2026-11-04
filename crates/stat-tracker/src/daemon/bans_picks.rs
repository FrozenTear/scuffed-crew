//! The wiring for the bans and picks reader (`cv-v1`,
//! [`stat_tracker::shadow::bans_picks`]): which saved pre-game frames it
//! reads, when, and what the game record and the app status get from it.
//!
//! The pre-game saver ([`super::pregame_dump`]) already finds the three
//! screens: the ban vote, the ban results and the hero select, each as its
//! last frame (the first frame of a screen is mid transition, which the
//! reader must not be given). One exception: a game whose hero select has
//! only a first frame (a one-tick select) gives that frame for the team
//! ring of the ban results, and for nothing else: its picks are not
//! stored. [`BansPicksFeed`] sees every batch the saver
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
use stat_tracker::model::{BansPicksMissing, BansPicksStatus};
use stat_tracker::shadow::bans_picks::{
    BanRead, BanResults, BansPicksSetting, FRAME_H, FRAME_W, HeroSelect, Layout, RECOGNIZER_ID,
    Read, Reader, TEMPLATE_DIR, self_ring_colour,
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

/// The sentence for a read that found the bans but never saw the hero
/// select, so the picks are stored empty.
pub(crate) const NO_SELECT_NOTICE: &str =
    "Bans were read but hero select was not seen, so picks are empty.";

/// The reader, as far as this wiring uses it. A trait so a test can count
/// the calls and make one fail.
pub(crate) trait Engine: Send + Sync {
    fn vote(&self, img: &RgbImage) -> Option<Vec<Read>>;
    fn select(&self, img: &RgbImage) -> Option<HeroSelect>;
    fn bans(&self, img: &RgbImage, rings: &[[u8; 3]]) -> Option<BanResults>;
}

impl Engine for Reader {
    fn vote(&self, img: &RgbImage) -> Option<Vec<Read>> {
        self.read_ban_vote(img)
    }
    fn select(&self, img: &RgbImage) -> Option<HeroSelect> {
        self.read_hero_select(img)
    }
    fn bans(&self, img: &RgbImage, rings: &[[u8; 3]]) -> Option<BanResults> {
        self.read_ban_results_rings(img, rings)
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
    /// The hero select's first frame. Read only for the team ring, and
    /// only when there is no `select`.
    select_first: Option<Arc<DynamicImage>>,
    results: Option<Arc<DynamicImage>>,
    /// Ban results the saver staged and has not promoted yet.
    staged: Option<Arc<DynamicImage>>,
}

impl Frames {
    fn any(&self) -> bool {
        self.vote.is_some() || self.select.is_some() || self.results.is_some()
    }

    /// The hero select's first frame, when it is to be read for the team
    /// ring: there are ban results and no last frame of the select.
    fn ring_fallback(&self) -> Option<&Arc<DynamicImage>> {
        match (&self.select, &self.results) {
            (None, Some(_)) => self.select_first.as_ref(),
            _ => None,
        }
    }
}

/// What one read gave: the JSON for the game and the sentence for the app
/// status (empty when all went well).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) stored: Option<String>,
    pub(crate) notice: String,
    /// Why the game's bans and picks are as they are, for the game record.
    pub(crate) status: BansPicksStatus,
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
    /// The game whose frames were read last (the session it was recorded
    /// under), so a later capture of that game is no game without screens.
    read_for: Option<String>,
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
            tracing::info!(
                target: "scuffed_stat_tracker",
                config = read_bans_picks,
                env = setting.is_on(),
                "bans and picks: reader off, no frames will be read"
            );
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
            read_for: None,
        }
    }

    /// One batch the saver sends. A shot of a new game folder drops the old
    /// game's frames.
    pub(crate) fn observe(&mut self, ops: &[PregameOp<Arc<DynamicImage>>]) {
        for op in ops {
            match op {
                PregameOp::Save(shot) | PregameOp::Stage(shot) => {
                    self.enter(&shot.folder);
                    let staged = matches!(op, PregameOp::Stage(_));
                    let read = shot.name == PregamePhase::HeroBan.last_name()
                        || shot.name == PregamePhase::HeroSelect.last_name()
                        || shot.name == PregamePhase::HeroSelect.first_name()
                        || shot.name == BAN_RESULTS_NAME;
                    if self.done {
                        // One line per frame the reader would have taken; a
                        // staged frame is replaced every poll, so not those.
                        if read && !staged {
                            tracing::info!(target: "scuffed_stat_tracker", name = shot.name, game = %shot.folder, reason = "this game's frames were read already", "bans and picks: frame not held");
                        }
                        continue;
                    }
                    let frame = Arc::clone(&shot.frame);
                    if shot.name == PregamePhase::HeroBan.last_name() {
                        self.frames.vote = Some(frame);
                    } else if shot.name == PregamePhase::HeroSelect.last_name() {
                        self.frames.select = Some(frame);
                    } else if shot.name == PregamePhase::HeroSelect.first_name() {
                        self.frames.select_first = Some(frame);
                    } else if shot.name == BAN_RESULTS_NAME && staged {
                        self.frames.staged = Some(frame);
                    } else if shot.name == BAN_RESULTS_NAME {
                        self.frames.results = Some(frame);
                    } else {
                        continue;
                    }
                    if !staged {
                        tracing::info!(target: "scuffed_stat_tracker", name = shot.name, game = %shot.folder, "bans and picks: frame held for the reader");
                    }
                    self.fed_at = Some(Utc::now());
                }
                PregameOp::Promote { folder, name } => {
                    if self.folder.as_deref() == Some(folder)
                        && *name == BAN_RESULTS_NAME
                        && let Some(frame) = self.frames.staged.take()
                    {
                        tracing::info!(target: "scuffed_stat_tracker", name = *name, game = %folder, "bans and picks: frame held for the reader (staged frame saved)");
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
            if self.has_frames() {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    old = self.folder.as_deref().unwrap_or("none"),
                    new = folder,
                    "bans and picks: the last game's frames were never read, a new game folder opened"
                );
            }
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

    /// Show "Bans and picks were not read: `why`." in the app's Tracker
    /// health, for a game that is recorded without them.
    pub(crate) fn publish_not_read(&self, why: &str) {
        let notice = not_read(why);
        tracing::info!(target: "scuffed_stat_tracker", notice = %notice, "bans and picks not read, the game is recorded without them");
        stat_tracker::ocr_status::publish_bans_picks_notice(&self.data_dir, &notice);
    }

    /// The frames held were read for `session_id` already.
    pub(crate) fn was_read_for(&self, session_id: &str) -> bool {
        self.read_for.as_deref() == Some(session_id)
    }

    /// Note that the frames were read for `session_id`.
    pub(crate) fn mark_read_for(&mut self, session_id: &str) {
        self.read_for = Some(session_id.to_string());
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
        let folder = self.folder.clone().unwrap_or_default();
        let joined = tokio::task::spawn_blocking(move || {
            read_frames(engine, &loader, frames, linked, &folder)
        })
        .await;
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
                status: BansPicksStatus::ReaderFailed,
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
    folder: &str,
) -> (Option<Arc<dyn Engine>>, Outcome) {
    let fail = |engine, notice, status| {
        (
            engine,
            Outcome {
                stored: None,
                notice,
                status,
            },
        )
    };
    let shown = [
        frames.vote.as_ref(),
        frames.select.as_ref(),
        frames.results.as_ref(),
        frames.ring_fallback(),
    ];
    // The reader refuses any other size, so ask before loading it.
    if let Some(odd) = shown
        .iter()
        .copied()
        .flatten()
        .find(|f| (f.width(), f.height()) != (FRAME_W, FRAME_H))
    {
        return fail(
            cached,
            resolution_notice(odd.width(), odd.height()),
            BansPicksStatus::FrameSize {
                width: odd.width(),
                height: odd.height(),
            },
        );
    }
    let engine = match cached {
        Some(engine) => engine,
        None => match loader() {
            Ok(engine) => engine,
            Err(LoadFailure::NotInstalled) => {
                return fail(
                    None,
                    missing_notice(linked),
                    BansPicksStatus::PackMissing { linked },
                );
            }
            Err(LoadFailure::Failed(e)) => {
                return fail(None, load_notice(&e), BansPicksStatus::PackFailed);
            }
        },
    };
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        read_with(engine.as_ref(), &frames)
    }));
    match read {
        Ok(Some(stored)) => {
            let json = serde_json::to_string(&stored).ok();
            let missing = missing_screens(&frames);
            let status = if json.is_none() {
                BansPicksStatus::ReaderFailed
            } else if missing.is_empty() {
                BansPicksStatus::Ok
            } else {
                BansPicksStatus::Partial { missing }
            };
            let notice = if json.is_none() {
                not_read("the result could not be written")
            } else if select_missing(&frames, &stored) {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    game = folder,
                    "bans and picks: bans were read but the hero select was not seen, picks are empty"
                );
                NO_SELECT_NOTICE.to_string()
            } else {
                String::new()
            };
            (
                Some(engine),
                Outcome {
                    stored: json,
                    notice,
                    status,
                },
            )
        }
        _ => fail(
            Some(engine),
            not_read("the reader failed on this game's screens"),
            BansPicksStatus::ReaderFailed,
        ),
    }
}

/// The screens this game has no frame for. A hero select with only a first
/// frame counts as missing: that frame gives the team ring and no picks.
fn missing_screens(frames: &Frames) -> Vec<BansPicksMissing> {
    let mut missing = Vec::new();
    if frames.vote.is_none() {
        missing.push(BansPicksMissing::BanVote);
    }
    if frames.select.is_none() {
        missing.push(BansPicksMissing::HeroSelect);
    }
    if frames.results.is_none() {
        missing.push(BansPicksMissing::BanResults);
    }
    missing
}

/// The bans were read (or their screens were there) and the hero select
/// was not seen at all: the picks come out empty with nothing to say why.
/// A select with only its first frame was seen, and is left as it was.
fn select_missing(frames: &Frames, stored: &Stored) -> bool {
    frames.select.is_none()
        && frames.select_first.is_none()
        && (frames.results.is_some()
            || frames.vote.is_some()
            || !stored.bans.is_empty()
            || stored.lobby_ban.is_some())
}

/// `None` when a frame that was there could not be read.
fn read_with(engine: &dyn Engine, frames: &Frames) -> Option<Stored> {
    let rgb = |f: &Arc<DynamicImage>| f.to_rgb8();
    let select = match &frames.select {
        Some(f) => Some(engine.select(&rgb(f))?),
        None => None,
    };
    // The team ring comes from the select's last frame, or else its first.
    // The first frame gives the ring only: its picks are dropped, and a
    // first frame the reader cannot read leaves the ring unset rather than
    // failing the read. The ban vote screen shows the same ring, so its last
    // frame is the second source, for a select that was missed or faded.
    let select_ring = match &select {
        Some(s) => Some(s.ring),
        None => frames
            .ring_fallback()
            .and_then(|f| engine.select(&rgb(f)))
            .map(|s| s.ring),
    };
    let vote_ring = frames
        .vote
        .as_ref()
        .map(rgb)
        .filter(|f| f.dimensions() == (FRAME_W, FRAME_H))
        .map(|f| self_ring_colour(&f));
    let rings: Vec<[u8; 3]> = select_ring.into_iter().chain(vote_ring).collect();
    let results = match &frames.results {
        Some(f) => Some(engine.bans(&rgb(f), &rings)?),
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

#[cfg(test)]
mod notice_tests {
    use super::*;
    use stat_tracker::shadow::bans_picks::Choice;

    /// Reads the ban results and nothing else is asked of it.
    struct BansOnly;

    fn hero(name: &str) -> Read {
        Read {
            value: Some(Choice::Hero(name.to_string())),
            confidence: 0.9,
            score: 0.9,
            margin: 0.2,
        }
    }

    impl Engine for BansOnly {
        fn vote(&self, _: &RgbImage) -> Option<Vec<Read>> {
            Some(vec![hero("ana")])
        }
        fn select(&self, _: &RgbImage) -> Option<HeroSelect> {
            Some(HeroSelect {
                picks: vec![hero("mercy")],
                ring: [1, 2, 3],
            })
        }
        fn bans(&self, _: &RgbImage, _: &[[u8; 3]]) -> Option<BanResults> {
            Some(BanResults {
                layout: Layout::Plain,
                bans: vec![BanRead {
                    read: hero("genji"),
                    team: None,
                }],
                lobby_ban: None,
            })
        }
    }

    fn frame() -> Arc<DynamicImage> {
        Arc::new(DynamicImage::new_rgb8(FRAME_W, FRAME_H))
    }

    fn run(frames: Frames) -> Outcome {
        let loader: Loader = Arc::new(|| Ok(Arc::new(BansOnly) as Arc<dyn Engine>));
        read_frames(None, &loader, frames, true, "20261103_120000").1
    }

    #[test]
    fn results_without_a_select_store_empty_picks_and_a_notice() {
        let outcome = run(Frames {
            results: Some(frame()),
            ..Frames::default()
        });
        let stored = outcome.stored.expect("a result");
        assert!(stored.contains("genji"), "{stored}");
        assert!(stored.contains("\"picks\":[]"), "{stored}");
        assert!(!outcome.notice.is_empty());
        assert_eq!(outcome.notice, NO_SELECT_NOTICE);
        assert!(!outcome.notice.contains('\u{2014}'));
        assert_eq!(
            outcome.status,
            BansPicksStatus::Partial {
                missing: vec![BansPicksMissing::BanVote, BansPicksMissing::HeroSelect]
            }
        );
    }

    #[test]
    fn a_ban_vote_without_a_select_gets_the_notice_too() {
        let outcome = run(Frames {
            vote: Some(frame()),
            ..Frames::default()
        });
        assert_eq!(outcome.notice, NO_SELECT_NOTICE);
    }

    #[test]
    fn a_select_with_the_results_leaves_no_notice() {
        let outcome = run(Frames {
            results: Some(frame()),
            select: Some(frame()),
            ..Frames::default()
        });
        let stored = outcome.stored.expect("a result");
        assert!(stored.contains("mercy"), "{stored}");
        assert_eq!(outcome.notice, "");
        assert_eq!(
            outcome.status,
            BansPicksStatus::Partial {
                missing: vec![BansPicksMissing::BanVote]
            }
        );
    }

    #[test]
    fn a_select_with_only_its_first_frame_gets_no_notice() {
        let outcome = run(Frames {
            vote: Some(frame()),
            results: Some(frame()),
            select_first: Some(frame()),
            ..Frames::default()
        });
        assert_eq!(outcome.notice, "");
        assert_eq!(
            outcome.status,
            BansPicksStatus::Partial {
                missing: vec![BansPicksMissing::HeroSelect]
            }
        );
    }

    #[test]
    fn all_three_screens_are_ok() {
        let outcome = run(Frames {
            vote: Some(frame()),
            results: Some(frame()),
            select: Some(frame()),
            ..Frames::default()
        });
        assert_eq!(outcome.status, BansPicksStatus::Ok);
        assert_eq!(outcome.notice, "");
    }
}

#[cfg(test)]
#[path = "tests_bans_picks.rs"]
mod tests;
