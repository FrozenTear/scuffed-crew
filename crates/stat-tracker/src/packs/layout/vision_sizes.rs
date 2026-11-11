//! Which game capture sizes the installed hero icon packs declare, and the
//! member's answers to the one-time ask for a size they do not declare.
//!
//! # Declared sizes come from the installed packs
//!
//! Nothing here lists sizes. Sync records every installed pack's name in
//! `templates/pack-versions.json` ([`super::versions_path`]). A hero
//! portrait pack (a name starting with `heroes-`) declares the capture size
//! it was cut for with a size tag at the end of its name, before `.tar`:
//!
//! * `-<W>x<H>`, for example `heroes-v3-3440x1440.tar`: that size.
//! * `-<H>` with three or four digits, for example `heroes-v2-1080.tar`: the
//!   16:9 size that tall, here 1920x1080. The width is rounded to whole
//!   pixels, so `-768` declares 1365x768, not 1366x768.
//! * no size tag (`heroes-v1.tar`, `heroes-v2.tar`): [`BASE_SIZE`],
//!   2560x1440, the size the first packs were cut at.
//!
//! Any other tag (`heroes-v2-beta.tar`) declares nothing.
//!
//! The pack manifests carry no size or collecting field today, so the name
//! is the declaration. A size the installed packs do not declare is
//! [`Support::Unsupported`], with an exact width x height match. That is
//! narrower than the reader: the hero icon reader crops the centered 16:9
//! game area and picks its templates by board height, so a 16:9 size such
//! as 3840x2160 can be read with the shipped packs and still be asked
//! about. The ask names what is true: no installed pack declares that size.
//!
//! The installed pack list is fixed in the tracker (`PACKS`), so a pack for
//! a new size comes with a tracker update. Once sync has recorded it, the
//! next Tab at that size sees it declared and the ask is taken down
//! ([`Decision::Clear`]), without a restart.
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
//! `<data_dir>/vision-size-asks.json` ([`Asks`]) keeps the member's nos:
//! "No thanks" for one size, and "Don't ask again" for every size. Only a
//! click writes it, so an ask that was never answered comes back after a
//! restart. The app writes it; the daemon reads it. A file that does not
//! read is never written over: [`record_answer`] refuses, and only
//! [`reset_unreadable`] (the member's own click) replaces it, after keeping
//! a copy of its bytes.
//!
//! [`may_collect_hero_icons`] checks only the member's `hero_icon` consent
//! at an undeclared size: a no on file sends no request at all, and
//! otherwise only that consent on the website lets it through.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io::Write as _;
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

    /// The 16:9 size `height` px tall. The width is rounded to whole pixels
    /// (`(height * 16 + 4) / 9`), so 768 gives 1365, not the 1366 of
    /// common panels.
    fn wide_16_9(height: u32) -> Self {
        Self::new((height * 16 + 4) / 9, height)
    }
}

impl std::fmt::Display for ScreenSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// `v` and digits, the version part of a pack name.
fn is_version(part: &str) -> bool {
    part.strip_prefix('v')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The capture size a pack name declares (see the module docs). `None` for
/// a pack that is not a hero portrait pack, or one whose tag is not a size.
pub fn declared_size(pack_name: &str) -> Option<ScreenSize> {
    let stem = pack_name.strip_suffix(".tar").unwrap_or(pack_name);
    let rest = stem.strip_prefix("heroes-")?;
    let Some((_, tag)) = rest.rsplit_once('-') else {
        // `heroes-v1`, `heroes-v2`: no size tag.
        return is_version(rest).then_some(BASE_SIZE);
    };
    if let Some(size) = ScreenSize::parse(tag) {
        return Some(size);
    }
    let height_tag = (3..=4).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_digit());
    match tag.parse::<u32>() {
        Ok(height) if height_tag && height > 0 => Some(ScreenSize::wide_16_9(height)),
        _ => None,
    }
}

/// Whether the installed packs declare a capture size.
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

/// Whether the installed packs declare `size`, exactly.
pub fn support_in(data_dir: &Path, size: ScreenSize) -> Support {
    match supported_sizes(data_dir) {
        Err(err) => Support::Unreadable(err),
        Ok(sizes) if sizes.is_empty() => Support::NotYetKnown,
        Ok(sizes) if sizes.contains(&size) => Support::Supported,
        Ok(_) => Support::Unsupported,
    }
}

/// A no the member clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// "No thanks": a no for this one size.
    Dismissed,
    /// "Don't ask again": a no for every size, this one included.
    Never,
}

/// The answers file. Sizes are kept as `<W>x<H>` text. Fields this version
/// does not know (an older `asked` list, a newer field) are kept as they
/// are when the file is written again.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Asks {
    /// Sizes answered "No thanks".
    pub dismissed: BTreeSet<String>,
    /// Sizes answered "Don't ask again" (each also sets `never_again`).
    pub never: BTreeSet<String>,
    /// "Don't ask again" was clicked: no size is asked about.
    pub never_again: bool,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

pub fn asks_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ASKS_FILE)
}

/// Where [`Asks::save`] writes before its rename.
fn temp_path(data_dir: &Path) -> PathBuf {
    data_dir.join(format!(".{ASKS_FILE}.{}", std::process::id()))
}

/// Write `bytes` to a new file at `path` and flush it to disk. `create_new`
/// refuses a path that is already there.
fn write_synced(path: &Path, bytes: &[u8], create_new: bool) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .create_new(create_new)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

impl Asks {
    /// The answers on disk. No file is no answers; a file that does not read
    /// is `Err`, and callers then neither ask nor collect, and never write
    /// over it.
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

