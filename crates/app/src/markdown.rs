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
//! The page title is already the `h1`, so a body never emits one. A body that
//! uses `#` is shifted down one level (`#` becomes `h2`, `##` becomes `h3`).
//! A body that starts at `##`, the usual way to write sections, is left as
//! written, so its sections are `h2` and the outline has no gap after the
//! title.
//!
//! Layout rules:
//! - A single line break inside a paragraph becomes `<br>`, so posts written
//!   as plain text (shown with `pre-wrap` before markdown) keep their lines.
//! - Each table is wrapped in a `md-table-wrap` box and each code block is a
//!   `md-code` box. Both scroll sideways inside the article instead of
//!   widening the page, and both take keyboard focus (`tabindex="0"` plus a
//!   name) so they can be scrolled without a pointer. They are named groups,
//!   not landmarks: an article with several would otherwise list several
//!   regions with the same name.
//! - A task list checkbox is named "Done" or "Not done". It is disabled, so
//!   the name says what the mark means and the item text follows it.
//! - A paragraph that is only an image with a title (`![alt](url "caption")`)
//!   becomes a `<figure>` with the title as its `<figcaption>`.
//!
//! Containers that hold rendered markdown add the `md-body` class and inject
//! [`MARKDOWN_CSS`] once, so blog and wiki share one prose style.

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html,
};

/// `StyleOnce` id for [`MARKDOWN_CSS`].
pub const MARKDOWN_CSS_ID: &str = "scuffed-css-markdown-body";

