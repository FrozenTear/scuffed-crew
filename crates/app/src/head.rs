//! Per-route document head for the WASM app.
//!
//! `document::Title` and `document::Meta` append a new tag and then ignore
//! later prop changes, so a navigation would leave the previous title and
//! description in the document. The web build writes the existing tags through
//! `web_sys` (`Document::set_title` does not eval a string, which the site CSP
//! forbids). Desktop still uses `document::Title` for the window title.
//!
//! `og:url` is absolute only when settings name a public http(s) origin
//! under `public_base_url`. Without that origin the tag is left out.
//! Canonical stays on the route path, which the browser resolves against
//! the current document. No deploy host is baked in.

use dioxus::prelude::*;

use crate::routes::Route;
use crate::state::site_settings::SiteSettingsState;
use crate::state::{document_title, loaded_site_settings, use_site_settings};

/// Shell `<title>` before an org name is known. Matches `Dioxus.toml`.
pub const SHELL_TITLE: &str = "Community";

/// Shell description before site settings supply one. Matches `index.html`.
pub const SHELL_DESCRIPTION: &str = "A community site for teams, events, and scheduled play.";

/// Faces the stylesheets actually set. Space Grotesk 500 is not used on
/// `--font-head` (body/UI 500 is Inter). `display=swap` keeps text visible
/// while the files download. The files stay on Google Fonts.
pub const FONT_STYLESHEET: &str = "https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=Space+Grotesk:wght@400;600;700&family=JetBrains+Mono:wght@400;500;600;700&display=swap";

const OG_IMAGE_PATH: &str = "/assets/apple-touch-icon.png";
const TITLE_SEP: &str = " | ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentHead {
    pub title: String,
    pub description: String,
    pub og_title: String,
    pub og_description: String,
    pub og_type: &'static str,
    pub og_site_name: String,
    pub og_url: Option<String>,
    pub og_image: Option<String>,
    pub canonical: Option<String>,
}

/// What one head slot should become. `Clear` drops every copy so a private
/// page cannot keep the previous public canonical.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupWrite {
    Replace(String),
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadPlan {
    pub title: GroupWrite,
    pub description: GroupWrite,
    pub og_title: GroupWrite,
    pub og_description: GroupWrite,
    pub og_type: GroupWrite,
    pub og_site_name: GroupWrite,
    pub og_url: GroupWrite,
    pub og_image: GroupWrite,
    pub canonical: GroupWrite,
}

/// In-memory head used by tests. The DOM writer follows the same rule:
/// replace leaves one value, clear leaves none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadSlots {
    pub title: Vec<String>,
    pub description: Vec<String>,
    pub og_title: Vec<String>,
    pub og_description: Vec<String>,
    pub og_type: Vec<String>,
    pub og_site_name: Vec<String>,
    pub og_url: Vec<String>,
    pub og_image: Vec<String>,
    pub canonical: Vec<String>,
}

enum PageLabel {
    Home,
    Named(String),
}

#[component]
pub fn HeadLayout() -> Element {
    let site_settings = use_site_settings();
    use_effect(move || {
        let route = router().current::<Route>();
        let head = current_head(site_settings, &route);
        install_document_head(&head);
    });
    let route = use_route::<Route>();
    let head = current_head(site_settings, &route);
    rsx! {
        {desktop_title_element(&head.title)}
        Outlet::<Route> {}
    }
}

fn current_head(site_settings: SiteSettingsState, route: &Route) -> DocumentHead {
    let resolved = site_settings.resolved.read();
    let loaded = loaded_site_settings(resolved.as_ref());
    let org = loaded.map(|settings| settings.org_name.as_str());
    let description = loaded
        .map(|settings| settings.site_description.as_str())
        .unwrap_or("");
    document_head(route, org, description, configured_public_base().as_deref())
}

/// Desktop window title. The web build sets `document.title` with `web_sys`
/// instead, so Dioxus does not eval the assignment.
fn desktop_title_element(title: &str) -> Element {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        let _ = title;
        rsx! {}
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        rsx! {
            document::Title { "{title}" }
        }
    }
}

