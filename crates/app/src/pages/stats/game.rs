//! One personal game: result, map, mode, hero, and every stat with its unsure
//! mark.
//!
//! There is no single-game endpoint. The page reads the member's own match
//! list (all time, newest first, the same rows History shows) and picks the
//! row by id. When the id is past the loaded pages, "Look further back" raises
//! the page budget instead of guessing.

use chrono::{DateTime, Utc};
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::member_pending;
use crate::hooks::{LIST_MAX_PAGES, use_api_list_with};
use crate::routes::Route;
use crate::state::use_auth;

use super::history::{cell_is_unsure, marked_value, outcome_class, show_new_reader_badge};
use super::role::game_role_label;
use super::{format_date, map_game_mode, member_stats_url};

/// The member's own match list. No season, so every game is in reach.
const MY_MATCHES_PATH: &str = "/api/stats/me/matches";

const EDITED_BADGE: &str = "Edited";
const EDITED_TITLE: &str = "Corrected by hand in the tracker";
const NEW_READER_BADGE: &str = "New reader";
const NEW_READER_TITLE: &str = "Read by the new stat reader";

const GAME_CSS: &str = r#"
    .game-page.stats-page {
        max-width: 760px;
        margin: 0 auto;
        padding: 2rem 1.5rem;
    }
    .game-back {
        display: inline-block;
        margin-bottom: 1rem;
        font-size: 0.85rem;
        color: var(--text-2);
        text-decoration: none;
    }
    .game-back:hover { color: var(--text); }
    .game-head {
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
        padding: 1.1rem 1.25rem;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
    }
    .game-outcome {
        font-family: var(--font-head);
        font-size: 1rem;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .game-page .game-title {
        font-family: var(--font-head);
        font-size: 1.6rem;
        margin: 0;
        color: var(--text);
    }
    .game-meta {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.35rem 0.6rem;
        margin: 0;
        font-size: 0.9rem;
        color: var(--text-2);
    }
    .game-badges {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        margin-top: 0.25rem;
    }
    .game-stats {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 0.6rem;
        margin: 1rem 0 0;
    }
    .game-stat {
        padding: 0.75rem 0.9rem;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        min-width: 0;
    }
    .game-stat dt {
        font-size: 0.7rem;
        font-weight: 600;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--text-3);
    }
    .game-stat dd {
        margin: 0.2rem 0 0;
        font-size: 1.35rem;
        font-weight: 700;
        font-variant-numeric: tabular-nums;
        color: var(--text);
    }
    .game-unsure-note {
        margin: 0.9rem 0 0;
        font-size: 0.85rem;
        color: var(--text-2);
    }
    @media (max-width: 520px) {
        .game-stats { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    }
"#;

/// One row of `GET /api/stats/me/matches`, with the fields this page shows.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(super) struct GameRow {
    id: String,
    hero: String,
    map_name: String,
    #[serde(default)]
    game_mode: String,
    #[serde(default)]
    role: String,
    outcome: String,
    elims: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    healing: u32,
    /// Rows from before mitigation was returned read as 0.
    #[serde(default)]
    mitigation: u32,
    played_at: DateTime<Utc>,
    /// True when a value was corrected by hand in the tracker.
    #[serde(default)]
    edited: bool,
    /// Reader id. Missing or null reads as `ocr-v1`.
    #[serde(default)]
    recognizer: Option<String>,
    /// Cell names the reader flagged. Missing or null means no marks.
    #[serde(default)]
    suspect_fields: Option<Vec<String>>,
}

impl GameRow {
    fn fields(&self) -> &[String] {
        self.suspect_fields.as_deref().unwrap_or(&[])
    }
}

/// App path for one game. History rows link here.
pub(super) fn match_path(id: &str) -> String {
    Route::StatsMatch { id: id.to_string() }.to_string()
}

/// A plain primary click stays in the app. Modifier or middle clicks keep the
/// browser default (new tab or window), like the router's own links.
fn opens_in_app(no_modifiers: bool, primary: bool) -> bool {
    no_modifiers && primary
}

