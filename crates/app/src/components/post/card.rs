use dioxus::prelude::*;

use crate::components::chat::{ReactionBar, ReactionCount};

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct FeedPost {
    pub id: String,
    pub pubkey: String,
    pub author_name: Option<String>,
    pub content: String,
    pub hashtags: Vec<String>,
    pub created_at: i64,
    #[serde(default)]
    pub reactions: Vec<ReactionCountDto>,
    #[serde(default)]
    pub reply_count: u32,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct ReactionCountDto {
    pub emoji: String,
    pub count: u32,
    #[serde(default)]
    pub reacted_by_me: bool,
}

const CARD_CSS: &str = r#"
.post-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.25rem 1.5rem;
    transition: border-color 0.15s;
}
.post-card:hover {
    border-color: var(--accent-soft);
}
.post-card-header {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin-bottom: 0.75rem;
}
.post-avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14px;
    font-weight: 700;
    flex-shrink: 0;
}
.post-author {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 0.95rem;
    color: var(--text);
}
.post-time {
    font-size: 0.7rem;
    color: var(--text-3);
}
.post-content {
    color: var(--text-2);
    font-size: 0.9rem;
    line-height: 1.7;
    margin-bottom: 0.75rem;
    white-space: pre-wrap;
    word-break: break-word;
}
.post-content a {
    color: var(--accent);
    text-decoration: underline;
}
.post-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-bottom: 0.75rem;
}
.post-tag {
    background: var(--accent-soft);
    color: var(--accent);
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    font-size: 0.7rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s;
}
.post-tag:hover {
    background: color-mix(in srgb, var(--accent) 27%, transparent);
}
.post-footer {
    display: flex;
    align-items: center;
    gap: 1rem;
}
.post-reply-count {
    font-size: 0.75rem;
    color: var(--text-3);
    display: flex;
    align-items: center;
    gap: 0.3rem;
}
"#;

#[component]
pub fn PostCard(post: FeedPost, on_tag_click: Option<EventHandler<String>>) -> Element {
    let author = post.author_name.as_deref().unwrap_or("Anonymous");
    let initial = author
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    let time_ago = relative_time(post.created_at);

    let reactions: Vec<ReactionCount> = post
        .reactions
        .iter()
        .map(|r| ReactionCount {
            emoji: r.emoji.clone(),
            count: r.count,
            reacted_by_me: r.reacted_by_me,
        })
        .collect();

    rsx! {
        style { {CARD_CSS} }
        article { class: "post-card",
            div { class: "post-card-header",
                div { class: "post-avatar", "{initial}" }
                div {
                    div { class: "post-author", "{author}" }
                    div { class: "post-time", "{time_ago}" }
                }
            }

            div { class: "post-content",
                {linkify_content(&post.content)}
            }

            if !post.hashtags.is_empty() {
                div { class: "post-tags",
                    for tag in post.hashtags.iter() {
                        {render_tag(tag.clone(), &on_tag_click)}
                    }
                }
            }

            div { class: "post-footer",
                ReactionBar {
                    event_id: post.id.clone(),
                    event_author_pubkey: post.pubkey.clone(),
                    reactions: reactions,
                }
                if post.reply_count > 0 {
                    span { class: "post-reply-count",
                        {format!("{} {}", post.reply_count, if post.reply_count == 1 { "reply" } else { "replies" })}
                    }
                }
            }
        }
    }
}

fn render_tag(tag: String, handler: &Option<EventHandler<String>>) -> Element {
    let tag_display = tag.clone();
    let tag_emit = tag.clone();
    match handler {
        Some(h) => {
            let h = *h;
            rsx! {
                span {
                    class: "post-tag",
                    onclick: move |_| h.call(tag_emit.clone()),
                    "#{tag_display}"
                }
            }
        }
        None => {
            rsx! {
                span { class: "post-tag", "#{tag_display}" }
            }
        }
    }
}

fn linkify_content(content: &str) -> Element {
    let mut parts: Vec<Element> = Vec::new();
    let mut last = 0;

    for (start, part) in content.match_indices("http") {
        if start > last {
            let text = content[last..start].to_string();
            parts.push(rsx! { "{text}" });
        }
        let end = content[start..]
            .find(|c: char| c.is_whitespace())
            .map(|i| start + i)
            .unwrap_or(content.len());
        let url = content[start..end].to_string();
        let url2 = url.clone();
        parts.push(rsx! { a { href: "{url}", target: "_blank", rel: "noopener", "{url2}" } });
        last = end;
        let _ = part;
    }

    if last < content.len() {
        let text = content[last..].to_string();
        parts.push(rsx! { "{text}" });
    }

    rsx! {
        for (i, el) in parts.into_iter().enumerate() {
            Fragment { key: "{i}", {el} }
        }
    }
}

fn relative_time(unix_secs: i64) -> String {
    let now = js_sys::Date::now() as i64 / 1000;
    let diff = now - unix_secs;

    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        let m = diff / 60;
        format!("{m}m ago")
    } else if diff < 86400 {
        let h = diff / 3600;
        format!("{h}h ago")
    } else if diff < 604800 {
        let d = diff / 86400;
        format!("{d}d ago")
    } else {
        let w = diff / 604800;
        format!("{w}w ago")
    }
}
