//! Round robin: one row per team, one numbered column per opponent, then the
//! win, loss and draw counts. Each result cell says Won, Lost or Draw in a
//! word; colour only backs the word up.

use dioxus::prelude::*;
use std::collections::HashMap;

use super::single_elim::BracketMatch;
use super::swiss::ShortHead;

/// One team's view of a match against another team.
#[derive(Debug, Clone, PartialEq)]
struct CellResult {
    my_score: Option<u32>,
    their_score: Option<u32>,
    winner_id: Option<String>,
    replay_codes: Vec<String>,
}

/// How a played or unplayed pairing reads from the row team's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Won,
    Lost,
    Draw,
    ToPlay,
}

impl Outcome {
    fn word(self) -> &'static str {
        match self {
            Outcome::Won => "Won",
            Outcome::Lost => "Lost",
            Outcome::Draw => "Draw",
            Outcome::ToPlay => "To play",
        }
    }

    fn class(self) -> &'static str {
        match self {
            Outcome::Won => "rr-word is-win",
            Outcome::Lost => "rr-word is-loss",
            Outcome::Draw => "rr-word",
            Outcome::ToPlay => "rr-word is-muted",
        }
    }
}

fn outcome(cell: Option<&CellResult>, row_id: &str) -> Outcome {
    match cell {
        Some(c) if c.winner_id.as_deref() == Some(row_id) => Outcome::Won,
        Some(c) if c.winner_id.is_some() => Outcome::Lost,
        Some(c) if c.my_score.is_some() => Outcome::Draw,
        _ => Outcome::ToPlay,
    }
}

/// "2 to 0" from the row team's side, when both scores are known.
fn score_text(cell: Option<&CellResult>) -> Option<String> {
    let cell = cell?;
    Some(format!("{} to {}", cell.my_score?, cell.their_score?))
}

#[derive(Debug, Clone, PartialEq)]
struct Standing {
    id: String,
    wins: u32,
    losses: u32,
    draws: u32,
    diff: i32,
}

fn results_grid(matches: &[BracketMatch]) -> HashMap<(String, String), CellResult> {
    let mut results = HashMap::new();
    for m in matches {
        if let (Some(a), Some(b)) = (&m.participant_a_id, &m.participant_b_id) {
            results.insert(
                (a.clone(), b.clone()),
                CellResult {
                    my_score: m.score_a,
                    their_score: m.score_b,
                    winner_id: m.winner_id.clone(),
                    replay_codes: m.replay_codes.clone(),
                },
            );
            results.insert(
                (b.clone(), a.clone()),
                CellResult {
                    my_score: m.score_b,
                    their_score: m.score_a,
                    winner_id: m.winner_id.clone(),
                    replay_codes: m.replay_codes.clone(),
                },
            );
        }
    }
    results
}

/// Standings by wins, then by score difference.
fn standings(
    participant_ids: &[String],
    results: &HashMap<(String, String), CellResult>,
) -> Vec<Standing> {
    let mut rows: Vec<Standing> = participant_ids
        .iter()
        .map(|pid| {
            let mut row = Standing {
                id: pid.clone(),
                wins: 0,
                losses: 0,
                draws: 0,
                diff: 0,
            };
            for other in participant_ids.iter().filter(|other| *other != pid) {
                let cell = results.get(&(pid.clone(), other.clone()));
                if let Some(c) = cell {
                    row.diff += c.my_score.unwrap_or(0) as i32 - c.their_score.unwrap_or(0) as i32;
                }
                match outcome(cell, pid) {
                    Outcome::Won => row.wins += 1,
                    Outcome::Lost => row.losses += 1,
                    Outcome::Draw => row.draws += 1,
                    Outcome::ToPlay => {}
                }
            }
            row
        })
        .collect();
    rows.sort_by(|a, b| b.wins.cmp(&a.wins).then_with(|| b.diff.cmp(&a.diff)));
    rows
}

