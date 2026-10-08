//! Public leaderboards page (#6).

use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use crate::components::ui::{Card, HeroSelect, Pill, PillTone, SeasonSelect, use_stats_season};
use crate::routes::Route;
use crate::util::encode_query;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct LeaderboardRow {
    member_id: String,
    display_name: String,
    games: u32,
    winrate: f32,
    kd: f64,
}

const PAGE_CSS: &str = r#"
    .lb-page {
        padding: 3rem 2rem;
        max-width: 800px;
        margin: 0 auto;
    }
    .lb-page h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        margin: 0 0 0.5rem;
        color: var(--text);
    }
    .lb-sub {
        color: var(--text-3);
        font-size: 0.9rem;
        margin-bottom: 1.5rem;
    }
    .lb-tabs {
        display: flex;
        gap: 0.5rem;
        margin-bottom: 1.25rem;
        flex-wrap: wrap;
    }
    .lb-tab {
        background: var(--surface-2);
        border: 1px solid var(--border);
        color: var(--text-2);
        padding: 0.4rem 0.85rem;
        border-radius: 6px;
        cursor: pointer;
        font-size: 0.85rem;
        font-weight: 600;
    }
    .lb-tab.active {
        background: var(--accent);
        border-color: var(--accent);
        color: var(--accent-fg);
    }
    .lb-status {
        color: var(--text-3);
        text-align: center;
        padding: 2rem 0;
    }
    .lb-table {
        width: 100%;
        border-collapse: collapse;
        font-size: 0.9rem;
    }
    .lb-table th {
        text-align: left;
        font-family: var(--font-mono);
        font-size: 0.68rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--text-3);
        padding: 0.5rem 0.6rem;
        border-bottom: 1px solid var(--border);
    }
    .lb-table td {
        padding: 0.65rem 0.6rem;
        border-bottom: 1px solid var(--border);
        color: var(--text);
    }
    .lb-table a {
        color: var(--accent);
        font-weight: 600;
        text-decoration: none;
    }
    .lb-table a:hover { text-decoration: underline; }
    .lb-rank {
        font-family: var(--font-mono);
        color: var(--text-3);
        width: 2.5rem;
    }
    .lb-num {
        font-family: var(--font-mono);
        font-variant-numeric: tabular-nums;
    }
    .lb-filters { display: flex; gap: 1rem; flex-wrap: wrap; align-items: flex-end; }
    .lb-hero {
        max-width: 280px;
        margin-bottom: 1.25rem;
    }
"#;

/// `Hold` is the skipped request while a saved season is unresolved. It
/// renders as Loading. It is not a finished failure.
#[derive(Debug, PartialEq)]
enum LeaderboardLoad {
    Hold,
    Failed,
    Rows(Vec<LeaderboardRow>),
}

/// `Some(Hold)` when the season filter has not resolved yet. A non-empty
/// path is fetched by the caller.
fn leaderboard_hold(path: &str) -> Option<LeaderboardLoad> {
    if path.is_empty() {
        Some(LeaderboardLoad::Hold)
    } else {
        None
    }
}

/// Status line for the leaderboard card. `None` means render the table.
fn leaderboard_message(
    load: Option<&LeaderboardLoad>,
    hero_filtered: bool,
) -> Option<&'static str> {
    match load {
        None | Some(LeaderboardLoad::Hold) => Some("Loading..."),
        Some(LeaderboardLoad::Failed) => Some("Couldn't load leaderboards."),
        Some(LeaderboardLoad::Rows(list)) if list.is_empty() && hero_filtered => {
            Some("No ranked matches on this hero yet.")
        }
        Some(LeaderboardLoad::Rows(list)) if list.is_empty() => {
            Some("No ranked matches yet. Upload stats from the tracker.")
        }
        Some(LeaderboardLoad::Rows(_)) => None,
    }
}

