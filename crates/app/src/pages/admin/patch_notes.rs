//! Admin › Patch Notes — officer CRUD for the Strategy catalog.
//!
//! Public `GET /api/strategy/patch-notes` (`{ "data": [...] }`) is unchanged.
//! Writes use the officer routes from #92 (session cookie, same as other admin
//! pages). Nested hero/section blobs are structured at the card level; the
//! per-hero `changes` array is a JSON textarea so the form stays usable.

use dioxus::prelude::*;

use crate::components::{
    AccessDenied, ConfirmDialog, DataTable, FormModal, Toast, admin_pending, use_toast,
};
use crate::hooks::{ModalController, use_api};
use crate::state::use_auth;
use crate::util::encode_query;
use scuffed_api_client::{ApiClient, ClientError};
use scuffed_types::{
    ApiSuccess, PatchChange, PatchHeroUpdate, PatchNote, PatchSection, UpdatePatchNoteRequest,
};

const CHANGE_TYPES: &[&str] = &["buff", "nerf", "adjustment", "bugfix"];

const PAGE_CSS: &str = r#"
    .nested-stack { display: flex; flex-direction: column; gap: 0.75rem; }
    .nested-block {
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 0.85rem;
        background: var(--surface);
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
    }
    .nested-block-head {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 0.5rem;
    }
    .nested-block-title {
        font-family: var(--font-head);
        font-size: 0.75rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--text-3);
        margin: 0;
    }
    .form-textarea.mono {
        font-family: var(--font-mono);
        min-height: 6rem;
    }
"#;

#[derive(Debug, Clone, PartialEq)]
struct HeroDraft {
    hero_id: String,
    hero_name: String,
    change_type: String,
    changes_json: String,
    dev_comment: String,
}

#[derive(Debug, Clone, PartialEq)]
struct SectionDraft {
    category: String,
    items_text: String,
}

fn empty_hero_draft() -> HeroDraft {
    HeroDraft {
        hero_id: String::new(),
        hero_name: String::new(),
        change_type: "adjustment".into(),
        changes_json: "[]".into(),
        dev_comment: String::new(),
    }
}

fn empty_section_draft() -> SectionDraft {
    SectionDraft {
        category: String::new(),
        items_text: String::new(),
    }
}

fn hero_id_from_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if matches!(c, ' ' | '-' | '_') && !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

fn is_yyyy_mm_dd(s: &str) -> bool {
    let s = s.trim();
    s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn patch_note_write_path(version: &str) -> String {
    format!("/api/strategy/patch-notes/{}", encode_query(version.trim()))
}

fn format_save_error(e: &ClientError) -> String {
    if e.http_status() == Some(409) {
        "That version already exists (409). Use a different version number.".into()
    } else {
        format!("Failed to save: {e}")
    }
}

fn is_blank_changes_json(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t == "[]"
}

fn is_blank_hero(h: &HeroDraft) -> bool {
    h.hero_id.trim().is_empty()
        && h.hero_name.trim().is_empty()
        && h.dev_comment.trim().is_empty()
        && is_blank_changes_json(&h.changes_json)
}

fn is_blank_section(s: &SectionDraft) -> bool {
    s.category.trim().is_empty() && s.items_text.trim().is_empty()
}

fn parse_changes_json(raw: &str) -> Result<Vec<PatchChange>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed).map_err(|e| format!("Hero changes JSON is invalid: {e}"))
}

fn pretty_changes_json(changes: &[PatchChange]) -> String {
    serde_json::to_string_pretty(changes).unwrap_or_else(|_| "[]".into())
}

