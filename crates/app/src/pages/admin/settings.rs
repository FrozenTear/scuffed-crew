use dioxus::prelude::*;

use crate::components::{Toast, use_toast};
use scuffed_api_client::ApiClient;
use scuffed_types::api::UpdateSettingsRequest;
use scuffed_types::{
    ContentAlign, HomepageContent, NavConfig, NavPlacement, PublicLayout, SiteSettings,
};

#[component]
pub fn AdminSettings() -> Element {
    let mut toast = use_toast();
    let mut saving = use_signal(|| false);
    let mut loaded = use_signal(|| false);

    let mut org_name = use_signal(String::new);
    let mut site_description = use_signal(String::new);
    let mut recruitment_open = use_signal(|| false);
    let mut recruitment_message = use_signal(String::new);
    let mut min_age = use_signal(|| "16".to_string());
    let mut forum_backend = use_signal(|| "local".to_string());
    let mut extra_relay_urls = use_signal(String::new);
    let mut public_layout = use_signal(|| PublicLayout::Hub);
    let mut content_align = use_signal(|| ContentAlign::Left);
    let mut homepage = use_signal(HomepageContent::default);
    let mut nav = use_signal(NavConfig::default);
    let mut page_bg_color = use_signal(String::new);
    let mut page_bg_image_url = use_signal(String::new);

    let _settings = use_resource(move || async move {
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
                content_align.set(s.homepage.content_align);
                homepage.set(s.homepage);
                let mut n = s.nav;
                n.normalize();
                nav.set(n);
                page_bg_color.set(s.page_bg_color);
                page_bg_image_url.set(s.page_bg_image_url);
                loaded.set(true);
                Some(true)
            }
            Err(_) => None,
        }
    });

    let on_save = move |_| {
        let age: u32 = min_age().trim().parse().unwrap_or(16);
        // Merge alignment from its own signal so nested write() can't get lost.
        let mut hp = homepage();
        hp.content_align = content_align();
        let mut n = nav();
        n.normalize();
        let body = UpdateSettingsRequest {
            org_name: Some(org_name().trim().to_string()),
            site_description: Some(site_description().trim().to_string()),
            recruitment_open: Some(recruitment_open()),
            recruitment_message: Some(recruitment_message().trim().to_string()),
            min_age: Some(age),
            forum_backend: Some(forum_backend()),
            extra_relay_urls: Some(extra_relay_urls().trim().to_string()),
            public_layout: Some(public_layout()),
            homepage: Some(hp.clone()),
            nav: Some(n),
            page_bg_color: Some(page_bg_color().trim().to_string()),
            page_bg_image_url: Some(page_bg_image_url().trim().to_string()),
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
                    content_align.set(s.homepage.content_align);
                    homepage.set(s.homepage);
                    let mut n = s.nav;
                    n.normalize();
                    nav.set(n);
                    page_bg_color.set(s.page_bg_color);
                    page_bg_image_url.set(s.page_bg_image_url);
                    toast.show(Toast::success("Settings saved."));
                }
                Err(e) => toast.show(Toast::error(format!("Failed to save settings: {e}"))),
            }
            saving.set(false);
        });
    };

    rsx! {
        div { class: "admin-toolbar",
            h1 { "Settings" }
            button {
                class: "btn-add",
                disabled: saving() || !loaded(),
                onclick: on_save,
                if saving() { "Saving..." } else { "Save Settings" }
            }
        }

        if !loaded() {
            p { style: "color: var(--text-3);", "Loading settings…" }
        } else {
            div { class: "form-section",
                h2 { "Organization" }
                div { class: "form-field",
                    label { class: "form-label", "Organization Name" }
                    input {
                        class: "form-input",
                        value: "{org_name}",
                        oninput: move |e| org_name.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Site Description" }
                    textarea {
                        class: "form-textarea",
                        value: "{site_description}",
                        oninput: move |e| site_description.set(e.value()),
                    }
                }
            }

            div { class: "form-section",
                div { style: "display:flex;justify-content:space-between;align-items:center;flex-wrap:wrap;gap:0.75rem;margin-bottom:0.5rem;",
                    h2 { style: "margin:0;", "Navigation" }
                    button {
                        class: "btn-add",
                        r#type: "button",
                        style: "background:transparent;border:1px solid var(--border);",
                        onclick: move |_| {
                            nav.set(NavConfig::default());
                            toast.show(Toast::success(
                                "Nav reset to lean defaults — click Save to persist.",
                            ));
                        },
                        "Reset defaults"
                    }
                }
                p { style: "color:var(--text-3);font-size:0.85rem;margin-bottom:0.85rem;",
                    "Pick which pages show in the top bar, the More menu, or stay hidden. Hidden pages still work via direct URL. Logo, Apply, and account stay fixed."
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
                h2 { "Public homepage layout" }
                p { style: "color:var(--text-3);font-size:0.85rem;margin-bottom:0.75rem;",
                    "Hub hides empty marketing sections. Landing keeps them visible with empty-state copy."
                }
                div { class: "form-field", style: "margin-bottom:0.85rem;",
                    label { class: "form-label", "Layout mode" }
                    select {
                        class: "form-input",
                        value: "{public_layout().as_str()}",
                        onchange: move |e| public_layout.set(PublicLayout::from_str_lossy(&e.value())),
                        option { value: "hub", "Hub (recommended)" }
                        option { value: "landing", "Landing" }
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Text alignment" }
                    p { style: "color:var(--text-3);font-size:0.8rem;margin:0 0 0.4rem;",
                        "Center only affects the hero. Body sections stay left so headers and lists match."
                    }
                    select {
                        class: "form-input",
                        value: "{content_align().as_str()}",
                        onchange: move |e| {
                            content_align.set(ContentAlign::from_str_lossy(&e.value()));
                        },
                        option { value: "left", "Left" }
                        option { value: "center", "Center" }
                    }
                }
            }

            div { class: "form-section",
                h2 { "Page background" }
                p { style: "color:var(--text-3);font-size:0.85rem;margin-bottom:0.75rem;",
                    "Solid color and optional image for the public site. Leave color empty to use the theme default (dark/light). Image sits behind content, cover-fitted."
                }
                div { class: "form-field", style: "margin-bottom:0.85rem;",
                    label { class: "form-label", "Background color" }
                    div { style: "display:flex;align-items:center;gap:0.65rem;flex-wrap:wrap;",
                        input {
                            r#type: "color",
                            // color input needs a full #rrggbb value
                            value: {
                                let c = page_bg_color();
                                if c.len() == 7 && c.starts_with('#') { c } else { "#17171d".into() }
                            },
                            oninput: move |e| page_bg_color.set(e.value()),
                            style: "width:3rem;height:2.25rem;padding:0;border:1px solid var(--border);border-radius:6px;background:transparent;cursor:pointer;",
                        }
                        input {
                            class: "form-input",
                            style: "flex:1;min-width:8rem;",
                            placeholder: "#17171d (empty = theme default)",
                            value: "{page_bg_color}",
                            oninput: move |e| page_bg_color.set(e.value()),
                        }
                        button {
                            class: "btn-add",
                            r#type: "button",
                            style: "background:transparent;border:1px solid var(--border);",
                            onclick: move |_| page_bg_color.set(String::new()),
                            "Clear"
                        }
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Background image URL" }
                    p { style: "color:var(--text-3);font-size:0.8rem;margin:0 0 0.4rem;",
                        "https://… or a site path like /uploads/bg.jpg. Leave empty for no image."
                    }
                    div { style: "display:flex;align-items:center;gap:0.65rem;flex-wrap:wrap;",
                        input {
                            class: "form-input",
                            style: "flex:1;min-width:12rem;",
                            placeholder: "https://… or /uploads/…",
                            value: "{page_bg_image_url}",
                            oninput: move |e| page_bg_image_url.set(e.value()),
                        }
                        button {
                            class: "btn-add",
                            r#type: "button",
                            style: "background:transparent;border:1px solid var(--border);",
                            onclick: move |_| page_bg_image_url.set(String::new()),
                            "Clear"
                        }
                    }
                }
            }

            div { class: "form-section",
                h2 { "Recruitment" }
                div { class: "form-checkbox-row",
                    input {
                        r#type: "checkbox",
                        checked: recruitment_open(),
                        onchange: move |e| recruitment_open.set(e.checked()),
                    }
                    label { class: "form-label", "Recruitment Open" }
                }
                div { class: "form-field",
                    label { class: "form-label", "Message when closed" }
                    textarea {
                        class: "form-textarea",
                        value: "{recruitment_message}",
                        oninput: move |e| recruitment_message.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Minimum Age" }
                    input {
                        class: "form-input",
                        r#type: "number",
                        min: "0",
                        value: "{min_age}",
                        oninput: move |e| min_age.set(e.value()),
                    }
                }
            }

            div { class: "form-section",
                div { style: "display:flex;justify-content:space-between;align-items:center;flex-wrap:wrap;gap:0.75rem;margin-bottom:0.75rem;",
                    h2 { style: "margin:0;", "Homepage text" }
                    button {
                        class: "btn-add",
                        style: "background:transparent;border:1px solid var(--border);",
                        onclick: move |_| {
                            let d = HomepageContent::default();
                            content_align.set(d.content_align);
                            homepage.set(d);
                            toast.show(Toast::success("Reset to defaults — click Save to persist."));
                        },
                        "Reset defaults"
                    }
                }
                p { style: "color:var(--text-3);font-size:0.85rem;margin-bottom:1rem;",
                    "Edit public homepage copy without touching code. List fields: one item per line."
                }

                Fieldset { title: "Hero",
                    StrField { label: "Badge", value: homepage().hero_badge.clone(), on_change: move |v| homepage.write().hero_badge = v }
                    StrField { label: "Title", value: homepage().hero_title.clone(), on_change: move |v| homepage.write().hero_title = v }
                    StrField { label: "Title accent", value: homepage().hero_title_accent.clone(), on_change: move |v| homepage.write().hero_title_accent = v }
                    AreaField { label: "Subtitle", value: homepage().hero_sub.clone(), on_change: move |v| homepage.write().hero_sub = v }
                    StrField { label: "Primary CTA", value: homepage().cta_primary.clone(), on_change: move |v| homepage.write().cta_primary = v }
                    StrField { label: "Secondary CTA", value: homepage().cta_secondary.clone(), on_change: move |v| homepage.write().cta_secondary = v }
                }

                Fieldset { title: "Ethos",
                    StrField { label: "Kicker", value: homepage().ethos_kicker.clone(), on_change: move |v| homepage.write().ethos_kicker = v }
                    StrField { label: "Title", value: homepage().ethos_title.clone(), on_change: move |v| homepage.write().ethos_title = v }
                    AreaField { label: "Body", value: homepage().ethos_body.clone(), on_change: move |v| homepage.write().ethos_body = v }
                    LinesField { label: "Rules", value: homepage().ethos_rules.clone(), on_change: move |v| homepage.write().ethos_rules = v }
                }

                Fieldset { title: "Squads",
                    StrField { label: "Kicker", value: homepage().teams_kicker.clone(), on_change: move |v| homepage.write().teams_kicker = v }
                    StrField { label: "Title", value: homepage().teams_title.clone(), on_change: move |v| homepage.write().teams_title = v }
                    StrField { label: "Empty", value: homepage().teams_empty.clone(), on_change: move |v| homepage.write().teams_empty = v }
                }

                Fieldset { title: "Announcements",
                    StrField { label: "Kicker", value: homepage().news_kicker.clone(), on_change: move |v| homepage.write().news_kicker = v }
                    StrField { label: "Title", value: homepage().news_title.clone(), on_change: move |v| homepage.write().news_title = v }
                    StrField { label: "Empty", value: homepage().news_empty.clone(), on_change: move |v| homepage.write().news_empty = v }
                    StrField { label: "View all", value: homepage().news_view_all.clone(), on_change: move |v| homepage.write().news_view_all = v }
                }

                Fieldset { title: "Tournaments",
                    StrField { label: "Kicker", value: homepage().tournaments_kicker.clone(), on_change: move |v| homepage.write().tournaments_kicker = v }
                    StrField { label: "Title", value: homepage().tournaments_title.clone(), on_change: move |v| homepage.write().tournaments_title = v }
                    StrField { label: "Empty", value: homepage().tournaments_empty.clone(), on_change: move |v| homepage.write().tournaments_empty = v }
                    StrField { label: "View all", value: homepage().tournaments_view_all.clone(), on_change: move |v| homepage.write().tournaments_view_all = v }
                }

                Fieldset { title: "Schedule",
                    StrField { label: "Kicker", value: homepage().schedule_kicker.clone(), on_change: move |v| homepage.write().schedule_kicker = v }
                    StrField { label: "Title", value: homepage().schedule_title.clone(), on_change: move |v| homepage.write().schedule_title = v }
                    StrField { label: "Empty", value: homepage().schedule_empty.clone(), on_change: move |v| homepage.write().schedule_empty = v }
                    StrField { label: "Calendar CTA", value: homepage().calendar_cta.clone(), on_change: move |v| homepage.write().calendar_cta = v }
                }

                Fieldset { title: "Recruiting",
                    StrField { label: "Kicker", value: homepage().recruit_kicker.clone(), on_change: move |v| homepage.write().recruit_kicker = v }
                    StrField { label: "Title", value: homepage().recruit_title.clone(), on_change: move |v| homepage.write().recruit_title = v }
                    AreaField { label: "Body", value: homepage().recruit_body.clone(), on_change: move |v| homepage.write().recruit_body = v }
                    StrField { label: "CTA", value: homepage().recruit_cta.clone(), on_change: move |v| homepage.write().recruit_cta = v }
                    StrField { label: "Expectations title", value: homepage().recruit_expectations_title.clone(), on_change: move |v| homepage.write().recruit_expectations_title = v }
                    LinesField { label: "Expectations", value: homepage().recruit_expectations.clone(), on_change: move |v| homepage.write().recruit_expectations = v }
                    StrField { label: "Never-ask title", value: homepage().never_ask_title.clone(), on_change: move |v| homepage.write().never_ask_title = v }
                    AreaField { label: "Never-ask body", value: homepage().never_ask_body.clone(), on_change: move |v| homepage.write().never_ask_body = v }
                    StrField { label: "Seeking label", value: homepage().seeking_label.clone(), on_change: move |v| homepage.write().seeking_label = v }
                    LinesField { label: "Seeking tags", value: homepage().seeking_tags.clone(), on_change: move |v| homepage.write().seeking_tags = v }
                }

                Fieldset { title: "Footer",
                    StrField { label: "Footer note", value: homepage().footer_note.clone(), on_change: move |v| homepage.write().footer_note = v }
                }
            }

            div { class: "form-section",
                h2 { "Forum" }
                div { class: "form-field",
                    label { class: "form-label", "Forum Backend" }
                    select {
                        class: "form-input",
                        value: "{forum_backend}",
                        onchange: move |e| forum_backend.set(e.value()),
                        option { value: "local", "Local (SurrealDB)" }
                        option { value: "nostr", "Nostr (Relay)" }
                    }
                    p {
                        style: "font-size: 0.75rem; color: var(--text-3); margin-top: 0.35rem;",
                        "Controls where forum data is stored. \"Local\" uses the database. \"Nostr\" uses the relay (requires relay setup)."
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Extra Relay URLs" }
                    textarea {
                        class: "form-textarea",
                        placeholder: "ws://relay2:7777\nwss://relay.example.com",
                        value: "{extra_relay_urls}",
                        oninput: move |e| extra_relay_urls.set(e.value()),
                    }
                    p {
                        style: "font-size: 0.75rem; color: var(--text-3); margin-top: 0.35rem;",
                        "Additional relay URLs for multi-relay publishing (one per line). Events are published to the primary relay (NOSTR_RELAY_URL) and all extra relays."
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
        div { style: "margin-bottom:1rem;padding:0.85rem;border:1px solid var(--border);border-radius:8px;background:var(--surface);",
            h3 {
                style: "font-family:var(--font-mono);font-size:0.7rem;letter-spacing:0.1em;text-transform:uppercase;color:var(--text-3);margin:0 0 0.25rem;",
                "{title}"
            }
            p { style: "color:var(--text-3);font-size:0.78rem;margin:0 0 0.65rem;", "{hint}" }
            if items.is_empty() {
                p { style: "color:var(--text-3);font-size:0.85rem;margin:0;", "None" }
            } else {
                ul { style: "list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.4rem;",
                    for (id, label) in items {
                        li {
                            key: "{id}",
                            style: "display:flex;align-items:center;gap:0.5rem;flex-wrap:wrap;padding:0.4rem 0.55rem;border:1px solid var(--border);border-radius:6px;background:var(--surface-2);",
                            span { style: "flex:1;min-width:6rem;font-weight:500;", "{label}" }
                            span { style: "font-family:var(--font-mono);font-size:0.65rem;color:var(--text-3);", "{id}" }
                            select {
                                class: "form-input",
                                style: "width:auto;min-width:7rem;padding:0.25rem 0.4rem;font-size:0.8rem;",
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
fn Fieldset(title: &'static str, children: Element) -> Element {
    rsx! {
        div { style: "margin:1.25rem 0;padding-top:0.75rem;border-top:1px solid var(--border);",
            h3 {
                style: "font-family:var(--font-mono);font-size:0.7rem;letter-spacing:0.1em;text-transform:uppercase;color:var(--text-3);margin:0 0 0.75rem;",
                "{title}"
            }
            {children}
        }
    }
}

#[component]
fn StrField(label: &'static str, value: String, on_change: EventHandler<String>) -> Element {
    rsx! {
        div { class: "form-field", style: "margin-bottom:0.65rem;",
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
        div { class: "form-field", style: "margin-bottom:0.65rem;",
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
        div { class: "form-field", style: "margin-bottom:0.65rem;",
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