/// The one prose style for rendered markdown inside an `.md-body` container
/// (blog posts and wiki pages). Pages add no element rules of their own.
///
/// Tailwind preflight resets headings to body size and weight and removes
/// list markers and indent, so this restores them. Colors, fonts, sizes and
/// spacing come from the design tokens only.
///
/// Blocks are spaced by `--md-block-gap`. The first/last-child margin resets
/// sit last on purpose: they tie on specificity with
/// `.md-body .md-table-wrap` (0,2,0), and inside one sheet the later rule
/// wins, so a leading or trailing table loses its outer margin. They beat the
/// element rules (0,1,1) in any order.
///
/// Task items drop their marker in both shapes: tight (`li > input`) and
/// loose (`li > p > input`, where the parser wraps the item text in `<p>`).
pub const MARKDOWN_CSS: &str = r#"
    .md-body {
        --md-block-gap: var(--space-4);
        min-width: 0;
        max-width: var(--content-max-narrow);
        color: var(--text);
        font-family: var(--font-body);
        font-size: var(--text-md);
        line-height: var(--leading-body);
        overflow-wrap: anywhere;
    }
    .md-body h2,
    .md-body h3,
    .md-body h4,
    .md-body h5,
    .md-body h6 {
        margin: var(--space-5) 0 var(--space-2);
        color: var(--text);
        font-family: var(--font-head);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-head-tracking);
        line-height: var(--leading-ui);
    }
    .md-body h2 {
        margin-top: var(--space-6);
        font-size: var(--text-lg);
    }
    .md-body h3 { font-size: var(--text-lg); }
    .md-body h4 { font-size: var(--text-md); }
    .md-body h5 { font-size: var(--text-sm); }
    .md-body h6 {
        font-size: var(--text-sm);
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: var(--font-label-tracking);
    }
    .md-body p,
    .md-body ul,
    .md-body ol,
    .md-body blockquote,
    .md-body pre,
    .md-body figure {
        margin: 0 0 var(--md-block-gap);
    }
    .md-body ul {
        list-style: disc;
        padding-inline-start: var(--space-5);
    }
    .md-body ol {
        list-style: decimal;
        padding-inline-start: var(--space-5);
    }
    .md-body ul ul {
        list-style: circle;
    }
    .md-body ul ul ul {
        list-style: square;
    }
    .md-body li > ul,
    .md-body li > ol {
        margin: var(--space-2) 0 0;
    }
    .md-body li + li {
        margin-top: var(--space-2);
    }
    .md-body li:has(
        > input[type="checkbox"],
        > p:first-child > input[type="checkbox"]:first-child
    ) {
        list-style: none;
    }
    .md-body li > input[type="checkbox"],
    .md-body li > p:first-child > input[type="checkbox"]:first-child {
        margin-inline-end: var(--space-2);
        vertical-align: middle;
    }
    /* In-text links: accent ink plus an underline, so not colour alone. */
    .md-body a {
        color: var(--accent-text);
        text-decoration: underline;
    }
    .md-body a:hover {
        text-decoration: none;
    }
    .md-body blockquote {
        padding: var(--space-3) var(--space-4);
        border-inline-start: var(--space-1) solid var(--accent-solid);
        background: var(--surface);
        color: var(--text-3);
    }
    .md-body blockquote > :last-child {
        margin-bottom: 0;
    }
    .md-body img {
        display: block;
        max-width: 100%;
        height: auto;
        border-radius: var(--radius-card);
    }
    .md-body figure img {
        box-sizing: border-box;
        width: 100%;
        border: 1px solid var(--border);
    }
    .md-body figcaption {
        margin-top: var(--space-2);
        color: var(--text-3);
        font-size: var(--text-sm);
    }
    .md-body hr {
        margin: var(--space-6) 0;
        border: 0;
        border-top: 1px solid var(--border);
    }
    /* Code scrolls sideways inside its own box; the page never does. */
    .md-body pre {
        max-width: 100%;
        padding: var(--space-4);
        overflow-x: auto;
        border: 1px solid var(--border);
        border-radius: var(--radius-card);
        /* Scroll hint: a soft edge on the side that has more to show. The
           two edge shades stay put; the two fills travel with the text and
           cover the shade at the end they have reached. Without it a long
           line just stops at the border and reads as cut off. */
        background:
            linear-gradient(to right, var(--surface-sunken) 50%, transparent) left center / var(--space-8) 100% no-repeat local,
            linear-gradient(to left, var(--surface-sunken) 50%, transparent) right center / var(--space-8) 100% no-repeat local,
            linear-gradient(to right, var(--border-strong), transparent) left center / var(--space-3) 100% no-repeat scroll,
            linear-gradient(to left, var(--border-strong), transparent) right center / var(--space-3) 100% no-repeat scroll,
            var(--surface-sunken);
        color: var(--text);
        font-family: var(--font-mono);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
        white-space: pre;
        overflow-wrap: normal;
    }
    .md-body code {
        font-family: var(--font-mono);
        font-size: var(--text-sm);
    }
    .md-body :not(pre) > code {
        padding: 0 var(--space-1);
        border: 1px solid var(--border);
        border-radius: var(--radius-control);
        background: var(--surface-sunken);
    }
    .md-body pre,
    .md-body .md-table-wrap {
        scrollbar-width: thin;
        scrollbar-color: var(--border-strong) transparent;
    }
    .md-body pre:focus-visible,
    .md-body .md-table-wrap:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .md-body .md-table-wrap {
        max-width: 100%;
        overflow-x: auto;
        margin: 0 0 var(--md-block-gap);
    }
    .md-body .md-table-wrap > table {
        margin: 0;
        border-collapse: collapse;
        font-size: var(--text-sm);
        overflow-wrap: normal;
    }
    .md-body th,
    .md-body td {
        padding: var(--space-2) var(--space-3);
        border: 1px solid var(--border);
        text-align: left;
    }
    .md-body th {
        color: var(--text);
        font-weight: var(--font-label-weight);
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
    // Author HTML is never emitted as HTML: the parser reports it as
    // `Html`/`InlineHtml`, which `sanitize` turns into escaped text. Every
    // `Html` event pushed below is our own wrapper markup.
    let parsed: Vec<Event<'_>> = Parser::new_ext(source, parser_options()).collect();
    let shift = uses_title_level(&parsed);
    let events: Vec<Event<'_>> = parsed
        .into_iter()
        .map(|event| sanitize(event, shift))
        .collect();
    let mut laid_out: Vec<Event<'_>> = Vec::with_capacity(events.len() + 8);
    let mut index = 0;
    while index < events.len() {
        match &events[index] {
            Event::Start(Tag::Table(_)) => {
                laid_out.push(Event::Html(CowStr::Borrowed(TABLE_WRAP_OPEN)));
                laid_out.push(events[index].clone());
            }
            Event::End(TagEnd::Table) => {
                laid_out.push(events[index].clone());
                laid_out.push(Event::Html(CowStr::Borrowed(TABLE_WRAP_CLOSE)));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                laid_out.push(Event::Html(CowStr::from(code_open(kind))));
            }
            Event::End(TagEnd::CodeBlock) => {
                laid_out.push(Event::Html(CowStr::Borrowed(CODE_CLOSE)));
            }
            Event::TaskListMarker(checked) => {
                laid_out.push(Event::Html(CowStr::Borrowed(task_marker(*checked))));
            }
            Event::Start(Tag::Paragraph) => {
                if let Some(figure) = captioned_image(&events, index) {
                    push_figure(&mut laid_out, &events, index, &figure);
                    index = figure.paragraph_end + 1;
                    continue;
                }
                laid_out.push(events[index].clone());
            }
            other => laid_out.push(other.clone()),
        }
        index += 1;
    }
    let mut out = String::with_capacity(source.len() * 3 / 2);
    html::push_html(&mut out, laid_out.into_iter());
    out
}

