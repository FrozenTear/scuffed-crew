use leptos::prelude::*;
use std::collections::HashMap;

use super::match_card::{MatchCard, MatchCardState};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct BracketMatch {
    pub id: String,
    pub round_id: String,
    pub bracket_position: u32,
    pub participant_a_id: Option<String>,
    pub participant_b_id: Option<String>,
    pub score_a: Option<u32>,
    pub score_b: Option<u32>,
    pub winner_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct BracketRound {
    pub id: String,
    pub round_number: u32,
    pub stage: String,
}

fn match_state(status: &str) -> MatchCardState {
    match status {
        "in_progress" => MatchCardState::InProgress,
        "completed" => MatchCardState::Completed,
        "bye" => MatchCardState::Bye,
        "scheduled" => MatchCardState::Scheduled,
        _ => MatchCardState::Pending,
    }
}

/// Full single elimination bracket visualization.
#[component]
pub fn BracketSingleElim(
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
) -> impl IntoView {
    let mut filtered_rounds: Vec<BracketRound> = rounds
        .iter()
        .filter(|r| r.stage == "main" || r.stage == "winners")
        .cloned()
        .collect();
    filtered_rounds.sort_by_key(|r| r.round_number);

    let total = filtered_rounds.len();
    let round_labels: Vec<String> = filtered_rounds
        .iter()
        .enumerate()
        .map(|(i, _)| {
            if i == total - 1 {
                "Final".to_string()
            } else if total > 1 && i == total - 2 {
                "Semifinals".to_string()
            } else if total > 2 && i == total - 3 {
                "Quarterfinals".to_string()
            } else {
                format!("Round {}", i + 1)
            }
        })
        .collect();

    let names = participant_names.clone();
    let names2 = participant_names.clone();
    let matches2 = matches.clone();
    let filtered2 = filtered_rounds.clone();
    let labels2 = round_labels.clone();

    view! {
        // Desktop horizontal bracket
        <div class="bracket-desktop">
            <div class="bracket-container">
                <div class="bracket">
                    {filtered_rounds.into_iter().enumerate().map(|(ri, round)| {
                        let mut rm: Vec<BracketMatch> = matches.iter()
                            .filter(|m| m.round_id == round.id)
                            .cloned()
                            .collect();
                        rm.sort_by_key(|m| m.bracket_position);
                        let label = round_labels[ri].clone();
                        let names = names.clone();

                        view! {
                            <div class="bracket-round">
                                <div class="bracket-round-title">{label}</div>
                                {rm.into_iter().map(|m| {
                                    let a_name = m.participant_a_id.as_ref()
                                        .and_then(|id| names.get(id))
                                        .cloned()
                                        .unwrap_or_else(|| "TBD".to_string());
                                    let b_name = m.participant_b_id.as_ref()
                                        .and_then(|id| names.get(id))
                                        .cloned()
                                        .unwrap_or_else(|| "TBD".to_string());
                                    let winner = m.winner_id.as_ref().and_then(|w| {
                                        if m.participant_a_id.as_ref() == Some(w) { Some("a") }
                                        else if m.participant_b_id.as_ref() == Some(w) { Some("b") }
                                        else { None }
                                    }).map(|s| s.to_string());

                                    view! {
                                        <MatchCard
                                            team_a_name=a_name
                                            team_b_name=b_name
                                            seed_a=None
                                            seed_b=None
                                            score_a=m.score_a
                                            score_b=m.score_b
                                            winner=winner
                                            state=match_state(&m.status)
                                        />
                                    }
                                }).collect_view()}
                            </div>
                        }
                    }).collect_view()}
                </div>
            </div>
        </div>

        // Mobile vertical list
        <div class="bracket-mobile">
            {filtered2.into_iter().enumerate().map(|(ri, round)| {
                let mut rm: Vec<BracketMatch> = matches2.iter()
                    .filter(|m| m.round_id == round.id)
                    .cloned()
                    .collect();
                rm.sort_by_key(|m| m.bracket_position);
                let label = labels2[ri].clone();
                let names = names2.clone();

                view! {
                    <div class="bracket-mobile-round">
                        <div class="bracket-mobile-round-title">{label}</div>
                        <div class="bracket-mobile-matches">
                            {rm.into_iter().map(|m| {
                                let a_name = m.participant_a_id.as_ref()
                                    .and_then(|id| names.get(id))
                                    .cloned()
                                    .unwrap_or_else(|| "TBD".to_string());
                                let b_name = m.participant_b_id.as_ref()
                                    .and_then(|id| names.get(id))
                                    .cloned()
                                    .unwrap_or_else(|| "TBD".to_string());
                                let winner = m.winner_id.as_ref().and_then(|w| {
                                    if m.participant_a_id.as_ref() == Some(w) { Some("a") }
                                    else if m.participant_b_id.as_ref() == Some(w) { Some("b") }
                                    else { None }
                                }).map(|s| s.to_string());

                                view! {
                                    <MatchCard
                                        team_a_name=a_name
                                        team_b_name=b_name
                                        seed_a=None
                                        seed_b=None
                                        score_a=m.score_a
                                        score_b=m.score_b
                                        winner=winner
                                        state=match_state(&m.status)
                                    />
                                }
                            }).collect_view()}
                        </div>
                    </div>
                }
            }).collect_view()}
        </div>
    }
}