#[component]
pub fn Leaderboards() -> Element {
    let mut metric = use_signal(|| "winrate".to_string());
    let mut hero = use_signal(|| None::<String>);
    // Same saved season as My Stats and member stats, including the
    // "current" sentinel. A pick here changes those pages too.
    let season = use_stats_season();
    let rows = use_resource(move || {
        let m = metric();
        let h = hero();
        let mut url = format!("/api/public/leaderboards?metric={m}&limit=50");
        if let Some(h) = h {
            url.push_str(&format!("&hero={}", encode_query(&h)));
        }
        // Empty while a saved season is unresolved — do not send it raw.
        let url = season.fetch_path(&url);
        async move {
            if let Some(held) = leaderboard_hold(&url) {
                return held;
            }
            match ApiClient::web().fetch::<Vec<LeaderboardRow>>(&url).await {
                Ok(list) => LeaderboardLoad::Rows(list),
                Err(_) => LeaderboardLoad::Failed,
            }
        }
    });

    rsx! {
        style { {PAGE_CSS} }
        main { class: "lb-page",
            h1 { "Leaderboards" }
            p { class: "lb-sub", "Ranked from uploaded personal stats (OCR). Sparse data is normal." }

            div { class: "lb-tabs",
                button {
                    class: if metric() == "winrate" { "lb-tab active" } else { "lb-tab" },
                    onclick: move |_| metric.set("winrate".into()),
                    "Win rate"
                }
                button {
                    class: if metric() == "kd" { "lb-tab active" } else { "lb-tab" },
                    onclick: move |_| metric.set("kd".into()),
                    "K/D"
                }
                button {
                    class: if metric() == "games" { "lb-tab active" } else { "lb-tab" },
                    onclick: move |_| metric.set("games".into()),
                    "Games"
                }
            }

            div { class: "lb-filters",
                div { class: "lb-hero",
                    HeroSelect {
                        label: "Hero".to_string(),
                        value: hero(),
                        onchange: move |h| hero.set(h),
                    }
                }
                div { class: "lb-hero",
                    SeasonSelect {
                        label: "Season".to_string(),
                        seasons: season.season_list(),
                        seasons_error: season.seasons_error(),
                        on_retry: move |_| season.retry(),
                        value: season.selected_id(),
                        onchange: move |s| season.choose(s),
                    }
                }
            }

            Card {
                {
                    let snapshot = rows.read();
                    let filtered = hero().is_some();
                    match (
                        leaderboard_message(snapshot.as_ref(), filtered),
                        snapshot.as_ref(),
                    ) {
                        (Some(message), _) => rsx! { p { class: "lb-status", "{message}" } },
                        (None, Some(LeaderboardLoad::Rows(list))) => rsx! {
                            table { class: "lb-table",
                                thead {
                                    tr {
                                        th { "#" }
                                        th { "Player" }
                                        th { "Games" }
                                        th { "WR" }
                                        th { "K/D" }
                                    }
                                }
                                tbody {
                                    for (i, r) in list.iter().enumerate() {
                                        {
                                            let rank = i + 1;
                                            let wr = format!("{:.0}%", r.winrate * 100.0);
                                            let kd = format!("{:.2}", r.kd);
                                            rsx! {
                                                tr { key: "{r.member_id}",
                                                    td { class: "lb-rank", "{rank}" }
                                                    td {
                                                        Link {
                                                            to: Route::MemberProfile { id: r.member_id.clone() },
                                                            "{r.display_name}"
                                                        }
                                                    }
                                                    td { class: "lb-num", "{r.games}" }
                                                    td { class: "lb-num",
                                                        Pill { tone: PillTone::Accent, "{wr}" }
                                                    }
                                                    td { class: "lb-num", "{kd}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        },
                        _ => rsx! {},
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_season_path_holds_instead_of_failing() {
        assert_eq!(
            leaderboard_hold(""),
            Some(LeaderboardLoad::Hold),
            "an empty season path is the unresolved hold, not a failure"
        );
        assert_eq!(
            leaderboard_hold("/api/public/leaderboards?metric=winrate"),
            None
        );
    }

    #[test]
    fn hold_renders_loading_not_the_failure() {
        assert_eq!(leaderboard_message(None, false), Some("Loading..."));
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Hold), false),
            Some("Loading...")
        );
        assert_ne!(
            leaderboard_message(Some(&LeaderboardLoad::Hold), false),
            Some("Couldn't load leaderboards.")
        );
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Failed), false),
            Some("Couldn't load leaderboards.")
        );
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Rows(Vec::new())), true),
            Some("No ranked matches on this hero yet.")
        );
        assert_eq!(
            leaderboard_message(Some(&LeaderboardLoad::Rows(Vec::new())), false),
            Some("No ranked matches yet. Upload stats from the tracker.")
        );
        assert_eq!(
            leaderboard_message(
                Some(&LeaderboardLoad::Rows(vec![LeaderboardRow {
                    member_id: "m1".into(),
                    display_name: "A".into(),
                    games: 1,
                    winrate: 1.0,
                    kd: 1.0,
                }])),
                false
            ),
            None
        );
    }
}