    /// Write through a synced temporary file and a rename, so a reader never
    /// sees half a file. On any error the temporary file is removed and the
    /// file on disk is left as it was.
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let tmp = temp_path(data_dir);
        let written = write_synced(&tmp, &bytes, false)
            .and_then(|()| std::fs::rename(&tmp, asks_path(data_dir)));
        if let Err(err) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(err);
        }
        // The rename itself, on disk. Best effort, as in `packs::layout`.
        if let Ok(dir) = std::fs::File::open(data_dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    /// "No thanks" for this size, or "Don't ask again" for any size.
    pub fn declined(&self, size: ScreenSize) -> bool {
        let key = size.to_string();
        self.never_again || self.dismissed.contains(&key) || self.never.contains(&key)
    }

    pub fn mark(&mut self, size: ScreenSize, answer: Answer) {
        let key = size.to_string();
        match answer {
            Answer::Dismissed => {
                self.dismissed.insert(key);
            }
            Answer::Never => {
                self.never.insert(key);
                self.never_again = true;
            }
        }
    }
}

/// Add `answer` for `size` to the answers file. A file that does not read
/// is an error and is left as it is: starting it again would lose the nos
/// already in it.
pub fn record_answer(data_dir: &Path, size: ScreenSize, answer: Answer) -> std::io::Result<()> {
    let mut asks = Asks::load(data_dir).map_err(std::io::Error::other)?;
    asks.mark(size, answer);
    asks.save(data_dir)
}

/// Replace an answers file that does not read with one that holds a "No
/// thanks" for `size`, the member's own repair. The old bytes are copied
/// first to `vision-size-asks.json.unreadable-<unix seconds>` (never over an
/// existing copy), then the new file is renamed over the old one. If either
/// step fails, the unreadable file is left in place and the error returned.
/// A file that reads is not replaced: the no is added to it. Returns the
/// copy's path when one was made.
pub fn reset_unreadable(data_dir: &Path, size: ScreenSize) -> std::io::Result<Option<PathBuf>> {
    if Asks::load(data_dir).is_ok() {
        record_answer(data_dir, size, Answer::Dismissed)?;
        return Ok(None);
    }
    let old = std::fs::read(asks_path(data_dir))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = data_dir.join(format!("{ASKS_FILE}.unreadable-{stamp}"));
    write_synced(&backup, &old, true)?;
    let mut asks = Asks::default();
    asks.mark(size, Answer::Dismissed);
    asks.save(data_dir)?;
    Ok(Some(backup))
}

/// What to show for the size of a Tab frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing: a declared size, a no on file, or not known yet.
    Quiet,
    /// The ask.
    Ask(ScreenSize),
    /// Could not tell: a warning, no ask. The text is for the log.
    Warn(ScreenSize, String),
    /// The ask or warning shown for this size is no longer due (a pack now
    /// declares it, or the member said no): take it down.
    Clear(ScreenSize),
}

/// What [`Asker`] last decided for a size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Up {
    Quiet,
    Ask,
    Warn,
}

/// Decides once per size per run, and looks again while an ask or warning
/// is up. A size whose support is not known yet is looked at again on the
/// next Tab.
#[derive(Debug, Default)]
pub struct Asker {
    settled: BTreeMap<ScreenSize, Up>,
}

impl Asker {
    /// `None` when nothing changes for `size`: it was decided quiet before
    /// in this run (`support` and `asks` are then not read), or the ask or
    /// warning shown for it is still the right one. An ask or warning that
    /// is no longer due is [`Decision::Clear`]; one that turned into the
    /// other (a reset answers file with no no in it) is sent again.
    pub fn decide(
        &mut self,
        size: ScreenSize,
        support: impl FnOnce() -> Support,
        asks: impl FnOnce() -> Result<Asks, String>,
    ) -> Option<Decision> {
        let prev = self.settled.get(&size).copied();
        if prev == Some(Up::Quiet) {
            return None;
        }
        let fresh = match support() {
            Support::NotYetKnown => None,
            Support::Supported => Some((Up::Quiet, Decision::Quiet)),
            Support::Unreadable(err) => Some((Up::Warn, Decision::Warn(size, err))),
            Support::Unsupported => Some(match asks() {
                Ok(asks) if asks.declined(size) => (Up::Quiet, Decision::Quiet),
                Ok(_) => (Up::Ask, Decision::Ask(size)),
                Err(err) => (Up::Warn, Decision::Warn(size, err)),
            }),
        };
        let Some((up, decision)) = fresh else {
            // Not known yet: say nothing new, look again next Tab.
            return prev.is_none().then_some(Decision::Quiet);
        };
        match prev {
            Some(prev) if prev == up => None,
            Some(_) if up == Up::Quiet => {
                self.settled.insert(size, Up::Quiet);
                Some(Decision::Clear(size))
            }
            _ => {
                self.settled.insert(size, up);
                Some(decision)
            }
        }
    }
}

/// May the tracker cut or send hero icons at `size`? This checks only the
/// member's `hero_icon` consent, and only for a size the installed packs do
/// not declare ([`Support::Unsupported`]): any other support answer (an
/// unreadable or missing pack record, a declared size) is `false` before any
/// request. Then a no on file ("No thanks" for `size`, "Don't ask again"),
/// or an answers file that does not read, is `false` with no request.
/// Otherwise `fetch_hero_icon_consent` is the one consent read, and its
/// `None` (no answer, an error) is a no.
pub async fn may_collect_hero_icons<F, Fut>(
    data_dir: &Path,
    size: ScreenSize,
    fetch_hero_icon_consent: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<bool>>,
{
    if support_in(data_dir, size) != Support::Unsupported {
        return false;
    }
    match Asks::load(data_dir) {
        Ok(asks) if !asks.declined(size) => fetch_hero_icon_consent().await == Some(true),
        _ => false,
    }
}

#[cfg(test)]
#[path = "tests_vision_sizes.rs"]
mod tests;
