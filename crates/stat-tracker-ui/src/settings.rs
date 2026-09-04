//! Settings form + screen. Writes today's `Config` fields via `Config::save`.
//!
//! Layout: two-column masonry (Maps density tokens), Stored data spanning the
//! pane, Save as a full-width footer strip. Does not invent daemon config keys.
//! `data_dir` and `ocr_threads` are preserved from the loaded file.

use iced::widget::{Row, button, checkbox, column, container, row, space, text, text_input};
use iced::{Alignment, Element, Fill, Padding};
use stat_tracker::config::{AutoDetectConfig, Config, SyncConfig};

use crate::app::{Message, TrackerApp};
use crate::layout::settings_columns;
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, PAD_INNER, SIZE_BODY, SIZE_LABEL,
    SIZE_META, SIZE_TITLE, TEXT, TEXT_2, TEXT_3,
};
use crate::update;
use crate::widgets;

/// Seconds / counts — must stay compact on ultrawide.
pub(crate) const FIELD_NUMERIC: f32 = 168.0;
/// Shortcut, in-game name.
pub(crate) const FIELD_SHORT: f32 = 280.0;
/// URL, token, process list. Capped so they do not become a full-pane bar.
pub(crate) const FIELD_TEXT: f32 = 480.0;

const _: () = {
    assert!(FIELD_NUMERIC <= 200.0);
    assert!(FIELD_SHORT < 400.0);
    assert!(FIELD_TEXT <= 560.0);
    assert!(FIELD_NUMERIC < FIELD_SHORT);
    assert!(FIELD_SHORT < FIELD_TEXT);
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    PlayerName,
    SessionWindow,
    ProcessNames,
    PollInterval,
    Cooldown,
    SyncUrl,
    SyncToken,
    OverlayHotkey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsToggle {
    AutoDetect,
    DebugOcr,
    OverlayHotkey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsForm {
    pub capture_output: String,
    pub player_name: String,
    pub session_window_secs: String,
    pub game_process_names: String,
    pub auto_detect_enabled: bool,
    pub poll_interval_secs: String,
    pub cooldown_secs: String,
    pub sync_url: String,
    pub sync_token: String,
    pub debug_ocr: bool,
    pub overlay_hotkey: String,
    pub overlay_hotkey_enabled: bool,
}

impl Default for SettingsForm {
    fn default() -> Self {
        Self::from_config(&Config::default())
    }
}

impl SettingsForm {
    pub fn from_config(config: &Config) -> Self {
        Self {
            capture_output: config.capture_output.clone().unwrap_or_default(),
            player_name: config.player_name.clone().unwrap_or_default(),
            session_window_secs: config.session_window_secs.to_string(),
            game_process_names: config.game_process_names.join(", "),
            auto_detect_enabled: config.auto_detect.enabled,
            poll_interval_secs: config.auto_detect.poll_interval_secs.to_string(),
            cooldown_secs: config.auto_detect.cooldown_secs.to_string(),
            sync_url: config
                .sync
                .as_ref()
                .map(|s| s.server_url.clone())
                .unwrap_or_default(),
            sync_token: config
                .sync
                .as_ref()
                .map(|s| s.token.clone())
                .unwrap_or_default(),
            debug_ocr: config.debug_ocr,
            overlay_hotkey: crate::hotkey::DEFAULT_BIND.to_string(),
            overlay_hotkey_enabled: true,
        }
    }

    pub fn set_text(&mut self, field: SettingsField, value: String) {
        match field {
            SettingsField::PlayerName => self.player_name = value,
            SettingsField::SessionWindow => self.session_window_secs = value,
            SettingsField::ProcessNames => self.game_process_names = value,
            SettingsField::PollInterval => self.poll_interval_secs = value,
            SettingsField::Cooldown => self.cooldown_secs = value,
            SettingsField::SyncUrl => self.sync_url = value,
            SettingsField::SyncToken => self.sync_token = value,
            SettingsField::OverlayHotkey => self.overlay_hotkey = value,
        }
    }

    pub fn set_toggle(&mut self, toggle: SettingsToggle, value: bool) {
        match toggle {
            SettingsToggle::AutoDetect => self.auto_detect_enabled = value,
            SettingsToggle::DebugOcr => self.debug_ocr = value,
            SettingsToggle::OverlayHotkey => self.overlay_hotkey_enabled = value,
        }
    }

    /// Map the form onto `base`, keeping `data_dir` and `ocr_threads`.
    pub fn to_config(&self, base: &Config) -> Config {
        Config {
            data_dir: base.data_dir.clone(),
            capture_output: nonempty(&self.capture_output),
            player_name: nonempty(&self.player_name),
            sync: sync_from_fields(&self.sync_url, &self.sync_token),
            auto_detect: AutoDetectConfig {
                enabled: self.auto_detect_enabled,
                poll_interval_secs: parse_u64(&self.poll_interval_secs, 4),
                cooldown_secs: parse_u64(&self.cooldown_secs, 120),
            },
            session_window_secs: parse_u64(&self.session_window_secs, 1800),
            game_process_names: parse_process_names(&self.game_process_names),
            debug_ocr: self.debug_ocr,
            ocr_threads: base.ocr_threads,
        }
    }
}

pub fn nonempty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Both URL and token are required — same as the Dioxus save path.
pub fn sync_from_fields(url: &str, token: &str) -> Option<SyncConfig> {
    let server_url = url.trim();
    let token = token.trim();
    if server_url.is_empty() || token.is_empty() {
        None
    } else {
        Some(SyncConfig {
            server_url: server_url.to_string(),
            token: token.to_string(),
        })
    }
}

pub fn parse_u64(raw: &str, fallback: u64) -> u64 {
    raw.trim().parse().unwrap_or(fallback)
}

/// Comma or newline list. Empty → empty vec (process gate off, as in Config).
pub fn parse_process_names(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub fn save_config(config: &Config) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())
}

/// Layout-only width cap. Save / bind behaviour is unchanged.
pub(crate) fn field_max_width(field: SettingsField) -> f32 {
    match field {
        SettingsField::SessionWindow | SettingsField::PollInterval | SettingsField::Cooldown => {
            FIELD_NUMERIC
        }
        SettingsField::PlayerName | SettingsField::OverlayHotkey => FIELD_SHORT,
        SettingsField::ProcessNames | SettingsField::SyncUrl | SettingsField::SyncToken => {
            FIELD_TEXT
        }
    }
}

/// Every settings card. Order here is the 1-column stack and the masonry input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsSection {
    Daemon,
    Capture,
    Companion,
    Player,
    AutoDetect,
    Sync,
    Ocr,
    Diagnostics,
    Data,
}

