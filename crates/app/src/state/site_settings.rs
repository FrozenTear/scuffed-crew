//! One `GET /api/settings` for public branding.
//!
//! Home, the public nav, the document title, and the community page used to
//! each fetch settings and paint the product default org name until the
//! response arrived. Pending and failed loads stay blank so that default
//! never flashes.
//!
//! The HTML shell may include the same JSON an anonymous `GET /api/settings`
//! returns, in `<script id="sc-settings" type="application/json">` just before
//! `</head>`. That block is read synchronously on boot so the first render can
//! paint the real accent, mark, and copy. `/api/settings` still revalidates.
//! A missing or invalid block is not the template: callers stay pending until
//! the fetch resolves, and a failed fetch with no prior value is an error.

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::SiteSettings;

use crate::util::{FetchClass, classify_fetch};

/// Element id of the server-injected public settings JSON.
pub const EMBEDDED_SETTINGS_ID: &str = "sc-settings";

#[derive(Clone, Copy)]
pub struct SiteSettingsState {
    resource: Resource<Result<SiteSettings, String>>,
    pub refresh: Signal<u32>,
    /// Last settings safe to paint: the embedded block, then each successful fetch.
    last_good: Signal<Option<SiteSettings>>,
}

/// Call once from the root `App` component.
pub fn provide_site_settings() -> SiteSettingsState {
    let refresh = use_signal(|| 0u32);
    // Synchronous: the script is already in the document when WASM starts.
    let last_good = use_signal(read_embedded_settings);
    let resource = use_resource(move || {
        let _tick = refresh();
        let mut last_good = last_good;
        async move {
            let fetched = ApiClient::web()
                .fetch::<SiteSettings>("/api/settings")
                .await
                .map_err(|err| err.to_string());
            let (next, result) = commit_fetch(last_good(), fetched);
            if result.is_ok() {
                last_good.set(next);
            }
            result
        }
    });
    let state = SiteSettingsState {
        resource,
        refresh,
        last_good,
    };
    use_context_provider(|| state);
    state
}

impl SiteSettingsState {
    /// Slot callers should classify. Embedded (or last successful) settings
    /// count as ready while a fetch is in flight or has failed.
    pub fn resolved(&self) -> Option<Result<SiteSettings, String>> {
        let fetched = self.resource.read();
        let embedded = self.last_good.read();
        resolve_settings(fetched.as_ref(), embedded.as_ref())
    }
}

/// Parse the text of `#sc-settings`. `None`, blank, and invalid JSON are absent.
pub fn parse_embedded_settings(json: Option<&str>) -> Option<SiteSettings> {
    let json = json?.trim();
    if json.is_empty() {
        return None;
    }
    match serde_json::from_str::<SiteSettings>(json) {
        Ok(settings) => Some(settings),
        Err(err) => {
            warn_embedded_parse_once(&err);
            None
        }
    }
}

/// A successful fetch replaces the painted settings. A failure leaves them.
fn commit_fetch(
    previous: Option<SiteSettings>,
    fetched: Result<SiteSettings, String>,
) -> (Option<SiteSettings>, Result<SiteSettings, String>) {
    match fetched {
        Ok(settings) => (Some(settings.clone()), Ok(settings)),
        Err(err) => (previous, Err(err)),
    }
}

fn warn_embedded_parse_once(err: &serde_json::Error) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::warn!("ignored invalid #sc-settings JSON: {err}");
}

/// Prefer a successful fetch. Otherwise keep embedded / last-good settings.
/// An error is visible only when nothing has been painted yet.
pub fn resolve_settings(
    fetched: Option<&Result<SiteSettings, String>>,
    embedded: Option<&SiteSettings>,
) -> Option<Result<SiteSettings, String>> {
    match (fetched, embedded) {
        (Some(Ok(settings)), _) => Some(Ok(settings.clone())),
        (_, Some(settings)) => Some(Ok(settings.clone())),
        (Some(Err(err)), None) => Some(Err(err.clone())),
        (None, None) => None,
    }
}

fn read_embedded_settings() -> Option<SiteSettings> {
    parse_embedded_settings(embedded_settings_text().as_deref())
}

