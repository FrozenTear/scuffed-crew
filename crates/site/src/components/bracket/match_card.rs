use leptos::prelude::*;

/// A participant slot within a match card.
#[component]
pub fn TeamSlot(
    #[prop(into)] name: String,
    seed: Option<u32>,
    score: Option<u32>,
    #[prop(default = false)] is_winner: bool,
    #[prop(default = false)] is_loser: bool,
    #[prop(default = false)] is_tbd: bool,
) -> impl IntoView {
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

    view! {
        <div class=class>
            {seed.map(|s| view! { <span class="team-slot-seed">{format!("[{s}]")}</span> })}
            <span class=name_class>{name}</span>
            {score.map(|s| view! { <span class="team-slot-score">{s}</span> })}
        </div>
    }
}

/// Possible match display states.
#[derive(Clone, Copy, PartialEq)]
pub enum MatchCardState {
    Pending,
    Scheduled,
    InProgress,
    Completed,
    Bye,
}

/// A single match in a bracket (two team slots + status).
#[component]
pub fn MatchCard(
    #[prop(into)] team_a_name: String,
    #[prop(into)] team_b_name: String,
    seed_a: Option<u32>,
    seed_b: Option<u32>,
    score_a: Option<u32>,
    score_b: Option<u32>,
    winner: Option<String>,
    #[prop(default = MatchCardState::Pending)] state: MatchCardState,
) -> impl IntoView {
    let card_class = match state {
        MatchCardState::InProgress => "match-card live",
        MatchCardState::Completed => "match-card completed",
        MatchCardState::Bye => "match-card bye",
        _ => "match-card",
    };

    let is_a_winner = winner.as_deref() == Some("a");
    let is_b_winner = winner.as_deref() == Some("b");
    let has_winner = winner.is_some();

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

    let a_tbd = team_a_name == "TBD";
    let b_tbd = team_b_name == "TBD";

    view! {
        <div class=card_class>
            <TeamSlot
                name=team_a_name
                seed=seed_a
                score=score_a
                is_winner=is_a_winner
                is_loser={has_winner && !is_a_winner}
                is_tbd=a_tbd
            />
            <TeamSlot
                name=team_b_name
                seed=seed_b
                score=score_b
                is_winner=is_b_winner
                is_loser={has_winner && !is_b_winner}
                is_tbd=b_tbd
            />
            {status_text.map(|text| view! {
                <div class=status_class>{text}</div>
            })}
        </div>
    }
}
