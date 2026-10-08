//! Release notes for the desktop app.
//!
//! The player-facing text is `crates/stat-tracker/CHANGELOG.md`, compiled
//! into this binary so it is available offline. Iced 0.14 already renders
//! Markdown (`iced::widget::markdown`, feature `markdown`, which uses
//! `pulldown-cmark`). Headings, paragraphs, lists, inline code, and links
//! come from that widget. No separate Markdown crate is added.
//!
//! `### Install` blocks stay in the changelog for the GitHub release page.
//! This view drops them. The first paragraph of each release is the player
//! summary and is drawn ahead of the rest.

use iced::widget::markdown::{self, Item};
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Font, Length, Padding};

use crate::app::Message;
use crate::theme::{
    self, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_FEATURED,
    SIZE_META, SIZE_TITLE, TEXT, TEXT_2,
};

/// Changelog shipped inside the GUI binary.
pub const BUNDLED_CHANGELOG: &str = include_str!("../../stat-tracker/CHANGELOG.md");

/// One release, after Install blocks are removed and the summary is split out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseNotes {
    pub version: String,
    /// First paragraph. Plain language, shown ahead of `body_markdown`.
    pub summary: String,
    /// Remaining Markdown (headings, paragraphs, lists, code, links).
    pub body_markdown: String,
}

/// Notes ready to draw. Markdown is parsed once, not on every frame.
pub struct ShownRelease {
    pub version: String,
    pub summary: String,
    pub items: Vec<Item>,
}

/// How the installed version compares with the version stored in `ui_state.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeenChange {
    /// Missing installed version, or it matches the stored one.
    Unchanged,
    /// No stored version yet.
    FirstRun,
    /// Installed version is newer than the stored one.
    Upgrade,
    /// Installed version is older than the stored one.
    Downgrade,
}

pub fn classify_seen(last_seen: Option<&str>, installed: Option<&str>) -> SeenChange {
    let Some(installed_key) = installed.and_then(version_key) else {
        return SeenChange::Unchanged;
    };
    let Some(seen_key) = last_seen.and_then(version_key) else {
        return SeenChange::FirstRun;
    };
    match installed_key.cmp(&seen_key) {
        std::cmp::Ordering::Equal => SeenChange::Unchanged,
        std::cmp::Ordering::Greater => SeenChange::Upgrade,
        std::cmp::Ordering::Less => SeenChange::Downgrade,
    }
}

pub fn launch_subtitle(change: SeenChange) -> &'static str {
    match change {
        SeenChange::Upgrade => "What's new since the last version you opened.",
        SeenChange::FirstRun | SeenChange::Downgrade | SeenChange::Unchanged => {
            "Notes for the version installed now."
        }
    }
}

/// Notes to open on launch. Empty when there is nothing new to show.
pub fn notes_for_change(
    changelog: &str,
    change: SeenChange,
    last_seen: Option<&str>,
    installed: &str,
) -> Vec<ReleaseNotes> {
    match change {
        SeenChange::Unchanged => Vec::new(),
        SeenChange::FirstRun | SeenChange::Downgrade => {
            section_for(changelog, installed).into_iter().collect()
        }
        SeenChange::Upgrade => {
            let Some(seen) = last_seen else {
                return section_for(changelog, installed).into_iter().collect();
            };
            sections_between(changelog, seen, installed)
        }
    }
}

/// The section whose heading is `version`, if the changelog has one.
pub fn section_for(changelog: &str, version: &str) -> Option<ReleaseNotes> {
    let want = version_key(version)?;
    raw_sections(changelog).into_iter().find_map(|(ver, body)| {
        (version_key(&ver) == Some(want)).then(|| player_from_section(&ver, &body))
    })
}

