//! The bracket as the frames draw it: round labels in one header row, a
//! column of slots under each, inside a focusable scroller with a name.

use dioxus::prelude::*;
use std::collections::HashMap;

use super::match_card::{MatchCard, MatchCardState, Side, TO_BE_DECIDED};

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

/// The side whose participant id is the match's `winner_id`.
pub(super) fn resolve_winner(m: &BracketMatch) -> Option<Side> {
    let winner = m.winner_id.as_ref()?;
    if m.participant_a_id.as_ref() == Some(winner) {
        Some(Side::A)
    } else if m.participant_b_id.as_ref() == Some(winner) {
        Some(Side::B)
    } else {
        None
    }
}

/// Label of round `index` (from 0) in a knockout of `total` rounds.
pub(super) fn round_label(index: usize, total: usize) -> String {
    match total - index {
        1 => "Final".to_string(),
        2 => "Semi-finals".to_string(),
        3 => "Quarter-finals".to_string(),
        _ => format!("Round {}", index + 1),
    }
}

/// Labels for rounds that are not a knockout ladder: "Round 1", "Round 2".
pub(super) fn numbered_labels(total: usize) -> Vec<String> {
    (1..=total).map(|n| format!("Round {n}")).collect()
}

/// The hint above a scroller, built from the real column names: "Swipe for
/// Final and Champion". `None` while the bracket has fewer than three columns,
/// since two fit a phone without scrolling.
pub(super) fn swipe_hint(labels: &[String]) -> Option<String> {
    if labels.len() < 3 {
        return None;
    }
    let later = &labels[1..];
    let list = match later {
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        _ => format!("{} through {}", later[0], later[later.len() - 1]),
    };
    Some(format!("Swipe for {list}"))
}

/// Heading level of the round labels: H2 right under the page H1, H3 when the
/// bracket sits under a section title of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadLevel {
    H2,
    H3,
}

/// The heading a round starts with: visually hidden, since the header row
/// shows the same words above the column.
fn round_heading(head: HeadLevel, text: &str) -> Element {
    match head {
        HeadLevel::H2 => rsx! { h2 { class: "visually-hidden", "{text}" } },
        HeadLevel::H3 => rsx! { h3 { class: "visually-hidden", "{text}" } },
    }
}

