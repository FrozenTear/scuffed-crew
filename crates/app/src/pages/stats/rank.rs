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
//! A value shows only when its confidence is at least [`MIN_CONFIDENCE`]. A
//! value with a missing or lower confidence is hidden, never guessed. The
//! queue is the read queue title; a missing or unsure title files the game
//! under "Queue unknown". The same goes for the role ("Role unknown").
//!
//! This is opt in: with no rank data on any game, nothing here renders.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use scuffed_types::api::resolve_rank;
use scuffed_types::{MatchRank, RankField, RankRole};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::components::ui::ChipButton;

/// Lowest confidence a rank value needs to show. rank-v1 reads open queue
/// (no role line) at exactly 0.5 and accepts no icon read below about 0.53,
/// so this keeps every value the reader accepted and hides weaker ones.
pub(super) const MIN_CONFIDENCE: f32 = 0.5;

/// Bucket label for a game whose queue title is missing or unsure.
pub(super) const QUEUE_UNKNOWN: &str = "Queue unknown";
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
/// reader id, are no rank.
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
        || rank.next_mode_hint.value.is_some()
        || rank.queue_title.value.is_some()
}

/// The value when its confidence clears [`MIN_CONFIDENCE`], else None.
fn sure<T: Clone>(field: &RankField<T>) -> Option<T> {
    match field.confidence {
        Some(c) if c >= MIN_CONFIDENCE => field.value.clone(),
        _ => None,
    }
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

/// What one game's rank shows: only values read with enough confidence.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct RankShown {
    pub role: Option<RankRole>,
    /// Tier name in title case, e.g. "Diamond".
    pub tier: Option<String>,
    /// 1..=5.
    pub division: Option<u8>,
    /// 0..=100.
    pub progress: Option<u8>,
    /// Progress points gained or lost this game, -100..=100.
    pub change: Option<i32>,
    /// Queue title as read, single spaced, in capitals (the bucket key).
    pub queue: Option<String>,
}

