mod bans_picks;
mod game;
mod heroes;
mod history;
mod history_filter;
mod maps;
mod overview;
mod rank;
pub(crate) mod role;

pub use game::StatsMatch;

use dioxus::prelude::*;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{MemberSettingsResponse, UpdateMemberSettingsRequest};

use crate::components::member_gate::Wall;
use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pill, PillTone, SeasonSelect,
    TabButton, btn_class, use_season_change, use_stats_season,
};
use crate::components::{Toast, member_pending, use_toast};
use crate::hooks::use_api_with;
use crate::routes::Route;
use crate::sample::Tally;
use crate::state::{AuthState, use_auth, use_relay_availability_state};
use crate::stats_privacy;

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
    /// Bans and picks for the card line. Missing, null or a value that does
    /// not parse is None. See [`bans_picks`].
    #[serde(default, deserialize_with = "bans_picks::lenient")]
    bans_picks: Option<scuffed_types::BansPicks>,
    /// Rank update read for this game. Missing, null, `{}`, all-null or a
    /// value that does not parse is None. See [`rank`].
    #[serde(default, deserialize_with = "rank::lenient")]
    rank: Option<scuffed_types::MatchRank>,
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

/// The one tabpanel under the My Stats tabs.
const MY_STATS_PANEL: &str = "my-stats-panel";

/// Id of a My Stats tab button. The tabpanel is labelled by the selected one.
fn stats_tab_id(tab: StatsTab) -> &'static str {
    match tab {
        StatsTab::Overview => "my-stats-tab-overview",
        StatsTab::Heroes => "my-stats-tab-heroes",
        StatsTab::Maps => "my-stats-tab-maps",
        StatsTab::History => "my-stats-tab-history",
    }
}

// -- Shared helpers --

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
        "var(--text-subtle)"
    } else if wins2 > total {
        "var(--chart-wr-up)"
    } else {
        "var(--chart-wr-down)"
    }
}

/// Platform line on `/stats`. Linux supports Wayland and X11. No other OS builds.
const TRACKER_PLATFORM_COPY: &str =
    "It runs on Linux, on both Wayland and X11. There is no Windows or macOS build.";

/// The stat tracker's manifest, read at compile time. Its `[package]`
/// `version` is the tracker release: `crates/stat-tracker/dist/check-tag-version.sh`
/// and `crates/stat-tracker/src/tests_release_pins.rs` fail when it differs
/// from the release tag, so the version bump that cuts a release also
/// updates the install note here. No separate bump step.
const TRACKER_CORE_MANIFEST: &str = include_str!("../../../../stat-tracker-core/Cargo.toml");

/// `[package]` `version = "..."` from a Cargo manifest. Same rule as
/// `package_version` in `check-tag-version.sh`: the first `version` line
/// inside `[package]`, value between the first pair of double quotes.
fn manifest_package_version(manifest: &str) -> Option<&str> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = line.strip_prefix("version") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let (_, quoted) = rest.split_once('"')?;
        let (value, _) = quoted.split_once('"')?;
        return Some(value).filter(|value| !value.is_empty());
    }
    None
}

/// Stat-tracker release (`MAJOR.MINOR.PATCH-alpha.N` style), from
/// [`TRACKER_CORE_MANIFEST`]. Named in the install note on `/stats` when
/// health does not send `tracker_latest`.
fn tracker_release_version() -> Option<&'static str> {
    manifest_package_version(TRACKER_CORE_MANIFEST)
}

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
/// core, so `MAJOR.MINOR.PATCH-alpha.N` reads `alpha.N`. A value that is already that
/// short label is kept. The server's `tracker_latest` wins when present;
/// otherwise the label comes from [`tracker_release_version`].
fn tracker_notice_label(server: Option<&str>) -> String {
    let non_blank = |value: &&str| !value.is_empty();
    let Some(raw) = server
        .map(str::trim)
        .filter(non_blank)
        .or_else(|| tracker_release_version().map(str::trim).filter(non_blank))
    else {
        return "the latest release".to_string();
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

/// The calendar day of `at` in `zone`. The offset is the zone's offset at
/// `at` itself, so a game played before a daylight saving change keeps the
/// day it was played on.
fn day_in<Tz: chrono::TimeZone>(at: DateTime<Utc>, zone: &Tz) -> chrono::NaiveDate {
    at.with_timezone(zone).date_naive()
}

/// The calendar day a game is shown on, everywhere on the stats pages: the
/// viewer's own day in the browser (as the match page's local time), the
/// UTC day in native tests. Each game converts with its own instant, never
/// with today's offset.
pub(super) fn local_day(at: DateTime<Utc>) -> chrono::NaiveDate {
    #[cfg(target_arch = "wasm32")]
    let day = day_in(at, &chrono::Local);
    #[cfg(not(target_arch = "wasm32"))]
    let day = day_in(at, &Utc);
    day
}

/// The instant `at` on the viewer's clock in the browser (UTC in native
/// tests), with the offset that applied at `at` itself. Its date is always
/// [`local_day`].
pub(super) fn local_time(at: DateTime<Utc>) -> DateTime<chrono::FixedOffset> {
    #[cfg(target_arch = "wasm32")]
    let local = at.with_timezone(&chrono::Local).fixed_offset();
    #[cfg(not(target_arch = "wasm32"))]
    let local = at.fixed_offset();
    local
}

/// "16 Jul 2026, 00:30 UTC+2": a date and time on the viewer's own clock
/// ([`local_time`]) with the offset it is in ([`utc_offset_label`]). The
/// stats match page and the tracker tokens page both write times this way.
pub(crate) fn local_date_time(at: &DateTime<Utc>) -> String {
    date_time_label(&local_time(*at))
}

/// A time already on some clock, written with that clock's offset.
fn date_time_label(local: &DateTime<chrono::FixedOffset>) -> String {
    format!(
        "{} {}",
        local.format("%-d %b %Y, %H:%M"),
        utc_offset_label(local.offset())
    )
}

/// `UTC`, `UTC+2`, `UTC-5` or `UTC+5:30`: the offset a local time is in.
pub(super) fn utc_offset_label(offset: &chrono::FixedOffset) -> String {
    let east = offset.local_minus_utc() / 60;
    if east == 0 {
        return "UTC".to_string();
    }
    let sign = if east > 0 { '+' } else { '-' };
    let (hours, minutes) = (east.abs() / 60, east.abs() % 60);
    if minutes == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{minutes:02}")
    }
}

/// Shared map → game mode (used by maps tab + overview mode chips).
/// Delegates to [`scuffed_types::MapName`] so accented/unaccented live names
/// (Paraíso/Paraiso, Esperança/Esperanca, Watchpoint: Grímsvötn/Grimsvotn)
/// and Neon Junction bucket correctly.
pub(super) fn map_game_mode(name: &str) -> &'static str {
    scuffed_types::MapName::game_mode_label(name)
}

/// Every set-up-the-tracker link on `/stats`: the install steps lower on
/// the same page.
pub(super) const TRACKER_ANCHOR: &str = "#stats-tracker";

/// True when the picker holds one season by id and that season is not the
/// current one. All time and "Current season" are not; a pinned id that is
/// current now is not; an id missing from the list is treated as past.
fn picked_season_is_not_current(
    selected: Option<&str>,
    seasons: Option<&[scuffed_types::Season]>,
) -> bool {
    match selected.map(str::trim).filter(|id| !id.is_empty()) {
        None | Some("current") => false,
        Some(id) => seasons.is_some_and(|list| !list.iter().any(|s| s.id == id && s.is_current)),
    }
}

/// The zero-match card's message. All time has no season to name.
fn no_logged_games_message(season: Option<&str>) -> &'static str {
    match season {
        None => {
            "Nothing is logged yet. Install the stat tracker below and play: your games land here on their own."
        }
        Some(_) => {
            "Nothing is logged for this season. Install the stat tracker below and play: your games land here on their own."
        }
    }
}

/// Lead under "Your stats": who is signed in and which games the figures
/// cover. `season` is the picker value: `None` is all time, `current` follows
/// the current season, anything else is one pinned season.
fn stats_lead(username: Option<&str>, season: Option<&str>) -> String {
    let scope = match season.map(str::trim).filter(|id| !id.is_empty()) {
        None => "Figures are from all of your logged matches.",
        Some("current") => "Figures are from your logged matches this season.",
        Some(_) => "Figures are from your logged matches in the season you picked.",
    };
    match username.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("Signed in as {name}. {scope}"),
        None => scope.to_string(),
    }
}

/// K/D tile text: two decimals, or "N/A" before there is a death to divide by.
/// What the K/D tile knows. "N/A" means a ratio with no deaths, so it is
/// never shown while the role rows are still loading or failed.
#[derive(Debug, Clone, Copy, PartialEq)]
enum KdTile {
    /// The role rows have not arrived.
    Loading,
    /// The role rows failed; Time on role carries the error and Retry.
    Failed,
    /// Loaded: the ratio, or None with no deaths.
    Ready(Option<f64>),
}

/// The tile value (visible) and, when the value is a dash, the words a
/// screen reader hears instead.
fn kd_text(kd: KdTile) -> (String, Option<&'static str>) {
    match kd {
        KdTile::Ready(Some(kd)) => (format!("{kd:.2}"), None),
        KdTile::Ready(None) => ("N/A".to_string(), None),
        KdTile::Loading => ("-".to_string(), Some("Loading")),
        KdTile::Failed => ("-".to_string(), Some("Not available")),
    }
}

/// The three summary tiles of the frame: win rate, matches, K/D. The win
/// rate follows the small-sample rule in [`crate::sample`]: under 10 decided
/// games the tile shows the record and reads "Too few games". From 10 on, the
/// record sits in a line under the row.
fn summary_tiles(season: Tally, kd: KdTile) -> Element {
    let rate = season.label();
    let (rate_label, record) = if season.has_win_rate() {
        ("Win rate", Some(season.record()))
    } else {
        (crate::sample::TOO_FEW_GAMES, None)
    };
    let (kd, kd_words) = kd_text(kd);
    rsx! {
        div { class: "stats-summary",
            div { class: "stats-tiles",
                div { class: "stat-tile",
                    div { class: "stat-tile-value", "{rate}" }
                    div { class: "stat-tile-label", "{rate_label}" }
                }
                div { class: "stat-tile",
                    div { class: "stat-tile-value", "{season.games}" }
                    div { class: "stat-tile-label", "Matches" }
                }
                div { class: "stat-tile",
                    if let Some(words) = kd_words {
                        div { class: "stat-tile-value",
                            span { aria_hidden: "true", "{kd}" }
                            span { class: "visually-hidden", "{words}" }
                        }
                    } else {
                        div { class: "stat-tile-value", "{kd}" }
                    }
                    div { class: "stat-tile-label", "K/D" }
                }
            }
            if let Some(record) = record {
                p { class: "stats-summary-note", "Record {record}. K/D is elims per death." }
            } else {
                p { class: "stats-summary-note", "K/D is elims per death." }
            }
        }
    }
}

