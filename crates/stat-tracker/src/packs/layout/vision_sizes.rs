//! Which game capture sizes the hero icon reader has icon spots for, and
//! the member's answers to the one-time ask for a size it has none for.
//!
//! # Supported sizes come from the installed packs
//!
//! Nothing here lists sizes. Sync records every installed pack's name in
//! `templates/pack-versions.json` ([`super::versions_path`]). A hero
//! portrait pack (a name starting with `heroes-`) declares the capture size
//! it was cut for with a size tag at the end of its name, before `.tar`:
//!
//! * `-<W>x<H>`, for example `heroes-v3-3440x1440.tar`: that size.
//! * `-<H>` with three or four digits, for example `heroes-v2-1080.tar`: the
//!   16:9 size that tall, here 1920x1080.
//! * no size tag (`heroes-v1.tar`, `heroes-v2.tar`): [`BASE_SIZE`],
//!   2560x1440, the size the first packs were cut at.
//!
//! The pack manifests carry no size or collecting field today, so the name
//! is the declaration. A pack for a new size stops the ask as soon as it is
//! installed and recorded, with no change to this file. A size the
//! installed packs do not declare is unsupported.
//!
//! # Fail safe
//!
//! Recording never waits on any of this. A record that does not read is
//! [`Support::Unreadable`]: the member gets a warning, not an ask, and
//! nothing is cut or sent. No record yet, or one with no hero pack in it,
//! is [`Support::NotYetKnown`]: packs may still be downloading, so nothing
//! is said and the next Tab looks again.
//!
//! # Answers
//!
//! `<data_dir>/vision-size-asks.json` ([`Asks`]) keeps, per size, that the
//! ask was shown, closed, or answered "Don't ask again". The app writes it;
//! the daemon reads it, so all three survive a restart. [`may_collect`] is
//! the gate any icon or training crop collection at an unsupported size
//! must pass: a closed ask or "Don't ask again" sends no request at all, and
//! otherwise only the member's `hero_icon` consent on the website lets it
//! through.

use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};

/// The size a hero pack with no size tag in its name was cut for.
pub const BASE_SIZE: ScreenSize = ScreenSize {
    width: 2560,
    height: 1440,
};

/// File name of [`Asks`] under the data dir.
pub const ASKS_FILE: &str = "vision-size-asks.json";

/// A game capture size, in pixels. Written `<W>x<H>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenSize {
    pub width: u32,
    pub height: u32,
}

impl ScreenSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// `3440x1440` (an `X` works too). `None` for anything else, a zero
    /// edge included.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let (w, h) = raw.split_once(['x', 'X'])?;
        let digits =
            |s: &str| !s.is_empty() && s.len() <= 5 && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(w) || !digits(h) {
            return None;
        }
        let size = Self::new(w.parse().ok()?, h.parse().ok()?);
        (size.width > 0 && size.height > 0).then_some(size)
    }

    /// The 16:9 size `height` px tall, rounded to whole pixels.
    fn wide_16_9(height: u32) -> Self {
        Self::new((height * 16 + 4) / 9, height)
    }
}

impl std::fmt::Display for ScreenSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// The capture size a pack name declares (see the module docs). `None` for
/// a pack that is not a hero portrait pack.
pub fn declared_size(pack_name: &str) -> Option<ScreenSize> {
    let stem = pack_name.strip_suffix(".tar").unwrap_or(pack_name);
    let rest = stem.strip_prefix("heroes-")?;
    let tag = rest.rsplit('-').next().unwrap_or(rest);
    if let Some(size) = ScreenSize::parse(tag) {
        return Some(size);
    }
    let height_tag = rest.contains('-')
        && (3..=4).contains(&tag.len())
        && tag.bytes().all(|b| b.is_ascii_digit());
    match tag.parse::<u32>() {
        Ok(height) if height_tag && height > 0 => Some(ScreenSize::wide_16_9(height)),
        _ => Some(BASE_SIZE),
    }
}

/// Whether the installed packs cover a capture size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    Supported,
    Unsupported,
    /// No pack record yet, or no hero pack in it.
    NotYetKnown,
    /// The pack record is there but did not read. For the log only.
    Unreadable(String),
}

/// Every size the installed hero packs declare, from the pack record.
pub fn supported_sizes(data_dir: &Path) -> Result<BTreeSet<ScreenSize>, String> {
    let path = super::versions_path(data_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    let record: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&bytes).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(record
        .keys()
        .filter_map(|name| declared_size(name))
        .collect())
}

