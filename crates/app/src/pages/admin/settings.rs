use dioxus::prelude::*;

use crate::components::{AccessDenied, Toast, use_toast};
use crate::routes::Route;
use crate::state::site_settings::SiteSettingsState;
use crate::state::use_auth;
use crate::theme::brand::{AccentCheck, BrandConfig};
use crate::theme::tokens::BG_DARK;
use scuffed_api_client::ApiClient;
use scuffed_types::api::UpdateSettingsRequest;
use scuffed_types::{
    ContentAlign, HomeShell, HomeSkin, HomepageContent, PublicLayout, SiteSettings,
    homepage_preset_by_id, homepage_presets,
};

/// Two pages send `PUT /api/settings`, both admin-only: this one and brand
/// settings, whose body carries the accent pair (and `brand_font_display`
/// when the admin changed the font) and no other settings key. This page
/// owns the officers flag, so it always sends the key. `None` would omit it
/// (`skip_serializing_if`) and leave a stale flag on the server.
fn admin_settings_officers_flag(enabled: bool) -> Option<bool> {
    Some(enabled)
}

/// The team form edits name, game, color, and division. It has no lore-quote
/// field, so this hint must not say the switch edits one.
fn officers_can_edit_teams_hint() -> &'static str {
    "On: Officers can edit a team's name, game, color, and division in Admin → Teams. Lore quote is not on that form, so this switch does not change it. Creating teams stays admin-only. Off (default): those edits stay admin-only. Roster changes are not affected."
}

/// The Save bar, by the class it renders with.
const SAVE_BAR_SELECTOR: &str = ".settings-save-bar";

/// Custom property on the root that holds the Save bar's measured height.
/// The page's `scroll-padding-bottom` reserves the tab bar plus this, so
/// the strip the browser keeps clear is always as tall as the bar really
/// is, also when a failed save adds a line to it (QA 706 fix4).
const SAVE_BAR_HEIGHT_VAR: &str = "--settings-save-bar-height";

/// Keep [`SAVE_BAR_HEIGHT_VAR`] equal to the bar's border-box height
/// (`onresize`, a ResizeObserver, runs on mount and on every change), and
/// remove it when the page goes (`None`).
#[cfg(target_arch = "wasm32")]
fn sync_save_bar_height(height: Option<f64>) {
    use wasm_bindgen::JsCast;
    let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
        .and_then(|root| root.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    let style = root.style();
    match height.filter(|height| height.is_finite() && *height > 0.0) {
        Some(height) => {
            let _ = style.set_property(SAVE_BAR_HEIGHT_VAR, &format!("{}px", height.ceil()));
        }
        None => {
            let _ = style.remove_property(SAVE_BAR_HEIGHT_VAR);
        }
    }
}

/// Outside the browser there is no page to scroll.
#[cfg(not(target_arch = "wasm32"))]
fn sync_save_bar_height(_height: Option<f64>) {}

/// Firefox scrolls a newly focused control into view only when none of it
/// is inside the scroll-padded view. A control that straddles the padding
/// line (the top of the stuck Save bar) stays where it is, behind the bar.
/// Chromium scrolls it fully clear. So, after a keyboard focus move in the
/// form, a focused control that reaches below the top of the bar is scrolled
/// up until its bottom is on the bar's top, measured against the bar's live
/// box, so a bar that has just grown is cleared too.
///
/// Only keyboard focus (`:focus-visible`): a mouse press focuses on
/// `mousedown`, and a scroll then would move the control out from under
/// the pointer. Controls inside the bar itself are left alone.
#[cfg(target_arch = "wasm32")]
fn keep_focus_above_save_bar() {
    use crate::layouts::focus_reveal::{TOP_BAR_SELECTOR, scroll_focus_into_band};
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(focused) = document.active_element() else {
        return;
    };
    if !focused.matches(":focus-visible").unwrap_or(false) {
        return;
    }
    let Some(bar) = document.query_selector(SAVE_BAR_SELECTOR).ok().flatten() else {
        return;
    };
    if bar.contains(Some(&focused)) {
        return;
    }
    let clear_top = document
        .query_selector(TOP_BAR_SELECTOR)
        .ok()
        .flatten()
        .map_or(0.0, |top| top.get_bounding_client_rect().bottom().max(0.0));
    scroll_focus_into_band(&focused, clear_top, bar.get_bounding_client_rect().top());
}

/// Outside the browser there is no page to scroll.
#[cfg(not(target_arch = "wasm32"))]
fn keep_focus_above_save_bar() {}

/// The accents the last applied identity pack suggests.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PackAccents {
    /// The applied pack's name. The highlighted card can be a different
    /// pack, so the note names this one.
    pack: String,
    dark: String,
    light: String,
}

/// Brand accents from an identity pack, waiting for Save. The accent itself
/// is edited on the brand settings page, so without a pending pair this
/// page's save leaves it alone.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PackBrand {
    /// The "Also set brand accents" box.
    ticked: bool,
    /// What the last Apply pack offered. Kept while the box is unticked, so
    /// ticking it again brings the same pair back without a second Apply
    /// pack (which would also reset copy, shell and skin).
    offer: Option<PackAccents>,
    /// Why the last Apply pack's pair was not offered: it fails the same
    /// contrast lock as brand settings, so Save never writes it.
    refused: Option<String>,
}

impl PackBrand {
    fn new() -> Self {
        Self {
            ticked: true,
            offer: None,
            refused: None,
        }
    }

    /// Apply pack was pressed for `preset`. A pack with no suggested accent
    /// offers nothing and drops an older pack's offer.
    fn apply(&mut self, preset: &scuffed_types::HomepagePreset) {
        let brand = preset.suggested_brand;
        self.offer_pair(preset.name, brand.accent_dark, brand.accent_light);
    }

    /// The pair `pack` suggests goes through the brand settings contrast
    /// lock (`AccentCheck`) before Save may write it. A refused pair is
    /// named in the note instead, and the stored accent stays.
    fn offer_pair(&mut self, pack: &str, dark: &str, light: &str) {
        self.offer = None;
        self.refused = None;
        let (dark, light) = (dark.trim(), light.trim());
        if dark.is_empty() {
            return;
        }
        let check = AccentCheck::for_brand(&BrandConfig::from_settings(dark, light));
        match check.fail_reason() {
            None => {
                self.offer = Some(PackAccents {
                    pack: pack.to_string(),
                    dark: dark.to_string(),
                    light: light.to_string(),
                });
            }
            Some(reason) => {
                self.refused = Some(format!(
                    "Save keeps the current accent. The {pack} pack's accents fail the brand contrast check: {reason}"
                ));
            }
        }
    }

    fn set_ticked(&mut self, on: bool) {
        self.ticked = on;
    }

    /// The server answered (a load or a successful save): nothing is waiting.
    fn clear(&mut self) {
        self.offer = None;
        self.refused = None;
    }

    /// The pair Save will write, if any.
    fn pending(&self) -> Option<&PackAccents> {
        self.offer.as_ref().filter(|_| self.ticked)
    }

    /// The dark and light accent keys of the save body. `None` omits the
    /// key, so the server keeps the stored accent.
    fn save_accents(&self) -> (Option<String>, Option<String>) {
        match self.pending() {
            Some(accents) => (Some(accents.dark.clone()), Some(accents.light.clone())),
            None => (None, None),
        }
    }

    /// The pending pair in words, or nothing. The accent fields that used
    /// to show it live on the brand settings page now, so this line is the
    /// only place the pending pair is visible.
    fn note(&self) -> String {
        match self.pending() {
            Some(PackAccents { pack, dark, light }) => format!(
                "Save will also set the brand accents from the {pack} pack: {dark} in dark, {light} in light. Untick \"Also set brand accents\" to keep the current accent."
            ),
            None if self.ticked => self.refused.clone().unwrap_or_default(),
            None => String::new(),
        }
    }
}

/// The live line under Apply pack. Always rendered; only the text changes.
/// Its spacing is in the stylesheet, where the empty line gives it back.
#[component]
fn PackBrandStatus(note: String) -> Element {
    rsx! {
        p { class: "settings-hint pack-brand-status", role: "status", "{note}" }
    }
}

/// The way out to brand settings, with the warning it is described by.
#[component]
fn BrandSettingsLink() -> Element {
    rsx! {
        Link {
            class: "btn is-ghost",
            to: Route::AdminBrandSettings {},
            "aria-describedby": "admin-settings-brand-link-warning",
            "Open brand settings"
        }
        p {
            class: "settings-hint",
            id: "admin-settings-brand-link-warning",
            "{BRAND_LINK_WARNING}"
        }
    }
}

/// Next to the link out of this unsaved form.
const BRAND_LINK_WARNING: &str =
    "Save first. Opening brand settings leaves this page and drops unsaved edits here.";

/// Checkbox values last accepted by the server. A failed save shows these
/// again, not the draft the request tried to write.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SettingsToggles {
    recruitment_open: bool,
    strategies_enabled: bool,
    officers_can_edit_teams: bool,
    sections: scuffed_types::HomepageSections,
}

