//! Release notes for the desktop app.
//!
//! The player-facing text is `crates/stat-tracker/CHANGELOG.md`, compiled
//! into this binary so it is available offline. Iced 0.14 already renders
//! Markdown (`iced::widget::markdown`, feature `markdown`, which uses
//! `pulldown-cmark`). Headings, paragraphs, lists, inline code, and links
//! come from that widget. No separate Markdown crate is added.
//!
//! `### Install` blocks stay in the changelog for the GitHub release page.
//! This view drops them. Each release is a card: the player summary, then
//! the `### Highlights` bullets, then the remaining text under Details
//! (closed until opened). Only the latest few cards show until the reader
//! asks for older releases.

use std::collections::HashSet;

use iced::widget::markdown::{self, Item};
use iced::widget::{button, column, container, rich_text, row, scrollable, text};
use iced::{Alignment, Element, Fill, Font, Length, Padding};

use crate::app::Message;
use crate::theme::{
    self, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_FEATURED,
    SIZE_META, SIZE_TITLE, TEXT, TEXT_2,
};

/// Changelog shipped inside the GUI binary.
pub const BUNDLED_CHANGELOG: &str = include_str!("../../stat-tracker/CHANGELOG.md");

/// Cards kept on screen before "Show older releases".
pub const VISIBLE_RELEASES: usize = 3;

/// Readable card width, about 70 to 80 characters of body text.
const READING_WIDTH: f32 = 600.0;
/// Player summary, a step above body text.
const LEAD_SIZE: f32 = 17.0;

/// One release, after Install blocks are removed and the summary is split out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseNotes {
    pub version: String,
    /// First paragraph. Plain language, shown ahead of the highlights.
    pub summary: String,
    /// `### Highlights` bullets, plain text, in changelog order.
    pub highlights: Vec<String>,
    /// Remaining Markdown (headings, paragraphs, lists, code, links).
    pub body_markdown: String,
    /// Short date from a GitHub `published_at`, when that release was fetched.
    pub published_on: Option<String>,
}

/// Notes ready to draw. Markdown is parsed once, not on every frame.
pub struct ShownRelease {
    pub version: String,
    pub summary: String,
    pub highlights: Vec<String>,
    pub items: Vec<Item>,
    pub published_on: Option<String>,
}

/// Which notes surface owns a Details toggle or the older-releases button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotesSurface {
    Update,
    Dialog,
}

/// What the card list needs besides the release text.
pub struct NotesUi<'a> {
    pub surface: NotesSurface,
    pub installed: Option<&'a str>,
    pub offered: Option<&'a str>,
    pub open_details: &'a HashSet<String>,
    pub show_older: bool,
}

/// A GitHub release body plus the publish time, when the API sent one.
pub struct RemoteRelease<'a> {
    pub version: &'a str,
    pub body: &'a str,
    pub published_at: Option<&'a str>,
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

/// Notes for an offered update. A non-empty cleaned GitHub body supplies the
/// technical text. When that body has no `### Highlights` list, the bundled
/// summary and highlights stay in front and the GitHub text moves under
/// Details. The bundled section is the fallback when GitHub has no usable text.
pub fn notes_for_update(
    bundled: &str,
    current: &str,
    latest: &str,
    remote: &[RemoteRelease<'_>],
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
    for remote in remote {
        if let Some(key) = version_key(remote.version)
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
            highlights: notes.highlights,
            published_on: notes.published_on,
        })
        .collect()
}

/// Releases drawn before the older-releases button. Newest first already.
pub fn releases_on_screen<T>(sections: &[T], show_older: bool) -> &[T] {
    if show_older || sections.len() <= VISIBLE_RELEASES {
        sections
    } else {
        &sections[..VISIBLE_RELEASES]
    }
}

/// Details stay closed until that version is in the open set.
pub fn details_open(open: &HashSet<String>, version: &str) -> bool {
    open.contains(version)
}