fn parser_options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// Plain text of the first paragraph of `source`, cut to `max_chars` with
/// `...` when longer. Empty when the source has no paragraph. Used for list
/// excerpts, so it carries no markup at all.
pub fn plain_excerpt(source: &str, max_chars: usize) -> String {
    let mut text = String::new();
    let mut in_paragraph = false;
    let mut image_depth = 0usize;
    for event in Parser::new_ext(source, parser_options()) {
        match event {
            Event::Start(Tag::Paragraph) => in_paragraph = true,
            Event::End(TagEnd::Paragraph) if in_paragraph => {
                if !text.trim().is_empty() {
                    break;
                }
                // A paragraph that was only an image: try the next one.
                text.clear();
                in_paragraph = false;
            }
            Event::Start(Tag::Image { .. }) => image_depth += 1,
            Event::End(TagEnd::Image) => image_depth = image_depth.saturating_sub(1),
            Event::Text(part) | Event::Code(part) if in_paragraph && image_depth == 0 => {
                text.push_str(&part);
            }
            Event::SoftBreak | Event::HardBreak if in_paragraph => text.push(' '),
            _ => {}
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut = crate::util::truncate_chars(&text, max_chars);
    if cut.len() < text.len() {
        format!("{}...", cut.trim_end())
    } else {
        text
    }
}

/// Opens the scroll box around a table. Focusable and named so a wide table
/// can be scrolled from the keyboard. A group, not a landmark.
const TABLE_WRAP_OPEN: &str =
    "<div class=\"md-table-wrap\" tabindex=\"0\" role=\"group\" aria-label=\"Table\">";
const TABLE_WRAP_CLOSE: &str = "</div>";

/// Opens a code block. The `pre` is the scroll box: focusable and named.
const CODE_OPEN: &str =
    "<pre class=\"md-code\" tabindex=\"0\" role=\"group\" aria-label=\"Code sample\">";
const CODE_CLOSE: &str = "</code></pre>\n";

/// The checkbox of a task list item, named by its state. Same shape as the
/// writer's own (`disabled`, then `type`), plus the name.
fn task_marker(checked: bool) -> &'static str {
    if checked {
        "<input disabled=\"\" type=\"checkbox\" checked=\"\" aria-label=\"Done\"/>\n"
    } else {
        "<input disabled=\"\" type=\"checkbox\" aria-label=\"Not done\"/>\n"
    }
}

/// `<pre ...><code>` for a block. `sanitize` has already reduced a fenced
/// block's info string to a safe language word (or nothing).
fn code_open(kind: &CodeBlockKind<'_>) -> String {
    match kind {
        CodeBlockKind::Fenced(lang) if !lang.is_empty() => {
            format!("{CODE_OPEN}<code class=\"language-{lang}\">")
        }
        _ => format!("{CODE_OPEN}<code>"),
    }
}

/// A paragraph that holds one titled image and nothing else.
struct CaptionedImage<'a> {
    /// Index of the image's `End` event.
    image_end: usize,
    /// Index of the paragraph's `End` event.
    paragraph_end: usize,
    caption: CowStr<'a>,
}

