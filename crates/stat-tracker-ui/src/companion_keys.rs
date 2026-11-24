//! Companion page keys and the "Hold the stream page" switch (0.7.1 C5).
//!
//! Settings live in `ui_state.json` with the hide bind, so they never count
//! as changes waiting: they are not `config.toml` keys. The desktop app
//! handles the evdev binds and the `companion_next`, `companion_prev` and
//! `companion_safe` file triggers, writes `<data_dir>/companion_page.json`,
//! and applies auto-advance from the status file's `companion_stage` (the C3
//! shape). The companion process reads the page file on its tick.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use iced::{Subscription, Task};
use serde::Deserialize;

use crate::app::{Message, TrackerApp};
use crate::hotkey::{self, HotkeyAction};
use crate::overlay_pages::{self, PageState, Stage, vote_maps_read};
use crate::seasons;

/// Q6 choice: the spec names Super+Shift+T for safe-page, but not next or
/// previous. These two parse with the existing bind parser and do not reuse
/// C (hide) or T (safe-page).
pub const DEFAULT_NEXT_BIND: &str = "Super+Shift+N";
pub const DEFAULT_PREV_BIND: &str = "Super+Shift+P";
/// The safe-page bind the spec names.
pub const DEFAULT_SAFE_BIND: &str = overlay_pages::DEFAULT_SAFE_BIND;

pub const NEXT_TRIGGER: &str = "companion_next";
pub const PREV_TRIGGER: &str = "companion_prev";
pub const SAFE_TRIGGER: &str = "companion_safe";

/// How often the companion re-reads the page file, so a key press is not
/// a second late. Snapshot refresh still short-circuits on mtime.
pub const COMPANION_TICK: std::time::Duration = std::time::Duration::from_millis(250);

/// Next, previous and safe-page binds, plus "Hold the stream page".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageKeys {
    pub next_bind: String,
    pub prev_bind: String,
    pub safe_bind: String,
    pub hold_stream_page: bool,
}

impl Default for PageKeys {
    fn default() -> Self {
        Self {
            next_bind: DEFAULT_NEXT_BIND.to_string(),
            prev_bind: DEFAULT_PREV_BIND.to_string(),
            safe_bind: DEFAULT_SAFE_BIND.to_string(),
            hold_stream_page: false,
        }
    }
}

impl PageKeys {
    /// Trim; blank or unparsable binds fall back to the default for that key.
    pub fn normalized(
        next_bind: &str,
        prev_bind: &str,
        safe_bind: &str,
        hold_stream_page: bool,
    ) -> Self {
        Self {
            next_bind: normalize_bind(next_bind, DEFAULT_NEXT_BIND),
            prev_bind: normalize_bind(prev_bind, DEFAULT_PREV_BIND),
            safe_bind: normalize_bind(safe_bind, DEFAULT_SAFE_BIND),
            hold_stream_page,
        }
    }

    /// `Ok` with a display form, or `Err` when any typed bind is invalid.
    /// A blank field falls back to that key's default.
    pub fn validate_for_save(&self) -> Result<Self, String> {
        Ok(Self {
            next_bind: validate_bind("next page", &self.next_bind, DEFAULT_NEXT_BIND)?,
            prev_bind: validate_bind("previous page", &self.prev_bind, DEFAULT_PREV_BIND)?,
            safe_bind: validate_bind("safe page", &self.safe_bind, DEFAULT_SAFE_BIND)?,
            hold_stream_page: self.hold_stream_page,
        })
    }
}

fn normalize_bind(raw: &str, default: &str) -> String {
    validate_bind("bind", raw, default).unwrap_or_else(|_| default.to_string())
}

fn validate_bind(name: &str, raw: &str, default: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(default.to_string());
    }
    hotkey::parse_bind(trimmed)
        .map(|bind| bind.to_string())
        .map_err(|e| format!("{name}: {e}"))
}

/// A next, previous or safe-page command from a key or a file trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageCmd {
    Next,
    Prev,
    Safe,
}

