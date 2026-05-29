use dioxus::prelude::*;
use std::collections::HashMap;

use super::single_elim::BracketMatch;

/// Per-cell result: (my_score, their_score, winner_id, replay_codes).
type CellResult = (Option<u32>, Option<u32>, Option<String>, Vec<String>);

#[component]
pub fn RoundRobinTable(
    matches: Vec<BracketMatch>,
    participant_names: HashMap<String, String>,
    participant_ids: Vec<String>,
) -> Element {
    // Build results grid: (row_id, col_id) -> (my_score, their_score, winner, codes)
    let mut results: HashMap<(String, String), CellResult> = HashMap::new();
    for m in &matches {
        if let (Some(a), Some(b)) = (&m.participant_a_id, &m.participant_b_id) {
            results.insert(
                (a.clone(), b.clone()),
                (
                    m.score_a,
                    m.score_b,
                    m.winner_id.clone(),
                    m.replay_codes.clone(),
                ),
            );
            results.insert(
                (b.clone(), a.clone()),
                (
                    m.score_b,
                    m.score_a,
                    m.winner_id.clone(),
                    m.replay_codes.clone(),
                ),
            );
        }
    }

    // Compute standings
    let mut standings: Vec<(String, u32, u32, u32, i32)> = participant_ids
        .iter()
        .map(|pid| {
            let mut wins = 0u32;
            let mut losses = 0u32;
            let mut draws = 0u32;
            let mut diff = 0i32;
            for other in &participant_ids {
                if other == pid {
                    continue;
                }
                if let Some((my_score, their_score, winner, _)) =
                    results.get(&(pid.clone(), other.clone()))
                {
                    diff += my_score.unwrap_or(0) as i32 - their_score.unwrap_or(0) as i32;
                    if winner.as_ref() == Some(pid) {
                        wins += 1;
                    } else if winner.is_some() {
                        losses += 1;
                    } else if my_score.is_some() {
                        draws += 1;
                    }
                }
            }
            (pid.clone(), wins, losses, draws, diff)
        })
        .collect();
    standings.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.4.cmp(&a.4)));

    let sorted_ids: Vec<String> = standings.iter().map(|s| s.0.clone()).collect();

    rsx! {
        div { class: "rr-table-container",
            table { class: "rr-table",
                thead {
                    tr {
                        th { "#" }
                        th { "Team" }
                        for (i, _) in sorted_ids.iter().enumerate() {
                            th { "{i + 1}" }
                        }
                        th { "W" }
                        th { "L" }
                        th { "D" }
                    }
                }
                tbody {
                    for (row_idx, row_id) in sorted_ids.iter().enumerate() {
                        {
                            let name = participant_names.get(row_id).cloned().unwrap_or_else(|| "TBD".to_string());
                            let stats = standings.iter().find(|s| &s.0 == row_id).cloned();
                            let (wins, losses, draws) = stats.map(|s| (s.1, s.2, s.3)).unwrap_or((0, 0, 0));

                            rsx! {
                                tr {
                                    td { "{row_idx + 1}" }
                                    td { class: "rr-name", "{name}" }
                                    for col_id in sorted_ids.iter() {
                                        {
                                            if row_id == col_id {
                                                rsx! { td { class: "rr-self", "\u{2014}" } }
                                            } else if let Some((my_score, their_score, winner, codes)) = results.get(&(row_id.clone(), col_id.clone())) {
                                                let class = if winner.as_ref() == Some(row_id) {
                                                    "rr-win"
                                                } else if winner.is_some() {
                                                    "rr-loss"
                                                } else {
                                                    "rr-pending"
                                                };
                                                let text = match (my_score, their_score) {
                                                    (Some(a), Some(b)) => format!("{a}-{b}"),
                                                    _ => "\u{2014}".to_string(),
                                                };
                                                if codes.is_empty() {
                                                    rsx! { td { class: "{class}", "{text}" } }
                                                } else {
                                                    let tooltip = codes.join(", ");
                                                    rsx! {
                                                        td { class: "{class}", title: "{tooltip}",
                                                            span { class: "rr-score-with-codes", "{text}" }
                                                        }
                                                    }
                                                }
                                            } else {
                                                rsx! { td { class: "rr-pending", "\u{2014}" } }
                                            }
                                        }
                                    }
                                    td { "{wins}" }
                                    td { "{losses}" }
                                    td { "{draws}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