/// `Some` when the paragraph opening at `start` is exactly one image whose
/// title is not blank.
fn captioned_image<'a>(events: &[Event<'a>], start: usize) -> Option<CaptionedImage<'a>> {
    let Some(Event::Start(Tag::Image { title, .. })) = events.get(start + 1) else {
        return None;
    };
    if title.trim().is_empty() {
        return None;
    }
    let mut depth = 0usize;
    let mut image_end = None;
    for (offset, event) in events[start + 1..].iter().enumerate() {
        match event {
            Event::Start(Tag::Image { .. }) => depth += 1,
            Event::End(TagEnd::Image) => {
                depth -= 1;
                if depth == 0 {
                    image_end = Some(start + 1 + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let image_end = image_end?;
    match events.get(image_end + 1) {
        Some(Event::End(TagEnd::Paragraph)) => Some(CaptionedImage {
            image_end,
            paragraph_end: image_end + 1,
            caption: title.clone(),
        }),
        _ => None,
    }
}

/// `<figure><img ...><figcaption>caption</figcaption></figure>` in place of
/// the paragraph at `start`. The caption is pushed as text, so the writer
/// escapes it; the image loses its `title` since the caption now shows it.
fn push_figure<'a>(
    out: &mut Vec<Event<'a>>,
    events: &[Event<'a>],
    start: usize,
    figure: &CaptionedImage<'a>,
) {
    out.push(Event::Html(CowStr::Borrowed("<figure>")));
    for event in &events[start + 1..=figure.image_end] {
        out.push(match event {
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                id,
                ..
            }) => Event::Start(Tag::Image {
                link_type: *link_type,
                dest_url: dest_url.clone(),
                title: CowStr::Borrowed(""),
                id: id.clone(),
            }),
            other => other.clone(),
        });
    }
    out.push(Event::Html(CowStr::Borrowed("<figcaption>")));
    out.push(Event::Text(figure.caption.clone()));
    out.push(Event::Html(CowStr::Borrowed("</figcaption></figure>\n")));
}

/// True when the body has a `#` heading. The page title owns `h1`, so such a
/// body has every heading moved down one level.
fn uses_title_level(events: &[Event<'_>]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            })
        )
    })
}

/// Per-event safety rewrites. Wrapper markup is added after this pass.
/// `shift` moves headings down one level (see [`uses_title_level`]).
fn sanitize(event: Event<'_>, shift: bool) -> Event<'_> {
    match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        Event::SoftBreak => Event::HardBreak,
        // Heading attributes are off in `options`; clear them anyway so
        // turning that option on later cannot let authors set ids or classes.
        Event::Start(Tag::Heading { level, .. }) => Event::Start(Tag::Heading {
            level: heading_level(level, shift),
            id: None,
            classes: Vec::new(),
            attrs: Vec::new(),
        }),
        Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(code_lang(&info))))
        }
        Event::End(TagEnd::Heading(level)) => {
            Event::End(TagEnd::Heading(heading_level(level, shift)))
        }
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

