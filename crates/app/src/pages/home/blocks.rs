//! Presentational homepage blocks. No network hooks: data is passed in.

use std::collections::HashMap;

use dioxus::prelude::*;
use scuffed_types::{ContentAlign, HomeSectionId, HomepageContent, TeamsPresentation};

use super::data::{
    Announcement, Event, HomeTournament, Overview, OverviewTeam, RecentResult, TeamRecord,
    UpcomingMatch, day_name,
};
use crate::components::retry_label;
use crate::components::ui::{Pill, PillTone, btn_class};
use crate::pages::article_author::{entry_date, short_date};
use crate::routes::Route;

/// Whether a homepage list has finished. Empty copy is only for [`ListPhase::Ready`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListPhase {
    Pending,
    Failed,
    Ready,
}

/// True only when the fetch settled on an empty list.
/// Pending paints a loading line. Failed paints an error and Retry.
pub fn shows_empty_copy(phase: ListPhase, is_empty: bool) -> bool {
    phase == ListPhase::Ready && is_empty
}

/// Pending sections stay mounted so the layout does not jump when the list lands.
/// Ready follows the data (or the shell's empty policy). Failed follows that policy.
pub fn keep_section(phase: ListPhase, has_items: bool, show_when_empty: bool) -> bool {
    match phase {
        ListPhase::Pending => true,
        ListPhase::Ready => has_items || show_when_empty,
        ListPhase::Failed => show_when_empty,
    }
}

/// Text of a list's status line: what is loading, what failed, or nothing.
fn list_status_text(phase: ListPhase, subject: &str) -> String {
    match phase {
        ListPhase::Pending => format!("Loading {subject}"),
        ListPhase::Failed => format!("Couldn't load {subject}."),
        ListPhase::Ready => String::new(),
    }
}

/// Retry control for a list. `None` hides it, `Some(busy)` keeps it mounted.
/// After a click it stays on screen, busy, while the refetch runs, so the
/// focused button is not unmounted.
fn list_retry(phase: ListPhase, retried: bool) -> Option<bool> {
    match phase {
        ListPhase::Failed => Some(false),
        ListPhase::Pending if retried => Some(true),
        _ => None,
    }
}

/// The status line of one homepage list, plus its Retry.
///
/// Render it in every phase, at the same place in the panel: the line is a
/// live region, so it has to exist before its text changes ("Loading the
/// schedule", "Couldn't load the schedule.", then empty). Retry sits after the
/// line, not inside it, and is named after what it loads. While the refetch
/// runs it stays mounted and `aria-disabled`, so keyboard focus stays on it.
///
/// `subject` reads after "Loading" and "Couldn't load": "the schedule",
/// "teams".
#[component]
fn ListStatus(phase: ListPhase, subject: String, retry: Signal<u32>) -> Element {
    // Set by a Retry click. Only a retry makes a settled list pending again,
    // so a stale `true` never shows a busy button for anything else.
    let mut retried = use_signal(|| false);
    let text = list_status_text(phase, &subject);
    let control = list_retry(phase, retried());
    rsx! {
        p { class: "home-muted home-list-status", role: "status", "{text}" }
        if let Some(busy) = control {
            button {
                r#type: "button",
                class: "fetch-error__retry is-compact",
                aria_label: "{retry_label(busy)} loading {subject}",
                aria_busy: if busy { "true" },
                aria_disabled: if busy { "true" },
                onclick: move |_| {
                    if !busy {
                        retried.set(true);
                        retry += 1;
                    }
                },
                "{retry_label(busy)}"
            }
        }
    }
}

/// Kicker plus the section H2. Empty kickers are not rendered.
#[component]
fn SectionHead(kicker: String, title: String) -> Element {
    rsx! {
        div { class: "home-head",
            if !kicker.trim().is_empty() {
                p { class: "home-kicker", "{kicker}" }
            }
            h2 { class: "ui-section-title", "{title}" }
        }
    }
}

// ---------------------------------------------------------------------------
// Hero
// ---------------------------------------------------------------------------

/// Hero class for the admin "Hero text alignment" setting. Only the hero
/// follows it: cards and section heads stay left.
pub fn hero_class(align: ContentAlign) -> &'static str {
    match align {
        ContentAlign::Left => "home-hero",
        ContentAlign::Center => "home-hero is-centered",
    }
}

