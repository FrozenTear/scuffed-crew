use dioxus::prelude::*;

use serde::Deserialize;

use super::stats::load_error_state;
use super::stats::role::{
    RolePanel, hide_member_role_error, member_roles_path, role_aggs_from_rows, role_panel,
    role_result,
};
use crate::components::ui::{SeasonSelect, TabButton, use_stats_season};
use crate::components::{DataTable, member_pending};
use crate::hooks::use_api_with;
use crate::sample::{
    HERO_RULE_NOTE, MAP_RULE_NOTE, OTHER_HEROES, OTHER_RESULT_CLASS, Tally, fold_small, other_label,
};
use crate::state::use_auth;
use crate::stats_privacy;

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

/// Text class for a hero or map result: muted until it has a win rate.
fn result_class(tally: Tally) -> &'static str {
    if tally.has_win_rate() {
        "stats-wr"
    } else {
        "stats-wr muted"
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MemberStatsTab {
    Overview,
    Heroes,
    Maps,
}

/// The one tabpanel under the member stats tabs.
const MEMBER_STATS_PANEL: &str = "member-stats-panel";

/// Id of a member stats tab button. The tabpanel is labelled by the selected one.
fn member_tab_id(tab: MemberStatsTab) -> &'static str {
    match tab {
        MemberStatsTab::Overview => "member-stats-tab-overview",
        MemberStatsTab::Heroes => "member-stats-tab-heroes",
        MemberStatsTab::Maps => "member-stats-tab-maps",
    }
}

const MEMBER_STATS_CSS: &str = r#"
    .member-stats.stats-page {
        max-width: 1100px;
        margin: 0 auto;
        padding: 2rem 1.5rem;
    }
    .member-stats.stats-page h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin-bottom: 1.5rem;
    }
    .member-stats .stats-tabs {
        display: flex;
        gap: 0.25rem;
        margin-bottom: 1.5rem;
        border-bottom: 1px solid var(--border);
        overflow-x: auto;
    }
    .member-stats .stats-tab {
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
        white-space: nowrap;
    }
    .member-stats .stats-tab:hover { color: var(--text); }
    .member-stats .stats-tab.active {
        color: var(--accent);
        border-bottom-color: var(--accent);
    }
    .member-stats .stats-summary {
        display: grid;
        grid-template-columns: minmax(220px, 2fr) minmax(140px, 1fr);
        gap: 1rem;
        margin-bottom: 2rem;
    }
    .member-stats .stat-tile {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        text-align: center;
        display: flex;
        flex-direction: column;
        justify-content: center;
    }
    .member-stats .stat-tile-value {
        font-family: var(--font-head);
        font-size: 1.9rem;
        color: var(--text);
        letter-spacing: 2px;
        line-height: 1;
    }
    .member-stats .stat-tile-hero .stat-tile-value {
        font-size: 2.8rem;
        color: var(--accent);
    }
    .member-stats .stat-tile-label {
        font-size: 0.75rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.05em;
        margin-top: 0.35rem;
    }
    .member-stats .stat-tile-record {
        font-size: 0.9rem;
        color: var(--text-2);
        margin-top: 0.4rem;
        white-space: nowrap;
    }
    .member-stats .stats-wr { font-weight: 600; color: var(--text); font-variant-numeric: tabular-nums; }
    .member-stats .stats-wr.muted { color: var(--text-3); font-weight: 400; }
    .member-stats .stats-row-muted { opacity: 0.55; }
    .member-stats .stats-gate-note { font-size: 0.75rem; color: var(--text-3); margin: 0 0 0.75rem; }
    .member-stats.stats-page .data-table th:not(:first-child),
    .member-stats.stats-page .data-table td:not(:first-child) {
        text-align: right;
        font-variant-numeric: tabular-nums;
    }
    .member-stats.stats-page .data-table-scroll {
        overflow-x: auto;
        -webkit-overflow-scrolling: touch;
        max-width: 100%;
    }
    .member-stats .stats-season-row { max-width: 280px; margin-bottom: 1.25rem; }
    @media (max-width: 720px) {
        .member-stats.stats-page { padding: 1.25rem 1rem; }
        .member-stats .stats-summary { grid-template-columns: 1fr; }
    }
"#;