/// Two-column masonry. Stored data spans the pane so the last row is not
/// a skyscraper + empty pocket.
pub(crate) const MASONRY_SECTIONS: &[SettingsSection] = &[
    SettingsSection::Daemon,
    SettingsSection::Capture,
    SettingsSection::Companion,
    SettingsSection::Player,
    SettingsSection::AutoDetect,
    SettingsSection::Sync,
    SettingsSection::Ocr,
    SettingsSection::Diagnostics,
];

pub(crate) const SPANNING_SECTION: SettingsSection = SettingsSection::Data;
pub(crate) const SAVE_LABEL: &str = "Save settings";

/// Relative visual height after caption tightening. Used only to pack columns.
pub(crate) fn section_weight(section: SettingsSection) -> u16 {
    match section {
        SettingsSection::Daemon => 3,
        SettingsSection::Capture => 3,
        SettingsSection::Companion => 3,
        SettingsSection::Player => 4,
        SettingsSection::AutoDetect => 3,
        SettingsSection::Sync => 3,
        SettingsSection::Ocr => 3,
        SettingsSection::Diagnostics => 2,
        SettingsSection::Data => 3,
    }
}

/// Greedy masonry: each section goes into the currently shorter column.
/// Tie → leftmost. One column returns the input order.
pub(crate) fn pack_columns(sections: &[SettingsSection], cols: usize) -> Vec<Vec<SettingsSection>> {
    let cols = cols.max(1);
    if cols == 1 {
        return vec![sections.to_vec()];
    }
    let mut columns = vec![Vec::new(); cols];
    let mut heights = vec![0u16; cols];
    for &section in sections {
        let i = heights
            .iter()
            .enumerate()
            .min_by_key(|&(i, h)| (*h, i))
            .map(|(i, _)| i)
            .unwrap_or(0);
        columns[i].push(section);
        heights[i] = heights[i].saturating_add(section_weight(section));
    }
    columns
}