#[component]
pub fn RoundRobinTable(
    /// Tournament name, for the scroller's accessible name.
    name: String,
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
    participant_ids: Vec<String>,
) -> Element {
    let results = results_grid(&matches);
    let rows = standings(&participant_ids, &results);
    let team_name = |id: &str| -> String {
        participant_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| super::match_card::TO_BE_DECIDED.to_string())
    };

    rsx! {
        p { class: "bracket-hint",
            span { class: "swipe-arrow", aria_hidden: "true" }
            "Swipe for every opponent. Numbered columns are the teams by rank."
        }
        div {
            class: "bracket-table-scroll",
            role: "region",
            tabindex: "0",
            aria_label: "{name} results grid",
            table { class: "bracket-table",
                thead {
                    tr {
                        th { scope: "col", class: "is-num",
                            span { aria_hidden: "true", "#" }
                            span { class: "visually-hidden", "Rank" }
                        }
                        th { scope: "col", "Team" }
                        for (i, col) in rows.iter().enumerate() {
                            th { key: "{col.id}", scope: "col",
                                span { aria_hidden: "true", "{i + 1}" }
                                span { class: "visually-hidden", "Against {team_name(&col.id)}" }
                            }
                        }
                        ShortHead { short: "W", full: "Wins" }
                        ShortHead { short: "L", full: "Losses" }
                        ShortHead { short: "D", full: "Draws" }
                    }
                }
                tbody {
                    for (row_idx, row) in rows.iter().enumerate() {
                        tr { key: "{row.id}",
                            td { class: "is-num is-muted", "{row_idx + 1}" }
                            th { scope: "row", "{team_name(&row.id)}" }
                            for col in rows.iter() {
                                if row.id == col.id {
                                    td { class: "rr-self",
                                        span { class: "visually-hidden", "Same team" }
                                    }
                                } else {
                                    {
                                        let cell = results.get(&(row.id.clone(), col.id.clone()));
                                        let result = outcome(cell, &row.id);
                                        let score = score_text(cell);
                                        let codes = cell
                                            .map(|c| c.replay_codes.join(", "))
                                            .filter(|codes| !codes.is_empty());
                                        rsx! {
                                            td {
                                                span { class: result.class(), "{result.word()}" }
                                                if let Some(score) = score {
                                                    span { class: "rr-detail", "{score}" }
                                                }
                                                if let Some(codes) = codes {
                                                    span { class: "rr-detail", "Replay codes: {codes}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            td { class: "is-num", "{row.wins}" }
                            td { class: "is-num", "{row.losses}" }
                            td { class: "is-num", "{row.draws}" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::single_elim::test_data::*;
    use super::*;
    use crate::components::ui::test_support::render;

    fn group() -> Vec<BracketMatch> {
        let mut played = game(
            "m:1",
            "r:1",
            0,
            Some("p:a"),
            Some("p:b"),
            Some((2, 0)),
            Some("p:a"),
            "completed",
        );
        played.replay_codes = vec!["A1B2C3".to_string()];
        vec![
            played,
            game(
                "m:2",
                "r:1",
                1,
                Some("p:b"),
                Some("p:c"),
                Some((1, 1)),
                None,
                "completed",
            ),
            game(
                "m:3",
                "r:1",
                2,
                Some("p:a"),
                Some("p:c"),
                None,
                None,
                "pending",
            ),
        ]
    }

    fn ids() -> Vec<String> {
        ["p:c", "p:b", "p:a"].map(String::from).to_vec()
    }

    #[test]
    fn standings_count_wins_losses_and_draws() {
        let rows = standings(&ids(), &results_grid(&group()));
        let line: Vec<(&str, u32, u32, u32)> = rows
            .iter()
            .map(|r| (r.id.as_str(), r.wins, r.losses, r.draws))
            .collect();
        assert_eq!(line, [("p:a", 1, 0, 0), ("p:c", 0, 0, 1), ("p:b", 0, 1, 1)]);
    }

    fn app() -> Element {
        rsx! {
            RoundRobinTable {
                name: "Test Cup",
                matches: group(),
                participant_names: names(),
                participant_ids: ids(),
            }
        }
    }

    #[test]
    fn every_result_cell_carries_a_word() {
        let html = render(app);
        assert!(
            html.contains(
                "<span class=\"rr-word is-win\">Won</span><span class=\"rr-detail\">2 to 0</span>\
                 <span class=\"rr-detail\">Replay codes: A1B2C3</span>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<span class=\"rr-word is-loss\">Lost</span><span class=\"rr-detail\">0 to 2</span>"
            ),
            "{html}"
        );
        assert_eq!(html.matches(">Draw</span>").count(), 2, "{html}");
        assert_eq!(html.matches(">To play</span>").count(), 2, "{html}");
        assert_eq!(html.matches(">Same team</span>").count(), 3, "{html}");
        // No tooltip only content.
        assert!(!html.contains("title="), "{html}");
    }

    #[test]
    fn table_is_in_a_named_focusable_scroller_with_real_headers() {
        let html = render(app);
        assert!(
            html.contains(
                "<div class=\"bracket-table-scroll\" role=\"region\" tabindex=\"0\" \
                 aria-label=\"Test Cup results grid\">"
            ),
            "{html}"
        );
        assert!(html.contains("<p class=\"bracket-hint\">"), "{html}");
        assert!(
            html.contains("<th scope=\"row\">Squad Alpha</th>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"visually-hidden\">Against Squad Alpha</span>"),
            "{html}"
        );
        assert!(!html.contains("p:a"), "raw participant id leaked: {html}");
    }
}
