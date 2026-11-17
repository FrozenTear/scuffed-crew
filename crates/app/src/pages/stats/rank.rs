//! Rank on personal games: a badge on each game and a progress chart per
//! queue and role.
//!
//! Match JSON (list rows and `GET /api/stats/me/matches/{id}`) may carry an
//! optional `rank` object, the shared [`scuffed_types::MatchRank`]. Every
//! field is `{ "value": ..., "confidence": 0..1 }` and either part may be
//! null:
//!
//! ```json
//! { "role": { "value": "Support", "confidence": 0.9 },
//!   "role_rank": { "value": "Diamond", "confidence": 0.95 },
//!   "division": { "value": 2, "confidence": 0.875 },
//!   "progress_pct": { "value": 45, "confidence": 0.8 },
//!   "progress_change": { "value": 23, "confidence": 0.75 },
//!   "queue_title": { "value": "ROLE QUEUE", "confidence": 0.9 },
//!   "rank_recognizer": "rank-v1" }
//! ```
//!
//! The field is optional all the way down. Absent, null, `{}`, or an object
//! with no value in any field is no rank. The raw value goes through the same
//! [`resolve_rank`] the server uses, so a bad part becomes null on its own and
//! never costs the game.
//!
//! Any value that is not null shows, whatever its confidence: rank-v1 and the
//! tracker never send a value they rejected, and word fields (tier, tags) can
//! be accepted below 0.5. Out of range numbers are still dropped.
//!
//! The queue is the read queue title. The tracker does not send one yet, so
//! it is null for now; a null title is simply not shown and the game is
//! grouped by role alone. The queue never comes from `next_mode_hint` (the
//! next queue banner, not this game's) or `game_mode` (the map mode). A
//! missing role shows as "Role unknown".
//!
//! `progress_pct` is -100..=100; a negative value means demotion protection
//! and shows as is ("-15%").
//!
//! `progress_change` is the pill on the progress bar in progress percentage
//! points, -100..=100, shown as "+23%".
//!
//! The badge is opt in: with no rank data on a game, it renders nothing.
//!
//! The chart ([`RankChart`]) plots absolute ladder position, so a rank up
//! stays an up: `tier * 500 + (5 - division) * 100 + progress`, from Bronze 5
//! at 0 to Champion 1 at 4500 (see [`ladder_position`]). A recognised tier
//! with no division sits at the tier midpoint. A tier word that is not on the
//! ladder is not plotted: the game keeps its slot in time, a note says so,
//! and the table lists it as unplaced. The SVG is decorative. Names, dates,
//! marks and the 44px hit targets are HTML, and the same data is in a hidden
//! summary and a table.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use scuffed_types::api::resolve_rank;
use scuffed_types::{MatchRank, RankField, RankRole};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::components::ui::inline_confirm::focus_element_by_id;
use crate::components::ui::{ChipButton, ChipRow, btn_class};

/// Bucket label for a game whose role is missing or unsure.
pub(super) const ROLE_UNKNOWN: &str = "Role unknown";

const CHART_TITLE: &str = "Rank progress";

/// Read the field leniently: null, an object, or a string holding the
/// object's JSON. Anything else, `{}`, or an object with no value is None.
pub(super) fn lenient<'de, D: Deserializer<'de>>(d: D) -> Result<Option<MatchRank>, D::Error> {
    let raw = Value::deserialize(d)?;
    Ok(parse_rank(&raw))
}

/// See [`lenient`].
pub(super) fn parse_rank(raw: &Value) -> Option<MatchRank> {
    let decoded;
    let value = match raw {
        Value::String(text) => {
            decoded = serde_json::from_str::<Value>(text).ok()?;
            &decoded
        }
        other => other,
    };
    resolve_rank(Some(value)).filter(has_any_value)
}

/// True when at least one field holds a value. Confidences alone, or only the
/// reader id, are no rank. `next_mode_hint` does not count: it is the next
/// queue banner, not a rank for this game.
fn has_any_value(rank: &MatchRank) -> bool {
    rank.role.value.is_some()
        || rank.role_rank.value.is_some()
        || rank.division.value.is_some()
        || rank.progress_pct.value.is_some()
        || rank.progress_change.value.is_some()
        || rank.change_tags.value.is_some()
        || rank.higher_than_pct.value.is_some()
        || rank.hero_sr.value.is_some()
        || rank.hero_sr_change.value.is_some()
        || rank.queue_title.value.is_some()
}

/// The value when it is not null. No confidence cutoff: the reader only
/// sends values it accepted.
fn value_of<T: Clone>(field: &RankField<T>) -> Option<T> {
    field.value.clone()
}

/// Trimmed, single spaced, or None when blank.
fn clean_text(text: &str) -> Option<String> {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!joined.is_empty()).then_some(joined)
}

/// "ROLE QUEUE" to "Role Queue". The screen draws these in capitals.
fn title_case(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn role_name(role: RankRole) -> &'static str {
    match role {
        RankRole::Tank => "Tank",
        RankRole::Damage => "Damage",
        RankRole::Support => "Support",
        RankRole::Open => "Open queue",
    }
}

fn role_order(role: Option<RankRole>) -> u8 {
    match role {
        Some(RankRole::Tank) => 0,
        Some(RankRole::Damage) => 1,
        Some(RankRole::Support) => 2,
        Some(RankRole::Open) => 3,
        None => 4,
    }
}

/// What one game's rank shows: every non-null value that is in range.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct RankShown {
    pub role: Option<RankRole>,
    /// Tier name in title case, e.g. "Diamond".
    pub tier: Option<String>,
    /// 1..=5.
    pub division: Option<u8>,
    /// -100..=100. A negative value is demotion protection and shows as is.
    pub progress: Option<i32>,
    /// Progress points gained or lost this game, -100..=100.
    pub change: Option<i32>,
    /// Queue title as read, single spaced, in capitals (the bucket key).
    pub queue: Option<String>,
}

impl RankShown {
    /// None when no field has a value to show.
    pub(super) fn from_rank(rank: &MatchRank) -> Option<Self> {
        let shown = Self {
            role: value_of(&rank.role),
            tier: value_of(&rank.role_rank)
                .and_then(|t| clean_text(&t))
                .map(|t| title_case(&t.to_lowercase())),
            division: value_of(&rank.division).filter(|d| (1..=5).contains(d)),
            progress: value_of(&rank.progress_pct).filter(|p| (-100..=100).contains(p)),
            change: value_of(&rank.progress_change).filter(|c| (-100..=100).contains(c)),
            queue: value_of(&rank.queue_title)
                .and_then(|q| clean_text(&q))
                .map(|q| q.to_uppercase()),
        };
        (shown != Self::default()).then_some(shown)
    }

    /// "Diamond 2", "Diamond", "Division 2", or None.
    pub(super) fn rank_text(&self) -> Option<String> {
        match (&self.tier, self.division) {
            (Some(t), Some(d)) => Some(format!("{t} {d}")),
            (Some(t), None) => Some(t.clone()),
            (None, Some(d)) => Some(format!("Division {d}")),
            (None, None) => None,
        }
    }

    /// The queue in title case, or None while the tracker sends no title.
    pub(super) fn queue_label(&self) -> Option<String> {
        self.queue.as_deref().map(title_case_upper)
    }

    pub(super) fn role_label(&self) -> &'static str {
        self.role.map(role_name).unwrap_or(ROLE_UNKNOWN)
    }

    /// "Support, Role Queue", or the role alone. See [`context_label`].
    pub(super) fn context(&self) -> String {
        context_label(self.role, self.queue.as_deref())
    }

    /// The short visible pieces: rank, progress, change.
    fn badge_parts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if let Some(rank) = self.rank_text() {
            parts.push(rank);
        }
        if let Some(p) = self.progress {
            parts.push(format!("{p}%"));
        }
        if let Some(c) = self.change {
            parts.push(change_text(c));
        }
        parts
    }

    /// One sentence for screen readers and the tooltip.
    pub(super) fn sentence(&self) -> String {
        let mut parts = vec![self.context()];
        if let Some(rank) = self.rank_text() {
            parts.push(rank);
        }
        if let Some(p) = self.progress {
            parts.push(format!("{p}% progress"));
        }
        if let Some(c) = self.change {
            parts.push(format!("{} this game", change_text(c)));
        }
        format!("Rank: {}", parts.join(", "))
    }
}

/// Role and queue for display: "Support, Role Queue". A null queue is left
/// out, and a queue that only repeats the role ("Open queue" with the title
/// "OPEN QUEUE") shows once.
pub(super) fn context_label(role: Option<RankRole>, queue: Option<&str>) -> String {
    let role = role.map(role_name).unwrap_or(ROLE_UNKNOWN);
    match queue.map(title_case_upper) {
        Some(q) if !q.eq_ignore_ascii_case(role) => format!("{role}, {q}"),
        _ => role.to_string(),
    }
}

/// Title case for a key that is stored in capitals.
fn title_case_upper(key: &str) -> String {
    title_case(&key.to_lowercase())
}

/// "+23%", "-15%", "0%".
pub(super) fn change_text(change: i32) -> String {
    if change > 0 {
        format!("+{change}%")
    } else {
        format!("{change}%")
    }
}

/// Anything that is not id-safe becomes an underscore. Same rule as the
/// bans and picks ids.
fn id_part(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Id of a game's badge text. Built from the match id and the field name.
pub(super) fn badge_id(match_id: &str) -> String {
    format!("rank-{}-badge", id_part(match_id))
}

pub(super) const RANK_CSS: &str = r#"
    .rank-badge {
        display: inline-flex;
        flex-wrap: wrap;
        align-items: baseline;
        gap: 0 var(--space-1);
        max-width: 100%;
        padding: 0 var(--space-2);
        background: var(--surface-sunken);
        border: 1px solid var(--border);
        border-radius: var(--radius-pill);
        font-size: var(--text-xs);
        color: var(--text);
        font-variant-numeric: tabular-nums;
        overflow-wrap: anywhere;
    }
    .rank-badge-sep { color: var(--text-subtle); }
    .match-card .match-rank {
        grid-column: 1 / -1;
        min-width: 0;
    }
    .game-rank { margin: var(--space-2) 0 0; }
    .game-rank-queue {
        display: inline-block;
        white-space: nowrap;
        color: var(--text-subtle);
        font-size: var(--text-xs);
        margin-left: var(--space-2);
    }
"#;

/// A compact read-only badge for a match card. Renders nothing without a rank
/// worth showing. Nothing in it is interactive, so it is safe inside a card
/// that is a link.
pub(super) fn rank_badge_line(match_id: &str, rank: Option<&MatchRank>) -> Element {
    let Some(shown) = rank.and_then(RankShown::from_rank) else {
        return rsx! {};
    };
    let parts = shown.badge_parts();
    if parts.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "match-rank",
            {badge(match_id, &shown, &parts)}
        }
    }
}

fn badge(match_id: &str, shown: &RankShown, parts: &[String]) -> Element {
    let id = badge_id(match_id);
    let sentence = shown.sentence();
    rsx! {
        span { class: "rank-badge", id: "{id}", title: "{sentence}",
            span { class: "visually-hidden", "{sentence}" }
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    span { class: "rank-badge-sep", aria_hidden: "true", "·" }
                }
                span { aria_hidden: "true", "{part}" }
            }
        }
    }
}

/// The rank line on the game page: badge plus queue. Nothing without a rank.
pub(super) fn rank_detail(match_id: &str, rank: Option<&MatchRank>) -> Element {
    let Some(shown) = rank.and_then(RankShown::from_rank) else {
        return rsx! {};
    };
    let parts = shown.badge_parts();
    if parts.is_empty() {
        return rsx! {};
    }
    let context = shown.context();
    rsx! {
        p { class: "game-rank",
            {badge(match_id, &shown, &parts)}
            span { class: "game-rank-queue", aria_hidden: "true", "{context}" }
        }
    }
}

// -- Chart --

/// How a game ended, when the caller knows. Shown as a word, never as colour
/// alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GameResult {
    Win,
    Loss,
    Draw,
}

impl GameResult {
    /// The stored outcome word ("victory", "defeat", "draw", or the aliases
    /// "win" and "loss"), any case. Anything else is unknown, so no result
    /// line is shown.
    pub(super) fn from_outcome(outcome: &str) -> Option<Self> {
        match outcome.trim().to_ascii_lowercase().as_str() {
            "win" | "victory" => Some(Self::Win),
            "loss" | "defeat" => Some(Self::Loss),
            "draw" => Some(Self::Draw),
            _ => None,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Win => "Win",
            Self::Loss => "Loss",
            Self::Draw => "Draw",
        }
    }

    /// "a win", for the summary sentence.
    fn phrase(self) -> &'static str {
        match self {
            Self::Win => "a win",
            Self::Loss => "a loss",
            Self::Draw => "a draw",
        }
    }
}

/// One game with a rank, as the chart needs it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RankPoint {
    pub match_id: String,
    pub played_at: DateTime<Utc>,
    pub shown: RankShown,
    /// None when the caller did not say or the outcome is not a known word.
    pub result: Option<GameResult>,
}

/// Every ranked game of one (queue, role), oldest first.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RankSeries {
    /// Queue key in capitals, or None while no title is read.
    pub queue: Option<String>,
    pub role: Option<RankRole>,
    pub points: Vec<RankPoint>,
}

/// True when the game has something the chart or its table can show: a tier
/// word, a division or a progress value.
fn listable(shown: &RankShown) -> bool {
    shown.tier.is_some() || shown.division.is_some() || shown.progress.is_some()
}

impl RankSeries {
    /// Stable, id-safe key built from the queue and role.
    pub(super) fn key(&self) -> String {
        let queue = self.queue.as_deref().map(id_part);
        let role = self.role.map(|r| id_part(role_name(r)));
        format!(
            "q-{}-r-{}",
            queue.as_deref().unwrap_or("unknown"),
            role.as_deref().unwrap_or("unknown")
        )
    }

    pub(super) fn label(&self) -> String {
        context_label(self.role, self.queue.as_deref())
    }

    /// Games the chart lists: plotted ones and unplaced ones.
    fn listed(&self) -> impl Iterator<Item = &RankPoint> {
        self.points.iter().filter(|p| listable(&p.shown))
    }

    fn latest(&self) -> Option<DateTime<Utc>> {
        self.points.iter().map(|p| p.played_at).max()
    }
}

/// Group games by (queue, role). Games without a rank to show are skipped.
/// Order: known queues A to Z, then games with no queue title; inside each,
/// Tank, Damage, Support, Open queue, then "Role unknown". Points oldest
/// first. No game carries a result; see [`rank_series_with_outcomes`].
pub(super) fn rank_series<'a>(
    games: impl IntoIterator<Item = (&'a str, DateTime<Utc>, Option<&'a MatchRank>)>,
) -> Vec<RankSeries> {
    rank_series_with_outcomes(
        games
            .into_iter()
            .map(|(id, played_at, rank)| (id, played_at, rank, None)),
    )
}

/// [`rank_series`], plus each game's stored outcome word ("win", "loss",
/// "draw") so the readout and the table can say Win or Loss.
pub(super) fn rank_series_with_outcomes<'a>(
    games: impl IntoIterator<
        Item = (
            &'a str,
            DateTime<Utc>,
            Option<&'a MatchRank>,
            Option<&'a str>,
        ),
    >,
) -> Vec<RankSeries> {
    let mut out: Vec<RankSeries> = Vec::new();
    for (id, played_at, rank, outcome) in games {
        let Some(shown) = rank.and_then(RankShown::from_rank) else {
            continue;
        };
        let point = RankPoint {
            match_id: id.to_string(),
            played_at,
            shown,
            result: outcome.and_then(GameResult::from_outcome),
        };
        let (queue, role) = (point.shown.queue.clone(), point.shown.role);
        match out.iter_mut().find(|s| s.queue == queue && s.role == role) {
            Some(series) => series.points.push(point),
            None => out.push(RankSeries {
                queue,
                role,
                points: vec![point],
            }),
        }
    }
    for series in &mut out {
        series.points.sort_by(|a, b| {
            a.played_at
                .cmp(&b.played_at)
                .then(a.match_id.cmp(&b.match_id))
        });
    }
    out.sort_by(|a, b| {
        (a.queue.is_none(), &a.queue, role_order(a.role)).cmp(&(
            b.queue.is_none(),
            &b.queue,
            role_order(b.role),
        ))
    });
    out
}

/// Series the chart offers: those with at least one game it can list.
pub(super) fn chart_series(series: Vec<RankSeries>) -> Vec<RankSeries> {
    series
        .into_iter()
        .filter(|s| s.listed().next().is_some())
        .collect()
}

/// The series shown when the member has not picked one: the one with the
/// latest game.
fn default_key(series: &[RankSeries]) -> Option<String> {
    series
        .iter()
        .max_by_key(|s| s.latest())
        .map(RankSeries::key)
}

// -- Ladder maths --