/// Versions strictly after `after` and up through `through`, newest first.
pub fn sections_between(changelog: &str, after: &str, through: &str) -> Vec<ReleaseNotes> {
    let Some(after) = version_key(after) else {
        return Vec::new();
    };
    let Some(through) = version_key(through) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (ver, body) in raw_sections(changelog) {
        let Some(key) = version_key(&ver) else {
            continue;
        };
        if key > after && key <= through {
            rows.push((key, player_from_section(&ver, &body)));
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    rows.into_iter().map(|(_, notes)| notes).collect()
}

/// Every bundled release, newest first, with Install blocks removed.
pub fn all_notes(changelog: &str) -> Vec<ReleaseNotes> {
    let mut rows = Vec::new();
    for (ver, body) in raw_sections(changelog) {
        if let Some(key) = version_key(&ver) {
            rows.push((key, player_from_section(&ver, &body)));
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    rows.into_iter().map(|(_, notes)| notes).collect()
}

/// Notes for an offered update. `remote` is `(version, GitHub release body)`,
/// newest or not. A non-empty cleaned body wins. The bundled section is the
/// fallback when GitHub has no usable text for that version.
pub fn notes_for_update(
    bundled: &str,
    current: &str,
    latest: &str,
    remote: &[(&str, &str)],
) -> Vec<ReleaseNotes> {
    let Some(latest_key) = version_key(latest) else {
        return Vec::new();
    };
    let Some(current_key) = version_key(current) else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    for (ver, _) in raw_sections(bundled) {
        if let Some(key) = version_key(&ver)
            && key > current_key
            && key <= latest_key
            && !keys.contains(&key)
        {
            keys.push(key);
        }
    }
    for (ver, _) in remote {
        if let Some(key) = version_key(ver)
            && key > current_key
            && key <= latest_key
            && !keys.contains(&key)
        {
            keys.push(key);
        }
    }
    keys.sort_by_key(|key| std::cmp::Reverse(*key));
    keys.into_iter()
        .filter_map(|key| notes_for_key(bundled, key, remote))
        .collect()
}

pub fn render(notes: Vec<ReleaseNotes>) -> Vec<ShownRelease> {
    notes
        .into_iter()
        .map(|notes| ShownRelease {
            items: markdown::parse(&notes.body_markdown).collect(),
            version: notes.version,
            summary: notes.summary,
        })
        .collect()
}

pub fn notes_column(sections: &[ShownRelease]) -> Element<'_, Message> {
    let mut col = column![].spacing(18).width(Fill);
    for section in sections {
        col = col.push(release_block(section));
    }
    col.into()
}

pub fn dialog<'a>(subtitle: &'a str, sections: &'a [ShownRelease]) -> Element<'a, Message> {
    let header = row![
        column![
            text("What's new")
                .size(SIZE_FEATURED)
                .font(FONT_EXTRABOLD)
                .color(TEXT),
            text(subtitle.to_string())
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        ]
        .spacing(4)
        .width(Fill),
        button(
            text("Close")
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        )
        .padding(Padding::from([8, 16]))
        .style(theme::chip(true))
        .on_press(Message::DismissNotes),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let card = container(
        column![
            header,
            scrollable(
                container(notes_column(sections))
                    .padding(Padding {
                        right: 8.0,
                        ..Padding::ZERO
                    })
                    .width(Fill),
            )
            .height(Fill)
            .width(Fill),
        ]
        .spacing(16)
        .height(Fill),
    )
    .padding(24)
    .width(Length::Fixed(720.0))
    .height(Fill)
    .style(theme::surface_panel);

    let mut backdrop = theme::BG;
    backdrop.a = 0.88;

    container(card)
        .padding(28)
        .center(Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(backdrop)),
            ..container::Style::default()
        })
        .into()
}

fn notes_for_key(
    bundled: &str,
    key: (u32, u32, u32),
    remote: &[(&str, &str)],
) -> Option<ReleaseNotes> {
    if let Some((_, body)) = remote.iter().find(|(ver, _)| version_key(ver) == Some(key)) {
        let from_remote = from_github_body(&display_version(key), body);
        if usable(&from_remote) {
            return Some(from_remote);
        }
    }
    section_for(bundled, &display_version(key))
}

fn from_github_body(version: &str, body: &str) -> ReleaseNotes {
    let stripped = hide_sections(body, |_level, title| {
        title.eq_ignore_ascii_case("install")
            || title.starts_with("Commits since")
            || title.starts_with("Requirements")
    });
    let stripped =
        drop_leading_paragraph_if(&stripped, |summary| summary.starts_with("Prebuilt Linux"));
    let (summary, rest) = split_summary(&stripped);
    ReleaseNotes {
        version: version.to_string(),
        summary,
        body_markdown: rest.trim().to_string(),
    }
}

fn player_from_section(version: &str, raw: &str) -> ReleaseNotes {
    let stripped = hide_sections(raw, |_level, title| title.eq_ignore_ascii_case("install"));
    let (summary, rest) = split_summary(&stripped);
    ReleaseNotes {
        version: version.to_string(),
        summary,
        body_markdown: rest.trim().to_string(),
    }
}

fn usable(notes: &ReleaseNotes) -> bool {
    !notes.summary.trim().is_empty() || !notes.body_markdown.trim().is_empty()
}

fn raw_sections(changelog: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current: Option<String> = None;
    let mut buf: Vec<&str> = Vec::new();
    for line in changelog.lines() {
        if let Some(ver) = version_heading(line) {
            if let Some(ver) = current.take() {
                sections.push((ver, buf.join("\n")));
                buf.clear();
            }
            current = Some(ver);
        } else if current.is_some() {
            buf.push(line);
        }
    }
    if let Some(ver) = current {
        sections.push((ver, buf.join("\n")));
    }
    sections
}

fn version_heading(line: &str) -> Option<String> {
    let (level, title) = atx(line)?;
    if level != 2 {
        return None;
    }
    let key = version_key(title)?;
    Some(display_version(key))
}

fn version_key(raw: &str) -> Option<(u32, u32, u32)> {
    let core = raw.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next().unwrap_or(core);
    if core.is_empty() || core.split_whitespace().nth(1).is_some() {
        return None;
    }
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn display_version((major, minor, patch): (u32, u32, u32)) -> String {
    format!("{major}.{minor}.{patch}")
}

/// Drop ATX sections whose title matches `drop_title`, through the next
/// heading of the same or higher rank. Fenced code is not a heading.
fn hide_sections(markdown: &str, drop_title: impl Fn(usize, &str) -> bool) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skip_until: Option<usize> = None;
    let mut in_fence = false;
    for line in markdown.lines() {
        let fence = line.trim_start().starts_with("```");
        if fence {
            if skip_until.is_none() {
                in_fence = !in_fence;
                out.push(line);
            }
            continue;
        }
        if !in_fence && let Some((level, title)) = atx(line) {
            if let Some(limit) = skip_until
                && level <= limit
            {
                skip_until = None;
            }
            if skip_until.is_none() && drop_title(level, title) {
                skip_until = Some(level);
                continue;
            }
        }
        if skip_until.is_none() {
            out.push(line);
        }
    }
    let mut joined = out.join("\n");
    if markdown.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

fn atx(line: &str) -> Option<(usize, &str)> {
    if !line.starts_with('#') {
        return None;
    }
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = line.get(level..)?;
    if !rest.starts_with(' ') && !rest.is_empty() {
        return None;
    }
    let title = rest.trim().trim_end_matches('#').trim();
    Some((level, title))
}

fn split_summary(text: &str) -> (String, String) {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    if i >= lines.len() {
        return (String::new(), String::new());
    }
    if atx(lines[i]).is_some() {
        return (String::new(), text.trim().to_string());
    }
    let start = i;
    while i < lines.len() && !lines[i].trim().is_empty() && atx(lines[i]).is_none() {
        i += 1;
    }
    let summary = lines[start..i]
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ");
    let rest = lines[i..].join("\n");
    (summary, rest.trim().to_string())
}

fn drop_leading_paragraph_if(text: &str, pred: impl Fn(&str) -> bool) -> String {
    let (summary, rest) = split_summary(text);
    if pred(&summary) {
        rest
    } else {
        text.trim().to_string()
    }
}

fn release_block(section: &ShownRelease) -> Element<'_, Message> {
    let mut col = column![
        text(format!("v{}", section.version))
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
    ]
    .spacing(8)
    .width(Fill);
    if !section.summary.is_empty() {
        col = col.push(summary_callout(&section.summary));
    }
    if !section.items.is_empty() {
        col = col.push(
            container(markdown::view_with(
                section.items.iter(),
                markdown_settings(),
                &NotesViewer,
            ))
            .width(Fill),
        );
    }
    col.into()
}

fn summary_callout(summary: &str) -> Element<'_, Message> {
    container(
        text(summary.to_string())
            .size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([8, 12]))
    .width(Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG)),
        text_color: Some(TEXT),
        border: iced::Border {
            color: theme::ACCENT,
            width: 1.0,
            radius: theme::inner_radius(),
        },
        ..container::Style::default()
    })
    .into()
}

