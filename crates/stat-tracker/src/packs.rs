//! Reader packs from `GET /api/tracker/packs`.
//!
//! The list is a JSON array of `name`, `version`, `sha256`, and `size`.
//! The tracker installs every pack in [`PACKS`], each into its own folder
//! under `<data_dir>/templates/`. Each pack's sha256 and size come from that
//! signed-in list, not from a hash pinned in this crate. The daemon token is
//! the same bearer used for stat uploads. Each file is a plain ustar archive
//! with one top-level `manifest.json`.
//!
//! The hero pack [`HEROES_PACK_NAME`] lands in
//! [`crate::shadow::heroes::HeroTemplates::dir_in`] as a top-level
//! `<hero-key>.png` or `special/<class>*.png`, the layout
//! [`HeroTemplates::load_dir`] reads. `heroes-v2.tar` and
//! `heroes-v2-1080.tar` use the same layout; the new reader prefers them
//! over heroes-v1 when complete ([`crate::shadow::heroes::HeroSet`]).
//! `maps-v1.tar` is the new reader's map banner pack
//! ([`crate::shadow::banner::MapDirs`]) and `words-v1.tar` its result word
//! pack ([`crate::shadow::result::ResultDirs`]). `names-v1.tar` is the
//! name reader's letter pack ([`crate::names::pack::dir_in`]). `bans-v1.tar`
//! is downloaded into a folder no reader opens yet.
//! Each pack has its own [`PackLayout`]; any path outside it refuses that
//! pack only.
//!
//! Size and sha256 are checked against the list before anything is
//! unpacked. Each pack is staged and swapped into place on its own, so one
//! bad or failed pack never blocks, rolls back, or deletes another. A pack
//! the server does not list keeps whatever is installed. The token is never
//! written to a log.
//!
//! Daemon startup ([`sync_on_startup`]) syncs `heroes-v1.tar` only and hands
//! its result to [`PackKeeper`], which runs in the background, so capture
//! never waits on the other packs. The keeper syncs them and, while the
//! daemon runs, keeps trying whatever is not in place: after a busy lock, a
//! 429, packs switched off, a network error or a refused download (the
//! waits are the table on `Backoff`). It reuses the list it has, sends one
//! request every 10 s once the site has answered 429, and downloads a pack
//! it refused for a fixed reason once. The new reader starts on heroes-v1 and
//! re-loads its hero set once heroes-v2 or heroes-v2-1080 is swapped in
//! ([`crate::shadow::heroes::LiveHeroSet`]), and loads maps-v1 the same way
//! once it lands ([`crate::shadow::banner::LiveMaps`]). The name reader
//! loads names-v1 ([`crate::names::LiveNames`]) on a background thread at
//! startup when the folder is there, and again each time the keeper
//! installs a names-v1 pack ([`PackKeeper::on_installed`]): that reload
//! checks the installed files against the manifest sync verified and swaps
//! the pack in only when they match. The daemon does not wait. It reads
//! with the Tesseract rows already in hand until a load finishes. The setup
//! guide ([`sync_reader_packs`]) syncs all of them. Every run
//! holds an exclusive `flock` on `templates/.pack-sync.lock`; a second run
//! at the same time is [`PackSync::Busy`] and sends no request.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::config::validate_sync_server_url;
use crate::shadow::heroes::HeroTemplates;

/// `Retry-After` waits stay inside this range. The setup guide uses this
/// same clamp before it adds a wait to a clock.
pub const RETRY_AFTER_MIN_SECS: u64 = 1;
pub const RETRY_AFTER_MAX_SECS: u64 = 3600;
/// Used when a 429 names no wait.
pub const RATE_LIMIT_FALLBACK_SECS: u64 = 10;
/// Hard cap while streaming a pack. The list's `size` is not this cap.
pub const PACK_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// File name of the hero template pack on the pack list.
pub const HEROES_PACK_NAME: &str = "heroes-v1.tar";

/// File name of the hero portrait pack the new reader reads at 1440p. To
/// move the tracker to a newer build (say `heroes-v3.tar`), change this
/// name, [`HEROES_PORTRAIT_1080_PACK_NAME`], and the folders in
/// `shadow::heroes` (`TEMPLATE_SUBDIR_V2`, `TEMPLATE_SUBDIR_V2_1080`).
pub const HEROES_PORTRAIT_PACK_NAME: &str = "heroes-v2.tar";

/// File name of the hero portrait pack the new reader reads at 1080p.
pub const HEROES_PORTRAIT_1080_PACK_NAME: &str = "heroes-v2-1080.tar";

/// What may sit inside one pack. Paths are checked before anything is
/// written, and a path outside the layout refuses that pack only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackLayout {
    /// `heroes-v1.tar`, unchanged from alpha.6: one top-level file, or one
    /// file in `special/`.
    HeroesV1,
    /// Hero portraits: top-level `<hero-key>.png` or `special/<name>.png`.
    Portraits,
    /// Ban icons: top-level `<hero-key>.png` only.
    BanIcons,
    /// Map-name glyphs: `atlas.json` and `g<4 hex digits>.png`.
    MapGlyphs,
    /// Word art: top-level `<word_id>.png` and `<name>.json`.
    Words,
    /// Letter templates: the six files [`crate::names::pack`] loads, and
    /// nothing else. No subdirectory.
    Names,
}

/// One pack the tracker installs.
#[derive(Debug, Clone, Copy)]
pub struct PackSpec {
    /// Exact `name` on the server list and in the inner manifest.
    pub name: &'static str,
    /// Folder under the data dir this pack is unpacked into.
    pub dir: fn(&Path) -> PathBuf,
    pub layout: PackLayout,
    /// When the server lists packs but not this one, report a mismatch
    /// instead of [`PackSync::Missing`]. Only `heroes-v1.tar`, as in alpha.6.
    pub required: bool,
}

impl PackSpec {
    pub fn dir_in(&self, data_dir: &Path) -> PathBuf {
        (self.dir)(data_dir)
    }
}

/// Every pack the tracker syncs, in download order. `heroes-v1.tar` goes
/// first so a rate limit or refused token hits it the way it did in alpha.6.
/// The hero packs are read by a reader today (heroes-v1 by the shadow log and
/// as the new reader's fallback, heroes-v2 and heroes-v2-1080 by the new
/// reader), and so are maps-v1 (the new reader's map banner), words-v1 (its
/// result words) and names-v1 (the name reader's letter pack). bans-v1 is
/// not opened by a reader yet.
pub const PACKS: &[PackSpec] = &[
    PackSpec {
        name: HEROES_PACK_NAME,
        dir: HeroTemplates::dir_in,
        layout: PackLayout::HeroesV1,
        required: true,
    },
    PackSpec {
        name: HEROES_PORTRAIT_PACK_NAME,
        dir: |data_dir| data_dir.join(crate::shadow::heroes::TEMPLATE_SUBDIR_V2),
        layout: PackLayout::Portraits,
        required: false,
    },
    PackSpec {
        name: HEROES_PORTRAIT_1080_PACK_NAME,
        dir: |data_dir| data_dir.join(crate::shadow::heroes::TEMPLATE_SUBDIR_V2_1080),
        layout: PackLayout::Portraits,
        required: false,
    },
    PackSpec {
        name: "bans-v1.tar",
        dir: |data_dir| data_dir.join("templates").join("bans-v1"),
        layout: PackLayout::BanIcons,
        required: false,
    },
    PackSpec {
        name: "maps-v1.tar",
        dir: |data_dir| data_dir.join("templates").join("maps-v1"),
        layout: PackLayout::MapGlyphs,
        required: false,
    },
    PackSpec {
        name: "words-v1.tar",
        dir: |data_dir| data_dir.join("templates").join("words-v1"),
        layout: PackLayout::Words,
        required: false,
    },
    PackSpec {
        name: crate::names::pack::PACK_NAME,
        dir: crate::names::pack::dir_in,
        layout: PackLayout::Names,
        required: false,
    },
];

/// The spec for `name`, if the tracker installs that pack.
pub fn pack_spec(name: &str) -> Option<&'static PackSpec> {
    PACKS.iter().find(|spec| spec.name == name)
}

pub const PACK_SAVED: &str = "Reader pack saved.";
pub const PACK_CURRENT: &str = "The reader pack is already installed.";
pub const PACK_UNAVAILABLE: &str = "Reader packs aren't set up on the server yet. The tracker still works. Try again later or ask an admin.";
pub const PACK_UNSUPPORTED: &str = "This server does not offer reader packs yet.";
pub const PACK_NOT_LISTED: &str = "The site does not offer this reader pack yet.";
pub const PACK_TOO_BIG: &str = "The reader pack is too large to save.";
pub const PACK_MISMATCH: &str =
    "The reader pack did not match the site. The installed pack was left in place.";
pub const PACK_UNSAFE: &str =
    "The reader pack was not safe to unpack. The installed pack was left in place.";
pub const PACK_FAILED: &str =
    "Could not download the reader pack. The installed pack was left in place.";
pub const PACK_AUTH: &str = "Your sync token isn't valid. Sign in again from Settings.";
pub const PACK_FORBIDDEN: &str = "The site refused the reader pack (403). Check you're signed in with a member account. The installed pack was left in place.";
pub const PACK_SIGN_IN: &str = "Sign in first. Then the tracker can download the reader pack.";
pub const PACK_BUSY: &str = "A reader pack sync is already running. Try again in a moment.";

/// A huge `Retry-After` must not be added to a clock.
pub fn clamp_retry_after(seconds: u64) -> u64 {
    seconds.clamp(RETRY_AFTER_MIN_SECS, RETRY_AFTER_MAX_SECS)
}

/// Header seconds when present, otherwise [`RATE_LIMIT_FALLBACK_SECS`], then clamped.
pub fn rate_limit_seconds(retry_after: Option<u64>) -> u64 {
    clamp_retry_after(retry_after.unwrap_or(RATE_LIMIT_FALLBACK_SECS))
}

/// Start of every [`rate_limit_message`].
const RATE_LIMIT_PREFIX: &str = "Too many tries, wait ";

pub fn rate_limit_message(seconds: u64) -> String {
    let seconds = clamp_retry_after(seconds);
    format!("{RATE_LIMIT_PREFIX}{seconds} seconds and try again")
}

/// Outcome for one pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackSync {
    Installed,
    Current,
    /// HTTP 503 whose body is `{"error":"packs_disabled"}`. Callers keep the
    /// installed files. A proxy 502 or 503 is [`PackSync::Failed`] instead.
    Unavailable,
    /// HTTP 404 on the list. This server does not have the route yet.
    Unsupported,
    /// The server list does not name this optional pack. Its installed
    /// folder, if any, is kept.
    Missing,
    /// Another sync (the daemon or the app) holds the pack sync lock. This
    /// run changed nothing and sent no request. Not a failure.
    Busy,
    Failed(String),
}

impl PackSync {
    pub fn guide_result(self) -> Result<String, String> {
        match self {
            Self::Installed => Ok(PACK_SAVED.to_string()),
            Self::Current => Ok(PACK_CURRENT.to_string()),
            Self::Unavailable => Ok(PACK_UNAVAILABLE.to_string()),
            Self::Unsupported => Ok(PACK_UNSUPPORTED.to_string()),
            Self::Missing => Ok(PACK_NOT_LISTED.to_string()),
            Self::Busy => Ok(PACK_BUSY.to_string()),
            Self::Failed(message) => Err(message),
        }
    }

    fn is_ready(&self) -> bool {
        matches!(self, Self::Installed | Self::Current)
    }

    /// The rate-limit sentence, when this is a 429 failure.
    fn rate_limited(&self) -> Option<&str> {
        match self {
            Self::Failed(message) if message.starts_with(RATE_LIMIT_PREFIX) => Some(message),
            _ => None,
        }
    }

    /// The seconds a 429 asked for, when this is a 429 failure. Read back
    /// from [`rate_limit_message`], the one place a run keeps that wait.
    fn rate_limit_secs(&self) -> Option<u64> {
        self.rate_limited()?
            .strip_prefix(RATE_LIMIT_PREFIX)?
            .split(' ')
            .next()?
            .parse()
            .ok()
    }

    /// A failure the same bytes would get again; see [`is_fixed_refusal`].
    fn is_fixed_refusal(&self) -> bool {
        matches!(self, Self::Failed(message) if is_fixed_refusal(message))
    }

    /// Nothing a later try would change: in place, not offered by the
    /// site, or a server without reader packs.
    fn is_settled(&self) -> bool {
        matches!(
            self,
            Self::Installed | Self::Current | Self::Missing | Self::Unsupported
        )
    }
}

/// One line of a [`PackReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackStatus {
    pub name: &'static str,
    pub outcome: PackSync,
}

/// Outcome of one sync: one entry per pack the run covered, in [`PACKS`]
/// order. [`sync_reader_packs`] covers all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackReport {
    pub packs: Vec<PackStatus>,
}

/// Reported for a pack a run did not cover.
static NOT_COVERED: PackSync = PackSync::Missing;

impl PackReport {
    /// The same outcome for every pack in `scope`, for failures before any
    /// pack is looked at (sign-in, the lock, list errors).
    fn all(scope: Scope, outcome: PackSync) -> Self {
        Self {
            packs: scope
                .specs()
                .map(|spec| PackStatus {
                    name: spec.name,
                    outcome: outcome.clone(),
                })
                .collect(),
        }
    }

    /// This run's packs, then the packs of the run after it.
    fn followed_by(mut self, next: PackReport) -> Self {
        self.packs.extend(next.packs);
        self
    }

    pub fn get(&self, name: &str) -> Option<&PackSync> {
        self.packs
            .iter()
            .find(|status| status.name == name)
            .map(|status| &status.outcome)
    }

    /// Outcome for `heroes-v1.tar`, the pack the hero reader uses.
    /// [`PackSync::Missing`] when this run did not cover it.
    pub fn heroes(&self) -> &PackSync {
        self.get(HEROES_PACK_NAME).unwrap_or(&NOT_COVERED)
    }

    /// `(ready, listed)` for the packs other than `heroes-v1.tar`. A pack
    /// is listed when the server named it, and ready when it is installed
    /// or already current.
    pub fn extras_ready(&self) -> (usize, usize) {
        let extras = self
            .packs
            .iter()
            .filter(|status| status.name != HEROES_PACK_NAME);
        let mut ready = 0;
        let mut listed = 0;
        for status in extras {
            if status.outcome != PackSync::Missing {
                listed += 1;
            }
            if status.outcome.is_ready() {
                ready += 1;
            }
        }
        (ready, listed)
    }

    /// The setup guide line: the `heroes-v1.tar` message, plus a count of
    /// the other packs when the hero pack is ready and the server lists any
    /// of them, plus the rate-limit sentence when one of them hit a 429. A
    /// server that lists only `heroes-v1.tar` gets the alpha.6 message
    /// unchanged.
    pub fn guide_result(self) -> Result<String, String> {
        let heroes = self.heroes().clone();
        let ready_heroes = heroes.is_ready();
        let (ready, listed) = self.extras_ready();
        let message = heroes.guide_result()?;
        if !(ready_heroes && listed > 0) {
            return Ok(message);
        }
        let mut line = format!("{message} {}", extra_packs_message(ready, listed));
        let limited = self
            .packs
            .iter()
            .filter(|status| status.name != HEROES_PACK_NAME)
            .find_map(|status| status.outcome.rate_limited());
        if let Some(limited) = limited {
            line.push(' ');
            line.push_str(limited);
            line.push('.');
        }
        Ok(line)
    }
}

pub fn extra_packs_message(ready: usize, listed: usize) -> String {
    format!("Extra reader packs ready: {ready} of {listed}.")
}

/// One pack a run swapped into place, for [`PackKeeper::on_installed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPack {
    /// List name, such as `names-v1.tar`.
    pub name: &'static str,
    /// The version the list and the inner manifest both gave.
    pub version: String,
    /// sha256 of the inner `manifest.json` this install checked every file
    /// against. A reader that loads the folder again can check the manifest
    /// it finds on disk is that same one.
    pub manifest_sha256: String,
}

/// Called by [`PackKeeper`] for every pack it installs.
type InstalledHook = Box<dyn Fn(&InstalledPack) + Send>;

/// Which packs one run covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    All,
    HeroesOnly,
    ExtrasOnly,
}

impl Scope {
    fn specs(self) -> impl Iterator<Item = &'static PackSpec> {
        PACKS.iter().filter(move |spec| match self {
            Self::All => true,
            Self::HeroesOnly => spec.name == HEROES_PACK_NAME,
            Self::ExtrasOnly => spec.name != HEROES_PACK_NAME,
        })
    }
}

/// Log each pack a run installed.
fn log_installed(report: &PackReport) {
    for status in &report.packs {
        if status.outcome == PackSync::Installed {
            tracing::info!(pack = status.name, "reader pack installed");
        }
    }
}

/// Daemon startup. Syncs `heroes-v1.tar` only, the same two requests as
/// alpha.6, so capture waits on nothing new. The result goes to
/// [`PackKeeper`], which the daemon runs in the background once capture is
/// up: it syncs the other packs and keeps trying whatever is not in place.
/// Only an install is logged here. What went wrong, and that there will be
/// another try, is the keeper's line to write. Never logs the token.
pub async fn sync_on_startup(server_url: &str, token: &str, data_dir: &Path) -> StartupSync {
    sync_on_startup_capped(server_url, token, data_dir, PACK_MAX_BYTES).await
}

async fn sync_on_startup_capped(
    server_url: &str,
    token: &str,
    data_dir: &Path,
    max_bytes: u64,
) -> StartupSync {
    let run = run_sync(
        server_url,
        token,
        data_dir,
        max_bytes,
        Scope::HeroesOnly,
        RunPlan::default(),
    )
    .await;
    log_installed(&run.report);
    StartupSync {
        server_url: server_url.to_string(),
        run,
    }
}

/// What [`sync_on_startup`] came to, for [`PackKeeper`] to carry on from:
/// the heroes-v1 outcome and, when it was read, the list.
pub struct StartupSync {
    /// The server the list is from.
    server_url: String,
    run: SyncRun,
}

impl StartupSync {
    /// Startup read the list and no request hit a stop (refused token, rate
    /// limit, packs switched off). The keeper's first try then goes on with
    /// the other packs at once. Otherwise it starts with a wait.
    fn goes_on(&self) -> bool {
        self.run.entries.is_some() && !self.run.stopped
    }
}

/// Waits, in seconds, while another process holds the pack sync lock. The
/// last one repeats.
const BUSY_WAITS_SECS: [u64; 3] = [5, 15, 60];
/// Waits, in seconds, after packs switched off (503), a network error, a
/// refused token or a refused download. The last one repeats.
const FAILED_WAITS_SECS: [u64; 4] = [60, 300, 900, 1800];
/// The least wait, in seconds, after the first, second, third... 429 since
/// a pack last arrived. The server's own figure counts when it is longer.
/// The last one repeats.
const LIMITED_WAITS_SECS: [u64; 6] = [0, 10, 20, 40, 80, 120];
/// No wait between two tries is shorter than this.
const RETRY_FLOOR_SECS: u64 = 5;
/// The longest wait the keeper takes from a 429's own figure.
const RATE_LIMIT_RETRY_MAX_SECS: u64 = 120;
/// Once the site has answered 429, the job sends one request per try and
/// waits this long between tries. The site gives one request back every
/// 10 s (`pack_governor_config` in site-server), so the job then takes no
/// more than comes back, and other trackers behind the same address get
/// their share.
const PACED_GAP_SECS: u64 = 10;
/// A kept list this old is read again before the next download.
const LIST_MAX_AGE_SECS: u64 = 30 * 60;

/// `waits[tries]` as a wait, or the last one once `tries` is past the end.
fn nth_wait(waits: &[u64], tries: usize) -> Duration {
    let last = waits.len().saturating_sub(1);
    Duration::from_secs(waits.get(tries.min(last)).copied().unwrap_or(0))
}

/// Added to a 429 wait, so trackers behind one address (they share the
/// server's rate limit) do not all come back in the same second. Fixed for
/// one process: 0 to 4.75 s in quarter seconds, from the pid.
fn pid_jitter(pid: u32) -> Duration {
    Duration::from_millis(u64::from(pid % 20) * 250)
}

/// True for a refusal the same bytes would get again: wrong size or
/// sha256, a layout the tracker does not accept, or too large.
fn is_fixed_refusal(message: &str) -> bool {
    message == PACK_MISMATCH || message == PACK_UNSAFE || message == PACK_TOO_BIG
}

/// Downloads the tracker refused for a fixed reason in this daemon run, by
/// pack name: the list entry that was refused and the message. A pack is
/// not downloaded again while the list still shows that same entry.
type Refused = BTreeMap<String, (PackListEntry, String)>;

/// The message `entry` was refused with, when it is the entry that was
/// refused before.
fn known_refusal(refused: Option<&Refused>, entry: &PackListEntry) -> Option<String> {
    let (was, message) = refused?.get(&entry.name)?;
    (was == entry).then(|| message.clone())
}

/// What one try left undone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Verdict {
    /// Another process held the pack sync lock.
    busy: bool,
    /// A 429, with the seconds it asked for.
    limited: Option<u64>,
    /// The site has packs switched off (503 `packs_disabled`).
    disabled: bool,
    /// Something that may pass by itself: the network, a refused token.
    failed: bool,
    /// A download refused for a fixed reason ([`is_fixed_refusal`]). It is
    /// not fetched again until the list shows another entry for it.
    refused: bool,
    /// Downloads left for the next try because the job is paced
    /// ([`PACED_GAP_SECS`]).
    held: bool,
}

/// The one reason a wait is logged with, when a try had several.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
    Failed,
    Disabled,
    Limited,
    Busy,
    /// Nothing went wrong: the next download is due after the paced gap.
    Paced,
}

impl Verdict {
    fn of(report: &PackReport) -> Self {
        let mut verdict = Self::default();
        for status in &report.packs {
            match &status.outcome {
                outcome if outcome.is_settled() => {}
                PackSync::Busy => verdict.busy = true,
                PackSync::Unavailable => verdict.disabled = true,
                outcome => match outcome.rate_limit_secs() {
                    Some(seconds) => verdict.limited = verdict.limited.max(Some(seconds)),
                    None if outcome.is_fixed_refusal() => verdict.refused = true,
                    None => verdict.failed = true,
                },
            }
        }
        verdict
    }

    /// Every pack is settled: there is nothing to try again.
    fn settled(&self) -> bool {
        *self == Self::default()
    }

    fn why(&self) -> Why {
        match self {
            Self { failed: true, .. } => Why::Failed,
            Self { disabled: true, .. } => Why::Disabled,
            Self {
                limited: Some(_), ..
            } => Why::Limited,
            Self { busy: true, .. } => Why::Busy,
            Self { held: true, .. } => Why::Paced,
            // Only a pack refused for a fixed reason is left.
            _ => Why::Failed,
        }
    }
}

/// How long [`PackKeeper`] waits between two tries.
///
/// | The try ended on | Wait before the next try |
/// | --- | --- |
/// | another process holds the lock | 5 s, 15 s, then 60 s each time |
/// | a 429 | the server's `Retry-After`, kept to 1..=120 s, and from the second 429 since a pack last arrived at least 10, 20, 40, 80, then 120 s; plus [`pid_jitter`] |
/// | downloads held back while paced | [`PACED_GAP_SECS`] |
/// | packs switched off (503), network error, refused token, a pack refused for a fixed reason | 1 min, 5 min, 15 min, then 30 min each time |
///
/// No wait is under [`RETRY_FLOOR_SECS`]. A try that ended on several of
/// these waits the longest of them, with one exception: while downloads
/// are held back, a pack that is known to be refused does not slow them
/// down (it is not fetched anyway). The busy step starts over once the
/// lock was free. The 429 step starts over when a pack arrives
/// ([`Backoff::pack_arrived`]), not when only the list got through: a
/// tracker that can read the list and is refused every download still
/// backs off to one request in two minutes. The failure step never starts
/// over while the daemon runs.
#[derive(Debug, Default)]
struct Backoff {
    busy_tries: usize,
    limited_tries: usize,
    failed_tries: usize,
}

impl Backoff {
    /// A download got through and was installed: the next 429 is the first
    /// one again.
    fn pack_arrived(&mut self) {
        self.limited_tries = 0;
    }

    /// The wait before the next try, or `None` when nothing is left to try.
    fn wait_after(&mut self, verdict: Verdict, jitter: Duration) -> Option<Duration> {
        if verdict.settled() {
            return None;
        }
        let mut wait = Duration::from_secs(RETRY_FLOOR_SECS);
        if verdict.busy {
            wait = wait.max(nth_wait(&BUSY_WAITS_SECS, self.busy_tries));
            self.busy_tries += 1;
        } else {
            self.busy_tries = 0;
        }
        if let Some(seconds) = verdict.limited {
            let asked = seconds.clamp(RETRY_AFTER_MIN_SECS, RATE_LIMIT_RETRY_MAX_SECS);
            let least = nth_wait(&LIMITED_WAITS_SECS, self.limited_tries);
            wait = wait.max(Duration::from_secs(asked).max(least) + jitter);
            self.limited_tries += 1;
        }
        if verdict.held {
            wait = wait.max(Duration::from_secs(PACED_GAP_SECS));
        }
        if verdict.failed || verdict.disabled || (verdict.refused && !verdict.held) {
            wait = wait.max(nth_wait(&FAILED_WAITS_SECS, self.failed_tries));
            self.failed_tries += 1;
        }
        Some(wait)
    }
}

/// The packs that are not on disk and that this try did not settle. A
/// pack the try did not cover counts (startup looks at heroes-v1 only, a
/// paced try at one pack); a pack the site does not offer does not.
/// Blocking (it reads the versions file); see [`missing_off_runtime`].
fn missing_packs(data_dir: &Path, report: &PackReport) -> Vec<&'static str> {
    let versions = read_versions(data_dir);
    PACKS
        .iter()
        .filter(|spec| !(versions.contains_key(spec.name) && in_place(data_dir, spec)))
        .filter(|spec| !report.get(spec.name).is_some_and(PackSync::is_settled))
        .map(|spec| spec.name)
        .collect()
}

/// [`missing_packs`] on the blocking pool. Empty if that work panicked.
async fn missing_off_runtime(data_dir: PathBuf, report: PackReport) -> Vec<&'static str> {
    off_runtime(move || missing_packs(&data_dir, &report))
        .await
        .unwrap_or_default()
}

/// `a, b, c`, or `None` for no names, which leaves the field out of the line.
fn name_list(names: &[&str]) -> Option<String> {
    (!names.is_empty()).then(|| names.join(", "))
}

/// [`PackKeeper`]'s log lines. A wait is written when its reason or the
/// set of missing packs changes, not on every try, so hours without the
/// site are one line. The lines carry pack names, the wait and a fixed
/// sentence: never the server URL, a header or the token.
#[derive(Default)]
struct Voice {
    said: Option<(Why, Vec<&'static str>)>,
    arrived: Vec<&'static str>,
    waits: u32,
}

impl Voice {
    /// Note the packs a try installed. `true` when there was one.
    fn arrivals(&mut self, report: &PackReport) -> bool {
        let before = self.arrived.len();
        let installed = report
            .packs
            .iter()
            .filter(|status| status.outcome == PackSync::Installed);
        self.arrived.extend(installed.map(|status| status.name));
        self.arrived.len() > before
    }

    fn waiting(
        &mut self,
        report: &PackReport,
        missing: Vec<&'static str>,
        verdict: Verdict,
        wait: Duration,
    ) {
        self.waits += 1;
        let why = verdict.why();
        let wait_secs = wait.as_secs();
        if why == Why::Paced {
            // Not news: the 429 line said the tracker is waiting, and the
            // closing line names what arrived.
            tracing::debug!(wait_secs, "reader packs: next download after the paced gap");
            return;
        }
        if self
            .said
            .as_ref()
            .is_some_and(|(said, packs)| *said == why && *packs == missing)
        {
            tracing::debug!(wait_secs, "reader packs: nothing new, trying again later");
            return;
        }
        let names = name_list(&missing);
        let names = names.as_deref();
        match why {
            Why::Paced => {}
            Why::Busy => tracing::info!(
                wait_secs,
                "another reader pack sync is running; the tracker will try again"
            ),
            Why::Limited => tracing::info!(
                wait_secs,
                missing = names,
                "the site asked the tracker to wait before it downloads more reader packs; the tracker will try again"
            ),
            Why::Disabled => tracing::info!(
                wait_secs,
                missing = names,
                "the site has reader pack downloads turned off; the tracker will try again"
            ),
            Why::Failed => {
                let failed: Vec<(&str, &str)> = report
                    .packs
                    .iter()
                    .filter_map(|status| match &status.outcome {
                        PackSync::Failed(message) if status.outcome.rate_limit_secs().is_none() => {
                            Some((status.name, message.as_str()))
                        }
                        _ => None,
                    })
                    .collect();
                let reason = failed.first().map_or(PACK_FAILED, |(_, message)| *message);
                // When every pack carries the failure it came before any
                // pack was looked at (the list, the sign-in). Naming them
                // all would name packs that are installed and fine.
                let own: Vec<&str> = if failed.len() < report.packs.len() {
                    failed.iter().map(|(name, _)| *name).collect()
                } else {
                    Vec::new()
                };
                let refused = name_list(&own);
                let refused = refused.as_deref();
                if missing.is_empty() {
                    tracing::warn!(
                        wait_secs,
                        refused,
                        reason,
                        "reader packs could not be checked for updates; the tracker keeps the installed packs and will try again"
                    );
                } else {
                    tracing::warn!(
                        wait_secs,
                        missing = names,
                        refused,
                        reason,
                        "reader packs are still missing; the tracker will try again"
                    );
                }
            }
        }
        self.said = Some((why, missing));
    }

