use leptos::prelude::*;
use std::collections::HashMap;

use super::match_card::{MatchCard, MatchCardState};
use super::single_elim::{BracketMatch, BracketRound};

fn match_state(status: &str) -> MatchCardState {
    match status {
        "in_progress" => MatchCardState::InProgress,
        "completed" => MatchCardState::Completed,
        "bye" => MatchCardState::Bye,
        "scheduled" => MatchCardState::Scheduled,
        _ => MatchCardState::Pending,
    }
}

fn render_bracket_section(
    label: String,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    names: HashMap<String, String>,
) -> impl IntoView {
    view! {
        <div>
            <div class="double-elim-section-label">{label}</div>
            <div class="bracket-container">
                <div class="bracket">
                    {rounds.into_iter().enumerate().map(|(ri, round)| {
                        let mut rm: Vec<BracketMatch> = matches.iter()
                            .filter(|m| m.round_id == round.id)
                            .cloned()
                            .collect();
                        rm.sort_by_key(|m| m.bracket_position);
                        let round_label = format!("Round {}", ri + 1);
                        let names = names.clone();

                        view! {
                            <div class="bracket-round">
                                <div class="bracket-round-title">{round_label}</div>
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
                                        if m.participant_a_id.as_ref() == Some(w) { Some("a".to_string()) }
                                        else if m.participant_b_id.as_ref() == Some(w) { Some("b".to_string()) }
                                        else { None }
                                    });

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
    }
}

/// Double elimination bracket with winners, losers, and grand final sections.
#[component]
pub fn BracketDoubleElim(
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
) -> impl IntoView {
    let mut winners_rounds: Vec<BracketRound> = rounds
        .iter()
        .filter(|r| r.stage == "winners")
        .cloned()
        .collect();
    winners_rounds.sort_by_key(|r| r.round_number);

    let mut losers_rounds: Vec<BracketRound> = rounds
        .iter()
        .filter(|r| r.stage == "losers")
        .cloned()
        .collect();
    losers_rounds.sort_by_key(|r| r.round_number);

    let gf_rounds: Vec<BracketRound> = rounds
        .iter()
        .filter(|r| r.stage == "grand_final")
        .cloned()
        .collect();

    let m1 = matches.clone();
    let m2 = matches.clone();
    let m3 = matches.clone();
    let n1 = participant_names.clone();
    let n2 = participant_names.clone();
    let n3 = participant_names.clone();

    view! {
        <div class="double-elim-container">
            {render_bracket_section("Winners Bracket".to_string(), winners_rounds, m1, n1)}
            {render_bracket_section("Losers Bracket".to_string(), losers_rounds, m2, n2)}
            {if !gf_rounds.is_empty() {
                Some(render_bracket_section("Grand Final".to_string(), gf_rounds, m3, n3))
            } else {
                None
            }}
        </div>
    }
}