fn markdown_settings() -> markdown::Settings {
    let mut style = markdown::Style::from(theme::iced_theme());
    style.font = FONT_MEDIUM;
    style.inline_code_font = Font::MONOSPACE;
    style.code_block_font = Font::MONOSPACE;
    style.inline_code_color = theme::ACCENT;
    style.link_color = theme::ACCENT;
    let mut settings = markdown::Settings::with_text_size(SIZE_BODY, style);
    settings.h1_size = SIZE_TITLE.into();
    settings.h2_size = 18.0.into();
    settings.h3_size = 16.0.into();
    settings.spacing = 10.0.into();
    settings.code_size = SIZE_META.into();
    settings
}

struct NotesViewer;

impl<'a> markdown::Viewer<'a, Message> for NotesViewer {
    fn on_link_click(url: markdown::Uri) -> Message {
        Message::OpenNotesLink(url)
    }

    fn heading(
        &self,
        mut settings: markdown::Settings,
        level: &'a markdown::HeadingLevel,
        text: &'a markdown::Text,
        index: usize,
    ) -> Element<'a, Message> {
        settings.style.font = FONT_BOLD;
        markdown::heading(settings, level, text, index, Message::OpenNotesLink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "\
# Stat Tracker changelog

Preamble that is not a release.

## 0.4.2

Player summary for two. It stays short.

More detail with `code` and a [link](https://example.com/notes).

- first bullet
- second bullet

### Install

```sh
curl secret-two
```

### Kept

after the install block

## 0.4.1

Summary for one.

### Install

curl secret-one

## 0.4.0

Older summary.

A fenced heading is not a section:

```
### Install
still visible
```
";

    #[test]
    fn section_for_reads_the_current_version_summary() {
        let notes = section_for(FIXTURE, "v0.4.2").expect("section");
        assert_eq!(notes.version, "0.4.2");
        assert_eq!(notes.summary, "Player summary for two. It stays short.");
        assert!(notes.body_markdown.contains("More detail"));
        assert!(notes.body_markdown.contains("`code`"));
        assert!(notes.body_markdown.contains("https://example.com/notes"));
        assert!(notes.body_markdown.contains("- first bullet"));
        assert!(section_for(FIXTURE, "0.9.9").is_none());
    }

    #[test]
    fn range_is_newest_first_and_excludes_the_already_installed_version() {
        let notes = sections_between(FIXTURE, "0.4.0", "0.4.2");
        let versions: Vec<_> = notes.iter().map(|n| n.version.as_str()).collect();
        assert_eq!(versions, ["0.4.2", "0.4.1"]);
        assert!(sections_between(FIXTURE, "0.4.2", "0.4.2").is_empty());
        assert!(sections_between(FIXTURE, "nope", "0.4.2").is_empty());
    }

    #[test]
    fn install_blocks_are_hidden_and_later_sections_stay() {
        let notes = section_for(FIXTURE, "0.4.2").expect("section");
        assert!(!notes.summary.contains("Install"));
        assert!(!notes.body_markdown.contains("### Install"));
        assert!(!notes.body_markdown.contains("curl secret-two"));
        assert!(notes.body_markdown.contains("### Kept"));
        assert!(notes.body_markdown.contains("after the install block"));
        let older = section_for(FIXTURE, "0.4.0").expect("older");
        assert!(
            older.body_markdown.contains("### Install"),
            "a fenced line is code, not an Install heading: {}",
            older.body_markdown
        );
        assert!(older.body_markdown.contains("still visible"));
    }

    #[test]
    fn seen_version_classifies_first_run_same_upgrade_and_downgrade() {
        assert_eq!(classify_seen(None, Some("0.4.2")), SeenChange::FirstRun);
        assert_eq!(classify_seen(Some(""), Some("0.4.2")), SeenChange::FirstRun);
        assert_eq!(
            classify_seen(Some("v0.4.2"), Some("0.4.2")),
            SeenChange::Unchanged
        );
        assert_eq!(
            classify_seen(Some("0.4.0"), Some("0.4.2")),
            SeenChange::Upgrade
        );
        assert_eq!(
            classify_seen(Some("0.4.2"), Some("0.4.1")),
            SeenChange::Downgrade
        );
        assert_eq!(classify_seen(Some("0.4.2"), None), SeenChange::Unchanged);
        assert_eq!(
            classify_seen(Some("0.4.2"), Some("not-a-version")),
            SeenChange::Unchanged
        );

        let first = notes_for_change(FIXTURE, SeenChange::FirstRun, None, "0.4.2");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].version, "0.4.2");

        assert!(
            notes_for_change(FIXTURE, SeenChange::Unchanged, Some("0.4.2"), "0.4.2").is_empty()
        );

        let upgraded = notes_for_change(FIXTURE, SeenChange::Upgrade, Some("0.4.0"), "0.4.2");
        assert_eq!(
            upgraded
                .iter()
                .map(|n| n.version.as_str())
                .collect::<Vec<_>>(),
            ["0.4.2", "0.4.1"]
        );

        let downgraded = notes_for_change(FIXTURE, SeenChange::Downgrade, Some("0.4.2"), "0.4.1");
        assert_eq!(downgraded.len(), 1);
        assert_eq!(downgraded[0].version, "0.4.1");
        assert_eq!(downgraded[0].summary, "Summary for one.");
    }