pub(crate) fn column_weight(column: &[SettingsSection]) -> u16 {
    column.iter().copied().map(section_weight).sum()
}

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let demo = app.fixture.is_some();
    let cols = settings_columns(content_width);
    let mut col = column![].spacing(GRID_GAP).width(Fill);

    if let Some(info) = &app.update {
        col = col.push(update::banner(info));
    }

    if demo {
        col = col.push(widgets::empty_surface(
            "Demo mode — settings, the tracker service, and stored data are not changed.",
        ));
    }

    let packed = pack_columns(MASONRY_SECTIONS, cols);
    let columns: Vec<Vec<Element<'_, Message>>> = packed
        .into_iter()
        .map(|ids| {
            ids.into_iter()
                .map(|id| section_card(id, app, demo))
                .collect()
        })
        .collect();
    col = col
        .push(section_columns(columns))
        .push(section_card(SPANNING_SECTION, app, demo))
        .push(save_footer(demo));

    col.into()
}

fn section_card(id: SettingsSection, app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    match id {
        SettingsSection::Daemon => daemon_card(app, demo),
        SettingsSection::Capture => capture_card(app, demo),
        SettingsSection::Companion => companion_card(app, demo),
        SettingsSection::Player => player_card(app, demo),
        SettingsSection::AutoDetect => auto_detect_card(app, demo),
        SettingsSection::Sync => sync_card(app, demo),
        SettingsSection::Ocr => ocr_card(app, demo),
        SettingsSection::Diagnostics => diagnostics_card(app, demo),
        SettingsSection::Data => data_card(app, demo),
    }
}

fn section_columns<'a>(columns: Vec<Vec<Element<'a, Message>>>) -> Element<'a, Message> {
    if columns.len() <= 1 {
        let mut col = column![].spacing(GRID_GAP).width(Fill);
        if let Some(cards) = columns.into_iter().next() {
            for card in cards {
                col = col.push(card);
            }
        }
        return col.into();
    }
    let mut row = Row::new()
        .spacing(GRID_GAP)
        .align_y(Alignment::Start)
        .width(Fill);
    for cards in columns {
        let mut col = column![].spacing(GRID_GAP).width(Fill);
        for card in cards {
            col = col.push(card);
        }
        row = row.push(col);
    }
    row.into()
}

fn daemon_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let d = &app.daemon;
    let mut actions = row![].spacing(8).align_y(Alignment::Center);
    if !demo && !app.daemon_busy {
        if d.running() {
            actions = actions
                .push(action_btn(
                    "Stop",
                    false,
                    Message::Daemon(crate::daemon::DaemonVerb::Stop),
                ))
                .push(action_btn(
                    "Restart",
                    true,
                    Message::Daemon(crate::daemon::DaemonVerb::Restart),
                ));
        } else {
            actions = actions.push(action_btn(
                "Start",
                true,
                Message::Daemon(crate::daemon::DaemonVerb::Start),
            ));
        }
        if d.service_installed {
            actions = actions.push(action_btn(
                if d.autostart {
                    "Don’t start on login"
                } else {
                    "Start on login"
                },
                false,
                Message::ToggleAutostart,
            ));
        }
    }

    let mut body = column![
        row![
            text(d.status_label())
                .size(SIZE_BODY)
                .font(FONT_SEMIBOLD)
                .color(if d.running() { theme::OK } else { TEXT_2 }),
            space().width(Fill),
            text(if let Some(pid) = d.pid {
                format!("process {pid}")
            } else {
                String::new()
            })
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
        ]
        .align_y(Alignment::Center),
        text(if d.service_installed {
            if d.autostart {
                "Starts with this computer"
            } else {
                "Installed — will not start on login"
            }
        } else {
            "No login service installed — Start launches the tracker directly"
        })
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_3),
    ]
    .spacing(6);

    if app.daemon_busy {
        body = body.push(
            text("Working…")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }
    body = body.push(actions);

    settings_card("Tracker service", body.into())
}