pub fn document_head(
    route: &Route,
    org_name: Option<&str>,
    site_description: &str,
    public_base: Option<&str>,
) -> DocumentHead {
    let org = document_title(org_name);
    let base = sanitize_public_base(public_base);
    let title = compose_title(route, &org);
    let description = compose_description(route, site_description);
    let site_name = if org.is_empty() {
        SHELL_TITLE.to_string()
    } else {
        org
    };
    let (og_url, og_image, canonical) = if is_public_route(route) {
        let path = route_path(route);
        // A relative og:url is dropped. Crawlers that do not run wasm only
        // see the shell, and a relative og:url is not a document URL.
        // Canonical may stay on the path.
        let og_url = base.as_ref().map(|origin| format!("{origin}{path}"));
        (
            og_url,
            Some(join_base(base.as_deref(), OG_IMAGE_PATH)),
            Some(join_base(base.as_deref(), &path)),
        )
    } else {
        (None, None, None)
    };
    DocumentHead {
        og_title: title.clone(),
        og_description: description.clone(),
        og_type: "website",
        og_site_name: site_name,
        og_url,
        og_image,
        canonical,
        title,
        description,
    }
}

pub fn plan_head(head: &DocumentHead) -> HeadPlan {
    HeadPlan {
        title: GroupWrite::Replace(head.title.clone()),
        description: GroupWrite::Replace(head.description.clone()),
        og_title: GroupWrite::Replace(head.og_title.clone()),
        og_description: GroupWrite::Replace(head.og_description.clone()),
        og_type: GroupWrite::Replace(head.og_type.to_string()),
        og_site_name: GroupWrite::Replace(head.og_site_name.clone()),
        og_url: optional_write(head.og_url.clone()),
        og_image: optional_write(head.og_image.clone()),
        canonical: optional_write(head.canonical.clone()),
    }
}

fn optional_write(value: Option<String>) -> GroupWrite {
    match value {
        Some(value) => GroupWrite::Replace(value),
        None => GroupWrite::Clear,
    }
}

/// Collapse one slot. A replace drops every previous value, including
/// duplicates, then stores the new one.
pub fn write_group(mut existing: Vec<String>, write: &GroupWrite) -> Vec<String> {
    existing.clear();
    if let GroupWrite::Replace(value) = write {
        existing.push(value.clone());
    }
    existing
}

pub fn apply_plan(mut slots: HeadSlots, plan: &HeadPlan) -> HeadSlots {
    slots.title = write_group(slots.title, &plan.title);
    slots.description = write_group(slots.description, &plan.description);
    slots.og_title = write_group(slots.og_title, &plan.og_title);
    slots.og_description = write_group(slots.og_description, &plan.og_description);
    slots.og_type = write_group(slots.og_type, &plan.og_type);
    slots.og_site_name = write_group(slots.og_site_name, &plan.og_site_name);
    slots.og_url = write_group(slots.og_url, &plan.og_url);
    slots.og_image = write_group(slots.og_image, &plan.og_image);
    slots.canonical = write_group(slots.canonical, &plan.canonical);
    slots
}

/// Public origin from anonymous settings JSON.
///
/// `SiteSettings` has no base field today. Only `public_base_url` is read,
/// and only when it is a public http(s) origin with no path. Loopback and
/// anything else fall through.
pub fn public_base_from_settings_json(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let raw = value.get("public_base_url")?.as_str()?;
    sanitize_public_base(Some(raw))
}

pub fn sanitize_public_base(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let (scheme, rest) = split_http_scheme(raw)?;
    // An origin may end with one slash. A real path still contains `/`.
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty()
        || rest.contains('@')
        || rest.contains('/')
        || rest.contains('?')
        || rest.contains('#')
        || rest.contains('\\')
        || rest.contains(' ')
    {
        return None;
    }
    let host = rest.split(':').next()?.trim();
    if !is_public_host(host) {
        return None;
    }
    Some(format!("{scheme}://{rest}"))
}

fn split_http_scheme(raw: &str) -> Option<(&'static str, &str)> {
    for (scheme, prefix) in [("https", "https://"), ("http", "http://")] {
        if raw.len() >= prefix.len() && raw[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return Some((scheme, &raw[prefix.len()..]));
        }
    }
    None
}

fn is_public_host(host: &str) -> bool {
    if host.is_empty() || host.starts_with('[') || host.starts_with('.') {
        return false;
    }
    let host = host.trim_matches('.');
    if host.eq_ignore_ascii_case("localhost")
        || host.eq_ignore_ascii_case("127.0.0.1")
        || host.eq_ignore_ascii_case("::1")
        || host.eq_ignore_ascii_case("0.0.0.0")
    {
        return false;
    }
    let suffix = host.rsplit('.').next().unwrap_or(host);
    !suffix.eq_ignore_ascii_case("local") && !suffix.eq_ignore_ascii_case("localhost")
}

