//! The full-height chat thread of the `d1-chat` frame: a head bar with the
//! page's H1, the connection status and a toolbar, then a body that takes the
//! rest of the height.
//!
//! [`ThreadHead`] is shared by the live widget and by the page states that
//! have no widget yet (loading, no teams, no channels), so the head bar does
//! not move when the room connects.

use dioxus::prelude::*;

use crate::components::ui::PageHeader;

/// Inner gutters use `--space-card-pad`: 16px at the default density, as the
/// frame draws them, and it follows the density setting.
pub const THREAD_CSS: &str = r#"
.chat-thread {
  box-sizing: border-box;
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--surface);
  color: var(--text);
  font-family: var(--font-body);
}
.chat-thread__head {
  box-sizing: border-box;
  flex: 0 0 auto;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
  min-height: var(--hit-target-min);
  padding: var(--space-2) var(--space-card-pad);
  border-bottom: 1px solid var(--border);
}
.chat-thread__head-main {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
/* The frame sets the room name at the thread head size, not the page size. */
.chat-thread__head .page-header h1 {
  font-size: var(--text-lg);
}
.chat-thread__toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
  max-width: 100%;
}
.chat-thread__team {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
  max-width: 100%;
}
.chat-thread__team .field-input {
  width: auto;
  min-width: 0;
  max-width: 100%;
}
/* Page states with no room yet: a wall, an empty card or a status line. */
.chat-thread__body {
  box-sizing: border-box;
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--space-stack);
  padding: var(--space-section) var(--space-page);
  background: var(--bg);
}
.chat-thread__note {
  margin: 0;
  color: var(--text-3);
  font-size: var(--text-md);
  line-height: var(--leading-body);
}
"#;

pub const THREAD_CSS_ID: &str = "scuffed-css-components-chat-thread-css";

/// Accessible name of the thread region ("Squad Alpha chat").
pub fn thread_label(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() || title.eq_ignore_ascii_case("chat") {
        "Chat".to_string()
    } else if title.to_ascii_lowercase().ends_with(" chat") {
        title.to_string()
    } else {
        format!("{title} chat")
    }
}

/// Head bar of the thread. `title` is the page's one H1.
#[component]
pub fn ThreadHead(
    title: String,
    /// The connection status line under the title.
    #[props(default)]
    status: Option<Element>,
    /// Controls at the end of the bar (the team picker).
    #[props(default)]
    toolbar: Option<Element>,
) -> Element {
    rsx! {
        crate::styles::StyleOnce { id: THREAD_CSS_ID, css: THREAD_CSS }
        div { class: "chat-thread__head",
            div { class: "chat-thread__head-main",
                PageHeader { title }
                if let Some(status) = status {
                    {status}
                }
            }
            if let Some(toolbar) = toolbar {
                div { class: "chat-thread__toolbar", {toolbar} }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

    #[component]
    fn Probe(with_extras: bool) -> Element {
        rsx! {
            ThreadHead {
                title: "Squad Alpha",
                status: with_extras.then(|| rsx! { p { class: "probe-status", "Connected" } }),
                toolbar: with_extras.then(|| rsx! { span { class: "probe-tool", "Team" } }),
            }
        }
    }

    fn render(with_extras: bool) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, ProbeProps { with_extras });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn head_holds_the_one_h1_then_the_status_then_the_toolbar() {
        let html = render(true);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Squad Alpha</h1>"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        let h1 = html.find("<h1").expect("h1");
        let status = html.find("probe-status").expect("status");
        let tool = html.find("probe-tool").expect("toolbar");
        assert!(h1 < status && status < tool, "{html}");
        assert!(html.contains("class=\"chat-thread__toolbar\""), "{html}");
    }

    #[test]
    fn head_without_extras_renders_no_empty_toolbar() {
        let html = render(false);
        assert!(html.contains("<h1>Squad Alpha</h1>"), "{html}");
        assert!(!html.contains("chat-thread__toolbar"), "{html}");
    }

    #[test]
    fn region_name_reads_as_a_room() {
        assert_eq!(thread_label("Squad Alpha"), "Squad Alpha chat");
        assert_eq!(thread_label("Team chat"), "Team chat");
        assert_eq!(thread_label("Chat"), "Chat");
        assert_eq!(thread_label("  "), "Chat");
    }

    #[test]
    fn thread_css_is_on_tokens_and_fills_the_height() {
        assert_sizes_from_tokens(THREAD_CSS);
        let thread = css_rule(THREAD_CSS, ".chat-thread {");
        for decl in ["flex: 1 1 auto", "min-height: 0", "flex-direction: column"] {
            assert!(has_decl(&thread, decl), "missing {decl}: {thread}");
        }
        let head = css_rule(THREAD_CSS, ".chat-thread__head {");
        for decl in [
            "flex: 0 0 auto",
            "flex-wrap: wrap",
            "min-height: var(--hit-target-min)",
            "padding: var(--space-2) var(--space-card-pad)",
            "border-bottom: 1px solid var(--border)",
        ] {
            assert!(has_decl(&head, decl), "missing {decl}: {head}");
        }
        let body = css_rule(THREAD_CSS, ".chat-thread__body {");
        assert!(has_decl(&body, "overflow-y: auto"), "{body}");
        assert!(
            has_decl(&body, "padding: var(--space-section) var(--space-page)"),
            "{body}"
        );
    }
}
