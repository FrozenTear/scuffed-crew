//! Admin › Patch Notes: officer CRUD for the Strategy catalog.
//!
//! Frozen write contract: Contabo `6cf6260` / #92. Do not invent routes or
//! fields. Public `GET /api/strategy/patch-notes` (`{ "data": [PatchNote] }`)
//! is unchanged. Writes use OfficerUser session cookies:
//! - POST `/api/strategy/patch-notes` full `PatchNote` → 201; dup version → 409
//! - PUT `/api/strategy/patch-notes/{version}` partial `UpdatePatchNoteRequest`
//! - DELETE `/api/strategy/patch-notes/{version}` → 200; missing → 404
//! - anon → 401; member → 403
//!
//! Fields: `version`, `date`, `title`, `url`, `hero_updates`, `sections`.
//! Nested hero `changes` stay a JSON textarea so the form stays usable.

use dioxus::prelude::*;

use crate::components::ui::{
    EmptyCard, PageHeader, PageShell, PageWidth, Pager, page_slice, show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, AccessDenied, AdminDate, ConfirmDialog, DataTable, FormModal, RowActions,
    Toast, admin_list_pending, row_menu_toggle_id, use_toast,
};
use crate::hooks::{ModalController, use_api};
use crate::state::use_auth;
use crate::util::encode_query;
use scuffed_api_client::{ApiClient, ClientError};
use scuffed_types::{
    ApiSuccess, PatchChange, PatchHeroUpdate, PatchNote, PatchSection, UpdatePatchNoteRequest,
};

const CHANGE_TYPES: &[&str] = &["buff", "nerf", "adjustment", "bugfix"];