    #[test]
    fn update_notes_prefer_github_body_and_fall_back_to_the_bundle() {
        let remote_two = "\
Prebuilt Linux x86_64 build of the Overwatch 2 stat tracker (daemon + Iced GUI).

Player summary from GitHub.

Detail line with `inline`.

- a bullet

### Install

curl secret-remote

## Commits since 0.4.1

- some commit

## Requirements & install

Need GTK.
";
        let remote = [
            ("0.4.2", remote_two),
            ("0.4.3", "Brand new summary.\n\nOnly on GitHub.\n"),
        ];
        let notes = notes_for_update(FIXTURE, "0.4.0", "0.4.3", &remote);
        let versions: Vec<_> = notes.iter().map(|n| n.version.as_str()).collect();
        assert_eq!(versions, ["0.4.3", "0.4.2", "0.4.1"]);

        assert_eq!(notes[0].summary, "Brand new summary.");
        assert!(notes[0].body_markdown.contains("Only on GitHub."));

        assert_eq!(notes[1].summary, "Player summary from GitHub.");
        assert!(notes[1].body_markdown.contains("Detail line"));
        assert!(notes[1].body_markdown.contains("- a bullet"));
        assert!(!notes[1].body_markdown.contains("Prebuilt Linux"));
        assert!(!notes[1].body_markdown.contains("curl secret-remote"));
        assert!(!notes[1].body_markdown.contains("some commit"));
        assert!(!notes[1].body_markdown.contains("Need GTK"));

        assert_eq!(notes[2].summary, "Summary for one.");

        let fallback = notes_for_update(FIXTURE, "0.4.1", "0.4.2", &[("0.4.2", "   \n")]);
        assert_eq!(fallback.len(), 1);
        assert_eq!(
            fallback[0].summary,
            "Player summary for two. It stays short."
        );
    }

