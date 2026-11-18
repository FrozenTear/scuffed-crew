//! One bracket slot: team, result line, team (`.bracket-slot` in the frames).
//!
//! The winner is never marked by colour alone. The result line says "Won" and
//! carries a small mark that points at the winning name, the winning name is
//! heavier, and assistive tech hears ", winner" after it.

use dioxus::prelude::*;

/// Shown for a side that is not known yet.
pub const TO_BE_DECIDED: &str = "To be decided";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchCardState {
    Pending,
    Scheduled,
    InProgress,
    Completed,
    Bye,
}

/// Which side of a match won.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

/// The result line of a slot: "Won 2 to 0", "In progress, 1 to 0",
/// "To be decided". A win puts the winner's score first.
pub fn slot_meta(
    state: MatchCardState,
    winner: Option<Side>,
    score_a: Option<u32>,
    score_b: Option<u32>,
) -> String {
    let scores = score_a.zip(score_b);
    match (state, winner) {
        (MatchCardState::Bye, _) => "Bye".to_string(),
        (_, Some(side)) => match (scores, side) {
            (Some((a, b)), Side::A) => format!("Won {a} to {b}"),
            (Some((a, b)), Side::B) => format!("Won {b} to {a}"),
            (None, _) => "Won".to_string(),
        },
        (MatchCardState::Completed, None) => match scores {
            Some((a, b)) if a == b => format!("Draw {a} to {b}"),
            Some((a, b)) => format!("Finished {a} to {b}"),
            None => "Finished".to_string(),
        },
        (MatchCardState::InProgress, None) => match scores {
            Some((a, b)) => format!("In progress, {a} to {b}"),
            None => "In progress".to_string(),
        },
        (MatchCardState::Scheduled, None) => "Scheduled".to_string(),
        (MatchCardState::Pending, None) => TO_BE_DECIDED.to_string(),
    }
}

/// One side of a slot. `None` is a side that is not known yet.
#[component]
fn SlotTeam(name: Option<String>, #[props(default = false)] is_winner: bool) -> Element {
    match name {
        Some(name) => rsx! {
            p { class: if is_winner { "slot-team is-winner" } else { "slot-team" },
                "{name}"
                if is_winner {
                    span { class: "visually-hidden", ", winner" }
                }
            }
        },
        None => rsx! { p { class: "slot-team is-open", "{TO_BE_DECIDED}" } },
    }
}

