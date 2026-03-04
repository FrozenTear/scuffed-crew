use dioxus::prelude::*;
use std::collections::HashMap;

use super::match_card::MatchCard;
use super::single_elim::{BracketMatch, BracketRound, match_state};

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

#[component]
fn BracketSection(
    label: String,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    names: HashMap<String, String>,
) -> Element {
    rsx! {
        div {
            div { class: "double-elim-section-label", "{label}" }
            div { class: "bracket-container",
                div { class: "bracket",
                    for (ri, round) in rounds.iter().enumerate() {
                        {
                            let mut rm: Vec<&BracketMatch> = matches.iter()
                                .filter(|m| m.round_id == round.id)
                                .collect();
                            rm.sort_by_key(|m| m.bracket_position);
                            let round_label = format!("Round {}", ri + 1);

                            rsx! {
                                div { class: "bracket-round",
                                    div { class: "bracket-round-title", "{round_label}" }
                                    for m in rm.iter() {
                                        {
                                            let a_name = m.participant_a_id.as_ref()
                                                .and_then(|id| names.get(id))
                                                .cloned()
                                                .unwrap_or_else(|| "TBD".to_string());
                                            let b_name = m.participant_b_id.as_ref()
                                                .and_then(|id| names.get(id))
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
}

#[component]
pub fn BracketDoubleElim(
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
) -> Element {
    let mut winners: Vec<BracketRound> = rounds.iter().filter(|r| r.stage == "winners").cloned().collect();
    winners.sort_by_key(|r| r.round_number);

    let mut losers: Vec<BracketRound> = rounds.iter().filter(|r| r.stage == "losers").cloned().collect();
    losers.sort_by_key(|r| r.round_number);

    let gf: Vec<BracketRound> = rounds.iter().filter(|r| r.stage == "grand_final").cloned().collect();

    rsx! {
        div { class: "double-elim-container",
            BracketSection {
                label: "Winners Bracket".to_string(),
                rounds: winners,
                matches: matches.clone(),
                names: participant_names.clone(),
            }
            BracketSection {
                label: "Losers Bracket".to_string(),
                rounds: losers,
                matches: matches.clone(),
                names: participant_names.clone(),
            }
            if !gf.is_empty() {
                BracketSection {
                    label: "Grand Final".to_string(),
                    rounds: gf,
                    matches: matches,
                    names: participant_names,
                }
            }
        }
    }
}
