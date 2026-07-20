use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{EmptyState, HeroSelect, Pill, PillTone};
use crate::routes::Route;
use crate::util::encode_query;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize)]
struct PublicMember {
    id: String,
    display_name: String,
    org_role: String,
    bio: Option<String>,
    avatar_url: Option<String>,
    #[allow(dead_code)]
    joined_at: String,
    /// Hero-scoped performance, only present when the roster was fetched with a
    /// `?hero=` filter (contract Q2). A nested block — NOT sibling fields.
    /// Absent for members who have not played the selected hero (or when no
    /// hero filter is active), so it stays optional and serde-defaults to None.
    #[serde(default)]
    hero_scoped: Option<HeroScoped>,
}

/// Nested per-hero stat block attached to a [`PublicMember`] when the roster is
/// fetched with `?hero=`. `winrate` is a 0.0–1.0 fraction (multiply by 100 for
/// display).
#[derive(Debug, Clone, Deserialize)]
struct HeroScoped {
    games: u32,
    winrate: f32,
}

#[derive(Deserialize)]
struct MembersResponse {
    data: Vec<PublicMember>,
    /// Present when more pages exist (same contract as `CursorResponse`).
    #[serde(default)]
    next_cursor: Option<String>,
}

/// Backend max page size (`PaginationParams` clamps to 1..=100).
const HERO_FILTER_PAGE_LIMIT: u32 = 100;
/// Safety cap while walking cursors under a hero filter (100 × 20 = 2000 members).
/// Large enough for growth; avoids unbounded request loops.
const HERO_FILTER_MAX_PAGES: usize = 20;

/// Fetch roster members. Unfiltered: first page only (prior UX). Hero filter:
/// walk `next_cursor` so players past offset 100 still appear (HS-DR FE-1).
async fn fetch_roster_members(hero: Option<&str>) -> Option<Vec<PublicMember>> {
    match hero {
        None => ApiClient::web()
            .fetch::<MembersResponse>("/api/public/members")
            .await
            .ok()
            .map(|r| r.data),
        Some(h) => {
            let mut all = Vec::new();
            let mut cursor: Option<String> = None;
            for _ in 0..HERO_FILTER_MAX_PAGES {
                let mut path = format!(
                    "/api/public/members?hero={}&limit={HERO_FILTER_PAGE_LIMIT}",
                    encode_query(h)
                );
                if let Some(c) = cursor.as_deref() {
                    path.push_str(&format!("&cursor={}", encode_query(c)));
                }
                let page = ApiClient::web()
                    .fetch::<MembersResponse>(&path)
                    .await
                    .ok()?;
                all.extend(page.data);
                match page.next_cursor {
                    Some(c) if !c.is_empty() => cursor = Some(c),
                    _ => break,
                }
            }
            Some(all)
        }
    }
}

