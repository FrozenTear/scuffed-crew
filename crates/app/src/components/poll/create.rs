use dioxus::prelude::*;
use serde::Serialize;

use crate::components::ui::FieldError;
use crate::components::ui::inline_confirm::{focus_element_by_id_soon, focus_field_by_id};
use crate::components::{Toast, use_toast};
use scuffed_api_client::ApiClient;

#[derive(Serialize)]
struct CreatePollBody {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    options: Vec<String>,
    #[serde(default)]
    allow_multiple: bool,
}

pub(crate) const CREATE_CSS: &str = r#"
.poll-create {
  box-sizing: border-box;
  min-width: 0;
  margin: 0;
  padding: var(--space-section);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.poll-create-title { margin-bottom: var(--space-4); }
.poll-create-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
  margin: 0;
  padding: 0;
  border: 0;
}
.poll-create-field + .poll-create-field,
.poll-create-field + .poll-create-check { margin-top: var(--space-5); }
.poll-create-field > legend {
  padding: 0;
  margin-bottom: var(--space-2);
}
.poll-create-options {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.poll-create-option-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.poll-create-option-row .field-input {
  flex: 1 1 auto;
  min-width: 0;
}
.poll-create-option-row .btn { flex: 0 0 auto; }
.poll-create-add { align-self: flex-start; }
.poll-create-check {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-height: var(--hit-target-min);
  color: var(--text);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-ui);
  cursor: pointer;
}
.poll-create-check input {
  flex: 0 0 auto;
  width: var(--space-5);
  height: var(--space-5);
  margin: 0;
  accent-color: var(--accent-solid);
}
.poll-create-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin-top: var(--space-5);
}
"#;

const TITLE_ID: &str = "poll-create-title-input";
const TITLE_ERROR_ID: &str = "poll-create-title-err";
const DESCRIPTION_ID: &str = "poll-create-description";
const OPTIONS_ERROR_ID: &str = "poll-create-options-err";

/// DOM id of an option input. The form exists once per page, so the place of
/// the option in it is enough.
fn option_input_id(index: usize) -> String {
    format!("poll-create-option-{index}")
}

/// What is wrong with the form, as the last submit found it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FormErrors {
    title: bool,
    options: bool,
}

/// The options that have text, trimmed.
fn filled_options(options: &[String]) -> Vec<String> {
    options
        .iter()
        .map(|o| o.trim().to_string())
        .filter(|o| !o.is_empty())
        .collect()
}

/// A poll needs a question and at least two options.
fn form_errors(title: &str, options: &[String]) -> FormErrors {
    FormErrors {
        title: title.trim().is_empty(),
        options: filled_options(options).len() < 2,
    }
}

