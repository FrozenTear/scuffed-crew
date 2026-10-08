//! Public leaderboards page (#6).

use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use crate::components::ui::{Card, HeroSelect, Pill, PillTone, SeasonSelect};
use crate::routes::Route;
use crate::util::{encode_query, season_url};

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
    .lb-updated {
        color: var(--text-3);
        font-size: 0.8rem;
        margin: -0.75rem 0 1.25rem;
    }
"#;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct LeaderboardPayload {
    rows: Vec<LeaderboardRow>,
    /// Missing on an older response. Empty or unparseable hides the label.
    #[serde(default)]
    cached_at: Option<String>,
}

/// "Updated just now" / "Updated 12s ago" / "Updated 3m ago" / "Updated 2h ago".
///
/// A timestamp ahead of `now` (clock skew) reads as just now. Parsing stays
/// in [`parse_cached_at`]; a missing stamp is not passed in.
fn updated_label(
    cached_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let secs = now.signed_duration_since(cached_at).num_seconds().max(0);
    if secs < 5 {
        "Updated just now".into()
    } else if secs < 60 {
        format!("Updated {secs}s ago")
    } else if secs < 3600 {
        format!("Updated {}m ago", secs / 60)
    } else {
        format!("Updated {}h ago", secs / 3600)
    }
}

fn parse_cached_at(raw: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

#[component]
pub fn Leaderboards() -> Element {
    let mut metric = use_signal(|| "winrate".to_string());
    let mut hero = use_signal(|| None::<String>);
    let mut season = use_signal(|| None::<String>);
    // Last stamp that arrived. A filter change does not clear this, so the
    // header keeps its label until the next payload lands. A failed fetch
    // does clear it, so the label does not sit above the error.
    let mut shown_at = use_signal(|| None::<chrono::DateTime<chrono::Utc>>);
    let mut label_tick = use_signal(|| 0u32);
    // `use_future` spawns on this component's scope. Dioxus drops that task
    // when the page unmounts, which stops the timer.
    let _label_timer = use_future(move || async move {
        loop {
            #[cfg(feature = "web")]
            {
                gloo_timers::future::TimeoutFuture::new(15_000).await;
            }
            #[cfg(not(feature = "web"))]
            {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
            label_tick += 1;
        }
    });
    let board = use_resource(move || {
        let m = metric();
        let h = hero();
        let se = season();
        async move {
            let mut url = format!("/api/public/leaderboards?metric={m}&limit=50");
            if let Some(h) = h {
                url.push_str(&format!("&hero={}", encode_query(&h)));
            }
            let url = season_url(&url, se);
            match ApiClient::web().fetch::<LeaderboardPayload>(&url).await {
                Ok(payload) => {
                    shown_at.set(parse_cached_at(payload.cached_at.as_deref()));
                    Some(payload.rows)
                }
                Err(_) => {
                    shown_at.set(None);
                    None
                }
            }
        }
    });
    let _tick = label_tick();
    let updated = match shown_at() {
        Some(at) => updated_label(at, chrono::Utc::now()),
        None => String::new(),
    };

    rsx! {
        style { {PAGE_CSS} }
        main { class: "lb-page",
            h1 { "Leaderboards" }
            p { class: "lb-sub", "Ranked from uploaded personal stats (OCR). Sparse data is normal." }
            if !updated.is_empty() {
                p { class: "lb-updated", "{updated}" }
            }

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
                        value: season(),
                        onchange: move |s| season.set(s),
                    }
                }
            }

            Card {
                {
                    match board.read().as_ref() {
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

#[cfg(test)]
mod tests {
    use super::{LeaderboardPayload, parse_cached_at, updated_label};
    use chrono::{TimeZone, Utc};

    fn at(secs_from_now: i64, now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
        now + chrono::Duration::seconds(secs_from_now)
    }

    #[test]
    fn updated_label_just_now_seconds_and_minutes() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(updated_label(at(0, now), now), "Updated just now");
        assert_eq!(updated_label(at(-4, now), now), "Updated just now");
        assert_eq!(updated_label(at(-5, now), now), "Updated 5s ago");
        assert_eq!(updated_label(at(-59, now), now), "Updated 59s ago");
        assert_eq!(updated_label(at(-60, now), now), "Updated 1m ago");
        assert_eq!(updated_label(at(-125, now), now), "Updated 2m ago");
    }

    #[test]
    fn updated_label_hours_start_at_sixty_minutes() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(updated_label(at(-59 * 60, now), now), "Updated 59m ago");
        assert_eq!(updated_label(at(-60 * 60, now), now), "Updated 1h ago");
        assert_eq!(updated_label(at(-2 * 3600, now), now), "Updated 2h ago");
        assert_eq!(updated_label(at(-5 * 3600, now), now), "Updated 5h ago");
    }

    #[test]
    fn updated_label_future_stamp_reads_as_just_now() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(updated_label(at(30, now), now), "Updated just now");
    }

    #[test]
    fn parse_cached_at_hides_malformed_stamps() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        assert_eq!(parse_cached_at(None), None);
        assert_eq!(parse_cached_at(Some("")), None);
        assert_eq!(parse_cached_at(Some("   ")), None);
        assert_eq!(parse_cached_at(Some("not-a-timestamp")), None);
        assert_eq!(parse_cached_at(Some("2026-13-99")), None);
        let raw = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert_eq!(parse_cached_at(Some(&raw)), Some(now));
    }

    #[test]
    fn payload_without_cached_at_still_parses() {
        let payload: LeaderboardPayload = serde_json::from_str(
            r#"{"rows":[{"member_id":"m","display_name":"M","games":1,"winrate":1.0,"kd":1.0}]}"#,
        )
        .unwrap();
        assert!(payload.cached_at.is_none());
        assert_eq!(payload.rows.len(), 1);
        assert_eq!(parse_cached_at(payload.cached_at.as_deref()), None);
    }
}
