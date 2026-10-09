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

const REQUIRED_TOOLS: &[&str] = &["curl", "tar", "bash", "mktemp"];

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

/// Stop before install. Start again only when install succeeded, so a failed
/// or partial install is not launched.
pub fn should_restart_daemon(stop_daemon: bool, install_ok: bool) -> bool {
    stop_daemon && install_ok
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
            hint: "The Copy button still works — paste the command into a terminal that can write the prefix.".into(),
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
    select_newer_release_on(installed_update_channel(), &current, &releases)
}

fn installed_update_channel() -> UpdateChannel {
    stat_tracker::config::Config::read_stored()
        .ok()
        .filter(|config| config.wants_prerelease_updates())
        .map(|_| UpdateChannel::Prerelease)
        .unwrap_or(UpdateChannel::Stable)
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

trait UpdateIo {
    async fn control_daemon(&self, data_dir: &Path, verb: DaemonVerb, service_installed: bool);
    async fn install(
        &self,
        url: &str,
        script: &Path,
        latest: &str,
        prefix: &Path,
        tag: &str,
    ) -> Result<String, String>;
}

struct LiveUpdateIo;

impl UpdateIo for LiveUpdateIo {
    async fn control_daemon(&self, data_dir: &Path, verb: DaemonVerb, service_installed: bool) {
        let _ = crate::daemon::run_verb(data_dir.to_path_buf(), verb, service_installed).await;
    }

    async fn install(
        &self,
        url: &str,
        script: &Path,
        latest: &str,
        prefix: &Path,
        tag: &str,
    ) -> Result<String, String> {
        download_and_run_bootstrap(url, script, latest, prefix, tag).await
    }
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
        io.control_daemon(&req.data_dir, DaemonVerb::Stop, req.service_installed)
            .await;
    }

    let work = std::env::temp_dir().join(format!(
        "sst-gui-update-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    if let Err(e) = std::fs::create_dir_all(&work) {
        return note_daemon_left_stopped(
            req.stop_daemon,
            Err(format!("Could not create a temp dir for the update: {e}")),
        );
    }
    let script = work.join("bootstrap.sh");
    let url = bootstrap_raw_url(&tag);
    let install = io.install(&url, &script, &req.latest, &prefix, &tag).await;
    let _ = std::fs::remove_dir_all(&work);
    if should_restart_daemon(req.stop_daemon, install.is_ok()) {
        io.control_daemon(&req.data_dir, DaemonVerb::Start, req.service_installed)
            .await;
    }
    note_daemon_left_stopped(req.stop_daemon, install)
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
    let out = cmd
        .output()
        .await
        .map_err(|e| format!("Could not run bootstrap.sh: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stderr}\n{stdout}");
    if !out.status.success() {
        return Err(format!(
            "Installer failed ({tag}): {}. Copy the command and run it in a terminal if this keeps failing.",
            tail_useful(&combined)
        ));
    }
    Ok(format!(
        "Installed {tag}. Restart this window to run the new GUI. {}",
        tail_useful(&combined)
    ))
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

pub fn banner<'a>(
    info: &UpdateInfo,
    progress: &UpdateProgress,
    plan: &UpdatePlan,
    notes: &'a [crate::notes::ShownRelease],
    notes_ui: crate::notes::NotesUi<'a>,
    notes_expanded: bool,
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
                text(msg.clone())
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

    let mut update_btn = button(
        text(if running {
            "Installing…"
        } else {
            "Update now"
        })
        .size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .color(TEXT),
    )
    .padding(Padding::from([8, 16]))
    .style(theme::chip(can_run));
    if can_run {
        update_btn = update_btn.on_press(Message::RunUpdate);
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
    fn failed_install_does_not_restart() {
        assert!(!should_restart_daemon(true, false));
        assert!(should_restart_daemon(true, true));
        assert!(!should_restart_daemon(false, true));
        assert!(!should_restart_daemon(false, false));
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
        verbs: std::sync::Mutex<Vec<DaemonVerb>>,
        urls: std::sync::Mutex<Vec<String>>,
        install_result: Result<String, String>,
    }

    impl UpdateIo for FakeIo {
        async fn control_daemon(
            &self,
            _data_dir: &Path,
            verb: DaemonVerb,
            _service_installed: bool,
        ) {
            self.verbs.lock().expect("verbs").push(verb);
        }

        async fn install(
            &self,
            url: &str,
            _script: &Path,
            _latest: &str,
            _prefix: &Path,
            _tag: &str,
        ) -> Result<String, String> {
            self.urls.lock().expect("urls").push(url.to_string());
            self.install_result.clone()
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

    #[tokio::test]
    async fn failed_install_stops_and_does_not_start() {
        let dir = tempfile::tempdir().expect("tempdir");
        let io = FakeIo {
            verbs: std::sync::Mutex::new(Vec::new()),
            urls: std::sync::Mutex::new(Vec::new()),
            install_result: Err("disk full".into()),
        };
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
        let io = FakeIo {
            verbs: std::sync::Mutex::new(Vec::new()),
            urls: std::sync::Mutex::new(Vec::new()),
            install_result: Ok("installed".into()),
        };
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
        let io = FakeIo {
            verbs: std::sync::Mutex::new(Vec::new()),
            urls: std::sync::Mutex::new(Vec::new()),
            install_result: Err("nope".into()),
        };
        let err = run_planned_update(&run_request(false), ready_plan(dir.path()), &io)
            .await
            .expect_err("install failed");
        assert!(!err.contains("left stopped"), "{err}");
        assert!(io.verbs.lock().expect("verbs").is_empty());
    }
}