    fn done(&self, report: &PackReport) {
        for status in &report.packs {
            if status.outcome == PackSync::Missing {
                tracing::debug!(pack = status.name, "reader pack not listed by the server");
            }
        }
        let unsupported = |status: &PackStatus| status.outcome == PackSync::Unsupported;
        if report.packs.iter().any(unsupported) {
            tracing::info!("reader packs are not supported on this server yet");
        }
        if self.waits > 0 {
            let arrived = name_list(&self.arrived);
            tracing::info!(
                tries = self.waits + 1,
                arrived = arrived.as_deref(),
                "reader pack sync finished; the tracker stopped retrying"
            );
        }
    }
}

/// The server URL and token for one try. It has no `Debug`, so the token
/// cannot reach a log.
pub struct PackCredentials {
    pub server_url: String,
    pub token: String,
}

/// What [`PackKeeper`]'s first try came to, sent once on the channel from
/// [`PackKeeper::first_try_done`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstTry {
    /// The packs that are still not on disk, by list name. Empty when
    /// every pack is in place or not offered by the site.
    pub missing: Vec<&'static str>,
    /// The wait before the job's next try, or `None` when the job has
    /// ended. A short wait (a 429, a busy lock, the paced gap) means packs
    /// can still arrive soon; a wait of a minute or more means they will
    /// not.
    pub next_try_in: Option<Duration>,
}

type Nap = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// A list kept from one try for the tries after it.
struct KeptList {
    /// The server it is from. A list is not used for another server.
    server_url: String,
    entries: Vec<PackListEntry>,
    /// How long ago it was read, counted in the job's own waits.
    age: Duration,
}

/// One try as the job's loop sees it.
struct Tried {
    report: PackReport,
    /// Downloads left for the next try by the paced limit.
    held_back: usize,
}

impl Tried {
    fn verdict(&self) -> Verdict {
        Verdict {
            held: self.held_back > 0,
            ..Verdict::of(&self.report)
        }
    }
}

/// The daemon's one background job for reader packs: make every pack in
/// [`PACKS`] current, and keep trying while the daemon runs.
///
/// Its first try carries on from [`sync_on_startup`]: with that run's list
/// it syncs the other packs, so a first install is still eight requests
/// (the list, then every pack).
/// If startup did not get that far (busy lock, 429, packs switched off, a
/// network error, a refused token) nothing is sent now and that outcome
/// sets the first wait.
///
/// A later try downloads only the packs that are missing or out of date.
/// It reuses the list it has: the list is read again only when it is
/// [`LIST_MAX_AGE_SECS`] old, or when a download showed it is out of date
/// (a listed pack was a 404, or its bytes did not match the listed size
/// and sha256). So a try after a 429 is the refused download and nothing
/// before it.
///
/// Once the site has answered 429 the job is paced for the rest of its
/// life: one request per try, [`PACED_GAP_SECS`] apart.
///
/// A pack refused for a fixed reason (size, sha256, layout) is downloaded
/// once. It is fetched again only when the list shows another entry for
/// it, or after a restart.
///
/// The job stops once every pack is in place or not offered; until then
/// it waits as `Backoff` says and tries again, one try at a time. Each try
/// takes the pack sync lock, so the setup guide and a second process still
/// take turns with it.
///
/// Capture never waits on it. Dropping it (the daemon stopping) between
/// tries leaves nothing behind; a download in flight is dropped before
/// anything is written.
pub struct PackKeeper {
    data_dir: PathBuf,
    /// The startup outcome, and whether the first try goes on from it at
    /// once ([`StartupSync::goes_on`]).
    startup: Option<(PackReport, bool)>,
    credentials: Box<dyn Fn() -> Option<PackCredentials> + Send>,
    max_bytes: u64,
    jitter: Duration,
    sleep: Box<dyn FnMut(Duration) -> Nap + Send>,
    list: Option<KeptList>,
    refused: Refused,
    /// The site has answered 429: one request per try from here on.
    paced: bool,
    first_try: Option<tokio::sync::oneshot::Sender<FirstTry>>,
    on_installed: Option<InstalledHook>,
}

impl PackKeeper {
    /// `startup` is `None` when no startup sync ran. `credentials` is asked
    /// before every try and gives the sync client's current URL and token;
    /// while it gives `None` (sync paused on a rejected token) nothing is
    /// sent.
    pub fn new(
        data_dir: &Path,
        startup: Option<StartupSync>,
        credentials: impl Fn() -> Option<PackCredentials> + Send + 'static,
    ) -> Self {
        let mut keeper = Self {
            data_dir: data_dir.to_path_buf(),
            startup: None,
            credentials: Box::new(credentials),
            max_bytes: PACK_MAX_BYTES,
            jitter: pid_jitter(std::process::id()),
            sleep: Box::new(|wait| Box::pin(tokio::time::sleep(wait))),
            list: None,
            refused: Refused::new(),
            paced: false,
            first_try: None,
            on_installed: None,
        };
        if let Some(startup) = startup {
            let goes_on = startup.goes_on();
            let StartupSync {
                server_url,
                mut run,
            } = startup;
            keeper.learn(&server_url, &mut run, Duration::ZERO);
            keeper.startup = Some((run.report, goes_on));
        }
        keeper
    }

    /// True when startup read the pack list and hit no stop, so the first
    /// try downloads the other packs at once. False when the first thing
    /// this job does is wait (startup got no list, was refused, or did not
    /// run): a caller that wants to give the downloads a moment has
    /// nothing to wait for then.
    pub fn starts_with_downloads(&self) -> bool {
        matches!(self.startup, Some((_, true)))
    }

    /// A channel that gets one [`FirstTry`] when the first try is over,
    /// whatever it came to. Call it before [`Self::run`]. `run` itself
    /// returns only when every pack is settled, which can be hours later,
    /// so a caller that holds something up for the downloads (startup with
    /// `ocr = "off"` asked for) waits on this and not on the task:
    ///
    /// ```text
    /// let first = job.first_try_done();
    /// tokio::spawn(job.run());
    /// let first = tokio::time::timeout(PACK_WAIT, first).await;
    /// ```
    ///
    /// The channel closes without a message if the job is dropped first.
    /// A second call replaces the first channel.
    pub fn first_try_done(&mut self) -> tokio::sync::oneshot::Receiver<FirstTry> {
        let (done, first) = tokio::sync::oneshot::channel();
        self.first_try = Some(done);
        first
    }

    /// Run `hook` for every pack this job installs, right after it is
    /// swapped into place and its version is stored. The daemon uses it to
    /// reload the name reader once names-v1 lands, so a first start does not
    /// need a second one. `hook` runs on the job's task: it must hand any
    /// slow work to a thread of its own. Call it before [`Self::run`]. A
    /// second call replaces the first hook.
    pub fn on_installed(&mut self, hook: impl Fn(&InstalledPack) + Send + 'static) {
        self.on_installed = Some(Box::new(hook));
    }

    /// Meant for `tokio::spawn`. Returns once every pack is settled.
    pub async fn run(mut self) {
        let mut backoff = Backoff::default();
        let mut voice = Voice::default();
        let mut first_try = self.first_try.take();
        let mut tried = self.first_pass().await;
        loop {
            if voice.arrivals(&tried.report) {
                backoff.pack_arrived();
            }
            let verdict = tried.verdict();
            let wait = backoff.wait_after(verdict, self.jitter);
            let missing = if wait.is_some() || first_try.is_some() {
                missing_off_runtime(self.data_dir.clone(), tried.report.clone()).await
            } else {
                Vec::new()
            };
            if let Some(first) = first_try.take() {
                // Nobody listening is fine.
                let _ = first.send(FirstTry {
                    missing: missing.clone(),
                    next_try_in: wait,
                });
            }
            let Some(wait) = wait else {
                voice.done(&tried.report);
                return;
            };
            voice.waiting(&tried.report, missing, verdict, wait);
            (self.sleep)(wait).await;
            if let Some(list) = &mut self.list {
                list.age += wait;
            }
            tried = self.attempt(Scope::All).await;
        }
    }

    /// The startup outcome plus, when the first try goes on from it at
    /// once, the packs startup did not cover.
    async fn first_pass(&mut self) -> Tried {
        match self.startup.take() {
            Some((report, true)) => {
                let rest = self.attempt(Scope::ExtrasOnly).await;
                Tried {
                    report: report.followed_by(rest.report),
                    held_back: rest.held_back,
                }
            }
            Some((report, false)) => Tried {
                report,
                held_back: 0,
            },
            None => self.attempt(Scope::All).await,
        }
    }

    async fn attempt(&mut self, scope: Scope) -> Tried {
        let Some(credentials) = (self.credentials)() else {
            return Tried {
                report: PackReport::all(scope, PackSync::Failed(PACK_SIGN_IN.to_string())),
                held_back: 0,
            };
        };
        let max_age = Duration::from_secs(LIST_MAX_AGE_SECS);
        let kept = self
            .list
            .take()
            .filter(|list| list.server_url == credentials.server_url && list.age < max_age);
        let age = kept.as_ref().map_or(Duration::ZERO, |list| list.age);
        let listed = kept.map(|list| list.entries);
        let plan = RunPlan {
            listed,
            max_requests: self.paced.then_some(1),
            refused: Some(&self.refused),
        };
        let mut run = run_sync(
            &credentials.server_url,
            &credentials.token,
            &self.data_dir,
            self.max_bytes,
            scope,
            plan,
        )
        .await;
        log_installed(&run.report);
        if let Some(hook) = &self.on_installed {
            for installed in &run.installed {
                hook(installed);
            }
        }
        self.learn(&credentials.server_url, &mut run, age);
        Tried {
            report: run.report,
            held_back: run.held_back,
        }
    }

    /// Keep what a run found out for the tries after it: its list (unless
    /// a download showed the list is out of date), what it refused, and
    /// that the site said to slow down. `age` is the age of the list the
    /// run was handed.
    fn learn(&mut self, server_url: &str, run: &mut SyncRun, age: Duration) {
        self.list = match run.entries.take() {
            Some(entries) if !run.stale => Some(KeptList {
                server_url: server_url.to_string(),
                entries,
                age: if run.read_list { Duration::ZERO } else { age },
            }),
            _ => None,
        };
        for (entry, message) in run.refused.drain(..) {
            self.refused.insert(entry.name.clone(), (entry, message));
        }
        if Verdict::of(&run.report).limited.is_some() {
            self.paced = true;
        }
    }
}

/// Every pack, as the setup guide's "Download reader pack" runs it.
pub async fn sync_reader_packs(
    server_url: &str,
    token: &str,
    data_dir: &Path,
    max_bytes: u64,
) -> PackReport {
    run_sync(
        server_url,
        token,
        data_dir,
        max_bytes,
        Scope::All,
        RunPlan::default(),
    )
    .await
    .report
}

/// What one run may reuse and how far it may go. The default is a plain
/// run: read the list, then download every pack that is not current.
#[derive(Default)]
struct RunPlan<'a> {
    /// A list read earlier, used instead of a list request.
    listed: Option<Vec<PackListEntry>>,
    /// The most requests this run may send; the list request counts.
    /// `None` is no limit.
    max_requests: Option<usize>,
    /// Downloads refused before; see [`known_refusal`].
    refused: Option<&'a Refused>,
}

/// What one run came to.
struct SyncRun {
    /// One entry per pack the run looked at, in [`PACKS`] order. A pack
    /// left for later by [`RunPlan::max_requests`] has no entry.
    report: PackReport,
    /// The list the run had: read now, or handed in. `None` when it had
    /// none (the list request failed, or the run ended before it).
    entries: Option<Vec<PackListEntry>>,
    /// This run sent the list request and got the list.
    read_list: bool,
    /// A download hit a stop (refused token, rate limit, packs switched
    /// off) and the packs after it were not tried.
    stopped: bool,
    /// A download showed the list is out of date: a listed pack was a 404,
    /// or its bytes did not match the listed size and sha256.
    stale: bool,
    /// Packs that need a download and were left for a later run by
    /// [`RunPlan::max_requests`].
    held_back: usize,
    /// Downloads this run refused for a fixed reason.
    refused: Vec<(PackListEntry, String)>,
    /// Packs this run swapped into place, in [`PACKS`] order.
    installed: Vec<InstalledPack>,
}

impl SyncRun {
    /// A run that ended before it looked at a pack: every pack in `scope`
    /// gets `outcome`. `listed` is the list it was handed, given back
    /// unused.
    fn early(scope: Scope, outcome: PackSync, listed: Option<Vec<PackListEntry>>) -> Self {
        Self {
            report: PackReport::all(scope, outcome),
            entries: listed,
            read_list: false,
            stopped: false,
            stale: false,
            held_back: 0,
            refused: Vec::new(),
            installed: Vec::new(),
        }
    }
}

/// Take one request off a budget. `false` when none is left.
fn take_request(left: &mut Option<usize>) -> bool {
    match left {
        None => true,
        Some(0) => false,
        Some(left) => {
            *left -= 1;
            true
        }
    }
}

/// One run over `scope`, as `plan` allows.
async fn run_sync(
    server_url: &str,
    token: &str,
    data_dir: &Path,
    max_bytes: u64,
    scope: Scope,
    plan: RunPlan<'_>,
) -> SyncRun {
    if token.trim().is_empty() || server_url.trim().is_empty() {
        let outcome = PackSync::Failed(PACK_SIGN_IN.to_string());
        return SyncRun::early(scope, outcome, plan.listed);
    }
    if validate_sync_server_url(server_url).is_err() {
        let outcome = PackSync::Failed(PACK_FAILED.to_string());
        return SyncRun::early(scope, outcome, plan.listed);
    }
    // One sync at a time across processes, taken before any request. The
    // daemon and the app share pack-versions.json and the request budget.
    let _lock = match SyncLock::try_acquire(data_dir) {
        Ok(Some(lock)) => lock,
        Ok(None) => return SyncRun::early(scope, PackSync::Busy, plan.listed),
        Err(err) => {
            tracing::warn!(error = %err, "reader pack sync lock was not taken");
            let outcome = PackSync::Failed(PACK_FAILED.to_string());
            return SyncRun::early(scope, outcome, plan.listed);
        }
    };
    sweep_stale_staging(data_dir);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            let outcome = PackSync::Failed(PACK_FAILED.to_string());
            return SyncRun::early(scope, outcome, plan.listed);
        }
    };
    let base = server_url.trim().trim_end_matches('/');
    let mut requests_left = plan.max_requests;
    let (entries, read_list) = match plan.listed {
        Some(entries) => (entries, false),
        None => match fetch_list(&client, base, token).await {
            Ok(entries) => {
                take_request(&mut requests_left);
                (entries, true)
            }
            Err(outcome) => return SyncRun::early(scope, outcome, None),
        },
    };
    let fetch = Fetch {
        client: &client,
        base,
        token,
        max_bytes,
    };
    // Read under the lock, so a run never writes back a stale map.
    let mut versions = read_versions(data_dir);
    let mut packs = Vec::new();
    // A refused token, a rate limit, or packs switched off applies to every
    // download, so the packs after it are not tried. Their folders stay.
    let mut stopped: Option<PackSync> = None;
    let mut stale = false;
    let mut held_back = 0;
    let mut refused = Vec::new();
    let mut installed = Vec::new();
    for spec in scope.specs() {
        let entry = match select_pack(&entries, spec) {
            Ok(entry) => entry,
            Err(outcome) => {
                packs.push(PackStatus {
                    name: spec.name,
                    outcome,
                });
                continue;
            }
        };
        // Checked before the stop, so a pack that is already in place is
        // reported as current, not as the stop's failure.
        let outcome = if is_current(data_dir, spec, entry, &versions) {
            PackSync::Current
        } else if let Some(message) = known_refusal(plan.refused, entry) {
            PackSync::Failed(message)
        } else if let Some(stop) = &stopped {
            stop.clone()
        } else if !take_request(&mut requests_left) {
            held_back += 1;
            continue;
        } else {
            let outcome = match sync_one(&fetch, data_dir, spec, entry).await {
                Step::Installed(manifest_sha256) => {
                    installed.push(InstalledPack {
                        name: spec.name,
                        version: entry.version.clone(),
                        manifest_sha256,
                    });
                    PackSync::Installed
                }
                Step::Done(outcome) => outcome,
                Step::Gone => {
                    stale = true;
                    PackSync::Failed(PACK_FAILED.to_string())
                }
                Step::Stop(outcome) => {
                    stopped = Some(outcome.clone());
                    outcome
                }
            };
            match &outcome {
                PackSync::Installed => {
                    // Keep the other packs' versions, including an alpha.6
                    // file that only names heroes-v1.tar.
                    versions.insert(spec.name.to_string(), entry.version.clone());
                    let (dir, snapshot) = (data_dir.to_path_buf(), versions.clone());
                    let written = off_runtime(move || write_versions(&dir, &snapshot))
                        .await
                        .unwrap_or_else(|| Err("version write did not finish".to_string()));
                    if let Err(message) = written {
                        tracing::warn!(
                            pack = spec.name,
                            error = %message,
                            "reader pack version was not stored"
                        );
                    }
                }
                PackSync::Failed(message) if is_fixed_refusal(message) => {
                    // The bytes did not match what the list says: the list
                    // may have moved on since it was read.
                    stale |= message == PACK_MISMATCH;
                    refused.push((entry.clone(), message.clone()));
                }
                _ => {}
            }
            outcome
        };
        packs.push(PackStatus {
            name: spec.name,
            outcome,
        });
    }
    SyncRun {
        report: PackReport { packs },
        entries: Some(entries),
        read_list,
        stopped: stopped.is_some(),
        stale,
        held_back,
        refused,
        installed,
    }
}

async fn fetch_list(
    client: &reqwest::Client,
    base: &str,
    token: &str,
) -> Result<Vec<PackListEntry>, PackSync> {
    let listed = match get_bytes(
        client,
        &format!("{base}/api/tracker/packs"),
        token,
        64 * 1024,
    )
    .await
    {
        Get::Ok(bytes) => bytes,
        Get::Unavailable => return Err(PackSync::Unavailable),
        Get::Unsupported => return Err(PackSync::Unsupported),
        Get::Auth => return Err(PackSync::Failed(PACK_AUTH.to_string())),
        Get::Forbidden => return Err(PackSync::Failed(PACK_FORBIDDEN.to_string())),
        Get::Limited(seconds) => return Err(PackSync::Failed(rate_limit_message(seconds))),
        Get::Failed(message) => return Err(PackSync::Failed(message)),
    };
    serde_json::from_slice(&listed).map_err(|_| PackSync::Failed(PACK_MISMATCH.to_string()))
}

/// The recorded version matches the list and the folder is there.
/// pack-versions.json can outlive a deleted pack folder; then it downloads.
fn is_current(
    data_dir: &Path,
    spec: &PackSpec,
    entry: &PackListEntry,
    installed: &BTreeMap<String, String>,
) -> bool {
    installed
        .get(spec.name)
        .is_some_and(|version| version == &entry.version)
        && in_place(data_dir, spec)
}

/// The pack's folder is there, with its `manifest.json` when the layout
/// keeps one ([`PackLayout::keeps_manifest`]). 0.5.0-rc.1 installed
/// names-v1 without its manifest, which the name reader refuses; that
/// folder is not in place, so the next sync downloads the pack again.
fn in_place(data_dir: &Path, spec: &PackSpec) -> bool {
    let dir = spec.dir_in(data_dir);
    dir.is_dir()
        && (!spec.layout.keeps_manifest()
            || dir
                .join("manifest.json")
                .symlink_metadata()
                .is_ok_and(|meta| meta.is_file()))
}

/// File locked for the length of one sync, so the daemon and the app never
/// sync at the same time.
fn sync_lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join(".pack-sync.lock")
}

/// Exclusive `flock` on [`sync_lock_path`]. Released when dropped, or when
/// the process dies.
///
/// Drop unlocks with `LOCK_UN` before the descriptor is closed. `open`
/// already sets `O_CLOEXEC`, so a child drops the inherited descriptor at
/// exec. `close` alone still leaves the lock held until that exec, and a
/// keeper that tries again in that window sees [`PackSync::Busy`].
/// `F_OFD_SETLK` is the same kind of lock: the child shares it until exec
/// or an explicit unlock.
struct SyncLock {
    _file: std::fs::File,
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // SAFETY: this descriptor is the one `try_acquire` locked, and it
        // is still open: `Drop` runs before the `File` field is dropped.
        // `LOCK_UN` removes that flock from the open file description,
        // including the copy a child inherited across `fork`.
        unsafe {
            libc::flock(self._file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

impl SyncLock {
    /// `Ok(None)` when another sync holds it. Never waits.
    fn try_acquire(data_dir: &Path) -> std::io::Result<Option<Self>> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let path = sync_lock_path(data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)?;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            return Ok(Some(Self { _file: file }));
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            Ok(None)
        } else {
            Err(err)
        }
    }

    /// `true` when this descriptor will be closed in a child at exec.
    #[cfg(test)]
    fn is_cloexec(&self) -> bool {
        use std::os::fd::AsRawFd;
        // SAFETY: `fd` is the descriptor `try_acquire` opened, still open.
        let flags = unsafe { libc::fcntl(self._file.as_raw_fd(), libc::F_GETFD) };
        flags >= 0 && (flags & libc::FD_CLOEXEC) != 0
    }
}

/// A `.<pack>-next-*` or `.<pack>-prev-*` folder this old is left from a
/// sync that was killed. Live syncs hold the lock, so none of theirs is here.
const STALE_STAGING: std::time::Duration = std::time::Duration::from_secs(3600);

/// Remove staging and backup folders left by a killed sync. Runs under the
/// sync lock. Only names this module makes, only real folders (not links),
/// and only when older than [`STALE_STAGING`]. Also removes versions tmp
/// files left by a kill.
fn sweep_stale_staging(data_dir: &Path) {
    let templates = data_dir.join("templates");
    let Ok(entries) = std::fs::read_dir(&templates) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        if name.starts_with(VERSIONS_TMP_PREFIX) {
            // Only a run holding this lock writes one, so this one is left.
            if path.symlink_metadata().is_ok_and(|meta| meta.is_file())
                && let Err(err) = std::fs::remove_file(&path)
            {
                tracing::warn!(file = %name, error = %err, "left reader pack versions file was not removed");
            }
            continue;
        }
        // The pack this name belongs to, and whether it is a `-prev-` backup.
        let owner = name.strip_prefix('.').and_then(|rest| {
            PACKS.iter().find_map(|spec| {
                let tail = rest.strip_prefix(dir_label(&spec.dir_in(data_dir)).as_str())?;
                if tail.starts_with("-next-") {
                    Some((spec, false))
                } else if tail.starts_with("-prev-") {
                    Some((spec, true))
                } else {
                    None
                }
            })
        });
        let Some((spec, backup)) = owner else {
            continue;
        };
        // A `-prev-` folder is the old live folder renamed, so its mtime is
        // the old pack's. While the live folder is missing (a kill inside
        // the fallback swap) it is the only copy of that pack: keep it.
        if backup && !spec.dir_in(data_dir).exists() {
            continue;
        }
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        let stale = meta
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= STALE_STAGING);
        if meta.is_dir() && stale {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    tracing::info!(folder = %name, "removed a reader pack folder left by a stopped sync")
                }
                Err(err) => {
                    tracing::warn!(folder = %name, error = %err, "left reader pack folder was not removed")
                }
            }
        }
    }
}

/// [`sync_one`] result. `Stop` also ends the loop for the packs after it.
enum Step {
    /// Swapped into place. Carries the sha256 of the inner manifest.
    Installed(String),
    Done(PackSync),
    /// The site listed the pack and then had no such file (404). The pack
    /// failed, and the list is out of date.
    Gone,
    Stop(PackSync),
}

/// What every pack download in one sync shares.
struct Fetch<'a> {
    client: &'a reqwest::Client,
    base: &'a str,
    token: &'a str,
    max_bytes: u64,
}

async fn sync_one(
    fetch: &Fetch<'_>,
    data_dir: &Path,
    spec: &'static PackSpec,
    entry: &PackListEntry,
) -> Step {
    let parent = pack_parent(data_dir, spec);
    if let Err(message) = std::fs::create_dir_all(&parent) {
        tracing::warn!(pack = spec.name, error = %message, "reader pack directory was not created");
        return Step::Done(PackSync::Failed(PACK_FAILED.to_string()));
    }
    let bytes = match get_bytes(
        fetch.client,
        &format!("{}/api/tracker/packs/{}", fetch.base, spec.name),
        fetch.token,
        fetch.max_bytes,
    )
    .await
    {
        Get::Ok(bytes) => bytes,
        Get::Unavailable => return Step::Stop(PackSync::Unavailable),
        Get::Unsupported => return Step::Gone,
        Get::Auth => return Step::Stop(PackSync::Failed(PACK_AUTH.to_string())),
        Get::Forbidden => return Step::Stop(PackSync::Failed(PACK_FORBIDDEN.to_string())),
        Get::Limited(seconds) => {
            return Step::Stop(PackSync::Failed(rate_limit_message(seconds)));
        }
        Get::Failed(message) => return Step::Done(PackSync::Failed(message)),
    };
    let (data_dir, entry) = (data_dir.to_path_buf(), entry.clone());
    let installed = off_runtime(move || install_pack(&data_dir, spec, &entry, &bytes))
        .await
        .unwrap_or_else(|| Err(PackSync::Failed(PACK_FAILED.to_string())));
    match installed {
        Ok(manifest_sha256) => Step::Installed(manifest_sha256),
        Err(outcome) => Step::Done(outcome),
    }
}

/// Run blocking file work (hashing, unpacking, fsync) on tokio's blocking
/// pool, so the background pack job never holds up a runtime worker.
/// `None` if the work panicked.
async fn off_runtime<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work).await.ok()
}

/// The file side of one download: check the bytes against the list, unpack
/// into a staging folder and swap it in. Blocking; see [`off_runtime`].
/// `Ok` carries the sha256 of the inner manifest; an `Err` is the
/// [`PackSync::Failed`] to report.
fn install_pack(
    data_dir: &Path,
    spec: &PackSpec,
    entry: &PackListEntry,
    bytes: &[u8],
) -> Result<String, PackSync> {
    #[cfg(test)]
    tests::note_blocking_work(data_dir);
    let live = spec.dir_in(data_dir);
    let parent = pack_parent(data_dir, spec);
    // List size and sha256 are checked before any file is written.
    if bytes.len() as u64 != entry.size || sha256_hex(bytes) != entry.sha256 {
        return Err(PackSync::Failed(PACK_MISMATCH.to_string()));
    }
    let staging = parent.join(format!(".{}-next-{}", dir_label(&live), unique_suffix()));
    if std::fs::create_dir(&staging).is_err() {
        return Err(PackSync::Failed(PACK_FAILED.to_string()));
    }
    let manifest_sha256 =
        match unpack_pack(bytes, spec.layout, &entry.name, &entry.version, &staging) {
            Ok(sha) => sha,
            Err(message) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(PackSync::Failed(message.to_string()));
            }
        };
    if let Err(message) = swap_dir(&staging, &live) {
        let _ = std::fs::remove_dir_all(&staging);
        tracing::warn!(pack = spec.name, error = %message, "reader pack was not swapped into place");
        return Err(PackSync::Failed(PACK_FAILED.to_string()));
    }
    Ok(manifest_sha256)
}

/// The folder a pack's live, staging and backup folders sit in.
fn pack_parent(data_dir: &Path, spec: &PackSpec) -> PathBuf {
    spec.dir_in(data_dir)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| data_dir.to_path_buf())
}

/// Last path part of a pack folder, for staging and backup names.
fn dir_label(live: &Path) -> String {
    live.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "pack".to_string())
}

fn unique_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    )
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct PackListEntry {
    name: String,
    version: String,
    sha256: String,
    size: u64,
}

/// The list entry named exactly `spec.name`. Other names are ignored. An
/// empty list means there is nothing to install. A required pack that a
/// non-empty list leaves out is a mismatch; an optional one is
/// [`PackSync::Missing`].
fn select_pack<'a>(
    entries: &'a [PackListEntry],
    spec: &PackSpec,
) -> Result<&'a PackListEntry, PackSync> {
    let Some(entry) = entries.iter().find(|entry| entry.name == spec.name) else {
        return Err(if !spec.required {
            PackSync::Missing
        } else if entries.is_empty() {
            PackSync::Current
        } else {
            PackSync::Failed(PACK_MISMATCH.to_string())
        });
    };
    if !is_sha256(&entry.sha256) || !is_version(&entry.version) {
        return Err(PackSync::Failed(PACK_MISMATCH.to_string()));
    }
    Ok(entry)
}

