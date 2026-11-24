use dioxus::prelude::*;
use serde::Serialize;

use crate::components::{Toast, ToastState, use_toast};
use scuffed_api_client::ApiClient;

const QUICK_EMOJIS: &[(&str, &str)] = &[
    ("\u{1f44d}", "thumbs up"),
    ("\u{2764}\u{fe0f}", "heart"),
    ("\u{1f602}", "laugh"),
    ("\u{1f62e}", "wow"),
    ("\u{1f525}", "fire"),
    ("\u{1f389}", "party"),
    ("\u{1f44e}", "thumbs down"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct ReactionCount {
    pub emoji: String,
    pub count: u32,
    pub reacted_by_me: bool,
}

#[derive(Serialize)]
struct ReactBody {
    event_id: String,
    event_author_pubkey: String,
    content: String,
}

/// Reactions are the shared `chip` (44px, selected is fill plus border plus
/// weight). Only the layout and the emoji size are set here. The picker opens
/// as a row inside the bar, so it cannot run off a narrow screen.
const REACTION_CSS: &str = r#"
.reaction-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
.reaction-pill__emoji,
.reaction-pick {
  font-size: var(--text-lg);
}
.reaction-pill__count {
  font-variant-numeric: tabular-nums;
}
/* Your own reaction: drawn selected, but it is a count, not a control. */
.reaction-pill.is-mine {
  cursor: default;
}
.reaction-picker {
  flex: 1 0 100%;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
"#;

/// What your own reaction reads as. There is no un-react, so it is not a
/// toggle: no `aria-pressed`, and not a button that would add another.
fn mine_text(emoji: &str, count: u32) -> String {
    if count == 1 {
        format!("{emoji}, 1 reaction, yours")
    } else {
        format!("{emoji}, {count} reactions, including yours")
    }
}

/// Accessible name of a reaction chip: what a press does, then the count.
fn pill_name(emoji: &str, count: u32) -> String {
    if count == 1 {
        format!("React with {emoji}, 1 reaction")
    } else {
        format!("React with {emoji}, {count} reactions")
    }
}

/// Id of the picker row of one post. It comes from the event id, so two posts
/// on a page never share it.
fn picker_id(event_id: &str) -> String {
    format!("reaction-picker-{event_id}")
}

#[component]
pub fn ReactionBar(
    event_id: String,
    event_author_pubkey: String,
    reactions: Vec<ReactionCount>,
) -> Element {
    let mut picker_open = use_signal(|| false);
    let toasts = use_toast();
    let is_open = picker_open();
    let picker = picker_id(&event_id);

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-chat-reaction_bar-reaction-css", css: REACTION_CSS }
        div { class: "reaction-bar", role: "group", aria_label: "Reactions",
            for r in reactions.iter() {
                {render_pill(
                    r.clone(),
                    event_id.clone(),
                    event_author_pubkey.clone(),
                    toasts,
                )}
            }
            button {
                r#type: "button",
                class: "chip reaction-add",
                aria_label: "Add reaction",
                aria_expanded: if is_open { "true" } else { "false" },
                aria_controls: is_open.then(|| picker.clone()),
                onclick: move |_| picker_open.toggle(),
                span { aria_hidden: "true", "+" }
            }
            if is_open {
                div {
                    class: "reaction-picker",
                    id: "{picker}",
                    role: "group",
                    aria_label: "Pick a reaction",
                    for &(emoji, label) in QUICK_EMOJIS.iter() {
                        {render_picker_btn(
                            emoji,
                            label,
                            event_id.clone(),
                            event_author_pubkey.clone(),
                            toasts,
                            picker_open,
                        )}
                    }
                }
            }
        }
    }
}

fn render_pill(r: ReactionCount, event_id: String, author: String, toasts: ToastState) -> Element {
    let emoji = r.emoji.clone();
    if r.reacted_by_me {
        let text = mine_text(&r.emoji, r.count);
        return rsx! {
            span { class: "chip active reaction-pill is-mine",
                span { class: "visually-hidden", "{text}" }
                span { class: "reaction-pill__emoji", aria_hidden: "true", "{r.emoji}" }
                span { class: "reaction-pill__count", aria_hidden: "true", "{r.count}" }
            }
        };
    }
    let name = pill_name(&r.emoji, r.count);

    rsx! {
        button {
            r#type: "button",
            class: "chip reaction-pill",
            aria_label: "{name}",
            onclick: move |_| {
                let event_id = event_id.clone();
                let author = author.clone();
                let emoji = emoji.clone();
                let toasts = toasts;
                spawn(async move {
                    send_reaction(&event_id, &author, &emoji, toasts).await;
                });
            },
            span { class: "reaction-pill__emoji", aria_hidden: "true", "{r.emoji}" }
            span { class: "reaction-pill__count", aria_hidden: "true", "{r.count}" }
        }
    }
}

fn render_picker_btn(
    emoji: &str,
    label: &str,
    event_id: String,
    author: String,
    toasts: ToastState,
    mut picker_open: Signal<bool>,
) -> Element {
    let emoji_owned = emoji.to_string();

    rsx! {
        button {
            r#type: "button",
            class: "chip reaction-pick",
            aria_label: "React with {label}",
            onclick: move |_| {
                let event_id = event_id.clone();
                let author = author.clone();
                let emoji = emoji_owned.clone();
                let toasts = toasts;
                picker_open.set(false);
                spawn(async move {
                    send_reaction(&event_id, &author, &emoji, toasts).await;
                });
            },
            span { aria_hidden: "true", "{emoji}" }
        }
    }
}

async fn send_reaction(event_id: &str, author_pubkey: &str, content: &str, mut toasts: ToastState) {
    let body = ReactBody {
        event_id: event_id.to_string(),
        event_author_pubkey: author_pubkey.to_string(),
        content: content.to_string(),
    };

    match ApiClient::web()
        .post_json::<_, serde_json::Value>("/api/nostr/react", &body)
        .await
    {
        Ok(_) => {}
        Err(e) => {
            // The one mapping says why (and logs it): an unreadable answer
            // may mean the reaction is in, so no blind "try again".
            toasts.show(Toast::error_from("Couldn't add the reaction.", &e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ToastProvider;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

    fn bar() -> Element {
        rsx! {
            ToastProvider {
                ReactionBar {
                    event_id: "ev01",
                    event_author_pubkey: "pk01",
                    reactions: vec![
                        ReactionCount { emoji: "\u{1f44d}".into(), count: 3, reacted_by_me: true },
                        ReactionCount { emoji: "\u{1f525}".into(), count: 1, reacted_by_me: false },
                    ],
                }
            }
        }
    }

    /// The bar only; the toast regions of the provider come after it.
    fn render() -> String {
        let mut dom = VirtualDom::new(bar);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        let end = html.find("<div class=\"toast-container\"").expect("toasts");
        html[..end].to_string()
    }

    #[test]
    fn every_reaction_control_has_a_name_and_a_state() {
        let html = render();
        assert!(
            html.contains("<div class=\"reaction-bar\" role=\"group\" aria-label=\"Reactions\">"),
            "{html}"
        );
        // Your own reaction is shown selected but is not a control: a press
        // would only add another, so it is not a toggle either.
        assert!(
            html.contains(
                "<span class=\"chip active reaction-pill is-mine\"><span class=\"visually-hidden\">\u{1f44d}, 3 reactions, including yours</span>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<button type=\"button\" class=\"chip reaction-pill\" aria-label=\"React with \u{1f525}, 1 reaction\">"),
            "{html}"
        );
        assert!(!html.contains("aria-pressed"), "{html}");
        assert!(
            html.contains(
                "<button type=\"button\" class=\"chip reaction-add\" aria-label=\"Add reaction\" aria-expanded=\"false\">"
            ),
            "{html}"
        );
        assert_eq!(html.matches("<button").count(), 2, "{html}");
        assert_eq!(
            html.matches("<button type=\"button\" class=\"chip ")
                .count(),
            2,
            "every control is the shared chip: {html}"
        );
        assert!(
            !html.contains("reaction-picker"),
            "closed by default: {html}"
        );
        assert!(!html.contains("style="), "{html}");
        assert!(!html.contains("<a "), "{html}");
    }

    #[test]
    fn names_and_ids_come_from_the_data() {
        assert_eq!(pill_name("x", 1), "React with x, 1 reaction");
        assert_eq!(pill_name("x", 0), "React with x, 0 reactions");
        assert_eq!(pill_name("x", 12), "React with x, 12 reactions");
        assert_eq!(mine_text("x", 1), "x, 1 reaction, yours");
        assert_eq!(mine_text("x", 4), "x, 4 reactions, including yours");
        assert_eq!(picker_id("ev01"), "reaction-picker-ev01");
        assert_ne!(picker_id("ev01"), picker_id("ev02"));
        for (emoji, label) in QUICK_EMOJIS {
            assert!(!emoji.is_empty() && !label.is_empty());
        }
    }

    #[test]
    fn reaction_css_is_on_tokens_and_the_picker_stays_in_flow() {
        assert_sizes_from_tokens(REACTION_CSS);
        assert!(!REACTION_CSS.contains("position: absolute"));
        assert!(!REACTION_CSS.contains("color-mix"));
        let picker = css_rule(REACTION_CSS, ".reaction-picker {");
        assert!(has_decl(&picker, "flex: 1 0 100%"), "{picker}");
        assert!(has_decl(&picker, "flex-wrap: wrap"), "{picker}");
    }
}