fn capture_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut chips = row![output_chip(
        "Auto (first monitor)",
        app.settings.capture_output.is_empty(),
        Message::SelectOutput(None),
    )]
    .spacing(8);
    for name in &app.outputs {
        chips = chips.push(output_chip(
            name,
            app.settings.capture_output == *name,
            Message::SelectOutput(Some(name.clone())),
        ));
    }

    let backend = match app.backend {
        None => "Looking for a capture method…",
        Some(stat_tracker::capture::CaptureBackend::Wayshot) => "Wayland",
        Some(stat_tracker::capture::CaptureBackend::X11) => "X11",
        Some(stat_tracker::capture::CaptureBackend::Portal) => "Desktop portal",
        Some(stat_tracker::capture::CaptureBackend::None) => "None available",
    };

    let capture_label = if app.backend.is_none() {
        "Looking for a capture method…"
    } else if app.capturing {
        "Capturing…"
    } else {
        "Capture now"
    };

    let mut body = column![
        field_caption("Monitor"),
        chips.wrap(),
        text(format!("Capture method: {backend}"))
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    ]
    .spacing(6);

    if !demo {
        let mut btn = button(
            text(capture_label)
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        )
        .padding(Padding::from([8, 16]))
        .style(theme::chip(true));
        if app.backend.is_some() && !app.capturing {
            btn = btn.on_press(Message::CaptureNow);
        }
        body = body.push(btn);
    }

    if let Some(err) = &app.preview_error {
        body = body.push(
            text(err.clone())
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::DANGER),
        );
    }

    if let Some((handle, when)) = &app.preview {
        body = body.push(
            text(format!("Preview — {when}"))
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
        body = body.push(
            iced::widget::image(handle.clone())
                .width(Fill)
                .height(iced::Length::Fixed(280.0)),
        );
        body = body.push(
            text("What the tracker sees — scoreboard should be visible.")
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }

    settings_card("Capture", body.into())
}

fn companion_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut body = column![
        checkbox(app.settings.overlay_hotkey_enabled)
            .label("Keyboard shortcut to hide or show the overlay")
            .on_toggle(|v| Message::SettingsToggle(SettingsToggle::OverlayHotkey, v))
            .size(16)
            .text_size(SIZE_BODY)
            .style(checkbox_style),
        hint("Works in fullscreen. Overlay is click-through."),
    ]
    .spacing(6);

    if app.settings.overlay_hotkey_enabled {
        body = body.push(field_input(
            "Shortcut",
            crate::hotkey::DEFAULT_BIND,
            &app.settings.overlay_hotkey,
            SettingsField::OverlayHotkey,
            false,
            demo,
            Some("Super = Windows key. Join with +."),
        ));
    }

    settings_card("Companion", body.into())
}

fn player_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let body = column![
        compact_row(
            Some(field_input(
                "In-game name",
                "e.g. FROZEN",
                &app.settings.player_name,
                SettingsField::PlayerName,
                false,
                demo,
                Some("Scoreboard name as shown in-game — no #1234."),
            )),
            Some(field_input(
                "Session window (seconds)",
                "1800",
                &app.settings.session_window_secs,
                SettingsField::SessionWindow,
                false,
                demo,
                Some("Nearby games count as one session."),
            )),
        ),
        field_input(
            "Game process names",
            "Overwatch.exe",
            &app.settings.game_process_names,
            SettingsField::ProcessNames,
            false,
            demo,
            Some("Comma-separated. Empty = always allow capture."),
        ),
    ]
    .spacing(6);
    settings_card("Player and sessions", body.into())
}