/// Page CSS: the hero and section blocks of the editor, and the version cell.
const PAGE_CSS: &str = r#"
    .patch-group {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        min-width: 0;
    }
    .patch-group-title {
        margin: 0;
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-md);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
    }
    .nested-stack { display: flex; flex-direction: column; gap: var(--space-3); }
    .nested-block {
        border: 1px solid var(--border);
        border-radius: var(--radius-card);
        padding: var(--space-card-pad);
        background: var(--surface);
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }
    .nested-block-head {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: var(--space-2);
    }
    .nested-block-title {
        margin: 0;
        color: var(--text-3);
        font-family: var(--font-head);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
    }
    textarea.field-input.patch-json {
        font-family: var(--font-mono);
        font-size: var(--text-sm);
    }
    .patch-version {
        font-family: var(--font-mono);
        font-size: var(--text-sm);
        overflow-wrap: anywhere;
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

/// Map #92 write statuses to officer-facing copy. 401/403/409/404 are the
/// live Contabo contract; everything else keeps the raw client message.
fn format_write_error(verb: &str, e: &ClientError) -> String {
    match e.http_status() {
        Some(401) => "Sign in required (401). Officer session expired or missing.".into(),
        Some(403) => "Officer access required (403). Members cannot write patch notes.".into(),
        Some(409) => "That version already exists (409). Use a different version number.".into(),
        Some(404) => "Patch note not found (404). It may have already been deleted.".into(),
        _ => format!("Failed to {verb}: {e}"),
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
        .unwrap_or("No title")
        .to_string()
}

/// A version or record key as a DOM id part: characters outside
/// `[A-Za-z0-9_-]` (the dots in `2.18.1`) become `-`.
fn id_part(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// DOM id of one editor field: the version being edited (`new` for an unsaved
/// note) plus the field name.
fn patch_field_id(version: Option<&str>, field: &str) -> String {
    let key = version.filter(|v| !v.trim().is_empty()).unwrap_or("new");
    format!("patch-{}-{}", id_part(key), id_part(field))
}

fn edit_button_id(version: &str) -> String {
    patch_field_id(Some(version), "edit")
}

fn actions_id(version: &str) -> String {
    patch_field_id(Some(version), "actions")
}

/// "2 heroes, 1 section": what a note holds, for the list.
fn contents_summary(note: &PatchNote) -> String {
    let count = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    format!(
        "{}, {}",
        count(note.hero_updates.len(), "hero", "heroes"),
        count(note.sections.len(), "section", "sections")
    )
}

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-patch-notes-list";
/// The header's "New patch note" button.
const NEW_ID: &str = "patch-notes-new";
/// The empty card's "New patch note" button.
const NEW_EMPTY_ID: &str = "patch-notes-new-empty";

#[component]
pub fn AdminPatchNotes() -> Element {
    let auth = use_auth();
    let mut notes = use_api::<ApiSuccess<Vec<PatchNote>>>("/api/strategy/patch-notes");
    let mut toast = use_toast();
    let mut page = use_signal(|| 1usize);

    let mut modal = ModalController::<String>::new();
    let mut delete_modal = ModalController::<PatchNote>::new();

    let mut form_version = use_signal(String::new);
    let mut form_date = use_signal(String::new);
    let mut form_title = use_signal(String::new);
    let mut form_url = use_signal(String::new);
    let mut form_heroes = use_signal(Vec::<HeroDraft>::new);
    let mut form_sections = use_signal(Vec::<SectionDraft>::new);
    // The dialog's live line: a check that failed, or a failed save with the
    // server's reason.
    let mut form_error = use_signal(|| None::<String>);
    // Id of the button that opened the editor, for focus on Cancel.
    let mut opener = use_signal(|| None::<String>);

    let mut reset_form = move || {
        form_version.set(String::new());
        form_date.set(String::new());
        form_title.set(String::new());
        form_url.set(String::new());
        form_heroes.set(Vec::new());
        form_sections.set(Vec::new());
        form_error.set(None);
    };

    let mut open_create = move |from: &'static str| {
        reset_form();
        opener.set(Some(from.to_string()));
        modal.show_empty();
    };

    let mut open_edit = move |note: PatchNote| {
        form_version.set(note.version.clone());
        form_date.set(note.date.clone());
        form_title.set(note.title.clone().unwrap_or_default());
        form_url.set(note.url.clone());
        form_heroes.set(note.hero_updates.iter().map(hero_from_note).collect());
        form_sections.set(note.sections.iter().map(section_from_note).collect());
        form_error.set(None);
        opener.set(Some(edit_button_id(&note.version)));
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
                form_error.set(Some(msg.clone()));
                toast.show(Toast::error(msg));
                return;
            }
        };
        form_error.set(None);
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
                    let message = format_write_error("save", &e);
                    form_error.set(Some(message.clone()));
                    toast.show(Toast::error(message));
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
                    toast.show(Toast::error(format_write_error("delete", &e)));
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

    let list = {
        let data = notes.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&notes, "patch notes"),
            Some(envelope) => {
                let all = &envelope.data;
                let total = all.len();
                let rows = page_slice(all, page(), ADMIN_PAGE_SIZE).to_vec();
                rsx! {
                    if all.is_empty() {
                        EmptyCard {
                            title: "No patch notes yet",
                            message: "Notes you add show on the public Patch Notes page. Add the latest Overwatch patch to start the list.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    id: NEW_EMPTY_ID,
                                    class: "btn is-primary",
                                    onclick: move |_| open_create(NEW_EMPTY_ID),
                                    "New patch note"
                                }
                            },
                        }
                    } else {
                        PatchNotesTable {
                            rows,
                            on_edit: move |n: PatchNote| open_edit(n),
                            on_delete: move |n: PatchNote| open_delete(n),
                        }
                    }
                    Pager {
                        id: "patch-notes-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "patch notes",
                        noun_singular: "patch note",
                        show_page_count: true,
                        on_change: move |next| {
                            page.set(next);
                            show_list_top(LIST_ID);
                        },
                    }
                }
            }
        }
    };

    let record = modal.get_target();
    let editing = record.is_some();
    let delete_target = delete_modal.get_target();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-patch_notes-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Wide, class: "admin-patch-notes-page",
            PageHeader {
                title: "Patch notes",
                lead: "Notes here appear on the public Patch Notes page. The version is the note's address, so it cannot change after you create it.",
                actions: rsx! {
                    button {
                        r#type: "button",
                        id: NEW_ID,
                        class: "btn is-primary",
                        onclick: move |_| open_create(NEW_ID),
                        "New patch note"
                    }
                },
            }
            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Patch notes",
                {list}
            }
        }

        FormModal {
            title: if editing { "Edit patch note".to_string() } else { "New patch note".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            submit_label: if editing { None } else { Some("Create".to_string()) },
            error: form_error(),
            restore_focus_id: opener(),
            on_close: on_close,
            on_submit: on_submit,
            wide: true,
            PatchNoteFields {
                record,
                version: form_version,
                date: form_date,
                title: form_title,
                url: form_url,
                heroes: form_heroes,
                sections: form_sections,
            }
        }

        ConfirmDialog {
            title: "Delete patch note".to_string(),
            message: format!(
                "Delete version \"{}\"? This removes it from the public Patch Notes page.",
                delete_target.as_ref().map(|n| n.version.clone()).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            confirm_label: "Delete".to_string(),
            restore_focus_id: delete_target.as_ref().map(|n| row_menu_toggle_id(&actions_id(&n.version))),
            on_confirm: on_confirm_delete,
            on_cancel: on_cancel_delete,
        }
    }
}

