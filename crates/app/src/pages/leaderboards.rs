//! Public leaderboards page.

use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use crate::components::ui::{Card, HeroSelect, Pill, PillTone, SeasonSelect, use_stats_season};
use crate::routes::Route;
use crate::sample::{LIST_RULE_NOTE, ListResult, RATE_MIN_GAMES, list_result};
use crate::util::encode_query;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct LeaderboardRow {
    member_id: String,
    display_name: String,
    games: u32,
    /// Victories in `games`. Older servers omit the three counts.
    #[serde(default)]
    wins: u32,
    /// Defeats in `games`.
    #[serde(default)]
    losses: u32,
    /// Draws in `games`.
    #[serde(default)]
    draws: u32,
    winrate: f32,
    kd: f64,
}

const PAGE_CSS: &str = r#"
    .lb-page {
        padding: 3rem 2rem;
        max-width: 800px;
        margin: 0 auto;
    }
    .lb-page h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        margin: 0 0 0.5rem;
        color: var(--text);
    }
    .lb-sub {
        color: var(--text-3);
        font-size: 0.9rem;
        margin-bottom: 1.5rem;
    }
    .lb-tabs {
        display: flex;
        gap: 0.5rem;
        margin-bottom: 1.25rem;
        flex-wrap: wrap;
    }
    .lb-tab {
        background: var(--surface-2);
        border: 1px solid var(--border);
        color: var(--text-2);
        padding: 0.4rem 0.85rem;
        border-radius: 6px;
        cursor: pointer;
        font-size: 0.85rem;
        font-weight: 600;
    }
    .lb-tab.active {
        background: var(--accent);
        border-color: var(--accent);
        color: var(--accent-fg);
    }
    .lb-status {
        color: var(--text-3);
        text-align: center;
        padding: 2rem 0;
    }
    .lb-table {
        width: 100%;
        border-collapse: collapse;
        font-size: 0.9rem;
    }
    .lb-table th {
        text-align: left;
        font-family: var(--font-mono);
        font-size: 0.68rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--text-3);
        padding: 0.5rem 0.6rem;
        border-bottom: 1px solid var(--border);
    }
    .lb-table td {
        padding: 0.65rem 0.6rem;
        border-bottom: 1px solid var(--border);
        color: var(--text);
    }
    .lb-table a {
        color: var(--accent);
        font-weight: 600;
        text-decoration: none;
    }
    .lb-table a:hover { text-decoration: underline; }
    .lb-rank {
        font-family: var(--font-mono);
        color: var(--text-3);
        width: 2.5rem;
    }
    .lb-wr-muted {
        color: var(--text-3);
        font-size: 0.8rem;
    }
    .lb-rule-note {
        color: var(--text-3);
        font-size: 0.8rem;
        margin: 0.75rem 0 0;
    }
    .lb-unranked {
        margin-top: 1.5rem;
    }
    .lb-unranked h2 {
        font-family: var(--font-head);
        font-size: 1.1rem;
        margin: 0 0 0.25rem;
        color: var(--text);
    }
    .lb-unranked-sub {
        color: var(--text-3);
        font-size: 0.8rem;
        margin: 0 0 0.75rem;
    }
    .lb-num {
        font-family: var(--font-mono);
        font-variant-numeric: tabular-nums;
    }
    .lb-filters { display: flex; gap: 1rem; flex-wrap: wrap; align-items: flex-end; }
    .lb-hero {
        max-width: 280px;
        margin-bottom: 1.25rem;
    }
    .lb-updated {
        color: var(--text-3);
        font-size: 0.8rem;
        margin: -0.75rem 0 1.25rem;
    }
"#;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct LeaderboardPayload {
    rows: Vec<LeaderboardRow>,
    /// Win rate board only: players under the ranking floor, sent by a
    /// newer server. Missing or empty means the page splits `rows` itself.
    #[serde(default)]
    unranked: Vec<LeaderboardRow>,
    /// Missing on an older response. Empty or unparseable hides the label.
    #[serde(default)]
    cached_at: Option<String>,
}