fn auto_detect_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut body = column![
        checkbox(app.settings.auto_detect_enabled)
            .label("Watch for match start and end")
            .on_toggle(|v| Message::SettingsToggle(SettingsToggle::AutoDetect, v))
            .size(16)
            .text_size(SIZE_BODY)
            .style(checkbox_style),
    ]
    .spacing(6);

    if app.settings.auto_detect_enabled {
        body = body.push(compact_row(
            Some(field_input(
                "Check every (seconds)",
                "4",
                &app.settings.poll_interval_secs,
                SettingsField::PollInterval,
                false,
                demo,
                None,
            )),
            Some(field_input(
                "Wait after detection (seconds)",
                "120",
                &app.settings.cooldown_secs,
                SettingsField::Cooldown,
                false,
                demo,
                None,
            )),
        ));
    }

    settings_card("Auto-detect", body.into())
}

fn sync_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let body = column![
        field_input(
            "Website URL",
            "https://your-site.com",
            &app.settings.sync_url,
            SettingsField::SyncUrl,
            false,
            demo,
            None,
        ),
        field_input(
            "Account token",
            "paste the token from the website",
            &app.settings.sync_token,
            SettingsField::SyncToken,
            true,
            demo,
            Some("Both needed to upload. Blank either to turn sync off."),
        ),
    ]
    .spacing(6);
    settings_card("Website sync", body.into())
}

fn ocr_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let installed = if app.tessdata_installed {
        "Reading model is installed"
    } else {
        "Reading model is missing — install it to read the scoreboard"
    };
    let mut actions = row![].spacing(8);
    if !demo && !app.tessdata_busy {
        actions = actions
            .push(action_btn(
                "Install reading model",
                true,
                Message::InstallModel,
            ))
            .push(action_btn(
                "Rebuild reading model",
                false,
                Message::RebuildModel,
            ));
    }
    let mut body = column![
        text(installed)
            .size(SIZE_BODY)
            .font(FONT_MEDIUM)
            .color(if app.tessdata_installed {
                theme::OK
            } else {
                theme::WARN
            }),
        text("Install if missing. Rebuild retrains (needs network, a few minutes).")
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    ]
    .spacing(6);
    if app.tessdata_busy {
        body = body.push(
            text("Working on the reading model…")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }
    body = body.push(actions);
    settings_card("Scoreboard reading", body.into())
}

fn diagnostics_card(app: &TrackerApp, _demo: bool) -> Element<'_, Message> {
    let body = column![
        checkbox(app.settings.debug_ocr)
            .label("Save debug images from each capture")
            .on_toggle(|v| Message::SettingsToggle(SettingsToggle::DebugOcr, v))
            .size(16)
            .text_size(SIZE_BODY)
            .style(checkbox_style),
        text("Writes images under the data folder. Slows capture — leave off unless diagnosing.")
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    ]
    .spacing(6);
    settings_card("Diagnostics", body.into())
}

fn data_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let running = app.daemon.running();
    let mut copy = column![
        text("Compact rewrites the store and keeps one backup. Delete removes local matches only — not the website.")
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    ]
    .spacing(6);

    if running {
        copy = copy.push(
            text("Stop the tracker before compact or delete.")
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(theme::WARN),
        );
    }
    if app.confirm_clear {
        copy = copy.push(
            text("Really delete all local match history? This cannot be undone.")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::DANGER),
        );
    }
    if app.vacuum_busy {
        copy = copy.push(
            text("Compacting…")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }

    let mut actions = row![].spacing(8).align_y(Alignment::Center);
    if !demo && !running && !app.vacuum_busy {
        actions = actions.push(action_btn("Compact stored data", false, Message::Vacuum));
        if app.confirm_clear {
            actions = actions
                .push(action_btn("Cancel", false, Message::ConfirmClear))
                .push(
                    button(
                        text("Yes, delete everything")
                            .size(SIZE_META)
                            .font(FONT_SEMIBOLD)
                            .color(TEXT),
                    )
                    .padding(Padding::from([8, 16]))
                    .style(theme::danger_btn(true))
                    .on_press(Message::ClearData),
                );
        } else {
            actions = actions.push(
                button(
                    text("Delete all local match data")
                        .size(SIZE_META)
                        .font(FONT_SEMIBOLD)
                        .color(theme::DANGER),
                )
                .padding(Padding::from([8, 16]))
                .style(theme::danger_btn(false))
                .on_press(Message::ConfirmClear),
            );
        }
    }

    let body = row![copy.width(Fill), actions]
        .align_y(Alignment::Start)
        .spacing(GRID_GAP)
        .width(Fill);
    settings_card("Stored data", body.into())
}

