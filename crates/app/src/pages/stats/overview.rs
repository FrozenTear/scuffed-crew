use chrono::{DateTime, Datelike, Utc};
use dioxus::prelude::*;

use scuffed_types::RoleStats;

use crate::components::ui::{Pill, PillTone, SkeletonBars, btn_class};
use crate::hooks::ApiResource;

use super::game::{match_path, open_match};
use super::history::outcome_word;
use super::history_filter::{Outcome, outcome_kind};
use super::rank::{RankChart, RankChartLoading, RankWindow, chart_series, rank_series_with_outcomes};
use super::role::{RolePanel, role_aggs_from_rows, role_panel, role_result, role_shares};
use super::{HeroStats, MapStats, MatchPage, PersonalMatch, load_error_state, map_game_mode};
use crate::sample::Tally;

/// Rows listed under "Recent form". The fetch is the last 10 games; the
/// sentence above the list counts all of them.
const FORM_ROWS: usize = 5;

/// Role-card result text: plain text tokens (no traffic light); a role
/// without a win rate yet (under 10 decided games) renders muted.
fn wr_value_class(tally: Tally) -> &'static str {
    if !tally.has_win_rate() {
        "role-card-wr muted"
    } else {
        "role-card-wr"
    }
}

/// Soft-fill tone of a "Recent form" badge. The badge always carries the
/// word from [`outcome_word`], so the result is never colour alone.
fn outcome_tone(result: Outcome) -> PillTone {
    match result {
        Outcome::Win => PillTone::Ok,
        Outcome::Loss => PillTone::Danger,
        Outcome::Draw => PillTone::Neutral,
    }
}

/// "Last 10: 6 wins, 3 losses, 1 draw." over every fetched game.
fn form_summary(rows: &[PersonalMatch]) -> String {
    let count = |want: Outcome| {
        rows.iter()
            .filter(|m| outcome_kind(&m.outcome) == want)
            .count()
    };
    let part = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let (wins, losses, draws) = (
        count(Outcome::Win),
        count(Outcome::Loss),
        count(Outcome::Draw),
    );
    let mut parts = vec![part(wins, "win", "wins"), part(losses, "loss", "losses")];
    if draws > 0 {
        parts.push(part(draws, "draw", "draws"));
    }
    let span = if rows.len() == 1 {
        "Last game".to_string()
    } else {
        format!("Last {} games", rows.len())
    };
    format!("{span}: {}.", parts.join(", "))
}

/// Date as the frames write it: `12 Nov`, with the year once it is not `now`'s.
/// The day is the viewer's own, like History and the rank chart.
fn form_date(played_at: &DateTime<Utc>, now: &DateTime<Utc>) -> String {
    let day = super::local_day(*played_at);
    if day.year() == super::local_day(*now).year() {
        day.format("%-d %b").to_string()
    } else {
        day.format("%-d %b %Y").to_string()
    }
}

/// Mode aggregate for the overview tiles. `result` follows the small-sample
/// rule: the win rate from 10 decided games, the record below that.
struct ModeChip {
    name: &'static str,
    matches: u32,
    result: String,
}

fn mode_chips(maps: &[MapStats]) -> Vec<ModeChip> {
    const ORDER: &[&str] = &["Escort", "Hybrid", "Control", "Push", "Flashpoint", "Clash"];
    let mut out = Vec::new();
    for &mode in ORDER {
        let total = Tally::sum(
            maps.iter()
                .filter(|map| map_game_mode(&map.map_name) == mode)
                .map(|map| map.tally()),
        );
        if total.games > 0 {
            out.push(ModeChip {
                name: mode,
                matches: total.games,
                result: total.label(),
            });
        }
    }
    out
}

/// "1 game" or "12 games".
fn games_text(n: u32) -> String {
    if n == 1 {
        "1 game".to_string()
    } else {
        format!("{n} games")
    }
}

