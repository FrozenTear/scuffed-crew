use dioxus::prelude::*;

use crate::hooks::ApiResource;

use super::overview::hero_to_role;
use super::{MatchPage, format_date, load_error_state};

/// The tracker stores outcomes as `victory` / `defeat` / `draw` (see the
/// upload filter in site-server routes/stats.rs); `win` / `loss` are accepted
/// as aliases. DEFEAT must land on the serious/danger class — before this
/// matched only `win`/`loss`, every card fell through to the draw class and
/// DEFEAT rendered gold.
fn outcome_class(outcome: &str) -> &'static str {
    match outcome.to_lowercase().as_str() {
        "victory" | "win" => "outcome-win",
        "defeat" | "loss" => "outcome-loss",
        _ => "outcome-draw",
    }
}

pub(super) fn history_tab(
    matches: ApiResource<MatchPage>,
    mut page_cursor: Signal<Option<String>>,
    mut cursor_history: Signal<Vec<Option<String>>>,
    mut outcome_filter: Signal<&'static str>,
    mut role_filter: Signal<&'static str>,
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
            let of = outcome_filter();
            let rf = role_filter();

            let rows: Vec<_> = page
                .data
                .iter()
                .filter(|m| {
                    let o_ok = match of {
                        "all" => true,
                        "win" => {
                            m.outcome.eq_ignore_ascii_case("win")
                                || m.outcome.eq_ignore_ascii_case("victory")
                        }
                        "loss" => {
                            m.outcome.eq_ignore_ascii_case("loss")
                                || m.outcome.eq_ignore_ascii_case("defeat")
                        }
                        "draw" => {
                            !m.outcome.eq_ignore_ascii_case("win")
                                && !m.outcome.eq_ignore_ascii_case("victory")
                                && !m.outcome.eq_ignore_ascii_case("loss")
                                && !m.outcome.eq_ignore_ascii_case("defeat")
                        }
                        _ => m.outcome.eq_ignore_ascii_case(of),
                    };
                    let r_ok = rf == "all"
                        || m.role.eq_ignore_ascii_case(rf)
                        || hero_to_role(&m.hero).eq_ignore_ascii_case(rf);
                    o_ok && r_ok
                })
                .collect();

            rsx! {
                div { class: "stats-filters",
                    div { class: "filter-group",
                        span { class: "filter-label", "Outcome" }
                        {[["all", "All"], ["win", "Win"], ["loss", "Loss"], ["draw", "Draw"]].iter().map(|&[k, label]| {
                            let active = of == k;
                            rsx! {
                                button {
                                    key: "{k}",
                                    class: if active { "filter-chip active" } else { "filter-chip" },
                                    onclick: move |_| outcome_filter.set(k),
                                    "{label}"
                                }
                            }
                        })}
                    }
                    div { class: "filter-group",
                        span { class: "filter-label", "Role" }
                        {[["all", "All"], ["Tank", "Tank"], ["Damage", "Damage"], ["Support", "Support"]].iter().map(|&[k, label]| {
                            let active = rf.eq_ignore_ascii_case(k);
                            rsx! {
                                button {
                                    key: "{k}",
                                    class: if active { "filter-chip active" } else { "filter-chip" },
                                    onclick: move |_| role_filter.set(k),
                                    "{label}"
                                }
                            }
                        })}
                    }
                    p { class: "filter-hint", "Filters apply to loaded rows (this page)." }
                }

                if rows.is_empty() {
                    p { class: "empty-state", "No matches match these filters on this page." }
                } else {
                    div { class: "match-cards",
                        for m in rows.iter() {
                            {
                                let oc = outcome_class(&m.outcome);
                                let date = format_date(&m.played_at);
                                let abs = m.played_at.format("%Y-%m-%d %H:%M UTC").to_string();
                                let map_label = if m.map_name.trim().is_empty() {
                                    "Unknown map".to_string()
                                } else {
                                    m.map_name.clone()
                                };
                                rsx! {
                                    div { class: "match-card", key: "{m.id}",
                                        div { class: "match-outcome {oc}", "{m.outcome}" }
                                        div {
                                            div { class: "match-hero", "{m.hero}" }
                                            div { class: "match-map", "{map_label} · {m.role}" }
                                        }
                                        div { class: "match-scoreline",
                                            "{m.elims}E / {m.deaths}D / {m.assists}A · {m.damage} dmg · {m.healing} heal"
                                        }
                                        div { class: "match-date", title: "{abs}", "{date}" }
                                    }
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
