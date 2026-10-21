//! Update banner: version check, in-app install, and Copy for the bootstrap command.
//!
//! Check still uses GitHub Releases (`stat-tracker-v*`). "Update now" downloads
//! `bootstrap.sh` from that release tag (not `main`) and runs it with
//! `STAT_TRACKER_TAG` pinned to the same tag. `main`'s script stays the
//! fresh-install entrypoint so older GUIs keep working; it re-execs the tag.
//! If the prefix is not writable or tools are missing, the banner says so
//! instead of offering a dead button.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::daemon::DaemonVerb;

use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill, Padding};

use crate::app::Message;
use crate::restart_prompt::RestartNeed;
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, PAD_INNER, SIZE_BODY, SIZE_META, SIZE_TITLE, TEXT,
    TEXT_2, TEXT_3,
};

const REPO: &str = "FrozenTear/scuffed-crew";
const BOOTSTRAP_REPO_PATH: &str = "crates/stat-tracker/dist/bootstrap.sh";

/// Fresh-install one-liner. `bootstrap.sh` on `main` resolves the release and
/// re-execs that tag's copy. Older installed GUIs hardcode this URL.
pub const UPDATE_CMD: &str = "curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash";

/// Stable raw URL of `bootstrap.sh` on `main`. Not used for a pinned update.
pub const BOOTSTRAP_URL: &str = "https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh";

/// Raw GitHub URL of `bootstrap.sh` at `git_ref` (a branch or a release tag).
pub fn bootstrap_raw_url(git_ref: &str) -> String {
    format!("https://raw.githubusercontent.com/{REPO}/{git_ref}/{BOOTSTRAP_REPO_PATH}")
}

/// Tools `bootstrap.sh` needs. python3 reads the GitHub release JSON and
/// does the safe tar extract; sha256sum checks the download (a missing one
/// refuses the install).
const REQUIRED_TOOLS: &[&str] = &["curl", "tar", "bash", "mktemp", "python3", "sha256sum"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub latest: String,
    pub current: String,
    pub url: String,
    /// GitHub release bodies for versions newer than `current`, newest first.
    /// Empty bodies mean the bundled changelog is the fallback.
    pub release_bodies: Vec<ReleaseBody>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseBody {
    pub version: String,
    pub body: String,
    /// GitHub `published_at`, when the API included one.
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateProgress {
    #[default]
    Idle,
    Running,
    Succeeded(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePlan {
    Ready { prefix: PathBuf, tag: String },
    Blocked { reason: String, hint: String },
}

impl UpdatePlan {
    pub fn can_run(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// Version used for the update banner.
///
/// Never uses this crate's `0.1.0`. Order:
/// 1. runtime `SST_RELEASE_VERSION` (installer / release packaging)
/// 2. compile-time `SST_RELEASE_VERSION` (release CI, same as the old GUI)
/// 3. installed daemon: `scuffed-stat-tracker --version`
///
/// If none of those resolve, the check is skipped (no false "update" against 0.1.0).
pub fn current_version() -> Option<String> {
    resolve_current_version(
        std::env::var("SST_RELEASE_VERSION").ok().as_deref(),
        option_env!("SST_RELEASE_VERSION"),
        installed_daemon_version().as_deref(),
    )
}

/// Pure resolver so tests can pin the three sources.
pub fn resolve_current_version(
    runtime_env: Option<&str>,
    compile_env: Option<&str>,
    daemon_version: Option<&str>,
) -> Option<String> {
    for raw in [runtime_env, compile_env, daemon_version]
        .into_iter()
        .flatten()
    {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if parse_semver(trimmed).is_some() {
            return Some(trimmed.trim_start_matches('v').to_string());
        }
    }
    None
}

/// Version line for `stat-tracker-gui --version`.
///
/// This is the daemon package version, not this crate's `0.1.0`. Release
/// builds that set `SST_RELEASE_VERSION` print that tag, same as the daemon.
pub fn gui_version_line() -> String {
    format!("stat-tracker-gui {}", daemon_package_version())
}

pub fn daemon_package_version() -> String {
    if let Some(compiled) = option_env!("SST_RELEASE_VERSION") {
        let trimmed = compiled.trim().trim_start_matches('v').trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    crate::notes::stat_tracker_package_version(include_str!("../../stat-tracker/Cargo.toml"))
        .unwrap_or("0.0.0")
        .to_string()
}

/// `scuffed-stat-tracker --version` prints `scuffed-stat-tracker 0.3.3`.
pub fn parse_daemon_version_line(line: &str) -> Option<String> {
    let line = line.trim();
    let rest = line
        .strip_prefix("scuffed-stat-tracker")
        .or_else(|| line.strip_prefix("stat-tracker-gui"))
        .unwrap_or(line)
        .trim();
    let ver = rest.trim_start_matches('v').trim();
    parse_semver(ver)?;
    Some(ver.to_string())
}

fn installed_daemon_version() -> Option<String> {
    let exe = crate::daemon::find_daemon_binary()?;
    let out = std::process::Command::new(exe)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_daemon_version_line(&String::from_utf8_lossy(&out.stdout))
}

/// `major.minor.patch` plus an optional prerelease. Build metadata (`+...`)
/// is ignored. A final release sorts after every prerelease of the same
/// triple, so `0.4.24 < 0.5.0-alpha.1 < 0.5.0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemVer {
    major: u32,
    minor: u32,
    patch: u32,
    pre: Vec<PreId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PreId {
    Num(u64),
    Text(String),
}

impl SemVer {
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// A prerelease the prerelease channel may offer: its first identifier
    /// is `alpha` or `rc` (also `rc1`, `alpha2`). Anything else, such as a
    /// `-test.1` build for a test session, is never offered. A version with
    /// no prerelease part is a release and passes.
    pub fn is_offered_prerelease(&self) -> bool {
        match self.pre.first() {
            None => true,
            Some(PreId::Num(_)) => false,
            Some(PreId::Text(text)) => {
                let text = text.to_ascii_lowercase();
                ["alpha", "rc"].iter().any(|kind| {
                    text.strip_prefix(kind)
                        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
                })
            }
        }
    }
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if self.pre.is_empty() {
            return Ok(());
        }
        write!(f, "-")?;
        for (index, id) in self.pre.iter().enumerate() {
            if index > 0 {
                write!(f, ".")?;
            }
            match id {
                PreId::Num(n) => write!(f, "{n}")?,
                PreId::Text(text) => write!(f, "{text}")?,
            }
        }
        Ok(())
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => cmp_pre(&self.pre, &other.pre),
            })
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PreId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Num(left), Self::Num(right)) => left.cmp(right),
            (Self::Text(left), Self::Text(right)) => left.cmp(right),
            (Self::Num(_), Self::Text(_)) => std::cmp::Ordering::Less,
            (Self::Text(_), Self::Num(_)) => std::cmp::Ordering::Greater,
        }
    }
}

impl PartialOrd for PreId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn cmp_pre(left: &[PreId], right: &[PreId]) -> std::cmp::Ordering {
    for (a, b) in left.iter().zip(right.iter()) {
        let ord = a.cmp(b);
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    left.len().cmp(&right.len())
}

fn pre_id(piece: &str) -> Option<PreId> {
    if piece.is_empty() || piece.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    if piece.bytes().all(|b| b.is_ascii_digit()) {
        return Some(PreId::Num(piece.parse().ok()?));
    }
    Some(PreId::Text(piece.to_string()))
}

/// Parse `MAJOR.MINOR.PATCH` with an optional `-prerelease` and `+build`.
/// A leading `v` is ignored. `None` on anything unparseable.
pub fn parse_semver(s: &str) -> Option<SemVer> {
    let raw = s.trim().trim_start_matches('v');
    if raw.is_empty() || raw.split_whitespace().nth(1).is_some() {
        return None;
    }
    let raw = raw.split('+').next().unwrap_or(raw);
    let (core, pre) = match raw.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (raw, None),
    };
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    let mut ids = Vec::new();
    if let Some(pre) = pre {
        if pre.is_empty() {
            return None;
        }
        for piece in pre.split('.') {
            ids.push(pre_id(piece)?);
        }
    }
    Some(SemVer {
        major,
        minor,
        patch,
        pre: ids,
    })
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Which GitHub releases the updater may offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateChannel {
    /// Release builds only. An alpha or rc is not offered.
    Stable,
    /// Release builds and prereleases. The newest semver still wins, so
    /// `0.5.0` is offered ahead of `0.5.0-alpha.1`.
    Prerelease,
}

pub fn release_tag(latest: &str) -> String {
    let core = latest.trim().trim_start_matches('v');
    format!("stat-tracker-v{core}")
}

/// Command shown in the banner / copied to the clipboard.
///
/// The tag is assigned on `bash`, not `curl`: `VAR=x curl … | bash` does not
/// export `VAR` into the shell that runs the script. The script URL is the
/// release tag, not `main`.
pub fn pinned_install_command(latest: &str) -> String {
    let tag = release_tag(latest);
    format!(
        "curl --proto '=https' -fsSL {} | STAT_TRACKER_TAG={tag} bash",
        bootstrap_raw_url(&tag)
    )
}

/// Settings / Copy. Prefer the advertised release, then the installed version,
/// and only then the `main` fresh-install entrypoint.
pub fn install_command_for(advertised: Option<&str>, current: Option<&str>) -> String {
    for candidate in [advertised, current].into_iter().flatten() {
        let trimmed = candidate.trim();
        if !trimmed.is_empty() && parse_semver(trimmed).is_some() {
            return pinned_install_command(trimmed);
        }
    }
    UPDATE_CMD.to_string()
}

/// Test hook `bootstrap.sh` honors. The GUI must not forward it into the
/// installer, or a developer's shell could redirect the release download.
pub const BOOTSTRAP_FETCH_HOOK_ENV: &str = "STAT_TRACKER_BOOTSTRAP_FETCH_CMD";

trait InstallerEnv {
    fn remove_env(&mut self, key: &str);
    fn set_env(&mut self, key: &str, value: &str);
}

impl InstallerEnv for std::process::Command {
    fn remove_env(&mut self, key: &str) {
        self.env_remove(key);
    }
    fn set_env(&mut self, key: &str, value: &str) {
        self.env(key, value);
    }
}

impl InstallerEnv for tokio::process::Command {
    fn remove_env(&mut self, key: &str) {
        self.env_remove(key);
    }
    fn set_env(&mut self, key: &str, value: &str) {
        self.env(key, value);
    }
}

fn apply_installer_env(cmd: &mut impl InstallerEnv, latest: &str, prefix: &Path) {
    cmd.remove_env(BOOTSTRAP_FETCH_HOOK_ENV);
    for (key, value) in bootstrap_env(latest, prefix) {
        cmd.set_env(key, &value);
    }
}

pub fn bootstrap_env(latest: &str, prefix: &Path) -> Vec<(&'static str, String)> {
    vec![
        ("STAT_TRACKER_TAG", release_tag(latest)),
        ("STAT_TRACKER_CHANNEL", "stable".into()),
        ("STAT_TRACKER_PREFIX", prefix.display().to_string()),
        // The GUI already downloaded this tag's script. Skip the re-exec.
        ("STAT_TRACKER_BOOTSTRAP_PINNED", "1".into()),
    ]
}

/// Stop before install. Start again when the install succeeded, or when it
/// failed and the previous binaries were put back, so a failed update never
/// leaves the tracker stopped. A failed install with no working rollback is
/// not started.
pub fn should_restart_daemon(stop_daemon: bool, install_ok: bool, rolled_back: bool) -> bool {
    stop_daemon && (install_ok || rolled_back)
}

/// Binaries the installer replaces under `PREFIX/bin`, saved before an
/// in-app update so a failed one can put them back.
const ROLLBACK_BINARIES: &[&str] = &["scuffed-stat-tracker", "stat-tracker-gui"];

/// Copy the installed binaries into `backup`. Returns the names saved. The
/// daemon binary has to be saved for a rollback to restart anything.
pub fn backup_binaries(prefix: &Path, backup: &Path) -> std::io::Result<Vec<&'static str>> {
    std::fs::create_dir_all(backup)?;
    let mut saved = Vec::new();
    for name in ROLLBACK_BINARIES {
        let live = prefix.join("bin").join(name);
        if live.is_file() {
            std::fs::copy(&live, backup.join(name))?;
            saved.push(*name);
        }
    }
    Ok(saved)
}

/// Put the saved binaries back, each through a temp file and a rename in
/// `PREFIX/bin`, so a half-written binary is never at the live path.
pub fn restore_binaries(prefix: &Path, backup: &Path, saved: &[&str]) -> std::io::Result<()> {
    let bin = prefix.join("bin");
    for name in saved {
        let tmp = bin.join(format!(".{name}.rollback"));
        std::fs::copy(backup.join(name), &tmp)?;
        if let Err(e) = std::fs::rename(&tmp, bin.join(name)) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    }
    Ok(())
}

pub fn default_install_prefix() -> PathBuf {
    if let Ok(p) = std::env::var("STAT_TRACKER_PREFIX") {
        let t = p.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
        && bin.file_name().is_some_and(|n| n == "bin")
        && let Some(prefix) = bin.parent()
    {
        return prefix.to_path_buf();
    }
    dirs::home_dir()
        .map(|h| h.join(".local"))
        .unwrap_or_else(|| PathBuf::from("/usr/local"))
}

/// Prefix "Restart now" starts the new binary from: the one the in-app
/// update installed to, else the default install prefix. The restart
/// prompt watches the binary under the same prefix.
pub fn restart_prefix(plan: &UpdatePlan) -> PathBuf {
    match plan {
        UpdatePlan::Ready { prefix, .. } => prefix.clone(),
        UpdatePlan::Blocked { .. } => default_install_prefix(),
    }
}

pub fn prefix_from_exe(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    Some(bin.parent()?.to_path_buf())
}

pub fn missing_tools(has: impl Fn(&str) -> bool) -> Vec<&'static str> {
    REQUIRED_TOOLS
        .iter()
        .copied()
        .filter(|name| !has(name))
        .collect()
}

pub fn explain_unwritable(prefix: &Path, err: &std::io::Error) -> String {
    format!(
        "Cannot write to {} ({err}). The install prefix is not writable from this app \
         (permissions or a sandbox). Copy the command and run it in a terminal, or set \
         STAT_TRACKER_PREFIX to a writable directory (default is ~/.local).",
        prefix.display()
    )
}

fn prefix_write_error(prefix: &Path) -> Option<String> {
    let bin = prefix.join("bin");
    if let Err(e) = std::fs::create_dir_all(&bin) {
        return Some(explain_unwritable(prefix, &e));
    }
    let probe = bin.join(".sst-gui-write-probe");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            None
        }
        Err(e) => Some(explain_unwritable(prefix, &e)),
    }
}

