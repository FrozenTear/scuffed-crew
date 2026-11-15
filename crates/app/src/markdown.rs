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
//!
//! Layout rules:
//! - A single line break inside a paragraph becomes `<br>`, so posts written
//!   as plain text (shown with `pre-wrap` before markdown) keep their lines.
//! - Each table is wrapped in `<div class="md-table-wrap">` so a wide table
//!   scrolls sideways inside the content card instead of overflowing it.
//!
//! Containers that hold rendered markdown add the `md-body` class and inject
//! [`MARKDOWN_CSS`] once, so blog and wiki share one set of element styles.

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html,
};

/// `StyleOnce` id for [`MARKDOWN_CSS`].
pub const MARKDOWN_CSS_ID: &str = "scuffed-css-markdown-body";

/// Element styles for rendered markdown inside an `.md-body` container.
///
/// Tailwind preflight resets headings to body size and weight and removes
/// list markers and indent, so this restores them. Colors, fonts and sizes
/// come from the design tokens only. Page CSS keeps the container's own look
/// (font size, paragraph spacing, `pre` background).
///
/// The first/last-child margin resets sit last on purpose: they tie on
/// specificity with `.md-body .md-table-wrap` (0,2,0), and inside one sheet
/// the later rule wins, so a leading or trailing table loses its outer margin
/// no matter which `StyleOnce` mounts first. They also beat page rules like
/// `.wiki-content p` (0,1,1), so pages must not repeat them.
///
/// The table wrapper's bottom margin follows `--md-block-gap` (default
/// `1rem`), so a page whose blocks use a different gap sets that property on
/// its container and tables line up with its other blocks.
///
/// Task items drop their marker in both shapes: tight (`li > input`) and
/// loose (`li > p > input`, where the parser wraps the item text in `<p>`).
pub const MARKDOWN_CSS: &str = r#"
    .md-body h1,
    .md-body h2,
    .md-body h3,
    .md-body h4,
    .md-body h5,
    .md-body h6 {
        color: var(--text);
        line-height: 1.3;
        margin: 1.6em 0 0.6em;
    }
    .md-body h1,
    .md-body h2,
    .md-body h3 {
        font-family: var(--font-head);
    }
    .md-body h1 { font-size: var(--text-3xl); font-weight: 700; }
    .md-body h2 { font-size: var(--text-2xl); font-weight: 700; }
    .md-body h3 { font-size: var(--text-xl); font-weight: 600; }
    .md-body h4 { font-size: var(--text-lg); font-weight: 600; }
    .md-body h5 { font-size: var(--text-base); font-weight: 600; }
    .md-body h6 {
        font-size: var(--text-sm);
        font-weight: 600;
        color: var(--text-2);
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .md-body ul {
        list-style: disc;
        padding-left: 1.5rem;
    }
    .md-body ol {
        list-style: decimal;
        padding-left: 1.5rem;
    }
    .md-body ul ul {
        list-style: circle;
    }
    .md-body ul ul ul {
        list-style: square;
    }
    .md-body li > ul,
    .md-body li > ol {
        margin: 0.25rem 0;
    }
    .md-body li + li {
        margin-top: 0.25rem;
    }
    .md-body li:has(
        > input[type="checkbox"],
        > p:first-child > input[type="checkbox"]:first-child
    ) {
        list-style: none;
    }
    .md-body li > input[type="checkbox"],
    .md-body li > p:first-child > input[type="checkbox"]:first-child {
        margin-right: 0.5rem;
        vertical-align: middle;
    }
    .md-body .md-table-wrap {
        max-width: 100%;
        overflow-x: auto;
        margin: 0 0 var(--md-block-gap, 1rem);
    }
    .md-body .md-table-wrap > table {
        margin: 0;
    }
    .md-body th {
        color: var(--text);
        font-weight: 600;
        text-align: left;
    }
    .md-body > :first-child {
        margin-top: 0;
    }
    .md-body > :last-child {
        margin-bottom: 0;
    }
"#;

/// Render untrusted markdown to HTML that is safe to inject.
pub fn render_markdown(source: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events = Parser::new_ext(source, options).flat_map(|event| match event {
        // Our own wrapper markup. Author HTML is never emitted as HTML: the
        // parser reports it as `Html`/`InlineHtml`, which `sanitize` turns
        // into escaped text.
        Event::Start(Tag::Table(align)) => [
            Some(Event::Html(CowStr::Borrowed(TABLE_WRAP_OPEN))),
            Some(Event::Start(Tag::Table(align))),
        ],
        Event::End(TagEnd::Table) => [
            Some(Event::End(TagEnd::Table)),
            Some(Event::Html(CowStr::Borrowed(TABLE_WRAP_CLOSE))),
        ],
        other => [Some(sanitize(other)), None],
    });
    let mut out = String::with_capacity(source.len() * 3 / 2);
    html::push_html(&mut out, events.flatten());
    out
}

const TABLE_WRAP_OPEN: &str = "<div class=\"md-table-wrap\">";
const TABLE_WRAP_CLOSE: &str = "</div>";

/// Per-event safety and layout rewrites (everything except table wrapping).
fn sanitize(event: Event<'_>) -> Event<'_> {
    match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        Event::SoftBreak => Event::HardBreak,
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
    }
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
    fn single_line_breaks_are_kept() {
        let html = render_markdown("line one\nline two\n\nnext para\n");
        assert!(html.contains("line one<br />\nline two"), "{html}");
        assert_eq!(html.matches("<br").count(), 1, "{html}");
        assert!(html.contains("<p>next para</p>"), "{html}");
    }

    #[test]
    fn tables_are_wrapped_for_sideways_scroll() {
        let html = render_markdown("| a | b |\n|---|---|\n| 1 | 2 |\n\nafter\n");
        assert!(
            html.starts_with("<div class=\"md-table-wrap\"><table>"),
            "{html}"
        );
        assert!(html.contains("</table>\n</div>"), "{html}");
        assert_eq!(html.matches("<div").count(), 1, "{html}");
        assert_eq!(html.matches("</div>").count(), 1, "{html}");
        assert!(html.contains("<p>after</p>"), "{html}");
    }

    #[test]
    fn author_div_markup_is_still_escaped() {
        let html = render_markdown("<div class=\"md-table-wrap\">x</div>\n");
        assert!(!html.contains("<div"), "{html}");
        assert!(html.contains("&lt;div"), "{html}");
    }

    #[test]
    fn author_html_in_table_cells_is_escaped() {
        let html = render_markdown(
            "| a | b |\n|---|---|\n| <script>x</script> | <img src=x onerror=alert(1)> |\n",
        );
        assert!(html.contains("<td>&lt;script&gt;"), "{html}");
        assert!(html.contains("<td>&lt;img"), "{html}");
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<img"), "{html}");
    }

    #[test]
    fn table_cells_cannot_forge_the_wrapper_or_unsafe_urls() {
        let html = render_markdown(concat!(
            "| <div class=\"md-table-wrap\"> | <!-- note --> |\n",
            "|---|---|\n",
            "| [x](javascript:alert(1)) | ![y](data:text/html,x) |\n",
        ));
        assert_eq!(html.matches("<div").count(), 1, "{html}");
        assert_eq!(html.matches("</div>").count(), 1, "{html}");
        assert!(
            html.starts_with("<div class=\"md-table-wrap\"><table>"),
            "{html}"
        );
        assert!(
            html.contains("<th>&lt;div class=\"md-table-wrap\"&gt;</th>"),
            "{html}"
        );
        assert!(html.contains("<th>&lt;!-- note --&gt;</th>"), "{html}");
        assert!(!html.contains("<!--"), "{html}");
        assert!(html.contains("<td><a href=\"#\">x</a></td>"), "{html}");
        assert!(html.contains("<img src=\"#\" alt=\"y\""), "{html}");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(!html.contains("data:"), "{html}");
    }

    #[test]
    fn code_block_newlines_stay_newlines() {
        let html = render_markdown("```\nline one\nline two\n```\n\n    a\n    b\n");
        assert!(
            html.contains("<pre><code>line one\nline two\n</code></pre>"),
            "{html}"
        );
        assert!(html.contains("<pre><code>a\nb\n</code></pre>"), "{html}");
        assert!(!html.contains("<br"), "{html}");
    }

    #[test]
    fn edge_margin_resets_come_after_the_table_wrap_rule() {
        // Equal specificity, so source order decides; keep the resets last.
        let wrap = MARKDOWN_CSS.find(".md-body .md-table-wrap {").unwrap();
        let first = MARKDOWN_CSS.find(".md-body > :first-child {").unwrap();
        let last = MARKDOWN_CSS.find(".md-body > :last-child {").unwrap();
        assert!(wrap < first && wrap < last);
    }

    #[test]
    fn task_list_shapes_match_the_css() {
        // Tight: the checkbox is a direct child of the `li`.
        let html = render_markdown("- [ ] todo\n- [x] done\n");
        assert!(
            html.contains("<li><input disabled=\"\" type=\"checkbox\"/>\ntodo</li>"),
            "{html}"
        );
        // Loose: the parser wraps the item in `<p>` and the checkbox is that
        // paragraph's first child, which is what MARKDOWN_CSS matches.
        let html = render_markdown("- [ ] todo\n\n- [x] done\n");
        assert!(
            html.contains("<li>\n<p><input disabled=\"\" type=\"checkbox\"/>\ntodo</p>\n</li>"),
            "{html}"
        );
        assert!(MARKDOWN_CSS.contains("> p:first-child > input[type=\"checkbox\"]:first-child"));
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
