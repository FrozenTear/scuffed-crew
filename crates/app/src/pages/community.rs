use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ui::{BtnVariant, Button};
use crate::components::{Toast, use_toast};
use crate::state::{loaded_site_settings, use_site_settings};
use scuffed_api_client::ApiClient;

/// Intro line. A missing org name stays generic so the page never says "My Clan".
fn community_intro(org_name: Option<&str>) -> String {
    match org_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("Your {name} account is backed by a Nostr keypair.\n\n"),
        None => "Your account is backed by a Nostr keypair.\n\n".to_string(),
    }
}

/// Fields this page reads from `GET /api/public/overview`.
///
/// Mirrors `PublicOverview` in `crates/site-server/src/routes/public.rs`:
/// `member_count` plus `teams`. The route also sends `games`, `events`,
/// `announcements`, `settings`, `upcoming_matches`, and `recent_results`.
/// Those are ignored here. Team rows are not read field by field; the stats
/// block shows `teams.len()`.
#[derive(Debug, Clone, Deserialize)]
struct PublicOverview {
    member_count: u64,
    #[serde(default)]
    teams: Vec<serde::de::IgnoredAny>,
}

#[derive(Serialize)]
struct CommunityCreateBody {
    community_id: String,
    name: String,
    description: Option<String>,
    rules: Option<String>,
    image: Option<String>,
}