/// Click handler for a History row anchor. History rows render without a
/// router in tests, so they use a plain anchor plus this instead of Link.
pub(super) fn open_match(evt: MouseEvent, id: String) {
    let no_modifiers = evt.modifiers().is_empty();
    let primary = evt.trigger_button() == Some(MouseButton::Primary);
    if opens_in_app(no_modifiers, primary) {
        evt.prevent_default();
        navigator().push(Route::StatsMatch { id });
    }
}

/// Where the lookup stands for the loaded rows.
#[derive(Debug, PartialEq)]
enum Lookup<'a> {
    Found(&'a GameRow),
    /// Not in the loaded rows, but more pages are left.
    LookFurther,
    /// Not in the member's match list at all.
    Missing,
}

fn lookup<'a>(rows: &'a [GameRow], id: &str, truncated: bool) -> Lookup<'a> {
    match rows.iter().find(|row| row.id == id) {
        Some(row) => Lookup::Found(row),
        None if truncated => Lookup::LookFurther,
        None => Lookup::Missing,
    }
}

/// Known cells the reader flagged, each counted once.
fn unsure_count(fields: &[String]) -> usize {
    scuffed_types::SUSPECT_FIELD_NAMES
        .iter()
        .filter(|name| cell_is_unsure(fields, name))
        .count()
}

/// One line under the stat grid, or None when nothing is unsure.
fn unsure_summary(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some(
            "1 unsure read. The question mark shows which one. Check and edit if wrong.".into(),
        ),
        n => Some(format!(
            "{n} unsure reads. Question marks show which ones. Check and edit if wrong."
        )),
    }
}

/// One numeric stat on the page.
#[derive(Debug, Clone, PartialEq)]
struct StatLine {
    label: &'static str,
    value: u32,
    unsure: bool,
}

/// Stats in scoreboard order: E, A, D, then damage, healing, mitigation.
fn stat_lines(row: &GameRow) -> Vec<StatLine> {
    let fields = row.fields();
    [
        ("Eliminations", "e", row.elims),
        ("Assists", "a", row.assists),
        ("Deaths", "d", row.deaths),
        ("Damage", "dmg", row.damage),
        ("Healing", "h", row.healing),
        ("Mitigation", "mit", row.mitigation),
    ]
    .into_iter()
    .map(|(label, field, value)| StatLine {
        label,
        value,
        unsure: cell_is_unsure(fields, field),
    })
    .collect()
}

fn map_label(map_name: &str) -> String {
    let name = map_name.trim();
    if name.is_empty() {
        "Unknown map".to_string()
    } else {
        name.to_string()
    }
}

/// Stored mode, or the mode the map implies for rows that left it blank.
fn mode_label(game_mode: &str, map_name: &str) -> String {
    let mode = game_mode.trim();
    if mode.is_empty() {
        map_game_mode(map_name).to_string()
    } else {
        mode.to_string()
    }
}

/// The game body. No Link inside, so tests can render it without a router.
fn game_detail(row: &GameRow) -> Element {
    let fields = row.fields();
    let oc = outcome_class(&row.outcome);
    let map = map_label(&row.map_name);
    let mode = mode_label(&row.game_mode, &row.map_name);
    let role = game_role_label(&row.role, &row.hero);
    let date = format_date(&row.played_at);
    let abs = row.played_at.format("%Y-%m-%d %H:%M UTC").to_string();
    let machine_date = row.played_at.to_rfc3339();
    let new_reader = show_new_reader_badge(row.recognizer.as_deref().unwrap_or(""));
    let edited = row.edited;
    let stats = stat_lines(row);
    let summary = unsure_summary(unsure_count(fields)).unwrap_or_default();
    rsx! {
        header { class: "game-head",
            {marked_value(
                &format!("game-outcome {oc}"),
                &row.outcome,
                "Result",
                false,
                cell_is_unsure(fields, "result"),
            )}
            h1 { class: "game-title",
                {marked_value("", &map, "Map", false, cell_is_unsure(fields, "map"))}
            }
            p { class: "game-meta",
                {marked_value("game-mode", &mode, "Mode", false, cell_is_unsure(fields, "mode"))}
                span { aria_hidden: "true", "·" }
                {marked_value("game-hero", &row.hero, "Hero", false, cell_is_unsure(fields, "hero"))}
                span { aria_hidden: "true", "·" }
                span { class: "game-role", "{role}" }
                span { aria_hidden: "true", "·" }
                time { class: "game-date", datetime: "{machine_date}", title: "{abs}", "{date}" }
            }
            if new_reader || edited {
                div { class: "game-badges",
                    if new_reader {
                        span { class: "tracker-badge reader-badge", title: NEW_READER_TITLE, "{NEW_READER_BADGE}" }
                    }
                    if edited {
                        span { class: "tracker-badge game-edited-badge", title: EDITED_TITLE, "{EDITED_BADGE}" }
                    }
                }
            }
        }
        dl { class: "game-stats",
            for stat in stats {
                div { class: "game-stat", key: "{stat.label}",
                    dt { "{stat.label}" }
                    dd {
                        {marked_value("game-stat-value", &stat.value.to_string(), stat.label, false, stat.unsure)}
                    }
                }
            }
        }
        if !summary.is_empty() {
            p { class: "game-unsure-note", "{summary}" }
        }
    }
}

