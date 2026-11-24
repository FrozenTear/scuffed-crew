//! Companion page keys and the "Hold the stream page" switch (0.7.1 C5).
//!
//! Settings live in `ui_state.json` with the hide bind, so they never count
//! as changes waiting: they are not `config.toml` keys. The desktop app
//! handles the evdev binds and the `companion_next`, `companion_prev` and
//! `companion_safe` file triggers, writes `<data_dir>/companion_page.json`,
//! and applies auto-advance from the status file's `companion_stage` (the C3
//! shape), through `read_companion_stage` and the C3 freshness bound. A
//! missing pid, a pid that does not match, a stale `seen`, or a `seen`
//! more than [`stat_tracker::companion_stage::SEEN_FUTURE_SLACK_SECS`]
//! ahead is ignored, so the last page stays. The companion process
//! reads the page file on its tick.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use iced::{Subscription, Task};

use crate::app::{Message, TrackerApp};
use crate::hotkey::{self, HotkeyAction};
use crate::overlay_pages::{self, PageState, Stage, vote_maps_read};
use crate::seasons;

/// Q6 choice: the spec names Super+Shift+T for safe-page, but not next or
/// previous. N and B parse with the existing bind parser, do not reuse C
/// (hide) or T (safe-page), and are not stock niri default binds. Super+Shift+P
/// is niri's power-off-monitors.
pub const DEFAULT_NEXT_BIND: &str = "Super+Shift+N";
pub const DEFAULT_PREV_BIND: &str = "Super+Shift+B";
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