/// "Updated just now" / "Updated 12s ago" / "Updated 3m ago" / "Updated 2h ago".
///
/// A timestamp ahead of `now` (clock skew) reads as just now. Parsing stays
/// in [`parse_cached_at`]; a missing stamp is not passed in.
fn updated_label(
    cached_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let secs = now.signed_duration_since(cached_at).num_seconds().max(0);
    if secs < 5 {
        "Updated just now".into()
    } else if secs < 60 {
        format!("Updated {secs}s ago")
    } else if secs < 3600 {
        format!("Updated {}m ago", secs / 60)
    } else {
        format!("Updated {}h ago", secs / 3600)
    }
}

fn parse_cached_at(raw: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// `Hold` is the skipped request while a saved season is unresolved. It
/// renders as Loading. It is not a finished failure.
#[derive(Debug, PartialEq)]
enum LeaderboardLoad {
    Hold,
    Failed,
    /// Ranked rows, then the server's unranked rows (often empty).
    Rows(Vec<LeaderboardRow>, Vec<LeaderboardRow>),
}

/// `Some(Hold)` when the season filter has not resolved yet. A non-empty
/// path is fetched by the caller.
fn leaderboard_hold(path: &str) -> Option<LeaderboardLoad> {
    if path.is_empty() {
        Some(LeaderboardLoad::Hold)
    } else {
        None
    }
}

/// Status line for the leaderboard card. `None` means render the table.
fn leaderboard_message(
    load: Option<&LeaderboardLoad>,
    hero_filtered: bool,
) -> Option<&'static str> {
    match load {
        None | Some(LeaderboardLoad::Hold) => Some("Loading..."),
        Some(LeaderboardLoad::Failed) => Some("Couldn't load leaderboards."),
        Some(LeaderboardLoad::Rows(list, extra))
            if list.is_empty() && extra.is_empty() && hero_filtered =>
        {
            Some("No ranked matches on this hero yet.")
        }
        Some(LeaderboardLoad::Rows(list, extra)) if list.is_empty() && extra.is_empty() => {
            Some("No ranked matches yet. Upload stats from the tracker.")
        }
        Some(LeaderboardLoad::Rows(..)) => None,
    }
}

/// WR cell under the small-sample rule: the win rate from 10 games, the
/// record below that, muted under 3 games. Display only: the row order
/// is whatever the server sent.
fn wr_cell(row: &LeaderboardRow) -> ListResult {
    list_result(row.games, row.wins, row.losses, row.draws, row.winrate)
}

/// Fewest games to be ranked on the Win rate board. The server still
/// sends rows from `LEADERBOARD_MIN_GAMES` (5); rows below this floor move
/// to the unranked list. The K/D and Games boards are not split.
const WINRATE_RANK_MIN_GAMES: u32 = RATE_MIN_GAMES;

/// Caption under the ranked Win rate table.
const WINRATE_BOARD_NOTE: &str = "Ranked by win rate from 10 games.";
/// Status line when the Win rate board has rows but none with 10 games.
const WINRATE_NONE_RANKED: &str = "Nobody has 10 games yet.";
/// Heading of the unranked list under the Win rate board.
const UNRANKED_TITLE: &str = "Not enough games yet";
/// Caption under the unranked heading.
const UNRANKED_NOTE: &str = "Record so far. Players join the win rate ranking at 10 games.";

/// Rows for one board: `ranked` in the server's order, and `unranked`
/// for the list under the Win rate board.
#[derive(Debug, Clone, PartialEq)]
struct BoardSplit {
    ranked: Vec<LeaderboardRow>,
    unranked: Vec<LeaderboardRow>,
}