/// `/stats/match/:id`: one of the signed-in member's own games.
#[component]
pub fn StatsMatch(id: String) -> Element {
    let auth = use_auth();
    let games = use_api_list_with::<GameRow>(move || {
        member_stats_url(&auth(), MY_MATCHES_PATH.to_string())
    });
    let truncated = *games.truncated.read();
    let shown = *games.shown.read();
    let mut budget = games.page_budget;
    let data = games.data.read();
    let rows = data.as_ref().and_then(|d| d.as_ref());

    let body = match rows {
        None => member_pending(&auth(), &games, "this game"),
        Some(rows) => match lookup(rows, &id, truncated) {
            Lookup::Found(row) => game_detail(row),
            Lookup::LookFurther => rsx! {
                div { class: "empty-state",
                    p { "This game is not in your latest {shown} games." }
                    button {
                        class: "ui-btn ui-btn--md",
                        onclick: move |_| budget += LIST_MAX_PAGES,
                        "Look further back"
                    }
                }
            },
            Lookup::Missing => rsx! {
                p { class: "empty-state",
                    "This game is not in your match history. It may have been deleted."
                }
            },
        },
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-stats-mod-stats-css", css: super::STATS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-stats-game-game-css", css: GAME_CSS }
        div { class: "stats-page my-stats game-page",
            Link { class: "game-back", to: Route::Stats {}, "Back to My Stats" }
            {body}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn row(id: &str, suspect: &[&str]) -> GameRow {
        GameRow {
            id: id.to_string(),
            hero: "Ana".to_string(),
            map_name: "Ilios".to_string(),
            game_mode: "control".to_string(),
            role: "Support".to_string(),
            outcome: "victory".to_string(),
            elims: 11,
            deaths: 22,
            assists: 33,
            damage: 4444,
            healing: 5555,
            mitigation: 666,
            played_at: Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap(),
            edited: false,
            recognizer: Some("ocr-v1".to_string()),
            suspect_fields: Some(suspect.iter().map(|s| (*s).to_string()).collect()),
        }
    }

    #[test]
    fn match_route_round_trips() {
        assert_eq!(match_path("abc123"), "/stats/match/abc123");
        assert_eq!(
            "/stats/match/abc123".parse::<Route>().unwrap(),
            Route::StatsMatch {
                id: "abc123".into()
            }
        );
    }

    #[test]
    fn only_a_plain_primary_click_stays_in_the_app() {
        assert!(opens_in_app(true, true));
        assert!(!opens_in_app(false, true));
        assert!(!opens_in_app(true, false));
        assert!(!opens_in_app(false, false));
    }

    #[test]
    fn lookup_finds_by_id_and_knows_when_to_look_further() {
        let rows = vec![row("a", &[]), row("b", &[])];
        assert_eq!(lookup(&rows, "b", false), Lookup::Found(&rows[1]));
        assert_eq!(lookup(&rows, "b", true), Lookup::Found(&rows[1]));
        assert_eq!(lookup(&rows, "z", true), Lookup::LookFurther);
        assert_eq!(lookup(&rows, "z", false), Lookup::Missing);
        assert_eq!(lookup(&[], "a", false), Lookup::Missing);
    }

    #[test]
    fn unsure_count_counts_known_cells_once() {
        let fields: Vec<String> = ["e", "e", "map", "nope", "r3.dmg"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(unsure_count(&fields), 2);
        assert_eq!(unsure_count(&[]), 0);
        let all: Vec<String> = scuffed_types::SUSPECT_FIELD_NAMES
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(unsure_count(&all), scuffed_types::SUSPECT_FIELD_NAMES.len());
    }

    #[test]
    fn unsure_summary_is_quiet_when_sure() {
        assert_eq!(unsure_summary(0), None);
        assert!(unsure_summary(1).unwrap().starts_with("1 unsure read."));
        assert!(unsure_summary(3).unwrap().starts_with("3 unsure reads."));
    }

    #[test]
    fn stat_lines_follow_scoreboard_order_and_carry_marks() {
        let lines = stat_lines(&row("a", &["d", "mit", "map"]));
        let labels: Vec<_> = lines.iter().map(|l| l.label).collect();
        assert_eq!(
            labels,
            [
                "Eliminations",
                "Assists",
                "Deaths",
                "Damage",
                "Healing",
                "Mitigation"
            ]
        );
        let values: Vec<_> = lines.iter().map(|l| l.value).collect();
        assert_eq!(values, [11, 33, 22, 4444, 5555, 666]);
        let unsure: Vec<_> = lines.iter().filter(|l| l.unsure).map(|l| l.label).collect();
        assert_eq!(unsure, ["Deaths", "Mitigation"]);
    }

    #[test]
    fn labels_fill_blank_map_and_mode() {
        assert_eq!(map_label("  "), "Unknown map");
        assert_eq!(map_label("Ilios"), "Ilios");
        assert_eq!(mode_label("push", "Ilios"), "push");
        assert_eq!(mode_label("", "Ilios"), map_game_mode("Ilios"));
    }

    #[test]
    fn match_list_json_deserializes_with_extra_and_missing_fields() {
        let json = r#"{
            "id": "m1", "member_id": "u1", "session_id": "",
            "hero": "Ana", "map_name": "Ilios", "game_mode": "control",
            "role": "Support", "outcome": "defeat",
            "elims": 1, "deaths": 2, "assists": 3, "damage": 4, "healing": 5,
            "played_at": "2026-03-15T12:00:00Z", "uploaded_at": "2026-03-15T12:10:00Z",
            "edited": true, "recognizer": "cv-v1", "name_recognizer": null,
            "suspect_fields": ["h"]
        }"#;
        let parsed: GameRow = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.mitigation, 0);
        assert!(parsed.edited);
        assert_eq!(parsed.fields(), ["h".to_string()]);
    }

    fn render(row: GameRow) -> String {
        let mut dom = VirtualDom::new_with_props(|props: GameRow| game_detail(&props), row);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn page_shows_every_stat_and_marks_only_unsure_ones() {
        let html = render(row("a", &["hero", "dmg", "nope"]));
        for text in [
            "victory", "Ilios", "control", "Ana", "Support", "11", "33", "22", "4444", "5555",
            "666",
        ] {
            assert!(html.contains(text), "{text} missing: {html}");
        }
        assert_eq!(html.matches("stat-unsure-mark").count(), 2, "{html}");
        assert!(html.contains("title=\"Hero: unsure read."), "{html}");
        assert!(html.contains("title=\"Damage: unsure read."), "{html}");
        assert!(html.contains("2 unsure reads."), "{html}");
        assert!(html.contains("outcome-win"), "{html}");
        assert!(
            !html.contains(NEW_READER_BADGE),
            "ocr-v1 has no badge: {html}"
        );
        assert!(!html.contains(EDITED_BADGE), "{html}");
        assert!(
            !html.contains('\u{2014}') && !html.contains('\u{2013}'),
            "{html}"
        );
    }

    #[test]
    fn page_shows_reader_and_edited_badges_and_no_note_when_sure() {
        let mut game = row("a", &[]);
        game.recognizer = Some("cv-v1".to_string());
        game.edited = true;
        game.outcome = "defeat".to_string();
        let html = render(game);
        assert!(html.contains(NEW_READER_BADGE), "{html}");
        assert!(html.contains(EDITED_BADGE), "{html}");
        assert!(html.contains("outcome-loss"), "{html}");
        assert!(!html.contains("stat-unsure-mark"), "{html}");
        assert!(!html.contains("unsure read"), "{html}");
    }
}