    #[test]
    fn bundled_recent_releases_lead_with_a_summary_and_hide_install() {
        assert!(BUNDLED_CHANGELOG.contains("first paragraph under the `## X.Y.Z` heading"));
        for version in ["0.4.23", "0.4.22", "0.4.21", "0.4.20", "0.4.19"] {
            let raw = workflow_body(BUNDLED_CHANGELOG, version);
            assert!(
                raw.contains("### Install"),
                "{version} must keep its Install block for the release workflow"
            );
            assert!(
                raw.contains("bootstrap.sh"),
                "{version} Install block should still carry the bootstrap command"
            );
            let notes = section_for(BUNDLED_CHANGELOG, version).expect(version);
            assert!(
                notes.summary.len() > 40,
                "{version} summary: {}",
                notes.summary
            );
            assert!(!notes.summary.contains("###"), "{}", notes.summary);
            assert!(!notes.body_markdown.contains("### Install"), "{version}");
            assert!(
                !notes.body_markdown.contains("bootstrap.sh"),
                "{version} player view leaked the install command"
            );
            assert!(
                !notes.body_markdown.starts_with(&notes.summary),
                "{version} summary should be split out of the body"
            );
        }
        let latest = section_for(BUNDLED_CHANGELOG, "0.4.23").expect("latest");
        assert!(latest.summary.contains("private log"));
        assert!(latest.body_markdown.contains("shadow digit reader"));
        let boundaries = section_for(BUNDLED_CHANGELOG, "0.4.19").expect("0.4.19");
        assert!(
            boundaries
                .body_markdown
                .contains("board-order state machine")
        );
    }

