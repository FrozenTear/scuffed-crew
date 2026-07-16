//! Public match detail page — VOD / replay / scores for a single fixture.

use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{Card, Pill, PillTone};
use crate::routes::Route;

use super::public_fetch::{PublicFetch, fetch_public};

#[derive(Debug, Clone, Deserialize)]
struct MatchDetailData {
    id: String,
    team_id: String,
    opponent: String,
    #[serde(default)]
    score_us: Option<u32>,
    #[serde(default)]
    score_them: Option<u32>,
    map_name: Option<String>,
    game_mode: Option<String>,
    match_type: String,
    #[serde(default)]
    played_at: Option<String>,
    #[serde(default)]
    scheduled_at: Option<String>,
    #[serde(default)]
    vod_url: Option<String>,
    #[serde(default)]
    replay_code: Option<String>,
    team_name: String,
    game_name: Option<String>,
}

const PAGE_CSS: &str = r#"
    .match-page {
        padding: 3rem 2rem;
        max-width: 720px;
        margin: 0 auto;
    }
    .match-status {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
    .match-header {
        margin-bottom: 1.75rem;
    }
    .match-kicker {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--text-3);
        margin-bottom: 0.4rem;
    }
    .match-vs {
        font-family: var(--font-head);
        font-size: clamp(1.5rem, 3vw, 2rem);
        font-weight: 700;
        color: var(--text);
        margin: 0 0 0.5rem;
        line-height: 1.2;
    }
    .match-vs .opp { color: var(--accent); }
    .match-meta-row {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
        align-items: center;
        margin-top: 0.6rem;
    }
    .match-score {
        font-family: var(--font-head);
        font-size: 2.4rem;
        font-weight: 700;
        letter-spacing: 0.04em;
        margin: 0.5rem 0 0;
    }
    .match-score.win { color: var(--success, #22c55e); }
    .match-score.loss { color: var(--danger); }
    .match-score.draw { color: var(--warn); }
    .match-score.pending { color: var(--text-3); font-size: 1.4rem; }
    .match-section {
        margin-top: 1.5rem;
    }
    .match-section h2 {
        font-family: var(--font-head);
        font-size: 1rem;
        text-transform: uppercase;
        letter-spacing: 0.06em;
        color: var(--text-3);
        margin: 0 0 0.75rem;
    }
    .match-media a {
        color: var(--accent);
        font-weight: 600;
        text-decoration: none;
    }
    .match-media a:hover { text-decoration: underline; }
    .match-replay {
        font-family: var(--font-mono);
        font-size: 1.1rem;
        letter-spacing: 0.08em;
        color: var(--text);
        background: var(--surface-2);
        border: 1px solid var(--border);
        padding: 0.6rem 0.85rem;
        display: inline-block;
        border-radius: 6px;
    }
    .match-back {
        margin-top: 2rem;
        padding-top: 1.25rem;
        border-top: 1px solid var(--border);
    }
    .match-back a {
        color: var(--accent);
        font-weight: 600;
        text-decoration: none;
        font-size: 0.9rem;
    }
    .match-back a:hover { text-decoration: underline; }
    .match-dl {
        display: grid;
        grid-template-columns: auto 1fr;
        gap: 0.35rem 1rem;
        font-size: 0.9rem;
    }
    .match-dl dt {
        color: var(--text-3);
        font-family: var(--font-mono);
        font-size: 0.72rem;
        text-transform: uppercase;
        letter-spacing: 0.06em;
        padding-top: 0.15rem;
    }
    .match-dl dd { margin: 0; color: var(--text); }
"#;

#[component]
pub fn MatchDetail(id: String) -> Element {
    let match_id = id.clone();
    let data = use_resource(move || {
        let id = match_id.clone();
        async move {
            if id.is_empty() {
                return PublicFetch::NotFound;
            }
            fetch_public::<MatchDetailData>(&format!("/api/public/matches/{id}")).await
        }
    });

    rsx! {
        style { {PAGE_CSS} }
        main { class: "match-page",
            {
                match data.read().as_ref() {
                    None => rsx! { p { class: "match-status", "Loading..." } },
                    Some(PublicFetch::NotFound) => rsx! {
                        p { class: "match-status", "Match not found." }
                        div { class: "match-back",
                            Link { to: Route::Home {}, "Back home" }
                        }
                    },
                    Some(PublicFetch::Failed) => rsx! {
                        p { class: "match-status", "Couldn't load this match. Try again later." }
                        div { class: "match-back",
                            Link { to: Route::Home {}, "Back home" }
                        }
                    },
                    Some(PublicFetch::Found(m)) => rsx! { {render_match(m)} },
                }
            }
        }
    }
}

fn render_match(m: &MatchDetailData) -> Element {
    let (score_text, score_class) = match (m.score_us, m.score_them) {
        (Some(u), Some(t)) => {
            let class = match u.cmp(&t) {
                std::cmp::Ordering::Greater => "win",
                std::cmp::Ordering::Less => "loss",
                std::cmp::Ordering::Equal => "draw",
            };
            (format!("{u}–{t}"), class)
        }
        _ => ("Scheduled".into(), "pending"),
    };
    let when = m
        .played_at
        .as_deref()
        .or(m.scheduled_at.as_deref())
        .map(|s| s.chars().take(16).collect::<String>().replace('T', " "))
        .unwrap_or_else(|| "TBD".into());
    let game = m.game_name.clone().unwrap_or_default();
    let has_media = m.vod_url.as_ref().is_some_and(|u| !u.is_empty())
        || m.replay_code.as_ref().is_some_and(|c| !c.is_empty());

    rsx! {
        div { class: "match-header",
            p { class: "match-kicker",
                if game.is_empty() { "{m.match_type}" } else { "{game} · {m.match_type}" }
            }
            h1 { class: "match-vs",
                "{m.team_name} "
                span { class: "opp", "vs {m.opponent}" }
            }
            p { class: "match-score {score_class}", "{score_text}" }
            div { class: "match-meta-row",
                Pill { tone: PillTone::Neutral, "{when} UTC" }
                if let Some(map) = &m.map_name {
                    if !map.is_empty() {
                        Pill { tone: PillTone::Accent, "{map}" }
                    }
                }
                if let Some(mode) = &m.game_mode {
                    if !mode.is_empty() {
                        Pill { tone: PillTone::Neutral, "{mode}" }
                    }
                }
            }
        }

        Card {
            div { class: "match-section",
                h2 { "Details" }
                dl { class: "match-dl",
                    dt { "Team" }
                    dd {
                        Link {
                            to: Route::TeamPage { id: m.team_id.clone() },
                            "{m.team_name}"
                        }
                    }
                    dt { "Opponent" }
                    dd { "{m.opponent}" }
                    if let Some(pa) = &m.played_at {
                        dt { "Played" }
                        dd { "{pa}" }
                    }
                    if let Some(sa) = &m.scheduled_at {
                        dt { "Scheduled" }
                        dd { "{sa}" }
                    }
                }
            }
        }

        if has_media {
            Card {
                div { class: "match-section match-media",
                    h2 { "Media" }
                    if let Some(url) = &m.vod_url {
                        if !url.is_empty() {
                            p {
                                a {
                                    href: "{url}",
                                    target: "_blank",
                                    rel: "noopener noreferrer",
                                    "Watch VOD"
                                }
                            }
                        }
                    }
                    if let Some(code) = &m.replay_code {
                        if !code.is_empty() {
                            p { class: "match-replay", "Replay: {code}" }
                        }
                    }
                }
            }
        }

        div { class: "match-back",
            Link {
                to: Route::TeamPage { id: m.team_id.clone() },
                "← Back to {m.team_name}"
            }
        }
    }
}
