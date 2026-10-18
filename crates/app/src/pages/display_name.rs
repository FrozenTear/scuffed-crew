//! Copy for display-name rejections from the member API.
//!
//! The server answers `{"error":"display_name_taken"}` (409) and
//! `{"error":"display_name_invalid"}` (400). Match on the `error` code.
//!
//! Forms that send a name into those writes:
//! - profile edit (`PUT /api/members/{id}`, `display_name`)
//! - first-admin setup (`POST /api/auth/setup`, username stored as the display name)
//!
//! Accepting an application (`PATCH /api/applications/{id}`) does not send a
//! name. The server copies the account username onto the new member and can
//! return the same codes. The apply form does not set a display name.

use dioxus::prelude::*;
use scuffed_api_client::ClientError;

/// Profile edit and first-admin setup. The person at the keyboard typed the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayNameAudience {
    Editor,
    /// Officer or admin accepting an application. The name belongs to the applicant.
    Accept,
}

pub const DISPLAY_NAME_TAKEN: &str = "That name is taken. Pick another one.";

/// Empty after normalization, or a control character (including line separators).
/// Visible characters other than letters, numbers, and spaces are allowed.
pub const DISPLAY_NAME_INVALID: &str =
    "That name can't be used. It is blank or contains a control character.";

pub const DISPLAY_NAME_TAKEN_ON_ACCEPT: &str =
    "That name is already taken by another member. Ask them to pick a different name.";

fn json_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(str::to_owned))
}

/// `Some` only for the two display-name codes. Anything else is `None` so the
/// caller keeps its existing toast or banner.
pub fn display_name_error_message(
    err: &ClientError,
    audience: DisplayNameAudience,
) -> Option<&'static str> {
    let code = json_error_code(err.http_body()?)?;
    match (code.as_str(), audience) {
        ("display_name_taken", DisplayNameAudience::Editor) => Some(DISPLAY_NAME_TAKEN),
        ("display_name_taken", DisplayNameAudience::Accept) => Some(DISPLAY_NAME_TAKEN_ON_ACCEPT),
        ("display_name_invalid", _) => Some(DISPLAY_NAME_INVALID),
        _ => None,
    }
}

/// Field after a failed save. `value` is what stays in the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameFieldAfterResponse {
    pub value: String,
    pub error: Option<&'static str>,
    /// Not a display-name code. The caller shows its existing failure copy.
    pub keep_existing_failure: bool,
}

/// Known codes set `error` and leave the submitted text in place.
/// If `current` differs from `typed_at_submit`, the person already edited the
/// field, so the rejection is not put back.
pub fn settle_name_field(
    typed_at_submit: &str,
    current: &str,
    err: &ClientError,
    audience: DisplayNameAudience,
) -> NameFieldAfterResponse {
    let message = display_name_error_message(err, audience);
    if current != typed_at_submit {
        NameFieldAfterResponse {
            value: current.to_string(),
            error: None,
            keep_existing_failure: message.is_none(),
        }
    } else {
        NameFieldAfterResponse {
            value: typed_at_submit.to_string(),
            error: message,
            keep_existing_failure: message.is_none(),
        }
    }
}

/// Editing the name field replaces the text and drops the inline error.
pub fn name_field_after_edit(typed: String) -> (String, Option<&'static str>) {
    (typed, None)
}

pub fn applicant_name_error_id(app_id: &str) -> String {
    let safe: String = app_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("application-name-error-{safe}")
}

pub fn described_by_when_error(error: Option<&str>, error_id: &str) -> Option<String> {
    error
        .filter(|text| !text.is_empty())
        .map(|_| error_id.to_string())
}

fn name_field_aria(has_error: bool, error_id: &str) -> (Option<&'static str>, Option<String>) {
    if has_error {
        (Some("true"), Some(error_id.to_string()))
    } else {
        (None, None)
    }
}

