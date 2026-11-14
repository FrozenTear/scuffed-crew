//! Author names for blog posts.
//!
//! Articles only carry `author_member_id`. Names come from the public member
//! profile (`GET /api/public/members/{id}`), the same endpoint the member
//! profile page uses. The raw id is never shown: a missing, inactive or failed
//! lookup reads as [`AUTHOR_FALLBACK`].
//!
//! Lookups are keyed per author. An id only gets an entry in [`AuthorNames`]
//! once its lookup has finished, so a stale or empty map from an earlier run
//! of the resource cannot stand in for an author that is still loading.

use std::collections::{HashMap, HashSet};

use dioxus::prelude::*;
use serde::Deserialize;

use super::public_fetch::{PublicFetch, fetch_public};

pub(crate) const AUTHOR_FALLBACK: &str = "Crew member";

/// Finished lookups: `Some(name)` on success, `None` when the lookup failed
/// or the member is not public. Ids still loading have no entry.
pub(crate) type AuthorNames = HashMap<String, Option<String>>;

#[derive(Deserialize)]
struct PublicMemberName {
    display_name: String,
}

/// Label for `member_id`, or `None` while its lookup has not finished.
pub(crate) fn author_label(names: Option<&AuthorNames>, member_id: &str) -> Option<String> {
    let resolved = names?.get(member_id)?;
    let name = resolved
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(AUTHOR_FALLBACK);
    Some(name.to_string())
}

/// Byline span for `member_id`, or nothing while its lookup is pending.
pub(crate) fn author_byline(names: Option<&AuthorNames>, member_id: &str) -> Element {
    match author_label(names, member_id) {
        Some(label) => rsx! { span { "by {label}" } },
        None => rsx! {},
    }
}

/// Non-empty ids in first-seen order, each once.
fn distinct_author_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| !id.is_empty() && seen.insert(id.clone()))
        .collect()
}

/// Look up display names for the given member ids, one request per distinct
/// id. Every looked-up id gets an entry, `None` if it failed to resolve.
pub(crate) async fn fetch_author_names(ids: Vec<String>) -> AuthorNames {
    let mut names = AuthorNames::new();
    for id in distinct_author_ids(ids) {
        let path = format!("/api/public/members/{}", crate::util::encode_query(&id));
        let name = match fetch_public::<PublicMemberName>(&path).await {
            PublicFetch::Found(m) => Some(m.display_name),
            _ => None,
        };
        names.insert(id, name);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "member:7f3a9c2e1b";

    #[test]
    fn known_author_shows_display_name() {
        let names = AuthorNames::from([(ID.to_string(), Some("Tear".to_string()))]);
        assert_eq!(author_label(Some(&names), ID).as_deref(), Some("Tear"));
    }

    #[test]
    fn pending_author_has_no_label() {
        // No map yet, or a map from an earlier run that lacks this id.
        let stale = AuthorNames::new();
        let other = AuthorNames::from([("member:other".to_string(), Some("X".to_string()))]);
        for names in [None, Some(&stale), Some(&other)] {
            assert_eq!(author_label(names, ID), None);
        }
    }

    #[test]
    fn raw_id_is_never_shown() {
        let failed = AuthorNames::from([(ID.to_string(), None)]);
        let blank = AuthorNames::from([(ID.to_string(), Some("   ".to_string()))]);
        for names in [&failed, &blank] {
            let label = author_label(Some(names), ID).expect("lookup finished");
            assert_eq!(label, AUTHOR_FALLBACK);
            assert!(!label.contains(ID));
        }
    }

    #[test]
    fn distinct_ids_dedup_and_skip_empty() {
        let ids = ["a", "b", "", "a", "c", "b"].map(String::from).to_vec();
        assert_eq!(distinct_author_ids(ids), ["a", "b", "c"]);
    }

    #[derive(Props, Clone, PartialEq)]
    struct BylineProbe {
        names: Option<AuthorNames>,
    }

    #[allow(non_snake_case)]
    fn Probe(props: BylineProbe) -> Element {
        rsx! { div { {author_byline(props.names.as_ref(), ID)} } }
    }

    fn render_byline(names: Option<AuthorNames>) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, BylineProbe { names });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn pending_state_renders_no_byline() {
        for names in [None, Some(AuthorNames::new())] {
            let html = render_byline(names);
            assert!(!html.contains("by "), "{html}");
            assert!(!html.contains(AUTHOR_FALLBACK), "{html}");
            assert!(!html.contains(ID), "{html}");
        }
    }

    #[test]
    fn hostile_display_name_renders_as_text() {
        let hostile = "<img src=x onerror=alert(1)>&";
        let names = AuthorNames::from([(ID.to_string(), Some(hostile.to_string()))]);
        let html = render_byline(Some(names));
        assert!(!html.contains("<img"), "{html}");
        assert!(
            html.contains("by &#60;img src=x onerror=alert(1)&#62;&#38;</span>"),
            "{html}"
        );
        assert!(html.contains("<span>by "), "{html}");
    }
}