impl PageCmd {
    pub fn apply(self, state: PageState) -> PageState {
        match self {
            PageCmd::Next => overlay_pages::next_page(state),
            PageCmd::Prev => overlay_pages::previous_page(state),
            PageCmd::Safe => overlay_pages::safe_page(state),
        }
    }
}

/// Last seen mtimes of the three page-file triggers.
#[derive(Debug, Clone, Default)]
pub struct TriggerMtimes {
    pub next: Option<SystemTime>,
    pub prev: Option<SystemTime>,
    pub safe: Option<SystemTime>,
}

pub fn page_file(data_dir: &Path) -> PathBuf {
    data_dir.join("companion_page.json")
}

pub fn trigger_path(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join(name)
}

/// Missing file / unreadable file -> the default Tonight page.
pub fn load_page_state(data_dir: &Path) -> PageState {
    let Ok(bytes) = std::fs::read(page_file(data_dir)) else {
        return PageState::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save_page_state(data_dir: &Path, state: PageState) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let bytes = serde_json::to_vec_pretty(&state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(page_file(data_dir), bytes)
}

pub fn load_page_keys(data_dir: &Path) -> PageKeys {
    seasons::load_page_keys(data_dir)
}

pub fn save_page_keys(data_dir: &Path, keys: &PageKeys) -> std::io::Result<()> {
    seasons::save_page_keys(data_dir, keys)
}

/// Hide bind plus the three page binds, when the companion is enabled.
pub fn hotkey_subscriptions(app: &TrackerApp) -> Vec<Subscription<Message>> {
    let mut out = Vec::new();
    if !app.companion_enabled() {
        return out;
    }
    if app.overlay_hotkey.enabled
        && let Ok(bind) = hotkey::parse_bind(&app.overlay_hotkey.bind)
    {
        out.push(hotkey::subscription(bind));
    }
    out.extend(page_subscriptions(&app.page_keys));
    out
}

/// Evdev subscriptions for the three page binds.
pub fn page_subscriptions(keys: &PageKeys) -> Vec<Subscription<Message>> {
    let mut out = Vec::new();
    let binds = [
        (keys.next_bind.as_str(), HotkeyAction::NextPage),
        (keys.prev_bind.as_str(), HotkeyAction::PrevPage),
        (keys.safe_bind.as_str(), HotkeyAction::SafePage),
    ];
    for (bind, action) in binds {
        if let Ok(parsed) = hotkey::parse_bind(bind) {
            out.push(hotkey::subscription_with(parsed, action));
        }
    }
    out
}

/// True when `path` mtime advanced since `last`. An existing file at first
/// sight does not fire (startup), same as [`hotkey::consume_toggle`].
pub fn consume_trigger(path: &Path, last: &mut Option<SystemTime>) -> bool {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    match (mtime, *last) {
        (Some(now), Some(prev)) if now > prev => {
            *last = Some(now);
            true
        }
        (Some(now), None) => {
            *last = Some(now);
            false
        }
        (None, _) => {
            *last = None;
            false
        }
        _ => false,
    }
}

/// The first page trigger whose file mtime advanced, in next, previous,
/// safe-page order.
pub fn consume_page_triggers(data_dir: &Path, last: &mut TriggerMtimes) -> Option<PageCmd> {
    if consume_trigger(&trigger_path(data_dir, NEXT_TRIGGER), &mut last.next) {
        return Some(PageCmd::Next);
    }
    if consume_trigger(&trigger_path(data_dir, PREV_TRIGGER), &mut last.prev) {
        return Some(PageCmd::Prev);
    }
    if consume_trigger(&trigger_path(data_dir, SAFE_TRIGGER), &mut last.safe) {
        return Some(PageCmd::Safe);
    }
    None
}

/// Map the status file's `companion_stage` (C3) to a page-model stage.
///
/// Missing file, missing field, or `unknown` -> `None`, so the last page
/// stays. Hero select and hero ban are `MapLocked { map_read: true }`:
/// C3 does not publish the locked map name, and the stage itself means
/// the map has locked.
pub fn stage_from_status(data_dir: &Path) -> Option<Stage> {
    let bytes = std::fs::read(stat_tracker::ocr_status::status_path(data_dir)).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let stage = value.get("companion_stage")?;
    if stage.is_null() {
        return None;
    }
    let dto: StageDto = serde_json::from_value(stage.clone()).ok()?;
    match dto.kind.as_str() {
        "map_vote" => Some(Stage::MapVote {
            maps_read: vote_maps_read(&dto.maps),
        }),
        "hero_select" | "hero_ban" => Some(Stage::MapLocked { map_read: true }),
        "in_game" => Some(Stage::MatchStart),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct StageDto {
    kind: String,
    #[serde(default)]
    maps: Vec<String>,
}

pub fn apply_status(state: PageState, data_dir: &Path, hold_stream_page: bool) -> PageState {
    match stage_from_status(data_dir) {
        Some(stage) => overlay_pages::on_stage(state, stage, hold_stream_page),
        None => state,
    }
}

impl TrackerApp {
    /// Apply a page command and write the page file at once.
    pub(crate) fn apply_companion_page(&mut self, cmd: PageCmd) -> Task<Message> {
        let state = cmd.apply(load_page_state(&self.data_dir));
        if let Err(e) = save_page_state(&self.data_dir, state) {
            tracing::warn!(error = %e, "failed to write companion_page.json");
        }
        Task::none()
    }

    /// File triggers and auto-advance from the status file.
    pub(crate) fn tick_companion_pages(&mut self) {
        if let Some(cmd) = consume_page_triggers(&self.data_dir, &mut self.page_triggers) {
            let _ = self.apply_companion_page(cmd);
        }
        let state = load_page_state(&self.data_dir);
        let next = apply_status(state, &self.data_dir, self.page_keys.hold_stream_page);
        if next != state
            && let Err(e) = save_page_state(&self.data_dir, next)
        {
            tracing::warn!(error = %e, "failed to write companion_page.json");
        }
    }
}

/// A bind the Settings page is editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageBind {
    Next,
    Prev,
    Safe,
}

impl PageBind {
    pub fn get(self, keys: &PageKeys) -> &str {
        match self {
            PageBind::Next => &keys.next_bind,
            PageBind::Prev => &keys.prev_bind,
            PageBind::Safe => &keys.safe_bind,
        }
    }

    pub fn set(self, keys: &mut PageKeys, value: String) {
        match self {
            PageBind::Next => keys.next_bind = value,
            PageBind::Prev => keys.prev_bind = value,
            PageBind::Safe => keys.safe_bind = value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay_pages::Page;

    fn parse_ok(raw: &str) -> String {
        hotkey::parse_bind(raw).expect(raw).to_string()
    }

    #[test]
    fn default_binds_parse_and_normalize() {
        assert_eq!(parse_ok(DEFAULT_NEXT_BIND), "Super+Shift+N");
        assert_eq!(parse_ok(DEFAULT_PREV_BIND), "Super+Shift+P");
        assert_eq!(parse_ok(DEFAULT_SAFE_BIND), "Super+Shift+T");
        let keys = PageKeys::normalized(" super+shift+n ", "", "nope", true);
        assert_eq!(keys.next_bind, "Super+Shift+N");
        assert_eq!(keys.prev_bind, DEFAULT_PREV_BIND);
        assert_eq!(keys.safe_bind, DEFAULT_SAFE_BIND);
        assert!(keys.hold_stream_page);
        assert!(keys.validate_for_save().is_ok());
        // A letter with no modifier is not a valid bind.
        let bad = PageKeys {
            next_bind: "N".into(),
            ..PageKeys::default()
        };
        assert!(bad.validate_for_save().is_err());
    }

    #[test]
    fn file_triggers_fire_on_a_later_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let mut last = TriggerMtimes::default();
        assert_eq!(consume_page_triggers(dir.path(), &mut last), None);
        let next = trigger_path(dir.path(), NEXT_TRIGGER);
        std::fs::write(&next, b"").unwrap();
        // First sight of an existing file does not fire.
        assert_eq!(consume_page_triggers(dir.path(), &mut last), None);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&next, b"").unwrap();
        assert_eq!(
            consume_page_triggers(dir.path(), &mut last),
            Some(PageCmd::Next)
        );
        assert_eq!(consume_page_triggers(dir.path(), &mut last), None);

        let prev = trigger_path(dir.path(), PREV_TRIGGER);
        std::fs::write(&prev, b"").unwrap();
        assert_eq!(consume_page_triggers(dir.path(), &mut last), None);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&prev, b"").unwrap();
        assert_eq!(
            consume_page_triggers(dir.path(), &mut last),
            Some(PageCmd::Prev)
        );

        let safe = trigger_path(dir.path(), SAFE_TRIGGER);
        std::fs::write(&safe, b"").unwrap();
        assert_eq!(consume_page_triggers(dir.path(), &mut last), None);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&safe, b"").unwrap();
        assert_eq!(
            consume_page_triggers(dir.path(), &mut last),
            Some(PageCmd::Safe)
        );
    }

    #[test]
    fn hold_plus_next_page_clears_the_hold() {
        let held = overlay_pages::safe_page(PageState::default());
        assert!(held.held);
        assert_eq!(held.page, Page::Tonight);
        let next = PageCmd::Next.apply(held);
        assert!(!next.held);
        assert_eq!(next.page, Page::MapVote);
    }

    #[test]
    fn hold_the_stream_page_suppresses_auto_advance() {
        let dir = tempfile::tempdir().unwrap();
        let status = stat_tracker::ocr_status::status_path(dir.path());
        std::fs::write(
            &status,
            r#"{"companion_stage":{"kind":"map_vote","maps":["Busan","Ilios","Lijiang Tower"],"since":1}}"#,
        )
        .unwrap();
        let start = PageState::default();
        let held = apply_status(start, dir.path(), true);
        assert_eq!(held.page, Page::Tonight);
        assert_eq!(held.stage, Stage::MapVote { maps_read: true });
        let advanced = apply_status(start, dir.path(), false);
        assert_eq!(advanced.page, Page::MapVote);
    }

    #[test]
    fn status_maps_lock_and_match_start() {
        let dir = tempfile::tempdir().unwrap();
        let status = stat_tracker::ocr_status::status_path(dir.path());
        std::fs::write(
            &status,
            r#"{"companion_stage":{"kind":"hero_select","maps":[],"since":2}}"#,
        )
        .unwrap();
        let start = PageState::default();
        let locked = apply_status(start, dir.path(), false);
        assert_eq!(locked.page, Page::PicksAndBans);
        assert_eq!(locked.stage, Stage::MapLocked { map_read: true });

        std::fs::write(
            &status,
            r#"{"companion_stage":{"kind":"in_game","maps":[],"since":3}}"#,
        )
        .unwrap();
        let back = apply_status(locked, dir.path(), true);
        assert_eq!(back.page, Page::Tonight);
        assert_eq!(back.stage, Stage::MatchStart);
    }

    #[test]
    fn page_file_round_trips_and_unknown_status_leaves_the_page() {
        let dir = tempfile::tempdir().unwrap();
        let state = overlay_pages::safe_page(PageState {
            page: Page::Tonight,
            held: false,
            stage: Stage::MapVote { maps_read: true },
            ..PageState::default()
        });
        save_page_state(dir.path(), state).unwrap();
        assert_eq!(load_page_state(dir.path()), state);

        let status = stat_tracker::ocr_status::status_path(dir.path());
        std::fs::write(
            &status,
            r#"{"companion_stage":{"kind":"unknown","maps":[],"since":9}}"#,
        )
        .unwrap();
        assert_eq!(apply_status(state, dir.path(), false), state);
        std::fs::write(&status, r#"{}"#).unwrap();
        assert_eq!(apply_status(state, dir.path(), false), state);
    }
}
