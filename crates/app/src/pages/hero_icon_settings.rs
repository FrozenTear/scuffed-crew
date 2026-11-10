//! Training crops section of Edit Profile.
//!
//! The site is the only place a member turns each training crop kind on or
//! off (`/api/training-crops/consent`). The tracker shows the choice read-only
//! and links here. Three switches: hero icons, stat digit crops and map name
//! banners. Hero icon consent is the old hero icon opt-in, so this section
//! also keeps the hero icon list: thumbnails of icons already sent and delete
//! all. Turning hero icons off while icons are stored asks what to do with
//! them. Keep is the default.

use dioxus::prelude::*;

use scuffed_api_client::ApiClient;
use scuffed_types::api::MyHeroIcons;

use crate::components::ui::inline_confirm::{
    focus_element_by_id, focus_element_by_id_soon, key_cancels,
};
use crate::components::ui::{BtnVariant, Button, InlineConfirm};
use crate::hero_icons_ui::{self as text, OptOutChoice};
use crate::training_crops_ui::{self as crops, ConsentKind, CropConsent};

const CSS: &str = r#"
    .training-crops__group { border: 0; margin: 0 0 var(--space-2); padding: 0; min-width: 0;
        display: flex; flex-direction: column; gap: var(--space-3); }
    .training-crops__group legend { font-size: var(--text-sm); font-weight: 600; color: var(--text);
        padding: 0; margin-bottom: var(--space-2); }
    .training-crops__item { min-width: 0; }
    .training-crops__item .profile-edit-hint { margin: var(--space-1) 0 0; }
    .training-crops__name { min-width: 0; overflow-wrap: anywhere; }
    .hero-icons__intro { color: var(--text-2); font-size: var(--text-sm); line-height: 1.5; margin: 0 0 var(--space-4); }
    .hero-icons__switch { max-width: 100%; display: inline-flex; align-items: center; gap: var(--space-3);
        background: transparent; border: 0; padding: var(--space-1) 0; color: var(--text);
        font-family: var(--font-body); font-size: var(--text-base); font-weight: 600; cursor: pointer;
        text-align: left; }
    .hero-icons__switch:disabled { opacity: .5; cursor: not-allowed; }
    .hero-icons__switch:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; border-radius: var(--radius-md); }
    .hero-icons__track { flex: none; width: 2.5rem; height: 1.4rem; border-radius: 999px;
        background: var(--surface-2); border: 1px solid var(--border); position: relative;
        transition: background .12s; }
    .hero-icons__thumb { position: absolute; top: 2px; left: 2px; width: calc(1.4rem - 6px);
        height: calc(1.4rem - 6px); border-radius: 50%; background: var(--text-2); transition: left .12s; }
    .hero-icons__switch[aria-checked="true"] .hero-icons__track { background: var(--accent); border-color: var(--accent); }
    .hero-icons__switch[aria-checked="true"] .hero-icons__thumb { left: calc(100% - 1.4rem + 4px); background: var(--accent-fg); }
    .hero-icons__ask { margin: var(--space-3) 0; padding: var(--space-3); border: 1px solid var(--border);
        border-radius: var(--radius-md); display: flex; flex-direction: column; gap: var(--space-2); }
    .hero-icons__ask legend { font-size: var(--text-sm); font-weight: 600; color: var(--text); padding: 0 var(--space-1); }
    .hero-icons__choice { display: flex; align-items: flex-start; gap: var(--space-2); font-size: var(--text-sm); color: var(--text); }
    .hero-icons__choice input { margin-top: .2rem; }
    .hero-icons__actions { display: flex; flex-wrap: wrap; gap: var(--space-2); justify-content: flex-end; }
    .hero-icons__sent { margin: var(--space-4) 0 var(--space-2); font-size: var(--text-sm); font-weight: 600; color: var(--text); }
    .hero-icons__grid { list-style: none; margin: 0 0 var(--space-4); padding: 0; display: grid;
        grid-template-columns: repeat(auto-fill, minmax(5rem, 1fr)); gap: var(--space-2); }
    .hero-icons__tile { display: flex; flex-direction: column; align-items: center; gap: var(--space-1);
        background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-md);
        padding: var(--space-2); min-width: 0; }
    .hero-icons__tile img { width: 4rem; height: 4rem; object-fit: contain; image-rendering: pixelated; }
    .hero-icons__tile span { font-size: var(--text-xs); color: var(--text-2); max-width: 100%;
        overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .hero-icons__status { min-height: 1.5em; margin: var(--space-3) 0 0; font-size: var(--text-sm); color: var(--text-2); }
    .hero-icons__retry { margin-top: var(--space-2); }
"#;

#[component]
pub fn TrainingCropSettings(member_id: String) -> Element {
    // Consent for the three kinds.
    let mut consent = use_signal(|| None::<CropConsent>);
    let mut consent_failed = use_signal(|| false);
    let mut consent_status = use_signal(|| crops::LOADING.to_string());
    let mut saving = use_signal(|| false);
    let mut consent_refresh = use_signal(|| 0u64);

    // Hero icons already sent.
    let mut mine = use_signal(|| None::<MyHeroIcons>);
    let mut store_off = use_signal(|| false);
    let mut load_failed = use_signal(|| false);
    let mut status = use_signal(|| text::LOADING.to_string());
    let mut busy = use_signal(|| false);
    let mut asking = use_signal(|| false);
    let mut choice = use_signal(OptOutChoice::default);
    let mut confirming_delete = use_signal(|| false);
    let mut refresh = use_signal(|| 0u64);

    use_resource(move || async move {
        let _generation = consent_refresh();
        match ApiClient::web()
            .fetch::<CropConsent>(crops::CONSENT_PATH)
            .await
        {
            Ok(loaded) => {
                consent.set(Some(loaded));
                consent_failed.set(false);
                consent_status.set(String::new());
            }
            Err(e) => {
                consent_failed.set(true);
                consent_status.set(crops::consent_failure_text("Loading your choices", &e));
            }
        }
    });

    use_resource(move || async move {
        let _generation = refresh();
        match ApiClient::web()
            .fetch::<MyHeroIcons>("/api/hero-icons/mine")
            .await
        {
            Ok(list) => {
                store_off.set(false);
                load_failed.set(false);
                if mine.peek().is_none() {
                    status.set(String::new());
                }
                mine.set(Some(list));
            }
            Err(e) if text::is_store_disabled(&e) => {
                store_off.set(true);
                load_failed.set(false);
                status.set(String::new());
            }
            Err(e) => {
                load_failed.set(true);
                status.set(text::failure_text("Loading your hero icons", &e));
            }
        }
    });

    let current = consent();
    // Hero icon consent is the old opt-in. Use the list's copy until consent loads.
    let hero_on = current.map_or_else(
        || {
            mine.read()
                .as_ref()
                .is_some_and(|m| m.icon_contribution_opt_in)
        },
        |c| c.hero_icon,
    );
    let count = mine.read().as_ref().map_or(0, |m| m.count);
    let switches_disabled = current.is_none() || saving();

    // Show the change at once, send only the kind that changed (the server
    // keeps the others, so a stale tab cannot turn a kind back on), then take
    // all three values from the server's answer. On failure the switch goes
    // back.
    let save_consent = {
        let member_id = member_id.clone();
        use_callback(
            move |(kind, on, choice_made): (ConsentKind, bool, OptOutChoice)| {
                let Some(before) = *consent.peek() else {
                    return;
                };
                let member_id = member_id.clone();
                let next = before.with(kind, on);
                consent.set(Some(next));
                saving.set(true);
                consent_status.set(crops::saving_status(kind));
                spawn(async move {
                    let saved = ApiClient::web()
                        .put_json::<_, CropConsent>(
                            crops::CONSENT_PATH,
                            &crops::consent_update(kind, on),
                        )
                        .await;
                    match saved {
                        Ok(saved) => {
                            consent.set(Some(saved));
                            let now_on = saved.get(kind);
                            let mut detail = None;
                            if kind == ConsentKind::HeroIcon {
                                mine.with_mut(|m| {
                                    if let Some(m) = m {
                                        m.icon_contribution_opt_in = now_on;
                                    }
                                });
                                let stored = mine.peek().as_ref().map_or(0, |m| m.count);
                                if now_on {
                                    // Nothing stored changes when sharing turns on.
                                } else if choice_made == OptOutChoice::Delete && stored > 0 {
                                    match ApiClient::web().delete("/api/hero-icons/mine").await {
                                        Ok(()) => {
                                            mine.with_mut(|m| {
                                                if let Some(m) = m {
                                                    m.count = 0;
                                                    m.icons.clear();
                                                }
                                            });
                                            detail =
                                                text::opt_out_detail(OptOutChoice::Delete, stored);
                                        }
                                        Err(e) => {
                                            detail = Some(format!(
                                                "{} Use Delete all my icons to try again.",
                                                text::failure_text("Deleting your icons", &e)
                                            ))
                                        }
                                    }
                                    refresh += 1;
                                } else {
                                    detail = text::opt_out_detail(OptOutChoice::Keep, stored);
                                }
                            }
                            consent_status.set(crops::saved_status(
                                kind,
                                now_on,
                                detail.as_deref(),
                            ));
                        }
                        Err(e) => {
                            consent.set(Some(before));
                            let what = format!("Saving {}", kind.noun().to_lowercase());
                            consent_status.set(crops::failed_status(
                                kind,
                                &crops::consent_failure_text(&what, &e),
                            ));
                        }
                    }
                    if kind == ConsentKind::HeroIcon {
                        asking.set(false);
                    }
                    saving.set(false);
                    focus_element_by_id_soon(crops::switch_id(&member_id, kind));
                });
            },
        )
    };

    let on_switch = {
        let member_id = member_id.clone();
        use_callback(move |kind: ConsentKind| {
            let Some(now) = *consent.peek() else {
                return;
            };
            let on = now.get(kind);
            if kind == ConsentKind::HeroIcon {
                confirming_delete.set(false);
                let stored = mine.peek().as_ref().map_or(0, |m| m.count);
                if on && text::opt_out_needs_question(stored) {
                    choice.set(OptOutChoice::Keep);
                    asking.set(true);
                    focus_element_by_id_soon(text::choice_id(&member_id, OptOutChoice::Keep));
                    return;
                }
            }
            save_consent.call((kind, !on, OptOutChoice::Keep));
        })
    };

    let hero_switch_id = crops::switch_id(&member_id, ConsentKind::HeroIcon);

    let cancel_ask = {
        let target = hero_switch_id.clone();
        move |_| {
            asking.set(false);
            focus_element_by_id(&target);
        }
    };

    let delete_all = move |_| {
        busy.set(true);
        let before = count;
        spawn(async move {
            match ApiClient::web().delete("/api/hero-icons/mine").await {
                Ok(()) => {
                    mine.with_mut(|m| {
                        if let Some(m) = m {
                            m.count = 0;
                            m.icons.clear();
                        }
                    });
                    status.set(text::status_deleted_all(before));
                }
                Err(e) => status.set(text::failure_text("Deleting your icons", &e)),
            }
            confirming_delete.set(false);
            busy.set(false);
            refresh += 1;
        });
    };

    let section_id = crops::section_id(&member_id);
    let heading_id = format!("{section_id}-heading");
    let consent_status_id = crops::consent_status_id(&member_id);
    let status_id = text::status_id(&member_id);
    let question_id = text::question_id(&member_id);
    let keep_id = text::choice_id(&member_id, OptOutChoice::Keep);
    let delete_id = text::choice_id(&member_id, OptOutChoice::Delete);
    let delete_prefix = text::delete_all_prefix(&member_id);
    let delete_trigger = format!("{delete_prefix}-trigger");
    let ask_go_id = format!("{question_id}-go");
    let ask_cancel_id = format!("{question_id}-cancel");
    let esc_target = hero_switch_id.clone();
    let list = mine.read().clone();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-hero_icon_settings-css", css: CSS }

        section {
            id: "{section_id}",
            class: "profile-edit-section",
            "aria-labelledby": "{heading_id}",
            h2 { id: "{heading_id}", "{crops::HEADING}" }
            p { class: "hero-icons__intro", "{crops::INTRO}" }
            p { class: "hero-icons__intro", "{crops::WHO_SEES}" }

            fieldset { class: "training-crops__group",
                legend { "{crops::GROUP_LEGEND}" }
                for kind in ConsentKind::ALL {
                    div { class: "training-crops__item", key: "{kind.key()}",
                        button {
                            id: "{crops::switch_id(&member_id, kind)}",
                            class: "hero-icons__switch",
                            r#type: "button",
                            role: "switch",
                            "aria-checked": if current.is_some_and(|c| c.get(kind)) { "true" } else { "false" },
                            "aria-labelledby": "{crops::label_id(&member_id, kind)}",
                            "aria-describedby": "{crops::hint_id(&member_id, kind)}",
                            disabled: switches_disabled,
                            onclick: move |_| on_switch.call(kind),
                            span { class: "hero-icons__track", "aria-hidden": "true",
                                span { class: "hero-icons__thumb" }
                            }
                            span {
                                id: "{crops::label_id(&member_id, kind)}",
                                class: "training-crops__name",
                                "{kind.label()}"
                            }
                        }
                        p { id: "{crops::hint_id(&member_id, kind)}", class: "profile-edit-hint",
                            "{kind.hint()}"
                        }
                    }
                }
            }

            if asking() {
                fieldset {
                    id: "{question_id}",
                    class: "hero-icons__ask",
                    onkeydown: move |e| {
                        if key_cancels(&e.key()) {
                            e.stop_propagation();
                            asking.set(false);
                            focus_element_by_id(&esc_target);
                        }
                    },
                    legend { "{text::opt_out_question(count)}" }
                    div { class: "hero-icons__choice",
                        input {
                            id: "{keep_id}",
                            r#type: "radio",
                            name: "{question_id}",
                            value: "keep",
                            checked: choice() == OptOutChoice::Keep,
                            onchange: move |_| choice.set(OptOutChoice::Keep),
                        }
                        label { r#for: "{keep_id}", "{text::KEEP_LABEL}" }
                    }
                    div { class: "hero-icons__choice",
                        input {
                            id: "{delete_id}",
                            r#type: "radio",
                            name: "{question_id}",
                            value: "delete",
                            checked: choice() == OptOutChoice::Delete,
                            onchange: move |_| choice.set(OptOutChoice::Delete),
                        }
                        label { r#for: "{delete_id}", "{text::DELETE_LABEL}" }
                    }
                    div { class: "hero-icons__actions",
                        button {
                            id: "{ask_cancel_id}",
                            class: "inline-confirm__cancel",
                            r#type: "button",
                            disabled: saving() || busy(),
                            onclick: cancel_ask,
                            "Cancel"
                        }
                        button {
                            id: "{ask_go_id}",
                            class: "inline-confirm__go",
                            r#type: "button",
                            disabled: saving() || busy(),
                            onclick: move |_| {
                                save_consent.call((ConsentKind::HeroIcon, false, choice()))
                            },
                            "Turn off hero icons"
                        }
                    }
                }
            }

            if consent_failed() && current.is_none() {
                div { class: "hero-icons__retry",
                    Button {
                        id: "{section_id}-consent-retry",
                        variant: BtnVariant::Ghost,
                        onclick: move |_| {
                            consent_status.set(crops::LOADING.to_string());
                            consent_refresh += 1;
                        },
                        "Retry loading your choices"
                    }
                }
            }

            p {
                id: "{consent_status_id}",
                class: "hero-icons__status",
                role: "status",
                "{consent_status}"
            }

            if store_off() {
                p { class: "profile-edit-hint", "{text::STORE_DISABLED}" }
            } else if let Some(list) = list {
                if !hero_on && list.count > 0 {
                    p { class: "profile-edit-hint", "{text::KEPT_HIDDEN_NOTE}" }
                }

                h3 { class: "hero-icons__sent",
                    "Hero icons you have sent: {list.count} of {list.max_icons}"
                }
                if list.icons.is_empty() {
                    p { class: "profile-edit-hint", "{text::NONE_SENT}" }
                } else {
                    ul { class: "hero-icons__grid",
                        for icon in list.icons.iter() {
                            li { class: "hero-icons__tile", key: "{icon.id}",
                                img {
                                    id: "{text::thumb_id(&icon.id)}",
                                    src: "{icon.thumbnail_url}",
                                    alt: "{text::icon_alt(&icon.hero, &icon.uploaded_at)}",
                                    width: "64",
                                    height: "64",
                                    loading: "lazy",
                                }
                                span { "aria-hidden": "true", "{icon.hero}" }
                            }
                        }
                    }
                }

                if confirming_delete() {
                    InlineConfirm {
                        id: delete_prefix.clone(),
                        title: text::delete_all_title(list.count),
                        body: text::DELETE_ALL_BODY.to_string(),
                        confirm_label: "Delete all".to_string(),
                        busy: busy(),
                        on_confirm: delete_all,
                        on_cancel: {
                            let trigger = delete_trigger.clone();
                            move |_| {
                                confirming_delete.set(false);
                                text::focus_when_rendered(trigger.clone());
                            }
                        },
                    }
                } else {
                    Button {
                        id: delete_trigger.clone(),
                        variant: BtnVariant::Danger,
                        disabled: busy() || saving() || list.count == 0 || asking(),
                        onclick: move |_| confirming_delete.set(true),
                        "Delete all my icons"
                    }
                }
            } else if load_failed() {
                div { class: "hero-icons__retry",
                    Button {
                        id: "{section_id}-retry",
                        variant: BtnVariant::Ghost,
                        onclick: move |_| {
                            status.set(text::LOADING.to_string());
                            refresh += 1;
                        },
                        "Retry loading hero icons"
                    }
                }
            }

            p {
                id: "{status_id}",
                class: "hero-icons__status",
                role: "status",
                "{status}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn section() -> Element {
        rsx! { TrainingCropSettings { member_id: "m1".to_string() } }
    }

    #[test]
    fn first_render_has_both_live_regions_with_loading_text() {
        let html = render(section);
        assert!(
            html.contains("id=\"training-crops-m1-consent-status\""),
            "{html}"
        );
        assert!(html.contains("id=\"hero-icons-m1-status\""), "{html}");
        assert_eq!(html.matches("role=\"status\"").count(), 2, "{html}");
        assert!(html.contains(crops::LOADING), "{html}");
        assert!(html.contains(text::LOADING), "{html}");
    }

    #[test]
    fn section_is_named_by_its_heading_and_carries_the_member_id() {
        let html = render(section);
        assert!(html.contains("id=\"training-crops-m1\""), "{html}");
        assert!(
            html.contains("aria-labelledby=\"training-crops-m1-heading\""),
            "{html}"
        );
        assert!(html.contains("Training crops"), "{html}");
        assert!(html.contains("never whole screens"), "{html}");
    }

    #[test]
    fn three_switches_named_by_their_own_labels() {
        let html = render(section);
        assert_eq!(html.matches("role=\"switch\"").count(), 3, "{html}");
        for kind in ConsentKind::ALL {
            let switch = crops::switch_id("m1", kind);
            let label = crops::label_id("m1", kind);
            let hint = crops::hint_id("m1", kind);
            assert!(html.contains(&format!("id=\"{switch}\"")), "{html}");
            assert!(
                html.contains(&format!("aria-labelledby=\"{label}\"")),
                "{html}"
            );
            assert!(html.contains(&format!("id=\"{label}\"")), "{html}");
            assert!(
                html.contains(&format!("aria-describedby=\"{hint}\"")),
                "{html}"
            );
            assert!(html.contains(kind.label()), "{html}");
        }
        assert!(!html.contains("full_screen"), "{html}");
        assert!(!html.to_lowercase().contains("full screen"), "{html}");
    }

    #[test]
    fn switches_are_off_and_disabled_while_loading() {
        let html = render(section);
        assert_eq!(html.matches("aria-checked=\"false\"").count(), 3, "{html}");
        assert!(!html.contains("aria-checked=\"true\""), "{html}");
        for kind in ConsentKind::ALL {
            let switch = crops::switch_id("m1", kind);
            let start = html.find(&format!("id=\"{switch}\"")).unwrap();
            let tag_start = html[..start].rfind('<').unwrap();
            let tag_end = start + html[start..].find('>').unwrap();
            let tag = &html[tag_start..tag_end];
            assert!(tag.contains("disabled"), "{tag}");
        }
    }

    #[test]
    fn nothing_is_a_button_inside_a_link() {
        let html = render(section);
        assert!(!html.contains("<a "), "{html}");
    }
}
