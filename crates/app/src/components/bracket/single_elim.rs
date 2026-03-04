use dioxus::prelude::*;
use std::collections::HashMap;

use super::match_card::{MatchCard, MatchCardState};

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
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
    #[serde(default)]
    pub replay_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct BracketRound {
    pub id: String,
    pub round_number: u32,
    pub stage: String,
}

pub fn match_state(status: &str) -> MatchCardState {
    match status {
        "in_progress" => MatchCardState::InProgress,
        "completed" => MatchCardState::Completed,
        "bye" => MatchCardState::Bye,
        "scheduled" => MatchCardState::Scheduled,
        _ => MatchCardState::Pending,
    }
}

fn resolve_winner(m: &BracketMatch) -> Option<String> {
    m.winner_id.as_ref().and_then(|w| {
        if m.participant_a_id.as_ref() == Some(w) {
            Some("a".to_string())
        } else if m.participant_b_id.as_ref() == Some(w) {
            Some("b".to_string())
        } else {
            None
        }
    })
}

fn round_label(index: usize, total: usize) -> String {
    if index == total - 1 {
        "Final".to_string()
    } else if total > 1 && index == total - 2 {
        "Semifinals".to_string()
    } else if total > 2 && index == total - 3 {
        "Quarterfinals".to_string()
    } else {
        format!("Round {}", index + 1)
    }
}

#[component]
pub fn BracketSingleElim(
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
) -> Element {
    let mut filtered: Vec<BracketRound> = rounds
        .iter()
        .filter(|r| r.stage == "main" || r.stage == "winners")
        .cloned()
        .collect();
    filtered.sort_by_key(|r| r.round_number);
    let total = filtered.len();

    rsx! {
        // Desktop horizontal bracket
        div { class: "bracket-desktop",
            div { class: "bracket-container",
                div { class: "bracket",
                    for (ri, round) in filtered.iter().enumerate() {
                        {
                            let mut rm: Vec<&BracketMatch> = matches.iter()
                                .filter(|m| m.round_id == round.id)
                                .collect();
                            rm.sort_by_key(|m| m.bracket_position);
                            let label = round_label(ri, total);

                            rsx! {
                                div { class: "bracket-round",
                                    div { class: "bracket-round-title", "{label}" }
                                    for m in rm.iter() {
                                        {
                                            let a_name = m.participant_a_id.as_ref()
                                                .and_then(|id| participant_names.get(id))
                                                .cloned()
                                                .unwrap_or_else(|| "TBD".to_string());
                                            let b_name = m.participant_b_id.as_ref()
                                                .and_then(|id| participant_names.get(id))
                                                .cloned()
                                                .unwrap_or_else(|| "TBD".to_string());
                                            let winner = resolve_winner(m);

                                            rsx! {
                                                MatchCard {
                                                    team_a_name: a_name,
                                                    team_b_name: b_name,
                                                    seed_a: None::<u32>,
                                                    seed_b: None::<u32>,
                                                    score_a: m.score_a,
                                                    score_b: m.score_b,
                                                    winner: winner,
                                                    state: match_state(&m.status),
                                                    replay_codes: m.replay_codes.clone(),
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Mobile vertical list
        div { class: "bracket-mobile",
            for (ri, round) in filtered.iter().enumerate() {
                {
                    let mut rm: Vec<&BracketMatch> = matches.iter()
                        .filter(|m| m.round_id == round.id)
                        .collect();
                    rm.sort_by_key(|m| m.bracket_position);
                    let label = round_label(ri, total);

                    rsx! {
                        div { class: "bracket-mobile-round",
                            div { class: "bracket-mobile-round-title", "{label}" }
                            div { class: "bracket-mobile-matches",
                                for m in rm.iter() {
                                    {
                                        let a_name = m.participant_a_id.as_ref()
                                            .and_then(|id| participant_names.get(id))
                                            .cloned()
                                            .unwrap_or_else(|| "TBD".to_string());
                                        let b_name = m.participant_b_id.as_ref()
                                            .and_then(|id| participant_names.get(id))
                                            .cloned()
                                            .unwrap_or_else(|| "TBD".to_string());
                                        let winner = resolve_winner(m);

                                        rsx! {
                                            MatchCard {
                                                team_a_name: a_name,
                                                team_b_name: b_name,
                                                seed_a: None::<u32>,
                                                seed_b: None::<u32>,
                                                score_a: m.score_a,
                                                score_b: m.score_b,
                                                winner: winner,
                                                state: match_state(&m.status),
                                                replay_codes: m.replay_codes.clone(),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