/// Split a board's rows. On the Win rate board, rows with fewer than
/// [`WINRATE_RANK_MIN_GAMES`] games leave the ranking and go to
/// `unranked`, joined by the server's own `server_unranked` rows when a
/// newer server sends them. A player shows once: a ranked player is
/// dropped from `unranked`, and a repeat in `unranked` keeps the first
/// copy. `unranked` is sorted by most games, then by name. Ranked rows
/// keep the server's order (by win rate). Other boards keep every row
/// ranked and ignore `server_unranked`.
fn board_split(
    metric: &str,
    rows: &[LeaderboardRow],
    server_unranked: &[LeaderboardRow],
) -> BoardSplit {
    if metric != "winrate" {
        return BoardSplit {
            ranked: rows.to_vec(),
            unranked: Vec::new(),
        };
    }
    let (ranked, below): (Vec<_>, Vec<_>) = rows
        .iter()
        .cloned()
        .partition(|r| r.games >= WINRATE_RANK_MIN_GAMES);
    let mut seen: std::collections::HashSet<String> =
        ranked.iter().map(|r| r.member_id.clone()).collect();
    let mut unranked: Vec<LeaderboardRow> = server_unranked
        .iter()
        .cloned()
        .chain(below)
        .filter(|r| seen.insert(r.member_id.clone()))
        .collect();
    unranked.sort_by(|a, b| {
        b.games
            .cmp(&a.games)
            .then_with(|| {
                a.display_name
                    .to_lowercase()
                    .cmp(&b.display_name.to_lowercase())
            })
            .then_with(|| a.member_id.cmp(&b.member_id))
    });
    BoardSplit { ranked, unranked }
}

/// Caption under a board's ranked table.
fn board_note(metric: &str) -> &'static str {
    if metric == "winrate" {
        WINRATE_BOARD_NOTE
    } else {
        LIST_RULE_NOTE
    }
}

/// Accent pill for a win rate, neutral pill for a record.
fn wr_tone(wr: &ListResult) -> PillTone {
    if wr.is_rate {
        PillTone::Accent
    } else {
        PillTone::Neutral
    }
}

