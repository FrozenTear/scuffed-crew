//! Small shared helpers used across pages and components.

/// First `max_chars` Unicode scalars of `s`.
///
/// Never panics and never slices inside a scalar. Short strings are returned
/// unchanged. `max_chars == 0` yields an empty string.
pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Trimmed `url` when it is safe to place in an `href`.
///
/// Only `http://` and `https://` (any ASCII case) are allowed. `javascript:`,
/// `data:`, `vbscript:`, mixed-case or whitespace-prefixed variants, leading
/// C0 controls, protocol-relative URLs, and relative paths are `None`.
/// Whitespace, a control character, or a backslash anywhere is `None`.
/// Callers should render `None` as plain text or omit the link.
pub fn http_href(url: &str) -> Option<&str> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    // Leading C0 (U+0000..=U+001F) is not all whitespace, so `trim` leaves
    // NUL and similar in place. Browsers drop those bytes before the scheme.
    // Reject the value; do not strip it and link the remainder.
    if url.starts_with(is_c0_control) {
        return None;
    }
    let http = starts_with_ignore_ascii_case(url, "http://");
    let https = starts_with_ignore_ascii_case(url, "https://");
    if !http && !https {
        return None;
    }
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    Some(url)
}

/// U+0000..=U+001F. URL parsers remove these before scheme parsing.
fn is_c0_control(c: char) -> bool {
    matches!(c, '\u{0000}'..='\u{001F}')
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    let bytes = value.as_bytes();
    let prefix = prefix.as_bytes();
    bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// A slice of user-authored text, with real `http://` / `https://` URLs split out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextLink<'a> {
    Text(&'a str),
    Href(&'a str),
}

/// Split `content` into plain text and linkable URLs.
///
/// Only a token that is itself an `http://` or `https://` URL (any ASCII case)
/// becomes [`TextLink::Href`]. A substring that merely contains `http`,
/// including `http:javascript:alert(1)`, stays text. The scheme must sit at
/// the start of the string or after a non-alphanumeric character.
pub fn linkify_http_spans(content: &str) -> Vec<TextLink<'_>> {
    let mut parts = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    while i < content.len() {
        if http_url_at(content, i) {
            if text_start < i {
                parts.push(TextLink::Text(&content[text_start..i]));
            }
            let rest = &content[i..];
            let end_rel = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let token = &content[i..i + end_rel];
            let (url, trailing) = split_trailing_url_punctuation(token);
            match http_href(url) {
                Some(href) => parts.push(TextLink::Href(href)),
                None => parts.push(TextLink::Text(url)),
            }
            if !trailing.is_empty() {
                parts.push(TextLink::Text(trailing));
            }
            i += end_rel;
            text_start = i;
            continue;
        }
        let Some(ch) = content[i..].chars().next() else {
            break;
        };
        i += ch.len_utf8();
    }
    if text_start < content.len() {
        parts.push(TextLink::Text(&content[text_start..]));
    }
    parts
}

/// Sentence punctuation stuck to a URL stays text (`https://example.com).`).
fn split_trailing_url_punctuation(token: &str) -> (&str, &str) {
    let trimmed = token.trim_end_matches(|c: char| {
        matches!(
            c,
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>' | '\'' | '"'
        )
    });
    (trimmed, &token[trimmed.len()..])
}

fn http_url_at(content: &str, index: usize) -> bool {
    if index > 0
        && let Some(prev) = content[..index].chars().next_back()
        && prev.is_ascii_alphanumeric()
    {
        return false;
    }
    let rest = &content[index..];
    starts_with_ignore_ascii_case(rest, "https://")
        || starts_with_ignore_ascii_case(rest, "http://")
}

/// How to render a URL that was stored earlier and is not re-checked server-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredUrl<'a> {
    /// Safe to place in an `href`.
    Link(&'a str),
    /// Non-empty, but not `http://` or `https://`. Render as text, not a link.
    Plain(&'a str),
}

/// `None` when `url` is empty. Otherwise a link only if [`http_href`] accepts it.
pub fn stored_http_url(url: &str) -> Option<StoredUrl<'_>> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        None
    } else if let Some(href) = http_href(trimmed) {
        Some(StoredUrl::Link(href))
    } else {
        Some(StoredUrl::Plain(trimmed))
    }
}

/// How a `use_resource` slot that stores `Result` should be shown.
/// Loading, failure, and a successful payload stay distinct — an error is not
/// "still loading" and an empty payload is not a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchClass {
    Loading,
    Error,
    Ready,
}

pub fn classify_fetch<T, E>(slot: Option<&Result<T, E>>) -> FetchClass {
    match slot {
        None => FetchClass::Loading,
        Some(Err(_)) => FetchClass::Error,
        Some(Ok(_)) => FetchClass::Ready,
    }
}

/// English count noun: `1 player`, `0 players`, `2 players`.
pub fn pluralize(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("1 {singular}")
    } else {
        format!("{n} {plural}")
    }
}