pub struct UpdateProbes<'a> {
    pub has_tool: &'a dyn Fn(&str) -> bool,
    pub prefix: PathBuf,
    pub os: &'a str,
    pub arch: &'a str,
    pub check_write: bool,
}

impl UpdateProbes<'static> {
    pub fn live() -> Self {
        Self {
            has_tool: &crate::clipboard::tool_on_path,
            prefix: default_install_prefix(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            check_write: true,
        }
    }
}

pub fn evaluate_plan(latest: &str) -> UpdatePlan {
    evaluate_plan_with(latest, UpdateProbes::live())
}

pub fn evaluate_plan_with(latest: &str, probes: UpdateProbes<'_>) -> UpdatePlan {
    if probes.os != "linux" {
        return UpdatePlan::Blocked {
            reason: format!("In-app update is Linux-only (this OS: {}).", probes.os),
            hint: "Copy the command and run it on a Linux x86_64 machine.".into(),
        };
    }
    if probes.arch != "x86_64" && probes.arch != "amd64" {
        return UpdatePlan::Blocked {
            reason: format!(
                "Prebuilt releases are x86_64 only (this machine: {}).",
                probes.arch
            ),
            hint: "Copy the command and run it on x86_64, or build from source.".into(),
        };
    }
    let missing = missing_tools(probes.has_tool);
    if !missing.is_empty() {
        return UpdatePlan::Blocked {
            reason: format!("Missing tools on PATH: {}.", missing.join(", ")),
            hint: "Install them, then press Update now, or copy the command into a terminal."
                .into(),
        };
    }
    if probes.check_write
        && let Some(reason) = prefix_write_error(&probes.prefix)
    {
        return UpdatePlan::Blocked {
            hint: "The Copy button still works. Paste the command into a terminal that can write the prefix.".into(),
            reason,
        };
    }
    UpdatePlan::Ready {
        prefix: probes.prefix,
        tag: release_tag(latest),
    }
}

/// Query GitHub Releases; `None` on failure or when already current.
///
/// `per_page=100` so a player who skipped several releases still gets those
/// bodies for the in-app notes. The bundled changelog covers anything the
/// response leaves out.
pub async fn check_for_update() -> Option<UpdateInfo> {
    let current = current_version()?;
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    let client = reqwest::Client::builder()
        .user_agent("scuffed-stat-tracker-gui")
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    let releases: Vec<serde_json::Value> = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    select_newer_release_on(installed_update_channel(&current), &current, &releases)
}

fn installed_update_channel(current: &str) -> UpdateChannel {
    let stored = stat_tracker::config::Config::read_stored()
        .map(|config| config.update_channel)
        .unwrap_or_default();
    effective_update_channel(&stored, current)
}

/// Channel the update check uses for this install.
///
/// `update_channel = "prerelease"` in config.toml opts in. Any other value
/// that is set (the Settings checkbox writes `"stable"`) means stable. When
/// the key is missing or empty, a release build stays on stable, but an alpha
/// or rc build stays on the prerelease channel. Otherwise an alpha installed
/// with a pinned tag only looks at stable releases, which are all older than
/// it, and is never offered the next alpha.
/// True when [`effective_update_channel`] is `Prerelease`. `installed` is
/// `None` when the version could not be read; that counts as a release.
pub fn prerelease_updates_effective(update_channel: &str, installed: Option<&str>) -> bool {
    effective_update_channel(update_channel, installed.unwrap_or_default())
        == UpdateChannel::Prerelease
}

pub fn effective_update_channel(update_channel: &str, current: &str) -> UpdateChannel {
    let raw = update_channel.trim();
    if raw.eq_ignore_ascii_case("prerelease") {
        return UpdateChannel::Prerelease;
    }
    if !raw.is_empty() {
        return UpdateChannel::Stable;
    }
    match parse_semver(current) {
        Some(ver) if ver.is_prerelease() && ver.is_offered_prerelease() => {
            UpdateChannel::Prerelease
        }
        _ => UpdateChannel::Stable,
    }
}

/// Newest stable `stat-tracker-v*` newer than `current`, plus the bodies of
/// every stable release between them (newest first).
pub fn select_newer_release(current: &str, releases: &[serde_json::Value]) -> Option<UpdateInfo> {
    select_newer_release_on(UpdateChannel::Stable, current, releases)
}

/// Same as [`select_newer_release`], but `Prerelease` may offer an alpha or rc
/// when it is the newest version. Stable never does, including when the
/// GitHub prerelease flag was left off.
pub fn select_newer_release_on(
    channel: UpdateChannel,
    current: &str,
    releases: &[serde_json::Value],
) -> Option<UpdateInfo> {
    let cur = parse_semver(current)?;
    struct Parsed {
        ver: SemVer,
        ver_str: String,
        html: String,
        body: String,
        published_at: Option<String>,
    }
    let mut parsed = Vec::new();
    for release in releases {
        if release["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let Some(ver_str) = release["tag_name"]
            .as_str()
            .and_then(|tag| tag.strip_prefix("stat-tracker-v"))
        else {
            continue;
        };
        let Some(ver) = parse_semver(ver_str) else {
            continue;
        };
        let flagged = release["prerelease"].as_bool().unwrap_or(false);
        if channel == UpdateChannel::Stable && (flagged || ver.is_prerelease()) {
            continue;
        }
        // Only alpha and rc builds go out on the prerelease channel. A test
        // build (`stat-tracker-v0.5.0-test.1`) is never offered to anyone.
        if !ver.is_offered_prerelease() {
            continue;
        }
        parsed.push(Parsed {
            ver,
            ver_str: ver_str.to_string(),
            html: release["html_url"].as_str().unwrap_or_default().to_string(),
            body: release["body"].as_str().unwrap_or_default().to_string(),
            published_at: release["published_at"].as_str().map(str::to_string),
        });
    }
    let best = parsed.iter().max_by(|a, b| a.ver.cmp(&b.ver))?;
    if best.ver <= cur {
        return None;
    }
    let latest_ver = best.ver.clone();
    let latest = best.ver_str.clone();
    let url = best.html.clone();
    let mut notes: Vec<_> = parsed
        .into_iter()
        .filter(|item| item.ver > cur && item.ver <= latest_ver)
        .collect();
    notes.sort_by(|a, b| b.ver.cmp(&a.ver));
    Some(UpdateInfo {
        latest,
        current: current.trim().trim_start_matches('v').to_string(),
        url,
        release_bodies: notes
            .into_iter()
            .map(|item| ReleaseBody {
                version: item.ver_str,
                body: item.body,
                published_at: item.published_at,
            })
            .collect(),
    })
}

/// How often a running app checks for a new release (GUI-7). The app lives
/// in the tray for days; a check at start only never showed a new rc.
pub const UPDATE_RECHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// True when this Tick should check for an update again. Never in fixture
/// mode, never while an update runs, and never before the first check
/// (the one at start) was made.
pub fn update_recheck_due(
    live: bool,
    progress: &UpdateProgress,
    last_check: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    live && !matches!(progress, UpdateProgress::Running)
        && last_check.is_some_and(|at| now.saturating_duration_since(at) >= UPDATE_RECHECK_EVERY)
}

/// A re-check replaces the offer only with a different version. A failed
/// check (`None`) keeps the banner; the same version keeps the open notes.
pub fn recheck_applies(shown: Option<&UpdateInfo>, fetched: Option<&UpdateInfo>) -> bool {
    match (shown, fetched) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(shown), Some(fetched)) => shown.latest != fetched.latest,
    }
}

