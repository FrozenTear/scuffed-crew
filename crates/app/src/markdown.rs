//! Markdown to HTML for article and wiki bodies.
//!
//! Safety rules, since the output goes into `dangerous_inner_html`:
//! - Raw HTML in the source (block or inline) is escaped and shown as text.
//! - Link and image URLs must be relative, a fragment, or use http, https or
//!   mailto. Anything else (`javascript:`, `data:`, `vbscript:` and so on) is
//!   replaced with `#`.
//! - Text and attribute values are escaped by pulldown-cmark's HTML writer.
//! - A fenced code block gets `class="language-<x>"` only when the first word
//!   of its info string is `[A-Za-z0-9_+-]+`; otherwise it gets no class.
//! - Headings never carry an id, classes or other attributes.
//!
//! Headings are shifted down one level (`#` becomes `h2`) because the page
//! title is already the `h1`.

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html,
};

/// Render untrusted markdown to HTML that is safe to inject.
pub fn render_markdown(source: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events = Parser::new_ext(source, options).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        // Heading attributes are off in `options`; clear them anyway so
        // turning that option on later cannot let authors set ids or classes.
        Event::Start(Tag::Heading { level, .. }) => Event::Start(Tag::Heading {
            level: demote(level),
            id: None,
            classes: Vec::new(),
            attrs: Vec::new(),
        }),
        Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(code_lang(&info))))
        }
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

/// Language for a fenced block's class: the first word of the info string if
/// it is only `[A-Za-z0-9_+-]`, else empty (the writer then emits no class).
fn code_lang(info: &str) -> CowStr<'static> {
    let word = info.split_ascii_whitespace().next().unwrap_or("");
    let ok = word
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'));
    if ok {
        CowStr::from(word.to_string())
    } else {
        CowStr::Borrowed("")
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

    fn assert_neutralised(src: &str) {
        let html = render_markdown(src);
        assert!(
            html.contains("href=\"#\"") || html.contains("src=\"#\""),
            "{src} -> {html}"
        );
        assert!(
            !html.to_ascii_lowercase().contains("=\"javascript"),
            "{src} -> {html}"
        );
    }

    #[test]
    fn autolinks_are_checked() {
        let html = render_markdown("<https://example.com/a> <mailto:a@b.c>");
        assert!(html.contains("href=\"https://example.com/a\""), "{html}");
        assert!(html.contains("href=\"mailto:a@b.c\""), "{html}");
        assert_neutralised("<javascript:alert(1)>");
        assert_neutralised("<JAVASCRIPT:alert(1)>");
    }

    #[test]
    fn reference_links_are_checked() {
        let html = render_markdown("[ok][r]\n\n[r]: https://example.com \"T\"\n");
        assert!(html.contains("href=\"https://example.com\""), "{html}");
        assert!(html.contains("title=\"T\""), "{html}");
        assert_neutralised("[x][r]\n\n[r]: javascript:alert(1)\n");
        assert_neutralised("![x][r]\n\n[r]: javascript:alert(1)\n");
    }

    #[test]
    fn encoded_and_mixed_case_schemes_are_neutralised() {
        for src in [
            concat!("[x](&#", "106;avascript:alert(1))"),
            "[x](&#x6A;avascript:alert(1))",
            "[x](javascript&colon;alert(1))",
            "[x](%6Aavascript:alert(1))",
            "[x](jAvAsCrIpT:alert(1))",
            "[x](JAVASCRIPT:alert(1))",
            "[x]( javascript:alert(1))",
        ] {
            assert_neutralised(src);
        }
        // An encoded colon is not a scheme separator, so the browser reads
        // this as a relative path. It stays, but it cannot run script.
        let html = render_markdown("[x](javascript%3Aalert(1))");
        assert!(html.contains("href=\"javascript%3Aalert(1)\""), "{html}");
    }

    #[test]
    fn link_titles_and_image_alt_are_escaped() {
        let html = render_markdown("[x](/a \"t\\\" onmouseover=\\\"alert(1)\")");
        assert!(
            html.contains("title=\"t&quot; onmouseover=&quot;alert(1)\""),
            "{html}"
        );

        let html = render_markdown("![a\" onerror=\"alert(1) <b>x</b>](/i.png)");
        assert!(html.contains("<img src=\"/i.png\" alt=\""), "{html}");
        assert!(!html.contains("\" onerror"), "{html}");
        assert!(!html.contains("<b>"), "{html}");
    }

    #[test]
    fn html_comments_are_escaped() {
        let html = render_markdown("<!-- hidden -->\n\nText <!-- inline --> here.\n");
        assert!(!html.contains("<!--"), "{html}");
        assert!(html.contains("&lt;!-- hidden --&gt;"), "{html}");
        assert!(html.contains("&lt;!-- inline --&gt;"), "{html}");
    }

    #[test]
    fn fence_info_class_is_restricted() {
        let cases = [
            ("```rust\nx\n```\n", Some("language-rust")),
            ("```rust ignore extra\nx\n```\n", Some("language-rust")),
            ("```c++\nx\n```\n", Some("language-c++")),
            ("```objective_c-2\nx\n```\n", Some("language-objective_c-2")),
            ("```\nx\n```\n", None),
            ("~~~\tpy\nx\n~~~\n", Some("language-py")),
            ("```rust\tevil-class\nx\n```\n", Some("language-rust")),
            ("```x\"onclick=\"alert(1)\nx\n```\n", None),
            ("```a.b\nx\n```\n", None),
            ("```a&#9;evil\nx\n```\n", Some("language-a")),
            (concat!("```ru&#", "115;t\nx\n```\n"), Some("language-rust")),
            ("```<script>\nx\n```\n", None),
            ("```\u{a0}nbsp\nx\n```\n", None),
        ];
        for (src, class) in cases {
            let html = render_markdown(src);
            match class {
                Some(c) => assert!(
                    html.starts_with(&format!("<pre><code class=\"{c}\">")),
                    "{src:?} -> {html}"
                ),
                None => assert!(html.starts_with("<pre><code>"), "{src:?} -> {html}"),
            }
        }
        let html = render_markdown("    indented\n");
        assert!(html.starts_with("<pre><code>"), "{html}");
    }

    #[test]
    fn heading_attributes_are_not_emitted() {
        let html = render_markdown("# Title {#evil .cls data-x=1}\n");
        assert!(html.starts_with("<h2>"), "{html}");
        assert!(!html.contains(" id="), "{html}");
        assert!(!html.contains(" class="), "{html}");
        assert!(!html.contains("data-x=\"1\""), "{html}");
    }
}
