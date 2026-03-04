use dioxus::prelude::*;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SwissStanding {
    pub participant_id: String,
    pub participant_name: String,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    pub game_wins: u32,
    pub game_losses: u32,
    pub buchholz: f64,
    pub rank: u32,
}

#[component]
pub fn SwissStandings(standings: Vec<SwissStanding>) -> Element {
    rsx! {
        table { class: "swiss-standings",
            thead {
                tr {
                    th { "#" }
                    th { "Team" }
                    th { "W" }
                    th { "L" }
                    th { "D" }
                    th { "GW" }
                    th { "GL" }
                    th { "Buch." }
                }
            }
            tbody {
                for s in standings.iter() {
                    tr {
                        td { class: "rank-col", "{s.rank}" }
                        td { "{s.participant_name}" }
                        td { class: "record-col", "{s.wins}" }
                        td { class: "record-col", "{s.losses}" }
                        td { class: "record-col", "{s.draws}" }
                        td { "{s.game_wins}" }
                        td { "{s.game_losses}" }
                        td { "{s.buchholz:.1}" }
                    }
                }
            }
        }
    }
}
