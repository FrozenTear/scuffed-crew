//! Hero icon sharing section of Edit Profile.
//!
//! A member chooses whether the tracker may send hero icons for tracker
//! training, sees thumbnails of icons already sent, and can delete them. Turning
//! sharing off asks what to do with icons already sent. Keep is the default.
//! Icons are private. Officers see only a count.

use dioxus::prelude::*;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{HeroIconOptIn, MyHeroIcons};

use crate::components::ui::inline_confirm::{
    focus_element_by_id, focus_element_by_id_soon, key_cancels,
};
use crate::components::ui::{BtnVariant, Button, InlineConfirm};
use crate::hero_icons_ui::{self as text, OptOutChoice};

const CSS: &str = r#"
    .hero-icons__intro { color: var(--text-2); font-size: var(--text-sm); line-height: 1.5; margin: 0 0 var(--space-4); }
    .hero-icons__switch { display: inline-flex; align-items: center; gap: var(--space-3);
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
pub fn HeroIconSettings(member_id: String) -> Element {
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

    let opted_in = mine
        .read()
        .as_ref()
        .is_some_and(|m| m.icon_contribution_opt_in);
    let count = mine.read().as_ref().map_or(0, |m| m.count);

    // Flip the flag on the server, then mirror it locally.
    let set_opt_in = {
        let member_id = member_id.clone();
        use_callback(move |(on, choice_made): (bool, OptOutChoice)| {
            let member_id = member_id.clone();
            busy.set(true);
            spawn(async move {
                let body = HeroIconOptIn {
                    icon_contribution_opt_in: on,
                };
                let saved = ApiClient::web()
                    .put_json::<_, HeroIconOptIn>("/api/hero-icons/opt-in", &body)
                    .await;
                match saved {
                    Ok(saved) => {
                        let on = saved.icon_contribution_opt_in;
                        mine.with_mut(|m| {
                            if let Some(m) = m {
                                m.icon_contribution_opt_in = on;
                            }
                        });
                        if on {
                            status.set(text::status_opted_in());
                        } else if choice_made == OptOutChoice::Delete && count > 0 {
                            match ApiClient::web().delete("/api/hero-icons/mine").await {
                                Ok(()) => {
                                    mine.with_mut(|m| {
                                        if let Some(m) = m {
                                            m.count = 0;
                                            m.icons.clear();
                                        }
                                    });
                                    status.set(text::status_opted_out(OptOutChoice::Delete, count));
                                }
                                Err(e) => status.set(format!(
                                    "Sharing is off. {} Use Delete all my icons to try again.",
                                    text::failure_text("Deleting your icons", &e)
                                )),
                            }
                            refresh += 1;
                        } else {
                            status.set(text::status_opted_out(OptOutChoice::Keep, count));
                        }
                    }
                    Err(e) => status.set(text::failure_text("Saving your choice", &e)),
                }
                asking.set(false);
                busy.set(false);
                focus_element_by_id_soon(text::switch_id(&member_id));
            });
        })
    };

    let on_switch = {
        let member_id = member_id.clone();
        move |_| {
            confirming_delete.set(false);
            if !opted_in {
                set_opt_in.call((true, OptOutChoice::Keep));
            } else if text::opt_out_needs_question(count) {
                choice.set(OptOutChoice::Keep);
                asking.set(true);
                focus_element_by_id_soon(text::choice_id(&member_id, OptOutChoice::Keep));
            } else {
                set_opt_in.call((false, OptOutChoice::Keep));
            }
        }
    };

    let cancel_ask = {
        let member_id = member_id.clone();
        move |_| {
            asking.set(false);
            focus_element_by_id(&text::switch_id(&member_id));
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

    let heading_id = format!("{}-heading", text::section_id(&member_id));
    let switch_id = text::switch_id(&member_id);
    let hint_id = text::switch_hint_id(&member_id);
    let status_id = text::status_id(&member_id);
    let question_id = text::question_id(&member_id);
    let keep_id = text::choice_id(&member_id, OptOutChoice::Keep);
    let delete_id = text::choice_id(&member_id, OptOutChoice::Delete);
    let delete_prefix = text::delete_all_prefix(&member_id);
    let delete_trigger = format!("{delete_prefix}-trigger");
    let ask_go_id = format!("{question_id}-go");
    let ask_cancel_id = format!("{question_id}-cancel");
    let esc_target = switch_id.clone();
    let list = mine.read().clone();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-hero_icon_settings-css", css: CSS }

        section {
            id: "{text::section_id(&member_id)}",
            class: "profile-edit-section",
            "aria-labelledby": "{heading_id}",
            h2 { id: "{heading_id}", "Hero icons" }
            p { class: "hero-icons__intro", "{text::SECTION_INTRO}" }

            if store_off() {
                p { class: "profile-edit-hint", "{text::STORE_DISABLED}" }
            } else if let Some(list) = list {
                button {
                    id: "{switch_id}",
                    class: "hero-icons__switch",
                    r#type: "button",
                    role: "switch",
                    "aria-checked": if opted_in { "true" } else { "false" },
                    "aria-describedby": "{hint_id}",
                    disabled: busy(),
                    onclick: on_switch,
                    span { class: "hero-icons__track", "aria-hidden": "true",
                        span { class: "hero-icons__thumb" }
                    }
                    span { "{text::SWITCH_LABEL}" }
                }
                p { id: "{hint_id}", class: "profile-edit-hint",
                    if opted_in { "{text::SWITCH_HINT_ON}" } else { "{text::SWITCH_HINT_OFF}" }
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
                                disabled: busy(),
                                onclick: cancel_ask,
                                "Cancel"
                            }
                            button {
                                id: "{ask_go_id}",
                                class: "inline-confirm__go",
                                r#type: "button",
                                disabled: busy(),
                                onclick: move |_| set_opt_in.call((false, choice())),
                                "Turn off sharing"
                            }
                        }
                    }
                }

                if !opted_in && list.count > 0 {
                    p { class: "profile-edit-hint", "{text::KEPT_HIDDEN_NOTE}" }
                }

                h3 { class: "hero-icons__sent",
                    "Icons you have sent: {list.count} of {list.max_icons}"
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
                        disabled: busy() || list.count == 0 || asking(),
                        onclick: move |_| confirming_delete.set(true),
                        "Delete all my icons"
                    }
                }
            } else if load_failed() {
                div { class: "hero-icons__retry",
                    Button {
                        id: "{text::section_id(&member_id)}-retry",
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
        rsx! { HeroIconSettings { member_id: "m1".to_string() } }
    }

    #[test]
    fn first_render_has_the_live_region_with_loading_text() {
        let html = render(section);
        assert!(html.contains("id=\"hero-icons-m1-status\""), "{html}");
        assert!(html.contains("role=\"status\""), "{html}");
        assert!(html.contains(text::LOADING), "{html}");
    }

    #[test]
    fn section_is_named_by_its_heading_and_carries_the_member_id() {
        let html = render(section);
        assert!(html.contains("id=\"hero-icons-m1\""), "{html}");
        assert!(
            html.contains("aria-labelledby=\"hero-icons-m1-heading\""),
            "{html}"
        );
        assert!(html.contains("Hero icons"), "{html}");
        assert!(html.contains("private training data"), "{html}");
    }

    #[test]
    fn nothing_is_a_button_inside_a_link() {
        let html = render(section);
        assert!(!html.contains("<a "), "{html}");
    }
}