/// Percent-encode a query-parameter value so values with spaces or non-ASCII
/// (e.g. "Soldier: 76", "Lúcio", nostr pubkeys/timestamps) survive the URL.
/// Keeps the RFC 3986 unreserved set literal (`A-Za-z0-9-_.~`); everything else
/// becomes `%XX` over UTF-8 bytes.
/// Format an ISO-8601 timestamp for admin tables: `2026-07-10T19:30:35.657Z`
/// becomes `2026-07-10 19:30`. Values that don't look like a timestamp pass
/// through unchanged, so raw/legacy strings still render.
pub fn format_datetime(iso: &str) -> String {
    match (iso.get(..10), iso.get(11..16)) {
        (Some(date), Some(time)) if iso.as_bytes().get(10) == Some(&b'T') => {
            format!("{date} {time}")
        }
        _ => iso.to_string(),
    }
}

pub fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Append `season=<id>` to an API path (`?` or `&` as needed). `None` = all
/// time, path returned unchanged — the server treats a missing/blank season
/// as "total".
pub fn season_url(path: &str, season: Option<&str>) -> String {
    match season {
        Some(id) if !id.is_empty() => {
            let sep = if path.contains('?') { '&' } else { '?' };
            format!("{path}{sep}season={}", encode_query(id))
        }
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_is_length_safe() {
        assert_eq!(truncate_chars("", 8), "");
        assert_eq!(truncate_chars("abc", 8), "abc");
        assert_eq!(truncate_chars("abcdefgh", 8), "abcdefgh");
        assert_eq!(truncate_chars("abcdefghi", 8), "abcdefgh");
        assert_eq!(truncate_chars("hello", 0), "");
        // 9 scalars, 2 bytes each — a byte slice of 8 would split a scalar.
        let multibyte = "ééééééééé";
        let short = truncate_chars(multibyte, 8);
        assert_eq!(short.chars().count(), 8);
        assert!(multibyte.starts_with(short));
        assert!(short.is_char_boundary(short.len()));
        assert_eq!(truncate_chars("éé", 8), "éé");
    }

    #[test]
    fn http_href_allows_only_http_and_https() {
        assert_eq!(
            http_href("https://example.com/notes"),
            Some("https://example.com/notes")
        );
        assert_eq!(
            http_href("  HTTP://example.com/x  "),
            Some("HTTP://example.com/x")
        );
        assert_eq!(
            http_href("https://example.com/a%20b"),
            Some("https://example.com/a%20b")
        );
        assert_eq!(http_href("javascript:alert(1)"), None);
        assert_eq!(http_href(" JavaScript:alert(1)"), None);
        assert_eq!(http_href("vbscript:msgbox"), None);
        assert_eq!(http_href(" VBScript:msgbox(1)"), None);
        assert_eq!(http_href("data:text/html,hi"), None);
        assert_eq!(http_href("//example.com"), None);
        assert_eq!(http_href("/relative"), None);
        assert_eq!(http_href("https:example.com"), None);
        assert_eq!(http_href("http:javascript:alert(1)"), None);
        assert_eq!(http_href(""), None);
        assert_eq!(http_href("   "), None);
        assert_eq!(http_href("https://exa mple.com"), None);
        assert_eq!(http_href("https://example.com/a\nb"), None);
        assert_eq!(http_href("https://example.com\\@evil.com"), None);
    }

    #[test]
    fn http_href_rejects_script_schemes_with_case_whitespace_and_controls() {
        assert_eq!(
            http_href("http://example.com/notes"),
            Some("http://example.com/notes")
        );
        assert_eq!(
            http_href("https://example.com/notes"),
            Some("https://example.com/notes")
        );
        assert_eq!(
            http_href("HTTPS://example.com/notes"),
            Some("HTTPS://example.com/notes")
        );

        assert_eq!(http_href("javascript:alert(1)"), None);
        assert_eq!(http_href(" JaVaScRiPt:alert(1)"), None);
        assert_eq!(http_href("\tJaVaScRiPt:alert(1)"), None);
        assert_eq!(http_href("\njavascript:alert(1)"), None);
        assert_eq!(http_href("\r\njavascript:alert(1)"), None);
        assert_eq!(http_href(" \u{0000}JaVaScRiPt:alert(1)"), None);
        assert_eq!(http_href("\u{0001}javascript:alert(1)"), None);
        assert_eq!(http_href("\u{0001}https://example.com"), None);
        assert_eq!(http_href("java\nscript:alert(1)"), None);
        assert_eq!(http_href("java\tscript:alert(1)"), None);

        assert_eq!(http_href("data:text/html,hi"), None);
        assert_eq!(http_href(" DaTa:text/html,hi"), None);
        assert_eq!(http_href("\u{000B}dAtA:text/html,<script>"), None);
        assert_eq!(http_href("\u{0001}data:text/html,hi"), None);

        assert_eq!(http_href("vbscript:msgbox(1)"), None);
        assert_eq!(http_href(" VbScRiPt:msgbox(1)"), None);
        assert_eq!(http_href("\u{0001}vbscript:msgbox(1)"), None);

        assert_eq!(http_href(""), None);
        assert_eq!(http_href("   "), None);
        assert_eq!(http_href("\u{0000}"), None);
        assert_eq!(http_href("\u{0001}"), None);
    }

    #[test]
    fn linkify_http_spans_only_links_real_http_urls() {
        assert_eq!(
            linkify_http_spans("http:javascript:alert(1)"),
            vec![TextLink::Text("http:javascript:alert(1)")]
        );
        assert_eq!(
            linkify_http_spans("javascript:alert(1)"),
            vec![TextLink::Text("javascript:alert(1)")]
        );
        assert_eq!(
            linkify_http_spans("prefixhttps://example.com"),
            vec![TextLink::Text("prefixhttps://example.com")]
        );
        assert_eq!(
            linkify_http_spans("see https://example.com/notes now"),
            vec![
                TextLink::Text("see "),
                TextLink::Href("https://example.com/notes"),
                TextLink::Text(" now"),
            ]
        );
        assert_eq!(
            linkify_http_spans("HTTP://example.com/x"),
            vec![TextLink::Href("HTTP://example.com/x")]
        );
        assert_eq!(
            linkify_http_spans("(https://example.com)"),
            vec![
                TextLink::Text("("),
                TextLink::Href("https://example.com"),
                TextLink::Text(")"),
            ]
        );
        assert_eq!(
            linkify_http_spans("https://example.com\\@evil.com"),
            vec![TextLink::Text("https://example.com\\@evil.com")]
        );
        assert!(linkify_http_spans("").is_empty());
        let linked = linkify_http_spans("a https://ok.example b http://also.example");
        assert!(linked.contains(&TextLink::Href("https://ok.example")));
        assert!(linked.contains(&TextLink::Href("http://also.example")));
        assert!(
            !linked
                .iter()
                .any(|part| matches!(part, TextLink::Href(href) if !href.contains("://")))
        );
    }

    #[test]
    fn stored_http_url_links_only_http_and_https() {
        assert_eq!(stored_http_url("   "), None);
        assert_eq!(stored_http_url(""), None);
        assert_eq!(
            stored_http_url("  https://vod.example/a  "),
            Some(StoredUrl::Link("https://vod.example/a"))
        );
        assert_eq!(
            stored_http_url("javascript:alert(1)"),
            Some(StoredUrl::Plain("javascript:alert(1)"))
        );
        assert_eq!(
            stored_http_url("http:javascript:alert(1)"),
            Some(StoredUrl::Plain("http:javascript:alert(1)"))
        );
        assert_eq!(
            stored_http_url("data:text/html,hi"),
            Some(StoredUrl::Plain("data:text/html,hi"))
        );
    }

    #[test]
    fn match_detail_and_posts_use_the_stored_url_helpers() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let match_detail = std::fs::read_to_string(root.join("pages/match_detail.rs")).unwrap();
        assert!(match_detail.contains("stored_http_url"));
        assert!(
            !match_detail.contains("href: \"{url}\""),
            "vod_url must not be copied straight into an href"
        );
        let card = std::fs::read_to_string(root.join("components/post/card.rs")).unwrap();
        assert!(card.contains("linkify_http_spans"));
        assert!(
            !card.contains("match_indices(\"http\")"),
            "substring matching on http links javascript payloads"
        );
    }

    #[test]
    fn format_datetime_trims_iso_to_the_minute() {
        assert_eq!(
            format_datetime("2026-07-10T22:55:07.962043010Z"),
            "2026-07-10 22:55"
        );
        assert_eq!(
            format_datetime("2026-07-10T19:30:35.657Z"),
            "2026-07-10 19:30"
        );
        assert_eq!(format_datetime("not a timestamp"), "not a timestamp");
    }

    #[test]
    fn classify_fetch_keeps_loading_error_and_ready_distinct() {
        assert_eq!(
            classify_fetch(None::<&Result<Vec<u8>, String>>),
            FetchClass::Loading
        );
        let err: Option<Result<(), String>> = Some(Err("offline".to_string()));
        assert_eq!(classify_fetch(err.as_ref()), FetchClass::Error);
        let empty: Option<Result<Vec<u8>, String>> = Some(Ok(vec![]));
        assert_eq!(classify_fetch(empty.as_ref()), FetchClass::Ready);
        assert!(matches!(empty, Some(Ok(rows)) if rows.is_empty()));
        let rows: Option<Result<Vec<i32>, String>> = Some(Ok(vec![1, 2]));
        assert_eq!(classify_fetch(rows.as_ref()), FetchClass::Ready);
    }
}
