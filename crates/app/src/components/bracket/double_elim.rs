//! Double elimination: a winners bracket, a losers bracket and the grand
//! final, each a titled block with its own scroller.

use dioxus::prelude::*;
use std::collections::HashMap;

use super::single_elim::{
    BracketColumns, BracketMatch, BracketRound, HeadLevel, numbered_labels, stage_rounds,
};

/// One stage of the bracket under its own H2. Round labels are H3.
#[component]
fn BracketSection(
    title: String,
    /// What the scroller holds: "Test Cup winners bracket".
    name: String,
    /// One label per round.
    labels: Vec<String>,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    names: HashMap<String, String>,
) -> Element {
    rsx! {
        section { class: "ui-stack",
            h2 { class: "ui-section-title", "{title}" }
            BracketColumns {
                section_title: title.clone(),
                name,
                labels,
                rounds,
                matches,
                participant_names: names,
                head: HeadLevel::H3,
            }
        }
    }
}

#[component]
pub fn BracketDoubleElim(
    /// Tournament name, for the scrollers' accessible names.
    name: String,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
) -> Element {
    let winners = stage_rounds(&rounds, &["winners"]);
    let losers = stage_rounds(&rounds, &["losers"]);
    let gf = stage_rounds(&rounds, &["grand_final"]);
    let gf_labels: Vec<String> = match gf.len() {
        1 => vec!["Grand final".to_string()],
        n => (1..=n).map(|i| format!("Grand final, game {i}")).collect(),
    };

    rsx! {
        BracketSection {
            title: "Winners bracket",
            name: "{name} winners bracket",
            labels: numbered_labels(winners.len()),
            rounds: winners,
            matches: matches.clone(),
            names: participant_names.clone(),
        }
        BracketSection {
            title: "Losers bracket",
            name: "{name} losers bracket",
            labels: numbered_labels(losers.len()),
            rounds: losers,
            matches: matches.clone(),
            names: participant_names.clone(),
        }
        if !gf.is_empty() {
            BracketSection {
                title: "Grand final",
                name: "{name} grand final",
                labels: gf_labels,
                rounds: gf,
                matches,
                names: participant_names,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::single_elim::test_data::*;
    use super::*;
    use crate::components::ui::test_support::render;

    fn app() -> Element {
        let rounds = vec![
            round("w:1", 1, "winners"),
            round("w:2", 2, "winners"),
            round("l:1", 1, "losers"),
            round("g:1", 1, "grand_final"),
        ];
        let matches = vec![
            game(
                "m:1",
                "w:1",
                0,
                Some("p:a"),
                Some("p:b"),
                Some((0, 2)),
                Some("p:b"),
                "completed",
            ),
            game("m:2", "w:2", 0, Some("p:b"), None, None, None, "pending"),
            game(
                "m:3",
                "l:1",
                0,
                Some("p:a"),
                Some("p:d"),
                None,
                None,
                "in_progress",
            ),
            game("m:4", "g:1", 0, None, None, None, None, "pending"),
        ];
        rsx! {
            BracketDoubleElim {
                name: "Test Cup",
                rounds,
                matches,
                participant_names: names(),
            }
        }
    }

    #[test]
    fn each_stage_is_a_titled_block_with_its_own_named_scroller() {
        let html = render(app);
        for title in ["Winners bracket", "Losers bracket", "Grand final"] {
            assert!(
                html.contains(&format!("<h2 class=\"ui-section-title\">{title}</h2>")),
                "{title}: {html}"
            );
        }
        for name in [
            "Test Cup winners bracket",
            "Test Cup losers bracket",
            "Test Cup grand final",
        ] {
            assert!(
                html.contains(&format!("aria-label=\"{name}\"")),
                "{name}: {html}"
            );
        }
        assert_eq!(html.matches("role=\"region\" tabindex=\"0\"").count(), 3);
        // Round headings sit under the stage title, one level down, each at
        // the start of its own round. The header row is decoration.
        assert!(
            html.contains("<section class=\"bracket-round\"><h3 class=\"visually-hidden\">Round 1</h3>"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"bracket-head\" aria-hidden=\"true\">Grand final</p>"),
            "{html}"
        );
        assert!(!html.contains("<h2 class=\"visually-hidden\""), "{html}");
        // The one grand final round is not a second "Grand final" heading
        // under the H2 of the same name.
        assert_eq!(html.matches(">Grand final</h").count(), 1, "{html}");
        // No Champion column outside the knockout view.
        assert!(!html.contains("Champion"), "{html}");
        // Two columns at most here: no swipe hint.
        assert!(!html.contains("bracket-hint"), "{html}");
    }

    #[test]
    fn second_side_winner_and_live_match_are_worded() {
        let html = render(app);
        assert!(
            html.contains(
                "<p class=\"slot-team\">Squad Alpha</p>\
                 <p class=\"slot-meta\"><span class=\"slot-mark is-down\" aria-hidden=\"true\"></span>\
                 <span class=\"visually-hidden\">Crew Beta: </span>Won 2 to 0</p>\
                 <p class=\"slot-team is-winner\">Crew Beta<span class=\"visually-hidden\">, winner</span></p>"
            ),
            "{html}"
        );
        assert!(html.contains("class=\"bracket-slot is-live\""), "{html}");
        assert!(
            html.contains("<p class=\"slot-meta\">In progress</p>"),
            "{html}"
        );
    }
}
