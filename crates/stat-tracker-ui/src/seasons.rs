//! Server seasons: fetch / cache / selection / Seasons screen (design §4.4, §5).
//!
//! `GET /api/public/seasons` on launch and every 30 minutes. Cached to
//! `<data_dir>/seasons.json`. Header selection persists in `<data_dir>/ui_state.json`.
//! Fixture mode never hits the network. Aggregation is half-open
//! `starts_at <= played_at < ends_at` (UTC) — same as the website.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use iced::widget::{Row, column, mouse_area, space, text};
use iced::{Element, Fill};
use scuffed_types::Season;
use serde::{Deserialize, Serialize};

use crate::aggregate::{Record, SeasonWindow, aggregate};
use crate::app::{Message, TrackerApp};
use crate::hotkey::{self, OverlayHotkey};
use crate::layout::seasons_columns;
use crate::model::{Game, SeasonSel};
use crate::theme::{
    FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, GRID_GAP, SIZE_FEATURED, SIZE_META, SIZE_TITLE, TEXT,
    TEXT_2, TEXT_3,
};
use crate::widgets;

/// How often live mode re-GETs `/api/public/seasons`.
pub const REFRESH_EVERY: chrono::TimeDelta = chrono::TimeDelta::minutes(30);

#[derive(Debug, Clone, Default)]
pub struct SeasonCache {
    pub seasons: Vec<Season>,
    pub fetched_at: Option<DateTime<Utc>>,
    pub from_network: bool,
}

/// One row on the Seasons screen (all-time first, then each server season).
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonRow {
    pub sel: SeasonSel,
    pub name: String,
    pub window_label: Option<String>,
    pub is_current: bool,
    pub record: Record,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UiStateFile {
    /// `None` / omitted = all time. Some(id) = that season.
    #[serde(default)]
    season: Option<String>,
    /// Session-scoped companion hide (`session_id` / `process`). `None` = auto.
    #[serde(default)]
    overlay_hidden_key: Option<String>,
    /// Companion show/hide shortcut (`Super+Shift+C`). Missing → default.
    #[serde(default)]
    overlay_hotkey: Option<String>,
    /// Missing → on. The overlay stays click-through; this is an evdev bind.
    #[serde(default)]
    overlay_hotkey_enabled: Option<bool>,
}

pub fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("seasons.json")
}

pub fn ui_state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ui_state.json")
}

pub fn load_cache(data_dir: &Path) -> SeasonCache {
    let path = cache_path(data_dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return SeasonCache::default();
    };
    let Ok(seasons) = serde_json::from_slice::<Vec<Season>>(&bytes) else {
        return SeasonCache::default();
    };
    let fetched_at = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(system_time_to_utc);
    SeasonCache {
        seasons,
        fetched_at,
        from_network: false,
    }
}

pub fn write_cache(data_dir: &Path, seasons: &[Season]) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let bytes = serde_json::to_vec_pretty(seasons)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(cache_path(data_dir), bytes)
}