#[component]
pub fn MatchCard(
    team_a: Option<String>,
    team_b: Option<String>,
    score_a: Option<u32>,
    score_b: Option<u32>,
    winner: Option<Side>,
    #[props(default = MatchCardState::Pending)] state: MatchCardState,
    #[props(default = vec![])] replay_codes: Vec<String>,
) -> Element {
    let meta = slot_meta(state, winner, score_a, score_b);
    let is_bye = state == MatchCardState::Bye;
    let class = if state == MatchCardState::InProgress {
        "bracket-slot is-live"
    } else {
        "bracket-slot"
    };
    let mark = match winner {
        Some(_) if is_bye => None,
        Some(Side::A) => Some("slot-mark is-up"),
        Some(Side::B) => Some("slot-mark is-down"),
        None => None,
    };
    // "Won 2 to 1" sits between the two names, so on its own it reads as if
    // the team above won. Assistive tech hears the winner's name first.
    let subject = match winner {
        Some(_) if is_bye => None,
        Some(Side::A) => Some(team_a.clone().unwrap_or_else(|| "The winner".to_string())),
        Some(Side::B) => Some(team_b.clone().unwrap_or_else(|| "The winner".to_string())),
        None => None,
    };
    let show_codes = !replay_codes.is_empty() && state == MatchCardState::Completed;
    let codes_last = replay_codes.len().saturating_sub(1);

    rsx! {
        div { class: "{class}",
            if is_bye {
                // A bye has one team: it goes through without a match.
                SlotTeam { name: team_a.or(team_b) }
                p { class: "slot-meta", "{meta}" }
            } else {
                SlotTeam { name: team_a, is_winner: winner == Some(Side::A) }
                p { class: "slot-meta",
                    if let Some(mark) = mark {
                        span { class: "{mark}", aria_hidden: "true" }
                    }
                    if let Some(subject) = &subject {
                        span { class: "visually-hidden", "{subject}: " }
                    }
                    "{meta}"
                }
                SlotTeam { name: team_b, is_winner: winner == Some(Side::B) }
            }
            if show_codes {
                p { class: "slot-codes",
                    "Replay codes: "
                    for (i, code) in replay_codes.iter().enumerate() {
                        span { class: "slot-code", "{code}" }
                        if i < codes_last {
                            ", "
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::render;

    #[test]
    fn result_line_says_who_won_in_words() {
        use MatchCardState::*;
        assert_eq!(
            slot_meta(Completed, Some(Side::A), Some(2), Some(0)),
            "Won 2 to 0"
        );
        // The winner's score comes first whichever side won.
        assert_eq!(
            slot_meta(Completed, Some(Side::B), Some(1), Some(2)),
            "Won 2 to 1"
        );
        assert_eq!(slot_meta(Completed, Some(Side::B), None, None), "Won");
        assert_eq!(slot_meta(Completed, None, Some(1), Some(1)), "Draw 1 to 1");
        assert_eq!(slot_meta(Completed, None, None, None), "Finished");
        assert_eq!(
            slot_meta(InProgress, None, Some(1), Some(0)),
            "In progress, 1 to 0"
        );
        assert_eq!(slot_meta(InProgress, None, None, None), "In progress");
        assert_eq!(slot_meta(Scheduled, None, None, None), "Scheduled");
        assert_eq!(slot_meta(Pending, None, None, None), "To be decided");
        assert_eq!(slot_meta(Bye, Some(Side::A), None, None), "Bye");
    }

    #[test]
    fn a_second_team_win_names_the_winner_on_the_result_line() {
        fn app() -> Element {
            rsx! {
                MatchCard {
                    team_a: Some("Squad Alpha".to_string()),
                    team_b: Some("Crew Beta".to_string()),
                    score_a: Some(1),
                    score_b: Some(2),
                    winner: Some(Side::B),
                    state: MatchCardState::Completed,
                }
            }
        }
        let html = render(app);
        assert!(
            html.contains(
                "<p class=\"slot-meta\"><span class=\"slot-mark is-down\" aria-hidden=\"true\"></span>\
                 <span class=\"visually-hidden\">Crew Beta: </span>Won 2 to 1</p>"
            ),
            "{html}"
        );
    }

    #[test]
    fn winner_has_a_word_a_mark_and_a_name_for_assistive_tech() {
        fn app() -> Element {
            rsx! {
                MatchCard {
                    team_a: Some("Squad Alpha".to_string()),
                    team_b: Some("Crew Beta".to_string()),
                    score_a: Some(2),
                    score_b: Some(0),
                    winner: Some(Side::A),
                    state: MatchCardState::Completed,
                }
            }
        }
        let html = render(app);
        assert_eq!(
            html,
            "<div class=\"bracket-slot\">\
             <p class=\"slot-team is-winner\">Squad Alpha<span class=\"visually-hidden\">, winner</span></p>\
             <p class=\"slot-meta\"><span class=\"slot-mark is-up\" aria-hidden=\"true\"></span>\
             <span class=\"visually-hidden\">Squad Alpha: </span>Won 2 to 0</p>\
             <p class=\"slot-team\">Crew Beta</p>\
             </div>"
        );
    }

    #[test]
    fn mark_points_down_when_the_second_team_won() {
        fn app() -> Element {
            rsx! {
                MatchCard {
                    team_a: Some("Squad Gamma".to_string()),
                    team_b: Some("Crew Delta".to_string()),
                    score_a: Some(1),
                    score_b: Some(2),
                    winner: Some(Side::B),
                    state: MatchCardState::Completed,
                }
            }
        }
        let html = render(app);
        assert!(
            html.contains("<p class=\"slot-team\">Squad Gamma</p>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<span class=\"slot-mark is-down\" aria-hidden=\"true\"></span>\
                 <span class=\"visually-hidden\">Crew Delta: </span>Won 2 to 1</p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"slot-team is-winner\">Crew Delta<span class=\"visually-hidden\">, winner</span></p>"),
            "{html}"
        );
        assert_eq!(html.matches(", winner").count(), 1, "{html}");
    }

    #[test]
    fn open_sides_and_pending_matches_use_the_frame_words() {
        fn app() -> Element {
            rsx! {
                MatchCard {
                    team_a: Some("Squad Alpha".to_string()),
                    team_b: None::<String>,
                    score_a: None::<u32>,
                    score_b: None::<u32>,
                    winner: None::<Side>,
                }
            }
        }
        let html = render(app);
        assert!(
            html.contains("<p class=\"slot-meta\">To be decided</p>"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"slot-team is-open\">To be decided</p>"),
            "{html}"
        );
        assert!(!html.contains("TBD"), "{html}");
        assert!(!html.contains("slot-mark"), "{html}");
    }

    #[test]
    fn live_bye_and_replay_codes() {
        fn live() -> Element {
            rsx! {
                MatchCard {
                    team_a: Some("Squad Alpha".to_string()),
                    team_b: Some("Crew Beta".to_string()),
                    score_a: Some(1),
                    score_b: Some(0),
                    winner: None::<Side>,
                    state: MatchCardState::InProgress,
                }
            }
        }
        let html = render(live);
        assert!(html.contains("class=\"bracket-slot is-live\""), "{html}");
        assert!(html.contains("In progress, 1 to 0"), "{html}");

        fn bye() -> Element {
            rsx! {
                MatchCard {
                    team_a: None::<String>,
                    team_b: Some("Crew Beta".to_string()),
                    score_a: None::<u32>,
                    score_b: None::<u32>,
                    winner: Some(Side::B),
                    state: MatchCardState::Bye,
                }
            }
        }
        let html = render(bye);
        assert_eq!(
            html,
            "<div class=\"bracket-slot\"><p class=\"slot-team\">Crew Beta</p>\
             <p class=\"slot-meta\">Bye</p></div>"
        );

        fn codes() -> Element {
            rsx! {
                MatchCard {
                    team_a: Some("Squad Alpha".to_string()),
                    team_b: Some("Crew Beta".to_string()),
                    score_a: Some(2),
                    score_b: Some(0),
                    winner: Some(Side::A),
                    state: MatchCardState::Completed,
                    replay_codes: vec!["A1B2C3".to_string(), "D4E5F6".to_string()],
                }
            }
        }
        let html = render(codes);
        assert!(
            html.contains(
                "<p class=\"slot-codes\">Replay codes: <span class=\"slot-code\">A1B2C3</span>, \
                 <span class=\"slot-code\">D4E5F6</span></p>"
            ),
            "{html}"
        );
        // No control that claims to copy: the codes are plain text.
        assert!(!html.contains("title="), "{html}");
        assert!(!html.contains("<button"), "{html}");
    }
}