fn save_footer(demo: bool) -> Element<'static, Message> {
    if demo {
        return space().height(0).into();
    }
    container(
        row![
            text("Writes config and the companion shortcut.")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
            space().width(Fill),
            action_btn(SAVE_LABEL, true, Message::SaveSettings),
        ]
        .align_y(Alignment::Center)
        .spacing(GRID_GAP),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

fn settings_card<'a>(title: &'static str, body: Element<'a, Message>) -> Element<'a, Message> {
    container(
        column![
            text(title).size(SIZE_TITLE).font(FONT_BOLD).color(TEXT),
            body,
        ]
        .spacing(8),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

fn field_caption(label: &'static str) -> text::Text<'static> {
    text(label)
        .size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .color(TEXT_2)
}

fn hint(copy: impl Into<String>) -> text::Text<'static> {
    text(copy.into())
        .size(SIZE_LABEL)
        .font(FONT_MEDIUM)
        .color(TEXT_3)
}

/// One or two compact fields, left-aligned. Trailing Fill keeps them from
/// stretching across an ultrawide card.
fn compact_row<'a>(
    left: Option<Element<'a, Message>>,
    right: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut r = row![]
        .spacing(GRID_GAP)
        .align_y(Alignment::Start)
        .width(Fill);
    if let Some(left) = left {
        r = r.push(left);
    }
    if let Some(right) = right {
        r = r.push(right);
    }
    r.push(space().width(Fill)).into()
}

fn field_input<'a>(
    label: &'static str,
    placeholder: &'static str,
    value: &'a str,
    field: SettingsField,
    secret: bool,
    demo: bool,
    hint_copy: Option<&'static str>,
) -> Element<'a, Message> {
    let mut input = text_input(placeholder, value)
        .padding(Padding::from([8, 10]))
        .size(SIZE_BODY)
        .style(theme::text_input_style)
        .secure(secret)
        .width(Fill);
    if !demo {
        input = input.on_input(move |v| Message::SettingsText(field, v));
    }
    let mut col = column![field_caption(label), input].spacing(4);
    if let Some(copy) = hint_copy {
        col = col.push(hint(copy));
    }
    col.width(Fill).max_width(field_max_width(field)).into()
}

fn output_chip(
    label: impl Into<String>,
    selected: bool,
    msg: Message,
) -> Element<'static, Message> {
    widgets::filter_chip(label.into(), selected, msg)
}

fn action_btn(label: &'static str, primary: bool, msg: Message) -> Element<'static, Message> {
    let label = text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT);
    if primary {
        button(label)
            .padding(Padding::from([8, 16]))
            .style(theme::chip(true))
            .on_press(msg)
            .into()
    } else {
        button(label)
            .padding(Padding::from([8, 16]))
            .style(theme::ghost_btn())
            .on_press(msg)
            .into()
    }
}

