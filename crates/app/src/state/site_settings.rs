//! One `GET /api/settings` for public branding.
//!
//! Home, the public nav, the document title, and the community page used to
//! each fetch settings and paint the product default org name until the
//! response arrived. Pending and failed loads stay blank so that default
//! never flashes.

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::SiteSettings;

use crate::util::{FetchClass, classify_fetch};

#[derive(Clone, Copy)]
pub struct SiteSettingsState {
    pub resource: Resource<Result<SiteSettings, String>>,
    pub refresh: Signal<u32>,
}

/// Call once from the root `App` component.
pub fn provide_site_settings() -> SiteSettingsState {
    let refresh = use_signal(|| 0u32);
    let resource = use_resource(move || {
        let _tick = refresh();
        async move {
            ApiClient::web()
                .fetch::<SiteSettings>("/api/settings")
                .await
                .map_err(|err| err.to_string())
        }
    });
    let state = SiteSettingsState { resource, refresh };
    use_context_provider(|| state);
    state
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