fn items_from_textarea(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn items_to_textarea(items: &[String]) -> String {
    items.join("\n")
}

fn hero_from_note(h: &PatchHeroUpdate) -> HeroDraft {
    HeroDraft {
        hero_id: h.hero_id.clone(),
        hero_name: h.hero_name.clone(),
        change_type: if h.change_type.trim().is_empty() {
            "adjustment".into()
        } else {
            h.change_type.clone()
        },
        changes_json: pretty_changes_json(&h.changes),
        dev_comment: h.dev_comment.clone().unwrap_or_default(),
    }
}

fn section_from_note(s: &PatchSection) -> SectionDraft {
    SectionDraft {
        category: s.category.clone(),
        items_text: items_to_textarea(&s.items),
    }
}

fn assemble_heroes(drafts: &[HeroDraft]) -> Result<Vec<PatchHeroUpdate>, String> {
    let mut out = Vec::new();
    for (i, draft) in drafts.iter().enumerate() {
        if is_blank_hero(draft) {
            continue;
        }
        let mut hero_name = draft.hero_name.trim().to_string();
        let mut hero_id = draft.hero_id.trim().to_string();
        if hero_id.is_empty() {
            hero_id = hero_id_from_name(&hero_name);
        }
        if hero_name.is_empty() {
            hero_name = hero_id.clone();
        }
        if hero_id.is_empty() {
            return Err(format!(
                "Hero update {} needs a hero name or hero id.",
                i + 1
            ));
        }
        let change_type = {
            let t = draft.change_type.trim();
            if t.is_empty() {
                "adjustment".into()
            } else {
                t.to_string()
            }
        };
        let changes = parse_changes_json(&draft.changes_json)?;
        let comment = draft.dev_comment.trim();
        out.push(PatchHeroUpdate {
            hero_id,
            hero_name,
            change_type,
            changes,
            dev_comment: if comment.is_empty() {
                None
            } else {
                Some(comment.to_string())
            },
        });
    }
    Ok(out)
}

fn assemble_sections(drafts: &[SectionDraft]) -> Result<Vec<PatchSection>, String> {
    let mut out = Vec::new();
    for (i, draft) in drafts.iter().enumerate() {
        if is_blank_section(draft) {
            continue;
        }
        let category = draft.category.trim().to_string();
        if category.is_empty() {
            return Err(format!("Section {} needs a category.", i + 1));
        }
        out.push(PatchSection {
            category,
            items: items_from_textarea(&draft.items_text),
        });
    }
    Ok(out)
}

fn assemble_patch_note(
    version: &str,
    date: &str,
    title: &str,
    url: &str,
    heroes: &[HeroDraft],
    sections: &[SectionDraft],
) -> Result<PatchNote, String> {
    let version = version.trim().to_string();
    let date = date.trim().to_string();
    let url = url.trim().to_string();
    if version.is_empty() {
        return Err("Version is required.".into());
    }
    if date.is_empty() {
        return Err("Date is required.".into());
    }
    if !is_yyyy_mm_dd(&date) {
        return Err("Date must be YYYY-MM-DD.".into());
    }
    if url.is_empty() {
        return Err("Official URL is required.".into());
    }
    let title = title.trim();
    Ok(PatchNote {
        version,
        date,
        title: if title.is_empty() {
            None
        } else {
            Some(title.to_string())
        },
        url,
        hero_updates: assemble_heroes(heroes)?,
        sections: assemble_sections(sections)?,
    })
}

fn display_title(note: &PatchNote) -> String {
    note.title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("—")
        .to_string()
}

#[component]
pub fn AdminPatchNotes() -> Element {
    let auth = use_auth();
    let mut notes = use_api::<ApiSuccess<Vec<PatchNote>>>("/api/strategy/patch-notes");
    let mut toast = use_toast();

    let mut modal = ModalController::<String>::new();
    let mut delete_modal = ModalController::<PatchNote>::new();

    let mut form_version = use_signal(String::new);
    let mut form_date = use_signal(String::new);
    let mut form_title = use_signal(String::new);
    let mut form_url = use_signal(String::new);
    let mut form_heroes = use_signal(Vec::<HeroDraft>::new);
    let mut form_sections = use_signal(Vec::<SectionDraft>::new);

    let reset_form = move || {
        form_version.set(String::new());
        form_date.set(String::new());
        form_title.set(String::new());
        form_url.set(String::new());
        form_heroes.set(Vec::new());
        form_sections.set(Vec::new());
    };

    let open_create = move |_| {
        reset_form();
        modal.show_empty();
    };

    let mut open_edit = move |note: PatchNote| {
        form_version.set(note.version.clone());
        form_date.set(note.date.clone());
        form_title.set(note.title.clone().unwrap_or_default());
        form_url.set(note.url.clone());
        form_heroes.set(note.hero_updates.iter().map(hero_from_note).collect());
        form_sections.set(note.sections.iter().map(section_from_note).collect());
        modal.show(note.version);
    };

    let mut open_delete = move |note: PatchNote| {
        delete_modal.show(note);
    };

    let on_close = move |_| {
        modal.close();
    };

    let on_submit = move |_| {
        let assembled = match assemble_patch_note(
            &form_version(),
            &form_date(),
            &form_title(),
            &form_url(),
            &form_heroes(),
            &form_sections(),
        ) {
            Ok(note) => note,
            Err(msg) => {
                toast.show(Toast::error(msg));
                return;
            }
        };
        let edit_version = modal.get_target();

        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(version) = edit_version {
                let body = UpdatePatchNoteRequest {
                    date: Some(assembled.date),
                    title: Some(assembled.title),
                    url: Some(assembled.url),
                    hero_updates: Some(assembled.hero_updates),
                    sections: Some(assembled.sections),
                };
                client
                    .put_json::<_, PatchNote>(&patch_note_write_path(&version), &body)
                    .await
            } else {
                client
                    .post_json::<_, PatchNote>("/api/strategy/patch-notes", &assembled)
                    .await
            };

            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Patch note saved."));
                    modal.close();
                    notes.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format_save_error(&e)));
                }
            }
        });
    };

    let on_confirm_delete = move |_| {
        let Some(target) = delete_modal.get_target() else {
            return;
        };
        let version = target.version.clone();
        delete_modal.close();
        spawn(async move {
            match ApiClient::web()
                .delete(&patch_note_write_path(&version))
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Patch note deleted."));
                    notes.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to delete: {e}")));
                }
            }
        });
    };

    let on_cancel_delete = move |_| {
        delete_modal.close();
    };

    if !auth().is_officer_or_above() {
        return rsx! {
            AccessDenied { message: "You need officer permissions to manage patch notes.".to_string() }
        };
    }

    let editing = modal.get_target().is_some();

    rsx! {
        style { {PAGE_CSS} }

        div { class: "admin-toolbar",
            h1 { "Patch Notes" }
            button { class: "btn-add", onclick: open_create, "+ New Patch Note" }
        }
        p { class: "empty-state", style: "text-align:left;padding:0 0 1rem;margin:0;",
            "Published notes appear on the public Strategy Patch Notes page. "
            "Version cannot be changed after create — duplicate versions return 409."
        }

        {
            let data = notes.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => admin_pending(&notes, "patch notes"),
                Some(envelope) if envelope.data.is_empty() => rsx! {
                    p { class: "empty-state", "No patch notes yet." }
                },
                Some(envelope) => rsx! {
                    DataTable { headers: vec!["Version", "Date", "Title", "Actions"],
                        for note in envelope.data.iter() {
                            {
                                let edit_note = note.clone();
                                let delete_note = note.clone();
                                let title = display_title(note);
                                rsx! {
                                    tr { key: "{note.version}",
                                        td { code { "{note.version}" } }
                                        td { "{note.date}" }
                                        td { "{title}" }
                                        td {
                                            div { class: "row-actions",
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_edit(edit_note.clone()),
                                                    "Edit"
                                                }
                                                button {
                                                    class: "row-btn danger",
                                                    onclick: move |_| open_delete(delete_note.clone()),
                                                    "Delete"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }

        FormModal {
            title: if editing { "Edit Patch Note".to_string() } else { "New Patch Note".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,
            wide: true,

            div { class: "form-grid",
                div { class: "form-field",
                    label { class: "form-label", "Version" }
                    input {
                        class: "form-input",
                        r#type: "text",
                        value: "{form_version}",
                        placeholder: "2.18.1",
                        disabled: editing,
                        oninput: move |e| form_version.set(e.value()),
                    }
                    p { class: "form-hint",
                        if editing {
                            "Version is the URL key and cannot be changed."
                        } else {
                            "Must be unique. Duplicate versions return 409."
                        }
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Date" }
                    input {
                        class: "form-input",
                        r#type: "date",
                        value: "{form_date}",
                        oninput: move |e| form_date.set(e.value()),
                    }
                    p { class: "form-hint", "YYYY-MM-DD, shown as-is on the public page." }
                }
                div { class: "form-field span-full",
                    label { class: "form-label", "Title (optional)" }
                    input {
                        class: "form-input",
                        r#type: "text",
                        value: "{form_title}",
                        placeholder: "Mid-season balance",
                        oninput: move |e| form_title.set(e.value()),
                    }
                }
                div { class: "form-field span-full",
                    label { class: "form-label", "Official URL" }
                    input {
                        class: "form-input",
                        r#type: "url",
                        value: "{form_url}",
                        placeholder: "https://overwatch.blizzard.com/en-us/news/patch-notes/",
                        oninput: move |e| form_url.set(e.value()),
                    }
                }
            }

            div { class: "form-field",
                div { class: "nested-block-head",
                    label { class: "form-label", "Hero updates" }
                    button {
                        class: "row-btn",
                        r#type: "button",
                        onclick: move |_| form_heroes.write().push(empty_hero_draft()),
                        "+ Add hero"
                    }
                }
                p { class: "form-hint",
                    "changes is a JSON array of {{ability?, description, change_type}}."
                }
                if form_heroes().is_empty() {
                    p { class: "form-hint", "None yet — public cards still render without hero blocks." }
                } else {
                    div { class: "nested-stack",
                        for (i, hero) in form_heroes().into_iter().enumerate() {
                            {
                                let hero_id = hero.hero_id.clone();
                                let hero_name = hero.hero_name.clone();
                                let change_type = hero.change_type.clone();
                                let changes_json = hero.changes_json.clone();
                                let dev_comment = hero.dev_comment.clone();
                                rsx! {
                                    div { class: "nested-block", key: "hero-{i}",
                                        div { class: "nested-block-head",
                                            p { class: "nested-block-title", "Hero {i + 1}" }
                                            button {
                                                class: "row-btn danger",
                                                r#type: "button",
                                                onclick: move |_| {
                                                    let mut list = form_heroes();
                                                    if i < list.len() {
                                                        list.remove(i);
                                                        form_heroes.set(list);
                                                    }
                                                },
                                                "Remove"
                                            }
                                        }
                                        div { class: "form-grid",
                                            div { class: "form-field",
                                                label { class: "form-label", "Hero name" }
                                                input {
                                                    class: "form-input",
                                                    r#type: "text",
                                                    value: "{hero_name}",
                                                    placeholder: "Ana",
                                                    oninput: move |e| {
                                                        form_heroes.with_mut(|list| {
                                                            if let Some(h) = list.get_mut(i) {
                                                                h.hero_name = e.value();
                                                            }
                                                        });
                                                    },
                                                }
                                            }
                                            div { class: "form-field",
                                                label { class: "form-label", "Hero id" }
                                                input {
                                                    class: "form-input",
                                                    r#type: "text",
                                                    value: "{hero_id}",
                                                    placeholder: "ana",
                                                    oninput: move |e| {
                                                        form_heroes.with_mut(|list| {
                                                            if let Some(h) = list.get_mut(i) {
                                                                h.hero_id = e.value();
                                                            }
                                                        });
                                                    },
                                                }
                                            }
                                            div { class: "form-field",
                                                label { class: "form-label", "Change type" }
                                                select {
                                                    class: "form-select",
                                                    value: "{change_type}",
                                                    onchange: move |e| {
                                                        form_heroes.with_mut(|list| {
                                                            if let Some(h) = list.get_mut(i) {
                                                                h.change_type = e.value();
                                                            }
                                                        });
                                                    },
                                                    for t in CHANGE_TYPES {
                                                        option { value: "{t}", "{t}" }
                                                    }
                                                }
                                            }
                                            div { class: "form-field",
                                                label { class: "form-label", "Dev comment (optional)" }
                                                input {
                                                    class: "form-input",
                                                    r#type: "text",
                                                    value: "{dev_comment}",
                                                    oninput: move |e| {
                                                        form_heroes.with_mut(|list| {
                                                            if let Some(h) = list.get_mut(i) {
                                                                h.dev_comment = e.value();
                                                            }
                                                        });
                                                    },
                                                }
                                            }
                                            div { class: "form-field span-full",
                                                label { class: "form-label", "Changes (JSON)" }
                                                textarea {
                                                    class: "form-textarea mono",
                                                    value: "{changes_json}",
                                                    oninput: move |e| {
                                                        form_heroes.with_mut(|list| {
                                                            if let Some(h) = list.get_mut(i) {
                                                                h.changes_json = e.value();
                                                            }
                                                        });
                                                    },
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "form-field",
                div { class: "nested-block-head",
                    label { class: "form-label", "Sections" }
                    button {
                        class: "row-btn",
                        r#type: "button",
                        onclick: move |_| form_sections.write().push(empty_section_draft()),
                        "+ Add section"
                    }
                }
                p { class: "form-hint", "Category + one item per line (Bug Fixes, Maps, Competitive, …)." }
                if form_sections().is_empty() {
                    p { class: "form-hint", "None yet." }
                } else {
                    div { class: "nested-stack",
                        for (i, section) in form_sections().into_iter().enumerate() {
                            {
                                let category = section.category.clone();
                                let items_text = section.items_text.clone();
                                rsx! {
                                    div { class: "nested-block", key: "section-{i}",
                                        div { class: "nested-block-head",
                                            p { class: "nested-block-title", "Section {i + 1}" }
                                            button {
                                                class: "row-btn danger",
                                                r#type: "button",
                                                onclick: move |_| {
                                                    let mut list = form_sections();
                                                    if i < list.len() {
                                                        list.remove(i);
                                                        form_sections.set(list);
                                                    }
                                                },
                                                "Remove"
                                            }
                                        }
                                        div { class: "form-field",
                                            label { class: "form-label", "Category" }
                                            input {
                                                class: "form-input",
                                                r#type: "text",
                                                value: "{category}",
                                                placeholder: "Bug Fixes",
                                                oninput: move |e| {
                                                    form_sections.with_mut(|list| {
                                                        if let Some(s) = list.get_mut(i) {
                                                            s.category = e.value();
                                                        }
                                                    });
                                                },
                                            }
                                        }
                                        div { class: "form-field",
                                            label { class: "form-label", "Items (one per line)" }
                                            textarea {
                                                class: "form-textarea",
                                                value: "{items_text}",
                                                oninput: move |e| {
                                                    form_sections.with_mut(|list| {
                                                        if let Some(s) = list.get_mut(i) {
                                                            s.items_text = e.value();
                                                        }
                                                    });
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        ConfirmDialog {
            title: "Delete Patch Note".to_string(),
            message: format!(
                "Delete version \"{}\"? This removes it from the public Strategy Patch Notes page.",
                delete_modal.get_target().map(|n| n.version).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            on_confirm: on_confirm_delete,
            on_cancel: on_cancel_delete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_change() -> PatchChange {
        PatchChange {
            ability: Some("Biotic Rifle".into()),
            description: "Reload slightly faster.".into(),
            change_type: "buff".into(),
        }
    }

    #[test]
    fn list_envelope_unwraps_data() {
        let body = r#"{"data":[{"version":"1.0.0","date":"2026-01-01","url":"https://x.test"}]}"#;
        let parsed: ApiSuccess<Vec<PatchNote>> = serde_json::from_str(body).expect("envelope");
        assert_eq!(parsed.data.len(), 1);
        assert_eq!(parsed.data[0].version, "1.0.0");
        assert!(parsed.data[0].hero_updates.is_empty());
    }

    #[test]
    fn write_path_encodes_version() {
        assert_eq!(
            patch_note_write_path("2.18.1"),
            "/api/strategy/patch-notes/2.18.1"
        );
        assert_eq!(
            patch_note_write_path("  a/b  "),
            "/api/strategy/patch-notes/a%2Fb"
        );
    }

    #[test]
    fn date_must_be_iso_day() {
        assert!(is_yyyy_mm_dd("2026-09-14"));
        assert!(!is_yyyy_mm_dd("09/14/2026"));
        assert!(!is_yyyy_mm_dd("2026-9-14"));
        assert!(!is_yyyy_mm_dd(""));
    }

    #[test]
    fn hero_id_slug_from_display_name() {
        assert_eq!(hero_id_from_name("Ana"), "ana");
        assert_eq!(hero_id_from_name("Wrecking Ball"), "wrecking_ball");
        assert_eq!(hero_id_from_name("D.Va"), "dva");
    }

    #[test]
    fn assemble_rejects_missing_required_and_bad_json() {
        let err =
            assemble_patch_note("", "2026-01-01", "", "https://x.test", &[], &[]).unwrap_err();
        assert!(err.contains("Version"));
        let err = assemble_patch_note("1.0.0", "nope", "", "https://x.test", &[], &[]).unwrap_err();
        assert!(err.contains("YYYY-MM-DD"));
        let err = assemble_patch_note("1.0.0", "2026-01-01", "", "", &[], &[]).unwrap_err();
        assert!(err.contains("URL"));
        let bad = HeroDraft {
            hero_id: "ana".into(),
            hero_name: "Ana".into(),
            change_type: "buff".into(),
            changes_json: "not-json".into(),
            dev_comment: String::new(),
        };
        let err = assemble_patch_note("1.0.0", "2026-01-01", "T", "https://x.test", &[bad], &[])
            .unwrap_err();
        assert!(err.contains("JSON"));
    }

    #[test]
    fn assemble_skips_blank_rows_and_fills_hero_id() {
        let hero = HeroDraft {
            hero_id: String::new(),
            hero_name: "Freja".into(),
            change_type: "buff".into(),
            changes_json: serde_json::to_string(&[sample_change()]).expect("json"),
            dev_comment: "  ".into(),
        };
        let section = SectionDraft {
            category: "Bug Fixes".into(),
            items_text: "Fixed a tooltip.\n\nAnother fix.\n".into(),
        };
        let note = assemble_patch_note(
            " 4.0.0 ",
            "2026-09-14",
            "  Launch  ",
            " https://x.test/notes ",
            &[empty_hero_draft(), hero],
            &[empty_section_draft(), section],
        )
        .expect("ok");
        assert_eq!(note.version, "4.0.0");
        assert_eq!(note.title.as_deref(), Some("Launch"));
        assert_eq!(note.hero_updates.len(), 1);
        assert_eq!(note.hero_updates[0].hero_id, "freja");
        assert!(note.hero_updates[0].dev_comment.is_none());
        assert_eq!(note.sections[0].items, ["Fixed a tooltip.", "Another fix."]);
    }

    #[test]
    fn conflict_toast_is_explicit() {
        let err = ClientError::Http {
            status: 409,
            body: r#"{"error":"Patch note version already exists"}"#.into(),
        };
        let msg = format_save_error(&err);
        assert!(msg.contains("409"), "{msg}");
        assert!(msg.contains("already exists"), "{msg}");
        let other = ClientError::Http {
            status: 400,
            body: r#"{"error":"date is required"}"#.into(),
        };
        let msg = format_save_error(&other);
        assert!(msg.contains("400"), "{msg}");
        assert!(!msg.contains("already exists"), "{msg}");
    }

    #[test]
    fn display_title_falls_back() {
        let mut note = PatchNote {
            version: "1".into(),
            date: "2026-01-01".into(),
            title: None,
            url: "https://x.test".into(),
            hero_updates: vec![],
            sections: vec![],
        };
        assert_eq!(display_title(&note), "—");
        note.title = Some("  Mid-season  ".into());
        assert_eq!(display_title(&note), "Mid-season");
    }
}
