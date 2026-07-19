//! Public leaderboards page (#6).

use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use crate::components::ui::{Card, HeroSelect, Pill, PillTone};
use crate::routes::Route;

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
    .lb-hero {
        max-width: 280px;
        margin-bottom: 1.25rem;
    }
"#;

#[component]
pub fn Leaderboards() -> Element {
    let mut metric = use_signal(|| "winrate".to_string());
    let mut hero = use_signal(|| None::<String>);
    let rows = use_resource(move || {
        let m = metric();
        let h = hero();
        async move {
            let mut url = format!("/api/public/leaderboards?metric={m}&limit=50");
            if let Some(h) = h {
                url.push_str(&format!("&hero={h}"));
            }
            ApiClient::web()
                .fetch::<Vec<LeaderboardRow>>(&url)
                .await
                .ok()
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

            div { class: "lb-hero",
                HeroSelect {
                    label: "Hero".to_string(),
                    value: hero(),
                    onchange: move |h| hero.set(h),
                }
            }

            Card {
                {
                    match rows.read().as_ref() {
                        None => rsx! { p { class: "lb-status", "Loading..." } },
                        Some(None) => rsx! { p { class: "lb-status", "Couldn't load leaderboards." } },
                        Some(Some(list)) if list.is_empty() && hero().is_some() => rsx! {
                            p { class: "lb-status", "No ranked matches on this hero yet." }
                        },
                        Some(Some(list)) if list.is_empty() => rsx! {
                            p { class: "lb-status", "No ranked matches yet. Upload stats from the tracker." }
                        },
                        Some(Some(list)) => rsx! {
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
                    }
                }
            }
        }
    }
}