fn read_ui_state_file(data_dir: &Path) -> Option<UiStateFile> {
    let bytes = std::fs::read(ui_state_path(data_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_ui_state_file(data_dir: &Path, file: &UiStateFile) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let bytes = serde_json::to_vec_pretty(file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(ui_state_path(data_dir), bytes)
}

/// `None` when the file is missing or unreadable — caller applies
/// [`default_selection`]. A present file with `season: null` is All time.
pub fn load_ui_state(data_dir: &Path) -> Option<SeasonSel> {
    let file = read_ui_state_file(data_dir)?;
    Some(match file.season.filter(|id| !id.is_empty()) {
        Some(id) => SeasonSel::Season(id),
        None => SeasonSel::AllTime,
    })
}

pub fn save_ui_state(data_dir: &Path, sel: &SeasonSel) -> std::io::Result<()> {
    let mut file = read_ui_state_file(data_dir).unwrap_or_default();
    file.season = sel.as_id().map(str::to_string);
    write_ui_state_file(data_dir, &file)
}

/// Missing file / missing field → Auto (show while the game is running).
pub fn load_overlay_hidden_key(data_dir: &Path) -> Option<String> {
    read_ui_state_file(data_dir).and_then(|f| f.overlay_hidden_key)
}

pub fn save_overlay_hidden_key(data_dir: &Path, key: Option<&str>) -> std::io::Result<()> {
    let mut file = read_ui_state_file(data_dir).unwrap_or_default();
    file.overlay_hidden_key = key.filter(|k| !k.is_empty()).map(str::to_string);
    write_ui_state_file(data_dir, &file)
}

/// Missing file / fields → enabled with [`hotkey::DEFAULT_BIND`].
pub fn load_overlay_hotkey(data_dir: &Path) -> OverlayHotkey {
    match read_ui_state_file(data_dir) {
        Some(file) => OverlayHotkey::normalized(
            file.overlay_hotkey_enabled.unwrap_or(true),
            file.overlay_hotkey
                .as_deref()
                .unwrap_or(hotkey::DEFAULT_BIND),
        ),
        None => OverlayHotkey::default(),
    }
}

pub fn save_overlay_hotkey(data_dir: &Path, hotkey: &OverlayHotkey) -> std::io::Result<()> {
    let mut file = read_ui_state_file(data_dir).unwrap_or_default();
    file.overlay_hotkey = Some(hotkey.bind.clone());
    file.overlay_hotkey_enabled = Some(hotkey.enabled);
    write_ui_state_file(data_dir, &file)
}

/// Default = the season marked current, else all time. Empty list → all time
/// (picker hidden, same as the website).
pub fn default_selection(seasons: &[Season]) -> SeasonSel {
    seasons
        .iter()
        .find(|s| s.is_current)
        .map(|s| SeasonSel::Season(s.id.clone()))
        .unwrap_or(SeasonSel::AllTime)
}

/// Restore a persisted choice, or fall back to [`default_selection`].
///
/// - No file (`None`) → current if marked, else all time.
/// - File says all time → stay all time even if a current season exists.
/// - File says a season that is gone → default again.
/// - No seasons on the server → all time (picker hidden).
pub fn resolve_selection(persisted: Option<SeasonSel>, seasons: &[Season]) -> SeasonSel {
    if seasons.is_empty() {
        return SeasonSel::AllTime;
    }
    match persisted {
        None => default_selection(seasons),
        Some(SeasonSel::AllTime) => SeasonSel::AllTime,
        Some(SeasonSel::Season(id)) => {
            if seasons.iter().any(|s| s.id == id) {
                SeasonSel::Season(id)
            } else {
                default_selection(seasons)
            }
        }
    }
}

pub fn window_for(sel: &SeasonSel, seasons: &[Season]) -> Option<SeasonWindow> {
    let id = sel.as_id()?;
    seasons.iter().find(|s| s.id == id).map(|s| SeasonWindow {
        starts_at: s.starts_at,
        ends_at: s.ends_at,
    })
}

pub fn show_season_picker(seasons: &[Season]) -> bool {
    !seasons.is_empty()
}

pub async fn fetch_seasons(url: String) -> Result<Vec<Season>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GET {url} → {}", resp.status()));
    }
    resp.json::<Vec<Season>>().await.map_err(|e| e.to_string())
}

pub fn seasons_url_from_server(server_url: &str) -> String {
    format!("{}/api/public/seasons", server_url.trim_end_matches('/'))
}

/// Fixtures never hit the network — production `[]` would wipe the sample list.
pub fn should_fetch_seasons(fixture_active: bool, url: Option<&str>) -> bool {
    !fixture_active && url.is_some()
}

/// Launch fetch, or another GET when `REFRESH_EVERY` has elapsed.
pub fn should_refetch(
    fixture_active: bool,
    url: Option<&str>,
    last_attempt: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    if !should_fetch_seasons(fixture_active, url) {
        return false;
    }
    match last_attempt {
        None => true,
        Some(at) => now.signed_duration_since(at) >= REFRESH_EVERY,
    }
}

/// An empty 200 must not replace a cache written this run (or any existing list).
pub fn apply_fetched_seasons(existing: Vec<Season>, fetched: Vec<Season>) -> Vec<Season> {
    if fetched.is_empty() && !existing.is_empty() {
        existing
    } else {
        fetched
    }
}

pub fn format_window(starts_at: DateTime<Utc>, ends_at: DateTime<Utc>) -> String {
    format!(
        "{} – {}",
        starts_at.format("%d %b %Y"),
        ends_at.format("%d %b %Y")
    )
}

/// All-time row first, then each server season. Role filter is ignored so the
/// numbers match the website's season totals (no role chip on My Stats).
pub fn season_rows(games: &[Game], seasons: &[Season]) -> Vec<SeasonRow> {
    let mut rows = Vec::with_capacity(seasons.len() + 1);
    rows.push(SeasonRow {
        sel: SeasonSel::AllTime,
        name: "All time".into(),
        window_label: None,
        is_current: false,
        record: aggregate(games, None, None).record,
    });
    for s in seasons {
        let window = SeasonWindow {
            starts_at: s.starts_at,
            ends_at: s.ends_at,
        };
        rows.push(SeasonRow {
            sel: SeasonSel::Season(s.id.clone()),
            name: s.name.clone(),
            window_label: Some(format_window(s.starts_at, s.ends_at)),
            is_current: s.is_current,
            record: aggregate(games, Some(window), None).record,
        });
    }
    rows
}

pub fn last_refreshed_note(cache: &SeasonCache) -> Option<String> {
    let at = cache.fetched_at?;
    let when = at.format("%Y-%m-%d %H:%M UTC");
    if cache.from_network {
        Some(format!("Last refreshed {when}"))
    } else {
        Some(format!("Last refreshed {when} · cached"))
    }
}

fn system_time_to_utc(t: SystemTime) -> Option<DateTime<Utc>> {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    DateTime::<Utc>::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
}

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let rows = season_rows(&app.games, &app.seasons.seasons);
    let cols = seasons_columns(content_width);
    let mut col = column![
        text("Seasons").size(SIZE_TITLE).font(FONT_BOLD).color(TEXT),
        text("Read-only. Seasons are managed on the website under /admin/seasons.")
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    ]
    .spacing(GRID_GAP)
    .width(Fill);

    if let Some(note) = last_refreshed_note(&app.seasons) {
        col = col.push(text(note).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_3));
    }

    if app.seasons.seasons.is_empty() {
        col = col.push(widgets::empty_surface(
            "No seasons on the server — stats are all time.",
        ));
    }

    for chunk in rows.chunks(cols) {
        let mut row = Row::new().spacing(GRID_GAP).width(Fill);
        for season in chunk {
            row = row.push(season_card(season, season.sel == app.season));
        }
        for _ in chunk.len()..cols {
            row = row.push(space().width(Fill));
        }
        col = col.push(row);
    }
    col.into()
}

