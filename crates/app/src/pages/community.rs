use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ui::{BtnVariant, Button};
use crate::components::{Toast, use_toast};
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize)]
struct PublicOverview {
    member_count: u64,
    team_count: u64,
    #[allow(dead_code)]
    upcoming_events: u64,
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

    rsx! {
        style { {PAGE_CSS} }

        main { class: "community-page",
            h1 { class: "community-page-title", "Community" }
            p { class: "community-subtitle",
                "Our community lives on the Nostr protocol — decentralized, censorship-resistant, and open."
            }

            div { class: "community-hero",
                div { class: "community-banner-placeholder",
                    span { "SCUFFED CREW" }
                }
                div { class: "community-body",
                    span { class: "community-nostr-badge", "Nostr-Native" }

                    h2 { class: "community-name", "The Scuffed Crew" }
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
                                div { class: "community-stat-value", "{stats.team_count}" }
                                div { class: "community-stat-label", "Teams" }
                            }
                        }
                    }
                }
            }

            CommunityFeatures {}

            if is_officer {
                OfficerCommunityActions {}
            }
        }
    }
}

#[component]
fn CommunityFeatures() -> Element {
    rsx! {
        div { class: "community-section",
            h3 { class: "community-section-title", "How It Works" }
            div { class: "community-rules",
                "Your Scuffed Crew account is backed by a Nostr keypair.\n\n\
                 • Your identity is portable — take it to any Nostr-compatible app\n\
                 • Messages flow through relays, not centralized servers\n\
                 • NIP-05 verification proves your identity: you@scuffed.gg\n\
                 • NIP-49 encrypted backups keep your keys safe\n\
                 • NIP-25 reactions let you engage with community content\n\n\
                 Visit the Identity page to set up your Nostr identity."
            }
        }
    }
}

#[component]
fn OfficerCommunityActions() -> Element {
    let mut toasts = use_toast();
    let mut publishing = use_signal(|| false);

    let publish_community = move |_| {
        spawn(async move {
            publishing.set(true);
            let body = CommunityCreateBody {
                community_id: "scuffed-crew".to_string(),
                name: "The Scuffed Crew".to_string(),
                description: Some(
                    "Competitive gaming community on Nostr. Your keys, your identity.".to_string(),
                ),
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
