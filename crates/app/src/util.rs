//! Small shared helpers used across pages and components.

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
