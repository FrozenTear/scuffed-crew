//! The feed composer: one text box, an optional tag line and Post.
//!
//! The post is shown in the list at once and sent in the background. If the
//! send fails the caller is told (so it can take the post out again), the
//! draft goes back into the box and an error shows under the composer.

use dioxus::prelude::*;
use serde::Serialize;

use super::card::FeedPost;
use crate::components::ui::FieldError;
use crate::components::{Toast, use_toast};
use crate::util::log_client_error;
use scuffed_api_client::ApiClient;

#[derive(Serialize)]
struct CreatePostBody {
    content: String,
    hashtags: Vec<String>,
}

/// The text box. The error line under the composer points at it.
const TEXT_ID: &str = "feed-compose";
const TAGS_ID: &str = "feed-compose-tags";
const TAGS_HINT_ID: &str = "feed-compose-tags-hint";
const ERROR_ID: &str = "feed-compose-error";

const EMPTY_ERROR: &str = "Write something before you post.";
const SEND_ERROR: &str = "Couldn't publish your post. Check your connection and try again.";

const COMPOSE_CSS: &str = r#"
.post-compose {
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
  padding: var(--space-card-pad);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.post-compose-tags {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
.post-compose-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
"#;

/// `#lfg, customs` as `["lfg", "customs"]`: split on commas, trimmed, without
/// the leading `#`, blanks dropped.
fn parse_tags(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|t| t.trim().trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Only an empty box is a fault in the text box. A failed send is not, so it
/// shows under the composer without marking the box invalid.
fn text_is_invalid(error: &str) -> bool {
    error == EMPTY_ERROR
}

/// `on_post` gets the post as soon as Post is pressed. `on_failed` gets that
/// post's id when the server did not take it.
#[component]
pub fn PostCompose(
    on_post: EventHandler<FeedPost>,
    #[props(default)] on_failed: Option<EventHandler<String>>,
) -> Element {
    let mut content = use_signal(String::new);
    let mut tags_input = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut error = use_signal(String::new);
    let mut toasts = use_toast();

    let handle_submit = move |_| {
        if submitting() {
            return;
        }
        let text = content().trim().to_string();
        if text.is_empty() {
            error.set(EMPTY_ERROR.to_string());
            return;
        }
        error.set(String::new());

        let tags_text = tags_input();
        let hashtags = parse_tags(&tags_text);

        let pending_id = format!("pending-{}", js_sys::Date::now() as u64);
        let optimistic = FeedPost {
            id: pending_id.clone(),
            pubkey: String::new(),
            author_name: Some("You".to_string()),
            content: text.clone(),
            hashtags: hashtags.clone(),
            created_at: (js_sys::Date::now() / 1000.0) as i64,
            reactions: vec![],
            reply_count: 0,
        };
        on_post.call(optimistic);

        content.set(String::new());
        tags_input.set(String::new());

        // Set before the task starts, so a second click in the same frame is
        // ignored too.
        submitting.set(true);
        spawn(async move {
            let body = CreatePostBody {
                content: text.clone(),
                hashtags,
            };
            match ApiClient::web()
                .post_json::<_, serde_json::Value>("/api/nostr/post", &body)
                .await
            {
                Ok(_) => {
                    toasts.show(Toast::success("Post published"));
                }
                Err(e) => {
                    log_client_error("feed post", &e);
                    if let Some(on_failed) = on_failed {
                        on_failed.call(pending_id);
                    }
                    // Hand the draft back unless a new one was started.
                    if content.peek().trim().is_empty() {
                        content.set(text);
                        tags_input.set(tags_text);
                    }
                    error.set(SEND_ERROR.to_string());
                    toasts.show(Toast::error("Couldn't publish your post"));
                }
            }
            submitting.set(false);
        });
    };

    let busy = submitting();
    let message = error();
    let invalid = text_is_invalid(&message);

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-post-compose-compose-css", css: COMPOSE_CSS }
        section { class: "post-compose", aria_label: "New post",
            textarea {
                id: TEXT_ID,
                class: "field-input",
                aria_label: "Write a post",
                aria_describedby: ERROR_ID,
                aria_invalid: if invalid { "true" },
                placeholder: "Share a short update with the crew.",
                value: "{content}",
                oninput: move |e| {
                    content.set(e.value());
                    if !error.peek().is_empty() {
                        error.set(String::new());
                    }
                },
            }
            div { class: "post-compose-tags",
                label { class: "field-label", r#for: TAGS_ID, "Tags (optional)" }
                input {
                    id: TAGS_ID,
                    class: "field-input",
                    r#type: "text",
                    autocomplete: "off",
                    aria_describedby: TAGS_HINT_ID,
                    value: "{tags_input}",
                    oninput: move |e| tags_input.set(e.value()),
                }
                p { class: "field-hint", id: TAGS_HINT_ID,
                    "Separate tags with commas, for example lfg, customs."
                }
            }
            // Always in the tree, so a later error is announced.
            FieldError { id: ERROR_ID, text: message, role: "alert" }
            div { class: "post-compose-actions",
                // Never natively disabled: a disabled button drops keyboard
                // focus. While a post is sent it is busy and ignores clicks.
                button {
                    r#type: "button",
                    class: "btn is-primary",
                    aria_disabled: if busy { "true" },
                    onclick: handle_submit,
                    if busy { "Posting..." } else { "Post" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ToastProvider;
    use crate::components::ui::test_support::render;

    fn composer() -> Element {
        rsx! {
            ToastProvider {
                PostCompose { on_post: move |_| {} }
            }
        }
    }

    #[test]
    fn tags_are_split_trimmed_and_unprefixed() {
        assert_eq!(parse_tags("#lfg, customs ,, "), vec!["lfg", "customs"]);
        assert!(parse_tags("  ").is_empty());
    }

    #[test]
    fn every_control_has_a_name_and_the_error_line_is_always_there() {
        let html = render(composer);
        assert!(
            html.contains("<section class=\"post-compose\" aria-label=\"New post\">"),
            "{html}"
        );
        assert!(
            html.contains(
                "<textarea id=\"feed-compose\" class=\"field-input\" aria-label=\"Write a post\" aria-describedby=\"feed-compose-error\""
            ),
            "{html}"
        );
        assert!(
            html.contains("placeholder=\"Share a short update with the crew.\""),
            "{html}"
        );
        assert!(
            html.contains(
                "<label class=\"field-label\" for=\"feed-compose-tags\">Tags (optional)</label>"
            ),
            "{html}"
        );
        assert!(
            html.contains("aria-describedby=\"feed-compose-tags-hint\""),
            "{html}"
        );
        // Empty, with its role, before any error.
        assert!(
            html.contains("<p class=\"field-error\" id=\"feed-compose-error\" role=\"alert\"></p>"),
            "{html}"
        );
        assert!(!html.contains("aria-invalid"), "{html}");
        assert!(
            html.contains("<button type=\"button\" class=\"btn is-primary\">Post</button>"),
            "{html}"
        );
        assert!(!html.contains(" disabled"), "{html}");
    }

    #[test]
    fn only_an_empty_box_marks_the_text_invalid() {
        assert!(text_is_invalid(EMPTY_ERROR));
        assert!(!text_is_invalid(SEND_ERROR));
        assert!(!text_is_invalid(""));
    }

    #[test]
    fn composer_css_uses_tokens_only() {
        assert!(
            !COMPOSE_CSS.contains("rem"),
            "fixed sizes in the composer CSS"
        );
        assert!(COMPOSE_CSS.contains("padding: var(--space-card-pad)"));
    }
}
