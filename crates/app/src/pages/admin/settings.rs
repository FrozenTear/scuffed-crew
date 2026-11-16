use dioxus::prelude::*;

use crate::components::{AccessDenied, Toast, use_toast};
use crate::routes::Route;
use crate::state::site_settings::SiteSettingsState;
use crate::state::use_auth;
use crate::theme::tokens::BG_DARK;
use scuffed_api_client::ApiClient;
use scuffed_types::api::UpdateSettingsRequest;
use scuffed_types::{
    ContentAlign, HomeShell, HomeSkin, HomepageContent, NavConfig, NavPlacement, PublicLayout,
    SiteSettings, homepage_preset_by_id, homepage_presets,
};

/// Two pages send `PUT /api/settings`, both admin-only: this one and brand
/// settings, whose body carries the accent pair (and `brand_font_display`
/// once the server returns that key) and no other settings key. This page
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
}

impl PackBrand {
    fn new() -> Self {
        Self {
            ticked: true,
            offer: None,
        }
    }

    /// Apply pack was pressed for `preset`. A pack with no suggested accent
    /// offers nothing and drops an older pack's offer.
    fn apply(&mut self, preset: &scuffed_types::HomepagePreset) {
        let brand = preset.suggested_brand;
        self.offer = (!brand.accent_dark.is_empty()).then(|| PackAccents {
            pack: preset.name.to_string(),
            dark: brand.accent_dark.to_string(),
            light: brand.accent_light.to_string(),
        });
    }

    fn set_ticked(&mut self, on: bool) {
        self.ticked = on;
    }

