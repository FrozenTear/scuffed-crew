use dioxus::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

use crate::hooks::CursorPage;
use crate::routes::Route;
use scuffed_api_client::ApiClient;
use scuffed_types::{PublicLayout, SiteSettings};

// --- Data types ---

#[derive(Debug, Clone, Deserialize)]
struct Overview {
    teams: Vec<OverviewTeam>,
    games: Vec<OverviewGame>,
    member_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct OverviewGame {
    id: String,
    name: String,
}

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
struct TeamRecord {
    wins: u32,
    losses: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct Announcement {
    #[allow(dead_code)]
    id: String,
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
    #[allow(dead_code)]
    id: String,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
}

const HOME_CSS: &str = r#"
    .home-wrap {
        position: relative;
        min-height: 100%;
    }
    .home {
        max-width: 1040px;
        margin: 0 auto;
        padding: 0 1.25rem 5rem;
        position: relative;
        z-index: 1;
    }

    /* ——— HERO: big, loud, clipped ——— */
    .home-hero {
        position: relative;
        margin: 0 -1.25rem;
        padding: 3.25rem 1.25rem 2.5rem;
        overflow: hidden;
        border-bottom: 1px solid var(--border);
        background:
            linear-gradient(105deg, rgba(124,58,237,0.12) 0%, transparent 42%),
            linear-gradient(180deg, var(--surface) 0%, var(--bg) 100%);
    }
    .home-hero::before {
        content: '';
        position: absolute;
        right: -8%;
        top: -20%;
        width: min(52vw, 420px);
        height: 140%;
        background:
            repeating-linear-gradient(
                -18deg,
                transparent,
                transparent 11px,
                rgba(124,58,237,0.06) 11px,
                rgba(124,58,237,0.06) 12px
            );
        transform: skewX(-12deg);
        pointer-events: none;
    }
    .home-hero::after {
        content: 'SC';
        position: absolute;
        right: 4%;
        bottom: -0.15em;
        font-family: var(--font-head);
        font-size: clamp(6rem, 22vw, 12rem);
        line-height: 0.8;
        letter-spacing: 0.02em;
        color: transparent;
        -webkit-text-stroke: 1px rgba(240,238,232,0.07);
        pointer-events: none;
        user-select: none;
    }
    .home-hero-inner { position: relative; z-index: 2; max-width: 40rem; }
    .home-badge {
        display: inline-flex;
        align-items: center;
        gap: 0.5rem;
        font-family: var(--font-mono);
        font-size: 0.68rem;
        letter-spacing: 0.16em;
        text-transform: uppercase;
        color: var(--accent);
        margin-bottom: 1rem;
        padding: 0.3rem 0.65rem 0.3rem 0.45rem;
        border: 1px solid rgba(124,58,237,0.45);
        background: rgba(124,58,237,0.1);
        clip-path: polygon(0 0, 100% 0, 100% calc(100% - 6px), calc(100% - 6px) 100%, 0 100%);
    }
    .home-badge::before {
        content: '';
        width: 6px;
        height: 6px;
        background: var(--accent);
        box-shadow: 0 0 8px var(--accent);
        animation: pulse-dot 1.6s ease-in-out infinite;
    }
    @keyframes pulse-dot {
        0%, 100% { opacity: 1; }
        50% { opacity: 0.35; }
    }
    .home-title {
        font-family: var(--font-head);
        font-size: clamp(3.4rem, 12vw, 6.5rem);
        line-height: 0.88;
        letter-spacing: 0.02em;
        color: var(--text);
        margin: 0;
        text-transform: uppercase;
        text-shadow: 3px 0 0 rgba(124,58,237,0.35);
    }
    .home-title em {
        font-style: normal;
        display: block;
        color: var(--accent);
        text-shadow:
            0 0 40px rgba(124,58,237,0.35),
            -2px 0 0 rgba(74,158,255,0.25);
    }
    .home-sub {
        margin: 1.25rem 0 0;
        max-width: 28rem;
        color: var(--text);
        font-size: 1.05rem;
        line-height: 1.55;
        font-weight: 400;
        border-left: 3px solid var(--accent);
        padding-left: 0.9rem;
    }
    .home-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 0.65rem;
        margin-top: 1.75rem;
    }
    .home-metrics {
        display: flex;
        flex-wrap: wrap;
        margin-top: 2rem;
        max-width: 28rem;
        border: 1px solid var(--border);
        background: rgba(8,8,12,0.55);
    }
    .home-metric {
        padding: 0.85rem 1.15rem;
        border-right: 1px solid var(--border);
        min-width: 5.5rem;
        flex: 1 1 auto;
    }
    .home-metric:last-child { border-right: none; }
    .home-metric strong {
        display: block;
        font-family: var(--font-head);
        font-size: 2rem;
        letter-spacing: 0.04em;
        color: var(--text);
        line-height: 1;
    }
    .home-metric span {
        font-family: var(--font-mono);
        font-size: 0.6rem;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        color: var(--text-3);
    }
    @media (max-width: 520px) {
        .home-metrics { grid-template-columns: 1fr; }
        .home-metric { border-right: none; border-bottom: 1px solid var(--border); }
        .home-metric:last-child { border-bottom: none; }
    }

