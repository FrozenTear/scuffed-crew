//! Author names for blog posts.
//!
//! Articles only carry `author_member_id`. Names come from the public member
//! profile (`GET /api/public/members/{id}`), the same endpoint the member
//! profile page uses. The raw id is never shown: a missing, inactive or failed
//! lookup reads as [`AUTHOR_FALLBACK`].

use std::collections::HashMap;

use serde::Deserialize;

use super::public_fetch::{PublicFetch, fetch_public};

pub(crate) const AUTHOR_FALLBACK: &str = "Crew member";

#[derive(Deserialize)]
struct PublicMemberName {
    display_name: String,
}

/// Display name for `member_id`, or [`AUTHOR_FALLBACK`].
pub(crate) fn author_label(names: Option<&HashMap<String, String>>, member_id: &str) -> String {
    names
        .and_then(|n| n.get(member_id))
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map_or_else(|| AUTHOR_FALLBACK.to_string(), str::to_string)
}

/// Look up display names for the given member ids, once per distinct id.
/// Ids that fail to resolve are left out of the map.
pub(crate) async fn fetch_author_names(ids: Vec<String>) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let path = format!("/api/public/members/{}", crate::util::encode_query(&id));
        if let PublicFetch::Found(m) = fetch_public::<PublicMemberName>(&path).await {
            names.insert(id, m.display_name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "member:7f3a9c2e1b";

    #[test]
    fn known_author_shows_display_name() {
        let names = HashMap::from([(ID.to_string(), "Tear".to_string())]);
        assert_eq!(author_label(Some(&names), ID), "Tear");
    }

    #[test]
    fn raw_id_is_never_shown() {
        let empty = HashMap::new();
        let blank = HashMap::from([(ID.to_string(), "   ".to_string())]);
        for names in [None, Some(&empty), Some(&blank)] {
            let label = author_label(names, ID);
            assert_eq!(label, AUTHOR_FALLBACK);
            assert!(!label.contains(ID));
        }
    }
}