/// A new companion starts on Tonight, keeping the hold, the last stage
/// and `in_match`. Launch is when a stream is most likely up.
pub fn start_companion_pages(data_dir: &Path) -> PageState {
    let state = PageState {
        page: overlay_pages::Page::Tonight,
        ..load_page_state(data_dir)
    };
    if let Err(e) = save_page_state(data_dir, state) {
        tracing::warn!(error = %e, "failed to write companion_page.json");
    }
    state
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
/// Reads through [`stat_tracker::companion_stage::read_companion_stage`]
/// and ignores a stage that is not
/// [`stat_tracker::companion_stage::stage_is_current`] (a stale `seen`,
/// or a `seen` more than
/// [`stat_tracker::companion_stage::SEEN_FUTURE_SLACK_SECS`] ahead).
/// Missing file, missing field, no live pid, a pid that does not
/// match, a rejected `seen`, or `unknown` -> `None`, so the last page
/// stays. Hero select
/// and hero ban are `MapLocked { map_read: false }` until detect produces
/// InGame or another match-on signal, so the page does not auto-advance
/// to Bans and picks (Tonight comes back, or stays, through a lock and a
/// later unread vote). Flip `map_read` in the PR that adds InGame.
pub fn stage_from_status(
    data_dir: &Path,
    live_pid: Option<u32>,
    now: i64,
    poll_interval: Duration,
) -> Option<Stage> {
    let pid = live_pid?;
    let stage = stat_tracker::companion_stage::read_companion_stage(data_dir, pid)?;
    if !stat_tracker::companion_stage::stage_is_current(stage.seen, now, poll_interval) {
        return None;
    }
    match stage.kind {
        stat_tracker::companion_stage::StageKind::MapVote => Some(Stage::MapVote {
            maps_read: vote_maps_read(&stage.maps),
        }),
        // map_read stays false until detect produces InGame (or another
        // match-on signal). Auto-advance to Bans and picks would leave
        // that page up through the match, because InGame is not produced
        // today and C3 leaves the last lock in the status file.
        stat_tracker::companion_stage::StageKind::HeroSelect
        | stat_tracker::companion_stage::StageKind::HeroBan => {
            Some(Stage::MapLocked { map_read: false })
        }
        stat_tracker::companion_stage::StageKind::InGame => Some(Stage::MatchStart),
        stat_tracker::companion_stage::StageKind::Unknown => None,
    }
}

pub fn apply_status(
    state: PageState,
    data_dir: &Path,
    live_pid: Option<u32>,
    now: i64,
    poll_interval: Duration,
    hold_stream_page: bool,
) -> PageState {
    match stage_from_status(data_dir, live_pid, now, poll_interval) {
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
        let now = chrono::Utc::now().timestamp();
        let poll = self
            .saved_config
            .auto_detect
            .poll_interval(self.saved_config.reader.is_new());
        let next = apply_status(
            state,
            &self.data_dir,
            self.daemon.pid,
            now,
            poll,
            self.page_keys.hold_stream_page,
        );
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

    fn draft(self, drafts: &PageBindDrafts) -> Option<&str> {
        match self {
            PageBind::Next => drafts.next.as_deref(),
            PageBind::Prev => drafts.prev.as_deref(),
            PageBind::Safe => drafts.safe.as_deref(),
        }
    }

    pub fn set_draft(self, drafts: &mut PageBindDrafts, value: String) {
        match self {
            PageBind::Next => drafts.next = Some(value),
            PageBind::Prev => drafts.prev = Some(value),
            PageBind::Safe => drafts.safe = Some(value),
        }
    }

    pub fn clear_draft(self, drafts: &mut PageBindDrafts) {
        match self {
            PageBind::Next => drafts.next = None,
            PageBind::Prev => drafts.prev = None,
            PageBind::Safe => drafts.safe = None,
        }
    }

    /// The field text: a draft while typing, otherwise the committed bind.
    pub fn shown<'a>(self, keys: &'a PageKeys, drafts: &'a PageBindDrafts) -> &'a str {
        self.draft(drafts).unwrap_or_else(|| self.get(keys))
    }
}

/// Typed page-bind fields that are not yet committed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageBindDrafts {
    pub next: Option<String>,
    pub prev: Option<String>,
    pub safe: Option<String>,
}

fn bind_key(raw: &str) -> Option<String> {
    hotkey::parse_bind(raw).ok().map(|b| b.to_string())
}

/// Commit one page bind. A blank draft is refused, and a bind that matches
/// the hide bind or another page bind is refused.
pub fn commit_page_bind(
    keys: &PageKeys,
    kind: PageBind,
    raw: &str,
    hide_bind: &str,
) -> Result<PageKeys, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("The shortcut cannot be blank.".into());
    }
    let parsed = hotkey::parse_bind(trimmed)?;
    let parsed = parsed.to_string();
    if bind_key(hide_bind).as_deref() == Some(parsed.as_str()) {
        return Err("That shortcut is already the hide bind.".into());
    }
    for other in [PageBind::Next, PageBind::Prev, PageBind::Safe] {
        if other != kind && bind_key(other.get(keys)).as_deref() == Some(parsed.as_str()) {
            return Err("That shortcut is already used by another page key.".into());
        }
    }
    let mut next = keys.clone();
    kind.set(&mut next, parsed);
    Ok(next)
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
        assert_eq!(parse_ok(DEFAULT_PREV_BIND), "Super+Shift+B");
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
        let keys = PageKeys::default();
        assert_eq!(
            commit_page_bind(&keys, PageBind::Next, "  ", "Super+Shift+C").unwrap_err(),
            "The shortcut cannot be blank."
        );
        assert_eq!(
            commit_page_bind(&keys, PageBind::Next, "Super+Shift+C", "Super+Shift+C").unwrap_err(),
            "That shortcut is already the hide bind."
        );
        assert_eq!(
            commit_page_bind(&keys, PageBind::Prev, "Super+Shift+T", "Super+Shift+C").unwrap_err(),
            "That shortcut is already used by another page key."
        );
        let saved =
            commit_page_bind(&keys, PageBind::Next, "Super+Shift+J", "Super+Shift+C").unwrap();
        assert_eq!(saved.next_bind, "Super+Shift+J");
        assert_eq!(saved.prev_bind, DEFAULT_PREV_BIND);
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

    const LIVE_PID: u32 = 7;
    const NOW: i64 = 1_000;

    fn poll() -> std::time::Duration {
        stat_tracker::config::NEW_READER_POLL_INTERVAL
    }

    fn write_stage(dir: &std::path::Path, pid: u32, kind: &str, maps: &str, seen: i64) {
        let status = stat_tracker::ocr_status::status_path(dir);
        std::fs::write(
            status,
            format!(
                r#"{{"pid":{pid},"companion_stage":{{"kind":"{kind}","maps":{maps},"since":{seen},"seen":{seen}}}}}"#
            ),
        )
        .unwrap();
    }

    fn apply_now(state: PageState, dir: &std::path::Path, hold: bool) -> PageState {
        apply_status(state, dir, Some(LIVE_PID), NOW, poll(), hold)
    }

    #[test]
    fn hold_the_stream_page_suppresses_auto_advance() {
        let dir = tempfile::tempdir().unwrap();
        write_stage(
            dir.path(),
            LIVE_PID,
            "map_vote",
            r#"["Busan","Ilios","Lijiang Tower"]"#,
            NOW,
        );
        let start = PageState::default();
        let held = apply_now(start, dir.path(), true);
        assert_eq!(held.page, Page::Tonight);
        assert_eq!(held.stage, Stage::MapVote { maps_read: true });
        let advanced = apply_now(start, dir.path(), false);
        assert_eq!(advanced.page, Page::MapVote);
    }

    #[test]
    fn status_maps_lock_and_match_start() {
        let dir = tempfile::tempdir().unwrap();
        write_stage(dir.path(), LIVE_PID, "hero_select", "[]", NOW);
        let start = PageState::default();
        let locked = apply_now(start, dir.path(), false);
        assert_eq!(locked.page, Page::Tonight);
        assert_eq!(locked.stage, Stage::MapLocked { map_read: false });

        write_stage(dir.path(), LIVE_PID, "in_game", "[]", NOW);
        let back = apply_now(locked, dir.path(), true);
        assert_eq!(back.page, Page::Tonight);
        assert_eq!(back.stage, Stage::MatchStart);
    }

    #[test]
    fn hero_select_then_unread_vote_leaves_tonight() {
        let dir = tempfile::tempdir().unwrap();
        write_stage(dir.path(), LIVE_PID, "hero_select", "[]", NOW);
        let s = apply_now(PageState::default(), dir.path(), false);
        assert_eq!(s.page, Page::Tonight);
        write_stage(dir.path(), LIVE_PID, "map_vote", "[]", NOW);
        let s = apply_now(s, dir.path(), false);
        assert_eq!(s.page, Page::Tonight);
        assert_ne!(s.page, Page::PicksAndBans);
        assert!(!s.in_match);
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

        write_stage(dir.path(), LIVE_PID, "unknown", "[]", NOW);
        assert_eq!(apply_now(state, dir.path(), false), state);
        std::fs::write(stat_tracker::ocr_status::status_path(dir.path()), r#"{}"#).unwrap();
        assert_eq!(apply_now(state, dir.path(), false), state);
    }

    #[test]
    fn a_stale_seen_or_wrong_pid_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let start = PageState::default();
        let bound = stat_tracker::companion_stage::seen_current_for_secs(poll());
        // Age bound+1 is stale (10 s at the new reader's 1.5 s poll, then +1).
        write_stage(
            dir.path(),
            LIVE_PID,
            "map_vote",
            r#"["Busan","Ilios","Lijiang Tower"]"#,
            NOW - (bound + 1),
        );
        assert_eq!(apply_now(start, dir.path(), false), start);

        write_stage(
            dir.path(),
            LIVE_PID,
            "map_vote",
            r#"["Busan","Ilios","Lijiang Tower"]"#,
            NOW,
        );
        let wrong = apply_status(start, dir.path(), Some(LIVE_PID + 1), NOW, poll(), false);
        assert_eq!(wrong, start);
        let none = apply_status(start, dir.path(), None, NOW, poll(), false);
        assert_eq!(none, start);
        let current = apply_now(start, dir.path(), false);
        assert_eq!(current.page, Page::MapVote);
    }

    #[test]
    fn a_seen_further_ahead_than_the_slack_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let start = PageState::default();
        let slack = stat_tracker::companion_stage::SEEN_FUTURE_SLACK_SECS;
        write_stage(
            dir.path(),
            LIVE_PID,
            "map_vote",
            r#"["Busan","Ilios","Lijiang Tower"]"#,
            NOW + slack + 1,
        );
        assert_eq!(apply_now(start, dir.path(), false), start);

        write_stage(
            dir.path(),
            LIVE_PID,
            "map_vote",
            r#"["Busan","Ilios","Lijiang Tower"]"#,
            NOW + 1,
        );
        let within = apply_now(start, dir.path(), false);
        assert_eq!(within.page, Page::MapVote);
    }
}