/// The Overview tab: rank progress, time on role and recent form first (the
/// `d1-stats-signed` frame, under the three summary tiles the page renders),
/// then role results, top heroes and mode results. `on_history` opens the
/// History tab. `player` is the signed-in name the rank section's hidden
/// summary and table caption use.
pub(super) fn overview_tab(
    heroes: ApiResource<Vec<HeroStats>>,
    roles: ApiResource<Vec<RoleStats>>,
    maps: ApiResource<Vec<MapStats>>,
    form: ApiResource<MatchPage>,
    on_history: EventHandler<()>,
    player: Option<String>,
) -> Element {
    let role_err = roles.error.read().clone();
    let role_data = roles.data.read();
    let rows = role_data.as_ref().and_then(|d| d.as_ref());
    let phase = role_panel(role_err.as_deref(), rows.map(|rows| rows.as_slice()));
    let aggs = match phase {
        RolePanel::Ready(rows) => role_aggs_from_rows(rows),
        RolePanel::Pending | RolePanel::Failed => Vec::new(),
    };
    let shares = role_shares(&aggs);
    let has_roles = !shares.is_empty();

    let h_err = heroes.error.read().clone();
    let h_data = heroes.data.read();
    let h_list = h_data.as_ref().and_then(|d| d.as_ref());
    // Ranking needs a win rate, so only heroes with 10+ games.
    let top5: Vec<(&HeroStats, f64)> = h_list
        .map(|list| {
            let mut by_wr: Vec<_> = list
                .iter()
                .filter_map(|h| h.tally().win_rate_pct().map(|wr| (h, wr)))
                .collect();
            by_wr.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            by_wr.into_iter().take(5).collect()
        })
        .unwrap_or_default();

    let maps_err = maps.error.read().is_some();
    let maps_guard = maps.data.read();
    let maps_loaded = maps_guard.as_ref().and_then(|d| d.as_ref());
    // A held fetch finishes as `Some(None)`. Flattening treats that as still
    // loading, the same way the heroes tab does. An empty list is the only
    // "no data" state.
    let maps_waiting = maps_loaded.is_none();
    let maps_owned: Vec<MapStats> = maps_loaded.cloned().unwrap_or_default();
    let chips = mode_chips(&maps_owned);
    drop(maps_guard);

    let form_err = form.error.read().is_some();
    let form_guard = form.data.read();
    let form_loaded = form_guard.as_ref().and_then(|d| d.as_ref());
    let form_waiting = form_loaded.is_none();
    let form_rows: Vec<PersonalMatch> = form_loaded.map(|p| p.data.clone()).unwrap_or_default();
    drop(form_guard);
    // Empty when no loaded game has a rank: the section then shows its own
    // "No rank read" state, as the rank-empty frame draws it. The outcome word
    // lets the readout and table say Win or Loss.
    let rank_chart = chart_series(rank_series_with_outcomes(form_rows.iter().map(|m| {
        (
            m.id.as_str(),
            m.played_at,
            m.rank.as_ref(),
            Some(m.outcome.as_str()),
        )
    })));
    let summary = form_summary(&form_rows);
    let now = Utc::now();

    // One line for assistive tech, always in the page: it says "loading"
    // while any block still waits and is empty once every block has settled.
    let waiting = matches!(phase, RolePanel::Pending)
        || (h_err.is_none() && h_list.is_none())
        || (!maps_err && maps_waiting)
        || (!form_err && form_waiting);
    let status = if waiting { "Loading your overview" } else { "" };

    rsx! {
        div { class: "stats-overview",
            p { class: "visually-hidden", role: "status", "{status}" }

            // Rank progress, directly under the summary tiles. A failed match
            // fetch leaves it out: "Recent form" carries the error and Retry,
            // and an empty chart would wrongly say there are no ranked games.
            if !form_waiting {
                RankChart { series: rank_chart, window: RankWindow::Recent(form_rows.len()), player }
            } else if !form_err {
                RankChartLoading {}
            }

            section { class: "ui-stack", aria_labelledby: "stats-roles-title",
                h2 { class: "ui-section-title", id: "stats-roles-title", "Time on role" }
                match phase {
                    RolePanel::Pending => rsx! {
                        p { class: "stats-state is-loading", "Loading role breakdown..." }
                        SkeletonBars { rows: 3 }
                    },
                    RolePanel::Failed => load_error_state("role breakdown", roles.refresh),
                    RolePanel::Ready(_) if !has_roles => rsx! {
                        p { class: "stats-state is-empty", "No role stats yet." }
                    },
                    RolePanel::Ready(_) => rsx! {
                        div { class: "stats-card role-split",
                            for share in shares.iter() {
                                div { class: "role-row", key: "{share.name}",
                                    span { class: "role-name", "{share.name}" }
                                    div { class: "role-track", aria_hidden: "true",
                                        div { class: "role-fill", style: "width:{share.pct}%" }
                                    }
                                    span { class: "role-pct", "{share.pct}%" }
                                }
                            }
                            p { class: "role-split-note", "Share of your logged games on each role." }
                        }
                    },
                }
            }

            section { class: "ui-stack", aria_labelledby: "stats-form-title",
                h2 { class: "ui-section-title", id: "stats-form-title", "Recent form" }
                if form_rows.is_empty() {
                    if form_err {
                        {load_error_state("recent matches", form.refresh)}
                    } else if form_waiting {
                        p { class: "stats-state is-loading", "Loading form..." }
                        SkeletonBars { rows: 3 }
                    } else {
                        p { class: "stats-state is-empty", "No recent matches." }
                    }
                } else {
                    p { class: "form-summary", "{summary}" }
                    div { class: "form-list",
                        for m in form_rows.iter().take(FORM_ROWS) {
                            {form_row(m, &now)}
                        }
                    }
                    div { class: "form-more",
                        button {
                            r#type: "button",
                            class: btn_class(false),
                            onclick: move |_| on_history.call(()),
                            "See all matches"
                        }
                    }
                }
            }

            div { class: "overview-grid",
                if has_roles {
                    section { class: "ui-stack", aria_labelledby: "stats-role-results-title",
                        h2 { class: "ui-section-title", id: "stats-role-results-title", "Role results" }
                        div { class: "role-cards",
                            for r in aggs.iter().filter(|r| r.matches > 0) {
                                {
                                    let tally = r.tally();
                                    let result = role_result(tally);
                                    let wr_cls = wr_value_class(tally);
                                    let border_color = r.color;
                                    let name = r.name;
                                    let games = games_text(r.matches);
                                    let avg_e = r.avg_elims();
                                    let avg_d = r.avg_deaths();
                                    let avg_dmg = r.avg_damage();
                                    let avg_heal = r.avg_healing();
                                    rsx! {
                                        div {
                                            class: "stats-card role-card",
                                            key: "{name}",
                                            style: "border-inline-start-color:{border_color};",
                                            div { class: "role-card-info",
                                                div { class: "role-card-name", "{name}" }
                                                div { class: "role-card-sub",
                                                    "{games} · {avg_e:.1} elims, {avg_d:.1} deaths · {avg_dmg:.0} damage, {avg_heal:.0} healing"
                                                }
                                            }
                                            div { class: "{wr_cls}", "{result}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                section { class: "ui-stack", aria_labelledby: "stats-top-heroes-title",
                    h2 { class: "ui-section-title", id: "stats-top-heroes-title", "Top heroes" }
                    if h_err.is_some() {
                        {load_error_state("hero rankings", heroes.refresh)}
                    } else if h_list.is_none() {
                        p { class: "stats-state is-loading", "Loading hero rankings..." }
                    } else if top5.is_empty() {
                        p { class: "stats-state is-empty", "Need 10+ games on a hero for rankings." }
                    } else {
                        div { class: "stats-card mini-hero-list",
                            for &(h, wr) in top5.iter() {
                                div { class: "mini-hero-row", key: "{h.hero}",
                                    span { class: "mini-hero-name", "{h.hero}" }
                                    span { class: "mini-hero-meta", "{games_text(h.matches)}" }
                                    span { class: "mini-hero-wr", "{wr:.0}%" }
                                }
                            }
                            p { class: "role-split-note", "By win rate, from 10 games on a hero." }
                        }
                    }
                }
                section { class: "ui-stack", aria_labelledby: "stats-modes-title",
                    h2 { class: "ui-section-title", id: "stats-modes-title", "Mode win rates" }
                    if chips.is_empty() {
                        if maps_err {
                            {load_error_state("maps", maps.refresh)}
                        } else if maps_waiting {
                            p { class: "stats-state is-loading", "Loading maps..." }
                        } else {
                            p { class: "stats-state is-empty", "No map data yet." }
                        }
                    } else {
                        div { class: "mode-tiles",
                            for c in chips.iter() {
                                div { class: "stats-card mode-tile", key: "{c.name}",
                                    span { class: "mode-tile-name", "{c.name}" }
                                    span { class: "mode-tile-wr", "{c.result}" }
                                    span { class: "mode-tile-n", "{games_text(c.matches)}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One "Recent form" card: the map as a link to the game, the date, the
/// result as a word and the hero, then the same word in a soft-fill badge.
/// The ids come from the game's record id.
fn form_row(m: &PersonalMatch, now: &DateTime<Utc>) -> Element {
    let result = outcome_kind(&m.outcome);
    let word = outcome_word(&m.outcome);
    let date = form_date(&m.played_at, now);
    let iso = m.played_at.format("%Y-%m-%d").to_string();
    let title_id = format!("form-{}-title", m.id);
    let map = if m.map_name.trim().is_empty() {
        "Unknown map"
    } else {
        m.map_name.as_str()
    };
    let hero = m.hero.trim();
    let href = match_path(&m.id);
    let target_id = m.id.clone();
    rsx! {
        article { class: "stats-card form-row", key: "{m.id}", aria_labelledby: "{title_id}",
            div { class: "form-row-main",
                h3 { class: "form-row-title", id: "{title_id}",
                    // A plain anchor plus `open_match`, like the History rows.
                    a {
                        href: "{href}",
                        onclick: move |evt: MouseEvent| open_match(evt, target_id.clone()),
                        "{map}"
                    }
                }
                p { class: "form-row-meta",
                    time { datetime: "{iso}", "{date}" }
                    " · {word}"
                    if !hero.is_empty() {
                        " as {hero}"
                    }
                }
            }
            Pill { tone: outcome_tone(result), "{word}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ApiResource;

    fn resource_of<T: Clone + 'static>(value: Option<T>) -> ApiResource<T> {
        let refresh = use_signal(|| 0u64);
        let error = use_signal(|| None::<String>);
        let truncated = use_signal(|| false);
        let shown = use_signal(|| 0usize);
        let page_budget = use_signal(|| 1usize);
        let data = use_resource(move || {
            let value = value.clone();
            async move { value }
        });
        ApiResource {
            data,
            refresh,
            error,
            truncated,
            shown,
            page_budget,
        }
    }

    fn pump(dom: &mut VirtualDom) {
        for _ in 0..8 {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
    }

    fn game(id: &str, outcome: &str, map: &str, hero: &str, played_at: &str) -> PersonalMatch {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "hero": hero,
            "map_name": map,
            "game_mode": "control",
            "role": "Support",
            "outcome": outcome,
            "elims": 1,
            "deaths": 2,
            "assists": 3,
            "damage": 4,
            "healing": 5,
            "played_at": played_at,
        }))
        .expect("match row")
    }

    fn role_row(role: &str, matches: u32) -> RoleStats {
        RoleStats {
            role: role.to_string(),
            matches,
            wins: matches / 2,
            losses: matches - matches / 2,
            draws: 0,
            avg_elims: 8.0,
            avg_deaths: 4.0,
            avg_damage: 1000.0,
            avg_healing: 500.0,
        }
    }

    fn render(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        pump(&mut dom);
        dioxus_ssr::render(&dom)
    }

    /// The overview without the "Rank progress" block, which is the rank
    /// component's own markup (see `rank.rs`) and sits between the status
    /// line and "Time on role".
    fn own(html: &str) -> String {
        let roles = html
            .find("<section class=\"ui-stack\" aria-labelledby=\"stats-roles-title\"")
            .expect("time on role");
        let head_end = html.find("</p>").expect("status line") + "</p>".len();
        assert!(head_end <= roles, "{html}");
        format!("{}{}", &html[..head_end], &html[roles..])
    }

    fn populated() -> Element {
        let heroes = resource_of(Some(Vec::<HeroStats>::new()));
        let roles = resource_of(Some(vec![
            role_row("Tank", 8),
            role_row("Damage", 5),
            role_row("Support", 7),
        ]));
        let maps = resource_of(Some(Vec::<MapStats>::new()));
        let form = resource_of(Some(MatchPage {
            data: vec![
                game("m-win", "victory", "Ilios", "Ana", "2026-11-12T19:00:00Z"),
                game(
                    "m-loss",
                    "defeat",
                    "Busan",
                    "Kiriko",
                    "2026-11-10T19:00:00Z",
                ),
                game("m-draw", "draw", "Oasis", "", "2026-11-08T19:00:00Z"),
            ],
            next_cursor: None,
        }));
        overview_tab(
            heroes,
            roles,
            maps,
            form,
            EventHandler::new(|_| {}),
            Some("Player01".to_string()),
        )
    }

    #[test]
    fn held_overview_fetches_stay_on_loading_text() {
        fn view() -> Element {
            let heroes = resource_of(None::<Vec<HeroStats>>);
            let roles = resource_of(None::<Vec<RoleStats>>);
            let maps = resource_of(None::<Vec<MapStats>>);
            let form = resource_of(None::<MatchPage>);
            overview_tab(heroes, roles, maps, form, EventHandler::new(|_| {}), None)
        }

        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        pump(&mut dom);
        let html = dioxus_ssr::render(&dom);
        assert!(
            html.contains("Loading maps"),
            "a finished held maps fetch must stay on loading: {html}"
        );
        assert!(
            html.contains("Loading form"),
            "a finished held form fetch must stay on loading: {html}"
        );
        assert!(
            !html.contains("No map data yet."),
            "held maps must not look empty: {html}"
        );
        assert!(
            !html.contains("No recent matches."),
            "held form must not look empty: {html}"
        );
        assert!(
            html.contains("stats-state is-loading"),
            "a held fetch uses the loading class: {html}"
        );
        assert!(
            !html.contains("stats-state is-empty"),
            "a held fetch must not use the empty class: {html}"
        );
        assert!(
            html.contains("<p class=\"visually-hidden\" role=\"status\">Loading your overview</p>"),
            "the status line says the overview is loading: {html}"
        );
        assert!(
            html.contains("aria-busy=\"true\"") && html.contains("Loading rank progress."),
            "rank progress shows its loading state while the matches load: {html}"
        );
        assert!(!html.contains("No ranked games in"), "{html}");
    }

    #[test]
    fn rank_progress_sits_under_the_tiles_with_the_player_name_or_drops_out_on_error() {
        // Loaded with no ranked game: the section's own empty state, named.
        let full = render(populated);
        assert!(
            full.contains("Rank progress for Player01. There are no ranked games in your last"),
            "{full}"
        );
        let rank = full.find("class=\"rank-block\"").expect("rank section");
        let roles = full.find("Time on role").unwrap();
        assert!(rank < roles, "{full}");

        // A failed match fetch: Recent form shows the error and Retry, and
        // no empty or loading rank section claims anything.
        fn failed() -> Element {
            let heroes = resource_of(Some(Vec::<HeroStats>::new()));
            let roles = resource_of(Some(Vec::<RoleStats>::new()));
            let maps = resource_of(Some(Vec::<MapStats>::new()));
            let failed_error = use_signal(|| Some("HTTP error 500: Internal error".to_string()));
            let form = ApiResource {
                error: failed_error,
                ..resource_of(None::<MatchPage>)
            };
            overview_tab(heroes, roles, maps, form, EventHandler::new(|_| {}), None)
        }
        let html = render(failed);
        assert!(!html.contains("rank-block"), "{html}");
        assert!(html.contains("load recent matches."), "{html}");
        assert!(html.contains(">Retry</button>"), "{html}");
    }

    #[test]
    fn empty_overview_lists_say_there_is_no_data() {
        fn view() -> Element {
            let heroes = resource_of(Some(Vec::<HeroStats>::new()));
            let roles = resource_of(Some(Vec::<RoleStats>::new()));
            let maps = resource_of(Some(Vec::<MapStats>::new()));
            let form = resource_of(Some(MatchPage {
                data: Vec::new(),
                next_cursor: None,
            }));
            overview_tab(heroes, roles, maps, form, EventHandler::new(|_| {}), None)
        }

        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        pump(&mut dom);
        let html = own(&dioxus_ssr::render(&dom));
        assert!(html.contains("No map data yet."), "{html}");
        assert!(html.contains("No recent matches."), "{html}");
        assert!(html.contains("No role stats yet."), "{html}");
        assert!(html.contains("stats-state is-empty"), "{html}");
        assert!(!html.contains("Loading maps"), "{html}");
        assert!(!html.contains("Loading form"), "{html}");
        // The live region stays in the page; only its text goes away.
        assert!(
            html.contains("<p class=\"visually-hidden\" role=\"status\"></p>"),
            "{html}"
        );
        assert_eq!(html.matches("role=\"status\"").count(), 1, "{html}");
    }

    #[test]
    fn overview_has_no_h1_no_main_and_h2_section_titles() {
        let full = render(populated);
        assert!(
            full.find("class=\"rank-").expect("rank progress") < full.find("Time on role").unwrap(),
            "rank progress sits first, under the summary tiles: {full}"
        );
        let html = own(&full);
        assert!(!html.contains("<h1"), "the page header owns the H1: {html}");
        assert!(!html.contains("<main"), "the shell owns main: {html}");
        for title in [
            "Time on role",
            "Recent form",
            "Role results",
            "Top heroes",
            "Mode win rates",
        ] {
            assert!(
                html.contains(&format!("\">{title}</h2>")),
                "{title} is an h2: {html}"
            );
        }
        assert_eq!(
            html.matches("<h2 class=\"ui-section-title\"").count(),
            5,
            "every overview block uses the shared section title: {html}"
        );
        // The frame order: time on role, then recent form, then the rest.
        let roles = html.find("Time on role").unwrap();
        let form = html.find("Recent form").unwrap();
        let rest = html.find("Role results").unwrap();
        assert!(roles < form && form < rest, "{html}");
    }

    #[test]
    fn time_on_role_rows_carry_a_label_and_a_percent() {
        let html = render(populated);
        for (role, pct) in [("Tank", 40), ("Damage", 25), ("Support", 35)] {
            let row = format!(
                "<span class=\"role-name\">{role}</span>\
                 <div class=\"role-track\" aria-hidden=\"true\">\
                 <div class=\"role-fill\" style=\"width:{pct}%\"></div></div>\
                 <span class=\"role-pct\">{pct}%</span>"
            );
            assert!(html.contains(&row), "{role}: {html}");
        }
        assert!(!html.contains(">Unknown<"), "no empty Unknown row: {html}");
    }

    #[test]
    fn recent_form_says_win_or_loss_in_words_with_ids_from_the_game() {
        let html = render(populated);
        assert!(
            html.contains("Last 3 games: 1 win, 1 loss, 1 draw."),
            "{html}"
        );
        for (id, word, tone) in [
            ("m-win", "Win", "ui-pill--ok"),
            ("m-loss", "Loss", "ui-pill--danger"),
            ("m-draw", "Draw", "ui-pill--neutral"),
        ] {
            let start = html
                .find(&format!("aria-labelledby=\"form-{id}-title\""))
                .unwrap_or_else(|| panic!("row {id}: {html}"));
            let row = &html[start..];
            let row = &row[..row.find("</article>").unwrap()];
            assert!(row.contains(&format!("id=\"form-{id}-title\"")), "{row}");
            assert!(
                row.contains(&format!("href=\"/stats/match/{id}\"")),
                "{row}"
            );
            assert!(
                row.contains(&format!("<span class=\"ui-pill {tone}\">{word}</span>")),
                "the badge carries the word: {row}"
            );
            assert!(
                row.contains(&format!(" · {word}")),
                "the meta line says the result too: {row}"
            );
            assert!(!row.contains("<button"), "no button inside the row: {row}");
        }
        assert!(
            html.contains("<time datetime=\"2026-11-12\">12 Nov"),
            "{html}"
        );
        assert!(html.contains(" · Win as Ana"), "{html}");
        assert!(
            !html.contains(" as </p>"),
            "a blank hero adds no words: {html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" class=\"btn is-ghost\">See all matches</button>"
            ),
            "{html}"
        );
    }

    #[test]
    fn form_dates_follow_the_frames() {
        let now = "2026-11-17T10:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let this_year = "2026-11-05T23:30:00Z".parse::<DateTime<Utc>>().unwrap();
        let last_year = "2025-12-31T23:30:00Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(form_date(&this_year, &now), "5 Nov");
        assert_eq!(form_date(&last_year, &now), "31 Dec 2025");
    }

    #[test]
    fn form_summary_counts_every_fetched_game() {
        let rows = vec![
            game("a", "victory", "Ilios", "Ana", "2026-11-12T19:00:00Z"),
            game("b", "win", "Ilios", "Ana", "2026-11-11T19:00:00Z"),
            game("c", "defeat", "Ilios", "Ana", "2026-11-10T19:00:00Z"),
        ];
        assert_eq!(form_summary(&rows), "Last 3 games: 2 wins, 1 loss.");
        assert_eq!(form_summary(&rows[..1]), "Last game: 1 win, 0 losses.");
    }

    #[test]
    fn overview_copy_has_no_long_dashes() {
        let src = include_str!("overview.rs");
        assert!(!src.contains('\u{2014}') && !src.contains('\u{2013}'));
        assert!(!src.contains('\u{2026}'), "loading text uses three dots");
    }
}