#[component]
pub fn Leaderboards() -> Element {
    let mut metric = use_signal(|| "winrate".to_string());
    let mut hero = use_signal(|| None::<String>);
    // Same saved season as My Stats and member stats, including the
    // "current" sentinel. A pick here changes those pages too.
    let season = use_stats_season();
    // Last stamp that arrived. A filter change and an unresolved season
    // hold do not clear this, so the header keeps its label until the next
    // payload lands. A failed fetch does clear it.
    let mut shown_at = use_signal(|| None::<chrono::DateTime<chrono::Utc>>);
    let mut label_tick = use_signal(|| 0u32);
    // `use_future` spawns on this component's scope. Dioxus drops that task
    // when the page unmounts, which stops the timer.
    let _label_timer = use_future(move || async move {
        loop {
            #[cfg(feature = "web")]
            {
                gloo_timers::future::TimeoutFuture::new(15_000).await;
            }
            #[cfg(not(feature = "web"))]
            {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
            label_tick += 1;
        }
    });
    let rows = use_resource(move || {
        let m = metric();
        let h = hero();
        let mut url = format!("/api/public/leaderboards?metric={m}&limit=50");
        if let Some(h) = h {
            url.push_str(&format!("&hero={}", encode_query(&h)));
        }
        // Empty while a saved season is unresolved. Do not send it raw.
        let url = season.fetch_path(&url);
        async move {
            // The board travels with its rows, so a tab switch never splits
            // the previous board's rows by the new board's rule.
            if let Some(held) = leaderboard_hold(&url) {
                return (m, held);
            }
            let load = match ApiClient::web().fetch::<LeaderboardPayload>(&url).await {
                Ok(payload) => {
                    shown_at.set(parse_cached_at(payload.cached_at.as_deref()));
                    LeaderboardLoad::Rows(payload.rows, payload.unranked)
                }
                Err(_) => {
                    shown_at.set(None);
                    LeaderboardLoad::Failed
                }
            };
            (m, load)
        }
    });
    let (message, split, note) = {
        let snapshot = rows.read();
        let message =
            leaderboard_message(snapshot.as_ref().map(|(_, load)| load), hero().is_some());
        let (split, note) = match snapshot.as_ref() {
            Some((board, LeaderboardLoad::Rows(list, extra))) => {
                (Some(board_split(board, list, extra)), board_note(board))
            }
            _ => (None, LIST_RULE_NOTE),
        };
        (message, split, note)
    };
    // Only a rendered board has an unranked list.
    let unranked = match (message, split.as_ref()) {
        (None, Some(split)) => split.unranked.clone(),
        _ => Vec::new(),
    };
    let _tick = label_tick();
    let updated = match shown_at() {
        Some(at) => updated_label(at, chrono::Utc::now()),
        None => String::new(),
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-leaderboards-page-css", css: PAGE_CSS }
        main { class: "lb-page",
            h1 { "Leaderboards" }
            p { class: "lb-sub", "Ranked from uploaded personal stats (OCR). Sparse data is normal." }
            if !updated.is_empty() {
                p { class: "lb-updated", "{updated}" }
            }

            div { class: "lb-tabs",
                button {
                    class: if metric() == "winrate" { "lb-tab active" } else { "lb-tab" },
                    onclick: move |_| metric.set("winrate".into()),
                    "Win rate"
                }
                button {
                    class: if metric() == "kd" { "lb-tab active" } else { "lb-tab" },
                    onclick: move |_| metric.set("kd".into()),
                    "K/D"
                }
                button {
                    class: if metric() == "games" { "lb-tab active" } else { "lb-tab" },
                    onclick: move |_| metric.set("games".into()),
                    "Games"
                }
            }

            div { class: "lb-filters",
                div { class: "lb-hero",
                    HeroSelect {
                        id: "leaderboard-hero".to_string(),
                        name: "leaderboard-hero".to_string(),
                        label: "Hero".to_string(),
                        value: hero(),
                        onchange: move |h| hero.set(h),
                    }
                }
                div { class: "lb-hero",
                    SeasonSelect {
                        id: "leaderboard-season".to_string(),
                        name: "leaderboard-season".to_string(),
                        label: "Season".to_string(),
                        seasons: season.season_list(),
                        seasons_error: season.seasons_error(),
                        on_retry: move |_| season.retry(),
                        value: season.selected_id(),
                        onchange: move |s| season.choose(s),
                    }
                }
            }

            Card {
                if let Some(message) = message {
                    p { class: "lb-status", "{message}" }
                } else if let Some(split) = split.as_ref() {
                    if split.ranked.is_empty() {
                        p { class: "lb-status", "{WINRATE_NONE_RANKED}" }
                    } else {
                        table { class: "lb-table",
                            thead {
                                tr {
                                    th { "#" }
                                    th { "Player" }
                                    th { "Games" }
                                    th { "WR" }
                                    th { "K/D" }
                                }
                            }
                            tbody {
                                for (i, r) in split.ranked.iter().enumerate() {
                                    {
                                        let rank = i + 1;
                                        let wr = wr_cell(r);
                                        let kd = format!("{:.2}", r.kd);
                                        rsx! {
                                            tr { key: "{r.member_id}",
                                                td { class: "lb-rank", "{rank}" }
                                                td {
                                                    Link {
                                                        to: Route::MemberProfile { id: r.member_id.clone() },
                                                        "{r.display_name}"
                                                    }
                                                }
                                                td { class: "lb-num", "{r.games}" }
                                                td { class: "lb-num",
                                                    if wr.muted {
                                                        span { class: "lb-wr-muted", "{wr.text}" }
                                                    } else {
                                                        Pill { tone: wr_tone(&wr), "{wr.text}" }
                                                    }
                                                }
                                                td { class: "lb-num", "{kd}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        p { class: "lb-rule-note", "{note}" }
                    }
                }
            }

            if !unranked.is_empty() {
                section { class: "lb-unranked",
                    Card {
                        h2 { "{UNRANKED_TITLE}" }
                        p { class: "lb-unranked-sub", "{UNRANKED_NOTE}" }
                        table { class: "lb-table",
                            thead {
                                tr {
                                    th { "Player" }
                                    th { "Games" }
                                    th { "Record" }
                                    th { "K/D" }
                                }
                            }
                            tbody {
                                for r in unranked.iter() {
                                    {
                                        let record = wr_cell(r);
                                        let kd = format!("{:.2}", r.kd);
                                        rsx! {
                                            tr { key: "{r.member_id}",
                                                td {
                                                    Link {
                                                        to: Route::MemberProfile { id: r.member_id.clone() },
                                                        "{r.display_name}"
                                                    }
                                                }
                                                td { class: "lb-num", "{r.games}" }
                                                td { class: "lb-num",
                                                    if record.muted {
                                                        span { class: "lb-wr-muted", "{record.text}" }
                                                    } else {
                                                        Pill { tone: wr_tone(&record), "{record.text}" }
                                                    }
                                                }
                                                td { class: "lb-num", "{kd}" }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn at(secs_from_now: i64, now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
        now + chrono::Duration::seconds(secs_from_now)
    }

    #[test]
    fn filter_controls_pass_stable_id_and_name() {
        let src = include_str!("leaderboards.rs");
        for id in ["leaderboard-hero", "leaderboard-season"] {
            assert!(
                src.contains(&format!("id: \"{id}\".to_string()")),
                "{id} id"
            );
            assert!(
                src.contains(&format!("name: \"{id}\".to_string()")),
                "{id} name"
            );
        }
    }

    #[test]
    fn updated_label_just_now_seconds_and_minutes() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(updated_label(at(0, now), now), "Updated just now");
        assert_eq!(updated_label(at(-4, now), now), "Updated just now");
        assert_eq!(updated_label(at(-5, now), now), "Updated 5s ago");
        assert_eq!(updated_label(at(-59, now), now), "Updated 59s ago");
        assert_eq!(updated_label(at(-60, now), now), "Updated 1m ago");
        assert_eq!(updated_label(at(-125, now), now), "Updated 2m ago");
    }

    #[test]
    fn updated_label_hours_start_at_sixty_minutes() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(updated_label(at(-59 * 60, now), now), "Updated 59m ago");
        assert_eq!(updated_label(at(-60 * 60, now), now), "Updated 1h ago");
        assert_eq!(updated_label(at(-2 * 3600, now), now), "Updated 2h ago");
        assert_eq!(updated_label(at(-5 * 3600, now), now), "Updated 5h ago");
    }

    #[test]
    fn updated_label_future_stamp_reads_as_just_now() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(updated_label(at(30, now), now), "Updated just now");
    }

    #[test]
    fn parse_cached_at_hides_malformed_stamps() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(parse_cached_at(None), None);
        assert_eq!(parse_cached_at(Some("")), None);
        assert_eq!(parse_cached_at(Some("   ")), None);
        assert_eq!(parse_cached_at(Some("not-a-timestamp")), None);
        assert_eq!(parse_cached_at(Some("2026-13-99")), None);
        let raw = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert_eq!(parse_cached_at(Some(&raw)), Some(now));
    }

    #[test]
    fn payload_without_cached_at_still_parses() {
        let payload: LeaderboardPayload = serde_json::from_str(
            r#"{"rows":[{"member_id":"m","display_name":"M","games":1,"winrate":1.0,"kd":1.0}]}"#,
        )
        .unwrap();
        assert!(payload.cached_at.is_none());
        assert_eq!(payload.rows.len(), 1);
        assert_eq!(parse_cached_at(payload.cached_at.as_deref()), None);
    }

    #[test]
    fn unresolved_season_path_holds_instead_of_failing() {
        assert_eq!(
            leaderboard_hold(""),
            Some(LeaderboardLoad::Hold),
            "an empty season path is the unresolved hold, not a failure"
        );
        assert_eq!(
            leaderboard_hold("/api/public/leaderboards?metric=winrate"),
            None
        );
    }

    fn row(games: u32, wins: u32, losses: u32, draws: u32, winrate: f32) -> LeaderboardRow {
        LeaderboardRow {
            member_id: "m".into(),
            display_name: "M".into(),
            games,
            wins,
            losses,
            draws,
            winrate,
            kd: 1.0,
        }
    }

    #[test]
    fn wr_cell_follows_the_small_sample_rule() {
        let two = wr_cell(&row(2, 2, 0, 0, 1.0));
        assert_eq!(two.text, "2W-0L");
        assert!(two.muted);

        let six = wr_cell(&row(6, 3, 2, 1, 0.5));
        assert_eq!(six.text, "3W-2L-1D");
        assert!(!six.muted);
        assert!(matches!(wr_tone(&six), PillTone::Neutral));

        let ten = wr_cell(&row(10, 8, 2, 0, 0.8));
        assert_eq!(ten.text, "80%");
        assert!(!ten.muted);
        assert!(matches!(wr_tone(&ten), PillTone::Accent));
    }

    #[test]
    fn leaderboard_row_reads_the_record_and_older_rows() {
        let new: LeaderboardRow = serde_json::from_str(
            r#"{"member_id":"m","display_name":"M","games":9,"wins":5,"losses":3,"draws":1,"winrate":0.5555556,"kd":1.2}"#,
        )
        .unwrap();
        assert_eq!(wr_cell(&new).text, "5W-3L-1D");

        let old: LeaderboardRow = serde_json::from_str(
            r#"{"member_id":"m","display_name":"M","games":6,"winrate":0.5,"kd":1.0}"#,
        )
        .unwrap();
        assert_eq!((old.wins, old.losses, old.draws), (0, 0, 0));
        assert_eq!(wr_cell(&old).text, "3W");
    }

    fn named(id: &str, name: &str, games: u32, wins: u32, winrate: f32) -> LeaderboardRow {
        LeaderboardRow {
            member_id: id.into(),
            display_name: name.into(),
            games,
            wins,
            losses: games - wins,
            draws: 0,
            winrate,
            kd: 1.0,
        }
    }

    fn ids(rows: &[LeaderboardRow]) -> Vec<&str> {
        rows.iter().map(|r| r.member_id.as_str()).collect()
    }

    #[test]
    fn winrate_board_ranks_from_ten_games_only() {
        let rows = vec![
            named("nine", "Nine", 9, 9, 1.0),
            named("ten", "Ten", 10, 6, 0.6),
        ];
        let split = board_split("winrate", &rows, &[]);
        assert_eq!(ids(&split.ranked), vec!["ten"]);
        assert_eq!(ids(&split.unranked), vec!["nine"]);
        assert_eq!(wr_cell(&split.ranked[0]).text, "60%");
        assert_eq!(wr_cell(&split.unranked[0]).text, "9W-0L");
    }

    #[test]
    fn winrate_board_keeps_server_order_for_ranked_rows() {
        let rows = vec![
            named("a", "A", 12, 9, 0.75),
            named("short", "Short", 5, 5, 1.0),
            named("b", "B", 20, 12, 0.6),
            named("c", "C", 10, 5, 0.5),
        ];
        let split = board_split("winrate", &rows, &[]);
        assert_eq!(ids(&split.ranked), vec!["a", "b", "c"]);
        assert_eq!(ids(&split.unranked), vec!["short"]);
    }

    #[test]
    fn winrate_board_with_ten_plus_only_has_an_empty_unranked_list() {
        let rows = vec![named("a", "A", 10, 7, 0.7), named("b", "B", 31, 15, 0.48)];
        let split = board_split("winrate", &rows, &[]);
        assert_eq!(ids(&split.ranked), vec!["a", "b"]);
        assert!(split.unranked.is_empty());
        assert!(board_split("winrate", &[], &[]).unranked.is_empty());
    }

    #[test]
    fn unranked_list_sorts_by_games_then_name() {
        let rows = vec![
            named("m1", "zed", 6, 3, 0.5),
            named("m2", "Amy", 9, 4, 0.44),
            named("m3", "bob", 6, 2, 0.33),
            named("m4", "Cat", 5, 5, 1.0),
            named("m5", "Ann", 9, 6, 0.67),
        ];
        let split = board_split("winrate", &rows, &[]);
        assert!(split.ranked.is_empty());
        assert_eq!(ids(&split.unranked), vec!["m2", "m5", "m3", "m1", "m4"]);
    }

    #[test]
    fn unranked_rows_show_the_record_or_wins_from_an_older_server() {
        let mut drawn = named("d", "D", 9, 5, 5.0 / 9.0);
        drawn.losses = 3;
        drawn.draws = 1;
        let old: LeaderboardRow = serde_json::from_str(
            r#"{"member_id":"o","display_name":"O","games":7,"winrate":0.5714286,"kd":1.0}"#,
        )
        .unwrap();
        let split = board_split("winrate", &[drawn, old], &[]);
        let texts: Vec<String> = split.unranked.iter().map(|r| wr_cell(r).text).collect();
        assert_eq!(texts, vec!["5W-3L-1D", "4W"]);
        assert!(split.unranked.iter().all(|r| !wr_cell(r).is_rate));
    }

    #[test]
    fn server_unranked_list_is_used_and_ranked_rows_still_need_ten() {
        let rows = vec![
            named("a", "A", 14, 10, 0.71),
            named("short", "Short", 6, 6, 1.0),
        ];
        let server = vec![
            named("s1", "Sam", 7, 4, 0.57),
            named("short", "Short", 6, 6, 1.0),
            named("s2", "Bea", 9, 3, 0.33),
        ];
        let split = board_split("winrate", &rows, &server);
        assert_eq!(ids(&split.ranked), vec!["a"]);
        assert_eq!(ids(&split.unranked), vec!["s2", "s1", "short"]);
    }

    #[test]
    fn nobody_shows_twice_across_ranked_and_unranked() {
        let rows = vec![named("a", "A", 10, 6, 0.6)];
        let server = vec![
            named("a", "A", 10, 6, 0.6),
            named("b", "B", 8, 4, 0.5),
            named("b", "B", 8, 4, 0.5),
        ];
        let split = board_split("winrate", &rows, &server);
        assert_eq!(ids(&split.ranked), vec!["a"]);
        assert_eq!(ids(&split.unranked), vec!["b"]);
    }

    #[test]
    fn payload_reads_unranked_or_defaults_to_empty() {
        let newer: LeaderboardPayload = serde_json::from_str(
            r#"{"rows":[{"member_id":"a","display_name":"A","games":10,"wins":6,"losses":4,"draws":0,"winrate":0.6,"kd":1.0}],"unranked":[{"member_id":"b","display_name":"B","games":9,"wins":5,"losses":3,"draws":1,"winrate":0.5555556,"kd":1.0}]}"#,
        )
        .unwrap();
        assert_eq!(ids(&newer.unranked), vec!["b"]);
        let split = board_split("winrate", &newer.rows, &newer.unranked);
        assert_eq!(wr_cell(&split.unranked[0]).text, "5W-3L-1D");

        let older: LeaderboardPayload = serde_json::from_str(
            r#"{"rows":[{"member_id":"b","display_name":"B","games":9,"winrate":0.5555556,"kd":1.0}]}"#,
        )
        .unwrap();
        assert!(older.unranked.is_empty());
        let split = board_split("winrate", &older.rows, &older.unranked);
        assert!(split.ranked.is_empty());
        assert_eq!(ids(&split.unranked), vec!["b"]);
    }

    #[test]
    fn unranked_only_board_still_renders() {
        assert_eq!(
            leaderboard_message(
                Some(&LeaderboardLoad::Rows(
                    Vec::new(),
                    vec![named("b", "B", 7, 3, 0.43)]
                )),
                false
            ),
            None
        );
    }

    #[test]
    fn kd_and_games_boards_are_not_split() {
        let rows = vec![named("a", "A", 5, 3, 0.6), named("b", "B", 12, 6, 0.5)];
        for metric in ["kd", "games"] {
            let split = board_split(metric, &rows, &[named("x", "X", 6, 3, 0.5)]);
            assert_eq!(ids(&split.ranked), vec!["a", "b"], "{metric}");
            assert!(split.unranked.is_empty(), "{metric}");
            assert_eq!(board_note(metric), LIST_RULE_NOTE);
        }
        assert_eq!(board_note("winrate"), WINRATE_BOARD_NOTE);
    }

    #[test]
    fn winrate_copy_has_no_long_dashes() {
        for text in [
            WINRATE_BOARD_NOTE,
            WINRATE_NONE_RANKED,
            UNRANKED_TITLE,
            UNRANKED_NOTE,
        ] {
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "{text}"
            );
        }
    }

    #[test]
    fn hold_renders_loading_not_the_failure() {
        assert_eq!(leaderboard_message(None, false), Some("Loading..."));
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Hold), false),
            Some("Loading...")
        );
        assert_ne!(
            leaderboard_message(Some(&LeaderboardLoad::Hold), false),
            Some("Couldn't load leaderboards.")
        );
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Failed), false),
            Some("Couldn't load leaderboards.")
        );
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Rows(Vec::new(), Vec::new())), true),
            Some("No ranked matches on this hero yet.")
        );
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Rows(Vec::new(), Vec::new())), false),
            Some("No ranked matches yet. Upload stats from the tracker.")
        );
        assert_eq!(
            leaderboard_message(
                Some(&LeaderboardLoad::Rows(
                    vec![LeaderboardRow {
                        member_id: "m1".into(),
                        display_name: "A".into(),
                        games: 1,
                        wins: 1,
                        losses: 0,
                        draws: 0,
                        winrate: 1.0,
                        kd: 1.0,
                    }],
                    Vec::new()
                )),
                false
            ),
            None
        );
    }
}