/// Fetch-failed state with a retry button (bumps the resource's refresh
/// counter). `reason` is the resource's mapped error (`ApiResource::error`),
/// so the line says why (QA on 748); none leaves the lead alone.
pub(crate) fn load_error_state(
    what: &'static str,
    reason: Option<String>,
    mut refresh: Signal<u64>,
) -> Element {
    rsx! {
        LoadError {
            message: crate::client_error::join_lead(
                &format!("Couldn't load {what}."),
                reason.as_deref().unwrap_or_default(),
            ),
            on_retry: move |_| refresh += 1,
        }
    }
}

const STATS_CSS: &str = r#"
    .my-stats.stats-page {
        /* One horizontal inset for history headers and cards. The card pad
           token follows the density chosen in Settings (data-density on the
           root element). */
        --history-pad-x: var(--space-card-pad);
        --history-border: 1px;
    }
    /* The page column is the shared PageShell (wide). The sign-in wall starts
       at its left edge, under the title, as the frame draws it. */
    .my-stats.stats-page .stats-shell > .wall { margin: 0; }
    /* The failed and gate cards in the tabpanel start at the same left edge
       as the tiles, the tabs and the sections under them. */
    .my-stats .stats-panel > .wall { margin: 0; }
    .my-stats .stats-visibility-note {
        margin: var(--space-2) 0 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    .my-stats .stats-visibility-note a {
        color: var(--accent-text);
        text-decoration: underline;
    }
    /* One row under the page header: the tabs, then the season picker and
       the link to the leaderboards. */
    .stats-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        flex-wrap: wrap;
        gap: var(--space-3) var(--space-4);
        min-width: 0;
    }
    .stats-header-actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        min-width: 0;
        max-width: 100%;
    }
    .stats-header-actions .season-select select { min-height: var(--hit-target-min); }
    /* The tabs are the shared chips in a ChipRow. */
    .my-stats .stats-tabs { flex: 0 1 auto; }
    /* The tabpanel: summary tiles, then the selected tab. */
    .stats-panel {
        display: flex;
        flex-direction: column;
        gap: var(--space-section);
        min-width: 0;
    }
    .stats-panel:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .stats-tab-body { min-width: 0; }
    .stats-tab-body:empty { display: none; }
    /* A card on the stats pages: the card tokens, padded by density. */
    .stats-card {
        box-sizing: border-box;
        min-width: 0;
        padding: var(--card-padding);
        background: var(--card-bg);
        color: var(--card-fg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
    }
    /* A block that is loading or has nothing to show yet. */
    .stats-state {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-md);
        line-height: var(--leading-body);
    }
    /* Win-rate text values: plain text tokens, muted below the min-games gate.
       No traffic light; polarity lives in the bar vs the 50% hairline. */
    .my-stats .stats-wr {
        display: inline-block;
        font-size: var(--text-sm);
        font-weight: var(--font-label-weight);
        color: var(--text);
        font-variant-numeric: tabular-nums;
    }
    .my-stats .stats-wr.muted { color: var(--text-subtle); font-weight: 400; }
    /* Outcome chips: status tokens as status only. Win = good, defeat =
       serious/danger, draw = neutral text (never gold). */
    .outcome-win { color: var(--win); font-weight: 600; }
    .outcome-loss { color: var(--loss); font-weight: 600; }
    .outcome-draw { color: var(--text-3); font-weight: 600; }
    /* W5b history layout: outcome | identity | E D A | dmg heal | date.
       Header + cards share the outer grid; numeric cols live in .match-stats
       with fixed tracks so values column-scan. Tighter than free scoreline. */
    .match-list { display: flex; flex-direction: column; gap: var(--space-1); min-width: 0; }
    .match-list-head,
    .match-card {
        display: grid;
        box-sizing: border-box;
        border: var(--history-border) solid transparent;
        padding-left: var(--history-pad-x);
        padding-right: var(--history-pad-x);
        grid-template-columns:
            minmax(calc(var(--space-8) * 2), calc(var(--space-10) * 2))
            minmax(calc(var(--space-12) * 2), 1.4fr)
            auto
            minmax(calc(var(--space-10) * 2), auto);
        gap: var(--space-1) var(--space-3);
        align-items: center;
    }
    .match-list-head .match-stats,
    .match-card .match-stats {
        box-sizing: border-box;
        padding: 0;
    }
    .match-stats {
        display: grid;
        grid-template-columns:
            var(--space-10) var(--space-10) var(--space-10)
            calc(var(--space-8) * 2) calc(var(--space-8) * 2);
        gap: var(--space-1) var(--space-2);
        align-items: center;
        justify-self: end;
    }
    .match-list-head {
        padding-top: 0;
        padding-bottom: var(--space-1);
        border-color: transparent;
        font-family: var(--font-head);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
        color: var(--text-subtle);
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
        min-height: var(--density-row);
        background: var(--card-bg);
        border-color: var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
        padding-top: var(--space-2);
        padding-bottom: var(--space-2);
        font-size: var(--text-md);
        transition: background 0.15s;
    }
    .match-card:hover { background: var(--surface-sunken); }
    /* History rows open the game page through a stretched link: an empty anchor
       that covers the whole row. Marked values sit above it, so a button is
       never inside the link. */
    .match-card-link { position: relative; isolation: isolate; cursor: pointer; }
    .match-card-overlay {
        position: absolute;
        inset: 0;
        border-radius: inherit;
        color: inherit;
        text-decoration: none;
    }
    .match-card-overlay:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .match-cards {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        margin: 0;
        padding: 0;
        list-style: none;
    }
    /* The result is the shared pill (Win, Loss or Draw as a word on a soft
       fill). This cell only places it. */
    .match-card .match-outcome {
        font-size: var(--text-sm);
        line-height: var(--leading-ui);
        display: flex;
        align-items: center;
        min-width: 0;
        overflow: hidden;
    }
    .match-identity { min-width: 0; }
    .match-hero-line {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: var(--space-2);
        min-width: 0;
    }
    .match-card .match-hero {
        color: var(--text);
        font-family: var(--font-head);
        font-weight: var(--font-label-weight);
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
    .reader-badge-short { display: none; }
    /* Unsure cells: a dotted underline plus a question mark, not color alone. */
    .stat-unsure .stat-value-text {
        text-decoration-line: underline;
        text-decoration-style: dotted;
        text-underline-offset: 0.18em;
    }
    /* The mark is a button, so reset the browser look and keep the old size.
       It and its open note are positioned above the row overlay link, so
       tapping either one never opens the game. Each sets that in its own single
       rule, so no other rule can override it by order. */
    .stat-unsure-mark {
        position: relative;
        z-index: 1;
        flex-shrink: 0;
        margin: 0 0 0 0.1em;
        padding: 0 0.2em;
        border: 0;
        background: none;
        color: inherit;
        font-family: inherit;
        cursor: pointer;
        font-size: var(--text-xs);
        font-weight: 700;
        vertical-align: super;
        line-height: 0;
        text-decoration: none;
    }
    /* One size for the hit box and for the clearance the open note keeps from it. */
    .stat-unsure { --unsure-hit: var(--space-6); }
    /* The glyph is about 9px with a zero-height box, far below a tap target.
       The hit box is a 24 by 24px ::after centered on the glyph. It is part of
       the button, so it sits above the row overlay link with the button's
       z-index, and a tap a little off the glyph still lands on the mark. The
       focus ring is drawn on that box, not on the collapsed button. */
    .stat-unsure-mark::after {
        content: "";
        position: absolute;
        top: 50%;
        left: 50%;
        width: var(--unsure-hit);
        height: var(--unsure-hit);
        transform: translate(-50%, -50%);
        border-radius: var(--radius-control);
    }
    .stat-unsure-mark:focus-visible {
        outline: none;
    }
    .stat-unsure-mark:focus-visible::after {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: 0;
    }
    /* The note is a live region that is always in the page. Closed, it is
       visually collapsed (out of flow, 1px, clipped), never display: none, so
       screen readers still announce its text when it fills. */
    .stat-unsure-note {
        position: absolute;
        width: 1px;
        height: 1px;
        margin: -1px;
        padding: 0;
        border: 0;
        overflow: hidden;
        clip-path: inset(50%);
        white-space: nowrap;
    }
    /* Open: a small note that wraps under the value and stays out of the
       clipped value text. It starts outside the mark's hit box: when it shares
       the line with the mark (the hero cell) a left margin of half the box
       clears the box's right edge, and when it wraps under the value a top
       margin of a quarter of the box clears the box's bottom edge. The stat
       cells always wrap it under the value, so they set the left gap to zero
       and keep their width. A click on the note closes it, like the mark. */
    .stat-unsure-note.stat-unsure-note-open {
        position: relative;
        z-index: 1;
        display: block;
        flex: 0 0 100%;
        box-sizing: border-box;
        width: max-content;
        height: auto;
        max-width: min(calc(var(--space-12) * 5), 100%);
        min-width: var(--unsure-note-min, 0);
        margin: calc(var(--unsure-hit) / 4) 0 0 var(--unsure-note-gap, calc(var(--unsure-hit) / 2));
        cursor: pointer;
        overflow: visible;
        clip-path: none;
        font-family: var(--font-body, inherit);
        font-size: var(--text-xs);
        font-weight: 400;
        line-height: var(--leading-ui);
        letter-spacing: normal;
        text-transform: none;
        text-align: left;
        text-decoration: none;
        white-space: normal;
        color: var(--text-3);
    }
    .match-card .match-outcome:has(.stat-unsure-note-open),
    .match-card .match-hero:has(.stat-unsure-note-open) {
        overflow: visible;
        flex-wrap: wrap;
    }
    .stat-unsure:has(.stat-unsure-note-open) { flex-wrap: wrap; }
    /* The outcome and hero cells clip their overflow, which would cut the
       mark's 24px hit box off at the cell edge, and a tap on the cut part would
       open the game. A cell that holds a mark does not clip: the ellipsis lives
       on the value text, which shrinks on its own, so the cell stays inside its
       column either way. */
    .match-card .match-outcome:has(.stat-unsure-mark),
    .match-card .match-hero:has(.stat-unsure-mark) {
        overflow: visible;
    }
    /* The stat columns are 2 to 4rem wide, too narrow for a sentence. A cell
       with an open note sets a minimum note width, which beats the 100% cap, so
       the note is wider than its cell and spills sideways into the empty space
       under its neighbours. E, D and A grow it to the right, Dmg and Heal to
       the left, so it stays inside the stat block. The other cells line up at
       the top, so no value moves under the note. */
    .match-stat:has(.stat-unsure-note-open) {
        --unsure-note-min: calc(var(--space-10) * 4);
        --unsure-note-gap: 0;
    }
    .match-stat:not(.match-stat-wide):has(.stat-unsure-note-open) { justify-content: flex-start; }
    .match-stat:not(.match-stat-wide):has(.stat-unsure-note-open) .stat-value-text { margin-left: auto; }
    .match-stats:has(.stat-unsure-note-open) { align-items: start; }
    .stat-flagged {
        flex: 0 0 auto;
        max-width: 100%;
        white-space: nowrap;
    }
    .match-card .match-map {
        color: var(--text-3);
        font-size: var(--text-sm);
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        column-gap: var(--space-1);
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
        color: var(--text-3);
        font-size: var(--text-sm);
        font-variant-numeric: tabular-nums;
    }
    .match-card .match-date {
        color: var(--text-3);
        font-size: var(--text-xs);
        white-space: nowrap;
    }

    /* Summary: three tiles in the Home stat treatment (display-size value,
       tabular figures, label under the value), 3-up at every width. */
    .my-stats .stats-summary {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        min-width: 0;
    }
    .my-stats .stats-tiles {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: var(--space-3);
    }
    .my-stats .stat-tile {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-1);
        min-width: 0;
        padding: var(--card-padding);
        background: var(--card-bg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
        text-align: center;
    }
    .my-stats .stat-tile-value {
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        font-size: var(--text-2xl);
        letter-spacing: var(--font-head-tracking);
        line-height: var(--leading-ui);
        font-variant-numeric: tabular-nums;
        color: var(--text);
        overflow-wrap: anywhere;
        text-box-trim: trim-both;
        text-box-edge: cap alphabetic;
    }
    .my-stats .stat-tile-label {
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-ui);
        text-box-trim: trim-both;
        text-box-edge: cap alphabetic;
    }
    .my-stats .stats-summary-note {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }

    /* Overview: the frame blocks first (rank progress, time on role, recent
       form), then role results, top heroes and mode win rates. */
    .stats-overview {
        display: flex;
        flex-direction: column;
        gap: var(--space-section);
        min-width: 0;
    }
    /* Time on role: label, bar and percent on one line. */
    .role-split { display: grid; gap: var(--space-3); }
    .role-row {
        display: grid;
        grid-template-columns: calc(var(--space-12) + var(--space-4)) minmax(0, 1fr) auto;
        gap: var(--space-3);
        align-items: center;
        color: var(--text);
        font-size: var(--text-md);
        line-height: var(--leading-ui);
    }
    .role-name { min-width: 0; overflow-wrap: anywhere; }
    .role-track {
        height: var(--space-3);
        background: var(--surface-sunken);
        border-radius: var(--radius-pill);
        overflow: hidden;
    }
    .role-fill { height: 100%; background: var(--accent-solid); }
    .role-pct {
        min-width: var(--space-10);
        text-align: end;
        font-variant-numeric: tabular-nums;
    }
    .role-split-note {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-xs);
        line-height: var(--leading-body);
    }
    /* Recent form: one card per game, the result badge hugs its word. */
    .form-summary {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    .form-list { display: grid; gap: var(--space-3); }
    .form-row {
        position: relative;
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: var(--space-3) var(--space-4);
        align-items: start;
    }
    .form-row-main { min-width: 0; }
    .form-row-title {
        margin: 0 0 var(--space-2);
        font-family: var(--font-head);
        font-size: var(--text-lg);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-head-tracking);
        line-height: var(--leading-ui);
        color: var(--text);
        overflow-wrap: anywhere;
    }
    /* The title link covers the whole card, so the target is the card. */
    .form-row-title a { color: var(--accent-text); text-decoration: none; }
    .form-row-title a::after {
        content: "";
        position: absolute;
        inset: 0;
        border-radius: inherit;
    }
    .form-row-title a:hover { text-decoration: underline; }
    .form-row:has(.form-row-title a:hover) { border-color: var(--border-strong); }
    .form-row-title a:focus-visible { outline: none; }
    .form-row-title a:focus-visible::after {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .form-row-meta {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
        overflow-wrap: anywhere;
    }
    .form-row > .ui-pill {
        justify-self: start;
        align-self: start;
        width: max-content;
    }
    .form-more { display: flex; }
    /* Role results, top heroes, mode win rates. */
    .overview-grid {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: var(--space-section) var(--space-4);
        align-items: start;
    }
    .role-cards { display: flex; flex-direction: column; gap: var(--space-3); }
    /* A long result ("Too few games, 3W-3L") drops under the stat line
       instead of squeezing it to three lines; a short rate stays beside it. */
    .role-card {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2) var(--space-3);
        border-inline-start: var(--space-1) solid transparent;
    }
    .role-card-info { flex: 1 1 calc(var(--space-12) * 5); min-width: 0; }
    .role-card-name {
        font-weight: var(--font-label-weight);
        font-size: var(--text-md);
        color: var(--text);
    }
    .role-card-sub {
        margin-top: var(--space-1);
        color: var(--text-3);
        font-size: var(--text-xs);
        line-height: var(--leading-body);
    }
    .role-card-wr {
        font-family: var(--font-head);
        font-size: var(--text-lg);
        font-weight: var(--font-head-weight);
        color: var(--text);
        font-variant-numeric: tabular-nums;
        text-align: end;
    }
    .role-card-wr.muted {
        color: var(--text-3);
        font-family: var(--font-body);
        font-size: var(--text-sm);
        font-weight: var(--font-body-weight);
    }
    .mini-hero-list { display: flex; flex-direction: column; }
    .mini-hero-row {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto auto;
        gap: var(--space-3);
        align-items: center;
        min-height: var(--density-row);
        border-bottom: 1px solid var(--border);
        font-size: var(--text-md);
    }
    .mini-hero-name {
        min-width: 0;
        color: var(--text);
        font-weight: var(--font-label-weight);
        overflow-wrap: anywhere;
    }
    .mini-hero-meta { color: var(--text-3); font-size: var(--text-sm); }
    .mini-hero-wr {
        color: var(--text);
        font-weight: var(--font-label-weight);
        font-variant-numeric: tabular-nums;
    }
    .mini-hero-list .role-split-note { margin-top: var(--space-3); }
    .mode-tiles {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: var(--space-3);
    }
    .mode-tile { display: flex; flex-direction: column; gap: var(--space-1); }
    .mode-tile-name {
        color: var(--text-3);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
    }
    .mode-tile-wr {
        font-family: var(--font-head);
        font-size: var(--text-lg);
        font-weight: var(--font-head-weight);
        color: var(--text);
        font-variant-numeric: tabular-nums;
    }
    .mode-tile-n { color: var(--text-3); font-size: var(--text-xs); }

    /* The Heroes and Maps tabs keep their rules next to their markup
       (heroes.rs, maps.rs, and the shared pieces in stats_member.rs). */

    /* Phone: the tiles stay 3-up with the smaller value step, a role name
       wraps above its bar, and a form card puts its badge under the text. */
    @media (max-width: 820px) {
        .my-stats .stats-tabs { flex: 1 1 100%; }
        .my-stats .stats-tiles { gap: var(--space-2); }
        .my-stats .stat-tile {
            min-height: var(--hit-target-min);
            padding: var(--space-2);
        }
        .my-stats .stat-tile-value { font-size: var(--text-lg); }
        .my-stats .stat-tile-label {
            font-size: var(--text-xs);
            overflow-wrap: anywhere;
        }
        .role-row { grid-template-columns: minmax(0, 1fr) auto; }
        .role-row .role-name { grid-column: 1 / -1; }
        .form-row { grid-template-columns: minmax(0, 1fr); }
        .overview-grid { grid-template-columns: minmax(0, 1fr); }
        /* Issue 725 (WCAG 2.4.11): the tabpanel is a Tab stop, and a panel
           taller than the band between the top bar and the phone tab bar
           cannot be scrolled clear of them (focus_reveal.rs leaves a box
           that can never fit where it is), so it sat partly under the tab
           bar. Capped at that band (the viewport less the shell chrome and
           the page's two scroll-padding gaps, on the small viewport so it
           fits with the browser toolbar shown), it always fits and scrolls
           inside itself, which its tabindex already lets keyboard users do.
           The padding keeps the focus rings of the controls at its edges
           inside the scroller; the negative inline margin keeps the content
           on the page's left edge. */
        .my-stats .stats-panel {
            box-sizing: border-box;
            --stats-panel-ring-room: calc(var(--focus-ring-width) + var(--focus-ring-offset));
            max-height: calc(100vh - var(--shell-chrome-height) - 2 * var(--space-2));
            max-height: calc(100svh - var(--shell-chrome-height) - 2 * var(--space-2));
            overflow-y: auto;
            padding: var(--stats-panel-ring-room);
            margin-inline: calc(var(--stats-panel-ring-room) * -1);
        }
    }

    /* Stat tracker block, under the stats. Members with matches get one slim
       row; the install note is a quiet line, not a warning banner. */
    .tracker-slim {
        display: flex;
        align-items: center;
        justify-content: space-between;
        flex-wrap: wrap;
        gap: var(--space-3) var(--space-4);
    }
    .tracker-slim-text {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: var(--space-2) var(--space-3);
        min-width: 0;
    }
    .tracker-slim-actions {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
    }
    .tracker-release-notice {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    .tracker-setup {
        display: flex;
        flex-direction: column;
        gap: var(--space-stack);
        min-width: 0;
    }
    .tracker-setup:empty { display: none; }

    /* Daemon settings panel */
    .daemon-settings summary {
        box-sizing: border-box;
        min-height: var(--hit-target-min);
        padding: var(--space-3) 0;
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-md);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
        cursor: pointer;
    }
    .daemon-settings summary:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .daemon-settings-row {
        display: flex;
        gap: var(--space-3);
        align-items: center;
        flex-wrap: wrap;
        margin: var(--space-2) 0;
    }
    .daemon-settings-row .field-label { white-space: nowrap; }
    .daemon-settings-row .daemon-settings-input {
        flex: 1 1 calc(var(--space-12) * 3);
        width: auto;
        min-width: 0;
        font-family: var(--font-mono);
    }

    /* Tracker download / install panel */
    .tracker-download {
        box-sizing: border-box;
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        min-width: 0;
        padding: var(--card-padding);
        background: var(--card-bg);
        color: var(--card-fg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
    }
    .tracker-download-head {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        flex-wrap: wrap;
    }
    .tracker-download-head h3 {
        margin: 0;
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-lg);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-head-tracking);
        line-height: var(--leading-ui);
    }
    .tracker-download p {
        margin: 0;
        max-width: var(--content-max-narrow);
        color: var(--text-3);
        font-size: var(--text-md);
        line-height: var(--leading-body);
    }
    .tracker-download p a {
        color: var(--accent-text);
        text-decoration: underline;
    }
    .tracker-download code {
        color: var(--text);
        font-family: var(--font-mono);
        overflow-wrap: anywhere;
    }
    .tracker-download code.tracker-install {
        display: block;
        padding: var(--space-3);
        background: var(--surface-sunken);
        border: 1px solid var(--border);
        border-radius: var(--radius-control);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
        user-select: all;
    }
    .tracker-download-links {
        display: flex;
        gap: var(--space-2);
        flex-wrap: wrap;
    }

    /* W3: filters, density, overview extras */
    /* Tab filters are the shared chips now (ChipRow, `.chip`). A small sample
       is quieter through its ink, never through opacity, so the text keeps
       its contrast. */
    .my-stats .stats-row-muted { color: var(--text-subtle); }
    [data-density="comfortable"] .stats-page .match-card {
        padding-top: var(--space-3);
        padding-bottom: var(--space-3);
    }
    [data-density="compact"] .stats-page .match-card {
        padding-top: var(--space-1);
        padding-bottom: var(--space-1);
    }
    [data-density="compact"] .stats-page .match-cards { gap: var(--space-1); }

    /* W4 polish: tables, mobile, hold layout */
    .my-stats.stats-page .match-stat {
        font-variant-numeric: tabular-nums;
    }
    @media (max-width: 900px) {
        /* Wide header needs the single-row tracks. Phone widths bring the
           stat labels back in the 720px block below. */
        .match-list-head { display: none; }
    }
    @media (max-width: 720px) {
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
            grid-template-columns: calc(var(--space-8) * 2) minmax(0, 1fr) auto;
            grid-template-areas:
                "outcome identity date"
                "stats stats stats";
            gap: var(--space-1) var(--space-2);
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
    }
