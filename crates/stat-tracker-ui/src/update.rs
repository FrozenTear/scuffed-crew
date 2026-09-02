//! "Update available" banner — detect + notify only, same as the Dioxus GUI.
//!
//! Never downloads or executes an installer. The user re-runs the published
//! one-liner or opens the release page.

use std::time::Duration;

use iced::widget::{button, column, container, text};
use iced::{Element, Fill, Padding};

use crate::app::Message;
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, PAD_INNER, SIZE_BODY, SIZE_META, SIZE_TITLE, TEXT,
    TEXT_2,
};

const REPO: &str = "FrozenTear/scuffed-crew";

/// Installer one-liner surfaced in the banner (matches the website).
pub const UPDATE_CMD: &str = "curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub latest: String,
    pub current: String,
    pub url: String,
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

pub fn banner(info: &UpdateInfo) -> Element<'static, Message> {
    let url = info.url.clone();
    container(
        column![
            text(format!("Update available — v{}", info.latest))
                .size(SIZE_TITLE)
                .font(FONT_BOLD)
                .color(TEXT),
            text(format!(
                "You're on v{}. Update by re-running the installer:",
                info.current
            ))
            .size(SIZE_BODY)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
            text(UPDATE_CMD.to_string())
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::ACCENT),
            button(
                text("View release notes")
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(TEXT),
            )
            .padding(Padding::from([8, 16]))
            .style(theme::chip(true))
            .on_press(Message::OpenUpdate(url)),
        ]
        .spacing(10),
    )
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
}