    /* ——— Sections (quieter than hero; no competing underlines) ——— */
    .home-block {
        padding: 2.25rem 0;
        border-bottom: 1px solid var(--border);
    }
    .home-block:last-of-type { border-bottom: none; }
    .home-kicker {
        font-family: var(--font-mono);
        font-size: 0.62rem;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        color: var(--text-3);
        margin-bottom: 0.35rem;
    }
    .home-kicker.compete {
        color: #f0b232;
    }
    .home-heading {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: clamp(1.15rem, 2.4vw, 1.45rem);
        color: var(--text);
        letter-spacing: 0.02em;
        margin: 0 0 0.5rem;
        text-transform: none;
        line-height: 1.2;
    }
    /* Underlines only on hero — sections stay quiet */
    .home-heading::after {
        content: none;
    }
    .home-body {
        color: var(--text-2);
        font-size: 0.95rem;
        line-height: 1.65;
        max-width: 36rem;
        margin: 0.5rem 0 0;
    }

    /* Numbered rules — punchy */
    .rules {
        list-style: none;
        margin: 1.5rem 0 0;
        padding: 0;
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 0.75rem;
    }
    @media (max-width: 700px) {
        .rules { grid-template-columns: 1fr; }
    }
    .rules li {
        display: grid;
        grid-template-columns: auto 1fr;
        gap: 0.85rem;
        align-items: start;
        padding: 1rem 1rem 1rem 0.9rem;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-left: 3px solid var(--accent);
        color: var(--text);
        font-size: 0.92rem;
        line-height: 1.45;
        transition: border-color 0.15s, transform 0.15s;
    }
    .rules li:hover {
        border-color: rgba(124,58,237,0.45);
        transform: translateX(2px);
    }
    .rules li .rn {
        font-family: var(--font-head);
        font-size: 1.5rem;
        letter-spacing: 0.04em;
        color: var(--accent);
        line-height: 1;
        min-width: 1.5rem;
    }

