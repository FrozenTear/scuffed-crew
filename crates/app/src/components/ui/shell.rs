//! Page body container and the shared page header.
//!
//! The site shell owns `<main id="main">`. A page body starts with
//! [`PageShell`], which is a `div`: it centres the content at one of the three
//! content widths, pads it with `--space-page`, and spaces its direct children
//! with `--space-section`. [`PageHeader`] is the one H1 of the page plus an
//! optional lead.

use dioxus::prelude::*;

/// Content width of a page body (`ux-spec/layouts.md`, content width table).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageWidth {
    /// 720: long reads (articles, patch notes, forms).
    Narrow,
    /// 1040: most public pages.
    #[default]
    Default,
    /// 1280: tables and stats.
    Wide,
}

impl PageWidth {
    /// Modifier class next to `ui-shell`. Empty for the default width.
    pub const fn modifier(self) -> &'static str {
        match self {
            PageWidth::Narrow => "ui-shell--narrow",
            PageWidth::Default => "",
            PageWidth::Wide => "ui-shell--wide",
        }
    }
}

/// `ui-shell`, the width modifier and the page's own class, in that order.
fn shell_class(width: PageWidth, extra: &str) -> String {
    ["ui-shell", width.modifier(), extra.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub const SHELL_CSS: &str = r#"
.ui-shell {
  box-sizing: border-box;
  width: 100%;
  max-width: var(--content-max-default);
  min-width: 0;
  margin: 0 auto;
  padding: var(--space-section) var(--space-page);
  display: flex;
  flex-direction: column;
  gap: var(--space-section);
}
.ui-shell--narrow { max-width: var(--content-max-narrow); }
.ui-shell--wide { max-width: var(--content-max-wide); }
/* Flex children default to min-width auto, so a wide table would push the
   page sideways. Zero specificity: a page rule still wins. */
:where(.ui-shell > *) { min-width: 0; }
/* A block inside the page: heading plus content, one stack step apart. */
.ui-stack {
  display: flex;
  flex-direction: column;
  gap: var(--space-stack);
  min-width: 0;
}

.page-header { margin: 0; min-width: 0; }
.page-header h1 {
  margin: 0;
  font-family: var(--font-head);
  font-weight: var(--font-head-weight);
  letter-spacing: var(--font-head-tracking);
  text-transform: var(--font-head-transform);
  line-height: var(--leading-display);
  font-size: var(--text-3xl);
  color: var(--text);
  overflow-wrap: anywhere;
  text-box-trim: trim-both;
  text-box-edge: cap alphabetic;
}
.page-lead {
  margin: var(--space-2) 0 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
  max-width: var(--content-max-narrow);
}
/* The H2 of a block inside the page ("Schedule", "House rules"). The name
   is prefixed: the stats pages have a `section-title` of their own. */
.ui-section-title {
  margin: 0;
  font-family: var(--font-head);
  font-weight: var(--font-head-weight);
  letter-spacing: var(--font-head-tracking);
  text-transform: var(--font-head-transform);
  line-height: var(--leading-display);
  font-size: var(--text-xl);
  color: var(--text);
  overflow-wrap: anywhere;
}
.page-header-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin-top: var(--space-4);
}
@supports not (text-box-trim: trim-both) {
  .page-header h1 { transform: translateY(0.04em); }
}
"#;

/// The page body container. Renders a `div`, never `main`.
///
/// `class` is the page's own class for page-scoped CSS (`members-page`).
#[component]
pub fn PageShell(
    #[props(default)] width: PageWidth,
    #[props(default)] class: String,
    children: Element,
) -> Element {
    let class = shell_class(width, &class);
    rsx! { div { class: "{class}", {children} } }
}

/// The page's single H1, an optional lead, an optional meta line and an
/// optional action row.
#[component]
pub fn PageHeader(
    title: String,
    #[props(default)] lead: Option<String>,
    /// A line between the title and the actions (an article's byline).
    #[props(default)]
    meta: Option<Element>,
    /// Buttons or links under the lead ("Log in to start a thread").
    #[props(default)]
    actions: Option<Element>,
) -> Element {
    rsx! {
        header { class: "page-header",
            h1 { "{title}" }
            if let Some(lead) = lead {
                p { class: "page-lead", "{lead}" }
            }
            if let Some(meta) = meta {
                {meta}
            }
            if let Some(actions) = actions {
                div { class: "page-header-actions", {actions} }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{
        assert_sizes_from_tokens, css_rule, has_decl, render,
    };

    #[test]
    fn shell_is_a_div_at_the_default_width() {
        fn app() -> Element {
            rsx! { PageShell { p { "body" } } }
        }
        let html = render(app);
        assert_eq!(html, "<div class=\"ui-shell\"><p>body</p></div>");
        assert!(!html.contains("<main"), "{html}");
    }

    #[test]
    fn shell_takes_a_width_and_the_page_class() {
        fn app() -> Element {
            rsx! {
                PageShell { width: PageWidth::Narrow, class: "blog-post-page", "a" }
                PageShell { width: PageWidth::Wide, "b" }
                PageShell { class: " news-page ", "c" }
            }
        }
        let html = render(app);
        assert!(
            html.contains("<div class=\"ui-shell ui-shell--narrow blog-post-page\">a</div>"),
            "{html}"
        );
        assert!(
            html.contains("<div class=\"ui-shell ui-shell--wide\">b</div>"),
            "{html}"
        );
        assert!(
            html.contains("<div class=\"ui-shell news-page\">c</div>"),
            "{html}"
        );
    }

    #[test]
    fn widths_and_gutters_come_from_tokens() {
        let base = css_rule(SHELL_CSS, ".ui-shell {");
        for decl in [
            "max-width: var(--content-max-default)",
            "padding: var(--space-section) var(--space-page)",
            "gap: var(--space-section)",
            "box-sizing: border-box",
            "margin: 0 auto",
            "min-width: 0",
        ] {
            assert!(has_decl(&base, decl), "missing {decl}: {base}");
        }
        assert!(has_decl(
            &css_rule(SHELL_CSS, ".ui-shell--narrow {"),
            "max-width: var(--content-max-narrow)"
        ));
        assert!(has_decl(
            &css_rule(SHELL_CSS, ".ui-shell--wide {"),
            "max-width: var(--content-max-wide)"
        ));
        assert!(has_decl(
            &css_rule(SHELL_CSS, ".ui-stack {"),
            "gap: var(--space-stack)"
        ));
    }

    #[test]
    fn header_has_one_h1_and_an_optional_lead() {
        fn app() -> Element {
            rsx! {
                PageHeader { title: "Members", lead: "The public roster." }
            }
        }
        let html = render(app);
        assert_eq!(
            html,
            "<header class=\"page-header\"><h1>Members</h1>\
             <p class=\"page-lead\">The public roster.</p></header>"
        );

        fn bare() -> Element {
            rsx! { PageHeader { title: "News" } }
        }
        let html = render(bare);
        assert_eq!(html, "<header class=\"page-header\"><h1>News</h1></header>");
    }

    #[test]
    fn header_actions_sit_under_the_lead() {
        fn app() -> Element {
            rsx! {
                PageHeader {
                    title: "General",
                    lead: "Open talk.",
                    actions: rsx! { a { href: "/login", "Log in" } },
                }
            }
        }
        let html = render(app);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        let lead = html.find("page-lead").expect("lead");
        let actions = html.find("page-header-actions").expect("actions");
        assert!(lead < actions, "{html}");
        assert!(html.contains("<a href=\"/login\">Log in</a>"), "{html}");
    }

    #[test]
    fn header_meta_sits_between_the_title_and_the_actions() {
        fn app() -> Element {
            rsx! {
                PageHeader {
                    title: "Tank basics",
                    meta: rsx! { p { class: "article-byline", "Player03" } },
                    actions: rsx! { button { r#type: "button", "Edit" } },
                }
            }
        }
        let html = render(app);
        assert_eq!(
            html,
            "<header class=\"page-header\"><h1>Tank basics</h1>\
             <p class=\"article-byline\">Player03</p>\
             <div class=\"page-header-actions\"><button type=\"button\">Edit</button></div></header>"
        );
    }

    #[test]
    fn header_type_comes_from_tokens() {
        assert_sizes_from_tokens(SHELL_CSS);
        let h1 = css_rule(SHELL_CSS, ".page-header h1 {");
        for decl in [
            "font-family: var(--font-head)",
            "font-size: var(--text-3xl)",
            "font-weight: var(--font-head-weight)",
            "color: var(--text)",
            "margin: 0",
        ] {
            assert!(has_decl(&h1, decl), "missing {decl}: {h1}");
        }
        let section = css_rule(SHELL_CSS, ".ui-section-title {");
        for decl in [
            "font-family: var(--font-head)",
            "font-size: var(--text-xl)",
            "font-weight: var(--font-head-weight)",
            "margin: 0",
        ] {
            assert!(has_decl(&section, decl), "missing {decl}: {section}");
        }
        let lead = css_rule(SHELL_CSS, ".page-lead {");
        assert!(has_decl(&lead, "color: var(--text-3)"), "{lead}");
        assert!(has_decl(&lead, "font-size: var(--text-md)"), "{lead}");
        assert!(
            has_decl(&lead, "max-width: var(--content-max-narrow)"),
            "{lead}"
        );
    }
}
