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
/// Only `http://` and `https://` (any ASCII case) are allowed. Anything else —
/// `javascript:`, `data:`, protocol-relative, relative, or a value with
/// whitespace, control characters, or a backslash — is `None`. Callers should
/// render `None` as plain text or omit it.
pub fn http_href(url: &str) -> Option<&str> {
    let url = url.trim();
    if url.is_empty() {
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

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    let bytes = value.as_bytes();
    let prefix = prefix.as_bytes();
    bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
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
pub fn season_url(path: &str, season: Option<String>) -> String {
    match season {
        Some(id) if !id.is_empty() => {
            let sep = if path.contains('?') { '&' } else { '?' };
            format!("{path}{sep}season={}", encode_query(&id))
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