    /* Live board — one column when only one side has data */
    .live-grid {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 1rem;
        margin-top: 0.85rem;
    }
    .live-grid.single {
        grid-template-columns: 1fr;
        max-width: 36rem;
    }
    @media (max-width: 720px) {
        .live-grid { grid-template-columns: 1fr; }
    }
    .live-panel {
        background: var(--surface-2);
        border: 1px solid var(--border);
        padding: 1.1rem 1.15rem 1.25rem;
        position: relative;
        overflow: hidden;
    }
    .live-panel::before {
        content: '';
        position: absolute;
        top: 0; left: 0; right: 0;
        height: 2px;
        background: linear-gradient(90deg, var(--border), transparent 80%);
    }
    /* Compete / tournaments: gold accent (not brand purple) */
    .live-panel.compete::before {
        background: linear-gradient(90deg, #f0b232, transparent 75%);
    }
    .live-panel .home-heading {
        font-size: 1.15rem;
    }
    .live-list {
        list-style: none;
        margin: 0.85rem 0 0;
        padding: 0;
    }
    .live-list li {
        display: flex;
        justify-content: space-between;
        gap: 1rem;
        align-items: baseline;
        padding: 0.65rem 0;
        border-bottom: 1px solid var(--border);
        font-size: 0.9rem;
    }
    .live-list li:last-child { border-bottom: none; }
    .live-list a {
        color: var(--text);
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1rem;
    }
    .live-list a:hover { color: var(--accent); }
    .live-meta {
        font-family: var(--font-mono);
        font-size: 0.68rem;
        color: var(--text-3);
        white-space: nowrap;
    }
    .tag {
        font-family: var(--font-mono);
        font-size: 0.58rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        color: #f0b232;
        border: 1px solid rgba(240, 178, 50, 0.4);
        padding: 0.15rem 0.4rem;
        background: rgba(240, 178, 50, 0.1);
    }
    .tag.live {
        color: #ff6b6b;
        border-color: rgba(214, 48, 49, 0.5);
        background: rgba(214, 48, 49, 0.12);
    }
    .tag.open {
        color: #f0b232;
        border-color: rgba(240, 178, 50, 0.45);
        background: rgba(240, 178, 50, 0.1);
    }
    .home-link.compete {
        color: #f0b232;
    }
    .home-link.compete:hover {
        color: #fbbf24;
        border-bottom-color: #f0b232;
    }

    /* Team rows — shared fixed tracks so every column lines up across rows */
    .team-rows {
        margin-top: 1.1rem;
        border-top: 1px solid var(--border);
        /* name | game | roster | division | record */
        --tm-cols: minmax(0, 1.4fr) minmax(7.5rem, 0.9fr) 5.5rem 6.5rem 3.25rem;
    }
    .team-head,
    .team-row {
        display: grid;
        grid-template-columns: var(--tm-cols);
        column-gap: 1rem;
        align-items: baseline;
        padding: 0.7rem 0.5rem 0.7rem 0.65rem;
    }
    .team-head {
        padding-top: 0.55rem;
        padding-bottom: 0.45rem;
        border-bottom: 1px solid var(--border);
        font-family: var(--font-mono);
        font-size: 0.58rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--text-3);
    }
    .team-head span:nth-child(3),
    .team-head span:nth-child(4),
    .team-head span:nth-child(5) {
        text-align: right;
    }
    .team-row {
        border-bottom: 1px solid var(--border);
        font-size: 0.88rem;
        position: relative;
        transition: background 0.15s;
    }
    .team-row::before {
        content: '';
        position: absolute;
        left: 0; top: 0; bottom: 0;
        width: 0;
        background: var(--accent);
        transition: width 0.15s;
    }
    .team-row:hover {
        background: rgba(124,58,237,0.06);
    }
    .team-row:hover::before { width: 3px; }
    .team-row .tm-name {
        font-family: var(--font-head);
        font-weight: 700;
        color: var(--text);
        font-size: 1.05rem;
        letter-spacing: 0.02em;
        min-width: 0;
    }
    .team-row .tm-game {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--text-3);
        min-width: 0;
    }
    .team-row .tm-roster {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: var(--text-3);
        text-align: right;
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
    }
    .team-row .tm-div {
        color: var(--text-2);
        font-size: 0.8rem;
        text-align: right;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .team-row .tm-wl {
        font-family: var(--font-mono);
        font-size: 0.8rem;
        letter-spacing: 0.04em;
        color: var(--text-2);
        text-align: right;
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
    }
    .team-lore {
        grid-column: 1 / -1;
        color: var(--text-3);
        font-size: 0.8rem;
        font-style: italic;
        margin-top: 0.15rem;
    }
    @media (max-width: 720px) {
        .team-rows {
            --tm-cols: minmax(0, 1fr) auto;
        }
        .team-head span:nth-child(n+3),
        .team-row .tm-roster,
        .team-row .tm-div { display: none; }
        .team-head span:nth-child(2) { display: none; }
        .team-row .tm-game { display: none; }
        .team-row .tm-wl { justify-self: end; }
    }

    /* News */
    .news-rows { margin-top: 1rem; }
    .news-row {
        padding: 1rem 0 1.1rem;
        border-bottom: 1px solid var(--border);
        transition: padding-left 0.15s;
    }
    .news-row:hover { padding-left: 0.4rem; }
    .news-row:last-child { border-bottom: none; }
    .news-row time {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        color: var(--text-3);
        letter-spacing: 0.1em;
    }
    .news-row h3 {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1.2rem;
        color: var(--text);
        margin: 0.25rem 0 0.4rem;
    }
    .news-row p {
        color: var(--text-2);
        font-size: 0.9rem;
        line-height: 1.55;
        margin: 0;
        max-width: 40rem;
    }
    .pin {
        font-family: var(--font-mono);
        font-size: 0.58rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        color: var(--accent);
        margin-left: 0.5rem;
        border: 1px solid rgba(124,58,237,0.4);
        padding: 0.05rem 0.3rem;
    }

    /* Recruit banner */
    .recruit-banner {
        margin-top: 0.5rem;
        display: grid;
        grid-template-columns: 1.15fr 1fr;
        gap: 0;
        border: 1px solid var(--border);
        background: var(--surface-2);
        overflow: hidden;
    }
    @media (max-width: 720px) {
        .recruit-banner { grid-template-columns: 1fr; }
    }
    .recruit-left {
        padding: 1.75rem 1.5rem;
        background:
            linear-gradient(135deg, rgba(124,58,237,0.18) 0%, transparent 55%),
            var(--surface-2);
        border-right: 1px solid var(--border);
    }
    @media (max-width: 720px) {
        .recruit-left { border-right: none; border-bottom: 1px solid var(--border); }
    }
    .recruit-right {
        padding: 1.5rem 1.35rem;
        background: var(--surface);
    }
    .expect-list {
        list-style: none;
        margin: 0.5rem 0 0;
        padding: 0;
    }
    .expect-list li {
        display: flex;
        gap: 0.55rem;
        padding: 0.45rem 0;
        color: var(--text-2);
        font-size: 0.86rem;
        border-bottom: 1px solid var(--border);
    }
    .expect-list li::before {
        content: "▸";
        color: var(--accent);
        font-weight: 700;
        flex-shrink: 0;
    }
    .never-box {
        margin-top: 1rem;
        padding: 0.85rem 0.9rem;
        border: 1px dashed rgba(124,58,237,0.35);
        background: rgba(8,8,12,0.4);
    }
    .never-box h4 {
        font-family: var(--font-mono);
        font-size: 0.62rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--accent);
        margin: 0 0 0.4rem;
    }
    .never-box p {
        margin: 0;
        font-size: 0.8rem;
        color: var(--text-2);
        line-height: 1.5;
    }
    .seek-tags {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        margin-top: 1rem;
    }
    .seek-tag {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        padding: 0.3rem 0.55rem;
        border: 1px solid rgba(124,58,237,0.4);
        color: var(--text);
        background: rgba(124,58,237,0.12);
    }

    .home-link {
        display: inline-block;
        margin-top: 0.9rem;
        font-family: var(--font-mono);
        font-size: 0.7rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--accent);
        border-bottom: 1px solid transparent;
    }
    .home-link:hover {
        color: var(--text);
        border-bottom-color: var(--accent);
    }

    .btn {
        display: inline-flex;
        align-items: center;
        padding: 0.7rem 1.25rem;
        font-family: var(--font-mono);
        font-size: 0.72rem;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        border-radius: 0;
        border: 1px solid transparent;
        cursor: pointer;
        text-decoration: none;
        transition: transform 0.15s, filter 0.15s, border-color 0.15s, color 0.15s, box-shadow 0.15s;
        clip-path: polygon(0 0, 100% 0, 100% calc(100% - 8px), calc(100% - 8px) 100%, 0 100%);
    }
    .btn-primary {
        background: var(--accent);
        color: #fff;
        box-shadow: 0 0 0 0 rgba(124,58,237,0.4);
    }
    .btn-primary:hover {
        filter: brightness(1.12);
        transform: translateY(-1px);
        box-shadow: 0 0 28px rgba(124,58,237,0.4);
    }
    .btn-outline {
        background: transparent;
        border-color: var(--border);
        color: var(--text);
    }
    .btn-outline:hover {
        border-color: var(--text-2);
        color: var(--text);
        transform: translateY(-1px);
    }

    .muted {
        color: var(--text-3);
        font-size: 0.88rem;
    }
    .home-foot {
        margin-top: 2.5rem;
        padding-top: 1.25rem;
        border-top: 1px solid var(--border);
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        color: var(--text-3);
    }

    /* ——— Alignment: Center = hero (+ recruit CTA feel) only; body always left ——— */
    .home.align-center .home-hero-inner {
        margin-left: auto;
        margin-right: auto;
        text-align: center;
        max-width: 42rem;
    }
    .home.align-center .home-sub {
        margin-left: auto;
        margin-right: auto;
        border-left: none;
        border-top: 3px solid var(--accent);
        padding-left: 0;
        padding-top: 0.85rem;
        max-width: 32rem;
    }
    .home.align-center .home-actions {
        justify-content: center;
    }
    .home.align-center .home-metrics {
        margin-left: auto;
        margin-right: auto;
    }
    .home.align-center .home-hero::after {
        right: 50%;
        transform: translateX(50%);
        opacity: 0.85;
    }
    .home.align-center .home-badge {
        margin-left: auto;
        margin-right: auto;
    }
    /* Body blocks intentionally stay left — no mixed centered headers over left lists */
    .home.align-center .home-block {
        text-align: left;
    }
    .home.align-center .recruit-left {
        text-align: left;
    }
    .home.align-center .recruit-left .btn {
        /* keep CTA easy to hit under centered brand, but block stays left */
    }
