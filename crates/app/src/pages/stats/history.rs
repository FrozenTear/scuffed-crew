use dioxus::prelude::*;

use crate::hooks::ApiResource;

use super::{MatchPage, format_date, load_error_state};

fn outcome_class(outcome: &str) -> &'static str {
    match outcome.to_lowercase().as_str() {
        "win" => "outcome-win",
        "loss" => "outcome-loss",
        _ => "outcome-draw",
    }
}

pub(super) fn history_tab(
    matches: ApiResource<MatchPage>,
    mut page_cursor: Signal<Option<String>>,
    mut cursor_history: Signal<Vec<Option<String>>>,
) -> Element {
    let err = matches.error.read().clone();
    let data = matches.data.read();
    let page = data.as_ref().and_then(|d| d.as_ref());
    match page {
        None if err.is_some() => load_error_state("match history", matches.refresh),
        None => rsx! { p { class: "loading-state", "Loading match history..." } },
        Some(page) if page.data.is_empty() => rsx! {
            p { class: "empty-state", "No matches recorded yet." }
        },
        Some(page) => {
            let has_next = page.next_cursor.is_some();
            let next_c = page.next_cursor.clone();
            let can_prev = cursor_history().len() > 1;
            rsx! {
                div { class: "match-cards",
                    for m in page.data.iter() {
                        {
                            let oc = outcome_class(&m.outcome);
                            let date = format_date(&m.played_at);
                            rsx! {
                                div { class: "match-card", key: "{m.id}",
                                    div { class: "match-outcome {oc}", "{m.outcome}" }
                                    div {
                                        div { class: "match-hero", "{m.hero}" }
                                        div { class: "match-map", "{m.map_name} · {m.role}" }
                                    }
                                    div { class: "match-scoreline",
                                        "{m.elims}E / {m.deaths}D / {m.assists}A · {m.damage} dmg · {m.healing} heal"
                                    }
                                    div { class: "match-date", "{date}" }
                                }
                            }
                        }
                    }
                }
                div { class: "stats-pagination",
                    button {
                        disabled: !can_prev,
                        onclick: move |_| {
                            let mut hist = cursor_history();
                            if hist.len() > 1 {
                                hist.pop();
                                let prev = hist.last().cloned().flatten();
                                cursor_history.set(hist);
                                page_cursor.set(prev);
                            }
                        },
                        "Previous"
                    }
                    button {
                        disabled: !has_next,
                        onclick: move |_| {
                            if let Some(nc) = &next_c {
                                let mut hist = cursor_history();
                                hist.push(Some(nc.clone()));
                                cursor_history.set(hist);
                                page_cursor.set(Some(nc.clone()));
                            }
                        },
                        "Next"
                    }
                }
            }
        }
    }
}