fn compose_title(route: &Route, org: &str) -> String {
    match page_label(route) {
        PageLabel::Home => {
            if org.is_empty() {
                SHELL_TITLE.to_string()
            } else {
                org.to_string()
            }
        }
        PageLabel::Named(name) => {
            if org.is_empty() {
                name
            } else {
                format!("{name}{TITLE_SEP}{org}")
            }
        }
    }
}

fn compose_description(route: &Route, site_description: &str) -> String {
    let site = site_description.trim();
    match page_label(route) {
        PageLabel::Home => {
            if site.is_empty() {
                SHELL_DESCRIPTION.to_string()
            } else {
                site.to_string()
            }
        }
        PageLabel::Named(name) => {
            if site.is_empty() {
                format!("{name} on this community site.")
            } else {
                format!("{name}. {site}")
            }
        }
    }
}

fn page_label(route: &Route) -> PageLabel {
    match route {
        Route::Home {} => PageLabel::Home,
        Route::Setup {} => named("Setup"),
        Route::Login {} => named("Log in"),
        Route::LinkDevice {} => named("Link a device"),
        Route::Members {} => named("Members"),
        Route::MemberProfile { id } => named(format!("Member | {id}")),
        Route::TeamPage { id } => named(format!("Team | {id}")),
        Route::MatchDetail { id } => named(format!("Match | {id}")),
        Route::News {} => named("News"),
        Route::Apply {} => named("Apply"),
        Route::Tournaments {} => named("Tournaments"),
        Route::Tournament { id } => named(format!("Tournament | {id}")),
        Route::IdentitySettings {} => named("Identity"),
        Route::ProfileSettings {} => named("Profile"),
        Route::MyReports {} => named("Reports"),
        Route::DmInbox {} => named("Messages"),
        Route::DmThread { .. } => named("Message"),
        Route::TeamChat {} => named("Chat"),
        Route::Community {} => named("Community"),
        Route::Feed {} => named("Feed"),
        Route::Polls {} => named("Polls"),
        Route::Scrims {} => named("Scrims"),
        Route::Events {} => named("Events"),
        Route::Blog {} => named("Blog"),
        Route::BlogPost { slug } => named(format!("Blog | {slug}")),
        Route::Wiki {} => named("Wiki"),
        Route::WikiPage { topic } => named(format!("Wiki | {topic}")),
        Route::Forum {} => named("Forum"),
        Route::ForumBoardPage { slug } => named(format!("Forum | {slug}")),
        Route::ForumThread { id } => named(format!("Thread | {id}")),
        Route::Leaderboards {} => named("Leaderboards"),
        Route::Stats {} => named("Stats"),
        Route::StatsTokens {} => named("Stats tokens"),
        Route::StatsMember { id } => named(format!("Player stats | {id}")),
        Route::PatchNotes {} => named("Patch notes"),
        Route::AdminDashboard {} => named("Admin dashboard"),
        Route::AdminMembers {} => named("Admin members"),
        Route::AdminGames {} => named("Admin games"),
        Route::AdminTeams {} => named("Admin teams"),
        Route::AdminSchedule {} => named("Admin schedule"),
        Route::AdminApplications {} => named("Admin applications"),
        Route::AdminMatches {} => named("Admin matches"),
        Route::AdminSeasons {} => named("Admin seasons"),
        Route::AdminTournaments {} => named("Admin tournaments"),
        Route::AdminAnnouncements {} => named("Admin announcements"),
        Route::AdminArticles {} => named("Admin articles"),
        Route::AdminPatchNotes {} => named("Admin patch notes"),
        Route::AdminForum {} => named("Admin forum"),
        Route::AdminAuditLog {} => named("Admin audit log"),
        Route::AdminModeration {} => named("Admin moderation"),
        Route::AdminReports {} => named("Admin reports"),
        Route::AdminRelay {} => named("Admin relay"),
        Route::AdminSettings {} => named("Admin settings"),
        Route::StrategyBrowse {} => named("Strategies"),
        Route::StrategyMy {} => named("My strategies"),
        Route::StrategyHeroes {} => named("Heroes"),
        Route::StrategyMeta {} => named("Meta"),
        Route::StrategyPatchNotes {} => named("Strategy patch notes"),
        Route::StrategyEditorNew {} => named("Strategy editor"),
        Route::StrategyEditor { id } => named(format!("Strategy editor | {id}")),
        Route::NotFound { .. } => named("Page not found"),
    }
}

