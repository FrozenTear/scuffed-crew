use leptos::prelude::*;
use serde::Deserialize;

use scuffed_auth::client::api::fetch_json;

#[derive(Debug, Clone, Deserialize)]
struct OverviewGame {
    #[allow(dead_code)]
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OverviewTeam {
    #[allow(dead_code)]
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Overview {
    teams: Vec<OverviewTeam>,
    games: Vec<OverviewGame>,
    member_count: usize,
}

#[component]
pub fn Hero() -> impl IntoView {
    let overview = LocalResource::new(|| async {
        fetch_json::<Overview>("/api/public/overview").await.ok()
    });

    let team_count = move || {
        overview.get().flatten().map(|o| o.teams.len()).unwrap_or(0)
    };
    let member_count = move || {
        overview.get().flatten().map(|o| o.member_count).unwrap_or(0)
    };
    let game_count = move || {
        overview.get().flatten().map(|o| o.games.len()).unwrap_or(0)
    };

    view! {
        <section class="hero">
            <div class="hero-bg"></div>
            <div class="hero-grid"></div>
            <div class="hero-scanline"></div>
            <div class="hero-scanline"></div>
            <div class="hero-spotlight"></div>
            <svg class="hero-emblem" viewBox="0 0 400 400" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M200 20 L360 120 L340 320 L200 380 L60 320 L40 120 Z" />
                <path d="M200 60 L320 140 L305 290 L200 340 L95 290 L80 140 Z" />
                <path d="M200 100 L280 160 L270 260 L200 300 L130 260 L120 160 Z" />
                <line x1="200" y1="20" x2="200" y2="380" />
                <line x1="40" y1="120" x2="340" y2="320" />
                <line x1="360" y1="120" x2="60" y2="320" />
                <path d="M200 170 L230 200 L200 230 L170 200 Z" />
            </svg>
            <div class="hero-content">
                <div class="hero-badge">
                    <span class="badge-dot"></span>
                    "Multi-Game Crew \u{2014} EMEA \u{2014} Est. 2026"
                </div>
                <h1 class="hero-title">"The Scuffed"<br/><span class="purple">"Crew"</span></h1>
                <p class="hero-sub">"A multi-game crew built on old-school clan principles. Small teams, real structure, scheduled play nights. No ghost members. No dead servers."</p>
                <div class="hero-actions">
                    <a href="#join" class="btn btn-primary">"Apply to Join"</a>
                    <a href="#teams" class="btn btn-outline">"Our Teams"</a>
                </div>
                <div class="hero-stats">
                    <div class="hero-stat">
                        <div class="hero-stat-val">{team_count}</div>
                        <div class="hero-stat-label">"Active Teams"</div>
                    </div>
                    <div class="hero-stat">
                        <div class="hero-stat-val">{member_count}</div>
                        <div class="hero-stat-label">"Members"</div>
                    </div>
                    <div class="hero-stat">
                        <div class="hero-stat-val">{game_count}</div>
                        <div class="hero-stat-label">"Games"</div>
                    </div>
                </div>
            </div>
        </section>
    }
}
