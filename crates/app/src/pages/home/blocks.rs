//! Presentational homepage blocks. No network hooks — data is passed in.

use std::collections::HashMap;

use dioxus::prelude::*;
use scuffed_types::{HomeSectionId, HomepageContent, TeamsPresentation};

use super::data::{
    Announcement, Event, HomeTournament, Overview, OverviewTeam, RecentResult, UpcomingMatch,
    day_name,
};
use crate::routes::Route;

// ---------------------------------------------------------------------------
// Hero
// ---------------------------------------------------------------------------

#[component]
pub fn HeroBlock(
    content: HomepageContent,
    initials: String,
    recruitment_open: bool,
    show_secondary_cta: bool,
    /// Anchor for secondary CTA (`#ethos` or `#squads`).
    secondary_href: String,
    metric_squads: Option<usize>,
    metric_members: Option<usize>,
    metric_games: Option<usize>,
) -> Element {
    let show_metrics =
        metric_squads.is_some() || metric_members.is_some() || metric_games.is_some();
    rsx! {
        header { class: "home-hero",
            div {
                class: "home-hero-mark",
                aria_hidden: "true",
                "{initials}"
            }
            div { class: "home-hero-rail",
                div { class: "home-hero-inner",
                    div { class: "home-badge", "{content.hero_badge}" }
                    h1 { class: "home-title",
                        "{content.hero_title}"
                        if !content.hero_title_accent.is_empty() {
                            em { "{content.hero_title_accent}" }
                        }
                    }
                    p { class: "home-sub", "{content.hero_sub}" }
                    div { class: "home-actions",
                        if recruitment_open {
                            Link { to: Route::Apply {}, class: "btn btn-primary", "{content.cta_primary}" }
                        }
                        if show_secondary_cta {
                            a { href: "{secondary_href}", class: "btn btn-outline", "{content.cta_secondary}" }
                        }
                    }
                    if show_metrics {
                        div { class: "home-metrics",
                            if let Some(n) = metric_squads {
                                div { class: "home-metric",
                                    strong { "{n}" }
                                    span { "Active squads" }
                                }
                            }
                            if let Some(n) = metric_members {
                                div { class: "home-metric",
                                    strong { "{n}" }
                                    span { "Members" }
                                }
                            }
                            if let Some(n) = metric_games {
                                div { class: "home-metric",
                                    strong { "{n}" }
                                    span { "Games" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ethos
// ---------------------------------------------------------------------------

#[component]
pub fn EthosBlock(content: HomepageContent) -> Element {
    rsx! {
        section { id: "ethos", class: "home-block",
            div { class: "home-kicker", "{content.ethos_kicker}" }
            h2 { class: "home-heading", "{content.ethos_title}" }
            p { class: "home-body", "{content.ethos_body}" }
            ul { class: "rules",
                for (i, rule) in content.ethos_rules.iter().enumerate() {
                    {
                        let n = format!("{:02}", i + 1);
                        rsx! {
                            li {
                                span { class: "rn", "{n}" }
                                span { "{rule}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Live (schedule + tournaments + next match + results ticker)
// ---------------------------------------------------------------------------

#[component]
pub fn LiveBlock(
    content: HomepageContent,
    events: Vec<Event>,
    live_tournaments: Vec<HomeTournament>,
    upcoming_matches: Vec<UpcomingMatch>,
    recent_results: Vec<RecentResult>,
    show_schedule: bool,
    show_tourneys: bool,
    show_next_match: bool,
    show_results: bool,
) -> Element {
    if !show_schedule && !show_tourneys && !show_next_match && !show_results {
        return rsx! {};
    }
    let panel_count = [show_schedule, show_tourneys, show_next_match, show_results]
        .iter()
        .filter(|b| **b)
        .count();
    let grid_class = if panel_count <= 1 {
        "live-grid single"
    } else {
        "live-grid"
    };
    let has_events = !events.is_empty();
    let has_tourneys = !live_tournaments.is_empty();
    let next = upcoming_matches.first().cloned();
    let has_results = !recent_results.is_empty();

    rsx! {
        section { class: "home-block",
            if show_results && has_results {
                ResultsTicker { results: recent_results.clone() }
            }
            div { class: "{grid_class}",
                if show_next_match {
                    NextMatchPanel { upcoming: next }
                }
                if show_schedule {
                    div { class: "live-panel",
                        div { class: "home-kicker", "{content.schedule_kicker}" }
                        h2 { class: "home-heading", "{content.schedule_title}" }
                        if has_events {
                            ul { class: "live-list",
                                for e in events.iter() {
                                    {
                                        let day = day_name(e.day_of_week);
                                        rsx! {
                                            li {
                                                span { "{e.title}" }
                                                span { class: "live-meta", "{day} · {e.time} {e.timezone}" }
                                            }
                                        }
                                    }
                                }
                            }
                            a { href: "/api/calendar/all.ics", class: "home-link", "{content.calendar_cta}" }
                        } else {
                            p { class: "muted", "{content.schedule_empty}" }
                        }
                    }
                }
                if show_tourneys {
                    div { class: "live-panel compete",
                        div { class: "home-kicker compete", "{content.tournaments_kicker}" }
                        h2 { class: "home-heading", "{content.tournaments_title}" }
                        if has_tourneys {
                            ul { class: "live-list",
                                for t in live_tournaments.iter() {
                                    {
                                        let status = if t.status == "in_progress" { "Live" } else { "Open" };
                                        let tag_class = if t.status == "in_progress" { "tag live" } else { "tag open" };
                                        rsx! {
                                            li {
                                                Link { to: Route::Tournament { id: t.id.clone() }, "{t.name}" }
                                                span { class: "live-meta",
                                                    span { class: "{tag_class}", "{status}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Link {
                                to: Route::Tournaments {},
                                class: "home-link compete",
                                "{content.tournaments_view_all}"
                            }
                        } else {
                            p { class: "muted", "{content.tournaments_empty}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn NextMatchPanel(upcoming: Option<UpcomingMatch>) -> Element {
    rsx! {
        div { class: "live-panel next-match",
            div { class: "home-kicker", "Next match" }
            h2 { class: "home-heading", "Coming up" }
            if let Some(m) = upcoming {
                {
                    let when: String = m.scheduled_at.chars().take(16).collect::<String>().replace('T', " ");
                    let game = m.game_name.clone().unwrap_or_default();
                    let meta = if game.is_empty() {
                        format!("{} · {}", m.team_name, m.match_type)
                    } else {
                        format!("{} · {} · {}", m.team_name, game, m.match_type)
                    };
                    rsx! {
                        div { class: "next-match-card",
                            p { class: "next-match-vs",
                                span { class: "next-match-team", "{m.team_name}" }
                                span { class: "next-match-sep", " vs " }
                                span { class: "next-match-opp", "{m.opponent}" }
                            }
                            p { class: "next-match-when", "{when} UTC" }
                            p { class: "live-meta", "{meta}" }
                        }
                    }
                }
            } else {
                p { class: "muted", "No public fixtures scheduled." }
            }
        }
    }
}

#[component]
fn ResultsTicker(results: Vec<RecentResult>) -> Element {
    rsx! {
        div { class: "results-ticker", "aria-label": "Recent results",
            span { class: "results-ticker-label", "Results" }
            div { class: "results-ticker-track",
                for r in results.iter() {
                    {
                        let score = match (r.score_us, r.score_them) {
                            (Some(u), Some(t)) => format!("{u}–{t}"),
                            _ => "—".into(),
                        };
                        let outcome_class = format!("ticker-chip {}", r.outcome);
                        let date: String = r.played_at.chars().take(10).collect();
                        rsx! {
                            span { class: "{outcome_class}",
                                span { class: "ticker-teams", "{r.team_name} vs {r.opponent}" }
                                span { class: "ticker-score", "{score}" }
                                span { class: "ticker-date", "{date}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Teams
// ---------------------------------------------------------------------------

#[component]
pub fn TeamsBlock(
    content: HomepageContent,
    overview: Option<Overview>,
    presentation: TeamsPresentation,
) -> Element {
    rsx! {
        section { id: "squads", class: "home-block",
            div { class: "home-kicker", "{content.teams_kicker}" }
            h2 { class: "home-heading", "{content.teams_title}" }
            {
                match overview.as_ref() {
                    Some(data) if !data.teams.is_empty() => {
                        let game_map: HashMap<String, String> = data
                            .games
                            .iter()
                            .map(|g| (g.id.clone(), g.name.clone()))
                            .collect();
                        match presentation {
                            TeamsPresentation::Table => rsx! {
                                div { class: "team-rows",
                                    div { class: "team-head",
                                        span { "Squad" }
                                        span { "Game" }
                                        span { "Roster" }
                                        span { "Division" }
                                        span { "W–L" }
                                    }
                                    for team in data.teams.iter() {
                                        { render_team_row(team, &game_map) }
                                    }
                                }
                            },
                            TeamsPresentation::Cards => rsx! {
                                div { class: "team-cards",
                                    for team in data.teams.iter() {
                                        { render_team_card(team, &game_map) }
                                    }
                                }
                            },
                            TeamsPresentation::Compact => rsx! {
                                div { class: "team-compact",
                                    for team in data.teams.iter() {
                                        { render_team_chip(team, &game_map) }
                                    }
                                }
                            },
                        }
                    }
                    Some(_) => rsx! { p { class: "muted", "{content.teams_empty}" } },
                    None => rsx! { p { class: "muted", "Loading squads…" } },
                }
            }
        }
    }
}

fn render_team_row(team: &OverviewTeam, game_map: &HashMap<String, String>) -> Element {
    let game_name = game_map
        .get(&team.game_id)
        .cloned()
        .unwrap_or_else(|| team.game_id.clone());
    let forming = team.roster_count == 0;
    let wl = if team.record.wins == 0 && team.record.losses == 0 {
        "—".to_string()
    } else {
        format!("{}–{}", team.record.wins, team.record.losses)
    };
    let division = team.division.clone().unwrap_or_else(|| "Internal".into());
    let lore = team.lore_quote.clone().unwrap_or_default();
    let roster_n = team.roster_count;
    let row_class = if forming {
        "team-row forming"
    } else {
        "team-row"
    };
    let roster_class = if forming {
        "tm-roster forming"
    } else {
        "tm-roster"
    };
    let roster_label = if forming {
        "Open".to_string()
    } else {
        roster_n.to_string()
    };

    rsx! {
        div { class: "{row_class}",
            div { class: "tm-name",
                Link {
                    to: Route::TeamPage { id: team.id.clone() },
                    class: "tm-name-link",
                    "{team.name}"
                }
                if forming {
                    span { class: "tm-forming", "Forming" }
                }
                if !lore.is_empty() {
                    div { class: "team-lore", "“{lore}”" }
                }
            }
            div { class: "tm-game", "{game_name}" }
            div { class: "{roster_class}", "{roster_label}" }
            div { class: "tm-div", "{division}" }
            div { class: "tm-wl", "{wl}" }
        }
    }
}

fn render_team_card(team: &OverviewTeam, game_map: &HashMap<String, String>) -> Element {
    let game_name = game_map
        .get(&team.game_id)
        .cloned()
        .unwrap_or_else(|| team.game_id.clone());
    let forming = team.roster_count == 0;
    let card_class = if forming {
        "team-card forming"
    } else {
        "team-card"
    };
    let roster = if forming {
        "Open roster".to_string()
    } else {
        crate::util::pluralize(team.roster_count, "player", "players")
    };
    let wl = if team.record.wins == 0 && team.record.losses == 0 {
        String::new()
    } else {
        format!(" · {}–{}", team.record.wins, team.record.losses)
    };
    rsx! {
        Link { to: Route::TeamPage { id: team.id.clone() }, class: "{card_class}",
            div { class: "tc-name", "{team.name}" }
            div { class: "tc-meta", "{game_name} · {roster}{wl}" }
        }
    }
}

fn render_team_chip(team: &OverviewTeam, game_map: &HashMap<String, String>) -> Element {
    let game_name = game_map
        .get(&team.game_id)
        .cloned()
        .unwrap_or_else(|| team.game_id.clone());
    let forming = team.roster_count == 0;
    let chip_class = if forming {
        "team-chip forming"
    } else {
        "team-chip"
    };
    rsx! {
        Link { to: Route::TeamPage { id: team.id.clone() }, class: "{chip_class}",
            "{team.name} · {game_name}"
        }
    }
}

// ---------------------------------------------------------------------------
// News
// ---------------------------------------------------------------------------

#[component]
pub fn NewsBlock(content: HomepageContent, announcements: Vec<Announcement>) -> Element {
    rsx! {
        section { class: "home-block",
            div { class: "home-kicker", "{content.news_kicker}" }
            h2 { class: "home-heading", "{content.news_title}" }
            if announcements.is_empty() {
                p { class: "muted", "{content.news_empty}" }
            } else {
                div { class: "news-rows",
                    for a in announcements.iter().take(4) {
                        { render_news_row(a) }
                    }
                }
                Link { to: Route::News {}, class: "home-link", "{content.news_view_all}" }
            }
        }
    }
}

fn render_news_row(a: &Announcement) -> Element {
    let date: String = a.created_at.chars().take(10).collect();
    rsx! {
        article { class: "news-row",
            time { "{date}" }
            if a.pinned {
                span { class: "pin", "Pinned" }
            }
            h3 { "{a.title}" }
            p { "{a.content}" }
        }
    }
}

// ---------------------------------------------------------------------------
// Recruit
// ---------------------------------------------------------------------------

#[component]
pub fn RecruitBlock(content: HomepageContent) -> Element {
    rsx! {
        section { id: "recruit", class: "home-block",
            div { class: "home-kicker", "{content.recruit_kicker}" }
            h2 { class: "home-heading", "{content.recruit_title}" }
            div { class: "recruit-banner",
                div { class: "recruit-left",
                    p { class: "home-body", style: "margin-top:0;", "{content.recruit_body}" }
                    div { style: "margin-top:1.25rem;",
                        Link { to: Route::Apply {}, class: "btn btn-primary", "{content.recruit_cta}" }
                    }
                    if !content.seeking_tags.is_empty() {
                        div { class: "seek-tags",
                            span {
                                class: "home-kicker",
                                style: "width:100%;margin:0;",
                                "{content.seeking_label}"
                            }
                            for tag in content.seeking_tags.iter() {
                                span { class: "seek-tag", "{tag}" }
                            }
                        }
                    }
                }
                div { class: "recruit-right",
                    div { class: "home-kicker", "{content.recruit_expectations_title}" }
                    ul { class: "expect-list",
                        for line in content.recruit_expectations.iter() {
                            li { "{line}" }
                        }
                    }
                    div { class: "never-box",
                        h4 { "{content.never_ask_title}" }
                        p { "{content.never_ask_body}" }
                    }
                }
            }
        }
    }
}

/// Whether the Teams block will render (for secondary CTA targeting).
pub fn teams_will_render(sections_teams: bool, teams_empty: bool, show_when_empty: bool) -> bool {
    if !sections_teams {
        return false;
    }
    if !teams_empty {
        return true;
    }
    show_when_empty
}

/// Live panel visibility from shell empty-policy + section flags + data.
pub fn live_panel_flags(
    shell_show_empty: bool,
    sections_schedule: bool,
    sections_tournaments: bool,
    has_events: bool,
    has_tourneys: bool,
) -> (bool, bool) {
    let show_schedule = sections_schedule && (has_events || shell_show_empty);
    let show_tourneys = sections_tournaments && (has_tourneys || shell_show_empty);
    (show_schedule, show_tourneys)
}

#[allow(dead_code)]
pub fn _section_id_for_debug(id: HomeSectionId) -> &'static str {
    match id {
        HomeSectionId::Ethos => "ethos",
        HomeSectionId::Live => "live",
        HomeSectionId::Teams => "teams",
        HomeSectionId::News => "news",
        HomeSectionId::Recruit => "recruit",
    }
}