fn named(label: impl Into<String>) -> PageLabel {
    PageLabel::Named(label.into())
}

fn is_public_route(route: &Route) -> bool {
    matches!(
        route,
        Route::Home {}
            | Route::Members {}
            | Route::MemberProfile { .. }
            | Route::TeamPage { .. }
            | Route::MatchDetail { .. }
            | Route::News {}
            | Route::Apply {}
            | Route::Tournaments {}
            | Route::Tournament { .. }
            | Route::Community {}
            | Route::Feed {}
            | Route::Events {}
            | Route::Blog {}
            | Route::BlogPost { .. }
            | Route::Wiki {}
            | Route::WikiPage { .. }
            | Route::Forum {}
            | Route::ForumBoardPage { .. }
            | Route::ForumThread { .. }
            | Route::Leaderboards {}
            | Route::PatchNotes {}
            | Route::StrategyBrowse {}
            | Route::StrategyHeroes {}
            | Route::StrategyMeta {}
            | Route::StrategyPatchNotes {}
    )
}

fn route_path(route: &Route) -> String {
    let path = route.to_string();
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}

fn join_base(base: Option<&str>, path: &str) -> String {
    match base {
        Some(base) => format!("{base}{path}"),
        None => path.to_string(),
    }
}

fn configured_public_base() -> Option<String> {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        if let Some(json) = crate::state::site_settings::embedded_settings_json()
            && let Some(base) = public_base_from_settings_json(&json)
        {
            return Some(base);
        }
    }
    None
}