    #[test]
    fn iced_parser_keeps_heading_list_and_paragraph() {
        let items: Vec<Item> = markdown::parse(
            "## Hello\n\n- one\n- two\n\nA `code` span and [link](https://example.com).",
        )
        .collect();
        assert!(matches!(items.first(), Some(Item::Heading(_, _))));
        assert!(items.iter().any(|item| matches!(item, Item::List { .. })));
        assert!(items.iter().any(|item| matches!(item, Item::Paragraph(_))));
    }

    #[test]
    fn last_seen_file_roundtrip_feeds_the_four_cases() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        assert_eq!(crate::seasons::load_last_seen_version(path), None);
        assert_eq!(classify_seen(None, Some("0.4.20")), SeenChange::FirstRun);

        crate::seasons::save_ui_state(path, &crate::model::SeasonSel::Season("s17".into()))
            .expect("season");
        crate::seasons::save_last_seen_version(path, "0.4.20").expect("save");
        let seen = crate::seasons::load_last_seen_version(path);
        assert_eq!(seen.as_deref(), Some("0.4.20"));
        assert_eq!(
            crate::seasons::load_ui_state(path),
            Some(crate::model::SeasonSel::Season("s17".into())),
            "remembering a version must keep the season selection"
        );
        assert_eq!(
            classify_seen(seen.as_deref(), Some("v0.4.20")),
            SeenChange::Unchanged
        );
        assert_eq!(
            classify_seen(seen.as_deref(), Some("0.4.23")),
            SeenChange::Upgrade
        );

        crate::seasons::save_last_seen_version(path, "0.4.23").expect("upgrade stored");
        let seen = crate::seasons::load_last_seen_version(path);
        assert_eq!(
            classify_seen(seen.as_deref(), Some("0.4.19")),
            SeenChange::Downgrade
        );
    }

    /// Same slice the release workflow's awk keeps: the body under `## {ver}`
    /// until the next `##` heading, heading line itself excluded.
    fn workflow_body(changelog: &str, version: &str) -> String {
        let heading = format!("## {version}");
        let mut collecting = false;
        let mut lines = Vec::new();
        for line in changelog.lines() {
            if line == heading {
                collecting = true;
                continue;
            }
            if collecting && line.starts_with("## ") {
                break;
            }
            if collecting {
                lines.push(line);
            }
        }
        lines.join("\n")
    }
}