const PAGE_CSS: &str = r#"
    .community-page {
        padding: 3rem 2rem;
        max-width: 900px;
        margin: 0 auto;
    }
    .community-page-title {
        font-family: var(--font-head);
        font-size: 2.5rem;
        color: var(--text);
        letter-spacing: 3px;
        margin: 0 0 0.5rem;
    }
    .community-subtitle {
        color: var(--text-2);
        font-size: 0.95rem;
        margin: 0 0 2rem;
    }
    .community-hero {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 12px;
        overflow: hidden;
        margin-bottom: 2rem;
    }
    .community-banner {
        width: 100%;
        height: 200px;
        object-fit: cover;
        display: block;
    }
    .community-banner-placeholder {
        width: 100%;
        height: 200px;
        background: linear-gradient(135deg, var(--accent) 0%, color-mix(in srgb, var(--accent) 40%, var(--bg)) 100%);
        display: flex;
        align-items: center;
        justify-content: center;
    }
    .community-banner-placeholder span {
        font-family: var(--font-head);
        font-size: 3rem;
        color: color-mix(in srgb, var(--text) 30%, transparent);
        letter-spacing: 8px;
    }
    .community-body {
        padding: 1.5rem 2rem 2rem;
    }
    .community-name {
        font-family: var(--font-head);
        font-size: 1.5rem;
        font-weight: 700;
        color: var(--text);
        margin: 0 0 0.5rem;
    }
    .community-desc {
        color: var(--text-2);
        font-size: 0.9rem;
        line-height: 1.6;
        margin: 0 0 1.5rem;
    }
    .community-stats {
        display: flex;
        gap: 2rem;
        margin-bottom: 1.5rem;
    }
    .community-stat {
        text-align: center;
    }
    .community-stat-value {
        font-family: var(--font-head);
        font-size: 2rem;
        color: var(--accent);
    }
    .community-stat-label {
        font-size: 0.75rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.05em;
    }
    .community-section {
        margin-top: 2rem;
    }
    .community-section-title {
        font-family: var(--font-head);
        font-size: 1.1rem;
        font-weight: 700;
        color: var(--text);
        margin: 0 0 0.75rem;
        padding-bottom: 0.5rem;
        border-bottom: 1px solid var(--border);
    }
    .community-rules {
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1rem 1.25rem;
        color: var(--text-2);
        font-size: 0.85rem;
        line-height: 1.6;
        white-space: pre-wrap;
    }
    .community-mods {
        display: flex;
        flex-wrap: wrap;
        gap: 1rem;
    }
    .community-mod {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.5rem 0.75rem;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 8px;
    }
    .community-mod-avatar {
        width: 28px;
        height: 28px;
        border-radius: 50%;
        background: var(--accent-soft);
        color: var(--accent);
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 12px;
        font-weight: 600;
        flex-shrink: 0;
    }
    .community-mod-avatar img {
        width: 100%;
        height: 100%;
        border-radius: 50%;
        object-fit: cover;
    }
    .community-mod-name {
        font-size: 0.85rem;
        color: var(--text);
        font-weight: 600;
    }
    .community-relay {
        display: inline-flex;
        align-items: center;
        gap: 0.5rem;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 6px;
        padding: 0.5rem 1rem;
        font-family: var(--font-mono);
        font-size: 0.8rem;
        color: var(--text-2);
    }
    .community-relay-dot {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--ok);
    }
    .community-nostr-badge {
        display: inline-flex;
        align-items: center;
        gap: 0.4rem;
        background: var(--accent-soft);
        color: var(--accent);
        padding: 0.25rem 0.75rem;
        border-radius: 999px;
        font-size: 0.7rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        margin-bottom: 1rem;
    }
    .community-loading {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
    @media (max-width: 768px) {
        .community-page { padding: 2rem 1rem; }
        .community-stats { gap: 1rem; }
        .community-body { padding: 1rem; }
    }
"#;

#[component]
pub fn Community() -> Element {
    let overview = use_resource(|| async {
        ApiClient::web()
            .fetch::<PublicOverview>("/api/public/overview")
            .await
            .ok()
    });

    let site_settings = use_site_settings();
    let resolved = site_settings.resolved.read();
    let org_name = loaded_site_settings(resolved.as_ref()).map(|s| s.org_name.clone());

    let me = use_resource(|| async {
        ApiClient::web()
            .fetch::<scuffed_types::MeResponse>("/api/auth/me")
            .await
            .ok()
    });

    let overview_data = overview.read();
    let overview_ref = overview_data.as_ref().and_then(|d| d.as_ref());
    let me_data = me.read();
    let is_officer = me_data
        .as_ref()
        .and_then(|d| d.as_ref())
        .and_then(|m| m.member.as_ref())
        .map(|member| matches!(member.org_role.as_str(), "officer" | "admin"))
        .unwrap_or(false);
    let banner_label = org_name.as_ref().map(|name| name.to_uppercase());

    rsx! {
        style { {PAGE_CSS} }

        main { class: "community-page",
            h1 { class: "community-page-title", "Community" }
            p { class: "community-subtitle",
                "Our community lives on the Nostr protocol — decentralized, censorship-resistant, and open."
            }

            div { class: "community-hero",
                div { class: "community-banner-placeholder",
                    if let Some(label) = banner_label {
                        span { "{label}" }
                    }
                }
                div { class: "community-body",
                    span { class: "community-nostr-badge", "Nostr-Native" }

                    h2 { class: "community-name",
                        if let Some(name) = org_name.clone() {
                            "{name}"
                        } else {
                            span { class: "brand-pending", aria_hidden: "true" }
                        }
                    }
                    p { class: "community-desc",
                        "A competitive gaming community built on Nostr. No central servers own your identity — your keys, your account, everywhere."
                    }

                    if let Some(stats) = overview_ref {
                        div { class: "community-stats",
                            div { class: "community-stat",
                                div { class: "community-stat-value", "{stats.member_count}" }
                                div { class: "community-stat-label", "Members" }
                            }
                            div { class: "community-stat",
                                div { class: "community-stat-value", "{stats.teams.len()}" }
                                div { class: "community-stat-label", "Teams" }
                            }
                        }
                    }
                }
            }

            CommunityFeatures { org_name: org_name.clone() }

            if is_officer {
                if let Some(name) = org_name.clone() {
                    OfficerCommunityActions { org_name: name }
                }
            }
        }
    }
}

#[component]
fn CommunityFeatures(org_name: Option<String>) -> Element {
    let intro = community_intro(org_name.as_deref());
    rsx! {
        div { class: "community-section",
            h3 { class: "community-section-title", "How It Works" }
            div { class: "community-rules",
                "{intro}\
                 • Your identity is portable — take it to any Nostr-compatible app\n\
                 • Messages flow through relays, not centralized servers\n\
                 • NIP-05 verification proves your identity on this org domain\n\
                 • NIP-49 encrypted backups keep your keys safe\n\
                 • NIP-25 reactions let you engage with community content\n\n\
                 Visit the Identity page to set up your Nostr identity."
            }
        }
    }
}

#[component]
fn OfficerCommunityActions(org_name: String) -> Element {
    let mut toasts = use_toast();
    let mut publishing = use_signal(|| false);
    let name_for_publish = org_name.clone();

    let publish_community = move |_| {
        let name = name_for_publish.clone();
        spawn(async move {
            publishing.set(true);
            let body = CommunityCreateBody {
                community_id: "scuffed-crew".to_string(),
                name: name.clone(),
                description: Some(format!(
                    "{name} — competitive gaming community on Nostr. Your keys, your identity."
                )),
                rules: Some(
                    "1. Be respectful\n2. No drama, no politics\n3. Age 16+\n4. Have fun"
                        .to_string(),
                ),
                image: None,
            };

            match ApiClient::web()
                .post_json::<_, serde_json::Value>("/api/nostr/community", &body)
                .await
            {
                Ok(_) => {
                    toasts.show(Toast::success("Community definition published to relay"));
                }
                Err(e) => {
                    toasts.show(Toast::error(format!("Failed to publish: {e}")));
                }
            }
            publishing.set(false);
        });
    };

    rsx! {
        div { class: "community-section",
            h3 { class: "community-section-title", "Officer Actions" }
            Button {
                variant: BtnVariant::Primary,
                disabled: publishing(),
                onclick: publish_community,
                if publishing() { "Publishing..." } else { "Publish Community to Relay" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intro_omits_a_name_until_settings_load() {
        let pending = community_intro(None);
        assert!(pending.starts_with("Your account is backed"));
        assert!(!pending.contains("My Clan"));
        assert_eq!(
            community_intro(Some("Night Owls")),
            "Your Night Owls account is backed by a Nostr keypair.\n\n"
        );
        assert!(!community_intro(Some("  ")).contains("My Clan"));
    }

    fn parse_overview(json: serde_json::Value) -> PublicOverview {
        serde_json::from_value(json).expect("overview payload")
    }

    /// Hand-written sample of `PublicOverview` in
    /// `crates/site-server/src/routes/public.rs`. The app crate does not depend
    /// on the server crate, so this is not built by serializing that type.
    /// Field names match the server struct. Nested objects follow `Team` (flattened
    /// into `TeamOverview`), `TeamRecord`, `Game`, `Event`, `Announcement`,
    /// `SiteSettings` in `crates/db/src/types.rs`, and `UpcomingMatch` /
    /// `RecentResult` in `crates/types`.
    fn server_shaped_overview() -> serde_json::Value {
        serde_json::json!({
            "teams": [
                {
                    "id": "team-1",
                    "name": "Night Owls",
                    "game_id": "ow2",
                    "color": null,
                    "division": "Open",
                    "lore_quote": null,
                    "logo_url": null,
                    "is_active": true,
                    "created_at": "2026-01-15T00:00:00Z",
                    "roster_count": 5,
                    "record": { "wins": 3, "losses": 1, "draws": 0 }
                },
                {
                    "id": "team-2",
                    "name": "Day Hawks",
                    "game_id": "ow2",
                    "color": null,
                    "division": null,
                    "lore_quote": "Hold the high ground.",
                    "logo_url": null,
                    "is_active": true,
                    "created_at": "2026-02-01T00:00:00Z",
                    "roster_count": 4,
                    "record": { "wins": 0, "losses": 2, "draws": 1 }
                }
            ],
            "games": [
                {
                    "id": "ow2",
                    "name": "Overwatch 2",
                    "abbreviation": "OW2",
                    "is_active": true,
                    "created_at": "2026-01-01T00:00:00Z"
                }
            ],
            "events": [
                {
                    "id": "event-1",
                    "title": "Scrim night",
                    "day_of_week": 2,
                    "time": "19:00",
                    "timezone": "UTC",
                    "duration_minutes": 120,
                    "is_recurring": true,
                    "team_id": "team-1",
                    "created_by": "member-1",
                    "is_active": true,
                    "is_public": true
                }
            ],
            "announcements": [
                {
                    "id": "ann-1",
                    "title": "Welcome",
                    "content": "Season starts soon.",
                    "author_id": "member-1",
                    "pinned": true,
                    "is_active": true,
                    "created_at": "2026-03-01T12:00:00Z",
                    "updated_at": "2026-03-01T12:00:00Z"
                }
            ],
            "settings": {
                "id": "settings",
                "org_name": "Night Owls",
                "site_description": "Competitive gaming community",
                "recruitment_open": true,
                "strategies_enabled": true,
                "officers_can_edit_teams": false,
                "recruitment_message": "Apply in the form.",
                "min_age": 16,
                "forum_backend": "local",
                "extra_relay_urls": "",
                "home_shell": "ops_hub",
                "home_skin": "clean",
                "public_layout": "hub",
                "homepage_json": "{}",
                "nav_json": "{}",
                "page_bg_color": "",
                "page_bg_image_url": "",
                "brand_accent_dark": "",
                "brand_accent_light": "",
                "updated_at": "2026-03-01T12:00:00Z"
            },
            "member_count": 12,
            "upcoming_matches": [
                {
                    "id": "match-1",
                    "team_id": "team-1",
                    "team_name": "Night Owls",
                    "game_name": "Overwatch 2",
                    "opponent": "Rivals",
                    "match_type": "official",
                    "scheduled_at": "2026-04-01T18:00:00Z"
                }
            ],
            "recent_results": [
                {
                    "id": "match-0",
                    "team_id": "team-1",
                    "team_name": "Night Owls",
                    "opponent": "Rivals",
                    "score_us": 2,
                    "score_them": 1,
                    "outcome": "win",
                    "match_type": "official",
                    "played_at": "2026-03-20T18:00:00Z"
                }
            ]
        })
    }

    #[test]
    fn server_shaped_overview_parses_member_and_team_counts() {
        let overview = parse_overview(server_shaped_overview());
        assert_eq!(overview.member_count, 12);
        assert_eq!(overview.teams.len(), 2);
    }

    #[test]
    fn empty_teams_list_counts_as_zero() {
        let mut payload = server_shaped_overview();
        payload["teams"] = serde_json::json!([]);
        payload["member_count"] = serde_json::json!(4);
        let overview = parse_overview(payload);
        assert_eq!(overview.member_count, 4);
        assert_eq!(overview.teams.len(), 0);
    }

    #[test]
    fn missing_teams_field_defaults_to_zero() {
        let mut payload = server_shaped_overview();
        payload.as_object_mut().expect("object").remove("teams");
        let overview = parse_overview(payload);
        assert_eq!(overview.member_count, 12);
        assert_eq!(overview.teams.len(), 0);
    }
}