pub fn open_release_page(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

pub struct UpdateRunRequest {
    pub latest: String,
    pub data_dir: PathBuf,
    pub stop_daemon: bool,
    pub service_installed: bool,
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for x in chars.by_ref() {
                if x.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn tail_useful(output: &str) -> String {
    let cleaned = strip_ansi(output);
    cleaned
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("installer finished")
        .to_string()
}

/// The longest an in-app install may run. The tracker is stopped while it
/// runs, so an installer that hangs (a stalled download) is stopped here,
/// the previous binaries are put back and the tracker is started again.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The installer's stdout and stderr go to this file in the data dir, not to
/// pipes. A pipe closes when this app exits and the next line the installer
/// writes kills it with SIGPIPE, mid-install.
pub const INSTALL_LOG: &str = "update-install.log";

trait UpdateIo {
    async fn control_daemon(
        &self,
        data_dir: &Path,
        verb: DaemonVerb,
        service_installed: bool,
    ) -> Result<String, String>;
    /// Sleep between checks for a game in progress.
    async fn pause(&self, d: std::time::Duration);
    async fn install(
        &self,
        url: &str,
        script: &Path,
        latest: &str,
        prefix: &Path,
        tag: &str,
        log: &Path,
    ) -> Result<String, String>;
    /// How long [`UpdateIo::install`] may run before it is dropped.
    fn install_timeout(&self) -> Duration {
        INSTALL_TIMEOUT
    }
}

struct LiveUpdateIo;

impl UpdateIo for LiveUpdateIo {
    async fn control_daemon(
        &self,
        data_dir: &Path,
        verb: DaemonVerb,
        service_installed: bool,
    ) -> Result<String, String> {
        crate::daemon::run_verb(data_dir.to_path_buf(), verb, service_installed).await
    }

    async fn pause(&self, d: std::time::Duration) {
        tokio::time::sleep(d).await;
    }

    async fn install(
        &self,
        url: &str,
        script: &Path,
        latest: &str,
        prefix: &Path,
        tag: &str,
        log: &Path,
    ) -> Result<String, String> {
        download_and_run_bootstrap(url, script, latest, prefix, tag, log).await
    }
}

/// The tracker has a game open: `active_game.json` exists in the data dir
/// from the game's start until it is closed and saved. A file not written
/// for [`GAME_FILE_STALE`] is left over (a crash or a game the tracker gave
/// up on) and does not count.
pub fn game_in_progress(data_dir: &Path) -> bool {
    stat_tracker::replace::game_file_in_progress(data_dir)
}

/// An `active_game.json` older than this is not a game in progress.
pub const GAME_FILE_STALE: std::time::Duration = std::time::Duration::from_secs(2 * 60 * 60);

/// Shown on the update button while the update waits for the open game.
pub const WAITING_FOR_GAME: &str = "Waiting for your game to finish before updating";

static WAITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True while an update is waiting for the open game to be saved.
pub fn waiting_for_game() -> bool {
    WAITING.load(std::sync::atomic::Ordering::Relaxed)
}

/// How often the update checks whether the open game was saved.
pub const GAME_WAIT_STEP: std::time::Duration = std::time::Duration::from_secs(10);
/// The longest the update waits for an open game before it goes ahead.
pub const GAME_WAIT_MAX: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Wait (up to [`GAME_WAIT_MAX`]) until no game is open, so stopping the
/// tracker for the install never cuts a game that is not saved yet.
/// Returns whether it had to wait.
async fn wait_for_game_saved(data_dir: &Path, io: &impl UpdateIo) -> bool {
    let mut waited = std::time::Duration::ZERO;
    let mut any = false;
    while game_in_progress(data_dir) && waited < GAME_WAIT_MAX {
        any = true;
        WAITING.store(true, std::sync::atomic::Ordering::Relaxed);
        io.pause(GAME_WAIT_STEP).await;
        waited += GAME_WAIT_STEP;
    }
    WAITING.store(false, std::sync::atomic::Ordering::Relaxed);
    any
}

pub async fn run_in_app_update(req: UpdateRunRequest) -> Result<String, String> {
    let plan = evaluate_plan(&req.latest);
    run_planned_update(&req, plan, &LiveUpdateIo).await
}

async fn run_planned_update(
    req: &UpdateRunRequest,
    plan: UpdatePlan,
    io: &impl UpdateIo,
) -> Result<String, String> {
    let UpdatePlan::Ready { prefix, tag } = plan else {
        let UpdatePlan::Blocked { reason, hint } = plan else {
            unreachable!("evaluate_plan is Ready or Blocked");
        };
        return Err(format!("{reason} {hint}"));
    };

    if req.stop_daemon {
        // alpha.8: the restart waits until the open game is saved.
        if wait_for_game_saved(&req.data_dir, io).await {
            tracing::info!("update waited for the game in progress to be saved");
        }
        if let Err(e) = io
            .control_daemon(&req.data_dir, DaemonVerb::Stop, req.service_installed)
            .await
        {
            tracing::warn!(error = %e, "update: stopping the tracker failed; installing anyway");
        }
    }

    // The sequence number keeps two runs in the same millisecond (parallel
    // tests) from sharing, and then deleting, one backup folder.
    static RUN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let work = std::env::temp_dir().join(format!(
        "sst-gui-update-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        RUN_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    if let Err(e) = std::fs::create_dir_all(&work) {
        return note_daemon_left_stopped(
            req.stop_daemon,
            Err(format!("Could not create a temp dir for the update: {e}")),
        );
    }
    let backup = work.join("previous");
    let saved = match backup_binaries(&prefix, &backup) {
        Ok(saved) => saved,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            let mut msg = format!(
                "Could not save the current version before updating, so nothing was changed: {e}"
            );
            if req.stop_daemon {
                let start = io
                    .control_daemon(&req.data_dir, DaemonVerb::Start, req.service_installed)
                    .await;
                append_start_failure(&mut msg, start);
            }
            return Err(msg);
        }
    };
    let script = work.join("bootstrap.sh");
    let url = bootstrap_raw_url(&tag);
    let log = install_log_path(&req.data_dir, &work);
    let limit = io.install_timeout();
    // Dropping the install future on timeout kills the installer and its
    // children (kill_on_drop plus a process-group kill in the live IO), so
    // nothing is still writing to PREFIX/bin when the rollback below runs.
    let install = match tokio::time::timeout(
        limit,
        io.install(&url, &script, &req.latest, &prefix, &tag, &log),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!(?limit, "update: installer timed out and was stopped");
            Err(install_timeout_message(limit))
        }
    };
    let mut rolled_back = false;
    let install = match install {
        Ok(msg) => Ok(msg),
        Err(msg) if saved.contains(&"scuffed-stat-tracker") => {
            match restore_binaries(&prefix, &backup, &saved) {
                Ok(()) => {
                    rolled_back = true;
                    tracing::warn!(error = %msg, "update failed: previous binaries restored");
                    Err(format!("{msg} The previous version was put back."))
                }
                Err(e) => Err(format!(
                    "{msg} Putting the previous version back failed too: {e}."
                )),
            }
        }
        Err(msg) => Err(msg),
    };
    let _ = std::fs::remove_dir_all(&work);
    let start = if should_restart_daemon(req.stop_daemon, install.is_ok(), rolled_back) {
        Some(
            io.control_daemon(&req.data_dir, DaemonVerb::Start, req.service_installed)
                .await,
        )
    } else {
        None
    };
    let install = match start {
        Some(start) => match install {
            Ok(mut msg) => {
                append_start_failure(&mut msg, start);
                Ok(msg)
            }
            Err(mut msg) => {
                if start.is_ok() && rolled_back {
                    msg.push_str(" The tracker was started again.");
                }
                append_start_failure(&mut msg, start);
                Err(msg)
            }
        },
        None => install,
    };
    if rolled_back {
        return install;
    }
    note_daemon_left_stopped(req.stop_daemon, install)
}

/// The installer log in the data dir, or in the update's temp dir when the
/// data dir is missing.
fn install_log_path(data_dir: &Path, work: &Path) -> PathBuf {
    if data_dir.is_dir() {
        data_dir.join(INSTALL_LOG)
    } else {
        work.join(INSTALL_LOG)
    }
}

pub fn install_timeout_message(limit: Duration) -> String {
    let secs = limit.as_secs();
    let took = if secs >= 60 && secs.is_multiple_of(60) {
        format!("{} minutes", secs / 60)
    } else {
        format!("{secs} seconds")
    };
    format!("The installer did not finish within {took}, so it was stopped.")
}

/// A Start that failed after an update is said in the result, so the
/// player knows the tracker is not running.
fn append_start_failure(msg: &mut String, start: Result<String, String>) {
    if let Err(e) = start {
        tracing::warn!(error = %e, "update: starting the tracker again failed");
        msg.push_str(&format!(
            " Starting the tracker again failed ({e}). Press Start in Settings, under Tracker."
        ));
    }
}

fn note_daemon_left_stopped(
    stop_daemon: bool,
    install: Result<String, String>,
) -> Result<String, String> {
    match install {
        Err(msg) if stop_daemon => Err(format!(
            "{msg} The tracker service was left stopped so a failed install is not started."
        )),
        other => other,
    }
}

async fn download_and_run_bootstrap(
    url: &str,
    script: &Path,
    latest: &str,
    prefix: &Path,
    tag: &str,
    log: &Path,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("scuffed-stat-tracker-gui")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Could not start the downloader: {e}"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Could not download bootstrap.sh: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Could not download bootstrap.sh (HTTP {}). Copy the command and run it in a terminal.",
            response.status()
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Could not read bootstrap.sh: {e}"))?;
    std::fs::write(script, &bytes).map_err(|e| format!("Could not write bootstrap.sh: {e}"))?;

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(script);
    apply_installer_env(&mut cmd, latest, prefix);
    let status = run_logged(cmd, log).await?;
    let combined = std::fs::read_to_string(log).unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "Installer failed ({tag}): {}. Copy the command and run it in a terminal if this keeps failing.",
            tail_useful(&combined)
        ));
    }
    // The restart hint is added where the message is shown, once the GUI
    // knows whether this install changed the app (G3).
    Ok(format!("Installed {tag}. {}", tail_useful(&combined)))
}

/// Runs `cmd` with stdout and stderr appended to a fresh `log` and stdin
/// closed. The child gets its own process group. Dropping the returned
/// future (the install timeout) kills the whole group, so a curl that bash
/// started does not keep running. A clean exit leaves the group alone.
async fn run_logged(
    mut cmd: tokio::process::Command,
    log: &Path,
) -> Result<std::process::ExitStatus, String> {
    let out = std::fs::File::create(log)
        .map_err(|e| format!("Could not create the installer log {}: {e}", log.display()))?;
    let err = out
        .try_clone()
        .map_err(|e| format!("Could not open the installer log: {e}"))?;
    cmd.stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not run bootstrap.sh: {e}"))?;
    let mut group = KillGroupOnDrop(child.id());
    let status = child
        .wait()
        .await
        .map_err(|e| format!("Could not wait for bootstrap.sh: {e}"));
    group.0 = None;
    status
}