/// The ladder, low to high. Top 500 is a leaderboard placement, not a tier.
const TIERS: [&str; 9] = [
    "Bronze",
    "Silver",
    "Gold",
    "Platinum",
    "Emerald",
    "Diamond",
    "Master",
    "Grandmaster",
    "Champion",
];
/// One division is 100 units of progress.
const DIVISION_UNITS: i32 = 100;
/// Five divisions per tier.
const TIER_SPAN: i32 = 5 * DIVISION_UNITS;
/// Champion 1 at 100%.
const CHAMPION_TOP: i32 = TIERS.len() as i32 * TIER_SPAN;
/// Top of the scale. The band from [`CHAMPION_TOP`] to here is held for a
/// later Top 500 marker, so adding it never moves a tier. It is not drawn.
const SCALE_CEILING: i32 = CHAMPION_TOP + TIER_SPAN;

/// Index of a tier word on the ladder, whatever its case. None for a word
/// that is not a tier (a misread such as "Emerlad").
fn tier_index(word: &str) -> Option<usize> {
    let word = word.trim();
    TIERS.iter().position(|t| t.eq_ignore_ascii_case(word))
}

/// Absolute ladder position: Bronze 5 at 0% is 0, Champion 1 at 100% is 4500.
///
/// With a division: `tier * 500 + (5 - division) * 100 + progress`. Without
/// one: the midpoint of the tier, and the progress is not added (progress is
/// per division, so it would invent a placement in division 5). With a
/// division but no progress: the midpoint of that division, by the same
/// reasoning. Negative progress (demotion protection) sits below the
/// division floor.
fn ladder_position(tier: usize, division: Option<u8>, progress: Option<i32>) -> i32 {
    let base = tier as i32 * TIER_SPAN;
    let pos = match division {
        Some(d) => {
            base + (5 - i32::from(d)) * DIVISION_UNITS + progress.unwrap_or(DIVISION_UNITS / 2)
        }
        None => base + TIER_SPAN / 2,
    };
    pos.clamp(0, CHAMPION_TOP)
}

/// The y range for plotted positions `lo..=hi`: whole divisions, one below
/// and at least one above, inside 0 and [`SCALE_CEILING`].
fn axis_window(lo: i32, hi: i32) -> (i32, i32) {
    let ymin = ((lo.div_euclid(DIVISION_UNITS) - 1) * DIVISION_UNITS).max(0);
    let ymax = ((hi.div_euclid(DIVISION_UNITS) + 2) * DIVISION_UNITS).min(SCALE_CEILING);
    if ymax <= ymin {
        (ymin, ymin + 2 * DIVISION_UNITS)
    } else {
        (ymin, ymax)
    }
}

/// The visible part of one tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TierBand {
    tier: usize,
    start: i32,
    end: i32,
}

/// Tier bands inside the window, bottom up. The reserved band above
/// Champion is left out.
fn tier_bands(ymin: i32, ymax: i32) -> Vec<TierBand> {
    let mut bands = Vec::new();
    let mut start = ymin;
    while start < ymax && start < CHAMPION_TOP {
        let tier = (start / TIER_SPAN) as usize;
        let end = ymax.min((tier as i32 + 1) * TIER_SPAN);
        bands.push(TierBand { tier, start, end });
        start = end;
    }
    bands
}

/// One number per visible division and the position of its centre. A
/// division with less than 40 units in view gets no number.
fn division_labels(ymin: i32, ymax: i32) -> Vec<(i32, f64)> {
    let top = ymax.min(CHAMPION_TOP);
    let mut labels = Vec::new();
    let mut start = ymin.div_euclid(DIVISION_UNITS) * DIVISION_UNITS;
    while start < top {
        let end = start + DIVISION_UNITS;
        let (lo, hi) = (start.max(ymin), end.min(top));
        if hi - lo >= DIVISION_UNITS * 2 / 5 {
            let division = 5 - start.rem_euclid(TIER_SPAN) / DIVISION_UNITS;
            labels.push((division, f64::from(lo + hi) / 2.0));
        }
        start = end;
    }
    labels
}

// Plot box, in SVG units. The SVG stretches to its box (no aspect ratio), and
// every mark and label is HTML placed in percent, so these are ratios only.
const VIEW_W: f64 = 656.0;
const VIEW_H: f64 = 280.0;
const PAD_X: f64 = 8.0;
const PLOT_W: f64 = VIEW_W - 2.0 * PAD_X;
/// Length of a division tick, left of the axis.
const TICK_W: f64 = 6.0;

/// Left edge of game `i` of `n`, in percent of the canvas. Games are evenly
/// spaced in play order; an unplaced game keeps its slot.
fn x_pct(i: usize, n: usize) -> f64 {
    let x = if n <= 1 {
        PAD_X + PLOT_W / 2.0
    } else {
        PAD_X + PLOT_W * i as f64 / (n - 1) as f64
    };
    x / VIEW_W * 100.0
}

/// Top of a ladder position, in percent of the canvas: `ymax` is 0 and
/// `ymin` is 100.
fn y_pct(pos: f64, ymin: i32, ymax: i32) -> f64 {
    (1.0 - (pos - f64::from(ymin)) / f64::from(ymax - ymin)) * 100.0
}

/// "12 Nov 2026", on the viewer's own day like every date on the stats
/// pages ([`super::local_day`]).
fn long_date(at: DateTime<Utc>) -> String {
    super::local_day(at).format("%-d %b %Y").to_string()
}

/// "12 Nov", for the x axis.
fn short_date(at: DateTime<Utc>) -> String {
    super::local_day(at).format("%-d %b").to_string()
}

/// Where a game sits on the ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    tier: usize,
    pos: i32,
}

/// One game in the chart and its table.
#[derive(Debug, Clone, PartialEq)]
struct ChartRow {
    match_id: String,
    /// "12 Nov 2026".
    date: String,
    /// "12 Nov".
    date_short: String,
    /// "Emerald 5", the raw word for an unplaced game ("Emerlad 5"), or
    /// "Rank not read".
    rank: String,
    /// None when the tier word is missing or not a tier: not plotted.
    placed: Option<Placed>,
    division: Option<u8>,
    progress: Option<i32>,
    /// "Up 22 points", "Up to Emerald 5, from Platinum 1 at 96%", ...
    change: String,
    /// The rank differs from the last plotted game: drawn as a diamond.
    rank_changed: bool,
    /// A rank change that moved down the ladder.
    derank: bool,
    result: Option<GameResult>,
    /// The tier word of an unplaced game, when one was read.
    unknown_word: Option<String>,
}

impl ChartRow {
    /// "Emerald 5 · 30%", the readout and phone table form.
    fn rank_line(&self) -> String {
        match self.progress {
            Some(p) => format!("{} · {p}%", self.rank),
            None => self.rank.clone(),
        }
    }

    /// "Emerald 5 at 30 percent", for sentences.
    fn rank_words(&self) -> String {
        match self.progress {
            Some(p) => format!("{} at {p} percent", self.rank),
            None => self.rank.clone(),
        }
    }

    /// Accessible name of the game's hit target.
    fn point_label(&self) -> String {
        let mut label = format!("{}, {}, {}", self.date, self.rank_words(), self.change);
        if let Some(result) = self.result {
            label.push_str(", ");
            label.push_str(result.word());
        }
        label
    }

    /// The Change cell: the change, plus the result when known.
    fn change_cell(&self) -> String {
        match self.result {
            Some(result) => format!("{} ({})", self.change, result.word()),
            None => self.change.clone(),
        }
    }

    fn progress_cell(&self) -> String {
        match self.progress {
            Some(p) => format!("{p}%"),
            None => "Not read".to_string(),
        }
    }
}

/// What the change wording compares: the last plotted game and this one.
struct Step<'a> {
    rank: &'a str,
    tier: usize,
    division: Option<u8>,
    pos: i32,
    progress: Option<i32>,
}

/// Whether two games sit on the same rank, judged only on what both read.
/// Different tiers, or two read divisions that differ, are a rank change.
/// Same tier with a division read on one side only is unknown: None.
fn same_rank(prev: &Step<'_>, cur: &Step<'_>) -> Option<bool> {
    if prev.tier != cur.tier {
        return Some(false);
    }
    match (prev.division, cur.division) {
        (Some(a), Some(b)) => Some(a == b),
        (None, None) => Some(true),
        _ => None,
    }
}

/// Change since the last plotted game, in words. A field the tracker did not
/// read on either game is never counted as a move: an unread division or
/// progress plots at a midpoint, and that midpoint is not a change.
fn change_words(prev: Option<&Step<'_>>, cur: &Step<'_>) -> String {
    let Some(prev) = prev else {
        return "First point".to_string();
    };
    let delta = cur.pos - prev.pos;
    match same_rank(prev, cur) {
        None => return "Same tier, division not read".to_string(),
        Some(true) if prev.progress.is_none() || cur.progress.is_none() => {
            return "Same rank, progress not read".to_string();
        }
        Some(true) => {
            return match delta {
                0 => "No change".to_string(),
                d => format!(
                    "{} {} {}",
                    if d > 0 { "Up" } else { "Down" },
                    d.abs(),
                    if d.abs() == 1 { "point" } else { "points" }
                ),
            };
        }
        Some(false) => {}
    }
    let direction = match delta {
        d if d > 0 => "Up to",
        d if d < 0 => "Down to",
        _ => "Now",
    };
    let from = if cur.tier == prev.tier {
        String::new()
    } else {
        match prev.progress {
            Some(p) => format!(", from {} at {p}%", prev.rank),
            None => format!(", from {}", prev.rank),
        }
    };
    format!("{direction} {}{from}", cur.rank)
}

/// The series as chart rows, oldest first.
fn chart_rows(series: &RankSeries) -> Vec<ChartRow> {
    let mut rows: Vec<ChartRow> = Vec::new();
    let mut last_placed: Option<usize> = None;
    for point in series.listed() {
        let shown = &point.shown;
        let tier = shown.tier.as_deref().and_then(tier_index);
        let mut row = ChartRow {
            match_id: point.match_id.clone(),
            date: long_date(point.played_at),
            date_short: short_date(point.played_at),
            rank: String::new(),
            placed: None,
            division: shown.division,
            progress: shown.progress,
            change: String::new(),
            rank_changed: false,
            derank: false,
            result: point.result,
            unknown_word: None,
        };
        match tier {
            Some(tier) => {
                let pos = ladder_position(tier, shown.division, shown.progress);
                row.rank = match shown.division {
                    Some(d) => format!("{} {d}", TIERS[tier]),
                    None => TIERS[tier].to_string(),
                };
                row.placed = Some(Placed { tier, pos });
                let prev = last_placed.map(|i| &rows[i]);
                let prev_step = prev.and_then(|p| {
                    p.placed.map(|placed| Step {
                        rank: &p.rank,
                        tier: placed.tier,
                        division: p.division,
                        pos: placed.pos,
                        progress: p.progress,
                    })
                });
                let cur = Step {
                    rank: &row.rank,
                    tier,
                    division: shown.division,
                    pos,
                    progress: shown.progress,
                };
                row.change = change_words(prev_step.as_ref(), &cur);
                if let Some(prev) = &prev_step {
                    row.rank_changed = same_rank(prev, &cur) == Some(false);
                    row.derank = row.rank_changed && pos < prev.pos;
                }
                last_placed = Some(rows.len());
            }
            None => {
                row.rank = shown
                    .rank_text()
                    .unwrap_or_else(|| "Rank not read".to_string());
                row.unknown_word = shown.tier.clone();
                row.change = if shown.tier.is_some() {
                    "Unplaced, rank not recognised".to_string()
                } else {
                    "Unplaced, rank not read".to_string()
                };
            }
        }
        rows.push(row);
    }
    rows
}

/// The distinct misread words of the unplaced rows, quoted: `"Emerlad"`.
fn quoted_words(unplaced: &[&ChartRow]) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for word in unplaced.iter().filter_map(|r| r.unknown_word.as_deref()) {
        let quoted = format!("\"{word}\"");
        if !words.contains(&quoted) {
            words.push(quoted);
        }
    }
    words
}

/// The visible note for games that are not plotted, or None when every game
/// is: `1 game not plotted: rank "Emerlad" not recognised`.
fn unplaced_note(rows: &[ChartRow]) -> Option<String> {
    let unplaced: Vec<&ChartRow> = rows.iter().filter(|r| r.placed.is_none()).collect();
    if unplaced.is_empty() {
        return None;
    }
    let n = unplaced.len();
    let games = if n == 1 { "game" } else { "games" };
    let words = quoted_words(&unplaced);
    let unread = unplaced.iter().any(|r| r.unknown_word.is_none());
    let reason = match (words.is_empty(), unread) {
        (true, _) => "rank not read".to_string(),
        (false, false) => format!("rank {} not recognised", words.join(", ")),
        (false, true) => format!("rank {} not recognised, or not read", words.join(", ")),
    };
    Some(format!("{n} {games} not plotted: {reason}"))
}

/// The summary sentences about unplaced games, or an empty string.
fn unplaced_summary(rows: &[ChartRow]) -> String {
    let unplaced: Vec<&ChartRow> = rows.iter().filter(|r| r.placed.is_none()).collect();
    match unplaced.as_slice() {
        [] => String::new(),
        [one] => {
            let why = match &one.unknown_word {
                Some(word) => {
                    format!(
                        "with rank \"{word}\" was not plotted because the rank was not recognised"
                    )
                }
                None => "was not plotted because the rank was not read".to_string(),
            };
            format!(
                " One game on {} {why}. That game is listed in the table as unplaced.",
                one.date
            )
        }
        many => format!(
            " {} games were not plotted because the rank was not recognised or not read. Those games are listed in the table as unplaced.",
            many.len()
        ),
    }
}

/// "A, B, and C".
fn join_clauses(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [head @ .., last] => format!("{}, and {last}", head.join(", ")),
    }
}

/// Which games the chart was given. The chart only sees the loaded games, so
/// its copy names that window rather than claiming the whole season.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RankWindow {
    /// The latest games, newest first from the API: Overview's recent form.
    Recent(usize),
    /// One page of match history, by page number from 1.
    Page(usize),
}

impl RankWindow {
    /// "in your last 10 games", "on page 3 of your match history".
    fn period(self) -> String {
        match self {
            RankWindow::Recent(1) => "in your last game".to_string(),
            RankWindow::Recent(n) => format!("in your last {n} games"),
            RankWindow::Page(1) => "on the first page of your match history".to_string(),
            RankWindow::Page(n) => format!("on page {n} of your match history"),
        }
    }

    /// Kicker of the first card. Only the newest games hold the current rank.
    fn now_kicker(self) -> &'static str {
        match self {
            RankWindow::Recent(_) | RankWindow::Page(1) => "Current rank",
            RankWindow::Page(_) => "Latest rank on this page",
        }
    }

    /// Kicker of the second card.
    fn range_kicker(self) -> &'static str {
        match self {
            RankWindow::Recent(_) => "Recent range",
            RankWindow::Page(_) => "Range on this page",
        }
    }
}

/// The visually hidden text alternative of the plot. `player` is the name
/// to use; without one the text speaks to the viewer ("You started at ...").
fn summary_text(player: Option<&str>, rows: &[ChartRow], window: RankWindow) -> String {
    let (intro, subject) = match player {
        Some(name) => (format!("Rank progress for {name}"), name.to_string()),
        None => ("Your rank progress".to_string(), "You".to_string()),
    };
    let placed: Vec<&ChartRow> = rows.iter().filter(|r| r.placed.is_some()).collect();
    let tail = unplaced_summary(rows);
    let (Some(first), Some(last)) = (placed.first(), placed.last()) else {
        return format!(
            "{intro}. There are no ranked games {}, so the chart is empty.{tail}",
            window.period()
        );
    };
    if placed.len() == 1 {
        let result = first
            .result
            .map(|r| format!(", {}", r.phrase()))
            .unwrap_or_default();
        return format!(
            "{intro}. One ranked game on {}: {}{result}. A single game is shown as a point, with no line.{tail}",
            first.date,
            first.rank_words()
        );
    }
    let tier_of = |row: &ChartRow| row.placed.map(|p| p.tier);
    let pos_of = |row: &ChartRow| row.placed.map(|p| p.pos);
    let mut parts = vec![format!("{subject} started at {}", first.rank_words())];
    // The first move into a higher tier: the case the old chart drew as a drop.
    let tier_up = (1..placed.len()).find(|&i| tier_of(placed[i]) > tier_of(placed[i - 1]));
    if let Some(i) = tier_up {
        // The game before the tier-up, said as a rise or a fall from the
        // start: a dip before the promotion is not a rise.
        let before = placed[i - 1];
        match (i > 1).then(|| pos_of(before).cmp(&pos_of(first))) {
            Some(std::cmp::Ordering::Greater) => {
                parts.push(format!("rose to {}", before.rank_words()));
            }
            Some(std::cmp::Ordering::Less) => {
                parts.push(format!("fell to {}", before.rank_words()));
            }
            _ => {}
        }
        if i < placed.len() - 1 {
            parts.push(format!(
                "then ranked up to {} (that move is up on this chart, not a drop)",
                placed[i].rank_words()
            ));
        }
    }
    let peak = (0..placed.len())
        .rev()
        .max_by_key(|&i| pos_of(placed[i]))
        .unwrap_or(0);
    if peak != 0 && peak != placed.len() - 1 && Some(peak) != tier_up {
        parts.push(format!("peaked at {}", placed[peak].rank));
    }
    let ending = if last.derank {
        format!(" after a derank on {}", last.date)
    } else if last.rank_changed {
        format!(" after a rank up on {}", last.date)
    } else {
        String::new()
    };
    parts.push(format!("ended at {}{ending}", last.rank_words()));
    format!(
        "{intro} from {} to {}. The chart plots absolute ladder position, so a rank up stays an up. {}.{tail}",
        first.date,
        last.date,
        join_clauses(&parts)
    )
}

