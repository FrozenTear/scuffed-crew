//! One feed post: who wrote it and when, the text, its tags and reactions.
//!
//! The author line is the display name and a date, never a key. A post with
//! no name on record reads as [`AUTHOR_FALLBACK`].

use dioxus::prelude::*;

use crate::components::chat::{ReactionBar, ReactionCount};
use crate::components::ui::ChipRow;
use crate::pages::article_author::{AUTHOR_FALLBACK, initials};
use crate::util::pluralize;

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
  box-sizing: border-box;
  min-width: 0;
  padding: var(--space-card-pad);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.post-author-line {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin: 0 0 var(--space-4);
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
.post-avatar {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: var(--control-size-sm);
  height: var(--control-size-sm);
  border-radius: var(--radius-pill);
  border: 1px solid var(--border-strong);
  background: var(--surface);
  color: var(--text);
  font-family: var(--font-head);
  font-size: var(--text-xs);
  font-weight: var(--font-head-weight);
}
.post-author { overflow-wrap: anywhere; }
.post-time { white-space: nowrap; }
.post-content {
  margin: 0;
  color: var(--text);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.post-content a {
  color: var(--accent-text);
  text-decoration: underline;
}
.post-content a:hover { text-decoration: none; }
.post-card .chip-row { margin-top: var(--space-3); }
.post-tag-text {
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
.post-footer {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
  margin-top: var(--space-3);
}
.post-reply-count {
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
@media (max-width: 820px) {
  .post-avatar {
    width: var(--hit-target-min);
    height: var(--hit-target-min);
    font-size: var(--text-sm);
  }
}
"#;

/// `active_tag` is the tag the list is filtered by, if any: its chip reads as
/// pressed on every card that carries it.
#[component]
pub fn PostCard(
    post: FeedPost,
    on_tag_click: Option<EventHandler<String>>,
    #[props(default)] active_tag: Option<String>,
) -> Element {
    let author = author_name(post.author_name.as_deref());
    let avatar = initials(&author);
    let stamp = post_stamp(post.created_at, local_offset_minutes(post.created_at));
    let tags = unique_tags(&post.hashtags);

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
        crate::styles::StyleOnce { id: "scuffed-css-components-post-card-card-css", css: CARD_CSS }
        article { class: "post-card", aria_label: "Post by {author}",
            p { class: "post-author-line",
                span { class: "post-avatar", aria_hidden: "true", "{avatar}" }
                span { class: "post-author",
                    "{author}"
                    if stamp.is_some() {
                        span { aria_hidden: "true", " ·" }
                    }
                }
                if let Some(stamp) = stamp {
                    time { class: "post-time", datetime: "{stamp.datetime}", "{stamp.text}" }
                }
            }

            p { class: "post-content", {linkify_content(&post.content)} }

            if !tags.is_empty() {
                ChipRow { label: "Tags",
                    for tag in tags.iter() {
                        {render_tag(tag, on_tag_click, active_tag.as_deref())}
                    }
                }
            }

            div { class: "post-footer",
                ReactionBar {
                    event_id: post.id.clone(),
                    event_author_pubkey: post.pubkey.clone(),
                    reactions,
                }
                if post.reply_count > 0 {
                    span { class: "post-reply-count",
                        {pluralize(post.reply_count as usize, "reply", "replies")}
                    }
                }
            }
        }
    }
}

/// The display name, or the fallback when it is missing or blank.
fn author_name(name: Option<&str>) -> String {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(AUTHOR_FALLBACK)
        .to_string()
}

/// Each tag once (any case, first spelling kept), so the chip keys stay
/// unique within the card.
fn unique_tags(tags: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for tag in tags {
        if !seen.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
            seen.push(tag.clone());
        }
    }
    seen
}

/// A tag as a filter chip when the list can be filtered, plain text when not.
/// The chip has no id: nothing points at it, and a tag is free text (it can
/// hold spaces), so it would not make a valid id.
fn render_tag(
    tag: &str,
    handler: Option<EventHandler<String>>,
    active_tag: Option<&str>,
) -> Element {
    let tag = tag.to_string();
    match handler {
        Some(handler) => {
            let pressed = active_tag.is_some_and(|active| active.eq_ignore_ascii_case(&tag));
            let emit = tag.clone();
            rsx! {
                button {
                    key: "{tag}",
                    r#type: "button",
                    class: "chip",
                    aria_pressed: if pressed { "true" } else { "false" },
                    aria_label: "Show posts tagged #{tag}",
                    onclick: move |_| handler.call(emit.clone()),
                    "#{tag}"
                }
            }
        }
        None => rsx! {
            span { key: "{tag}", class: "post-tag-text", "#{tag}" }
        },
    }
}

fn linkify_content(content: &str) -> Element {
    let parts = crate::util::linkify_http_spans(content);
    rsx! {
        for (i, part) in parts.into_iter().enumerate() {
            Fragment { key: "{i}",
                {match part {
                    crate::util::TextLink::Text(text) => {
                        let text = text.to_string();
                        rsx! { "{text}" }
                    }
                    crate::util::TextLink::Href(url) => {
                        let url = url.to_string();
                        let label = url.clone();
                        rsx! {
                            a { href: "{url}", target: "_blank", rel: "noopener", "{label}" }
                        }
                    }
                }}
            }
        }
    }
}

/// The machine value and the visible text of a post's time.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PostStamp {
    /// UTC, for `<time datetime>`.
    datetime: String,
    /// `12 Nov 2026, 17:40` on the viewer's clock.
    text: String,
}

/// `offset_minutes` is the viewer's offset east of UTC at that moment.
/// `None` for a time chrono cannot represent.
fn post_stamp(unix_secs: i64, offset_minutes: i64) -> Option<PostStamp> {
    let utc = chrono::DateTime::from_timestamp(unix_secs, 0)?;
    let local = utc.checked_add_signed(chrono::Duration::minutes(offset_minutes))?;
    Some(PostStamp {
        datetime: utc.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        text: local.format("%-d %b %Y, %H:%M").to_string(),
    })
}

/// Minutes east of UTC on the viewer's clock. Native builds (tests) use UTC.
#[cfg(target_arch = "wasm32")]
fn local_offset_minutes(unix_secs: i64) -> i64 {
    let date = js_sys::Date::new(&(unix_secs as f64 * 1000.0).into());
    let west = date.get_timezone_offset();
    if west.is_finite() { -(west as i64) } else { 0 }
}

#[cfg(not(target_arch = "wasm32"))]
fn local_offset_minutes(_unix_secs: i64) -> i64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ToastProvider;
    use crate::components::ui::test_support::render;

    /// 12 Nov 2026, 17:40 UTC.
    const POSTED: i64 = 1_794_505_200;

    fn post() -> FeedPost {
        FeedPost {
            id: "note1".to_string(),
            pubkey: "9d1c44a0e2".to_string(),
            author_name: Some("Player02".to_string()),
            content: "Customs are on tonight. See https://example.org/lobby".to_string(),
            hashtags: vec!["lfg".to_string(), "customs".to_string()],
            created_at: POSTED,
            reactions: Vec::new(),
            reply_count: 2,
        }
    }

    fn with_filter() -> Element {
        rsx! {
            ToastProvider {
                PostCard {
                    post: post(),
                    on_tag_click: move |_| {},
                    active_tag: Some("LFG".to_string()),
                }
            }
        }
    }

    fn nameless_plain() -> Element {
        let mut p = post();
        p.author_name = Some("  ".to_string());
        p.reply_count = 1;
        rsx! {
            ToastProvider { PostCard { post: p } }
        }
    }

    #[test]
    fn stamp_is_a_date_and_a_time_on_the_viewer_clock() {
        let utc = post_stamp(POSTED, 0).expect("stamp");
        assert_eq!(utc.datetime, "2026-11-12T17:40:00Z");
        assert_eq!(utc.text, "12 Nov 2026, 17:40");

        // One hour east: the visible time moves, the machine value does not.
        let east = post_stamp(POSTED, 60).expect("stamp");
        assert_eq!(east.datetime, "2026-11-12T17:40:00Z");
        assert_eq!(east.text, "12 Nov 2026, 18:40");

        // A west offset can cross midnight; the day has no leading zero.
        let west = post_stamp(1_793_581_200, -120).expect("stamp");
        assert_eq!(west.text, "1 Nov 2026, 23:00");

        assert_eq!(post_stamp(i64::MAX, 0), None);
    }

    #[test]
    fn a_repeated_tag_shows_once() {
        let tags: Vec<String> = ["lfg", "LFG", "customs", "lfg"]
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(unique_tags(&tags), vec!["lfg", "customs"]);
    }

    #[test]
    fn initials_match_the_account_avatar() {
        // The shared helper the articles use, not a copy.
        let prod = include_str!("card.rs").split("mod tests").next().unwrap();
        assert!(
            !prod.contains("fn initials("),
            "a copy of article_author::initials"
        );
        assert!(
            !prod.contains("const AUTHOR_FALLBACK"),
            "a copy of the fallback name"
        );
        assert_eq!(initials("Player02"), "P2");
        assert_eq!(initials("Player03"), "P3");
        assert_eq!(initials("player two"), "PT");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn author_line_is_a_name_and_a_time_never_a_key() {
        let html = render(with_filter);
        assert!(
            html.contains("<article class=\"post-card\" aria-label=\"Post by Player02\">"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"post-avatar\" aria-hidden=\"true\">P2</span>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<time class=\"post-time\" datetime=\"2026-11-12T17:40:00Z\">12 Nov 2026, 17:40</time>"
            ),
            "{html}"
        );
        assert!(!html.contains("9d1c44a0e2"), "raw key leaked: {html}");
        assert!(html.contains(">2 replies</span>"), "{html}");
        assert!(
            html.contains(
                "<a href=\"https://example.org/lobby\" target=\"_blank\" rel=\"noopener\">"
            ),
            "{html}"
        );

        let html = render(nameless_plain);
        assert!(
            html.contains("aria-label=\"Post by Crew member\""),
            "{html}"
        );
        assert!(html.contains(">1 reply</span>"), "{html}");
    }

    #[test]
    fn tags_are_named_pressable_buttons() {
        let html = render(with_filter);
        assert!(
            html.contains("<div class=\"chip-row\" role=\"group\" aria-label=\"Tags\">"),
            "{html}"
        );
        // The active tag is pressed, whatever its case.
        assert!(
            html.contains(
                "<button type=\"button\" class=\"chip\" aria-pressed=\"true\" aria-label=\"Show posts tagged #lfg\">#lfg</button>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" class=\"chip\" aria-pressed=\"false\" aria-label=\"Show posts tagged #customs\">#customs</button>"
            ),
            "{html}"
        );

        // Tags are free text, so the chips carry no id built from them.
        assert!(!html.contains("id=\"post-"), "{html}");

        // No filter handler: plain text, no control.
        let html = render(nameless_plain);
        assert!(
            html.contains("<span class=\"post-tag-text\">#lfg</span>"),
            "{html}"
        );
        assert!(!html.contains("class=\"chip\""), "{html}");
    }

    #[test]
    fn card_css_uses_tokens_and_a_44px_phone_avatar() {
        assert!(!CARD_CSS.contains("rem"), "fixed sizes in the card CSS");
        let phone = CARD_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block")
            .1;
        assert!(phone.contains("width: var(--hit-target-min)"), "{phone}");
        // The phone frame sets the initials one step up with the bigger disc.
        assert!(phone.contains("font-size: var(--text-sm)"), "{phone}");
    }
}
