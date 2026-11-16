//! Admin brand settings (`/admin/brand`): one brand accent and the display
//! font, with a live preview and a live contrast check.
//!
//! The draft lives in this page only. The previews paint it through inline
//! custom properties on their own nodes; the site chrome keeps the saved
//! accent until `PUT /api/settings` succeeds and the saved value is handed
//! to [`SiteSettingsState::apply_saved`].
//!
//! The server still stores a dark and a light accent. This page writes the
//! one accent to both.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{AccessDenied, Toast, use_toast};
use crate::state::brand_font::{BrandFontField, BrandFontState};
use crate::state::site_settings::SiteSettingsState;
use crate::state::use_auth;
use crate::styles::customise::{CUSTOMISE_CSS, CUSTOMISE_CSS_ID};
use crate::theme::brand::{AccentCheck, BrandConfig, SchemeCheck, format_ratio, normalize_hex};
use crate::theme::fonts::DisplayFont;
use crate::theme::tokens::BRAND_ACCENT_DARK;
use scuffed_api_client::ApiClient;
use scuffed_types::SiteSettings;
use scuffed_types::api::UpdateSettingsRequest;

/// `GET` and `PUT /api/settings` as this page reads them: the shared
/// settings plus the optional display font key (see `state::brand_font`).
///
/// Flatten trap: serde hands a flattened key to the first struct that
/// declares it. This works only while `SiteSettings` has no
/// `brand_font_display` field. The day the shared type gains one, `settings`
/// takes the key, `font` reads `None`, the picker stays locked, and
/// `settings_json_without_the_font_key_still_loads` fails. Drop
/// `BrandFontField` here in the same change and read the typed field.
#[derive(Debug, Clone, Deserialize)]
struct BrandSettingsView {
    #[serde(flatten)]
    settings: SiteSettings,
    #[serde(flatten)]
    font: BrandFontField,
}

/// The brand part of a settings save. Every other settings key is omitted,
/// so the server leaves it as it is.
#[derive(Debug, Clone, Serialize)]
struct BrandSaveBody {
    #[serde(flatten)]
    settings: UpdateSettingsRequest,
    /// A [`DisplayFont::id`]. Sent only when the server returned the key.
    #[serde(skip_serializing_if = "Option::is_none")]
    brand_font_display: Option<String>,
}

/// What the server last stored.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SavedBrand {
    /// Settings record id; element ids are built from it.
    pub id: String,
    pub accent_dark: String,
    pub accent_light: String,
    /// `None`: the server has no display font field yet.
    pub font: Option<DisplayFont>,
}

impl SavedBrand {
    fn from_view(view: &BrandSettingsView) -> Self {
        Self {
            id: view.settings.id.clone(),
            accent_dark: view.settings.brand_accent_dark.trim().to_string(),
            accent_light: view.settings.brand_accent_light.trim().to_string(),
            font: view.font.font(),
        }
    }

    /// The saved light accent, when an older save set one that differs.
    fn separate_light_accent(&self) -> Option<&str> {
        let light = self.accent_light.as_str();
        (!light.is_empty() && !light.eq_ignore_ascii_case(&self.accent_dark)).then_some(light)
    }
}

/// The unsaved edit.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BrandDraft {
    /// The hex field as typed. Empty means the product default.
    pub accent: String,
    pub font: DisplayFont,
}

impl BrandDraft {
    fn from_saved(saved: &SavedBrand) -> Self {
        Self {
            accent: saved.accent_dark.clone(),
            font: saved.font.unwrap_or_default(),
        }
    }

    fn product_default() -> Self {
        Self {
            accent: String::new(),
            font: DisplayFont::default(),
        }
    }
}

/// What the hex field accepts, as `normalize_hex` reads it: 3 or 6 hex
/// digits, and the `#` is optional.
const INVALID_HEX_REASON: &str = "Enter a hex color: 3 or 6 digits, with or without the #.";

/// The draft accent, judged. `brand` is what the previews paint.
struct Verdict {
    /// The accent to store: empty for the product default. `None` when the
    /// field is not a hex colour.
    accent: Option<String>,
    brand: BrandConfig,
    check: AccentCheck,
    /// Why Save is blocked. `None` when it is not.
    reason: Option<String>,
}

impl Verdict {
    fn of(draft: &BrandDraft, saved: &SavedBrand) -> Self {
        let typed = draft.accent.trim();
        let accent = if typed.is_empty() {
            Some(String::new())
        } else {
            normalize_hex(typed)
        };
        // A field that is not a colour keeps the saved accent in the preview.
        let brand = match &accent {
            Some(hex) => BrandConfig::from_settings(hex, hex),
            None => BrandConfig::from_settings(&saved.accent_dark, &saved.accent_light),
        };
        let check = AccentCheck::for_brand(&brand);
        let reason = match &accent {
            None => Some(INVALID_HEX_REASON.to_string()),
            Some(_) => check.fail_reason(),
        };
        Self {
            accent,
            brand,
            check,
            reason,
        }
    }