pub fn notes_column<'a>(sections: &'a [ShownRelease], ui: NotesUi<'a>) -> Element<'a, Message> {
    let visible = releases_on_screen(sections, ui.show_older);
    let mut col = column![].spacing(16).width(Fill);
    for section in visible {
        col = col.push(release_card(section, &ui));
    }
    if !ui.show_older && sections.len() > VISIBLE_RELEASES {
        col = col.push(
            button(
                text("Show older releases")
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(TEXT),
            )
            .padding(Padding::from([8, 16]))
            .style(theme::ghost_btn())
            .on_press(Message::ShowOlderReleases(ui.surface)),
        );
    }
    col.into()
}

pub fn dialog<'a>(
    subtitle: &'a str,
    sections: &'a [ShownRelease],
    ui: NotesUi<'a>,
) -> Element<'a, Message> {
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
                container(notes_column(sections, ui))
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
    remote: &[RemoteRelease<'_>],
) -> Option<ReleaseNotes> {
    let published_on = remote
        .iter()
        .find(|row| version_key(row.version) == Some(key))
        .and_then(|row| row.published_at)
        .and_then(format_release_date);
    let bundled_notes = section_for(bundled, &display_version(key));
    if let Some(row) = remote
        .iter()
        .find(|row| version_key(row.version) == Some(key))
    {
        let mut from_remote = from_github_body(&display_version(key), row.body);
        if usable(&from_remote) {
            if from_remote.highlights.is_empty()
                && let Some(bundled_notes) = &bundled_notes
            {
                from_remote.highlights.clone_from(&bundled_notes.highlights);
                if !bundled_notes.summary.is_empty() && from_remote.summary != bundled_notes.summary
                {
                    let mut details = String::new();
                    let lead = from_remote.summary.trim();
                    let rest = from_remote.body_markdown.trim();
                    if !lead.is_empty() {
                        details.push_str(lead);
                        if !rest.is_empty() {
                            details.push_str("\n\n");
                        }
                    }
                    details.push_str(rest);
                    from_remote.summary.clone_from(&bundled_notes.summary);
                    from_remote.body_markdown = details;
                }
            }
            from_remote.published_on = published_on;
            return Some(from_remote);
        }
    }
    bundled_notes.map(|mut notes| {
        notes.published_on = published_on;
        notes
    })
}

fn from_github_body(version: &str, body: &str) -> ReleaseNotes {
    let stripped = hide_sections(body, |_level, title| {
        title.eq_ignore_ascii_case("install")
            || title.starts_with("Commits since")
            || title.starts_with("Requirements")
    });
    let stripped =
        drop_leading_paragraph_if(&stripped, |summary| summary.starts_with("Prebuilt Linux"));
    player_text(version, &stripped)
}

fn player_from_section(version: &str, raw: &str) -> ReleaseNotes {
    let stripped = hide_sections(raw, |_level, title| title.eq_ignore_ascii_case("install"));
    player_text(version, &stripped)
}

fn player_text(version: &str, raw: &str) -> ReleaseNotes {
    let highlights = extract_highlights(raw);
    let stripped = strip_highlights(raw);
    let (summary, rest) = split_summary(&stripped);
    ReleaseNotes {
        version: version.to_string(),
        summary,
        highlights,
        body_markdown: rest.trim().to_string(),
        published_on: None,
    }
}

fn usable(notes: &ReleaseNotes) -> bool {
    !notes.summary.trim().is_empty()
        || !notes.highlights.is_empty()
        || !notes.body_markdown.trim().is_empty()
}