/// SIGKILLs a process group when dropped while armed. The leader is not
/// reaped before this runs (the drop happens before `Child`'s), so its pid,
/// which is the group id, cannot have been reused.
struct KillGroupOnDrop(Option<u32>);

impl Drop for KillGroupOnDrop {
    fn drop(&mut self) {
        if let Some(pgid) = self.0.take().and_then(|p| i32::try_from(p).ok())
            && pgid > 0
        {
            // SAFETY: kill(2) with a negative pid signals that group; no
            // memory is touched.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
}

/// Notes stay closed until the player presses What's new.
pub fn update_notes_expanded_default() -> bool {
    false
}

/// The banner draws release cards only after that button.
pub fn banner_shows_notes(expanded: bool) -> bool {
    expanded
}

/// Heading for the compact banner. One line, no extra punctuation.
pub fn banner_title(latest: &str) -> String {
    format!("Update available: v{latest}")
}

/// Characters of the player summary kept on the compact banner.
pub const BANNER_LEAD_CHARS: usize = 110;

/// First sentence of the newest release, cut on a word boundary when it is long.
pub fn banner_lead(summary: &str) -> String {
    let sentence = first_sentence(summary.trim());
    truncate_clean(sentence, BANNER_LEAD_CHARS)
}

fn first_sentence(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'.' {
            let rest = text[i + 1..].trim_start();
            if rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_uppercase()) {
                return text[..=i].trim();
            }
        }
        i += 1;
    }
    text
}

fn truncate_clean(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let mut end = text.len();
    for (seen, (i, _)) in text.char_indices().enumerate() {
        if seen == keep {
            end = i;
            break;
        }
    }
    let cut = text[..end].trim_end();
    let cut = match cut.rfind(' ') {
        Some(space) if space > 0 => &cut[..space],
        _ => cut,
    };
    let cut = cut.trim_end_matches(['.', ',', ';', ':']);
    format!("{cut}...")
}

/// Shown after a successful in-app update. True with or without a tray:
/// the button starts the new binary and quits this one.
pub const RESTART_HINT: &str = "Press Restart now to run the new version.";

/// Shown after a successful in-app update that left the app's own build
/// unchanged (no GUI or shared lib code changed). The tracker service was
/// restarted. This window keeps its compiled version until it next starts,
/// so About, problem reports and a channel-switch update check still use the
/// older version string; a recheck may offer the release that is already
/// installed, which reinstalls the same files.
pub const NO_RESTART_HINT: &str = "The app itself did not change, so no restart is needed.";

/// Label of the banner's first button when no restart is needed.
pub const NO_RESTART_LABEL: &str = "No restart needed";

/// The hint after "Installed vX.". Until the installed build is read, it
/// offers Restart now, as before G3.
pub fn success_hint(need: RestartNeed) -> &'static str {
    match need {
        RestartNeed::NotNeeded => NO_RESTART_HINT,
        RestartNeed::Needed | RestartNeed::Unknown => RESTART_HINT,
    }
}

/// The installer's message plus [`success_hint`], as its own sentence on its
/// own line. The installer's last line often has no full stop
/// ("Done. Launch with: stat-tracker-gui").
pub fn success_line(msg: &str, need: RestartNeed) -> String {
    let msg = msg.trim_end();
    let stop = if msg.ends_with(['.', '!', '?']) {
        ""
    } else {
        "."
    };
    format!("{msg}{stop}\n{}", success_hint(need))
}

/// Label and message of the banner's first button. After a successful
/// install it restarts the GUI, because the single-instance lock means a
/// relaunch would only bring back this (old) process. When the install did
/// not change the app's build, there is nothing to restart.
pub fn primary_action(
    progress: &UpdateProgress,
    can_run: bool,
    need: RestartNeed,
) -> (&'static str, Option<Message>) {
    match progress {
        UpdateProgress::Running if waiting_for_game() => (WAITING_FOR_GAME, None),
        UpdateProgress::Running => ("Installing…", None),
        UpdateProgress::Succeeded(_) if need == RestartNeed::NotNeeded => (NO_RESTART_LABEL, None),
        UpdateProgress::Succeeded(_) => ("Restart now", Some(Message::RestartGui)),
        _ if can_run => ("Update now", Some(Message::RunUpdate)),
        _ => ("Update now", None),
    }
}

pub fn banner<'a>(
    info: &UpdateInfo,
    progress: &UpdateProgress,
    plan: &UpdatePlan,
    notes: &'a [crate::notes::ShownRelease],
    notes_ui: crate::notes::NotesUi<'a>,
    notes_expanded: bool,
    restart: RestartNeed,
) -> Element<'a, Message> {
    let url = info.url.clone();
    let cmd = pinned_install_command(&info.latest);
    let running = matches!(progress, UpdateProgress::Running);
    let can_run = plan.can_run() && !running && !matches!(progress, UpdateProgress::Succeeded(_));
    let lead = notes
        .first()
        .map(|section| banner_lead(&section.summary))
        .filter(|lead| !lead.is_empty());

    let mut body = column![
        text(banner_title(&info.latest))
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
    ]
    .spacing(8);
    if let Some(lead) = lead {
        body = body.push(text(lead).size(SIZE_BODY).font(FONT_MEDIUM).color(TEXT_2));
    }

    match progress {
        UpdateProgress::Idle => {}
        UpdateProgress::Running => {
            body = body.push(
                text("Downloading the release and running the installer… The tracker service is stopped for the install.")
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3),
            );
        }
        UpdateProgress::Succeeded(msg) => {
            body = body.push(
                text(success_line(msg, restart))
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(theme::OK),
            );
        }
        UpdateProgress::Failed(msg) => {
            body = body.push(
                text(msg.clone())
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(theme::DANGER),
            );
        }
    }

    if let UpdatePlan::Blocked { reason, hint } = plan
        && !matches!(progress, UpdateProgress::Succeeded(_))
    {
        body = body.push(
            text(format!("{reason} {hint}"))
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::WARN),
        );
    }

    body = body.push(
        text(cmd)
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(theme::ACCENT),
    );

    let (label, on_press) = primary_action(progress, can_run, restart);
    let mut update_btn = button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
        .padding(Padding::from([8, 16]))
        .style(theme::chip(on_press.is_some()));
    if let Some(message) = on_press {
        update_btn = update_btn.on_press(message);
    }

    let actions = row![
        update_btn,
        button(
            text("Copy command")
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        )
        .padding(Padding::from([8, 16]))
        .style(theme::ghost_btn())
        .on_press(Message::CopyUpdateCmd),
        button(
            text("Open on GitHub")
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        )
        .padding(Padding::from([8, 16]))
        .style(theme::ghost_btn())
        .on_press(Message::OpenUpdate(url)),
        button(
            text(if banner_shows_notes(notes_expanded) {
                "Hide what's new"
            } else {
                "What's new"
            })
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
        )
        .padding(Padding::from([8, 16]))
        .style(theme::ghost_btn())
        .on_press(Message::ToggleUpdateNotes),
    ]
    .spacing(8);

    body = body.push(actions);
    if banner_shows_notes(notes_expanded) {
        body = body.push(update_notes_block(info, notes, notes_ui));
    }

    container(body)
        .padding(PAD_INNER)
        .width(Fill)
        .style(|_t| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::SURFACE)),
            text_color: Some(TEXT),
            border: iced::Border {
                color: theme::WARN,
                width: 1.0,
                radius: theme::card_radius(),
            },
            ..iced::widget::container::Style::default()
        })
        .into()
}