    fn blocked(&self) -> bool {
        self.reason.is_some()
    }
}

/// The body a Save sends, or `None` when the contrast lock blocks it.
fn save_body(draft: &BrandDraft, saved: &SavedBrand) -> Option<BrandSaveBody> {
    let verdict = Verdict::of(draft, saved);
    if verdict.blocked() {
        return None;
    }
    let accent = verdict.accent?;
    Some(BrandSaveBody {
        settings: UpdateSettingsRequest {
            brand_accent_dark: Some(accent.clone()),
            brand_accent_light: Some(accent),
            ..UpdateSettingsRequest::default()
        },
        brand_font_display: saved.font.map(|_| draft.font.id().to_string()),
    })
}

/// An element id from the settings record id and a field name.
fn field_id(record: &str, field: &str) -> String {
    let record: String = record
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{record}-{field}")
}

#[cfg(test)]
thread_local! {
    /// Test seam: the body `GET /api/settings` answers with.
    static TEST_BRAND_VIEW_JSON: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

async fn load_brand_view() -> Result<BrandSettingsView, String> {
    #[cfg(test)]
    if let Some(json) = TEST_BRAND_VIEW_JSON.with(|slot| slot.borrow().clone()) {
        return serde_json::from_str(&json).map_err(|err| err.to_string());
    }
    ApiClient::web()
        .fetch::<BrandSettingsView>("/api/settings")
        .await
        .map_err(|err| err.to_string())
}

#[component]
pub fn AdminBrandSettings() -> Element {
    let auth = use_auth();
    let mut reload_tick = use_signal(|| 0u32);
    let brand_font = try_use_context::<BrandFontState>();
    let settings = use_resource(move || {
        let _tick = reload_tick();
        async move {
            let view = load_brand_view().await?;
            if let Some(brand_font) = brand_font {
                brand_font.apply_saved(view.font.font());
            }
            Ok::<_, String>(SavedBrand::from_view(&view))
        }
    });

    if !auth().is_admin() {
        return rsx! {
            AccessDenied { message: "You need admin permissions to change the brand.".to_string() }
        };
    }

    let loaded = settings.read().clone();
    let status = match &loaded {
        None => "Loading brand settings...".to_string(),
        Some(Err(err)) => format!("Failed to load brand settings: {err}"),
        Some(Ok(_)) => String::new(),
    };
    let failed = matches!(loaded, Some(Err(_)));

    rsx! {
        crate::styles::StyleOnce { id: CUSTOMISE_CSS_ID, css: CUSTOMISE_CSS }
        div { class: "customise",
            header { class: "customise-header",
                h1 { "Brand settings" }
                p { class: "customise-lead",
                    "Admins set the accent and the display font for everyone. Members still choose their own theme and density on the Appearance page."
                }
            }
            p { class: "customise-status", role: "status", "{status}" }
            if failed {
                div { class: "customise-actions",
                    button {
                        class: "customise-btn",
                        r#type: "button",
                        onclick: move |_| reload_tick += 1,
                        "Retry"
                    }
                }
            }
            if let Some(Ok(saved)) = loaded {
                BrandSettingsForm { saved }
            }
        }
    }
}

/// Owns the draft and the save. `saved` is the value the page loaded.
#[component]
fn BrandSettingsForm(saved: SavedBrand) -> Element {
    let mut toast = use_toast();
    let site_settings = try_use_context::<SiteSettingsState>();
    let brand_font = try_use_context::<BrandFontState>();
    let mut saved_now = use_signal(|| saved.clone());
    let mut draft = use_signal(|| BrandDraft::from_saved(&saved));
    let mut saving = use_signal(|| false);

    let on_save = move |_| {
        if saving() {
            return;
        }
        // The contrast lock: a blocked draft has no body, so nothing is sent.
        let Some(body) = save_body(&draft.read(), &saved_now.read()) else {
            return;
        };
        saving.set(true);
        spawn(async move {
            match ApiClient::web()
                .put_json::<_, BrandSettingsView>("/api/settings", &body)
                .await
            {
                Ok(view) => {
                    let next = SavedBrand::from_view(&view);
                    // Only now does the chrome follow the new brand.
                    if let Some(site_settings) = site_settings {
                        site_settings.apply_saved(view.settings);
                    }
                    if let Some(brand_font) = brand_font {
                        brand_font.apply_saved(next.font);
                    }
                    draft.set(BrandDraft::from_saved(&next));
                    saved_now.set(next);
                    toast.show(Toast::success("Brand saved."));
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to save brand: {e}")));
                }
            }
            saving.set(false);
        });
    };

    rsx! {
        BrandFormBody {
            draft,
            saved: saved_now(),
            saving: saving(),
            on_save,
        }
    }
}

/// The form itself. It only reads and writes `draft`.
#[component]
fn BrandFormBody(
    mut draft: Signal<BrandDraft>,
    saved: SavedBrand,
    saving: bool,
    on_save: EventHandler<()>,
) -> Element {
    let current = draft();
    let verdict = Verdict::of(&current, &saved);
    let blocked = verdict.blocked();
    let reason = verdict.reason.clone().unwrap_or_default();
    let invalid = verdict.accent.is_none();

    let hex_id = field_id(&saved.id, "brand-accent-hex");
    let reason_id = field_id(&saved.id, "brand-accent-reason");
    let accent_title_id = field_id(&saved.id, "brand-accent-title");
    let font_title_id = field_id(&saved.id, "brand-font-display-title");
    let font_note_id = field_id(&saved.id, "brand-font-display-note");
    let font_name = field_id(&saved.id, "brand-font-display");

    // The swatch and the colour input show the dark accent of the preview.
    let swatch = verdict.brand.accent_dark.clone();
    let font_supported = saved.font.is_some();
    let font_note = if font_supported {
        String::new()
    } else {
        format!(
            "The display font cannot be changed yet. The site uses {}.",
            DisplayFont::default().label()
        )
    };
    let save_locked = blocked || saving;
    // Save rewrites a separate light accent, so the note that says so is
    // part of what Save is described by, next to the block reason.
    let light_note_id = field_id(&saved.id, "brand-accent-light-note");
    let separate_light = saved.separate_light_accent().map(str::to_string);
    let mut save_describedby = Vec::new();
    if blocked {
        save_describedby.push(reason_id.clone());
    }
    if separate_light.is_some() {
        save_describedby.push(light_note_id.clone());
    }
    let save_describedby = save_describedby.join(" ");

    rsx! {
        section { class: "customise-card", "aria-labelledby": "{accent_title_id}",
            p { class: "customise-kicker", "Accent" }
            h2 { class: "customise-title", id: "{accent_title_id}", "One brand color" }
            p {
                "This accent is used in both dark and light. Soft, text, and ink-on-accent colors are derived from it."
            }
            div { class: "customise-accent-row",
                label { class: "customise-swatch", style: "background-color: {swatch};",
                    input {
                        r#type: "color",
                        "aria-label": "Pick accent color",
                        value: "{swatch}",
                        oninput: move |e| draft.write().accent = e.value(),
                    }
                }
                div { class: "customise-field",
                    label { r#for: "{hex_id}", "Accent hex" }
                    input {
                        id: "{hex_id}",
                        r#type: "text",
                        spellcheck: "false",
                        autocomplete: "off",
                        placeholder: "{BRAND_ACCENT_DARK}",
                        "aria-invalid": if invalid { "true" },
                        "aria-describedby": if blocked { "{reason_id}" },
                        value: "{current.accent}",
                        oninput: move |e| draft.write().accent = e.value(),
                    }
                }
            }
            p { class: "customise-hint", "Leave the hex empty to use the product default." }
            div { class: "customise-contrast-grid",
                PreviewCard { check: verdict.check.dark.clone(), font: current.font }
                PreviewCard { check: verdict.check.light.clone(), font: current.font }
            }
            // Always rendered; only the text changes.
            p { class: "customise-reason", id: "{reason_id}", role: "status", "{reason}" }
        }
        section { class: "customise-card", "aria-labelledby": "{font_title_id}",
            p { class: "customise-kicker", "Font" }
            h2 { class: "customise-title", id: "{font_title_id}", "Display font" }
            p {
                "Used for headings, buttons, and the top bar. Body text stays IBM Plex Sans."
            }
            div {
                class: "customise-choice-row",
                role: "radiogroup",
                "aria-labelledby": "{font_title_id}",
                "aria-describedby": if !font_supported { "{font_note_id}" },
                for font in DisplayFont::ALL {
                    {
                        let selected = current.font == font;
                        let mut class = String::from("customise-choice");
                        if selected {
                            class.push_str(" is-selected");
                        }
                        if !font_supported {
                            class.push_str(" is-locked");
                        }
                        rsx! {
                            label {
                                key: "{font.id()}",
                                class: "{class}",
                                style: "font-family: {font.family_var()};",
                                input {
                                    r#type: "radio",
                                    name: "{font_name}",
                                    value: "{font.id()}",
                                    checked: selected,
                                    disabled: !font_supported,
                                    onchange: move |_| draft.write().font = font,
                                }
                                span { class: "customise-choice-mark", "aria-hidden": "true" }
                                "{font.label()}"
                            }
                        }
                    }
                }
            }
            p { class: "customise-hint", id: "{font_note_id}", "{font_note}" }
        }
        div { class: "customise-actions",
            if let Some(light) = separate_light {
                p { class: "customise-note", id: "{light_note_id}",
                    "The light theme has its own saved accent ({light}). Save brand replaces it: both themes get the one accent above."
                }
            }
            button {
                class: if save_locked { "customise-btn" } else { "customise-btn is-primary" },
                r#type: "button",
                "aria-disabled": if save_locked { "true" } else { "false" },
                "aria-describedby": if !save_describedby.is_empty() { "{save_describedby}" },
                onclick: move |_| on_save.call(()),
                if saving { "Saving..." } else { "Save brand" }
            }
            button {
                class: "customise-btn",
                r#type: "button",
                onclick: move |_| draft.set(BrandDraft::product_default()),
                "Reset to default"
            }
        }
    }
}

/// One theme's preview. The draft reaches it only through inline custom
/// properties on this node, so nothing outside the card changes.
#[component]
fn PreviewCard(check: SchemeCheck, font: DisplayFont) -> Element {
    let (theme, title) = match check.scheme {
        crate::theme::brand::Scheme::Dark => ("dark", "Dark preview"),
        crate::theme::brand::Scheme::Light => ("light", "Light preview"),
    };
    let style = format!("{} {}", check.roles.css_decls(), font.css_decl());
    rsx! {
        article {
            class: "customise-contrast-card",
            "data-theme": "{theme}",
            "data-preview": "{theme}",
            style: "{style}",
            h3 { "{title}" }
            // A sample of a primary button, not a control.
            span { class: "customise-preview-btn", "aria-hidden": "true", "Join us" }
            p { class: "customise-contrast-sample", "Accent text on the page background." }
            ContrastLine {
                label: "Ink on accent",
                kind: "on-accent",
                ratio: check.ink_on_accent,
                passes: check.ink_passes(),
            }
            ContrastLine {
                label: "Accent on background",
                kind: "accent-text",
                ratio: check.accent_on_bg,
                passes: check.text_passes(),
            }
        }
    }
}

/// A ratio with the word Pass or Fails, and a warning shape on a fail.
#[component]
fn ContrastLine(label: &'static str, kind: &'static str, ratio: f64, passes: bool) -> Element {
    let ratio = format_ratio(ratio);
    rsx! {
        div { class: "customise-contrast-line",
            span { "{label}" }
            if passes {
                span { class: "is-pass", "data-contrast": "{kind}", "{ratio} Pass" }
            } else {
                span { class: "is-fail", "data-contrast": "{kind}",
                    span { class: "customise-fail-icon", "aria-hidden": "true" }
                    "{ratio} Fails"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ToastProvider;
    use crate::state::AuthState;
    use crate::theme::brand::{
        TEST_BLOCKED_ACCENT, TEST_INVALID_ACCENTS, TEST_MINT_ACCENT, TEST_OTHER_ACCENT,
        TEST_SHORT_ACCENT,
    };
    use scuffed_types::{OrgRole, UserInfo};

    fn saved(accent: &str) -> SavedBrand {
        SavedBrand {
            id: "site".into(),
            accent_dark: accent.into(),
            accent_light: accent.into(),
            font: None,
        }
    }

    fn draft(accent: &str) -> BrandDraft {
        BrandDraft {
            accent: accent.into(),
            font: DisplayFont::Urbanist,
        }
    }

    /// The chrome stylesheet for `saved_accent` next to the form with
    /// `draft_accent`, as the app root and the page render them.
    fn render_form(saved_accent: &'static str, draft_accent: &'static str) -> String {
        render_saved(saved(saved_accent), draft_accent)
    }

    /// The form for one stored brand and one typed accent.
    fn render_saved(saved: SavedBrand, draft_accent: &'static str) -> String {
        #[component]
        fn Harness(saved: SavedBrand, draft_accent: &'static str) -> Element {
            let draft = use_signal(|| draft(draft_accent));
            rsx! {
                ToastProvider {
                    BrandFormBody {
                        draft,
                        saved,
                        saving: false,
                        on_save: move |_| {},
                    }
                }
            }
        }
        let mut dom = VirtualDom::new_with_props(
            Harness,
            HarnessProps {
                saved,
                draft_accent,
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// Settings JSON as the server sends it today, with both accents set.
    fn settings_json(accent: &str) -> String {
        let accent = serde_json::to_string(accent.trim()).unwrap();
        format!(
            r#"{{"id":"site","org_name":"Clan","site_description":"desc","recruitment_open":true,"recruitment_message":"msg","min_age":16,"forum_backend":"local","extra_relay_urls":"","brand_accent_dark":{accent},"brand_accent_light":{accent},"updated_at":"2026-09-21T00:00:00Z"}}"#
        )
    }

    fn settle(dom: &mut VirtualDom) {
        for _ in 0..12 {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
    }

    thread_local! {
        static TEST_DRAFT: std::cell::Cell<Option<Signal<BrandDraft>>> =
            const { std::cell::Cell::new(None) };
        static TEST_SITE: std::cell::Cell<Option<SiteSettingsState>> =
            const { std::cell::Cell::new(None) };
    }

    struct ClearHooks;

    impl Drop for ClearHooks {
        fn drop(&mut self) {
            crate::state::site_settings::set_test_embedded_json(None);
            crate::state::site_settings::mock_settings_fetch(None);
            TEST_BRAND_VIEW_JSON.with(|slot| *slot.borrow_mut() = None);
            TEST_DRAFT.with(|slot| slot.set(None));
            TEST_SITE.with(|slot| slot.set(None));
        }
    }

    /// The opening tag of the Save button.
    fn save_button(html: &str) -> &str {
        let end = html.find(">Save brand<").expect("save button");
        let start = html[..end].rfind("<button").expect("button tag");
        &html[start..end]
    }

    #[test]
    fn contrast_lock_blocks_save_with_aria_attributes() {
        let html = render_form(TEST_MINT_ACCENT, TEST_BLOCKED_ACCENT);
        let button = save_button(&html);
        assert!(button.contains("aria-disabled=\"true\""), "{button}");
        assert!(
            button.contains("aria-describedby=\"site-brand-accent-reason\""),
            "{button}"
        );
        // Focusable: blocked through aria and the handler, not `disabled`.
        assert!(!button.contains(" disabled"), "{button}");
        assert!(!button.contains("tabindex"), "{button}");
        // The id it points at is the visible reason.
        assert!(
            html.contains(
                "id=\"site-brand-accent-reason\" role=\"status\">Ink on this accent does not reach 4.5:1 in the dark preview.</p>"
            ),
            "{html}"
        );
        assert!(html.contains("Fails"), "{html}");
        assert!(html.contains("customise-fail-icon"), "{html}");

        // The handler half of the lock: no request body exists.
        assert!(save_body(&draft(TEST_BLOCKED_ACCENT), &saved(TEST_MINT_ACCENT)).is_none());
        let src = include_str!("brand_settings.rs");
        let handler = src
            .split("let on_save = move |_| {")
            .nth(1)
            .expect("save handler");
        let guard = handler.find("save_body(").expect("lock check");
        let request = handler.find(".put_json").expect("request");
        assert!(guard < request, "the lock must run before the request");
    }

    #[test]
    fn a_passing_accent_leaves_save_open() {
        let html = render_form(TEST_MINT_ACCENT, TEST_OTHER_ACCENT);
        let button = save_button(&html);
        assert!(button.contains("aria-disabled=\"false\""), "{button}");
        assert!(!button.contains("aria-describedby"), "{button}");
        assert!(button.contains("is-primary"), "{button}");
        // The live region is there, empty.
        assert!(
            html.contains("id=\"site-brand-accent-reason\" role=\"status\"></p>"),
            "{html}"
        );
        assert!(!html.contains("Fails"), "{html}");
    }

    #[test]
    fn mint_preview_shows_the_frame_ratios() {
        let html = render_form(TEST_MINT_ACCENT, TEST_MINT_ACCENT);
        assert_eq!(html.matches("9.9:1 Pass").count(), 2, "{html}");
        assert!(html.contains("5.4:1 Pass"), "{html}");
        assert!(html.contains("5.0:1 Pass"), "{html}");
        assert!(html.contains("data-preview=\"dark\""), "{html}");
        assert!(html.contains("data-preview=\"light\""), "{html}");
    }

    #[test]
    fn text_that_is_not_a_hex_colour_blocks_save() {
        for typed in TEST_INVALID_ACCENTS {
            let html = render_form(TEST_MINT_ACCENT, typed);
            let button = save_button(&html);
            assert!(button.contains("aria-disabled=\"true\""), "{typed}");
            assert!(html.contains("aria-invalid=\"true\""), "{typed}");
            assert!(
                html.contains("Enter a hex color: 3 or 6 digits, with or without the #."),
                "{typed}"
            );
            assert!(save_body(&draft(typed), &saved(TEST_MINT_ACCENT)).is_none());
        }
    }

    /// The message names every form the field takes, and each of them saves.
    #[test]
    fn the_hex_message_matches_what_the_field_accepts() {
        assert!(INVALID_HEX_REASON.contains("3 or 6 digits"));
        assert!(INVALID_HEX_REASON.contains("with or without the #"));
        let six = TEST_OTHER_ACCENT.trim_start_matches('#');
        for typed in [TEST_OTHER_ACCENT, six, TEST_SHORT_ACCENT, "abc"] {
            let html = render_form(TEST_MINT_ACCENT, typed);
            assert!(!html.contains(INVALID_HEX_REASON), "{typed}");
            assert!(!html.contains("aria-invalid"), "{typed}");
            let body = save_body(&draft(typed), &saved(TEST_MINT_ACCENT)).expect(typed);
            let stored = body.settings.brand_accent_dark.expect("accent");
            assert_eq!(stored.len(), 7, "{typed}");
            assert!(stored.starts_with('#'), "{typed}");
        }
        for typed in ["ab", "abcd", "abcde", "abcdefa"] {
            let html = render_form(TEST_MINT_ACCENT, typed);
            assert!(html.contains(INVALID_HEX_REASON), "{typed}");
        }
    }

    /// Save rewrites a separate light accent. The note that says so sits in
    /// the action row and is part of the Save button's description.
    #[test]
    fn save_is_described_by_the_separate_light_accent_note() {
        let mut split = saved(TEST_MINT_ACCENT);
        split.accent_light = TEST_OTHER_ACCENT.into();
        let html = render_saved(split.clone(), TEST_MINT_ACCENT);
        let button = save_button(&html);
        assert!(
            button.contains("aria-describedby=\"site-brand-accent-light-note\""),
            "{button}"
        );
        let actions = html
            .split("class=\"customise-actions\"")
            .nth(1)
            .expect("action row");
        let note = format!(
            "<p class=\"customise-note\" id=\"site-brand-accent-light-note\">The light theme has its own saved accent ({TEST_OTHER_ACCENT}). Save brand replaces it: both themes get the one accent above.</p>"
        );
        assert!(actions.contains(&note), "{actions}");
        assert!(actions.find(&note) < actions.find(">Save brand<"));

        // Blocked as well: the reason comes first, the note is still there.
        let html = render_saved(split, TEST_BLOCKED_ACCENT);
        let button = save_button(&html);
        assert!(
            button.contains(
                "aria-describedby=\"site-brand-accent-reason site-brand-accent-light-note\""
            ),
            "{button}"
        );

        // One saved accent: no note, and nothing for Save to point at.
        let html = render_form(TEST_MINT_ACCENT, TEST_MINT_ACCENT);
        assert!(!html.contains("brand-accent-light-note"), "{html}");
        assert!(!save_button(&html).contains("aria-describedby"));
    }

    /// The selection is a shape: each font choice carries the ring the
    /// selected dot is drawn in (the CSS half is pinned in
    /// `styles::customise`).
    #[test]
    fn every_font_choice_draws_the_selection_mark() {
        let html = render_form(TEST_MINT_ACCENT, TEST_MINT_ACCENT);
        assert_eq!(
            html.matches("<span class=\"customise-choice-mark\" aria-hidden=\"true\"></span>")
                .count(),
            DisplayFont::ALL.len(),
            "{html}"
        );
        assert_eq!(html.matches("customise-choice is-selected").count(), 1);
    }

    /// The chrome stylesheet here is built the way the app root builds it
    /// (`chrome_theme_css_for` over the shared settings), so it can move: the
    /// last step proves that. A draft must not move it; a save must.
    #[test]
    fn chrome_accent_follows_saved_settings_and_not_the_draft() {
        #[component]
        fn Harness() -> Element {
            let site = crate::state::provide_site_settings();
            TEST_SITE.with(|slot| slot.set(Some(site)));
            let chrome = {
                let resolved = site.resolved.read();
                crate::theme::chrome_theme_css_for(crate::state::loaded_site_settings(
                    resolved.as_ref(),
                ))
            };
            let draft = use_signal(|| draft(TEST_MINT_ACCENT));
            TEST_DRAFT.with(|slot| slot.set(Some(draft)));
            rsx! {
                style { id: "chrome", "{chrome}" }
                ToastProvider {
                    BrandFormBody {
                        draft,
                        saved: saved(TEST_MINT_ACCENT),
                        saving: false,
                        on_save: move |_| {},
                    }
                }
            }
        }
        fn split(html: &str) -> (&str, &str) {
            let (before, rest) = html
                .split_once("<style id=\"chrome\">")
                .expect("chrome css");
            assert!(!before.contains("--accent"), "{before}");
            rest.split_once("</style>").expect("end of chrome css")
        }

        let _clear = ClearHooks;
        crate::state::site_settings::set_test_embedded_json(Some(settings_json(TEST_MINT_ACCENT)));
        crate::state::site_settings::mock_settings_fetch(Some(settings_json(TEST_MINT_ACCENT)));
        let saved_roles = BrandConfig::from_settings(TEST_MINT_ACCENT, TEST_MINT_ACCENT);
        let draft_roles = BrandConfig::from_settings(TEST_OTHER_ACCENT, TEST_OTHER_ACCENT);

        let mut dom = VirtualDom::new(Harness);
        dom.rebuild_in_place();
        settle(&mut dom);
        let html = dioxus_ssr::render(&dom);
        let (chrome_saved, _) = split(&html);
        let chrome_saved = chrome_saved.to_string();
        assert!(chrome_saved.contains(&saved_roles.dark.css_decls()));
        assert!(chrome_saved.contains(&saved_roles.light.css_decls()));

        // Type a passing draft. The chrome stylesheet is byte for byte the
        // same; only the previews change.
        let mut typed = TEST_DRAFT.with(|slot| slot.get().expect("draft signal"));
        typed.set(draft(TEST_OTHER_ACCENT));
        settle(&mut dom);
        let html = dioxus_ssr::render(&dom);
        let (chrome, page) = split(&html);
        assert_eq!(chrome, chrome_saved, "a draft must not reach the chrome");
        assert!(!chrome.contains(&draft_roles.dark.css_decls()));

        // The draft is on the two preview cards (and the swatch) only.
        assert_eq!(
            page.matches(&draft_roles.dark.css_decls()).count(),
            1,
            "{page}"
        );
        assert_eq!(
            page.matches(&draft_roles.light.css_decls()).count(),
            1,
            "{page}"
        );
        let outside_previews: String = page
            .split("<article")
            .enumerate()
            .map(|(i, part)| match (i, part.split_once("</article>")) {
                (0, _) => part,
                (_, Some((_, after))) => after,
                (_, None) => "",
            })
            .collect();
        assert!(!outside_previews.contains("--accent"), "{outside_previews}");
        assert!(
            !outside_previews.contains("--brand-accent"),
            "{outside_previews}"
        );

        // Only a successful save hands the brand to the shared settings.
        let src = include_str!("brand_settings.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert_eq!(prod.matches(".apply_saved(view.settings)").count(), 1);
        let save = prod
            .split(".put_json::<_, BrandSettingsView>(\"/api/settings\", &body)")
            .nth(1)
            .expect("settings PUT");
        let ok_arm = &save[..save.find("Err(e) =>").expect("error arm")];
        assert!(ok_arm.contains("site_settings.apply_saved(view.settings)"));
        assert!(
            !prod.contains("StyleLive"),
            "the draft must not write a document stylesheet"
        );

        // What the success arm does: the saved settings reach the shared
        // state, and now the same chrome stylesheet does move.
        let saved_settings: SiteSettings =
            serde_json::from_str(&settings_json(TEST_OTHER_ACCENT)).expect("settings JSON");
        let site = TEST_SITE.with(|slot| slot.get().expect("settings state"));
        site.apply_saved(saved_settings);
        settle(&mut dom);
        let html = dioxus_ssr::render(&dom);
        let (chrome, _) = split(&html);
        assert_ne!(chrome, chrome_saved, "a save must reach the chrome");
        assert!(chrome.contains(&draft_roles.dark.css_decls()), "{chrome}");
        assert!(chrome.contains(&draft_roles.light.css_decls()), "{chrome}");
        assert!(!chrome.contains(&saved_roles.dark.css_decls()));
    }

    #[test]
    fn save_writes_the_one_accent_to_both_themes_and_nothing_else() {
        let body =
            save_body(&draft(TEST_SHORT_ACCENT), &saved(TEST_MINT_ACCENT)).expect("passing accent");
        let value = serde_json::to_value(&body).expect("serialize");
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["brand_accent_dark", "brand_accent_light"]);
        assert_eq!(value["brand_accent_dark"], value["brand_accent_light"]);
        assert_eq!(value["brand_accent_dark"].as_str().unwrap().len(), 7);

        // Reset to default stores the empty accent.
        let body = save_body(&BrandDraft::product_default(), &saved(TEST_MINT_ACCENT))
            .expect("the product default passes");
        let value = serde_json::to_value(&body).expect("serialize");
        assert_eq!(value["brand_accent_dark"], "");
        assert_eq!(value["brand_accent_light"], "");
    }

    #[test]
    fn display_font_is_locked_until_the_server_has_the_field() {
        // No key in the response: radios disabled, the key is never sent.
        let html = render_form(TEST_MINT_ACCENT, TEST_MINT_ACCENT);
        assert_eq!(html.matches("type=\"radio\"").count(), 2, "{html}");
        assert_eq!(html.matches("disabled=true").count(), 2, "{html}");
        assert!(
            html.contains("The display font cannot be changed yet. The site uses Urbanist."),
            "{html}"
        );
        let mut plex = draft(TEST_MINT_ACCENT);
        plex.font = DisplayFont::IbmPlexSans;
        let body = save_body(&plex, &saved(TEST_MINT_ACCENT)).unwrap();
        assert!(body.brand_font_display.is_none());

        // The key is there: the choice is sent as a listed id.
        let mut with_font = saved(TEST_MINT_ACCENT);
        with_font.font = Some(DisplayFont::Urbanist);
        let body = save_body(&plex, &with_font).unwrap();
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(
            value[crate::state::brand_font::BRAND_FONT_FIELD],
            "ibm-plex-sans"
        );

        // The picker is a fixed list: no free-text font or URL field.
        assert_eq!(html.matches("type=\"text\"").count(), 1, "{html}");
        assert!(!html.contains("type=\"url\""), "{html}");
    }

    #[test]
    fn settings_json_without_the_font_key_still_loads() {
        let json = r#"{"id":"site","org_name":"Clan","site_description":"desc","recruitment_open":true,"recruitment_message":"msg","min_age":16,"forum_backend":"local","extra_relay_urls":"","updated_at":"2026-09-21T00:00:00Z"}"#;
        let view: BrandSettingsView = serde_json::from_str(json).expect("today's settings JSON");
        assert_eq!(SavedBrand::from_view(&view).font, None);

        let with_key = json.replacen('{', r#"{"brand_font_display":"ibm-plex-sans","#, 1);
        let view: BrandSettingsView = serde_json::from_str(&with_key).expect("future JSON");
        assert_eq!(
            SavedBrand::from_view(&view).font,
            Some(DisplayFont::IbmPlexSans)
        );
        assert_eq!(view.settings.org_name, "Clan");
    }

    #[test]
    fn element_ids_come_from_the_settings_record() {
        assert_eq!(
            field_id("site", "brand-accent-hex"),
            "site-brand-accent-hex"
        );
        assert_eq!(
            field_id("site_settings:main", "brand-accent-hex"),
            "site-settings-main-brand-accent-hex"
        );
        let html = render_form(TEST_MINT_ACCENT, TEST_MINT_ACCENT);
        assert!(html.contains("for=\"site-brand-accent-hex\""), "{html}");
        assert!(html.contains("id=\"site-brand-accent-hex\""), "{html}");
        assert!(html.contains("aria-label=\"Pick accent color\""), "{html}");
    }

    /// Who is looking at `/admin/brand`.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Viewer {
        Guest,
        /// Signed in; `None` is an account with no member row.
        Account(Option<OrgRole>),
    }

    /// A router that mounts the page alone, without the admin shell. The
    /// shell has its own officer gate; this test is about the page's gate,
    /// and `AccessDenied` needs a router for its link.
    #[derive(Routable, Clone, PartialEq, Debug)]
    enum PageOnly {
        #[route("/")]
        Page {},
    }

    #[component]
    fn Page() -> Element {
        rsx! {
            ToastProvider { AdminBrandSettings {} }
        }
    }

    /// The page for `viewer`, with the settings fetch answered (through the
    /// test seam) and polled to the end. Returns the HTML before the fetch
    /// resolved and after.
    fn render_page(viewer: Viewer) -> (String, String) {
        #[component]
        fn Harness(viewer: Viewer) -> Element {
            let auth = use_signal(|| AuthState {
                user: match viewer {
                    Viewer::Guest => None,
                    Viewer::Account(role) => Some(UserInfo {
                        id: "u1".into(),
                        username: "Player01".into(),
                        avatar_url: None,
                        role,
                    }),
                },
                loading: false,
            });
            use_context_provider(|| auth);
            let history = use_hook(|| {
                std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path("/"))
            });
            rsx! {
                dioxus::router::components::HistoryProvider {
                    history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                    Router::<PageOnly> {}
                }
            }
        }
        TEST_BRAND_VIEW_JSON
            .with(|slot| *slot.borrow_mut() = Some(settings_json(TEST_MINT_ACCENT)));
        let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { viewer });
        dom.rebuild_in_place();
        let pending = dioxus_ssr::render(&dom);
        settle(&mut dom);
        (pending, dioxus_ssr::render(&dom))
    }

    const BRAND_CONTROLS: [&str; 7] = [
        "type=\"color\"",
        "type=\"radio\"",
        "Accent hex",
        "Display font",
        "Save brand",
        "Reset to default",
        "customise-contrast-card",
    ];

    #[test]
    fn members_cannot_see_brand_controls() {
        let _clear = ClearHooks;

        // The settings have loaded, so an admin gets every control. This is
        // what makes the absence below mean something: the same needles in
        // the same loaded state.
        let (pending, admin) = render_page(Viewer::Account(Some(OrgRole::Admin)));
        assert!(
            pending.contains("role=\"status\">Loading brand settings...</p>"),
            "{pending}"
        );
        assert!(!pending.contains("Save brand"), "{pending}");
        assert!(!admin.contains("Access Denied"), "{admin}");
        assert_eq!(admin.matches("<h1").count(), 1, "{admin}");
        assert!(admin.contains("Brand settings"), "{admin}");
        for control in BRAND_CONTROLS {
            assert!(
                admin.contains(control),
                "admin is missing {control}: {admin}"
            );
        }

        for viewer in [
            Viewer::Guest,
            Viewer::Account(None),
            Viewer::Account(Some(OrgRole::Recruit)),
            Viewer::Account(Some(OrgRole::Member)),
            Viewer::Account(Some(OrgRole::Officer)),
        ] {
            let (pending, loaded) = render_page(viewer);
            for html in [pending, loaded] {
                assert!(html.contains("Access Denied"), "{viewer:?}: {html}");
                assert!(!html.contains("Brand settings"), "{viewer:?}: {html}");
                for control in BRAND_CONTROLS {
                    assert!(!html.contains(control), "{viewer:?} sees {control}");
                }
            }
        }
    }
}