impl RankShown {
    /// None when nothing clears the confidence bar.
    pub(super) fn from_rank(rank: &MatchRank) -> Option<Self> {
        let shown = Self {
            role: sure(&rank.role),
            tier: sure(&rank.role_rank)
                .and_then(|t| clean_text(&t))
                .map(|t| title_case(&t.to_lowercase())),
            division: sure(&rank.division).filter(|d| (1..=5).contains(d)),
            progress: sure(&rank.progress_pct)
                .and_then(|p| u8::try_from(p).ok())
                .filter(|p| *p <= 100),
            change: sure(&rank.progress_change).filter(|c| (-100..=100).contains(c)),
            queue: sure(&rank.queue_title)
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

    pub(super) fn queue_label(&self) -> String {
        self.queue
            .as_deref()
            .map(title_case_upper)
            .unwrap_or_else(|| QUEUE_UNKNOWN.to_string())
    }

    pub(super) fn role_label(&self) -> &'static str {
        self.role.map(role_name).unwrap_or(ROLE_UNKNOWN)
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
        let mut parts = vec![format!("{}, {}", self.role_label(), self.queue_label())];
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
        gap: 0.1rem 0.35rem;
        max-width: 100%;
        padding: 0.05rem 0.5rem;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 999px;
        font-size: 0.75rem;
        color: var(--text);
        font-variant-numeric: tabular-nums;
        overflow-wrap: anywhere;
    }
    .rank-badge-sep { color: var(--text-3); }
    .match-card .match-rank {
        grid-column: 1 / -1;
        min-width: 0;
    }
    .game-rank { margin: 0.5rem 0 0; }
    .game-rank-queue { color: var(--text-3); font-size: 0.8rem; margin-left: 0.4rem; }

    .rank-chart {
        margin: 0 0 1rem;
        padding: 0.9rem 1rem;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        min-width: 0;
    }
    .rank-chart-heading {
        margin: 0 0 0.6rem;
        font-family: var(--font-head);
        font-size: 1rem;
        color: var(--text);
    }
    .rank-chart-status { margin: 0.5rem 0; font-size: 0.8rem; color: var(--text-2); }
    .rank-chart-plot { max-width: 640px; }
    .rank-chart-svg { display: block; width: 100%; height: auto; overflow: visible; }
    .rank-chart-grid { stroke: var(--border); stroke-width: 1; }
    .rank-chart-axis { fill: var(--text-3); font-size: 9px; }
    .rank-chart-line { fill: none; stroke: var(--chart-1); stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
    .rank-chart-dot { fill: var(--chart-1); stroke: var(--surface); stroke-width: 2; }
    .rank-chart-ends {
        display: flex;
        justify-content: space-between;
        gap: 0.5rem;
        font-size: 0.75rem;
        color: var(--text-3);
    }
    .rank-chart-note { margin: 0.4rem 0 0; font-size: 0.75rem; color: var(--text-3); }
    .rank-chart-table-wrap { margin-top: 0.5rem; font-size: 0.8rem; }
    .rank-chart-table-wrap summary { cursor: pointer; color: var(--text-2); }
    .rank-chart-table { width: 100%; border-collapse: collapse; margin-top: 0.4rem; }
    .rank-chart-table th,
    .rank-chart-table td {
        padding: 0.25rem 0.4rem;
        text-align: left;
        border-bottom: 1px solid var(--border);
        font-variant-numeric: tabular-nums;
        overflow-wrap: anywhere;
    }
    .rank-chart-table th { color: var(--text-3); font-weight: 600; }
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
            span { class: "sr-only", "{sentence}" }
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
    let context = format!("{}, {}", shown.role_label(), shown.queue_label());
    rsx! {
        p { class: "game-rank",
            {badge(match_id, &shown, &parts)}
            span { class: "game-rank-queue", aria_hidden: "true", "{context}" }
        }
    }
}

// -- Chart --

/// One game with a rank, as the chart needs it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RankPoint {
    pub match_id: String,
    pub played_at: DateTime<Utc>,
    pub shown: RankShown,
}

/// Every ranked game of one (queue, role), oldest first.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RankSeries {
    /// Queue key in capitals, or None for "Queue unknown".
    pub queue: Option<String>,
    pub role: Option<RankRole>,
    pub points: Vec<RankPoint>,
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
        let queue = self
            .queue
            .as_deref()
            .map(title_case_upper)
            .unwrap_or_else(|| QUEUE_UNKNOWN.to_string());
        let role = self.role.map(role_name).unwrap_or(ROLE_UNKNOWN);
        format!("{role}, {queue}")
    }

    /// Points with a progress value, the ones the line draws.
    fn plotted(&self) -> impl Iterator<Item = &RankPoint> {
        self.points.iter().filter(|p| p.shown.progress.is_some())
    }

    fn latest(&self) -> Option<DateTime<Utc>> {
        self.points.iter().map(|p| p.played_at).max()
    }
}

