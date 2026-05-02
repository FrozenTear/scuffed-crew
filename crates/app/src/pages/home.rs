use dioxus::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

use scuffed_api_client::ApiClient;
use crate::components::SectionHeader;
use crate::components::bracket::BRACKET_STYLES;
use crate::routes::Route;

// --- Data types ---

#[derive(Debug, Clone, Deserialize)]
struct Overview {
    teams: Vec<OverviewTeam>,
    games: Vec<OverviewGame>,
    member_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct OverviewGame { id: String, name: String }

#[derive(Debug, Clone, Deserialize)]
struct OverviewTeam {
    name: String,
    game_id: String,
    division: Option<String>,
    lore_quote: Option<String>,
    roster_count: usize,
    record: TeamRecord,
}

#[derive(Debug, Clone, Deserialize)]
struct TeamRecord { wins: u32, losses: u32 }

#[derive(Debug, Clone, Deserialize)]
struct Announcement {
    #[allow(dead_code)] id: String,
    title: String,
    content: String,
    pinned: bool,
    created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct HomeTournament {
    id: String,
    name: String,
    format: String,
    status: String,
    is_external: bool,
    starts_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Event {
    #[allow(dead_code)] id: String,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
}

#[derive(Deserialize)]
struct CursorPage<T> { data: Vec<T> }

// --- CSS ---

const HOME_CSS: &str = r#"
    .hero { position: relative; min-height: 100vh; display: flex; align-items: center; justify-content: center; overflow: hidden; }
    .hero-bg { position: absolute; inset: 0; background: radial-gradient(ellipse at 50% 40%, rgba(124,58,237,0.10), transparent 70%); }
    .hero-content { position: relative; z-index: 2; text-align: center; max-width: 700px; padding: 2rem; }
    .hero-badge { display: inline-flex; align-items: center; gap: 0.5rem; padding: 0.3rem 0.8rem; border: 1px solid var(--border); border-radius: 999px; font-size: 0.7rem; color: var(--text-muted); margin-bottom: 1.5rem; text-transform: uppercase; letter-spacing: 0.06em; }
    .badge-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--accent); }
    .hero-title { font-family: 'Bebas Neue', sans-serif; font-size: clamp(3rem, 8vw, 6rem); line-height: 0.95; color: var(--text-bright); letter-spacing: 4px; margin: 0; }
    .hero-title .purple { color: var(--accent); }
    .hero-sub { color: var(--text-secondary); font-size: 1rem; line-height: 1.7; margin: 1.5rem auto; max-width: 550px; }
    .hero-actions { display: flex; gap: 1rem; justify-content: center; flex-wrap: wrap; margin: 2rem 0; }
    .hero-stats { display: flex; gap: 3rem; justify-content: center; margin-top: 2rem; }
    .hero-stat-val { font-family: 'Bebas Neue', sans-serif; font-size: 2rem; color: var(--text-bright); }
    .hero-stat-label { font-size: 0.7rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.06em; }
    .hero-emblem { position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%); width: 400px; height: 400px; color: rgba(124,58,237,0.08); pointer-events: none; }
    .divider { height: 1px; background: var(--border); margin: 0 2rem; }
    section { padding: 5rem 2rem; max-width: 1000px; margin: 0 auto; }
    .sec-label { font-family: 'DM Mono', monospace; font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.08em; margin-bottom: 0.5rem; }
    .sec-label-purple { color: var(--accent); }
    .sec-label-red { color: #ef4444; }
    .sec-label-blue { color: #3b82f6; }
    .sec-title { font-family: 'Bebas Neue', sans-serif; font-size: 2.5rem; color: var(--text-bright); letter-spacing: 3px; margin: 0 0 0.5rem; }
    .sec-desc { color: var(--text-secondary); max-width: 600px; line-height: 1.7; }
    .pillars { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 1.25rem; margin-top: 2rem; }
    .pillar { background: var(--bg-card); border: 1px solid var(--border); border-radius: 10px; padding: 1.5rem; transition: border-color 0.2s; }
    .pillar:hover { border-color: var(--accent-soft); }
    .pillar h3 { font-family: 'Rajdhani', sans-serif; font-size: 1.1rem; font-weight: 700; margin: 0.75rem 0 0.5rem; color: var(--text-bright); }
    .pillar p { color: var(--text-secondary); font-size: 0.85rem; line-height: 1.6; }
    .pillar-icon { width: 36px; height: 36px; color: var(--accent); }
    .pillar-icon svg { width: 100%; height: 100%; }
    .teams-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 1rem; margin-top: 2rem; }
    .team-card { background: var(--bg-card); border: 1px solid var(--border); border-radius: 10px; padding: 1.25rem; transition: border-color 0.2s; }
    .team-card:hover { border-color: var(--accent-soft); }
    .team-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 0.5rem; }
    .team-name { font-family: 'Rajdhani', sans-serif; font-weight: 700; font-size: 1.1rem; color: var(--text-bright); }
    .team-game { font-size: 0.65rem; padding: 0.1rem 0.5rem; border-radius: 999px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
    .game-ow { background: #f9731633; color: #f97316; }
    .game-dest { background: #3b82f633; color: #60a5fa; }
    .game-other { background: #6b728033; color: #9ca3af; }
    .team-lore { color: var(--text-muted); font-style: italic; font-size: 0.8rem; margin-bottom: 0.75rem; line-height: 1.5; }
    .team-meta { display: flex; gap: 1.5rem; }
    .team-meta-val { font-family: 'DM Mono', monospace; font-weight: 700; color: var(--text-bright); }
    .team-meta-label { font-size: 0.65rem; color: var(--text-muted); text-transform: uppercase; }
    .team-division { margin-top: 0.75rem; font-size: 0.75rem; color: var(--text-muted); }
    .news-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 1rem; margin-top: 2rem; }
    .news-card { background: var(--bg-card); border: 1px solid var(--border); border-radius: 10px; padding: 1.25rem; }
    .news-meta { display: flex; align-items: center; gap: 0.5rem; font-size: 0.7rem; color: var(--text-muted); margin-bottom: 0.5rem; }
    .news-pin { background: #7c3aed33; color: #a78bfa; padding: 0.1rem 0.4rem; border-radius: 4px; font-size: 0.6rem; font-weight: 600; text-transform: uppercase; }
    .news-title { font-family: 'Rajdhani', sans-serif; font-size: 1.1rem; font-weight: 700; color: var(--text-bright); margin: 0 0 0.4rem; }
    .news-body { color: var(--text-secondary); font-size: 0.85rem; line-height: 1.6; }
    .news-more { text-align: center; margin-top: 1.5rem; }
    .comms-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 1.25rem; margin-top: 2rem; }
    .comm-card { background: var(--bg-card); border: 1px solid var(--border); border-radius: 10px; padding: 1.5rem; }
    .comm-card-header { display: flex; align-items: center; gap: 0.75rem; margin-bottom: 0.75rem; }
    .comm-card-header h3 { font-family: 'Rajdhani', sans-serif; font-weight: 700; font-size: 1.1rem; color: var(--text-bright); margin: 0; }
    .comm-icon-svg { width: 28px; height: 28px; color: var(--accent); }
    .comm-icon-svg svg { width: 100%; height: 100%; }
    .comm-card p { color: var(--text-secondary); font-size: 0.85rem; line-height: 1.6; }
    .comm-tags { margin-top: 0.75rem; }
    .comm-tag { font-size: 0.65rem; padding: 0.15rem 0.5rem; border-radius: 999px; font-weight: 600; text-transform: uppercase; }
    .comm-public { background: #10b98133; color: #34d399; }
    .comm-members { background: #3b82f633; color: #60a5fa; }
    .why-matrix { margin-top: 2rem; background: var(--bg-card); border: 1px solid var(--border); border-radius: 10px; padding: 1.5rem; }
    .why-matrix-header { font-family: 'Rajdhani', sans-serif; font-weight: 700; font-size: 1rem; color: var(--text-bright); margin-bottom: 0.75rem; }
    .why-matrix-body p { color: var(--text-secondary); font-size: 0.85rem; line-height: 1.6; margin-bottom: 0.75rem; }
    .why-tradeoffs { display: grid; grid-template-columns: 1fr 1fr; gap: 1.5rem; margin-top: 1rem; }
    .why-tradeoff h4 { font-family: 'Rajdhani', sans-serif; font-weight: 700; color: var(--text-bright); margin: 0 0 0.5rem; font-size: 0.9rem; }
    .why-tradeoff-item { font-size: 0.8rem; color: var(--text-secondary); margin-bottom: 0.35rem; display: flex; gap: 0.5rem; }
    .why-tradeoff-item .marker { color: var(--accent); font-weight: 700; }
    .sched-strip { display: grid; grid-template-columns: repeat(7, 1fr); gap: 0.5rem; margin-top: 2rem; }
    .sched-active { background: var(--bg-card); border: 1px solid var(--border); border-radius: 8px; padding: 1rem; text-align: center; }
    .sched-off { background: var(--bg-card); border: 1px solid var(--border); border-radius: 8px; padding: 1rem; text-align: center; opacity: 0.3; color: var(--text-muted); font-size: 0.85rem; }
    .day-label { font-family: 'DM Mono', monospace; font-size: 0.7rem; color: var(--text-muted); text-transform: uppercase; margin-bottom: 0.25rem; }
    .day-event { font-weight: 600; font-size: 0.85rem; color: var(--text-bright); }
    .day-time { font-size: 0.7rem; color: var(--text-muted); margin-top: 0.25rem; }
    .sched-calendar-link { text-align: center; margin-top: 1.5rem; }
    .recruit-wrap { display: grid; grid-template-columns: 1fr 1fr; gap: 3rem; }
    .recruit-wrap h2 { font-family: 'Bebas Neue', sans-serif; font-size: 2.5rem; color: var(--text-bright); letter-spacing: 3px; margin: 0.5rem 0; }
    .recruit-wrap h3 { font-family: 'Rajdhani', sans-serif; font-weight: 700; color: var(--text-bright); margin: 0 0 1rem; }
    .recruit-left p { color: var(--text-secondary); font-size: 0.9rem; line-height: 1.7; }
    .req { display: flex; gap: 0.5rem; color: var(--text-secondary); font-size: 0.85rem; margin-bottom: 0.5rem; }
    .req-marker { color: var(--accent); }
    .recruit-seeking { margin-top: 1.5rem; }
    .recruit-seeking-label { font-size: 0.7rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.06em; margin-bottom: 0.5rem; }
    .recruit-seeking-tags { display: flex; gap: 0.5rem; flex-wrap: wrap; }
    .recruit-tag { font-size: 0.7rem; padding: 0.2rem 0.6rem; border-radius: 999px; font-weight: 600; }
    .recruit-tag-ow { background: #f9731633; color: #f97316; }
    .recruit-tag-dest { background: #3b82f633; color: #60a5fa; }
    .never-ask { margin-top: 1.5rem; background: var(--bg-surface); border-radius: 8px; padding: 1rem; }
    .never-ask-header { font-family: 'Rajdhani', sans-serif; font-weight: 700; font-size: 0.85rem; color: var(--text-bright); margin-bottom: 0.5rem; }
    .never-ask-body { color: var(--text-muted); font-size: 0.8rem; line-height: 1.6; }
    .btn { display: inline-flex; align-items: center; gap: 0.5rem; padding: 0.6rem 1.4rem; border-radius: 6px; font-size: 0.9rem; font-weight: 600; text-transform: uppercase; letter-spacing: 0.03em; text-decoration: none; transition: all 0.2s; border: none; cursor: pointer; }
    .btn-primary { background: var(--accent); color: white; }
    .btn-primary:hover { filter: brightness(1.15); box-shadow: 0 0 20px var(--accent-glow); }
    .btn-outline { background: transparent; border: 1px solid var(--border); color: var(--text-secondary); }
    .btn-outline:hover { border-color: var(--accent-soft); color: var(--text-bright); }
    .btn-secondary { background: var(--bg-card); border: 1px solid var(--border); color: var(--text-secondary); }
    .btn-secondary:hover { border-color: var(--accent-soft); color: var(--text-bright); }
    @media (max-width: 768px) {
        .sched-strip { grid-template-columns: 1fr; }
        .recruit-wrap { grid-template-columns: 1fr; gap: 2rem; }
        .why-tradeoffs { grid-template-columns: 1fr; }
    }
"#;

// --- Component ---

#[component]
pub fn Home() -> Element {
    let overview = use_resource(|| async {
        ApiClient::web().fetch::<Overview>("/api/public/overview").await.ok()
    });
    let announcements = use_resource(|| async {
        ApiClient::web().fetch::<CursorPage<Announcement>>("/api/announcements").await.ok().map(|r| r.data)
    });
    let tournaments_res = use_resource(|| async {
        ApiClient::web().fetch::<CursorPage<HomeTournament>>("/api/tournaments").await.ok().map(|r| r.data)
    });
    let events = use_resource(|| async {
        ApiClient::web().fetch::<CursorPage<Event>>("/api/events").await.ok().map(|r| r.data)
    });

    let (team_count, member_count, game_count) = {
        let o = overview.read();
        let o = o.as_ref().and_then(|o| o.as_ref());
        (o.map(|o| o.teams.len()).unwrap_or(0), o.map(|o| o.member_count).unwrap_or(0), o.map(|o| o.games.len()).unwrap_or(0))
    };

    rsx! {
        style { {HOME_CSS} }
        style { {BRACKET_STYLES} }

        // Hero
        section { class: "hero",
            div { class: "hero-bg" }
            svg { class: "hero-emblem", view_box: "0 0 400 400", fill: "none", stroke: "currentColor", stroke_width: "1.5",
                path { d: "M200 20 L360 120 L340 320 L200 380 L60 320 L40 120 Z" }
                path { d: "M200 60 L320 140 L305 290 L200 340 L95 290 L80 140 Z" }
                path { d: "M200 100 L280 160 L270 260 L200 300 L130 260 L120 160 Z" }
                line { x1: "200", y1: "20", x2: "200", y2: "380" }
                line { x1: "40", y1: "120", x2: "340", y2: "320" }
                line { x1: "360", y1: "120", x2: "60", y2: "320" }
                path { d: "M200 170 L230 200 L200 230 L170 200 Z" }
            }
            div { class: "hero-content",
                div { class: "hero-badge",
                    span { class: "badge-dot" }
                    "Multi-Game Crew \u{2014} EMEA \u{2014} Est. 2026"
                }
                h1 { class: "hero-title", "The Scuffed" br {} span { class: "purple", "Crew" } }
                p { class: "hero-sub", "A multi-game crew built on old-school clan principles. Small teams, real structure, scheduled play nights. No ghost members. No dead servers." }
                div { class: "hero-actions",
                    Link { to: Route::Apply {}, class: "btn btn-primary", "Apply to Join" }
                    a { href: "#teams", class: "btn btn-outline", "Our Teams" }
                }
                div { class: "hero-stats",
                    div { class: "hero-stat", div { class: "hero-stat-val", "{team_count}" } div { class: "hero-stat-label", "Active Teams" } }
                    div { class: "hero-stat", div { class: "hero-stat-val", "{member_count}" } div { class: "hero-stat-label", "Members" } }
                    div { class: "hero-stat", div { class: "hero-stat-val", "{game_count}" } div { class: "hero-stat-label", "Games" } }
                }
            }
        }

        div { class: "divider" }

        // About
        section { id: "about",
            SectionHeader { label: "// The Ethos", title: "Not a server. A clan.", color: "purple", description: "A structured gaming org with game-specific squads and scheduled play nights. No drama. Life comes first \u{2014} the games come second, but we still show up." }
            div { class: "pillars",
                div { class: "pillar",
                    div { class: "pillar-icon", svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", circle { cx: "12", cy: "6", r: "2" } circle { cx: "6", cy: "18", r: "2" } circle { cx: "18", cy: "18", r: "2" } } }
                    h3 { "Squad structure" }
                    p { "Small teams of 5+5 named after in-game lore. Your squad is your crew \u{2014} the org is the scaffold that holds it together." }
                }
                div { class: "pillar",
                    div { class: "pillar-icon", svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", rect { x: "2", y: "4", width: "8", height: "8", rx: "1" } rect { x: "14", y: "12", width: "8", height: "8", rx: "1" } } }
                    h3 { "Multi-game" }
                    p { "Overwatch, Destiny 2, and whatever comes next. The crew spans games \u{2014} your squad plays one, the org plays them all." }
                }
                div { class: "pillar",
                    div { class: "pillar-icon", svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", rect { x: "9", y: "2", width: "6", height: "10", rx: "3" } path { d: "M5 10a7 7 0 0 0 14 0" } } }
                    h3 { "Dedicated voice" }
                    p { "Play nights run on TeamSpeak \u{2014} self-hosted, low latency, built for competitive gaming. Matrix handles everything else." }
                }
                div { class: "pillar",
                    div { class: "pillar-icon", svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", path { d: "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" } } }
                    h3 { "One rule" }
                    p { "No politics, no drama, no soapboxes. Show up, communicate, have fun. Ghost for weeks and your slot opens up." }
                }
            }
        }

        div { class: "divider" }

        // Teams
        section { id: "teams",
            SectionHeader { label: "// Active Squads", title: "The Teams", color: "red", description: "Each team carries a name from the lore of the game they play. Your team is your identity within the org." }
            div { class: "teams-grid",
                {match overview.read().as_ref().and_then(|o| o.as_ref()) {
                    Some(data) => {
                        let game_map: HashMap<String, String> = data.games.iter().map(|g| (g.id.clone(), g.name.clone())).collect();
                        rsx! { for team in data.teams.iter() { { render_team_card(team, &game_map) } } }
                    },
                    None => rsx! { p { style: "color: var(--text-muted); text-align: center;", "Loading teams..." } },
                }}
            }
        }

        div { class: "divider" }

        // Announcements
        section { id: "news",
            SectionHeader { label: "// Latest News", title: "Announcements", color: "purple", description: "What's happening in the crew." }
            div { class: "news-grid",
                { let list = announcements.read().as_ref().and_then(|a| a.as_ref()).cloned().unwrap_or_default();
                  rsx! { for a in list.iter().take(3) { { render_news_card(a) } } }
                }
            }
            div { class: "news-more", Link { to: Route::News {}, class: "btn btn-secondary", "View All News" } }
        }

        div { class: "divider" }

        // Tournaments
        section { id: "tournaments",
            SectionHeader { label: "// Compete", title: "Tournaments", color: "purple", description: "Active and upcoming tournaments." }
            {
                let list = tournaments_res.read().as_ref().and_then(|t| t.as_ref()).cloned().unwrap_or_default();
                let visible: Vec<&HomeTournament> = list.iter().filter(|t| t.status == "registration" || t.status == "in_progress").take(4).collect();
                if visible.is_empty() {
                    rsx! { p { style: "color: var(--text-muted); text-align: center;", "No active tournaments right now." } }
                } else {
                    rsx! {
                        div { class: "tournament-home-grid",
                            for t in visible.iter() { { render_tournament_card(t) } }
                        }
                    }
                }
            }
            div { style: "text-align: center; margin-top: 1.5rem;", Link { to: Route::Tournaments {}, class: "btn btn-secondary", "View All Tournaments" } }
        }

        div { class: "divider" }

        // Comms
        section { id: "comms",
            SectionHeader { label: "// Communication", title: "How we talk", color: "blue", description: "Matrix for text, TeamSpeak for voice. Self-hosted, no middleman." }
            div { class: "comms-grid",
                div { class: "comm-card",
                    div { class: "comm-card-header",
                        div { class: "comm-icon-svg", svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", path { d: "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" } } }
                        h3 { "Matrix (Commet)" }
                    }
                    p { "All text lives here \u{2014} announcements, scheduling, casual chat, recruitment. Self-hosted on our server." }
                    div { class: "comm-tags", span { class: "comm-tag comm-public", "Open to all" } }
                }
                div { class: "comm-card",
                    div { class: "comm-card-header",
                        div { class: "comm-icon-svg", svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", path { d: "M3 18v-6a9 9 0 0 1 18 0v6" } path { d: "M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3z" } path { d: "M3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z" } } }
                        h3 { "TeamSpeak" }
                    }
                    p { "Voice comms for play nights and scrims. Self-hosted, low latency, no distractions mid-match." }
                    div { class: "comm-tags", span { class: "comm-tag comm-members", "Rostered members" } }
                }
            }
            div { class: "why-matrix",
                div { class: "why-matrix-header", "Why Matrix instead of Discord?" }
                div { class: "why-matrix-body",
                    p { "We use Matrix because we\u{2019}d rather own our infrastructure than rent it. Our server, our data, our rules." }
                    div { class: "why-tradeoffs",
                        div { class: "why-tradeoff",
                            h4 { "What you gain" }
                            div { class: "why-tradeoff-item", span { class: "marker", "+" } " Self-hosted \u{2014} we own the server and data" }
                            div { class: "why-tradeoff-item", span { class: "marker", "+" } " E2E encrypted messages, files, and calls" }
                            div { class: "why-tradeoff-item", span { class: "marker", "+" } " Built-in calendar rooms with .ics sync" }
                            div { class: "why-tradeoff-item", span { class: "marker", "+" } " No third-party account required" }
                            div { class: "why-tradeoff-item", span { class: "marker", "+" } " Community lives on our hardware" }
                        }
                        div { class: "why-tradeoff",
                            h4 { "What you give up" }
                            div { class: "why-tradeoff-item", span { class: "marker", "\u{2212}" } " Smaller bot ecosystem" }
                            div { class: "why-tradeoff-item", span { class: "marker", "\u{2212}" } " Fewer people already have accounts" }
                            div { class: "why-tradeoff-item", span { class: "marker", "\u{2212}" } " Screen sharing is less polished" }
                            div { class: "why-tradeoff-item", span { class: "marker", "\u{2212}" } " Search isn\u{2019}t as refined yet" }
                            div { class: "why-tradeoff-item", span { class: "marker", "\u{2212}" } " Newer platform \u{2014} still growing" }
                        }
                    }
                }
            }
        }

        div { class: "divider" }

        // Schedule
        section { id: "schedule",
            SectionHeader { label: "// Weekly Rhythm", title: "Play Nights", color: "purple", description: "No obligation to hit every session. Show up when life allows." }
            div { class: "sched-strip",
                {
                    let event_list = events.read().as_ref().and_then(|e| e.as_ref()).cloned().unwrap_or_default();
                    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
                    rsx! {
                        for (day, day_name) in days.iter().enumerate() {
                            {
                                let day_events: Vec<&Event> = event_list.iter().filter(|e| e.day_of_week == day as u8).collect();
                                if let Some(event) = day_events.first() {
                                    let time_display = format!("{} {}", event.time, event.timezone);
                                    rsx! { div { class: "sched-active", div { class: "day-label", "{day_name}" } div { class: "day-event", "{event.title}" } div { class: "day-time", "{time_display}" } } }
                                } else {
                                    rsx! { div { class: "sched-off", "{day_name}" } }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "sched-calendar-link", a { href: "/api/calendar/all.ics", class: "btn btn-outline", "Subscribe to Calendar" } }
        }

        div { class: "divider" }

        // Recruit
        section { id: "join",
            div { class: "recruit-wrap",
                div { class: "recruit-left",
                    div { class: "sec-label sec-label-purple", "// Recruitment Open" }
                    h2 { "Want in?" }
                    p { "We keep rosters intentional. Join our Matrix server, hop in a few play nights, and we\u{2019}ll match you with a team that fits your schedule and skill level." }
                    div { style: "display:flex;gap:1rem;flex-wrap:wrap;margin-top:1rem;",
                        a { href: "#", class: "btn btn-primary", "Join Matrix" }
                    }
                    div { class: "recruit-seeking",
                        div { class: "recruit-seeking-label", "Currently looking for" }
                        div { class: "recruit-seeking-tags",
                            span { class: "recruit-tag recruit-tag-ow", "OW2 DPS" }
                            span { class: "recruit-tag recruit-tag-ow", "OW2 Support" }
                            span { class: "recruit-tag recruit-tag-dest", "D2 PvP" }
                        }
                    }
                }
                div { class: "recruit-right",
                    h3 { "What we expect" }
                    div { class: "req", span { class: "req-marker", "\u{203a}" } span { "16+ \u{2014} old enough to communicate and commit" } }
                    div { class: "req", span { class: "req-marker", "\u{203a}" } span { "PC only \u{2014} our teams play on PC" } }
                    div { class: "req", span { class: "req-marker", "\u{203a}" } span { "Communicate \u{2014} let your squad know if you can\u{2019}t make it" } }
                    div { class: "req", span { class: "req-marker", "\u{203a}" } span { "No toxicity \u{2014} competitive is fine, being a jerk isn\u{2019}t" } }
                    div { class: "req", span { class: "req-marker", "\u{203a}" } span { "Mic required for play nights" } }
                    div { class: "req", span { class: "req-marker", "\u{203a}" } span { "Willing to install TeamSpeak when you make a roster" } }
                    div { class: "never-ask",
                        div { class: "never-ask-header", "What we\u{2019}ll never ask for" }
                        div { class: "never-ask-body", "Your real name \u{00b7} Your email \u{00b7} Your phone number \u{00b7} Your social media \u{00b7} Your location \u{00b7} Your government ID \u{00b7} Access to your contacts \u{00b7} Permission to scan your processes" }
                    }
                }
            }
        }
    }
}

// --- Render helpers ---

fn render_team_card(team: &OverviewTeam, game_map: &HashMap<String, String>) -> Element {
    let game_name = game_map.get(&team.game_id).cloned().unwrap_or_else(|| team.game_id.clone());
    let badge_class = if game_name.to_lowercase().contains("overwatch") { "team-game game-ow" }
        else if game_name.to_lowercase().contains("destiny") { "team-game game-dest" }
        else { "team-game game-other" };
    let wl = if team.record.wins == 0 && team.record.losses == 0 { "\u{2014}".to_string() }
        else { format!("{}-{}", team.record.wins, team.record.losses) };
    let division = team.division.clone().unwrap_or_else(|| "Scrims & Internal".into());
    let lore = team.lore_quote.clone().unwrap_or_default();

    rsx! {
        div { class: "team-card",
            div { class: "team-header",
                div { class: "team-name", "{team.name}" }
                span { class: "{badge_class}", "{game_name}" }
            }
            if !lore.is_empty() {
                div { class: "team-lore", "\u{201c}{lore}\u{201d}" }
            }
            div { class: "team-meta",
                div { class: "team-meta-item", span { class: "team-meta-val", "{team.roster_count}" } span { class: "team-meta-label", " Roster" } }
                div { class: "team-meta-item", span { class: "team-meta-val", "{wl}" } span { class: "team-meta-label", " W-L" } }
            }
            div { class: "team-division", "{division}" }
        }
    }
}

fn render_news_card(a: &Announcement) -> Element {
    let date: String = a.created_at.chars().take(10).collect();
    rsx! {
        article { class: "news-card",
            div { class: "news-meta",
                time { "{date}" }
                if a.pinned { span { class: "news-pin", "Pinned" } }
            }
            h3 { class: "news-title", "{a.title}" }
            p { class: "news-body", "{a.content}" }
        }
    }
}

fn render_tournament_card(t: &HomeTournament) -> Element {
    let fmt = match t.format.as_str() { "single_elim" => "Single Elim", "double_elim" => "Double Elim", "round_robin" => "Round Robin", "swiss" => "Swiss", _ => &t.format };
    let status = match t.status.as_str() { "registration" => "Registration Open", "in_progress" => "Live", "completed" => "Completed", _ => &t.status };
    let status_class = format!("tournament-card-status {}", t.status);
    let date: String = t.starts_at.as_ref().map(|d| d.chars().take(10).collect()).unwrap_or_default();

    rsx! {
        Link { to: Route::Tournament { id: t.id.clone() }, class: "tournament-card",
            div { class: "tournament-card-name", "{t.name}" }
            div { class: "tournament-card-meta",
                span { "{fmt}" }
                if !date.is_empty() { span { "{date}" } }
                if t.is_external { span { "External" } }
            }
            span { class: "{status_class}", "{status}" }
        }
    }
}