enum Get {
    Ok(Vec<u8>),
    Unavailable,
    Unsupported,
    Auth,
    Forbidden,
    Limited(u64),
    Failed(String),
}

async fn get_bytes(client: &reqwest::Client, url: &str, token: &str, max_bytes: u64) -> Get {
    let response = match client.get(url).bearer_auth(token).send().await {
        Ok(response) => response,
        Err(_) => return Get::Failed(PACK_FAILED.to_string()),
    };
    let status = response.status().as_u16();
    if status == 401 {
        return Get::Auth;
    }
    if status == 403 {
        return Get::Forbidden;
    }
    if status == 404 {
        return Get::Unsupported;
    }
    if status == 502 || status == 503 {
        let body = error_body(response).await;
        if status == 503 && packs_disabled_body(&body) {
            return Get::Unavailable;
        }
        return Get::Failed(PACK_FAILED.to_string());
    }
    if status == 429 {
        let header = retry_after_header(response.headers());
        let body = error_body(response).await;
        let from_json = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value.get("retry_after").and_then(|item| item.as_u64()));
        return Get::Limited(rate_limit_seconds(header.or(from_json)));
    }
    if !response.status().is_success() {
        return Get::Failed(PACK_FAILED.to_string());
    }
    match read_body_capped(response, max_bytes).await {
        Ok(bytes) => Get::Ok(bytes),
        Err(message) => Get::Failed(message.to_string()),
    }
}

/// Largest error body read for `packs_disabled` or `retry_after`. A proxy
/// page or a stalled body past this is not read.
const ERROR_BODY_MAX: u64 = 4 * 1024;

async fn error_body(response: reqwest::Response) -> String {
    read_body_capped(response, ERROR_BODY_MAX)
        .await
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// True only for the site's packs-disabled body. A proxy page is not this.
fn packs_disabled_body(body: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    value.get("error").and_then(|error| error.as_str()) == Some("packs_disabled")
}

fn retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse().ok())
}

async fn read_body_capped(
    mut response: reqwest::Response,
    max_bytes: u64,
) -> Result<Vec<u8>, &'static str> {
    if response.content_length().is_some_and(|len| len > max_bytes) {
        return Err(PACK_TOO_BIG);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| PACK_FAILED)? {
        let next = (body.len() as u64).saturating_add(chunk.len() as u64);
        if next > max_bytes {
            return Err(PACK_TOO_BIG);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn unpack_pack(
    bytes: &[u8],
    layout: PackLayout,
    expected_name: &str,
    expected_version: &str,
    staging: &Path,
) -> Result<String, &'static str> {
    let files = read_ustar(bytes, layout)?;
    if files
        .iter()
        .filter(|(path, _)| path == "manifest.json")
        .count()
        != 1
    {
        return Err(PACK_MISMATCH);
    }
    let manifest_bytes = files
        .iter()
        .find(|(path, _)| path == "manifest.json")
        .map(|(_, data)| data.as_slice())
        .ok_or(PACK_MISMATCH)?;
    let manifest: InnerManifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| PACK_MISMATCH)?;
    if manifest.name != expected_name || manifest.version != expected_version {
        return Err(PACK_MISMATCH);
    }
    if manifest.files.is_empty() {
        return Err(PACK_MISMATCH);
    }
    let mut listed = BTreeSet::new();
    for file in &manifest.files {
        let path = layout.file_path(&file.path)?;
        if path == "manifest.json" || !listed.insert(path) {
            return Err(PACK_UNSAFE);
        }
        if !is_sha256(&file.sha256) {
            return Err(PACK_MISMATCH);
        }
    }
    let mut seen = BTreeSet::new();
    for (path, data) in &files {
        if path == "manifest.json" {
            continue;
        }
        if !seen.insert(path.clone()) {
            return Err(PACK_UNSAFE);
        }
        let listed_file = manifest
            .files
            .iter()
            .find(|file| layout.file_path(&file.path).ok().as_deref() == Some(path.as_str()))
            .ok_or(PACK_MISMATCH)?;
        if data.len() as u64 != listed_file.size || sha256_hex(data) != listed_file.sha256 {
            return Err(PACK_MISMATCH);
        }
        let dest = staging.join(path);
        if !dest.starts_with(staging) {
            return Err(PACK_UNSAFE);
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|_| PACK_FAILED)?;
        }
        if dest.symlink_metadata().is_ok() {
            return Err(PACK_UNSAFE);
        }
        let mut file = std::fs::File::create(&dest).map_err(|_| PACK_FAILED)?;
        file.write_all(data).map_err(|_| PACK_FAILED)?;
        file.sync_all().map_err(|_| PACK_FAILED)?;
    }
    if seen != listed {
        return Err(PACK_MISMATCH);
    }
    if layout.keeps_manifest() {
        // The bytes checked above, so the reader sees the manifest every
        // file was just verified against.
        let dest = staging.join("manifest.json");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .map_err(|_| PACK_FAILED)?;
        file.write_all(manifest_bytes).map_err(|_| PACK_FAILED)?;
        file.sync_all().map_err(|_| PACK_FAILED)?;
    }
    Ok(sha256_hex(manifest_bytes))
}

#[derive(Debug, serde::Deserialize)]
struct InnerManifest {
    name: String,
    version: String,
    files: Vec<InnerFile>,
}

#[derive(Debug, serde::Deserialize)]
struct InnerFile {
    path: String,
    sha256: String,
    size: u64,
}

pub(crate) fn swap_dir(staging: &Path, live: &Path) -> Result<(), String> {
    let _ = sync_dir(staging);
    if live.exists() {
        match exchange_paths(staging, live) {
            Ok(()) => {
                if let Err(err) = std::fs::remove_dir_all(staging) {
                    tracing::warn!(
                        error = %err,
                        "previous reader pack was left beside the new one"
                    );
                }
                let _ = sync_dir(live.parent().unwrap_or(live));
                return Ok(());
            }
            Err(err) => {
                tracing::warn!(error = %err, "atomic pack exchange was not available");
            }
        }
    } else {
        std::fs::rename(staging, live).map_err(|err| err.to_string())?;
        let _ = sync_dir(live.parent().unwrap_or(live));
        return Ok(());
    }

    let parent = live.parent().unwrap_or_else(|| Path::new("."));
    let backup = parent.join(format!(".{}-prev-{}", dir_label(live), unique_suffix()));
    std::fs::rename(live, &backup).map_err(|err| err.to_string())?;
    if let Err(err) = std::fs::rename(staging, live) {
        let _ = std::fs::rename(&backup, live);
        return Err(err.to_string());
    }
    let _ = std::fs::remove_dir_all(&backup);
    let _ = sync_dir(parent);
    Ok(())
}

fn sync_dir(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

/// Swap two directory entries in one syscall when the kernel allows it.
/// After this returns, `live` is the new tree and `staging` is the old one.
fn exchange_paths(staging: &Path, live: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let staging_c = std::ffi::CString::new(staging.as_os_str().as_bytes())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
    let live_c = std::ffi::CString::new(live.as_os_str().as_bytes())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
    let rc = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            staging_c.as_ptr(),
            libc::AT_FDCWD,
            live_c.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn versions_path(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join("pack-versions.json")
}

fn read_versions(data_dir: &Path) -> BTreeMap<String, String> {
    let Ok(bytes) = std::fs::read(versions_path(data_dir)) else {
        return BTreeMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Start of every tmp file [`write_versions`] makes. Only a run holding the
/// sync lock writes one, so any found at the start of a run was left by a kill.
const VERSIONS_TMP_PREFIX: &str = "pack-versions.json.tmp-";

fn write_versions(data_dir: &Path, versions: &BTreeMap<String, String>) -> Result<(), String> {
    #[cfg(test)]
    tests::note_blocking_work(data_dir);
    let path = versions_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    // A name of its own, so two writers never share one tmp file.
    let tmp = path.with_file_name(format!("{VERSIONS_TMP_PREFIX}{}", unique_suffix()));
    let bytes = serde_json::to_vec(versions).map_err(|err| err.to_string())?;
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)
    })();
    if let Err(err) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.to_string());
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_version(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// heroes-v1: a template is one file next to `manifest.json`, or one file in
/// `special/`. Unchanged from alpha.6.
fn heroes_v1_file_path(raw: &str) -> Result<String, &'static str> {
    let path = safe_rel_path(raw).ok_or(PACK_UNSAFE)?;
    let mut parts = path.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(_), None, None) => Ok(path),
        (Some("special"), Some(file), None) if !file.is_empty() => Ok(path),
        _ => Err(PACK_UNSAFE),
    }
}

/// Longest file stem the strict layouts accept.
const MAX_STEM: usize = 64;

/// `<stem><ext>` where the stem is 1 to [`MAX_STEM`] bytes that all pass `allowed`.
fn is_plain_name(raw: &str, ext: &str, allowed: fn(u8) -> bool) -> bool {
    raw.strip_suffix(ext)
        .is_some_and(|stem| !stem.is_empty() && stem.len() <= MAX_STEM && stem.bytes().all(allowed))
}

/// Hero keys as in `HERO_PACK_FILES`: lowercase, digits, `_` and `-`.
fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
}

/// Word ids: lowercase, digits and `_`.
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
}

/// A word id the words-v1 layout allows as a file stem: 1 to 64 of `a-z`,
/// `0-9`, `_`. No separator, no `..`, never absolute. The result reader
/// checks layout ids from a pack's `layouts.json` with this before it joins
/// them into a path.
pub(crate) fn is_word_id(stem: &str) -> bool {
    !stem.is_empty() && stem.len() <= MAX_STEM && stem.bytes().all(is_word_byte)
}

/// Template files of names-v1, besides `manifest.json`. Those six plus the
/// manifest are the seven names this layout stores. No other name is stored.
fn is_names_v1_file(raw: &str) -> bool {
    matches!(
        raw,
        "glyphs.png" | "glyphs.txt" | "marks.png" | "marks.txt" | "compose.txt" | "whiten.f32"
    )
}