/// Group games by (queue, role). Games without a rank to show are skipped.
/// Order: known queues A to Z, then "Queue unknown"; inside a queue, Tank,
/// Damage, Support, Open queue, then "Role unknown". Points oldest first.
pub(super) fn rank_series<'a>(
    games: impl IntoIterator<Item = (&'a str, DateTime<Utc>, Option<&'a MatchRank>)>,
) -> Vec<RankSeries> {
    let mut out: Vec<RankSeries> = Vec::new();
    for (id, played_at, rank) in games {
        let Some(shown) = rank.and_then(RankShown::from_rank) else {
            continue;
        };
        let point = RankPoint {
            match_id: id.to_string(),
            played_at,
            shown,
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

/// Series the chart offers: those with at least one progress value.
pub(super) fn chart_series(series: Vec<RankSeries>) -> Vec<RankSeries> {
    series
        .into_iter()
        .filter(|s| s.plotted().next().is_some())
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

// Plot box, in SVG units. The SVG scales to its width, so these are ratios.
const VIEW_W: f64 = 320.0;
const VIEW_H: f64 = 140.0;
const PAD_L: f64 = 30.0;
const PAD_R: f64 = 8.0;
const PAD_T: f64 = 8.0;
const PAD_B: f64 = 8.0;

/// (x, y) for each plotted point, evenly spaced in play order. Progress is
/// 0 at the bottom and 100 at the top.
fn plot_coords(progress: &[u8]) -> Vec<(f64, f64)> {
    let plot_w = VIEW_W - PAD_L - PAD_R;
    let plot_h = VIEW_H - PAD_T - PAD_B;
    let n = progress.len();
    progress
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let x = if n <= 1 {
                PAD_L + plot_w / 2.0
            } else {
                PAD_L + plot_w * i as f64 / (n - 1) as f64
            };
            let y = PAD_T + plot_h * (100.0 - f64::from(*p)) / 100.0;
            (x, y)
        })
        .collect()
}

/// Indexes where the line starts again: progress restarts when the rank or
/// division changes, so a drop from 90% to 10% after a rank up is not drawn
/// as a loss.
fn line_runs(points: &[&RankPoint]) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..=points.len() {
        let breaks =
            i == points.len() || points[i].shown.rank_text() != points[i - 1].shown.rank_text();
        if breaks {
            runs.push(start..i);
            start = i;
        }
    }
    runs
}

fn status_text(series: &RankSeries) -> String {
    let n = series.plotted().count();
    let games = if n == 1 { "game" } else { "games" };
    format!(
        "Showing {}: {n} {games} with rank progress.",
        series.label()
    )
}

/// Rank progress over time for one (queue, role) at a time, with a selector.
/// The caller renders this only when [`chart_series`] is not empty.
#[component]
pub(super) fn RankChart(series: Vec<RankSeries>) -> Element {
    let mut picked = use_signal(|| None::<String>);
    let fallback = default_key(&series);
    let current_key = picked()
        .filter(|k| series.iter().any(|s| &s.key() == k))
        .or(fallback);
    let Some(current) = current_key
        .as_ref()
        .and_then(|k| series.iter().find(|s| &s.key() == k))
    else {
        return rsx! {};
    };

    let plotted: Vec<&RankPoint> = current.plotted().collect();
    let progress: Vec<u8> = plotted.iter().filter_map(|p| p.shown.progress).collect();
    let coords = plot_coords(&progress);
    let runs = line_runs(&plotted);
    let status = status_text(current);
    let label = current.label();
    let key = current.key();
    let svg_label = format!("{CHART_TITLE} for {label}, oldest game on the left");
    let first = plotted
        .first()
        .map(|p| p.played_at.format("%b %d").to_string())
        .unwrap_or_default();
    let last = plotted
        .last()
        .map(|p| p.played_at.format("%b %d").to_string())
        .unwrap_or_default();
    let table_id = format!("rank-chart-{key}-table");

    rsx! {
        section { class: "rank-chart", aria_labelledby: "rank-chart-heading",
            h2 { class: "rank-chart-heading", id: "rank-chart-heading", "{CHART_TITLE}" }
            if series.len() > 1 {
                div { class: "filter-group", role: "group", aria_labelledby: "rank-chart-picker-label",
                    span { class: "filter-label", id: "rank-chart-picker-label", "Queue and role" }
                    for s in series.iter() {
                        {
                            let k = s.key();
                            let pressed = Some(&k) == current_key.as_ref();
                            let pick = k.clone();
                            rsx! {
                                ChipButton {
                                    key: "{k}",
                                    class: "filter-chip".to_string(),
                                    label: s.label(),
                                    pressed,
                                    onclick: move |_| picked.set(Some(pick.clone())),
                                }
                            }
                        }
                    }
                }
            }
            p { class: "rank-chart-status", role: "status", "{status}" }
            div { class: "rank-chart-plot",
                svg {
                    class: "rank-chart-svg",
                    view_box: "0 0 {VIEW_W} {VIEW_H}",
                    role: "img",
                    "aria-label": "{svg_label}",
                    for pct in [0u8, 50, 100] {
                        {
                            let y = PAD_T + (VIEW_H - PAD_T - PAD_B) * (100.0 - f64::from(pct)) / 100.0;
                            rsx! {
                                g { key: "{pct}",
                                    line {
                                        class: "rank-chart-grid",
                                        x1: "{PAD_L}",
                                        x2: "{VIEW_W - PAD_R}",
                                        y1: "{y}",
                                        y2: "{y}",
                                    }
                                    text {
                                        class: "rank-chart-axis",
                                        x: "{PAD_L - 4.0}",
                                        y: "{y + 3.0}",
                                        text_anchor: "end",
                                        "{pct}%"
                                    }
                                }
                            }
                        }
                    }
                    for run in runs.iter().filter(|r| r.len() > 1) {
                        {
                            let points = coords[run.clone()]
                                .iter()
                                .map(|(x, y)| format!("{x:.1},{y:.1}"))
                                .collect::<Vec<_>>()
                                .join(" ");
                            rsx! {
                                polyline { key: "{run.start}", class: "rank-chart-line", points: "{points}" }
                            }
                        }
                    }
                    for (point, (x, y)) in plotted.iter().zip(coords.iter()) {
                        {
                            let tip = format!(
                                "{}: {}",
                                point.played_at.format("%b %d, %Y"),
                                point.shown.badge_parts().join(", ")
                            );
                            rsx! {
                                circle {
                                    key: "{point.match_id}",
                                    class: "rank-chart-dot",
                                    cx: "{x:.1}",
                                    cy: "{y:.1}",
                                    r: "4",
                                    title { "{tip}" }
                                }
                            }
                        }
                    }
                }
                div { class: "rank-chart-ends", aria_hidden: "true",
                    span { "{first}" }
                    span { "{last}" }
                }
            }
            p { class: "rank-chart-note", "From the games loaded on this page. The line breaks where the rank changes." }
            details { class: "rank-chart-table-wrap",
                summary { "Show values as a table" }
                table { class: "rank-chart-table", id: "{table_id}",
                    caption { class: "sr-only", "{CHART_TITLE} for {label}" }
                    thead {
                        tr {
                            th { scope: "col", "Date" }
                            th { scope: "col", "Rank" }
                            th { scope: "col", "Progress" }
                            th { scope: "col", "Change" }
                        }
                    }
                    tbody {
                        for point in plotted.iter() {
                            tr { key: "{point.match_id}",
                                td { "{point.played_at.format(\"%Y-%m-%d\")}" }
                                td { {point.shown.rank_text().unwrap_or_default()} }
                                td { {point.shown.progress.map(|p| format!("{p}%")).unwrap_or_default()} }
                                td { {point.shown.change.map(change_text).unwrap_or_default()} }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn full_rank_shows_every_sure_value() {
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
        assert_eq!(shown.queue_label(), "Role Queue");
        assert_eq!(
            shown.sentence(),
            "Rank: Support, Role Queue, Diamond 2, 45% progress, +23% this game"
        );
    }

    #[test]
    fn partial_rank_hides_low_missing_and_out_of_range_values() {
        let r = rank(
            r#"{
            "role_rank": { "value": "Gold", "confidence": 0.49 },
            "division": { "value": 3, "confidence": 0.7 },
            "progress_pct": { "value": -15, "confidence": 0.9 },
            "progress_change": { "value": -8 },
            "queue_title": { "value": "OPEN QUEUE", "confidence": 0.2 }
        }"#,
        )
        .expect("has values");
        let shown = RankShown::from_rank(&r).unwrap();
        assert_eq!(shown.tier, None, "confidence below the bar");
        assert_eq!(shown.division, Some(3));
        assert_eq!(shown.progress, None, "negative progress is not a value");
        assert_eq!(shown.change, None, "no confidence is hidden");
        assert_eq!(shown.queue, None);
        assert_eq!(shown.badge_parts(), ["Division 3"]);
        assert_eq!(shown.queue_label(), QUEUE_UNKNOWN);
        assert_eq!(shown.role_label(), ROLE_UNKNOWN);
    }

    #[test]
    fn rank_with_values_but_nothing_sure_shows_nothing() {
        let r = rank(r#"{"role_rank": {"value": "Gold", "confidence": 0.1}}"#).unwrap();
        assert_eq!(RankShown::from_rank(&r), None);
        let html = render_badge("m1", Some(r));
        assert_eq!(html, "");
    }

    #[test]
    fn change_text_signs() {
        assert_eq!(change_text(5), "+5%");
        assert_eq!(change_text(-5), "-5%");
        assert_eq!(change_text(0), "0%");
    }

    fn game(queue: Option<&str>, role: &str, progress: u8) -> MatchRank {
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
    fn buckets_by_queue_and_role_with_queue_unknown_last() {
        let low_queue = rank(
            r#"{"queue_title": {"value": "ROLE QUEUE", "confidence": 0.3},
                "role": {"value": "Tank", "confidence": 0.9},
                "progress_pct": {"value": 10, "confidence": 0.9}}"#,
        )
        .unwrap();
        let ranks = [
            game(Some("ROLE QUEUE"), "Support", 40),
            game(None, "Tank", 20),
            game(Some("role queue"), "Support", 30),
            game(Some("ROLE QUEUE"), "Tank", 50),
            low_queue,
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
                "Open queue, Open Queue",
                "Tank, Role Queue",
                "Support, Role Queue",
                "Tank, Queue unknown",
            ]
        );
        let support = &series[2];
        // Oldest first: m2 (day 8) before m0 (day 10).
        let ids: Vec<&str> = support.points.iter().map(|p| p.match_id.as_str()).collect();
        assert_eq!(ids, ["m2", "m0"]);
        let unknown = &series[3];
        assert_eq!(unknown.queue, None);
        assert_eq!(
            unknown.points.len(),
            2,
            "missing and unsure queue share a bucket"
        );
        assert_eq!(unknown.key(), "q-unknown-r-tank");
        assert_eq!(series[1].key(), "q-role_queue-r-tank");
        // Default pick is the series with the latest game (m0, day 10).
        assert_eq!(
            default_key(&series).as_deref(),
            Some("q-role_queue-r-support")
        );
    }

    #[test]
    fn no_rank_data_means_no_series() {
        let series = rank_series([("a", at(1), None), ("b", at(2), None)]);
        assert!(series.is_empty());
        assert!(chart_series(series).is_empty());
    }

    #[test]
    fn chart_drops_series_without_progress() {
        let r = rank(r#"{"role_rank": {"value": "Gold", "confidence": 0.9}}"#).unwrap();
        let series = rank_series([("a", at(1), Some(&r))]);
        assert_eq!(series.len(), 1);
        assert!(chart_series(series).is_empty());
    }

    #[test]
    fn line_breaks_where_rank_changes() {
        let mk = |tier: &str, div: u8| RankPoint {
            match_id: format!("{tier}{div}"),
            played_at: at(1),
            shown: RankShown {
                tier: Some(tier.into()),
                division: Some(div),
                progress: Some(50),
                ..RankShown::default()
            },
        };
        let pts = [mk("Gold", 2), mk("Gold", 2), mk("Gold", 1), mk("Gold", 1)];
        let refs: Vec<&RankPoint> = pts.iter().collect();
        assert_eq!(line_runs(&refs), vec![0..2, 2..4]);
        assert!(line_runs(&[]).is_empty());
    }

    #[test]
    fn plot_coords_map_progress_to_height() {
        let c = plot_coords(&[0, 100]);
        assert_eq!(c[0], (PAD_L, VIEW_H - PAD_B));
        assert_eq!(c[1], (VIEW_W - PAD_R, PAD_T));
        let one = plot_coords(&[50]);
        assert_eq!(one.len(), 1);
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

    fn render_chart(series: Vec<RankSeries>) -> String {
        let mut dom = VirtualDom::new_with_props(RankChart, RankChartProps { series });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
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
        assert_eq!(html.matches(r#"role="status""#).count(), 1, "{html}");
        assert!(
            html.contains("Showing Support, Role Queue: 1 game"),
            "{html}"
        );
        assert!(
            html.contains(r#"aria-labelledby="rank-chart-picker-label""#),
            "{html}"
        );
        assert!(html.contains(">Tank, Queue unknown</button>"), "{html}");
        assert!(html.contains(r#"aria-pressed="true""#), "{html}");
        assert!(html.contains(r#"role="img""#), "{html}");
        assert!(html.contains("<table"), "{html}");
        assert!(
            html.contains(r#"id="rank-chart-q-role_queue-r-support-table""#),
            "{html}"
        );
    }

    #[test]
    fn chart_with_one_series_has_no_picker_but_keeps_status() {
        let a = game(Some("ROLE QUEUE"), "Support", 40);
        let html = render_chart(chart_series(rank_series([("m1", at(2), Some(&a))])));
        assert!(!html.contains("<button"), "{html}");
        assert!(html.contains(r#"role="status""#), "{html}");
    }

    #[test]
    fn copy_has_no_em_dash_and_no_streamer() {
        let src = include_str!("rank.rs");
        assert!(!src.contains('\u{2014}'));
        assert!(!src.to_lowercase().contains(concat!("the ", "streamer")));
    }
}