#[cfg(test)]
thread_local! {
    static TEST_EMBEDDED_JSON: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_test_embedded_json(json: Option<String>) {
    TEST_EMBEDDED_JSON.with(|slot| *slot.borrow_mut() = json);
}

fn embedded_settings_text() -> Option<String> {
    #[cfg(test)]
    {
        if let Some(json) = TEST_EMBEDDED_JSON.with(|slot| slot.borrow().clone()) {
            return Some(json);
        }
    }
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        web_sys::window()?
            .document()?
            .get_element_by_id(EMBEDDED_SETTINGS_ID)?
            .text_content()
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        None
    }
}

pub fn use_site_settings() -> SiteSettingsState {
    use_context::<SiteSettingsState>()
}

/// `Some` only after a successful load. Pending and failed stay `None`
/// so callers cannot fall back to a fake org name.
pub fn loaded_site_settings(slot: Option<&Result<SiteSettings, String>>) -> Option<&SiteSettings> {
    match classify_fetch(slot) {
        FetchClass::Ready => slot.and_then(|result| result.as_ref().ok()),
        FetchClass::Loading | FetchClass::Error => None,
    }
}

/// Document title. Empty until the real org name is known.
pub fn document_title(org_name: Option<&str>) -> String {
    match org_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => name.to_string(),
        None => String::new(),
    }
}

