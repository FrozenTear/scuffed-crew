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

use crate::components::admin_shared::focus_after_delete_soon;

use scuffed_api_client::ApiClient;
use scuffed_types::api::MyHeroIcons;

use crate::components::ui::InlineConfirm;
use crate::components::ui::inline_confirm::{
    focus_element_by_id, focus_element_by_id_soon, key_cancels,
};
use crate::hero_icons_ui::{self as text, OptOutChoice};
use crate::training_crops_ui::{self as crops, ConsentKind, CropConsent};

/// The section sits under the profile form as its own titled card
/// (`account-section` / `account-card` from profile_settings.rs). Switches are
/// 44px rows; the track and thumb are drawn from the spacing tokens.
const CSS: &str = r#"
    .training-crops__group { border: 0; margin: 0; padding: 0; min-width: 0;
        display: flex; flex-direction: column; gap: var(--space-3); }
    .training-crops__group legend { font-family: var(--font-head); font-size: var(--text-sm);
        font-weight: var(--font-label-weight); line-height: var(--leading-ui); color: var(--text);
        padding: 0; margin: 0 0 var(--space-2); }
    .training-crops__item { min-width: 0; display: flex; flex-direction: column; gap: var(--space-1); }
    .training-crops__name { min-width: 0; overflow-wrap: anywhere; }
    .hero-icons__switch { box-sizing: border-box; max-width: 100%; min-height: var(--hit-target-min);
        display: inline-flex; align-items: center; gap: var(--space-3);
        background: transparent; border: 0; padding: 0; margin: 0; color: var(--text);
        font-family: var(--font-body); font-size: var(--text-md); font-weight: var(--font-label-weight);
        line-height: var(--leading-body); cursor: pointer; text-align: left; }
    .hero-icons__switch:disabled { opacity: 0.6; cursor: progress; }
    .hero-icons__switch:focus-visible { outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset); border-radius: var(--radius-control); }
    .hero-icons__track { box-sizing: border-box; flex: none; position: relative;
        width: var(--space-10); height: var(--space-6); border-radius: var(--radius-pill);
        background: var(--surface-sunken); border: 1px solid var(--border-strong); }
    .hero-icons__thumb { position: absolute; top: 50%; left: var(--space-1);
        width: var(--space-4); height: var(--space-4); transform: translateY(-50%);
        border-radius: var(--radius-pill); background: var(--text-3); }
    .hero-icons__switch[aria-checked="true"] .hero-icons__track { background: var(--accent-solid);
        border-color: var(--accent-solid); }
    .hero-icons__switch[aria-checked="true"] .hero-icons__thumb {
        left: calc(100% - var(--space-4) - var(--space-1)); background: var(--on-accent); }
    .hero-icons__ask { margin: 0; padding: var(--space-4); border: 1px solid var(--border-strong);
        border-radius: var(--radius-control); display: flex; flex-direction: column; gap: var(--space-3);
        min-width: 0; }
    .hero-icons__ask legend { font-family: var(--font-head); font-size: var(--text-sm);
        font-weight: var(--font-label-weight); color: var(--text); padding: 0 var(--space-1); }
    .hero-icons__choice { display: flex; align-items: center; gap: var(--space-3);
        min-height: var(--hit-target-min); font-size: var(--text-md); line-height: var(--leading-body);
        color: var(--text); }
    .hero-icons__choice input { flex: 0 0 auto; margin: 0; accent-color: var(--accent-solid);
        width: calc(var(--space-4) + var(--space-1) / 2); height: calc(var(--space-4) + var(--space-1) / 2); }
    .hero-icons__sent { font-family: var(--font-head); font-size: var(--text-md);
        font-weight: var(--font-label-weight); line-height: var(--leading-ui); color: var(--text); }
    .hero-icons__grid { list-style: none; padding: 0; display: grid;
        grid-template-columns: repeat(auto-fill, minmax(calc(var(--space-10) * 2), 1fr)); gap: var(--space-2); }
    .hero-icons__tile { display: flex; flex-direction: column; align-items: center; gap: var(--space-1);
        background: var(--surface-sunken); border: 1px solid var(--border); border-radius: var(--radius-control);
        padding: var(--space-2); min-width: 0; }
    .hero-icons__tile img { width: calc(var(--space-12) + var(--space-4));
        height: calc(var(--space-12) + var(--space-4)); object-fit: contain; image-rendering: pixelated; }
    .hero-icons__tile span { font-size: var(--text-xs); color: var(--text-3); max-width: 100%;
        overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    /* Forced colours drop the fills, so the knob would vanish. It keeps
       system colours, and On also fills the track edge with Highlight, so
       the state shows by position and by colour. */
    @media (forced-colors: active) {
        .hero-icons__track { border-color: ButtonText; }
        .hero-icons__thumb { forced-color-adjust: none; background: ButtonText; }
        .hero-icons__switch[aria-checked="true"] .hero-icons__track {
            border-color: Highlight;
            border-width: var(--focus-ring-width);
        }
        .hero-icons__switch[aria-checked="true"] .hero-icons__thumb { background: Highlight; }
    }
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
            if saving() || busy() {
                return;
            }
            asking.set(false);
            focus_element_by_id(&target);
        }
    };

    let delete_prefix = text::delete_all_prefix(&member_id);
    let delete_trigger = format!("{delete_prefix}-trigger");
    let delete_prefix_for_focus = delete_prefix.clone();
    let delete_trigger_for_focus = delete_trigger.clone();
    let delete_all = move |_| {
        if busy() {
            return;
        }
        busy.set(true);
        let before = count;
        // B9: the confirm strip, and the focus in it, goes when the delete
        // ends. Focus comes back to Delete all my icons, which says there is
        // nothing left to delete, next to the status line.
        let strip_cancel = format!("{delete_prefix_for_focus}-cancel");
        let back_to = vec![delete_trigger_for_focus.clone()];
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
            focus_after_delete_soon(strip_cancel, back_to);
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
    let ask_go_id = format!("{question_id}-go");
    let ask_cancel_id = format!("{question_id}-cancel");
    let esc_target = hero_switch_id.clone();
    let list = mine.read().clone();

    let ask_busy = saving() || busy();
    let can_delete = move |count: u32| !(busy() || saving() || count == 0 || asking());

    rsx! {
        crate::styles::StyleOnce { id: super::profile_settings::ACCOUNT_CSS_ID, css: super::profile_settings::ACCOUNT_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-hero_icon_settings-css", css: CSS }

        section {
            id: "{section_id}",
            class: "account-section training-crops",
            "aria-labelledby": "{heading_id}",
            h2 { id: "{heading_id}", class: "ui-section-title", "{crops::HEADING}" }
            div { class: "account-card",
                p { class: "account-lead", "{crops::INTRO}" }
                p { class: "account-lead", "{crops::WHO_SEES}" }

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
                            p { id: "{crops::hint_id(&member_id, kind)}", class: "field-hint",
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
                        label { class: "hero-icons__choice", r#for: "{keep_id}",
                            input {
                                id: "{keep_id}",
                                r#type: "radio",
                                name: "{question_id}",
                                value: "keep",
                                checked: choice() == OptOutChoice::Keep,
                                onchange: move |_| choice.set(OptOutChoice::Keep),
                            }
                            "{text::KEEP_LABEL}"
                        }
                        label { class: "hero-icons__choice", r#for: "{delete_id}",
                            input {
                                id: "{delete_id}",
                                r#type: "radio",
                                name: "{question_id}",
                                value: "delete",
                                checked: choice() == OptOutChoice::Delete,
                                onchange: move |_| choice.set(OptOutChoice::Delete),
                            }
                            "{text::DELETE_LABEL}"
                        }
                        div { class: "account-actions",
                            button {
                                id: "{ask_go_id}",
                                class: "btn is-primary",
                                r#type: "button",
                                aria_disabled: if ask_busy { "true" },
                                onclick: move |_| {
                                    if !(saving() || busy()) {
                                        save_consent.call((ConsentKind::HeroIcon, false, choice()))
                                    }
                                },
                                "Turn off hero icons"
                            }
                            button {
                                id: "{ask_cancel_id}",
                                class: "btn is-ghost",
                                r#type: "button",
                                aria_disabled: if ask_busy { "true" },
                                onclick: cancel_ask,
                                "Cancel"
                            }
                        }
                    }
                }

                if consent_failed() && current.is_none() {
                    div { class: "account-actions",
                        button {
                            id: "{section_id}-consent-retry",
                            r#type: "button",
                            class: "btn is-ghost",
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
                    class: if consent_failed() { "account-status is-error" } else { "account-status" },
                    role: "status",
                    "{consent_status}"
                }

                if store_off() {
                    p { class: "field-hint", "{text::STORE_DISABLED}" }
                } else if let Some(list) = list {
                    if !hero_on && list.count > 0 {
                        p { class: "field-hint", "{text::KEPT_HIDDEN_NOTE}" }
                    }

                    h3 { class: "hero-icons__sent",
                        "Hero icons you have sent: {list.count} of {list.max_icons}"
                    }
                    if list.icons.is_empty() {
                        p { class: "field-hint", "{text::NONE_SENT}" }
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
                        div { class: "account-actions",
                            button {
                                id: delete_trigger.clone(),
                                r#type: "button",
                                class: "btn is-danger",
                                aria_disabled: if !can_delete(list.count) { "true" },
                                onclick: move |_| {
                                    if can_delete(count) {
                                        confirming_delete.set(true);
                                    }
                                },
                                "Delete all my icons"
                            }
                        }
                    }
                } else if load_failed() {
                    div { class: "account-actions",
                        button {
                            id: "{section_id}-retry",
                            r#type: "button",
                            class: "btn is-ghost",
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
                    class: if load_failed() { "account-status is-error" } else { "account-status" },
                    role: "status",
                    "{status}"
                }
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
    fn switch_state_survives_forced_colours() {
        // Vision nit 15 on PR 706: state was fill and knob only, and the
        // knob is a fill.
        use crate::components::ui::test_support::media_block;
        let forced = media_block(CSS, "@media (forced-colors: active)");
        assert!(
            forced.contains(
                ".hero-icons__thumb { forced-color-adjust: none; background: ButtonText; }"
            ),
            "{forced}"
        );
        assert!(
            forced.contains(".hero-icons__switch[aria-checked=\"true\"] .hero-icons__thumb { background: Highlight; }"),
            "{forced}"
        );
        assert!(forced.contains("border-color: Highlight;"), "{forced}");
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

    #[test]
    fn section_is_a_titled_account_card() {
        let html = render(section);
        assert!(
            html.contains("class=\"account-section training-crops\""),
            "{html}"
        );
        assert!(
            html.contains("<h2 id=\"training-crops-m1-heading\" class=\"ui-section-title\">"),
            "{html}"
        );
        assert!(html.contains("class=\"account-card\""), "{html}");
        assert!(!html.contains("<h1"), "the page owns the H1: {html}");
        assert!(!html.contains("profile-edit-hint"), "{html}");
    }

    #[test]
    fn css_uses_tokens_and_switches_are_44px() {
        use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};
        assert_sizes_from_tokens(CSS);
        let switch = css_rule(CSS, ".hero-icons__switch {");
        assert!(
            has_decl(&switch, "min-height: var(--hit-target-min)"),
            "{switch}"
        );
        let choice = css_rule(CSS, ".hero-icons__choice {");
        assert!(
            has_decl(&choice, "min-height: var(--hit-target-min)"),
            "{choice}"
        );
        assert!(!CSS.contains("999px"));
    }
}
