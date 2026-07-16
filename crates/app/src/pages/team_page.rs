use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{Card, Pill, PillTone};
use crate::routes::Route;

use super::public_fetch::{PublicFetch, fetch_public};

#[derive(Debug, Clone, Deserialize)]
struct RosterMember {
    member_id: String,
    display_name: String,
    avatar_url: Option<String>,
    team_role: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TeamRecord {
    wins: u32,
    losses: u32,
    draws: u32,
}

/// Public match row. Intentionally omits `notes` / `recorded_by` — the public
/// projection is being narrowed and those fields must not be depended on.
#[derive(Debug, Clone, Deserialize)]
struct TeamMatch {
    /// Kept for future deep links to match pages.
    #[allow(dead_code)]
    id: String,
    opponent: String,
    score_us: Option<u32>,
    score_them: Option<u32>,
    map_name: Option<String>,
    game_mode: Option<String>,
    match_type: String,
    /// RFC3339 when played; server recent_matches only returns played rows.
    #[serde(default)]
    played_at: Option<String>,
    #[serde(default)]
    scheduled_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TeamEvent {
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TeamDetailData {
    #[allow(dead_code)]
    id: String,
    name: String,
    division: Option<String>,
    lore_quote: Option<String>,
    logo_url: Option<String>,
    color: Option<String>,
    game_name: Option<String>,
    #[serde(default)]
    roster: Vec<RosterMember>,
    record: TeamRecord,
    #[serde(default)]
    recent_matches: Vec<TeamMatch>,
    #[serde(default)]
    upcoming_events: Vec<TeamEvent>,
}

fn day_name(d: u8) -> &'static str {
    match d {
        0 => "Mon",
        1 => "Tue",
        2 => "Wed",
        3 => "Thu",
        4 => "Fri",
        5 => "Sat",
        6 => "Sun",
        _ => "—",
    }
}

const PAGE_CSS: &str = r#"
    .team-page {
        padding: 3rem 2rem;
        max-width: 800px;
        margin: 0 auto;
    }
    .team-page-loading,
    .team-page-missing {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
    .team-page-header {
        display: flex;
        align-items: center;
        gap: 2rem;
        margin-bottom: 2.5rem;
    }
    .team-page-logo {
        width: 120px;
        height: 120px;
        border-radius: var(--radius-md, 9px);
        overflow: hidden;
        background: var(--surface-2);
        border: 1px solid var(--border);
        display: flex;
        align-items: center;
        justify-content: center;
        flex-shrink: 0;
    }
    .team-page-logo img {
        width: 100%;
        height: 100%;
        object-fit: cover;
    }
    .team-page-logo .team-initials {
        font-family: var(--font-head);
        font-size: 2.4rem;
        color: var(--accent);
        letter-spacing: 3px;
    }
    .team-page-info {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
        min-width: 0;
    }
    .team-page-name {
        font-family: var(--font-head);
        font-size: 2.2rem;
        color: var(--text);
        letter-spacing: 2px;
        margin: 0;
        line-height: 1;
    }
    .team-page-pills {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        flex-wrap: wrap;
    }
    .team-page-game {
        font-family: var(--font-mono);
        font-size: 0.7rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--text-3);
        margin: 0;
    }
    .team-page-lore {
        color: var(--text-3);
        font-size: 0.85rem;
        font-style: italic;
        margin: 0;
    }
    .team-page-record {
        font-family: var(--font-mono);
        font-size: 0.85rem;
        color: var(--text-2);
        font-variant-numeric: tabular-nums;
        margin: 0;
    }
    .team-page-record strong {
        color: var(--text);
    }
    .team-section {
        margin-bottom: 2rem;
    }
    .team-section h2 {
        font-family: var(--font-head);
        font-size: 1.2rem;
        font-weight: 700;
        color: var(--text);
        margin: 0 0 0.75rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .team-roster-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
        gap: 0.75rem;
    }
    .team-roster-link {
        display: block;
        color: inherit;
        text-decoration: none;
        min-width: 0;
    }
    .team-roster-row {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        min-width: 0;
    }
    .team-roster-avatar {
        width: 40px;
        height: 40px;
        border-radius: 50%;
        overflow: hidden;
        background: var(--surface-2);
        display: flex;
        align-items: center;
        justify-content: center;
        flex-shrink: 0;
    }
    .team-roster-avatar img {
        width: 100%;
        height: 100%;
        object-fit: cover;
    }
    .team-roster-avatar .member-initials {
        font-family: var(--font-head);
        font-size: 0.9rem;
        color: var(--accent);
        letter-spacing: 1px;
    }
    .team-roster-name {
        font-family: var(--font-head);
        font-weight: 700;
        color: var(--text);
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .team-match-rows {
        border-top: 1px solid var(--border);
    }
    .team-match-row {
        display: grid;
        grid-template-columns: 4.5rem 3.5rem minmax(0, 1fr) auto;
        column-gap: 1rem;
        align-items: baseline;
        padding: 0.7rem 0.5rem;
        border-bottom: 1px solid var(--border);
        font-size: 0.88rem;
    }
    .team-match-date {
        font-family: var(--font-mono);
        font-size: 0.7rem;
        color: var(--text-3);
        white-space: nowrap;
    }
    .team-match-score {
        font-family: var(--font-mono);
        font-weight: 700;
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
    }
    .team-match-score.win { color: var(--ok, #22c55e); }
    .team-match-score.loss { color: var(--danger, #ef4444); }
    .team-match-score.draw { color: var(--text-2); }
    .team-match-opponent {
        color: var(--text);
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .team-match-meta {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: var(--text-3);
        text-align: right;
        white-space: nowrap;
    }
    .team-schedule-list {
        list-style: none;
        margin: 0;
        padding: 0;
        border-top: 1px solid var(--border);
    }
    .team-schedule-list li {
        display: flex;
        justify-content: space-between;
        align-items: baseline;
        gap: 1rem;
        padding: 0.7rem 0.5rem;
        border-bottom: 1px solid var(--border);
        font-size: 0.88rem;
        color: var(--text);
    }
    .team-schedule-meta {
        font-family: var(--font-mono);
        font-size: 0.7rem;
        color: var(--text-3);
        white-space: nowrap;
    }
    .team-page-back {
        margin-top: 2rem;
        padding-top: 1.5rem;
        border-top: 1px solid var(--border);
    }
    .team-page-back a {
        color: var(--accent);
        text-decoration: none;
        font-size: 0.85rem;
        font-weight: 600;
    }
    .team-page-back a:hover {
        text-decoration: underline;
    }
    @media (max-width: 600px) {
        .team-page-header {
            flex-direction: column;
            text-align: center;
        }
        .team-page-pills {
            justify-content: center;
        }
        .team-match-row {
            grid-template-columns: 3.5rem minmax(0, 1fr);
        }
        .team-match-date,
        .team-match-meta { display: none; }
    }
"#;

#[component]
pub fn TeamPage(id: String) -> Element {
    let id_clone = id.clone();
    let team = use_resource(move || {
        let id = id_clone.clone();
        async move {
            if id.is_empty() {
                return PublicFetch::NotFound;
            }
            fetch_public::<TeamDetailData>(&format!("/api/public/teams/{id}")).await
        }
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "team-page",
            {
                let data = team.read();
                match data.as_ref() {
                    None => rsx! { p { class: "team-page-loading", "Loading..." } },
                    Some(PublicFetch::NotFound) => rsx! { p { class: "team-page-missing", "Team not found" } },
                    Some(PublicFetch::Failed) => rsx! {
                        p { class: "team-page-missing", "Couldn't load this team. Check your connection and try again." }
                    },
                    Some(PublicFetch::Found(t)) => rsx! {
                        {render_header(t)}

                        if !t.roster.is_empty() {
                            div { class: "team-section",
                                h2 { "Roster" }
                                div { class: "team-roster-grid",
                                    for m in t.roster.iter() {
                                        {render_roster_card(m)}
                                    }
                                }
                            }
                        }

                        if !t.recent_matches.is_empty() {
                            div { class: "team-section",
                                h2 { "Recent Matches" }
                                div { class: "team-match-rows",
                                    for m in t.recent_matches.iter() {
                                        {render_match_row(m)}
                                    }
                                }
                            }
                        }

                        if !t.upcoming_events.is_empty() {
                            div { class: "team-section",
                                h2 { "Schedule" }
                                ul { class: "team-schedule-list",
                                    for e in t.upcoming_events.iter() {
                                        {
                                            let day = day_name(e.day_of_week);
                                            rsx! {
                                                li {
                                                    span { "{e.title}" }
                                                    span { class: "team-schedule-meta",
                                                        "{day} · {e.time} {e.timezone}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        div { class: "team-page-back",
                            Link { to: Route::Home {}, "Back to home" }
                        }
                    },
                }
            }
        }
    }
}

fn team_initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

fn render_header(t: &TeamDetailData) -> Element {
    let initials = team_initials(&t.name);
    let lore = t.lore_quote.clone().unwrap_or_default();
    let game_name = t.game_name.clone().unwrap_or_default();
    let logo_style = t
        .color
        .as_ref()
        .map(|c| format!("border-color: {c};"))
        .unwrap_or_default();

    rsx! {
        div { class: "team-page-header",
            div { class: "team-page-logo", style: "{logo_style}",
                if let Some(url) = &t.logo_url {
                    img { src: "{url}", alt: "{t.name}" }
                } else {
                    span { class: "team-initials", "{initials}" }
                }
            }
            div { class: "team-page-info",
                h1 { class: "team-page-name", "{t.name}" }
                div { class: "team-page-pills",
                    if let Some(division) = &t.division {
                        Pill { tone: PillTone::Accent, "{division}" }
                    }
                }
                if !game_name.is_empty() {
                    p { class: "team-page-game", "{game_name}" }
                }
                if !lore.is_empty() {
                    p { class: "team-page-lore", "“{lore}”" }
                }
                p { class: "team-page-record",
                    strong { "{t.record.wins}W" }
                    " – "
                    strong { "{t.record.losses}L" }
                    " – "
                    strong { "{t.record.draws}D" }
                }
            }
        }
    }
}

fn render_roster_card(m: &RosterMember) -> Element {
    let initials = team_initials(&m.display_name);
    let role_tone = match m.team_role.as_str() {
        "captain" => PillTone::Warn,
        "coach" => PillTone::Accent,
        _ => PillTone::Neutral,
    };

    rsx! {
        Link {
            to: Route::MemberProfile { id: m.member_id.clone() },
            class: "team-roster-link",
            Card {
                div { class: "team-roster-row",
                    div { class: "team-roster-avatar",
                        if let Some(url) = &m.avatar_url {
                            img { src: "{url}", alt: "{m.display_name}" }
                        } else {
                            span { class: "member-initials", "{initials}" }
                        }
                    }
                    span { class: "team-roster-name", "{m.display_name}" }
                    Pill { tone: role_tone, "{m.team_role}" }
                }
            }
        }
    }
}

fn render_match_row(m: &TeamMatch) -> Element {
    let date: String = m
        .played_at
        .as_deref()
        .or(m.scheduled_at.as_deref())
        .map(|s| s.chars().take(10).collect())
        .unwrap_or_else(|| "TBD".into());
    let (score, outcome_class) = match (m.score_us, m.score_them) {
        (Some(us), Some(them)) => {
            let class = match us.cmp(&them) {
                std::cmp::Ordering::Greater => "win",
                std::cmp::Ordering::Less => "loss",
                std::cmp::Ordering::Equal => "draw",
            };
            (format!("{us}–{them}"), class)
        }
        _ => ("—".to_string(), "draw"),
    };
    let mut meta_parts: Vec<String> = Vec::new();
    if let Some(map) = &m.map_name
        && !map.is_empty()
    {
        meta_parts.push(map.clone());
    }
    if let Some(mode) = &m.game_mode
        && !mode.is_empty()
    {
        meta_parts.push(mode.clone());
    }
    meta_parts.push(m.match_type.clone());
    let meta = meta_parts.join(" · ");

    rsx! {
        div { class: "team-match-row",
            span { class: "team-match-date", "{date}" }
            span { class: "team-match-score {outcome_class}", "{score}" }
            span { class: "team-match-opponent", "vs {m.opponent}" }
            span { class: "team-match-meta", "{meta}" }
        }
    }
}