/// Name input with its error in the same field. The `value` is whatever was typed.
#[component]
pub fn DisplayNameField(
    id: String,
    label: String,
    value: String,
    #[props(default)] error: Option<String>,
    #[props(default)] maxlength: Option<i64>,
    #[props(default)] autocomplete: Option<String>,
    field_class: String,
    oninput: EventHandler<FormEvent>,
) -> Element {
    let error_id = format!("{id}-error");
    let (invalid, described_by) = name_field_aria(error.is_some(), &error_id);
    let maxlength = maxlength.map(|n| n.to_string());
    rsx! {
        div { class: "{field_class}",
            label { r#for: "{id}", "{label}" }
            input {
                id: "{id}",
                r#type: "text",
                value: "{value}",
                maxlength,
                autocomplete,
                aria_invalid: invalid,
                aria_describedby: described_by,
                oninput: move |e| oninput.call(e),
            }
            if let Some(text) = error {
                p {
                    id: "{error_id}",
                    class: "name-field-error",
                    role: "alert",
                    "{text}"
                }
            }
        }
    }
}

/// Applicant name on the accept row, with the rejection under that name.
#[component]
pub fn ApplicantNameCell(name: String, error: Option<String>, error_id: String) -> Element {
    rsx! {
        span { "{name}" }
        if let Some(text) = error {
            p {
                id: "{error_id}",
                class: "name-field-error",
                role: "alert",
                "{text}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, body: &str) -> ClientError {
        ClientError::http(status, body, None)
    }

    fn render(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn maps_taken_and_invalid_codes() {
        let taken = http(409, r#"{"error":"display_name_taken"}"#);
        let invalid = http(
            400,
            r#"{"error":"display_name_invalid","detail":"control"}"#,
        );
        assert_eq!(
            display_name_error_message(&taken, DisplayNameAudience::Editor),
            Some(DISPLAY_NAME_TAKEN)
        );
        assert_eq!(
            display_name_error_message(&taken, DisplayNameAudience::Accept),
            Some(DISPLAY_NAME_TAKEN_ON_ACCEPT)
        );
        assert_eq!(
            display_name_error_message(&invalid, DisplayNameAudience::Editor),
            Some(DISPLAY_NAME_INVALID)
        );
        assert_eq!(
            display_name_error_message(&invalid, DisplayNameAudience::Accept),
            Some(DISPLAY_NAME_INVALID)
        );
        assert!(!DISPLAY_NAME_TAKEN.contains('\u{2014}'));
        assert!(!DISPLAY_NAME_INVALID.contains('\u{2014}'));
        assert!(!DISPLAY_NAME_TAKEN_ON_ACCEPT.contains('\u{2014}'));
        assert!(!DISPLAY_NAME_INVALID.contains("letters, numbers"));
    }

    #[test]
    fn unknown_errors_stay_unmapped() {
        let cases = [
            http(409, r#"{"error":"duplicate"}"#),
            http(400, r#"{"error":"Check your input"}"#),
            http(400, "display_name_invalid"),
            http(409, r#"{"error":" display_name_taken "}"#),
            http(500, r#"{"error":"Internal server error"}"#),
            http(409, ""),
            ClientError::Network("offline".into()),
            ClientError::Deserialize("missing field".into()),
        ];
        for err in &cases {
            assert_eq!(
                display_name_error_message(err, DisplayNameAudience::Editor),
                None,
                "{err}"
            );
            assert_eq!(
                display_name_error_message(err, DisplayNameAudience::Accept),
                None,
                "{err}"
            );
        }
    }

    #[test]
    fn rejection_keeps_the_typed_text() {
        let typed = "  Ada\u{200b} ";
        let err = http(409, r#"{"error":"display_name_taken"}"#);
        let settled = settle_name_field(typed, typed, &err, DisplayNameAudience::Editor);
        assert_eq!(settled.value, typed);
        assert_eq!(settled.error, Some(DISPLAY_NAME_TAKEN));
        assert!(!settled.keep_existing_failure);

        let unknown = http(500, r#"{"error":"Internal server error"}"#);
        let settled = settle_name_field(typed, typed, &unknown, DisplayNameAudience::Editor);
        assert_eq!(settled.value, typed);
        assert_eq!(settled.error, None);
        assert!(settled.keep_existing_failure);
    }

    #[test]
    fn editing_the_field_clears_the_error_and_keeps_the_new_text() {
        let (next, error) = name_field_after_edit("Ada 2".to_string());
        assert_eq!(next, "Ada 2");
        assert_eq!(error, None);

        let typed = "Ada";
        let edited = "Ada 2";
        let err = http(409, r#"{"error":"display_name_taken"}"#);
        let settled = settle_name_field(typed, edited, &err, DisplayNameAudience::Editor);
        assert_eq!(settled.value, edited);
        assert_eq!(settled.error, None);
        assert!(!settled.keep_existing_failure);

        let unknown = ClientError::Network("offline".into());
        let settled = settle_name_field(typed, edited, &unknown, DisplayNameAudience::Accept);
        assert_eq!(settled.value, edited);
        assert_eq!(settled.error, None);
        assert!(settled.keep_existing_failure);
    }

    #[test]
    fn accept_row_keeps_the_applicant_label() {
        let label = "Ada Lovelace";
        let err = http(409, r#"{"error":"display_name_taken"}"#);
        let settled = settle_name_field(label, label, &err, DisplayNameAudience::Accept);
        assert_eq!(settled.value, label);
        assert_eq!(settled.error, Some(DISPLAY_NAME_TAKEN_ON_ACCEPT));
        assert_eq!(
            applicant_name_error_id("application:app1"),
            "application-name-error-application-app1"
        );
        assert_eq!(
            described_by_when_error(Some("taken"), "application-name-error-app1").as_deref(),
            Some("application-name-error-app1")
        );
        assert_eq!(
            described_by_when_error(None, "application-name-error-app1"),
            None
        );
        assert_eq!(
            described_by_when_error(Some(""), "application-name-error-app1"),
            None
        );
    }

    fn field_with_error() -> Element {
        rsx! {
            DisplayNameField {
                id: "profile-display-name".to_string(),
                label: "Display name".to_string(),
                value: "  Ada ".to_string(),
                error: Some(DISPLAY_NAME_TAKEN.to_string()),
                maxlength: 64,
                field_class: "profile-edit-field".to_string(),
                oninput: |_| {},
            }
        }
    }

    fn field_without_error() -> Element {
        rsx! {
            DisplayNameField {
                id: "setup-user".to_string(),
                label: "Username".to_string(),
                value: "Admin".to_string(),
                autocomplete: "username".to_string(),
                field_class: "setup-field".to_string(),
                oninput: |_| {},
            }
        }
    }

    #[test]
    fn rendered_field_keeps_typed_text_and_points_at_the_alert() {
        let html = render(field_with_error);
        let input = html
            .split("<input")
            .nth(1)
            .expect("input")
            .split('>')
            .next()
            .expect("input tag");
        assert!(
            input.contains("value=\"  Ada \""),
            "typed text stays in the input: {input}"
        );
        assert!(input.contains("aria-invalid=\"true\""), "{input}");
        assert!(
            input.contains("aria-describedby=\"profile-display-name-error\""),
            "{input}"
        );
        assert!(html.contains("role=\"alert\""), "{html}");
        assert!(html.contains("id=\"profile-display-name-error\""), "{html}");
        assert!(html.contains(DISPLAY_NAME_TAKEN), "{html}");
        assert!(!html.contains('\u{2014}'), "{html}");
    }

    #[test]
    fn clean_field_omits_the_alert() {
        let html = render(field_without_error);
        assert!(html.contains("value=\"Admin\""), "{html}");
        assert!(!html.contains("role=\"alert\""), "{html}");
        assert!(!html.contains("aria-invalid"), "{html}");
        assert!(!html.contains("aria-describedby"), "{html}");
        assert!(html.contains("id=\"setup-user\""), "{html}");
        assert!(html.contains("autocomplete=\"username\""), "{html}");
    }

    fn applicant_with_error() -> Element {
        rsx! {
            ApplicantNameCell {
                name: "Ada Lovelace".to_string(),
                error: Some(DISPLAY_NAME_TAKEN_ON_ACCEPT.to_string()),
                error_id: "application-name-error-app1".to_string(),
            }
        }
    }

    #[test]
    fn applicant_cell_keeps_the_name_next_to_the_alert() {
        let html = render(applicant_with_error);
        assert!(html.contains("Ada Lovelace"), "{html}");
        assert!(html.contains(DISPLAY_NAME_TAKEN_ON_ACCEPT), "{html}");
        assert!(html.contains("role=\"alert\""), "{html}");
        assert!(
            html.contains("id=\"application-name-error-app1\""),
            "{html}"
        );
        assert!(!html.contains('\u{2014}'), "{html}");
    }
}
