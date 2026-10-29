mod game;
mod heroes;
mod history;
mod history_filter;
mod maps;
mod overview;
pub(crate) mod role;

pub use game::StatsMatch;

use dioxus::prelude::*;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{MemberSettingsResponse, UpdateMemberSettingsRequest};

use crate::components::ui::{SeasonSelect, use_season_change, use_stats_season};
use crate::components::{Toast, member_pending, use_toast};
use crate::hooks::use_api_with;
use crate::routes::Route;
use crate::sample::Tally;
use crate::state::{AuthState, use_auth, use_relay_availability_state};

// -- Shared data models (fetched once here, consumed by the tab modules) --

#[derive(Debug, Clone, Deserialize)]
struct PersonalStats {
    #[allow(dead_code)]
    member_id: String,
    total_matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct HeroStats {
    hero: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
    avg_elims: f64,
    avg_deaths: f64,
    avg_damage: f64,
    avg_healing: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct MapStats {
    map_name: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

impl HeroStats {
    fn tally(&self) -> Tally {
        Tally::new(self.matches, self.wins, self.losses, self.draws)
    }
}

impl MapStats {
    fn tally(&self) -> Tally {
        Tally::new(self.matches, self.wins, self.losses, self.draws)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PersonalMatch {
    #[allow(dead_code)]
    id: String,
    hero: String,
    map_name: String,
    game_mode: String,
    role: String,
    outcome: String,
    elims: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    healing: u32,
    /// Match JSON from before mitigation was returned. A missing value is 0.
    #[serde(default)]
    mitigation: u32,
    played_at: DateTime<Utc>,
    /// Reader id. Missing or null is None, which reads as `ocr-v1`.
    #[serde(default)]
    recognizer: Option<String>,
    /// Cell names the reader flagged. Missing or null is None (no marks).
    #[serde(default)]
    suspect_fields: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct MatchPage {
    data: Vec<PersonalMatch>,
    next_cursor: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum StatsTab {
    Overview,
    Heroes,
    Maps,
    History,
}

// -- Shared helpers --

fn winrate_pct(wins: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        (wins as f64 / total as f64) * 100.0
    }
}

/// Class for win-rate TEXT values. Plain text tokens, never a traffic light.
/// Polarity is carried by the bar against the 50% reference hairline; the only
/// text distinction is sample-size honesty: a result without a win rate yet
/// (a record or "No games", see [`crate::sample`]) is muted.
pub(super) fn wr_text_class(tally: Tally) -> &'static str {
    if tally.has_win_rate() {
        "stats-wr"
    } else {
        "stats-wr muted"
    }
}

/// Fill token for a winrate BAR: two-pole diverging encoding (W5a).
/// - `> 50%` → cool up-pole, `< 50%` → warm down-pole (poles are chart
///   tokens, never status colors)
/// - exactly 50.0% (`wins * 2 == matches`, integer-exact) → neutral midpoint:
///   a dead-even record has no polarity, and a diverging scale is two hues +
///   a neutral midpoint; painting an even record blue would claim a lead
///   that isn't there
/// - no win rate yet (under 10 decided games) → neutral too, so a muted row never
///   claims a pole (muting overrides polarity; the row also renders at
///   reduced opacity via the existing `muted` treatment)
pub(super) fn wr_bar_color(tally: Tally) -> &'static str {
    let wins2 = u64::from(tally.wins) * 2;
    let total = u64::from(tally.games);
    if !tally.has_win_rate() || wins2 == total {
        "var(--text-3)"
    } else if wins2 > total {
        "var(--chart-wr-up)"
    } else {
        "var(--chart-wr-down)"
    }
}

/// Role cards mute under this many games. Heroes, maps and seasons follow
/// the small-sample rule in [`crate::sample`] instead.
pub(super) const MIN_GAMES: u32 = crate::sample::RECORD_MIN_GAMES;

/// Platform line on `/stats`. Linux supports Wayland and X11. No other OS builds.
const TRACKER_PLATFORM_COPY: &str =
    "It runs on Linux, on both Wayland and X11. There is no Windows or macOS build.";

/// Stat-tracker build named in the install note on `/stats` when health
/// does not send `tracker_latest`. One constant, so the banner tracks the
/// current build instead of an older alpha.
const TRACKER_RELEASE: &str = "rc.9";

/// `true` when `core` is `MAJOR.MINOR.PATCH` with numeric components.
fn is_release_core(core: &str) -> bool {
    let mut parts = core.split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    [major, minor, patch]
        .into_iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Short install label. A bare `MAJOR.MINOR.PATCH-label` drops the numeric
/// core so it matches [`TRACKER_RELEASE`]. A value that is already that
/// short label is kept.
fn tracker_notice_label(server: Option<&str>) -> String {
    let Some(raw) = server.map(str::trim).filter(|value| !value.is_empty()) else {
        return TRACKER_RELEASE.to_string();
    };
    match raw.split_once('-') {
        Some((core, rest)) if is_release_core(core) && !rest.is_empty() => rest.to_string(),
        _ => raw.to_string(),
    }
}

fn tracker_release_notice_text(server: Option<&str>) -> String {
    format!(
        "Install {} between games, not during one.",
        tracker_notice_label(server)
    )
}

fn format_date(dt: &DateTime<Utc>) -> String {
    let now = Utc::now();
    let secs = now.signed_duration_since(*dt).num_seconds().max(0);
    if secs < 45 {
        "now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 86_400 * 14 {
        format!("{}d ago", secs / 86_400)
    } else {
        dt.format("%b %d, %Y").to_string()
    }
}

/// Shared map → game mode (used by maps tab + overview mode chips).
/// Delegates to [`scuffed_types::MapName`] so accented/unaccented live names
/// (Paraíso/Paraiso, Esperança/Esperanca, Watchpoint: Grímsvötn/Grimsvotn)
/// and Neon Junction bucket correctly.
pub(super) fn map_game_mode(name: &str) -> &'static str {
    scuffed_types::MapName::game_mode_label(name)
}

fn initial_density() -> &'static str {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
            && let Ok(Some(v)) = storage.get_item("stats-ui-density")
            && v == "comfortable"
        {
            return "comfortable";
        }
    }
    "compact"
}

fn persist_density(d: &str) {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
        {
            let _ = storage.set_item("stats-ui-density", d);
        }
    }
    #[cfg(not(feature = "web"))]
    let _ = d;
}

/// Fetch-failed state with a retry button (bumps the resource's refresh counter).
pub(crate) fn load_error_state(what: &'static str, mut refresh: Signal<u64>) -> Element {
    rsx! {
        div { class: "stats-load-error",
            p { "Couldn't load {what}." }
            button { onclick: move |_| refresh += 1, "Retry" }
        }
    }
}

const STATS_CSS: &str = r#"
    .my-stats.stats-page {
        max-width: 1100px;
        margin: 0 auto;
        padding: 2rem 1.5rem;
        /* One horizontal inset for history headers and cards. Density overrides it. */
        --history-pad-x: 0.85rem;
        --history-border: 1px;
    }
    .my-stats.stats-page h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin-bottom: 0.5rem;
    }
    .stats-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        flex-wrap: wrap;
        gap: 1rem;
        margin-bottom: 1.5rem;
    }
    .stats-header-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
        min-width: 0;
        max-width: 100%;
    }
    .stats-header-actions a, .stats-header-actions button {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
        text-decoration: none;
    }
    .stats-header-actions a:hover, .stats-header-actions button:hover {
        color: var(--text);
        border-color: var(--accent-soft);
    }
    .my-stats .stats-tabs {
        display: flex;
        gap: 0.25rem;
        margin-bottom: 1.5rem;
        border-bottom: 1px solid var(--border);
        padding-bottom: 0;
    }
    .my-stats .stats-tab {
        padding: 0.6rem 1.2rem;
        border: none;
        background: none;
        color: var(--text-2);
        font-family: var(--font-head);
        font-size: 0.9rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        cursor: pointer;
        border-bottom: 2px solid transparent;
        margin-bottom: -1px;
        transition: color 0.15s, border-color 0.15s;
    }
    .my-stats .stats-tab:hover {
        color: var(--text);
    }
    .my-stats .stats-tab.active {
        color: var(--accent);
        border-bottom-color: var(--accent);
    }
    /* Win-rate text values: plain text tokens, muted below the min-games gate.
       No traffic light; polarity lives in the bar vs the 50% hairline. */
    .my-stats .stats-wr {
        display: inline-block;
        font-size: 0.8rem;
        font-weight: 600;
        color: var(--text);
        font-variant-numeric: tabular-nums;
    }
    .my-stats .stats-wr.muted { color: var(--text-3); font-weight: 400; }
    .stats-gate-note {
        font-size: 0.7rem;
        color: var(--text-3);
        margin: 0 0 0.75rem;
    }
    /* Outcome chips: status tokens as status only. Win = good, defeat =
       serious/danger, draw = neutral text (never gold). */
    .outcome-win { color: var(--ok); font-weight: 600; }
    .outcome-loss { color: var(--danger); font-weight: 600; }
    .outcome-draw { color: var(--text-2); font-weight: 600; }
    /* W5b history layout: outcome | identity | E D A | dmg heal | date.
       Header + cards share the outer grid; numeric cols live in .match-stats
       with fixed tracks so values column-scan. Tighter than free scoreline. */
    .match-list { display: flex; flex-direction: column; gap: 0.15rem; min-width: 0; }
    .match-list-head,
    .match-card {
        display: grid;
        box-sizing: border-box;
        border: var(--history-border) solid transparent;
        padding-left: var(--history-pad-x);
        padding-right: var(--history-pad-x);
        grid-template-columns:
            minmax(4.5rem, 5.5rem)
            minmax(7rem, 1.4fr)
            auto
            minmax(4.25rem, auto);
        gap: 0.35rem 0.65rem;
        align-items: center;
    }
    .match-list-head .match-stats,
    .match-card .match-stats {
        box-sizing: border-box;
        padding: 0;
    }
    .match-stats {
        display: grid;
        grid-template-columns: 2.25rem 2.25rem 2.25rem 3.75rem 3.75rem;
        gap: 0.2rem 0.45rem;
        align-items: center;
        justify-self: end;
    }
    .match-list-head {
        padding-top: 0;
        padding-bottom: 0.15rem;
        border-color: transparent;
        font-size: 0.65rem;
        font-weight: 600;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--text-3);
    }
    /* Header and value cells in a column share padding, box model, and alignment. */
    .match-h-outcome,
    .match-card .match-outcome,
    .match-h-id,
    .match-card .match-identity {
        box-sizing: border-box;
        padding: 0;
        text-align: left;
    }
    .match-h-stat,
    .match-card .match-stat,
    .match-h-date,
    .match-card .match-date {
        box-sizing: border-box;
        padding: 0;
        text-align: right;
    }
    .match-card {
        background: var(--surface);
        border-color: var(--border);
        border-radius: 8px;
        padding-top: 0.5rem;
        padding-bottom: 0.5rem;
        font-size: 0.85rem;
        transition: background 0.15s;
    }
    .match-card:hover { background: var(--surface-2); }
    /* History rows open the game page. Keep the row's own colors. */
    a.match-card-link { color: inherit; text-decoration: none; cursor: pointer; }
    a.match-card-link:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    .match-cards { display: flex; flex-direction: column; gap: 0.3rem; }
    .match-card .match-outcome {
        font-family: var(--font-head);
        font-size: 0.85rem;
        font-weight: 700;
        text-transform: uppercase;
        line-height: 1.15;
        display: flex;
        align-items: baseline;
        min-width: 0;
        overflow: hidden;
    }
    .match-identity { min-width: 0; }
    .match-hero-line {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.4rem;
        min-width: 0;
    }
    .match-card .match-hero {
        color: var(--text);
        font-weight: 500;
        display: inline-flex;
        align-items: baseline;
        min-width: 0;
        max-width: 100%;
        overflow: hidden;
    }
    /* Ellipsis lives on the value, so the unsure mark stays outside the clip. */
    .stat-value-text {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .match-stat .stat-value-text,
    .stat-flagged .stat-value-text {
        overflow: visible;
        text-overflow: clip;
        flex: 0 0 auto;
    }
    .match-hero-line .tracker-badge { flex-shrink: 1; }
    .reader-badge-short { display: none; }
    .sr-only {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }
    /* Unsure cells: a dotted underline plus a question mark, not color alone. */
    .stat-unsure .stat-value-text {
        text-decoration-line: underline;
        text-decoration-style: dotted;
        text-underline-offset: 0.18em;
    }
    .stat-unsure-mark {
        flex-shrink: 0;
        margin-left: 0.1em;
        font-size: 0.7em;
        font-weight: 700;
        vertical-align: super;
        line-height: 0;
        text-decoration: none;
    }
    .stat-flagged {
        flex: 0 0 auto;
        max-width: 100%;
        white-space: nowrap;
    }
    .match-card .match-map {
        color: var(--text-2);
        font-size: 0.75rem;
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        column-gap: 0.25rem;
        min-width: 0;
    }
    .match-map-name { min-width: 0; max-width: 100%; }
    .match-stat.stat-unsure {
        display: flex;
        justify-content: flex-end;
        align-items: baseline;
        white-space: nowrap;
    }
    .match-stat {
        color: var(--text-2);
        font-size: 0.8rem;
        font-variant-numeric: tabular-nums;
    }
    .match-card .match-date {
        color: var(--text-3);
        font-size: 0.75rem;
        white-space: nowrap;
    }
    .stats-pagination {
        display: flex;
        justify-content: center;
        gap: 0.75rem;
        margin-top: 1.5rem;
    }
    .stats-pagination button {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
    }
    .stats-pagination button:hover:not(:disabled) {
        color: var(--text);
        border-color: var(--accent-soft);
    }
    .stats-pagination button:disabled { opacity: 0.4; cursor: not-allowed; }

    /* Summary: win-rate lead tile + secondary tiles */
    .my-stats .stats-summary {
        display: grid;
        grid-template-columns: minmax(220px, 2fr) minmax(140px, 1fr);
        gap: 1rem;
        margin-bottom: 2rem;
    }
    .my-stats .stat-tile {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        text-align: center;
        display: flex;
        flex-direction: column;
        justify-content: center;
    }
    .my-stats .stat-tile-value {
        font-family: var(--font-head);
        font-size: 1.9rem;
        color: var(--text);
        letter-spacing: 2px;
        line-height: 1;
    }
    .my-stats .stat-tile-hero .stat-tile-value {
        font-size: 2.8rem;
        color: var(--accent);
    }
    .my-stats .stat-tile-label {
        font-size: 0.75rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.05em;
        margin-top: 0.35rem;
    }
    .my-stats .stat-tile-record {
        font-size: 0.9rem;
        color: var(--text-2);
        margin-top: 0.4rem;
        white-space: nowrap;
    }

    /* Load error + retry */
    .stats-load-error {
        text-align: center;
        padding: 2rem 1rem;
        color: var(--text-2);
        font-size: 0.9rem;
    }
    .stats-load-error p { margin: 0 0 0.75rem; }
    .stats-load-error button {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
    }
    .stats-load-error button:hover {
        color: var(--text);
        border-color: var(--accent-soft);
    }

    /* Overview: role layout */
    .overview-grid {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 1.5rem;
        margin-top: 1rem;
    }
    .overview-section {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
    }
    .overview-section h3 {
        font-family: var(--font-head);
        font-size: 0.85rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin: 0 0 1rem;
    }
    .role-cards { display: flex; flex-direction: column; gap: 0.75rem; }
    .role-card {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        padding: 0.6rem 0.75rem;
        border-radius: 8px;
        background: var(--bg);
        border-left: 3px solid transparent;
    }
    .role-card-info { flex: 1; }
    .role-card-name {
        font-weight: 600;
        font-size: 0.85rem;
        color: var(--text);
    }
    .role-card-sub {
        font-size: 0.75rem;
        color: var(--text-3);
        margin-top: 0.15rem;
    }
    .role-card-wr {
        font-family: var(--font-head);
        font-size: 1.1rem;
        font-weight: 700;
        color: var(--text);
        font-variant-numeric: tabular-nums;
    }
    .role-card-wr.muted { color: var(--text-3); font-weight: 400; }

    /* Heroes: chart + table */
    .heroes-chart-section {
        margin-bottom: 1.5rem;
    }
    .section-title {
        font-family: var(--font-head);
        font-size: 0.8rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin-bottom: 0.75rem;
    }

    /* Maps: game mode groups */
    .map-mode-group { margin-bottom: 1.5rem; }
    .map-mode-header {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.6rem;
        padding-bottom: 0.4rem;
        border-bottom: 1px solid var(--border);
    }
    .map-mode-name {
        font-family: var(--font-head);
        font-size: 0.85rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .map-mode-agg {
        font-size: 0.75rem;
        color: var(--text-3);
        margin-left: auto;
    }

    @media (max-width: 768px) {
        .overview-grid { grid-template-columns: 1fr; }
    }
    @media (max-width: 640px) {
        .my-stats .stats-summary { grid-template-columns: 1fr; }
    }

    /* Slim tracker row (shown once matches exist) */
    .tracker-slim {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        flex-wrap: wrap;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 0.6rem 1rem;
        margin-bottom: 1.5rem;
        font-size: 0.85rem;
    }
    .tracker-slim-label {
        font-family: var(--font-head);
        font-size: 0.8rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
    }
    .tracker-slim-sep { color: var(--text-3); }
    .tracker-slim-note { color: var(--text-2); }
    .tracker-slim-actions {
        margin-left: auto;
        display: flex;
        gap: 0.5rem;
    }
    .tracker-slim-actions a, .tracker-slim-actions button {
        padding: 0.3rem 0.8rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
        text-decoration: none;
    }
    .tracker-slim-actions a:hover, .tracker-slim-actions button:hover {
        color: var(--text);
        border-color: var(--accent-soft);
    }
    .tracker-release-notice {
        flex: 1 0 100%;
        margin: 0 0 0.65rem;
        padding: 0.55rem 0.85rem;
        border-radius: 8px;
        border: 1px solid var(--warn);
        background: color-mix(in srgb, var(--warn) 12%, transparent);
        color: var(--text);
        font-size: 0.85rem;
        line-height: 1.4;
    }

    /* Daemon settings panel */
    .daemon-settings {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        margin-bottom: 1.5rem;
    }
    .daemon-settings summary {
        font-family: var(--font-head);
        font-size: 0.85rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        cursor: pointer;
        user-select: none;
        list-style: none;
        display: flex;
        align-items: center;
        gap: 0.5rem;
    }
    .daemon-settings summary::-webkit-details-marker { display: none; }
    .daemon-settings[open] summary { margin-bottom: 1rem; }
    .daemon-settings-row {
        display: flex;
        gap: 0.75rem;
        align-items: center;
        flex-wrap: wrap;
    }
    .daemon-settings-label {
        font-size: 0.8rem;
        color: var(--text-2);
        white-space: nowrap;
    }
    .daemon-settings-input {
        flex: 1;
        min-width: 160px;
        padding: 0.4rem 0.75rem;
        background: var(--bg);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text);
        font-size: 0.85rem;
        font-family: var(--font-mono);
    }
    .daemon-settings-input:focus {
        border-color: var(--accent-soft);
    }
    .daemon-settings-hint {
        font-size: 0.75rem;
        color: var(--text-3);
        margin-top: 0.5rem;
    }

    /* Tracker download / install panel */
    .tracker-download {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        margin-bottom: 1.5rem;
    }
    .tracker-download-head {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        flex-wrap: wrap;
        margin-bottom: 0.75rem;
    }
    .tracker-download-head h3 {
        font-family: var(--font-head);
        font-size: 0.85rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin: 0;
    }
    .tracker-badge {
        display: inline-block;
        padding: 0.15rem 0.55rem;
        border-radius: 4px;
        background: var(--surface-2);
        border: 1px solid var(--warn);
        color: var(--warn);
        font-size: 0.7rem;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .tracker-download p {
        font-size: 0.85rem;
        color: var(--text-2);
        margin: 0 0 0.75rem;
        line-height: 1.5;
    }
    .tracker-download code.tracker-install {
        display: block;
        background: var(--bg);
        border: 1px solid var(--border);
        border-radius: 4px;
        padding: 0.6rem 0.75rem;
        font-family: var(--font-mono);
        font-size: 0.8rem;
        color: var(--accent);
        word-break: break-all;
        margin: 0 0 0.75rem;
        user-select: all;
    }
    .tracker-download-links {
        display: flex;
        gap: 0.5rem;
        flex-wrap: wrap;
    }
    .tracker-download-links a {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        text-decoration: none;
        transition: all 0.15s;
    }
    .tracker-download-links a:hover {
        color: var(--text);
        border-color: var(--accent-soft);
    }

    /* W3: filters, density, overview extras */
    .stats-filters {
        display: flex;
        flex-wrap: wrap;
        gap: 1rem 1.5rem;
        margin-bottom: 1.25rem;
        align-items: center;
    }
    .filter-group {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        align-items: center;
    }
    .filter-label {
        font-size: 0.7rem;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        color: var(--text-3);
        margin-right: 0.25rem;
    }
    .my-stats.stats-page .filter-chip {
        padding: 0.25rem 0.65rem;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.75rem;
        cursor: pointer;
    }
    .my-stats.stats-page .filter-chip.active {
        border-color: var(--accent);
        color: var(--text);
        background: color-mix(in srgb, var(--accent) 12%, var(--bg));
    }
    .filter-hint {
        font-size: 0.7rem;
        color: var(--text-3);
        margin: 0;
        width: 100%;
    }
    .my-stats .stats-row-muted { opacity: 0.55; }
    .density-toggle {
        font-size: 0.75rem;
        padding: 0.3rem 0.65rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        cursor: pointer;
    }
    .density-toggle:hover { color: var(--text); border-color: var(--accent-soft); }
    .stats-page[data-density="comfortable"] .overview-grid { gap: 1.75rem; }
    .stats-page[data-density="comfortable"] { --history-pad-x: 1.1rem; }
    .stats-page[data-density="comfortable"] .match-card {
        padding-top: 0.75rem;
        padding-bottom: 0.75rem;
    }
    .stats-page[data-density="compact"] { --history-pad-x: 0.75rem; }
    .stats-page[data-density="compact"] .match-card {
        padding-top: 0.4rem;
        padding-bottom: 0.4rem;
    }
    .stats-page[data-density="compact"] .match-cards { gap: 0.2rem; }
    .stats-page[data-density="compact"] .overview-grid { gap: 1rem; }
    .overview-form { margin-top: 1rem; }
    .mini-hero-list { display: flex; flex-direction: column; gap: 0.45rem; }
    .mini-hero-row {
        display: grid;
        grid-template-columns: 1fr auto auto;
        gap: 0.75rem;
        font-size: 0.85rem;
        padding: 0.35rem 0;
        border-bottom: 1px solid var(--border);
    }
    .mini-hero-name { color: var(--text); font-weight: 500; }
    .mini-hero-meta { color: var(--text-3); }
    .mini-hero-wr { color: var(--accent); font-weight: 600; font-variant-numeric: tabular-nums; }
    .mode-chips { display: flex; flex-wrap: wrap; gap: 0.5rem; }
    .mode-chip {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        padding: 0.55rem 0.75rem;
        border-radius: 8px;
        border: 1px solid var(--border);
        background: var(--bg);
        min-width: 4.5rem;
    }
    .mode-chip-name { font-size: 0.7rem; color: var(--text-3); text-transform: uppercase; }
    .mode-chip-wr { font-size: 1.1rem; font-weight: 700; color: var(--text); font-variant-numeric: tabular-nums; }
    .mode-chip-n { font-size: 0.7rem; color: var(--text-3); }
    .form-strip { display: flex; flex-wrap: wrap; gap: 0.35rem; }
    .form-chip {
        width: 1.6rem; height: 1.6rem;
        display: inline-flex; align-items: center; justify-content: center;
        border-radius: 4px;
        font-size: 0.7rem; font-weight: 700;
    }
    .form-chip.win { background: color-mix(in srgb, var(--ok) 25%, transparent); color: var(--ok); }
    .form-chip.loss { background: color-mix(in srgb, var(--danger) 25%, transparent); color: var(--danger); }
    .form-chip.draw { background: color-mix(in srgb, var(--text-3) 20%, transparent); color: var(--text-3); }
    .map-callouts { display: flex; flex-wrap: wrap; gap: 0.75rem; margin-bottom: 1.25rem; }
    .map-callout {
        display: flex; flex-wrap: wrap; gap: 0.5rem 0.75rem; align-items: baseline;
        padding: 0.6rem 0.9rem; border-radius: 8px; border: 1px solid var(--border);
        background: var(--surface); font-size: 0.85rem;
    }
    .map-callout.best { border-left: 3px solid var(--ok); }
    .map-callout.worst { border-left: 3px solid var(--danger); }
    .map-callout-label { font-size: 0.7rem; text-transform: uppercase; color: var(--text-3); }
    .map-callout-name { font-weight: 600; color: var(--text); }
    .map-callout-meta { color: var(--text-2); font-variant-numeric: tabular-nums; }

    /* W4 polish: tables, mobile, hold layout */
    .my-stats.stats-page .data-table th:not(:first-child),
    .my-stats.stats-page .data-table td:not(:first-child) {
        text-align: right;
        font-variant-numeric: tabular-nums;
    }
    .my-stats.stats-page .data-table-scroll {
        overflow-x: auto;
        -webkit-overflow-scrolling: touch;
        max-width: 100%;
    }
    .my-stats.stats-page .match-stat {
        font-variant-numeric: tabular-nums;
    }
    @media (max-width: 900px) {
        /* Wide header needs the single-row tracks. Phone widths bring the
           stat labels back in the 720px block below. */
        .match-list-head { display: none; }
    }
    @media (max-width: 720px) {
        .my-stats.stats-page { padding: 1.25rem 1rem; }
        /* Phone widths: the season select takes the full actions row so the
           closed control is wide enough to show "Current season". */
        .stats-header { align-items: stretch; }
        .stats-header-actions { flex: 1 1 100%; }
        .stats-header-actions .season-select {
            flex: 1 1 100%;
            min-width: 0;
            width: 100%;
            max-width: 100%;
        }
        .stats-header-actions .season-select select {
            width: 100%;
            min-width: 0;
            max-width: 100%;
        }
        .overview-grid { grid-template-columns: 1fr; }
        .my-stats .stats-summary { grid-template-columns: 1fr; }
        /* W5b mobile: row1 outcome | identity | date; row2 equal stats strip.
           Compact E/D/A/DMG/HEAL labels sit on that same five-column track.
           The 900px rule hides the wide header; this block shows it again. */
        .match-list-head {
            display: grid;
            grid-template-columns: minmax(0, 1fr);
        }
        .match-list-head .match-h-outcome,
        .match-list-head .match-h-id,
        .match-list-head .match-h-date {
            display: none;
        }
        .match-list-head .match-stats {
            justify-self: stretch;
            width: 100%;
            grid-template-columns: repeat(5, minmax(0, 1fr));
        }
        .match-list-head .match-h-stat {
            white-space: nowrap;
            overflow: hidden;
            text-overflow: ellipsis;
        }
        .match-card {
            grid-template-columns: 4.5rem minmax(0, 1fr) auto;
            grid-template-areas:
                "outcome identity date"
                "stats stats stats";
            gap: 0.3rem 0.5rem;
        }
        .match-card .match-outcome { grid-area: outcome; }
        .match-card .match-identity { grid-area: identity; }
        .match-card .match-stats {
            grid-area: stats;
            justify-self: stretch;
            grid-template-columns: repeat(5, minmax(0, 1fr));
        }
        .match-card .match-date { grid-area: date; }
        .reader-badge-full {
            display: none;
        }
        .reader-badge-short {
            display: inline;
        }
        .match-hero-line {
            flex-wrap: wrap;
        }
        .match-hero-line .tracker-badge {
            flex-shrink: 1;
        }
        .my-stats .stats-tabs { overflow-x: auto; flex-wrap: nowrap; }
        .my-stats .stats-tab { white-space: nowrap; padding: 0.55rem 0.9rem; }
        .map-callouts { flex-direction: column; }
    }
"#;

/// Release note inside the tracker install and reinstall blocks.
fn tracker_release_notice(text: &str) -> Element {
    let text = text.to_string();
    rsx! {
        p { class: "tracker-release-notice", "{text}" }
    }
}

/// Member stats endpoints 401 when there is no session. Skip them until auth
/// settles on a signed-in user, so a signed-out visit does not fire those requests.
fn member_stats_url(auth: &AuthState, path: String) -> String {
    if auth.loading || !auth.is_logged_in() {
        String::new()
    } else {
        path
    }
}

#[component]
pub fn Stats() -> Element {
    let auth = use_auth();
    let relay = use_relay_availability_state();
    let tracker_notice = tracker_release_notice_text(relay.tracker_latest().as_deref());
    // "Total or per season": one selection drives every tab's fetch.
    // Nothing saved is all time. "Current season" is stored as a sentinel so a
    // rollover follows. Every season row is pinned by id, including the one
    // that is current now, an upcoming season, and a second flagged season.
    let season = use_stats_season();
    let stats = use_api_with::<PersonalStats>(move || {
        member_stats_url(&auth(), season.fetch_path("/api/stats/me"))
    });
    let heroes = use_api_with::<Vec<HeroStats>>(move || {
        member_stats_url(&auth(), season.fetch_path("/api/stats/me/heroes"))
    });
    let role_stats = use_api_with::<Vec<scuffed_types::RoleStats>>(move || {
        member_stats_url(&auth(), season.fetch_path(role::my_roles_path()))
    });
    let maps = use_api_with::<Vec<MapStats>>(move || {
        member_stats_url(&auth(), season.fetch_path("/api/stats/me/maps"))
    });
    let server_settings = use_api_with::<MemberSettingsResponse>(move || {
        member_stats_url(&auth(), "/api/stats/settings".to_string())
    });

    let mut tab = use_signal(|| StatsTab::Overview);
    let mut toast = use_toast();

    // Local editable state for the player_name field
    let mut player_name_input: Signal<String> = use_signal(String::new);
    let mut settings_saving = use_signal(|| false);

    // Populate input once settings load (only on first load, not on every re-render)
    let mut settings_loaded = use_signal(|| false);
    {
        let data = server_settings.data.read();
        if !settings_loaded() {
            let s = data.as_ref().and_then(|d| d.as_ref());
            if let Some(s) = s {
                let name = s.player_name.clone().unwrap_or_default();
                drop(data);
                player_name_input.set(name);
                settings_loaded.set(true);
            }
        }
    }

    let save_settings = move |_| {
        let name = player_name_input().trim().to_string();
        settings_saving.set(true);
        spawn(async move {
            let body = UpdateMemberSettingsRequest {
                player_name: if name.is_empty() { None } else { Some(name) },
                // The name field does not change who can see stats.
                stats_visibility: None,
            };
            match ApiClient::web()
                .put_json::<_, MemberSettingsResponse>("/api/stats/settings", &body)
                .await
            {
                Ok(_) => toast.show(Toast::success("Settings saved.")),
                Err(e) => toast.show(Toast::error(format!("Save failed: {e}"))),
            }
            settings_saving.set(false);
        });
    };

    let mut page_cursor = use_signal(|| Option::<String>::None);
    let mut cursor_history: Signal<Vec<Option<String>>> = use_signal(|| vec![None]);
    // History pagination belongs to one season window. Restart it whenever the
    // resolved season moves, including a pick synced from another tab. Page 1
    // is left alone so a season change does not refetch it twice.
    use_season_change(season, move || {
        if page_cursor.peek().is_some() || cursor_history.peek().len() > 1 {
            page_cursor.set(None);
            cursor_history.set(vec![None]);
        }
    });

    let matches = use_api_with::<MatchPage>(move || {
        let base = match page_cursor() {
            Some(c) => format!("/api/stats/me/matches?limit=25&cursor={c}"),
            None => "/api/stats/me/matches?limit=25".to_string(),
        };
        member_stats_url(&auth(), season.fetch_path(&base))
    });
    // Overview form strip: same endpoint, limit=10, no new backend (Q3).
    let form_matches = use_api_with::<MatchPage>(move || {
        member_stats_url(&auth(), season.fetch_path("/api/stats/me/matches?limit=10"))
    });

    let mut density = use_signal(initial_density);
    let hero_role = use_signal(|| "All");
    let hero_sort = use_signal(|| "matches");
    let hist_outcome = use_signal(|| "all");
    let hist_role = use_signal(|| "all");
    let hist_mode = use_signal(|| history_filter::ALL.to_string());

    // Progressive disclosure: once the member has tracked matches, the setup
    // chrome collapses to one slim row. Zero-match members keep the full
    // onboarding cards; that's their primary UI. While the summary is still
    // loading (or failed) we show the slim row rather than flashing onboarding.
    let has_matches = stats
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|s| s.total_matches > 0);
    let slim_tracker = has_matches != Some(false);
    let mut setup_open = use_signal(|| false);
    let show_full_setup = !slim_tracker || setup_open();

    let tab_body = match tab() {
        StatsTab::Overview => overview::overview_tab(heroes, role_stats, maps, form_matches),
        StatsTab::Heroes => heroes::heroes_tab(heroes, hero_role, hero_sort),
        StatsTab::Maps => maps::maps_tab(maps),
        StatsTab::History => history::history_tab(
            matches,
            page_cursor,
            cursor_history,
            hist_outcome,
            hist_role,
            hist_mode,
        ),
    };

    let dens = density();

    if !auth().loading && !auth().is_logged_in() {
        return rsx! {
            crate::styles::StyleOnce { id: "scuffed-css-pages-stats-mod-stats-css", css: STATS_CSS }
            div { class: "stats-page my-stats",
                h1 { "My Stats" }
                {member_pending(&auth(), &stats, "stats")}
            }
        };
    }

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-stats-mod-stats-css", css: STATS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-admin-shared", css: crate::styles::admin::CSS }

        div { class: "stats-page my-stats", "data-density": dens,
            div { class: "stats-header",
                h1 { "My Stats" }
                div { class: "stats-header-actions",
                    SeasonSelect {
                        id: "stats-season".to_string(),
                        name: "stats-season".to_string(),
                        seasons: season.season_list(),
                        seasons_error: season.seasons_error(),
                        on_retry: move |_| season.retry(),
                        value: season.selected_id(),
                        onchange: move |s: Option<String>| season.choose(s),
                    }
                    button {
                        class: "density-toggle",
                        title: "Toggle compact / comfortable density",
                        onclick: move |_| {
                            let next = if density() == "compact" {
                                "comfortable"
                            } else {
                                "compact"
                            };
                            density.set(next);
                            persist_density(next);
                        },
                        if dens == "compact" { "Density: Compact" } else { "Density: Comfortable" }
                    }
                    Link { to: Route::Leaderboards {}, "Leaderboards" }
                }
            }

            if slim_tracker {
                div { class: "tracker-slim",
                    {tracker_release_notice(&tracker_notice)}
                    span { class: "tracker-slim-label", "Tracker" }
                    span { class: "tracker-slim-sep", "·" }
                    span { class: "tracker-slim-note", "Linux only" }
                    div { class: "tracker-slim-actions",
                        button {
                            onclick: move |_| setup_open.toggle(),
                            if setup_open() { "Hide instructions" } else { "Reinstall instructions" }
                        }
                        Link { to: Route::StatsTokens {}, "Daemon Tokens" }
                    }
                }
            }

            if show_full_setup {
                // Daemon settings (collapsible)
                details { class: "daemon-settings",
                    summary { "⚙ Daemon Settings" }
                    div { class: "daemon-settings-row",
                        label { class: "daemon-settings-label", r#for: "player-name-input", "In-game name" }
                        input {
                            id: "player-name-input",
                            class: "daemon-settings-input",
                            r#type: "text",
                            placeholder: "e.g. FROZEN",
                            value: player_name_input(),
                            oninput: move |e| player_name_input.set(e.value()),
                        }
                        button {
                            class: "btn-primary",
                            disabled: settings_saving(),
                            onclick: save_settings,
                            if settings_saving() { "Saving…" } else { "Save" }
                        }
                    }
                    p { class: "daemon-settings-hint",
                        "The daemon uses this name to find your row on replay and post-match scoreboards."
                    }
                }

                // Tracker download / install. The slim row already shows the
                // notice, so this copy is only for members who have no matches.
                div { class: "tracker-download",
                    if !slim_tracker {
                        {tracker_release_notice(&tracker_notice)}
                    }
                    div { class: "tracker-download-head",
                        h3 { "Get the Stat Tracker" }
                        span { class: "tracker-badge", "Linux only" }
                    }
                    p {
                        "The tracker auto-captures your Overwatch scoreboards via OCR and uploads your stats here. "
                        {TRACKER_PLATFORM_COPY}
                    }
                    p { "On Arch or CachyOS, install it from the AUR (yay -S works too)." }
                    p {
                        "Used the install script before? Run "
                        code { "scuffed-stat-tracker-uninstall" }
                        " first, before the AUR install. It keeps your saved games and settings; the tracker relearns your name crops over a few games."
                    }
                    code { class: "tracker-install", "paru -S scuffed-stat-tracker-bin" }
                    p { "Then turn on the background service:" }
                    code { class: "tracker-install", "systemctl --user enable --now scuffed-stat-tracker.service" }
                    p {
                        "AUR installs update with "
                        code { "paru -Syu" }
                        ". Script installs keep updating from inside the app."
                    }
                    p { "On other distros, or without an AUR helper, use the install script:" }
                    code { class: "tracker-install",
                        "curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_CHANNEL=prerelease bash"
                    }
                    p {
                        "After installing, start the GUI, then paste this server's URL and a "
                        Link { to: Route::StatsTokens {}, "daemon token" }
                        " to begin uploading."
                    }
                    div { class: "tracker-download-links",
                        a {
                            href: "https://github.com/FrozenTear/scuffed-crew/releases/latest",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            "Latest release"
                        }
                    }
                }
            }

            // Summary (always visible)
            {
                let data = stats.data.read();
                let s = data.as_ref().and_then(|d| d.as_ref());
                match s {
                    None => member_pending(&auth(), &stats, "stats"),
                    Some(s) if s.total_matches == 0 => rsx! {
                        p { class: "empty-state",
                            "No matches tracked yet. Install the tracker above and play. Your stats land here automatically."
                        }
                    },
                    Some(s) => {
                        let season = Tally::new(s.total_matches, s.wins, s.losses, s.draws);
                        rsx! {
                            div { class: "stats-summary",
                                {crate::sample::season_tile(season)}
                                div { class: "stat-tile",
                                    div { class: "stat-tile-value", "{s.total_matches}" }
                                    div { class: "stat-tile-label", "Matches" }
                                }
                            }
                        }
                    }
                }
            }

            // Tabs
            div { class: "stats-tabs",
                button {
                    class: if tab() == StatsTab::Overview { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(StatsTab::Overview),
                    "Overview"
                }
                button {
                    class: if tab() == StatsTab::Heroes { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(StatsTab::Heroes),
                    "Heroes"
                }
                button {
                    class: if tab() == StatsTab::Maps { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(StatsTab::Maps),
                    "Maps"
                }
                button {
                    class: if tab() == StatsTab::History { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| {
                        tab.set(StatsTab::History);
                        page_cursor.set(None);
                        cursor_history.set(vec![None]);
                    },
                    "History"
                }
            }

            // Tab content
            {tab_body}
        }
    }
}

#[cfg(test)]
mod match_row_tests {
    use super::{MatchPage, PersonalMatch};

    fn row_json(extra: &str) -> String {
        format!(
            r#"{{
                "id": "m1",
                "hero": "Ana",
                "map_name": "Ilios",
                "game_mode": "control",
                "role": "Support",
                "outcome": "victory",
                "elims": 1,
                "deaths": 2,
                "assists": 3,
                "damage": 4,
                "healing": 5,
                "played_at": "2026-03-15T12:00:00Z"{extra}
            }}"#
        )
    }

    #[test]
    fn missing_reader_fields_default_to_old_reader_and_empty() {
        let row: PersonalMatch = serde_json::from_str(&row_json("")).expect("row");
        assert!(row.recognizer.is_none());
        assert!(row.suspect_fields.is_none());
        assert_eq!(row.mitigation, 0);
        assert_eq!(
            scuffed_types::effective_recognizer(row.recognizer.as_deref().unwrap_or("")),
            "ocr-v1"
        );

        let row = row_json("");
        let page: MatchPage =
            serde_json::from_str(&format!(r#"{{"data":[{row}],"next_cursor":null}}"#))
                .expect("page");
        assert!(page.data[0].recognizer.is_none());
        assert!(page.data[0].suspect_fields.is_none());
    }

    #[test]
    fn null_recognizer_and_suspect_fields_keep_the_page() {
        let raw = row_json(r#","recognizer":null,"suspect_fields":null"#);
        let page: MatchPage =
            serde_json::from_str(&format!(r#"{{"data":[{raw}],"next_cursor":null}}"#))
                .expect("page");
        assert!(page.data[0].recognizer.is_none());
        assert!(page.data[0].suspect_fields.is_none());
        let stored = page.data[0].recognizer.as_deref().unwrap_or("");
        assert_eq!(scuffed_types::effective_recognizer(stored), "ocr-v1");
    }

    #[test]
    fn empty_recognizer_counts_as_ocr_v1() {
        let raw = row_json(r#","recognizer":"","suspect_fields":[]"#);
        let row: PersonalMatch = serde_json::from_str(&raw).expect("row");
        assert_eq!(row.recognizer.as_deref(), Some(""));
        assert_eq!(row.suspect_fields.as_deref(), Some(&[][..]));
        assert_eq!(
            scuffed_types::effective_recognizer(row.recognizer.as_deref().unwrap_or("")),
            "ocr-v1"
        );
    }

    #[test]
    fn present_reader_fields_deserialize() {
        let raw = row_json(
            r#","mitigation":9,"recognizer":"cv-v1","suspect_fields":["hero","not-a-cell"]"#,
        );
        let page: MatchPage =
            serde_json::from_str(&format!(r#"{{"data":[{raw}],"next_cursor":null}}"#))
                .expect("page");
        assert_eq!(page.data[0].recognizer.as_deref(), Some("cv-v1"));
        assert_eq!(
            page.data[0].suspect_fields,
            Some(vec!["hero".to_string(), "not-a-cell".to_string()])
        );
        assert_eq!(page.data[0].mitigation, 9);
        assert!(page.next_cursor.is_none());
    }
}

#[cfg(test)]
mod header_layout_tests {
    use super::STATS_CSS;

    fn block_after<'a>(css: &'a str, marker: &str) -> &'a str {
        let start = css
            .find(marker)
            .unwrap_or_else(|| panic!("missing {marker}"));
        &css[start..]
    }

    #[test]
    fn phone_width_gives_the_season_select_the_full_row() {
        let actions = block_after(STATS_CSS, ".stats-header-actions {");
        let actions_head = actions.split('}').next().unwrap();
        assert!(actions_head.contains("flex-wrap: wrap"), "{actions_head}");
        assert!(actions_head.contains("min-width: 0"), "{actions_head}");

        let phone = block_after(STATS_CSS, "@media (max-width: 720px)");
        let season = block_after(phone, ".stats-header-actions .season-select {");
        let season_head = season.split('}').next().unwrap();
        assert!(season_head.contains("flex: 1 1 100%"), "{season_head}");
        assert!(season_head.contains("min-width: 0"), "{season_head}");
        assert!(has_declaration(season_head, "width: 100%"), "{season_head}");
        let select = block_after(phone, ".stats-header-actions .season-select select {");
        let select_head = select.split('}').next().unwrap();
        assert!(select_head.contains("min-width: 0"), "{select_head}");
        assert!(has_declaration(select_head, "width: 100%"), "{select_head}");
    }

    fn has_declaration(block: &str, decl: &str) -> bool {
        block
            .split(';')
            .any(|part| part.lines().any(|line| line.trim() == decl))
    }

    #[test]
    fn width_declaration_does_not_match_max_width() {
        assert!(has_declaration(
            "width: 100%; max-width: 100%;",
            "width: 100%"
        ));
        assert!(!has_declaration("max-width: 100%;", "width: 100%"));
        assert!(has_declaration("max-width: 100%;", "max-width: 100%"));
    }

    #[test]
    fn narrow_history_row_keeps_badge_mark_and_flagged_value() {
        let phone = block_after(STATS_CSS, "@media (max-width: 720px)");
        let full = block_after(phone, ".reader-badge-full");
        let full_head = full.split('}').next().unwrap();
        assert!(
            has_declaration(full_head, "display: none"),
            "narrow widths hide the long badge: {full_head}"
        );
        let short = block_after(phone, ".reader-badge-short");
        let short_head = short.split('}').next().unwrap();
        assert!(
            has_declaration(short_head, "display: inline"),
            "narrow widths show the short badge: {short_head}"
        );
        let line = block_after(phone, ".match-hero-line");
        let line_head = line.split('}').next().unwrap();
        assert!(line_head.contains("flex-wrap: wrap"), "{line_head}");
        let badge = block_after(phone, ".match-hero-line .tracker-badge");
        let badge_head = badge.split('}').next().unwrap();
        assert!(badge_head.contains("flex-shrink: 1"), "{badge_head}");

        let value = block_after(STATS_CSS, ".stat-value-text {");
        let value_head = value.split('}').next().unwrap();
        assert!(
            value_head.contains("text-overflow: ellipsis"),
            "only the value text is clipped: {value_head}"
        );
        let hero = block_after(STATS_CSS, ".match-card .match-hero {");
        let hero_head = hero.split('}').next().unwrap();
        assert!(
            !hero_head.contains("text-overflow"),
            "the hero box must not clip its mark: {hero_head}"
        );
        let mark = block_after(STATS_CSS, ".stat-unsure-mark {");
        let mark_head = mark.split('}').next().unwrap();
        assert!(mark_head.contains("flex-shrink: 0"), "{mark_head}");

        let map = block_after(STATS_CSS, ".match-card .match-map {");
        let map_head = map.split('}').next().unwrap();
        assert!(map_head.contains("flex-wrap: wrap"), "{map_head}");
        assert!(
            !map_head.contains("white-space: nowrap"),
            "the map line must not ellipsize flagged values: {map_head}"
        );
        let flagged = block_after(STATS_CSS, ".stat-flagged {");
        let flagged_head = flagged.split('}').next().unwrap();
        assert!(flagged_head.contains("flex: 0 0 auto"), "{flagged_head}");
        assert!(
            flagged_head.contains("white-space: nowrap"),
            "{flagged_head}"
        );
    }

    #[test]
    fn phone_history_shows_compact_stat_headings() {
        let phone = block_after(STATS_CSS, "@media (max-width: 720px)");
        let head = block_after(phone, ".match-list-head {");
        let head_block = head.split('}').next().unwrap();
        assert!(
            has_declaration(head_block, "display: grid"),
            "390px widths show the history headings: {head_block}"
        );
        assert!(
            !has_declaration(head_block, "display: none"),
            "{head_block}"
        );
        let stats = block_after(phone, ".match-list-head .match-stats {");
        let stats_block = stats.split('}').next().unwrap();
        assert!(
            stats_block.contains("repeat(5, minmax(0, 1fr))"),
            "E/D/A/DMG/HEAL share the phone stat tracks: {stats_block}"
        );
        let hidden = block_after(phone, ".match-list-head .match-h-outcome,");
        assert!(
            hidden.contains("display: none"),
            "only the empty outcome, map, and date header cells stay hidden: {hidden}"
        );
        assert!(
            !phone.contains(".stat-unsure-mark { display: none"),
            "the unsure mark stays visible at phone width"
        );
    }

    #[test]
    fn history_header_and_values_share_column_box() {
        assert!(
            !STATS_CSS.contains("0.85rem + 1px"),
            "header inset must use the same pad as the cards"
        );
        let page = block_after(STATS_CSS, ".my-stats.stats-page {");
        let page_block = page.split('}').next().unwrap();
        assert!(
            has_declaration(page_block, "--history-pad-x: 0.85rem"),
            "{page_block}"
        );
        assert!(
            has_declaration(page_block, "--history-border: 1px"),
            "{page_block}"
        );
        assert!(
            STATS_CSS
                .contains(".stats-page[data-density=\"comfortable\"] { --history-pad-x: 1.1rem; }")
        );
        assert!(
            STATS_CSS
                .contains(".stats-page[data-density=\"compact\"] { --history-pad-x: 0.75rem; }")
        );

        let shell = block_after(STATS_CSS, ".match-list-head,\n    .match-card {");
        let shell_block = shell.split('}').next().unwrap();
        assert!(
            has_declaration(shell_block, "box-sizing: border-box"),
            "{shell_block}"
        );
        assert!(
            has_declaration(shell_block, "padding-left: var(--history-pad-x)"),
            "{shell_block}"
        );
        assert!(
            has_declaration(shell_block, "padding-right: var(--history-pad-x)"),
            "{shell_block}"
        );
        assert!(
            shell_block.contains("border: var(--history-border) solid transparent"),
            "{shell_block}"
        );

        let text_cols = block_after(
            STATS_CSS,
            ".match-h-outcome,\n    .match-card .match-outcome,",
        );
        let text_block = text_cols.split('}').next().unwrap();
        assert!(text_block.contains(".match-h-id"), "{text_block}");
        assert!(
            text_block.contains(".match-card .match-identity"),
            "{text_block}"
        );
        assert_shared_cell(text_block, "left");

        let num_cols = block_after(STATS_CSS, ".match-h-stat,\n    .match-card .match-stat,");
        let num_block = num_cols.split('}').next().unwrap();
        assert!(num_block.contains(".match-h-date"), "{num_block}");
        assert!(num_block.contains(".match-card .match-date"), "{num_block}");
        assert_shared_cell(num_block, "right");

        let tracks = block_after(
            STATS_CSS,
            ".match-list-head .match-stats,\n    .match-card .match-stats {",
        );
        let tracks_block = tracks.split('}').next().unwrap();
        assert!(
            has_declaration(tracks_block, "box-sizing: border-box"),
            "{tracks_block}"
        );
        assert!(
            has_declaration(tracks_block, "padding: 0"),
            "{tracks_block}"
        );
    }

    fn assert_shared_cell(block: &str, align: &str) {
        assert!(has_declaration(block, "box-sizing: border-box"), "{block}");
        assert!(has_declaration(block, "padding: 0"), "{block}");
        assert!(
            has_declaration(block, &format!("text-align: {align}")),
            "{block}"
        );
    }
}

#[cfg(test)]
mod tracker_platform_copy_tests {
    use super::{TRACKER_PLATFORM_COPY, TRACKER_RELEASE, tracker_release_notice_text};

    /// The install blurb on `/stats`, from the heading through the curl command.
    fn tracker_download_blurb(src: &str) -> &str {
        let start = src
            .find("Get the Stat Tracker")
            .expect("stats page heading");
        let rest = &src[start..];
        let end = rest.find("tracker-install").expect("install command");
        &rest[..end]
    }

    #[test]
    fn stats_page_names_wayland_and_x11_without_an_em_dash() {
        assert_eq!(
            TRACKER_PLATFORM_COPY,
            "It runs on Linux, on both Wayland and X11. There is no Windows or macOS build."
        );
        assert!(TRACKER_PLATFORM_COPY.contains("Wayland"));
        assert!(TRACKER_PLATFORM_COPY.contains("X11"));
        assert!(
            !TRACKER_PLATFORM_COPY.contains('\u{2014}'),
            "{TRACKER_PLATFORM_COPY}"
        );
        assert!(
            !TRACKER_PLATFORM_COPY.contains('\u{2013}'),
            "{TRACKER_PLATFORM_COPY}"
        );

        let src = include_str!("mod.rs");
        let blurb = tracker_download_blurb(src);
        assert!(
            blurb.contains("TRACKER_PLATFORM_COPY"),
            "the /stats install blurb must render the platform line: {blurb}"
        );
        // Built at runtime so this test file does not itself contain the old phrase.
        let wayland_only = ["Wayland", "only"].join(" ");
        assert!(
            !blurb.contains(&wayland_only),
            "the /stats install blurb still limits the tracker to one display server: {blurb}"
        );
        assert!(!blurb.contains('\u{2014}'), "{blurb}");
        assert!(
            !src.contains(&wayland_only),
            "crates/app stats page still limits the tracker to one display server"
        );
    }

    #[test]
    fn release_notice_names_the_current_tracker_build() {
        assert_eq!(TRACKER_RELEASE, "rc.9");
        let pinned = tracker_release_notice_text(None);
        assert_eq!(
            pinned,
            format!("Install {TRACKER_RELEASE} between games, not during one.")
        );
        assert!(!pinned.contains('\u{2014}'));
        assert!(!pinned.contains('\u{2013}'));
        assert!(pinned.contains(TRACKER_RELEASE));
        let src = include_str!("mod.rs");
        // Built here so this test file does not itself contain the old labels.
        for previous in [
            ["alpha", "8"],
            ["alpha", "10"],
            ["alpha", "12"],
            ["rc", "1"],
            ["rc", "2"],
            ["rc", "3"],
            ["rc", "4"],
            ["rc", "5"],
            ["rc", "6"],
            ["rc", "7"],
            ["rc", "8"],
        ] {
            let label = previous.join(".");
            assert!(
                !src.contains(&label),
                "the install note must not keep the previous tracker build {label}"
            );
        }
        let prod = src.split("mod tracker_platform_copy_tests").next().unwrap();
        assert_eq!(
            prod.matches("rc.9").count(),
            1,
            "the build name lives only on TRACKER_RELEASE"
        );
        assert!(
            !prod.contains("Install alpha."),
            "the sentence must be built from TRACKER_RELEASE"
        );
        let call = "{tracker_release_notice(&tracker_notice)}";
        let slim = src.find("if slim_tracker {").expect("slim tracker");
        assert!(
            !src[..slim].contains(call),
            "the notice is not a page-wide banner"
        );
        let full = src[slim..].find("if show_full_setup").expect("full setup");
        let slim_body = &src[slim..slim + full];
        assert!(
            slim_body.contains(call),
            "the collapsed reinstall row still shows the notice"
        );
        let download = src
            .find("class: \"tracker-download\"")
            .expect("install block");
        let download_body = &src[download..];
        let guard = download_body
            .find("if !slim_tracker {")
            .expect("install notice is skipped while the slim row is up");
        assert!(
            download_body[guard..].contains(call),
            "members with no matches still see the notice in the install block"
        );
    }

    #[test]
    fn install_script_command_installs_the_prerelease_the_note_names() {
        let src = include_str!("mod.rs");
        let prod = src.split("mod tracker_platform_copy_tests").next().unwrap();
        let script = "crates/stat-tracker/dist/bootstrap.sh";
        let at = prod.find(script).expect("install script command");
        let line = prod[at..].lines().next().unwrap();
        // The channel is assigned on bash, not curl, so bootstrap.sh sees it.
        assert!(
            line.contains("| STAT_TRACKER_CHANNEL=prerelease bash"),
            "the install script must take the prerelease channel the note names: {line}"
        );
    }

    #[test]
    fn aur_steps_uninstall_the_script_build_before_installing() {
        let src = include_str!("mod.rs");
        let prod = src.split("mod tracker_platform_copy_tests").next().unwrap();
        let uninstall = prod
            .find("code { \"scuffed-stat-tracker-uninstall\" }")
            .expect("uninstall step");
        let paru = prod.find("paru -S scuffed").expect("AUR install step");
        let enable = prod.find("systemctl --user enable").expect("enable step");
        assert!(
            uninstall < paru && paru < enable,
            "the script uninstaller disables the shared service, so it runs before the AUR steps"
        );
    }

    #[test]
    fn release_notice_formats_tracker_latest_like_the_pinned_fallback() {
        let pinned = tracker_release_notice_text(None);
        assert_eq!(
            tracker_release_notice_text(Some("0.5.0-rc.9")),
            pinned,
            "the current bare version reads the same as the pin"
        );
        assert_eq!(
            tracker_release_notice_text(Some("  0.5.0-alpha.11  ")),
            "Install alpha.11 between games, not during one."
        );
        assert_eq!(tracker_release_notice_text(Some("")), pinned);
        assert_eq!(tracker_release_notice_text(Some("   ")), pinned);
        assert_eq!(tracker_release_notice_text(Some("\n")), pinned);
        let shown = tracker_release_notice_text(Some("0.5.0-rc.9"));
        assert!(!shown.contains("0.5.0-"));
        assert!(!shown.contains("stat-tracker-v"));
        assert!(!shown.contains('\u{2014}'));
    }
}

#[cfg(test)]
mod signed_out_tests {
    use super::member_stats_url;
    use crate::state::AuthState;
    use scuffed_types::{OrgRole, UserInfo};

    fn member() -> AuthState {
        AuthState {
            user: Some(UserInfo {
                id: "u1".into(),
                username: "dev".into(),
                avatar_url: None,
                role: Some(OrgRole::Member),
            }),
            loading: false,
        }
    }

    #[test]
    fn signed_out_and_loading_skip_member_stats_urls() {
        let signed_out = AuthState {
            user: None,
            loading: false,
        };
        assert_eq!(member_stats_url(&signed_out, "/api/stats/me".into()), "");
        let loading = AuthState::new();
        assert!(loading.loading);
        assert_eq!(member_stats_url(&loading, "/api/stats/settings".into()), "");
        assert_eq!(
            member_stats_url(&member(), "/api/stats/me/heroes".into()),
            "/api/stats/me/heroes"
        );
    }

    #[test]
    fn signed_out_branch_returns_before_the_stats_chrome() {
        let src = include_str!("mod.rs");
        let prod = src.split("mod signed_out_tests").next().unwrap();
        let gate = prod
            .find("if !auth().loading && !auth().is_logged_in()")
            .expect("signed-out gate");
        let chrome = prod.find("Density: Compact").expect("density control");
        assert!(
            gate < chrome,
            "signed-out visitors must not reach the stats chrome"
        );
        assert!(
            !prod[gate..chrome].contains("Couldn't load"),
            "the sign-in return must not render load errors"
        );
        assert_eq!(
            prod.matches("member_stats_url(&auth()").count(),
            7,
            "summary, heroes, roles, maps, settings, history, and form"
        );
        for path in [
            "\"/api/stats/me\"",
            "\"/api/stats/me/heroes\"",
            "role::my_roles_path()",
            "\"/api/stats/me/maps\"",
            "\"/api/stats/settings\"",
            "\"/api/stats/me/matches?limit=25\"",
            "\"/api/stats/me/matches?limit=10\"",
        ] {
            assert!(prod.contains(path), "missing member stats path {path}");
        }
    }
}

#[cfg(test)]
mod map_mode_tests {
    use super::map_game_mode;

    #[test]
    fn grimsvotn_variants_bucket_as_escort() {
        for name in [
            "Watchpoint: Grímsvötn",
            "Watchpoint: Grimsvotn",
            "Watchpoint: Grímsvotn",
            "Watchpoint: Grimsvötn",
            "Watchpoint: Gri\u{0301}msvo\u{0308}tn",
            "WATCHPOINT: grímsvötn",
            "watchpoint grimsvotn",
            "watchpointgrimsvotn",
            "grimsvotn",
            "grímsvötn",
            "grimsvötn",
            "grímsvotn",
        ] {
            assert_eq!(map_game_mode(name), "Escort", "{name:?}");
        }
        assert_eq!(map_game_mode("Watchpoint: Gibraltar"), "Escort");
        assert_eq!(map_game_mode("Watchpoint"), "Escort");
        assert_eq!(map_game_mode("gibraltar"), "Escort");
        assert_eq!(map_game_mode("not a map"), "Other");
    }
}