/// The rows: version, date, title and contents, then Edit in the row and
/// Delete behind "Actions" on phone.
#[component]
fn PatchNotesTable(
    rows: Vec<PatchNote>,
    on_edit: EventHandler<PatchNote>,
    on_delete: EventHandler<PatchNote>,
) -> Element {
    rsx! {
        DataTable {
            headers: vec!["Version", "Title", "Contents", "Date", "Actions"],
            num_cols: vec![3],
            cards: true,
            label: "Patch notes".to_string(),
            for note in rows {
                {
                    let edit = note.clone();
                    let delete = note.clone();
                    let title = display_title(&note);
                    let contents = contents_summary(&note);
                    let version = note.version.clone();
                    rsx! {
                        tr { key: "{note.version}",
                            td { class: "cell-who", code { class: "patch-version", "{note.version}" } }
                            // On phone the title is the line under the version,
                            // not a labelled line after the meta.
                            td { class: "cell-badge patch-title", "{title}" }
                            td { class: "cell-meta", "{contents}" }
                            td { class: "cell-meta num", AdminDate { iso: note.date.clone() } }
                            td { class: "cell-actions",
                                RowActions {
                                    id: actions_id(&note.version),
                                    label: "Actions for version {version}",
                                    inline_on_desktop: true,
                                    more: rsx! {
                                        button {
                                            r#type: "button",
                                            class: "btn is-danger",
                                            aria_label: "Delete version {version}",
                                            onclick: move |_| on_delete.call(delete.clone()),
                                            "Delete"
                                        }
                                    },
                                    button {
                                        r#type: "button",
                                        id: edit_button_id(&note.version),
                                        class: "btn is-ghost",
                                        aria_label: "Edit version {version}",
                                        onclick: move |_| on_edit.call(edit.clone()),
                                        "Edit"
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

/// The editor fields. Ids come from the version being edited plus the field
/// name (and the block number for hero and section blocks).
#[component]
fn PatchNoteFields(
    record: Option<String>,
    version: Signal<String>,
    date: Signal<String>,
    title: Signal<String>,
    url: Signal<String>,
    heroes: Signal<Vec<HeroDraft>>,
    sections: Signal<Vec<SectionDraft>>,
) -> Element {
    let editing = record.is_some();
    let key = record.as_deref();
    let id = |field: &str| patch_field_id(key, field);
    let (version_id, version_hint_id) = (id("version"), id("version-hint"));
    let (date_id, date_hint_id) = (id("date"), id("date-hint"));
    let title_id = id("title");
    let url_id = id("url");
    let (heroes_head_id, heroes_hint_id) = (id("heroes-head"), id("heroes-hint"));
    let (sections_head_id, sections_hint_id) = (id("sections-head"), id("sections-hint"));
    let key_owned = key.map(str::to_string);
    rsx! {
        div { class: "form-grid",
            div { class: "form-field",
                label { class: "field-label", r#for: "{version_id}", "Version" }
                input {
                    id: "{version_id}",
                    class: "field-input",
                    r#type: "text",
                    value: "{version}",
                    placeholder: "2.18.1",
                    readonly: editing,
                    aria_describedby: "{version_hint_id}",
                    oninput: move |e| version.set(e.value()),
                }
                p { id: "{version_hint_id}", class: "field-hint",
                    if editing {
                        "Version is the URL key and cannot be changed."
                    } else {
                        "Must be unique. Duplicate versions return 409."
                    }
                }
            }
            div { class: "form-field",
                label { class: "field-label", r#for: "{date_id}", "Date" }
                input {
                    id: "{date_id}",
                    class: "field-input",
                    r#type: "date",
                    value: "{date}",
                    aria_describedby: "{date_hint_id}",
                    oninput: move |e| date.set(e.value()),
                }
                p { id: "{date_hint_id}", class: "field-hint", "YYYY-MM-DD, shown as-is on the public page." }
            }
            div { class: "form-field span-full",
                label { class: "field-label", r#for: "{title_id}", "Title (optional)" }
                input {
                    id: "{title_id}",
                    class: "field-input",
                    r#type: "text",
                    value: "{title}",
                    placeholder: "Mid-season balance",
                    oninput: move |e| title.set(e.value()),
                }
            }
            div { class: "form-field span-full",
                label { class: "field-label", r#for: "{url_id}", "Official URL" }
                input {
                    id: "{url_id}",
                    class: "field-input",
                    r#type: "url",
                    value: "{url}",
                    placeholder: "https://overwatch.blizzard.com/en-us/news/patch-notes/",
                    oninput: move |e| url.set(e.value()),
                }
            }
        }

        section { class: "patch-group", aria_labelledby: "{heroes_head_id}",
            div { class: "nested-block-head",
                h3 { id: "{heroes_head_id}", class: "patch-group-title", "Hero updates" }
                button {
                    class: "btn is-ghost",
                    r#type: "button",
                    onclick: move |_| heroes.write().push(empty_hero_draft()),
                    "Add hero"
                }
            }
            p { id: "{heroes_hint_id}", class: "field-hint",
                if heroes.read().is_empty() {
                    "None yet. Public cards still render without hero blocks. "
                }
                "Changes is a JSON array of {{ability?, description, change_type}}."
            }
            if !heroes.read().is_empty() {
                div { class: "nested-stack",
                    for (i, hero) in heroes().into_iter().enumerate() {
                        HeroBlock {
                            key: "hero-{i}",
                            record: key_owned.clone(),
                            index: i,
                            hero,
                            heroes,
                            hint_id: heroes_hint_id.clone(),
                        }
                    }
                }
            }
        }

        section { class: "patch-group", aria_labelledby: "{sections_head_id}",
            div { class: "nested-block-head",
                h3 { id: "{sections_head_id}", class: "patch-group-title", "Sections" }
                button {
                    class: "btn is-ghost",
                    r#type: "button",
                    onclick: move |_| sections.write().push(empty_section_draft()),
                    "Add section"
                }
            }
            p { id: "{sections_hint_id}", class: "field-hint",
                if sections.read().is_empty() {
                    "None yet. "
                }
                "A category plus one item per line (Bug Fixes, Maps, Competitive)."
            }
            if !sections.read().is_empty() {
                div { class: "nested-stack",
                    for (i, section) in sections().into_iter().enumerate() {
                        SectionBlock {
                            key: "section-{i}",
                            record: key_owned.clone(),
                            index: i,
                            section,
                            sections,
                        }
                    }
                }
            }
        }
    }
}

/// One hero update block, named by its "Hero 1" title.
#[component]
fn HeroBlock(
    record: Option<String>,
    index: usize,
    hero: HeroDraft,
    heroes: Signal<Vec<HeroDraft>>,
    hint_id: String,
) -> Element {
    let i = index;
    let n = i + 1;
    let id = |field: &str| patch_field_id(record.as_deref(), &format!("hero-{n}-{field}"));
    let (head_id, name_id, hero_id_id) = (id("title"), id("name"), id("id"));
    let (type_id, comment_id, changes_id) = (id("type"), id("comment"), id("changes"));
    let mut edit = move |apply: &dyn Fn(&mut HeroDraft)| {
        heroes.with_mut(|list| {
            if let Some(h) = list.get_mut(i) {
                apply(h);
            }
        });
    };
    rsx! {
        div { class: "nested-block", role: "group", aria_labelledby: "{head_id}",
            div { class: "nested-block-head",
                p { id: "{head_id}", class: "nested-block-title", "Hero {n}" }
                button {
                    class: "btn is-danger",
                    r#type: "button",
                    aria_label: "Remove hero {n}",
                    onclick: move |_| {
                        let mut list = heroes();
                        if i < list.len() {
                            list.remove(i);
                            heroes.set(list);
                        }
                    },
                    "Remove"
                }
            }
            div { class: "form-grid",
                div { class: "form-field",
                    label { class: "field-label", r#for: "{name_id}", "Hero name" }
                    input {
                        id: "{name_id}",
                        class: "field-input",
                        r#type: "text",
                        value: "{hero.hero_name}",
                        placeholder: "Ana",
                        oninput: move |e| edit(&|h| h.hero_name = e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: "{hero_id_id}", "Hero id" }
                    input {
                        id: "{hero_id_id}",
                        class: "field-input",
                        r#type: "text",
                        value: "{hero.hero_id}",
                        placeholder: "ana",
                        oninput: move |e| edit(&|h| h.hero_id = e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: "{type_id}", "Change type" }
                    select {
                        id: "{type_id}",
                        class: "field-input",
                        value: "{hero.change_type}",
                        onchange: move |e| edit(&|h| h.change_type = e.value()),
                        for t in CHANGE_TYPES {
                            option { value: "{t}", selected: hero.change_type == *t, "{t}" }
                        }
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: "{comment_id}", "Dev comment (optional)" }
                    input {
                        id: "{comment_id}",
                        class: "field-input",
                        r#type: "text",
                        value: "{hero.dev_comment}",
                        oninput: move |e| edit(&|h| h.dev_comment = e.value()),
                    }
                }
                div { class: "form-field span-full",
                    label { class: "field-label", r#for: "{changes_id}", "Changes (JSON)" }
                    textarea {
                        id: "{changes_id}",
                        class: "field-input patch-json",
                        value: "{hero.changes_json}",
                        aria_describedby: "{hint_id}",
                        oninput: move |e| edit(&|h| h.changes_json = e.value()),
                    }
                }
            }
        }
    }
}

/// One section block, named by its "Section 1" title.
#[component]
fn SectionBlock(
    record: Option<String>,
    index: usize,
    section: SectionDraft,
    sections: Signal<Vec<SectionDraft>>,
) -> Element {
    let i = index;
    let n = i + 1;
    let id = |field: &str| patch_field_id(record.as_deref(), &format!("section-{n}-{field}"));
    let (head_id, category_id, items_id) = (id("title"), id("category"), id("items"));
    let mut edit = move |apply: &dyn Fn(&mut SectionDraft)| {
        sections.with_mut(|list| {
            if let Some(s) = list.get_mut(i) {
                apply(s);
            }
        });
    };
    rsx! {
        div { class: "nested-block", role: "group", aria_labelledby: "{head_id}",
            div { class: "nested-block-head",
                p { id: "{head_id}", class: "nested-block-title", "Section {n}" }
                button {
                    class: "btn is-danger",
                    r#type: "button",
                    aria_label: "Remove section {n}",
                    onclick: move |_| {
                        let mut list = sections();
                        if i < list.len() {
                            list.remove(i);
                            sections.set(list);
                        }
                    },
                    "Remove"
                }
            }
            div { class: "form-field",
                label { class: "field-label", r#for: "{category_id}", "Category" }
                input {
                    id: "{category_id}",
                    class: "field-input",
                    r#type: "text",
                    value: "{section.category}",
                    placeholder: "Bug Fixes",
                    oninput: move |e| edit(&|s| s.category = e.value()),
                }
            }
            div { class: "form-field",
                label { class: "field-label", r#for: "{items_id}", "Items (one per line)" }
                textarea {
                    id: "{items_id}",
                    class: "field-input",
                    value: "{section.items_text}",
                    oninput: move |e| edit(&|s| s.items_text = e.value()),
                }
            }
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

    fn http(status: u16, error: &str) -> ClientError {
        ClientError::http(status, format!(r#"{{"error":"{error}"}}"#), None)
    }

    #[test]
    fn write_toasts_match_live_contract_statuses() {
        let msg = format_write_error("save", &http(409, "Patch note version already exists"));
        assert!(msg.contains("409"), "{msg}");
        assert!(msg.contains("already exists"), "{msg}");
        let msg = format_write_error("save", &http(401, "Authentication required"));
        assert!(msg.contains("401"), "{msg}");
        let msg = format_write_error("save", &http(403, "Officer access required"));
        assert!(msg.contains("403"), "{msg}");
        assert!(msg.contains("Members cannot write"), "{msg}");
        let msg = format_write_error("delete", &http(404, "Patch note not found"));
        assert!(msg.contains("404"), "{msg}");
        let msg = format_write_error("save", &http(400, "date is required"));
        assert!(msg.contains("400"), "{msg}");
        assert!(!msg.contains("already exists"), "{msg}");
    }

    #[test]
    fn create_body_is_full_patch_note_contract() {
        let note = assemble_patch_note(
            "4.0.0",
            "2026-09-14",
            "Launch",
            "https://x.test/notes",
            &[],
            &[],
        )
        .expect("ok");
        let value = serde_json::to_value(&note).expect("json");
        for key in [
            "version",
            "date",
            "title",
            "url",
            "hero_updates",
            "sections",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert!(value.get("created_at").is_none());
        assert!(value.get("updated_at").is_none());
        assert_eq!(value["hero_updates"], serde_json::json!([]));
        assert_eq!(value["sections"], serde_json::json!([]));
    }

    #[test]
    fn update_body_is_partial_and_omits_version() {
        let body = UpdatePatchNoteRequest {
            date: Some("2026-09-14".into()),
            title: Some(Some("Renamed".into())),
            url: None,
            hero_updates: None,
            sections: None,
        };
        let value = serde_json::to_value(&body).expect("json");
        assert!(
            value.get("version").is_none(),
            "version is the path key, not a PUT field"
        );
        assert_eq!(value["date"], "2026-09-14");
        assert_eq!(value["title"], "Renamed");
        // Frozen #92 type has no skip_serializing_if; unset Options are JSON null
        // and deserialize back to None (leave unchanged).
        assert!(value["url"].is_null());
        assert!(value["hero_updates"].is_null());
        assert!(value["sections"].is_null());
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
        assert_eq!(display_title(&note), "No title");
        note.title = Some("  Mid-season  ".into());
        assert_eq!(display_title(&note), "Mid-season");
    }

    fn render(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// Every `<label for="x">` has a control with `id="x"`, and ids are unique.
    fn assert_labels_tied(html: &str) -> usize {
        let mut seen = 0;
        for part in html.split("<label").skip(1) {
            let head = part.split('>').next().unwrap_or_default();
            let target = head
                .split("for=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_else(|| panic!("label without for: {head}"));
            assert_eq!(
                html.matches(&format!("id=\"{target}\"")).count(),
                1,
                "control for {target}: {html}"
            );
            seen += 1;
        }
        seen
    }

    fn sample_note() -> PatchNote {
        PatchNote {
            version: "2.18.1".into(),
            date: "2026-11-10".into(),
            title: None,
            url: "https://x.test".into(),
            hero_updates: vec![],
            sections: vec![PatchSection {
                category: "Bug Fixes".into(),
                items: vec!["Fixed a tooltip.".into()],
            }],
        }
    }

    #[test]
    fn page_has_one_h1_and_a_labelled_wait_in_the_wide_shell() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/patch-notes", Some(scuffed_types::OrgRole::Officer));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-patch-notes-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Patch notes</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains("role=\"status\">Loading patch notes...</p>"),
            "{html}"
        );
        // Nothing in the page itself is styled inline (the shared toast
        // host outside the page is not this page's markup).
        let page = &html[html.find("admin-patch-notes-page").expect("page")..];
        let page = &page[..page.find("</main>").expect("main end")];
        assert!(!page.contains("style="), "{page}");
        assert!(!html.contains('\u{2014}'), "{html}");
    }

    #[test]
    fn rows_are_cards_with_contents_and_a_folded_delete() {
        fn view() -> Element {
            rsx! {
                PatchNotesTable { rows: vec![sample_note()], on_edit: |_| {}, on_delete: |_| {} }
            }
        }
        let html = render(view);
        assert!(html.contains("data-table-scroll is-cards"), "{html}");
        assert!(
            html.contains("<th scope=\"col\" class=\"num\">Date</th>"),
            "{html}"
        );
        assert!(html.contains(">0 heroes, 1 section</td>"), "{html}");
        assert!(
            html.contains("class=\"cell-badge patch-title\">No title</td>"),
            "{html}"
        );
        assert!(html.contains("id=\"patch-2-18-1-edit\""), "{html}");
        let group = &html[html.find("class=\"row-menu-group\"").expect("group")..];
        assert!(
            group.contains("aria-label=\"Delete version 2.18.1\""),
            "{group}"
        );
    }

    #[test]
    fn editor_fields_are_labelled_per_block() {
        fn view() -> Element {
            let version = use_signal(|| "2.18.1".to_string());
            let date = use_signal(String::new);
            let title = use_signal(String::new);
            let url = use_signal(String::new);
            let heroes = use_signal(|| vec![empty_hero_draft(), empty_hero_draft()]);
            let sections = use_signal(|| vec![empty_section_draft()]);
            rsx! {
                PatchNoteFields {
                    record: Some("2.18.1".to_string()),
                    version,
                    date,
                    title,
                    url,
                    heroes,
                    sections,
                }
            }
        }
        let html = render(view);
        // 4 note fields, 5 per hero, 2 per section.
        assert_eq!(assert_labels_tied(&html), 4 + 5 * 2 + 2, "{html}");
        assert!(html.contains("id=\"patch-2-18-1-hero-2-name\""), "{html}");
        assert!(html.contains("aria-label=\"Remove hero 2\""), "{html}");
        assert!(html.contains("readonly"), "{html}");
        assert!(!html.contains("disabled"), "{html}");
    }

    #[test]
    fn page_css_sizes_come_from_tokens() {
        crate::components::ui::test_support::assert_sizes_from_tokens(PAGE_CSS);
    }

    #[test]
    fn contents_count_words() {
        let mut note = sample_note();
        assert_eq!(contents_summary(&note), "0 heroes, 1 section");
        note.sections.clear();
        assert_eq!(contents_summary(&note), "0 heroes, 0 sections");
        assert_eq!(patch_field_id(None, "url"), "patch-new-url");
    }
}