/// Accessible name of the plot group.
fn group_label(placed: usize, unplaced: usize) -> String {
    let mut label = if placed == 1 {
        "Rank progress chart with one game. A table with the same data is below.".to_string()
    } else {
        "Rank progress chart. Arrow keys move between games. A table with the same data is below."
            .to_string()
    };
    match unplaced {
        0 => {}
        1 => label.push_str(" One game is not plotted."),
        n => label.push_str(&format!(" {n} games are not plotted.")),
    }
    label
}

/// Where a key moves the roving tab stop from game `current` of `len`:
/// arrows step, Home and End jump. None for any other key.
fn nav_target(key: &Key, current: usize, len: usize) -> Option<usize> {
    let last = len.checked_sub(1)?;
    match key {
        Key::ArrowRight | Key::ArrowDown => Some((current + 1).min(last)),
        Key::ArrowLeft | Key::ArrowUp => Some(current.saturating_sub(1)),
        Key::Home => Some(0),
        Key::End => Some(last),
        _ => None,
    }
}

/// What the pointer and focus do to a game's hit target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointerEvent {
    Enter,
    Leave,
    Focus,
    Blur,
}

/// The hovered game and the focused game, by match id. Hover wins while it
/// lasts; leaving falls back to the focused game, and blur clears that, so a
/// highlight never outlives the pointer or the focus that made it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Pointer {
    hovered: Option<String>,
    focused: Option<String>,
}

impl Pointer {
    fn apply(&mut self, event: PointerEvent, id: &str) {
        let slot = match event {
            PointerEvent::Enter | PointerEvent::Leave => &mut self.hovered,
            PointerEvent::Focus | PointerEvent::Blur => &mut self.focused,
        };
        match event {
            PointerEvent::Enter | PointerEvent::Focus => *slot = Some(id.to_string()),
            // Only the game that set it clears it: a late leave or blur from
            // another game keeps the current one.
            PointerEvent::Leave | PointerEvent::Blur => {
                if slot.as_deref() == Some(id) {
                    *slot = None;
                }
            }
        }
    }

    /// The game the readout and the highlight follow.
    fn active(&self) -> Option<String> {
        self.hovered.clone().or_else(|| self.focused.clone())
    }
}

/// One plotted game's hit target, built before the render so the loop body
/// is the keyed button itself.
struct Hit {
    at: usize,
    match_id: String,
    id: String,
    class: String,
    style: String,
    label: String,
}

/// Width of one game's slot on the x axis, as `--rank-slot` on the hit
/// layer. A hit target is never wider than its slot, so with many games on a
/// narrow plot the targets sit side by side instead of stacking (the table
/// is the full-size alternative). One game gets the whole plot.
fn hit_slot_style(n: usize) -> String {
    let slot = if n <= 1 {
        PLOT_W
    } else {
        PLOT_W / (n - 1) as f64
    };
    format!("--rank-slot:{:.2}%", slot / VIEW_W * 100.0)
}

/// Id of a game's hit target. Built from the series key, the match id and
/// the field name.
fn point_id(series_key: &str, match_id: &str) -> String {
    format!("rank-chart-{series_key}-{}-point", id_part(match_id))
}

/// "Showing Support, Role Queue: 13 games, 1 not plotted."
fn status_text(series: &RankSeries, rows: &[ChartRow]) -> String {
    let n = rows.len();
    let games = if n == 1 { "game" } else { "games" };
    let unplaced = rows.iter().filter(|r| r.placed.is_none()).count();
    let tail = if unplaced == 0 {
        String::new()
    } else {
        format!(", {unplaced} not plotted")
    };
    format!(
        "Showing {}: {n} {games} with rank progress{tail}.",
        series.label()
    )
}

// Blizzard's official career-profile rank emblems, kept in the repo byte for
// byte as the design sources ship them (`ux-spec/gen/rank-icons/`). They may
// be scaled in CSS and never recoloured, cropped or edited. A test holds each
// source file to its byte length.
/// The emblem folder, hash-free like the fonts. Note: `dx build --release`
/// repacks every PNG losslessly (file bytes change, decoded pixels do not),
/// for folder and image assets alike, as it does for the favicons.
static EMBLEM_DIR: manganis::Asset = asset!(
    "/assets/rank-icons",
    manganis::AssetOptions::folder().with_hash_suffix(false)
);

/// Emblem file stems, by ladder index.
const EMBLEM_FILES: [&str; 9] = [
    "bronze",
    "silver",
    "gold",
    "platinum",
    "emerald",
    "diamond",
    "master",
    "grandmaster",
    // The official `Rank_UltimateTier` file.
    "champion",
];

/// Served URL of a tier's emblem, by ladder index.
fn emblem_url(tier: usize) -> Option<String> {
    let stem = EMBLEM_FILES.get(tier)?;
    Some(format!("{EMBLEM_DIR}/rank-{stem}.png"))
}

/// The emblem slot: decorative, next to a typographic tier name. Without a
/// recognised tier the slot is empty and takes no width and no gap.
fn emblem(tier: Option<usize>, large: bool) -> Element {
    let size = if large { "32" } else { "20" };
    match tier.and_then(emblem_url) {
        Some(src) => rsx! {
            span {
                class: if large { "rank-chart-icon-slot has-icon is-lg" } else { "rank-chart-icon-slot has-icon" },
                aria_hidden: "true",
                img { src: "{src}", alt: "", width: "{size}", height: "{size}" }
            }
        },
        None => rsx! {
            span { class: "rank-chart-icon-slot", aria_hidden: "true" }
        },
    }
}

/// The 12px credit Blizzard's terms ask for, at the end of the section.
fn credit() -> Element {
    rsx! {
        p { class: "rank-chart-credit", "Rank emblems \u{a9} Blizzard Entertainment, Inc." }
    }
}

/// All rank chart rules. [`RankChart`] and [`RankChartLoading`] inject it.
pub(super) const RANK_CHART_CSS: &str = r#"
/* No outer margin: Overview and History both space their sections with a
   flex gap, so a margin here would double the gap under the chart. */
