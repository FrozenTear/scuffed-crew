use leptos::prelude::*;
use serde::Deserialize;

use scuffed_auth::client::api::fetch_json;

use crate::components::SectionHeader;

#[derive(Debug, Clone, Deserialize)]
struct TeamRecord {
    wins: u32,
    losses: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct GameData {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TeamData {
    name: String,
    game_id: String,
    division: Option<String>,
    lore_quote: Option<String>,
    roster_count: usize,
    record: TeamRecord,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicOverview {
    teams: Vec<TeamData>,
    games: Vec<GameData>,
}

fn game_css_class(game: &str) -> &'static str {
    let lower = game.to_lowercase();
    if lower.contains("overwatch") {
        "ow"
    } else if lower.contains("destiny") {
        "destiny"
    } else {
        "other"
    }
}

fn game_badge_class(game: &str) -> &'static str {
    let lower = game.to_lowercase();
    if lower.contains("overwatch") {
        "game-ow"
    } else if lower.contains("destiny") {
        "game-dest"
    } else {
        "game-other"
    }
}

fn format_record(record: &TeamRecord) -> String {
    if record.wins == 0 && record.losses == 0 {
        "\u{2014}".to_string()
    } else {
        format!("{}-{}", record.wins, record.losses)
    }
}

#[component]
pub fn Teams() -> impl IntoView {
    let overview = LocalResource::new(|| async {
        fetch_json::<PublicOverview>("/api/public/overview")
            .await
            .ok()
    });

    view! {
        <section id="teams">
            <SectionHeader
                label="// Active Squads"
                title="The Teams"
                color="red"
                description="Each team carries a name from the lore of the game they play. Your team is your identity within the org."
            />

            <div class="teams-grid">
                {move || match overview.get().flatten() {
                    Some(data) => {
                        let game_map: std::collections::HashMap<String, String> = data.games.iter()
                            .map(|g| (g.id.clone(), g.name.clone()))
                            .collect();
                        data.teams.into_iter().enumerate().map(|(i, team)| {
                        let game_name = game_map.get(&team.game_id).cloned().unwrap_or_else(|| team.game_id.clone());
                        let game_class = game_css_class(&game_name);
                        let badge_class = game_badge_class(&game_name);
                        let wl = format_record(&team.record);
                        let division = team.division.unwrap_or_else(|| "Scrims & Internal".into());
                        let lore = team.lore_quote.unwrap_or_default();
                        let delay = (i + 1).to_string();

                        view! {
                            <div
                                class="team-card"
                                data-game=game_class
                                data-reveal=""
                                data-reveal-delay=delay
                            >
                                <div class="team-header">
                                    <div class="team-name">{team.name}</div>
                                    <span class=format!("team-game {badge_class}")>{game_name}</span>
                                </div>
                                <div class="team-lore">{format!("\u{201C}{lore}\u{201D}")}</div>
                                <div class="team-meta">
                                    <div class="team-meta-item">
                                        <span class="team-meta-val">{team.roster_count.to_string()}</span>
                                        <span class="team-meta-label">"Roster"</span>
                                    </div>
                                    <div class="team-meta-item">
                                        <span class="team-meta-val">{wl}</span>
                                        <span class="team-meta-label">"W-L"</span>
                                    </div>
                                </div>
                                <div class="team-division">
                                    <span class="faceit-dot"></span>
                                    {division}
                                </div>
                            </div>
                        }
                    }).collect_view().into_any()
                    },
                    None => view! {
                        <p style="color: var(--text-muted); text-align: center;">"Loading teams..."</p>
                    }.into_any(),
                }}
            </div>
        </section>
    }
}