"#;

#[component]
pub fn Home() -> Element {
    let settings = use_resource(|| async {
        ApiClient::web()
            .fetch::<SiteSettings>("/api/settings")
            .await
            .ok()
    });
    let overview = use_resource(|| async {
        ApiClient::web()
            .fetch::<Overview>("/api/public/overview")
            .await
            .ok()
    });
    let announcements = use_resource(|| async {
        ApiClient::web()
            .fetch::<CursorPage<Announcement>>("/api/announcements")
            .await
            .ok()
            .map(|r| r.data)
    });
    let tournaments_res = use_resource(|| async {
        ApiClient::web()
            .fetch::<CursorPage<HomeTournament>>("/api/tournaments")
            .await
            .ok()
            .map(|r| r.data)
    });
    let events = use_resource(|| async {
        ApiClient::web()
            .fetch::<CursorPage<Event>>("/api/events")
            .await
            .ok()
            .map(|r| r.data)
    });

    let content = settings
        .read()
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(|s| s.homepage.clone())
        .unwrap_or_default();
    let layout = settings
        .read()
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(|s| s.public_layout)
        .unwrap_or(PublicLayout::Hub);
    let org_name = settings
        .read()
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(|s| s.org_name.clone())
        .unwrap_or_else(|| "The Scuffed Crew".into());
    let recruitment_open = settings
        .read()
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(|s| s.recruitment_open)
        .unwrap_or(true);

    // Metrics: only show values that tell a real story (skip empty/zero fluff).
    let (metric_squads, metric_members, metric_games) = {
        let o = overview.read();
        let o = o.as_ref().and_then(|o| o.as_ref());
        match o {
            Some(data) => {
                let with_roster = data
                    .teams
                    .iter()
                    .filter(|t| t.roster_count > 0)
                    .count();
                // Prefer squads that actually have people; hide "4 empty teams" as a vanity number.
                let squads = if with_roster > 0 {
                    Some(with_roster)
                } else {
                    None
                };
                let members = (data.member_count > 0).then_some(data.member_count);
                let games = (!data.games.is_empty()).then_some(data.games.len());
                (squads, members, games)
            }
            None => (None, None, None),
        }
    };
    let show_metrics =
        metric_squads.is_some() || metric_members.is_some() || metric_games.is_some();
    let home_class = format!("home {}", content.content_align.css_class());

    rsx! {
        style { {HOME_CSS} }
        div { class: "home-wrap",
        div { class: "{home_class}",
            // —— Hero ——
            header { class: "home-hero",
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
                        a { href: "#squads", class: "btn btn-outline", "{content.cta_secondary}" }
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

            // —— Ethos (numbered rules) ——
            section { class: "home-block",
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

            // —— What's on (schedule + tournaments) ——
            // Hub: only show panels that have data (no empty twin). Landing: show both.
            {
                let event_list = events.read().as_ref().and_then(|e| e.as_ref()).cloned().unwrap_or_default();
                let tourney_list = tournaments_res.read().as_ref().and_then(|t| t.as_ref()).cloned().unwrap_or_default();
                let live: Vec<HomeTournament> = tourney_list
                    .iter()
                    .filter(|t| t.status == "registration" || t.status == "in_progress")
                    .take(5)
                    .cloned()
                    .collect();
                let has_events = !event_list.is_empty();
                let has_tourneys = !live.is_empty();
                let is_landing = layout == PublicLayout::Landing;
                let show_schedule = has_events || is_landing;
                let show_tourneys = has_tourneys || is_landing;
                let show_live = show_schedule || show_tourneys;
                let both = show_schedule && show_tourneys;
                let grid_class = if both { "live-grid" } else { "live-grid single" };

                if show_live {
                    rsx! {
                        section { class: "home-block",
                            div { class: "{grid_class}",
                                if show_schedule {
                                    div { class: "live-panel",
                                        div { class: "home-kicker", "{content.schedule_kicker}" }
                                        h2 { class: "home-heading", "{content.schedule_title}" }
                                        if has_events {
                                            ul { class: "live-list",
                                                for e in event_list.iter() {
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
                                                for t in live.iter() {
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
                } else {
                    rsx! {}
                }
            }

            // —— Squads ——
            section { id: "squads", class: "home-block",
                div { class: "home-kicker", "{content.teams_kicker}" }
                h2 { class: "home-heading", "{content.teams_title}" }
                {
                    match overview.read().as_ref().and_then(|o| o.as_ref()) {
                        Some(data) if !data.teams.is_empty() => {
                            let game_map: HashMap<String, String> = data
                                .games
                                .iter()
                                .map(|g| (g.id.clone(), g.name.clone()))
                                .collect();
                            rsx! {
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
                            }
                        }
                        Some(_) => rsx! { p { class: "muted", "{content.teams_empty}" } },
                        None => rsx! { p { class: "muted", "Loading squads…" } },
                    }
                }
            }

            // —— Announcements (skip empty on hub) ——
            {
                let list = announcements
                    .read()
                    .as_ref()
                    .and_then(|a| a.as_ref())
                    .cloned()
                    .unwrap_or_default();
                let show = !list.is_empty() || layout == PublicLayout::Landing;
                if show {
                    rsx! {
                        section { class: "home-block",
                            div { class: "home-kicker", "{content.news_kicker}" }
                            h2 { class: "home-heading", "{content.news_title}" }
                            if list.is_empty() {
                                p { class: "muted", "{content.news_empty}" }
                            } else {
                                div { class: "news-rows",
                                    for a in list.iter().take(4) {
                                        { render_news_row(a) }
                                    }
                                }
                                Link { to: Route::News {}, class: "home-link", "{content.news_view_all}" }
                            }
                        }
                    }
                } else {
                    rsx! {}
                }
            }

            // —— Recruit ——
            if recruitment_open {
                section { class: "home-block",
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

            p { class: "home-foot",
                "{content.footer_note}"
                " · {org_name}"
            }
        }
        } // home-wrap
    }
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

fn render_team_row(team: &OverviewTeam, game_map: &HashMap<String, String>) -> Element {
    let game_name = game_map
        .get(&team.game_id)
        .cloned()
        .unwrap_or_else(|| team.game_id.clone());
    let wl = if team.record.wins == 0 && team.record.losses == 0 {
        "—".to_string()
    } else {
        format!("{}–{}", team.record.wins, team.record.losses)
    };
    let division = team
        .division
        .clone()
        .unwrap_or_else(|| "Internal".into());
    let lore = team.lore_quote.clone().unwrap_or_default();
    let roster_n = team.roster_count;

    rsx! {
        div { class: "team-row",
            div { class: "tm-name",
                "{team.name}"
                if !lore.is_empty() {
                    div { class: "team-lore", "“{lore}”" }
                }
            }
            div { class: "tm-game", "{game_name}" }
            div { class: "tm-roster", "{roster_n}" }
            div { class: "tm-div", "{division}" }
            div { class: "tm-wl", "{wl}" }
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