/// Rounds side by side. `labels` has one entry per round. With `champion` a
/// last column names the winner of the last round's only match.
#[component]
pub(super) fn BracketColumns(
    /// What the scroller holds: "Test Cup bracket".
    name: String,
    labels: Vec<String>,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
    #[props(default = false)] champion: bool,
    #[props(default = HeadLevel::H2)] head: HeadLevel,
    /// Title of the section the columns sit under. A round with the same
    /// label gets no heading of its own: the section heading already says it.
    #[props(default)]
    section_title: Option<String>,
) -> Element {
    if rounds.is_empty() {
        return rsx! { p { class: "bracket-note", "No rounds have been drawn yet." } };
    }

    let by_round: Vec<Vec<&BracketMatch>> = rounds
        .iter()
        .map(|round| {
            let mut list: Vec<&BracketMatch> =
                matches.iter().filter(|m| m.round_id == round.id).collect();
            list.sort_by_key(|m| m.bracket_position);
            list
        })
        .collect();

    let mut heads = labels.clone();
    if champion {
        heads.push("Champion".to_string());
    }
    let cols = heads.len();
    // The hint is visible text on narrow screens only (CSS), and never part
    // of the scroller's name: the name says what the region holds.
    let hint = swipe_hint(&heads);

    // The champion is the winner of the last round when that round is one
    // match (the final).
    let champion_name: Option<String> = match by_round.last().map(Vec::as_slice) {
        Some([last]) => last
            .winner_id
            .as_ref()
            .and_then(|id| participant_names.get(id))
            .cloned(),
        _ => None,
    };
    let name_of = |id: &Option<String>| -> Option<String> {
        id.as_ref()
            .and_then(|id| participant_names.get(id))
            .cloned()
    };

    rsx! {
        if let Some(hint) = &hint {
            p { class: "bracket-hint",
                span { class: "swipe-arrow", aria_hidden: "true" }
                "{hint}"
            }
        }
        div { class: "bracket-clip",
            div {
                class: "bracket-scroller",
                role: "region",
                tabindex: "0",
                aria_label: "{name}",
                div { class: "bracket", style: "--bracket-cols: {cols};",
                    // The visible header row is decoration: each round
                    // carries its own heading first, so a screen reader meets
                    // a round's title right before that round's matches.
                    for text in heads.iter() {
                        p { key: "{text}", class: "bracket-head", aria_hidden: "true", "{text}" }
                    }
                    for (ri, round) in rounds.iter().enumerate() {
                        section {
                            key: "{round.id}",
                            class: "bracket-round",
                            if section_title.as_deref() != Some(heads[ri].as_str()) {
                                {round_heading(head, &heads[ri])}
                            }
                            for m in by_round[ri].iter() {
                                MatchCard {
                                    key: "{m.id}",
                                    team_a: name_of(&m.participant_a_id),
                                    team_b: name_of(&m.participant_b_id),
                                    score_a: m.score_a,
                                    score_b: m.score_b,
                                    winner: resolve_winner(m),
                                    state: match_state(&m.status),
                                    replay_codes: m.replay_codes.clone(),
                                }
                            }
                        }
                    }
                    if champion {
                        section { class: "bracket-round",
                            {round_heading(head, "Champion")}
                            div { class: "bracket-slot",
                                if let Some(winner) = &champion_name {
                                    p { class: "slot-team is-winner", "{winner}" }
                                    p { class: "slot-meta", "Won the final" }
                                } else {
                                    p { class: "slot-team is-open", "{TO_BE_DECIDED}" }
                                    p { class: "slot-meta", "After the final" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "bracket-fade", aria_hidden: "true" }
        }
    }
}

/// Rounds of the main (or winners) stage, in order.
pub(super) fn stage_rounds(rounds: &[BracketRound], stages: &[&str]) -> Vec<BracketRound> {
    let mut filtered: Vec<BracketRound> = rounds
        .iter()
        .filter(|r| stages.contains(&r.stage.as_str()))
        .cloned()
        .collect();
    filtered.sort_by_key(|r| r.round_number);
    filtered
}

/// A knockout: Semi-finals, Final, then the Champion column.
#[component]
pub fn BracketSingleElim(
    /// Tournament name, for the scroller's accessible name.
    name: String,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
) -> Element {
    let filtered = stage_rounds(&rounds, &["main", "winners"]);
    let total = filtered.len();
    let labels: Vec<String> = (0..total).map(|i| round_label(i, total)).collect();

    rsx! {
        BracketColumns {
            name: "{name} bracket",
            labels,
            rounds: filtered,
            matches,
            participant_names,
            champion: true,
        }
    }
}

#[cfg(test)]
pub(super) mod test_data {
    use super::*;

    pub fn round(id: &str, number: u32, stage: &str) -> BracketRound {
        BracketRound {
            id: id.to_string(),
            round_number: number,
            stage: stage.to_string(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn game(
        id: &str,
        round: &str,
        pos: u32,
        a: Option<&str>,
        b: Option<&str>,
        score: Option<(u32, u32)>,
        winner: Option<&str>,
        status: &str,
    ) -> BracketMatch {
        BracketMatch {
            id: id.to_string(),
            round_id: round.to_string(),
            bracket_position: pos,
            participant_a_id: a.map(str::to_string),
            participant_b_id: b.map(str::to_string),
            score_a: score.map(|s| s.0),
            score_b: score.map(|s| s.1),
            winner_id: winner.map(str::to_string),
            status: status.to_string(),
            replay_codes: Vec::new(),
        }
    }

    /// The four teams of the frame.
    pub fn names() -> HashMap<String, String> {
        [
            ("p:a", "Squad Alpha"),
            ("p:b", "Crew Beta"),
            ("p:c", "Squad Gamma"),
            ("p:d", "Crew Delta"),
        ]
        .into_iter()
        .map(|(id, name)| (id.to_string(), name.to_string()))
        .collect()
    }

    /// The frame's Test Cup: two semi-finals played, the final still open.
    pub fn test_cup() -> (Vec<BracketRound>, Vec<BracketMatch>) {
        (
            // Out of order on purpose: the view sorts by round number.
            vec![round("r:2", 2, "main"), round("r:1", 1, "main")],
            vec![
                game(
                    "m:3",
                    "r:2",
                    0,
                    Some("p:a"),
                    Some("p:c"),
                    None,
                    None,
                    "pending",
                ),
                game(
                    "m:2",
                    "r:1",
                    1,
                    Some("p:c"),
                    Some("p:d"),
                    Some((2, 1)),
                    Some("p:c"),
                    "completed",
                ),
                game(
                    "m:1",
                    "r:1",
                    0,
                    Some("p:a"),
                    Some("p:b"),
                    Some((2, 0)),
                    Some("p:a"),
                    "completed",
                ),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_data::*;
    use super::*;
    use crate::components::ui::test_support::render;

    fn labels(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn round_labels_follow_the_frame_wording() {
        assert_eq!(round_label(0, 1), "Final");
        assert_eq!(round_label(0, 2), "Semi-finals");
        assert_eq!(round_label(1, 2), "Final");
        assert_eq!(round_label(0, 3), "Quarter-finals");
        assert_eq!(round_label(0, 4), "Round 1");
        assert_eq!(round_label(1, 4), "Quarter-finals");
        assert_eq!(numbered_labels(2), ["Round 1", "Round 2"]);
    }

    #[test]
    fn hint_is_built_from_the_real_column_names() {
        assert_eq!(
            swipe_hint(&labels(&["Semi-finals", "Final", "Champion"])).as_deref(),
            Some("Swipe for Final and Champion")
        );
        assert_eq!(
            swipe_hint(&labels(&[
                "Quarter-finals",
                "Semi-finals",
                "Final",
                "Champion"
            ]))
            .as_deref(),
            Some("Swipe for Semi-finals, Final and Champion")
        );
        assert_eq!(
            swipe_hint(&labels(&[
                "Round 1",
                "Quarter-finals",
                "Semi-finals",
                "Final",
                "Champion"
            ]))
            .as_deref(),
            Some("Swipe for Quarter-finals through Champion")
        );
        // Two columns fit a phone: no hint.
        assert_eq!(swipe_hint(&labels(&["Final", "Champion"])), None);
        assert_eq!(swipe_hint(&[]), None);
    }

    fn cup() -> Element {
        let (rounds, matches) = test_cup();
        rsx! {
            BracketSingleElim {
                name: "Test Cup",
                rounds,
                matches,
                participant_names: names(),
            }
        }
    }

    #[test]
    fn bracket_matches_the_frame_structure() {
        let html = render(cup);
        // One hint line with the arrow, right above the scroller.
        assert!(
            html.starts_with(
                "<p class=\"bracket-hint\"><span class=\"swipe-arrow\" aria-hidden=\"true\"></span>\
                 Swipe for Final and Champion</p><div class=\"bracket-clip\">"
            ),
            "{html}"
        );
        assert_eq!(html.matches("bracket-hint").count(), 1, "{html}");
        // The scroller is focusable and named for what it holds; the swipe
        // hint is not part of the name.
        assert!(
            html.contains(
                "<div class=\"bracket-scroller\" role=\"region\" tabindex=\"0\" \
                 aria-label=\"Test Cup bracket\">"
            ),
            "{html}"
        );
        // Three labels in one header row, hidden from assistive tech, then
        // three rounds, each opening with its own heading.
        assert!(
            html.contains(
                "<div class=\"bracket\" style=\"--bracket-cols: 3;\">\
                 <p class=\"bracket-head\" aria-hidden=\"true\">Semi-finals</p>\
                 <p class=\"bracket-head\" aria-hidden=\"true\">Final</p>\
                 <p class=\"bracket-head\" aria-hidden=\"true\">Champion</p>\
                 <section class=\"bracket-round\"><h2 class=\"visually-hidden\">Semi-finals</h2>\
                 <div class=\"bracket-slot\">"
            ),
            "{html}"
        );
        assert_eq!(html.matches("<section class=\"bracket-round\"").count(), 3);
        // Reading order: each heading comes right before its round's
        // matches, never all the headings first.
        let semis = html.find("<h2 class=\"visually-hidden\">Semi-finals</h2>").unwrap();
        let final_head = html.find("<h2 class=\"visually-hidden\">Final</h2>").unwrap();
        let first_match = html.find("Won 2 to 0").unwrap();
        assert!(semis < first_match && first_match < final_head, "{html}");
        assert_eq!(html.matches("<h2").count(), 3, "{html}");
        assert_eq!(html.matches("class=\"bracket-slot\"").count(), 4, "{html}");
        // The fade is decoration, after the scroller inside the clip.
        assert!(
            html.ends_with("<div class=\"bracket-fade\" aria-hidden=\"true\"></div></div>"),
            "{html}"
        );
        // One tree: no second, phone only copy of the bracket.
        assert_eq!(html.matches("Crew Delta").count(), 1, "{html}");
    }

    #[test]
    fn slots_are_ordered_and_say_who_won() {
        let html = render(cup);
        let alpha = html.find("Won 2 to 0").expect("first semi-final");
        let gamma = html.find("Won 2 to 1").expect("second semi-final");
        assert!(alpha < gamma, "{html}");
        assert_eq!(html.matches(", winner").count(), 2, "{html}");
        assert_eq!(html.matches("slot-mark is-up").count(), 2, "{html}");
        // The final is not played and the champion is open.
        assert!(
            html.contains(
                "<p class=\"slot-team\">Squad Alpha</p><p class=\"slot-meta\">To be decided</p>\
                 <p class=\"slot-team\">Squad Gamma</p>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<section class=\"bracket-round\"><h2 class=\"visually-hidden\">Champion</h2>\
                 <div class=\"bracket-slot\"><p class=\"slot-team is-open\">To be decided</p>\
                 <p class=\"slot-meta\">After the final</p></div></section>"
            ),
            "{html}"
        );
        assert!(!html.contains("p:a"), "raw participant id leaked: {html}");
    }

    #[test]
    fn champion_is_named_once_the_final_has_a_winner() {
        fn app() -> Element {
            let (rounds, mut matches) = test_cup();
            matches[0] = game(
                "m:3",
                "r:2",
                0,
                Some("p:a"),
                Some("p:c"),
                Some((1, 3)),
                Some("p:c"),
                "completed",
            );
            rsx! {
                BracketSingleElim {
                    name: "Test Cup",
                    rounds,
                    matches,
                    participant_names: names(),
                }
            }
        }
        let html = render(app);
        assert!(
            html.contains(
                "<p class=\"slot-team is-winner\">Squad Gamma</p>\
                 <p class=\"slot-meta\">Won the final</p>"
            ),
            "{html}"
        );
        assert!(html.contains("Won 3 to 1"), "{html}");
        assert!(!html.contains("After the final"), "{html}");
    }

    #[test]
    fn no_rounds_is_a_plain_note() {
        fn app() -> Element {
            rsx! {
                BracketSingleElim {
                    name: "Test Cup",
                    rounds: Vec::new(),
                    matches: Vec::new(),
                    participant_names: HashMap::new(),
                }
            }
        }
        assert_eq!(
            render(app),
            "<p class=\"bracket-note\">No rounds have been drawn yet.</p>"
        );
    }
}