/// True while the nav / hero should show a neutral placeholder, not a name.
pub fn brand_is_pending<T, E>(slot: Option<&Result<T, E>>) -> bool {
    !matches!(classify_fetch(slot), FetchClass::Ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_and_failed_settings_do_not_invent_an_org_name() {
        assert!(loaded_site_settings(None).is_none());
        let failed: Option<Result<SiteSettings, String>> = Some(Err("offline".into()));
        assert!(loaded_site_settings(failed.as_ref()).is_none());
        assert!(brand_is_pending(None::<&Result<(), ()>>));
        assert!(brand_is_pending(failed.as_ref()));
        assert_eq!(document_title(None), "");
        assert_eq!(document_title(Some("   ")), "");
        assert_ne!(document_title(None), "My Clan");
    }

    #[test]
    fn a_loaded_org_name_is_the_document_title() {
        let ready: Option<Result<&str, ()>> = Some(Ok("Clan"));
        assert!(!brand_is_pending(ready.as_ref()));
        assert_eq!(document_title(Some(" Night Owls ")), "Night Owls");
    }

    fn fixture(org_name: &str, hero_title: &str) -> String {
        let org = serde_json::to_string(org_name).unwrap();
        let hero = serde_json::to_string(hero_title).unwrap();
        let dark = serde_json::to_string(&format!("#{}", "15ac7d")).unwrap();
        let light = serde_json::to_string(&format!("#{}", "0e8f62")).unwrap();
        let mut out = String::from(r#"{"id":"site","org_name":"#);
        out.push_str(&org);
        out.push_str(
            r#","site_description":"desc","recruitment_open":true,"recruitment_message":"msg","min_age":16,"forum_backend":"local","extra_relay_urls":"","brand_accent_dark":"#,
        );
        out.push_str(&dark);
        out.push_str(r#","brand_accent_light":"#);
        out.push_str(&light);
        out.push_str(r#","homepage":{"hero_badge":"Community","hero_title":"#);
        out.push_str(&hero);
        out.push_str(
            r#","hero_title_accent":"Together","hero_sub":"Regular games.","cta_primary":"Join","cta_secondary":"Meet","ethos_kicker":"Join","ethos_title":"Pull up a chair","ethos_body":"Tell us.","ethos_rules":[],"teams_kicker":"Groups","teams_title":"Who","teams_empty":"No teams.","news_kicker":"n","news_title":"n","news_empty":"n","news_view_all":"n","tournaments_kicker":"n","tournaments_title":"n","tournaments_empty":"n","tournaments_view_all":"n","schedule_kicker":"n","schedule_title":"n","schedule_empty":"n","calendar_cta":"n","recruit_kicker":"n","recruit_title":"n","recruit_body":"n","recruit_cta":"n","recruit_expectations_title":"n","recruit_expectations":[],"never_ask_title":"n","never_ask_body":"n","seeking_label":"n","seeking_tags":[],"footer_note":""},"updated_at":"2026-10-06T00:00:00Z"}"#,
        );
        out
    }

    #[test]
    fn embedded_settings_parse_valid_invalid_and_absent() {
        assert!(parse_embedded_settings(None).is_none());
        assert!(parse_embedded_settings(Some("")).is_none());
        assert!(parse_embedded_settings(Some("   ")).is_none());
        assert!(parse_embedded_settings(Some("{")).is_none());
        assert!(parse_embedded_settings(Some("[]")).is_none());
        assert!(parse_embedded_settings(Some("null")).is_none());

        let parsed = parse_embedded_settings(Some(&fixture("Verified Org", "Play")))
            .expect("valid settings JSON");
        assert_eq!(parsed.org_name, "Verified Org");
        assert_eq!(parsed.homepage.hero_title, "Play");
        assert_eq!(parsed.homepage.hero_title_accent, "Together");
        assert_ne!(parsed.homepage.hero_title, "Your");
        assert_eq!(parsed.brand_accent_light, format!("#{}", "0e8f62"));

        // The server writes `<`, `>`, `&`, U+2028, and U+2029 as `\u` escapes.
        let escaped = fixture("X", "Play").replace(
            r#""org_name":"X""#,
            r#""org_name":"Crew \u003c/script\u003e\u0026\u2028\u2029""#,
        );
        let parsed = parse_embedded_settings(Some(&escaped)).expect("escaped JSON");
        assert_eq!(parsed.org_name, "Crew </script>&\u{2028}\u{2029}");
    }

    #[test]
    fn resolve_prefers_a_fresh_fetch_and_keeps_embedded_on_failure() {
        let embedded = parse_embedded_settings(Some(&fixture("Embedded Org", "Play"))).unwrap();
        let fetched = parse_embedded_settings(Some(&fixture("Fetched Org", "Night"))).unwrap();

        assert!(resolve_settings(None, None).is_none());

        let seeded = resolve_settings(None, Some(&embedded)).unwrap().unwrap();
        assert_eq!(seeded.org_name, "Embedded Org");
        assert_eq!(seeded.homepage.hero_title, "Play");

        let fresh = resolve_settings(Some(&Ok(fetched.clone())), Some(&embedded))
            .unwrap()
            .unwrap();
        assert_eq!(fresh.org_name, "Fetched Org");
        assert_eq!(fresh.homepage.hero_title, "Night");

        let kept = resolve_settings(Some(&Err("offline".into())), Some(&embedded))
            .unwrap()
            .unwrap();
        assert_eq!(kept.org_name, "Embedded Org");

        assert!(
            resolve_settings(Some(&Err("offline".into())), None)
                .unwrap()
                .is_err()
        );

        let (stored, result) = commit_fetch(Some(embedded.clone()), Ok(fetched.clone()));
        assert_eq!(stored.unwrap().org_name, "Fetched Org");
        assert_eq!(result.unwrap().org_name, "Fetched Org");
        let (kept, failed) = commit_fetch(Some(embedded), Err("offline".into()));
        assert_eq!(kept.unwrap().org_name, "Embedded Org");
        assert!(failed.is_err());
    }

    #[test]
    fn embedded_seed_is_visible_before_the_fetch() {
        set_test_embedded_json(Some(fixture("Seeded Org", "Play")));
        fn view() -> Element {
            let state = provide_site_settings();
            let resolved = state.resolved();
            let name = loaded_site_settings(resolved.as_ref())
                .map(|settings| settings.org_name.clone())
                .unwrap_or_default();
            rsx! { p { "{name}" } }
        }
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        set_test_embedded_json(None);
        assert!(html.contains("Seeded Org"), "{html}");
        assert!(!html.contains("Your"), "{html}");
        assert!(!html.contains("Gaming clan"), "{html}");
    }

    #[test]
    fn public_brand_surfaces_do_not_fall_back_to_my_clan() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for rel in [
            "pages/home/mod.rs",
            "layouts/public.rs",
            "main.rs",
            "pages/community.rs",
            "pages/apply.rs",
        ] {
            let text = std::fs::read_to_string(root.join(rel)).unwrap_or_else(|err| {
                panic!("read {rel}: {err}");
            });
            for needle in [
                "\"My Clan\".into()",
                "\"My Clan\".to_string()",
                "unwrap_or_else(|| \"My Clan\"",
                "\"The Scuffed Crew\".into()",
            ] {
                assert!(
                    !text.contains(needle),
                    "{rel} still paints a default org name ({needle})"
                );
            }
        }
    }
}
