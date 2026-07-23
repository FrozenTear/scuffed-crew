use dioxus::prelude::*;

use crate::components::{AccessDenied, Toast, use_toast};
use crate::state::use_auth;
use crate::theme::tokens::{BG_DARK, BRAND_ACCENT_DARK, BRAND_ACCENT_LIGHT};
use scuffed_api_client::ApiClient;
use scuffed_types::api::UpdateSettingsRequest;
use scuffed_types::{
    ContentAlign, HomeShell, HomeSkin, HomepageContent, NavConfig, NavPlacement, PublicLayout,
    SiteSettings, homepage_preset_by_id, homepage_presets,
};

#[component]
pub fn AdminSettings() -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let mut saving = use_signal(|| false);
    let mut loaded = use_signal(|| false);
    let mut load_error = use_signal(|| Option::<String>::None);
    let mut reload_tick = use_signal(|| 0u32);

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
    let mut brand_accent_dark = use_signal(String::new);
    let mut brand_accent_light = use_signal(String::new);
    // Selected identity pack id for “Apply pack”.
    let mut homepage_preset_id = use_signal(|| "neutral".to_string());
    let mut apply_suggested_brand = use_signal(|| true);
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
                    n.normalize();
                    nav.set(n);
                    page_bg_color.set(s.page_bg_color);
                    page_bg_image_url.set(s.page_bg_image_url);
                    brand_accent_dark.set(s.brand_accent_dark);
                    brand_accent_light.set(s.brand_accent_light);
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
        let body = UpdateSettingsRequest {
            org_name: Some(org_name().trim().to_string()),
            site_description: Some(site_description().trim().to_string()),
            recruitment_open: Some(recruitment_open()),
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
            brand_accent_dark: Some(brand_accent_dark().trim().to_string()),
            brand_accent_light: Some(brand_accent_light().trim().to_string()),
        };

        saving.set(true);
        spawn(async move {
            match ApiClient::web()
                .put_json::<_, SiteSettings>("/api/settings", &body)
                .await
            {
                Ok(s) => {
                    // Keep UI in sync with what the server actually stored.
                    public_layout.set(s.public_layout);
                    home_shell.set(s.home_shell);
                    home_skin.set(s.home_skin);
                    content_align.set(s.homepage.content_align);
                    homepage.set(s.homepage);
                    let mut n = s.nav;
                    n.normalize();
                    nav.set(n);
                    page_bg_color.set(s.page_bg_color);
                    page_bg_image_url.set(s.page_bg_image_url);
                    brand_accent_dark.set(s.brand_accent_dark);
                    brand_accent_light.set(s.brand_accent_light);
                    toast.show(Toast::success("Settings saved."));
                }
                Err(e) => toast.show(Toast::error(format!("Failed to save settings: {e}"))),
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
            div { style: "padding:1rem;border:1px solid var(--danger);border-radius:8px;background:color-mix(in srgb, var(--danger) 12%, transparent);margin-bottom:1rem;",
                p { style: "color:var(--danger);margin:0 0 0.75rem;", "Failed to load settings: {err}" }
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
            p { style: "color: var(--text-3);", "Loading settings…" }
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

            // —— Identity pack ——
            div { class: "form-section",
                h2 { "Homepage identity" }
                p { class: "form-section-lead",
                    "Start from a pack, then tune shell, skin, and sections. Apply pack fills copy — Save to persist."
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
                                checked: apply_suggested_brand(),
                                onchange: move |e| apply_suggested_brand.set(e.checked()),
                            }
                            "Also set brand accents"
                        }
                        button {
                            class: "btn-add",
                            r#type: "button",
                            onclick: move |_| {
                                let id = homepage_preset_id();
                                if let Some(preset) = homepage_preset_by_id(&id) {
                                    content_align.set(preset.content.content_align);
                                    homepage.set(preset.content);
                                    home_shell.set(preset.suggested_shell);
                                    home_skin.set(preset.suggested_skin);
                                    public_layout.set(preset.suggested_shell.to_public_layout());
                                    if apply_suggested_brand()
                                        && !preset.suggested_brand.accent_dark.is_empty()
                                    {
                                        brand_accent_dark
                                            .set(preset.suggested_brand.accent_dark.into());
                                        brand_accent_light
                                            .set(preset.suggested_brand.accent_light.into());
                                    }
                                    open_copy.set({
                                        let mut s = std::collections::HashSet::new();
                                        s.insert("hero".to_string());
                                        s
                                    });
                                    toast.show(Toast::success(format!(
                                        "Applied “{}” — click Save to persist.",
                                        preset.name,
                                    )));
                                } else {
                                    toast.show(Toast::error("Unknown pack."));
                                }
                            },
                            "Apply pack"
                        }
                    }
                }

                div { class: "form-section-card", style: "margin-top:0.85rem;",
                    label { class: "form-label", style: "margin-bottom:0.55rem;", "Shell" }
                    div { class: "option-tiles", style: "margin-bottom:1rem;",
                        {
                            let shells = [
                                (HomeShell::OpsHub, "Ops hub", "Dense board — schedule, roster table."),
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
                                (HomeSkin::Clean, "Clean", "Calmer product look — default for most packs."),
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
                    "Brand accents and page background. Empty accent = product default purple."
                }
                div { class: "form-section-card",
                    div { class: "settings-subhead", "Brand accents" }
                    div { class: "color-row",
                        div { class: "color-field",
                            label { class: "form-label", "Dark theme" }
                            div { class: "swatch-row",
                                input {
                                    r#type: "color",
                                    value: {
                                        let c = brand_accent_dark();
                                        if c.len() == 7 && c.starts_with('#') {
                                            c
                                        } else {
                                            BRAND_ACCENT_DARK.into()
                                        }
                                    },
                                    oninput: move |e| brand_accent_dark.set(e.value()),
                                }
                                input {
                                    class: "form-input",
                                    placeholder: "{BRAND_ACCENT_DARK}",
                                    value: "{brand_accent_dark}",
                                    oninput: move |e| brand_accent_dark.set(e.value()),
                                }
                            }
                        }
                        div { class: "color-field",
                            label { class: "form-label", "Light theme" }
                            div { class: "swatch-row",
                                input {
                                    r#type: "color",
                                    value: {
                                        let c = brand_accent_light();
                                        if c.len() == 7 && c.starts_with('#') {
                                            c
                                        } else {
                                            BRAND_ACCENT_LIGHT.into()
                                        }
                                    },
                                    oninput: move |e| brand_accent_light.set(e.value()),
                                }
                                input {
                                    class: "form-input",
                                    placeholder: "{BRAND_ACCENT_LIGHT}",
                                    value: "{brand_accent_light}",
                                    oninput: move |e| brand_accent_light.set(e.value()),
                                }
                            }
                        }
                        button {
                            class: "btn-ghost",
                            r#type: "button",
                            onclick: move |_| {
                                brand_accent_dark.set(String::new());
                                brand_accent_light.set(String::new());
                            },
                            "Clear accents"
                        }
                    }
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
                    label { class: "section-chip is-on", style: "margin-bottom:0.85rem;",
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
                                "Nav reset to small-org defaults — click Save to persist.",
                            ));
                        },
                        "Reset defaults"
                    }
                }
                p { class: "form-section-lead",
                    "Primary bar, More menu, or hidden. Small-org default: Members, Forum, Events, Stats primary; Tournaments / Scrims / Strategy under More. Hidden routes still work via URL."
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
                    hint: "Not in the nav — promote when you need them.",
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
        cfg.items_in(placement)
            .into_iter()
            .map(|i| {
                let label = NavConfig::catalog_label(&i.id)
                    .unwrap_or(i.id.as_str())
                    .to_string();
                (i.id.clone(), label)
            })
            .collect()
    };

    rsx! {
        div { class: "nav-column",
            h3 { "{title}" }
            p { class: "nav-hint", "{hint}" }
            if items.is_empty() {
                p { style: "color:var(--text-3);font-size:0.85rem;margin:0;", "None" }
            } else {
                ul { style: "list-style:none;margin:0;padding:0;display:flex;flex-direction:column;",
                    for (id, label) in items {
                        li {
                            key: "{id}",
                            class: "nav-row",
                            span { class: "nav-row-label", title: "{id}", "{label}" }
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
