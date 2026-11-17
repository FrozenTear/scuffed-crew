use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::LoadError;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SwissStanding {
    pub participant_id: String,
    pub participant_name: String,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    pub game_wins: u32,
    pub game_losses: u32,
    pub buchholz: f64,
    pub rank: u32,
}

/// The standings fetch. Only a finished fetch with no rows means no round
/// has been played: loading and failing say so in their own words.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum StandingsLoad {
    #[default]
    Loading,
    Failed,
    /// Failed, and Retry was pressed: the card stays, its button reads
    /// "Retrying..." and keeps focus until the new answer lands.
    Retrying,
    Ready(Vec<SwissStanding>),
}

/// A short column head with its full name for assistive tech.
#[component]
pub(super) fn ShortHead(short: String, full: String) -> Element {
    rsx! {
        th { scope: "col", class: "is-num",
            span { aria_hidden: "true", "{short}" }
            span { class: "visually-hidden", "{full}" }
        }
    }
}

/// Swiss standings. Eight columns do not fit a phone, so the table sits in a
/// scroller of its own: focusable, named, and labelled by the title above it.
#[component]
pub fn SwissStandings(
    /// Tournament name, for the scroller's accessible name.
    name: String,
    standings: StandingsLoad,
    /// Fetches the standings again after a failure.
    #[props(default)]
    on_retry: Option<EventHandler<()>>,
) -> Element {
    let standings = match standings {
        StandingsLoad::Loading => {
            return rsx! { p { class: "bracket-note", "Loading standings..." } };
        }
        StandingsLoad::Failed | StandingsLoad::Retrying => {
            let retrying = standings == StandingsLoad::Retrying;
            return rsx! {
                LoadError { message: "Couldn't load the standings.", on_retry, retrying }
            };
        }
        StandingsLoad::Ready(rows) if rows.is_empty() => {
            return rsx! {
                p { class: "bracket-note", "Standings show up here once the first round is played." }
            };
        }
        StandingsLoad::Ready(rows) => rows,
    };

    rsx! {
        p { class: "bracket-hint",
            span { class: "swipe-arrow", aria_hidden: "true" }
            "Swipe for game wins and Buchholz"
        }
        div {
            class: "bracket-table-scroll",
            role: "region",
            tabindex: "0",
            aria_label: "{name} standings",
            table { class: "bracket-table",
                thead {
                    tr {
                        th { scope: "col", class: "is-num",
                            span { aria_hidden: "true", "#" }
                            span { class: "visually-hidden", "Rank" }
                        }
                        th { scope: "col", "Team" }
                        ShortHead { short: "W", full: "Wins" }
                        ShortHead { short: "L", full: "Losses" }
                        ShortHead { short: "D", full: "Draws" }
                        ShortHead { short: "GW", full: "Game wins" }
                        ShortHead { short: "GL", full: "Game losses" }
                        ShortHead { short: "Buch.", full: "Buchholz score" }
                    }
                }
                tbody {
                    for s in standings.iter() {
                        tr { key: "{s.participant_id}",
                            td { class: "is-num is-muted", "{s.rank}" }
                            th { scope: "row", "{s.participant_name}" }
                            td { class: "is-num", "{s.wins}" }
                            td { class: "is-num", "{s.losses}" }
                            td { class: "is-num", "{s.draws}" }
                            td { class: "is-num", "{s.game_wins}" }
                            td { class: "is-num", "{s.game_losses}" }
                            td { class: "is-num", "{s.buchholz:.1}" }
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

    fn standing(n: u32) -> SwissStanding {
        SwissStanding {
            participant_id: format!("p:{n}"),
            participant_name: format!("Squad {n:02}"),
            wins: 3 - n,
            losses: n,
            draws: 0,
            game_wins: 6 - n,
            game_losses: n + 1,
            buchholz: 4.5,
            rank: n,
        }
    }

    #[test]
    fn standings_sit_in_a_named_focusable_scroller() {
        fn app() -> Element {
            rsx! {
                SwissStandings {
                    name: "Test Cup",
                    standings: StandingsLoad::Ready(vec![standing(1), standing(2)]),
                }
            }
        }
        let html = render(app);
        assert!(
            html.contains(
                "<div class=\"bracket-table-scroll\" role=\"region\" tabindex=\"0\" \
                 aria-label=\"Test Cup standings\">"
            ),
            "{html}"
        );
        assert!(html.contains("<th scope=\"row\">Squad 01</th>"), "{html}");
        assert!(
            html.contains("<span aria-hidden=\"true\">GW</span><span class=\"visually-hidden\">Game wins</span>"),
            "{html}"
        );
        assert_eq!(html.matches("scope=\"col\"").count(), 8, "{html}");
        assert!(html.contains(">4.5</td>"), "{html}");
        assert!(!html.contains("p:1"), "raw id leaked: {html}");
    }

    #[test]
    fn no_standings_is_a_note_not_an_empty_table() {
        fn app() -> Element {
            rsx! { SwissStandings { name: "Test Cup", standings: StandingsLoad::Ready(Vec::new()) } }
        }
        let html = render(app);
        assert!(html.contains("once the first round is played"), "{html}");
        assert!(!html.contains("<table"), "{html}");
    }

    #[test]
    fn loading_and_failed_standings_do_not_claim_no_round_was_played() {
        fn loading() -> Element {
            rsx! { SwissStandings { name: "Test Cup", standings: StandingsLoad::Loading } }
        }
        let html = render(loading);
        assert!(!html.contains("first round"), "{html}");
        assert!(html.contains("Loading standings..."), "{html}");
        fn failed() -> Element {
            rsx! {
                SwissStandings {
                    name: "Test Cup",
                    standings: StandingsLoad::Failed,
                    on_retry: move |_| {},
                }
            }
        }
        let html = render(failed);
        assert!(!html.contains("first round"), "{html}");
        assert!(html.contains("Couldn&#39;t load the standings."), "{html}");
        assert!(html.contains(">Retry</button>"), "{html}");
        assert!(!html.contains("<table"), "{html}");
        // Retry gives feedback: the same card, busy, with "Retrying...".
        fn retrying() -> Element {
            rsx! {
                SwissStandings {
                    name: "Test Cup",
                    standings: StandingsLoad::Retrying,
                    on_retry: move |_| {},
                }
            }
        }
        let html = render(retrying);
        assert!(html.contains("Couldn&#39;t load the standings."), "{html}");
        assert!(html.contains(">Retrying...</button>"), "{html}");
        assert!(html.contains(r#"aria-busy="true""#), "{html}");
        assert!(html.contains(r#"aria-disabled="true""#), "{html}");
        // The page maps a pending restart after an error to Retrying.
        let page = include_str!("../../pages/tournament.rs");
        let page = &page[..page.find("#[cfg(test)]").unwrap()];
        assert!(
            page.contains("UseResourceState::Pending => StandingsLoad::Retrying"),
            "tournament.rs"
        );
    }
}
