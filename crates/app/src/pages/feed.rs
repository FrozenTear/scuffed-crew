use dioxus::prelude::*;

use crate::components::post::{FeedPost, PostCard, PostCompose};
use scuffed_api_client::ApiClient;

const PAGE_CSS: &str = r#"
    .feed-page {
        padding: 3rem 2rem;
        max-width: 700px;
        margin: 0 auto;
    }
    .feed-page-title {
        font-family: var(--font-head);
        font-size: 2.5rem;
        color: var(--text);
        letter-spacing: 3px;
        margin: 0 0 0.25rem;
    }
    .feed-subtitle {
        color: var(--text-2);
        font-size: 0.9rem;
        margin: 0 0 2rem;
    }
    .feed-nostr-badge {
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
        margin-bottom: 1.5rem;
    }
    .feed-list {
        display: flex;
        flex-direction: column;
        gap: 1rem;
    }
    .feed-loading, .feed-empty {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
    .feed-filter-bar {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 1.25rem;
        flex-wrap: wrap;
    }
    .feed-filter-tag {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        background: var(--accent-soft);
        color: var(--accent);
        padding: 0.2rem 0.6rem;
        border-radius: 999px;
        font-size: 0.75rem;
        font-weight: 600;
    }
    .feed-filter-clear {
        background: none;
        border: 1px solid var(--border);
        color: var(--text-3);
        border-radius: 999px;
        padding: 0.15rem 0.5rem;
        font-size: 0.7rem;
        cursor: pointer;
        transition: border-color 0.15s, color 0.15s;
    }
    .feed-filter-clear:hover {
        border-color: var(--text-2);
        color: var(--text-2);
    }
    @media (max-width: 768px) {
        .feed-page { padding: 2rem 1rem; }
    }
"#;

#[component]
pub fn Feed() -> Element {
    let posts_resource = use_resource(|| async {
        ApiClient::web()
            .fetch::<Vec<FeedPost>>("/api/nostr/feed")
            .await
            .ok()
    });

    let me = use_resource(|| async {
        ApiClient::web()
            .fetch::<scuffed_types::MeResponse>("/api/auth/me")
            .await
            .ok()
    });

    let mut optimistic_posts = use_signal(Vec::<FeedPost>::new);
    let mut tag_filter = use_signal(|| Option::<String>::None);

    let is_member = me
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .and_then(|m| m.member.as_ref())
        .is_some();

    let on_post = move |post: FeedPost| {
        optimistic_posts.write().insert(0, post);
    };

    let on_tag_click = move |tag: String| {
        tag_filter.set(Some(tag));
    };

    rsx! {
        style { {PAGE_CSS} }

        main { class: "feed-page",
            h1 { class: "feed-page-title", "Feed" }
            p { class: "feed-subtitle",
                "The latest from The Scuffed Crew — powered by Nostr."
            }
            span { class: "feed-nostr-badge", "Kind 1 · Text Notes" }

            if is_member {
                PostCompose { on_post: on_post }
            }

            if let Some(ref tag) = *tag_filter.read() {
                div { class: "feed-filter-bar",
                    span { class: "feed-filter-tag", "#{tag}" }
                    button {
                        class: "feed-filter-clear",
                        onclick: move |_| tag_filter.set(None),
                        "clear"
                    }
                }
            }

            {
                let data = posts_resource.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "feed-loading", "Loading posts..." } },
                    Some(server_posts) => {
                        let mut all_posts = optimistic_posts().clone();
                        for sp in server_posts.iter() {
                            if !all_posts.iter().any(|p| p.id == sp.id) {
                                all_posts.push(sp.clone());
                            }
                        }

                        let filter = tag_filter().clone();
                        let filtered: Vec<&FeedPost> = all_posts
                            .iter()
                            .filter(|p| match &filter {
                                Some(tag) => p.hashtags.iter().any(|t| t.eq_ignore_ascii_case(tag)),
                                None => true,
                            })
                            .collect();

                        if filtered.is_empty() {
                            rsx! {
                                p { class: "feed-empty",
                                    if filter.is_some() {
                                        "No posts with that tag yet."
                                    } else {
                                        "No posts yet. Be the first to share something!"
                                    }
                                }
                            }
                        } else {
                            rsx! {
                                div { class: "feed-list",
                                    for post in filtered {
                                        PostCard {
                                            key: "{post.id}",
                                            post: post.clone(),
                                            on_tag_click: on_tag_click,
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
}