#[component]
pub fn StatsMember(id: String) -> Element {
    let auth = use_auth();
    let member_id = id.clone();
    let member_id_h = id.clone();
    let member_id_m = id.clone();
    let member_id_r = id.clone();

    // Same saved season as My Stats and leaderboards, including the
    // "current" sentinel. Nothing saved stays all time.
    let season = use_stats_season();

    let stats = use_api_with::<PersonalStats>(move || {
        season.fetch_path(&format!("/api/stats/member/{member_id}"))
    });

    let heroes = use_api_with::<Vec<HeroStats>>(move || {
        season.fetch_path(&format!("/api/stats/member/{member_id_h}/heroes"))
    });

    let maps = use_api_with::<Vec<MapStats>>(move || {
        season.fetch_path(&format!("/api/stats/member/{member_id_m}/maps"))
    });

    let roles = use_api_with::<Vec<scuffed_types::RoleStats>>(move || {
        season.fetch_path(&member_roles_path(&member_id_r))
    });

    let mut tab = use_signal(|| MemberStatsTab::Overview);

    // A refused summary (private, members only, bad id) is the whole page's
    // answer. Say so instead of showing tabs that never load.
    let denied = stats
        .error
        .read()
        .as_deref()
        .and_then(stats_privacy::member_stats_denied);
    if let Some(copy) = denied {
        return rsx! {
            crate::styles::StyleOnce { id: "scuffed-css-pages-stats_member-member-stats-css", css: MEMBER_STATS_CSS }
            crate::styles::StyleOnce { id: "scuffed-css-admin-shared", css: crate::styles::admin::CSS }

            div { class: "stats-page member-stats",
                h1 { "Member Stats" }
                p { class: "empty-state", role: "status", "{copy}" }
            }
        };
    }

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-stats_member-member-stats-css", css: MEMBER_STATS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-admin-shared", css: crate::styles::admin::CSS }

        div { class: "stats-page member-stats",
            h1 { "Member Stats" }
            div { class: "stats-season-row",
                SeasonSelect {
                    id: "member-stats-season".to_string(),
                    name: "member-stats-season".to_string(),
                    seasons: season.season_list(),
                    seasons_error: season.seasons_error(),
                    on_retry: move |_| season.retry(),
                    value: season.selected_id(),
                    onchange: move |s| season.choose(s),
                }
            }

            {
                let data = stats.data.read();
                let s = data.as_ref().and_then(|d| d.as_ref());
                match s {
                    None => member_pending(&auth(), &stats, "member stats"),
                    Some(s) if s.total_matches == 0 => rsx! {
                        p { class: "empty-state", "No matches tracked for this member yet." }
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

            div { class: "stats-tabs", role: "tablist", aria_label: "Stats sections",
                TabButton {
                    class: "stats-tab".to_string(),
                    label: "Overview".to_string(),
                    id: member_tab_id(MemberStatsTab::Overview).to_string(),
                    controls: MEMBER_STATS_PANEL.to_string(),
                    selected: tab() == MemberStatsTab::Overview,
                    onclick: move |_| tab.set(MemberStatsTab::Overview),
                }
                TabButton {
                    class: "stats-tab".to_string(),
                    label: "Heroes".to_string(),
                    id: member_tab_id(MemberStatsTab::Heroes).to_string(),
                    controls: MEMBER_STATS_PANEL.to_string(),
                    selected: tab() == MemberStatsTab::Heroes,
                    onclick: move |_| tab.set(MemberStatsTab::Heroes),
                }
                TabButton {
                    class: "stats-tab".to_string(),
                    label: "Maps".to_string(),
                    id: member_tab_id(MemberStatsTab::Maps).to_string(),
                    controls: MEMBER_STATS_PANEL.to_string(),
                    selected: tab() == MemberStatsTab::Maps,
                    onclick: move |_| tab.set(MemberStatsTab::Maps),
                }
            }

            div {
                id: MEMBER_STATS_PANEL,
                role: "tabpanel",
                aria_labelledby: member_tab_id(tab()),
                tabindex: "0",
                match tab() {
                    MemberStatsTab::Overview => rsx! {
                        {
                            let err = roles.error.read().clone();
                            let data = roles.data.read();
                            let rows = data.as_ref().and_then(|d| d.as_ref());
                            if err.as_deref().is_some_and(hide_member_role_error) {
                                rsx! {}
                            } else {
                                match role_panel(err.as_deref(), rows.map(|rows| rows.as_slice())) {
                                    RolePanel::Pending => rsx! {
                                        p { class: "loading-state", "Loading role breakdown..." }
                                    },
                                    RolePanel::Failed => load_error_state("role breakdown", roles.refresh),
                                    RolePanel::Ready(rows) => {
                                        let aggs = role_aggs_from_rows(rows);
                                        let shown: Vec<_> = aggs.iter().filter(|r| r.matches > 0).collect();
                                        if shown.is_empty() {
                                            rsx! {
                                                p { class: "empty-state",
                                                    style: "padding: 1rem 0; text-align: left;",
                                                    "No role stats yet."
                                                }
                                            }
                                        } else {
                                            rsx! {
                                                DataTable { headers: vec!["Role", "Matches", "Win rate"],
                                                    for agg in shown.iter() {
                                                        {
                                                            let tally = agg.tally();
                                                            let result = role_result(tally);
                                                            let wr_cls = result_class(tally);
                                                            let name = agg.name;
                                                            let matches = agg.matches;
                                                            rsx! {
                                                                tr { key: "{name}",
                                                                    td { "{name}" }
                                                                    td { "{matches}" }
                                                                    td { span { class: "{wr_cls}", "{result}" } }
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
                    },
                    MemberStatsTab::Heroes => rsx! {
                        {
                            let data = heroes.data.read();
                            let list = data.as_ref().and_then(|d| d.as_ref());
                            match list {
                                None => rsx! { p { class: "loading-state", "Loading hero stats..." } },
                                Some(list) if list.is_empty() => rsx! {
                                    p { class: "empty-state", "No hero stats recorded." }
                                },
                                Some(list) => {
                                    let all: Vec<&HeroStats> = list.iter().collect();
                                    // Under 3 games a hero folds into one "Other heroes" row.
                                    let (mut sorted, folded) = fold_small(&all, |h| h.tally());
                                    sorted.sort_by_key(|b| std::cmp::Reverse(b.matches));
                                    let other = Tally::sum(folded.iter().map(|h| h.tally()));
                                    let other_count = folded.len();
                                    let other_names = folded
                                        .iter()
                                        .map(|h| h.hero.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    // The pool mixes heroes, so it always shows the record, muted.
                                    let other_result = other_label(other);
                                    rsx! {
                                        p { class: "stats-gate-note", {HERO_RULE_NOTE} }
                                        DataTable { headers: vec!["Hero", "Matches", "Win %", "Avg Elims", "Avg Deaths", "Avg Dmg", "Avg Heal"],
                                            for hero in sorted.iter() {
                                                {
                                                    let tally = hero.tally();
                                                    let result = tally.label();
                                                    let wr_cls = result_class(tally);
                                                    rsx! {
                                                        tr { key: "{hero.hero}",
                                                            td { "{hero.hero}" }
                                                            td { "{hero.matches}" }
                                                            td { span { class: "{wr_cls}", "{result}" } }
                                                            td { "{hero.avg_elims:.1}" }
                                                            td { "{hero.avg_deaths:.1}" }
                                                            td { "{hero.avg_damage:.0}" }
                                                            td { "{hero.avg_healing:.0}" }
                                                        }
                                                    }
                                                }
                                            }
                                            if other_count > 0 {
                                                tr { key: "{OTHER_HEROES}", class: "stats-row-muted", title: "{other_names}",
                                                    td { "{OTHER_HEROES} ({other_count})" }
                                                    td { "{other.games}" }
                                                    td { span { class: "{OTHER_RESULT_CLASS}", "{other_result}" } }
                                                    td { "-" }
                                                    td { "-" }
                                                    td { "-" }
                                                    td { "-" }
                                                }
                                            }
                                        }
                                    }
                                },
                            }
                        }
                    },
                    MemberStatsTab::Maps => rsx! {
                        {
                            let data = maps.data.read();
                            let list = data.as_ref().and_then(|d| d.as_ref());
                            match list {
                                None => rsx! { p { class: "loading-state", "Loading map stats..." } },
                                Some(list) if list.is_empty() => rsx! {
                                    p { class: "empty-state", "No map stats recorded." }
                                },
                                Some(list) => {
                                    let mut sorted: Vec<_> = list.iter().collect();
                                    sorted.sort_by_key(|b| std::cmp::Reverse(b.matches));
                                    rsx! {
                                        p { class: "stats-gate-note", {MAP_RULE_NOTE} }
                                        DataTable { headers: vec!["Map", "Matches", "Win %", "Wins", "Losses", "Draws"],
                                            for map in sorted.iter() {
                                                {
                                                    let tally = map.tally();
                                                    let result = tally.label();
                                                    let wr_cls = result_class(tally);
                                                    rsx! {
                                                        tr { key: "{map.map_name}",
                                                            td { "{map.map_name}" }
                                                            td { "{map.matches}" }
                                                            td { span { class: "{wr_cls}", "{result}" } }
                                                            td { "{map.wins}" }
                                                            td { "{map.losses}" }
                                                            td { "{map.draws}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                },
                            }
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_tab_ids_are_unique_and_differ_from_the_panel() {
        let tabs = [
            MemberStatsTab::Overview,
            MemberStatsTab::Heroes,
            MemberStatsTab::Maps,
        ];
        let ids: Vec<&str> = tabs.iter().map(|t| member_tab_id(*t)).collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "{id}");
            assert_ne!(*id, MEMBER_STATS_PANEL);
        }
    }
}
