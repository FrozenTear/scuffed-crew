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
    pub url: String,
}

pub fn current_version() -> &'static str {
    option_env!("SST_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
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
    let cur = parse_semver(current_version())?;
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
                current_version()
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
}
