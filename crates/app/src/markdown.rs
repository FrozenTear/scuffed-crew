//! Markdown to HTML for article and wiki bodies.
//!
//! Safety rules, since the output goes into `dangerous_inner_html`:
//! - Raw HTML in the source (block or inline) is escaped and shown as text.
//! - Link and image URLs must be relative, a fragment, or use http, https or
//!   mailto. Anything else (`javascript:`, `data:`, `vbscript:` and so on) is
//!   replaced with `#`.
//! - Text and attribute values are escaped by pulldown-cmark's HTML writer.
//!
//! Headings are shifted down one level (`#` becomes `h2`) because the page
//! title is already the `h1`.

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

/// Render untrusted markdown to HTML that is safe to inject.
pub fn render_markdown(source: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events = Parser::new_ext(source, options).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        Event::Start(Tag::Heading {
            level,
            id,
            classes,
            attrs,
        }) => Event::Start(Tag::Heading {
            level: demote(level),
            id,
            classes,
            attrs,
        }),
        Event::End(TagEnd::Heading(level)) => Event::End(TagEnd::Heading(demote(level))),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: safe_url(dest_url),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: safe_url(dest_url),
            title,
            id,
        }),
        other => other,
    });
    let mut out = String::with_capacity(source.len() * 3 / 2);
    html::push_html(&mut out, events);
    out
}

fn demote(level: HeadingLevel) -> HeadingLevel {
    match level {
        HeadingLevel::H1 => HeadingLevel::H2,
        HeadingLevel::H2 => HeadingLevel::H3,
        HeadingLevel::H3 => HeadingLevel::H4,
        HeadingLevel::H4 => HeadingLevel::H5,
        HeadingLevel::H5 | HeadingLevel::H6 => HeadingLevel::H6,
    }
}

fn safe_url(url: CowStr<'_>) -> CowStr<'_> {
    if is_safe_url(&url) {
        url
    } else {
        CowStr::Borrowed("#")
    }
}

/// Relative paths, fragments, and http/https/mailto are allowed. A scheme is
/// whatever precedes the first `:` when no `/`, `?` or `#` comes before it.
fn is_safe_url(url: &str) -> bool {
    // Browsers drop ASCII whitespace and control characters inside a scheme
    // (`java\tscript:`), so judge the URL with them removed.
    let cleaned: String = url
        .trim()
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_ascii_control())
        .collect();
    match cleaned.find([':', '/', '?', '#']) {
        Some(i) if cleaned.as_bytes()[i] == b':' => {
            let scheme = cleaned[..i].to_ascii_lowercase();
            matches!(scheme.as_str(), "http" | "https" | "mailto")
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_heading_bold_and_list() {
        let html = render_markdown("# Patch day\n\nSome **bold** text.\n\n- one\n- two\n");
        assert!(html.contains("<h2>Patch day</h2>"), "{html}");
        assert!(html.contains("<strong>bold</strong>"), "{html}");
        assert!(html.contains("<li>one</li>"), "{html}");
        assert!(!html.contains("# Patch day"), "{html}");
        assert!(!html.contains("**"), "{html}");
    }

    #[test]
    fn headings_shift_down_one_level_and_cap_at_h6() {
        let html = render_markdown("## Two\n\n###### Six\n");
        assert!(html.contains("<h3>Two</h3>"), "{html}");
        assert!(html.contains("<h6>Six</h6>"), "{html}");
        assert!(!html.contains("<h1"), "{html}");
    }

    #[test]
    fn raw_html_is_escaped_not_injected() {
        let html = render_markdown(
            "<script>alert(1)</script>\n\nInline <img src=x onerror=alert(1)> here.\n",
        );
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn unsafe_link_schemes_are_neutralised() {
        for src in [
            "[x](javascript:alert(1))",
            "[x](JaVaScript:alert(1))",
            "[x](<java\tscript:alert(1)>)",
            "[x](data:text/html,hi)",
            "![x](vbscript:msgbox)",
        ] {
            let html = render_markdown(src);
            assert!(
                !html.to_ascii_lowercase().contains("script:"),
                "{src} -> {html}"
            );
            assert!(!html.contains("data:"), "{src} -> {html}");
            assert!(html.contains("=\"#\""), "{src} -> {html}");
        }
    }

    #[test]
    fn safe_links_are_kept() {
        let html = render_markdown(
            "[a](https://example.com) [b](/wiki/rules) [c](#top) [d](mailto:a@b.c) [e](page?x=1:2)",
        );
        assert!(html.contains("href=\"https://example.com\""), "{html}");
        assert!(html.contains("href=\"/wiki/rules\""), "{html}");
        assert!(html.contains("href=\"#top\""), "{html}");
        assert!(html.contains("href=\"mailto:a@b.c\""), "{html}");
        assert!(html.contains("href=\"page?x=1:2\""), "{html}");
    }

    #[test]
    fn attribute_values_are_escaped() {
        let html = render_markdown("[x](https://e.com/\"onmouseover=\"alert(1) \"t\")");
        assert!(!html.contains("\"onmouseover"), "{html}");
    }
}