fn checkbox_style(
    _theme: &iced::Theme,
    status: iced::widget::checkbox::Status,
) -> iced::widget::checkbox::Style {
    let selected = matches!(
        status,
        iced::widget::checkbox::Status::Active { is_checked: true }
            | iced::widget::checkbox::Status::Hovered { is_checked: true }
            | iced::widget::checkbox::Status::Disabled { is_checked: true }
    );
    iced::widget::checkbox::Style {
        background: iced::Background::Color(if selected { theme::ACCENT } else { theme::BG }),
        icon_color: TEXT,
        border: iced::Border {
            color: if selected {
                theme::ACCENT
            } else {
                theme::BORDER
            },
            width: 1.0,
            radius: theme::inner_radius(),
        },
        text_color: Some(TEXT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn base() -> Config {
        Config {
            data_dir: PathBuf::from("/tmp/sst-ui-settings-test"),
            ocr_threads: Some(2),
            ..Config::default()
        }
    }

    #[test]
    fn empty_optional_fields_become_none() {
        let form = SettingsForm {
            capture_output: "  ".into(),
            player_name: String::new(),
            session_window_secs: "1800".into(),
            game_process_names: String::new(),
            auto_detect_enabled: true,
            poll_interval_secs: "4".into(),
            cooldown_secs: "120".into(),
            sync_url: String::new(),
            sync_token: "tok".into(),
            debug_ocr: false,
            overlay_hotkey: crate::hotkey::DEFAULT_BIND.into(),
            overlay_hotkey_enabled: true,
        };
        let c = form.to_config(&base());
        assert!(c.capture_output.is_none());
        assert!(c.player_name.is_none());
        assert!(c.sync.is_none());
        assert!(c.game_process_names.is_empty());
        assert_eq!(c.data_dir, PathBuf::from("/tmp/sst-ui-settings-test"));
        assert_eq!(c.ocr_threads, Some(2));
    }

    #[test]
    fn sync_requires_both_fields() {
        assert!(sync_from_fields("", "tok").is_none());
        assert!(sync_from_fields("https://x", "").is_none());
        let s = sync_from_fields(" https://x ", " tok ").unwrap();
        assert_eq!(s.server_url, "https://x");
        assert_eq!(s.token, "tok");
    }

    #[test]
    fn process_names_split_comma_and_newline() {
        assert_eq!(
            parse_process_names("Overwatch.exe, wine"),
            vec!["Overwatch.exe", "wine"]
        );
        assert_eq!(
            parse_process_names("Overwatch.exe\nwine\n"),
            vec!["Overwatch.exe", "wine"]
        );
        assert!(parse_process_names("  ,  \n").is_empty());
    }

    #[test]
    fn number_fallbacks_match_config_defaults() {
        assert_eq!(parse_u64("nope", 1800), 1800);
        assert_eq!(parse_u64(" 90 ", 4), 90);
        let mut form = SettingsForm::from_config(&Config::default());
        form.poll_interval_secs = "x".into();
        form.cooldown_secs = "".into();
        form.session_window_secs = "oops".into();
        let c = form.to_config(&base());
        assert_eq!(c.auto_detect.poll_interval_secs, 4);
        assert_eq!(c.auto_detect.cooldown_secs, 120);
        assert_eq!(c.session_window_secs, 1800);
    }

    #[test]
    fn roundtrip_preserves_today_fields() {
        let mut base = base();
        base.capture_output = Some("DP-1".into());
        base.player_name = Some("Ada".into());
        base.sync = Some(SyncConfig {
            server_url: "https://crew.example".into(),
            token: "secret".into(),
        });
        base.auto_detect = AutoDetectConfig {
            enabled: false,
            poll_interval_secs: 8,
            cooldown_secs: 60,
        };
        base.session_window_secs = 900;
        base.game_process_names = vec!["Overwatch.exe".into()];
        base.debug_ocr = true;
        let form = SettingsForm::from_config(&base);
        let out = form.to_config(&base);
        assert_eq!(out.capture_output.as_deref(), Some("DP-1"));
        assert_eq!(out.player_name.as_deref(), Some("Ada"));
        assert_eq!(
            out.sync.as_ref().map(|s| s.server_url.as_str()),
            Some("https://crew.example")
        );
        assert_eq!(out.auto_detect.poll_interval_secs, 8);
        assert_eq!(out.session_window_secs, 900);
        assert_eq!(out.game_process_names, vec!["Overwatch.exe"]);
        assert!(out.debug_ocr);
        assert_eq!(out.ocr_threads, Some(2));
        assert_eq!(form.overlay_hotkey, crate::hotkey::DEFAULT_BIND);
        assert!(form.overlay_hotkey_enabled);
    }

    #[test]
    fn short_numeric_fields_do_not_span_ultrawide() {
        for field in [
            SettingsField::SessionWindow,
            SettingsField::PollInterval,
            SettingsField::Cooldown,
        ] {
            assert!(
                field_max_width(field) <= FIELD_NUMERIC,
                "{field:?} must stay a compact numeric"
            );
        }
        assert_eq!(field_max_width(SettingsField::SessionWindow), FIELD_NUMERIC);
        assert_eq!(field_max_width(SettingsField::PollInterval), FIELD_NUMERIC);
        assert_eq!(field_max_width(SettingsField::Cooldown), FIELD_NUMERIC);
    }

    #[test]
    fn settings_cards_use_maps_density_tokens() {
        use crate::theme::{PAGE_PAD_X, PAGE_PAD_Y};
        assert_eq!(PAGE_PAD_Y, 24.0);
        assert_eq!(PAGE_PAD_X, 32.0);
        assert_eq!(PAD_INNER, 12.0);
        assert_eq!(theme::RADIUS_CARD, 16.0);
        assert_eq!(theme::GRID_GAP, 12.0);
        assert_eq!(theme::SIZE_LABEL, 11.0);
        assert_eq!(theme::ACCENT, {
            iced::Color::from_rgb(
                0x8f as f32 / 255.0,
                0x73 as f32 / 255.0,
                0xff as f32 / 255.0,
            )
        });
    }

    #[test]
    fn settings_section_grid_is_one_or_two_columns() {
        use crate::layout::settings_columns;
        assert_eq!(settings_columns(200.0), 1);
        assert_eq!(settings_columns(480.0), 1);
        assert_eq!(settings_columns(972.0), 2);
        assert_eq!(settings_columns(10_000.0), 2);
    }

    #[test]
    fn text_fields_are_capped_but_wider_than_numerics() {
        assert_eq!(field_max_width(SettingsField::SyncUrl), FIELD_TEXT);
        assert_eq!(field_max_width(SettingsField::SyncToken), FIELD_TEXT);
        assert_eq!(field_max_width(SettingsField::ProcessNames), FIELD_TEXT);
        assert_eq!(field_max_width(SettingsField::PlayerName), FIELD_SHORT);
        assert_eq!(field_max_width(SettingsField::OverlayHotkey), FIELD_SHORT);
    }

    #[test]
    fn masonry_omits_stored_data_and_keeps_every_other_section() {
        assert_eq!(MASONRY_SECTIONS.len(), 8);
        assert_eq!(SPANNING_SECTION, SettingsSection::Data);
        assert!(!MASONRY_SECTIONS.contains(&SettingsSection::Data));
        let all = [
            SettingsSection::Daemon,
            SettingsSection::Capture,
            SettingsSection::Companion,
            SettingsSection::Player,
            SettingsSection::AutoDetect,
            SettingsSection::Sync,
            SettingsSection::Ocr,
            SettingsSection::Diagnostics,
            SettingsSection::Data,
        ];
        for section in all {
            assert!(
                MASONRY_SECTIONS.contains(&section) || section == SPANNING_SECTION,
                "missing {section:?}"
            );
        }
    }

    #[test]
    fn two_column_pack_balances_and_leaves_no_empty_column() {
        let packed = pack_columns(MASONRY_SECTIONS, 2);
        assert_eq!(packed.len(), 2);
        assert_eq!(
            packed[0],
            [
                SettingsSection::Daemon,
                SettingsSection::Companion,
                SettingsSection::AutoDetect,
                SettingsSection::Ocr,
            ]
        );
        assert_eq!(
            packed[1],
            [
                SettingsSection::Capture,
                SettingsSection::Player,
                SettingsSection::Sync,
                SettingsSection::Diagnostics,
            ]
        );
        assert_eq!(column_weight(&packed[0]), column_weight(&packed[1]));
        assert!(!packed[0].contains(&SettingsSection::Data));
        assert!(!packed[1].contains(&SettingsSection::Data));
    }

    #[test]
    fn one_column_pack_keeps_input_order() {
        let packed = pack_columns(MASONRY_SECTIONS, 1);
        assert_eq!(packed, vec![MASONRY_SECTIONS.to_vec()]);
    }

    #[test]
    fn save_footer_label_stays_primary_action() {
        assert_eq!(SAVE_LABEL, "Save settings");
        assert_eq!(theme::SIZE_LABEL, 11.0);
    }
}
