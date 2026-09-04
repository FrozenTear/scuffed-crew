//! Update banner: version check, in-app install, and Copy for the bootstrap command.
//!
//! Check still uses GitHub Releases (`stat-tracker-v*`). "Update now" downloads
//! `bootstrap.sh` and runs it with `STAT_TRACKER_TAG` pinned to the advertised
//! release. If the prefix is not writable or tools are missing, the banner
//! says so instead of offering a dead button.

use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill, Padding};

use crate::app::Message;
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, PAD_INNER, SIZE_BODY, SIZE_META, SIZE_TITLE, TEXT,
    TEXT_2, TEXT_3,
};

const REPO: &str = "FrozenTear/scuffed-crew";

/// Installer one-liner surfaced in the banner (matches the website).
pub const UPDATE_CMD: &str = "curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash";

pub const BOOTSTRAP_URL: &str = "https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh";

const REQUIRED_TOOLS: &[&str] = &["curl", "tar", "bash", "mktemp"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub latest: String,
    pub current: String,
    pub url: String,
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

/// Parse `MAJOR.MINOR.PATCH`; a leading `v` and any `-pre`/`+build` suffix
/// are stripped. `None` on anything unparseable.
pub fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let core = s.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next().unwrap_or(core);
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

pub fn release_tag(latest: &str) -> String {
    let core = latest.trim().trim_start_matches('v');
    format!("stat-tracker-v{core}")
}

/// Command shown in the banner / copied to the clipboard. Pins the advertised tag.
pub fn pinned_install_command(latest: &str) -> String {
    format!("STAT_TRACKER_TAG={} {}", release_tag(latest), UPDATE_CMD)
}

pub fn bootstrap_env(latest: &str, prefix: &Path) -> Vec<(&'static str, String)> {
    vec![
        ("STAT_TRACKER_TAG", release_tag(latest)),
        ("STAT_TRACKER_CHANNEL", "stable".into()),
        ("STAT_TRACKER_PREFIX", prefix.display().to_string()),
    ]
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
pub async fn check_for_update() -> Option<UpdateInfo> {
    let current = current_version()?;
    let cur = parse_semver(&current)?;
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=20");
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

    let mut best: Option<((u32, u32, u32), String, String)> = None;
    for r in releases {
        if r["draft"].as_bool().unwrap_or(false) || r["prerelease"].as_bool().unwrap_or(false) {
            continue;
        }
        let Some(ver_str) = r["tag_name"]
            .as_str()
            .and_then(|t| t.strip_prefix("stat-tracker-v"))
        else {
            continue;
        };
        let Some(ver) = parse_semver(ver_str) else {
            continue;
        };
        if best.as_ref().is_none_or(|(b, _, _)| ver > *b) {
            let html = r["html_url"].as_str().unwrap_or_default().to_string();
            best = Some((ver, ver_str.to_string(), html));
        }
    }

    let (latest_ver, latest_str, html_url) = best?;
    (latest_ver > cur).then_some(UpdateInfo {
        latest: latest_str,
        current,
        url: html_url,
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

pub async fn run_in_app_update(req: UpdateRunRequest) -> Result<String, String> {
    let plan = evaluate_plan(&req.latest);
    let UpdatePlan::Ready { prefix, tag } = plan else {
        let UpdatePlan::Blocked { reason, hint } = plan else {
            unreachable!("evaluate_plan is Ready or Blocked");
        };
        return Err(format!("{reason} {hint}"));
    };

    if req.stop_daemon {
        let _ = crate::daemon::run_verb(
            req.data_dir.clone(),
            crate::daemon::DaemonVerb::Stop,
            req.service_installed,
        )
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
    std::fs::create_dir_all(&work)
        .map_err(|e| format!("Could not create a temp dir for the update: {e}"))?;
    let script = work.join("bootstrap.sh");

    let install = download_and_run_bootstrap(&script, &req.latest, &prefix, &tag).await;
    let _ = std::fs::remove_dir_all(&work);
    if req.stop_daemon {
        let _ = crate::daemon::run_verb(
            req.data_dir,
            crate::daemon::DaemonVerb::Start,
            req.service_installed,
        )
        .await;
    }
    install
}

async fn download_and_run_bootstrap(
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
        .get(BOOTSTRAP_URL)
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
    for (key, value) in bootstrap_env(latest, prefix) {
        cmd.env(key, value);
    }
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

pub fn banner(
    info: &UpdateInfo,
    progress: &UpdateProgress,
    plan: &UpdatePlan,
) -> Element<'static, Message> {
    let url = info.url.clone();
    let cmd = pinned_install_command(&info.latest);
    let running = matches!(progress, UpdateProgress::Running);
    let can_run = plan.can_run() && !running && !matches!(progress, UpdateProgress::Succeeded(_));

    let mut body = column![
        text(format!("Update available — v{}", info.latest))
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
        text(format!(
            "You're on v{}. Update now downloads the release and runs the installer, or copy the command and run it in a terminal.",
            info.current
        ))
        .size(SIZE_BODY)
        .font(FONT_MEDIUM)
        .color(TEXT_2),
    ]
    .spacing(10);

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
            text("Release notes")
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        )
        .padding(Padding::from([8, 16]))
        .style(theme::ghost_btn())
        .on_press(Message::OpenUpdate(url)),
    ]
    .spacing(8);

    body = body.push(actions);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn semver_parses_and_orders() {
        assert_eq!(parse_semver("0.2.1"), Some((0, 2, 1)));
        assert_eq!(parse_semver("v1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_semver("0.2"), Some((0, 2, 0)));
        assert_eq!(parse_semver("0.3.0-rc1"), Some((0, 3, 0)));
        assert!(parse_semver("garbage").is_none());
        assert!(parse_semver("0.2.1").unwrap() > parse_semver("0.1.0").unwrap());
        assert!(parse_semver("0.2.0").unwrap() > parse_semver("0.1.9").unwrap());
        assert!(is_newer("0.3.0", "0.2.1"));
        assert!(!is_newer("0.2.1", "0.2.1"));
        assert!(!is_newer("nope", "0.1.0"));
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
    fn release_tag_and_pinned_command_use_stat_tracker_prefix() {
        assert_eq!(release_tag("0.4.7"), "stat-tracker-v0.4.7");
        assert_eq!(release_tag("v0.4.7"), "stat-tracker-v0.4.7");
        let cmd = pinned_install_command("0.4.7");
        assert!(
            cmd.starts_with("STAT_TRACKER_TAG=stat-tracker-v0.4.7 "),
            "{cmd}"
        );
        assert!(cmd.contains(UPDATE_CMD), "{cmd}");
        assert!(cmd.contains("bootstrap.sh"), "{cmd}");
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
    fn strip_ansi_drops_bootstrap_colors() {
        assert_eq!(
            strip_ansi("\u{1b}[0;32m[bootstrap]\u{1b}[0m Done."),
            "[bootstrap] Done."
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