fn install_document_head(head: &DocumentHead) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    apply_dom(&plan_head(head), &head.title);
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = head;
    }
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn apply_dom(plan: &HeadPlan, title: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(head_el) = document.query_selector("head").ok().flatten() else {
        return;
    };
    // Same contract as `write_group`: one remaining value, or none.
    sync_nodes(
        &document,
        child_nodes(&head_el, "title"),
        &plan.title,
        "title",
        &[],
        None,
    );
    document.set_title(title);
    sync_nodes(
        &document,
        meta_nodes(&head_el, "name", "description"),
        &plan.description,
        "meta",
        &[("name", "description")],
        Some("content"),
    );
    sync_nodes(
        &document,
        meta_nodes(&head_el, "property", "og:title"),
        &plan.og_title,
        "meta",
        &[("property", "og:title")],
        Some("content"),
    );
    sync_nodes(
        &document,
        meta_nodes(&head_el, "property", "og:description"),
        &plan.og_description,
        "meta",
        &[("property", "og:description")],
        Some("content"),
    );
    sync_nodes(
        &document,
        meta_nodes(&head_el, "property", "og:type"),
        &plan.og_type,
        "meta",
        &[("property", "og:type")],
        Some("content"),
    );
    sync_nodes(
        &document,
        meta_nodes(&head_el, "property", "og:site_name"),
        &plan.og_site_name,
        "meta",
        &[("property", "og:site_name")],
        Some("content"),
    );
    sync_nodes(
        &document,
        meta_nodes(&head_el, "property", "og:url"),
        &plan.og_url,
        "meta",
        &[("property", "og:url")],
        Some("content"),
    );
    sync_nodes(
        &document,
        meta_nodes(&head_el, "property", "og:image"),
        &plan.og_image,
        "meta",
        &[("property", "og:image")],
        Some("content"),
    );
    sync_nodes(
        &document,
        link_nodes(&head_el, "canonical"),
        &plan.canonical,
        "link",
        &[("rel", "canonical")],
        Some("href"),
    );
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn sync_nodes(
    document: &web_sys::Document,
    nodes: Vec<web_sys::Element>,
    write: &GroupWrite,
    tag: &str,
    attrs: &[(&str, &str)],
    value_attr: Option<&str>,
) {
    match write {
        GroupWrite::Clear => {
            for node in nodes {
                remove_node(&node);
            }
        }
        GroupWrite::Replace(value) => {
            let node = if let Some(first) = nodes.first() {
                for extra in nodes.iter().skip(1) {
                    remove_node(extra);
                }
                first.clone()
            } else {
                let Some(created) = create_in_head(document, tag, attrs) else {
                    return;
                };
                created
            };
            set_node_value(&node, value, value_attr);
        }
    }
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn set_node_value(node: &web_sys::Element, value: &str, value_attr: Option<&str>) {
    use wasm_bindgen::JsCast;

    if let Some(attr) = value_attr {
        let _ = node.set_attribute(attr, value);
    } else {
        let text: &web_sys::Node = node.unchecked_ref();
        text.set_text_content(Some(value));
    }
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn create_in_head(
    document: &web_sys::Document,
    tag: &str,
    attrs: &[(&str, &str)],
) -> Option<web_sys::Element> {
    use wasm_bindgen::JsCast;

    let head = document.query_selector("head").ok().flatten()?;
    let el = document.create_element(tag).ok()?;
    for (name, value) in attrs {
        let _ = el.set_attribute(name, value);
    }
    let head_node: &web_sys::Node = head.unchecked_ref();
    let el_node: &web_sys::Node = el.unchecked_ref();
    head_node.append_child(el_node).ok()?;
    Some(el)
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn remove_node(el: &web_sys::Element) {
    use wasm_bindgen::JsCast;

    let node: &web_sys::Node = el.unchecked_ref();
    if let Some(parent) = node.parent_node() {
        let _ = parent.remove_child(node);
    }
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn child_nodes(head: &web_sys::Element, tag: &str) -> Vec<web_sys::Element> {
    let Ok(list) = head.query_selector_all(tag) else {
        return Vec::new();
    };
    nodes_from_list(&list)
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn meta_nodes(head: &web_sys::Element, attr: &str, expected: &str) -> Vec<web_sys::Element> {
    child_nodes(head, "meta")
        .into_iter()
        .filter(|el| {
            el.get_attribute(attr)
                .is_some_and(|value| value.eq_ignore_ascii_case(expected))
        })
        .collect()
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn link_nodes(head: &web_sys::Element, rel: &str) -> Vec<web_sys::Element> {
    child_nodes(head, "link")
        .into_iter()
        .filter(|el| {
            el.get_attribute("rel").is_some_and(|value| {
                value
                    .split_whitespace()
                    .any(|part| part.eq_ignore_ascii_case(rel))
            })
        })
        .collect()
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn nodes_from_list(list: &web_sys::NodeList) -> Vec<web_sys::Element> {
    use wasm_bindgen::JsCast;

    let mut out = Vec::new();
    for index in 0..list.length() {
        let Some(node) = list.item(index) else {
            continue;
        };
        if let Ok(element) = node.dyn_into::<web_sys::Element>() {
            out.push(element);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public_fixtures() -> Vec<Route> {
        vec![
            Route::Home {},
            Route::Members {},
            Route::MemberProfile { id: "alpha".into() },
            Route::MemberProfile { id: "beta".into() },
            Route::TeamPage { id: "t1".into() },
            Route::MatchDetail { id: "m1".into() },
            Route::News {},
            Route::Apply {},
            Route::Tournaments {},
            Route::Tournament { id: "cup".into() },
            Route::Community {},
            Route::Feed {},
            Route::Events {},
            Route::Blog {},
            Route::BlogPost {
                slug: "hello".into(),
            },
            Route::BlogPost {
                slug: "world".into(),
            },
            Route::Wiki {},
            Route::WikiPage {
                topic: "rules".into(),
            },
            Route::Forum {},
            Route::ForumBoardPage {
                slug: "news".into(),
            },
            Route::ForumThread { id: "42".into() },
            Route::Leaderboards {},
            Route::PatchNotes {},
            Route::StrategyBrowse {},
            Route::StrategyHeroes {},
            Route::StrategyMeta {},
            Route::StrategyPatchNotes {},
        ]
    }

    #[test]
    fn each_public_route_sets_its_own_title_and_a_relative_canonical() {
        let mut titles = Vec::new();
        for route in public_fixtures() {
            let head = document_head(&route, Some(" Owls "), " Night games. ", None);
            let path = route.to_string();
            assert!(path.starts_with('/'), "{path}");
            if matches!(route, Route::Home {}) {
                assert_eq!(head.title, "Owls");
                assert_eq!(head.description, "Night games.");
            } else {
                assert!(head.title.ends_with(" | Owls"), "{}", head.title);
                assert_ne!(head.title, "Owls");
                assert!(
                    head.description
                        .starts_with(&head.title[..head.title.len() - " | Owls".len()])
                );
                assert!(
                    head.description.ends_with("Night games."),
                    "{}",
                    head.description
                );
            }
            assert_eq!(head.og_title, head.title);
            assert_eq!(head.og_description, head.description);
            assert_eq!(head.og_type, "website");
            assert_eq!(head.og_site_name, "Owls");
            assert_eq!(head.canonical.as_deref(), Some(path.as_str()));
            assert!(head.og_url.is_none(), "no relative og:url without a base");
            assert_eq!(head.og_image.as_deref(), Some(OG_IMAGE_PATH));
            assert!(!head.canonical.as_deref().unwrap().contains("://"));
            titles.push(head.title);
        }
        let mut unique = titles.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), titles.len(), "{titles:?}");
    }

    #[test]
    fn public_base_from_settings_is_absolute_and_loopback_stays_relative() {
        let head = document_head(
            &Route::News {},
            Some("Owls"),
            "Night games.",
            Some("https://example.com/"),
        );
        assert_eq!(head.canonical.as_deref(), Some("https://example.com/news"));
        assert_eq!(head.og_url.as_deref(), Some("https://example.com/news"));
        assert_eq!(
            head.og_image.as_deref(),
            Some("https://example.com/assets/apple-touch-icon.png")
        );
        assert_eq!(
            document_head(
                &Route::ForumBoardPage {
                    slug: "news".into()
                },
                Some("Owls"),
                "d",
                Some("https://example.com"),
            )
            .canonical
            .as_deref(),
            Some("https://example.com/forum/b/news")
        );

        for bad in [
            "http://localhost:3030",
            "http://127.0.0.1:3000",
            "https://user:pass@example.com",
            "https://example.com/path",
            "not a url",
            "",
            "   ",
        ] {
            let head = document_head(&Route::News {}, Some("Owls"), "d", Some(bad));
            assert_eq!(head.canonical.as_deref(), Some("/news"), "{bad}");
            assert!(head.og_url.is_none(), "{bad}");
        }
    }

    #[test]
    fn settings_json_base_skips_missing_and_invalid_keys() {
        assert!(public_base_from_settings_json("").is_none());
        assert!(public_base_from_settings_json("[]").is_none());
        assert!(public_base_from_settings_json(r#"{"org_name":"Owls"}"#).is_none());
        assert!(
            public_base_from_settings_json(r#"{"public_base_url":"http://localhost"}"#).is_none()
        );
        assert!(
            public_base_from_settings_json(r#"{"public_url":"https://example.com/"}"#).is_none()
        );
        assert!(
            public_base_from_settings_json(r#"{"canonical_base":"https://example.com"}"#).is_none()
        );
        assert_eq!(
            public_base_from_settings_json(r#"{"public_base_url":"https://example.com/"}"#)
                .as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn private_routes_clear_canonical_and_do_not_reuse_the_public_title() {
        let admin = document_head(
            &Route::AdminMembers {},
            Some("Owls"),
            "Night games.",
            Some("https://example.com"),
        );
        let members = document_head(&Route::Members {}, Some("Owls"), "Night games.", None);
        assert!(admin.canonical.is_none());
        assert!(admin.og_url.is_none());
        assert!(admin.og_image.is_none());
        assert_eq!(admin.title, "Admin members | Owls");
        assert_ne!(admin.title, members.title);
        assert_ne!(admin.description, members.description);
        assert_eq!(
            document_head(&Route::Home {}, None, "   ", None).title,
            SHELL_TITLE
        );
        assert_eq!(
            document_head(&Route::Members {}, None, "", None).title,
            "Members"
        );
    }

    #[test]
    fn navigation_replaces_title_and_description_without_duplicates() {
        let mut slots = HeadSlots {
            title: vec!["Community".into(), "STALE TITLE".into()],
            description: vec![SHELL_DESCRIPTION.into(), "STALE DESC".into()],
            og_title: vec!["Community".into()],
            og_description: vec![SHELL_DESCRIPTION.into(), "STALE OG".into()],
            og_type: vec!["website".into(), "website".into()],
            og_site_name: vec!["Community".into()],
            og_url: vec!["/".into()],
            og_image: vec![OG_IMAGE_PATH.into()],
            canonical: vec!["/".into(), "/stale".into()],
        };
        slots = apply_plan(
            slots,
            &plan_head(&document_head(
                &Route::Members {},
                Some("Owls"),
                "Night games.",
                None,
            )),
        );
        assert_singular(&slots);
        assert_eq!(slots.title, vec!["Members | Owls".to_string()]);
        assert_eq!(slots.description, vec!["Members. Night games.".to_string()]);
        assert_eq!(slots.canonical, vec!["/members".to_string()]);
        assert!(
            slots.og_url.is_empty(),
            "a public page with no base clears og:url"
        );
        assert!(slots.title.iter().all(|value| !value.contains("STALE")));
        assert!(
            slots
                .description
                .iter()
                .all(|value| !value.contains("STALE"))
        );
        assert!(slots.canonical.iter().all(|value| value != "/stale"));

        slots = apply_plan(
            slots,
            &plan_head(&document_head(
                &Route::News {},
                Some("Owls"),
                "Night games.",
                Some("https://example.com"),
            )),
        );
        assert_singular(&slots);
        assert_eq!(slots.title, vec!["News | Owls".to_string()]);
        assert_eq!(slots.description, vec!["News. Night games.".to_string()]);
        assert_eq!(slots.og_title, slots.title);
        assert_eq!(slots.og_description, slots.description);
        assert_eq!(
            slots.canonical,
            vec!["https://example.com/news".to_string()]
        );
        assert_eq!(slots.og_url, slots.canonical);
        assert_eq!(
            slots.og_image,
            vec!["https://example.com/assets/apple-touch-icon.png".to_string()]
        );

        slots = apply_plan(
            slots,
            &plan_head(&document_head(
                &Route::AdminDashboard {},
                Some("Owls"),
                "Night games.",
                Some("https://example.com"),
            )),
        );
        assert_singular(&slots);
        assert_eq!(slots.title, vec!["Admin dashboard | Owls".to_string()]);
        assert_ne!(slots.description, vec!["News. Night games.".to_string()]);
        assert!(slots.canonical.is_empty());
        assert!(slots.og_url.is_empty());
        assert!(slots.og_image.is_empty());
        assert_eq!(slots.og_type, vec!["website".to_string()]);
    }

    fn assert_singular(slots: &HeadSlots) {
        assert_eq!(slots.title.len(), 1, "{slots:?}");
        assert_eq!(slots.description.len(), 1);
        assert_eq!(slots.og_title.len(), 1);
        assert_eq!(slots.og_description.len(), 1);
        assert_eq!(slots.og_type.len(), 1);
        assert_eq!(slots.og_site_name.len(), 1);
        assert!(slots.og_url.len() <= 1);
        assert!(slots.og_image.len() <= 1);
        assert!(slots.canonical.len() <= 1);
    }

    #[test]
    fn font_request_uses_display_swap_and_drops_unused_space_grotesk_500() {
        assert!(FONT_STYLESHEET.contains("display=swap"));
        assert!(FONT_STYLESHEET.contains("Inter:wght@400;500;600;700"));
        assert!(FONT_STYLESHEET.contains("Space+Grotesk:wght@400;600;700"));
        assert!(!FONT_STYLESHEET.contains("Space+Grotesk:wght@500"));
        assert!(FONT_STYLESHEET.contains("JetBrains+Mono:wght@400;500;600;700"));
        assert!(!FONT_STYLESHEET.contains("fonts.gstatic.com/s/"));
    }

    #[test]
    fn head_source_does_not_hardcode_a_deploy_host() {
        let src = include_str!("head.rs");
        let end = src.find("mod tests").expect("tests");
        let code = &src[..end];
        assert!(!code.to_ascii_lowercase().contains("contabo"));
        assert!(!code.contains("scuffedcrew.no"));
    }

    #[test]
    fn favicon_fallbacks_match_the_pending_mark_size() {
        let png = include_bytes!("../assets/favicon-32.png");
        let apple = include_bytes!("../assets/apple-touch-icon.png");
        let ico = include_bytes!("../assets/favicon.ico");
        assert_eq!(png_size(png), (32, 32));
        assert_eq!(png_size(apple), (180, 180));
        assert_eq!(&ico[..4], &[0, 0, 1, 0]);
        assert_eq!(ico[4], 1, "one image");
        assert_eq!(ico[6], 32);
        assert_eq!(ico[7], 32);
    }

    fn png_size(bytes: &[u8]) -> (u32, u32) {
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "png magic");
        assert_eq!(&bytes[12..16], b"IHDR");
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        (width, height)
    }
}