.rank-block {
  display: grid;
  gap: var(--space-stack);
  margin: 0;
  min-width: 0;
}
.rank-block > .ui-section-title { margin: 0; }
.rank-status {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: var(--space-stack);
}
.rank-status-card {
  box-sizing: border-box;
  min-width: 0;
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  padding: var(--space-card-pad);
}
.rank-status-card .rank-kicker {
  margin: 0 0 var(--space-2);
  color: var(--text-3);
  font-family: var(--font-head);
  font-size: var(--text-xs);
  font-weight: var(--font-weight-600);
  letter-spacing: var(--tracking-caps);
  text-transform: uppercase;
}
.rank-status-card .rank-now {
  margin: 0;
  font-family: var(--font-head);
  font-size: var(--text-xl);
  font-weight: var(--font-head-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
  overflow-wrap: anywhere;
}
.rank-status-card .rank-sub {
  margin: var(--space-2) 0 0;
  color: var(--text-3);
  font-size: var(--text-sm);
}
.rank-now-row,
.rank-range-line {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.rank-range-pair {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  white-space: nowrap;
}
.rank-chart {
  box-sizing: border-box;
  min-width: 0;
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  padding: var(--space-card-pad);
}
.rank-chart-plot {
  display: grid;
  grid-template-columns: max-content max-content minmax(0, 1fr);
  grid-template-rows: auto auto;
  column-gap: var(--space-2);
  row-gap: var(--space-2);
  width: 100%;
  min-width: 0;
}
.rank-chart-yaxis {
  position: relative;
  grid-column: 1;
  grid-row: 1;
  pointer-events: none;
}
/* In flow and invisible: holds the column to the longest tier name plus the
   emblem, so the plot does not move when the window does. */
.rank-chart-y-sizer {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  visibility: hidden;
  height: 0;
  overflow: hidden;
  white-space: nowrap;
}
.rank-chart-y-item {
  position: absolute;
  left: 0;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  transform: translateY(-50%);
}
/* Empty: no width and no gap. It opens only around an emblem. */
.rank-chart-icon-slot { display: none; box-sizing: border-box; }
.rank-chart-icon-slot.has-icon {
  display: block;
  width: 20px;
  height: 20px;
  flex: 0 0 20px;
}
.rank-chart-icon-slot.has-icon.is-lg {
  width: 32px;
  height: 32px;
  flex: 0 0 32px;
}
.rank-chart-icon-slot img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
  object-position: center;
}
.rank-chart-y-name {
  font-family: var(--font-head);
  font-size: var(--text-xs);
  font-weight: var(--font-weight-600);
  color: var(--text);
  white-space: nowrap;
  line-height: var(--leading-ui);
}
.rank-chart-divs {
  position: relative;
  grid-column: 2;
  grid-row: 1;
  pointer-events: none;
  min-width: 1ch;
}
.rank-chart-div-label {
  position: absolute;
  right: 0;
  transform: translateY(-50%);
  font-family: var(--font-body);
  font-size: var(--text-xs);
  color: var(--text-3);
  line-height: 1;
  font-variant-numeric: tabular-nums;
}
.rank-chart-canvas {
  position: relative;
  grid-column: 3;
  grid-row: 1;
  min-width: 0;
}
.rank-chart-canvas svg {
  display: block;
  width: 100%;
  height: 280px;
  overflow: visible;
}
.rank-chart-x {
  position: relative;
  grid-column: 3;
  grid-row: 2;
  min-height: calc(var(--text-xs) + var(--space-2));
}
.rank-chart-x-label {
  position: absolute;
  top: 0;
  font-family: var(--font-body);
  font-size: var(--text-xs);
  color: var(--text-3);
  white-space: nowrap;
}
.rank-chart-x-label.is-first { transform: none; }
.rank-chart-x-label.is-last { transform: translateX(-100%); }
.rank-chart-x-label.is-only { transform: translateX(-50%); }
.rank-chart-band { fill: var(--neutral-soft); }
/* Light: the mint line is under 3:1 on the soft band, so the bands take the
   card fill and the tier lines alone mark the tiers. */
[data-theme="light"] .rank-chart-band { fill: var(--card-bg); }
.rank-chart-band-alt { fill: transparent; }
.rank-chart-tier-line,
.rank-chart-axis {
  fill: none;
  stroke: var(--border-strong);
  stroke-width: 1;
  vector-effect: non-scaling-stroke;
}
.rank-chart-line {
  fill: none;
  stroke: var(--chart-2);
  stroke-width: 3;
  stroke-linejoin: round;
  stroke-linecap: round;
  vector-effect: non-scaling-stroke;
}
.rank-chart-marks,
.rank-chart-hits {
  position: absolute;
  inset: 0;
  pointer-events: none;
}
.rank-chart-guide {
  display: none;
  position: absolute;
  top: 0;
  bottom: 0;
  width: 0;
  border-left: 1px solid var(--border-strong);
  transform: translateX(-50%);
  pointer-events: none;
}
.rank-chart-guide.is-on { display: block; }
.rank-chart-mark-dot,
.rank-chart-mark-change {
  position: absolute;
  box-sizing: border-box;
  pointer-events: none;
}
.rank-chart-mark-dot {
  width: 10px;
  height: 10px;
  border-radius: var(--radius-full);
  background: var(--chart-2);
  border: 2px solid var(--card-bg);
  transform: translate(-50%, -50%);
}
.rank-chart-mark-change {
  width: 12px;
  height: 12px;
  background: var(--card-bg);
  border: 2px solid var(--chart-2);
  transform: translate(-50%, -50%) rotate(45deg);
}
.rank-chart-marks .is-active.rank-chart-mark-dot { width: 14px; height: 14px; }
.rank-chart-marks .is-active.rank-chart-mark-change { width: 16px; height: 16px; }
/* 44px, but never wider than the game's own slot on the x axis. */
.rank-chart-hit {
  position: absolute;
  box-sizing: border-box;
  width: min(var(--hit-target-min), var(--rank-slot, var(--hit-target-min)));
  height: var(--hit-target-min);
  min-height: var(--hit-target-min);
  margin: 0;
  padding: 0;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: inherit;
  transform: translate(-50%, -50%);
  pointer-events: auto;
  cursor: default;
}
.rank-chart-hit:focus-visible,
.rank-chart-hit.is-active {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.rank-chart-legend {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-4);
  margin: var(--space-3) 0 0;
  padding: 0;
  list-style: none;
  color: var(--text-3);
  font-size: var(--text-xs);
}
.rank-chart-legend li {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.rank-chart-legend .rank-chart-mark-dot,
.rank-chart-legend .rank-chart-mark-change {
  position: static;
  transform: none;
  flex: 0 0 auto;
}
.rank-chart-legend .rank-chart-mark-change { transform: rotate(45deg); }
/* The live region is always in the page. Closed, it is an empty block with
   no box; open, it is the readout card. */
.rank-chart-readout { margin: 0; font-size: var(--text-sm); color: var(--text); }
.rank-chart-readout.is-open {
  margin: var(--space-3) 0 0;
  padding: var(--space-3);
  background: var(--surface-raised);
  border: 1px solid var(--border-strong);
  border-radius: var(--card-radius);
}
/* One game: the readout is open by default on phone only. */
/* `> .readout-rank` outranks `.rank-chart-readout .readout-rank` (flex),
   so no line leaks onto desktop. */
.rank-chart-readout.is-phone-default > *,
.rank-chart-readout.is-phone-default > .readout-rank { display: none; }
.rank-chart-readout p { margin: 0; }
.rank-chart-readout .readout-date {
  color: var(--text-3);
  font-size: var(--text-xs);
  font-weight: var(--font-weight-600);
  text-transform: uppercase;
  letter-spacing: var(--tracking-caps);
  margin: 0 0 var(--space-1);
  white-space: nowrap;
}
.rank-chart-readout .readout-rank {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-family: var(--font-head);
  font-weight: var(--font-weight-600);
  margin: 0 0 var(--space-1);
}
.rank-cell {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.rank-chart-picker-status { margin: 0; }
.rank-chart .rank-chart-table-toggle { margin-top: var(--space-3); }
.rank-chart-table-wrap {
  margin-top: var(--space-3);
  min-width: 0;
  max-width: 100%;
}
.rank-chart-table {
  width: 100%;
  margin: 0;
  border-collapse: collapse;
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
}
.rank-chart-table[hidden] { display: none; }
.rank-chart-table th,
.rank-chart-table td {
  text-align: left;
  padding: var(--space-2);
  border-bottom: 1px solid var(--card-border);
  vertical-align: top;
}
.rank-chart-table th {
  font-family: var(--font-head);
  font-size: var(--text-xs);
  font-weight: var(--font-weight-600);
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: var(--tracking-caps);
}
.rank-chart-table th:nth-child(1),
.rank-chart-table td:nth-child(1),
.rank-chart-table th:nth-child(2),
.rank-chart-table td:nth-child(2),
.rank-chart-table th:nth-child(3),
.rank-chart-table td:nth-child(3) {
  width: 1%;
  white-space: nowrap;
}
.rank-chart-table .date,
.rank-chart-table .rank-cell { white-space: nowrap; }
.rank-chart-table td:nth-child(4) { overflow-wrap: break-word; }
.rank-chart-table tr.is-unplaced td { color: var(--text-3); }
.rank-chart-empty {
  margin: 0;
  color: var(--text-3);
  font-size: var(--text-sm);
}
.rank-chart-note {
  margin: var(--space-3) 0 0;
  color: var(--text-3);
  font-size: var(--text-sm);
}
.rank-chart-credit {
  margin: 0;
  color: var(--text-3);
  font-size: var(--text-xs);
}
.rank-skel { display: grid; gap: var(--space-3); }
.rank-skel-line,
.rank-skel-block {
  display: block;
  background: var(--neutral-soft);
  border-radius: var(--radius-control);
  min-height: var(--space-5);
}
.rank-skel-block { min-height: calc(var(--space-8) * 6); }
.rank-skel-line.is-short { width: 40%; }
.rank-skel-line.is-mid { width: 70%; }
@media (max-width: 820px) {
  .rank-status { grid-template-columns: minmax(0, 1fr); }
  .rank-chart-plot { grid-template-columns: max-content minmax(0, 1fr); }
  .rank-chart-divs { display: none; }
  .rank-chart-canvas,
  .rank-chart-x { grid-column: 2; }
  /* Near square; shorter for one game so its readout fits the first screen. */
  .rank-chart-canvas svg { height: auto; aspect-ratio: 376 / 360; }
  .rank-chart-plot.is-single .rank-chart-canvas svg { aspect-ratio: 376 / 200; }
  .rank-chart-readout.is-phone-default {
    margin: var(--space-3) 0 0;
    padding: var(--space-3);
    background: var(--surface-raised);
    border: 1px solid var(--border-strong);
    border-radius: var(--card-radius);
  }
  .rank-chart-readout.is-phone-default > * { display: block; }
  .rank-chart-readout.is-phone-default > .readout-rank { display: flex; }
  /* Stacked rows, three lines per game: date, rank and progress, change. */
  .rank-chart-table { font-size: var(--text-xs); }
  .rank-chart-table thead {
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
  .rank-chart-table tbody tr {
    display: block;
    padding: var(--space-3) 0;
    border-bottom: 1px solid var(--card-border);
  }
  .rank-chart-table th,
  .rank-chart-table td {
    display: block;
    width: auto;
    padding: var(--space-1) 0;
    border: 0;
    white-space: normal;
  }
  .rank-chart-table td:nth-child(1) {
    width: auto;
    color: var(--text-3);
    font-size: var(--text-xs);
    font-weight: var(--font-weight-600);
    text-transform: uppercase;
    letter-spacing: var(--tracking-caps);
  }
  .rank-chart-table td:nth-child(2),
  .rank-chart-table td:nth-child(3) {
    display: inline;
    width: auto;
    padding: var(--space-1) 0;
    white-space: nowrap;
    font-family: var(--font-head);
    font-weight: var(--font-weight-600);
  }
  .rank-chart-table td:nth-child(2) .rank-cell {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    vertical-align: middle;
  }
  .rank-chart-table td:nth-child(3) { vertical-align: middle; }
  .rank-chart-table td:nth-child(3)::before { content: " \00b7  "; }
  /* One game: the point is shown active next to its open readout. */
  .rank-chart-guide.is-phone-default { display: block; }
  .rank-chart-marks .is-phone-default.rank-chart-mark-dot { width: 14px; height: 14px; }
  .rank-chart-marks .is-phone-default.rank-chart-mark-change { width: 16px; height: 16px; }
  .rank-chart-hit.is-phone-default {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
}
/* Forced colours drop backgrounds and author colours: the line, the marks
   and the active ring use system colours so the plot still reads. */
@media (forced-colors: active) {
  .rank-chart-band { fill: Canvas; }
  .rank-chart-tier-line,
  .rank-chart-axis { stroke: GrayText; }
  .rank-chart-line { stroke: CanvasText; }
  .rank-chart-mark-dot {
    forced-color-adjust: none;
    background: CanvasText;
    border-color: Canvas;
  }
  .rank-chart-mark-change {
    forced-color-adjust: none;
    background: Canvas;
    border-color: CanvasText;
  }
  .rank-chart-guide { border-color: CanvasText; }
  .rank-chart-hit.is-active { outline-color: Highlight; }
}
"#;

const STYLE_ID: &str = "scuffed-css-pages-stats-rank-chart-css";
const TITLE_ID: &str = "rank-chart-title";
const SUMMARY_ID: &str = "rank-chart-summary";
const NOTE_ID: &str = "rank-chart-note";
const TABLE_ID: &str = "rank-chart-table";

/// The two cards above the plot: the latest rank and the range, both over
/// the games in `window`.
fn status_cards(rows: &[ChartRow], window: RankWindow) -> Element {
    let placed: Vec<&ChartRow> = rows.iter().filter(|r| r.placed.is_some()).collect();
    let tier = |row: &ChartRow| row.placed.map(|p| p.tier);
    let (Some(current), Some(low), Some(high)) = (
        placed.last().copied(),
        placed
            .iter()
            .rev()
            .min_by_key(|row| row.placed.map(|p| p.pos))
            .copied(),
        placed
            .iter()
            .max_by_key(|row| row.placed.map(|p| p.pos))
            .copied(),
    ) else {
        let period = window.period();
        return rsx! {
            div { class: "rank-status",
                div { class: "rank-status-card",
                    p { class: "rank-kicker", "{window.now_kicker()}" }
                    p { class: "rank-now", "No rank read" }
                    p { class: "rank-sub", "No game {period} has a rank. Play a ranked match to see it here." }
                }
                div { class: "rank-status-card",
                    p { class: "rank-kicker", "{window.range_kicker()}" }
                    p { class: "rank-now", "None" }
                    p { class: "rank-sub", "A range appears once a game {period} has a rank." }
                }
            }
        };
    };
    let range_period = window.period();
    let sub = match (current.progress, current.division) {
        (Some(p), _) if p < 0 => format!("{p}%, demotion protection"),
        (Some(p), Some(_)) => format!("{p}% into the division"),
        (Some(p), None) => format!("{p}% progress, division not read"),
        (None, _) => "Progress not read".to_string(),
    };
    rsx! {
        div { class: "rank-status",
            div { class: "rank-status-card",
                p { class: "rank-kicker", "{window.now_kicker()}" }
                p { class: "rank-now rank-now-row",
                    {emblem(tier(current), true)}
                    "{current.rank}"
                }
                p { class: "rank-sub", "{sub}" }
            }
            div { class: "rank-status-card",
                p { class: "rank-kicker", "{window.range_kicker()}" }
                if low.rank == high.rank {
                    p { class: "rank-now rank-now-row",
                        {emblem(tier(low), true)}
                        "{low.rank}"
                    }
                } else {
                    p { class: "rank-now rank-range-line",
                        span { class: "rank-range-pair",
                            {emblem(tier(low), true)}
                            "{low.rank}"
                        }
                        span { class: "rank-range-to", "to" }
                        span { class: "rank-range-pair",
                            {emblem(tier(high), true)}
                            "{high.rank}"
                        }
                    }
                }
                p { class: "rank-sub", "Lowest to highest rank {range_period}." }
            }
        }
    }
}

/// The decorative SVG: tier bands, tier boundaries, axis, division ticks and
/// the line. Marks and every label are HTML.
fn plot_svg(rows: &[ChartRow], ymin: i32, ymax: i32) -> Element {
    let y = |pos: i32| y_pct(f64::from(pos), ymin, ymax) / 100.0 * VIEW_H;
    let right = PAD_X + PLOT_W;
    let tiers = tier_bands(ymin, ymax);
    // (tier, class, top, height) of each band, and the y of each boundary.
    let bands: Vec<(usize, &'static str, String, String)> = tiers
        .iter()
        .map(|band| {
            let class = if band.tier % 2 == 1 {
                "rank-chart-band"
            } else {
                "rank-chart-band-alt"
            };
            (
                band.tier,
                class,
                format!("{:.1}", y(band.end)),
                format!("{:.1}", y(band.start) - y(band.end)),
            )
        })
        .collect();
    let boundaries: Vec<(usize, String)> = tiers
        .iter()
        .skip(1)
        .map(|band| (band.tier, format!("{:.1}", y(band.start))))
        .collect();
    let tick_x = PAD_X - TICK_W;
    let ticks: Vec<String> = (0..)
        .map(|i| ymin.div_euclid(DIVISION_UNITS) * DIVISION_UNITS + i * DIVISION_UNITS)
        .take_while(|pos| *pos <= ymax.min(CHAMPION_TOP))
        .filter(|pos| *pos >= ymin)
        .map(|pos| format!("{:.1}", y(pos)))
        .collect();
    let n = rows.len();
    let line: Vec<String> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| row.placed.map(|p| (i, p.pos)))
        .map(|(i, pos)| format!("{:.1},{:.1}", x_pct(i, n) / 100.0 * VIEW_W, y(pos)))
        .collect();
    let path = (line.len() >= 2).then(|| format!("M{}", line.join(" L")));
    rsx! {
        svg {
            view_box: "0 0 {VIEW_W} {VIEW_H}",
            preserve_aspect_ratio: "none",
            role: "presentation",
            "aria-hidden": "true",
            "focusable": "false",
            for (tier, class, top, height) in bands.iter() {
                rect {
                    key: "band-{tier}",
                    class: "{class}",
                    x: "{PAD_X}",
                    y: "{top}",
                    width: "{PLOT_W}",
                    height: "{height}",
                }
            }
            for (tier, at) in boundaries.iter() {
                line {
                    key: "tier-{tier}",
                    class: "rank-chart-tier-line",
                    x1: "{PAD_X}",
                    x2: "{right}",
                    y1: "{at}",
                    y2: "{at}",
                }
            }
            line { class: "rank-chart-axis", x1: "{PAD_X}", x2: "{PAD_X}", y1: "0", y2: "{VIEW_H}" }
            line { class: "rank-chart-axis", x1: "{PAD_X}", x2: "{right}", y1: "{VIEW_H}", y2: "{VIEW_H}" }
            for (i, tick) in ticks.iter().enumerate() {
                line {
                    key: "tick-{i}",
                    class: "rank-chart-axis",
                    x1: "{tick_x}",
                    x2: "{PAD_X}",
                    y1: "{tick}",
                    y2: "{tick}",
                }
            }
            if let Some(d) = path {
                path { class: "rank-chart-line", d: "{d}" }
            }
        }
    }
}

/// The toggle and the table with the same data as the plot.
fn data_table(rows: &[ChartRow], caption: &str, mut open: Signal<bool>) -> Element {
    let is_open = open();
    rsx! {
        button {
            class: "{btn_class(false)} rank-chart-table-toggle",
            r#type: "button",
            aria_expanded: if is_open { "true" } else { "false" },
            aria_controls: TABLE_ID,
            onclick: move |_| open.set(!is_open),
            if is_open { "Hide rank progress table" } else { "Show rank progress as a table" }
        }
        div { class: "rank-chart-table-wrap",
            table { class: "rank-chart-table", id: TABLE_ID, hidden: !is_open,
                caption { class: "visually-hidden", "{caption}" }
                thead {
                    tr {
                        th { scope: "col", "Date" }
                        th { scope: "col", "Rank" }
                        th { scope: "col", "Progress" }
                        th { scope: "col", "Change" }
                    }
                }
                tbody {
                    for row in rows.iter() {
                        tr {
                            key: "{row.match_id}",
                            class: if row.placed.is_none() { "is-unplaced" },
                            td { span { class: "date", "{row.date}" } }
                            td {
                                span { class: "rank-cell",
                                    {emblem(row.placed.map(|p| p.tier), false)}
                                    "{row.rank}"
                                }
                            }
                            td { {row.progress_cell()} }
                            td { {row.change_cell()} }
                        }
                    }
                }
            }
        }
    }
}

/// Rank progress for one (queue, role) at a time, as absolute ladder
/// position, with the latest rank and range cards, a readout per
/// game and a table with the same data. With no series it is the empty
/// state. `player` is the name the hidden summary and the table caption use;
/// without it they speak to the viewer.
#[component]
pub(super) fn RankChart(
    series: Vec<RankSeries>,
    window: RankWindow,
    #[props(default)] player: Option<String>,
) -> Element {
    let mut picked = use_signal(|| None::<String>);
    let mut pointer = use_signal(Pointer::default);
    let table_open = use_signal(|| false);
    let fallback = default_key(&series);
    let current_key = picked()
        .filter(|k| series.iter().any(|s| &s.key() == k))
        .or(fallback);
    let current = current_key
        .as_ref()
        .and_then(|k| series.iter().find(|s| &s.key() == k));

    let rows = current.map(chart_rows).unwrap_or_default();
    let n = rows.len();
    let key = current.map(RankSeries::key).unwrap_or_default();
    let placed: Vec<(usize, &ChartRow, Placed)> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| row.placed.map(|p| (i, row, p)))
        .collect();
    let status = match current {
        Some(series) => status_text(series, &rows),
        None => "No ranked games to show.".to_string(),
    };
    let summary = summary_text(player.as_deref(), &rows, window);
    let period = window.period();
    let note = unplaced_note(&rows);
    let caption = match player.as_deref() {
        Some(name) => format!("{CHART_TITLE} for {name}, one row per game"),
        None => format!("Your {}, one row per game", CHART_TITLE.to_lowercase()),
    };
    let described = if note.is_some() {
        format!("{SUMMARY_ID} {NOTE_ID}")
    } else {
        SUMMARY_ID.to_string()
    };
    let label = group_label(placed.len(), n - placed.len());

    let (ymin, ymax) = axis_window(
        placed.iter().map(|(_, _, p)| p.pos).min().unwrap_or(0),
        placed.iter().map(|(_, _, p)| p.pos).max().unwrap_or(0),
    );
    // One active game for hover, focus and tap: the hovered one, else the
    // focused one. Leaving or blurring clears it, so no highlight is left
    // behind. A game that left the series (another queue or role was
    // picked) is no longer active.
    let active_id = pointer()
        .active()
        .filter(|id| placed.iter().any(|(_, row, _)| &row.match_id == id));
    let active_at = placed
        .iter()
        .position(|(_, row, _)| Some(&row.match_id) == active_id.as_ref());
    let tab_stop = active_at.unwrap_or(0);
    let single = placed.len() == 1;
    let readout = match active_at {
        Some(at) => Some(placed[at].1),
        None if single => Some(placed[0].1),
        None => None,
    };
    let readout_class = match (active_at.is_some(), readout.is_some()) {
        (true, _) => "rank-chart-readout is-open",
        (false, true) => "rank-chart-readout is-phone-default",
        (false, false) => "rank-chart-readout",
    };
    let hit_ids: std::rc::Rc<Vec<String>> = std::rc::Rc::new(
        placed
            .iter()
            .map(|(_, row, _)| point_id(&key, &row.match_id))
            .collect(),
    );
    let y_items: Vec<(usize, String)> = tier_bands(ymin, ymax)
        .into_iter()
        .map(|band| {
            let mid = f64::from(band.start + band.end) / 2.0;
            (band.tier, format!("top:{:.2}%", y_pct(mid, ymin, ymax)))
        })
        .collect();
    let div_items: Vec<(i32, String)> = division_labels(ymin, ymax)
        .into_iter()
        .map(|(division, mid)| (division, format!("top:{:.2}%", y_pct(mid, ymin, ymax))))
        .collect();
    // Where each plotted game sits on the canvas.
    let spots: Vec<String> = placed
        .iter()
        .map(|(i, _, p)| {
            format!(
                "left:{:.2}%;top:{:.2}%",
                x_pct(*i, n),
                y_pct(f64::from(p.pos), ymin, ymax)
            )
        })
        .collect();
    // One game, nothing picked: phone shows that point as active (CSS only).
    let phone_default = active_at.is_none() && single;
    let guide_style = active_at
        .or(phone_default.then_some(0))
        .map(|at| format!("left:{:.2}%", x_pct(placed[at].0, n)));
    let on_class = |at: usize| {
        if Some(at) == active_at {
            " is-active"
        } else if phone_default {
            " is-phone-default"
        } else {
            ""
        }
    };
    let x_slots: Vec<(usize, &'static str)> = match n {
        0 => Vec::new(),
        1 => vec![(0, "rank-chart-x-label is-only")],
        _ => vec![
            (0, "rank-chart-x-label is-first"),
            (n - 1, "rank-chart-x-label is-last"),
        ],
    };
    // (match id, class, style, date) per x label, so the loop body is the
    // keyed element itself.
    let x_labels: Vec<(String, &'static str, String, String)> = x_slots
        .into_iter()
        .map(|(i, class)| {
            let row = &rows[i];
            (
                row.match_id.clone(),
                class,
                format!("left:{:.2}%", x_pct(i, n)),
                row.date_short.clone(),
            )
        })
        .collect();
    // (key, label, pressed) per queue and role chip.
    let picks: Vec<(String, String, bool)> = series
        .iter()
        .map(|s| {
            let k = s.key();
            let pressed = Some(&k) == current_key.as_ref();
            (k, s.label(), pressed)
        })
        .collect();
    let hits: Vec<Hit> = placed
        .iter()
        .enumerate()
        .map(|(at, (_, row, _))| Hit {
            at,
            match_id: row.match_id.clone(),
            id: hit_ids[at].clone(),
            class: format!("rank-chart-hit{}", on_class(at)),
            style: spots[at].clone(),
            label: row.point_label(),
        })
        .collect();
    let hit_slot = hit_slot_style(n);

    rsx! {
        crate::styles::StyleOnce { id: STYLE_ID, css: RANK_CHART_CSS }
        section { class: "rank-block", aria_labelledby: TITLE_ID,
            h2 { class: "ui-section-title", id: TITLE_ID, "{CHART_TITLE}" }
            if series.len() > 1 {
                ChipRow { label: "Queue and role".to_string(),
                    for (k, label, pressed) in picks {
                        ChipButton {
                            key: "{k}",
                            id: format!("rank-chart-{k}-pick"),
                            class: "chip".to_string(),
                            label,
                            pressed,
                            onclick: {
                                let pick = k.clone();
                                move |_| picked.set(Some(pick.clone()))
                            },
                        }
                    }
                }
            }
            p { class: "visually-hidden rank-chart-picker-status", role: "status", "{status}" }
            {status_cards(&rows, window)}
            div { class: "rank-chart",
                p { class: "visually-hidden", id: SUMMARY_ID, "{summary}" }
                if placed.is_empty() {
                    p { class: "rank-chart-empty",
                        if n == 0 {
                            "No ranked games {period}. Play a ranked match and it will show up here."
                        } else {
                            "No game {period} could be placed on the ladder. The games are listed in the table."
                        }
                    }
                } else {
                    div {
                        class: if single { "rank-chart-plot is-single" } else { "rank-chart-plot" },
                        role: "group",
                        aria_label: "{label}",
                        aria_describedby: "{described}",
                        div { class: "rank-chart-yaxis", aria_hidden: "true",
                            div { class: "rank-chart-y-sizer",
                                {emblem(Some(7), false)}
                                span { class: "rank-chart-y-name", {TIERS[7]} }
                            }
                            for (tier, style) in y_items.iter() {
                                div {
                                    key: "{tier}",
                                    class: "rank-chart-y-item",
                                    style: "{style}",
                                    {emblem(Some(*tier), false)}
                                    span { class: "rank-chart-y-name", {TIERS[*tier]} }
                                }
                            }
                        }
                        div { class: "rank-chart-divs", aria_hidden: "true",
                            for (i, (division, style)) in div_items.iter().enumerate() {
                                span {
                                    key: "{i}",
                                    class: "rank-chart-div-label",
                                    style: "{style}",
                                    "{division}"
                                }
                            }
                        }
                        div { class: "rank-chart-canvas",
                            {plot_svg(&rows, ymin, ymax)}
                            div {
                                class: match (active_at.is_some(), phone_default) {
                                    (true, _) => "rank-chart-guide is-on",
                                    (false, true) => "rank-chart-guide is-phone-default",
                                    (false, false) => "rank-chart-guide",
                                },
                                aria_hidden: "true",
                                style: guide_style,
                            }
                            div { class: "rank-chart-marks", aria_hidden: "true",
                                for (at, (_, row, _)) in placed.iter().enumerate() {
                                    span {
                                        key: "{row.match_id}",
                                        class: format!(
                                            "{}{}",
                                            if row.rank_changed { "rank-chart-mark-change" } else { "rank-chart-mark-dot" },
                                            on_class(at),
                                        ),
                                        style: "{spots[at]}",
                                    }
                                }
                            }
                            div { class: "rank-chart-hits", style: "{hit_slot}",
                                for hit in hits {
                                    button {
                                        key: "{hit.match_id}",
                                        class: "{hit.class}",
                                        r#type: "button",
                                        id: "{hit.id}",
                                        style: "{hit.style}",
                                        tabindex: if hit.at == tab_stop { "0" } else { "-1" },
                                        aria_label: "{hit.label}",
                                        onmouseenter: {
                                            let id = hit.match_id.clone();
                                            move |_| pointer.with_mut(|p| p.apply(PointerEvent::Enter, &id))
                                        },
                                        onmouseleave: {
                                            let id = hit.match_id.clone();
                                            move |_| pointer.with_mut(|p| p.apply(PointerEvent::Leave, &id))
                                        },
                                        onfocus: {
                                            let id = hit.match_id.clone();
                                            move |_| pointer.with_mut(|p| p.apply(PointerEvent::Focus, &id))
                                        },
                                        onblur: {
                                            let id = hit.match_id.clone();
                                            move |_| pointer.with_mut(|p| p.apply(PointerEvent::Blur, &id))
                                        },
                                        onclick: {
                                            let id = hit.id.clone();
                                            move |_| focus_element_by_id(&id)
                                        },
                                        onkeydown: {
                                            let ids = hit_ids.clone();
                                            let at = hit.at;
                                            move |e: KeyboardEvent| {
                                                if let Some(next) = nav_target(&e.key(), at, ids.len()) {
                                                    e.prevent_default();
                                                    focus_element_by_id(&ids[next]);
                                                }
                                            }
                                        },
                                    }
                                }
                            }
                        }
                        div { class: "rank-chart-x",
                            for (match_id , class , left , date) in x_labels {
                                span { key: "{match_id}", class: "{class}", style: "{left}", "{date}" }
                            }
                        }
                    }
                    ul { class: "rank-chart-legend",
                        li {
                            span { class: "rank-chart-mark-dot", aria_hidden: "true" }
                            "Game"
                        }
                        li {
                            span { class: "rank-chart-mark-change", aria_hidden: "true" }
                            "Rank change"
                        }
                    }
                }
                div { class: "{readout_class}", role: "status", aria_live: "polite",
                    if let Some(row) = readout {
                        p { class: "readout-date", span { class: "date", "{row.date}" } }
                        p { class: "readout-rank",
                            {emblem(row.placed.map(|p| p.tier), false)}
                            {row.rank_line()}
                        }
                        p { "{row.change}" }
                        if let Some(result) = row.result {
                            p { {result.word()} }
                        }
                    }
                }
                if let Some(note) = note {
                    p { class: "rank-chart-note", id: NOTE_ID, "{note}" }
                }
                if n > 0 {
                    {data_table(&rows, &caption, table_open)}
                }
            }
            {credit()}
        }
    }
}

/// The loading state of the rank section: skeleton cards and plot, inside
/// the same heading. The section is `aria-busy` and says so in words.
#[component]
pub(super) fn RankChartLoading() -> Element {
    rsx! {
        crate::styles::StyleOnce { id: STYLE_ID, css: RANK_CHART_CSS }
        section { class: "rank-block", aria_labelledby: TITLE_ID, aria_busy: "true",
            h2 { class: "ui-section-title", id: TITLE_ID, "{CHART_TITLE}" }
            p { class: "visually-hidden", role: "status", "Loading rank progress." }
            div { class: "rank-status", aria_hidden: "true",
                div { class: "rank-status-card rank-skel",
                    span { class: "rank-skel-line is-short" }
                    span { class: "rank-skel-line is-mid" }
                }
                div { class: "rank-status-card rank-skel",
                    span { class: "rank-skel-line is-short" }
                    span { class: "rank-skel-line is-mid" }
                }
            }
            div { class: "rank-chart rank-skel", aria_hidden: "true",
                span { class: "rank-skel-line is-mid" }
                span { class: "rank-skel-block" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{css_rule, has_decl};
    use chrono::TimeZone;

    fn rank(json: &str) -> Option<MatchRank> {
        parse_rank(&serde_json::from_str::<Value>(json).expect("json"))
    }

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, 12, 0, 0).unwrap()
    }

    const FULL: &str = r#"{
        "role": { "value": "Support", "confidence": 0.9 },
        "role_rank": { "value": "DIAMOND", "confidence": 0.95 },
        "division": { "value": 2, "confidence": 0.875 },
        "progress_pct": { "value": 45, "confidence": 0.8 },
        "progress_change": { "value": 23, "confidence": 0.75 },
        "change_tags": { "value": ["WIN"], "confidence": 0.6 },
        "higher_than_pct": { "value": 47, "confidence": 1.0 },
        "hero_sr": { "value": 3125, "confidence": 0.25 },
        "hero_sr_change": { "value": -17, "confidence": 0.0 },
        "next_mode_hint": { "value": null, "confidence": 0.0 },
        "queue_title": { "value": "ROLE  QUEUE", "confidence": 0.8 },
        "rank_recognizer": "rank-v1"
    }"#;

    #[test]
    fn empty_null_and_all_null_objects_are_no_rank() {
        for json in [
            "null",
            "{}",
            "7",
            "[]",
            r#""garbled""#,
            r#""{}""#,
            r#"{"role": {"value": null, "confidence": null}, "division": {"value": null, "confidence": 0.9}}"#,
            r#"{"rank_recognizer": "rank-v1"}"#,
            r#"{"division": {"value": 9, "confidence": 0.9}}"#,
        ] {
            assert_eq!(rank(json), None, "{json}");
        }
    }

    #[test]
    fn row_field_is_lenient() {
        #[derive(Deserialize)]
        struct Row {
            #[serde(default, deserialize_with = "lenient")]
            rank: Option<MatchRank>,
        }
        for json in [
            r#"{}"#,
            r#"{"rank": null}"#,
            r#"{"rank": {}}"#,
            r#"{"rank": 3}"#,
        ] {
            let row: Row = serde_json::from_str(json).expect(json);
            assert!(row.rank.is_none(), "{json}");
        }
        let row: Row = serde_json::from_str(&format!(r#"{{"rank": {FULL}}}"#)).expect("full");
        assert!(row.rank.is_some());
        let as_string = serde_json::to_string(FULL).unwrap();
        let row: Row = serde_json::from_str(&format!(r#"{{"rank": {as_string}}}"#)).expect("str");
        assert!(row.rank.is_some());
    }

    #[test]
    fn full_rank_shows_every_value() {
        let shown = RankShown::from_rank(&rank(FULL).unwrap()).unwrap();
        assert_eq!(
            shown,
            RankShown {
                role: Some(RankRole::Support),
                tier: Some("Diamond".into()),
                division: Some(2),
                progress: Some(45),
                change: Some(23),
                queue: Some("ROLE QUEUE".into()),
            }
        );
        assert_eq!(shown.rank_text().as_deref(), Some("Diamond 2"));
        assert_eq!(shown.badge_parts(), ["Diamond 2", "45%", "+23%"]);
        assert_eq!(shown.queue_label().as_deref(), Some("Role Queue"));
        assert_eq!(
            shown.sentence(),
            "Rank: Support, Role Queue, Diamond 2, 45% progress, +23% this game"
        );
    }

    #[test]
    fn any_non_null_value_shows_whatever_its_confidence() {
        // Word fields pass the reader at about 0.42, so a low or missing
        // confidence still shows. Out of range numbers do not.
        let r = rank(
            r#"{
            "role_rank": { "value": "Gold", "confidence": 0.42 },
            "division": { "value": 3, "confidence": 0.7 },
            "progress_pct": { "value": -15, "confidence": 0.9 },
            "progress_change": { "value": -8 },
            "queue_title": { "value": null, "confidence": 0.0 }
        }"#,
        )
        .expect("has values");
        let shown = RankShown::from_rank(&r).unwrap();
        assert_eq!(shown.tier.as_deref(), Some("Gold"), "low confidence word");
        assert_eq!(shown.division, Some(3));
        assert_eq!(shown.progress, Some(-15), "demotion protection is a value");
        assert_eq!(shown.change, Some(-8), "missing confidence still shows");
        assert_eq!(shown.queue, None);
        assert_eq!(shown.badge_parts(), ["Gold 3", "-15%", "-8%"]);
        assert_eq!(shown.queue_label(), None);
        assert_eq!(shown.role_label(), ROLE_UNKNOWN);
        assert_eq!(
            shown.sentence(),
            "Rank: Role unknown, Gold 3, -15% progress, -8% this game"
        );
    }

    #[test]
    fn null_values_and_hidden_fields_show_nothing() {
        // Only fields the badge does not show, or a value out of range.
        for json in [
            r#"{"hero_sr": {"value": 3125, "confidence": 0.9}}"#,
            r#"{"progress_pct": {"value": 140, "confidence": 0.9}}"#,
            r#"{"progress_pct": {"value": -101, "confidence": 0.9}}"#,
        ] {
            let r = rank(json).expect(json);
            assert_eq!(RankShown::from_rank(&r), None, "{json}");
            assert_eq!(render_badge("m1", Some(r)), "", "{json}");
        }
    }

    #[test]
    fn next_mode_hint_alone_is_no_rank_and_never_the_queue() {
        let hint = r#"{"next_mode_hint": {"value": "OPEN QUEUE", "confidence": 0.9}}"#;
        assert_eq!(rank(hint), None);
        let r = rank(
            r#"{"next_mode_hint": {"value": "OPEN QUEUE", "confidence": 0.9},
                "role": {"value": "Tank", "confidence": 0.9},
                "progress_pct": {"value": 10, "confidence": 0.9}}"#,
        )
        .unwrap();
        let shown = RankShown::from_rank(&r).unwrap();
        assert_eq!(shown.queue, None);
        assert_eq!(shown.context(), "Tank");
    }

    #[test]
    fn open_role_with_open_queue_title_reads_once() {
        assert_eq!(
            context_label(Some(RankRole::Open), Some("OPEN QUEUE")),
            "Open queue"
        );
        assert_eq!(context_label(Some(RankRole::Open), None), "Open queue");
        assert_eq!(
            context_label(Some(RankRole::Support), Some("ROLE QUEUE")),
            "Support, Role Queue"
        );
        assert_eq!(context_label(None, None), ROLE_UNKNOWN);
    }

    #[test]
    fn change_text_signs() {
        assert_eq!(change_text(5), "+5%");
        assert_eq!(change_text(-5), "-5%");
        assert_eq!(change_text(0), "0%");
    }

    fn game(queue: Option<&str>, role: &str, progress: i32) -> MatchRank {
        let queue = queue
            .map(|q| format!(r#""queue_title": {{"value": "{q}", "confidence": 0.9}},"#))
            .unwrap_or_default();
        rank(&format!(
            r#"{{ {queue}
            "role": {{"value": "{role}", "confidence": 0.9}},
            "role_rank": {{"value": "Gold", "confidence": 0.9}},
            "division": {{"value": 3, "confidence": 0.9}},
            "progress_pct": {{"value": {progress}, "confidence": 0.9}} }}"#
        ))
        .unwrap()
    }

    #[test]
    fn buckets_by_queue_and_role_with_no_title_last() {
        let ranks = [
            game(Some("ROLE QUEUE"), "Support", 40),
            game(None, "Tank", 20),
            game(Some("role queue"), "Support", 30),
            game(Some("ROLE QUEUE"), "Tank", 50),
            game(None, "Tank", 10),
            game(Some("OPEN QUEUE"), "Open", 70),
        ];
        let games: Vec<(String, DateTime<Utc>)> = (0..ranks.len())
            .map(|i| (format!("m{i}"), at(10 - i as u32)))
            .collect();
        let series = rank_series(
            games
                .iter()
                .zip(ranks.iter())
                .map(|((id, t), r)| (id.as_str(), *t, Some(r)))
                .chain(std::iter::once(("none", at(1), None))),
        );
        let labels: Vec<String> = series.iter().map(RankSeries::label).collect();
        assert_eq!(
            labels,
            [
                "Open queue",
                "Tank, Role Queue",
                "Support, Role Queue",
                "Tank"
            ]
        );
        let support = &series[2];
        // Oldest first: m2 (day 8) before m0 (day 10).
        let ids: Vec<&str> = support.points.iter().map(|p| p.match_id.as_str()).collect();
        assert_eq!(ids, ["m2", "m0"]);
        let untitled = &series[3];
        assert_eq!(untitled.queue, None);
        assert_eq!(untitled.points.len(), 2, "null titles share a bucket");
        assert_eq!(untitled.key(), "q-unknown-r-tank");
        assert_eq!(series[1].key(), "q-role_queue-r-tank");
        // Default pick is the series with the latest game (m0, day 10).
        assert_eq!(
            default_key(&series).as_deref(),
            Some("q-role_queue-r-support")
        );
    }

    #[test]
    fn all_null_titles_group_by_role_alone() {
        // What alpha.3 sends: no queue title on any game.
        let a = game(None, "Support", 40);
        let b = game(None, "Support", 50);
        let c = game(None, "Damage", 30);
        let series = rank_series([
            ("m1", at(1), Some(&a)),
            ("m2", at(2), Some(&b)),
            ("m3", at(3), Some(&c)),
        ]);
        let labels: Vec<String> = series.iter().map(RankSeries::label).collect();
        assert_eq!(labels, ["Damage", "Support"]);
        assert_eq!(series[1].points.len(), 2);
    }

    #[test]
    fn no_rank_data_means_no_series() {
        let series = rank_series([("a", at(1), None), ("b", at(2), None)]);
        assert!(series.is_empty());
        assert!(chart_series(series).is_empty());
    }

    #[test]
    fn chart_drops_series_with_nothing_to_list() {
        // A role alone has no tier, division or progress: nothing to plot
        // and nothing to list.
        let r = rank(
            r#"{"role": {"value": "Tank", "confidence": 0.9},
            "progress_change": {"value": 5, "confidence": 0.9}}"#,
        )
        .unwrap();
        let series = rank_series([("a", at(1), Some(&r))]);
        assert_eq!(series.len(), 1);
        assert!(chart_series(series).is_empty());
        // A tier alone is plotted at the tier midpoint, so its series stays.
        let r = rank(r#"{"role_rank": {"value": "Gold", "confidence": 0.9}}"#).unwrap();
        let series = chart_series(rank_series([("a", at(1), Some(&r))]));
        assert_eq!(series.len(), 1);
        assert_eq!(
            chart_rows(&series[0])[0].placed,
            Some(Placed { tier: 2, pos: 1250 })
        );
    }

    #[test]
    fn negative_progress_shows_on_badge_chart_and_table() {
        let r = rank(
            r#"{"role": {"value": "Support", "confidence": 0.9},
                "role_rank": {"value": "Gold", "confidence": 0.9},
                "division": {"value": 3, "confidence": 0.9},
                "progress_pct": {"value": -15, "confidence": 0.9},
                "progress_change": {"value": -20, "confidence": 0.9}}"#,
        )
        .unwrap();
        let shown = RankShown::from_rank(&r).unwrap();
        assert_eq!(shown.progress, Some(-15));
        assert_eq!(shown.badge_parts(), ["Gold 3", "-15%", "-20%"]);
        let html = render_badge("m1", Some(r.clone()));
        assert!(html.contains(">-15%</span>"), "{html}");
        assert!(
            html.contains("Gold 3, -15% progress, -20% this game"),
            "{html}"
        );
        let html = render_chart(chart_series(rank_series([("m1", at(2), Some(&r))])));
        assert!(html.contains("1 game with rank progress"), "{html}");
        assert!(html.contains("-15%, demotion protection"), "{html}");
        let row = &chart_rows(&series_of(&[(2, "Gold", Some(3), Some(-15), None)]))[0];
        assert_eq!(
            row.placed,
            Some(Placed { tier: 2, pos: 1185 }),
            "below the division floor"
        );
        assert!(html.contains("<td>-15%</td>"), "{html}");
    }

    fn render_badge(id: &str, r: Option<MatchRank>) -> String {
        #[derive(Props, Clone, PartialEq)]
        struct P {
            id: String,
            r: Option<MatchRank>,
        }
        let mut dom = VirtualDom::new_with_props(
            |p: P| rank_badge_line(&p.id, p.r.as_ref()),
            P {
                id: id.to_string(),
                r,
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn render_detail(id: &str, r: Option<MatchRank>) -> String {
        #[derive(Props, Clone, PartialEq)]
        struct P {
            id: String,
            r: Option<MatchRank>,
        }
        let mut dom = VirtualDom::new_with_props(
            |p: P| rank_detail(&p.id, p.r.as_ref()),
            P {
                id: id.to_string(),
                r,
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn detail_hides_a_null_queue_and_shows_a_read_one() {
        let html = render_detail("m1", Some(game(None, "Support", 40)));
        assert!(html.contains(r#"class="game-rank-queue""#), "{html}");
        assert!(html.contains(">Support</span>"), "{html}");
        assert!(!html.to_lowercase().contains("queue unknown"), "{html}");
        assert!(html.contains("Rank: Support, Gold 3"), "{html}");
        let html = render_detail("m1", Some(game(Some("ROLE QUEUE"), "Support", 40)));
        assert!(html.contains(">Support, Role Queue</span>"), "{html}");
        let html = render_detail("m1", Some(game(Some("OPEN QUEUE"), "Open", 40)));
        assert!(!html.contains("Open queue, Open Queue"), "{html}");
        assert!(html.contains(">Open queue</span>"), "{html}");
        assert_eq!(render_detail("m1", None), "");
    }

    #[test]
    fn badge_renders_text_and_record_based_id() {
        let html = render_badge("personal_match:Abc 1", rank(FULL));
        assert!(
            html.contains(r#"id="rank-personal_match_abc_1-badge""#),
            "{html}"
        );
        assert!(html.contains("Diamond 2"), "{html}");
        assert!(html.contains("45%"), "{html}");
        assert!(html.contains("+23%"), "{html}");
        assert!(html.contains("Rank: Support, Role Queue"), "{html}");
        assert!(!html.contains("<button"), "{html}");
        assert!(!html.contains("<a"), "{html}");
        assert_eq!(render_badge("m1", None), "");
    }

    #[test]
    fn chart_has_status_named_controls_and_a_table() {
        let a = game(Some("ROLE QUEUE"), "Support", 40);
        let b = game(None, "Tank", 20);
        let series = chart_series(rank_series([
            ("m1", at(2), Some(&a)),
            ("m2", at(1), Some(&b)),
        ]));
        let html = render_chart(series);
        // The series line and the readout, both always rendered.
        assert_eq!(html.matches(r#"role="status""#).count(), 2, "{html}");
        assert!(
            html.contains(r#"role="status">Showing Support, Role Queue: 1 game"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<div class="chip-row" role="group" aria-label="Queue and role">"#),
            "{html}"
        );
        assert!(
            html.contains(r#"id="rank-chart-q-unknown-r-tank-pick""#),
            "chip id from the series: {html}"
        );
        assert!(html.contains(">Tank</button>"), "{html}");
        assert!(!html.contains("Queue unknown"), "{html}");
        assert!(html.contains(r#"aria-pressed="true""#), "{html}");
        assert!(
            !html.contains(r#"role="img""#),
            "the svg is decorative: {html}"
        );
        assert!(html.contains("<table"), "{html}");
        assert!(html.contains(r#"id="rank-chart-table""#), "{html}");
        assert!(
            html.contains(r#"aria-controls="rank-chart-table""#),
            "{html}"
        );
    }

    #[test]
    fn chart_with_one_series_has_no_picker_but_keeps_status() {
        let a = game(Some("ROLE QUEUE"), "Support", 40);
        let html = render_chart(chart_series(rank_series([("m1", at(2), Some(&a))])));
        assert!(!html.contains("aria-pressed"), "{html}");
        assert!(!html.contains("chip-row"), "{html}");
        assert!(
            html.contains(r#"role="status">Showing Support, Role Queue: 1 game"#),
            "{html}"
        );
    }

    // -- Chart --

    /// One game as the frames list it: (day, tier word, division, progress,
    /// outcome word).
    type Game = (
        u32,
        &'static str,
        Option<u8>,
        Option<i32>,
        Option<&'static str>,
    );

    /// The populated frame: a climb through Platinum, the rank up from
    /// Platinum 1 at 96% to Emerald 5 at 30%, a peak and a derank.
    const POPULATED: [Game; 13] = [
        (2, "Platinum", Some(3), Some(40), None),
        (3, "Platinum", Some(3), Some(62), Some("win")),
        (4, "Platinum", Some(2), Some(15), Some("win")),
        (5, "Platinum", Some(2), Some(48), Some("win")),
        (6, "Platinum", Some(2), Some(20), Some("loss")),
        (8, "Platinum", Some(2), Some(71), Some("win")),
        (9, "Platinum", Some(1), Some(10), Some("win")),
        (10, "Platinum", Some(1), Some(55), Some("win")),
        (11, "Platinum", Some(1), Some(96), Some("win")),
        (12, "Emerald", Some(5), Some(30), Some("win")),
        (13, "Emerald", Some(5), Some(58), Some("win")),
        (14, "Emerald", Some(4), Some(12), Some("win")),
        (15, "Emerald", Some(5), Some(40), Some("loss")),
    ];

    /// The unplaced frame: the same climb with one misread game.
    fn unplaced_games() -> Vec<Game> {
        let mut games = POPULATED[..10].to_vec();
        games.extend([
            (13, "Emerlad", Some(5), Some(44), Some("win")),
            (14, "Emerald", Some(5), Some(58), Some("win")),
            (15, "Emerald", Some(4), Some(12), Some("win")),
            (16, "Emerald", Some(5), Some(40), Some("loss")),
        ]);
        games
    }

    fn series_of(games: &[Game]) -> RankSeries {
        RankSeries {
            queue: None,
            role: Some(RankRole::Support),
            points: games
                .iter()
                .map(|(day, tier, division, progress, outcome)| RankPoint {
                    match_id: format!("personal_match:g{day}"),
                    played_at: at(*day),
                    shown: RankShown {
                        role: Some(RankRole::Support),
                        tier: Some((*tier).to_string()),
                        division: *division,
                        progress: *progress,
                        ..RankShown::default()
                    },
                    result: outcome.and_then(GameResult::from_outcome),
                })
                .collect(),
        }
    }

    fn render_chart(series: Vec<RankSeries>) -> String {
        render_chart_for(series, None)
    }

    fn render_chart_for(series: Vec<RankSeries>, player: Option<&str>) -> String {
        render_chart_in(series, player, RankWindow::Page(1))
    }

    fn render_chart_in(series: Vec<RankSeries>, player: Option<&str>, window: RankWindow) -> String {
        let mut dom = VirtualDom::new_with_props(
            RankChart,
            RankChartProps {
                series,
                window,
                player: player.map(str::to_string),
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// Every opening tag of one class, e.g. each hit target.
    fn tags_with_class<'a>(html: &'a str, class: &str) -> Vec<&'a str> {
        html.split('<')
            .filter(|tag| {
                tag.split_once("class=\"")
                    .and_then(|(_, rest)| rest.split_once('"'))
                    .is_some_and(|(classes, _)| classes.split(' ').any(|c| c == class))
            })
            .collect()
    }

    #[test]
    fn ladder_position_follows_the_spec_examples() {
        let tier = |word: &str| tier_index(word).expect(word);
        assert_eq!(ladder_position(tier("Bronze"), Some(5), Some(0)), 0);
        assert_eq!(ladder_position(tier("Platinum"), Some(1), Some(96)), 1996);
        assert_eq!(ladder_position(tier("Emerald"), Some(5), Some(30)), 2030);
        assert_eq!(
            ladder_position(tier("Champion"), Some(1), Some(100)),
            CHAMPION_TOP
        );
        assert_eq!(CHAMPION_TOP, 4500);
        assert_eq!(SCALE_CEILING, 5000, "one reserved band above Champion 1");
        // The rank up is +34: the line rises.
        let up = ladder_position(tier("Emerald"), Some(5), Some(30))
            - ladder_position(tier("Platinum"), Some(1), Some(96));
        assert_eq!(up, 34);
    }

    #[test]
    fn missing_division_plots_at_the_tier_midpoint() {
        let gold = tier_index("gold").expect("any case");
        // Not tier base plus progress, which would invent division 5.
        assert_eq!(ladder_position(gold, None, Some(80)), 2 * 500 + 250);
        assert_eq!(ladder_position(gold, None, None), 1250);
        // A division without progress sits at the middle of that division.
        assert_eq!(ladder_position(gold, Some(3), None), 1250);
        // Demotion protection sits below the division floor, never below 0.
        assert_eq!(ladder_position(gold, Some(3), Some(-15)), 1185);
        assert_eq!(ladder_position(0, Some(5), Some(-15)), 0);
    }

    #[test]
    fn tier_words_match_without_case_and_misreads_do_not() {
        assert_eq!(tier_index("EMERALD"), Some(4));
        assert_eq!(tier_index(" grandmaster "), Some(7));
        assert_eq!(tier_index("Emerlad"), None);
        assert_eq!(tier_index("Top 500"), None, "a placement, not a tier");
        assert_eq!(TIERS.len(), 9);
    }

    #[test]
    fn axis_is_windowed_to_the_range_inside_the_scale() {
        // Platinum 3 at 40% (1740) to Emerald 4 at 12% (2112).
        assert_eq!(axis_window(1740, 2112), (1600, 2300));
        // One game: Gold 3 at 50%.
        assert_eq!(axis_window(1250, 1250), (1100, 1400));
        assert_eq!(axis_window(0, 30), (0, 200), "floor is Bronze 5");
        assert_eq!(axis_window(4400, 4500), (4300, 4700));
        assert!(axis_window(4400, 4500).1 <= SCALE_CEILING);
        assert_eq!(
            tier_bands(1600, 2300),
            [
                TierBand {
                    tier: 3,
                    start: 1600,
                    end: 2000
                },
                TierBand {
                    tier: 4,
                    start: 2000,
                    end: 2300
                },
            ]
        );
        // The reserved band above Champion is never drawn.
        assert_eq!(
            tier_bands(4300, 4700),
            [TierBand {
                tier: 8,
                start: 4300,
                end: 4500
            }]
        );
    }

    #[test]
    fn division_numbers_sit_in_the_middle_of_each_division() {
        let labels = division_labels(1600, 2300);
        let numbers: Vec<i32> = labels.iter().map(|(d, _)| *d).collect();
        assert_eq!(numbers, [4, 3, 2, 1, 5, 4, 3], "bottom up, as the frame");
        for (_, mid) in &labels {
            assert_eq!(mid.rem_euclid(100.0), 50.0, "never on a gridline: {mid}");
        }
        assert_eq!(
            division_labels(4400, 4700).len(),
            1,
            "none above Champion 1"
        );
    }

    #[test]
    fn plot_percentages_span_the_canvas() {
        assert_eq!(x_pct(0, 13), PAD_X / VIEW_W * 100.0);
        assert_eq!(x_pct(12, 13), (VIEW_W - PAD_X) / VIEW_W * 100.0);
        assert_eq!(x_pct(0, 1), 50.0, "one game sits in the middle");
        assert_eq!(y_pct(2300.0, 1600, 2300), 0.0);
        assert_eq!(y_pct(1600.0, 1600, 2300), 100.0);
        assert!(y_pct(2030.0, 1600, 2300) < y_pct(1996.0, 1600, 2300));
    }

    #[test]
    fn change_wording_matches_the_frames() {
        let rows = chart_rows(&series_of(&POPULATED));
        let changes: Vec<&str> = rows.iter().map(|r| r.change.as_str()).collect();
        assert_eq!(
            changes,
            [
                "First point",
                "Up 22 points",
                "Up to Platinum 2",
                "Up 33 points",
                "Down 28 points",
                "Up 51 points",
                "Up to Platinum 1",
                "Up 45 points",
                "Up 41 points",
                "Up to Emerald 5, from Platinum 1 at 96%",
                "Up 28 points",
                "Up to Emerald 4",
                "Down to Emerald 5",
            ]
        );
        let diamonds: Vec<bool> = rows.iter().map(|r| r.rank_changed).collect();
        assert_eq!(diamonds.iter().filter(|d| **d).count(), 5);
        assert!(rows[12].derank && !rows[9].derank);
        assert_eq!(rows[9].date, "12 Nov 2026");
        assert_eq!(rows[9].date_short, "12 Nov");
        assert_eq!(rows[9].rank_line(), "Emerald 5 · 30%");
        assert_eq!(
            rows[9].point_label(),
            "12 Nov 2026, Emerald 5 at 30 percent, Up to Emerald 5, from Platinum 1 at 96%, Win"
        );
        assert_eq!(
            rows[0].point_label(),
            "2 Nov 2026, Platinum 3 at 40 percent, First point"
        );
    }

    #[test]
    fn a_rank_up_is_drawn_as_a_rise_on_one_line() {
        // The old chart broke the line where the rank changed, so a drop
        // from 96% to 30% after a rank up was not drawn as a loss. Now the
        // position itself rises and the line is one path.
        let rows = chart_rows(&series_of(&POPULATED));
        let pos = |i: usize| rows[i].placed.expect("placed").pos;
        assert_eq!((pos(8), pos(9)), (1996, 2030));
        assert!(pos(12) < pos(11), "the derank goes down");
        let html = render_chart(vec![series_of(&POPULATED)]);
        assert_eq!(
            html.matches("class=\"rank-chart-line\"").count(),
            1,
            "{html}"
        );
        assert_eq!(
            tags_with_class(&html, "rank-chart-mark-change").len(),
            5 + 1,
            "5 plus the legend"
        );
        assert_eq!(tags_with_class(&html, "rank-chart-mark-dot").len(), 8 + 1);
    }

    #[test]
    fn single_step_and_edge_change_words() {
        let step = |rank: &'static str, tier, division, pos, progress| Step {
            rank,
            tier,
            division,
            pos,
            progress,
        };
        let a = step("Gold 3", 2, Some(3), 1250, Some(50));
        assert_eq!(
            change_words(Some(&a), &step("Gold 3", 2, Some(3), 1251, Some(51))),
            "Up 1 point"
        );
        assert_eq!(
            change_words(Some(&a), &step("Gold 3", 2, Some(3), 1250, Some(50))),
            "No change"
        );
        assert_eq!(
            change_words(Some(&a), &step("Silver 1", 1, Some(1), 990, Some(90))),
            "Down to Silver 1, from Gold 3 at 50%"
        );
        let unread = step("Gold", 2, None, 1250, None);
        assert_eq!(
            change_words(Some(&unread), &step("Platinum 5", 3, Some(5), 1510, Some(10))),
            "Up to Platinum 5, from Gold"
        );
        // Same tier with the division read on one game only: not a move.
        assert_eq!(
            change_words(Some(&unread), &a),
            "Same tier, division not read"
        );
    }

    #[test]
    fn summary_sentence_matches_the_spec() {
        let rows = chart_rows(&series_of(&POPULATED));
        assert_eq!(
            summary_text(Some("Player01"), &rows, RankWindow::Page(1)),
            "Rank progress for Player01 from 2 Nov 2026 to 15 Nov 2026. \
             The chart plots absolute ladder position, so a rank up stays an up. \
             Player01 started at Platinum 3 at 40 percent, rose to Platinum 1 at 96 percent, \
             then ranked up to Emerald 5 at 30 percent (that move is up on this chart, not a drop), \
             peaked at Emerald 4, and ended at Emerald 5 at 40 percent after a derank on 15 Nov 2026."
        );
        let anon = summary_text(None, &rows, RankWindow::Page(1));
        assert!(
            anon.starts_with("Your rank progress from 2 Nov 2026"),
            "{anon}"
        );
        assert!(anon.contains("You started at Platinum 3"), "{anon}");
    }

    #[test]
    fn summary_for_one_game_and_for_none() {
        let one = chart_rows(&series_of(&[(8, "Gold", Some(3), Some(50), Some("win"))]));
        assert_eq!(
            summary_text(Some("Player01"), &one, RankWindow::Page(1)),
            "Rank progress for Player01. One ranked game on 8 Nov 2026: Gold 3 at 50 percent, a win. \
             A single game is shown as a point, with no line."
        );
        assert_eq!(
            summary_text(Some("Player01"), &[], RankWindow::Recent(10)),
            "Rank progress for Player01. There are no ranked games in your last 10 games, so the chart is empty."
        );
        assert_eq!(
            group_label(1, 0),
            "Rank progress chart with one game. A table with the same data is below."
        );
        assert_eq!(
            group_label(13, 1),
            "Rank progress chart. Arrow keys move between games. A table with the same data is below. One game is not plotted."
        );
    }

    #[test]
    fn a_misread_tier_is_unplaced_noted_and_listed() {
        let rows = chart_rows(&series_of(&unplaced_games()));
        assert_eq!(rows.len(), 14);
        let bad = &rows[10];
        assert_eq!(bad.placed, None);
        assert_eq!(bad.rank, "Emerlad 5", "the raw word and division");
        assert_eq!(bad.progress, Some(44));
        assert_eq!(bad.change, "Unplaced, rank not recognised");
        assert_eq!(bad.change_cell(), "Unplaced, rank not recognised (Win)");
        // The neighbours stay connected: the next change is from the last
        // plotted game, not from the unplaced one.
        assert_eq!(rows[11].change, "Up 28 points");
        assert_eq!(
            unplaced_note(&rows).as_deref(),
            Some("1 game not plotted: rank \"Emerlad\" not recognised")
        );
        assert_eq!(unplaced_note(&chart_rows(&series_of(&POPULATED))), None);
        let summary = summary_text(Some("Player01"), &rows, RankWindow::Page(1));
        assert!(
            summary.ends_with(
                "after a derank on 16 Nov 2026. One game on 13 Nov 2026 with rank \"Emerlad\" was not plotted \
                 because the rank was not recognised. That game is listed in the table as unplaced."
            ),
            "{summary}"
        );

        let html = render_chart(vec![series_of(&unplaced_games())]);
        // No mark and no hit target for it; its slot on the x axis stays.
        assert_eq!(tags_with_class(&html, "rank-chart-hit").len(), 13, "{html}");
        assert!(
            !html.contains("rank-chart-q-unknown-r-support-personal_match_g13-point"),
            "{html}"
        );
        assert!(
            html.contains("rank-chart-q-unknown-r-support-personal_match_g14-point"),
            "{html}"
        );
        assert!(
            html.contains(r#"<p class="rank-chart-note" id="rank-chart-note">1 game not plotted: rank &#34;Emerlad&#34; not recognised</p>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"aria-describedby="rank-chart-summary rank-chart-note""#),
            "{html}"
        );
        // Listed in the table, marked in words, with an empty emblem slot.
        let row = html
            .split("<tr")
            .find(|tr| tr.contains("Emerlad 5"))
            .expect("table row");
        assert!(row.contains("is-unplaced"), "{row}");
        assert!(row.contains("Unplaced, rank not recognised (Win)"), "{row}");
        assert!(!row.contains("<img"), "{row}");
        assert!(
            row.contains(r#"<span class="rank-chart-icon-slot" aria-hidden="true"></span>"#),
            "{row}"
        );
        // Current rank and range ignore it.
        assert!(
            !html
                .split("rank-chart-summary")
                .next()
                .unwrap()
                .contains("Emerlad"),
            "{html}"
        );
    }

    #[test]
    fn games_with_no_tier_word_are_unplaced_as_not_read() {
        let mut series = series_of(&[(8, "Gold", Some(3), Some(50), None)]);
        series.points[0].shown.tier = None;
        let rows = chart_rows(&series);
        assert_eq!(rows[0].rank, "Division 3");
        assert_eq!(rows[0].change, "Unplaced, rank not read");
        assert_eq!(
            unplaced_note(&rows).as_deref(),
            Some("1 game not plotted: rank not read")
        );
        let html = render_chart(vec![series]);
        assert!(html.contains("No rank read"), "{html}");
        assert!(html.contains("could be placed on the ladder"), "{html}");
        assert!(html.contains("<table"), "the game is still listed: {html}");
        assert!(!html.contains("rank-chart-hit"), "{html}");
    }

    #[test]
    fn keys_move_the_roving_tab_stop() {
        assert_eq!(nav_target(&Key::ArrowRight, 0, 13), Some(1));
        assert_eq!(nav_target(&Key::ArrowDown, 12, 13), Some(12));
        assert_eq!(nav_target(&Key::ArrowLeft, 0, 13), Some(0));
        assert_eq!(nav_target(&Key::ArrowUp, 5, 13), Some(4));
        assert_eq!(nav_target(&Key::Home, 5, 13), Some(0));
        assert_eq!(nav_target(&Key::End, 5, 13), Some(12));
        assert_eq!(nav_target(&Key::Enter, 5, 13), None);
        assert_eq!(nav_target(&Key::End, 0, 0), None);
    }

    #[test]
    fn chart_markup_is_one_section_with_the_spec_wiring() {
        let html = render_chart_for(vec![series_of(&POPULATED)], Some("Player01"));
        assert_eq!(html.matches("<h2").count(), 1, "{html}");
        assert!(
            html.contains(
                r#"<h2 class="ui-section-title" id="rank-chart-title">Rank progress</h2>"#
            ),
            "{html}"
        );
        assert!(
            html.contains(r#"aria-labelledby="rank-chart-title""#),
            "{html}"
        );
        assert!(!html.contains("<h1"), "the page header owns the h1: {html}");
        assert!(!html.contains("<main"), "the shell owns main: {html}");
        // The svg is decorative; names and dates are HTML.
        assert!(
            html.contains(r#"aria-hidden="true" focusable="false""#),
            "{html}"
        );
        assert!(!html.contains("<text"), "{html}");
        assert!(!html.contains(r#"role="img""#), "{html}");
        assert!(html.contains(r#"role="group""#), "{html}");
        assert!(
            html.contains(r#"aria-label="Rank progress chart. Arrow keys move between games. A table with the same data is below.""#),
            "{html}"
        );
        assert!(
            html.contains(r#"aria-describedby="rank-chart-summary""#),
            "{html}"
        );
        assert!(
            html.contains(r#"<p class="visually-hidden" id="rank-chart-summary">Rank progress for Player01 from 2 Nov 2026"#),
            "{html}"
        );
        // Two live regions, always rendered: the series line and the readout.
        assert_eq!(html.matches(r#"role="status""#).count(), 2, "{html}");
        assert!(
            html.contains(
                r#"<div class="rank-chart-readout" role="status" aria-live="polite"></div>"#
            ),
            "closed readout is still in the page: {html}"
        );
        // X labels: first and last date only.
        assert!(
            html.contains(">2 Nov</span>") && html.contains(">15 Nov</span>"),
            "{html}"
        );
        assert_eq!(
            tags_with_class(&html, "rank-chart-x-label").len(),
            2,
            "{html}"
        );
        // Legend and credit.
        assert!(html.contains("Rank change"), "{html}");
        assert!(
            html.contains("<p class=\"rank-chart-credit\">Rank emblems \u{a9} Blizzard Entertainment, Inc.</p>"),
            "{html}"
        );
        let credit_at = html.find("rank-chart-credit").unwrap();
        assert!(
            credit_at > html.find("</table>").unwrap(),
            "credit ends the section"
        );
    }

    #[test]
    fn hit_targets_are_named_buttons_with_one_tab_stop_and_data_ids() {
        let html = render_chart(vec![series_of(&POPULATED)]);
        let hits = tags_with_class(&html, "rank-chart-hit");
        assert_eq!(hits.len(), 13, "{html}");
        assert!(hits.iter().all(|h| h.starts_with("button ")), "{hits:?}");
        assert!(hits.iter().all(|h| h.contains("aria-label=\"")), "{hits:?}");
        assert_eq!(
            hits.iter()
                .filter(|h| h.contains(r#"tabindex="0""#))
                .count(),
            1,
            "roving tab stop: {hits:?}"
        );
        assert!(hits[0].contains(r#"tabindex="0""#), "{hits:?}");
        assert!(
            hits[9].contains(r#"id="rank-chart-q-unknown-r-support-personal_match_g12-point""#),
            "id from the series and the match id: {}",
            hits[9]
        );
        assert!(
            hits[12].contains(
                r#"aria-label="15 Nov 2026, Emerald 5 at 40 percent, Down to Emerald 5, Loss""#
            ),
            "{}",
            hits[12]
        );
        // Ids are unique.
        let mut ids: Vec<&str> = html
            .split(" id=\"")
            .skip(1)
            .filter_map(|s| s.split('"').next())
            .collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "{html}");
    }

    #[test]
    fn table_toggle_is_wired_and_rows_match_the_header() {
        let html = render_chart_for(vec![series_of(&POPULATED)], Some("Player01"));
        let toggle = tags_with_class(&html, "rank-chart-table-toggle");
        assert_eq!(toggle.len(), 1, "{html}");
        assert!(toggle[0].starts_with("button "), "{toggle:?}");
        assert!(
            toggle[0].contains("btn is-ghost"),
            "the shared button: {toggle:?}"
        );
        assert!(toggle[0].contains(r#"aria-expanded="false""#), "{toggle:?}");
        assert!(
            toggle[0].contains(r#"aria-controls="rank-chart-table""#),
            "{toggle:?}"
        );
        assert!(
            toggle[0].ends_with(">Show rank progress as a table"),
            "{toggle:?}"
        );
        let table = html.split("<table").nth(1).expect("table");
        assert!(
            table.starts_with(r#" class="rank-chart-table" id="rank-chart-table" hidden"#),
            "{table}"
        );
        assert!(
            table.contains(r#"<caption class="visually-hidden">Rank progress for Player01, one row per game</caption>"#),
            "{table}"
        );
        assert_eq!(table.matches("<th scope=\"col\"").count(), 4, "{table}");
        let body = table.split("<tbody>").nth(1).expect("tbody");
        let rows: Vec<&str> = body.split("<tr").skip(1).collect();
        assert_eq!(rows.len(), 13);
        for row in &rows {
            assert_eq!(
                row.matches("<td").count(),
                4,
                "cell count matches the header: {row}"
            );
        }
        // Win and loss are words, in the Change column.
        assert!(
            rows[1].contains("<td>Up 22 points (Win)</td>"),
            "{}",
            rows[1]
        );
        assert!(
            rows[4].contains("<td>Down 28 points (Loss)</td>"),
            "{}",
            rows[4]
        );
        assert!(
            rows[0].contains("<td>First point</td>"),
            "unknown result is left out: {}",
            rows[0]
        );
        assert!(
            rows[0].contains(r#"<span class="date">2 Nov 2026</span>"#),
            "{}",
            rows[0]
        );
    }

    #[test]
    fn cards_show_current_rank_and_range_with_large_emblems() {
        let html = render_chart(vec![series_of(&POPULATED)]);
        let cards = html
            .split("rank-chart-summary")
            .next()
            .expect("cards come before the chart");
        assert!(
            cards.contains("Current rank") && cards.contains("Range on this page"),
            "{cards}"
        );
        assert!(cards.contains("40% into the division"), "{cards}");
        assert_eq!(cards.matches("has-icon is-lg").count(), 3, "{cards}");
        assert_eq!(
            cards.matches(r#"width="32" height="32""#).count(),
            3,
            "{cards}"
        );
        assert!(
            cards.contains("Platinum 3") && cards.contains("Emerald 4"),
            "{cards}"
        );
        assert!(
            cards.contains(r#"<span class="rank-range-to">to</span>"#),
            "{cards}"
        );
        // Emblems are decorative: hidden slot, empty alt, next to the name.
        assert!(
            cards.contains(r#"aria-hidden="true"><img src=""#),
            "{cards}"
        );
        assert!(cards.contains(r#"rank-emerald.png" alt="" "#), "{cards}");
        // Same rank at both ends: one emblem and one name.
        let one = render_chart(vec![series_of(&[(
            8,
            "Gold",
            Some(3),
            Some(50),
            Some("win"),
        )])]);
        let cards = one.split("rank-chart-summary").next().unwrap();
        assert_eq!(cards.matches("has-icon is-lg").count(), 2, "{cards}");
        assert!(!cards.contains("rank-range-to"), "{cards}");
    }

    #[test]
    fn one_game_is_a_point_with_no_line_and_a_default_phone_readout() {
        let html = render_chart(vec![series_of(&[(
            8,
            "Gold",
            Some(3),
            Some(50),
            Some("win"),
        )])]);
        assert!(!html.contains("rank-chart-line"), "{html}");
        assert!(html.contains("rank-chart-plot is-single"), "{html}");
        assert_eq!(tags_with_class(&html, "rank-chart-hit").len(), 1, "{html}");
        assert_eq!(
            tags_with_class(&html, "rank-chart-x-label").len(),
            1,
            "{html}"
        );
        let readout = html
            .split(r#"<div class="rank-chart-readout is-phone-default" role="status" aria-live="polite">"#)
            .nth(1)
            .expect("readout");
        let readout = readout.split("</div>").next().unwrap();
        assert!(readout.contains("8 Nov 2026"), "{readout}");
        assert!(readout.contains("Gold 3 · 50%"), "{readout}");
        assert!(
            readout.contains("<p>First point</p><p>Win</p>"),
            "{readout}"
        );
        assert!(readout.contains(r#"width="20" height="20""#), "{readout}");
        assert!(
            html.contains(
                "Rank progress chart with one game. A table with the same data is below."
            ),
            "{html}"
        );
    }

    #[test]
    fn one_game_default_readout_stays_phone_only_and_lights_its_point() {
        let html = render_chart(vec![series_of(&[(
            8,
            "Gold",
            Some(3),
            Some(50),
            Some("win"),
        )])]);
        // Phone: the point looks active beside its open readout (frame
        // d1-stats-rank-one-game-phone), with a placed guide.
        assert!(
            html.contains(r#"class="rank-chart-guide is-phone-default" aria-hidden="true" style="left:50.00%""#),
            "{html}"
        );
        assert!(
            html.contains("rank-chart-mark-dot is-phone-default"),
            "{html}"
        );
        assert!(
            html.contains(r#"class="rank-chart-hit is-phone-default""#),
            "{html}"
        );
        let (desktop, phone) = RANK_CHART_CSS
            .split_once("@media (max-width: 820px)")
            .expect("phone block");
        // Desktop: the whole default readout is hidden. The rank line's own
        // flex rule must not win over the hide (it leaked "Gold 3 · 50%").
        let hide = css_rule(
            desktop,
            ".rank-chart-readout.is-phone-default > .readout-rank {",
        );
        assert!(has_decl(&hide, "display: none"), "{hide}");
        let show = css_rule(
            phone,
            ".rank-chart-readout.is-phone-default > .readout-rank {",
        );
        assert!(has_decl(&show, "display: flex"), "{show}");
        for marker in [
            ".rank-chart-guide.is-phone-default {",
            ".rank-chart-hit.is-phone-default {",
        ] {
            assert!(!desktop.contains(marker), "{marker} is phone only");
            assert!(phone.contains(marker), "{marker}");
        }
        // Two games: nothing is lit until a point is picked.
        let two = render_chart(vec![series_of(&[
            (8, "Gold", Some(3), Some(50), None),
            (9, "Gold", Some(3), Some(60), None),
        ])]);
        assert!(!two.contains("is-phone-default"), "{two}");
    }

    #[test]
    fn table_toggle_keeps_its_gap_over_the_shared_button_margin() {
        // `.btn` sets `margin: 0`; a lone class selector loses to it when the
        // button sheet loads later, so the toggle sat flush on the legend.
        let rule = css_rule(RANK_CHART_CSS, ".rank-chart .rank-chart-table-toggle {");
        assert!(has_decl(&rule, "margin-top: var(--space-3)"), "{rule}");
        assert!(!RANK_CHART_CSS.contains("\n.rank-chart-table-toggle {"));
    }

    #[test]
    fn empty_and_loading_states_say_so_in_words() {
        let html = render_chart_in(Vec::new(), Some("Player01"), RankWindow::Recent(10));
        assert!(html.contains("No rank read"), "{html}");
        assert!(html.contains(">None</p>"), "{html}");
        assert!(
            html.contains(
                "No ranked games in your last 10 games. Play a ranked match and it will show up here."
            ),
            "{html}"
        );
        assert!(
            html.contains("There are no ranked games in your last 10 games, so the chart is empty."),
            "{html}"
        );
        assert!(!html.contains("<table") && !html.contains("<img"), "{html}");
        assert!(!html.contains("rank-chart-legend"), "{html}");
        assert_eq!(html.matches(r#"role="status""#).count(), 2, "{html}");
        assert_eq!(html.matches("<h2").count(), 1, "{html}");

        let mut dom = VirtualDom::new(RankChartLoading);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains(r#"aria-busy="true""#), "{html}");
        assert!(html.contains(">Loading rank progress.</p>"), "{html}");
        assert!(
            html.contains(
                r#"<h2 class="ui-section-title" id="rank-chart-title">Rank progress</h2>"#
            ),
            "{html}"
        );
        assert!(
            !html.contains("<table") && !html.contains("rank-chart-legend"),
            "{html}"
        );
    }

    #[test]
    fn missing_division_shows_the_tier_alone() {
        let html = render_chart(vec![series_of(&[
            (8, "Gold", None, Some(80), None),
            (9, "Gold", Some(3), Some(60), None),
        ])]);
        assert!(
            html.contains("<td>80%</td>"),
            "the progress still shows: {html}"
        );
        let rows = chart_rows(&series_of(&[(8, "Gold", None, Some(80), None)]));
        assert_eq!(rows[0].rank, "Gold");
        assert_eq!(rows[0].placed, Some(Placed { tier: 2, pos: 1250 }));
    }

    #[test]
    fn outcomes_reach_the_points_and_unknown_words_do_not() {
        assert_eq!(GameResult::from_outcome("Win"), Some(GameResult::Win));
        assert_eq!(GameResult::from_outcome("loss"), Some(GameResult::Loss));
        assert_eq!(GameResult::from_outcome("draw"), Some(GameResult::Draw));
        assert_eq!(GameResult::from_outcome("unknown"), None);
        let a = game(None, "Support", 40);
        let series = rank_series_with_outcomes([
            ("m1", at(1), Some(&a), Some("win")),
            ("m2", at(2), Some(&a), None),
        ]);
        assert_eq!(series[0].points[0].result, Some(GameResult::Win));
        assert_eq!(series[0].points[1].result, None);
        let plain = rank_series([("m1", at(1), Some(&a))]);
        assert_eq!(plain[0].points[0].result, None);
    }

    #[test]
    fn chart_css_uses_tokens_and_the_spec_sizes() {
        // 44px, capped at the game's slot (see hit_targets_never_overlap).
        let hit = css_rule(RANK_CHART_CSS, ".rank-chart-hit {");
        assert!(
            has_decl(
                &hit,
                "width: min(var(--hit-target-min), var(--rank-slot, var(--hit-target-min)))"
            ),
            "{hit}"
        );
        assert!(has_decl(&hit, "height: var(--hit-target-min)"), "{hit}");
        let line = css_rule(RANK_CHART_CSS, ".rank-chart-line {");
        assert!(has_decl(&line, "stroke: var(--chart-2)"), "{line}");
        assert!(has_decl(&line, "stroke-width: 3"), "{line}");
        assert!(
            has_decl(&line, "vector-effect: non-scaling-stroke"),
            "{line}"
        );
        let empty_slot = css_rule(RANK_CHART_CSS, ".rank-chart-icon-slot {");
        assert!(
            has_decl(&empty_slot, "display: none"),
            "no width and no gap: {empty_slot}"
        );
        let credit = css_rule(RANK_CHART_CSS, ".rank-chart-credit {");
        assert!(has_decl(&credit, "font-size: var(--text-xs)"), "{credit}");
        assert!(has_decl(&credit, "color: var(--text-3)"), "{credit}");
        assert!(
            RANK_CHART_CSS.contains("outline: var(--focus-ring-width) solid var(--focus-ring)")
        );
        assert!(RANK_CHART_CSS.contains("@media (max-width: 820px)"));
        // No colour literals and no one-off type sizes.
        for banned in ["#", concat!("rg", "b("), concat!("hs", "l("), "rem;"] {
            assert!(!RANK_CHART_CSS.contains(banned), "{banned}");
        }
        for size in RANK_CHART_CSS.split("font-size:").skip(1) {
            assert!(
                size.trim_start().starts_with("var(--text-"),
                "{}",
                &size[..30]
            );
        }
    }

    /// FNV-1a, 64 bit: enough to pin a file's bytes without a hash crate.
    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    /// The emblem sources stay byte for byte as the design sources hold them:
    /// each file is pinned by length and by a hash of every byte.
    #[test]
    fn emblem_files_are_unchanged() {
        let pins: [(&str, &[u8], usize, u64); 9] = [
            (
                "bronze",
                include_bytes!("../../../assets/rank-icons/rank-bronze.png"),
                7075,
                0x59f0_113b_85de_21d8,
            ),
            (
                "silver",
                include_bytes!("../../../assets/rank-icons/rank-silver.png"),
                6127,
                0xcc25_9879_b50a_0e4f,
            ),
            (
                "gold",
                include_bytes!("../../../assets/rank-icons/rank-gold.png"),
                7548,
                0x10c0_a30d_0be6_65cc,
            ),
            (
                "platinum",
                include_bytes!("../../../assets/rank-icons/rank-platinum.png"),
                9142,
                0xa2a7_ef6d_361a_bf10,
            ),
            (
                "emerald",
                include_bytes!("../../../assets/rank-icons/rank-emerald.png"),
                9963,
                0xc908_99ab_9f2a_44cc,
            ),
            (
                "diamond",
                include_bytes!("../../../assets/rank-icons/rank-diamond.png"),
                11025,
                0x6dcc_73d6_69e2_5db4,
            ),
            (
                "master",
                include_bytes!("../../../assets/rank-icons/rank-master.png"),
                11483,
                0x27f5_a882_129e_4515,
            ),
            (
                "grandmaster",
                include_bytes!("../../../assets/rank-icons/rank-grandmaster.png"),
                12901,
                0xbdfd_f4a3_0bb2_0e9d,
            ),
            (
                "champion",
                include_bytes!("../../../assets/rank-icons/rank-champion.png"),
                13101,
                0x0dc1_0aab_fc73_a3e1,
            ),
        ];
        for (i, (name, bytes, len, hash)) in pins.iter().enumerate() {
            assert_eq!(bytes.len(), *len, "{name}");
            assert_eq!(fnv1a(bytes), *hash, "{name}: bytes changed");
            assert!(bytes.starts_with(b"\x89PNG"), "{name}");
            assert_eq!(TIERS[i].to_lowercase(), *name, "ladder order");
            let url = emblem_url(i).expect(name);
            assert!(url.ends_with(&format!("rank-{name}.png")), "{url}");
        }
        assert_eq!(emblem_url(9), None, "no emblem above Champion");
    }

    #[test]
    fn a_dip_before_the_promotion_is_not_called_a_rise() {
        // Platinum 1 at 60%, a loss to 40%, then the promotion: the game
        // before the tier-up is lower than the start.
        let rows = chart_rows(&series_of(&[
            (2, "Platinum", Some(1), Some(60), None),
            (3, "Platinum", Some(1), Some(40), Some("loss")),
            (4, "Emerald", Some(5), Some(10), Some("win")),
        ]));
        let text = summary_text(Some("Player01"), &rows, RankWindow::Page(1));
        assert!(!text.contains("rose to"), "{text}");
        assert!(text.contains("fell to Platinum 1 at 40 percent"), "{text}");
        // A climb before the promotion still reads as a rise.
        let rows = chart_rows(&series_of(&[
            (2, "Platinum", Some(1), Some(40), None),
            (3, "Platinum", Some(1), Some(60), Some("win")),
            (4, "Emerald", Some(5), Some(10), Some("win")),
        ]));
        let text = summary_text(Some("Player01"), &rows, RankWindow::Page(1));
        assert!(text.contains("rose to Platinum 1 at 60 percent"), "{text}");
    }

    #[test]
    fn unread_fields_never_count_as_a_change() {
        // Gold 3, then Gold with the division unread: no derank, no diamond.
        let rows = chart_rows(&series_of(&[
            (2, "Gold", Some(3), Some(80), None),
            (3, "Gold", None, Some(80), None),
            (4, "Gold", Some(3), None, None),
            (5, "Gold", Some(3), Some(70), None),
        ]));
        assert!(!rows[1].rank_changed && !rows[1].derank, "{:?}", rows[1]);
        assert_eq!(rows[1].change, "Same tier, division not read");
        assert!(!rows[2].rank_changed && !rows[2].derank, "{:?}", rows[2]);
        assert_eq!(rows[2].change, "Same tier, division not read");
        // Progress read on one game only: the midpoint is not a step.
        assert_eq!(rows[3].change, "Same rank, progress not read");
        assert!(!rows[3].rank_changed);
        // Read on both games, the step is still counted.
        let rows = chart_rows(&series_of(&[
            (2, "Gold", Some(3), None, None),
            (3, "Gold", Some(2), None, None),
        ]));
        assert_eq!(rows[1].change, "Up to Gold 2");
        assert!(rows[1].rank_changed && !rows[1].derank);
    }

    #[test]
    fn a_highlight_clears_when_the_pointer_or_focus_leaves() {
        let mut p = Pointer::default();
        p.apply(PointerEvent::Enter, "a");
        assert_eq!(p.active().as_deref(), Some("a"));
        p.apply(PointerEvent::Leave, "a");
        assert_eq!(p.active(), None, "hover alone leaves nothing behind");
        p.apply(PointerEvent::Focus, "b");
        p.apply(PointerEvent::Enter, "c");
        assert_eq!(p.active().as_deref(), Some("c"), "hover wins while it lasts");
        p.apply(PointerEvent::Leave, "c");
        assert_eq!(p.active().as_deref(), Some("b"), "back to the focused game");
        // A late blur from another game keeps the current one.
        p.apply(PointerEvent::Blur, "a");
        assert_eq!(p.active().as_deref(), Some("b"));
        p.apply(PointerEvent::Blur, "b");
        assert_eq!(p.active(), None);
        // The hit targets clear on leave and blur.
        let html = render_chart(vec![series_of(&POPULATED)]);
        assert!(!html.contains("is-active"), "{html}");
        let source = include_str!("rank.rs");
        let render = &source[source.find("for hit in hits {").expect("hit loop")..];
        let render = &render[..render.find("div { class: \"rank-chart-x\"").expect("x axis")];
        for handler in ["onmouseleave:", "onblur:"] {
            assert!(render.contains(handler), "{handler}");
        }
    }

    /// WCAG contrast of two `#rrggbb` colours.
    fn contrast(a: &str, b: &str) -> f64 {
        fn lum(hex: &str) -> f64 {
            let c = |i: usize| {
                let v = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).expect(hex)) / 255.0;
                if v <= 0.039_28 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5)
        }
        let (a, b) = (lum(a), lum(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// A `--name: #rrggbb;` value from a token sheet.
    fn hex_token(css: &str, name: &str) -> String {
        let at = css.find(&format!("{name}: #")).expect(name) + name.len() + 2;
        css[at..at + 7].to_string()
    }

    #[test]
    fn light_chart_line_keeps_three_to_one_on_every_band() {
        let primitives = include_str!("../../theme/css/primitives.css");
        let line = hex_token(primitives, "--chart-mint-light");
        let card = hex_token(primitives, "--neutral-0");
        // Light: the bands take the card fill, so the line only ever sits on
        // the card.
        let band = css_rule(RANK_CHART_CSS, "[data-theme=\"light\"] .rank-chart-band {");
        assert!(has_decl(&band, "fill: var(--card-bg)"), "{band}");
        let look = include_str!("../../theme/css/look_a.css");
        let light = &look[look.find("[data-theme=\"light\"] {").expect("light")..];
        assert!(light.contains("--surface: var(--neutral-0);"));
        assert!(light.contains("--chart-2: var(--chart-mint-light);"));
        assert!(contrast(&line, &card) >= 3.0, "{line} on {card}");
        // The soft band it replaces is under 3:1: the reason for the rule.
        let semantic = include_str!("../../theme/css/semantic.css");
        let light_soft = &semantic[semantic.find("pre-composited on A --surface #ffffff").expect("light soft")..];
        let soft = hex_token(light_soft, "--neutral-soft");
        assert!(contrast(&line, &soft) < 3.0, "{line} on {soft}");
        // The table toggle stays as the text fallback whenever the plot shows.
        let html = render_chart(vec![series_of(&POPULATED)]);
        assert!(html.contains("Show rank progress as a table"), "{html}");
    }

    #[test]
    fn copy_names_the_games_the_chart_was_given() {
        // Overview: ten unranked recent games are not "no rank yet".
        let mut series = series_of(&[(8, "Gold", Some(3), Some(50), None)]);
        series.points[0].shown.tier = None;
        let html = render_chart_in(vec![series], None, RankWindow::Recent(10));
        assert!(!html.contains("No rank yet"), "{html}");
        assert!(html.contains("No game in your last 10 games has a rank"), "{html}");
        assert!(html.contains("Recent range"), "{html}");
        // History page 3: its newest game is not the current rank.
        let html = render_chart_in(vec![series_of(&POPULATED)], None, RankWindow::Page(3));
        assert!(!html.contains("Current rank"), "{html}");
        assert!(html.contains("Latest rank on this page"), "{html}");
        assert!(html.contains("Range on this page"), "{html}");
        assert!(
            html.contains("Lowest to highest rank on page 3 of your match history."),
            "{html}"
        );
        // The first page and Overview do hold the current rank.
        let html = render_chart_in(vec![series_of(&POPULATED)], None, RankWindow::Recent(13));
        assert!(html.contains("Current rank"), "{html}");
        assert_eq!(RankWindow::Recent(1).period(), "in your last game");
    }

    #[test]
    fn hit_targets_never_overlap_their_neighbours() {
        // A 25-game history page: each target is capped at its slot width.
        let slot = hit_slot_style(25);
        assert_eq!(slot, format!("--rank-slot:{:.2}%", PLOT_W / 24.0 / VIEW_W * 100.0));
        let hit = css_rule(RANK_CHART_CSS, ".rank-chart-hit {");
        assert!(
            has_decl(
                &hit,
                "width: min(var(--hit-target-min), var(--rank-slot, var(--hit-target-min)))"
            ),
            "{hit}"
        );
        assert!(!has_decl(&hit, "min-width: var(--hit-target-min)"), "{hit}");
        assert!(has_decl(&hit, "height: var(--hit-target-min)"), "{hit}");
        // Neighbouring slots touch but do not overlap: the slot is the x step.
        assert!((x_pct(1, 25) - x_pct(0, 25) - PLOT_W / 24.0 / VIEW_W * 100.0).abs() < 1e-9);
        let html = render_chart(vec![series_of(&POPULATED)]);
        assert!(
            html.contains(&format!("class=\"rank-chart-hits\" style=\"{}\"", hit_slot_style(13))),
            "{html}"
        );
    }

    #[test]
    fn forced_colours_keep_the_line_and_marks_visible() {
        use crate::components::ui::test_support::media_block;
        let forced = media_block(RANK_CHART_CSS, "@media (forced-colors: active)");
        let line = css_rule(&forced, ".rank-chart-line {");
        assert!(has_decl(&line, "stroke: CanvasText"), "{line}");
        let dot = css_rule(&forced, ".rank-chart-mark-dot {");
        assert!(has_decl(&dot, "forced-color-adjust: none"), "{dot}");
        assert!(has_decl(&dot, "background: CanvasText"), "{dot}");
        let diamond = css_rule(&forced, ".rank-chart-mark-change {");
        assert!(has_decl(&diamond, "border-color: CanvasText"), "{diamond}");
        let active = css_rule(&forced, ".rank-chart-hit.is-active {");
        assert!(has_decl(&active, "outline-color: Highlight"), "{active}");
    }

    #[test]
    fn copy_has_no_em_dash_and_no_streamer() {
        let src = include_str!("rank.rs");
        assert!(!src.contains('\u{2014}'));
        assert!(!src.contains('\u{2013}'));
        assert!(!src.contains(concat!("Overwatch", " 2")));
        assert!(!src.to_lowercase().contains(concat!("the ", "streamer")));
    }
}