#[component]
pub fn HeroBlock(
    content: HomepageContent,
    recruitment_open: bool,
    show_secondary_cta: bool,
    /// Anchor for secondary CTA (`#ethos` or `#squads`).
    secondary_href: String,
) -> Element {
    let class = hero_class(content.content_align);
    rsx! {
        section { class: "{class}",
            if !content.hero_badge.trim().is_empty() {
                div { class: "home-banner", "{content.hero_badge}" }
            }
            h1 { class: "home-hero-title",
                "{content.hero_title}"
                if !content.hero_title_accent.is_empty() {
                    // A real space, so the two parts are two words on screen,
                    // in a copy and to a screen reader.
                    " "
                    em { "{content.hero_title_accent}" }
                }
            }
            if !content.hero_sub.trim().is_empty() {
                p { class: "home-hero-sub", "{content.hero_sub}" }
            }
            if recruitment_open || show_secondary_cta {
                div { class: "home-actions",
                    if recruitment_open {
                        Link { to: Route::Apply {}, class: btn_class(true),
                            "{content.cta_primary}"
                        }
                    }
                    if show_secondary_cta {
                        a {
                            href: "{secondary_href}",
                            class: btn_class(false),
                            "{content.cta_secondary}"
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stat tiles
// ---------------------------------------------------------------------------

/// Label under the game title count. One title reads singular.
fn game_titles_label(n: usize) -> &'static str {
    if n == 1 { "Game title" } else { "Game titles" }
}

/// Label under the squad count. One squad reads singular.
fn squads_label(n: usize) -> &'static str {
    if n == 1 {
        "Active squad"
    } else {
        "Active squads"
    }
}

/// Label under the member count. One member reads singular.
fn members_label(n: usize) -> &'static str {
    if n == 1 { "Member" } else { "Members" }
}

/// The three summary tiles under the hero. While overview is in flight the
/// row is reserved at the loaded height, then tiles without a value drop out.
#[component]
pub fn StatsBlock(
    metric_squads: Option<usize>,
    metric_members: Option<usize>,
    metric_games: Option<usize>,
    /// Overview is still in flight.
    metrics_pending: bool,
) -> Element {
    if metrics_pending {
        return rsx! {
            div { class: "home-stats",
                for label in ["Active squads", "Members", "Game titles"] {
                    div { class: "home-stat",
                        span { class: "home-stat-n",
                            span { class: "home-skel home-skel-stat", aria_hidden: "true" }
                        }
                        span { class: "home-stat-l", "{label}" }
                    }
                }
            }
        };
    }
    let tiles: Vec<(usize, &'static str)> = [
        metric_squads.map(|n| (n, squads_label(n))),
        metric_members.map(|n| (n, members_label(n))),
        metric_games.map(|n| (n, game_titles_label(n))),
    ]
    .into_iter()
    .flatten()
    .collect();
    if tiles.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "home-stats",
            for (n , label) in tiles {
                div { class: "home-stat",
                    span { class: "home-stat-n", "{n}" }
                    span { class: "home-stat-l", "{label}" }
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
        section { id: "ethos", class: "home-section",
            SectionHead { kicker: content.ethos_kicker.clone(), title: content.ethos_title.clone() }
            if !content.ethos_body.trim().is_empty() {
                p { class: "home-body", "{content.ethos_body}" }
            }
            if !content.ethos_rules.is_empty() {
                // `list-style: none` drops the list role in some browsers.
                ol { class: "home-rules", role: "list",
                    for (i , rule) in content.ethos_rules.iter().enumerate() {
                        {
                            let n = format!("{:02}", i + 1);
                            rsx! {
                                li {
                                    span { class: "home-rule-num", aria_hidden: "true", "{n}" }
                                    span { "{rule}" }
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
// Live (schedule + tournaments + next match + results)
// ---------------------------------------------------------------------------

#[component]
pub fn LiveBlock(
    content: HomepageContent,
    events: Vec<Event>,
    events_phase: ListPhase,
    live_tournaments: Vec<HomeTournament>,
    tourneys_phase: ListPhase,
    upcoming_matches: Vec<UpcomingMatch>,
    upcoming_phase: ListPhase,
    recent_results: Vec<RecentResult>,
    show_schedule: bool,
    show_tourneys: bool,
    show_next_match: bool,
    show_results: bool,
    retry_events: Signal<u32>,
    retry_tourneys: Signal<u32>,
    retry_overview: Signal<u32>,
) -> Element {
    if !show_schedule && !show_tourneys && !show_next_match && !show_results {
        return rsx! {};
    }
    let panel_count = [show_schedule, show_tourneys, show_next_match]
        .iter()
        .filter(|b| **b)
        .count();
    let grid_class = if panel_count <= 1 {
        "home-grid-2 is-single"
    } else {
        "home-grid-2"
    };
    let has_events = !events.is_empty();
    let has_tourneys = !live_tournaments.is_empty();
    let next = upcoming_matches.first().cloned();
    let has_results = !recent_results.is_empty();

    rsx! {
        section { class: "home-section",
            if show_results && upcoming_phase == ListPhase::Pending {
                div {
                    class: "home-results",
                    role: "region",
                    "aria-label": "Recent results",
                    span { class: "home-results-label", "Results" }
                    p { class: "home-muted", "Loading results" }
                }
            } else if show_results && has_results {
                ResultsStrip { results: recent_results.clone() }
            }
            if panel_count > 0 {
                div { class: "{grid_class}",
                    if show_next_match {
                        NextMatchPanel {
                            upcoming: next,
                            phase: upcoming_phase,
                            retry: retry_overview,
                        }
                    }
                    if show_schedule {
                        div { class: "home-card",
                            SectionHead {
                                kicker: content.schedule_kicker.clone(),
                                title: content.schedule_title.clone(),
                            }
                            ListStatus {
                                phase: events_phase,
                                subject: "the schedule",
                                retry: retry_events,
                            }
                            if shows_empty_copy(events_phase, !has_events) {
                                p { class: "home-muted", "{content.schedule_empty}" }
                            } else if events_phase == ListPhase::Ready && has_events {
                                ul { class: "home-list", role: "list",
                                    for e in events.iter() {
                                        {
                                            let day = day_name(e.day_of_week);
                                            rsx! {
                                                li {
                                                    span { "{e.title}" }
                                                    span { class: "home-meta", "{day} · {e.time} {e.timezone}" }
                                                }
                                            }
                                        }
                                    }
                                }
                                a {
                                    href: "/api/calendar/all.ics",
                                    class: "home-link",
                                    "{content.calendar_cta}"
                                }
                            }
                        }
                    }
                    if show_tourneys {
                        div { class: "home-card",
                            SectionHead {
                                kicker: content.tournaments_kicker.clone(),
                                title: content.tournaments_title.clone(),
                            }
                            ListStatus {
                                phase: tourneys_phase,
                                subject: "tournaments",
                                retry: retry_tourneys,
                            }
                            if shows_empty_copy(tourneys_phase, !has_tourneys) {
                                p { class: "home-muted", "{content.tournaments_empty}" }
                            } else if tourneys_phase == ListPhase::Ready && has_tourneys {
                                ul { class: "home-list", role: "list",
                                    for t in live_tournaments.iter() {
                                        {
                                            let (status, tone) = tournament_badge(&t.status);
                                            rsx! {
                                                li {
                                                    Link {
                                                        to: Route::Tournament {
                                                            id: t.id.clone(),
                                                        },
                                                        "{t.name}"
                                                    }
                                                    Pill { tone, "{status}" }
                                                }
                                            }
                                        }
                                    }
                                }
                                Link {
                                    to: Route::Tournaments {},
                                    class: "home-link",
                                    "{content.tournaments_view_all}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Word and pill tone for a tournament on the home list. The word carries
/// the state; the tone only backs it up.
fn tournament_badge(status: &str) -> (&'static str, PillTone) {
    if status == "in_progress" {
        ("Live", PillTone::Warn)
    } else {
        ("Open", PillTone::Accent)
    }
}

/// Word and pill tone for a result, or `None` when the outcome is unknown.
/// The result is always a word, never the score colour alone.
fn outcome_badge(outcome: &str) -> Option<(&'static str, PillTone)> {
    match outcome {
        "win" => Some(("Win", PillTone::Ok)),
        "loss" => Some(("Loss", PillTone::Danger)),
        "draw" => Some(("Draw", PillTone::Neutral)),
        _ => None,
    }
}

/// `2026-11-12T19:00:00Z` as "12 Nov 2026, 19:00 UTC", in the same date
/// format as News and Blog. A value that is not a timestamp is shown as sent.
fn match_when(iso: &str) -> String {
    match (short_date(iso), iso.get(11..16)) {
        (Some((_, date)), Some(time)) => format!("{date}, {time} UTC"),
        (Some((_, date)), None) => date,
        (None, _) => iso.to_string(),
    }
}

/// The date of a result as "1 Nov 2026", or the stored text when it is not a
/// date.
fn result_date(iso: &str) -> String {
    short_date(iso).map_or_else(|| iso.to_string(), |(_, text)| text)
}

#[component]
fn NextMatchPanel(
    upcoming: Option<UpcomingMatch>,
    phase: ListPhase,
    retry: Signal<u32>,
) -> Element {
    rsx! {
        div { class: "home-card",
            SectionHead { kicker: "Next match", title: "Coming up" }
            ListStatus { phase, subject: "the next match", retry }
            if let Some(m) = upcoming {
                {
                    let when = match_when(&m.scheduled_at);
                    let game = m.game_name.clone().unwrap_or_default();
                    let meta = if game.is_empty() {
                        format!("{} · {}", m.team_name, m.match_type)
                    } else {
                        format!("{} · {} · {}", m.team_name, game, m.match_type)
                    };
                    rsx! {
                        p { class: "home-match-vs",
                            span { "{m.team_name}" }
                            span { class: "home-match-sep", " vs " }
                            span { "{m.opponent}" }
                        }
                        p { class: "home-match-when", "{when}" }
                        p { class: "home-meta", "{meta}" }
                    }
                }
            } else if shows_empty_copy(phase, true) {
                p { class: "home-muted", "No public fixtures scheduled." }
            }
        }
    }
}

#[component]
fn ResultsStrip(results: Vec<RecentResult>) -> Element {
    rsx! {
        div {
            class: "home-results",
            role: "region",
            "aria-label": "Recent results",
            span { class: "home-results-label", "Results" }
            ul { class: "home-results-track", role: "list",
                for r in results.iter() {
                    {
                        let score = match (r.score_us, r.score_them) {
                            (Some(u), Some(t)) => format!("{u}-{t}"),
                            _ => "-".into(),
                        };
                        let outcome = outcome_badge(&r.outcome);
                        let date = result_date(&r.played_at);
                        rsx! {
                            li { key: "{r.id}", class: "home-result",
                                span { class: "home-result-teams", "{r.team_name} vs {r.opponent}" }
                                span { class: "home-result-score", "{score}" }
                                if let Some((word, tone)) = outcome {
                                    Pill { tone, "{word}" }
                                }
                                span { class: "home-result-date", "{date}" }
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
    phase: ListPhase,
    presentation: TeamsPresentation,
    retry: Signal<u32>,
) -> Element {
    rsx! {
        section { id: "squads", class: "home-section",
            SectionHead { kicker: content.teams_kicker.clone(), title: content.teams_title.clone() }
            ListStatus { phase, subject: "teams", retry }
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
                                div {
                                    class: "home-team-table",
                                    role: "table",
                                    "aria-label": "{content.teams_title}",
                                    div { class: "home-team-head", role: "row",
                                        span { role: "columnheader", "Squad" }
                                        span { role: "columnheader", "Game" }
                                        span { role: "columnheader", "Roster" }
                                        span { role: "columnheader", "Division" }
                                        span { role: "columnheader", "Record" }
                                    }
                                    for team in data.teams.iter() {
                                        {render_team_row(team, &game_map)}
                                    }
                                }
                            },
                            TeamsPresentation::Cards => rsx! {
                                div { class: "home-team-cards",
                                    for team in data.teams.iter() {
                                        {render_team_card(team, &game_map)}
                                    }
                                }
                            },
                            TeamsPresentation::Compact => rsx! {
                                div { class: "home-team-chips",
                                    for team in data.teams.iter() {
                                        {render_team_chip(team, &game_map)}
                                    }
                                }
                            },
                        }
                    }
                    Some(_) if shows_empty_copy(phase, true) => {
                        rsx! {
                            p { class: "home-muted", "{content.teams_empty}" }
                        }
                    }
                    // Loading and failure are the status line above.
                    _ => rsx! {},
                }
            }
        }
    }
}

/// True once a team has a finished match of any outcome.
fn has_played(record: &TeamRecord) -> bool {
    record.wins + record.losses + record.draws > 0
}

/// Record column in the teams table: `5W-2L` or `5W-2L-1D`, or `-` before
/// the first finished match.
fn row_record(record: &TeamRecord) -> String {
    if has_played(record) {
        crate::util::record_label(record.wins, record.losses, record.draws)
    } else {
        "-".to_string()
    }
}

/// Record suffix on a team card's meta line, empty before the first finished
/// match.
fn card_record(record: &TeamRecord) -> String {
    if has_played(record) {
        format!(
            " · {}",
            crate::util::record_label(record.wins, record.losses, record.draws)
        )
    } else {
        String::new()
    }
}

fn team_game_name(team: &OverviewTeam, game_map: &HashMap<String, String>) -> String {
    game_map
        .get(&team.game_id)
        .cloned()
        .unwrap_or_else(|| team.game_id.clone())
}

fn render_team_row(team: &OverviewTeam, game_map: &HashMap<String, String>) -> Element {
    let game_name = team_game_name(team, game_map);
    let forming = team.roster_count == 0;
    let wl = row_record(&team.record);
    let division = team.division.clone().unwrap_or_else(|| "Internal".into());
    let lore = team.lore_quote.clone().unwrap_or_default();
    let roster_label = if forming {
        "Open".to_string()
    } else {
        team.roster_count.to_string()
    };

    rsx! {
        div { key: "{team.id}", class: "home-team-row", role: "row",
            div { class: "home-team-name", role: "cell",
                Link {
                    to: Route::TeamPage {
                        id: team.id.clone(),
                    },
                    "{team.name}"
                }
                if forming {
                    Pill { tone: PillTone::Warn, "Forming" }
                }
                if !lore.is_empty() {
                    div { class: "home-team-lore", "“{lore}”" }
                }
            }
            div { class: "home-team-game", role: "cell", "{game_name}" }
            div { class: "home-team-roster", role: "cell", "{roster_label}" }
            div { class: "home-team-div", role: "cell", "{division}" }
            div { class: "home-team-record", role: "cell", "{wl}" }
        }
    }
}

fn render_team_card(team: &OverviewTeam, game_map: &HashMap<String, String>) -> Element {
    let game_name = team_game_name(team, game_map);
    let forming = team.roster_count == 0;
    let card_class = if forming {
        "home-card home-team-card is-forming"
    } else {
        "home-card home-team-card"
    };
    let roster = if forming {
        "Open roster".to_string()
    } else {
        crate::util::pluralize(team.roster_count, "player", "players")
    };
    let wl = card_record(&team.record);
    rsx! {
        Link {
            key: "{team.id}",
            to: Route::TeamPage {
                id: team.id.clone(),
            },
            class: "{card_class}",
            h3 { class: "home-card-title", "{team.name}" }
            p { class: "home-meta", "{game_name} · {roster}{wl}" }
        }
    }
}

fn render_team_chip(team: &OverviewTeam, game_map: &HashMap<String, String>) -> Element {
    let game_name = team_game_name(team, game_map);
    let forming = team.roster_count == 0;
    let chip_class = if forming {
        "home-team-chip is-forming"
    } else {
        "home-team-chip"
    };
    // The dashed edge alone does not say "forming".
    let suffix = if forming { " · Forming" } else { "" };
    rsx! {
        Link {
            key: "{team.id}",
            to: Route::TeamPage {
                id: team.id.clone(),
            },
            class: "{chip_class}",
            "{team.name} · {game_name}{suffix}"
        }
    }
}

// ---------------------------------------------------------------------------
// News
// ---------------------------------------------------------------------------

#[component]
pub fn NewsBlock(
    content: HomepageContent,
    announcements: Vec<Announcement>,
    phase: ListPhase,
    retry: Signal<u32>,
) -> Element {
    rsx! {
        section { class: "home-section",
            SectionHead { kicker: content.news_kicker.clone(), title: content.news_title.clone() }
            ListStatus { phase, subject: "news", retry }
            if shows_empty_copy(phase, announcements.is_empty()) {
                p { class: "home-muted", "{content.news_empty}" }
            } else if phase == ListPhase::Ready {
                div { class: "home-stack",
                    // Mapped, so the key on each card's root reaches the list.
                    {announcements.iter().take(4).map(render_news_card)}
                }
                Link { to: Route::News {}, class: "home-link", "{content.news_view_all}" }
            }
        }
    }
}

/// The date is the same "8 Nov 2026" the News page prints.
fn render_news_card(a: &Announcement) -> Element {
    rsx! {
        // The heading comes first in the document, so a card is announced
        // by its title; CSS draws the Pinned badge and date above it.
        article { key: "{a.id}", class: "home-card home-news-card",
            h3 { class: "home-card-title", "{a.title}" }
            div { class: "home-news-meta",
                if a.pinned {
                    Pill { "Pinned" }
                }
                {entry_date(&a.created_at)}
            }
            p { class: "home-news-body", "{a.content}" }
        }
    }
}

// ---------------------------------------------------------------------------
// Recruit
// ---------------------------------------------------------------------------

#[component]
pub fn RecruitBlock(content: HomepageContent) -> Element {
    let has_rules = !content.recruit_expectations.is_empty()
        || !content.never_ask_title.trim().is_empty()
        || !content.never_ask_body.trim().is_empty();
    let grid_class = if has_rules {
        "home-grid-2"
    } else {
        "home-grid-2 is-single"
    };
    rsx! {
        section { id: "recruit", class: "{grid_class}",
            article { class: "home-card",
                SectionHead {
                    kicker: content.recruit_kicker.clone(),
                    title: content.recruit_title.clone(),
                }
                p { "{content.recruit_body}" }
                div { class: "home-action-row",
                    Link { to: Route::Apply {}, class: btn_class(true), "{content.recruit_cta}" }
                    if !content.seeking_tags.is_empty() {
                        div { class: "home-tag-row",
                            if !content.seeking_label.trim().is_empty() {
                                span { class: "home-tag-label", "{content.seeking_label}" }
                            }
                            for tag in content.seeking_tags.iter() {
                                span { class: "home-tag", "{tag}" }
                            }
                        }
                    }
                }
            }
            if has_rules {
                article { class: "home-card",
                    if !content.recruit_expectations_title.trim().is_empty() {
                        h2 { class: "ui-section-title", "{content.recruit_expectations_title}" }
                    }
                    if !content.recruit_expectations.is_empty() {
                        ul { class: "home-bullets",
                            for line in content.recruit_expectations.iter() {
                                li { "{line}" }
                            }
                        }
                    }
                    if !content.never_ask_title.trim().is_empty() {
                        h3 { class: "home-card-title", "{content.never_ask_title}" }
                    }
                    if !content.never_ask_body.trim().is_empty() {
                        p { class: "home-muted", "{content.never_ask_body}" }
                    }
                }
            }
        }
    }
}

/// Whether the Teams block will render (for secondary CTA targeting).
pub fn teams_will_render(
    sections_teams: bool,
    phase: ListPhase,
    teams_empty: bool,
    show_when_empty: bool,
) -> bool {
    sections_teams && keep_section(phase, !teams_empty, show_when_empty)
}

/// Live panel visibility from shell empty-policy + section flags + list phase.
pub fn live_panel_flags(
    shell_show_empty: bool,
    sections_schedule: bool,
    sections_tournaments: bool,
    events_phase: ListPhase,
    has_events: bool,
    tourneys_phase: ListPhase,
    has_tourneys: bool,
) -> (bool, bool) {
    let show_schedule =
        sections_schedule && keep_section(events_phase, has_events, shell_show_empty);
    let show_tourneys =
        sections_tournaments && keep_section(tourneys_phase, has_tourneys, shell_show_empty);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn empty_overview() -> Overview {
        Overview {
            teams: vec![],
            games: vec![],
            member_count: 0,
            upcoming_matches: vec![],
            recent_results: vec![],
        }
    }

    #[test]
    fn home_records_use_the_shared_record_label() {
        let rec = |wins, losses, draws| TeamRecord {
            wins,
            losses,
            draws,
        };
        assert_eq!(row_record(&rec(5, 2, 0)), "5W-2L");
        assert_eq!(row_record(&rec(5, 2, 1)), "5W-2L-1D");
        assert_eq!(row_record(&rec(0, 0, 0)), "-");
        assert_eq!(row_record(&rec(0, 0, 2)), "0W-0L-2D");
        assert_eq!(card_record(&rec(5, 2, 1)), " · 5W-2L-1D");
        assert_eq!(card_record(&rec(3, 0, 0)), " · 3W-0L");
        assert_eq!(card_record(&rec(0, 0, 0)), "");
    }

    #[test]
    fn empty_copy_only_after_a_ready_empty_list() {
        assert!(!shows_empty_copy(ListPhase::Pending, true));
        assert!(!shows_empty_copy(ListPhase::Failed, true));
        assert!(!shows_empty_copy(ListPhase::Ready, false));
        assert!(shows_empty_copy(ListPhase::Ready, true));
    }

    #[test]
    fn teams_pending_is_loading_and_ready_empty_uses_the_empty_string() {
        fn pending() -> Element {
            rsx! {
                TeamsBlock {
                    content: HomepageContent::default(),
                    overview: None,
                    phase: ListPhase::Pending,
                    presentation: TeamsPresentation::Table,
                    retry: use_signal(|| 0u32),
                }
            }
        }
        fn ready_empty() -> Element {
            rsx! {
                TeamsBlock {
                    content: HomepageContent::default(),
                    overview: Some(empty_overview()),
                    phase: ListPhase::Ready,
                    presentation: TeamsPresentation::Table,
                    retry: use_signal(|| 0u32),
                }
            }
        }
        let empty = HomepageContent::default().teams_empty;
        let pending_html = render(pending);
        assert!(pending_html.contains("Loading"), "{pending_html}");
        assert!(!pending_html.contains(&empty), "{pending_html}");
        let ready_html = render(ready_empty);
        assert!(ready_html.contains(&empty), "{ready_html}");
    }

    #[test]
    fn news_pending_does_not_use_the_empty_string() {
        fn pending() -> Element {
            rsx! {
                NewsBlock {
                    content: HomepageContent::default(),
                    announcements: Vec::new(),
                    phase: ListPhase::Pending,
                    retry: use_signal(|| 0u32),
                }
            }
        }
        let empty = HomepageContent::default().news_empty;
        let html = render(pending);
        assert!(html.contains("Loading"), "{html}");
        assert!(!html.contains(&empty), "{html}");
    }

    fn noop() -> Signal<u32> {
        use_signal(|| 0u32)
    }

    #[test]
    fn failed_lists_show_an_error_and_retry() {
        fn teams() -> Element {
            rsx! {
                TeamsBlock {
                    content: HomepageContent::default(),
                    overview: None,
                    phase: ListPhase::Failed,
                    presentation: TeamsPresentation::Table,
                    retry: noop(),
                }
            }
        }
        fn news() -> Element {
            rsx! {
                NewsBlock {
                    content: HomepageContent::default(),
                    announcements: Vec::new(),
                    phase: ListPhase::Failed,
                    retry: noop(),
                }
            }
        }
        fn live() -> Element {
            rsx! {
                LiveBlock {
                    content: HomepageContent::default(),
                    events: Vec::new(),
                    events_phase: ListPhase::Failed,
                    live_tournaments: Vec::new(),
                    tourneys_phase: ListPhase::Failed,
                    upcoming_matches: Vec::new(),
                    upcoming_phase: ListPhase::Failed,
                    recent_results: Vec::new(),
                    show_schedule: true,
                    show_tourneys: true,
                    show_next_match: true,
                    show_results: true,
                    retry_events: noop(),
                    retry_tourneys: noop(),
                    retry_overview: noop(),
                }
            }
        }
        let teams_html = render(teams);
        assert!(teams_html.contains("load teams."), "{teams_html}");
        assert!(teams_html.contains("fetch-error__retry"), "{teams_html}");
        assert!(
            teams_html.contains("is-compact\" aria-label=\"Retry loading teams\""),
            "{teams_html}"
        );
        assert!(
            !teams_html.contains("role=\"status\" aria-label"),
            "{teams_html}"
        );
        assert!(teams_html.contains("Retry"), "{teams_html}");
        assert!(!teams_html.contains("Loading"), "{teams_html}");
        // The status line holds the text only; Retry comes after it.
        assert!(
            teams_html.contains(
                "<p class=\"home-muted home-list-status\" role=\"status\">Couldn&#39;t load teams.</p><button"
            ),
            "{teams_html}"
        );
        assert_eq!(
            teams_html.matches("role=\"status\"").count(),
            1,
            "{teams_html}"
        );
        let news_html = render(news);
        assert!(news_html.contains("load news."), "{news_html}");
        assert!(news_html.contains("Retry"), "{news_html}");
        let live_html = render(live);
        assert!(live_html.contains("load the schedule."), "{live_html}");
        assert!(live_html.contains("load tournaments."), "{live_html}");
        assert!(live_html.contains("load the next match."), "{live_html}");
        assert!(!live_html.contains("home-results"), "{live_html}");
    }

    #[test]
    fn pending_overview_reserves_metrics_and_the_results_ticker() {
        fn hero() -> Element {
            rsx! {
                HeroBlock {
                    content: HomepageContent::default(),
                    recruitment_open: false,
                    show_secondary_cta: true,
                    secondary_href: "#squads".to_string(),
                }
            }
        }
        fn stats() -> Element {
            rsx! {
                StatsBlock {
                    metric_squads: None,
                    metric_members: None,
                    metric_games: None,
                    metrics_pending: true,
                }
            }
        }
        fn live() -> Element {
            rsx! {
                LiveBlock {
                    content: HomepageContent::default(),
                    events: Vec::new(),
                    events_phase: ListPhase::Pending,
                    live_tournaments: Vec::new(),
                    tourneys_phase: ListPhase::Pending,
                    upcoming_matches: Vec::new(),
                    upcoming_phase: ListPhase::Pending,
                    recent_results: Vec::new(),
                    show_schedule: false,
                    show_tourneys: false,
                    show_next_match: false,
                    show_results: true,
                    retry_events: noop(),
                    retry_tourneys: noop(),
                    retry_overview: noop(),
                }
            }
        }
        // The tiles are a sibling of the hero now, not part of it.
        let hero_html = render(hero);
        assert!(!hero_html.contains("home-stat"), "{hero_html}");
        let stats_html = render(stats);
        assert_eq!(
            stats_html.matches("class=\"home-stat\"").count(),
            3,
            "{stats_html}"
        );
        assert_eq!(
            stats_html.matches("home-skel home-skel-stat").count(),
            3,
            "{stats_html}"
        );
        for label in ["Active squads", "Members", "Game titles"] {
            assert!(stats_html.contains(label), "{stats_html}");
        }
        let live_html = render(live);
        assert!(live_html.contains("class=\"home-results\""), "{live_html}");
        assert!(live_html.contains("Results"), "{live_html}");
        assert!(live_html.contains("Loading"), "{live_html}");
    }

    #[test]
    fn pending_sections_stay_and_failed_sections_follow_the_shell() {
        assert!(keep_section(ListPhase::Pending, false, false));
        assert!(!keep_section(ListPhase::Failed, false, false));
        assert!(keep_section(ListPhase::Failed, false, true));
        assert!(!keep_section(ListPhase::Ready, false, false));
        assert!(keep_section(ListPhase::Ready, true, false));
        let (schedule, tourneys) = live_panel_flags(
            false,
            true,
            true,
            ListPhase::Pending,
            false,
            ListPhase::Pending,
            false,
        );
        assert!(schedule && tourneys);
        let (schedule, tourneys) = live_panel_flags(
            false,
            true,
            true,
            ListPhase::Failed,
            false,
            ListPhase::Failed,
            false,
        );
        assert!(!schedule && !tourneys);
        assert!(teams_will_render(true, ListPhase::Pending, true, false));
        assert!(!teams_will_render(true, ListPhase::Failed, true, false));
        assert!(teams_will_render(true, ListPhase::Failed, true, true));
    }

    #[test]
    fn game_titles_label_is_singular_for_one() {
        assert_eq!(game_titles_label(1), "Game title");
        assert_eq!(game_titles_label(0), "Game titles");
        assert_eq!(game_titles_label(4), "Game titles");
    }

    #[test]
    fn stat_tile_says_game_titles_not_games() {
        fn hero() -> Element {
            rsx! {
                StatsBlock {
                    metric_squads: Some(2),
                    metric_members: Some(10),
                    metric_games: Some(3),
                    metrics_pending: false,
                }
            }
        }
        let html = render(hero);
        assert!(html.contains("Game titles"), "{html}");
        assert!(!html.contains(">Games<"), "{html}");
    }

    fn dirty_home_content() -> HomepageContent {
        let em = '\u{2014}';
        let en = '\u{2013}';
        crate::copy::plain_homepage(&HomepageContent {
            hero_title: format!("Play {em} together"),
            hero_sub: format!("Open {en} always"),
            ethos_rules: vec![format!("Squads of 5{en}10")],
            seeking_tags: vec![format!("DPS {em} Tank")],
            footer_note: format!("Bye {em}"),
            ..Default::default()
        })
    }

    #[test]
    fn normalised_home_copy_reaches_the_dom_without_long_dashes() {
        fn hero() -> Element {
            rsx! {
                HeroBlock {
                    content: dirty_home_content(),
                    recruitment_open: false,
                    show_secondary_cta: true,
                    secondary_href: "#squads".to_string(),
                }
            }
        }
        fn ethos() -> Element {
            rsx! {
                EthosBlock { content: dirty_home_content() }
            }
        }
        let long_dash = |s: &str| s.chars().any(|c| matches!(c, '\u{2014}' | '\u{2013}'));
        let hero_html = render(hero);
        assert!(!long_dash(&hero_html), "{hero_html}");
        assert!(hero_html.contains("Play, together"), "{hero_html}");
        let ethos_html = render(ethos);
        assert!(!long_dash(&ethos_html), "{ethos_html}");
        assert!(ethos_html.contains("Squads of 5 to 10"), "{ethos_html}");
    }

    // -- Redesign (S4a): frame markup -------------------------------------

    use std::cell::RefCell;

    /// What the one routed test scene renders. Blocks with a `Link` need a
    /// router above them.
    #[derive(Clone, PartialEq)]
    enum Scene {
        Hero(ContentAlign, bool),
        Teams(TeamsPresentation),
        News,
        Recruit,
        Live,
    }

    thread_local! {
        static SCENE: RefCell<Scene> = const { RefCell::new(Scene::News) };
    }

    #[derive(Clone, Debug, PartialEq, Routable)]
    enum TestRoute {
        #[route("/")]
        Stage {},
    }

    fn team(id: &str, name: &str, roster_count: usize) -> OverviewTeam {
        OverviewTeam {
            id: id.to_string(),
            name: name.to_string(),
            game_id: "ow".to_string(),
            division: None,
            lore_quote: None,
            roster_count,
            record: TeamRecord {
                wins: 3,
                losses: 1,
                draws: 0,
            },
        }
    }

    fn overview_with_teams() -> Overview {
        Overview {
            teams: vec![team("t1", "Squad Alpha", 8), team("t2", "Crew Beta", 0)],
            games: vec![super::super::data::OverviewGame {
                id: "ow".to_string(),
                name: "Overwatch".to_string(),
            }],
            member_count: 24,
            upcoming_matches: vec![],
            recent_results: vec![],
        }
    }

    fn result(id: &str, outcome: &str) -> RecentResult {
        RecentResult {
            id: id.to_string(),
            team_id: "t1".to_string(),
            team_name: "Squad Alpha".to_string(),
            opponent: "Crew Beta".to_string(),
            score_us: Some(3),
            score_them: Some(1),
            outcome: outcome.to_string(),
            match_type: "scrim".to_string(),
            played_at: "2026-11-01T19:00:00Z".to_string(),
        }
    }

    fn recruit_content() -> HomepageContent {
        HomepageContent {
            seeking_tags: vec!["Tank".to_string(), "Support".to_string()],
            ..Default::default()
        }
    }

    #[component]
    fn Stage() -> Element {
        match SCENE.with(|slot| slot.borrow().clone()) {
            Scene::Hero(align, recruitment_open) => rsx! {
                HeroBlock {
                    content: HomepageContent {
                        content_align: align,
                        ..Default::default()
                    },
                    recruitment_open,
                    show_secondary_cta: true,
                    secondary_href: "#squads".to_string(),
                }
            },
            Scene::Teams(presentation) => rsx! {
                TeamsBlock {
                    content: HomepageContent::default(),
                    overview: Some(overview_with_teams()),
                    phase: ListPhase::Ready,
                    presentation,
                    retry: noop(),
                }
            },
            Scene::News => rsx! {
                NewsBlock {
                    content: HomepageContent::default(),
                    announcements: vec![
                        Announcement {
                            id: "a1".to_string(),
                            title: "Stat Tracker update".to_string(),
                            content: "Hero selects and team bans.".to_string(),
                            pinned: true,
                            created_at: "2026-11-02T10:00:00Z".to_string(),
                        },
                        Announcement {
                            id: "a2".to_string(),
                            title: "Scrim night".to_string(),
                            content: "Friday.".to_string(),
                            pinned: false,
                            created_at: "2026-11-01T10:00:00Z".to_string(),
                        },
                    ],
                    phase: ListPhase::Ready,
                    retry: noop(),
                }
            },
            Scene::Recruit => rsx! {
                RecruitBlock { content: recruit_content() }
            },
            Scene::Live => rsx! {
                LiveBlock {
                    content: HomepageContent::default(),
                    events: Vec::new(),
                    events_phase: ListPhase::Ready,
                    live_tournaments: vec![
                        HomeTournament {
                            id: "c1".to_string(),
                            name: "Autumn Cup".to_string(),
                            format: "single_elim".to_string(),
                            status: "in_progress".to_string(),
                            is_external: false,
                            starts_at: None,
                        },
                    ],
                    tourneys_phase: ListPhase::Ready,
                    upcoming_matches: Vec::new(),
                    upcoming_phase: ListPhase::Ready,
                    recent_results: vec![
                        result("r1", "win"),
                        result("r2", "loss"),
                        result("r3", "unknown"),
                    ],
                    show_schedule: false,
                    show_tourneys: true,
                    show_next_match: false,
                    show_results: true,
                    retry_events: noop(),
                    retry_tourneys: noop(),
                    retry_overview: noop(),
                }
            },
        }
    }

    fn routed_app() -> Element {
        let history =
            use_hook(|| std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path("/")));
        rsx! {
            dioxus::router::components::HistoryProvider {
                history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                Router::<TestRoute> {}
            }
        }
    }

    fn render_scene(scene: Scene) -> String {
        SCENE.with(|slot| *slot.borrow_mut() = scene);
        render(routed_app)
    }

    /// No control nests another: `<a>` holds no `<button>` or `<a>`.
    fn assert_no_nested_controls(html: &str) {
        for chunk in html.split("<a ").skip(1) {
            let inner = chunk.split("</a>").next().unwrap_or_default();
            assert!(!inner.contains("<button"), "button in link: {inner}");
            assert!(!inner.contains("<a "), "link in link: {inner}");
        }
    }

    #[test]
    fn hero_follows_the_alignment_setting_and_has_the_only_h1() {
        assert_eq!(hero_class(ContentAlign::Left), "home-hero");
        assert_eq!(hero_class(ContentAlign::Center), "home-hero is-centered");
        let html = render_scene(Scene::Hero(ContentAlign::Center, true));
        assert!(
            html.contains("<section class=\"home-hero is-centered\">"),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(!html.contains("<main"), "{html}");
        assert!(!html.contains("<header"), "{html}");
        let badge = HomepageContent::default().hero_badge;
        assert!(
            html.contains(&format!("<div class=\"home-banner\">{badge}</div>")),
            "{html}"
        );
        // Banner, title, lead, actions, in the frame's order.
        let order: Vec<usize> = [
            "home-banner",
            "home-hero-title",
            "home-hero-sub",
            "home-actions",
        ]
        .iter()
        .map(|class| {
            html.find(class)
                .unwrap_or_else(|| panic!("{class}: {html}"))
        })
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{html}");
        assert!(html.contains("class=\"btn is-primary\""), "{html}");
        assert!(
            html.contains("href=\"#squads\" class=\"btn is-ghost\""),
            "{html}"
        );
        assert_no_nested_controls(&html);

        let left = render_scene(Scene::Hero(ContentAlign::Left, false));
        assert!(left.contains("<section class=\"home-hero\">"), "{left}");
        assert!(!left.contains("is-primary"), "{left}");
    }

    #[test]
    fn hero_drops_an_empty_banner_and_an_empty_action_row() {
        fn hero() -> Element {
            rsx! {
                HeroBlock {
                    content: HomepageContent {
                        hero_badge: "  ".to_string(),
                        ..Default::default()
                    },
                    recruitment_open: false,
                    show_secondary_cta: false,
                    secondary_href: "#squads".to_string(),
                }
            }
        }
        let html = render(hero);
        assert!(!html.contains("home-banner"), "{html}");
        assert!(!html.contains("home-actions"), "{html}");
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
    }

    #[test]
    fn stat_tiles_put_the_value_over_its_label_and_skip_missing_ones() {
        fn stats() -> Element {
            rsx! {
                StatsBlock {
                    metric_squads: Some(1),
                    metric_members: Some(24),
                    metric_games: None,
                    metrics_pending: false,
                }
            }
        }
        fn none() -> Element {
            rsx! {
                StatsBlock {
                    metric_squads: None,
                    metric_members: None,
                    metric_games: None,
                    metrics_pending: false,
                }
            }
        }
        let html = render(stats);
        assert_eq!(html.matches("class=\"home-stat\"").count(), 2, "{html}");
        assert!(
            html.contains(
                "<span class=\"home-stat-n\">1</span><span class=\"home-stat-l\">Active squad</span>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<span class=\"home-stat-n\">24</span><span class=\"home-stat-l\">Members</span>"
            ),
            "{html}"
        );
        assert!(!html.contains("home-skel"), "{html}");
        assert!(!render(none).contains("home-stats"));
    }

    #[test]
    fn count_labels_are_singular_for_one() {
        assert_eq!(squads_label(1), "Active squad");
        assert_eq!(squads_label(2), "Active squads");
        assert_eq!(members_label(1), "Member");
        assert_eq!(members_label(24), "Members");
    }

    #[test]
    fn ethos_is_a_headed_section_with_a_numbered_list() {
        fn ethos() -> Element {
            rsx! {
                EthosBlock { content: HomepageContent::default() }
            }
        }
        let html = render(ethos);
        let content = HomepageContent::default();
        assert!(
            html.contains("<section id=\"ethos\" class=\"home-section\">"),
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "<p class=\"home-kicker\">{}</p><h2 class=\"ui-section-title\">{}</h2>",
                content.ethos_kicker, content.ethos_title
            )),
            "{html}"
        );
        assert!(
            html.contains("<ol class=\"home-rules\" role=\"list\">"),
            "{html}"
        );
        assert_eq!(
            html.matches("<li>").count(),
            content.ethos_rules.len(),
            "{html}"
        );
        // The list already numbers the rules; the printed digits are decoration.
        assert!(
            html.contains("<span class=\"home-rule-num\" aria-hidden=\"true\">01</span>"),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 0, "{html}");
    }

    #[test]
    fn teams_table_cards_and_chips_link_each_team_once() {
        let table = render_scene(Scene::Teams(TeamsPresentation::Table));
        assert!(
            table.contains("class=\"home-team-table\" role=\"table\""),
            "{table}"
        );
        assert_eq!(table.matches("role=\"columnheader\"").count(), 5, "{table}");
        assert_eq!(table.matches("role=\"row\"").count(), 3, "{table}");
        assert_eq!(table.matches("role=\"cell\"").count(), 10, "{table}");
        // A forming squad says so in words.
        assert!(
            table.contains("<span class=\"ui-pill ui-pill--warn\">Forming</span>"),
            "{table}"
        );
        assert!(table.contains(">3W-1L<"), "{table}");

        let cards = render_scene(Scene::Teams(TeamsPresentation::Cards));
        assert!(
            cards.contains("class=\"home-card home-team-card\""),
            "{cards}"
        );
        assert!(
            cards.contains("class=\"home-card home-team-card is-forming\""),
            "{cards}"
        );
        assert!(
            cards.contains("<h3 class=\"home-card-title\">Squad Alpha</h3>"),
            "{cards}"
        );
        assert!(cards.contains("Overwatch · 8 players · 3W-1L"), "{cards}");
        assert!(cards.contains("Overwatch · Open roster"), "{cards}");

        let chips = render_scene(Scene::Teams(TeamsPresentation::Compact));
        assert!(chips.contains("Squad Alpha · Overwatch<"), "{chips}");
        assert!(
            chips.contains("Crew Beta · Overwatch · Forming<"),
            "{chips}"
        );

        for html in [&table, &cards, &chips] {
            assert_eq!(html.matches("href=\"/teams/t1\"").count(), 1, "{html}");
            assert_eq!(html.matches("<h2").count(), 1, "{html}");
            assert_eq!(html.matches("<h1").count(), 0, "{html}");
            assert_no_nested_controls(html);
        }
    }

    #[test]
    fn news_is_a_card_per_announcement_with_a_pinned_badge() {
        let html = render_scene(Scene::News);
        assert_eq!(
            html.matches("<article class=\"home-card home-news-card\">")
                .count(),
            2,
            "{html}"
        );
        assert_eq!(
            html.matches("<span class=\"ui-pill ui-pill--neutral\">Pinned</span>")
                .count(),
            1,
            "{html}"
        );
        assert!(
            html.contains("<time class=\"entry-date\" datetime=\"2026-11-02\">2 Nov 2026</time>"),
            "{html}"
        );
        assert!(
            html.contains("<h3 class=\"home-card-title\">Stat Tracker update</h3>"),
            "{html}"
        );
        // Each card starts with its heading; the badge and date follow it in
        // the document and are drawn above it by `order`.
        assert_eq!(
            html.matches(
                "<article class=\"home-card home-news-card\"><h3 class=\"home-card-title\">"
            )
            .count(),
            2,
            "{html}"
        );
        assert_eq!(
            html.matches("</h3><div class=\"home-news-meta\">").count(),
            2,
            "{html}"
        );
        let css = super::super::css::home_css_layers();
        assert!(
            css.contains(".home-news-card .home-news-meta { order: -1;"),
            "the meta row is drawn first"
        );
        let view_all = HomepageContent::default().news_view_all;
        assert!(
            html.contains(&format!("class=\"home-link\">{view_all}</a>")),
            "{html}"
        );
        assert_no_nested_controls(&html);
    }

    #[test]
    fn recruit_is_two_cards_with_apply_and_the_seeking_tags() {
        let html = render_scene(Scene::Recruit);
        let content = recruit_content();
        assert!(
            html.contains("<section id=\"recruit\" class=\"home-grid-2\">"),
            "{html}"
        );
        assert_eq!(
            html.matches("<article class=\"home-card\">").count(),
            2,
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "class=\"btn is-primary\">{}</a>",
                content.recruit_cta
            )),
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "<span class=\"home-tag-label\">{}</span>\
                 <span class=\"home-tag\">Tank</span><span class=\"home-tag\">Support</span>",
                content.seeking_label
            )),
            "{html}"
        );
        // H2 for each card, H3 under it: no skipped level.
        assert_eq!(html.matches("<h2").count(), 2, "{html}");
        assert!(
            html.contains(&format!(
                "<h3 class=\"home-card-title\">{}</h3>",
                content.never_ask_title
            )),
            "{html}"
        );
        assert!(!html.contains("<h4"), "{html}");
        assert!(!html.contains("style="), "{html}");
        assert_no_nested_controls(&html);
    }

    #[test]
    fn results_say_win_or_loss_in_words() {
        assert_eq!(outcome_badge("win"), Some(("Win", PillTone::Ok)));
        assert_eq!(outcome_badge("loss"), Some(("Loss", PillTone::Danger)));
        assert_eq!(outcome_badge("draw"), Some(("Draw", PillTone::Neutral)));
        assert_eq!(outcome_badge("unknown"), None);
        assert_eq!(tournament_badge("in_progress").0, "Live");
        assert_eq!(tournament_badge("registration").0, "Open");

        let html = render_scene(Scene::Live);
        assert!(
            html.contains("class=\"home-results\" role=\"region\" aria-label=\"Recent results\""),
            "{html}"
        );
        assert_eq!(html.matches("class=\"home-result\"").count(), 3, "{html}");
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--ok\">Win</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--danger\">Loss</span>"),
            "{html}"
        );
        // One tournament panel on its own takes the full row.
        assert!(html.contains("class=\"home-grid-2 is-single\""), "{html}");
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--warn\">Live</span>"),
            "{html}"
        );
        // Dates use the same day month year text as News and Blog.
        assert!(
            html.contains("<span class=\"home-result-date\">1 Nov 2026</span>"),
            "{html}"
        );
        assert_no_nested_controls(&html);
    }

    #[test]
    fn dates_read_day_month_year() {
        assert_eq!(match_when("2026-11-12T19:00:00Z"), "12 Nov 2026, 19:00 UTC");
        assert_eq!(match_when("2026-11-12"), "12 Nov 2026");
        assert_eq!(match_when("soon"), "soon");
        assert_eq!(result_date("2026-11-01T19:00:00Z"), "1 Nov 2026");
        assert_eq!(result_date("someday"), "someday");
    }

    #[test]
    fn list_status_line_is_in_every_phase_and_only_its_text_changes() {
        assert_eq!(
            list_status_text(ListPhase::Pending, "the schedule"),
            "Loading the schedule"
        );
        assert_eq!(
            list_status_text(ListPhase::Failed, "the schedule"),
            "Couldn't load the schedule."
        );
        assert_eq!(list_status_text(ListPhase::Ready, "the schedule"), "");
        // Retry shows for a failure, and stays (busy) for its own refetch.
        assert_eq!(list_retry(ListPhase::Failed, false), Some(false));
        assert_eq!(list_retry(ListPhase::Failed, true), Some(false));
        assert_eq!(list_retry(ListPhase::Pending, true), Some(true));
        assert_eq!(list_retry(ListPhase::Pending, false), None);
        assert_eq!(list_retry(ListPhase::Ready, true), None);

        fn pending() -> Element {
            rsx! { ListStatus { phase: ListPhase::Pending, subject: "news", retry: noop() } }
        }
        fn ready() -> Element {
            rsx! { ListStatus { phase: ListPhase::Ready, subject: "news", retry: noop() } }
        }
        assert_eq!(
            render(pending),
            "<p class=\"home-muted home-list-status\" role=\"status\">Loading news</p>"
        );
        assert_eq!(
            render(ready),
            "<p class=\"home-muted home-list-status\" role=\"status\"></p>"
        );
        // A ready list keeps the (empty) line in its panel.
        let html = render_scene(Scene::News);
        assert!(
            html.contains("<p class=\"home-muted home-list-status\" role=\"status\"></p>"),
            "{html}"
        );
    }
}