"#;

/// Release note inside the tracker install and reinstall blocks.
fn tracker_release_notice(text: &str) -> Element {
    let text = text.to_string();
    rsx! {
        p { class: "tracker-release-notice", "{text}" }
    }
}

/// `/stats` for a guest: the page chrome, what members get, Log in and a way
/// on to the public leaderboards (the `d1-stats-wall` frame).
fn signed_out_wall() -> Element {
    rsx! {
        PageShell { width: PageWidth::Wide, class: "stats-shell",
            PageHeader {
                title: "Stats",
                lead: "Personal match history, role split, and form. Log in to see yours.".to_string(),
            }
            Wall {
                title: "Log in to see your stats",
                lead: "Members get win rate, time on each role, and recent form from logged matches. Guests can still browse leaderboards and public tournament results.",
                actions: rsx! {
                    Link { to: Route::Login {}, class: btn_class(true), "Log in" }
                    Link { to: Route::Leaderboards {}, class: btn_class(false), "See the leaderboards" }
                },
            }
        }
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

    let visibility = server_settings
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|s| s.stats_visibility);

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
        // The button stays focusable while a save runs, so a second press
        // must not start a second save.
        if settings_saving() {
            return;
        }
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
                Err(e) => toast.show(Toast::error_from("Save failed", &e)),
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

    // K/D comes from the role rows. Until they load the tile is a dash,
    // never "N/A" (no deaths); the Time on role block carries the loading
    // and failed states for that fetch.
    let kd = match role_stats.data.read().as_ref().and_then(|d| d.as_ref()) {
        Some(rows) => KdTile::Ready(role::kd_ratio(rows)),
        None if role_stats.error.read().is_some() => KdTile::Failed,
        None => KdTile::Loading,
    };
    // "See all matches" under Recent form: the same as pressing the History tab.
    let open_history = use_callback(move |_: ()| {
        tab.set(StatsTab::History);
        page_cursor.set(None);
        cursor_history.set(vec![None]);
        // "See all matches" leaves with the Overview: focus the History tab
        // it opened, not the page body.
        crate::components::ui::inline_confirm::focus_element_by_id_soon(
            stats_tab_id(StatsTab::History).to_string(),
        );
    });
    // Zero matches in this season window: the empty card is the overview.
    let no_matches = has_matches == Some(false);
    // A season pinned by id that is not the current one: its newest game is
    // not the member's current rank, so the rank chart says "latest".
    let past_season = {
        let list = season.season_list();
        let list = list.read();
        picked_season_is_not_current(
            season.selected_id().as_deref(),
            list.as_ref().and_then(|l| l.as_deref()),
        )
    };
    let chart_for = rank::ChartFor {
        player: auth().user.as_ref().map(|user| user.username.clone()),
        past_season,
    };
    let tab_body = match tab() {
        StatsTab::Overview if no_matches => rsx! {},
        StatsTab::Overview => overview::overview_tab(
            heroes,
            role_stats,
            maps,
            form_matches,
            open_history,
            chart_for.clone(),
        ),
        StatsTab::Heroes => heroes::heroes_tab(heroes, hero_role, hero_sort),
        StatsTab::Maps => maps::maps_tab(maps),
        StatsTab::History => history::history_tab(
            matches,
            page_cursor,
            cursor_history,
            hist_outcome,
            hist_role,
            hist_mode,
            chart_for.clone(),
        ),
    };

    if !auth().loading && !auth().is_logged_in() {
        return rsx! {
            crate::styles::StyleOnce { id: "scuffed-css-pages-stats-mod-stats-css", css: STATS_CSS }
            div { class: "stats-page my-stats", {signed_out_wall()} }
        };
    }

    let lead = stats_lead(
        auth().user.as_ref().map(|user| user.username.as_str()),
        season.selected_id().as_deref(),
    );

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-stats-mod-stats-css", css: STATS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-admin-shared", css: crate::styles::admin::CSS }

        div { class: "stats-page my-stats",
            PageShell { width: PageWidth::Wide, class: "stats-shell",
                PageHeader {
                    title: "Your stats",
                    lead,
                    meta: visibility.map(|visibility| rsx! {
                        p { class: "stats-visibility-note",
                            "{stats_privacy::notice(visibility)} "
                            Link { to: Route::ProfileSettings {}, "Change in settings" }
                        }
                    }),
                }

                div { class: "stats-header",
                    div {
                        class: "chip-row is-scroll stats-tabs",
                        role: "tablist",
                        aria_label: "Your stats sections",
                        TabButton {
                            class: "chip stats-tab".to_string(),
                            label: "Overview".to_string(),
                            id: stats_tab_id(StatsTab::Overview).to_string(),
                            controls: MY_STATS_PANEL.to_string(),
                            selected: tab() == StatsTab::Overview,
                            onclick: move |_| tab.set(StatsTab::Overview),
                        }
                        TabButton {
                            class: "chip stats-tab".to_string(),
                            label: "Heroes".to_string(),
                            id: stats_tab_id(StatsTab::Heroes).to_string(),
                            controls: MY_STATS_PANEL.to_string(),
                            selected: tab() == StatsTab::Heroes,
                            onclick: move |_| tab.set(StatsTab::Heroes),
                        }
                        TabButton {
                            class: "chip stats-tab".to_string(),
                            label: "Maps".to_string(),
                            id: stats_tab_id(StatsTab::Maps).to_string(),
                            controls: MY_STATS_PANEL.to_string(),
                            selected: tab() == StatsTab::Maps,
                            onclick: move |_| tab.set(StatsTab::Maps),
                        }
                        TabButton {
                            class: "chip stats-tab".to_string(),
                            label: "History".to_string(),
                            id: stats_tab_id(StatsTab::History).to_string(),
                            controls: MY_STATS_PANEL.to_string(),
                            selected: tab() == StatsTab::History,
                            onclick: move |_| {
                                tab.set(StatsTab::History);
                                page_cursor.set(None);
                                cursor_history.set(vec![None]);
                            },
                        }
                    }
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
                        Link { to: Route::Leaderboards {}, class: btn_class(false), "Leaderboards" }
                    }
                }

                // The one tabpanel: the summary tiles, then the selected tab.
                div {
                    id: MY_STATS_PANEL,
                    class: "stats-panel",
                    role: "tabpanel",
                    aria_labelledby: stats_tab_id(tab()),
                    tabindex: "0",
                    {
                        let data = stats.data.read();
                        let s = data.as_ref().and_then(|d| d.as_ref());
                        match s {
                            None => member_pending(&auth(), &stats, "stats"),
                            Some(s) if s.total_matches == 0 => rsx! {
                                EmptyCard {
                                    title: "No logged matches yet",
                                    message: no_logged_games_message(season.selected_id().as_deref()),
                                    actions: rsx! {
                                        a { class: btn_class(true), href: TRACKER_ANCHOR, "Set up the tracker" }
                                        Link { to: Route::StatsTokens {}, class: btn_class(false), "Tracker tokens" }
                                    },
                                }
                            },
                            Some(s) => summary_tiles(
                                Tally::new(s.total_matches, s.wins, s.losses, s.draws),
                                kd,
                            ),
                        }
                    }
                    div { class: "stats-tab-body", {tab_body} }
                }

                // Tracker setup. Members with matches get one slim row; a
                // member with none gets the full install steps.
                section {
                    class: "ui-stack stats-tracker",
                    id: "stats-tracker",
                    aria_labelledby: "stats-tracker-title",
                    h2 { class: "ui-section-title", id: "stats-tracker-title", "Stat tracker" }
                    if slim_tracker {
                        div { class: "stats-card tracker-slim",
                            div { class: "tracker-slim-text",
                                Pill { tone: PillTone::Info, "Linux only" }
                                {tracker_release_notice(&tracker_notice)}
                            }
                            div { class: "tracker-slim-actions",
                                button {
                                    r#type: "button",
                                    class: btn_class(false),
                                    aria_expanded: if setup_open() { "true" } else { "false" },
                                    aria_controls: "stats-tracker-setup",
                                    onclick: move |_| setup_open.toggle(),
                                    if setup_open() { "Hide instructions" } else { "Reinstall instructions" }
                                }
                                Link { to: Route::StatsTokens {}, class: btn_class(false), "Tracker tokens" }
                            }
                        }
                    }

                    // Always in the page so the button above can name it; it
                    // is empty while the instructions are closed.
                    div { class: "tracker-setup", id: "stats-tracker-setup",
                        if show_full_setup {
                            // Daemon settings (collapsible)
                            details { class: "stats-card daemon-settings",
                                summary { "Daemon settings" }
                                div { class: "daemon-settings-row",
                                    label { class: "field-label", r#for: "player-name-input", "In-game name" }
                                    input {
                                        id: "player-name-input",
                                        class: "field-input daemon-settings-input",
                                        r#type: "text",
                                        placeholder: "e.g. FROZEN",
                                        aria_describedby: "player-name-hint",
                                        value: player_name_input(),
                                        oninput: move |e| player_name_input.set(e.value()),
                                    }
                                    button {
                                        r#type: "button",
                                        class: btn_class(true),
                                        aria_disabled: if settings_saving() { "true" },
                                        onclick: save_settings,
                                        if settings_saving() { "Saving..." } else { "Save" }
                                    }
                                }
                                p { class: "field-hint", id: "player-name-hint",
                                    "The daemon uses this name to find your row on replay and post-match scoreboards."
                                }
                            }

                            // Tracker download / install. The slim row already shows the
                            // notice, so this copy is only for members who have no matches.
                            div { class: "tracker-download",
                                div { class: "tracker-download-head",
                                    h3 { "Get the Stat Tracker" }
                                    Pill { tone: PillTone::Info, "Linux only" }
                                }
                                // The heading first, then the note about it.
                                if !slim_tracker {
                                    {tracker_release_notice(&tracker_notice)}
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
                                    Link { to: Route::StatsTokens {}, "tracker token" }
                                    " to begin uploading."
                                }
                                div { class: "tracker-download-links",
                                    a {
                                        class: btn_class(false),
                                        href: "https://github.com/FrozenTear/scuffed-crew/releases/latest",
                                        target: "_blank",
                                        rel: "noopener noreferrer",
                                        "Latest release (opens in a new tab)"
                                    }
                                }
                            }
                        }
                    }
                }
            }
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
    fn list_rows_read_bans_and_picks_when_present_and_ignore_them_when_not() {
        let plain: PersonalMatch = serde_json::from_str(&row_json("")).expect("row");
        assert!(plain.bans_picks.is_none());
        for extra in [
            r#","bans_picks":null"#,
            r#","bans_picks":"garbled""#,
            r#","bans_picks":7"#,
        ] {
            let row: PersonalMatch = serde_json::from_str(&row_json(extra)).expect(extra);
            assert!(row.bans_picks.is_none(), "{extra}");
        }
        let raw = row_json(
            r#","bans_picks":{"bans":[{"hero":"ana","team":"own"}],"lobby_ban":"mei","picks":["dva",null,"no_preference"],"ban_vote":[null],"recognizer":"cv-v1"}"#,
        );
        let page: MatchPage =
            serde_json::from_str(&format!(r#"{{"data":[{raw}],"next_cursor":null}}"#))
                .expect("page");
        assert!(page.data[0].bans_picks.is_some());
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
    fn open_note_starts_outside_the_mark_hit_box_and_takes_clicks() {
        // The hit box is centered on the glyph and --unsure-hit wide, so it
        // reaches half that far past the glyph's center. The open note keeps
        // half a box of left margin (same line, hero cell) and a quarter box of
        // top margin (wrapped under the value) so it never covers the box.
        let note = block_after(STATS_CSS, ".stat-unsure-note.stat-unsure-note-open {");
        let note_head = note.split('}').next().unwrap();
        assert!(
            has_declaration(
                note_head,
                "margin: calc(var(--unsure-hit) / 4) 0 0 var(--unsure-note-gap, calc(var(--unsure-hit) / 2))"
            ),
            "the note starts outside the hit box: {note_head}"
        );
        assert!(
            has_declaration(note_head, "cursor: pointer"),
            "a click on the open note closes it: {note_head}"
        );
        assert!(
            !note_head.contains("pointer-events"),
            "the open note takes the click: {note_head}"
        );
        // The stat cells always wrap the note under the value, where only the
        // top margin matters, so they drop the left gap and keep their width.
        let cell = block_after(STATS_CSS, ".match-stat:has(.stat-unsure-note-open) {");
        let cell_head = cell.split('}').next().unwrap();
        assert!(
            has_declaration(cell_head, "--unsure-note-gap: 0"),
            "{cell_head}"
        );
        // The gap is a property of the open rule only: no other open-note rule
        // sets a margin that could undo it.
        for rule in STATS_CSS
            .split('}')
            .filter(|rule| rule.contains("stat-unsure-note-open") && !rule.contains(":has("))
        {
            let head = rule.split('{').nth(1).unwrap_or("");
            if head.contains("margin") {
                assert!(head.contains("--unsure-hit"), "{rule}");
            }
        }
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
        // The reader badge (the shared pill) may shrink before the hero does,
        // at every width, so also on phone.
        let badge = block_after(
            super::history_filter::HISTORY_CSS,
            ".match-hero-line .reader-badge",
        );
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

        assert!(
            has_declaration(mark_head, "position: relative"),
            "{mark_head}"
        );
        assert!(has_declaration(mark_head, "z-index: 1"), "{mark_head}");

        // The open note must stay above the row overlay link in the final
        // cascade: every rule for the open class keeps it positioned.
        let open_rules: Vec<(&str, &str)> = STATS_CSS
            .split('}')
            .filter_map(|chunk| chunk.rsplit_once('{'))
            .map(|(before, body)| (before.rsplit([';', '{']).next().unwrap_or(before), body))
            .filter(|(selector, _)| {
                selector.contains("stat-unsure-note-open") && !selector.contains(":has(")
            })
            .collect();
        assert!(!open_rules.is_empty(), "missing the open note rule");
        for (selector, body) in &open_rules {
            assert!(
                !body.contains("position: static"),
                "{selector} would drop the note under the row link: {body}"
            );
            assert!(
                !body.contains("z-index: 0") && !body.contains("z-index: auto"),
                "{selector}: {body}"
            );
        }
        // A stat cell with an open note gives it a minimum width that beats the
        // 100% cap, so the note is not squeezed into a 2 to 4rem column.
        let note_rule = open_rules
            .iter()
            .find(|(selector, _)| {
                selector
                    .trim()
                    .ends_with(".stat-unsure-note.stat-unsure-note-open")
            })
            .expect("the open note rule")
            .1;
        assert!(
            has_declaration(note_rule, "min-width: var(--unsure-note-min, 0)"),
            "{note_rule}"
        );
        let cell_min = block_after(STATS_CSS, ".match-stat:has(.stat-unsure-note-open) {");
        let cell_min_head = cell_min.split('}').next().unwrap();
        assert!(
            cell_min_head.contains("--unsure-note-min: calc(var(--space-10) * 4)"),
            "{cell_min_head}"
        );
        let top = block_after(STATS_CSS, ".match-stats:has(.stat-unsure-note-open) {");
        let top_head = top.split('}').next().unwrap();
        assert!(
            top_head.contains("align-items: start"),
            "the other cells line up at the top: {top_head}"
        );
        // The mark's tap target is a 24 by 24px ::after box that carries the
        // focus ring, so the tiny glyph is not the only thing a tap can hit.
        let hit = block_after(STATS_CSS, ".stat-unsure-mark::after {");
        let hit_head = hit.split('}').next().unwrap();
        assert!(has_declaration(hit_head, "content: \"\""), "{hit_head}");
        assert!(
            has_declaration(hit_head, "position: absolute"),
            "{hit_head}"
        );
        assert!(
            has_declaration(hit_head, "width: var(--unsure-hit)"),
            "{hit_head}"
        );
        assert!(
            has_declaration(hit_head, "height: var(--unsure-hit)"),
            "{hit_head}"
        );
        assert!(
            STATS_CSS.contains(".stat-unsure { --unsure-hit: var(--space-6); }"),
            "the one place that sets the 24px box"
        );
        assert!(
            !hit_head.contains("pointer-events"),
            "the hit box must take the tap: {hit_head}"
        );
        assert!(
            !hit_head.contains("z-index"),
            "the box inherits the button's z-index, above the row link: {hit_head}"
        );
        // The outcome and hero cells clip overflow; a cell with a mark must not,
        // or the hit box is cut at the cell edge.
        let unclipped = block_after(
            STATS_CSS,
            ".match-card .match-hero:has(.stat-unsure-mark) {",
        );
        let unclipped_head = unclipped.split('}').next().unwrap();
        assert!(
            has_declaration(unclipped_head, "overflow: visible"),
            "a cell with a mark does not clip its hit box: {unclipped_head}"
        );
        assert!(
            STATS_CSS.contains(".match-card .match-outcome:has(.stat-unsure-mark),"),
            "the outcome cell is unclipped too"
        );
        let focus = block_after(STATS_CSS, ".stat-unsure-mark:focus-visible {");
        let focus_head = focus.split('}').next().unwrap();
        assert!(
            has_declaration(focus_head, "outline: none"),
            "the collapsed button draws no ring of its own: {focus_head}"
        );
        let ring = block_after(STATS_CSS, ".stat-unsure-mark:focus-visible::after {");
        let ring_head = ring.split('}').next().unwrap();
        assert!(
            ring_head.contains("outline: var(--focus-ring-width) solid var(--focus-ring)"),
            "{ring_head}"
        );
        let last = open_rules.last().unwrap().1;
        assert!(has_declaration(last, "position: relative"), "{last}");
        assert!(has_declaration(last, "z-index: 1"), "{last}");
        // The row link overlay has no z-index, so a z-index of 1 wins over it.
        let overlay = block_after(STATS_CSS, ".match-card-overlay {");
        let overlay_head = overlay.split('}').next().unwrap();
        assert!(!overlay_head.contains("z-index"), "{overlay_head}");

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
            has_declaration(page_block, "--history-pad-x: var(--space-card-pad)"),
            "{page_block}"
        );
        assert!(
            has_declaration(page_block, "--history-border: 1px"),
            "{page_block}"
        );
        // Density reaches the inset through the card pad token, so no density
        // rule sets a size of its own.
        assert_eq!(STATS_CSS.matches("--history-pad-x:").count(), 1);

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
    use super::{
        TRACKER_PLATFORM_COPY, manifest_package_version, tracker_release_notice_text,
        tracker_release_version,
    };

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

    /// The short label for a full tracker version, the way the install note
    /// shows it: `0.6.0-alpha.3` reads `alpha.3`, a plain `1.0.0` stays.
    fn short_label(version: &str) -> &str {
        version.split_once('-').map_or(version, |(_, label)| label)
    }

    #[test]
    fn tracker_release_comes_from_the_tracker_manifest() {
        let version = tracker_release_version().expect("stat-tracker-core [package] version");
        let core = include_str!("../../../../stat-tracker-core/Cargo.toml");
        assert!(
            core.lines()
                .any(|line| line.trim() == format!("version = \"{version}\"")),
            "{version} is not the version line of crates/stat-tracker-core/Cargo.toml"
        );
        // The daemon manifest is the release version the tag check reads first.
        let daemon = include_str!("../../../../stat-tracker/Cargo.toml");
        assert_eq!(
            manifest_package_version(daemon),
            Some(version),
            "the install note must name the tracker release version"
        );
        let (core_digits, _) = version.split_once('-').unwrap_or((version, ""));
        assert!(
            super::is_release_core(core_digits),
            "tracker version is not MAJOR.MINOR.PATCH[-label]: {version}"
        );
    }

    #[test]
    fn manifest_package_version_reads_only_the_package_table() {
        let manifest = "# version = \"9.9.9\"\n[workspace]\nversion = \"8.8.8\"\n\
            [package]\nname = \"x\"\nversion.workspace = true\n\
            version   =   \"1.2.3-beta.4\" # comment\n[dependencies]\nversion = \"7.7.7\"\n";
        assert_eq!(manifest_package_version(manifest), Some("1.2.3-beta.4"));
        assert_eq!(
            manifest_package_version("[package.metadata]\nversion = \"1.0.0\"\n"),
            None
        );
        assert_eq!(
            manifest_package_version("[package]\nname = \"x\"\n[lib]\nversion = \"1.0.0\"\n"),
            None
        );
        assert_eq!(
            manifest_package_version("[package]\nversion = \"\"\n"),
            None
        );
        assert_eq!(manifest_package_version(""), None);
    }

    #[test]
    fn release_notice_names_the_current_tracker_build() {
        let version = tracker_release_version().expect("stat-tracker-core [package] version");
        let label = short_label(version);
        assert!(!label.is_empty());
        let pinned = tracker_release_notice_text(None);
        assert_eq!(
            pinned,
            format!("Install {label} between games, not during one.")
        );
        assert!(!pinned.contains('\u{2014}'));
        assert!(!pinned.contains('\u{2013}'));
        assert!(pinned.contains(label));
        if version.contains('-') {
            assert!(
                !pinned.contains(version),
                "the note shows the short label, not {version}"
            );
        }
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
        // No hardcoded build: the stale pin is gone and the current release
        // is not written into the page source either.
        let stale = ["rc", "9"].join(".");
        assert!(
            !prod.contains(&stale),
            "the install note must not keep the stale build {stale}"
        );
        assert!(
            !prod.contains(&format!("\"{label}\"")) && !prod.contains(version),
            "the build name must come from the tracker manifest, not a literal"
        );
        assert_eq!(
            prod.matches("include_str!(\"../../../../stat-tracker-core/Cargo.toml\")")
                .count(),
            1,
            "the build name is read from crates/stat-tracker-core/Cargo.toml once"
        );
        assert!(
            !prod.contains("Install alpha."),
            "the sentence must be built from the tracker version"
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
        let version = tracker_release_version().expect("stat-tracker-core [package] version");
        let pinned = tracker_release_notice_text(None);
        assert_eq!(
            tracker_release_notice_text(Some(version)),
            pinned,
            "the current bare version reads the same as the fallback"
        );
        assert_eq!(
            tracker_release_notice_text(Some(&format!("  {version}  "))),
            pinned
        );
        assert_eq!(
            tracker_release_notice_text(Some("  0.5.0-alpha.11  ")),
            "Install alpha.11 between games, not during one."
        );
        // The server's tracker_latest wins over the compiled-in version.
        assert_eq!(
            tracker_release_notice_text(Some("9.4.1-beta.2")),
            "Install beta.2 between games, not during one."
        );
        assert_eq!(
            tracker_release_notice_text(Some("beta.2")),
            "Install beta.2 between games, not during one."
        );
        assert_eq!(tracker_release_notice_text(Some("")), pinned);
        assert_eq!(tracker_release_notice_text(Some("   ")), pinned);
        assert_eq!(tracker_release_notice_text(Some("\n")), pinned);
        let shown = tracker_release_notice_text(Some(version));
        if let Some((core, _)) = version.split_once('-') {
            assert!(!shown.contains(&format!("{core}-")));
        }
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
        // The page title is the first piece of the signed-in chrome.
        let chrome = prod
            .find("title: \"Your stats\"")
            .expect("signed-in chrome");
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
mod keyed_loop_tests {
    /// Loop bodies that are a block or a helper call give Dioxus no key to
    /// diff by. Every S4c page and the brackets key each item with a
    /// `Fragment` or the element.
    #[test]
    fn list_loops_key_their_items() {
        for (name, src) in [
            ("stats/mod.rs", include_str!("mod.rs")),
            ("stats/overview.rs", include_str!("overview.rs")),
            ("stats/rank.rs", include_str!("rank.rs")),
            ("stats/heroes.rs", include_str!("heroes.rs")),
            ("stats/maps.rs", include_str!("maps.rs")),
            ("stats/history.rs", include_str!("history.rs")),
            ("stats/history_filter.rs", include_str!("history_filter.rs")),
            ("stats/bans_picks.rs", include_str!("bans_picks.rs")),
            ("stats/game.rs", include_str!("game.rs")),
            ("stats/role.rs", include_str!("role.rs")),
            ("stats_member.rs", include_str!("../stats_member.rs")),
            ("stats_tokens.rs", include_str!("../stats_tokens.rs")),
            ("leaderboards.rs", include_str!("../leaderboards.rs")),
            ("match_detail.rs", include_str!("../match_detail.rs")),
            ("team_page.rs", include_str!("../team_page.rs")),
            ("tournament.rs", include_str!("../tournament.rs")),
            ("tournaments.rs", include_str!("../tournaments.rs")),
            (
                "bracket/bracket_view.rs",
                include_str!("../../components/bracket/bracket_view.rs"),
            ),
            (
                "bracket/double_elim.rs",
                include_str!("../../components/bracket/double_elim.rs"),
            ),
            (
                "bracket/single_elim.rs",
                include_str!("../../components/bracket/single_elim.rs"),
            ),
            (
                "bracket/round_robin.rs",
                include_str!("../../components/bracket/round_robin.rs"),
            ),
            (
                "bracket/swiss.rs",
                include_str!("../../components/bracket/swiss.rs"),
            ),
            (
                "bracket/match_card.rs",
                include_str!("../../components/bracket/match_card.rs"),
            ),
        ] {
            let body = src.split("#[cfg(test)]").next().unwrap();
            let lines: Vec<&str> = body.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let t = line.trim();
                if !(t.starts_with("for ") && t.contains(" in ") && t.ends_with('{')) {
                    continue;
                }
                let next = lines[i + 1..]
                    .iter()
                    .map(|l| l.trim())
                    .find(|l| !l.is_empty() && !l.starts_with("//"))
                    .unwrap_or_default();
                assert!(
                    !next.starts_with('{'),
                    "{name}:{}: unkeyed loop body `{next}`",
                    i + 1
                );
            }
        }
    }
}

#[cfg(test)]
mod local_day_tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed(secs: i32) -> chrono::FixedOffset {
        chrono::FixedOffset::east_opt(secs).unwrap()
    }

    #[test]
    fn the_day_follows_the_viewer_offset() {
        let late = Utc.with_ymd_and_hms(2026, 11, 12, 23, 30, 0).unwrap();
        assert_eq!(day_in(late, &Utc).to_string(), "2026-11-12");
        assert_eq!(day_in(late, &fixed(2 * 3600)).to_string(), "2026-11-13");
        assert_eq!(day_in(late, &fixed(-5 * 3600)).to_string(), "2026-11-12");
        let early = Utc.with_ymd_and_hms(2026, 11, 13, 0, 30, 0).unwrap();
        assert_eq!(day_in(early, &fixed(-3600)).to_string(), "2026-11-12");
    }

    /// A London-like zone: UTC in winter, one hour east from 29 March 2026
    /// 01:00 UTC to 25 October 2026 01:00 UTC.
    #[derive(Clone, Copy, Debug)]
    struct DstZone;

    impl DstZone {
        fn offset_at(at: chrono::NaiveDateTime) -> chrono::FixedOffset {
            let start = Utc
                .with_ymd_and_hms(2026, 3, 29, 1, 0, 0)
                .unwrap()
                .naive_utc();
            let end = Utc
                .with_ymd_and_hms(2026, 10, 25, 1, 0, 0)
                .unwrap()
                .naive_utc();
            if at >= start && at < end {
                fixed(3600)
            } else {
                fixed(0)
            }
        }
    }

    impl chrono::TimeZone for DstZone {
        type Offset = chrono::FixedOffset;
        fn from_offset(_: &chrono::FixedOffset) -> Self {
            DstZone
        }
        fn offset_from_local_date(
            &self,
            local: &chrono::NaiveDate,
        ) -> chrono::LocalResult<chrono::FixedOffset> {
            chrono::LocalResult::Single(Self::offset_at(local.and_hms_opt(12, 0, 0).unwrap()))
        }
        fn offset_from_local_datetime(
            &self,
            local: &chrono::NaiveDateTime,
        ) -> chrono::LocalResult<chrono::FixedOffset> {
            chrono::LocalResult::Single(Self::offset_at(*local))
        }
        fn offset_from_utc_date(&self, utc: &chrono::NaiveDate) -> chrono::FixedOffset {
            Self::offset_at(utc.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &chrono::NaiveDateTime) -> chrono::FixedOffset {
            Self::offset_at(*utc)
        }
    }

    #[test]
    fn each_game_keeps_its_own_day_across_a_dst_change() {
        // Played 23:30 UTC in summer: 00:30 the next day on the viewer's
        // summer clock. Viewed in November, today's offset is +0, which
        // would put the game a day early.
        let summer = Utc.with_ymd_and_hms(2026, 7, 1, 23, 30, 0).unwrap();
        assert_eq!(day_in(summer, &DstZone).to_string(), "2026-07-02");
        let today_offset = DstZone::offset_at(
            Utc.with_ymd_and_hms(2026, 11, 17, 12, 0, 0)
                .unwrap()
                .naive_utc(),
        );
        assert_eq!(day_in(summer, &today_offset).to_string(), "2026-07-01");
        // Either side of the autumn change, each game uses its own offset.
        let before = Utc.with_ymd_and_hms(2026, 10, 24, 23, 30, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 10, 25, 23, 30, 0).unwrap();
        assert_eq!(day_in(before, &DstZone).to_string(), "2026-10-25");
        assert_eq!(day_in(after, &DstZone).to_string(), "2026-10-25");
        // And the spring change.
        let spring = Utc.with_ymd_and_hms(2026, 3, 29, 23, 30, 0).unwrap();
        assert_eq!(day_in(spring, &DstZone).to_string(), "2026-03-30");
        let winter = Utc.with_ymd_and_hms(2026, 3, 28, 23, 30, 0).unwrap();
        assert_eq!(day_in(winter, &DstZone).to_string(), "2026-03-28");
    }

    #[test]
    fn offsets_are_labelled_from_utc() {
        use chrono::FixedOffset;
        let label = |secs| utc_offset_label(&FixedOffset::east_opt(secs).unwrap());
        assert_eq!(label(0), "UTC");
        assert_eq!(label(2 * 3600), "UTC+2");
        assert_eq!(label(-5 * 3600), "UTC-5");
        assert_eq!(label(5 * 3600 + 30 * 60), "UTC+5:30");
        assert_eq!(label(-(3 * 3600 + 30 * 60)), "UTC-3:30");
        // A date and time names the clock it is on.
        let at = Utc.with_ymd_and_hms(2026, 11, 2, 20, 5, 0).unwrap();
        let on = |secs| date_time_label(&at.with_timezone(&FixedOffset::east_opt(secs).unwrap()));
        assert_eq!(on(0), "2 Nov 2026, 20:05 UTC");
        assert_eq!(on(3600), "2 Nov 2026, 21:05 UTC+1");
        assert_eq!(on(-5 * 3600), "2 Nov 2026, 15:05 UTC-5");
        assert_eq!(local_date_time(&at), "2 Nov 2026, 20:05 UTC");
        // The local time and the local day are the same conversion.
        let at = Utc.with_ymd_and_hms(2026, 7, 15, 22, 30, 0).unwrap();
        assert_eq!(local_time(at).date_naive(), local_day(at));
        let source = include_str!("mod.rs");
        let body = &source[source.find("pub(super) fn local_time(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("at.with_timezone(&chrono::Local)"), "{body}");
    }

    #[test]
    fn local_day_converts_each_instant_not_todays_offset() {
        let source = include_str!("mod.rs");
        let body = &source[source.find("pub(super) fn local_day(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("day_in(at, &chrono::Local)"), "{body}");
        assert!(!body.contains("Local::now()"), "{body}");
        assert!(!body.contains("offset()"), "{body}");
    }

    #[test]
    fn history_recent_form_and_chart_dates_share_the_viewer_day() {
        // One source for the day: no stats date formats the UTC instant.
        for (name, source) in [
            ("history.rs", include_str!("history.rs")),
            ("overview.rs", include_str!("overview.rs")),
            ("rank.rs", include_str!("rank.rs")),
        ] {
            let body = source.split("#[cfg(test)]").next().unwrap();
            assert!(!body.contains("played_at.format(\"%-d"), "{name}");
            assert!(!body.contains("at.format(\"%-d"), "{name}");
            // Nor a bare UTC day as a machine value: a `<time datetime>` is
            // the instant (`to_rfc3339`), so it never names another day than
            // the text. A labelled "UTC" title with the time is fine.
            for line in body.lines().filter(|l| l.contains(".format(\"%Y-%m-%d")) {
                assert!(line.contains(" UTC\")"), "{name}: {line}");
            }
            assert!(body.contains("local_day("), "{name}");
        }
        // The Recent form row says the same thing in its markup.
        // The game page names the same day, with the time and its offset.
        let game = include_str!("game.rs");
        let game = game.split("#[cfg(test)]").next().unwrap();
        let time = &game[game.find("fn game_time(").unwrap()..];
        let time = &time[..time.find("\n}\n").unwrap()];
        assert!(time.contains("super::local_date_time(played_at)"), "{time}");
        assert!(!time.contains("played_at.format("), "{time}");
        let source = include_str!("mod.rs");
        let body = &source[source.find("pub(crate) fn local_date_time(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("date_time_label(&local_time(*at))"), "{body}");
        let overview = include_str!("overview.rs");
        let form_row = &overview[overview.find("fn form_row(").unwrap()..];
        let form_row = &form_row[..form_row.find("rsx!").unwrap()];
        assert!(form_row.contains("played_at.to_rfc3339()"), "{form_row}");
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

#[cfg(test)]
mod tab_tests {
    use super::*;

    #[test]
    fn my_stats_tab_ids_are_unique_and_differ_from_the_panel() {
        let tabs = [
            StatsTab::Overview,
            StatsTab::Heroes,
            StatsTab::Maps,
            StatsTab::History,
        ];
        let ids: Vec<&str> = tabs.iter().map(|t| stats_tab_id(*t)).collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "{id}");
            assert_ne!(*id, MY_STATS_PANEL);
        }
    }
}

#[cfg(test)]
mod shell_tests {
    use super::*;
    use crate::components::member_gate::test_route::wall;
    use crate::components::ui::test_support::{css_rule, has_decl, render, render_routed};
    use scuffed_types::{OrgRole, UserInfo};

    /// `/stats` as a guest (`None`) or as Player01 with `role`, with the
    /// contexts the app root provides. The site shell is not part of it, so
    /// the markup is the page body alone.
    fn stats_page(role: Option<OrgRole>) -> String {
        render_routed(move || {
            let auth = use_signal(|| AuthState {
                user: role.map(|role| UserInfo {
                    id: "u1".into(),
                    username: "Player01".into(),
                    avatar_url: None,
                    role: Some(role),
                }),
                loading: false,
            });
            use_context_provider(|| auth);
            crate::state::provide_site_settings();
            crate::state::provide_relay_availability();
            rsx! {
                crate::components::ToastProvider { Stats {} }
            }
        })
    }

    /// The page body, from its root element on.
    fn page(html: &str) -> &str {
        let start = html
            .find("<div class=\"stats-page my-stats\"")
            .unwrap_or_else(|| panic!("no stats page in {html}"));
        &html[start..]
    }

    #[test]
    fn guest_gets_the_wall_in_the_wide_chrome_with_one_h1() {
        let html = stats_page(None);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(!html.contains("<main"), "the shell owns main: {html}");
        let page = page(&html);
        assert!(
            page.contains("<div class=\"ui-shell ui-shell--wide stats-shell\">"),
            "{page}"
        );
        assert!(page.contains("<h1>Stats</h1>"), "{page}");
        assert!(
            page.contains("Personal match history, role split, and form. Log in to see yours."),
            "{page}"
        );
        let wall = wall(page);
        assert!(
            wall.contains("<h2 class=\"wall-title\">Log in to see your stats</h2>"),
            "{wall}"
        );
        assert!(
            wall.contains("Members get win rate, time on each role, and recent form"),
            "the wall says what members get: {wall}"
        );
        let login = wall.find("href=\"/login\"").expect("log in");
        let boards = wall.find("href=\"/leaderboards\"").expect("leaderboards");
        assert!(login < boards, "{wall}");
        assert!(wall.contains(">Log in</a>"), "{wall}");
        assert!(wall.contains(">See the leaderboards</a>"), "{wall}");
        assert!(wall.contains("class=\"btn is-primary\""), "{wall}");
        assert!(wall.contains("class=\"btn is-ghost\""), "{wall}");
        assert!(!page.contains("Sign in"), "the site says Log in: {page}");
        // None of the member chrome.
        for member_only in [
            "role=\"tablist\"",
            "data-density",
            "Stat tracker",
            "stats-season",
        ] {
            assert!(!page.contains(member_only), "{member_only}: {page}");
        }
    }

    #[test]
    fn member_gets_your_stats_with_one_h1_tabs_and_one_panel() {
        let html = stats_page(Some(OrgRole::Member));
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(!html.contains("<main"), "the shell owns main: {html}");
        let page = page(&html);
        // Density is the one chosen in Settings, on the root element: the
        // page sets none of its own and has no density control.
        assert!(
            page.starts_with("<div class=\"stats-page my-stats\">"),
            "{page}"
        );
        assert!(!page.contains("data-density"), "{page}");
        assert!(!page.contains("Density"), "{page}");
        assert!(
            page.contains("<div class=\"ui-shell ui-shell--wide stats-shell\">"),
            "{page}"
        );
        assert!(page.contains("<h1>Your stats</h1>"), "{page}");
        assert!(
            page.contains(
                "<p class=\"page-lead\">Signed in as Player01. Figures are from all of your logged matches.</p>"
            ),
            "{page}"
        );
        // Tabs: the shared chips in a named tablist, wired to the one panel.
        assert!(
            page.contains(
                "<div class=\"chip-row is-scroll stats-tabs\" role=\"tablist\" aria-label=\"Your stats sections\">"
            ),
            "{page}"
        );
        assert_eq!(page.matches("role=\"tab\"").count(), 4, "{page}");
        assert_eq!(page.matches("role=\"tabpanel\"").count(), 1, "{page}");
        assert_eq!(page.matches("aria-selected=\"true\"").count(), 1, "{page}");
        for tab in [
            StatsTab::Overview,
            StatsTab::Heroes,
            StatsTab::Maps,
            StatsTab::History,
        ] {
            let id = stats_tab_id(tab);
            assert!(
                page.contains(&format!("id=\"{id}\" aria-controls=\"{MY_STATS_PANEL}\"")),
                "{id}: {page}"
            );
        }
        assert!(
            page.contains(&format!(
                "id=\"{MY_STATS_PANEL}\" class=\"stats-panel\" role=\"tabpanel\" aria-labelledby=\"{}\" tabindex=\"0\"",
                stats_tab_id(StatsTab::Overview)
            )),
            "{page}"
        );
        assert!(page.contains("class=\"chip stats-tab active\""), "{page}");
        // The page-level control is the shared button, with a name.
        assert!(page.contains(">Leaderboards</a>"), "{page}");
        // The tracker block comes after the stats, under its own h2.
        let panel = page.find("role=\"tabpanel\"").unwrap();
        let tracker = page
            .find("<h2 class=\"ui-section-title\" id=\"stats-tracker-title\">Stat tracker</h2>")
            .expect("tracker heading");
        assert!(panel < tracker, "{page}");
        assert!(
            page.contains("aria-expanded=\"false\" aria-controls=\"stats-tracker-setup\"")
                && page.contains("id=\"stats-tracker-setup\""),
            "the reinstall button names the block it opens: {page}"
        );
        // The status line of the pending summary is in the page from the start.
        assert!(page.contains("role=\"status\">Loading stats</p>"), "{page}");
    }

    #[test]
    fn summary_is_three_tiles_with_the_label_under_the_value() {
        fn rate() -> Element {
            summary_tiles(Tally::new(13, 8, 4, 1), KdTile::Ready(Some(2.104)))
        }
        fn few() -> Element {
            summary_tiles(Tally::new(9, 6, 3, 0), KdTile::Ready(None))
        }
        let html = render(rate);
        assert_eq!(html.matches("class=\"stat-tile\"").count(), 3, "{html}");
        for (value, label) in [("61.5%", "Win rate"), ("13", "Matches"), ("2.10", "K/D")] {
            assert!(
                html.contains(&format!(
                    "<div class=\"stat-tile-value\">{value}</div><div class=\"stat-tile-label\">{label}</div>"
                )),
                "{label}: {html}"
            );
        }
        assert!(html.contains("Record 8W-4L-1D."), "{html}");

        // Under 10 decided games: the record, labelled, and no percent.
        let html = render(few);
        assert!(
            html.contains(
                "<div class=\"stat-tile-value\">6W-3L</div><div class=\"stat-tile-label\">Too few games</div>"
            ),
            "{html}"
        );
        assert!(!html.contains('%'), "{html}");
        assert!(
            html.contains(
                "<div class=\"stat-tile-value\">N/A</div><div class=\"stat-tile-label\">K/D</div>"
            ),
            "{html}"
        );
    }

    #[test]
    fn zero_match_copy_and_links_agree() {
        assert!(!no_logged_games_message(None).contains("season"));
        assert!(no_logged_games_message(Some("current")).contains("this season"));
        // Every "Set up the tracker" on /stats goes to the steps on the page.
        for (name, src) in [
            ("mod.rs", include_str!("mod.rs")),
            ("heroes.rs", include_str!("heroes.rs")),
            ("maps.rs", include_str!("maps.rs")),
            ("history.rs", include_str!("history.rs")),
        ] {
            let body = src.split("#[cfg(test)]").next().unwrap();
            for (at, _) in body.match_indices("\"Set up the tracker\"") {
                let line = &body[body[..at].rfind('\n').unwrap()..at];
                assert!(line.contains("TRACKER_ANCHOR"), "{name}: {line}");
            }
        }
        assert_eq!(TRACKER_ANCHOR, "#stats-tracker");
        // See all matches removes itself with the Overview: focus goes to
        // the History tab it opened.
        let src = include_str!("mod.rs");
        let open = &src[src.find("let open_history = use_callback").unwrap()..];
        let open = &open[..open.find("});").unwrap()];
        assert!(open.contains("focus_element_by_id_soon("), "{open}");
        assert!(open.contains("stats_tab_id(StatsTab::History)"), "{open}");
    }

    #[test]
    fn kd_reads_na_only_when_loaded_with_no_deaths() {
        fn loading() -> Element {
            summary_tiles(Tally::new(13, 8, 4, 1), KdTile::Loading)
        }
        fn failed() -> Element {
            summary_tiles(Tally::new(13, 8, 4, 1), KdTile::Failed)
        }
        for (name, html, words) in [
            ("loading", render(loading), "Loading"),
            ("failed", render(failed), "Not available"),
        ] {
            assert!(!html.contains("N/A"), "{name}: {html}");
            assert!(
                html.contains(&format!(
                    "<div class=\"stat-tile-value\"><span aria-hidden=\"true\">-</span>\
                     <span class=\"visually-hidden\">{words}</span></div><div class=\"stat-tile-label\">K/D</div>"
                )),
                "{name}: {html}"
            );
        }
    }

    /// Issue 725 (WCAG 2.4.11): at 390 the tabpanel, a Tab stop taller than
    /// the band between the top bar and the phone tab bar, sat partly under
    /// the tab bar when focused. On phone it is capped at that band, from
    /// the shell's own chrome height and the page's scroll-padding gaps on
    /// the small viewport, and scrolls inside itself, so the focused panel
    /// always fits clear of both bars. Its padding is the focus ring's room,
    /// so the scroller does not clip the rings of the controls at its edges.
    #[test]
    fn the_tabpanel_fits_between_the_phone_bars() {
        let phone = &STATS_CSS[STATS_CSS
            .find("@media (max-width: 820px)")
            .expect("phone block")..];
        let phone = &phone[..phone.find("\n    }\n").expect("phone block ends")];
        let panel = css_rule(phone, ".my-stats .stats-panel {");
        assert!(
            has_decl(
                &panel,
                "max-height: calc(100svh - var(--shell-chrome-height) - 2 * var(--space-2))"
            ),
            "{panel}"
        );
        // The vh line is the fallback, so it comes first.
        assert!(
            panel.find("max-height: calc(100vh -").expect("vh fallback")
                < panel.find("max-height: calc(100svh -").unwrap(),
            "{panel}"
        );
        assert!(has_decl(&panel, "overflow-y: auto"), "{panel}");
        assert!(has_decl(&panel, "box-sizing: border-box"), "{panel}");
        assert!(
            has_decl(
                &panel,
                "--stats-panel-ring-room: calc(var(--focus-ring-width) + var(--focus-ring-offset))"
            ),
            "{panel}"
        );
        assert!(
            has_decl(&panel, "padding: var(--stats-panel-ring-room)"),
            "{panel}"
        );
        assert!(
            has_decl(
                &panel,
                "margin-inline: calc(var(--stats-panel-ring-room) * -1)"
            ),
            "{panel}"
        );
        // The cap is the band the shell keeps focus in: the same gap the
        // page's scroll-padding leaves at each bar.
        let shell = include_str!("../../layouts/shell.rs");
        assert!(
            shell.contains("scroll-padding-top: calc(var(--nav-height-used) + var(--space-2))")
        );
        assert!(
            shell.contains("scroll-padding-bottom: calc(var(--tab-bar-height) + var(--space-2))")
        );
        assert!(shell.contains(
            "--shell-chrome-height: calc(var(--nav-height-used) + var(--tab-bar-height))"
        ));
        // It is still a Tab stop, so keyboard users can scroll it (the
        // markup is pinned in `member_gets_your_stats_with_one_h1_tabs_and_one_panel`).
    }

    #[test]
    fn tiles_stay_three_up_on_phone_and_badges_hug_their_word() {
        let tiles = css_rule(STATS_CSS, ".my-stats .stats-tiles {");
        assert!(
            has_decl(&tiles, "grid-template-columns: repeat(3, minmax(0, 1fr))"),
            "{tiles}"
        );
        let value = css_rule(STATS_CSS, ".my-stats .stat-tile-value {");
        assert!(has_decl(&value, "font-size: var(--text-2xl)"), "{value}");
        assert!(
            has_decl(&value, "font-variant-numeric: tabular-nums"),
            "{value}"
        );

        let phone = &STATS_CSS[STATS_CSS
            .find("@media (max-width: 820px)")
            .expect("phone block")..];
        let phone = &phone[..phone.find("\n    }\n").expect("phone block ends")];
        assert!(
            !phone.contains("grid-template-columns: 1fr") && !phone.contains(".stats-tiles { grid"),
            "the tiles keep three columns on phone: {phone}"
        );
        assert!(
            has_decl(
                &css_rule(phone, ".my-stats .stat-tile {"),
                "min-height: var(--hit-target-min)"
            ),
            "{phone}"
        );
        // Phone: the role name wraps above its bar.
        assert!(
            has_decl(
                &css_rule(phone, ".role-row .role-name {"),
                "grid-column: 1 / -1"
            ),
            "{phone}"
        );
        // The result badge hugs its word, in the one-column phone card too.
        let badge = css_rule(STATS_CSS, ".form-row > .ui-pill {");
        assert!(has_decl(&badge, "width: max-content"), "{badge}");
        assert!(has_decl(&badge, "justify-self: start"), "{badge}");
    }

    #[test]
    fn shell_rules_use_tokens_only() {
        // The rules this page owns. The other tabs' rules live in the same
        // sheet and are checked by their own tests.
        for marker in [
            ".stats-header {",
            ".stats-header-actions {",
            ".stats-panel {",
            ".my-stats .stats-panel {",
            ".stats-card {",
            ".stats-state {",
            ".my-stats .stats-visibility-note {",
            ".my-stats .stats-tiles {",
            ".my-stats .stat-tile {",
            ".my-stats .stat-tile-value {",
            ".my-stats .stat-tile-label {",
            ".stats-overview {",
            ".role-row {",
            ".role-track {",
            ".form-row {",
            ".form-row-title {",
            ".form-row-meta {",
            ".overview-grid {",
            ".role-card {",
            ".mini-hero-row {",
            ".mode-tile-name {",
            ".tracker-slim {",
            ".tracker-release-notice {",
            ".daemon-settings summary {",
            ".tracker-download {",
            ".tracker-download code.tracker-install {",
        ] {
            let rule = css_rule(STATS_CSS, marker);
            for unit in ["rem", "px;", "em;"] {
                let literal = rule
                    .split(';')
                    .filter(|decl| !decl.contains("border") && !decl.contains("--history-"))
                    .any(|decl| {
                        decl.contains(unit.trim_end_matches(';')) && {
                            let value = decl.split(':').nth(1).unwrap_or("");
                            value
                                .split(|c: char| {
                                    c.is_whitespace() || c == '(' || c == ')' || c == ','
                                })
                                .any(|part| {
                                    part.ends_with(unit.trim_end_matches(';'))
                                        && part
                                            .trim_end_matches(unit.trim_end_matches(';'))
                                            .parse::<f64>()
                                            .is_ok()
                                })
                        }
                    });
                assert!(!literal, "{marker} has a literal {unit} size: {rule}");
            }
            if rule.contains("font-size") {
                assert!(rule.contains("font-size: var(--text-"), "{marker}: {rule}");
            }
        }
        // The install note is a quiet line, not a warning banner.
        let notice = css_rule(STATS_CSS, ".tracker-release-notice {");
        assert!(
            !notice.contains("--warn") && !notice.contains("background"),
            "{notice}"
        );
    }

    #[test]
    fn density_rules_follow_the_root_density_from_settings() {
        // The root element carries the Settings density (`sc-density`), so
        // every density rule here hangs off an ancestor of the page, never
        // off the page wrapper, and the page keeps no density store.
        assert!(
            !STATS_CSS.contains(".stats-page[data-density"),
            "{STATS_CSS}"
        );
        for density in ["compact", "comfortable"] {
            assert!(
                STATS_CSS.contains(&format!("[data-density=\"{density}\"] .stats-page ")),
                "{density}"
            );
        }
        let source = include_str!("mod.rs");
        let store_key = ["stats-ui", "-density"].concat();
        assert!(
            !source.contains(&store_key),
            "the page keeps no density store"
        );
    }

    #[test]
    fn only_a_pinned_season_that_is_not_current_is_past() {
        let season = |id: &str, is_current: bool| scuffed_types::Season {
            id: id.to_string(),
            name: id.to_string(),
            starts_at: Utc::now(),
            ends_at: Utc::now(),
            is_current,
        };
        let list = [season("s1", false), season("s2", true)];
        let past = |sel| picked_season_is_not_current(sel, Some(&list));
        assert!(!past(None), "all time");
        assert!(!past(Some("current")), "follows the current season");
        assert!(!past(Some("s2")), "pinned, but current now");
        assert!(past(Some("s1")), "pinned past season");
        assert!(past(Some("gone")), "an id missing from the list");
        assert!(
            !picked_season_is_not_current(Some("s1"), None),
            "list not loaded"
        );
        // Both tabs pass it to the chart.
        let source = include_str!("mod.rs");
        let body = &source[source.find("let chart_for = rank::ChartFor").unwrap()..];
        let body = &body[..body.find("if !auth().loading").unwrap()];
        assert!(body.contains("past_season,\n    };"), "{body}");
        assert_eq!(body.matches("chart_for.clone(),").count(), 2, "{body}");
    }

    #[test]
    fn lead_names_the_member_and_the_season_window() {
        assert_eq!(
            stats_lead(Some("Player01"), None),
            "Signed in as Player01. Figures are from all of your logged matches."
        );
        assert_eq!(
            stats_lead(Some("Player01"), Some("current")),
            "Signed in as Player01. Figures are from your logged matches this season."
        );
        assert_eq!(
            stats_lead(Some("Player01"), Some("season:abc")),
            "Signed in as Player01. Figures are from your logged matches in the season you picked."
        );
        assert_eq!(
            stats_lead(Some("  "), Some("")),
            "Figures are from all of your logged matches."
        );
        assert!(!stats_lead(Some("Player01"), Some("season:abc")).contains("season:abc"));
    }

    #[test]
    fn load_error_is_the_shared_card_with_retry() {
        fn view() -> Element {
            let refresh = use_signal(|| 0u64);
            load_error_state("role breakdown", None, refresh)
        }
        let html = render(view);
        assert!(
            html.contains("class=\"fetch-error-wrap\" role=\"alert\""),
            "{html}"
        );
        assert!(html.contains("load role breakdown.</p>"), "{html}");
        assert!(html.contains("class=\"fetch-error__retry\""), "{html}");
        assert!(html.contains(">Retry</button>"), "{html}");

        // QA on 748: with the resource's reason, the line says why.
        fn offline() -> Element {
            let refresh = use_signal(|| 0u64);
            load_error_state(
                "recent matches",
                Some(crate::client_error::NETWORK_ERROR_MESSAGE.to_string()),
                refresh,
            )
        }
        let html = render(offline);
        assert!(
            html.contains(
                "Couldn&#39;t load recent matches. Could not reach the server. Check your connection and try again.</p>"
            ),
            "{html}"
        );
    }

    #[test]
    fn design_pass_spacing_and_target_fixes_hold() {
        // The daemon settings toggle is a full touch target (it measured
        // 43px at 390).
        let summary = css_rule(STATS_CSS, ".daemon-settings summary {");
        assert!(
            has_decl(&summary, "min-height: var(--hit-target-min)"),
            "{summary}"
        );
        assert!(has_decl(&summary, "box-sizing: border-box"), "{summary}");
        // Overview and History space the rank section with their flex gap
        // (`.stats-overview`, `.stats-view-body`), so the section has no
        // outer margin of its own and no page needs to cancel one.
        let rank = css_rule(super::rank::RANK_CHART_CSS, ".rank-block {");
        assert!(has_decl(&rank, "margin: 0"), "{rank}");
        assert!(!STATS_CSS.contains(".rank-block {"), "no override");
        let body = css_rule(
            crate::pages::stats_member::STATS_VIEWS_CSS,
            ".stats-view-body {",
        );
        assert!(has_decl(&body, "gap: var(--space-section)"), "{body}");
        // A failed or gate card in the tabpanel starts at the left edge.
        let wall = css_rule(STATS_CSS, ".my-stats .stats-panel > .wall {");
        assert!(has_decl(&wall, "margin: 0"), "{wall}");
        // A long role result wraps under the stat line.
        let card = css_rule(STATS_CSS, ".role-card {");
        assert!(has_decl(&card, "flex-wrap: wrap"), "{card}");
        let info = css_rule(STATS_CSS, ".role-card-info {");
        assert!(
            has_decl(&info, "flex: 1 1 calc(var(--space-12) * 5)"),
            "{info}"
        );
        // The install block reads heading first, then the release note.
        let src = include_str!("mod.rs");
        let prod = src.split("mod match_row_tests").next().unwrap();
        let block = &prod[prod.find("class: \"tracker-download\",").expect("block")..];
        let head = block.find("h3 { \"Get the Stat Tracker\" }").expect("head");
        let note = block
            .find("{tracker_release_notice(&tracker_notice)}")
            .expect("note");
        assert!(head < note, "heading before the note");
    }

    #[test]
    fn shell_copy_has_no_long_dashes() {
        let src = include_str!("mod.rs");
        let prod = src.split("mod match_row_tests").next().unwrap();
        assert!(!prod.contains('\u{2014}') && !prod.contains('\u{2013}'));
        assert!(
            !prod.contains('\u{2026}'),
            "three dots, not an ellipsis glyph"
        );
    }
}
