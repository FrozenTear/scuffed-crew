use dioxus::prelude::*;
pub const LABEL_CSS: &str = r#"
.ui-label { font-family: var(--font-mono); font-weight: 500; font-size: var(--text-xs);
  letter-spacing: .08em; text-transform: uppercase; color: var(--text-3); }
"#;

/// Trimmed attribute text. Blank values are omitted so they do not render
/// an empty `id`, `name`, or `for`.
pub(crate) fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Accessible name for a control that may also have a visible label.
///
/// A visible label with an id is associated through `for` / `id`, so no
/// `aria-label` is set (it would override the label). Otherwise the
/// accessible name is the visible text, or `fallback` when there is none.
pub(crate) fn field_aria_label(
    label: Option<&str>,
    id: Option<&str>,
    fallback: &str,
) -> Option<String> {
    let label = label.map(str::trim).filter(|text| !text.is_empty());
    let id = id.map(str::trim).filter(|text| !text.is_empty());
    if label.is_some() && id.is_some() {
        None
    } else {
        Some(label.unwrap_or(fallback).to_string())
    }
}

#[component]
pub fn Label(
    /// Associates this text with a control. Renders a real `<label for>`
    /// when set. Without it the text stays a span, same as before.
    #[props(default)]
    for_id: Option<String>,
    children: Element,
) -> Element {
    let for_id = nonempty(for_id);
    if let Some(for_id) = for_id {
        rsx! { label { class: "ui-label", r#for: "{for_id}", {children} } }
    } else {
        rsx! { span { class: "ui-label", {children} } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn associated_label_does_not_also_set_aria_label() {
        assert_eq!(
            field_aria_label(Some("Hero"), Some("leaderboard-hero"), "Hero"),
            None
        );
        assert_eq!(
            field_aria_label(Some("  Season  "), Some("leaderboard-season"), "Season"),
            None
        );
    }

    #[test]
    fn missing_association_uses_visible_text_or_fallback() {
        assert_eq!(
            field_aria_label(Some("Filter by hero"), None, "Hero").as_deref(),
            Some("Filter by hero")
        );
        assert_eq!(
            field_aria_label(None, Some("stats-season"), "Season").as_deref(),
            Some("Season")
        );
        assert_eq!(
            field_aria_label(Some("   "), None, "Hero").as_deref(),
            Some("Hero")
        );
        assert_eq!(
            field_aria_label(None, None, "Hero").as_deref(),
            Some("Hero")
        );
    }

    #[test]
    fn nonempty_drops_blank_attribute_text() {
        assert_eq!(nonempty(None), None);
        assert_eq!(nonempty(Some("  ".into())), None);
        assert_eq!(
            nonempty(Some(" leaderboard-hero ".into())).as_deref(),
            Some("leaderboard-hero")
        );
    }
}
