use leptos::prelude::*;
use std::collections::HashMap;

use super::double_elim::BracketDoubleElim;
use super::round_robin::RoundRobinTable;
use super::single_elim::{BracketMatch, BracketRound, BracketSingleElim};
use super::swiss::{SwissStanding, SwissStandings};

/// Dispatches to the correct bracket component based on tournament format.
#[component]
pub fn BracketView(
    format: String,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
    participant_ids: Vec<String>,
    swiss_standings: Vec<SwissStanding>,
) -> impl IntoView {
    match format.as_str() {
        "single_elim" => view! {
            <BracketSingleElim
                rounds=rounds
                matches=matches
                participant_names=participant_names
            />
        }
        .into_any(),

        "double_elim" => view! {
            <BracketDoubleElim
                rounds=rounds
                matches=matches
                participant_names=participant_names
            />
        }
        .into_any(),

        "round_robin" => view! {
            <RoundRobinTable
                matches=matches
                participant_names=participant_names
                participant_ids=participant_ids
            />
        }
        .into_any(),

        "swiss" => {
            let standings = swiss_standings;
            view! {
                <SwissStandings standings=standings/>
                <div class="tournament-section-title">"Round Pairings"</div>
                <BracketSingleElim
                    rounds=rounds
                    matches=matches
                    participant_names=participant_names
                />
            }
            .into_any()
        }

        _ => view! { <p style="color: var(--text-muted);">"Unknown tournament format."</p> }
            .into_any(),
    }
}