/// The poll form for officers. The submit button is never natively
/// disabled: a missing question or option gets an error line and focus.
#[component]
pub fn PollCreate(on_created: EventHandler<()>) -> Element {
    let mut title = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut options = use_signal(|| vec!["".to_string(), "".to_string()]);
    let mut allow_multiple = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    // Result of the last submit. Clean until one fails.
    let mut errors = use_signal(FormErrors::default);
    let mut toasts = use_toast();

    let invalid = errors();
    let title_error = if invalid.title {
        "Enter a question."
    } else {
        ""
    };
    let options_error = if invalid.options {
        "Add at least two options."
    } else {
        ""
    };
    let option_count = options().len();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-poll-create-create-css", css: CREATE_CSS }
        form {
            class: "poll-create",
            novalidate: true,
            aria_labelledby: "poll-create-title",
            onsubmit: move |e: FormEvent| {
                e.prevent_default();
                if submitting() {
                    return;
                }
                let t = title().trim().to_string();
                let opts = filled_options(&options());
                let found = form_errors(&t, &options());
                errors.set(found);
                if found.title {
                    focus_field_by_id(TITLE_ID, ".poll-create-field");
                    return;
                }
                if found.options {
                    // The first option that is still empty.
                    let empty = options()
                        .iter()
                        .position(|o| o.trim().is_empty())
                        .unwrap_or(0);
                    focus_field_by_id(&option_input_id(empty), ".poll-create-field");
                    return;
                }

                let desc = {
                    let d = description().trim().to_string();
                    if d.is_empty() { None } else { Some(d) }
                };
                let multi = allow_multiple();

                submitting.set(true);
                spawn(async move {
                    let body = CreatePollBody {
                        title: t,
                        description: desc,
                        options: opts,
                        allow_multiple: multi,
                    };
                    match ApiClient::web()
                        .post_json::<_, serde_json::Value>("/api/polls", &body)
                        .await
                    {
                        Ok(_) => {
                            toasts.show(Toast::success("Poll created"));
                            title.set(String::new());
                            description.set(String::new());
                            options.set(vec!["".to_string(), "".to_string()]);
                            allow_multiple.set(false);
                            on_created.call(());
                        }
                        Err(e) => {
                            toasts.show(Toast::error_from("Failed to create poll", &e));
                        }
                    }
                    submitting.set(false);
                });
            },
            h2 { class: "ui-section-title poll-create-title", id: "poll-create-title", "Create a poll" }

            div { class: "poll-create-field",
                label { class: "field-label", r#for: TITLE_ID, "Question" }
                input {
                    class: "field-input",
                    id: TITLE_ID,
                    name: "poll-title",
                    r#type: "text",
                    required: true,
                    aria_required: "true",
                    aria_invalid: if invalid.title { "true" },
                    aria_describedby: if invalid.title { TITLE_ERROR_ID },
                    value: "{title}",
                    oninput: move |e| title.set(e.value()),
                }
                FieldError {
                    id: TITLE_ERROR_ID,
                    text: title_error.to_string(),
                    role: "alert",
                }
            }

            div { class: "poll-create-field",
                label { class: "field-label", r#for: DESCRIPTION_ID, "Description (optional)" }
                input {
                    class: "field-input",
                    id: DESCRIPTION_ID,
                    name: "poll-description",
                    r#type: "text",
                    value: "{description}",
                    oninput: move |e| description.set(e.value()),
                }
            }

            fieldset {
                class: "poll-create-field",
                aria_describedby: if invalid.options { OPTIONS_ERROR_ID },
                legend { class: "field-label", "Options" }
                ul { class: "poll-create-options",
                    for i in 0..option_count {
                        li { key: "{i}", class: "poll-create-option-row",
                            input {
                                class: "field-input",
                                id: option_input_id(i),
                                name: "poll-option-{i}",
                                r#type: "text",
                                aria_label: "Option {i + 1}",
                                placeholder: "Option {i + 1}",
                                aria_invalid: if invalid.options && options()[i].trim().is_empty() { "true" },
                                value: "{options()[i]}",
                                oninput: move |e: Event<FormData>| {
                                    options.write()[i] = e.value();
                                },
                            }
                            if option_count > 2 {
                                button {
                                    r#type: "button",
                                    class: "btn is-ghost",
                                    aria_label: "Remove option {i + 1}",
                                    onclick: move |_| {
                                        options.write().remove(i);
                                        // This button is gone or renamed:
                                        // keep focus in the list.
                                        focus_element_by_id_soon(option_input_id(i.saturating_sub(1)));
                                    },
                                    "Remove"
                                }
                            }
                        }
                    }
                }
                FieldError {
                    id: OPTIONS_ERROR_ID,
                    text: options_error.to_string(),
                    role: "alert",
                }
                button {
                    r#type: "button",
                    class: "btn is-ghost poll-create-add",
                    onclick: move |_| {
                        let next = options().len();
                        options.write().push(String::new());
                        focus_element_by_id_soon(option_input_id(next));
                    },
                    "Add option"
                }
            }

            label { class: "poll-create-check",
                input {
                    r#type: "checkbox",
                    name: "poll-allow-multiple",
                    checked: allow_multiple(),
                    onchange: move |e: Event<FormData>| {
                        allow_multiple.set(e.checked());
                    },
                }
                "Allow multiple selections"
            }

            div { class: "poll-create-actions",
                button {
                    r#type: "submit",
                    class: "btn is-primary",
                    aria_busy: if submitting() { "true" },
                    aria_disabled: if submitting() { "true" },
                    if submitting() { "Creating..." } else { "Create poll" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ToastProvider;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

    fn app() -> Element {
        rsx! {
            ToastProvider {
                PollCreate { on_created: move |_| {} }
            }
        }
    }

    fn render() -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn every_control_has_a_name_and_ids_are_unique() {
        let html = render();
        assert!(html.contains("<form class=\"poll-create\""), "{html}");
        assert!(
            html.contains(
                "<h2 class=\"ui-section-title poll-create-title\" id=\"poll-create-title\">Create a poll</h2>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<label class=\"field-label\" for=\"poll-create-title-input\">Question</label>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<label class=\"field-label\" for=\"poll-create-description\">Description (optional)</label>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<legend class=\"field-label\">Options</legend>"),
            "{html}"
        );
        for n in 1..=2 {
            assert!(
                html.contains(&format!("aria-label=\"Option {n}\"")),
                "{html}"
            );
        }
        for id in [
            "poll-create-title",
            "poll-create-title-input",
            "poll-create-description",
            "poll-create-option-0",
            "poll-create-option-1",
            "poll-create-title-err",
            "poll-create-options-err",
        ] {
            assert_eq!(html.matches(&format!("id=\"{id}\"")).count(), 1, "{id}");
        }
        assert!(html.contains("Allow multiple selections</label>"), "{html}");
        assert!(html.contains(">Add option</button>"), "{html}");
        assert!(
            html.contains("<button type=\"submit\" class=\"btn is-primary\">Create poll</button>"),
            "{html}"
        );
        // Two options is the minimum, so there is nothing to remove yet.
        assert!(!html.contains("Remove option"), "{html}");
        // Never natively disabled, and no error before the first submit.
        assert!(!html.contains(" disabled"), "{html}");
        assert!(!html.contains("aria-invalid"), "{html}");
        for id in ["poll-create-title-err", "poll-create-options-err"] {
            assert!(
                html.contains(&format!(
                    "<p class=\"field-error\" id=\"{id}\" role=\"alert\"></p>"
                )),
                "{id}: {html}"
            );
        }
        assert!(!html.contains("<h1"), "{html}");
        assert!(!html.contains("<main"), "{html}");
    }

    #[test]
    fn a_poll_needs_a_question_and_two_options() {
        let opts = |list: &[&str]| list.iter().map(|o| o.to_string()).collect::<Vec<_>>();
        assert_eq!(
            form_errors(" ", &opts(&["", ""])),
            FormErrors {
                title: true,
                options: true
            }
        );
        assert_eq!(
            form_errors("When?", &opts(&["19:00", " "])),
            FormErrors {
                title: false,
                options: true
            }
        );
        assert_eq!(
            form_errors("When?", &opts(&["19:00", "", "20:00"])),
            FormErrors::default()
        );
        assert_eq!(
            filled_options(&opts(&[" 19:00 ", "", "20:00"])),
            opts(&["19:00", "20:00"])
        );
    }

    #[test]
    fn css_is_on_tokens_with_a_44px_checkbox_row() {
        assert_sizes_from_tokens(CREATE_CSS);
        let check = css_rule(CREATE_CSS, "\n.poll-create-check {");
        assert!(
            has_decl(&check, "min-height: var(--hit-target-min)"),
            "{check}"
        );
        let card = css_rule(CREATE_CSS, ".poll-create {");
        assert!(has_decl(&card, "padding: var(--space-section)"), "{card}");
    }
}