fn toggles_from_settings(s: &SiteSettings) -> SettingsToggles {
    SettingsToggles {
        recruitment_open: s.recruitment_open,
        strategies_enabled: s.strategies_enabled,
        officers_can_edit_teams: s.officers_can_edit_teams,
        sections: s.homepage.sections.clone(),
    }
}

fn toggles_shown_after_save(
    draft: &SettingsToggles,
    committed: &SettingsToggles,
    saved: bool,
) -> SettingsToggles {
    if saved {
        draft.clone()
    } else {
        committed.clone()
    }
}

/// The Strategies switch. The nav shows the Strategy group on its own; there
/// is no placement to set any more.
fn strategies_hint() -> &'static str {
    "On (default): the Strategy group is in the site nav. Off: it is hidden from the nav and /strategy pages are blocked. Existing plans are not deleted. Public Patch Notes (/patch-notes) are unchanged."
}

#[component]
pub fn AdminSettings() -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let mut saving = use_signal(|| false);
    let mut loaded = use_signal(|| false);
    let mut load_error = use_signal(|| Option::<String>::None);
    let mut reload_tick = use_signal(|| 0u32);
    // The save line under the form: always rendered, only its text changes.
    let mut save_state = use_signal(crate::components::SaveState::default);
    // The reserved strip under the form follows the bar; it goes with the page.
    use_drop(|| sync_save_bar_height(None));
    // APP3: the shared settings context feeds nav, brand accent, org name and
    // the strategies gate. `try_` so a mount without `App` still renders.
    let site_settings = try_use_context::<SiteSettingsState>();

    let mut org_name = use_signal(String::new);
    let mut site_description = use_signal(String::new);
    let mut recruitment_open = use_signal(|| false);
    let mut recruitment_message = use_signal(String::new);
    let mut min_age = use_signal(|| "16".to_string());
    let mut forum_backend = use_signal(|| "local".to_string());
    let mut extra_relay_urls = use_signal(String::new);
    let mut public_layout = use_signal(|| PublicLayout::Hub);
    let mut home_shell = use_signal(|| HomeShell::OpsHub);
    let mut home_skin = use_signal(|| HomeSkin::Clean);
    let mut content_align = use_signal(|| ContentAlign::Left);
    let mut homepage = use_signal(HomepageContent::default);
    let mut page_bg_color = use_signal(String::new);
    let mut page_bg_image_url = use_signal(String::new);
    // Brand accents an identity pack suggested, and the box that decides
    // whether Save writes them.
    let mut pack_brand = use_signal(PackBrand::new);
    let mut strategies_enabled = use_signal(|| true);
    let mut officers_can_edit_teams = use_signal(|| false);
    let mut committed_toggles = use_signal(|| None::<SettingsToggles>);
    // Selected identity pack id for “Apply pack”.
    let mut homepage_preset_id = use_signal(|| "neutral".to_string());
    // Which homepage-copy accordion panels are expanded (ids: hero, ethos, …).
    let mut open_copy = use_signal(|| {
        let mut s = std::collections::HashSet::new();
        s.insert("hero".to_string());
        s
    });

    let _settings = use_resource(move || {
        let _tick = reload_tick();
        async move {
            load_error.set(None);
            match ApiClient::web()
                .fetch::<SiteSettings>("/api/settings")
                .await
            {
                Ok(s) => {
                    let committed = toggles_from_settings(&s);
                    org_name.set(s.org_name);
                    site_description.set(s.site_description);
                    recruitment_open.set(s.recruitment_open);
                    recruitment_message.set(s.recruitment_message);
                    min_age.set(s.min_age.to_string());
                    forum_backend.set(s.forum_backend);
                    extra_relay_urls.set(s.extra_relay_urls);
                    public_layout.set(s.public_layout);
                    home_shell.set(s.home_shell);
                    home_skin.set(s.home_skin);
                    content_align.set(s.homepage.content_align);
                    homepage.set(s.homepage);
                    page_bg_color.set(s.page_bg_color);
                    page_bg_image_url.set(s.page_bg_image_url);
                    pack_brand.write().clear();
                    strategies_enabled.set(s.strategies_enabled);
                    officers_can_edit_teams.set(s.officers_can_edit_teams);
                    committed_toggles.set(Some(committed));
                    loaded.set(true);
                    Some(true)
                }
                Err(e) => {
                    load_error.set(Some(e.to_string()));
                    loaded.set(false);
                    None
                }
            }
        }
    });

    let mut on_save = move |_: MouseEvent| {
        let age: u32 = min_age().trim().parse().unwrap_or(16);
        // Merge alignment from its own signal so nested write() can't get lost.
        let mut hp = homepage();
        hp.content_align = content_align();
        // Dual-write layout from shell (no separate layout control).
        let shell = home_shell();
        let (brand_accent_dark, brand_accent_light) = pack_brand.read().save_accents();
        let body = UpdateSettingsRequest {
            org_name: Some(org_name().trim().to_string()),
            site_description: Some(site_description().trim().to_string()),
            recruitment_open: Some(recruitment_open()),
            strategies_enabled: Some(strategies_enabled()),
            officers_can_edit_teams: admin_settings_officers_flag(officers_can_edit_teams()),
            recruitment_message: Some(recruitment_message().trim().to_string()),
            min_age: Some(age),
            forum_backend: Some(forum_backend()),
            extra_relay_urls: Some(extra_relay_urls().trim().to_string()),
            home_shell: Some(shell),
            home_skin: Some(home_skin()),
            public_layout: Some(shell.to_public_layout()),
            homepage: Some(hp.clone()),
            // This page no longer edits nav placement. Leaving the key out
            // keeps the stored nav config exactly as it is.
            nav: None,
            page_bg_color: Some(page_bg_color().trim().to_string()),
            page_bg_image_url: Some(page_bg_image_url().trim().to_string()),
            brand_accent_dark,
            brand_accent_light,
            // The display font is set on Brand settings: None keeps the stored value.
            brand_font_display: None,
        };

        saving.set(true);
        save_state.set(crate::components::SaveState::Saving);
        spawn(async move {
            match ApiClient::web()
                .put_json::<_, SiteSettings>("/api/settings", &body)
                .await
            {
                Ok(s) => {
                    let committed = toggles_from_settings(&s);
                    if let Some(site_settings) = site_settings {
                        site_settings.apply_saved(s.clone());
                    }
                    // Keep UI in sync with what the server actually stored.
                    public_layout.set(s.public_layout);
                    home_shell.set(s.home_shell);
                    home_skin.set(s.home_skin);
                    content_align.set(s.homepage.content_align);
                    homepage.set(s.homepage);
                    page_bg_color.set(s.page_bg_color);
                    page_bg_image_url.set(s.page_bg_image_url);
                    pack_brand.write().clear();
                    strategies_enabled.set(s.strategies_enabled);
                    officers_can_edit_teams.set(s.officers_can_edit_teams);
                    recruitment_open.set(s.recruitment_open);
                    committed_toggles.set(Some(committed));
                    toast.show(Toast::success("Settings saved."));
                    save_state.set(crate::components::SaveState::Saved(
                        "Settings saved.".to_string(),
                    ));
                }
                Err(e) => {
                    if let Some(committed) = committed_toggles() {
                        let draft = SettingsToggles {
                            recruitment_open: recruitment_open(),
                            strategies_enabled: strategies_enabled(),
                            officers_can_edit_teams: officers_can_edit_teams(),
                            sections: homepage().sections.clone(),
                        };
                        let shown = toggles_shown_after_save(&draft, &committed, false);
                        recruitment_open.set(shown.recruitment_open);
                        strategies_enabled.set(shown.strategies_enabled);
                        officers_can_edit_teams.set(shown.officers_can_edit_teams);
                        homepage.with_mut(|hp| hp.sections = shown.sections);
                    }
                    toast.show(Toast::error(format!("Failed to save settings: {e}")));
                    // The reason stays on the page after the toast is gone.
                    // A network failure reads as one short sentence there;
                    // the browser's raw error (a whole stack in Chromium) is
                    // for the console only (QA 706 FIX5).
                    crate::util::log_client_error("settings save", &e);
                    save_state.set(crate::components::SaveState::failed(
                        "Could not save settings",
                        crate::client_error::user_message(&e),
                    ));
                }
            }
            saving.set(false);
        });
    };

    if !auth().is_admin() {
        return rsx! {
            AccessDenied { message: "You need admin permissions to view settings.".to_string() }
        };
    }

    let can_save = loaded() && !saving();
    let load_failed = load_error().map(|err| crate::components::load_error_text("settings", &err));

    // The blocks below keep their old indent on purpose: two open pull
    // requests edit them, and a re-indent would conflict with every hunk.
    rsx! {
        crate::components::ui::PageShell {
            width: crate::components::ui::PageWidth::Narrow,
            class: "admin-settings-page",
        crate::components::ui::PageHeader {
            title: "Settings",
            lead: "Organization, homepage, appearance, recruitment, features and forum. Nothing changes on the site until you save.",
            actions: rsx! {
                a {
                    class: "btn is-ghost",
                    href: "/",
                    target: "_blank",
                    rel: "noopener",
                    "View homepage"
                    span { class: "visually-hidden", " (opens in a new tab)" }
                }
            },
        }

        // The form and its Save row, one field step apart. A control that
        // takes focus is kept clear of the stuck Save bar (WCAG 2.4.11).
        div { class: "form-stack",
            onfocusin: move |_| keep_focus_above_save_bar(),
        if let Some(message) = load_failed {
            div { class: "fetch-error-wrap", role: "alert",
                p { class: "fetch-error", "{message}" }
                button {
                    class: "fetch-error__retry",
                    r#type: "button",
                    onclick: move |_| {
                        load_error.set(None);
                        reload_tick.set(reload_tick() + 1);
                    },
                    "Retry"
                }
            }
        } else if !loaded() {
            div { class: "admin-pending",
                p { class: "admin-pending-text", role: "status", "Loading settings..." }
                crate::components::ui::SkeletonBars { rows: 4 }
            }
        } else {
            div { class: "settings-page",
            div { class: "form-section",
                h2 { "Organization" }
                p { class: "form-section-lead",
                    "Public name and blurb. Name feeds nav mark and hero watermark initials."
                }
                div { class: "form-section-card",
                    div { class: "form-grid",
                        div { class: "form-field span-full",
                            label { class: "field-label", r#for: "settings-org-name", "Organization name" }
                            input {
                                class: "field-input",
                                id: "settings-org-name",
                                value: "{org_name}",
                                oninput: move |e| org_name.set(e.value()),
                            }
                        }
                        div { class: "form-field span-full",
                            label { class: "field-label", r#for: "settings-site-description", "Site description" }
                            textarea {
                                class: "field-input",
                                id: "settings-site-description",
                                value: "{site_description}",
                                oninput: move |e| site_description.set(e.value()),
                            }
                        }
                    }
                }
            }

            // Identity pack
            div { class: "form-section",
                h2 { "Homepage identity" }
                p { class: "form-section-lead",
                    "Start from a pack, then tune shell, skin, and sections. Apply pack fills copy. Save to persist."
                }
                div { class: "form-section-card",
                    h3 { class: "settings-subhead", id: "settings-pack-label", "Identity pack" }
                    div { class: "pack-grid", role: "group", aria_labelledby: "settings-pack-label",
                        for p in homepage_presets() {
                            Fragment { key: "{p.id}", {
                                let id = p.id.to_string();
                                let selected = homepage_preset_id() == p.id;
                                let card_class = if selected {
                                    "pack-card is-selected"
                                } else {
                                    "pack-card"
                                };
                                let shell = p.suggested_shell.as_str();
                                let skin = p.suggested_skin.as_str();
                                rsx! {
                                    button {
                                        class: "{card_class}",
                                        r#type: "button",
                                        aria_pressed: if selected { "true" } else { "false" },
                                        onclick: move |_| homepage_preset_id.set(id.clone()),
                                        span { class: "pack-card-name", "{p.name}" }
                                        span { class: "pack-card-desc", "{p.description}" }
                                        span { class: "pack-card-meta", "{shell} · {skin}" }
                                    }
                                }
                            } }
                        }
                    }
                    div { class: "pack-actions",
                        label { class: "section-chip",
                            input {
                                r#type: "checkbox",
                                checked: pack_brand.read().ticked,
                                // Unticking takes back a pair an earlier
                                // Apply pack left waiting for Save, and
                                // ticking again restores it.
                                onchange: move |e| pack_brand.write().set_ticked(e.checked()),
                            }
                            "Also set brand accents"
                        }
                        button {
                            class: "btn-add",
                            r#type: "button",
                            onclick: move |_| {
                                let id = homepage_preset_id();
                                if let Some(preset) = homepage_preset_by_id(&id) {
                                    pack_brand.write().apply(&preset);
                                    content_align.set(preset.content.content_align);
                                    homepage.set(preset.content);
                                    home_shell.set(preset.suggested_shell);
                                    home_skin.set(preset.suggested_skin);
                                    public_layout.set(preset.suggested_shell.to_public_layout());
                                    open_copy.set({
                                        let mut s = std::collections::HashSet::new();
                                        s.insert("hero".to_string());
                                        s
                                    });
                                    toast.show(Toast::success(format!(
                                        "Applied “{}”. Click Save to persist.",
                                        preset.name,
                                    )));
                                } else {
                                    toast.show(Toast::error("Unknown pack."));
                                }
                            },
                            "Apply pack"
                        }
                    }
                    PackBrandStatus { note: pack_brand.read().note() }
                }

                div { class: "form-section-card",
                    h3 { class: "settings-subhead", id: "settings-shell-label", "Shell" }
                    div { class: "option-tiles", role: "group", aria_labelledby: "settings-shell-label",
                        {
                            let shells = [
                                (HomeShell::OpsHub, "Ops hub", "Dense board. Schedule, roster table."),
                                (HomeShell::RecruitLanding, "Recruit landing", "Apply-first, with team cards."),
                                (HomeShell::Minimal, "Minimal", "Hero + apply; lean front door."),
                                (HomeShell::Manifesto, "Manifesto", "Principles first; comps quieter."),
                            ];
                            rsx! {
                                for (shell, title, blurb) in shells {
                                    Fragment { key: "{title}", {
                                        let selected = home_shell() == shell;
                                        let cls = if selected { "option-tile is-selected" } else { "option-tile" };
                                        rsx! {
                                            button {
                                                class: "{cls}",
                                                r#type: "button",
                                                aria_pressed: if selected { "true" } else { "false" },
                                                onclick: move |_| {
                                                    home_shell.set(shell);
                                                    public_layout.set(shell.to_public_layout());
                                                },
                                                div { class: "option-tile-title", "{title}" }
                                                div { class: "option-tile-blurb", "{blurb}" }
                                            }
                                        }
                                    } }
                                }
                            }
                        }
                    }
                    h3 { class: "settings-subhead", id: "settings-skin-label", "Skin" }
                    div { class: "option-tiles", role: "group", aria_labelledby: "settings-skin-label",
                        {
                            let skins = [
                                (HomeSkin::Clean, "Clean", "Sentence-case headings. Default for most packs."),
                                (HomeSkin::Esports, "Esports", "Upper-case headings, wider label tracking."),
                            ];
                            rsx! {
                                for (skin, title, blurb) in skins {
                                    Fragment { key: "{title}", {
                                        let selected = home_skin() == skin;
                                        let cls = if selected { "option-tile is-selected" } else { "option-tile" };
                                        rsx! {
                                            button {
                                                class: "{cls}",
                                                r#type: "button",
                                                aria_pressed: if selected { "true" } else { "false" },
                                                onclick: move |_| home_skin.set(skin),
                                                div { class: "option-tile-title", "{title}" }
                                                div { class: "option-tile-blurb", "{blurb}" }
                                            }
                                        }
                                    } }
                                }
                            }
                        }
                    }
                    div { class: "form-field",
                        label { class: "field-label", r#for: "settings-hero-align", "Hero text alignment" }
                        select {
                            class: "field-input",
                            id: "settings-hero-align",
                            aria_describedby: "settings-hero-align-hint",
                            value: "{content_align().as_str()}",
                            onchange: move |e| {
                                content_align.set(ContentAlign::from_str_lossy(&e.value()));
                            },
                            option { value: "left", "Left" }
                            option { value: "center", "Center" }
                        }
                        p { class: "settings-hint", id: "settings-hero-align-hint",
                            "Hero only. Body blocks stay left."
                        }
                    }
                }

                div { class: "form-section-card",
                    fieldset { class: "admin-check-group", aria_describedby: "settings-sections-hint",
                        // A sub-group like Identity pack, Shell and Skin: same level.
                        legend { class: "settings-subhead", "Visible sections" }
                        p { class: "settings-hint", id: "settings-sections-hint",
                            "Hero is always on. Empty data may still hide some blocks depending on shell."
                        }
                        {
                            let sections = [
                                ("ethos", "About / ethos", homepage().sections.ethos),
                                ("schedule", "Schedule", homepage().sections.schedule),
                                ("tournaments", "Tournaments", homepage().sections.tournaments),
                                ("teams", "Teams", homepage().sections.teams),
                                ("news", "Announcements", homepage().sections.news),
                                ("recruit", "Recruiting", homepage().sections.recruit),
                            ];
                            rsx! {
                                for (key, label, checked) in sections {
                                    label { class: "admin-check-row",
                                        input {
                                            r#type: "checkbox",
                                            id: "settings-section-{key}",
                                            checked: checked,
                                            onchange: move |e| {
                                                let on = e.checked();
                                                let mut hp = homepage.write();
                                                match key {
                                                    "ethos" => hp.sections.ethos = on,
                                                    "schedule" => hp.sections.schedule = on,
                                                    "tournaments" => hp.sections.tournaments = on,
                                                    "teams" => hp.sections.teams = on,
                                                    "news" => hp.sections.news = on,
                                                    "recruit" => hp.sections.recruit = on,
                                                    _ => {}
                                                }
                                            },
                                        }
                                        "{label}"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "form-section",
                h2 { "Appearance" }
                p { class: "form-section-lead",
                    "Page background. The brand accent and the display font have their own page."
                }
                div { class: "form-section-card",
                    h3 { class: "settings-subhead", "Brand" }
                    p { class: "settings-hint",
                        "One accent for both themes, with a live preview and a contrast check."
                    }
                    BrandSettingsLink {}
                    div { class: "settings-divider" }
                    h3 { class: "settings-subhead", "Page background" }
                    div { class: "color-row",
                        div { class: "color-field",
                            label { class: "field-label", r#for: "settings-bg-color", "Background color" }
                            div { class: "swatch-row",
                                input {
                                    r#type: "color",
                                    aria_label: "Background color picker",
                                    value: {
                                        let c = page_bg_color();
                                        if c.len() == 7 && c.starts_with('#') {
                                            c
                                        } else {
                                            BG_DARK.into()
                                        }
                                    },
                                    oninput: move |e| page_bg_color.set(e.value()),
                                }
                                input {
                                    class: "field-input",
                                    id: "settings-bg-color",
                                    placeholder: "Theme default",
                                    aria_describedby: "settings-bg-color-hint",
                                    value: "{page_bg_color}",
                                    oninput: move |e| page_bg_color.set(e.value()),
                                }
                            }
                            p { class: "settings-hint", id: "settings-bg-color-hint",
                                "Leave empty to use the theme's own background."
                            }
                        }
                        button {
                            class: "btn is-ghost",
                            r#type: "button",
                            onclick: move |_| page_bg_color.set(String::new()),
                            "Clear color"
                        }
                    }
                    div { class: "form-field",
                        label { class: "field-label", r#for: "settings-bg-image", "Background image URL" }
                        p { class: "settings-hint", id: "settings-bg-image-hint",
                            "A full https address, or a path such as /uploads/bg.jpg"
                        }
                        div { class: "settings-inline-row",
                            input {
                                class: "field-input",
                                id: "settings-bg-image",
                                aria_describedby: "settings-bg-image-hint",
                                value: "{page_bg_image_url}",
                                oninput: move |e| page_bg_image_url.set(e.value()),
                            }
                            button {
                                class: "btn is-ghost",
                                r#type: "button",
                                aria_label: "Clear background image",
                                onclick: move |_| page_bg_image_url.set(String::new()),
                                "Clear"
                            }
                        }
                    }
                }
            }

            div { class: "form-section",
                h2 { "Recruitment" }
                p { class: "form-section-lead", "Public apply pipeline and age gate." }
                div { class: "form-section-card",
                    label { class: "admin-check-row",
                        input {
                            r#type: "checkbox",
                            id: "settings-recruitment-open",
                            checked: recruitment_open(),
                            onchange: move |e| recruitment_open.set(e.checked()),
                        }
                        "Recruitment is open"
                    }
                    div { class: "form-grid",
                        div { class: "form-field span-full",
                            label { class: "field-label", r#for: "settings-recruitment-message", "Message when closed" }
                            textarea {
                                class: "field-input",
                                id: "settings-recruitment-message",
                                value: "{recruitment_message}",
                                oninput: move |e| recruitment_message.set(e.value()),
                            }
                        }
                        div { class: "form-field",
                            label { class: "field-label", r#for: "settings-min-age", "Minimum age" }
                            input {
                                class: "field-input",
                                id: "settings-min-age",
                                r#type: "number",
                                min: "0",
                                value: "{min_age}",
                                oninput: move |e| min_age.set(e.value()),
                            }
                        }
                    }
                }
            }

            div { class: "form-section",
                h2 { "Features" }
                p { class: "form-section-lead",
                    "Turn product areas on or off for this clan. Off hides them from the public site. Patch Notes stay available."
                }
                div { class: "form-section-card",
                    label { class: "admin-check-row",
                        input {
                            r#type: "checkbox",
                            id: "settings-strategies-enabled",
                            aria_describedby: "settings-strategies-hint",
                            checked: strategies_enabled(),
                            onchange: move |e| strategies_enabled.set(e.checked()),
                        }
                        "Strategies"
                    }
                    p { class: "settings-hint", id: "settings-strategies-hint",
                        "{strategies_hint()}"
                    }
                    label { class: "admin-check-row",
                        input {
                            r#type: "checkbox",
                            id: "settings-officers-edit-teams",
                            aria_describedby: "settings-officers-edit-teams-hint",
                            checked: officers_can_edit_teams(),
                            onchange: move |e| officers_can_edit_teams.set(e.checked()),
                        }
                        "Officers can edit teams"
                    }
                    p { class: "settings-hint", id: "settings-officers-edit-teams-hint",
                        "{officers_can_edit_teams_hint()}"
                    }
                }
            }

            div { class: "form-section",
                h2 { "Homepage text" }
                p { class: "form-section-lead",
                    "Fine-tune copy after a pack. Expand a section to edit. List fields: one item per line."
                }
                div { class: "copy-stack",
                    CopyPanel {
                        id: "hero",
                        title: "Hero",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { id: "settings-hp-hero-badge", label: "Badge", value: homepage().hero_badge.clone(), on_change: move |v| homepage.write().hero_badge = v }
                            StrField { id: "settings-hp-cta-primary", label: "Primary CTA", value: homepage().cta_primary.clone(), on_change: move |v| homepage.write().cta_primary = v }
                            StrField { id: "settings-hp-hero-title", label: "Title", value: homepage().hero_title.clone(), on_change: move |v| homepage.write().hero_title = v }
                            StrField { id: "settings-hp-hero-title-accent", label: "Title accent", value: homepage().hero_title_accent.clone(), on_change: move |v| homepage.write().hero_title_accent = v }
                            div { class: "span-full",
                                AreaField { id: "settings-hp-hero-sub", label: "Subtitle", value: homepage().hero_sub.clone(), on_change: move |v| homepage.write().hero_sub = v }
                            }
                            div { class: "span-full",
                                StrField { id: "settings-hp-cta-secondary", label: "Secondary CTA", value: homepage().cta_secondary.clone(), on_change: move |v| homepage.write().cta_secondary = v }
                            }
                        }
                    }
                    CopyPanel {
                        id: "ethos",
                        title: "Ethos",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { id: "settings-hp-ethos-kicker", label: "Kicker", value: homepage().ethos_kicker.clone(), on_change: move |v| homepage.write().ethos_kicker = v }
                            StrField { id: "settings-hp-ethos-title", label: "Title", value: homepage().ethos_title.clone(), on_change: move |v| homepage.write().ethos_title = v }
                            div { class: "span-full",
                                AreaField { id: "settings-hp-ethos-body", label: "Body", value: homepage().ethos_body.clone(), on_change: move |v| homepage.write().ethos_body = v }
                            }
                            div { class: "span-full",
                                LinesField { id: "settings-hp-ethos-rules", label: "Rules", value: homepage().ethos_rules.clone(), on_change: move |v| homepage.write().ethos_rules = v }
                            }
                        }
                    }
                    CopyPanel {
                        id: "squads",
                        title: "Squads",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { id: "settings-hp-teams-kicker", label: "Kicker", value: homepage().teams_kicker.clone(), on_change: move |v| homepage.write().teams_kicker = v }
                            StrField { id: "settings-hp-teams-title", label: "Title", value: homepage().teams_title.clone(), on_change: move |v| homepage.write().teams_title = v }
                            div { class: "span-full",
                                StrField { id: "settings-hp-teams-empty", label: "Empty", value: homepage().teams_empty.clone(), on_change: move |v| homepage.write().teams_empty = v }
                            }
                        }
                    }
                    CopyPanel {
                        id: "news",
                        title: "Announcements",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { id: "settings-hp-news-kicker", label: "Kicker", value: homepage().news_kicker.clone(), on_change: move |v| homepage.write().news_kicker = v }
                            StrField { id: "settings-hp-news-title", label: "Title", value: homepage().news_title.clone(), on_change: move |v| homepage.write().news_title = v }
                            StrField { id: "settings-hp-news-empty", label: "Empty", value: homepage().news_empty.clone(), on_change: move |v| homepage.write().news_empty = v }
                            StrField { id: "settings-hp-news-view-all", label: "View all", value: homepage().news_view_all.clone(), on_change: move |v| homepage.write().news_view_all = v }
                        }
                    }
                    CopyPanel {
                        id: "tournaments",
                        title: "Tournaments",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { id: "settings-hp-tournaments-kicker", label: "Kicker", value: homepage().tournaments_kicker.clone(), on_change: move |v| homepage.write().tournaments_kicker = v }
                            StrField { id: "settings-hp-tournaments-title", label: "Title", value: homepage().tournaments_title.clone(), on_change: move |v| homepage.write().tournaments_title = v }
                            StrField { id: "settings-hp-tournaments-empty", label: "Empty", value: homepage().tournaments_empty.clone(), on_change: move |v| homepage.write().tournaments_empty = v }
                            StrField { id: "settings-hp-tournaments-view-all", label: "View all", value: homepage().tournaments_view_all.clone(), on_change: move |v| homepage.write().tournaments_view_all = v }
                        }
                    }
                    CopyPanel {
                        id: "schedule",
                        title: "Schedule",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { id: "settings-hp-schedule-kicker", label: "Kicker", value: homepage().schedule_kicker.clone(), on_change: move |v| homepage.write().schedule_kicker = v }
                            StrField { id: "settings-hp-schedule-title", label: "Title", value: homepage().schedule_title.clone(), on_change: move |v| homepage.write().schedule_title = v }
                            StrField { id: "settings-hp-schedule-empty", label: "Empty", value: homepage().schedule_empty.clone(), on_change: move |v| homepage.write().schedule_empty = v }
                            StrField { id: "settings-hp-calendar-cta", label: "Calendar CTA", value: homepage().calendar_cta.clone(), on_change: move |v| homepage.write().calendar_cta = v }
                        }
                    }
                    CopyPanel {
                        id: "recruit",
                        title: "Recruiting",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { id: "settings-hp-recruit-kicker", label: "Kicker", value: homepage().recruit_kicker.clone(), on_change: move |v| homepage.write().recruit_kicker = v }
                            StrField { id: "settings-hp-recruit-title", label: "Title", value: homepage().recruit_title.clone(), on_change: move |v| homepage.write().recruit_title = v }
                            div { class: "span-full",
                                AreaField { id: "settings-hp-recruit-body", label: "Body", value: homepage().recruit_body.clone(), on_change: move |v| homepage.write().recruit_body = v }
                            }
                            StrField { id: "settings-hp-recruit-cta", label: "CTA", value: homepage().recruit_cta.clone(), on_change: move |v| homepage.write().recruit_cta = v }
                            StrField { id: "settings-hp-recruit-expectations-title", label: "Expectations title", value: homepage().recruit_expectations_title.clone(), on_change: move |v| homepage.write().recruit_expectations_title = v }
                            div { class: "span-full",
                                LinesField { id: "settings-hp-recruit-expectations", label: "Expectations", value: homepage().recruit_expectations.clone(), on_change: move |v| homepage.write().recruit_expectations = v }
                            }
                            StrField { id: "settings-hp-never-ask-title", label: "Never-ask title", value: homepage().never_ask_title.clone(), on_change: move |v| homepage.write().never_ask_title = v }
                            StrField { id: "settings-hp-seeking-label", label: "Seeking label", value: homepage().seeking_label.clone(), on_change: move |v| homepage.write().seeking_label = v }
                            div { class: "span-full",
                                AreaField { id: "settings-hp-never-ask-body", label: "Never-ask body", value: homepage().never_ask_body.clone(), on_change: move |v| homepage.write().never_ask_body = v }
                            }
                            div { class: "span-full",
                                LinesField { id: "settings-hp-seeking-tags", label: "Seeking tags", value: homepage().seeking_tags.clone(), on_change: move |v| homepage.write().seeking_tags = v }
                            }
                        }
                    }
                    CopyPanel {
                        id: "footer",
                        title: "Footer",
                        open_copy: open_copy,
                        StrField { id: "settings-hp-footer-note", label: "Footer note", value: homepage().footer_note.clone(), on_change: move |v| homepage.write().footer_note = v }
                    }
                }
            }

            div { class: "form-section",
                h2 { "Forum" }
                p { class: "form-section-lead", "Where forum threads live and optional extra Nostr relays." }
                div { class: "form-section-card",
                    div { class: "form-field",
                        label { class: "field-label", r#for: "settings-forum-backend", "Forum backend" }
                        select {
                            class: "field-input",
                            id: "settings-forum-backend",
                            aria_describedby: "settings-forum-backend-hint",
                            value: "{forum_backend}",
                            onchange: move |e| forum_backend.set(e.value()),
                            option { value: "local", "Local (SurrealDB)" }
                            option { value: "nostr", "Nostr (Relay)" }
                        }
                        p { class: "settings-hint", id: "settings-forum-backend-hint",
                            "Local uses the database. Nostr uses the relay (requires relay setup)."
                        }
                    }
                    div { class: "form-field",
                        label { class: "field-label", r#for: "settings-extra-relays", "Extra relay URLs" }
                        textarea {
                            class: "field-input",
                            id: "settings-extra-relays",
                            aria_describedby: "settings-extra-relays-hint",
                            value: "{extra_relay_urls}",
                            oninput: move |e| extra_relay_urls.set(e.value()),
                        }
                        p { class: "settings-hint", id: "settings-extra-relays-hint",
                            "One per line, for example wss://relay.example.com. Published to primary relay (NOSTR_RELAY_URL) plus these."
                        }
                    }
                }
            }
            }
        }

        // Save sits under the form, with its live line. Both are rendered in
        // every state; while the settings load, Save does nothing. The bar
        // sticks to the bottom of the view (above the phone tab bar), so Save
        // and a failure are in sight anywhere on this long page.
        div { class: "admin-form-actions settings-save-bar",
            onresize: move |evt: Event<ResizeData>| {
                sync_save_bar_height(evt.data().get_border_box_size().ok().map(|size| size.height));
            },
            button {
                class: "btn is-primary",
                r#type: "button",
                id: "settings-save",
                aria_disabled: if !can_save { "true" },
                aria_describedby: "settings-save-status",
                onclick: move |evt| {
                    if can_save {
                        on_save(evt);
                    }
                },
                if saving() { "Saving..." } else { "Save settings" }
            }
            crate::components::FormStatus { state: save_state(), id: "settings-save-status" }
        }
        }
        }
    }
}

#[component]
fn CopyPanel(
    id: &'static str,
    title: &'static str,
    mut open_copy: Signal<std::collections::HashSet<String>>,
    children: Element,
) -> Element {
    let is_open = open_copy().contains(id);
    let panel_class = if is_open {
        "copy-panel is-open"
    } else {
        "copy-panel"
    };
    let body_id = format!("settings-copy-{id}");
    rsx! {
        div { class: "{panel_class}",
            h3 { class: "copy-panel-heading",
                button {
                    class: "copy-panel-head",
                    r#type: "button",
                    id: "settings-copy-{id}-toggle",
                    aria_expanded: if is_open { "true" } else { "false" },
                    // The body is in the tree only while the panel is open.
                    aria_controls: is_open.then(|| body_id.clone()),
                    onclick: move |_| {
                        let mut set = open_copy();
                        if set.contains(id) {
                            set.remove(id);
                        } else {
                            set.insert(id.to_string());
                        }
                        open_copy.set(set);
                    },
                    span { class: "copy-panel-title", "{title}" }
                    span { class: "copy-panel-chevron", aria_hidden: "true" }
                }
            }
            if is_open {
                div { class: "copy-panel-body", id: "{body_id}", {children} }
            }
        }
    }
}

/// One line of homepage copy. `id` is page-unique (the content field's name).
#[component]
fn StrField(
    id: &'static str,
    label: &'static str,
    value: String,
    on_change: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: id, "{label}" }
            input {
                class: "field-input",
                id,
                value: "{value}",
                oninput: move |e| on_change.call(e.value()),
            }
        }
    }
}

#[component]
fn AreaField(
    id: &'static str,
    label: &'static str,
    value: String,
    on_change: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: id, "{label}" }
            textarea {
                class: "field-input",
                id,
                value: "{value}",
                oninput: move |e| on_change.call(e.value()),
            }
        }
    }
}

#[component]
fn LinesField(
    id: &'static str,
    label: &'static str,
    value: Vec<String>,
    on_change: EventHandler<Vec<String>>,
) -> Element {
    let joined = value.join("\n");
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: id, "{label} (one per line)" }
            textarea {
                class: "field-input",
                id,
                value: "{joined}",
                oninput: move |e| {
                    let lines = e
                        .value()
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>();
                    on_change.call(lines);
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::brand::{TEST_BLOCKED_ACCENT, TEST_EDGELESS_ACCENT};

    fn collect_rs_containing(
        dir: &std::path::Path,
        root: &std::path::Path,
        needle: &str,
        hits: &mut Vec<String>,
    ) {
        let entries =
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect_rs_containing(&path, root, needle, hits);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            if text.contains(needle) {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                hits.push(rel.display().to_string().replace('\\', "/"));
            }
        }
    }

    /// APP3: without this the public nav, accent, org name and the strategies
    /// gate keep the old values until a reload.
    #[test]
    fn appearance_subheads_are_headings_like_their_siblings() {
        // Vision nit 13 on PR 706: "Brand" and "Page background" were plain
        // divs while the sibling subheads are h3.
        let src = include_str!("settings.rs");
        let prod = src.split("mod tests").next().expect("production half");
        assert!(prod.contains("h3 { class: \"settings-subhead\", \"Brand\" }"));
        assert!(prod.contains("h3 { class: \"settings-subhead\", \"Page background\" }"));
        assert!(!prod.contains("div { class: \"settings-subhead\""));
    }

    /// QA 706 nit 1 (WCAG 2.4.11): Firefox leaves a focused control that
    /// straddles the top of the stuck Save bar behind it. The form wrapper
    /// that holds the bar runs the clear-the-bar check on every focus move,
    /// and the check looks up the bar by the class the bar really has.
    #[test]
    fn focus_moves_in_the_form_keep_the_control_above_the_save_bar() {
        let src = include_str!("settings.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        let stack = prod
            .find("div { class: \"form-stack\",")
            .expect("form wrapper");
        let wrapper = &prod[stack..];
        let handler = wrapper
            .find("onfocusin: move |_| keep_focus_above_save_bar(),")
            .expect("focus handler on the form wrapper");
        // The handler is the wrapper's first attribute, before any child.
        assert!(!wrapper[..handler].contains('}'), "{}", &wrapper[..handler]);
        // The bar is inside the wrapper the handler listens on.
        assert!(wrapper.contains("div { class: \"admin-form-actions settings-save-bar\","));
        assert!(prod.contains("const SAVE_BAR_SELECTOR: &str = \".settings-save-bar\";"));
        let check = prod
            .split("#[cfg(target_arch = \"wasm32\")]\nfn keep_focus_above_save_bar() {")
            .nth(1)
            .expect("check fn");
        let check = check.split("\n}\n").next().unwrap();
        assert!(check.contains(".query_selector(SAVE_BAR_SELECTOR)"));
        assert!(check.contains("bar.contains(Some(&focused))"));
        // Positioned against the bar's live box, not the reserved strip.
        assert!(check.contains(
            "scroll_focus_into_band(&focused, clear_top, bar.get_bounding_client_rect().top())"
        ));
    }

    /// Vision note on PR 706: a mouse press focuses a control on
    /// `mousedown`; a scroll then would move it out from under the pointer.
    /// The check returns before it measures anything unless the focus is
    /// keyboard focus (`:focus-visible`). browser-checks/focus.mjs clicks a
    /// checkbox that straddles the bar and checks the page did not move.
    #[test]
    fn only_keyboard_focus_is_moved_clear_of_the_save_bar() {
        let src = include_str!("settings.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        let check = prod
            .split("#[cfg(target_arch = \"wasm32\")]\nfn keep_focus_above_save_bar() {")
            .nth(1)
            .expect("check fn");
        let gate = check
            .find("if !focused.matches(\":focus-visible\").unwrap_or(false) {\n        return;\n    }")
            .expect(":focus-visible gate");
        let measure = check.find("get_bounding_client_rect").expect("measure");
        let scroll = check.find("scroll_focus_into_band(").expect("scroll");
        assert!(gate < measure && gate < scroll, "{check}");
    }

    /// QA 706 fix4: a failed save adds a line to the bar, so the strip the
    /// page reserves under the form follows the bar's measured height. The
    /// bar reports every size change (`onresize`) into the custom property
    /// that `scroll-padding-bottom` reads, and the page removes it on drop.
    #[test]
    fn the_reserved_strip_follows_the_save_bar_height() {
        let src = include_str!("settings.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert!(prod.contains("const SAVE_BAR_HEIGHT_VAR: &str = \"--settings-save-bar-height\";"));
        let bar = prod
            .split("div { class: \"admin-form-actions settings-save-bar\",")
            .nth(1)
            .expect("bar");
        assert!(bar.trim_start().starts_with(
            "onresize: move |evt: Event<ResizeData>| {\n                sync_save_bar_height(evt.data().get_border_box_size().ok().map(|size| size.height));"
        ), "{}", &bar[..200]);
        assert!(prod.contains("use_drop(|| sync_save_bar_height(None));"));
        let sync = prod
            .split(
                "#[cfg(target_arch = \"wasm32\")]\nfn sync_save_bar_height(height: Option<f64>) {",
            )
            .nth(1)
            .expect("sync fn");
        assert!(sync.contains(".style()"));
        assert!(sync.contains("style.set_property(SAVE_BAR_HEIGHT_VAR,"));
        assert!(sync.contains("style.remove_property(SAVE_BAR_HEIGHT_VAR)"));

        let css = crate::styles::admin::CSS;
        let rule = css
            .split("html:has(.settings-save-bar) {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("padding rule");
        assert!(
            rule.contains(
                "scroll-padding-bottom: calc(var(--tab-bar-height, 0%) + var(--settings-save-bar-height, "
            ),
            "{rule}"
        );
    }

    /// QA 706 FIX5: the Save bar's live line is clamped to three lines, so
    /// a long failure reason can never make the bar huge. The text is
    /// clipped, not removed, so a screen reader still gets all of it.
    #[test]
    fn the_save_bar_line_is_clamped_to_three_lines() {
        let css = crate::styles::admin::CSS;
        let rule = css
            .split(".settings-save-bar .admin-form-status:not(:empty) {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("clamp rule");
        for decl in [
            "display: -webkit-box;",
            "-webkit-box-orient: vertical;",
            "-webkit-line-clamp: 3;",
            "line-clamp: 3;",
            "overflow: hidden;",
            "overflow-wrap: anywhere;",
            "min-width: 0;",
        ] {
            assert!(rule.contains(decl), "{decl} missing in {rule}");
        }
        // Clipped, not hidden: no display: none, no aria-hidden on the line.
        assert!(!rule.contains("display: none"));
        let src = include_str!("settings.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert!(prod.contains(
            "crate::components::FormStatus { state: save_state(), id: \"settings-save-status\" }"
        ));
    }

    #[test]
    fn a_successful_save_updates_the_shared_settings() {
        let src = include_str!("settings.rs");
        let save = src
            .split(".put_json::<_, SiteSettings>(\"/api/settings\", &body)")
            .nth(1)
            .expect("settings PUT");
        let ok_arm = &save[..save.find("Err(e) =>").expect("error arm")];
        assert!(
            ok_arm.contains("site_settings.apply_saved(s.clone())"),
            "a successful save must write the saved settings into the context"
        );
    }

    fn preset(id: &str) -> scuffed_types::HomepagePreset {
        homepage_preset_by_id(id).unwrap_or_else(|| panic!("pack {id}"))
    }

    /// Two packs whose suggested accents differ, from the shared catalog.
    fn two_packs() -> (scuffed_types::HomepagePreset, scuffed_types::HomepagePreset) {
        let packs = homepage_presets();
        let first = packs
            .iter()
            .find(|p| !p.suggested_brand.accent_dark.is_empty())
            .expect("a pack with accents");
        let second = packs
            .iter()
            .find(|p| {
                !p.suggested_brand.accent_dark.is_empty()
                    && p.suggested_brand.accent_dark != first.suggested_brand.accent_dark
            })
            .expect("a second pack with other accents");
        (preset(first.id), preset(second.id))
    }

    fn body_accents(state: &PackBrand) -> serde_json::Value {
        let (brand_accent_dark, brand_accent_light) = state.save_accents();
        serde_json::to_value(UpdateSettingsRequest {
            brand_accent_dark,
            brand_accent_light,
            ..UpdateSettingsRequest::default()
        })
        .expect("serialize")
    }

    /// The accent fields moved to brand settings, so a pack's pending pair
    /// has to be shown here. Unticking the box takes it back, and ticking
    /// it again restores the same pair without a second Apply pack.
    #[test]
    fn a_pending_pack_accent_pair_is_shown_cleared_and_restored() {
        let (pack, _) = two_packs();
        let dark = pack.suggested_brand.accent_dark;
        let light = pack.suggested_brand.accent_light;

        // The page opens ticked with nothing waiting: no note, no keys.
        let mut state = PackBrand::new();
        assert!(state.ticked);
        assert_eq!(state.note(), "");
        assert_eq!(body_accents(&state), serde_json::json!({}));

        // Apply pack: the note names the pair, and Save sends exactly it.
        state.apply(&pack);
        let note = state.note();
        assert!(
            note.contains("Save will also set the brand accents"),
            "{note}"
        );
        assert!(note.contains(&format!("{dark} in dark")), "{note}");
        assert!(note.contains(&format!("{light} in light")), "{note}");
        assert_eq!(
            body_accents(&state),
            serde_json::json!({ "brand_accent_dark": dark, "brand_accent_light": light })
        );

        // Untick: nothing is pending, and the accent keys are omitted so
        // the server keeps the stored accent.
        state.set_ticked(false);
        assert_eq!(state.pending(), None);
        assert_eq!(state.note(), "");
        assert_eq!(body_accents(&state), serde_json::json!({}));

        // Tick again: the same pair is back, with the same note.
        state.set_ticked(true);
        assert_eq!(state.note(), note);
        assert_eq!(
            body_accents(&state),
            serde_json::json!({ "brand_accent_dark": dark, "brand_accent_light": light })
        );

        // A load or a successful save: nothing waits, the box stays ticked.
        state.clear();
        assert!(state.ticked);
        assert_eq!(state.note(), "");
        assert_eq!(body_accents(&state), serde_json::json!({}));
    }

    /// Apply pack with the box unticked writes nothing on Save, and the
    /// box never reads ticked with a blank note while a pack is applied:
    /// ticking it afterwards shows that pack's pair.
    #[test]
    fn applying_a_pack_with_the_box_unticked_sets_no_accents() {
        let (pack, _) = two_packs();
        let mut state = PackBrand::new();
        state.set_ticked(false);
        state.apply(&pack);
        assert_eq!(state.note(), "");
        assert_eq!(body_accents(&state), serde_json::json!({}));

        state.set_ticked(true);
        assert!(state.note().contains(pack.suggested_brand.accent_dark));
        assert_eq!(
            body_accents(&state)["brand_accent_dark"],
            pack.suggested_brand.accent_dark
        );
    }

    /// The note names the applied pack. Highlighting another card does not
    /// touch this state; applying another pack replaces the pair.
    #[test]
    fn the_note_names_the_applied_pack() {
        let (first, second) = two_packs();
        assert_ne!(first.name, second.name);
        let mut state = PackBrand::new();
        state.apply(&first);
        let note = state.note();
        assert!(
            note.contains(&format!("from the {} pack:", first.name)),
            "{note}"
        );
        assert!(!note.contains("this pack"), "{note}");
        assert!(!note.contains(second.name), "{note}");

        state.apply(&second);
        let note = state.note();
        assert!(
            note.contains(&format!("from the {} pack:", second.name)),
            "{note}"
        );
        assert!(!note.contains(first.name), "{note}");
        assert!(note.contains(second.suggested_brand.accent_dark), "{note}");
        assert_eq!(
            body_accents(&state)["brand_accent_light"],
            second.suggested_brand.accent_light
        );

        // A pack without a suggested accent offers nothing and does not
        // leave the older pack's pair waiting.
        let mut plain = second.clone();
        plain.suggested_brand = scuffed_types::PresetBrand::default();
        state.apply(&plain);
        assert_eq!(state.pending(), None);
        assert_eq!(state.note(), "");
        assert_eq!(body_accents(&state), serde_json::json!({}));
    }

    /// QA 694 nit 5: the pack path wrote its pair without the brand
    /// contrast lock. A pair the lock blocks is never offered, the note says
    /// why, and Save keeps the stored accent.
    #[test]
    fn a_pack_pair_that_fails_the_contrast_lock_is_never_saved() {
        let (first, _) = two_packs();
        let mut blocked = first.clone();
        blocked.suggested_brand = scuffed_types::PresetBrand {
            accent_dark: TEST_BLOCKED_ACCENT,
            accent_light: TEST_BLOCKED_ACCENT,
        };
        let mut state = PackBrand::new();
        state.apply(&first);
        assert!(state.pending().is_some());

        state.apply(&blocked);
        assert_eq!(
            state.pending(),
            None,
            "a blocked pair drops the older offer"
        );
        assert_eq!(body_accents(&state), serde_json::json!({}));
        let note = state.note();
        assert!(note.starts_with("Save keeps the current accent."), "{note}");
        assert!(
            note.contains(&format!("The {} pack's accents fail", first.name)),
            "{note}"
        );
        assert!(
            note.contains("Ink on this accent does not reach 4.5:1 in the dark theme."),
            "{note}"
        );

        // Ticking does not bring it back, and unticking hides the reason.
        state.set_ticked(false);
        assert_eq!(state.note(), "");
        state.set_ticked(true);
        assert_eq!(body_accents(&state), serde_json::json!({}));
        assert!(state.note().starts_with("Save keeps the current accent."));

        // A fill with no edge is refused the same way.
        let mut dark = first.clone();
        dark.suggested_brand = scuffed_types::PresetBrand {
            accent_dark: TEST_EDGELESS_ACCENT,
            accent_light: TEST_EDGELESS_ACCENT,
        };
        state.apply(&dark);
        assert_eq!(body_accents(&state), serde_json::json!({}));
        assert!(state.note().contains("lose their edge"), "{}", state.note());

        // The server answering clears the reason too.
        state.clear();
        assert_eq!(state.note(), "");
    }

    /// Every pack the catalog ships passes the lock, so the gate above
    /// never hides a shipped pack's pair.
    #[test]
    fn every_catalog_pack_pair_passes_the_contrast_lock() {
        for preset in homepage_presets() {
            if preset.suggested_brand.accent_dark.is_empty() {
                continue;
            }
            let mut state = PackBrand::new();
            state.apply(&preset);
            assert!(
                state.pending().is_some(),
                "{}: {}",
                preset.name,
                state.note()
            );
        }
    }

    fn render_element(element: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(element);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// The status line is always in the tree and carries no inline margin:
    /// the stylesheet spaces it, and gives the space back when it is empty.
    #[test]
    fn the_pack_status_line_is_always_rendered_and_leaves_no_gap_when_empty() {
        let empty = render_element(|| rsx! { PackBrandStatus { note: String::new() } });
        assert_eq!(
            empty,
            "<p class=\"settings-hint pack-brand-status\" role=\"status\"></p>"
        );
        let said = render_element(|| rsx! { PackBrandStatus { note: "Pending.".to_string() } });
        assert_eq!(
            said,
            "<p class=\"settings-hint pack-brand-status\" role=\"status\">Pending.</p>"
        );

        let css = crate::styles::admin::CSS;
        let hint = css.find(".settings-hint {").expect("hint rule");
        // The line is a direct child of the section card, so the empty rule
        // must also come after the card's hint spacing to win the tie.
        let card_hint = css
            .find(".form-section-card > .settings-hint {")
            .expect("card hint rule");
        let spaced = css
            .find(".pack-brand-status { margin-top: var(--space-2); }")
            .expect("spacing rule");
        let empty_rule = css
            .find(".pack-brand-status:empty { margin: 0; }")
            .expect("the empty line gives its margin back");
        assert!(hint < spaced && card_hint < spaced && spaced < empty_rule);
    }

    /// The page shows the state through these calls. Behaviour is covered
    /// above; this only holds the wiring.
    #[test]
    fn the_page_is_wired_to_the_pack_brand_state() {
        let src = include_str!("settings.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        for wiring in [
            "PackBrandStatus { note: pack_brand.read().note() }",
            "checked: pack_brand.read().ticked,",
            "onchange: move |e| pack_brand.write().set_ticked(e.checked()),",
            "pack_brand.write().apply(&preset);",
            "let (brand_accent_dark, brand_accent_light) = pack_brand.read().save_accents();",
        ] {
            assert!(prod.contains(wiring), "{wiring}");
        }
        assert_eq!(prod.matches("BrandSettingsLink {}").count(), 1);
    }

    const PACK_CLEAR: &str = "pack_brand.write().clear();";
    const LOAD_CALL: &str = ".fetch::<SiteSettings>(\"/api/settings\")";
    const SAVE_CALL: &str = ".put_json::<_, SiteSettings>(\"/api/settings\", &body)";

    /// The `Ok(s)` arm of the match that follows `call`: from its opening
    /// brace up to the `Err(e)` arm of the same match.
    fn ok_arm_after<'a>(prod: &'a str, call: &str) -> &'a str {
        assert_eq!(prod.matches(call).count(), 1, "{call}");
        let after = prod.split_once(call).unwrap().1;
        let arm = after.split_once("Ok(s) => {").expect("an Ok arm").1;
        arm.split_once("Err(e) => {").expect("an Err arm").0
    }

    /// How often the pack pair is cleared in the load Ok arm, in the save Ok
    /// arm, and on the whole page.
    fn pack_clears(prod: &str) -> (usize, usize, usize) {
        (
            ok_arm_after(prod, LOAD_CALL).matches(PACK_CLEAR).count(),
            ok_arm_after(prod, SAVE_CALL).matches(PACK_CLEAR).count(),
            prod.matches(PACK_CLEAR).count(),
        )
    }

    /// The pair is dropped after a load and after a good save, and nowhere
    /// else. A failed save must keep it, so the admin can save again.
    #[test]
    fn the_pack_pair_is_cleared_in_the_load_arm_and_the_ok_save_arm_only() {
        let src = include_str!("settings.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert_eq!(pack_clears(prod), (1, 1, 2));

        // The check sees a clear that leaves its arm. Moved after the save
        // match, it would also run on a failed save: same count, wrong arm.
        let (head, save) = prod.split_once(SAVE_CALL).unwrap();
        let after_match = "saving.set(false);";
        assert_eq!(save.matches(after_match).count(), 1);
        let moved = save.replacen(PACK_CLEAR, "", 1).replacen(
            after_match,
            &format!("{PACK_CLEAR}\n{after_match}"),
            1,
        );
        assert_eq!(pack_clears(&format!("{head}{SAVE_CALL}{moved}")), (1, 0, 2));

        // The same for the load clear, moved after the load match.
        let (head, load) = prod.split_once(LOAD_CALL).unwrap();
        let err_arm_end =
            "loaded.set(false);\n                    None\n                }\n            }\n";
        assert_eq!(load.matches(err_arm_end).count(), 1);
        let moved = load.replacen(PACK_CLEAR, "", 1).replacen(
            err_arm_end,
            &format!("{err_arm_end}{PACK_CLEAR}\n"),
            1,
        );
        assert_eq!(pack_clears(&format!("{head}{LOAD_CALL}{moved}")), (0, 1, 2));
    }

    /// A router that mounts only the link, so it resolves without the
    /// admin shell around it.
    #[derive(Routable, Clone, PartialEq, Debug)]
    enum LinkOnly {
        #[route("/")]
        LinkHost {},
    }

    #[component]
    fn LinkHost() -> Element {
        rsx! { BrandSettingsLink {} }
    }

    #[test]
    fn the_link_to_brand_settings_warns_about_unsaved_edits() {
        assert!(BRAND_LINK_WARNING.contains("drops unsaved edits"));
        fn host() -> Element {
            let history = use_hook(|| {
                std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path("/"))
            });
            rsx! {
                dioxus::router::components::HistoryProvider {
                    history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                    Router::<LinkOnly> {}
                }
            }
        }
        let html = render_element(host);
        let link = &html[html.find("<a ").expect("link")..];
        let (tag, rest) = link.split_once('>').expect("end of link tag");
        assert!(tag.contains("href=\"/admin/brand\""), "{tag}");
        assert!(
            tag.contains("aria-describedby=\"admin-settings-brand-link-warning\""),
            "{tag}"
        );
        assert!(rest.starts_with("Open brand settings</a>"), "{rest}");
        // The shared button recipe, not the legacy `btn-ghost` (Vision nit 3
        // on PR 706).
        assert!(tag.contains("class=\"btn is-ghost\""), "{tag}");
        // The id the link points at is the visible warning, once.
        assert_eq!(
            html.matches("id=\"admin-settings-brand-link-warning\"")
                .count(),
            1,
            "{html}"
        );
        let warning = &html[html
            .find("id=\"admin-settings-brand-link-warning\"")
            .unwrap()..];
        let warning = &warning[warning.find('>').unwrap() + 1..];
        assert!(
            warning.starts_with(&format!("{BRAND_LINK_WARNING}</p>")),
            "{warning}"
        );
    }

    #[test]
    fn failed_save_shows_the_committed_toggles() {
        let committed = SettingsToggles {
            recruitment_open: false,
            strategies_enabled: true,
            officers_can_edit_teams: false,
            sections: scuffed_types::HomepageSections {
                news: false,
                ..scuffed_types::HomepageSections::default()
            },
        };
        let draft = SettingsToggles {
            recruitment_open: true,
            strategies_enabled: false,
            officers_can_edit_teams: true,
            sections: scuffed_types::HomepageSections::default(),
        };
        assert_eq!(
            toggles_shown_after_save(&draft, &committed, false),
            committed
        );
        assert_eq!(toggles_shown_after_save(&draft, &committed, true), draft);
        assert_ne!(draft.recruitment_open, committed.recruitment_open);
        assert_ne!(draft.sections.news, committed.sections.news);
    }

    #[test]
    fn admin_settings_save_includes_officers_can_edit_teams() {
        for enabled in [false, true] {
            let body = UpdateSettingsRequest {
                strategies_enabled: Some(true),
                officers_can_edit_teams: admin_settings_officers_flag(enabled),
                ..UpdateSettingsRequest::default()
            };
            let value = serde_json::to_value(&body).expect("serialize settings save");
            assert_eq!(value["officers_can_edit_teams"], enabled);
            assert!(
                value.get("officers_can_edit_teams").is_some(),
                "Some(false) must still send the key"
            );
        }

        let omitted = serde_json::to_value(UpdateSettingsRequest::default()).expect("serialize");
        assert!(
            omitted.get("officers_can_edit_teams").is_none(),
            "omitted key is the officer-safe body; the admin save must not use Default for this field"
        );

        let src = include_str!("settings.rs");
        let save = src
            .split("let body = UpdateSettingsRequest")
            .nth(1)
            .expect("save body");
        let body_src = &save[..save.find("};").expect("end of save body")];
        assert!(
            body_src.contains("admin_settings_officers_flag(officers_can_edit_teams())"),
            "admin save must send the switch value"
        );
        assert!(
            !body_src.contains("officers_can_edit_teams: None"),
            "omitting the key would leave a stale flag on the server"
        );

        let hint = officers_can_edit_teams_hint();
        for field in ["name", "game", "color", "division"] {
            assert!(
                hint.to_lowercase().contains(field),
                "hint should name the field the team form edits: {field}"
            );
        }
        assert!(
            hint.contains("not on that form"),
            "hint must not claim lore quote is editable"
        );
        assert!(!hint.contains("and lore quote"));
        let team_request = include_str!("../../../../types/src/api/teams.rs");
        let create = team_request
            .split("struct CreateTeamRequest")
            .nth(1)
            .expect("CreateTeamRequest");
        let create = &create[..create.find('}').expect("end of CreateTeamRequest")];
        assert!(
            !create.contains("lore"),
            "the shared create body has no lore quote; the hint must follow that"
        );
        assert!(src.contains("officers_can_edit_teams_hint()"));
        assert!(
            src.contains("toggles_shown_after_save"),
            "a failed save must put the checkboxes back on the last server values"
        );

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut users = Vec::new();
        collect_rs_containing(&root, &root, "UpdateSettingsRequest", &mut users);
        users.sort();
        // Brand settings is admin-only too, and its body carries the two
        // accents and nothing else (its own test holds that), so it omits
        // this key and the server keeps the stored flag.
        assert_eq!(
            users,
            vec![
                "pages/admin/brand_settings.rs".to_string(),
                "pages/admin/settings.rs".to_string(),
            ],
            "only the two admin settings pages build a settings PUT, so officers never send this key"
        );
    }

    fn prod_source() -> &'static str {
        include_str!("settings.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("page source")
    }

    /// The nav placement editor is gone from admin. The save leaves the key
    /// out, so the server keeps the stored nav config as it is: nothing a
    /// stale bundle does not know can be dropped by a settings save.
    #[test]
    fn save_leaves_the_stored_nav_config_alone() {
        let prod = prod_source();
        let save = prod
            .split("let body = UpdateSettingsRequest")
            .nth(1)
            .expect("save body");
        let body_src = &save[..save.find("};").expect("end of save body")];
        assert!(body_src.contains("nav: None,"), "{body_src}");
        let omitted = serde_json::to_value(UpdateSettingsRequest::default()).expect("serialize");
        assert!(
            omitted.get("nav").is_none(),
            "an omitted nav key must not reach the server as null"
        );
        for gone in [
            "NavPlacement",
            "NavColumn",
            "editor_items",
            "set_placement",
            "move_item",
            "prepare_for_editor",
            "\"Navigation\"",
        ] {
            assert!(!prod.contains(gone), "{gone} is still on the settings page");
        }
        assert!(!crate::styles::admin::CSS.contains(".nav-column"));
        assert!(!crate::styles::admin::CSS.contains(".nav-row"));
    }

    /// Every field label on this page is tied to its control by id. (S4d
    /// allowed the two accent labels; S3 moved the accents to brand
    /// settings, so none is left untied.)
    #[test]
    fn field_labels_are_tied_by_id_and_nothing_is_styled_inline() {
        let prod = prod_source();
        let untied: Vec<&str> = prod
            .lines()
            .filter(|line| {
                line.contains("label { class: \"field-label\"")
                    || line.contains("label { class: \"form-label\"")
            })
            .filter(|line| !line.contains("r#for:"))
            .map(str::trim)
            .collect();
        assert!(untied.is_empty(), "labels without a for: {untied:?}");
        // Ids are fixed, page-unique names.
        let mut ids: Vec<&str> = prod
            .split("id: \"settings-")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        let count = ids.len();
        assert!(count > 40, "{count}");
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "a settings id is used twice");
        assert!(
            !prod.contains("style:"),
            "spacing comes from the shared classes"
        );
    }

    #[test]
    fn page_is_narrow_with_one_h1_and_an_always_rendered_save_line() {
        use crate::components::admin_shared::test_page::render_admin_route;
        use scuffed_types::OrgRole;

        let html = render_admin_route("/admin/settings", Some(OrgRole::Admin));
        assert!(
            html.contains("class=\"ui-shell ui-shell--narrow admin-settings-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Settings</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        // Nothing has loaded: a labelled wait, and Save does nothing yet.
        assert!(
            html.contains("role=\"status\">Loading settings...</p>"),
            "{html}"
        );
        assert!(
            html.contains(
                "id=\"settings-save\" aria-disabled=\"true\" aria-describedby=\"settings-save-status\""
            ),
            "{html}"
        );
        assert!(html.contains(">Save settings</button>"), "{html}");
        // The save line is in the tree before anything is saved.
        assert!(
            html.contains(
                "<p id=\"settings-save-status\" class=\"admin-form-status\" role=\"status\"></p>"
            ),
            "{html}"
        );

        // An officer gets the no-access page, with its own single H1.
        let denied = render_admin_route("/admin/settings", Some(OrgRole::Officer));
        assert_eq!(denied.matches("<h1").count(), 1, "{denied}");
        assert!(
            denied.contains("<h1>You do not have access</h1>"),
            "{denied}"
        );
        assert!(
            denied.contains("You need admin permissions to view settings."),
            "{denied}"
        );
        assert!(!denied.contains("id=\"settings-save\""), "{denied}");
    }

    #[test]
    fn strategies_hint_no_longer_talks_about_nav_placement() {
        let hint = strategies_hint();
        assert!(!hint.contains("Primary"), "{hint}");
        assert!(!hint.contains("More"), "{hint}");
        assert!(hint.contains("/strategy pages are blocked"), "{hint}");
    }
}