/// `2026-03-04T12:00:00Z` becomes `4 Mar 2026`. Anything else is omitted.
fn format_release_date(raw: &str) -> Option<String> {
    let date = raw.get(..10)?;
    let mut parts = date.split('-');
    let year = parts.next()?;
    let month: usize = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if year.len() != 4 || year.chars().any(|c| !c.is_ascii_digit()) {
        return None;
    }
    let month_name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .get(month.checked_sub(1)?)?;
    Some(format!("{day} {month_name} {year}"))
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

fn extract_highlights(markdown: &str) -> Vec<String> {
    highlight_lines(markdown).0
}

/// Drop the `### Highlights` heading and its bullet list. Following prose stays.
fn strip_highlights(markdown: &str) -> String {
    let drop_lines = highlight_lines(markdown).1;
    let mut out = String::new();
    for (index, line) in markdown.lines().enumerate() {
        if drop_lines.contains(&index) {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    if markdown.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Bullet text, plus the source line indexes of the heading and that list.
fn highlight_lines(markdown: &str) -> (Vec<String>, std::collections::BTreeSet<usize>) {
    let mut bullets = Vec::new();
    let mut drop_lines = std::collections::BTreeSet::new();
    let mut in_section = false;
    let mut in_fence = false;
    let mut started = false;
    for (index, line) in markdown.lines().enumerate() {
        let fence = line.trim_start().starts_with("```");
        if fence {
            if !in_section {
                in_fence = !in_fence;
            } else {
                break;
            }
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some((_level, title)) = atx(line) {
            if in_section {
                break;
            }
            if title.eq_ignore_ascii_case("highlights") {
                in_section = true;
                drop_lines.insert(index);
            }
            continue;
        }
        if !in_section {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if started {
                drop_lines.insert(index);
            }
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let item = item.trim();
            if !item.is_empty() {
                bullets.push(item.to_string());
                drop_lines.insert(index);
                started = true;
            }
            continue;
        }
        break;
    }
    // A blank line after the list belongs to the following prose.
    if let Some(last) = drop_lines.iter().next_back().copied()
        && markdown
            .lines()
            .nth(last)
            .is_some_and(|line| line.trim().is_empty())
    {
        drop_lines.remove(&last);
    }
    (bullets, drop_lines)
}

fn drop_leading_paragraph_if(text: &str, pred: impl Fn(&str) -> bool) -> String {
    let (summary, rest) = split_summary(text);
    if pred(&summary) {
        rest
    } else {
        text.trim().to_string()
    }
}

fn release_card<'a>(section: &'a ShownRelease, ui: &NotesUi<'a>) -> Element<'a, Message> {
    let open = details_open(ui.open_details, &section.version);
    let mut header = row![
        text(format!("v{}", section.version))
            .size(SIZE_TITLE)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    if let Some(date) = &section.published_on {
        header = header.push(
            text(date.clone())
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        );
    }
    header = header.push(iced::widget::space().width(Fill));
    if let Some(label) = release_badge(&section.version, ui.installed, ui.offered) {
        header = header.push(badge_pill(label));
    }

    let mut col = column![header].spacing(14).width(Fill);
    if !section.summary.is_empty() {
        col = col.push(
            text(section.summary.clone())
                .size(LEAD_SIZE)
                .font(FONT_MEDIUM)
                .color(TEXT)
                .line_height(1.5),
        );
    }
    if !section.highlights.is_empty() {
        let mut list = column![].spacing(10).width(Fill);
        for item in &section.highlights {
            list = list.push(
                text(format!("• {item}"))
                    .size(SIZE_BODY)
                    .font(FONT_MEDIUM)
                    .color(TEXT)
                    .line_height(1.5),
            );
        }
        col = col.push(list);
    }
    if !section.items.is_empty() {
        let label = if open { "Hide details" } else { "Details" };
        col = col.push(
            button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
                .padding(Padding::from([6, 14]))
                .style(theme::ghost_btn())
                .on_press(Message::ToggleReleaseDetails {
                    surface: ui.surface,
                    version: section.version.clone(),
                }),
        );
        if open {
            col = col.push(
                container(markdown::view_with(
                    section.items.iter(),
                    markdown_settings(),
                    &NotesViewer,
                ))
                .width(Fill),
            );
        }
    }

    container(col)
        .padding(20)
        .width(Fill)
        .max_width(READING_WIDTH)
        .style(theme::surface_panel)
        .into()
}

/// "Update available" for the offered release, "Installed" for this build,
/// "New" for a version newer than the one installed.
pub fn release_badge(
    version: &str,
    installed: Option<&str>,
    offered: Option<&str>,
) -> Option<&'static str> {
    let key = version_key(version)?;
    let installed_key = installed.and_then(version_key);
    let offered_key = offered.and_then(version_key);
    if offered_key == Some(key) && installed_key != Some(key) {
        return Some("Update available");
    }
    if installed_key == Some(key) {
        return Some("Installed");
    }
    if installed_key.is_some_and(|installed_key| key > installed_key) {
        return Some("New");
    }
    None
}

fn badge_pill(label: &'static str) -> Element<'static, Message> {
    let (bg, fg, border) = if label == "Installed" {
        (theme::SURFACE, theme::TEXT_2, theme::BORDER)
    } else {
        (theme::ACCENT, theme::TEXT, theme::ACCENT)
    };
    container(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(fg))
        .padding(Padding::from([3, 10]))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: Some(fg),
            border: iced::Border {
                color: border,
                width: 1.0,
                radius: theme::RADIUS_CHIP.into(),
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
    settings.spacing = 18.0.into();
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

    fn paragraph(
        &self,
        settings: markdown::Settings,
        text: &markdown::Text,
    ) -> Element<'a, Message> {
        rich_text(text.spans(settings.style))
            .size(settings.text_size)
            .line_height(1.5)
            .on_link_click(Message::OpenNotesLink)
            .into()
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

### Highlights

- Maps stay on the right game
- Long games are not cut off

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
        assert_eq!(
            notes.highlights,
            ["Maps stay on the right game", "Long games are not cut off"]
        );
        assert!(!notes.body_markdown.contains("### Highlights"));
        assert!(!notes.body_markdown.contains("Maps stay on the right game"));
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
            RemoteRelease {
                version: "0.4.2",
                body: remote_two,
                published_at: Some("2026-03-04T18:22:11Z"),
            },
            RemoteRelease {
                version: "0.4.3",
                body: "Brand new summary.\n\nOnly on GitHub.\n",
                published_at: None,
            },
        ];
        let notes = notes_for_update(FIXTURE, "0.4.0", "0.4.3", &remote);
        let versions: Vec<_> = notes.iter().map(|n| n.version.as_str()).collect();
        assert_eq!(versions, ["0.4.3", "0.4.2", "0.4.1"]);

        assert_eq!(notes[0].summary, "Brand new summary.");
        assert!(notes[0].body_markdown.contains("Only on GitHub."));
        assert!(notes[0].highlights.is_empty());
        assert!(notes[0].published_on.is_none());

        assert_eq!(notes[1].summary, "Player summary for two. It stays short.");
        assert_eq!(
            notes[1].highlights,
            ["Maps stay on the right game", "Long games are not cut off"]
        );
        assert_eq!(notes[1].published_on.as_deref(), Some("4 Mar 2026"));
        assert!(
            notes[1]
                .body_markdown
                .contains("Player summary from GitHub.")
        );
        assert!(notes[1].body_markdown.contains("Detail line"));
        assert!(notes[1].body_markdown.contains("- a bullet"));
        assert!(!notes[1].body_markdown.contains("Prebuilt Linux"));
        assert!(!notes[1].body_markdown.contains("curl secret-remote"));
        assert!(!notes[1].body_markdown.contains("some commit"));
        assert!(!notes[1].body_markdown.contains("Need GTK"));

        assert_eq!(notes[2].summary, "Summary for one.");

        let fallback = notes_for_update(
            FIXTURE,
            "0.4.1",
            "0.4.2",
            &[RemoteRelease {
                version: "0.4.2",
                body: "   \n",
                published_at: Some("not-a-date"),
            }],
        );
        assert_eq!(fallback.len(), 1);
        assert_eq!(
            fallback[0].summary,
            "Player summary for two. It stays short."
        );
        assert_eq!(
            fallback[0].highlights,
            ["Maps stay on the right game", "Long games are not cut off"]
        );
        assert!(fallback[0].published_on.is_none());

        let remote_highlights = "\
Player summary from GitHub.

### Highlights

- remote highlight

Detail line with `inline`.
";
        let preferred = notes_for_update(
            FIXTURE,
            "0.4.1",
            "0.4.2",
            &[RemoteRelease {
                version: "0.4.2",
                body: remote_highlights,
                published_at: None,
            }],
        );
        assert_eq!(preferred[0].summary, "Player summary from GitHub.");
        assert_eq!(preferred[0].highlights, ["remote highlight"]);
        assert!(!preferred[0].body_markdown.contains("remote highlight"));
        assert!(preferred[0].body_markdown.contains("Detail line"));
    }

    #[test]
    fn bundled_recent_releases_lead_with_a_summary_and_hide_install() {
        assert!(BUNDLED_CHANGELOG.contains("first paragraph under the `## X.Y.Z` heading"));
        assert!(BUNDLED_CHANGELOG.contains("a `### Highlights` list gives two to four"));
        assert!(BUNDLED_CHANGELOG.contains("Each bullet adds a concrete change the summary"));
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
            assert!(
                (2..=4).contains(&notes.highlights.len()),
                "{version} highlights: {:?}",
                notes.highlights
            );
            assert!(!notes.body_markdown.contains("### Highlights"), "{version}");
            for bullet in &notes.highlights {
                assert!(!bullet.contains('\u{2014}'), "{version} bullet: {bullet}");
                assert!(
                    !notes.summary.contains(bullet),
                    "{version} highlight repeats the summary: {bullet}"
                );
                assert!(
                    !notes.body_markdown.contains(bullet),
                    "{version} highlight leaked into details: {bullet}"
                );
            }
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
    fn details_start_closed_and_older_than_three_stay_hidden() {
        let notes = all_notes(BUNDLED_CHANGELOG);
        assert!(notes.len() > VISIBLE_RELEASES);
        let shown = releases_on_screen(&notes, false);
        assert_eq!(shown.len(), VISIBLE_RELEASES);
        assert_eq!(
            shown
                .iter()
                .map(|note| note.version.as_str())
                .collect::<Vec<_>>(),
            ["0.4.23", "0.4.22", "0.4.21"]
        );
        assert_eq!(releases_on_screen(&notes, true).len(), notes.len());

        let open = HashSet::new();
        assert!(notes.iter().all(|note| !details_open(&open, &note.version)));
        let mut open = open;
        open.insert("0.4.23".to_string());
        assert!(details_open(&open, "0.4.23"));
        assert!(!details_open(&open, "0.4.22"));
    }

    #[test]
    fn badges_mark_the_offered_update_the_install_and_newer_skips() {
        assert_eq!(
            release_badge("0.4.23", Some("0.4.20"), Some("0.4.23")),
            Some("Update available")
        );
        assert_eq!(
            release_badge("0.4.22", Some("0.4.20"), Some("0.4.23")),
            Some("New")
        );
        assert_eq!(
            release_badge("0.4.23", Some("0.4.23"), None),
            Some("Installed")
        );
        assert_eq!(
            release_badge("0.4.23", Some("0.4.23"), Some("0.4.23")),
            Some("Installed")
        );
        assert_eq!(release_badge("0.4.19", Some("0.4.23"), None), None);
        assert_eq!(
            format_release_date("2026-10-09T00:00:00Z").as_deref(),
            Some("9 Oct 2026")
        );
        assert!(format_release_date("yesterday").is_none());
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
