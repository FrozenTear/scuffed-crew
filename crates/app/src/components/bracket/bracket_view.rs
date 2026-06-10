use dioxus::prelude::*;
use std::collections::HashMap;

use super::double_elim::BracketDoubleElim;
use super::round_robin::RoundRobinTable;
use super::single_elim::{BracketMatch, BracketRound, BracketSingleElim};
use super::swiss::{SwissStanding, SwissStandings};

#[component]
pub fn BracketView(
    format: String,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
    participant_ids: Vec<String>,
    swiss_standings: Vec<SwissStanding>,
) -> Element {
    match format.as_str() {
        "single_elim" => rsx! {
            BracketSingleElim {
                rounds: rounds,
                matches: matches,
                participant_names: participant_names,
            }
        },

        "double_elim" => rsx! {
            BracketDoubleElim {
                rounds: rounds,
                matches: matches,
                participant_names: participant_names,
            }
        },

        "round_robin" => rsx! {
            RoundRobinTable {
                matches: matches,
                participant_names: participant_names,
                participant_ids: participant_ids,
            }
        },

        "swiss" => rsx! {
            SwissStandings { standings: swiss_standings }
            div { class: "tournament-section-title", "Round Pairings" }
            BracketSingleElim {
                rounds: rounds,
                matches: matches,
                participant_names: participant_names,
            }
        },

        _ => rsx! {
            p { style: "color: var(--text-3);", "Unknown tournament format." }
        },
    }
}
