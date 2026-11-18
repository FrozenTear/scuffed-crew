use dioxus::prelude::*;
use std::collections::HashMap;

use super::double_elim::BracketDoubleElim;
use super::round_robin::RoundRobinTable;
use super::single_elim::{
    BracketColumns, BracketMatch, BracketRound, BracketSingleElim, HeadLevel, numbered_labels,
    stage_rounds,
};
use super::swiss::{STANDINGS_TITLE_ID, StandingsLoad, SwissStandings};

/// The bracket of one tournament, by format. `name` is the tournament name:
/// it names the scrollers for assistive tech.
#[component]
pub fn BracketView(
    name: String,
    format: String,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
    participant_ids: Vec<String>,
    #[props(default)] swiss_standings: StandingsLoad,
    /// Fetches the Swiss standings again after a failure.
    #[props(default)]
    on_standings_retry: Option<EventHandler<()>>,
) -> Element {
    match format.as_str() {
        "single_elim" => rsx! {
            BracketSingleElim { name, rounds, matches, participant_names }
        },

        "double_elim" => rsx! {
            BracketDoubleElim { name, rounds, matches, participant_names }
        },

        "round_robin" => rsx! {
            section { class: "ui-stack",
                h2 { class: "ui-section-title", "Results" }
                RoundRobinTable { name, matches, participant_names, participant_ids }
            }
        },

        "swiss" => {
            // Swiss rounds are numbered: the last one is not a final.
            let swiss_rounds = stage_rounds(&rounds, &["main", "winners"]);
            let labels = numbered_labels(swiss_rounds.len());
            rsx! {
                section { class: "ui-stack", aria_labelledby: STANDINGS_TITLE_ID,
                    // Focus lands here when a Retry of the standings settles.
                    h2 {
                        id: STANDINGS_TITLE_ID,
                        class: "ui-section-title",
                        tabindex: "-1",
                        "Standings"
                    }
                    SwissStandings {
                        name: name.clone(),
                        standings: swiss_standings,
                        on_retry: on_standings_retry,
                    }
                }
                section { class: "ui-stack",
                    h2 { class: "ui-section-title", "Round pairings" }
                    BracketColumns {
                        name: "{name} round pairings",
                        labels,
                        rounds: swiss_rounds,
                        matches,
                        participant_names,
                        head: HeadLevel::H3,
                    }
                }
            }
        }

        _ => rsx! {
            p { class: "bracket-note", "This tournament uses a format this page cannot draw yet." }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::single_elim::test_data::*;
    use super::*;
    use crate::components::ui::test_support::render;

    fn swiss() -> Element {
        let (rounds, matches) = test_cup();
        rsx! {
            BracketView {
                name: "Test Cup",
                format: "swiss",
                rounds,
                matches,
                participant_names: names(),
                participant_ids: Vec::new(),
                swiss_standings: StandingsLoad::Ready(Vec::new()),
            }
        }
    }

    #[test]
    fn swiss_rounds_are_numbered_and_have_no_champion() {
        let html = render(swiss);
        // The Standings heading names its section and takes focus after a
        // Retry (QA 705 nit 1).
        assert!(
            html.contains(
                "<section class=\"ui-stack\" aria-labelledby=\"swiss-standings-title\"><h2 id=\"swiss-standings-title\" class=\"ui-section-title\" tabindex=\"-1\">Standings</h2>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<h2 class=\"ui-section-title\">Round pairings</h2>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<section class=\"bracket-round\"><h3 class=\"visually-hidden\">Round 1</h3>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<section class=\"bracket-round\"><h3 class=\"visually-hidden\">Round 2</h3>"
            ),
            "{html}"
        );
        for wrong in ["Final", "Semi-finals", "Champion"] {
            assert!(!html.contains(wrong), "{wrong}: {html}");
        }
        assert!(
            html.contains("aria-label=\"Test Cup round pairings\""),
            "{html}"
        );
    }

    #[test]
    fn unknown_format_says_so_in_plain_words() {
        fn app() -> Element {
            rsx! {
                BracketView {
                    name: "Test Cup",
                    format: "ladder",
                    rounds: Vec::new(),
                    matches: Vec::new(),
                    participant_names: HashMap::new(),
                    participant_ids: Vec::new(),
                    swiss_standings: StandingsLoad::Ready(Vec::new()),
                }
            }
        }
        assert_eq!(
            render(app),
            "<p class=\"bracket-note\">This tournament uses a format this page cannot draw yet.</p>"
        );
    }
}
