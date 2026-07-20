//! Small shared helpers used across pages and components.

/// Percent-encode a query-parameter value so values with spaces or non-ASCII
/// (e.g. "Soldier: 76", "Lúcio", nostr pubkeys/timestamps) survive the URL.
/// Keeps the RFC 3986 unreserved set literal (`A-Za-z0-9-_.~`); everything else
/// becomes `%XX` over UTF-8 bytes.
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