/// `g` plus four lowercase hex digits, then `.png`.
pub(crate) fn is_glyph_name(raw: &str) -> bool {
    raw.strip_prefix('g')
        .and_then(|rest| rest.strip_suffix(".png"))
        .is_some_and(|hex| {
            hex.len() == 4
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
}

impl PackLayout {
    /// `true` when [`unpack_pack`] stores the inner `manifest.json` in the
    /// pack folder, because the reader needs it. Only names-v1: its loader
    /// ([`crate::names::pack`]) reads the canvas, the glyph and mark counts,
    /// the params and every file's sha256 from it. The other layouts keep
    /// alpha.6's folders, without the manifest.
    pub fn keeps_manifest(self) -> bool {
        matches!(self, Self::Names)
    }

    /// The path a file is stored under, or [`PACK_UNSAFE`] when the layout
    /// does not allow it. `manifest.json` is accepted at the top level for
    /// every layout; [`unpack_pack`] reads it and writes it only for a
    /// layout that [`Self::keeps_manifest`].
    fn file_path(self, raw: &str) -> Result<String, &'static str> {
        if self == Self::HeroesV1 {
            return heroes_v1_file_path(raw);
        }
        if raw == "manifest.json" {
            return Ok(raw.to_string());
        }
        let ok = match self {
            Self::HeroesV1 => false,
            Self::Portraits => match raw.strip_prefix("special/") {
                Some(file) => is_plain_name(file, ".png", is_key_byte),
                None => is_plain_name(raw, ".png", is_key_byte),
            },
            Self::BanIcons => is_plain_name(raw, ".png", is_key_byte),
            Self::MapGlyphs => raw == "atlas.json" || is_glyph_name(raw),
            Self::Words => {
                is_plain_name(raw, ".png", is_word_byte)
                    || is_plain_name(raw, ".json", is_word_byte)
            }
            Self::Names => is_names_v1_file(raw),
        };
        if ok {
            Ok(raw.to_string())
        } else {
            Err(PACK_UNSAFE)
        }
    }

    /// The one directory entry a layout may carry: `special/` for hero packs.
    fn allows_dir(self, raw: &str) -> bool {
        match self {
            Self::HeroesV1 | Self::Portraits => safe_rel_path(raw).as_deref() == Some("special"),
            Self::BanIcons | Self::MapGlyphs | Self::Words | Self::Names => false,
        }
    }
}

fn safe_rel_path(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.starts_with('/') || raw.starts_with('\\') {
        return None;
    }
    if raw.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains('\0') {
            return None;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn read_ustar(bytes: &[u8], layout: PackLayout) -> Result<Vec<(String, Vec<u8>)>, &'static str> {
    let mut offset = 0usize;
    let mut files = Vec::new();
    while offset + 512 <= bytes.len() {
        let header = &bytes[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        if !checksum_ok(header) {
            return Err(PACK_UNSAFE);
        }
        let typeflag = header[156];
        if header[124] & 0x80 != 0 {
            return Err(PACK_UNSAFE);
        }
        let size = parse_octal(&header[124..136]).map_err(|_| PACK_UNSAFE)?;
        let name = entry_name(header).ok_or(PACK_UNSAFE)?;
        offset += 512;
        let data_end = offset.saturating_add(size as usize);
        if data_end > bytes.len() {
            return Err(PACK_UNSAFE);
        }
        let data = bytes[offset..data_end].to_vec();
        let padded = size.div_ceil(512) * 512;
        offset = offset.saturating_add(padded as usize);
        match typeflag {
            b'0' | b'\0' => {
                let path = layout.file_path(&name)?;
                files.push((path, data));
            }
            b'5' => {
                if !layout.allows_dir(&name) {
                    return Err(PACK_UNSAFE);
                }
            }
            _ => return Err(PACK_UNSAFE),
        }
    }
    if files.is_empty() {
        return Err(PACK_MISMATCH);
    }
    Ok(files)
}

fn entry_name(header: &[u8]) -> Option<String> {
    let name = c_string(&header[..100]);
    let prefix = c_string(&header[345..500]);
    let joined = if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    };
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn c_string(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn parse_octal(field: &[u8]) -> Result<u64, ()> {
    let mut value = 0u64;
    let mut saw = false;
    for &byte in field {
        if byte == 0 || byte == b' ' {
            if saw {
                break;
            }
            continue;
        }
        if !byte.is_ascii_digit() || byte > b'7' {
            return Err(());
        }
        saw = true;
        value = value
            .checked_mul(8)
            .ok_or(())?
            .checked_add(u64::from(byte - b'0'))
            .ok_or(())?;
    }
    Ok(value)
}

fn checksum_ok(header: &[u8]) -> bool {
    let Ok(recorded) = parse_octal(&header[148..156]) else {
        return false;
    };
    let sum = header.iter().enumerate().fold(0u64, |sum, (index, byte)| {
        sum + u64::from(if (148..156).contains(&index) {
            b' '
        } else {
            *byte
        })
    });
    sum == recorded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// The hero pack names the tracker asks the site for, in one place.
    #[test]
    fn hero_pack_names_are_the_listed_ones() {
        assert_eq!(HEROES_PACK_NAME, "heroes-v1.tar");
        assert_eq!(HEROES_PORTRAIT_PACK_NAME, "heroes-v2.tar");
        assert_eq!(HEROES_PORTRAIT_1080_PACK_NAME, "heroes-v2-1080.tar");
        for name in [
            HEROES_PACK_NAME,
            HEROES_PORTRAIT_PACK_NAME,
            HEROES_PORTRAIT_1080_PACK_NAME,
        ] {
            assert!(pack_spec(name).is_some(), "{name} is synced");
        }
        let data = Path::new("/d");
        assert_eq!(
            pack_spec(HEROES_PORTRAIT_PACK_NAME).unwrap().dir_in(data),
            data.join("templates").join("heroes-v2")
        );
        assert_eq!(
            pack_spec(HEROES_PORTRAIT_1080_PACK_NAME)
                .unwrap()
                .dir_in(data),
            data.join("templates").join("heroes-v2-1080")
        );
    }

    fn ustar_header(name: &str, size: u64, typeflag: u8) -> [u8; 512] {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(b"0000644");
        header[108..115].copy_from_slice(b"0000000");
        header[116..123].copy_from_slice(b"0000000");
        let size_field = format!("{size:011o}");
        header[124..135].copy_from_slice(size_field.as_bytes());
        header[136..147].copy_from_slice(b"00000000000");
        for byte in &mut header[148..156] {
            *byte = b' ';
        }
        header[156] = typeflag;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
        let checksum = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(checksum.as_bytes());
        header
    }

    fn ustar_file(name: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = ustar_header(name, data.len() as u64, b'0').to_vec();
        bytes.extend_from_slice(data);
        let pad = (512 - (data.len() % 512)) % 512;
        bytes.extend(std::iter::repeat_n(0u8, pad));
        bytes
    }

    fn pack_bytes(name: &str, version: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let listed: Vec<serde_json::Value> = files
            .iter()
            .map(|(path, data)| {
                serde_json::json!({
                    "path": path,
                    "sha256": sha256_hex(data),
                    "size": data.len(),
                })
            })
            .collect();
        let manifest = serde_json::json!({
            "name": name,
            "version": version,
            "files": listed,
        });
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let mut bytes = ustar_file("manifest.json", &manifest_bytes);
        for (path, data) in files {
            bytes.extend(ustar_file(path, data));
        }
        bytes.extend(std::iter::repeat_n(0u8, 1024));
        bytes
    }

    fn manifest_json(name: &str, version: &str, files: &[(&str, &str, u64)]) -> Vec<u8> {
        let listed: Vec<serde_json::Value> = files
            .iter()
            .map(|(path, sha, size)| {
                serde_json::json!({
                    "path": path,
                    "sha256": sha,
                    "size": size,
                })
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({
            "name": name,
            "version": version,
            "files": listed,
        }))
        .unwrap()
    }

    fn finish_tar(mut bytes: Vec<u8>) -> Vec<u8> {
        bytes.extend(std::iter::repeat_n(0u8, 1024));
        bytes
    }

    fn heroes_spec() -> &'static PackSpec {
        pack_spec(HEROES_PACK_NAME).expect("heroes-v1 spec")
    }

    fn unpack_result(bytes: &[u8]) -> Result<tempfile::TempDir, &'static str> {
        let dir = tempfile::tempdir().expect("tempdir");
        unpack_pack(
            bytes,
            PackLayout::HeroesV1,
            HEROES_PACK_NAME,
            "1",
            dir.path(),
        )
        .map(|_| dir)
    }

    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log").extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Fake {
        token: String,
        list_status: u16,
        list_body: Vec<u8>,
        list_retry: Option<u64>,
        file_status: u16,
        file_body: Vec<u8>,
        file_chunked: bool,
        file_hits: Arc<std::sync::atomic::AtomicUsize>,
        requested: Arc<Mutex<Vec<String>>>,
        /// Per-name `(status, body)` for `/api/tracker/packs/<name>`. A name
        /// not here gets `file_status` and `file_body`.
        named_files: BTreeMap<String, (u16, Vec<u8>)>,
    }

    /// Headers of one request. Blocks until the blank line. `None` when the
    /// peer closes before that, the next bytes do not arrive within 30
    /// seconds, or the head grows past 64 KiB.
    ///
    /// A shorter timeout already on `sock` is replaced with that 30 second
    /// wait. The fakes used to stop after 2 seconds and answer from whatever
    /// bytes had arrived. When the client was scheduled later than that, the
    /// fake answered 401, the keeper counted a failure, and a test that
    /// should have finished on the next try gave up instead. Thirty seconds
    /// is long enough for a loaded runner to send the header, and a client
    /// that connects and never writes fails instead of hanging the suite.
    fn read_http_head(sock: &mut std::net::TcpStream) -> Option<String> {
        sock.set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .expect("header wait");
        let mut buf = Vec::new();
        let mut tmp = [0u8; 2048];
        loop {
            match std::io::Read::read(sock, &mut tmp) {
                Ok(0) | Err(_) => return None,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                    if buf.len() > 64 * 1024 {
                        return None;
                    }
                }
            }
        }
        Some(String::from_utf8_lossy(&buf).into_owned())
    }

    #[test]
    fn a_split_request_header_is_not_refused_for_arriving_late() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (reading_tx, reading_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let _ = reading_tx.send(());
            // A caller that still had the old 2s budget, shrunk here so the
            // gap below is past it. `read_http_head` replaces this with 30s.
            sock.set_read_timeout(Some(std::time::Duration::from_millis(1)))
                .expect("timeout");
            let head = read_http_head(&mut sock).expect("full head");
            let authed = head
                .lines()
                .any(|line| line.eq_ignore_ascii_case("authorization: bearer pack-token"));
            let (status, body): (u16, &[u8]) = if authed { (200, b"ok") } else { (401, b"no") };
            let response = http_bytes(status, body, None, false);
            std::io::Write::write_all(&mut sock, &response).expect("write");
        });
        let mut client = std::net::TcpStream::connect(addr).expect("connect");
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("read timeout");
        client
            .set_write_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("write timeout");
        std::io::Write::write_all(&mut client, b"GET /api/tracker/packs HTTP/1.1\r\n")
            .expect("half");
        reading_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("server is in read");
        // Past the 1ms timeout set above. A helper that keeps that timeout
        // returns before this write, and the response is not 200.
        std::thread::sleep(std::time::Duration::from_millis(80));
        std::io::Write::write_all(&mut client, b"authorization: bearer pack-token\r\n\r\n")
            .expect("rest");
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut client, &mut buf).expect("response");
        let text = String::from_utf8_lossy(&buf);
        assert!(
            text.starts_with("HTTP/1.1 200 "),
            "a late header is the request, not a 401: {text}"
        );
        server.join().expect("server");
    }

    fn spawn_fake(fake: Arc<Fake>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            for _ in 0..64 {
                let Ok((mut sock, _)) = listener.accept() else {
                    break;
                };
                let Some(head) = read_http_head(&mut sock) else {
                    continue;
                };
                let authed = head.lines().any(|line| {
                    line.eq_ignore_ascii_case(&format!("authorization: bearer {}", fake.token))
                });
                let path = head
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("");
                fake.requested.lock().expect("paths").push(path.to_string());
                let response = if !authed {
                    http_bytes(401, br#"{"error":"Unauthorized"}"#, None, false)
                } else if path == "/api/tracker/packs" {
                    http_bytes(fake.list_status, &fake.list_body, fake.list_retry, false)
                } else if let Some(name) = path.strip_prefix("/api/tracker/packs/") {
                    fake.file_hits
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    match fake.named_files.get(name) {
                        Some((status, body)) => http_bytes(*status, body, None, false),
                        None => {
                            http_bytes(fake.file_status, &fake.file_body, None, fake.file_chunked)
                        }
                    }
                } else {
                    http_bytes(404, br#"{"error":"pack_not_found"}"#, None, false)
                };
                let _ = std::io::Write::write_all(&mut sock, &response);
            }
        });
        format!("http://{addr}")
    }

    fn http_bytes(status: u16, body: &[u8], retry_after: Option<u64>, chunked: bool) -> Vec<u8> {
        let reason = match status {
            200 => "OK",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            429 => "Too Many Requests",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            _ => "Error",
        };
        if chunked {
            let mut out = format!(
                "HTTP/1.1 {status} {reason}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            )
            .into_bytes();
            out.extend(format!("{:x}\r\n", body.len()).into_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\r\n0\r\n\r\n");
            return out;
        }
        let mut headers = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(seconds) = retry_after {
            headers.push_str(&format!("Retry-After: {seconds}\r\n"));
        }
        headers.push_str("\r\n");
        let mut out = headers.into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn list_json(name: &str, version: &str, bytes: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([{
            "name": name,
            "version": version,
            "sha256": sha256_hex(bytes),
            "size": bytes.len(),
        }]))
        .unwrap()
    }

    #[test]
    fn retry_after_clamp_matches_the_setup_guide_range() {
        assert_eq!(clamp_retry_after(0), 1);
        assert_eq!(clamp_retry_after(3601), 3600);
        assert_eq!(clamp_retry_after(u64::MAX), 3600);
        assert_eq!(rate_limit_seconds(Some(u64::MAX)), 3600);
        assert_eq!(
            rate_limit_message(u64::MAX),
            "Too many tries, wait 3600 seconds and try again"
        );
    }

    fn requested_paths() -> Arc<Mutex<Vec<String>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    #[test]
    fn list_sha256_is_taken_from_the_server_entry() {
        let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let raw =
            format!(r#"[{{"name":"heroes-v1.tar","version":"1","sha256":"{sha}","size":12}}]"#);
        let entries: Vec<PackListEntry> = serde_json::from_str(&raw).expect("list");
        let entry = select_pack(&entries, heroes_spec()).expect("named pack");
        assert_eq!(entry.name, HEROES_PACK_NAME);
        assert_eq!(entry.sha256, sha);
        assert_eq!(entry.size, 12);
        let other = vec![PackListEntry {
            name: "other.tar".into(),
            version: "1".into(),
            sha256: sha.into(),
            size: 12,
        }];
        assert_eq!(
            select_pack(&other, heroes_spec()).unwrap_err(),
            PackSync::Failed(PACK_MISMATCH.into())
        );
        assert_eq!(
            select_pack(&[], heroes_spec()).unwrap_err(),
            PackSync::Current
        );
        let named = vec![
            PackListEntry {
                name: "other.tar".into(),
                version: "9".into(),
                sha256: sha.into(),
                size: 4,
            },
            PackListEntry {
                name: HEROES_PACK_NAME.into(),
                version: "3".into(),
                sha256: sha.into(),
                size: 9,
            },
        ];
        let picked = select_pack(&named, heroes_spec()).expect("heroes by name");
        assert_eq!(picked.version, "3");
        assert_eq!(picked.size, 9);
        assert!(PACK_FORBIDDEN.contains("403"));
        assert!(!PACK_FORBIDDEN.contains('\u{2014}'));
        assert!(!PACK_FORBIDDEN.contains('\u{2013}'));
    }

    #[test]
    fn reader_layout_allows_special_and_rejects_other_folders() {
        let pixel = b"pixel";
        let ok = pack_bytes(
            HEROES_PACK_NAME,
            "1",
            &[("ana.png", pixel), ("special/x.png", pixel)],
        );
        let dir = unpack_result(&ok).expect("special/x.png");
        assert_eq!(
            std::fs::read(dir.path().join("special/x.png")).unwrap(),
            pixel
        );
        assert_eq!(std::fs::read(dir.path().join("ana.png")).unwrap(), pixel);

        let deep = pack_bytes(HEROES_PACK_NAME, "1", &[("special/a/b.png", pixel)]);
        assert_eq!(unpack_result(&deep).unwrap_err(), PACK_UNSAFE);
        let other = pack_bytes(HEROES_PACK_NAME, "1", &[("other/x.png", pixel)]);
        assert_eq!(unpack_result(&other).unwrap_err(), PACK_UNSAFE);
    }

    #[test]
    fn inner_hash_name_extra_missing_and_duplicates_are_rejected() {
        let pixel = b"pixel";
        let sha = sha256_hex(pixel);
        let wrong_sha = "ab".repeat(32);

        let bad_hash = finish_tar({
            let manifest = manifest_json(
                HEROES_PACK_NAME,
                "1",
                &[("ana.png", wrong_sha.as_str(), pixel.len() as u64)],
            );
            let mut bytes = ustar_file("manifest.json", &manifest);
            bytes.extend(ustar_file("ana.png", pixel));
            bytes
        });
        assert_eq!(unpack_result(&bad_hash).unwrap_err(), PACK_MISMATCH);

        let wrong_name = pack_bytes("other.tar", "1", &[("ana.png", pixel)]);
        assert_eq!(unpack_result(&wrong_name).unwrap_err(), PACK_MISMATCH);

        let extra = finish_tar({
            let manifest = manifest_json(
                HEROES_PACK_NAME,
                "1",
                &[("ana.png", sha.as_str(), pixel.len() as u64)],
            );
            let mut bytes = ustar_file("manifest.json", &manifest);
            bytes.extend(ustar_file("ana.png", pixel));
            bytes.extend(ustar_file("extra.png", b"more"));
            bytes
        });
        assert_eq!(unpack_result(&extra).unwrap_err(), PACK_MISMATCH);

        let missing = finish_tar({
            let side = b"side";
            let manifest = manifest_json(
                HEROES_PACK_NAME,
                "1",
                &[
                    ("ana.png", sha.as_str(), pixel.len() as u64),
                    ("side.png", sha256_hex(side).as_str(), side.len() as u64),
                ],
            );
            let mut bytes = ustar_file("manifest.json", &manifest);
            bytes.extend(ustar_file("ana.png", pixel));
            bytes
        });
        assert_eq!(unpack_result(&missing).unwrap_err(), PACK_MISMATCH);

        let duplicate_manifest = finish_tar({
            let manifest = manifest_json(
                HEROES_PACK_NAME,
                "1",
                &[
                    ("ana.png", sha.as_str(), pixel.len() as u64),
                    ("ana.png", sha.as_str(), pixel.len() as u64),
                ],
            );
            let mut bytes = ustar_file("manifest.json", &manifest);
            bytes.extend(ustar_file("ana.png", pixel));
            bytes
        });
        assert_eq!(unpack_result(&duplicate_manifest).unwrap_err(), PACK_UNSAFE);

        let duplicate_tar = finish_tar({
            let manifest = manifest_json(
                HEROES_PACK_NAME,
                "1",
                &[("ana.png", sha.as_str(), pixel.len() as u64)],
            );
            let mut bytes = ustar_file("manifest.json", &manifest);
            bytes.extend(ustar_file("ana.png", pixel));
            bytes.extend(ustar_file("ana.png", pixel));
            bytes
        });
        assert_eq!(unpack_result(&duplicate_tar).unwrap_err(), PACK_UNSAFE);

        let second_manifest = finish_tar({
            let manifest = manifest_json(
                HEROES_PACK_NAME,
                "1",
                &[("ana.png", sha.as_str(), pixel.len() as u64)],
            );
            let mut bytes = ustar_file("manifest.json", &manifest);
            bytes.extend(ustar_file("manifest.json", &manifest));
            bytes.extend(ustar_file("ana.png", pixel));
            bytes
        });
        assert_eq!(unpack_result(&second_manifest).unwrap_err(), PACK_MISMATCH);
    }

    #[test]
    fn ustar_rejects_absolute_paths_dotdot_and_links() {
        let escape = ustar_file("../escape.txt", b"nope");
        assert_eq!(
            read_ustar(&escape, PackLayout::HeroesV1).unwrap_err(),
            PACK_UNSAFE
        );
        let absolute = ustar_file("/tmp/escape.txt", b"nope");
        assert_eq!(
            read_ustar(&absolute, PackLayout::HeroesV1).unwrap_err(),
            PACK_UNSAFE
        );
        let mut link = ustar_header("link.txt", 0, b'2');
        link[157..161].copy_from_slice(b"dest");
        let sum: u64 = {
            let mut spaced = link;
            for byte in &mut spaced[148..156] {
                *byte = b' ';
            }
            spaced.iter().map(|byte| u64::from(*byte)).sum()
        };
        let checksum = format!("{sum:06o}\0 ");
        link[148..156].copy_from_slice(checksum.as_bytes());
        assert_eq!(
            read_ustar(&link, PackLayout::HeroesV1).unwrap_err(),
            PACK_UNSAFE
        );
    }

    #[tokio::test]
    async fn chunked_download_without_content_length_stops_at_the_cap() {
        let body = b"abcdefghijklmnopqrst";
        let fake = Arc::new(Fake {
            token: "pack-token".into(),
            list_status: 200,
            list_body: Vec::new(),
            list_retry: None,
            file_status: 200,
            file_body: body.to_vec(),
            file_chunked: true,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{base}/api/tracker/packs/{HEROES_PACK_NAME}"))
            .bearer_auth("pack-token")
            .send()
            .await
            .expect("chunked response");
        assert!(response.content_length().is_none());
        let err = read_body_capped(response, 8)
            .await
            .expect_err("over the cap");
        assert_eq!(err, PACK_TOO_BIG);
    }

    #[tokio::test]
    async fn synthetic_tar_unpacks_into_the_reader_layout() {
        let token = "pack-token";
        let hero: &[u8] = b"synthetic-hero";
        let special: &[u8] = b"synthetic-special";
        let bytes = pack_bytes(
            HEROES_PACK_NAME,
            "1",
            &[("ana.png", hero), ("special/placeholder_01.png", special)],
        );
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requested = requested_paths();
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 200,
            list_body: list_json(HEROES_PACK_NAME, "1", &bytes),
            list_retry: None,
            file_status: 200,
            file_body: bytes,
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: hits.clone(),
            requested: requested.clone(),
        });
        let base = spawn_fake(fake);
        let dir = tempfile::tempdir().expect("tempdir");
        let heroes = HeroTemplates::dir_in(dir.path());
        assert_eq!(heroes, dir.path().join("templates/heroes"));
        let first = sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES)
            .await
            .heroes()
            .clone();
        assert_eq!(first, PackSync::Installed);
        assert_eq!(std::fs::read(heroes.join("ana.png")).expect("hero"), hero);
        assert_eq!(
            std::fs::read(heroes.join("special/placeholder_01.png")).expect("special"),
            special
        );
        assert!(!heroes.join("manifest.json").exists());
        let stored: BTreeMap<String, String> =
            serde_json::from_slice(&std::fs::read(versions_path(dir.path())).unwrap()).unwrap();
        assert_eq!(stored.get(HEROES_PACK_NAME).map(String::as_str), Some("1"));
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        let paths = requested.lock().expect("paths").clone();
        assert!(paths.iter().any(|path| path == "/api/tracker/packs"));
        assert!(
            paths
                .iter()
                .any(|path| path == "/api/tracker/packs/heroes-v1.tar")
        );

        let second = sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES)
            .await
            .heroes()
            .clone();
        assert_eq!(second, PackSync::Current);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);

        std::fs::remove_dir_all(&heroes).expect("remove heroes");
        assert!(!heroes.exists());
        let again = sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES)
            .await
            .heroes()
            .clone();
        assert_eq!(again, PackSync::Installed);
        assert_eq!(
            std::fs::read(heroes.join("ana.png")).expect("redownloaded"),
            hero
        );
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_oversize_download_stops_during_sync() {
        let token = "pack-token";
        let bytes = pack_bytes(HEROES_PACK_NAME, "1", &[("ana.png", b"synthetic-hero")]);
        assert!(bytes.len() as u64 > 64);
        let dir = tempfile::tempdir().expect("tempdir");
        let heroes = HeroTemplates::dir_in(dir.path());
        std::fs::create_dir_all(&heroes).unwrap();
        std::fs::write(heroes.join("keep.txt"), b"old").unwrap();
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 200,
            list_body: list_json(HEROES_PACK_NAME, "1", &bytes),
            list_retry: None,
            file_status: 200,
            file_body: bytes,
            file_chunked: true,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let result = sync_reader_packs(base.as_str(), token, dir.path(), 64)
            .await
            .heroes()
            .clone();
        assert_eq!(result, PackSync::Failed(PACK_TOO_BIG.into()));
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");
        assert!(!heroes.join("ana.png").exists());
    }

    #[tokio::test]
    async fn a_cleartext_server_is_not_contacted() {
        let result = sync_reader_packs(
            "http://crew.example",
            "pack-token",
            Path::new("/tmp"),
            PACK_MAX_BYTES,
        )
        .await
        .heroes()
        .clone();
        assert_eq!(result, PackSync::Failed(PACK_FAILED.into()));
    }

    #[tokio::test]
    async fn a_bad_pack_keeps_the_old_files_and_does_not_log_the_token() {
        let token = "pack-token-secret";
        let dir = tempfile::tempdir().expect("tempdir");
        let heroes = HeroTemplates::dir_in(dir.path());
        std::fs::create_dir_all(&heroes).unwrap();
        std::fs::write(heroes.join("keep.txt"), b"old").unwrap();
        let bytes = pack_bytes(HEROES_PACK_NAME, "2", &[("ana.png", b"new")]);
        let listed = serde_json::to_vec(&serde_json::json!([{
            "name": HEROES_PACK_NAME,
            "version": "2",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "size": bytes.len(),
        }]))
        .unwrap();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let asked = requested_paths();
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 200,
            list_body: listed,
            list_retry: None,
            file_status: 200,
            file_body: bytes,
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: hits.clone(),
            requested: asked.clone(),
        });
        let base = spawn_fake(fake);
        let logs = capture_logs();
        let startup = sync_on_startup(base.as_str(), token, dir.path()).await;
        // The keeper says what went wrong and does not download the pack
        // again while the list shows the same entry.
        let creds = move || {
            Some(PackCredentials {
                server_url: base.clone(),
                token: token.to_string(),
            })
        };
        let (finished, waits) = rig_with(creds, dir.path(), Some(startup), 2, no_nap)
            .run()
            .await;
        let logs = logs.text();
        assert!(!finished, "a pack that never matches is never settled");
        assert_eq!(waits, [secs(60), secs(300), secs(900)]);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            asked.lock().unwrap().len(),
            3,
            "the list, the download, and the list once more"
        );
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");
        assert!(!heroes.join("ana.png").exists());
        assert!(logs.contains(PACK_MISMATCH), "{logs}");
        assert_eq!(
            logs.matches("reader packs are still missing").count(),
            1,
            "{logs}"
        );
        assert!(!logs.contains(token), "{logs}");
        assert_no_address_in_pack_lines(&logs);
    }

    #[tokio::test]
    async fn disabled_and_missing_routes_do_not_replace_installed_files() {
        let token = "pack-token";
        let dir = tempfile::tempdir().expect("tempdir");
        let heroes = HeroTemplates::dir_in(dir.path());
        std::fs::create_dir_all(&heroes).unwrap();
        std::fs::write(heroes.join("keep.txt"), b"old").unwrap();
        let disabled = Arc::new(Fake {
            token: token.into(),
            list_status: 503,
            list_body: br#"{"error":"packs_disabled"}"#.to_vec(),
            list_retry: None,
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(disabled);
        assert_eq!(
            sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES)
                .await
                .heroes()
                .clone(),
            PackSync::Unavailable
        );
        assert_eq!(
            PackSync::Unavailable.guide_result(),
            Ok("Reader packs aren't set up on the server yet. The tracker still works. Try again later or ask an admin.".to_string())
        );
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");

        let missing = Arc::new(Fake {
            token: token.into(),
            list_status: 404,
            list_body: br#"{"error":"not_found"}"#.to_vec(),
            list_retry: None,
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(missing);
        assert_eq!(
            sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES)
                .await
                .heroes()
                .clone(),
            PackSync::Unsupported
        );
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");
    }

    #[tokio::test]
    async fn a_proxy_502_or_503_is_a_failed_download() {
        async fn expect_failed(status: u16, body: &'static [u8]) {
            let token = "pack-token";
            let dir = tempfile::tempdir().expect("tempdir");
            let heroes = HeroTemplates::dir_in(dir.path());
            std::fs::create_dir_all(&heroes).unwrap();
            std::fs::write(heroes.join("keep.txt"), b"old").unwrap();
            let fake = Arc::new(Fake {
                token: token.into(),
                list_status: status,
                list_body: body.to_vec(),
                list_retry: None,
                file_status: 200,
                file_body: Vec::new(),
                file_chunked: false,
                named_files: BTreeMap::new(),
                file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                requested: requested_paths(),
            });
            let base = spawn_fake(fake);
            let result = sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES)
                .await
                .heroes()
                .clone();
            assert_eq!(
                result,
                PackSync::Failed(
                    "Could not download the reader pack. The installed pack was left in place."
                        .into()
                ),
                "HTTP {status}"
            );
            assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");
        }

        expect_failed(503, b"<html>upstream restart</html>").await;
        expect_failed(502, b"Bad Gateway").await;
        expect_failed(502, br#"{"error":"packs_disabled"}"#).await;
    }

    #[tokio::test]
    async fn rate_limit_uses_the_clamped_retry_after() {
        let token = "pack-token";
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 429,
            list_body: br#"{"error":"rate_limited","retry_after":1}"#.to_vec(),
            list_retry: Some(u64::MAX),
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let tmp = tempfile::tempdir().expect("tempdir");
        let result = sync_reader_packs(base.as_str(), token, tmp.path(), 32)
            .await
            .heroes()
            .clone();
        assert_eq!(
            result,
            PackSync::Failed("Too many tries, wait 3600 seconds and try again".into())
        );
    }

    #[tokio::test]
    async fn a_rejected_token_is_a_plain_401() {
        let fake = Arc::new(Fake {
            token: "expected".into(),
            list_status: 200,
            list_body: b"[]".to_vec(),
            list_retry: None,
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let tmp = tempfile::tempdir().expect("tempdir");
        let result = sync_reader_packs(base.as_str(), "wrong-token", tmp.path(), 32)
            .await
            .heroes()
            .clone();
        assert_eq!(
            result,
            PackSync::Failed("Your sync token isn't valid. Sign in again from Settings.".into())
        );
    }

    #[test]
    fn swap_exchanges_the_live_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let live = dir.path().join("heroes");
        let staging = dir.path().join(".heroes-next");
        std::fs::create_dir(&live).unwrap();
        std::fs::write(live.join("old.txt"), b"old").unwrap();
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("new.txt"), b"new").unwrap();
        swap_dir(&staging, &live).expect("swap");
        assert_eq!(std::fs::read(live.join("new.txt")).unwrap(), b"new");
        assert!(!live.join("old.txt").exists());
        assert!(!staging.exists());

        let fresh = dir.path().join("missing");
        let staged = dir.path().join(".heroes-fresh");
        std::fs::create_dir(&staged).unwrap();
        std::fs::write(staged.join("only.txt"), b"only").unwrap();
        swap_dir(&staged, &fresh).expect("create");
        assert_eq!(std::fs::read(fresh.join("only.txt")).unwrap(), b"only");
        assert!(!staged.exists());
    }

    #[tokio::test]
    async fn heroes_pack_is_downloaded_by_name_when_it_is_not_first() {
        let token = "pack-token";
        let bytes = pack_bytes(HEROES_PACK_NAME, "1", &[("ana.png", b"named")]);
        let list = serde_json::to_vec(&serde_json::json!([
            {
                "name": "other.tar",
                "version": "9",
                "sha256": "ab".repeat(32),
                "size": 4,
            },
            {
                "name": HEROES_PACK_NAME,
                "version": "1",
                "sha256": sha256_hex(&bytes),
                "size": bytes.len(),
            },
        ]))
        .unwrap();
        let requested = requested_paths();
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 200,
            list_body: list,
            list_retry: None,
            file_status: 200,
            file_body: bytes,
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested.clone(),
        });
        let base = spawn_fake(fake);
        let dir = tempfile::tempdir().expect("tempdir");
        let result = sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES)
            .await
            .heroes()
            .clone();
        assert_eq!(result, PackSync::Installed);
        let paths = requested.lock().expect("paths").clone();
        assert!(
            paths
                .iter()
                .any(|path| path == "/api/tracker/packs/heroes-v1.tar"),
            "{paths:?}"
        );
        assert!(
            paths
                .iter()
                .all(|path| path != "/api/tracker/packs/other.tar")
        );
    }

    #[tokio::test]
    async fn a_403_names_the_refusal_and_leaves_installed_files() {
        let token = "pack-token";
        let dir = tempfile::tempdir().expect("tempdir");
        let heroes = HeroTemplates::dir_in(dir.path());
        std::fs::create_dir_all(&heroes).unwrap();
        std::fs::write(heroes.join("keep.txt"), b"old").unwrap();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let listed = Arc::new(Fake {
            token: token.into(),
            list_status: 403,
            list_body: br#"{"error":"forbidden"}"#.to_vec(),
            list_retry: None,
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: hits.clone(),
            requested: requested_paths(),
        });
        let base = spawn_fake(listed);
        assert_eq!(
            sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES).await.heroes().clone(),
            PackSync::Failed(
                "The site refused the reader pack (403). Check you're signed in with a member account. The installed pack was left in place.".into()
            )
        );
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");

        let bytes = pack_bytes(HEROES_PACK_NAME, "2", &[("ana.png", b"new")]);
        let file = Arc::new(Fake {
            token: token.into(),
            list_status: 200,
            list_body: list_json(HEROES_PACK_NAME, "2", &bytes),
            list_retry: None,
            file_status: 403,
            file_body: bytes,
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(file);
        assert_eq!(
            sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES).await.heroes().clone(),
            PackSync::Failed(
                "The site refused the reader pack (403). Check you're signed in with a member account. The installed pack was left in place.".into()
            )
        );
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");
        assert!(!heroes.join("ana.png").exists());
    }

    // ---- multi-pack sync (alpha.7 P1) ----
    // Every pack below is built here from 1x1 PNGs this test generates.
    // No game art is read or stored.

    /// A 1x1 PNG made in the test, never game art.
    fn tiny_png(seed: u8) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([seed, 7, 9, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png)
            .expect("png");
        out.into_inner()
    }

    fn version_of(name: &str) -> &'static str {
        match name {
            "heroes-v2.tar" | "heroes-v2-1080.tar" | "bans-v1.tar" => "154088.1",
            _ => "1",
        }
    }

    /// File names in each pack's own layout, shaped like the real packs.
    fn sample_files(name: &str) -> Vec<(String, Vec<u8>)> {
        let names: &[&str] = match name {
            "heroes-v1.tar" => &["ana.png", "special/skull_01.png"],
            "heroes-v2.tar" | "heroes-v2-1080.tar" => {
                &["ana.png", "wrecking-ball.png", "special/placeholder_01.png"]
            }
            "bans-v1.tar" => &["ana.png", "soldier-76.png"],
            "maps-v1.tar" => &["atlas.json", "g0041.png", "g003a.png"],
            "words-v1.tar" => &["accolade_victory.png", "end_title_defeat.png", "words.json"],
            "names-v1.tar" => &[
                "glyphs.png",
                "glyphs.txt",
                "marks.png",
                "marks.txt",
                "compose.txt",
                "whiten.f32",
            ],
            other => panic!("no sample for {other}"),
        };
        if name == "names-v1.tar" {
            // Bytes only have to match the layout. They are not a letter pack.
            return names
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let data = if path.ends_with(".png") {
                        tiny_png(index as u8)
                    } else if path.ends_with(".f32") {
                        vec![index as u8; 16]
                    } else {
                        b"A\t0.5\n".to_vec()
                    };
                    (path.to_string(), data)
                })
                .collect();
        }
        names
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let data = if path.ends_with(".json") {
                    format!(r#"{{"synthetic":"{name}"}}"#).into_bytes()
                } else {
                    tiny_png(index as u8 ^ name.len() as u8)
                };
                (path.to_string(), data)
            })
            .collect()
    }

    fn build_pack(name: &str, version: &str, files: &[(String, Vec<u8>)]) -> Vec<u8> {
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(path, data)| (path.as_str(), data.as_slice()))
            .collect();
        pack_bytes(name, version, &refs)
    }

    fn entry_json(name: &str, version: &str, bytes: &[u8]) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "version": version,
            "sha256": sha256_hex(bytes),
            "size": bytes.len(),
        })
    }

    struct Served {
        base: String,
        hits: Arc<std::sync::atomic::AtomicUsize>,
        requested: Arc<Mutex<Vec<String>>>,
    }

    impl Served {
        fn hits(&self) -> usize {
            self.hits.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn downloaded(&self) -> Vec<String> {
            self.requested
                .lock()
                .expect("paths")
                .iter()
                .filter_map(|path| path.strip_prefix("/api/tracker/packs/"))
                .map(str::to_string)
                .collect()
        }
    }

    const TOKEN: &str = "pack-token";

    fn serve(list: Vec<serde_json::Value>, files: ServedFiles) -> Served {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requested = requested_paths();
        let fake = Arc::new(Fake {
            token: TOKEN.into(),
            list_status: 200,
            list_body: serde_json::to_vec(&list).unwrap(),
            list_retry: None,
            file_status: 404,
            file_body: br#"{"error":"pack_not_found"}"#.to_vec(),
            file_chunked: false,
            file_hits: hits.clone(),
            requested: requested.clone(),
            named_files: files,
        });
        Served {
            base: spawn_fake(fake),
            hits,
            requested,
        }
    }

    /// `(status, body)` per pack name, as the fake serves it.
    type ServedFiles = BTreeMap<String, (u16, Vec<u8>)>;
    /// The inner files of each synthetic pack, by pack name.
    type PackContents = BTreeMap<String, Vec<(String, Vec<u8>)>>;

    /// The server side of every pack: list entries plus files.
    fn all_packs() -> (Vec<serde_json::Value>, ServedFiles, PackContents) {
        let mut list = Vec::new();
        let mut files = BTreeMap::new();
        let mut contents = BTreeMap::new();
        for spec in PACKS {
            let inner = sample_files(spec.name);
            let bytes = build_pack(spec.name, version_of(spec.name), &inner);
            list.push(entry_json(spec.name, version_of(spec.name), &bytes));
            files.insert(spec.name.to_string(), (200, bytes));
            contents.insert(spec.name.to_string(), inner);
        }
        (list, files, contents)
    }

    async fn sync(served: &Served, data_dir: &Path) -> PackReport {
        sync_reader_packs(served.base.as_str(), TOKEN, data_dir, PACK_MAX_BYTES).await
    }

    fn stored_versions(data_dir: &Path) -> BTreeMap<String, String> {
        serde_json::from_slice(&std::fs::read(versions_path(data_dir)).expect("versions"))
            .expect("versions json")
    }

    fn plant_keep(data_dir: &Path, spec: &PackSpec) -> PathBuf {
        let dir = spec.dir_in(data_dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keep.txt"), b"old").unwrap();
        dir
    }

    fn outcome<'a>(report: &'a PackReport, name: &str) -> &'a PackSync {
        report
            .get(name)
            .unwrap_or_else(|| panic!("{name} in report"))
    }

    #[test]
    fn heroes_v1_keeps_its_alpha6_folder_and_the_others_get_their_own() {
        let data = Path::new("/data");
        let heroes = heroes_spec();
        assert_eq!(
            PACKS[0].name, HEROES_PACK_NAME,
            "heroes-v1 is fetched first"
        );
        assert_eq!(heroes.dir_in(data), HeroTemplates::dir_in(data));
        assert_eq!(heroes.dir_in(data), data.join("templates/heroes"));
        assert_eq!(heroes.layout, PackLayout::HeroesV1);
        let expected = [
            ("heroes-v2.tar", "templates/heroes-v2"),
            ("heroes-v2-1080.tar", "templates/heroes-v2-1080"),
            ("bans-v1.tar", "templates/bans-v1"),
            ("maps-v1.tar", "templates/maps-v1"),
            ("words-v1.tar", "templates/words-v1"),
            ("names-v1.tar", "templates/names-v1"),
        ];
        assert_eq!(PACKS.len(), expected.len() + 1);
        for (name, dir) in expected {
            let spec = pack_spec(name).expect(name);
            assert_eq!(spec.dir_in(data), data.join(dir), "{name}");
            assert!(!spec.required, "{name} is optional");
        }
        // Reader neutrality: the hero packs, maps-v1, words-v1 and names-v1
        // land in a folder a reader opens. bans-v1 does not. The
        // hand-installed maps and result folders are not pack folders.
        let reader_dirs = [
            crate::shadow::heroes::TEMPLATE_SUBDIR,
            crate::shadow::heroes::TEMPLATE_SUBDIR_V2,
            crate::shadow::heroes::TEMPLATE_SUBDIR_V2_1080,
            crate::shadow::result::TEMPLATE_SUBDIR,
            crate::shadow::result::WORDS_SUBDIR,
            crate::shadow::banner::TEMPLATE_SUBDIR,
            crate::shadow::banner::TEMPLATE_SUBDIR_V1,
            "templates/names-v1",
        ];
        let mut seen = BTreeSet::new();
        for spec in PACKS {
            let dir = spec.dir_in(data);
            assert!(seen.insert(dir.clone()), "{} shares a folder", spec.name);
            assert_ne!(dir, data.join("templates"));
            assert!(!versions_path(data).starts_with(&dir));
            let used_by_reader = reader_dirs.iter().any(|sub| dir == data.join(sub));
            assert_eq!(
                used_by_reader,
                matches!(
                    spec.name,
                    HEROES_PACK_NAME
                        | "heroes-v2.tar"
                        | "heroes-v2-1080.tar"
                        | "maps-v1.tar"
                        | "words-v1.tar"
                        | "names-v1.tar"
                ),
                "{}",
                spec.name
            );
            assert_ne!(
                dir,
                data.join(crate::shadow::banner::TEMPLATE_SUBDIR),
                "{} would overwrite a hand-installed map pack",
                spec.name
            );
            assert_ne!(
                dir,
                data.join(crate::shadow::result::TEMPLATE_SUBDIR),
                "{} would overwrite hand-installed result templates",
                spec.name
            );
        }
    }

    #[test]
    fn each_layout_accepts_its_own_names_and_refuses_the_rest() {
        let ok: &[(PackLayout, &str)] = &[
            (PackLayout::Portraits, "ana.png"),
            (PackLayout::Portraits, "wrecking-ball.png"),
            (PackLayout::Portraits, "soldier-76.png"),
            (PackLayout::Portraits, "special/skull_01.png"),
            (PackLayout::BanIcons, "junker-queen.png"),
            (PackLayout::MapGlyphs, "atlas.json"),
            (PackLayout::MapGlyphs, "g0041.png"),
            (PackLayout::MapGlyphs, "g003a.png"),
            (PackLayout::Words, "accolade_victory.png"),
            (PackLayout::Words, "layouts.json"),
            (PackLayout::Names, "glyphs.png"),
            (PackLayout::Names, "glyphs.txt"),
            (PackLayout::Names, "marks.png"),
            (PackLayout::Names, "marks.txt"),
            (PackLayout::Names, "compose.txt"),
            (PackLayout::Names, "whiten.f32"),
        ];
        for (layout, raw) in ok {
            assert_eq!(
                layout.file_path(raw).as_deref(),
                Ok(*raw),
                "{layout:?} {raw}"
            );
        }
        let long = format!("{}.png", "a".repeat(MAX_STEM + 1));
        let refused: &[(PackLayout, &str)] = &[
            (PackLayout::Portraits, "../x.png"),
            (PackLayout::Portraits, "special/a/b.png"),
            (PackLayout::Portraits, "other/x.png"),
            (PackLayout::Portraits, "./ana.png"),
            (PackLayout::Portraits, "Ana.png"),
            (PackLayout::Portraits, "ana.jpg"),
            (PackLayout::Portraits, "special/x.txt"),
            (PackLayout::Portraits, ".png"),
            (PackLayout::Portraits, "sp\\ana.png"),
            (PackLayout::Portraits, long.as_str()),
            (PackLayout::BanIcons, "special/skull_01.png"),
            (PackLayout::BanIcons, "ana.json"),
            (PackLayout::MapGlyphs, "g41.png"),
            (PackLayout::MapGlyphs, "g00zz.png"),
            (PackLayout::MapGlyphs, "g00411.png"),
            (PackLayout::MapGlyphs, "G0041.png"),
            (PackLayout::MapGlyphs, "g003A.png"),
            (PackLayout::MapGlyphs, "other.json"),
            (PackLayout::MapGlyphs, "ana.png"),
            (PackLayout::Words, "end-title.png"),
            (PackLayout::Words, "sub/words.json"),
            (PackLayout::Words, "words.txt"),
            (PackLayout::Names, "glyphs.jpg"),
            (PackLayout::Names, "Glyphs.png"),
            (PackLayout::Names, "sub/glyphs.png"),
            (PackLayout::Names, "whiten.f64"),
            (PackLayout::Names, "compose.json"),
            (PackLayout::Names, "../glyphs.png"),
        ];
        for (layout, raw) in refused {
            assert_eq!(layout.file_path(raw), Err(PACK_UNSAFE), "{layout:?} {raw}");
        }
        for layout in [
            PackLayout::HeroesV1,
            PackLayout::Portraits,
            PackLayout::BanIcons,
            PackLayout::MapGlyphs,
            PackLayout::Words,
            PackLayout::Names,
        ] {
            assert_eq!(
                layout.file_path("manifest.json").as_deref(),
                Ok("manifest.json")
            );
        }
        // heroes-v1 keeps the alpha.6 rule: any one top-level file.
        assert_eq!(
            PackLayout::HeroesV1.file_path("./Ana.txt").as_deref(),
            Ok("Ana.txt")
        );
        assert_eq!(
            PackLayout::HeroesV1.file_path("special/a/b.png"),
            Err(PACK_UNSAFE)
        );

        let mut dir = ustar_header("special/", 0, b'5').to_vec();
        dir.extend(ustar_file("ana.png", b"x"));
        assert!(read_ustar(&dir, PackLayout::Portraits).is_ok());
        assert!(read_ustar(&dir, PackLayout::HeroesV1).is_ok());
        assert_eq!(
            read_ustar(&dir, PackLayout::BanIcons).unwrap_err(),
            PACK_UNSAFE
        );
        assert_eq!(
            read_ustar(&dir, PackLayout::Words).unwrap_err(),
            PACK_UNSAFE
        );
        assert_eq!(
            read_ustar(&dir, PackLayout::Names).unwrap_err(),
            PACK_UNSAFE
        );
    }

    #[test]
    fn names_v1_is_optional_and_allows_its_seven_files() {
        let spec = pack_spec(crate::names::pack::PACK_NAME).expect("names-v1");
        assert!(
            !spec.required,
            "a missing names pack must not fail the hero install"
        );
        assert_eq!(spec.layout, PackLayout::Names);
        assert_eq!(
            spec.dir_in(Path::new("/data")),
            Path::new("/data/templates/names-v1")
        );
        assert_eq!(
            spec.dir_in(Path::new("/data")),
            crate::names::pack::dir_in(Path::new("/data"))
        );
        // manifest.json plus the six files NamePack::load reads.
        let allowed = [
            "manifest.json",
            "glyphs.png",
            "glyphs.txt",
            "marks.png",
            "marks.txt",
            "compose.txt",
            "whiten.f32",
        ];
        assert_eq!(allowed.len(), 7);
        for name in allowed {
            assert_eq!(
                PackLayout::Names.file_path(name).as_deref(),
                Ok(name),
                "{name}"
            );
        }
        assert_eq!(PackLayout::Names.file_path("letters.png"), Err(PACK_UNSAFE));
        assert_eq!(PackLayout::Names.file_path("Glyphs.png"), Err(PACK_UNSAFE));
        assert!(!PackLayout::Names.allows_dir("special"));
    }

    #[tokio::test]
    async fn all_six_packs_install_into_a_fresh_data_dir() {
        let (list, files, contents) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let report = sync(&served, dir.path()).await;
        for spec in PACKS {
            assert_eq!(
                outcome(&report, spec.name),
                &PackSync::Installed,
                "{}",
                spec.name
            );
            let live = spec.dir_in(dir.path());
            for (path, data) in &contents[spec.name] {
                assert_eq!(
                    &std::fs::read(live.join(path)).unwrap_or_default(),
                    data,
                    "{} {path}",
                    spec.name
                );
            }
            assert_eq!(
                live.join("manifest.json").exists(),
                spec.layout.keeps_manifest(),
                "only names-v1 keeps its manifest: {}",
                spec.name
            );
        }
        assert_eq!(served.hits(), PACKS.len());
        let versions = stored_versions(dir.path());
        for spec in PACKS {
            assert_eq!(
                versions.get(spec.name).map(String::as_str),
                Some(version_of(spec.name))
            );
        }
        let templates = dir.path().join("templates");
        let leftovers: Vec<String> = std::fs::read_dir(&templates)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.') && name != ".pack-sync.lock")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert_eq!(report.extras_ready(), (6, 6));
        assert_eq!(
            report.guide_result(),
            Ok("Reader pack saved. Extra reader packs ready: 6 of 6.".to_string())
        );
    }

    #[tokio::test]
    async fn a_resync_with_the_same_versions_downloads_nothing() {
        let (list, files, contents) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let first = sync(&served, dir.path()).await;
        assert!(first.packs.iter().all(|s| s.outcome == PackSync::Installed));
        assert_eq!(served.hits(), PACKS.len());
        let before = stored_versions(dir.path());

        let second = sync(&served, dir.path()).await;
        for spec in PACKS {
            assert_eq!(
                outcome(&second, spec.name),
                &PackSync::Current,
                "{}",
                spec.name
            );
        }
        assert_eq!(served.hits(), PACKS.len(), "no pack was downloaded again");
        assert_eq!(stored_versions(dir.path()), before);
        let bans = pack_spec("bans-v1.tar").unwrap().dir_in(dir.path());
        assert_eq!(
            std::fs::read(bans.join("ana.png")).unwrap(),
            contents["bans-v1.tar"][0].1
        );
        assert_eq!(
            second.guide_result(),
            Ok("The reader pack is already installed. Extra reader packs ready: 6 of 6.".into())
        );
    }

    #[tokio::test]
    async fn a_bad_sha_rejects_only_that_pack() {
        let (mut list, files, _) = all_packs();
        for entry in &mut list {
            if entry["name"] == "bans-v1.tar" {
                entry["sha256"] = serde_json::json!("ab".repeat(32));
            }
        }
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let bans = plant_keep(dir.path(), pack_spec("bans-v1.tar").unwrap());
        let report = sync(&served, dir.path()).await;
        for spec in PACKS {
            let expected = if spec.name == "bans-v1.tar" {
                PackSync::Failed(PACK_MISMATCH.into())
            } else {
                PackSync::Installed
            };
            assert_eq!(outcome(&report, spec.name), &expected, "{}", spec.name);
        }
        assert_eq!(std::fs::read(bans.join("keep.txt")).unwrap(), b"old");
        assert!(!bans.join("ana.png").exists());
        let versions = stored_versions(dir.path());
        assert!(!versions.contains_key("bans-v1.tar"));
        assert_eq!(versions.len(), PACKS.len() - 1);
        assert_eq!(report.extras_ready(), (5, 6));
    }

    #[tokio::test]
    async fn a_bad_path_rejects_only_that_pack() {
        let bad: &[(&str, &str)] = &[
            ("heroes-v2.tar", "../x.png"),
            ("bans-v1.tar", "sub/ana.png"),
            ("words-v1.tar", "accolade_victory.jpg"),
            ("maps-v1.tar", "g0041.webp"),
        ];
        for (bad_pack, bad_path) in bad {
            let (mut list, mut files, _) = all_packs();
            let mut inner = sample_files(bad_pack);
            inner.push((bad_path.to_string(), tiny_png(3)));
            let bytes = build_pack(bad_pack, version_of(bad_pack), &inner);
            for entry in &mut list {
                if entry["name"] == *bad_pack {
                    *entry = entry_json(bad_pack, version_of(bad_pack), &bytes);
                }
            }
            files.insert(bad_pack.to_string(), (200, bytes));
            let served = serve(list, files);
            let dir = tempfile::tempdir().expect("tempdir");
            let live = plant_keep(dir.path(), pack_spec(bad_pack).unwrap());
            let report = sync(&served, dir.path()).await;
            for spec in PACKS {
                let got = outcome(&report, spec.name);
                if spec.name == *bad_pack {
                    assert_eq!(got, &PackSync::Failed(PACK_UNSAFE.into()), "{bad_path}");
                } else {
                    assert_eq!(got, &PackSync::Installed, "{} with {bad_path}", spec.name);
                }
            }
            assert_eq!(std::fs::read(live.join("keep.txt")).unwrap(), b"old");
            assert!(!dir.path().join("x.png").exists());
            assert!(!dir.path().join("templates/x.png").exists());
            assert!(!stored_versions(dir.path()).contains_key(*bad_pack));
        }
    }

    #[tokio::test]
    async fn packs_disabled_keeps_every_live_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let live: Vec<PathBuf> = PACKS
            .iter()
            .map(|spec| plant_keep(dir.path(), spec))
            .collect();
        let disabled = Arc::new(Fake {
            token: TOKEN.into(),
            list_status: 503,
            list_body: br#"{"error":"packs_disabled"}"#.to_vec(),
            list_retry: None,
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
            named_files: BTreeMap::new(),
        });
        let base = spawn_fake(disabled);
        let report = sync_reader_packs(base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert!(
            report
                .packs
                .iter()
                .all(|s| s.outcome == PackSync::Unavailable),
            "{report:?}"
        );
        assert_eq!(report.guide_result(), Ok(PACK_UNAVAILABLE.to_string()));

        // Packs switched off between the list and a download: nothing after
        // it is fetched, and nothing installed is touched.
        let (list, mut files, _) = all_packs();
        files.insert(
            HEROES_PACK_NAME.to_string(),
            (503, br#"{"error":"packs_disabled"}"#.to_vec()),
        );
        let served = serve(list, files);
        let report = sync(&served, dir.path()).await;
        assert!(
            report
                .packs
                .iter()
                .all(|s| s.outcome == PackSync::Unavailable),
            "{report:?}"
        );
        assert_eq!(served.hits(), 1, "{:?}", served.downloaded());
        for dir in &live {
            assert_eq!(std::fs::read(dir.join("keep.txt")).unwrap(), b"old");
        }
    }

    #[tokio::test]
    async fn a_rate_limit_mid_sync_stops_the_rest_and_keeps_what_installed() {
        let (list, mut files, _) = all_packs();
        files.insert(
            "heroes-v2.tar".to_string(),
            (429, br#"{"error":"rate_limited"}"#.to_vec()),
        );
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let words = plant_keep(dir.path(), pack_spec("words-v1.tar").unwrap());
        let report = sync(&served, dir.path()).await;
        assert_eq!(outcome(&report, HEROES_PACK_NAME), &PackSync::Installed);
        let limited = PackSync::Failed(rate_limit_message(RATE_LIMIT_FALLBACK_SECS));
        for spec in &PACKS[1..] {
            assert_eq!(outcome(&report, spec.name), &limited, "{}", spec.name);
        }
        assert_eq!(
            served.downloaded(),
            vec![HEROES_PACK_NAME.to_string(), "heroes-v2.tar".to_string()]
        );
        assert_eq!(std::fs::read(words.join("keep.txt")).unwrap(), b"old");
        assert!(heroes_spec().dir_in(dir.path()).join("ana.png").exists());
    }

    #[tokio::test]
    async fn a_list_with_only_heroes_v1_syncs_it_and_marks_the_rest_missing() {
        let inner = sample_files(HEROES_PACK_NAME);
        let bytes = build_pack(HEROES_PACK_NAME, "1", &inner);
        let mut files = BTreeMap::new();
        files.insert(HEROES_PACK_NAME.to_string(), (200, bytes.clone()));
        let served = serve(vec![entry_json(HEROES_PACK_NAME, "1", &bytes)], files);
        let dir = tempfile::tempdir().expect("tempdir");
        // An extra pack installed earlier stays when the list drops it.
        let maps = plant_keep(dir.path(), pack_spec("maps-v1.tar").unwrap());
        let report = sync(&served, dir.path()).await;
        assert_eq!(outcome(&report, HEROES_PACK_NAME), &PackSync::Installed);
        for spec in &PACKS[1..] {
            assert_eq!(
                outcome(&report, spec.name),
                &PackSync::Missing,
                "{}",
                spec.name
            );
        }
        assert_eq!(served.downloaded(), vec![HEROES_PACK_NAME.to_string()]);
        assert_eq!(std::fs::read(maps.join("keep.txt")).unwrap(), b"old");
        assert_eq!(
            std::fs::read(HeroTemplates::dir_in(dir.path()).join("ana.png")).unwrap(),
            inner[0].1
        );
        assert_eq!(report.extras_ready(), (0, 0));
        // The alpha.6 guide line, unchanged.
        assert_eq!(report.guide_result(), Ok(PACK_SAVED.to_string()));
    }

    #[tokio::test]
    async fn a_list_without_heroes_v1_is_still_a_heroes_mismatch() {
        let (list, files, _) = all_packs();
        let list: Vec<_> = list
            .into_iter()
            .filter(|entry| entry["name"] != HEROES_PACK_NAME)
            .collect();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let report = sync(&served, dir.path()).await;
        assert_eq!(
            outcome(&report, HEROES_PACK_NAME),
            &PackSync::Failed(PACK_MISMATCH.into())
        );
        for spec in &PACKS[1..] {
            assert_eq!(
                outcome(&report, spec.name),
                &PackSync::Installed,
                "{}",
                spec.name
            );
        }
        assert_eq!(report.guide_result(), Err(PACK_MISMATCH.to_string()));
    }

    #[tokio::test]
    async fn an_alpha6_versions_file_is_read_and_kept() {
        let (list, files, _) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        // What alpha.6 leaves behind: the heroes folder and a one-key file.
        let heroes = plant_keep(dir.path(), heroes_spec());
        std::fs::write(versions_path(dir.path()), br#"{"heroes-v1.tar":"1"}"#).unwrap();
        let report = sync(&served, dir.path()).await;
        assert_eq!(outcome(&report, HEROES_PACK_NAME), &PackSync::Current);
        for spec in &PACKS[1..] {
            assert_eq!(
                outcome(&report, spec.name),
                &PackSync::Installed,
                "{}",
                spec.name
            );
        }
        assert!(
            !served
                .downloaded()
                .iter()
                .any(|name| name == HEROES_PACK_NAME)
        );
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");
        let versions = stored_versions(dir.path());
        assert_eq!(
            versions.get(HEROES_PACK_NAME).map(String::as_str),
            Some("1")
        );
        assert_eq!(versions.len(), PACKS.len());
        // The file stays a flat name-to-version map an alpha.6 build reads.
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(versions_path(dir.path())).unwrap()).unwrap();
        assert!(raw.as_object().unwrap().values().all(|v| v.is_string()));
    }

    // ---- review round 1 on #248 (F248-*, R248-A, N1) ----

    fn list_requests(served: &Served) -> usize {
        served
            .requested
            .lock()
            .expect("paths")
            .iter()
            .filter(|path| path.as_str() == "/api/tracker/packs")
            .count()
    }

    /// `all_packs` with `name` answering `status` on download.
    fn all_packs_with_download(name: &str, status: u16, body: &[u8]) -> Served {
        let (list, mut files, _) = all_packs();
        files.insert(name.to_string(), (status, body.to_vec()));
        serve(list, files)
    }

    #[tokio::test]
    async fn daemon_startup_fetches_only_heroes_v1_and_the_extras_follow() {
        let (list, files, contents) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let startup =
            sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert!(startup.goes_on(), "the list was read and is kept");
        // Capture starts after this: only the alpha.6 requests happened.
        assert_eq!(served.downloaded(), vec![HEROES_PACK_NAME.to_string()]);
        assert_eq!(list_requests(&served), 1);
        for spec in &PACKS[1..] {
            assert!(!spec.dir_in(dir.path()).exists(), "{}", spec.name);
        }
        let mut keeper = rig(&served.base, dir.path(), Some(startup), 0, no_nap).keeper;
        let report = keeper.first_pass().await.report;
        assert_eq!(
            report.packs.len(),
            PACKS.len(),
            "startup's pack, then the rest"
        );
        assert_eq!(report.heroes(), &PackSync::Installed);
        for spec in &PACKS[1..] {
            assert_eq!(
                outcome(&report, spec.name),
                &PackSync::Installed,
                "{}",
                spec.name
            );
            let (path, data) = &contents[spec.name][0];
            assert_eq!(
                &std::fs::read(spec.dir_in(dir.path()).join(path)).unwrap(),
                data
            );
        }
        assert_eq!(list_requests(&served), 1, "the startup list is reused");
        assert_eq!(served.hits(), PACKS.len());
        assert_eq!(stored_versions(dir.path()).len(), PACKS.len());
    }

    #[tokio::test]
    async fn a_list_failure_at_startup_is_logged_once() {
        let fake = Arc::new(Fake {
            token: TOKEN.into(),
            list_status: 403,
            list_body: br#"{"error":"forbidden"}"#.to_vec(),
            list_retry: None,
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        let startup = sync_on_startup(base.as_str(), TOKEN, dir.path()).await;
        assert!(startup.run.entries.is_none());
        // The keeper writes the line, with the wait before its next try.
        let (finished, waits) = rig(&base, dir.path(), Some(startup), 0, no_nap).run().await;
        let logs = logs.text();
        assert!(!finished);
        assert_eq!(waits, [secs(60)]);
        assert_eq!(
            logs.matches("reader packs are still missing").count(),
            1,
            "{logs}"
        );
        assert!(logs.contains(PACK_FORBIDDEN), "{logs}");
        assert!(logs.contains("wait_secs=60"), "{logs}");
        assert!(!logs.contains(TOKEN), "{logs}");
        assert_no_address_in_pack_lines(&logs);
    }

    #[tokio::test]
    async fn a_second_sync_while_one_runs_is_busy_and_sends_nothing() {
        let (list, files, _) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let held = SyncLock::try_acquire(dir.path())
            .expect("lock file")
            .expect("free");
        let report = sync(&served, dir.path()).await;
        assert!(
            report.packs.iter().all(|s| s.outcome == PackSync::Busy),
            "{report:?}"
        );
        assert!(served.requested.lock().unwrap().is_empty(), "no request");
        assert!(!versions_path(dir.path()).exists());
        assert_eq!(report.guide_result(), Ok(PACK_BUSY.to_string()));
        let startup =
            sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(startup.run.report.heroes(), &PackSync::Busy);
        assert!(startup.run.entries.is_none());
        assert!(served.requested.lock().unwrap().is_empty(), "no request");
        drop(held);
        let report = sync(&served, dir.path()).await;
        assert!(
            report
                .packs
                .iter()
                .all(|s| s.outcome == PackSync::Installed)
        );
    }

    #[tokio::test]
    async fn the_versions_file_does_not_use_a_shared_tmp_name() {
        // A leftover (or another writer's) fixed-name tmp must not block
        // recording versions.
        let (list, files, _) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("templates/pack-versions.json.tmp")).unwrap();
        let report = sync(&served, dir.path()).await;
        assert!(
            report
                .packs
                .iter()
                .all(|s| s.outcome == PackSync::Installed)
        );
        assert_eq!(stored_versions(dir.path()).len(), PACKS.len());
        let tmp_files: Vec<String> = std::fs::read_dir(dir.path().join("templates"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("pack-versions.json.tmp-"))
            .collect();
        assert!(tmp_files.is_empty(), "{tmp_files:?}");
    }

    #[tokio::test]
    async fn after_a_stop_packs_already_in_place_are_current() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (list, files, _) = all_packs();
        let first = serve(list, files);
        let installed = sync(&first, dir.path()).await;
        assert!(
            installed
                .packs
                .iter()
                .all(|s| s.outcome == PackSync::Installed)
        );

        // heroes-v2 gets a new version, and its download is rate limited.
        let (mut list, files, _) = all_packs();
        let bumped = build_pack("heroes-v2.tar", "2", &sample_files("heroes-v2.tar"));
        for entry in &mut list {
            if entry["name"] == "heroes-v2.tar" {
                *entry = entry_json("heroes-v2.tar", "2", &bumped);
            }
        }
        let mut files = files;
        files.insert(
            "heroes-v2.tar".to_string(),
            (429, br#"{"error":"rate_limited"}"#.to_vec()),
        );
        let served = serve(list, files);
        let report = sync(&served, dir.path()).await;
        let limited = rate_limit_message(RATE_LIMIT_FALLBACK_SECS);
        for spec in PACKS {
            let expected = if spec.name == "heroes-v2.tar" {
                PackSync::Failed(limited.clone())
            } else {
                PackSync::Current
            };
            assert_eq!(outcome(&report, spec.name), &expected, "{}", spec.name);
        }
        assert_eq!(served.downloaded(), vec!["heroes-v2.tar".to_string()]);
        assert_eq!(report.extras_ready(), (5, 6));
        assert_eq!(
            report.guide_result(),
            Ok(format!(
                "The reader pack is already installed. Extra reader packs ready: 5 of 6. {limited}."
            ))
        );
    }

    #[tokio::test]
    async fn a_mid_run_401_or_403_stops_the_rest() {
        for (status, message) in [(401u16, PACK_AUTH), (403, PACK_FORBIDDEN)] {
            let served = all_packs_with_download("heroes-v2.tar", status, br#"{"error":"no"}"#);
            let dir = tempfile::tempdir().expect("tempdir");
            let bans = plant_keep(dir.path(), pack_spec("bans-v1.tar").unwrap());
            let report = sync(&served, dir.path()).await;
            assert_eq!(outcome(&report, HEROES_PACK_NAME), &PackSync::Installed);
            for spec in &PACKS[1..] {
                assert_eq!(
                    outcome(&report, spec.name),
                    &PackSync::Failed(message.into()),
                    "HTTP {status} {}",
                    spec.name
                );
            }
            assert_eq!(
                served.downloaded(),
                vec![HEROES_PACK_NAME.to_string(), "heroes-v2.tar".to_string()],
                "HTTP {status}"
            );
            assert_eq!(std::fs::read(bans.join("keep.txt")).unwrap(), b"old");
        }
    }

    #[tokio::test]
    async fn an_unlisted_pack_stays_missing_after_a_stop() {
        let (list, mut files, _) = all_packs();
        let list: Vec<_> = list
            .into_iter()
            .filter(|entry| entry["name"] != "words-v1.tar")
            .collect();
        files.insert(
            "heroes-v2.tar".to_string(),
            (429, br#"{"error":"rate_limited"}"#.to_vec()),
        );
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let report = sync(&served, dir.path()).await;
        let limited = PackSync::Failed(rate_limit_message(RATE_LIMIT_FALLBACK_SECS));
        assert_eq!(outcome(&report, HEROES_PACK_NAME), &PackSync::Installed);
        for name in [
            "heroes-v2.tar",
            "heroes-v2-1080.tar",
            "bans-v1.tar",
            "maps-v1.tar",
            "names-v1.tar",
        ] {
            assert_eq!(outcome(&report, name), &limited, "{name}");
        }
        assert_eq!(outcome(&report, "words-v1.tar"), &PackSync::Missing);
        assert_eq!(report.extras_ready(), (0, 5));
    }

    fn age_folder(path: &Path, age: std::time::Duration) {
        let when = std::time::SystemTime::now() - age;
        std::fs::File::open(path)
            .expect("open folder")
            .set_modified(when)
            .expect("set mtime");
    }

    #[tokio::test]
    async fn folders_left_by_a_killed_sync_are_swept_after_an_hour() {
        let dir = tempfile::tempdir().expect("tempdir");
        let templates = dir.path().join("templates");
        let two_hours = std::time::Duration::from_secs(2 * 3600);
        let old_ours = [
            ".bans-v1-next-11-22",
            ".heroes-prev-33-44",
            ".words-v1-next-5-6",
        ];
        for name in old_ours {
            let path = templates.join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("x.png"), b"left").unwrap();
            age_folder(&path, two_hours);
        }
        let fresh = templates.join(".maps-v1-next-77-88");
        std::fs::create_dir_all(&fresh).unwrap();
        let foreign = templates.join(".other-next-1-2");
        std::fs::create_dir_all(&foreign).unwrap();
        age_folder(&foreign, two_hours);
        let live = plant_keep(dir.path(), pack_spec("bans-v1.tar").unwrap());
        age_folder(&live, two_hours);
        // heroes is installed, so its old `-prev-` backup is not the only copy.
        plant_keep(dir.path(), pack_spec(HEROES_PACK_NAME).unwrap());
        // A link with our name is not followed.
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("keep.txt"), b"outside").unwrap();
        let link = templates.join(".heroes-v2-next-9-9");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        // Old enough to sweep, so only "not a real folder" keeps it.
        age_link(&link, two_hours);

        let served = serve(Vec::new(), BTreeMap::new());
        let report = sync(&served, dir.path()).await;
        assert_eq!(report.heroes(), &PackSync::Current, "empty list");
        for name in old_ours {
            assert!(!templates.join(name).exists(), "{name} swept");
        }
        assert!(fresh.is_dir(), "a folder under an hour old stays");
        assert!(foreign.is_dir(), "a name this module does not make stays");
        assert_eq!(std::fs::read(live.join("keep.txt")).unwrap(), b"old");
        assert_eq!(
            std::fs::read(outside.path().join("keep.txt")).unwrap(),
            b"outside"
        );
        assert!(link.symlink_metadata().is_ok(), "an old link is kept");
    }

    #[tokio::test]
    async fn a_stalled_error_body_is_not_read_past_the_cap() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 2048];
            let _ = std::io::Read::read(&mut sock, &mut buf);
            // One byte over the 4 KiB cap, so any larger cap reads on and
            // stalls.
            let head = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4097\r\nConnection: close\r\n\r\n{\"error\":";
            let _ = std::io::Write::write_all(&mut sock, head);
            std::thread::sleep(std::time::Duration::from_secs(4));
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let started = std::time::Instant::now();
        let report =
            sync_reader_packs(&format!("http://{addr}"), TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let elapsed = started.elapsed();
        assert_eq!(report.heroes(), &PackSync::Failed(PACK_FAILED.into()));
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "error body was read to the end: {elapsed:?}"
        );
    }

    // ---- review round 2 on #248 (F248-11..15, R248-B, N2) ----

    /// The thread each blocking file job ran on, by data dir.
    static BLOCKING_WORK: Mutex<Vec<(PathBuf, std::thread::ThreadId)>> = Mutex::new(Vec::new());

    pub(super) fn note_blocking_work(data_dir: &Path) {
        BLOCKING_WORK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((data_dir.to_path_buf(), std::thread::current().id()));
    }

    fn blocking_work_threads(data_dir: &Path) -> Vec<std::thread::ThreadId> {
        BLOCKING_WORK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|(dir, _)| dir == data_dir)
            .map(|(_, thread)| *thread)
            .collect()
    }

    /// Set a symlink's own mtime, not its target's.
    fn age_link(path: &Path, age: std::time::Duration) {
        use std::os::unix::ffi::OsStrExt;
        let when = std::time::SystemTime::now() - age;
        let secs = when
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after epoch")
            .as_secs() as libc::time_t;
        let times = [
            libc::timespec {
                tv_sec: secs,
                tv_nsec: 0,
            },
            libc::timespec {
                tv_sec: secs,
                tv_nsec: 0,
            },
        ];
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("path");
        let rc = unsafe {
            libc::utimensat(
                libc::AT_FDCWD,
                c_path.as_ptr(),
                times.as_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        assert_eq!(rc, 0, "utimensat: {}", std::io::Error::last_os_error());
        let link_age = path
            .symlink_metadata()
            .unwrap()
            .modified()
            .unwrap()
            .elapsed()
            .unwrap();
        assert!(link_age >= STALE_STAGING, "link aged: {link_age:?}");
    }

    #[tokio::test]
    async fn a_stop_on_heroes_v1_at_startup_queues_no_extras() {
        let stops: [(u16, &[u8], Duration); 4] = [
            (401, br#"{"error":"Unauthorized"}"#, secs(60)),
            (403, br#"{"error":"forbidden"}"#, secs(60)),
            (
                429,
                br#"{"error":"rate_limited","retry_after":30}"#,
                secs(30) + TEST_JITTER,
            ),
            (503, br#"{"error":"packs_disabled"}"#, secs(60)),
        ];
        for (status, body, wait) in stops {
            let served = all_packs_with_download(HEROES_PACK_NAME, status, body);
            let dir = tempfile::tempdir().expect("tempdir");
            let startup =
                sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES)
                    .await;
            assert!(
                startup.run.entries.is_some(),
                "HTTP {status}: the list is kept"
            );
            assert!(!startup.goes_on(), "HTTP {status}: the keeper waits first");
            // The keeper sends nothing more before its first wait is over.
            let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 0, no_nap)
                .run()
                .await;
            assert!(!finished, "HTTP {status}");
            assert_eq!(waits, [wait], "HTTP {status}");
            assert_eq!(
                served.downloaded(),
                vec![HEROES_PACK_NAME.to_string()],
                "HTTP {status}"
            );
            assert_eq!(list_requests(&served), 1, "HTTP {status}");
        }
        // A failure that is not a stop (a bad hash) does not hold the others.
        let served = all_packs_with_download(HEROES_PACK_NAME, 200, b"not the listed pack");
        let dir = tempfile::tempdir().expect("tempdir");
        let startup =
            sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert!(startup.goes_on(), "a mismatch is not a stop");
        let mut keeper = rig(&served.base, dir.path(), Some(startup), 0, no_nap).keeper;
        let report = keeper.first_pass().await.report;
        assert_eq!(
            report.heroes(),
            &PackSync::Failed(PACK_MISMATCH.to_string())
        );
        assert!(
            report.packs[1..]
                .iter()
                .all(|s| s.outcome == PackSync::Installed),
            "{report:?}"
        );
        assert_eq!(
            list_requests(&served),
            2,
            "a download that did not match the list: the list is read again"
        );
    }

    struct Gated {
        base: String,
        arrived: tokio::sync::oneshot::Receiver<()>,
        release: std::sync::mpsc::Sender<()>,
    }

    /// A fake that serves `list` and `files` like [`serve`], but holds the
    /// download of `gate` until `release` is sent. `arrived` fires once that
    /// request is in.
    fn serve_gated(list: Vec<serde_json::Value>, files: ServedFiles, gate: &str) -> Gated {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (arrived_tx, arrived) = tokio::sync::oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::channel::<()>();
        let list = serde_json::to_vec(&list).unwrap();
        let gate = gate.to_string();
        std::thread::spawn(move || {
            let mut arrived_tx = Some(arrived_tx);
            for _ in 0..16 {
                let Ok((mut sock, _)) = listener.accept() else {
                    break;
                };
                let Some(head) = read_http_head(&mut sock) else {
                    continue;
                };
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let response = if path == "/api/tracker/packs" {
                    http_bytes(200, &list, None, false)
                } else if let Some(name) = path.strip_prefix("/api/tracker/packs/") {
                    if name == gate
                        && let Some(tx) = arrived_tx.take()
                    {
                        let _ = tx.send(());
                        let _ = release_rx.recv();
                    }
                    match files.get(name) {
                        Some((status, body)) => http_bytes(*status, body, None, false),
                        None => http_bytes(404, br#"{"error":"pack_not_found"}"#, None, false),
                    }
                } else {
                    http_bytes(404, br#"{"error":"pack_not_found"}"#, None, false)
                };
                let _ = std::io::Write::write_all(&mut sock, &response);
            }
        });
        Gated {
            base: format!("http://{addr}"),
            arrived,
            release,
        }
    }

    #[tokio::test]
    async fn the_lock_is_held_for_the_whole_run() {
        let (list, files, _) = all_packs();
        let gated = serve_gated(list, files, "heroes-v2.tar");
        let dir = tempfile::tempdir().expect("tempdir");
        let (base, data_dir) = (gated.base.clone(), dir.path().to_path_buf());
        let run = tokio::spawn(async move {
            sync_reader_packs(&base, TOKEN, &data_dir, PACK_MAX_BYTES).await
        });
        // The run has the list and heroes-v1, and waits on heroes-v2.
        gated.arrived.await.expect("the run reached heroes-v2");
        assert!(
            SyncLock::try_acquire(dir.path())
                .expect("lock file")
                .is_none(),
            "the run still holds the lock mid-way"
        );
        let second = sync_reader_packs(&gated.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert!(
            second.packs.iter().all(|s| s.outcome == PackSync::Busy),
            "{second:?}"
        );
        gated.release.send(()).expect("release");
        let report = run.await.expect("run");
        assert!(
            report
                .packs
                .iter()
                .all(|s| s.outcome == PackSync::Installed),
            "{report:?}"
        );
        assert!(
            SyncLock::try_acquire(dir.path())
                .expect("lock file")
                .is_some(),
            "released when the run ends"
        );
    }

    #[tokio::test]
    async fn a_prev_folder_is_kept_while_its_live_pack_is_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let templates = dir.path().join("templates");
        let two_hours = std::time::Duration::from_secs(2 * 3600);
        // Killed between rename(live, backup) and rename(staging, live):
        // the backup is the only copy of bans-v1, and its mtime is old.
        let only_copy = templates.join(".bans-v1-prev-11-22");
        std::fs::create_dir_all(&only_copy).unwrap();
        std::fs::write(only_copy.join("ana.png"), b"only copy").unwrap();
        age_folder(&only_copy, two_hours);
        // A backup whose live folder is in place still goes by age.
        plant_keep(dir.path(), pack_spec("maps-v1.tar").unwrap());
        let spare = templates.join(".maps-v1-prev-33-44");
        std::fs::create_dir_all(&spare).unwrap();
        age_folder(&spare, two_hours);

        let served = serve(Vec::new(), BTreeMap::new());
        let report = sync(&served, dir.path()).await;
        assert_eq!(report.heroes(), &PackSync::Current, "empty list");
        assert_eq!(
            std::fs::read(only_copy.join("ana.png")).unwrap(),
            b"only copy"
        );
        assert!(
            !spare.exists(),
            "an old backup beside its live pack is swept"
        );
    }

    #[tokio::test]
    async fn a_versions_tmp_left_by_a_kill_is_removed_under_the_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let templates = dir.path().join("templates");
        std::fs::create_dir_all(&templates).unwrap();
        let left = templates.join(format!("{VERSIONS_TMP_PREFIX}123-456"));
        std::fs::write(&left, br#"{"heroes-v1.tar":"1"}"#).unwrap();
        let versions = br#"{"heroes-v1.tar":"1"}"#;
        std::fs::write(versions_path(dir.path()), versions).unwrap();
        let served = serve(Vec::new(), BTreeMap::new());

        let held = SyncLock::try_acquire(dir.path())
            .expect("lock file")
            .expect("free");
        let busy = sync(&served, dir.path()).await;
        assert_eq!(busy.heroes(), &PackSync::Busy);
        assert!(left.exists(), "not touched without the lock");
        drop(held);

        let report = sync(&served, dir.path()).await;
        assert_eq!(report.heroes(), &PackSync::Current, "empty list");
        assert!(!left.exists(), "the left tmp file is removed");
        assert_eq!(std::fs::read(versions_path(dir.path())).unwrap(), versions);
    }

    #[tokio::test]
    async fn the_background_job_does_its_file_work_off_the_runtime_thread() {
        // A current-thread runtime: every task runs on this thread, so file
        // work done inline would be recorded with this thread's id.
        let runtime_thread = std::thread::current().id();
        let (list, files, _) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let startup =
            sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let mut keeper = rig(&served.base, dir.path(), Some(startup), 0, no_nap).keeper;
        let report = keeper.first_pass().await.report;
        assert!(
            report
                .packs
                .iter()
                .all(|s| s.outcome == PackSync::Installed),
            "{report:?}"
        );
        let threads = blocking_work_threads(dir.path());
        // One install and one versions write per pack.
        assert_eq!(threads.len(), 2 * PACKS.len(), "{threads:?}");
        assert!(
            threads.iter().all(|thread| *thread != runtime_thread),
            "hashing, unpacking and fsync ran on the runtime thread"
        );
    }

    // ---- P1c: the keeper retries while the daemon runs ----
    // The waits are recorded, not slept, so these tests check the real
    // schedule and still take milliseconds.

    const LIST_PATH: &str = "/api/tracker/packs";
    /// The jitter every keeper under test uses instead of its pid's.
    const TEST_JITTER: Duration = Duration::from_millis(750);

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    /// A keeper under test, the waits it asked for, and a signal for "it
    /// went to sleep for good".
    struct Rig {
        keeper: PackKeeper,
        naps: Arc<Mutex<Vec<Duration>>>,
        parked: Arc<tokio::sync::Notify>,
    }

    impl Rig {
        /// Run the keeper. `true` when it finished by itself, `false` when
        /// it was still waiting after its last allowed nap (the test then
        /// drops it, as the daemon stopping does). With every wait it asked
        /// for.
        async fn run(self) -> (bool, Vec<Duration>) {
            let finished = tokio::select! {
                () = self.keeper.run() => true,
                () = self.parked.notified() => false,
            };
            let waits = self.naps.lock().expect("naps").clone();
            (finished, waits)
        }
    }

    /// A nap that is over at once.
    fn no_nap(_: usize) -> Nap {
        Box::pin(std::future::ready(()))
    }

    /// A keeper whose waits are recorded and replaced by `on_nap(n)` for
    /// nap `n` (from 0). Nap number `max_naps` never ends.
    fn rig_with(
        credentials: impl Fn() -> Option<PackCredentials> + Send + 'static,
        data_dir: &Path,
        startup: Option<StartupSync>,
        max_naps: usize,
        mut on_nap: impl FnMut(usize) -> Nap + Send + 'static,
    ) -> Rig {
        let mut keeper = PackKeeper::new(data_dir, startup, credentials);
        keeper.jitter = TEST_JITTER;
        let naps = Arc::new(Mutex::new(Vec::new()));
        let parked = Arc::new(tokio::sync::Notify::new());
        let (seen, park) = (naps.clone(), parked.clone());
        keeper.sleep = Box::new(move |wait| {
            let nap = {
                let mut seen = seen.lock().expect("naps");
                seen.push(wait);
                seen.len() - 1
            };
            if nap >= max_naps {
                park.notify_one();
                return Box::pin(std::future::pending());
            }
            on_nap(nap)
        });
        Rig {
            keeper,
            naps,
            parked,
        }
    }

    /// [`rig_with`] for one server and [`TOKEN`].
    fn rig(
        base: &str,
        data_dir: &Path,
        startup: Option<StartupSync>,
        max_naps: usize,
        on_nap: impl FnMut(usize) -> Nap + Send + 'static,
    ) -> Rig {
        let base = base.to_string();
        let credentials = move || {
            Some(PackCredentials {
                server_url: base.clone(),
                token: TOKEN.to_string(),
            })
        };
        rig_with(credentials, data_dir, startup, max_naps, on_nap)
    }

    /// One answer of a [`serve_script`] fake: status, body, `Retry-After`.
    /// `None` hangs up without an answer, which the tracker sees as a
    /// network error.
    type Answer = Option<(u16, Vec<u8>, Option<u64>)>;

    /// A fake whose answer depends on the path and on how many times that
    /// path was asked before. A request without the bearer token is a 401.
    fn serve_script(script: impl Fn(&str, usize) -> Answer + Send + 'static) -> Served {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requested = requested_paths();
        let (file_hits, paths) = (hits.clone(), requested.clone());
        std::thread::spawn(move || {
            for _ in 0..256 {
                let Ok((mut sock, _)) = listener.accept() else {
                    break;
                };
                let Some(head) = read_http_head(&mut sock) else {
                    continue;
                };
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let authed = head.lines().any(|line| {
                    line.eq_ignore_ascii_case(&format!("authorization: bearer {TOKEN}"))
                });
                let asked = {
                    let mut paths = paths.lock().expect("paths");
                    let asked = paths.iter().filter(|seen| **seen == path).count();
                    paths.push(path.clone());
                    asked
                };
                if path != LIST_PATH {
                    file_hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                let answer = if authed {
                    script(&path, asked)
                } else {
                    Some((401, br#"{"error":"Unauthorized"}"#.to_vec(), None))
                };
                if let Some((status, body, retry_after)) = answer {
                    let response = http_bytes(status, &body, retry_after, false);
                    let _ = std::io::Write::write_all(&mut sock, &response);
                }
            }
        });
        Served {
            base: format!("http://{addr}"),
            hits,
            requested,
        }
    }

    /// The answer of a site with every pack: the list, or the named tar.
    fn healthy(list: &[u8], files: &ServedFiles, path: &str) -> Answer {
        if path == LIST_PATH {
            return Some((200, list.to_vec(), None));
        }
        let file = path
            .strip_prefix("/api/tracker/packs/")
            .and_then(|name| files.get(name));
        Some(match file {
            Some((status, body)) => (*status, body.clone(), None),
            None => (404, br#"{"error":"pack_not_found"}"#.to_vec(), None),
        })
    }

    /// The list and files of [`all_packs`], ready for a [`serve_script`]
    /// closure to answer with [`healthy`].
    fn all_packs_for_script() -> (Vec<u8>, ServedFiles, PackContents) {
        let (list, files, contents) = all_packs();
        (serde_json::to_vec(&list).unwrap(), files, contents)
    }

    fn requests(served: &Served) -> Vec<String> {
        served.requested.lock().expect("paths").clone()
    }

    /// `/api/tracker/packs/<name>` for each name.
    fn tar_paths(names: &[&str]) -> Vec<String> {
        names
            .iter()
            .map(|name| format!("/api/tracker/packs/{name}"))
            .collect()
    }

    fn pack_names() -> Vec<&'static str> {
        PACKS.iter().map(|spec| spec.name).collect()
    }

    /// The list request, then one download per name.
    fn list_then(names: &[&str]) -> Vec<String> {
        let mut paths = vec![LIST_PATH.to_string()];
        paths.extend(tar_paths(names));
        paths
    }

    fn assert_all_installed(data_dir: &Path, contents: &PackContents) {
        for spec in PACKS {
            for (path, data) in &contents[spec.name] {
                assert_eq!(
                    &std::fs::read(spec.dir_in(data_dir).join(path)).expect(spec.name),
                    data,
                    "{} {path}",
                    spec.name
                );
            }
        }
        assert_eq!(stored_versions(data_dir).len(), PACKS.len());
        assert_eq!(leftovers(data_dir), Vec::<String>::new());
    }

    /// Everything under `templates/` that is not a pack folder, the
    /// versions file or the lock file: staging and backup folders, tmp files.
    fn leftovers(data_dir: &Path) -> Vec<String> {
        let live: Vec<String> = PACKS
            .iter()
            .map(|spec| dir_label(&spec.dir_in(data_dir)))
            .collect();
        let mut left: Vec<String> = std::fs::read_dir(data_dir.join("templates"))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        left.retain(|name| {
            !live.contains(name) && name != "pack-versions.json" && name != ".pack-sync.lock"
        });
        left.sort();
        left
    }

    /// Log lines of this thread while a test runs.
    struct Logs {
        buf: Arc<Mutex<Vec<u8>>>,
        _guard: tracing::subscriber::DefaultGuard,
        /// A second registered dispatcher, so that this one is never the
        /// only one. While exactly one dispatcher is registered,
        /// tracing-core decides whether a log call is on from the thread
        /// that reaches it first and keeps that answer for every thread.
        /// A test on another thread, without a subscriber, could then turn
        /// a line off for this test (seen once on CI and about 1 run in
        /// 150 here). With two registered, every one of them is asked.
        _not_the_only_one: tracing::Dispatch,
    }

    fn capture_logs() -> Logs {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || SharedWriter(writer.clone()))
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        Logs {
            buf,
            _guard: guard,
            _not_the_only_one: tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()),
        }
    }

    impl Logs {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.buf.lock().expect("log")).into_owned()
        }
    }

    /// The pack module's own lines, at the levels a member's log has.
    fn pack_lines(logs: &str) -> Vec<&str> {
        logs.lines()
            .filter(|line| line.contains("stat_tracker::packs"))
            .filter(|line| line.contains(" INFO ") || line.contains(" WARN "))
            .collect()
    }

    /// No pack line carries the server address, a URL, a query or a header.
    fn assert_no_address_in_pack_lines(logs: &str) {
        for line in pack_lines(logs) {
            let lower = line.to_ascii_lowercase();
            for banned in ["http", "127.0.0.1", "?", "authorization", "bearer", "/api/"] {
                assert!(!lower.contains(banned), "{banned:?} in {line}");
            }
        }
    }

    /// No token anywhere, and no address in a pack line.
    fn assert_no_secrets(logs: &str) {
        assert!(!logs.contains(TOKEN), "{logs}");
        assert_no_address_in_pack_lines(logs);
    }

    fn lines_with<'a>(logs: &'a str, needle: &str) -> Vec<&'a str> {
        pack_lines(logs)
            .into_iter()
            .filter(|line| line.contains(needle))
            .collect()
    }

    const SAID_OFF: &str =
        "the site has reader pack downloads turned off; the tracker will try again";
    const SAID_LIMITED: &str = "the site asked the tracker to wait";
    const SAID_BUSY: &str = "another reader pack sync is running; the tracker will try again";
    const SAID_MISSING: &str = "reader packs are still missing; the tracker will try again";
    const SAID_UNCHECKED: &str = "reader packs could not be checked for updates";
    const SAID_DONE: &str = "reader pack sync finished; the tracker stopped retrying";

    fn verdict(outcomes: &[PackSync]) -> Verdict {
        Verdict::of(&PackReport {
            packs: outcomes
                .iter()
                .map(|outcome| PackStatus {
                    name: HEROES_PACK_NAME,
                    outcome: outcome.clone(),
                })
                .collect(),
        })
    }

    const NONE: Verdict = Verdict {
        busy: false,
        limited: None,
        disabled: false,
        failed: false,
        refused: false,
        held: false,
    };
    const BUSY: Verdict = Verdict { busy: true, ..NONE };
    const FAILED: Verdict = Verdict {
        failed: true,
        ..NONE
    };
    const DISABLED: Verdict = Verdict {
        disabled: true,
        ..NONE
    };
    const REFUSED: Verdict = Verdict {
        refused: true,
        ..NONE
    };
    const HELD: Verdict = Verdict { held: true, ..NONE };

    fn limited(seconds: u64) -> Verdict {
        Verdict {
            limited: Some(seconds),
            ..NONE
        }
    }

    /// The waits a fresh [`Backoff`] gives for `verdicts`, in whole
    /// milliseconds, with [`TEST_JITTER`].
    fn waits_ms(verdicts: &[Verdict]) -> Vec<u128> {
        let mut backoff = Backoff::default();
        verdicts
            .iter()
            .map(|verdict| {
                backoff
                    .wait_after(*verdict, TEST_JITTER)
                    .expect("not settled")
                    .as_millis()
            })
            .collect()
    }

    #[test]
    fn nothing_left_to_try_is_no_wait() {
        let mut backoff = Backoff::default();
        assert_eq!(NONE, Verdict::default());
        assert_eq!(backoff.wait_after(NONE, TEST_JITTER), None);
        assert!(NONE.settled());
        for unsettled in [BUSY, FAILED, DISABLED, REFUSED, HELD, limited(1)] {
            assert!(!unsettled.settled(), "{unsettled:?}");
            assert!(backoff.wait_after(unsettled, TEST_JITTER).is_some());
        }
    }

    #[test]
    fn a_busy_lock_waits_5_then_15_then_60_seconds() {
        assert_eq!(
            waits_ms(&[BUSY; 5]),
            [5_000, 15_000, 60_000, 60_000, 60_000]
        );
        // The step starts over once the lock was free.
        assert_eq!(
            waits_ms(&[BUSY, BUSY, limited(9), BUSY, BUSY]),
            [5_000, 15_000, 9_750, 5_000, 15_000]
        );
    }

    #[test]
    fn a_failure_waits_1_then_5_then_15_then_30_minutes() {
        assert_eq!(
            waits_ms(&[FAILED; 6]),
            [60_000, 300_000, 900_000, 1_800_000, 1_800_000, 1_800_000]
        );
        // Packs switched off and a refused pack walk the same steps.
        assert_eq!(
            waits_ms(&[DISABLED, FAILED, REFUSED, DISABLED, REFUSED]),
            [60_000, 300_000, 900_000, 1_800_000, 1_800_000]
        );
        // The step does not start over after a 429 or a busy lock.
        assert_eq!(
            waits_ms(&[FAILED, limited(9), BUSY, FAILED, limited(9), FAILED]),
            [60_000, 9_750, 5_000, 300_000, 10_750, 900_000]
        );
    }

    #[test]
    fn a_429_waits_what_the_server_asked_within_1_to_120_seconds_plus_jitter() {
        // 7 s is taken as asked, plus the jitter.
        assert_eq!(waits_ms(&[limited(7)]), [7_750]);
        // Under the floor: 0 counts as 1, and 1 s + jitter is still 5 s.
        for under in [0, 1, 4] {
            assert_eq!(waits_ms(&[limited(under)]), [5_000], "{under}");
        }
        assert_eq!(waits_ms(&[limited(5)]), [5_750]);
        // Over the cap: 120 s, however long the server said.
        for over in [120, 121, 3600, u64::MAX] {
            assert_eq!(waits_ms(&[limited(over)]), [120_750], "{over}");
        }
        // No jitter, no extra.
        let mut backoff = Backoff::default();
        assert_eq!(
            backoff.wait_after(limited(7), Duration::ZERO),
            Some(secs(7))
        );
    }

    #[test]
    fn a_429_after_a_429_waits_at_least_10_20_40_80_then_120_seconds() {
        assert_eq!(
            waits_ms(&[limited(7); 8]),
            [
                7_750, 10_750, 20_750, 40_750, 80_750, 120_750, 120_750, 120_750
            ]
        );
        // Never under what the server asked.
        assert_eq!(
            waits_ms(&[limited(30); 5]),
            [30_750, 30_750, 30_750, 40_750, 80_750]
        );
        // A try without a 429 does not start the step over by itself: the
        // list getting through, or a busy lock, is not a download.
        assert_eq!(
            waits_ms(&[
                limited(7),
                limited(7),
                limited(7),
                HELD,
                limited(7),
                limited(7)
            ]),
            [7_750, 10_750, 20_750, 10_000, 40_750, 80_750]
        );
        assert_eq!(
            waits_ms(&[limited(7), limited(7), BUSY, limited(7)]),
            [7_750, 10_750, 5_000, 20_750]
        );
        // A pack that arrived does.
        let mut backoff = Backoff::default();
        let wait = |backoff: &mut Backoff| {
            backoff
                .wait_after(limited(7), TEST_JITTER)
                .expect("not settled")
        };
        assert_eq!(wait(&mut backoff), ms(7_750));
        assert_eq!(wait(&mut backoff), ms(10_750));
        assert_eq!(wait(&mut backoff), ms(20_750));
        backoff.pack_arrived();
        assert_eq!(wait(&mut backoff), ms(7_750));
        assert_eq!(wait(&mut backoff), ms(10_750));
    }

    #[test]
    fn while_paced_the_next_download_is_ten_seconds_away() {
        assert_eq!(waits_ms(&[HELD; 3]), [10_000; 3]);
        // A pack that is known to be refused does not slow the others
        // down, and the failure step stays where it was.
        let held_and_refused = Verdict {
            held: true,
            refused: true,
            ..NONE
        };
        assert_eq!(
            waits_ms(&[held_and_refused, held_and_refused, REFUSED, REFUSED]),
            [10_000, 10_000, 60_000, 300_000]
        );
        // Something that failed for another reason does.
        let held_and_failed = Verdict {
            held: true,
            failed: true,
            ..NONE
        };
        assert_eq!(waits_ms(&[held_and_failed, HELD]), [60_000, 10_000]);
        // With a 429 as well: the longer of the two.
        let held_and_limited = |seconds| Verdict {
            held: true,
            limited: Some(seconds),
            ..NONE
        };
        assert_eq!(waits_ms(&[held_and_limited(2)]), [10_000]);
        assert_eq!(waits_ms(&[held_and_limited(30)]), [30_750]);
    }

    #[test]
    fn a_try_that_ended_on_several_things_waits_the_longest() {
        let both = |seconds: u64, failed: bool, busy: bool| Verdict {
            busy,
            limited: Some(seconds),
            failed,
            ..NONE
        };
        // A failure and a 429 in one try: the minute, not 10 s.
        assert_eq!(waits_ms(&[both(10, true, false)]), [60_000]);
        // The 429 asks for longer than the failure row.
        assert_eq!(waits_ms(&[both(100, true, false)]), [100_750]);
        // Later rows are longer than any 429.
        assert_eq!(
            waits_ms(&[FAILED, both(120, true, false)]),
            [60_000, 300_000]
        );
        // Busy and a 429.
        assert_eq!(
            waits_ms(&[
                both(2, false, true),
                both(2, false, true),
                both(2, false, true)
            ]),
            [5_000, 15_000, 60_000]
        );
        // A try with both steps moves both on.
        let busy_and_failed = Verdict {
            busy: true,
            failed: true,
            ..NONE
        };
        assert_eq!(
            waits_ms(&[busy_and_failed, BUSY, FAILED]),
            [60_000, 15_000, 300_000]
        );
    }

    #[test]
    fn the_jitter_comes_from_the_pid_and_stays_under_five_seconds() {
        assert_eq!(pid_jitter(0), Duration::ZERO);
        assert_eq!(pid_jitter(1), Duration::from_millis(250));
        assert_eq!(pid_jitter(19), Duration::from_millis(4750));
        assert_eq!(pid_jitter(20), Duration::ZERO);
        assert_eq!(pid_jitter(4_194_303), Duration::from_millis(750));
        assert!((0..5000).all(|pid| pid_jitter(pid) < secs(5)));
        assert_eq!(pid_jitter(u32::MAX), Duration::from_millis(3750));
    }

    #[test]
    fn a_request_budget_counts_down_and_none_is_no_limit() {
        let mut none = None;
        assert!(take_request(&mut none) && take_request(&mut none));
        assert_eq!(none, None);
        let mut two = Some(2);
        assert!(take_request(&mut two));
        assert!(take_request(&mut two));
        assert!(!take_request(&mut two));
        assert!(!take_request(&mut two));
        assert_eq!(two, Some(0));
    }

    #[test]
    fn a_refusal_is_known_only_for_the_same_list_entry() {
        let entry = |version: &str, sha: &str, size: u64| PackListEntry {
            name: "heroes-v2.tar".into(),
            version: version.into(),
            sha256: sha.into(),
            size,
        };
        let was = entry("1", "aa", 10);
        let mut refused = Refused::new();
        refused.insert(was.name.clone(), (was.clone(), PACK_MISMATCH.to_string()));
        let known = |entry: &PackListEntry| known_refusal(Some(&refused), entry);
        assert_eq!(known(&was), Some(PACK_MISMATCH.to_string()));
        // Another version, sha256 or size is another entry: fetched again.
        assert_eq!(known(&entry("2", "aa", 10)), None);
        assert_eq!(known(&entry("1", "bb", 10)), None);
        assert_eq!(known(&entry("1", "aa", 11)), None);
        let other = PackListEntry {
            name: "maps-v1.tar".into(),
            ..was.clone()
        };
        assert_eq!(known(&other), None);
        assert_eq!(known_refusal(None, &was), None);
        // Only what the same bytes would get again counts as fixed.
        for message in [PACK_MISMATCH, PACK_UNSAFE, PACK_TOO_BIG] {
            assert!(is_fixed_refusal(message), "{message}");
        }
        for message in [PACK_FAILED, PACK_AUTH, PACK_FORBIDDEN, PACK_SIGN_IN] {
            assert!(!is_fixed_refusal(message), "{message}");
        }
        assert!(!is_fixed_refusal(&rate_limit_message(10)));
    }

    #[test]
    fn a_verdict_reads_every_outcome() {
        let settled = [
            PackSync::Installed,
            PackSync::Current,
            PackSync::Missing,
            PackSync::Unsupported,
        ];
        assert_eq!(verdict(&settled), NONE);
        for outcome in &settled {
            assert!(outcome.is_settled(), "{outcome:?}");
        }
        assert_eq!(verdict(&[PackSync::Current, PackSync::Busy]), BUSY);
        assert_eq!(verdict(&[PackSync::Unavailable]), DISABLED);
        for message in [PACK_FAILED, PACK_AUTH, PACK_FORBIDDEN, PACK_SIGN_IN] {
            let outcome = PackSync::Failed(message.to_string());
            assert!(!outcome.is_settled());
            assert!(!outcome.is_fixed_refusal());
            assert_eq!(outcome.rate_limit_secs(), None);
            assert_eq!(
                verdict(&[PackSync::Installed, outcome]),
                FAILED,
                "{message}"
            );
        }
        // A refusal the same bytes would get again is kept apart.
        for message in [PACK_MISMATCH, PACK_UNSAFE, PACK_TOO_BIG] {
            let outcome = PackSync::Failed(message.to_string());
            assert!(!outcome.is_settled());
            assert!(outcome.is_fixed_refusal());
            assert_eq!(
                verdict(&[PackSync::Installed, outcome]),
                REFUSED,
                "{message}"
            );
        }
        // A 429 keeps its seconds; of two, the longer one counts.
        let limit = |seconds| PackSync::Failed(rate_limit_message(seconds));
        assert_eq!(verdict(&[limit(30)]), limited(30));
        assert_eq!(verdict(&[limit(30), limit(90), limit(45)]), limited(90));
        for (asked, kept) in [
            (0, 1),
            (1, 1),
            (10, 10),
            (120, 120),
            (3600, 3600),
            (99_999, 3600),
        ] {
            assert_eq!(limit(asked).rate_limit_secs(), Some(kept), "{asked}");
        }
        for other in [PackSync::Installed, PackSync::Busy, PackSync::Unavailable] {
            assert_eq!(other.rate_limit_secs(), None);
            assert!(!other.is_fixed_refusal());
        }
        // Everything a report can show at once. Held-back downloads are
        // not in a report; the job adds them.
        let all = verdict(&[
            PackSync::Busy,
            PackSync::Unavailable,
            limit(8),
            PackSync::Failed(PACK_FAILED.to_string()),
            PackSync::Failed(PACK_MISMATCH.to_string()),
        ]);
        assert_eq!(
            all,
            Verdict {
                busy: true,
                limited: Some(8),
                disabled: true,
                failed: true,
                refused: true,
                held: false,
            }
        );
        let tried = Tried {
            report: PackReport { packs: Vec::new() },
            held_back: 2,
        };
        assert_eq!(tried.verdict(), HELD);
        // The one reason the log line names.
        assert_eq!(all.why(), Why::Failed);
        let no_failure = Verdict {
            failed: false,
            ..all
        };
        assert_eq!(no_failure.why(), Why::Disabled);
        let not_off = Verdict {
            disabled: false,
            ..no_failure
        };
        assert_eq!(not_off.why(), Why::Limited);
        let not_limited = Verdict {
            limited: None,
            ..not_off
        };
        assert_eq!(not_limited.why(), Why::Busy);
        assert_eq!(BUSY.why(), Why::Busy);
        assert_eq!(HELD.why(), Why::Paced);
        let held_and_refused = Verdict {
            held: true,
            ..REFUSED
        };
        assert_eq!(held_and_refused.why(), Why::Paced);
        assert_eq!(REFUSED.why(), Why::Failed);
    }

    /// Finding 1 of the pack download test, step for step: two starts on
    /// one data dir. The one that takes the pack lock loses the pid guard
    /// and exits; the one that found the lock busy is the daemon that stays.
    #[tokio::test]
    async fn the_start_that_lost_the_pack_lock_still_gets_every_pack() {
        let (list, files, contents) = all_packs_for_script();
        let (gate, hold) = gate();
        let served = serve_script(move |path, _| {
            if path.ends_with(HEROES_PACK_NAME) {
                hold.hold();
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        // Start A has the lock and is downloading heroes-v1.
        let (base, data_dir) = (served.base.clone(), dir.path().to_path_buf());
        let start_a = tokio::spawn(async move {
            sync_on_startup_capped(&base, TOKEN, &data_dir, PACK_MAX_BYTES).await
        });
        gate.arrived.await.expect("A asked for heroes-v1");
        // Start B finds the lock busy: no request, no list.
        let start_b = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(start_b.run.report.heroes(), &PackSync::Busy);
        assert!(start_b.run.entries.is_none());
        assert_eq!(requests(&served), list_then(&[HEROES_PACK_NAME]), "A only");
        // A finishes heroes-v1, then finds B's pid file and exits. Its
        // keeper never runs.
        gate.release.send(()).expect("release");
        let start_a = start_a.await.expect("A");
        assert_eq!(start_a.run.report.heroes(), &PackSync::Installed);
        drop(start_a);
        // B is the daemon that stays, and its keeper gets the other five.
        let (finished, waits) = rig(&served.base, dir.path(), Some(start_b), 8, no_nap)
            .run()
            .await;
        assert!(finished);
        assert_eq!(waits, [secs(5)], "one busy wait, then the try");
        assert_all_installed(dir.path(), &contents);
        let mut expected = list_then(&[HEROES_PACK_NAME]);
        expected.extend(list_then(&pack_names()[1..]));
        assert_eq!(requests(&served), expected, "no pack is fetched twice");
    }

    #[tokio::test]
    async fn while_the_lock_stays_busy_the_keeper_waits_5_15_60_and_sends_nothing() {
        let (list, files, contents) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let other = SyncLock::try_acquire(dir.path())
            .expect("lock file")
            .expect("free");
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        // The other sync lets go during the fourth wait.
        let held = Arc::new(Mutex::new(Some(other)));
        let sent_by_nap = Arc::new(Mutex::new(Vec::new()));
        let (release, sent, paths) = (held.clone(), sent_by_nap.clone(), served.requested.clone());
        let on_nap = move |nap| {
            sent.lock().unwrap().push(paths.lock().unwrap().len());
            if nap == 3 {
                release.lock().unwrap().take();
            }
            no_nap(nap)
        };
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, on_nap)
            .run()
            .await;
        let logs = logs.text();
        assert!(finished);
        assert_eq!(waits, [secs(5), secs(15), secs(60), secs(60)]);
        assert_eq!(
            *sent_by_nap.lock().unwrap(),
            [0, 0, 0, 0],
            "busy sends nothing"
        );
        assert_all_installed(dir.path(), &contents);
        assert_eq!(requests(&served), list_then(&pack_names()));
        assert_eq!(lines_with(&logs, SAID_BUSY).len(), 1, "{logs}");
        assert!(
            lines_with(&logs, SAID_BUSY)[0].contains("wait_secs=5"),
            "{logs}"
        );
        let done = lines_with(&logs, SAID_DONE);
        assert_eq!(done.len(), 1, "{logs}");
        assert!(done[0].contains("tries=5"), "{logs}");
        assert_no_secrets(&logs);
    }

    #[tokio::test]
    async fn after_a_429_only_the_refused_download_is_sent_and_the_rest_ten_seconds_apart() {
        let (list, files, contents) = all_packs_for_script();
        let served = serve_script(move |path, asked| {
            if path.ends_with("/heroes-v2.tar") && asked == 0 {
                let body = br#"{"error":"rate_limited","retry_after":7}"#;
                return Some((429, body.to_vec(), Some(7)));
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert!(startup.goes_on());
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, no_nap)
            .run()
            .await;
        let logs = logs.text();
        assert!(finished);
        // What the server asked for, then one download every 10 s.
        assert_eq!(
            waits,
            [
                secs(7) + TEST_JITTER,
                secs(10),
                secs(10),
                secs(10),
                secs(10),
                secs(10)
            ]
        );
        assert_all_installed(dir.path(), &contents);
        // Startup (2), the refused download, then the six downloads. The
        // list is not asked for again.
        let mut expected = list_then(&[HEROES_PACK_NAME, "heroes-v2.tar"]);
        expected.extend(tar_paths(&pack_names()[1..]));
        assert_eq!(requests(&served), expected);
        let said = lines_with(&logs, SAID_LIMITED);
        assert_eq!(said.len(), 1, "{logs}");
        assert!(said[0].contains("wait_secs=7"), "{logs}");
        assert!(
            said[0].contains(&format!("missing=\"{}\"", pack_names()[1..].join(", "))),
            "{logs}"
        );
        let done = lines_with(&logs, SAID_DONE);
        assert_eq!(done.len(), 1, "{logs}");
        assert!(done[0].contains("tries=7"), "{logs}");
        assert!(
            done[0].contains(&format!("arrived=\"{}\"", pack_names().join(", "))),
            "{logs}"
        );
        // Seven installs, the 429 line and the closing line. The paced waits
        // are not written for a member.
        assert_eq!(pack_lines(&logs).len(), 9, "{logs}");
        assert_no_secrets(&logs);
    }

    #[tokio::test]
    async fn a_429_on_the_startup_list_holds_every_request_for_at_most_two_minutes() {
        let (list, files, contents) = all_packs_for_script();
        let served = serve_script(move |path, asked| {
            if path == LIST_PATH && asked == 0 {
                // No header: the wait is in the body, as the site sends it.
                let body = br#"{"error":"rate_limited","retry_after":300}"#;
                return Some((429, body.to_vec(), None));
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert!(startup.run.entries.is_none());
        let sent_by_nap = Arc::new(Mutex::new(Vec::new()));
        let (sent, paths) = (sent_by_nap.clone(), served.requested.clone());
        let on_nap = move |nap| {
            sent.lock().unwrap().push(paths.lock().unwrap().len());
            no_nap(nap)
        };
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, on_nap)
            .run()
            .await;
        assert!(finished);
        // 300 s is cut to 120 s. Then one request per try: the list, and
        // each pack 10 s after the one before.
        let mut expected_waits = vec![secs(120) + TEST_JITTER];
        expected_waits.extend([secs(10); 7]);
        assert_eq!(waits, expected_waits);
        assert_eq!(*sent_by_nap.lock().unwrap(), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_all_installed(dir.path(), &contents);
        let mut expected = vec![LIST_PATH.to_string()];
        expected.extend(list_then(&pack_names()));
        assert_eq!(requests(&served), expected);
    }

    #[tokio::test]
    async fn packs_switched_off_is_said_once_and_they_install_when_the_site_turns_them_on() {
        let (list, files, contents) = all_packs_for_script();
        let served = serve_script(move |path, asked| {
            if path == LIST_PATH && asked < 3 {
                return Some((503, br#"{"error":"packs_disabled"}"#.to_vec(), None));
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(startup.run.report.heroes(), &PackSync::Unavailable);
        let rig = rig(&served.base, dir.path(), Some(startup), 8, no_nap);
        assert!(!rig.keeper.starts_with_downloads(), "it waits first");
        let (finished, waits) = rig.run().await;
        let logs = logs.text();
        assert!(finished);
        assert_eq!(waits, [secs(60), secs(300), secs(900)]);
        assert_all_installed(dir.path(), &contents);
        // Three lists refused (startup and two tries), then list and six.
        let mut expected = vec![LIST_PATH.to_string(); 3];
        expected.extend(list_then(&pack_names()));
        assert_eq!(requests(&served), expected);
        let said = lines_with(&logs, SAID_OFF);
        assert_eq!(said.len(), 1, "once, not on every try:\n{logs}");
        assert!(said[0].contains(" INFO "), "{logs}");
        assert!(said[0].contains("wait_secs=60"), "{logs}");
        assert!(
            said[0].contains(&format!("missing=\"{}\"", pack_names().join(", "))),
            "{logs}"
        );
        let done = lines_with(&logs, SAID_DONE);
        assert_eq!(done.len(), 1, "{logs}");
        assert!(done[0].contains("tries=4"), "{logs}");
        assert!(
            done[0].contains(&format!("arrived=\"{}\"", pack_names().join(", "))),
            "{logs}"
        );
        assert_eq!(
            lines_with(&logs, "reader pack installed").len(),
            PACKS.len(),
            "{logs}"
        );
        assert!(lines_with(&logs, " WARN ").is_empty(), "{logs}");
        assert_no_secrets(&logs);
    }

    #[tokio::test]
    async fn a_dropped_connection_is_tried_again_after_a_minute() {
        let (list, files, contents) = all_packs_for_script();
        let served = serve_script(move |path, asked| {
            if path == LIST_PATH && asked == 0 {
                return None;
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(
            startup.run.report.heroes(),
            &PackSync::Failed(PACK_FAILED.to_string())
        );
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, no_nap)
            .run()
            .await;
        let logs = logs.text();
        assert!(finished);
        assert_eq!(waits, [secs(60)]);
        assert_all_installed(dir.path(), &contents);
        let mut expected = vec![LIST_PATH.to_string()];
        expected.extend(list_then(&pack_names()));
        assert_eq!(requests(&served), expected);
        assert_eq!(lines_with(&logs, SAID_MISSING).len(), 1, "{logs}");
        assert_eq!(lines_with(&logs, SAID_DONE).len(), 1, "{logs}");
        assert_no_secrets(&logs);
    }

    /// An address that refuses connections while the socket is kept: bound
    /// and never listening, so no other test can be given the port.
    fn refusing_base() -> (String, std::os::fd::OwnedFd) {
        use std::os::fd::FromRawFd;
        let len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        // SAFETY: socket, bind and getsockname on a new fd that `socket`
        // owns from the next line on; `addr` is a zeroed sockaddr_in of
        // the length passed.
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
            assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
            let socket = std::os::fd::OwnedFd::from_raw_fd(fd);
            let mut addr: libc::sockaddr_in = std::mem::zeroed();
            addr.sin_family = libc::AF_INET as libc::sa_family_t;
            addr.sin_addr.s_addr = u32::from(std::net::Ipv4Addr::LOCALHOST).to_be();
            let raw = std::ptr::addr_of_mut!(addr).cast::<libc::sockaddr>();
            assert_eq!(libc::bind(fd, raw, len), 0, "bind");
            let mut got = len;
            assert_eq!(libc::getsockname(fd, raw, &mut got), 0, "getsockname");
            let port = u16::from_be(addr.sin_port);
            (format!("http://127.0.0.1:{port}"), socket)
        }
    }

    #[tokio::test]
    async fn with_the_site_unreachable_the_log_names_the_missing_packs_and_not_heroes_v1() {
        let (list, files, contents) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        // An earlier run installed heroes-v1 and nothing else.
        let earlier = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(earlier.run.report.heroes(), &PackSync::Installed);
        drop(earlier);
        // This start: the site does not answer (no network yet).
        let (dead, _refusing) = refusing_base();
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&dead, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(
            startup.run.report.heroes(),
            &PackSync::Failed(PACK_FAILED.to_string())
        );
        // It answers again during the keeper's second wait.
        let base = Arc::new(Mutex::new(dead));
        let (now, live) = (base.clone(), served.base.clone());
        let credentials = move || {
            Some(PackCredentials {
                server_url: base.lock().unwrap().clone(),
                token: TOKEN.to_string(),
            })
        };
        let on_nap = move |nap| {
            if nap == 1 {
                *now.lock().unwrap() = live.clone();
            }
            no_nap(nap)
        };
        let (finished, waits) = rig_with(credentials, dir.path(), Some(startup), 8, on_nap)
            .run()
            .await;
        let logs = logs.text();
        assert!(finished, "still waiting: {waits:?}\n{logs}");
        assert_eq!(waits, [secs(60), secs(300)]);
        assert_all_installed(dir.path(), &contents);
        let said = lines_with(&logs, SAID_MISSING);
        assert_eq!(said.len(), 1, "two failed tries, one line:\n{logs}");
        assert!(said[0].contains(" WARN "), "{logs}");
        assert!(
            said[0].contains(&format!("missing=\"{}\"", pack_names()[1..].join(", "))),
            "{logs}"
        );
        assert!(!said[0].contains(HEROES_PACK_NAME), "{logs}");
        assert!(!said[0].contains("refused="), "{logs}");
        assert!(said[0].contains(PACK_FAILED), "{logs}");
        let done = lines_with(&logs, SAID_DONE);
        assert_eq!(done.len(), 1, "{logs}");
        assert!(
            done[0].contains(&format!("arrived=\"{}\"", pack_names()[1..].join(", "))),
            "{logs}"
        );
        assert_no_secrets(&logs);
    }

    #[tokio::test]
    async fn with_every_pack_installed_an_unreachable_site_is_an_update_check_that_failed() {
        let (list, files, _) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        sync(&served, dir.path()).await;
        let (dead, _refusing) = refusing_base();
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&dead, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let (finished, waits) = rig(&dead, dir.path(), Some(startup), 1, no_nap).run().await;
        let logs = logs.text();
        assert!(!finished);
        assert_eq!(waits, [secs(60), secs(300)]);
        let said = lines_with(&logs, SAID_UNCHECKED);
        assert_eq!(said.len(), 1, "{logs}");
        assert!(!said[0].contains(".tar"), "no pack is named:\n{logs}");
        assert!(lines_with(&logs, SAID_MISSING).is_empty(), "{logs}");
        assert_no_secrets(&logs);
    }

    #[tokio::test]
    async fn while_sync_is_paused_nothing_is_sent_and_a_new_token_is_used_when_it_comes() {
        let (list, files, contents) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        // No startup sync ran (the token was rejected earlier). Settings
        // gets a working token during the second wait.
        let signed_in = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (ready, base) = (signed_in.clone(), served.base.clone());
        let credentials = move || {
            ready
                .load(std::sync::atomic::Ordering::SeqCst)
                .then(|| PackCredentials {
                    server_url: base.clone(),
                    token: TOKEN.to_string(),
                })
        };
        let sent_by_nap = Arc::new(Mutex::new(Vec::new()));
        let (sent, paths) = (sent_by_nap.clone(), served.requested.clone());
        let on_nap = move |nap| {
            sent.lock().unwrap().push(paths.lock().unwrap().len());
            if nap == 1 {
                signed_in.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            no_nap(nap)
        };
        let rig = rig_with(credentials, dir.path(), None, 8, on_nap);
        assert!(!rig.keeper.starts_with_downloads(), "no startup sync ran");
        let (finished, waits) = rig.run().await;
        let logs = logs.text();
        assert!(finished);
        assert_eq!(waits, [secs(60), secs(300)]);
        assert_eq!(*sent_by_nap.lock().unwrap(), [0, 0]);
        assert_all_installed(dir.path(), &contents);
        assert_eq!(requests(&served), list_then(&pack_names()));
        let said = lines_with(&logs, SAID_MISSING);
        assert_eq!(said.len(), 1, "{logs}");
        assert!(said[0].contains(PACK_SIGN_IN), "{logs}");
        assert_no_secrets(&logs);
    }

    #[tokio::test]
    async fn a_refused_pack_is_downloaded_once_and_then_only_the_list_is_read() {
        let served = all_packs_with_download("heroes-v2.tar", 200, b"not the listed pack");
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        // Five waits pass; the keeper is still waiting in the sixth.
        let sent_by_nap = Arc::new(Mutex::new(Vec::new()));
        let (sent, paths) = (sent_by_nap.clone(), served.requested.clone());
        let on_nap = move |nap| {
            sent.lock().unwrap().push(paths.lock().unwrap().len());
            no_nap(nap)
        };
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 5, on_nap)
            .run()
            .await;
        let logs = logs.text();
        assert!(!finished, "a pack that never matches is never settled");
        assert_eq!(
            waits,
            [
                secs(60),
                secs(300),
                secs(900),
                secs(1800),
                secs(1800),
                secs(1800)
            ]
        );
        // The refused pack is downloaded once, and so is every other pack.
        let asked = requests(&served);
        let fetched = |name: &str| asked.iter().filter(|path| path.ends_with(name)).count();
        for spec in PACKS {
            assert_eq!(fetched(&format!("/{}", spec.name)), 1, "{}", spec.name);
        }
        // Startup and the first try are 8 requests. The next try reads the
        // list again, because the download did not match it. After that
        // the list is read only once it is 30 minutes old.
        assert_eq!(*sent_by_nap.lock().unwrap(), [8, 9, 9, 9, 10]);
        assert_eq!(list_requests(&served), 4);
        assert_eq!(asked.len(), 11);
        assert!(
            !pack_spec("heroes-v2.tar")
                .unwrap()
                .dir_in(dir.path())
                .exists()
        );
        assert_eq!(stored_versions(dir.path()).len(), PACKS.len() - 1);
        assert_eq!(leftovers(dir.path()), Vec::<String>::new());
        let said = lines_with(&logs, SAID_MISSING);
        assert_eq!(said.len(), 1, "one line for the refused pack:\n{logs}");
        assert!(said[0].contains(" WARN "), "{logs}");
        assert!(said[0].contains("missing=\"heroes-v2.tar\""), "{logs}");
        assert!(said[0].contains("refused=\"heroes-v2.tar\""), "{logs}");
        assert!(said[0].contains(PACK_MISMATCH), "{logs}");
        assert!(lines_with(&logs, SAID_DONE).is_empty(), "{logs}");
        assert_no_secrets(&logs);
    }

    #[tokio::test]
    async fn a_refused_pack_is_downloaded_again_when_its_list_entry_changes() {
        let (list, files, contents) = all_packs();
        // The site lists heroes-v2 and serves other bytes under that name.
        // From the third list on it is fixed: a new version, with its bytes.
        let fixed_tar = build_pack("heroes-v2.tar", "154088.2", &contents["heroes-v2.tar"]);
        let fixed_list: Vec<serde_json::Value> = list
            .iter()
            .map(|entry| {
                if entry["name"] == "heroes-v2.tar" {
                    entry_json("heroes-v2.tar", "154088.2", &fixed_tar)
                } else {
                    entry.clone()
                }
            })
            .collect();
        let (broken_list, fixed_list) = (
            serde_json::to_vec(&list).unwrap(),
            serde_json::to_vec(&fixed_list).unwrap(),
        );
        let fixed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let now_fixed = fixed.clone();
        let served = serve_script(move |path, asked| {
            use std::sync::atomic::Ordering::SeqCst;
            if path == LIST_PATH && asked >= 2 {
                now_fixed.store(true, SeqCst);
                return Some((200, fixed_list.clone(), None));
            }
            if path.ends_with("/heroes-v2.tar") {
                let body = if now_fixed.load(SeqCst) {
                    fixed_tar.clone()
                } else {
                    b"not the listed pack".to_vec()
                };
                return Some((200, body, None));
            }
            healthy(&broken_list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, no_nap)
            .run()
            .await;
        let logs = logs.text();
        assert!(finished, "the fixed pack settles it");
        assert!(fixed.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(waits, [secs(60), secs(300), secs(900), secs(1800)]);
        assert_all_installed(dir.path(), &contents);
        assert_eq!(stored_versions(dir.path())["heroes-v2.tar"], "154088.2");
        // Startup 2, the first try 6, the list after the mismatch, the
        // list at 30 minutes, and the pack again under its new entry.
        let asked = requests(&served);
        let fetched = |name: &str| asked.iter().filter(|path| path.ends_with(name)).count();
        assert_eq!(fetched("/heroes-v2.tar"), 2);
        assert_eq!(list_requests(&served), 3);
        assert_eq!(asked.len(), 11);
        assert_eq!(lines_with(&logs, SAID_MISSING).len(), 1, "{logs}");
        let done = lines_with(&logs, SAID_DONE);
        assert_eq!(done.len(), 1, "{logs}");
        assert!(done[0].contains("heroes-v2.tar"), "{logs}");
        assert_no_secrets(&logs);
    }

    #[tokio::test]
    async fn a_listed_pack_that_is_a_404_makes_the_next_try_read_the_list_again() {
        let (list, files, contents) = all_packs_for_script();
        let served = serve_script(move |path, asked| {
            if path.ends_with("/heroes-v2.tar") && asked == 0 {
                return Some((404, br#"{"error":"pack_not_found"}"#.to_vec(), None));
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, no_nap)
            .run()
            .await;
        assert!(finished);
        assert_eq!(waits, [secs(60)]);
        assert_all_installed(dir.path(), &contents);
        // The other four are not held up by the 404. The next try is the
        // list and the one pack.
        let mut expected = list_then(&pack_names());
        expected.extend(list_then(&["heroes-v2.tar"]));
        assert_eq!(requests(&served), expected);
    }

    #[tokio::test]
    async fn a_download_that_failed_on_the_network_is_tried_again_without_the_list() {
        let (list, files, contents) = all_packs_for_script();
        let served = serve_script(move |path, asked| {
            if path.ends_with("/heroes-v2.tar") && asked == 0 {
                return None;
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, no_nap)
            .run()
            .await;
        assert!(finished);
        assert_eq!(waits, [secs(60)]);
        assert_all_installed(dir.path(), &contents);
        let mut expected = list_then(&pack_names());
        expected.extend(tar_paths(&["heroes-v2.tar"]));
        assert_eq!(requests(&served), expected, "the list it has is used");
    }

    #[tokio::test]
    async fn a_fresh_install_is_seven_requests_and_no_wait() {
        let (list, files, contents) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let rig = rig(&served.base, dir.path(), Some(startup), 8, no_nap);
        assert!(rig.keeper.starts_with_downloads(), "startup read the list");
        let (finished, waits) = rig.run().await;
        let logs = logs.text();
        assert!(finished);
        assert_eq!(waits, Vec::<Duration>::new());
        assert_all_installed(dir.path(), &contents);
        assert_eq!(requests(&served), list_then(&pack_names()));
        // As before this change: one line per pack, nothing else.
        let lines = pack_lines(&logs);
        assert_eq!(lines.len(), PACKS.len(), "{logs}");
        assert!(
            lines
                .iter()
                .all(|line| line.contains("reader pack installed"))
        );
        assert_no_secrets(&logs);
    }

    #[tokio::test]
    async fn a_current_install_is_one_request_per_start_and_the_keeper_stops() {
        let (list, files, contents) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        sync(&served, dir.path()).await;
        assert_all_installed(dir.path(), &contents);
        let before = requests(&served).len();
        let logs = capture_logs();
        for start in 0..3 {
            let startup =
                sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
            let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, no_nap)
                .run()
                .await;
            assert!(finished, "start {start}");
            assert_eq!(waits, Vec::<Duration>::new(), "start {start}");
        }
        let logs = logs.text();
        assert_eq!(
            requests(&served)[before..],
            vec![LIST_PATH.to_string(); 3],
            "the list, once per start"
        );
        assert_eq!(pack_lines(&logs), Vec::<&str>::new(), "nothing to say");
    }

    #[tokio::test]
    async fn packs_the_site_does_not_offer_are_not_waited_for() {
        let (list, files, _) = all_packs();
        let only_heroes: Vec<serde_json::Value> = list
            .into_iter()
            .filter(|entry| entry["name"] == HEROES_PACK_NAME)
            .collect();
        let served = serve(only_heroes, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, no_nap)
            .run()
            .await;
        let logs = logs.text();
        assert!(finished, "an older site is not polled for packs it lacks");
        assert_eq!(waits, Vec::<Duration>::new());
        assert_eq!(requests(&served), list_then(&[HEROES_PACK_NAME]));
        assert_eq!(pack_lines(&logs).len(), 1, "{logs}");
        assert!(
            pack_lines(&logs)[0].contains("reader pack installed"),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_server_without_packs_is_said_once_and_not_asked_again() {
        let served = serve_script(|_, _| Some((404, br#"{"error":"not_found"}"#.to_vec(), None)));
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = capture_logs();
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(startup.run.report.heroes(), &PackSync::Unsupported);
        let (finished, waits) = rig(&served.base, dir.path(), Some(startup), 8, no_nap)
            .run()
            .await;
        let logs = logs.text();
        assert!(finished);
        assert_eq!(waits, Vec::<Duration>::new());
        assert_eq!(requests(&served), [LIST_PATH]);
        assert_eq!(
            lines_with(&logs, "reader packs are not supported on this server yet").len(),
            1,
            "{logs}"
        );
    }

    /// Lets a test hold one answer of a [`serve_script`] fake.
    struct Gate {
        arrived: tokio::sync::oneshot::Receiver<()>,
        release: std::sync::mpsc::Sender<()>,
    }

    /// The fake's side of a [`Gate`].
    struct Hold {
        arrived: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    fn gate() -> (Gate, Hold) {
        let (arrived_tx, arrived) = tokio::sync::oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let hold = Hold {
            arrived: Mutex::new(Some(arrived_tx)),
            release: Mutex::new(release_rx),
        };
        (Gate { arrived, release }, hold)
    }

    impl Hold {
        /// The first call tells the test the request is in and waits for
        /// the release (or for the test to drop its end). Later calls
        /// return at once.
        fn hold(&self) {
            let first = self.arrived.lock().expect("gate").take();
            if let Some(arrived) = first {
                let _ = arrived.send(());
                let _ = self.release.lock().expect("gate").recv();
            }
        }
    }

    #[tokio::test]
    async fn two_keepers_and_the_setup_guide_take_turns_on_the_lock() {
        let (list, files, contents) = all_packs_for_script();
        let (gate, hold) = gate();
        let served = serve_script(move |path, _| {
            if path.ends_with("/heroes-v2.tar") {
                hold.hold();
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        // Daemon A: startup done, its keeper is in the middle of heroes-v2.
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let a_done = Arc::new(tokio::sync::Notify::new());
        let (rig_a, done) = (
            rig(&served.base, dir.path(), Some(startup), 8, no_nap),
            a_done.clone(),
        );
        let keeper_a = tokio::spawn(async move {
            let ran = rig_a.run().await;
            done.notify_one();
            ran
        });
        gate.arrived.await.expect("A asked for heroes-v2");
        let in_flight = list_then(&[HEROES_PACK_NAME, "heroes-v2.tar"]);
        assert_eq!(requests(&served), in_flight);
        // The setup guide at the same time: busy, and it sends nothing.
        let guide = sync(&served, dir.path()).await;
        assert!(
            guide.packs.iter().all(|s| s.outcome == PackSync::Busy),
            "{guide:?}"
        );
        assert_eq!(requests(&served), in_flight);
        // A second keeper on the same data dir: busy, it waits its 5 s.
        // During that wait A is let go and finishes.
        let release = Mutex::new(Some(gate.release));
        let on_nap = move |_| -> Nap {
            release.lock().unwrap().take();
            let done = a_done.clone();
            Box::pin(async move { done.notified().await })
        };
        let (b_finished, b_waits) = rig(&served.base, dir.path(), None, 8, on_nap).run().await;
        let (a_finished, a_waits) = keeper_a.await.expect("A");
        assert!(a_finished && b_finished);
        assert_eq!(a_waits, Vec::<Duration>::new());
        assert_eq!(b_waits, [secs(5)]);
        assert_all_installed(dir.path(), &contents);
        // A: list and six. B: nothing while busy, then one list. No pack
        // is downloaded twice.
        let mut expected = list_then(&pack_names());
        expected.push(LIST_PATH.to_string());
        assert_eq!(requests(&served), expected);
        // And the setup guide afterwards: one list, all current.
        let guide = sync(&served, dir.path()).await;
        assert!(
            guide.packs.iter().all(|s| s.outcome == PackSync::Current),
            "{guide:?}"
        );
        assert_eq!(requests(&served).len(), expected.len() + 1);
    }

    /// `true` when a sync lock that has just been dropped can be taken
    /// again while the child forked under that lock is still alive.
    ///
    /// `fork` copies the open file description into the child. `close` on
    /// the parent's side does not release an `flock` until the child closes
    /// its copy (at exec, or when the child exits). The child here never
    /// execs: it waits until this function is done, so the inherited
    /// descriptor stays open across the drop and the next acquire.
    fn lock_drop_is_visible_while_a_child_holds_the_fd(data_dir: &Path) -> bool {
        let mut ready = [0; 2];
        let mut release = [0; 2];
        // SAFETY: two fresh pipes. The child inherits both ends and uses
        // only async-signal-safe calls (`close`, `write`, `read`, `_exit`).
        let pipes_ok =
            unsafe { libc::pipe(ready.as_mut_ptr()) == 0 && libc::pipe(release.as_mut_ptr()) == 0 };
        assert!(pipes_ok, "pipe: {}", std::io::Error::last_os_error());
        let held = SyncLock::try_acquire(data_dir)
            .expect("lock file")
            .expect("lock was free");
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
        if pid == 0 {
            unsafe {
                libc::close(ready[0]);
                libc::close(release[1]);
                let byte = 1u8;
                libc::write(ready[1], &byte as *const u8 as *const libc::c_void, 1);
                libc::close(ready[1]);
                let mut buf = [0u8; 1];
                libc::read(release[0], buf.as_mut_ptr() as *mut libc::c_void, 1);
                libc::close(release[0]);
                libc::_exit(0);
            }
        }
        struct Reap {
            pid: libc::pid_t,
            release_write: libc::c_int,
        }
        impl Drop for Reap {
            fn drop(&mut self) {
                let byte = 1u8;
                unsafe {
                    libc::write(
                        self.release_write,
                        &byte as *const u8 as *const libc::c_void,
                        1,
                    );
                    libc::close(self.release_write);
                    let mut status = 0;
                    libc::waitpid(self.pid, &mut status, 0);
                }
            }
        }
        let _reap = Reap {
            pid,
            release_write: release[1],
        };
        unsafe {
            libc::close(ready[1]);
            libc::close(release[0]);
        }
        let mut saw = [0u8; 1];
        let n = unsafe { libc::read(ready[0], saw.as_mut_ptr() as *mut libc::c_void, 1) };
        unsafe { libc::close(ready[0]) };
        assert_eq!(n, 1, "the child never signalled that it was alive");
        assert!(
            held.is_cloexec(),
            "the pack lock descriptor is inherited across exec"
        );
        drop(held);
        SyncLock::try_acquire(data_dir)
            .expect("lock file")
            .is_some()
    }

    #[test]
    fn dropping_the_sync_lock_frees_it_while_a_forked_child_still_has_the_fd() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cycles = 50;
        let mut missed = 0u32;
        for _ in 0..cycles {
            if !lock_drop_is_visible_while_a_child_holds_the_fd(dir.path()) {
                missed += 1;
            }
        }
        assert_eq!(
            missed, 0,
            "the pack lock was still held after drop in {missed}/{cycles} cycles while a forked child kept the inherited descriptor"
        );
    }

    fn lock_is_free(data_dir: &Path) -> bool {
        SyncLock::try_acquire(data_dir)
            .expect("lock file")
            .is_some()
    }

    /// The daemon's own runtime shape, so a test can stop it the way
    /// `main` returning on SIGTERM does: by dropping the runtime.
    fn daemon_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime")
    }

    #[test]
    fn stopping_the_daemon_while_the_keeper_sleeps_is_prompt_and_leaves_nothing_behind() {
        let served =
            serve_script(|_, _| Some((503, br#"{"error":"packs_disabled"}"#.to_vec(), None)));
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime = daemon_runtime();
        let startup = runtime.block_on(sync_on_startup_capped(
            &served.base,
            TOKEN,
            dir.path(),
            PACK_MAX_BYTES,
        ));
        // The real sleep, here a minute. The test only learns that it began.
        let base = served.base.clone();
        let mut keeper = PackKeeper::new(dir.path(), Some(startup), move || {
            Some(PackCredentials {
                server_url: base.clone(),
                token: TOKEN.to_string(),
            })
        });
        let (asleep_tx, asleep) = std::sync::mpsc::channel();
        keeper.sleep = Box::new(move |wait| {
            let _ = asleep_tx.send(wait);
            Box::pin(tokio::time::sleep(wait))
        });
        let task = runtime.spawn(keeper.run());
        let wait = asleep
            .recv_timeout(Duration::from_secs(30))
            .expect("the keeper went to sleep");
        assert_eq!(wait, secs(60));
        let stopping = std::time::Instant::now();
        drop(runtime);
        let took = stopping.elapsed();
        assert!(took < Duration::from_secs(5), "stopping took {took:?}");
        assert!(task.is_finished(), "the keeper is gone with the runtime");
        assert_eq!(
            requests(&served),
            [LIST_PATH],
            "nothing sent on the way out"
        );
        assert_eq!(leftovers(dir.path()), Vec::<String>::new());
        assert!(!versions_path(dir.path()).exists());
        assert!(lock_is_free(dir.path()));
    }

    #[test]
    fn stopping_the_daemon_during_a_download_leaves_no_partial_folder() {
        let (list, files, contents) = all_packs_for_script();
        let (gate, hold) = gate();
        let served = serve_script(move |path, _| {
            if path.ends_with("/heroes-v2.tar") {
                hold.hold();
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime = daemon_runtime();
        let startup = runtime.block_on(sync_on_startup_capped(
            &served.base,
            TOKEN,
            dir.path(),
            PACK_MAX_BYTES,
        ));
        let task = runtime.spawn(rig(&served.base, dir.path(), Some(startup), 8, no_nap).run());
        gate.arrived
            .blocking_recv()
            .expect("the keeper asked for heroes-v2");
        assert!(!lock_is_free(dir.path()), "held while the download runs");
        let stopping = std::time::Instant::now();
        drop(runtime);
        let took = stopping.elapsed();
        assert!(took < Duration::from_secs(5), "stopping took {took:?}");
        assert!(task.is_finished());
        drop(gate.release);
        // heroes-v1 from startup is whole; nothing of heroes-v2 was written.
        let heroes = &contents[HEROES_PACK_NAME][0];
        assert_eq!(
            &std::fs::read(heroes_spec().dir_in(dir.path()).join(&heroes.0)).unwrap(),
            &heroes.1
        );
        for spec in &PACKS[1..] {
            assert!(!spec.dir_in(dir.path()).exists(), "{}", spec.name);
        }
        assert_eq!(leftovers(dir.path()), Vec::<String>::new());
        assert_eq!(
            stored_versions(dir.path()).keys().collect::<Vec<_>>(),
            [HEROES_PACK_NAME]
        );
        assert!(lock_is_free(dir.path()), "freed when the try was dropped");
        // The next start carries on: one list and the five that are missing.
        let before = requests(&served).len();
        let restart = daemon_runtime();
        let startup = restart.block_on(sync_on_startup_capped(
            &served.base,
            TOKEN,
            dir.path(),
            PACK_MAX_BYTES,
        ));
        let (finished, waits) =
            restart.block_on(rig(&served.base, dir.path(), Some(startup), 8, no_nap).run());
        assert!(finished);
        assert_eq!(waits, Vec::<Duration>::new());
        assert_all_installed(dir.path(), &contents);
        assert_eq!(requests(&served)[before..], list_then(&pack_names()[1..]));
    }

    // ---- P1c fix round: the first-try signal ----

    #[tokio::test]
    async fn the_first_try_signal_says_what_is_missing_and_when_the_next_try_is() {
        // A first install without trouble: nothing missing, no next try.
        let (list, files, _) = all_packs();
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let mut job = rig(&served.base, dir.path(), Some(startup), 8, no_nap);
        let first = job.keeper.first_try_done();
        assert!(job.run().await.0);
        assert_eq!(
            first.await.expect("sent"),
            FirstTry {
                missing: Vec::new(),
                next_try_in: None
            }
        );

        // Packs switched off: everything is missing and the next try is a
        // minute away. The signal comes before that wait, not after it.
        let served =
            serve_script(|_, _| Some((503, br#"{"error":"packs_disabled"}"#.to_vec(), None)));
        let dir = tempfile::tempdir().expect("tempdir");
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let mut job = rig(&served.base, dir.path(), Some(startup), 0, no_nap);
        let first = job.keeper.first_try_done();
        assert!(!job.run().await.0, "still waiting");
        assert_eq!(
            first.await.expect("sent"),
            FirstTry {
                missing: pack_names(),
                next_try_in: Some(secs(60))
            }
        );

        // A 429 on a download: five are missing and the next try is soon.
        let (list, files, _) = all_packs_for_script();
        let served = serve_script(move |path, asked| {
            if path.ends_with("/heroes-v2.tar") && asked == 0 {
                let body = br#"{"error":"rate_limited","retry_after":7}"#;
                return Some((429, body.to_vec(), Some(7)));
            }
            healthy(&list, &files, path)
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let startup = sync_on_startup_capped(&served.base, TOKEN, dir.path(), PACK_MAX_BYTES).await;
        let mut job = rig(&served.base, dir.path(), Some(startup), 0, no_nap);
        let first = job.keeper.first_try_done();
        assert!(!job.run().await.0, "still waiting");
        assert_eq!(
            first.await.expect("sent"),
            FirstTry {
                missing: pack_names()[1..].to_vec(),
                next_try_in: Some(secs(7) + TEST_JITTER)
            }
        );

        // A job that is dropped before its first try closes the channel.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut job = rig(&served.base, dir.path(), None, 0, no_nap);
        let first = job.keeper.first_try_done();
        drop(job);
        assert!(first.await.is_err());
    }

    // ---- P1c fix round: against the site's real rate limit (F268-1) ----
    // The fake below keeps the bucket the way the site's limiter does, on a
    // clock that only the trackers' own waits move.

    /// The site's pack rate limit as governor 0.10 keeps it (GCRA): 8 in a
    /// burst, one more every 10 s, per address. A refused request costs
    /// nothing and is told the whole seconds to the next allowed time, plus
    /// one (`governor_error_response` in site-server).
    struct Bucket {
        /// The limiter's "theoretical arrival time". `None` until the
        /// first request.
        tat: Option<Duration>,
    }

    impl Bucket {
        const EVERY: Duration = Duration::from_secs(10);
        const BURST: u32 = 8;

        /// A bucket with `left` requests in it at time zero.
        fn with_left(left: u32) -> Self {
            Self {
                tat: Some(Self::EVERY * (Self::BURST - left)),
            }
        }

        /// `Ok` when the request is let through, `Err(retry_after)` in
        /// whole seconds when it is not.
        fn ask(&mut self, now: Duration) -> Result<(), u64> {
            let tau = Self::EVERY * (Self::BURST - 1);
            let tat = self.tat.unwrap_or(now);
            let earliest = tat.saturating_sub(tau);
            if now < earliest {
                return Err((earliest - now).as_secs() + 1);
            }
            self.tat = Some(tat.max(now) + Self::EVERY);
            Ok(())
        }
    }

    #[test]
    fn the_bucket_model_is_eight_then_one_every_ten_seconds() {
        let mut bucket = Bucket { tat: None };
        let at = Duration::from_secs_f64;
        for request in 0..8 {
            assert_eq!(bucket.ask(at(0.0)), Ok(()), "request {request}");
        }
        // Empty: the next one is allowed at 10 s. Asking does not cost.
        assert_eq!(bucket.ask(at(0.0)), Err(11));
        assert_eq!(bucket.ask(at(3.0)), Err(8));
        assert_eq!(bucket.ask(at(9.5)), Err(1));
        assert_eq!(bucket.ask(at(10.0)), Ok(()));
        assert_eq!(bucket.ask(at(10.0)), Err(11));
        assert_eq!(bucket.ask(at(12.0)), Err(9));
        assert_eq!(bucket.ask(at(20.0)), Ok(()));
        // After 80 quiet seconds the burst is whole again.
        for request in 0..8 {
            assert_eq!(bucket.ask(at(110.0)), Ok(()), "request {request}");
        }
        assert_eq!(bucket.ask(at(110.0)), Err(11));
        // The starting states the tests use.
        assert_eq!(Bucket::with_left(8).ask(at(0.0)), Ok(()));
        let mut four = Bucket::with_left(4);
        for request in 0..4 {
            assert_eq!(four.ask(at(0.0)), Ok(()), "request {request}");
        }
        assert_eq!(four.ask(at(0.0)), Err(11));
        assert_eq!(Bucket::with_left(0).ask(at(0.0)), Err(11));
        assert_eq!(Bucket::with_left(0).ask(at(10.0)), Ok(()));
    }

    /// Time for trackers that share one bucket. Each tracker is an actor.
    /// The clock jumps to the next wake-up once every actor is asleep or
    /// done, so no wait is slept for real and the order of events is the
    /// order of the waits.
    struct SimClock {
        state: Mutex<SimState>,
        moved: tokio::sync::Notify,
    }

    struct SimState {
        now: Duration,
        /// Actors that are neither asleep nor done.
        awake: usize,
        /// When each sleeping actor wakes.
        wakes: Vec<Duration>,
        /// The run is given up when the clock would pass this.
        limit: Duration,
        /// How many waits were asked for. A job that asks for more than
        /// [`SimClock::MAX_SLEEPS`] is spinning, and the run is given up.
        sleeps: usize,
        ran_out: bool,
    }

    impl SimState {
        /// Jump to the next wake-up if nobody is awake.
        fn settle(&mut self) {
            if self.awake > 0 || self.ran_out {
                return;
            }
            let Some(next) = self.wakes.iter().min().copied() else {
                return;
            };
            if next > self.limit {
                self.ran_out = true;
                return;
            }
            self.now = next;
            let asleep = self.wakes.len();
            self.wakes.retain(|wake| *wake > next);
            self.awake += asleep - self.wakes.len();
        }
    }

    impl SimClock {
        const MAX_SLEEPS: usize = 1000;

        fn new(actors: usize, limit: Duration) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(SimState {
                    now: Duration::ZERO,
                    awake: actors,
                    wakes: Vec::new(),
                    limit,
                    sleeps: 0,
                    ran_out: false,
                }),
                moved: tokio::sync::Notify::new(),
            })
        }

        fn now(&self) -> Duration {
            self.state.lock().expect("clock").now
        }

        /// The calling actor sleeps for `wait` of this clock's time.
        fn sleep(self: &Arc<Self>, wait: Duration) -> Nap {
            let wake = {
                let mut state = self.state.lock().expect("clock");
                let wake = state.now + wait;
                state.wakes.push(wake);
                state.awake -= 1;
                state.sleeps += 1;
                if state.sleeps > Self::MAX_SLEEPS {
                    state.ran_out = true;
                }
                state.settle();
                wake
            };
            self.moved.notify_waiters();
            let clock = Arc::clone(self);
            Box::pin(async move {
                loop {
                    let moved = clock.moved.notified();
                    tokio::pin!(moved);
                    let _ = moved.as_mut().enable();
                    {
                        let state = clock.state.lock().expect("clock");
                        if !state.ran_out && state.now >= wake && !state.wakes.contains(&wake) {
                            return;
                        }
                    }
                    moved.await;
                }
            })
        }

        /// The calling actor is done for good.
        fn actor_done(&self) {
            {
                let mut state = self.state.lock().expect("clock");
                state.awake -= 1;
                state.settle();
            }
            self.moved.notify_waiters();
        }

        /// Resolves when the clock gave up at its limit.
        async fn ran_out(&self) {
            loop {
                let moved = self.moved.notified();
                tokio::pin!(moved);
                let _ = moved.as_mut().enable();
                if self.state.lock().expect("clock").ran_out {
                    return;
                }
                moved.await;
            }
        }
    }

    /// A site with all six packs behind the real rate limit, and every
    /// request it saw: when, which path, which status.
    struct LimitedSite {
        base: String,
        seen: Arc<Mutex<Vec<(Duration, String, u16)>>>,
    }

    impl LimitedSite {
        fn statuses(&self) -> Vec<u16> {
            let seen = self.seen.lock().expect("seen");
            seen.iter().map(|(_, _, status)| *status).collect()
        }

        fn paths(&self) -> Vec<String> {
            let seen = self.seen.lock().expect("seen");
            seen.iter().map(|(_, path, _)| path.clone()).collect()
        }

        fn times(&self) -> Vec<Duration> {
            let seen = self.seen.lock().expect("seen");
            seen.iter().map(|(at, _, _)| *at).collect()
        }

        fn refused(&self) -> usize {
            self.statuses()
                .iter()
                .filter(|status| **status == 429)
                .count()
        }
    }

    fn serve_limited(clock: &Arc<SimClock>, bucket: Bucket) -> (LimitedSite, PackContents) {
        let (list, files, contents) = all_packs_for_script();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (clock, bucket, log) = (Arc::clone(clock), Mutex::new(bucket), seen.clone());
        let served = serve_script(move |path, _| {
            let now = clock.now();
            let answer = match bucket.lock().expect("bucket").ask(now) {
                Ok(()) => healthy(&list, &files, path),
                Err(wait) => {
                    let body = format!(r#"{{"error":"rate_limited","retry_after":{wait}}}"#);
                    Some((429, body.into_bytes(), Some(wait)))
                }
            };
            let status = answer.as_ref().map_or(0, |(status, _, _)| *status);
            log.lock()
                .expect("seen")
                .push((now, path.to_string(), status));
            answer
        });
        let site = LimitedSite {
            base: served.base,
            seen,
        };
        (site, contents)
    }

    /// One tracker on the simulated clock: it starts after `start_after`,
    /// runs its startup sync and then its job to the end. The second value
    /// collects the waits the job asked for.
    fn tracker(
        clock: &Arc<SimClock>,
        site: &LimitedSite,
        data_dir: &Path,
        start_after: Duration,
        jitter: Duration,
    ) -> (Nap, Arc<Mutex<Vec<Duration>>>) {
        let (clock, base, data_dir) =
            (Arc::clone(clock), site.base.clone(), data_dir.to_path_buf());
        let waits = Arc::new(Mutex::new(Vec::new()));
        let asked = waits.clone();
        let run = async move {
            if !start_after.is_zero() {
                clock.sleep(start_after).await;
            }
            let startup = sync_on_startup_capped(&base, TOKEN, &data_dir, PACK_MAX_BYTES).await;
            let server_url = base.clone();
            let mut keeper = PackKeeper::new(&data_dir, Some(startup), move || {
                Some(PackCredentials {
                    server_url: server_url.clone(),
                    token: TOKEN.to_string(),
                })
            });
            keeper.jitter = jitter;
            let napper = Arc::clone(&clock);
            keeper.sleep = Box::new(move |wait| {
                asked.lock().expect("waits").push(wait);
                napper.sleep(wait)
            });
            keeper.run().await;
            clock.actor_done();
        };
        (Box::pin(run), waits)
    }

    /// Run the trackers to the end. `false` when the clock reached its
    /// limit, or a tracker asked for too many waits, with one still not
    /// done.
    async fn run_trackers(clock: &Arc<SimClock>, trackers: Vec<Nap>) -> bool {
        let handles: Vec<_> = trackers.into_iter().map(tokio::spawn).collect();
        let all = async {
            for handle in handles {
                handle.await.expect("tracker");
            }
        };
        tokio::select! {
            () = all => true,
            () = clock.ran_out() => false,
        }
    }

    const SIM_LIMIT: Duration = Duration::from_secs(3600);

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[tokio::test]
    async fn against_the_real_limit_one_first_install_is_seven_requests_and_no_429() {
        let clock = SimClock::new(1, SIM_LIMIT);
        let (site, contents) = serve_limited(&clock, Bucket::with_left(8));
        let dir = tempfile::tempdir().expect("tempdir");
        let (tracker, waits) = tracker(&clock, &site, dir.path(), Duration::ZERO, TEST_JITTER);
        assert!(run_trackers(&clock, vec![tracker]).await);
        assert_eq!(*waits.lock().unwrap(), Vec::<Duration>::new());
        assert_all_installed(dir.path(), &contents);
        assert_eq!(site.paths(), list_then(&pack_names()));
        assert_eq!(site.statuses(), [200; 8]);
        assert_eq!(clock.now(), Duration::ZERO);
    }

    /// F268-1 as reviewed: the bucket runs out in the middle of a first
    /// install. The refused download is the next request, not the list, and
    /// the rest follow one every 10 s.
    #[tokio::test]
    async fn against_the_real_limit_a_429_in_the_middle_is_followed_by_the_downloads() {
        let clock = SimClock::new(1, SIM_LIMIT);
        let (site, contents) = serve_limited(&clock, Bucket::with_left(4));
        let dir = tempfile::tempdir().expect("tempdir");
        let (tracker, waits) = tracker(&clock, &site, dir.path(), Duration::ZERO, TEST_JITTER);
        assert!(
            run_trackers(&clock, vec![tracker]).await,
            "the job must end; it asked for {:?}",
            site.paths()
        );
        assert_all_installed(dir.path(), &contents);
        // The list and three packs fit. bans-v1 is refused with 11 s.
        let mut expected = list_then(&pack_names()[..3]);
        expected.extend(tar_paths(&["bans-v1.tar"]));
        expected.extend(tar_paths(&pack_names()[3..]));
        assert_eq!(site.paths(), expected);
        assert_eq!(
            site.statuses(),
            [200, 200, 200, 200, 429, 200, 200, 200, 200]
        );
        assert_eq!(
            *waits.lock().unwrap(),
            [secs(11) + TEST_JITTER, secs(10), secs(10), secs(10)]
        );
        assert_eq!(
            site.times()[5..],
            [ms(11_750), ms(21_750), ms(31_750), ms(41_750)]
        );
        assert_eq!(clock.now(), ms(41_750));
    }

    #[tokio::test]
    async fn against_the_real_limit_a_bucket_that_starts_empty_is_eight_requests() {
        let clock = SimClock::new(1, SIM_LIMIT);
        let (site, contents) = serve_limited(&clock, Bucket::with_left(0));
        let dir = tempfile::tempdir().expect("tempdir");
        let (tracker, waits) = tracker(&clock, &site, dir.path(), Duration::ZERO, TEST_JITTER);
        assert!(
            run_trackers(&clock, vec![tracker]).await,
            "the job must end; it asked for {:?}",
            site.paths()
        );
        assert_all_installed(dir.path(), &contents);
        // The refused list, the list, then the seven packs.
        let mut expected = vec![LIST_PATH.to_string()];
        expected.extend(list_then(&pack_names()));
        assert_eq!(site.paths(), expected);
        assert_eq!(site.refused(), 1);
        let mut expected_waits = vec![secs(11) + TEST_JITTER];
        expected_waits.extend([secs(10); 7]);
        assert_eq!(*waits.lock().unwrap(), expected_waits);
        // One request every 10 s, each 1.75 s after the bucket gave it.
        let times = site.times();
        assert_eq!(times[1], ms(11_750));
        assert!(
            times[1..]
                .windows(2)
                .all(|pair| pair[1] - pair[0] == secs(10))
        );
        assert_eq!(clock.now(), ms(81_750));
    }

    #[tokio::test]
    async fn against_the_real_limit_two_trackers_behind_one_address_both_finish() {
        let clock = SimClock::new(2, SIM_LIMIT);
        let (site, contents) = serve_limited(&clock, Bucket::with_left(8));
        let (dir_a, dir_b) = (
            tempfile::tempdir().expect("tempdir"),
            tempfile::tempdir().expect("tempdir"),
        );
        // B starts 3 s after A, as a second machine on release night would.
        let (a, waits_a) = tracker(&clock, &site, dir_a.path(), Duration::ZERO, TEST_JITTER);
        let (b, waits_b) = tracker(&clock, &site, dir_b.path(), secs(3), TEST_JITTER);
        assert!(
            run_trackers(&clock, vec![a, b]).await,
            "both jobs must end; they asked for {:?}",
            site.paths()
        );
        assert_all_installed(dir_a.path(), &contents);
        assert_all_installed(dir_b.path(), &contents);
        // A takes all 8. B's list is refused with 8 s, then the list and
        // its seven packs follow 10 s apart.
        assert_eq!(*waits_a.lock().unwrap(), Vec::<Duration>::new());
        let mut expected_waits = vec![secs(8) + TEST_JITTER];
        expected_waits.extend([secs(10); 7]);
        assert_eq!(*waits_b.lock().unwrap(), expected_waits);
        let mut expected = list_then(&pack_names());
        expected.push(LIST_PATH.to_string());
        expected.extend(list_then(&pack_names()));
        assert_eq!(site.paths(), expected);
        assert_eq!(site.statuses().len(), 17);
        assert_eq!(site.refused(), 1);
        assert_eq!(clock.now(), ms(81_750));
    }

    /// Both trackers want the same refilled request every 10 s. The one
    /// that comes second backs off further each time, so the first gets
    /// through, and then the second.
    #[tokio::test]
    async fn against_the_real_limit_two_trackers_on_an_empty_bucket_both_finish() {
        let clock = SimClock::new(2, SIM_LIMIT);
        let (site, contents) = serve_limited(&clock, Bucket::with_left(0));
        let (dir_a, dir_b) = (
            tempfile::tempdir().expect("tempdir"),
            tempfile::tempdir().expect("tempdir"),
        );
        let (a, waits_a) = tracker(&clock, &site, dir_a.path(), Duration::ZERO, ms(750));
        let (b, waits_b) = tracker(&clock, &site, dir_b.path(), Duration::ZERO, ms(2_250));
        assert!(
            run_trackers(&clock, vec![a, b]).await,
            "both jobs must end; they asked for {:?}",
            site.paths()
        );
        assert_all_installed(dir_a.path(), &contents);
        assert_all_installed(dir_b.path(), &contents);
        let (requests, refused, took) = (site.statuses().len(), site.refused(), clock.now());
        println!(
            "two trackers, empty bucket: {requests} requests, {refused} refused, done after {took:?}; \
             waits A {:?}, waits B {:?}",
            waits_a.lock().unwrap(),
            waits_b.lock().unwrap()
        );
        // 16 requests are needed (two lists, fourteen packs). A is refused
        // once and is done after 81.75 s. B comes second three more times,
        // waits 12.25, 22.25 and 42.25 s, and then has the bucket to
        // itself: 5 refused requests in all, both done after 160 s.
        assert_eq!(requests - refused, 16);
        assert_eq!(refused, 5);
        assert_eq!(took, secs(160));
        let mut expected_a = vec![ms(11_750)];
        expected_a.extend([secs(10); 7]);
        assert_eq!(*waits_a.lock().unwrap(), expected_a);
        let mut expected_b = vec![ms(13_250), ms(12_250), ms(22_250), ms(42_250)];
        expected_b.extend([secs(10); 7]);
        assert_eq!(*waits_b.lock().unwrap(), expected_b);
    }

    /// A names-v1 tar shaped like the published one: the synthetic letter
    /// pack from the name reader's tests, with its manifest at the top
    /// level. Returns the tar and the manifest bytes.
    fn names_tar(version: &str) -> (Vec<u8>, Vec<u8>) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("names-v1");
        crate::names::tests::write_pack(&dir, &["ABC", "CBA", "BAC"]);
        crate::names::tests::to_files_array(&dir, None, None);
        let manifest_path = dir.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["version"] = serde_json::Value::String(version.to_string());
        let manifest = serde_json::to_vec(&manifest).unwrap();
        let mut bytes = ustar_file("manifest.json", &manifest);
        for file in [
            "glyphs.png",
            "glyphs.txt",
            "marks.png",
            "marks.txt",
            "compose.txt",
            "whiten.f32",
        ] {
            bytes.extend(ustar_file(file, &std::fs::read(dir.join(file)).unwrap()));
        }
        (finish_tar(bytes), manifest)
    }

    /// Every pack from [`all_packs`], with names-v1 replaced by a real
    /// (synthetic) letter pack.
    fn all_packs_with_real_names(version: &str) -> (Vec<serde_json::Value>, ServedFiles, Vec<u8>) {
        let (mut list, mut files, _) = all_packs();
        let (tar, manifest) = names_tar(version);
        let name = crate::names::pack::PACK_NAME;
        for entry in &mut list {
            if entry["name"] == name {
                *entry = entry_json(name, version, &tar);
            }
        }
        files.insert(name.to_string(), (200, tar));
        (list, files, manifest)
    }

    /// The rc.1 bug on Soot: names-v1 was installed without its
    /// `manifest.json`, so the name reader refused it. The manifest is kept
    /// now, byte for byte, and the reader loads the installed folder.
    #[tokio::test]
    async fn names_v1_keeps_its_manifest_and_the_name_reader_loads_it() {
        let (list, files, manifest) = all_packs_with_real_names("full-20261019");
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let report = sync(&served, dir.path()).await;
        assert_eq!(
            outcome(&report, crate::names::pack::PACK_NAME),
            &PackSync::Installed
        );
        let live = crate::names::pack::dir_in(dir.path());
        assert_eq!(std::fs::read(live.join("manifest.json")).unwrap(), manifest);
        let heroes = pack_spec(HEROES_PACK_NAME).unwrap().dir_in(dir.path());
        assert!(
            !heroes.join("manifest.json").exists(),
            "heroes-v1 still has no manifest in its folder"
        );
        let switch = crate::names::NameSwitch::load(true, dir.path());
        switch.wait_reload();
        let pack = switch.pack().expect("the installed names-v1 loads");
        assert_eq!(pack.version, "full-20261019");
        assert_eq!(pack.templates(), 9);
    }

    /// A folder rc.1 left behind (the six files, no manifest, the version
    /// stored) is not current: the next sync downloads the pack again.
    #[tokio::test]
    async fn a_names_folder_without_its_manifest_is_downloaded_again() {
        let (list, files, manifest) = all_packs_with_real_names("full-20261019");
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            outcome(
                &sync(&served, dir.path()).await,
                crate::names::pack::PACK_NAME
            ),
            &PackSync::Installed
        );
        let live = crate::names::pack::dir_in(dir.path());
        std::fs::remove_file(live.join("manifest.json")).unwrap();
        let spec = pack_spec(crate::names::pack::PACK_NAME).unwrap();
        assert!(!in_place(dir.path(), spec));
        assert_eq!(
            missing_packs(dir.path(), &PackReport { packs: Vec::new() }),
            vec![crate::names::pack::PACK_NAME]
        );
        let report = sync(&served, dir.path()).await;
        assert_eq!(
            outcome(&report, crate::names::pack::PACK_NAME),
            &PackSync::Installed
        );
        assert_eq!(outcome(&report, HEROES_PACK_NAME), &PackSync::Current);
        assert_eq!(std::fs::read(live.join("manifest.json")).unwrap(), manifest);
    }

    /// The keeper tells the daemon about every install, with the version
    /// and the sha256 of the manifest the files were checked against, and
    /// the name reader that hook reloads ends up on names-v1.
    #[tokio::test]
    async fn the_keeper_reports_installs_and_the_name_reader_reloads() {
        let (list, files, manifest) = all_packs_with_real_names("full-20261019");
        let served = serve(list, files);
        let dir = tempfile::tempdir().expect("tempdir");
        let switch = crate::names::NameSwitch::load(true, dir.path());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = served.base.clone();
        let mut keeper = PackKeeper::new(dir.path(), None, move || {
            Some(PackCredentials {
                server_url: base.clone(),
                token: TOKEN.to_string(),
            })
        });
        let (log, names) = (seen.clone(), switch.clone());
        keeper.on_installed(move |installed| {
            log.lock().unwrap().push(installed.clone());
            if installed.name == crate::names::pack::PACK_NAME {
                names.reload_installed(crate::names::InstalledNames {
                    version: installed.version.clone(),
                    manifest_sha256: installed.manifest_sha256.clone(),
                });
            }
        });
        keeper.run().await;
        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            seen.iter().map(|p| p.name).collect::<Vec<_>>(),
            PACKS.iter().map(|spec| spec.name).collect::<Vec<_>>(),
            "one report per install, in PACKS order"
        );
        let names_v1 = seen
            .iter()
            .find(|p| p.name == crate::names::pack::PACK_NAME)
            .unwrap();
        assert_eq!(names_v1.version, "full-20261019");
        assert_eq!(names_v1.manifest_sha256, sha256_hex(&manifest));
        switch.wait_reload();
        let pack = switch.pack().expect("reloaded without a restart");
        assert_eq!(pack.version, "full-20261019");
    }
}
