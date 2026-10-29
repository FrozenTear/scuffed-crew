//! Who can see the member's stats on the site, as the tracker reads it.
//!
//! The site stores one of `public`, `members` or `private` per member and
//! sends it as the optional `stats_visibility` string in the daemon-config
//! response. The tracker only shows it and never changes it. Anything it
//! does not know reads as "no answer", so the Settings line stays hidden
//! rather than guess. That covers a server too old to send the field.

pub use scuffed_types::api::StatsVisibility;

/// A daemon-config body is a few dozen bytes. Anything bigger is not one.
pub(super) const MAX_BODY_BYTES: usize = 16 * 1024;

/// The visibility in a daemon-config response, or `None` for anything that is
/// not a 2xx answer naming exactly one of the three known values: a failed
/// request, bad JSON, a missing or null field (older servers), or a value a newer site
/// sends that this tracker does not know.
pub fn parse_stats_visibility(status: u16, body: &str) -> Option<StatsVisibility> {
    if !(200..300).contains(&status) || body.len() > MAX_BODY_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    match value.get("stats_visibility")?.as_str()? {
        "public" => Some(StatsVisibility::Public),
        "members" => Some(StatsVisibility::Members),
        "private" => Some(StatsVisibility::Private),
        _ => None,
    }
}