const PAGE_CSS: &str = r#"
    .members-page {
        padding: 3rem 2rem;
        max-width: 1000px;
        margin: 0 auto;
    }
    .members-page-title {
        font-family: var(--font-head);
        font-size: 2.5rem;
        color: var(--text);
        letter-spacing: 3px;
        margin: 0 0 2rem;
    }
    .members-toolbar {
        margin: 0 0 1.5rem;
        max-width: 260px;
    }
    .members-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
        gap: 1.25rem;
    }
    .member-card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
        text-decoration: none;
        text-align: center;
        transition: border-color 0.2s, transform 0.2s;
        display: block;
    }
    .member-card:hover {
        border-color: var(--accent-soft);
        transform: translateY(-2px);
    }
    .member-avatar {
        width: 72px;
        height: 72px;
        border-radius: 50%;
        margin: 0 auto 1rem;
        overflow: hidden;
        background: var(--surface-2);
        display: flex;
        align-items: center;
        justify-content: center;
    }
    .member-avatar img {
        width: 100%;
        height: 100%;
        object-fit: cover;
    }
    .member-initials {
        font-family: var(--font-head);
        font-size: 1.4rem;
        color: var(--accent);
        letter-spacing: 2px;
    }
    .member-name {
        font-family: var(--font-head);
        font-size: 1.1rem;
        font-weight: 700;
        color: var(--text);
        margin: 0 0 0.5rem;
    }
    .member-bio {
        color: var(--text-2);
        font-size: 0.8rem;
        line-height: 1.5;
        margin-top: 0.75rem;
        overflow: hidden;
        display: -webkit-box;
        -webkit-line-clamp: 3;
        -webkit-box-orient: vertical;
    }
    .member-hero-stat {
        margin-top: 0.75rem;
        font-family: var(--font-head);
        font-size: 0.8rem;
        color: var(--accent);
        letter-spacing: 0.5px;
    }
    .members-loading {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
"#;

#[component]
pub fn Members() -> Element {
    // `None` = "All heroes" (no filter). Drives both the HeroSelect value and
    // the resource re-fetch below.
    let mut hero = use_signal(|| None::<String>);

    // Reading `hero()` inside the closure (before `async move`) makes the
    // resource reactive: it re-fetches whenever the selection changes.
    let members = use_resource(move || {
        let hero = hero();
        async move {
            // Unfiltered: first page (unchanged). Hero filter: cursor-walk the
            // full org (up to HERO_FILTER_MAX_PAGES) then client-filter/sort —
            // single-page limit=100 still missed mains past offset 100 (FE-1).
            fetch_roster_members(hero.as_deref()).await
        }
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "members-page",
            h1 { class: "members-page-title", "Our Crew" }

            div { class: "members-toolbar",
                HeroSelect {
                    label: Some("Filter by hero".to_string()),
                    value: hero(),
                    onchange: move |v| hero.set(v),
                }
            }

            {
                let hero_sel = hero();
                let data = members.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match (data, &hero_sel) {
                    (None, _) => rsx! { p { class: "members-loading", "Loading..." } },
                    // No hero filter: render exactly as before.
                    (Some(list), None) => {
                        if list.is_empty() {
                            rsx! {
                                EmptyState { title: "No members yet.", message: "Check back soon." }
                            }
                        } else {
                            rsx! {
                                div { class: "members-grid",
                                    for m in list.iter() {
                                        {render_member_card(m, None)}
                                    }
                                }
                            }
                        }
                    }
                    // Hero filter active: keep only members who play it, sort by
                    // games desc then winrate desc, and badge each card.
                    (Some(list), Some(h)) => {
                        let mut filtered: Vec<&PublicMember> =
                            list.iter().filter(|m| m.hero_scoped.is_some()).collect();
                        filtered.sort_by(|a, b| {
                            let a = a.hero_scoped.as_ref().unwrap();
                            let b = b.hero_scoped.as_ref().unwrap();
                            b.games.cmp(&a.games).then_with(|| {
                                b.winrate
                                    .partial_cmp(&a.winrate)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                        });
                        if filtered.is_empty() {
                            rsx! {
                                EmptyState {
                                    title: "No one plays this hero yet.",
                                    message: "Try another hero, or check back once the crew logs more games.",
                                }
                            }
                        } else {
                            rsx! {
                                div { class: "members-grid",
                                    for m in filtered {
                                        {render_member_card(m, Some(h.as_str()))}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_member_card(m: &PublicMember, hero: Option<&str>) -> Element {
    let initials: String = m
        .display_name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let role_tone = match m.org_role.as_str() {
        "admin" => PillTone::Danger,
        "officer" => PillTone::Warn,
        _ => PillTone::Accent,
    };
    let bio = m.bio.clone().unwrap_or_default();

    // Hero-scoped stat line, only when a hero is selected AND this member has
    // played it (contract Q2 nested block present). winrate is a 0.0–1.0
    // fraction, shown as a whole-number percent to match other pages.
    let hero_stat = match (hero, m.hero_scoped.as_ref()) {
        (Some(h), Some(hs)) => Some(format!(
            "{} games · {:.0}% WR on {}",
            hs.games,
            hs.winrate * 100.0,
            h
        )),
        _ => None,
    };

    rsx! {
        Link { to: Route::MemberProfile { id: m.id.clone() }, class: "member-card",
            div { class: "member-avatar",
                if let Some(url) = &m.avatar_url {
                    img { src: "{url}", alt: "{m.display_name}" }
                } else {
                    span { class: "member-initials", "{initials}" }
                }
            }
            h3 { class: "member-name", "{m.display_name}" }
            Pill { tone: role_tone, "{m.org_role}" }
            if !bio.is_empty() {
                p { class: "member-bio", "{bio}" }
            }
            if let Some(stat) = &hero_stat {
                p { class: "member-hero-stat", "{stat}" }
            }
        }
    }
}