/// Maps grammar: name, featured WR, `N games · W–L`, muted window, win bar.
/// Undecided is appended only when some games have no outcome.
pub fn season_card_line(record: &Record) -> String {
    let base = format!("{} · {}", record.games_label(), record.wl_label());
    match record.undecided() {
        0 => base,
        1 => format!("{base} · 1 undecided"),
        n => format!("{base} · {n} undecided"),
    }
}

fn season_card(row: &SeasonRow, selected: bool) -> Element<'static, Message> {
    let title = if row.is_current {
        format!("{} · current", row.name)
    } else {
        row.name.clone()
    };
    let window = row
        .window_label
        .clone()
        .unwrap_or_else(|| "Every recorded game".into());
    let wr = format!("{:.0}%", row.record.win_rate_pct());
    let body = column![
        text(title).size(SIZE_TITLE).font(FONT_BOLD).color(TEXT),
        text(wr)
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        text(season_card_line(&row.record))
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
        text(window).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_3),
        widgets::win_bar_for(row.record.win_rate()),
    ]
    .spacing(6);

    mouse_area(widgets::surface_stat_card(
        widgets::map_stripe_outcome(&row.record),
        selected,
        crate::theme::HEIGHT_MAP,
        body.into(),
    ))
    .on_press(Message::SelectSeason(row.sel.clone()))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::FixtureKind;
    use crate::fixtures;
    use crate::model::{Game, GameOcr, Outcome, Role};
    use crate::snapshot::games_from_snapshot;
    use chrono::TimeZone;

    fn season(id: &str, current: bool) -> Season {
        Season {
            id: id.into(),
            name: id.into(),
            starts_at: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
            ends_at: Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap(),
            is_current: current,
        }
    }

    fn season_window(id: &str, current: bool, start: DateTime<Utc>, end: DateTime<Utc>) -> Season {
        Season {
            id: id.into(),
            name: id.into(),
            starts_at: start,
            ends_at: end,
            is_current: current,
        }
    }

    fn game(hero: &str, outcome: Outcome, at: DateTime<Utc>) -> Game {
        Game {
            session_id: format!("s-{hero}-{at}"),
            hero: hero.into(),
            map_name: "King's Row".into(),
            role: Role::Support,
            outcome,
            elims: 1,
            deaths: 0,
            assists: 0,
            damage: 0,
            healing: 0,
            mitigation: 0,
            played_at: at,
            edited: false,
            edited_fields: Vec::new(),
            ocr: GameOcr::default(),
            segments: Vec::new(),
        }
    }

    fn ts(y: i32, m: u32, d: u32, hh: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, hh, 0, 0).unwrap()
    }

    fn tmp(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sst-ui-{prefix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn fixture_never_fetches() {
        assert!(!should_fetch_seasons(
            true,
            Some("https://example.test/api/public/seasons")
        ));
        assert!(should_fetch_seasons(
            false,
            Some("https://example.test/api/public/seasons")
        ));
        assert!(!should_fetch_seasons(false, None));
        assert!(!should_refetch(
            true,
            Some("https://example.test/api/public/seasons"),
            None,
            Utc::now()
        ));
    }

    #[test]
    fn refetch_every_thirty_minutes() {
        let url = Some("https://example.test/api/public/seasons");
        let t0 = ts(2026, 9, 2, 12);
        assert!(should_refetch(false, url, None, t0));
        assert!(!should_refetch(false, url, Some(t0), t0));
        assert!(!should_refetch(
            false,
            url,
            Some(t0),
            t0 + chrono::TimeDelta::minutes(29)
        ));
        assert!(should_refetch(
            false,
            url,
            Some(t0),
            t0 + chrono::TimeDelta::minutes(30)
        ));
        assert!(!should_refetch(false, None, None, t0));
    }

    #[test]
    fn empty_fetch_does_not_wipe_existing() {
        let existing = vec![season("season-17", true)];
        let kept = apply_fetched_seasons(existing.clone(), Vec::new());
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "season-17");
    }

    #[test]
    fn nonempty_fetch_replaces_cache() {
        let existing = vec![season("old", false)];
        let fetched = vec![season("season-17", true)];
        let out = apply_fetched_seasons(existing, fetched);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "season-17");
    }

    #[test]
    fn default_selection_current_else_all_time() {
        assert_eq!(default_selection(&[]), SeasonSel::AllTime);
        assert_eq!(
            default_selection(&[season("s16", false), season("s17", false)]),
            SeasonSel::AllTime
        );
        assert_eq!(
            default_selection(&[season("s16", false), season("s17", true)]),
            SeasonSel::Season("s17".into())
        );
    }

    #[test]
    fn resolve_selection_persists_all_time_and_drops_stale() {
        let list = vec![season("s16", false), season("s17", true)];
        assert_eq!(
            resolve_selection(None, &list),
            SeasonSel::Season("s17".into()),
            "no file → current"
        );
        assert_eq!(
            resolve_selection(Some(SeasonSel::AllTime), &list),
            SeasonSel::AllTime,
            "explicit all time stays"
        );
        assert_eq!(
            resolve_selection(Some(SeasonSel::Season("s16".into())), &list),
            SeasonSel::Season("s16".into())
        );
        assert_eq!(
            resolve_selection(Some(SeasonSel::Season("gone".into())), &list),
            SeasonSel::Season("s17".into()),
            "stale id → current"
        );
        assert_eq!(
            resolve_selection(Some(SeasonSel::Season("s17".into())), &[]),
            SeasonSel::AllTime,
            "no seasons → picker hidden / all time"
        );
        assert!(!show_season_picker(&[]));
        assert!(show_season_picker(&list));
    }

    #[test]
    fn ui_state_roundtrip_distinguishes_missing_from_all_time() {
        let dir = tmp("uistate");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_ui_state(&dir), None);
        save_ui_state(&dir, &SeasonSel::AllTime).unwrap();
        assert_eq!(load_ui_state(&dir), Some(SeasonSel::AllTime));
        save_ui_state(&dir, &SeasonSel::Season("s17".into())).unwrap();
        assert_eq!(load_ui_state(&dir), Some(SeasonSel::Season("s17".into())));
        let raw = std::fs::read_to_string(ui_state_path(&dir)).unwrap();
        assert!(raw.contains("s17"), "{raw}");
        assert_eq!(
            load_overlay_hidden_key(&dir),
            None,
            "overlay hold defaults to Auto when the field is written with season"
        );
        save_overlay_hidden_key(&dir, Some("sess-1")).unwrap();
        assert_eq!(load_overlay_hidden_key(&dir).as_deref(), Some("sess-1"));
        assert_eq!(load_ui_state(&dir), Some(SeasonSel::Season("s17".into())));
        save_ui_state(&dir, &SeasonSel::AllTime).unwrap();
        assert_eq!(
            load_overlay_hidden_key(&dir).as_deref(),
            Some("sess-1"),
            "season save must not reset the overlay hold"
        );
        let default_hk = load_overlay_hotkey(&dir);
        assert!(default_hk.enabled);
        assert_eq!(default_hk.bind, crate::hotkey::DEFAULT_BIND);
        let custom = crate::hotkey::OverlayHotkey {
            enabled: false,
            bind: "Ctrl+Alt+O".into(),
        };
        save_overlay_hotkey(&dir, &custom).unwrap();
        assert_eq!(load_overlay_hotkey(&dir), custom);
        assert_eq!(
            load_overlay_hidden_key(&dir).as_deref(),
            Some("sess-1"),
            "hotkey save must not reset the overlay hold"
        );
        save_overlay_hidden_key(&dir, Some("sess-2")).unwrap();
        assert_eq!(load_overlay_hotkey(&dir), custom);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_refreshed_note_marks_cache() {
        let cache = SeasonCache {
            seasons: vec![],
            fetched_at: Some(ts(2026, 9, 2, 15)),
            from_network: false,
        };
        let note = last_refreshed_note(&cache).expect("note");
        assert!(note.contains("2026-09-02 15:00 UTC"), "{note}");
        assert!(note.contains("cached"), "{note}");
        let live = SeasonCache {
            from_network: true,
            ..cache
        };
        let note = last_refreshed_note(&live).expect("note");
        assert!(note.contains("Last refreshed"), "{note}");
        assert!(!note.contains("cached"), "{note}");
        assert!(last_refreshed_note(&SeasonCache::default()).is_none());
    }

    /// Games immediately before `ends_at` stay in the closing season; a game
    /// at `ends_at` belongs to the next window (half-open, UTC).
    #[test]
    fn games_on_both_sides_of_season_boundary() {
        let s16_end = ts(2026, 9, 1, 0);
        let s16 = season_window("s16", false, ts(2026, 6, 24, 0), s16_end);
        let s17 = season_window("s17", true, s16_end, ts(2026, 12, 10, 0));
        let before = game("Reinhardt", Outcome::Win, ts(2026, 8, 31, 23));
        let on_boundary = game("Ana", Outcome::Loss, s16_end);
        let after = game("Ashe", Outcome::Win, ts(2026, 9, 1, 1));
        let games = vec![before, on_boundary, after];
        let rows = season_rows(&games, &[s16.clone(), s17.clone()]);
        assert_eq!(rows[0].sel, SeasonSel::AllTime);
        assert_eq!(rows[0].record.games, 3);
        assert_eq!(rows[0].record.wins, 2);
        assert_eq!(rows[0].record.losses, 1);
        assert_eq!(rows[1].sel, SeasonSel::Season("s16".into()));
        assert_eq!(rows[1].record.games, 1, "Aug 31 23:00 is still S16");
        assert_eq!(rows[1].record.wins, 1);
        assert_eq!(rows[2].sel, SeasonSel::Season("s17".into()));
        assert_eq!(
            rows[2].record.games, 2,
            "played_at == ends_at of S16 is S17"
        );
        assert_eq!(rows[2].record.wins, 1);
        assert_eq!(rows[2].record.losses, 1);
        assert_eq!(
            window_for(
                &SeasonSel::Season("s16".into()),
                &[s16.clone(), s17.clone()]
            ),
            Some(SeasonWindow {
                starts_at: s16.starts_at,
                ends_at: s16.ends_at,
            })
        );
    }

    #[test]
    fn sample_fixture_season_counts() {
        let snap = fixtures::snapshot(FixtureKind::Sample);
        let games = games_from_snapshot(&snap);
        let seasons: Vec<Season> =
            serde_json::from_str(fixtures::sample_seasons_json()).expect("sample seasons");
        let rows = season_rows(&games, &seasons);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].record.games, 5);
        assert_eq!(
            (
                rows[0].record.wins,
                rows[0].record.losses,
                rows[0].record.draws
            ),
            (3, 2, 0)
        );
        // S16: Rein win + Soldier loss (18 Aug).
        assert_eq!(rows[1].name, "Season 16");
        assert_eq!(rows[1].record.games, 2);
        assert_eq!((rows[1].record.wins, rows[1].record.losses), (1, 1));
        // S17 (current): JQ win, Ashe loss, Ana win (2 Sep).
        assert_eq!(rows[2].name, "Season 17");
        assert!(rows[2].is_current);
        assert_eq!(rows[2].record.games, 3);
        assert_eq!((rows[2].record.wins, rows[2].record.losses), (2, 1));
        assert_eq!(
            default_selection(&seasons),
            SeasonSel::Season("season-17".into())
        );
    }

    #[test]
    fn format_window_utc_dates() {
        assert_eq!(
            format_window(ts(2026, 6, 24, 0), ts(2026, 9, 1, 0)),
            "24 Jun 2026 – 01 Sep 2026"
        );
    }

    #[test]
    fn season_card_line_matches_maps_grammar() {
        let decided = Record {
            games: 10,
            wins: 3,
            losses: 1,
            draws: 0,
        };
        assert_eq!(season_card_line(&decided), "10 games · 3–1");

        let draws = Record {
            games: 5,
            wins: 2,
            losses: 2,
            draws: 1,
        };
        assert_eq!(season_card_line(&draws), "5 games · 2–2–1");

        let mixed = Record {
            games: 62,
            wins: 10,
            losses: 15,
            draws: 0,
        };
        assert_eq!(season_card_line(&mixed), "62 games · 10–15 · 37 undecided");
    }

    #[test]
    fn season_cards_use_maps_tokens() {
        use crate::theme::{self, PAGE_PAD_X, PAGE_PAD_Y};
        assert_eq!(PAGE_PAD_Y, 24.0);
        assert_eq!(PAGE_PAD_X, 32.0);
        assert_eq!(theme::RADIUS_CARD, 16.0);
        assert_eq!(theme::HEIGHT_MAP, 148.0);
        assert_eq!(theme::STRIPE, 4.0);
        assert_eq!(theme::ACCENT, {
            iced::Color::from_rgb(
                0x8f as f32 / 255.0,
                0x73 as f32 / 255.0,
                0xff as f32 / 255.0,
            )
        });
    }
}