/// Whether the installed packs cover `size`.
pub fn support_in(data_dir: &Path, size: ScreenSize) -> Support {
    match supported_sizes(data_dir) {
        Err(err) => Support::Unreadable(err),
        Ok(sizes) if sizes.is_empty() => Support::NotYetKnown,
        Ok(sizes) if sizes.contains(&size) => Support::Supported,
        Ok(_) => Support::Unsupported,
    }
}

/// What the member did with the ask for one size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// The app showed it.
    Asked,
    /// Closed without an answer. Counts as a no.
    Dismissed,
    /// "Don't ask again". A no.
    Never,
}

/// The answers file. Sizes are kept as `<W>x<H>` text.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Asks {
    pub asked: BTreeSet<String>,
    pub dismissed: BTreeSet<String>,
    pub never: BTreeSet<String>,
}

pub fn asks_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ASKS_FILE)
}

impl Asks {
    /// The answers on disk. No file is no answers; a file that does not read
    /// is `Err`, and callers then neither ask nor collect.
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let path = asks_path(data_dir);
        match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|err| format!("{}: {err}", path.display()))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(format!("{}: {err}", path.display())),
        }
    }

    /// Write through a temporary file and a rename, so a reader never sees
    /// half a file.
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let path = asks_path(data_dir);
        let tmp = data_dir.join(format!(".{ASKS_FILE}.{}", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)
    }

    /// Shown before, or answered.
    pub fn was_asked(&self, size: ScreenSize) -> bool {
        let key = size.to_string();
        self.asked.contains(&key) || self.declined(size)
    }

    /// Closed, or "Don't ask again".
    pub fn declined(&self, size: ScreenSize) -> bool {
        let key = size.to_string();
        self.dismissed.contains(&key) || self.never.contains(&key)
    }

    pub fn mark(&mut self, size: ScreenSize, answer: Answer) {
        let key = size.to_string();
        let set = match answer {
            Answer::Asked => &mut self.asked,
            Answer::Dismissed => &mut self.dismissed,
            Answer::Never => &mut self.never,
        };
        set.insert(key);
    }
}

/// Add `answer` for `size` to the answers file. A file that does not read
/// is started again, so the newest answer is kept.
pub fn record_answer(data_dir: &Path, size: ScreenSize, answer: Answer) -> std::io::Result<()> {
    let mut asks = Asks::load(data_dir).unwrap_or_default();
    asks.mark(size, answer);
    asks.save(data_dir)
}

/// What to show for the size of a Tab frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing: a supported size, already asked, or not known yet.
    Quiet,
    /// The one-time ask.
    Ask(ScreenSize),
    /// Could not tell: a warning, no ask. The text is for the log.
    Warn(ScreenSize, String),
}

/// Decides once per size per run. A size whose support is not known yet is
/// looked at again on the next Tab.
#[derive(Debug, Default)]
pub struct Asker {
    settled: BTreeSet<ScreenSize>,
}

impl Asker {
    /// `None` when `size` was already decided in this run, so whatever was
    /// shown for it stays. `support` and `asks` are read only for a size
    /// not decided yet.
    pub fn decide(
        &mut self,
        size: ScreenSize,
        support: impl FnOnce() -> Support,
        asks: impl FnOnce() -> Result<Asks, String>,
    ) -> Option<Decision> {
        if self.settled.contains(&size) {
            return None;
        }
        let decision = match support() {
            Support::NotYetKnown => return Some(Decision::Quiet),
            Support::Supported => Decision::Quiet,
            Support::Unreadable(err) => Decision::Warn(size, err),
            Support::Unsupported => match asks() {
                Ok(asks) if asks.was_asked(size) => Decision::Quiet,
                Ok(_) => Decision::Ask(size),
                Err(err) => Decision::Warn(size, err),
            },
        };
        self.settled.insert(size);
        Some(decision)
    }
}

/// May the tracker cut or send hero icon or training crops at `size`, a size
/// the reader has no spots for? Only with a yes: the ask for `size` was not
/// closed or answered "Don't ask again", and the member's `hero_icon`
/// consent on the website is on. `fetch_hero_icon_consent` is the one
/// consent read; it is not called at all after a no, or when the answers
/// file does not read. Its `None` (no answer, an error) is a no.
pub async fn may_collect<F, Fut>(
    data_dir: &Path,
    size: ScreenSize,
    fetch_hero_icon_consent: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<bool>>,
{
    match Asks::load(data_dir) {
        Ok(asks) if !asks.declined(size) => fetch_hero_icon_consent().await == Some(true),
        _ => false,
    }
}

#[cfg(test)]
#[path = "tests_vision_sizes.rs"]
mod tests;
