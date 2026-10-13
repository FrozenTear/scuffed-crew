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
//! pack ([`crate::shadow::result::ResultDirs`]). The other packs are
//! downloaded into folders no reader opens yet.
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
//! back [`ExtraPacks`] to run in the background, so capture never waits on
//! the extras. The new reader starts on heroes-v1 and re-loads its hero set
//! once heroes-v2 or heroes-v2-1080 is swapped in
//! ([`crate::shadow::heroes::LiveHeroSet`]), and loads maps-v1 the same way
//! once it lands ([`crate::shadow::banner::LiveMaps`]). The setup guide
//! ([`sync_reader_packs`]) syncs all of them. Every run holds an exclusive `flock` on
//! `templates/.pack-sync.lock`; a second run at the same time is
//! [`PackSync::Busy`] and sends no request.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

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
/// reader), and so is maps-v1 (the new reader's map banner); the others are
/// not yet.
pub const PACKS: &[PackSpec] = &[
    PackSpec {
        name: HEROES_PACK_NAME,
        dir: HeroTemplates::dir_in,
        layout: PackLayout::HeroesV1,
        required: true,
    },
    PackSpec {
        name: "heroes-v2.tar",
        dir: |data_dir| data_dir.join("templates").join("heroes-v2"),
        layout: PackLayout::Portraits,
        required: false,
    },
    PackSpec {
        name: "heroes-v2-1080.tar",
        dir: |data_dir| data_dir.join("templates").join("heroes-v2-1080"),
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

fn log_report(report: &PackReport) {
    for status in &report.packs {
        let pack = status.name;
        match &status.outcome {
            PackSync::Installed => tracing::info!(pack, "reader pack installed"),
            PackSync::Current | PackSync::Unavailable => {}
            PackSync::Missing => tracing::debug!(pack, "reader pack not listed by the server"),
            PackSync::Busy => tracing::info!(pack, "another reader pack sync is running"),
            PackSync::Unsupported => {
                tracing::info!(pack, "reader packs are not supported on this server yet");
            }
            PackSync::Failed(message) => {
                tracing::warn!(pack, message, "reader pack left in place");
            }
        }
    }
}

/// Daemon startup. Syncs `heroes-v1.tar` only, the same two requests as
/// alpha.6, so capture waits on nothing new. The other packs come back as
/// [`ExtraPacks`] for the caller to run in the background once capture is
/// up. `None` when the list was not read (signed out, list error, busy),
/// or when the heroes-v1 download hit a stop (refused token, rate limit,
/// packs switched off): the extras would only repeat that request inside
/// the same window. Logs a plain warning on failure and never the token.
pub async fn sync_on_startup(server_url: &str, token: &str, data_dir: &Path) -> Option<ExtraPacks> {
    sync_on_startup_capped(server_url, token, data_dir, PACK_MAX_BYTES).await
}

async fn sync_on_startup_capped(
    server_url: &str,
    token: &str,
    data_dir: &Path,
    max_bytes: u64,
) -> Option<ExtraPacks> {
    let (report, entries) = run_sync(
        server_url,
        token,
        data_dir,
        max_bytes,
        Scope::HeroesOnly,
        None,
    )
    .await;
    log_report(&report);
    entries.map(|entries| ExtraPacks {
        server_url: server_url.to_string(),
        token: token.to_string(),
        data_dir: data_dir.to_path_buf(),
        max_bytes,
        entries,
    })
}

/// The packs other than `heroes-v1.tar`, still to sync after daemon
/// startup. They never hold up capture: heroes-v1 is the new reader's
/// fallback until the v2 hero packs land, and the other packs have no reader
/// yet. Reuses
/// the startup list, so a first install is still seven requests in all.
/// Holds the token; it has no `Debug` so the token cannot reach a log.
pub struct ExtraPacks {
    server_url: String,
    token: String,
    data_dir: PathBuf,
    max_bytes: u64,
    entries: Vec<PackListEntry>,
}

impl ExtraPacks {
    /// Sync the extra packs and log each outcome. Meant for `tokio::spawn`.
    pub async fn run(self) {
        let report = self.sync().await;
        log_report(&report);
    }

    async fn sync(self) -> PackReport {
        run_sync(
            &self.server_url,
            &self.token,
            &self.data_dir,
            self.max_bytes,
            Scope::ExtrasOnly,
            Some(self.entries),
        )
        .await
        .0
    }
}

/// Every pack, as the setup guide's "Download reader pack" runs it.
pub async fn sync_reader_packs(
    server_url: &str,
    token: &str,
    data_dir: &Path,
    max_bytes: u64,
) -> PackReport {
    run_sync(server_url, token, data_dir, max_bytes, Scope::All, None)
        .await
        .0
}

/// One run over `scope`. `listed` reuses a list already fetched. Returns the
/// report and the list it used, when it got one and no download hit a stop.
async fn run_sync(
    server_url: &str,
    token: &str,
    data_dir: &Path,
    max_bytes: u64,
    scope: Scope,
    listed: Option<Vec<PackListEntry>>,
) -> (PackReport, Option<Vec<PackListEntry>>) {
    if token.trim().is_empty() || server_url.trim().is_empty() {
        let outcome = PackSync::Failed(PACK_SIGN_IN.to_string());
        return (PackReport::all(scope, outcome), None);
    }
    if validate_sync_server_url(server_url).is_err() {
        let outcome = PackSync::Failed(PACK_FAILED.to_string());
        return (PackReport::all(scope, outcome), None);
    }
    // One sync at a time across processes, taken before any request. The
    // daemon and the app share pack-versions.json and the request budget.
    let _lock = match SyncLock::try_acquire(data_dir) {
        Ok(Some(lock)) => lock,
        Ok(None) => return (PackReport::all(scope, PackSync::Busy), None),
        Err(err) => {
            tracing::warn!(error = %err, "reader pack sync lock was not taken");
            let outcome = PackSync::Failed(PACK_FAILED.to_string());
            return (PackReport::all(scope, outcome), None);
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
            return (PackReport::all(scope, outcome), None);
        }
    };
    let base = server_url.trim().trim_end_matches('/');
    let entries = match listed {
        Some(entries) => entries,
        None => match fetch_list(&client, base, token).await {
            Ok(entries) => entries,
            Err(outcome) => return (PackReport::all(scope, outcome), None),
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
    for spec in scope.specs() {
        let outcome = match select_pack(&entries, spec) {
            Err(outcome) => outcome,
            // Checked before the stop, so a pack that is already in place
            // is reported as current, not as the stop's failure.
            Ok(entry) if is_current(data_dir, spec, entry, &versions) => PackSync::Current,
            Ok(_) if stopped.is_some() => stopped.clone().unwrap_or(PackSync::Unavailable),
            Ok(entry) => {
                let outcome = match sync_one(&fetch, data_dir, spec, entry).await {
                    Step::Done(outcome) => outcome,
                    Step::Stop(outcome) => {
                        stopped = Some(outcome.clone());
                        outcome
                    }
                };
                if outcome == PackSync::Installed {
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
                outcome
            }
        };
        packs.push(PackStatus {
            name: spec.name,
            outcome,
        });
    }
    // After a stop the list is not handed on, so no later run reuses it to
    // send the request the server just refused or rate limited.
    (PackReport { packs }, stopped.is_none().then_some(entries))
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
        && spec.dir_in(data_dir).is_dir()
}

/// File locked for the length of one sync, so the daemon and the app never
/// sync at the same time.
fn sync_lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join(".pack-sync.lock")
}

/// Exclusive `flock` on [`sync_lock_path`]. Released when dropped, or when
/// the process dies.
struct SyncLock {
    _file: std::fs::File,
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
    Done(PackSync),
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
        Get::Unsupported => return Step::Done(PackSync::Failed(PACK_FAILED.to_string())),
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
        .unwrap_or_else(|| PackSync::Failed(PACK_FAILED.to_string()));
    Step::Done(installed)
}

/// Run blocking file work (hashing, unpacking, fsync) on tokio's blocking
/// pool, so the background extras job never holds up a runtime worker.
/// `None` if the work panicked.
async fn off_runtime<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work).await.ok()
}

/// The file side of one download: check the bytes against the list, unpack
/// into a staging folder and swap it in. Blocking; see [`off_runtime`].
fn install_pack(data_dir: &Path, spec: &PackSpec, entry: &PackListEntry, bytes: &[u8]) -> PackSync {
    #[cfg(test)]
    tests::note_blocking_work(data_dir);
    let live = spec.dir_in(data_dir);
    let parent = pack_parent(data_dir, spec);
    // List size and sha256 are checked before any file is written.
    if bytes.len() as u64 != entry.size || sha256_hex(bytes) != entry.sha256 {
        return PackSync::Failed(PACK_MISMATCH.to_string());
    }
    let staging = parent.join(format!(".{}-next-{}", dir_label(&live), unique_suffix()));
    if std::fs::create_dir(&staging).is_err() {
        return PackSync::Failed(PACK_FAILED.to_string());
    }
    if let Err(message) = unpack_pack(bytes, spec.layout, &entry.name, &entry.version, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return PackSync::Failed(message.to_string());
    }
    if let Err(message) = swap_dir(&staging, &live) {
        let _ = std::fs::remove_dir_all(&staging);
        tracing::warn!(pack = spec.name, error = %message, "reader pack was not swapped into place");
        return PackSync::Failed(PACK_FAILED.to_string());
    }
    PackSync::Installed
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

#[derive(Debug, Clone, serde::Deserialize)]
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
) -> Result<(), &'static str> {
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
    Ok(())
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
    /// The path a file is stored under, or [`PACK_UNSAFE`] when the layout
    /// does not allow it. `manifest.json` is accepted at the top level for
    /// every layout; [`unpack_pack`] reads it and never writes it.
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
            Self::BanIcons | Self::MapGlyphs | Self::Words => false,
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
        .map(|()| dir)
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

    fn spawn_fake(fake: Arc<Fake>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            for _ in 0..64 {
                let Ok((mut sock, _)) = listener.accept() else {
                    break;
                };
                sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .expect("timeout");
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                loop {
                    match std::io::Read::read(&mut sock, &mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let head = String::from_utf8_lossy(&buf);
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
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 200,
            list_body: listed,
            list_retry: None,
            file_status: 200,
            file_body: bytes,
            file_chunked: false,
            named_files: BTreeMap::new(),
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let log = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || SharedWriter(writer.clone()))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        sync_on_startup(base.as_str(), token, dir.path()).await;
        drop(_guard);
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");
        assert!(!heroes.join("ana.png").exists());
        let logs = String::from_utf8(log.lock().unwrap().clone()).unwrap();
        assert!(logs.contains(PACK_MISMATCH), "{logs}");
        assert!(!logs.contains(token), "{logs}");
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
            other => panic!("no sample for {other}"),
        };
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

    /// The server side of all six packs: list entries plus files.
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
        ];
        assert_eq!(PACKS.len(), expected.len() + 1);
        for (name, dir) in expected {
            let spec = pack_spec(name).expect(name);
            assert_eq!(spec.dir_in(data), data.join(dir), "{name}");
            assert!(!spec.required, "{name} is optional");
        }
        // Reader neutrality: only the hero packs, maps-v1 and words-v1 land
        // in a folder a reader opens. The hand-installed maps and result
        // folders are not pack folders.
        let reader_dirs = [
            crate::shadow::heroes::TEMPLATE_SUBDIR,
            crate::shadow::heroes::TEMPLATE_SUBDIR_V2,
            crate::shadow::heroes::TEMPLATE_SUBDIR_V2_1080,
            crate::shadow::result::TEMPLATE_SUBDIR,
            crate::shadow::result::WORDS_SUBDIR,
            crate::shadow::banner::TEMPLATE_SUBDIR,
            crate::shadow::banner::TEMPLATE_SUBDIR_V1,
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
            assert!(!live.join("manifest.json").exists());
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
        assert_eq!(report.extras_ready(), (5, 5));
        assert_eq!(
            report.guide_result(),
            Ok("Reader pack saved. Extra reader packs ready: 5 of 5.".to_string())
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
            Ok("The reader pack is already installed. Extra reader packs ready: 5 of 5.".into())
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
        assert_eq!(report.extras_ready(), (4, 5));
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
        let extras =
            sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES)
                .await
                .expect("the list was read, so the extras are queued");
        // Capture starts after this: only the alpha.6 requests happened.
        assert_eq!(served.downloaded(), vec![HEROES_PACK_NAME.to_string()]);
        assert_eq!(list_requests(&served), 1);
        for spec in &PACKS[1..] {
            assert!(!spec.dir_in(dir.path()).exists(), "{}", spec.name);
        }
        let report = extras.sync().await;
        assert_eq!(report.packs.len(), PACKS.len() - 1);
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
        let log = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || SharedWriter(writer.clone()))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let extras = sync_on_startup(base.as_str(), TOKEN, dir.path()).await;
        drop(_guard);
        assert!(extras.is_none());
        let logs = String::from_utf8(log.lock().unwrap().clone()).unwrap();
        assert_eq!(
            logs.matches("reader pack left in place").count(),
            1,
            "{logs}"
        );
        assert!(!logs.contains(TOKEN), "{logs}");
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
        assert!(
            sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES)
                .await
                .is_none()
        );
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
        assert_eq!(report.extras_ready(), (4, 5));
        assert_eq!(
            report.guide_result(),
            Ok(format!(
                "The reader pack is already installed. Extra reader packs ready: 4 of 5. {limited}."
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
        ] {
            assert_eq!(outcome(&report, name), &limited, "{name}");
        }
        assert_eq!(outcome(&report, "words-v1.tar"), &PackSync::Missing);
        assert_eq!(report.extras_ready(), (0, 4));
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
        let stops: [(u16, &[u8]); 4] = [
            (401, br#"{"error":"Unauthorized"}"#),
            (403, br#"{"error":"forbidden"}"#),
            (429, br#"{"error":"rate_limited","retry_after":30}"#),
            (503, br#"{"error":"packs_disabled"}"#),
        ];
        for (status, body) in stops {
            let served = all_packs_with_download(HEROES_PACK_NAME, status, body);
            let dir = tempfile::tempdir().expect("tempdir");
            let extras =
                sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES)
                    .await;
            assert!(extras.is_none(), "HTTP {status} queues no extras job");
            assert_eq!(
                served.downloaded(),
                vec![HEROES_PACK_NAME.to_string()],
                "HTTP {status}"
            );
        }
        // A failure that is not a stop (a bad hash) still queues them.
        let served = all_packs_with_download(HEROES_PACK_NAME, 200, b"not the listed pack");
        let dir = tempfile::tempdir().expect("tempdir");
        let extras =
            sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES)
                .await
                .expect("a mismatch is not a stop");
        let report = extras.sync().await;
        assert!(
            report
                .packs
                .iter()
                .all(|s| s.outcome == PackSync::Installed),
            "{report:?}"
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
                sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .expect("timeout");
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                while !buf.windows(4).any(|window| window == b"\r\n\r\n") {
                    match std::io::Read::read(&mut sock, &mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).into_owned();
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
        let extras =
            sync_on_startup_capped(served.base.as_str(), TOKEN, dir.path(), PACK_MAX_BYTES)
                .await
                .expect("extras queued");
        let report = extras.sync().await;
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
}