fn update_notes_block<'a>(
    info: &UpdateInfo,
    notes: &'a [crate::notes::ShownRelease],
    notes_ui: crate::notes::NotesUi<'a>,
) -> Element<'a, Message> {
    if notes.is_empty() {
        return text(format!(
            "Release notes for v{} could not be loaded. They will show here after you update.",
            info.latest
        ))
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_2)
        .into();
    }
    let heading = if notes.len() > 1 {
        "What's new, including versions since the one you have installed."
    } else {
        "What's new in this update."
    };
    column![
        text(heading)
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2),
        crate::notes::notes_column(notes, notes_ui),
    ]
    .spacing(12)
    .width(Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::DaemonVerb;
    use std::path::PathBuf;

    #[test]
    fn select_newer_release_keeps_bodies_between_current_and_latest() {
        let releases = vec![
            serde_json::json!({
                "tag_name": "stat-tracker-v0.4.22",
                "draft": false,
                "prerelease": false,
                "html_url": "https://example.com/22",
                "body": "notes 22",
                "published_at": "2026-02-01T00:00:00Z"
            }),
            serde_json::json!({
                "tag_name": "stat-tracker-v0.4.23",
                "draft": false,
                "prerelease": false,
                "html_url": "https://example.com/23",
                "body": "notes 23"
            }),
            serde_json::json!({
                "tag_name": "stat-tracker-v0.4.24",
                "draft": true,
                "html_url": "https://example.com/draft",
                "body": "draft"
            }),
            serde_json::json!({
                "tag_name": "stat-tracker-v0.4.21",
                "draft": false,
                "html_url": "https://example.com/21",
                "body": "notes 21"
            }),
        ];
        let info = select_newer_release("0.4.21", &releases).expect("update");
        assert_eq!(info.latest, "0.4.23");
        assert_eq!(info.current, "0.4.21");
        assert_eq!(info.url, "https://example.com/23");
        assert_eq!(
            info.release_bodies
                .iter()
                .map(|body| (
                    body.version.as_str(),
                    body.body.as_str(),
                    body.published_at.as_deref()
                ))
                .collect::<Vec<_>>(),
            [
                ("0.4.23", "notes 23", None),
                ("0.4.22", "notes 22", Some("2026-02-01T00:00:00Z"))
            ]
        );
        assert!(select_newer_release("0.4.23", &releases).is_none());
    }

    #[test]
    fn update_banner_notes_start_collapsed() {
        assert!(!update_notes_expanded_default());
        assert!(!banner_shows_notes(update_notes_expanded_default()));
        assert!(banner_shows_notes(true));
        assert_eq!(banner_title("0.4.23"), "Update available: v0.4.23");
        assert!(!banner_title("0.4.23").contains('\u{2014}'));
    }

    #[test]
    fn banner_lead_keeps_one_sentence_and_cuts_on_a_word() {
        let summary = "You can turn on an extra number reader that only writes a private log on this computer. It does not change your saved games or what gets uploaded, and it stays off unless you enable it.";
        assert_eq!(
            banner_lead(summary),
            "You can turn on an extra number reader that only writes a private log on this computer."
        );
        let long = "alpha ".repeat(40);
        let cut = banner_lead(long.trim());
        assert!(cut.ends_with("..."), "{cut}");
        assert!(cut.chars().count() <= BANNER_LEAD_CHARS, "{cut}");
        assert!(!cut[..cut.len() - 3].ends_with(' '));
        assert_eq!(banner_lead("Short note."), "Short note.");
    }

    #[test]
    fn semver_parses_and_orders() {
        assert_eq!(parse_semver("0.2.1").unwrap().to_string(), "0.2.1");
        assert_eq!(parse_semver("v1.0.0").unwrap().to_string(), "1.0.0");
        assert_eq!(parse_semver("0.2").unwrap().to_string(), "0.2.0");
        assert!(parse_semver("0.3.0-rc1").unwrap().is_prerelease());
        assert!(parse_semver("0.3.0").unwrap() > parse_semver("0.3.0-rc1").unwrap());
        assert!(parse_semver("garbage").is_none());
        assert!(parse_semver("0.5.0-").is_none());
        assert!(parse_semver("0.2.1").unwrap() > parse_semver("0.1.0").unwrap());
        assert!(parse_semver("0.2.0").unwrap() > parse_semver("0.1.9").unwrap());
        assert!(is_newer("0.3.0", "0.2.1"));
        assert!(!is_newer("0.2.1", "0.2.1"));
        assert!(!is_newer("nope", "0.1.0"));
        assert!(parse_semver("0.4.24").unwrap() < parse_semver("0.5.0-alpha.1").unwrap());
        assert!(parse_semver("0.5.0-alpha.1").unwrap() < parse_semver("0.5.0").unwrap());
        assert!(is_newer("0.5.0-alpha.1", "0.4.24"));
        assert!(is_newer("0.5.0", "0.5.0-alpha.1"));
        assert!(!is_newer("0.5.0-alpha.1", "0.5.0"));
        assert!(is_newer("0.5.0-alpha.2", "0.5.0-alpha.1"));
        assert!(!is_newer("0.5.0-alpha.1", "0.5.0-alpha.2"));
        assert!(is_newer("0.5.0-alpha.10", "0.5.0-alpha.2"));
        assert!(parse_semver("0.5.0-1").unwrap() < parse_semver("0.5.0-alpha").unwrap());
    }

    fn gh_release(tag: &str, prerelease: bool, body: &str) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": prerelease,
            "html_url": "https://example.com/rel",
            "body": body,
        })
    }

    #[test]
    fn stable_skips_alpha_unless_the_channel_opts_in() {
        let alpha_only = vec![
            gh_release("stat-tracker-v0.5.0-alpha.1", true, "alpha notes"),
            gh_release("stat-tracker-v0.4.24", false, "stable notes"),
        ];
        assert!(
            select_newer_release("0.4.24", &alpha_only).is_none(),
            "stable must not be offered the alpha"
        );
        let opted = select_newer_release_on(UpdateChannel::Prerelease, "0.4.24", &alpha_only)
            .expect("prerelease channel");
        assert_eq!(opted.latest, "0.5.0-alpha.1");

        let unflagged = vec![gh_release(
            "stat-tracker-v0.5.0-alpha.1",
            false,
            "alpha without the github flag",
        )];
        assert!(
            select_newer_release("0.4.24", &unflagged).is_none(),
            "a prerelease suffix is skipped on stable even without the flag"
        );

        let with_final = vec![
            gh_release("stat-tracker-v0.5.0", false, "final notes"),
            gh_release("stat-tracker-v0.5.0-alpha.1", true, "alpha notes"),
            gh_release("stat-tracker-v0.4.24", false, "old notes"),
        ];
        let stable = select_newer_release("0.4.24", &with_final).expect("final");
        assert_eq!(stable.latest, "0.5.0");
        assert!(
            stable
                .release_bodies
                .iter()
                .all(|body| body.version != "0.5.0-alpha.1")
        );
        let from_alpha = select_newer_release("0.5.0-alpha.1", &with_final).expect("upgrade");
        assert_eq!(from_alpha.latest, "0.5.0");
    }

    #[test]
    fn alpha_numbers_compare_as_numbers_not_text() {
        let v = |s: &str| parse_semver(s).unwrap();
        assert!(v("0.5.0-alpha.3") < v("0.5.0-alpha.10"));
        assert!(v("0.5.0-alpha.10") < v("0.5.0-alpha.11"));
        assert!(v("0.5.0-alpha.3") < v("0.5.0-alpha.11"));
        assert!(v("0.5.0") > v("0.5.0-alpha.11"));
        assert!(is_newer("0.5.0-alpha.11", "0.5.0-alpha.3"));
        assert!(!is_newer("0.5.0-alpha.3", "0.5.0-alpha.11"));
    }

    #[test]
    fn rc3_is_offered_rc4_on_the_prerelease_channel_and_stable_is_not() {
        let releases = vec![
            gh_release("stat-tracker-v0.5.0-rc.4", true, "rc4"),
            gh_release("stat-tracker-v0.5.0-rc.3", true, "rc3"),
            gh_release("stat-tracker-v0.4.24", false, "stable"),
        ];
        let from_rc3 = select_newer_release_on(
            effective_update_channel("", "0.5.0-rc.3"),
            "0.5.0-rc.3",
            &releases,
        )
        .expect("an rc.3 install on the default channel is offered rc.4");
        assert_eq!(from_rc3.latest, "0.5.0-rc.4");
        assert!(
            select_newer_release("0.4.24", &releases).is_none(),
            "stable must not be offered rc.4"
        );
        assert!(
            select_newer_release_on(UpdateChannel::Prerelease, "0.5.0-rc.4", &releases).is_none(),
            "rc.4 is not offered itself or older rcs"
        );
    }

    #[test]
    fn rc2_is_offered_rc3_on_the_prerelease_channel_and_stable_is_not() {
        let releases = vec![
            gh_release("stat-tracker-v0.5.0-rc.3", true, "rc3"),
            gh_release("stat-tracker-v0.5.0-rc.2", true, "rc2"),
            gh_release("stat-tracker-v0.4.24", false, "stable"),
        ];
        let from_rc2 = select_newer_release_on(
            effective_update_channel("", "0.5.0-rc.2"),
            "0.5.0-rc.2",
            &releases,
        )
        .expect("an rc.2 install on the default channel is offered rc.3");
        assert_eq!(from_rc2.latest, "0.5.0-rc.3");
        assert!(
            select_newer_release("0.4.24", &releases).is_none(),
            "stable must not be offered rc.3"
        );
        assert!(
            select_newer_release_on(UpdateChannel::Prerelease, "0.5.0-rc.3", &releases).is_none(),
            "rc.3 is not offered itself or older rcs"
        );
    }

    #[test]
    fn rc1_is_offered_rc2_on_the_prerelease_channel_and_stable_is_not() {
        let releases = vec![
            gh_release("stat-tracker-v0.5.0-rc.2", true, "rc2"),
            gh_release("stat-tracker-v0.5.0-rc.1", true, "rc1"),
            gh_release("stat-tracker-v0.4.24", false, "stable"),
        ];
        let from_rc1 = select_newer_release_on(
            effective_update_channel("", "0.5.0-rc.1"),
            "0.5.0-rc.1",
            &releases,
        )
        .expect("an rc.1 install on the default channel is offered rc.2");
        assert_eq!(from_rc1.latest, "0.5.0-rc.2");
        let opted = select_newer_release_on(UpdateChannel::Prerelease, "0.5.0-rc.1", &releases)
            .expect("prerelease channel");
        assert_eq!(opted.latest, "0.5.0-rc.2");
        assert!(
            select_newer_release("0.4.24", &releases).is_none(),
            "stable must not be offered rc.2"
        );
        assert!(
            select_newer_release_on(UpdateChannel::Prerelease, "0.5.0-rc.2", &releases).is_none(),
            "rc.2 is not offered itself or rc.1"
        );
    }

    #[test]
    fn rc_sorts_between_the_alphas_and_the_release_and_stays_off_stable() {
        let v = |s: &str| parse_semver(s).unwrap();
        assert!(v("0.5.0-alpha.12") < v("0.5.0-rc.1"));
        assert!(v("0.5.0-alpha.13") < v("0.5.0-rc.1"));
        assert!(v("0.5.0-rc.1") < v("0.5.0-rc.2"));
        assert!(v("0.5.0-rc.1") < v("0.5.0"));
        assert!(is_newer("0.5.0-rc.1", "0.5.0-alpha.12"));
        assert!(!is_newer("0.5.0-rc.1", "0.5.0"));
        assert!(v("0.5.0-rc.1").is_offered_prerelease());

        let releases = vec![
            gh_release("stat-tracker-v0.5.0-rc.1", true, "rc1"),
            gh_release("stat-tracker-v0.5.0-alpha.12", true, "a12"),
            gh_release("stat-tracker-v0.4.24", false, "stable"),
        ];
        assert!(
            select_newer_release("0.4.24", &releases).is_none(),
            "stable must not be offered rc.1"
        );
        let unflagged = vec![gh_release("stat-tracker-v0.5.0-rc.1", false, "rc1")];
        assert!(
            select_newer_release("0.4.24", &unflagged).is_none(),
            "an rc suffix is skipped on stable even without the github flag"
        );
        assert_eq!(
            effective_update_channel("stable", "0.5.0-alpha.12"),
            UpdateChannel::Stable
        );
        let from_alpha = select_newer_release_on(
            effective_update_channel("", "0.5.0-alpha.12"),
            "0.5.0-alpha.12",
            &releases,
        )
        .expect("an alpha install on the default channel is offered rc.1");
        assert_eq!(from_alpha.latest, "0.5.0-rc.1");
        let opted = select_newer_release_on(UpdateChannel::Prerelease, "0.4.24", &releases)
            .expect("prerelease channel");
        assert_eq!(opted.latest, "0.5.0-rc.1");
        let with_final = vec![
            gh_release("stat-tracker-v0.5.0", false, "final"),
            gh_release("stat-tracker-v0.5.0-rc.1", true, "rc1"),
        ];
        let from_rc = select_newer_release("0.5.0-rc.1", &with_final).expect("final");
        assert_eq!(from_rc.latest, "0.5.0");
        assert!(select_newer_release_on(UpdateChannel::Prerelease, "0.5.0", &with_final).is_none());
    }

    #[test]
    fn unset_channel_follows_an_alpha_build() {
        use UpdateChannel::{Prerelease, Stable};
        assert_eq!(effective_update_channel("", "0.5.0-alpha.3"), Prerelease);
        assert_eq!(effective_update_channel("  ", "0.5.0-rc1"), Prerelease);
        assert_eq!(effective_update_channel("", "0.4.24"), Stable);
        assert_eq!(effective_update_channel("", "0.5.0-test.1"), Stable);
        assert_eq!(effective_update_channel("", "garbage"), Stable);
        assert_eq!(effective_update_channel("stable", "0.5.0-alpha.3"), Stable);
        assert_eq!(effective_update_channel("prerelease", "0.4.24"), Prerelease);
        assert_eq!(effective_update_channel("PreRelease", "0.4.24"), Prerelease);
    }

    #[test]
    fn alpha_3_on_the_default_channel_is_offered_alpha_11() {
        // Newest first, the way GitHub lists them, with a site tag and the
        // last stable release mixed in.
        let mut releases = vec![
            gh_release("stat-tracker-v0.5.0-alpha.11", true, "a11"),
            gh_release("stat-tracker-v0.5.0-alpha.9", true, "a9"),
            gh_release("stat-tracker-v0.5.0-alpha.10", true, "a10"),
        ];
        for n in (4..=8).rev() {
            releases.push(gh_release(
                &format!("stat-tracker-v0.5.0-alpha.{n}"),
                true,
                &format!("a{n}"),
            ));
        }
        releases.push(gh_release("stat-tracker-v0.5.0-alpha.3", true, "a3"));
        releases.push(gh_release("stat-tracker-v0.4.24", false, "stable"));
        releases.push(gh_release("site-2026-10-05-143-145", false, "site"));

        // Before the fix: an unset channel meant stable, every stable release
        // is older than alpha.3, so nothing was ever offered.
        assert!(select_newer_release("0.5.0-alpha.3", &releases).is_none());

        let channel = effective_update_channel("", "0.5.0-alpha.3");
        let info = select_newer_release_on(channel, "0.5.0-alpha.3", &releases)
            .expect("alpha.3 is offered an update");
        assert_eq!(info.latest, "0.5.0-alpha.11");
        assert_eq!(info.current, "0.5.0-alpha.3");
        let versions: Vec<_> = info
            .release_bodies
            .iter()
            .map(|body| body.version.as_str())
            .collect();
        assert_eq!(
            versions,
            [
                "0.5.0-alpha.11",
                "0.5.0-alpha.10",
                "0.5.0-alpha.9",
                "0.5.0-alpha.8",
                "0.5.0-alpha.7",
                "0.5.0-alpha.6",
                "0.5.0-alpha.5",
                "0.5.0-alpha.4",
            ]
        );

        // Already current: nothing to offer.
        let current = effective_update_channel("", "0.5.0-alpha.11");
        assert!(select_newer_release_on(current, "0.5.0-alpha.11", &releases).is_none());

        // A player who turned alpha updates off in Settings stays on stable.
        let off = effective_update_channel("stable", "0.5.0-alpha.3");
        assert!(select_newer_release_on(off, "0.5.0-alpha.3", &releases).is_none());

        // The final release still wins once it ships.
        releases.insert(0, gh_release("stat-tracker-v0.5.0", false, "final"));
        let info = select_newer_release_on(channel, "0.5.0-alpha.3", &releases).expect("final");
        assert_eq!(info.latest, "0.5.0");
    }

    #[test]
    fn success_offers_restart_now() {
        let done = UpdateProgress::Succeeded("ok".into());
        for need in [RestartNeed::Unknown, RestartNeed::Needed] {
            let (label, msg) = primary_action(&done, false, need);
            assert_eq!(label, "Restart now", "{need:?}");
            assert!(matches!(msg, Some(Message::RestartGui)));
            assert!(success_line("Installed v1.", need).ends_with(RESTART_HINT));
        }
        let (label, msg) = primary_action(&UpdateProgress::Idle, true, RestartNeed::Unknown);
        assert_eq!(label, "Update now");
        assert!(matches!(msg, Some(Message::RunUpdate)));
        assert!(
            primary_action(&UpdateProgress::Idle, false, RestartNeed::Unknown)
                .1
                .is_none()
        );
        assert!(
            primary_action(&UpdateProgress::Running, true, RestartNeed::Needed)
                .1
                .is_none()
        );
        assert!(
            !RESTART_HINT.contains("window"),
            "without a tray, closing the window is not a restart"
        );
    }

    #[test]
    fn success_hint_is_its_own_sentence_on_its_own_line() {
        let installer =
            "Installed stat-tracker-v1. [bootstrap] Done. Launch with: stat-tracker-gui";
        assert_eq!(
            success_line(installer, RestartNeed::Needed),
            "Installed stat-tracker-v1. [bootstrap] Done. Launch with: stat-tracker-gui.\nPress Restart now to run the new version."
        );
        assert_eq!(
            success_line("Installed v1. Done.  ", RestartNeed::NotNeeded),
            "Installed v1. Done.\nThe app itself did not change, so no restart is needed."
        );
    }

    #[test]
    fn daemon_only_update_says_no_restart_is_needed() {
        let done = UpdateProgress::Succeeded("Installed stat-tracker-v1.".into());
        let (label, msg) = primary_action(&done, false, RestartNeed::NotNeeded);
        assert_eq!(label, NO_RESTART_LABEL);
        assert!(msg.is_none(), "nothing to restart");
        let line = success_line("Installed stat-tracker-v1.", RestartNeed::NotNeeded);
        assert_eq!(
            line,
            "Installed stat-tracker-v1.\nThe app itself did not change, so no restart is needed."
        );
        assert!(!line.contains("Restart now"));
        for copy in [NO_RESTART_HINT, NO_RESTART_LABEL] {
            assert!(!copy.contains('\u{2014}') && !copy.contains('\u{2013}'));
        }
    }

    #[test]
    fn four_part_prerelease_orders_between_alphas() {
        let a5 = parse_semver("0.5.0-alpha.5").unwrap();
        let a51 = parse_semver("0.5.0-alpha.5.1").unwrap();
        let a6 = parse_semver("0.5.0-alpha.6").unwrap();
        assert!(a5 < a51, "alpha.5 < alpha.5.1");
        assert!(a51 < a6, "alpha.5.1 < alpha.6");
        assert!(a51 < parse_semver("0.5.0").unwrap());
        assert!(is_newer("0.5.0-alpha.5.1", "0.5.0-alpha.5"));
        assert!(!is_newer("0.5.0-alpha.5", "0.5.0-alpha.5.1"));

        let releases = vec![
            gh_release("stat-tracker-v0.5.0-alpha.5.1", true, "point fix"),
            gh_release("stat-tracker-v0.5.0-alpha.5", true, "alpha.5"),
        ];
        let offered =
            select_newer_release_on(UpdateChannel::Prerelease, "0.5.0-alpha.5", &releases)
                .expect("alpha.5 sees alpha.5.1 on the prerelease channel");
        assert_eq!(offered.latest, "0.5.0-alpha.5.1");
        assert!(
            select_newer_release_on(UpdateChannel::Prerelease, "0.5.0-alpha.5.1", &releases)
                .is_none()
        );
        assert!(select_newer_release("0.5.0-alpha.5", &releases).is_none());
    }

    #[test]
    fn resolve_prefers_env_then_compile_then_daemon() {
        assert_eq!(
            resolve_current_version(Some("0.3.3"), Some("0.2.0"), Some("0.1.9")).as_deref(),
            Some("0.3.3")
        );
        assert_eq!(
            resolve_current_version(Some(""), Some("v0.3.1"), Some("0.2.0")).as_deref(),
            Some("0.3.1")
        );
        assert_eq!(
            resolve_current_version(None, None, Some("0.3.3")).as_deref(),
            Some("0.3.3")
        );
        assert_eq!(resolve_current_version(None, None, None), None);
        assert_eq!(
            resolve_current_version(Some("not-a-version"), None, None),
            None
        );
    }

    #[test]
    fn daemon_version_line_parses() {
        assert_eq!(
            parse_daemon_version_line("scuffed-stat-tracker 0.3.3").as_deref(),
            Some("0.3.3")
        );
        assert_eq!(
            parse_daemon_version_line("scuffed-stat-tracker v0.3.3\n").as_deref(),
            Some("0.3.3")
        );
        assert!(parse_daemon_version_line("scuffed-stat-tracker").is_none());
        assert!(parse_daemon_version_line("garbage").is_none());
    }

    #[test]
    fn gui_version_line_reports_the_daemon_package() {
        let version = daemon_package_version();
        assert_ne!(version, env!("CARGO_PKG_VERSION"));
        assert_ne!(version, "0.1.0");
        assert_eq!(
            parse_daemon_version_line(&gui_version_line()).as_deref(),
            Some(version.as_str())
        );
        assert!(gui_version_line().starts_with("stat-tracker-gui "));
        assert!(!gui_version_line().contains('\u{2014}'));
        assert!(!gui_version_line().contains('\u{2013}'));
        if option_env!("SST_RELEASE_VERSION")
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            let packaged = crate::notes::stat_tracker_package_version(include_str!(
                "../../stat-tracker/Cargo.toml"
            ))
            .expect("daemon package version");
            assert_eq!(version, packaged);
        }
    }

    #[test]
    fn ui_crate_version_is_not_a_source() {
        // The UI package is 0.1.0; that must never be treated as "the tracker".
        assert_ne!(env!("CARGO_PKG_VERSION"), "0.3.3");
        assert_eq!(
            resolve_current_version(None, None, None),
            None,
            "no fallback to CARGO_PKG_VERSION"
        );
    }

    #[test]
    fn release_tag_and_pinned_command_fetch_that_tag() {
        assert_eq!(release_tag("0.4.7"), "stat-tracker-v0.4.7");
        assert_eq!(release_tag("v0.4.7"), "stat-tracker-v0.4.7");
        let cmd = pinned_install_command("0.4.7");
        let url = bootstrap_raw_url("stat-tracker-v0.4.7");
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.4.7/crates/stat-tracker/dist/bootstrap.sh"
        );
        assert_eq!(
            cmd,
            format!(
                "curl --proto '=https' -fsSL {url} | STAT_TRACKER_TAG=stat-tracker-v0.4.7 bash"
            )
        );
        assert!(!cmd.contains("/main/"), "{cmd}");
        assert_eq!(bootstrap_raw_url("main"), BOOTSTRAP_URL);
        assert!(UPDATE_CMD.contains(BOOTSTRAP_URL));
    }

    #[test]
    fn install_command_pins_advertised_tag_then_current() {
        let advertised = install_command_for(Some("0.4.15"), Some("0.4.14"));
        assert!(
            advertised.contains("/stat-tracker-v0.4.15/"),
            "{advertised}"
        );
        assert!(
            advertised.contains("STAT_TRACKER_TAG=stat-tracker-v0.4.15 bash"),
            "{advertised}"
        );
        assert!(!advertised.contains("/main/"), "{advertised}");

        let current = install_command_for(None, Some("v0.4.14"));
        assert!(current.contains("/stat-tracker-v0.4.14/"), "{current}");
        assert!(
            current.contains("STAT_TRACKER_TAG=stat-tracker-v0.4.14 bash"),
            "{current}"
        );

        assert_eq!(install_command_for(None, None), UPDATE_CMD);
        assert_eq!(install_command_for(Some("not-a-version"), None), UPDATE_CMD);
    }

    #[test]
    fn prerelease_channel_offers_only_alpha_and_rc() {
        for (ver, offered) in [
            ("0.5.0", true),
            ("0.5.0-alpha.10", true),
            ("0.5.0-rc.1", true),
            ("0.3.0-rc1", true),
            ("0.5.0-ALPHA.2", true),
            ("0.5.0-test.1", false),
            ("0.5.0-beta.1", false),
            ("0.5.0-1", false),
            ("0.5.0-alphatest", false),
            ("0.5.0-rcx.1", false),
        ] {
            assert_eq!(
                parse_semver(ver).unwrap().is_offered_prerelease(),
                offered,
                "{ver}"
            );
        }
    }

    #[test]
    fn a_test_build_is_never_offered_on_any_channel() {
        let releases = vec![
            gh_release("stat-tracker-v0.5.0-test.1", false, "test build"),
            gh_release("stat-tracker-v0.5.0-alpha.10", true, "alpha.10"),
            gh_release("stat-tracker-v0.5.0-alpha.9", true, "alpha.9"),
        ];
        let pre = select_newer_release_on(UpdateChannel::Prerelease, "0.5.0-alpha.9", &releases)
            .expect("alpha.10 is offered");
        assert_eq!(pre.latest, "0.5.0-alpha.10");
        assert!(
            pre.release_bodies
                .iter()
                .all(|b| b.version != "0.5.0-test.1")
        );
        assert!(
            select_newer_release_on(UpdateChannel::Prerelease, "0.5.0-alpha.10", &releases)
                .is_none()
        );
        assert!(select_newer_release("0.5.0-alpha.9", &releases).is_none());
        // Flagged as a prerelease on GitHub does not matter either.
        let flagged = vec![gh_release("stat-tracker-v0.5.0-test.1", true, "test")];
        assert!(
            select_newer_release_on(UpdateChannel::Prerelease, "0.5.0-alpha.9", &flagged).is_none()
        );
    }

    #[test]
    fn required_tools_include_python3_and_sha256sum() {
        let missing = missing_tools(|name| !matches!(name, "python3" | "sha256sum"));
        assert_eq!(missing, vec!["python3", "sha256sum"]);
        assert!(missing_tools(|_| true).is_empty());
    }

    #[test]
    fn failed_install_restarts_only_after_a_rollback() {
        assert!(!should_restart_daemon(true, false, false));
        assert!(should_restart_daemon(true, false, true));
        assert!(should_restart_daemon(true, true, false));
        assert!(!should_restart_daemon(false, true, false));
        assert!(!should_restart_daemon(false, false, true));
    }

    #[test]
    fn bootstrap_env_pins_tag_channel_and_prefix() {
        let env = bootstrap_env("0.4.7", Path::new("/tmp/prefix"));
        assert_eq!(
            env.iter()
                .find(|(k, _)| *k == "STAT_TRACKER_TAG")
                .map(|(_, v)| v.as_str()),
            Some("stat-tracker-v0.4.7")
        );
        assert_eq!(
            env.iter()
                .find(|(k, _)| *k == "STAT_TRACKER_CHANNEL")
                .map(|(_, v)| v.as_str()),
            Some("stable")
        );
        assert_eq!(
            env.iter()
                .find(|(k, _)| *k == "STAT_TRACKER_PREFIX")
                .map(|(_, v)| v.as_str()),
            Some("/tmp/prefix")
        );
        assert_eq!(
            env.iter()
                .find(|(k, _)| *k == "STAT_TRACKER_BOOTSTRAP_PINNED")
                .map(|(_, v)| v.as_str()),
            Some("1")
        );
    }

    #[test]
    fn prefix_from_exe_only_when_under_bin() {
        assert_eq!(
            prefix_from_exe(Path::new("/home/ada/.local/bin/stat-tracker-gui")),
            Some(PathBuf::from("/home/ada/.local"))
        );
        assert_eq!(
            prefix_from_exe(Path::new("/opt/scuffed/stat-tracker-gui")),
            None
        );
    }

    #[test]
    fn missing_curl_blocks_in_app_update() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan = evaluate_plan_with(
            "0.4.7",
            UpdateProbes {
                has_tool: &|n| n != "curl",
                prefix: dir.path().to_path_buf(),
                os: "linux",
                arch: "x86_64",
                check_write: false,
            },
        );
        match plan {
            UpdatePlan::Blocked { reason, hint } => {
                assert!(reason.contains("curl"), "{reason}");
                assert!(
                    hint.contains("terminal") || hint.contains("Update now"),
                    "{hint}"
                );
            }
            other => panic!("expected blocked, got {other:?}"),
        }
    }

    #[test]
    fn unwritable_prefix_blocks_with_copy_hint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let not_dir = dir.path().join("not-a-dir");
        std::fs::write(&not_dir, b"x").unwrap();
        let plan = evaluate_plan_with(
            "0.4.7",
            UpdateProbes {
                has_tool: &|_| true,
                prefix: not_dir,
                os: "linux",
                arch: "x86_64",
                check_write: true,
            },
        );
        match plan {
            UpdatePlan::Blocked { reason, hint } => {
                assert!(
                    reason.contains("not writable") || reason.contains("Cannot write"),
                    "{reason}"
                );
                assert!(hint.contains("Copy"), "{hint}");
            }
            other => panic!("expected blocked, got {other:?}"),
        }
    }

    #[test]
    fn ready_when_linux_x86_tools_and_writable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan = evaluate_plan_with(
            "0.4.7",
            UpdateProbes {
                has_tool: &|_| true,
                prefix: dir.path().to_path_buf(),
                os: "linux",
                arch: "x86_64",
                check_write: true,
            },
        );
        match plan {
            UpdatePlan::Ready { tag, prefix } => {
                assert_eq!(tag, "stat-tracker-v0.4.7");
                assert_eq!(prefix, dir.path());
            }
            other => panic!("expected ready, got {other:?}"),
        }
    }

    #[test]
    fn non_linux_is_blocked() {
        let plan = evaluate_plan_with(
            "0.4.7",
            UpdateProbes {
                has_tool: &|_| true,
                prefix: PathBuf::from("/tmp"),
                os: "macos",
                arch: "x86_64",
                check_write: false,
            },
        );
        assert!(!plan.can_run());
        match plan {
            UpdatePlan::Blocked { reason, .. } => {
                assert!(reason.contains("Linux-only"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn installer_env_strips_the_bootstrap_fetch_hook() {
        let mut cmd = std::process::Command::new("bash");
        cmd.arg("-c");
        cmd.arg(format!(
            "printf '%s|%s' \"${{{}-unset}}\" \"${{STAT_TRACKER_BOOTSTRAP_PINNED-}}\"",
            BOOTSTRAP_FETCH_HOOK_ENV
        ));
        cmd.env(BOOTSTRAP_FETCH_HOOK_ENV, "/tmp/not-a-real-fetch");
        apply_installer_env(&mut cmd, "0.4.15", Path::new("/tmp/prefix"));
        let out = cmd.output().expect("bash");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "unset|1",
            "the fetch hook must not reach the installer; the pin flag must"
        );
    }

    #[test]
    fn strip_ansi_drops_bootstrap_colors() {
        assert_eq!(
            strip_ansi("\u{1b}[0;32m[bootstrap]\u{1b}[0m Done."),
            "[bootstrap] Done."
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }

    struct FakeIo {
        pauses: std::sync::Mutex<u32>,
        game_dir: Option<PathBuf>,
        verbs: std::sync::Mutex<Vec<DaemonVerb>>,
        urls: std::sync::Mutex<Vec<String>>,
        install_result: Result<String, String>,
        /// The installer replaces the binaries, then never returns.
        install_hangs: bool,
        start_result: Result<String, String>,
        timeout: Duration,
    }

    impl FakeIo {
        fn new(install_result: Result<String, String>) -> Self {
            Self {
                pauses: Default::default(),
                game_dir: None,
                verbs: std::sync::Mutex::new(Vec::new()),
                urls: std::sync::Mutex::new(Vec::new()),
                install_result,
                install_hangs: false,
                start_result: Ok("Tracker started".into()),
                timeout: INSTALL_TIMEOUT,
            }
        }
    }

    impl UpdateIo for FakeIo {
        async fn control_daemon(
            &self,
            _data_dir: &Path,
            verb: DaemonVerb,
            _service_installed: bool,
        ) -> Result<String, String> {
            self.verbs.lock().expect("verbs").push(verb);
            match verb {
                DaemonVerb::Start => self.start_result.clone(),
                _ => Ok("ok".into()),
            }
        }

        async fn pause(&self, _d: std::time::Duration) {
            // The game "closes" after three checks.
            let mut n = self.pauses.lock().expect("pauses");
            *n += 1;
            if *n >= 3
                && let Some(dir) = self.game_dir.as_ref()
            {
                let _ = std::fs::remove_file(dir.join("active_game.json"));
            }
        }

        async fn install(
            &self,
            url: &str,
            _script: &Path,
            _latest: &str,
            prefix: &Path,
            _tag: &str,
            _log: &Path,
        ) -> Result<String, String> {
            self.urls.lock().expect("urls").push(url.to_string());
            // The installer got as far as replacing the binaries.
            for name in ROLLBACK_BINARIES {
                let live = prefix.join("bin").join(name);
                if live.is_file() {
                    std::fs::write(&live, b"new").unwrap();
                }
            }
            if self.install_hangs {
                std::future::pending::<()>().await;
            }
            self.install_result.clone()
        }

        fn install_timeout(&self) -> Duration {
            self.timeout
        }
    }

    fn ready_plan(prefix: &Path) -> UpdatePlan {
        UpdatePlan::Ready {
            prefix: prefix.to_path_buf(),
            tag: release_tag("0.4.15"),
        }
    }

    fn run_request(stop_daemon: bool) -> UpdateRunRequest {
        UpdateRunRequest {
            latest: "0.4.15".into(),
            data_dir: PathBuf::from("/tmp/sst-update-test"),
            stop_daemon,
            service_installed: false,
        }
    }

    #[test]
    fn a_left_over_game_file_does_not_hold_the_update() {
        let data = tempfile::tempdir().expect("tempdir");
        let path = data.path().join("active_game.json");
        std::fs::write(&path, "{}").unwrap();
        assert!(game_in_progress(data.path()));
        let old =
            std::time::SystemTime::now() - GAME_FILE_STALE - std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(!game_in_progress(data.path()));
    }

    #[tokio::test]
    async fn the_restart_waits_until_the_open_game_is_saved() {
        let data = tempfile::tempdir().expect("tempdir");
        std::fs::write(data.path().join("active_game.json"), "{}").unwrap();
        let prefix = tempfile::tempdir().expect("tempdir");
        let io = FakeIo {
            game_dir: Some(data.path().to_path_buf()),
            ..FakeIo::new(Ok("Installed.".into()))
        };
        let mut req = run_request(true);
        req.data_dir = data.path().to_path_buf();
        run_planned_update(&req, ready_plan(prefix.path()), &io)
            .await
            .expect("installed");
        // Three checks while the game was open, then stop, install, start.
        assert_eq!(*io.pauses.lock().unwrap(), 3);
        assert_eq!(
            io.verbs.lock().expect("verbs").as_slice(),
            &[DaemonVerb::Stop, DaemonVerb::Start]
        );
        assert!(!game_in_progress(data.path()));
    }

    #[tokio::test]
    async fn failed_install_stops_and_does_not_start() {
        let dir = tempfile::tempdir().expect("tempdir");
        let io = FakeIo::new(Err("disk full".into()));
        let err = run_planned_update(&run_request(true), ready_plan(dir.path()), &io)
            .await
            .expect_err("install failed");
        assert!(err.contains("disk full"), "{err}");
        assert!(
            err.contains("left stopped"),
            "failed install must not claim the service was restarted: {err}"
        );
        assert_eq!(
            io.verbs.lock().expect("verbs").as_slice(),
            &[DaemonVerb::Stop]
        );
        let urls = io.urls.lock().expect("urls").clone();
        assert_eq!(urls, vec![bootstrap_raw_url("stat-tracker-v0.4.15")]);
        assert!(!urls[0].contains("/main/"), "{}", urls[0]);
    }

    #[tokio::test]
    async fn successful_install_restarts_from_the_tag_url() {
        let dir = tempfile::tempdir().expect("tempdir");
        let io = FakeIo::new(Ok("installed".into()));
        let msg = run_planned_update(&run_request(true), ready_plan(dir.path()), &io)
            .await
            .expect("install ok");
        assert_eq!(msg, "installed");
        assert_eq!(
            io.verbs.lock().expect("verbs").as_slice(),
            &[DaemonVerb::Stop, DaemonVerb::Start]
        );
    }

    #[tokio::test]
    async fn install_without_a_running_daemon_does_not_start_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let io = FakeIo::new(Err("nope".into()));
        let err = run_planned_update(&run_request(false), ready_plan(dir.path()), &io)
            .await
            .expect_err("install failed");
        assert!(!err.contains("left stopped"), "{err}");
        assert!(io.verbs.lock().expect("verbs").is_empty());
    }

    fn prefix_with_binaries() -> tempfile::TempDir {
        let prefix = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(prefix.path().join("bin")).unwrap();
        for name in ROLLBACK_BINARIES {
            std::fs::write(prefix.path().join("bin").join(name), b"old").unwrap();
        }
        prefix
    }

    #[tokio::test]
    async fn a_failed_update_puts_the_previous_version_back_and_restarts() {
        let prefix = prefix_with_binaries();
        let io = FakeIo::new(Err("tessdata download failed".into()));
        let err = run_planned_update(&run_request(true), ready_plan(prefix.path()), &io)
            .await
            .expect_err("install failed");
        assert!(err.contains("tessdata download failed"), "{err}");
        assert!(err.contains("previous version was put back"), "{err}");
        assert!(err.contains("tracker was started again"), "{err}");
        assert!(!err.contains("left stopped"), "{err}");
        for name in ROLLBACK_BINARIES {
            assert_eq!(
                std::fs::read(prefix.path().join("bin").join(name)).unwrap(),
                b"old",
                "{name} restored"
            );
        }
        assert_eq!(
            io.verbs.lock().expect("verbs").as_slice(),
            &[DaemonVerb::Stop, DaemonVerb::Start],
            "the tracker is not left stopped"
        );
        let leftovers: Vec<_> = std::fs::read_dir(prefix.path().join("bin"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), ROLLBACK_BINARIES.len(), "{leftovers:?}");
    }

    #[tokio::test]
    async fn a_successful_update_keeps_the_new_binaries() {
        let prefix = prefix_with_binaries();
        let io = FakeIo::new(Ok("installed".into()));
        run_planned_update(&run_request(true), ready_plan(prefix.path()), &io)
            .await
            .expect("install ok");
        for name in ROLLBACK_BINARIES {
            assert_eq!(
                std::fs::read(prefix.path().join("bin").join(name)).unwrap(),
                b"new"
            );
        }
    }

    /// GUI-1: a stalled installer used to leave the tracker stopped and the
    /// banner on "updating" forever. It is stopped at the timeout, the old
    /// binaries go back and the tracker is started again.
    #[tokio::test]
    async fn a_hanging_installer_times_out_restores_and_restarts() {
        let prefix = prefix_with_binaries();
        let io = FakeIo {
            install_hangs: true,
            timeout: Duration::from_millis(50),
            ..FakeIo::new(Ok("never".into()))
        };
        let err = tokio::time::timeout(
            Duration::from_secs(10),
            run_planned_update(&run_request(true), ready_plan(prefix.path()), &io),
        )
        .await
        .expect("the update must not hang with the installer")
        .expect_err("a timed-out install is a failure");
        assert!(err.contains("did not finish"), "{err}");
        assert!(err.contains("previous version was put back"), "{err}");
        assert!(err.contains("tracker was started again"), "{err}");
        for name in ROLLBACK_BINARIES {
            assert_eq!(
                std::fs::read(prefix.path().join("bin").join(name)).unwrap(),
                b"old",
                "{name} restored after the timeout"
            );
        }
        assert_eq!(
            io.verbs.lock().expect("verbs").as_slice(),
            &[DaemonVerb::Stop, DaemonVerb::Start],
            "the tracker is not left stopped"
        );
    }

    #[test]
    fn install_timeout_message_names_the_limit() {
        assert_eq!(
            install_timeout_message(INSTALL_TIMEOUT),
            "The installer did not finish within 10 minutes, so it was stopped."
        );
        assert_eq!(
            install_timeout_message(Duration::from_secs(45)),
            "The installer did not finish within 45 seconds, so it was stopped."
        );
    }

    /// The Start result used to be dropped, so a tracker that did not come
    /// back after an update looked like a clean update.
    #[tokio::test]
    async fn a_failed_start_after_a_good_update_is_reported() {
        let prefix = prefix_with_binaries();
        let io = FakeIo {
            start_result: Err("unit failed".into()),
            ..FakeIo::new(Ok("Installed stat-tracker-v0.4.15.".into()))
        };
        let msg = run_planned_update(&run_request(true), ready_plan(prefix.path()), &io)
            .await
            .expect("the install itself worked");
        assert!(msg.starts_with("Installed stat-tracker-v0.4.15."), "{msg}");
        assert!(
            msg.contains("Starting the tracker again failed (unit failed)"),
            "{msg}"
        );
    }

    #[tokio::test]
    async fn a_failed_start_after_a_rollback_is_reported() {
        let prefix = prefix_with_binaries();
        let io = FakeIo {
            start_result: Err("unit failed".into()),
            ..FakeIo::new(Err("Installer failed".into()))
        };
        let err = run_planned_update(&run_request(true), ready_plan(prefix.path()), &io)
            .await
            .expect_err("install failed");
        assert!(err.contains("previous version was put back"), "{err}");
        assert!(!err.contains("tracker was started again"), "{err}");
        assert!(
            err.contains("Starting the tracker again failed (unit failed)"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn installer_output_goes_to_the_log_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join(INSTALL_LOG);
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c").arg("echo to-stdout; echo to-stderr >&2");
        let status = run_logged(cmd, &log).await.expect("ran");
        assert!(status.success());
        let text = std::fs::read_to_string(&log).expect("log");
        assert!(text.contains("to-stdout"), "{text}");
        assert!(text.contains("to-stderr"), "{text}");
    }

    fn process_gone(pid: u32) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(_) => true,
            // `pid (comm) Z ...`: dead, waiting to be reaped.
            Ok(stat) => stat
                .rsplit_once(") ")
                .is_some_and(|(_, rest)| rest.starts_with('Z')),
        }
    }

    /// The timeout drops the install future. That must kill the installer's
    /// children too (curl), not only bash.
    #[tokio::test]
    async fn dropping_the_installer_kills_its_children() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join(INSTALL_LOG);
        let pidfile = dir.path().join("child.pid");
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c").arg(format!(
            "sleep 60 & echo $! > '{}'; wait",
            pidfile.display()
        ));
        // Boxed so `drop` below really drops the future (a stack pin would
        // only drop the reference).
        let mut run = Box::pin(run_logged(cmd, &log));
        let mut child_pid = None;
        for _ in 0..200 {
            tokio::select! {
                _ = &mut run => panic!("the installer exited early"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            if let Some(pid) = std::fs::read_to_string(&pidfile)
                .ok()
                .and_then(|t| t.trim().parse::<u32>().ok())
            {
                child_pid = Some(pid);
                break;
            }
        }
        let pid = child_pid.expect("the child wrote its pid");
        assert!(!process_gone(pid), "the child runs before the drop");
        drop(run);
        let mut gone = false;
        for _ in 0..300 {
            if process_gone(pid) {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(gone, "child {pid} outlived the dropped installer");
    }

    #[test]
    fn update_recheck_is_due_every_six_hours() {
        let start = std::time::Instant::now();
        let later = |secs: u64| start + Duration::from_secs(secs);
        let idle = UpdateProgress::Idle;
        assert!(!update_recheck_due(true, &idle, Some(start), later(60)));
        assert!(!update_recheck_due(
            true,
            &idle,
            Some(start),
            later(6 * 3600 - 1)
        ));
        assert!(update_recheck_due(
            true,
            &idle,
            Some(start),
            later(6 * 3600)
        ));
        assert!(update_recheck_due(
            true,
            &UpdateProgress::Failed("x".into()),
            Some(start),
            later(7 * 3600)
        ));
        assert!(
            !update_recheck_due(true, &UpdateProgress::Running, Some(start), later(7 * 3600)),
            "not mid-update"
        );
        assert!(
            !update_recheck_due(false, &idle, Some(start), later(7 * 3600)),
            "not in fixture mode"
        );
        assert!(
            !update_recheck_due(true, &idle, None, later(7 * 3600)),
            "the start check comes first"
        );
    }

    #[test]
    fn recheck_applies_only_to_a_different_version() {
        let info = |v: &str| UpdateInfo {
            latest: v.into(),
            current: "0.5.0-rc.5".into(),
            url: String::new(),
            release_bodies: Vec::new(),
        };
        assert!(!recheck_applies(Some(&info("0.5.0-rc.6")), None));
        assert!(!recheck_applies(None, None));
        assert!(!recheck_applies(
            Some(&info("0.5.0-rc.6")),
            Some(&info("0.5.0-rc.6"))
        ));
        assert!(recheck_applies(
            Some(&info("0.5.0-rc.6")),
            Some(&info("0.5.0-rc.7"))
        ));
        assert!(recheck_applies(None, Some(&info("0.5.0-rc.6"))));
    }

    #[test]
    fn restore_binaries_replaces_the_live_files() {
        let prefix = prefix_with_binaries();
        let backup = tempfile::tempdir().expect("tempdir");
        let saved = backup_binaries(prefix.path(), backup.path()).unwrap();
        assert_eq!(saved, ROLLBACK_BINARIES.to_vec());
        std::fs::write(prefix.path().join("bin/scuffed-stat-tracker"), b"broken").unwrap();
        restore_binaries(prefix.path(), backup.path(), &saved).unwrap();
        assert_eq!(
            std::fs::read(prefix.path().join("bin/scuffed-stat-tracker")).unwrap(),
            b"old"
        );
    }
}