    /// The server answered (a load or a successful save): nothing is waiting.
    fn clear(&mut self) {
        self.offer = None;
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
            class: "btn-ghost",
            to: Route::AdminBrandSettings {},
            "aria-describedby": "admin-settings-brand-link-warning",
            "Open brand settings"
        }
        p {
            class: "settings-hint",
            id: "admin-settings-brand-link-warning",
            style: "margin-top:0.4rem;",
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

#[component]
pub fn AdminSettings() -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let mut saving = use_signal(|| false);
    let mut loaded = use_signal(|| false);
    let mut load_error = use_signal(|| Option::<String>::None);
    let mut reload_tick = use_signal(|| 0u32);
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
    let mut nav = use_signal(NavConfig::default);
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
                    let mut n = s.nav;
                    // Keep GET payload rows (e.g. live Contabo `patch_notes`).
                    // `normalize()` would drop ids this WASM catalog does not know.
                    n.prepare_for_editor();
                    nav.set(n);
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

    let on_save = move |_| {
        let age: u32 = min_age().trim().parse().unwrap_or(16);
        // Merge alignment from its own signal so nested write() can't get lost.
        let mut hp = homepage();
        hp.content_align = content_align();
        let mut n = nav();
        n.normalize();
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
            nav: Some(n),
            page_bg_color: Some(page_bg_color().trim().to_string()),
            page_bg_image_url: Some(page_bg_image_url().trim().to_string()),
            brand_accent_dark,
            brand_accent_light,
        };

        saving.set(true);
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
                    let mut n = s.nav;
                    n.prepare_for_editor();
                    nav.set(n);
                    page_bg_color.set(s.page_bg_color);
                    page_bg_image_url.set(s.page_bg_image_url);
                    pack_brand.write().clear();
                    strategies_enabled.set(s.strategies_enabled);
                    officers_can_edit_teams.set(s.officers_can_edit_teams);
                    recruitment_open.set(s.recruitment_open);
                    committed_toggles.set(Some(committed));
                    toast.show(Toast::success("Settings saved."));
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

    rsx! {
        div { class: "admin-toolbar sticky-actions",
            h1 { "Settings" }
            div { style: "display:flex;gap:0.5rem;flex-wrap:wrap;align-items:center;",
                a {
                    class: "btn-ghost",
                    href: "/",
                    target: "_blank",
                    rel: "noopener",
                    "View homepage"
                }
                button {
                    class: "btn-add",
                    disabled: saving() || !loaded(),
                    onclick: on_save,
                    if saving() { "Saving..." } else { "Save Settings" }
                }
            }
        }

        if let Some(err) = load_error() {
            div { style: "padding:1rem;border:1px solid var(--loss);border-radius:8px;background:color-mix(in srgb, var(--loss) 12%, transparent);margin-bottom:1rem;",
                p { style: "color:var(--loss);margin:0 0 0.75rem;", "Failed to load settings: {err}" }
                button {
                    class: "btn-add",
                    r#type: "button",
                    onclick: move |_| {
                        load_error.set(None);
                        reload_tick.set(reload_tick() + 1);
                    },
                    "Retry"
                }
            }
        } else if !loaded() {
            p { style: "color: var(--text-subtle);", "Loading settings…" }
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
                            label { class: "form-label", "Organization name" }
                            input {
                                class: "form-input",
                                value: "{org_name}",
                                oninput: move |e| org_name.set(e.value()),
                            }
                        }
                        div { class: "form-field span-full",
                            label { class: "form-label", "Site description" }
                            textarea {
                                class: "form-textarea",
                                style: "min-height:4rem;",
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
                    label { class: "form-label", style: "margin-bottom:0.5rem;", "Identity pack" }
                    div { class: "pack-grid",
                        for p in homepage_presets() {
                            {
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
                                        onclick: move |_| homepage_preset_id.set(id.clone()),
                                        span { class: "pack-card-name", "{p.name}" }
                                        span { class: "pack-card-desc", "{p.description}" }
                                        span { class: "pack-card-meta", "{shell} · {skin}" }
                                    }
                                }
                            }
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

                div { class: "form-section-card", style: "margin-top:0.85rem;",
                    label { class: "form-label", style: "margin-bottom:0.55rem;", "Shell" }
                    div { class: "option-tiles", style: "margin-bottom:1rem;",
                        {
                            let shells = [
                                (HomeShell::OpsHub, "Ops hub", "Dense board. Schedule, roster table."),
                                (HomeShell::RecruitLanding, "Recruit landing", "Apply-first, team cards, more space."),
                                (HomeShell::Minimal, "Minimal", "Hero + apply; lean front door."),
                                (HomeShell::Manifesto, "Manifesto", "Principles first; comps quieter."),
                            ];
                            rsx! {
                                for (shell, title, blurb) in shells {
                                    {
                                        let selected = home_shell() == shell;
                                        let cls = if selected { "option-tile is-selected" } else { "option-tile" };
                                        rsx! {
                                            button {
                                                class: "{cls}",
                                                r#type: "button",
                                                onclick: move |_| {
                                                    home_shell.set(shell);
                                                    public_layout.set(shell.to_public_layout());
                                                },
                                                div { class: "option-tile-title", "{title}" }
                                                div { class: "option-tile-blurb", "{blurb}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    label { class: "form-label", style: "margin-bottom:0.55rem;", "Skin" }
                    div { class: "option-tiles", style: "margin-bottom:1rem;",
                        {
                            let skins = [
                                (HomeSkin::Clean, "Clean", "Calmer product look. Default for most packs."),
                                (HomeSkin::Esports, "Esports", "Clipped badges, denser competitive DNA."),
                            ];
                            rsx! {
                                for (skin, title, blurb) in skins {
                                    {
                                        let selected = home_skin() == skin;
                                        let cls = if selected { "option-tile is-selected" } else { "option-tile" };
                                        rsx! {
                                            button {
                                                class: "{cls}",
                                                r#type: "button",
                                                onclick: move |_| home_skin.set(skin),
                                                div { class: "option-tile-title", "{title}" }
                                                div { class: "option-tile-blurb", "{blurb}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "form-field", style: "max-width:16rem;",
                        label { class: "form-label", "Hero text alignment" }
                        select {
                            class: "form-input",
                            value: "{content_align().as_str()}",
                            onchange: move |e| {
                                content_align.set(ContentAlign::from_str_lossy(&e.value()));
                            },
                            option { value: "left", "Left" }
                            option { value: "center", "Center" }
                        }
                        p { class: "settings-hint", "Hero only. Body blocks stay left." }
                    }
                }

                div { class: "form-section-card", style: "margin-top:0.85rem;",
                    label { class: "form-label", style: "margin-bottom:0.35rem;", "Visible sections" }
                    p { class: "settings-hint", style: "margin-bottom:0.65rem;",
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
                            div { class: "section-chips",
                                for (key, label, checked) in sections {
                                    {
                                        let chip_class = if checked {
                                            "section-chip is-on"
                                        } else {
                                            "section-chip"
                                        };
                                        rsx! {
                                            label { class: "{chip_class}",
                                                input {
                                                    r#type: "checkbox",
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
                }
            }

            div { class: "form-section",
                h2 { "Appearance" }
                p { class: "form-section-lead",
                    "Page background. The brand accent and the display font have their own page."
                }
                div { class: "form-section-card",
                    div { class: "settings-subhead", "Brand" }
                    p { class: "settings-hint", style: "margin-bottom:0.65rem;",
                        "One accent for both themes, with a live preview and a contrast check."
                    }
                    BrandSettingsLink {}
                    div { class: "settings-divider" }
                    div { class: "settings-subhead", "Page background" }
                    div { class: "color-row", style: "margin-bottom:0.85rem;",
                        div { class: "color-field", style: "max-width:18rem;flex:1 1 14rem;",
                            label { class: "form-label", "Background color" }
                            div { class: "swatch-row",
                                input {
                                    r#type: "color",
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
                                    class: "form-input",
                                    placeholder: "empty = theme default",
                                    value: "{page_bg_color}",
                                    oninput: move |e| page_bg_color.set(e.value()),
                                }
                            }
                        }
                        button {
                            class: "btn-ghost",
                            r#type: "button",
                            onclick: move |_| page_bg_color.set(String::new()),
                            "Clear color"
                        }
                    }
                    div { class: "form-field",
                        label { class: "form-label", "Background image URL" }
                        p { class: "settings-hint", style: "margin-bottom:0.4rem;",
                            "https://… or /uploads/bg.jpg"
                        }
                        div { style: "display:flex;align-items:center;gap:0.65rem;flex-wrap:wrap;",
                            input {
                                class: "form-input",
                                style: "flex:1;min-width:12rem;max-width:28rem;",
                                placeholder: "https://… or /uploads/…",
                                value: "{page_bg_image_url}",
                                oninput: move |e| page_bg_image_url.set(e.value()),
                            }
                            button {
                                class: "btn-ghost",
                                r#type: "button",
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
                    label {
                        class: if recruitment_open() { "section-chip is-on" } else { "section-chip" },
                        style: "margin-bottom:0.85rem;",
                        input {
                            r#type: "checkbox",
                            checked: recruitment_open(),
                            onchange: move |e| recruitment_open.set(e.checked()),
                        }
                        "Recruitment open"
                    }
                    div { class: "form-grid",
                        div { class: "form-field span-full",
                            label { class: "form-label", "Message when closed" }
                            textarea {
                                class: "form-textarea",
                                value: "{recruitment_message}",
                                oninput: move |e| recruitment_message.set(e.value()),
                            }
                        }
                        div { class: "form-field",
                            label { class: "form-label", "Minimum age" }
                            input {
                                class: "form-input",
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
                    {
                        let chip_class = if strategies_enabled() {
                            "section-chip is-on"
                        } else {
                            "section-chip"
                        };
                        rsx! {
                            label { class: "{chip_class}", style: "margin-bottom:0.65rem;",
                                input {
                                    r#type: "checkbox",
                                    checked: strategies_enabled(),
                                    onchange: move |e| strategies_enabled.set(e.checked()),
                                }
                                "Strategies"
                            }
                        }
                    }
                    p { class: "settings-hint",
                        "On (default): Strategies stays in Primary/More if you placed it there. Off: hide it from the public nav and block /strategy pages. Existing plans are not deleted. Public Patch Notes (/patch-notes) are unchanged."
                    }
                    {
                        let chip_class = if officers_can_edit_teams() {
                            "section-chip is-on"
                        } else {
                            "section-chip"
                        };
                        rsx! {
                            label { class: "{chip_class}", style: "margin:0.85rem 0 0.65rem;",
                                input {
                                    r#type: "checkbox",
                                    checked: officers_can_edit_teams(),
                                    onchange: move |e| officers_can_edit_teams.set(e.checked()),
                                }
                                "Officers can edit teams"
                            }
                        }
                    }
                    p { class: "settings-hint",
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
                            StrField { label: "Badge", value: homepage().hero_badge.clone(), on_change: move |v| homepage.write().hero_badge = v }
                            StrField { label: "Primary CTA", value: homepage().cta_primary.clone(), on_change: move |v| homepage.write().cta_primary = v }
                            StrField { label: "Title", value: homepage().hero_title.clone(), on_change: move |v| homepage.write().hero_title = v }
                            StrField { label: "Title accent", value: homepage().hero_title_accent.clone(), on_change: move |v| homepage.write().hero_title_accent = v }
                            div { class: "span-full",
                                AreaField { label: "Subtitle", value: homepage().hero_sub.clone(), on_change: move |v| homepage.write().hero_sub = v }
                            }
                            div { class: "span-full",
                                StrField { label: "Secondary CTA", value: homepage().cta_secondary.clone(), on_change: move |v| homepage.write().cta_secondary = v }
                            }
                        }
                    }
                    CopyPanel {
                        id: "ethos",
                        title: "Ethos",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { label: "Kicker", value: homepage().ethos_kicker.clone(), on_change: move |v| homepage.write().ethos_kicker = v }
                            StrField { label: "Title", value: homepage().ethos_title.clone(), on_change: move |v| homepage.write().ethos_title = v }
                            div { class: "span-full",
                                AreaField { label: "Body", value: homepage().ethos_body.clone(), on_change: move |v| homepage.write().ethos_body = v }
                            }
                            div { class: "span-full",
                                LinesField { label: "Rules", value: homepage().ethos_rules.clone(), on_change: move |v| homepage.write().ethos_rules = v }
                            }
                        }
                    }
                    CopyPanel {
                        id: "squads",
                        title: "Squads",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { label: "Kicker", value: homepage().teams_kicker.clone(), on_change: move |v| homepage.write().teams_kicker = v }
                            StrField { label: "Title", value: homepage().teams_title.clone(), on_change: move |v| homepage.write().teams_title = v }
                            div { class: "span-full",
                                StrField { label: "Empty", value: homepage().teams_empty.clone(), on_change: move |v| homepage.write().teams_empty = v }
                            }
                        }
                    }
                    CopyPanel {
                        id: "news",
                        title: "Announcements",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { label: "Kicker", value: homepage().news_kicker.clone(), on_change: move |v| homepage.write().news_kicker = v }
                            StrField { label: "Title", value: homepage().news_title.clone(), on_change: move |v| homepage.write().news_title = v }
                            StrField { label: "Empty", value: homepage().news_empty.clone(), on_change: move |v| homepage.write().news_empty = v }
                            StrField { label: "View all", value: homepage().news_view_all.clone(), on_change: move |v| homepage.write().news_view_all = v }
                        }
                    }
                    CopyPanel {
                        id: "tournaments",
                        title: "Tournaments",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { label: "Kicker", value: homepage().tournaments_kicker.clone(), on_change: move |v| homepage.write().tournaments_kicker = v }
                            StrField { label: "Title", value: homepage().tournaments_title.clone(), on_change: move |v| homepage.write().tournaments_title = v }
                            StrField { label: "Empty", value: homepage().tournaments_empty.clone(), on_change: move |v| homepage.write().tournaments_empty = v }
                            StrField { label: "View all", value: homepage().tournaments_view_all.clone(), on_change: move |v| homepage.write().tournaments_view_all = v }
                        }
                    }
                    CopyPanel {
                        id: "schedule",
                        title: "Schedule",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { label: "Kicker", value: homepage().schedule_kicker.clone(), on_change: move |v| homepage.write().schedule_kicker = v }
                            StrField { label: "Title", value: homepage().schedule_title.clone(), on_change: move |v| homepage.write().schedule_title = v }
                            StrField { label: "Empty", value: homepage().schedule_empty.clone(), on_change: move |v| homepage.write().schedule_empty = v }
                            StrField { label: "Calendar CTA", value: homepage().calendar_cta.clone(), on_change: move |v| homepage.write().calendar_cta = v }
                        }
                    }
                    CopyPanel {
                        id: "recruit",
                        title: "Recruiting",
                        open_copy: open_copy,
                        div { class: "form-grid",
                            StrField { label: "Kicker", value: homepage().recruit_kicker.clone(), on_change: move |v| homepage.write().recruit_kicker = v }
                            StrField { label: "Title", value: homepage().recruit_title.clone(), on_change: move |v| homepage.write().recruit_title = v }
                            div { class: "span-full",
                                AreaField { label: "Body", value: homepage().recruit_body.clone(), on_change: move |v| homepage.write().recruit_body = v }
                            }
                            StrField { label: "CTA", value: homepage().recruit_cta.clone(), on_change: move |v| homepage.write().recruit_cta = v }
                            StrField { label: "Expectations title", value: homepage().recruit_expectations_title.clone(), on_change: move |v| homepage.write().recruit_expectations_title = v }
                            div { class: "span-full",
                                LinesField { label: "Expectations", value: homepage().recruit_expectations.clone(), on_change: move |v| homepage.write().recruit_expectations = v }
                            }
                            StrField { label: "Never-ask title", value: homepage().never_ask_title.clone(), on_change: move |v| homepage.write().never_ask_title = v }
                            StrField { label: "Seeking label", value: homepage().seeking_label.clone(), on_change: move |v| homepage.write().seeking_label = v }
                            div { class: "span-full",
                                AreaField { label: "Never-ask body", value: homepage().never_ask_body.clone(), on_change: move |v| homepage.write().never_ask_body = v }
                            }
                            div { class: "span-full",
                                LinesField { label: "Seeking tags", value: homepage().seeking_tags.clone(), on_change: move |v| homepage.write().seeking_tags = v }
                            }
                        }
                    }
                    CopyPanel {
                        id: "footer",
                        title: "Footer",
                        open_copy: open_copy,
                        StrField { label: "Footer note", value: homepage().footer_note.clone(), on_change: move |v| homepage.write().footer_note = v }
                    }
                }
            }

            div { class: "form-section",
                div { style: "display:flex;justify-content:space-between;align-items:flex-start;flex-wrap:wrap;gap:0.75rem;margin-bottom:0.35rem;",
                    h2 { style: "margin:0;", "Navigation" }
                    button {
                        class: "btn-ghost",
                        r#type: "button",
                        onclick: move |_| {
                            nav.set(NavConfig::default());
                            toast.show(Toast::success(
                                "Nav reset to small-org defaults. Click Save to persist.",
                            ));
                        },
                        "Reset defaults"
                    }
                }
                p { class: "form-section-lead",
                    if strategies_enabled() {
                        "Primary bar, More menu, or hidden. Small-org default: Members, Forum, Events, Stats primary; Tournaments / Scrims / Strategy under More. Hidden routes still work via URL."
                    } else {
                        "Primary bar, More menu, or hidden. Strategies is off under Features. It will not appear in Primary or More even if placed here, and /strategy URLs are blocked. Patch Notes stay available."
                    }
                }
                NavColumn {
                    title: "Primary bar",
                    hint: "Shown across the top next to the logo.",
                    placement: NavPlacement::Primary,
                    nav: nav,
                }
                NavColumn {
                    title: "More menu",
                    hint: "Overflow dropdown. Hidden entirely if empty.",
                    placement: NavPlacement::More,
                    nav: nav,
                }
                NavColumn {
                    title: "Hidden",
                    hint: "Not in the nav. Promote when you need them.",
                    placement: NavPlacement::Hidden,
                    nav: nav,
                }
            }

            div { class: "form-section",
                h2 { "Forum" }
                p { class: "form-section-lead", "Where forum threads live and optional extra Nostr relays." }
                div { class: "form-section-card",
                    div { class: "form-field",
                        label { class: "form-label", "Forum backend" }
                        select {
                            class: "form-input",
                            value: "{forum_backend}",
                            onchange: move |e| forum_backend.set(e.value()),
                            option { value: "local", "Local (SurrealDB)" }
                            option { value: "nostr", "Nostr (Relay)" }
                        }
                        p { class: "settings-hint",
                            "Local uses the database. Nostr uses the relay (requires relay setup)."
                        }
                    }
                    div { class: "form-field", style: "margin-top:0.85rem;",
                        label { class: "form-label", "Extra relay URLs" }
                        textarea {
                            class: "form-textarea",
                            placeholder: "ws://relay2:7777\nwss://relay.example.com",
                            value: "{extra_relay_urls}",
                            oninput: move |e| extra_relay_urls.set(e.value()),
                        }
                        p { class: "settings-hint",
                            "One per line. Published to primary relay (NOSTR_RELAY_URL) plus these."
                        }
                    }
                }
            }
            }
        }
    }
}

#[component]
fn NavColumn(
    title: &'static str,
    hint: &'static str,
    placement: NavPlacement,
    mut nav: Signal<NavConfig>,
) -> Element {
    let items: Vec<(String, String)> = {
        let cfg = nav();
        cfg.editor_items(placement)
            .into_iter()
            .map(|row| (row.id, row.label))
            .collect()
    };

    rsx! {
        div { class: "nav-column",
            h3 { "{title}" }
            p { class: "nav-hint", "{hint}" }
            if items.is_empty() {
                p { style: "color:var(--text-subtle);font-size:0.85rem;margin:0;", "None" }
            } else {
                ul { style: "list-style:none;margin:0;padding:0;display:flex;flex-direction:column;",
                    for (id, label) in items {
                        li {
                            key: "nav-item-{id}",
                            class: "nav-row",
                            span { class: "nav-row-label", title: "{id}", "{label}" }
                            span { class: "nav-row-id", "{id}" }
                            select {
                                class: "form-input",
                                value: "{placement.as_str()}",
                                onchange: {
                                    let id = id.clone();
                                    move |e| {
                                        let p = NavPlacement::from_str_lossy(&e.value());
                                        nav.write().set_placement(&id, p);
                                    }
                                },
                                option { value: "primary", "Primary" }
                                option { value: "more", "More" }
                                option { value: "hidden", "Hidden" }
                            }
                            button {
                                class: "row-btn",
                                r#type: "button",
                                title: "Move up",
                                onclick: {
                                    let id = id.clone();
                                    move |_| nav.write().move_item(&id, -1)
                                },
                                "↑"
                            }
                            button {
                                class: "row-btn",
                                r#type: "button",
                                title: "Move down",
                                onclick: {
                                    let id = id.clone();
                                    move |_| nav.write().move_item(&id, 1)
                                },
                                "↓"
                            }
                        }
                    }
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
    rsx! {
        div { class: "{panel_class}",
            button {
                class: "copy-panel-head",
                r#type: "button",
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
                span { class: "copy-panel-chevron", if is_open { "▼" } else { "▶" } }
            }
            if is_open {
                div { class: "copy-panel-body", {children} }
            }
        }
    }
}

#[component]
fn StrField(label: &'static str, value: String, on_change: EventHandler<String>) -> Element {
    rsx! {
        div { class: "form-field",
            label { class: "form-label", "{label}" }
            input {
                class: "form-input",
                value: "{value}",
                oninput: move |e| on_change.call(e.value()),
            }
        }
    }
}

#[component]
fn AreaField(label: &'static str, value: String, on_change: EventHandler<String>) -> Element {
    rsx! {
        div { class: "form-field",
            label { class: "form-label", "{label}" }
            textarea {
                class: "form-textarea",
                value: "{value}",
                oninput: move |e| on_change.call(e.value()),
            }
        }
    }
}

#[component]
fn LinesField(
    label: &'static str,
    value: Vec<String>,
    on_change: EventHandler<Vec<String>>,
) -> Element {
    let joined = value.join("\n");
    rsx! {
        div { class: "form-field",
            label { class: "form-label", "{label} (one per line)" }
            textarea {
                class: "form-textarea",
                style: "min-height:5.5rem;",
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
        let spaced = css
            .find(".pack-brand-status { margin-top: 0.65rem; }")
            .expect("spacing rule");
        let empty_rule = css
            .find(".pack-brand-status:empty { margin: 0; }")
            .expect("the empty line gives its margin back");
        assert!(hint < spaced && spaced < empty_rule);
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
        // After a load and after a successful save.
        assert_eq!(prod.matches("pack_brand.write().clear();").count(), 2);
        assert_eq!(prod.matches("BrandSettingsLink {}").count(), 1);
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
}