fn heading_level(level: HeadingLevel, shift: bool) -> HeadingLevel {
    if shift { demote(level) } else { level }
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
    fn a_body_with_a_title_level_heading_shifts_down_one_and_caps_at_h6() {
        let html = render_markdown("# One\n\n## Two\n\n###### Six\n");
        assert!(html.contains("<h2>One</h2>"), "{html}");
        assert!(html.contains("<h3>Two</h3>"), "{html}");
        assert!(html.contains("<h6>Six</h6>"), "{html}");
        assert!(!html.contains("<h1"), "{html}");
    }

    /// Sections written as `##` are the article's `h2`: the spec size and
    /// spacing apply to them and the outline does not jump from `h1` to `h3`.
    #[test]
    fn a_body_that_starts_at_level_two_keeps_its_levels() {
        let html = render_markdown("## Two\n\n### Three\n\n###### Six\n");
        assert!(html.contains("<h2>Two</h2>"), "{html}");
        assert!(html.contains("<h3>Three</h3>"), "{html}");
        assert!(html.contains("<h6>Six</h6>"), "{html}");
        assert!(!html.contains("<h1"), "{html}");
        // A late `#` still shifts the whole body, so levels stay relative.
        let late = render_markdown("## Two\n\n# One\n");
        assert!(late.contains("<h3>Two</h3>"), "{late}");
        assert!(late.contains("<h2>One</h2>"), "{late}");
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
                    html.starts_with(&format!("{CODE_OPEN}<code class=\"{c}\">")),
                    "{src:?} -> {html}"
                ),
                None => assert!(
                    html.starts_with(&format!("{CODE_OPEN}<code>")),
                    "{src:?} -> {html}"
                ),
            }
        }
        let html = render_markdown("    indented\n");
        assert!(html.starts_with(&format!("{CODE_OPEN}<code>")), "{html}");
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
            html.starts_with(&format!("{TABLE_WRAP_OPEN}<table>")),
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
            html.starts_with(&format!("{TABLE_WRAP_OPEN}<table>")),
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
            html.contains(&format!(
                "{CODE_OPEN}<code>line one\nline two\n</code></pre>"
            )),
            "{html}"
        );
        assert!(
            html.contains(&format!("{CODE_OPEN}<code>a\nb\n</code></pre>")),
            "{html}"
        );
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
            html.contains(
                "<li><input disabled=\"\" type=\"checkbox\" aria-label=\"Not done\"/>\ntodo</li>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<li><input disabled=\"\" type=\"checkbox\" checked=\"\" aria-label=\"Done\"/>\ndone</li>"
            ),
            "{html}"
        );
        // Every checkbox has a name.
        assert_eq!(
            html.matches("<input ").count(),
            html.matches("aria-label=").count(),
            "{html}"
        );
        // Loose: the parser wraps the item in `<p>` and the checkbox is that
        // paragraph's first child, which is what MARKDOWN_CSS matches.
        let html = render_markdown("- [ ] todo\n\n- [x] done\n");
        assert!(
            html.contains(
                "<li>\n<p><input disabled=\"\" type=\"checkbox\" aria-label=\"Not done\"/>\ntodo</p>\n</li>"
            ),
            "{html}"
        );
        assert!(MARKDOWN_CSS.contains("> p:first-child > input[type=\"checkbox\"]:first-child"));
    }

    #[test]
    fn scroll_regions_are_focusable_and_named() {
        // A scrollable box needs keyboard focus and a name to announce. It
        // is a group: several landmarks with one name could not be told apart.
        for open in [CODE_OPEN, TABLE_WRAP_OPEN] {
            assert!(open.contains(" tabindex=\"0\""), "{open}");
            assert!(open.contains(" role=\"group\""), "{open}");
            assert!(open.contains(" aria-label=\""), "{open}");
            assert!(!open.contains("region"), "{open}");
        }
        let html = render_markdown("```py\nprint(1)\n```\n\n| a |\n|---|\n| 1 |\n");
        assert_eq!(html.matches("tabindex=\"0\"").count(), 2, "{html}");
        assert_eq!(html.matches("<pre ").count(), 1, "{html}");
        assert_eq!(html.matches("</pre>").count(), 1, "{html}");
    }

    #[test]
    fn code_block_text_is_escaped_and_cannot_close_the_block() {
        let html = render_markdown("```\n</code></pre><script>alert(1)</script>\n```\n");
        assert!(!html.contains("<script"), "{html}");
        assert_eq!(html.matches("</pre>").count(), 1, "{html}");
        assert!(html.contains("&lt;/code&gt;&lt;/pre&gt;"), "{html}");
    }

    #[test]
    fn author_markup_cannot_forge_a_focusable_region() {
        let html = render_markdown("<pre class=\"md-code\" tabindex=\"0\">x</pre>\n");
        assert!(!html.contains("<pre"), "{html}");
        assert!(html.starts_with("&lt;pre class="), "{html}");
    }

    #[test]
    fn titled_image_alone_in_a_paragraph_becomes_a_figure() {
        let html = render_markdown("![Night schedule](/img/night.png \"A typical Friday\")\n");
        assert_eq!(
            html,
            "<figure><img src=\"/img/night.png\" alt=\"Night schedule\" />\
             <figcaption>A typical Friday</figcaption></figure>\n"
        );
    }

    #[test]
    fn figure_caption_and_url_stay_safe() {
        let html =
            render_markdown("![a](javascript:alert(1) \"<script>x</script> \\\" onload=\\\"y\")\n");
        assert!(
            html.starts_with("<figure><img src=\"#\" alt=\"a\" />"),
            "{html}"
        );
        assert!(!html.contains("<script"), "{html}");
        assert!(
            html.contains("<figcaption>&lt;script&gt;x&lt;/script&gt;"),
            "{html}"
        );
        assert!(!html.contains("javascript:"), "{html}");
    }

    #[test]
    fn untitled_or_inline_images_stay_plain_images() {
        let html = render_markdown("![alt](/a.png)\n");
        assert_eq!(html, "<p><img src=\"/a.png\" alt=\"alt\" /></p>\n");

        let html = render_markdown("Before ![alt](/a.png \"T\") after\n");
        assert!(!html.contains("<figure"), "{html}");
        assert!(html.contains("title=\"T\""), "{html}");

        let html = render_markdown("![alt](/a.png \"T\") trailing\n");
        assert!(!html.contains("<figure"), "{html}");
    }

    #[test]
    fn excerpt_is_the_first_paragraph_as_plain_text() {
        let src = "# Tank basics\n\n![map](/m.png)\n\nSpace, **peels**, and `when`\nto [hold](/x) a shield.\n\nSecond.\n";
        assert_eq!(
            plain_excerpt(src, 200),
            "Space, peels, and when to hold a shield."
        );
        assert_eq!(plain_excerpt(src, 12), "Space, peels...");
        assert_eq!(plain_excerpt("# Only a heading\n", 50), "");
        assert_eq!(plain_excerpt("", 50), "");
        // Raw HTML is dropped, not carried into the excerpt.
        assert_eq!(plain_excerpt("Hi <b>there</b> you\n", 50), "Hi there you");
    }

    #[test]
    fn prose_style_follows_the_article_spec() {
        use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

        assert_sizes_from_tokens(MARKDOWN_CSS);
        let h2 = css_rule(MARKDOWN_CSS, ".md-body h2 {");
        assert!(has_decl(&h2, "font-size: var(--text-lg)"), "{h2}");
        assert!(has_decl(&h2, "margin-top: var(--space-6)"), "{h2}");
        let shared = css_rule(MARKDOWN_CSS, ".md-body h5,\n    .md-body h6 {");
        for decl in [
            "margin: var(--space-5) 0 var(--space-2)",
            "font-family: var(--font-head)",
            "font-weight: var(--font-label-weight)",
        ] {
            assert!(has_decl(&shared, decl), "missing {decl}: {shared}");
        }
        // H2 and H3 share the weight (the frame draws both semibold), and
        // page CSS uses semantic tokens, not primitives.
        let h3 = css_rule(MARKDOWN_CSS, ".md-body h3 {");
        assert!(!h3.contains("font-weight"), "{h3}");
        for primitive in ["--font-weight-", "--tracking-"] {
            assert!(!MARKDOWN_CSS.contains(primitive), "{primitive}");
        }
        let pre = css_rule(MARKDOWN_CSS, ".md-body pre {");
        for decl in [
            "overflow-x: auto",
            "max-width: 100%",
            "font-family: var(--font-mono)",
            "white-space: pre",
        ] {
            assert!(has_decl(&pre, decl), "missing {decl}: {pre}");
        }
        let body = css_rule(MARKDOWN_CSS, ".md-body {");
        assert!(
            has_decl(&body, "max-width: var(--content-max-narrow)"),
            "{body}"
        );
        assert!(has_decl(&body, "min-width: 0"), "{body}");
        let link = css_rule(MARKDOWN_CSS, ".md-body a {");
        assert!(has_decl(&link, "text-decoration: underline"), "{link}");
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
