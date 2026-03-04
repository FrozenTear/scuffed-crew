use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum MatchCardState {
    Pending,
    Scheduled,
    InProgress,
    Completed,
    Bye,
}

#[component]
pub fn TeamSlot(
    name: String,
    seed: Option<u32>,
    score: Option<u32>,
    #[props(default = false)] is_winner: bool,
    #[props(default = false)] is_loser: bool,
    #[props(default = false)] is_tbd: bool,
) -> Element {
    let class = if is_winner {
        "team-slot winner"
    } else if is_loser {
        "team-slot loser"
    } else {
        "team-slot"
    };

    let name_class = if is_tbd {
        "team-slot-name tbd"
    } else {
        "team-slot-name"
    };

    rsx! {
        div { class: "{class}",
            if let Some(s) = seed {
                span { class: "team-slot-seed", "[{s}]" }
            }
            span { class: "{name_class}", "{name}" }
            if let Some(s) = score {
                span { class: "team-slot-score", "{s}" }
            }
        }
    }
}

#[component]
pub fn MatchCard(
    team_a_name: String,
    team_b_name: String,
    seed_a: Option<u32>,
    seed_b: Option<u32>,
    score_a: Option<u32>,
    score_b: Option<u32>,
    winner: Option<String>,
    #[props(default = MatchCardState::Pending)] state: MatchCardState,
    #[props(default = vec![])] replay_codes: Vec<String>,
) -> Element {
    let card_class = match state {
        MatchCardState::InProgress => "match-card live",
        MatchCardState::Completed => "match-card completed",
        MatchCardState::Bye => "match-card bye",
        _ => "match-card",
    };

    let is_a_winner = winner.as_deref() == Some("a");
    let is_b_winner = winner.as_deref() == Some("b");
    let has_winner = winner.is_some();
    let a_tbd = team_a_name == "TBD";
    let b_tbd = team_b_name == "TBD";

    let status_text = match state {
        MatchCardState::InProgress => Some("LIVE"),
        MatchCardState::Scheduled => Some("Scheduled"),
        MatchCardState::Bye => Some("BYE"),
        _ => None,
    };
    let status_class = match state {
        MatchCardState::InProgress => "match-card-status live",
        _ => "match-card-status",
    };

    let show_codes = !replay_codes.is_empty() && state == MatchCardState::Completed;

    rsx! {
        div { class: "{card_class}",
            TeamSlot {
                name: team_a_name,
                seed: seed_a,
                score: score_a,
                is_winner: is_a_winner,
                is_loser: has_winner && !is_a_winner,
                is_tbd: a_tbd,
            }
            TeamSlot {
                name: team_b_name,
                seed: seed_b,
                score: score_b,
                is_winner: is_b_winner,
                is_loser: has_winner && !is_b_winner,
                is_tbd: b_tbd,
            }
            if let Some(text) = status_text {
                div { class: "{status_class}", "{text}" }
            }
            if show_codes {
                div { class: "match-replay-codes",
                    for code in replay_codes.iter() {
                        span {
                            class: "replay-code",
                            title: "Click to copy",
                            "{code}"
                        }
                    }
                }
            }
        }
    }
}
