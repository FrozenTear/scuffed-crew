use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pill, PillTone, show_list_top,
};
use crate::components::{
    AdminDate, ConfirmDialog, DataTable, FormModal, RowActions, Toast, admin_pending,
    row_menu_toggle_id, use_toast,
};
use crate::hooks::{ModalController, use_api_with};
use crate::state::use_auth;
use crate::upload::{
    IMAGE_UPLOAD_ACCEPT, UploadFailure, clear_file_input, post_file, selected_file,
    validate_image_upload,
};
use scuffed_api_client::ApiClient;

/// Matches audit-log page size. API default is 20 with no total; a full page
/// means another offset page may exist.
const PAGE_SIZE: u32 = 50;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Article {
    id: String,
    slug: String,
    title: String,
    content_markdown: String,
    summary: Option<String>,
    cover_image_url: Option<String>,
    published: bool,
    published_at: Option<String>,
    created_at: String,
}

#[derive(serde::Serialize)]
struct CreateArticleBody {
    title: String,
    slug: String,
    content_markdown: String,
    summary: Option<String>,
    cover_image_url: Option<String>,
}

#[derive(serde::Serialize)]
struct UpdateArticleBody {
    title: Option<String>,
    slug: Option<String>,
    content_markdown: Option<String>,
    summary: Option<Option<String>>,
    cover_image_url: Option<Option<String>>,
}

/// DOM id for one editor field, built from the article record id (or `new`
/// for an unsaved article) and the field name. Characters outside
/// `[A-Za-z0-9_-]` (the `:` in `article:abc`) become `-`.
fn article_field_id(record_id: Option<&str>, field: &str) -> String {
    let key = record_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or("new");
    let clean = |raw: &str| -> String {
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
    };
    format!("article-{}-{}", clean(key), clean(field))
}

/// Source for the cover preview: an `/uploads/images/...` path or an
/// absolute `https://` URL with a host, matching the server's
/// `normalize_cover_image_url`. Plain `http://`, `https://` with no host,
/// avatar paths, `..` segments, and other values the server refuses have no
/// preview.
fn cover_preview_src(url: &str) -> Option<&str> {
    let url = url.trim();
    if crate::upload::is_cover_image_upload_path(url) {
        // img src needs a rooted path; the no-slash form heals on save only.
        return url.starts_with('/').then_some(url);
    }
    if crate::upload::is_cover_https_url(url) {
        return Some(url);
    }
    None
}

/// Toast copy when `POST /api/upload/image` answers with an error status.
fn image_upload_failure_message(status: u16) -> String {
    match status {
        400 => {
            "The server could not read that image. Try a different JPEG, PNG, WebP, GIF, or AVIF."
                .to_string()
        }
        401 => "Your session has ended. Sign in again, then retry the upload.".to_string(),
        403 => "Only officers and admins can upload images.".to_string(),
        413 => "The image is too large, or your upload space is full.".to_string(),
        other => format!("Upload failed: HTTP {other}"),
    }
}

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-articles-list";
/// The header's "New article" button.
const NEW_ID: &str = "articles-new";
/// The empty card's "New article" button.
const NEW_EMPTY_ID: &str = "articles-new-empty";

/// Page CSS: the editor's long text box, the cover preview and the slug cell.
const PAGE_CSS: &str = r#"
    textarea.field-input.article-editor-content {
        min-height: calc(var(--hit-target-min) * 7);
        font-family: var(--font-mono);
        font-size: var(--text-sm);
    }
    .article-cover-preview {
        display: block;
        max-width: 100%;
        max-height: calc(var(--hit-target-min) * 4);
        border: 1px solid var(--border);
        border-radius: var(--radius-card);
        object-fit: cover;
    }
    .article-slug {
        font-family: var(--font-mono);
        font-size: var(--text-sm);
        overflow-wrap: anywhere;
    }
"#;

/// URL-safe slug: lowercase, hyphens, no spaces.
fn slugify(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if (c == '-' || c == '_' || c.is_whitespace()) && !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn edit_button_id(id: &str) -> String {
    article_field_id(Some(id), "edit")
}

fn actions_id(id: &str) -> String {
    article_field_id(Some(id), "actions")
}

/// The pager line of an offset page. The API has no total, so it says the
/// range of this page and its number, never "of N".
fn page_meta(page: u32, shown: usize) -> String {
    if shown == 0 {
        return format!("No articles on page {}", page + 1);
    }
    let first = page as usize * PAGE_SIZE as usize + 1;
    let last = first + shown - 1;
    if page == 0 && shown < PAGE_SIZE as usize {
        if shown == 1 {
            "1 article".to_string()
        } else {
            format!("{shown} articles")
        }
    } else {
        format!("Articles {first} to {last} · Page {}", page + 1)
    }
}

/// Field errors of the editor, in plain words. Empty when the field is fine.
fn editor_errors(title: &str, slug: &str, content: &str) -> (String, String, String) {
    let title_error = if title.trim().is_empty() {
        "Add a title.".to_string()
    } else {
        String::new()
    };
    let slug_error = if slugify(slug).is_empty() {
        "Add a slug with letters or numbers.".to_string()
    } else {
        String::new()
    };
    let content_error = if content.trim().is_empty() {
        "Add the article text.".to_string()
    } else {
        String::new()
    };
    (title_error, slug_error, content_error)
}

#[component]
pub fn AdminArticles() -> Element {
    let auth = use_auth();
    let is_admin = auth().is_admin();
    let mut page = use_signal(|| 0u32);
    let mut articles = use_api_with::<Vec<Article>>(move || {
        let offset = page() * PAGE_SIZE;
        format!("/api/articles/admin/all?limit={PAGE_SIZE}&offset={offset}")
    });
    let mut toast = use_toast();

    let mut modal = ModalController::<String>::new();
    let mut form_title = use_signal(String::new);
    let mut form_slug = use_signal(String::new);
    let mut form_content = use_signal(String::new);
    let mut form_summary = use_signal(String::new);
    let mut form_cover = use_signal(String::new);
    // Record id of the article being edited (None for a new one). Feeds DOM ids.
    let mut form_id = use_signal(|| None::<String>);
    let mut cover_uploading = use_signal(|| false);
    // Text of the always-rendered cover upload status region.
    let mut cover_status = use_signal(String::new);
    // True when cover_status is a client-side reject (unsupported type / over size).
    let mut cover_status_error = use_signal(|| false);
    // Bumped whenever the editor opens or closes, so a finished upload from an
    // earlier session of the editor does not overwrite the current cover field.
    let mut editor_session = use_signal(|| 0u64);
    let mut title_error = use_signal(String::new);
    let mut slug_error = use_signal(String::new);
    let mut content_error = use_signal(String::new);
    // The dialog's live line: a failed save with the server's reason.
    let mut form_error = use_signal(|| None::<String>);
    // Id of the button that opened the editor, for focus on Cancel.
    let mut opener = use_signal(|| None::<String>);

    let mut delete_modal = ModalController::<Article>::new();

    let mut reset_status = move || {
        cover_uploading.set(false);
        cover_status.set(String::new());
        cover_status_error.set(false);
        title_error.set(String::new());
        slug_error.set(String::new());
        content_error.set(String::new());
        form_error.set(None);
        editor_session += 1;
    };

    let mut open_create = move |from: &'static str| {
        form_title.set(String::new());
        form_slug.set(String::new());
        form_content.set(String::new());
        form_summary.set(String::new());
        form_cover.set(String::new());
        form_id.set(None);
        reset_status();
        opener.set(Some(from.to_string()));
        modal.show_empty();
    };

    let mut open_edit = move |a: Article| {
        opener.set(Some(edit_button_id(&a.id)));
        form_title.set(a.title);
        form_slug.set(a.slug.clone());
        form_content.set(a.content_markdown);
        form_summary.set(a.summary.unwrap_or_default());
        form_cover.set(a.cover_image_url.unwrap_or_default());
        form_id.set(Some(a.id));
        reset_status();
        modal.show(a.slug);
    };

    let mut open_delete = move |a: Article| {
        delete_modal.show(a);
    };

    let on_close = move |_| {
        editor_session += 1;
        modal.close();
    };

    let cover_file_id = article_field_id(form_id().as_deref(), "cover-file");

    let on_cover_file_change = {
        let input_id = cover_file_id.clone();
        move |_e: Event<FormData>| {
            let Some(file) = selected_file(&input_id) else {
                return;
            };
            clear_file_input(&input_id);
            if let Err(msg) = validate_image_upload(&file.type_(), file.size()) {
                cover_status.set(msg.to_string());
                cover_status_error.set(true);
                return;
            }
            let session = editor_session();
            cover_uploading.set(true);
            cover_status_error.set(false);
            cover_status.set("Uploading cover image...".to_string());
            spawn(async move {
                let result = post_file("/api/upload/image", &file).await;
                if editor_session() != session || !modal.is_open() {
                    return;
                }
                cover_uploading.set(false);
                match result {
                    Ok(url) => {
                        form_cover.set(url);
                        cover_status
                            .set("Cover image uploaded. Save the article to keep it.".to_string());
                        toast.show(Toast::success("Cover image uploaded."));
                    }
                    Err(failure) => {
                        let msg = match failure {
                            UploadFailure::Status(status) => image_upload_failure_message(status),
                            UploadFailure::Message(msg) => msg.to_string(),
                        };
                        cover_status.set(msg.clone());
                        cover_status_error.set(true);
                        toast.show(Toast::error(msg));
                    }
                }
            });
        }
    };

    let on_submit = move |_| {
        let title = form_title().trim().to_string();
        let slug = slugify(&form_slug());
        let content = form_content().trim().to_string();
        let (t_err, s_err, c_err) = editor_errors(&title, &slug, &content);
        let invalid = !(t_err.is_empty() && s_err.is_empty() && c_err.is_empty());
        title_error.set(t_err);
        slug_error.set(s_err);
        content_error.set(c_err);
        if invalid {
            let message =
                "Title, a valid slug (letters/numbers/hyphens), and content are required.";
            form_error.set(Some(message.to_string()));
            toast.show(Toast::error(message));
            return;
        }
        form_error.set(None);
        // Keep form in sync with what we send
        form_slug.set(slug.clone());
        let summary_val = form_summary().trim().to_string();
        let cover_val = form_cover().trim().to_string();
        let edit_slug = modal.get_target();

        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result: Result<Article, _> = if let Some(old_slug) = edit_slug {
                let body = UpdateArticleBody {
                    title: Some(title),
                    slug: Some(slug),
                    content_markdown: Some(content),
                    summary: Some(if summary_val.is_empty() {
                        None
                    } else {
                        Some(summary_val)
                    }),
                    cover_image_url: Some(if cover_val.is_empty() {
                        None
                    } else {
                        Some(cover_val)
                    }),
                };
                client
                    .put_json::<_, Article>(&format!("/api/articles/{old_slug}"), &body)
                    .await
            } else {
                let body = CreateArticleBody {
                    title,
                    slug,
                    content_markdown: content,
                    summary: if summary_val.is_empty() {
                        None
                    } else {
                        Some(summary_val)
                    },
                    cover_image_url: if cover_val.is_empty() {
                        None
                    } else {
                        Some(cover_val)
                    },
                };
                client.post_json::<_, Article>("/api/articles", &body).await
            };

            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Article saved."));
                    modal.close();
                    page.set(0);
                    articles.refresh += 1;
                }
                Err(e) => {
                    let message = format!("Could not save the article. {e}");
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
        let slug = target.slug.clone();
        delete_modal.close();
        spawn(async move {
            let client = ApiClient::web();
            match client.delete(&format!("/api/articles/{slug}")).await {
                Ok(_) => {
                    toast.show(Toast::success("Article deleted."));
                    articles.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Could not delete the article. {e}")));
                }
            }
        });
    };

    let on_cancel_delete = move |_| {
        delete_modal.close();
    };

    let on_set_published = move |(slug, publish): (String, bool)| {
        spawn(async move {
            let client = ApiClient::web();
            let action = if publish { "publish" } else { "unpublish" };
            match client
                .post_json_empty(
                    &format!("/api/articles/{slug}/{action}"),
                    &serde_json::Value::Null,
                )
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success(if publish {
                        "Article published."
                    } else {
                        "Article unpublished."
                    }));
                    articles.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Could not {action} the article. {e}"))),
            }
        });
    };

    let list = {
        let data = articles.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_pending(&articles, "articles"),
            Some(list) => {
                let current = page();
                let shown = list.len();
                let has_next = (shown as u32) >= PAGE_SIZE;
                let at_start = current == 0;
                let meta = page_meta(current, shown);
                let rows = list.clone();
                rsx! {
                    if list.is_empty() && at_start {
                        EmptyCard {
                            title: "No articles yet",
                            message: "Published articles appear on the blog. Start a draft; it stays hidden until you publish it.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    id: NEW_EMPTY_ID,
                                    class: "btn is-primary",
                                    onclick: move |_| open_create(NEW_EMPTY_ID),
                                    "New article"
                                }
                            },
                        }
                    } else if list.is_empty() {
                        EmptyCard {
                            title: "No articles on this page",
                            message: "The list ends on an earlier page. Go back to the first page to see the newest articles.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-ghost",
                                    onclick: move |_| {
                                        page.set(0);
                                        show_list_top(LIST_ID);
                                    },
                                    "First page"
                                }
                            },
                        }
                    } else {
                        ArticlesTable {
                            rows,
                            can_delete: is_admin,
                            on_edit: move |a: Article| open_edit(a),
                            on_delete: move |a: Article| open_delete(a),
                            on_set_published,
                        }
                    }
                    // Offset pages with no total: the shared pager's markup and
                    // classes, with a range line that never claims a total.
                    div { id: "articles-pager", class: "pager",
                        p { id: "articles-pager-meta", class: "pager-meta", role: "status", "{meta}" }
                        if !at_start || has_next {
                            nav { class: "pager-pages", aria_label: "Pagination",
                                button {
                                    r#type: "button",
                                    id: "articles-pager-prev",
                                    class: if at_start { "pager-step is-disabled" } else { "pager-step" },
                                    aria_label: "Previous page",
                                    aria_disabled: if at_start { "true" },
                                    onclick: move |_| {
                                        if page() > 0 {
                                            page -= 1;
                                            show_list_top(LIST_ID);
                                        }
                                    },
                                    "Previous"
                                }
                                button {
                                    r#type: "button",
                                    id: "articles-pager-next",
                                    class: if has_next { "pager-step" } else { "pager-step is-disabled" },
                                    aria_label: "Next page",
                                    aria_disabled: if !has_next { "true" },
                                    onclick: move |_| {
                                        if has_next {
                                            page += 1;
                                            show_list_top(LIST_ID);
                                        }
                                    },
                                    "Next"
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    let editing = modal.get_target().is_some();
    let delete_target = delete_modal.get_target();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-articles-css", css: PAGE_CSS }
        PageShell { width: PageWidth::Wide, class: "admin-articles-page",
            PageHeader {
                title: "Articles",
                lead: "Long posts for the blog. A draft stays hidden until you publish it.",
                actions: rsx! {
                    button {
                        r#type: "button",
                        id: NEW_ID,
                        class: "btn is-primary",
                        onclick: move |_| open_create(NEW_ID),
                        "New article"
                    }
                },
            }
            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Articles",
                {list}
            }
        }

        FormModal {
            title: if editing { "Edit article".to_string() } else { "New article".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            submit_label: if editing { None } else { Some("Create".to_string()) },
            error: form_error(),
            restore_focus_id: opener(),
            on_close: on_close,
            on_submit: on_submit,
            ArticleFields {
                record_id: form_id(),
                title: form_title,
                slug: form_slug,
                summary: form_summary,
                cover: form_cover,
                content: form_content,
                title_error: title_error(),
                slug_error: slug_error(),
                content_error: content_error(),
                cover_uploading: cover_uploading(),
                cover_status: cover_status(),
                cover_status_error: cover_status_error(),
                on_cover_file_change,
            }
        }

        ConfirmDialog {
            title: "Delete article".to_string(),
            message: format!(
                "Delete \"{}\"? It leaves the blog if it was published. This cannot be undone.",
                delete_target.as_ref().map(|a| a.title.clone()).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            confirm_label: "Delete".to_string(),
            restore_focus_id: delete_target.as_ref().map(|a| row_menu_toggle_id(&actions_id(&a.id))),
            on_confirm: on_confirm_delete,
            on_cancel: on_cancel_delete,
        }
    }
}

/// The rows: title, Published or Draft, slug, date, then Edit in the row and
/// Publish/Unpublish and Delete behind "Actions" on phone.
#[component]
fn ArticlesTable(
    rows: Vec<Article>,
    can_delete: bool,
    on_edit: EventHandler<Article>,
    on_delete: EventHandler<Article>,
    on_set_published: EventHandler<(String, bool)>,
) -> Element {
    rsx! {
        DataTable {
            headers: vec!["Title", "Status", "Slug", "Created", "Actions"],
            num_cols: vec![3],
            cards: true,
            label: "Articles".to_string(),
            for a in rows {
                {
                    let edit = a.clone();
                    let delete = a.clone();
                    let slug = a.slug.clone();
                    let title = a.title.clone();
                    let publish = !a.published;
                    let toggle_word = if a.published { "Unpublish" } else { "Publish" };
                    rsx! {
                        tr { key: "{a.id}",
                            td { class: "cell-who", "{a.title}" }
                            td { class: "cell-badge",
                                if a.published {
                                    Pill { tone: PillTone::Ok, "Published" }
                                } else {
                                    Pill { tone: PillTone::Warn, "Draft" }
                                }
                            }
                            td { class: "cell-meta", code { class: "article-slug", "{a.slug}" } }
                            td { class: "cell-meta num", AdminDate { iso: a.created_at.clone() } }
                            td { class: "cell-actions",
                                RowActions {
                                    id: actions_id(&a.id),
                                    label: "Actions for {title}",
                                    inline_on_desktop: true,
                                    more: rsx! {
                                        button {
                                            r#type: "button",
                                            class: if publish { "btn is-primary" } else { "btn is-ghost" },
                                            aria_label: "{toggle_word} {title}",
                                            onclick: move |_| on_set_published.call((slug.clone(), publish)),
                                            "{toggle_word}"
                                        }
                                        if can_delete {
                                            button {
                                                r#type: "button",
                                                class: "btn is-danger",
                                                aria_label: "Delete {title}",
                                                onclick: move |_| on_delete.call(delete.clone()),
                                                "Delete"
                                            }
                                        }
                                    },
                                    button {
                                        r#type: "button",
                                        id: edit_button_id(&a.id),
                                        class: "btn is-ghost",
                                        aria_label: "Edit {title}",
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

/// The editor fields. Ids come from the record id plus the field name; each
/// required field has an always-rendered error line tied to its control, and
/// the cover upload has an always-rendered status line.
#[component]
fn ArticleFields(
    record_id: Option<String>,
    title: Signal<String>,
    slug: Signal<String>,
    summary: Signal<String>,
    cover: Signal<String>,
    content: Signal<String>,
    title_error: String,
    slug_error: String,
    content_error: String,
    cover_uploading: bool,
    cover_status: String,
    cover_status_error: bool,
    on_cover_file_change: EventHandler<Event<FormData>>,
) -> Element {
    let key = record_id.as_deref();
    let id = |field: &str| article_field_id(key, field);
    let (title_id, title_error_id) = (id("title"), id("title-error"));
    let (slug_id, slug_hint_id, slug_error_id) = (id("slug"), id("slug-hint"), id("slug-error"));
    let summary_id = id("summary");
    let cover_url_id = id("cover-url");
    let cover_file_id = id("cover-file");
    let cover_hint_id = id("cover-hint");
    let cover_status_id = id("cover-status");
    let (content_id, content_hint_id, content_error_id) =
        (id("content"), id("content-hint"), id("content-error"));
    let cover_value = cover();
    let preview = cover_preview_src(&cover_value).map(str::to_string);
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: "{title_id}", "Title" }
            input {
                id: "{title_id}",
                class: "field-input",
                r#type: "text",
                value: "{title}",
                aria_invalid: if !title_error.is_empty() { "true" },
                aria_describedby: "{title_error_id}",
                oninput: move |e| title.set(e.value()),
            }
            FieldError { id: title_error_id.clone(), text: title_error.clone() }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: "{slug_id}", "Slug" }
            input {
                id: "{slug_id}",
                class: "field-input",
                r#type: "text",
                value: "{slug}",
                placeholder: "my-article-slug",
                aria_invalid: if !slug_error.is_empty() { "true" },
                aria_describedby: "{slug_hint_id} {slug_error_id}",
                oninput: move |e| slug.set(e.value()),
            }
            p { id: "{slug_hint_id}", class: "field-hint",
                "Letters, numbers and hyphens. The article lives at /blog/ plus the slug."
            }
            FieldError { id: slug_error_id.clone(), text: slug_error.clone() }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: "{summary_id}", "Summary (optional)" }
            input {
                id: "{summary_id}",
                class: "field-input",
                r#type: "text",
                value: "{summary}",
                oninput: move |e| summary.set(e.value()),
            }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: "{cover_url_id}", "Cover image URL (optional)" }
            input {
                id: "{cover_url_id}",
                class: "field-input",
                r#type: "text",
                value: "{cover}",
                oninput: move |e| cover.set(e.value()),
            }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: "{cover_file_id}", "Upload cover image" }
            input {
                id: "{cover_file_id}",
                class: "field-input",
                r#type: "file",
                accept: IMAGE_UPLOAD_ACCEPT,
                disabled: cover_uploading,
                aria_describedby: "{cover_hint_id} {cover_status_id}",
                onchange: move |e| on_cover_file_change.call(e),
            }
            p { id: "{cover_hint_id}", class: "field-hint",
                "JPEG, PNG, WebP, GIF, or AVIF, up to 5 MB. The uploaded path fills the URL field above."
            }
            p {
                id: "{cover_status_id}",
                class: if cover_status_error { "field-error" } else { "field-hint" },
                role: "status",
                if cover_status_error && !cover_status.is_empty() {
                    span { class: "field-error-icon", aria_hidden: "true" }
                }
                "{cover_status}"
            }
            if let Some(src) = preview {
                img { class: "article-cover-preview", src: "{src}", alt: "Cover image preview" }
            }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: "{content_id}", "Content (Markdown)" }
            textarea {
                id: "{content_id}",
                class: "field-input article-editor-content",
                value: "{content}",
                aria_invalid: if !content_error.is_empty() { "true" },
                aria_describedby: "{content_hint_id} {content_error_id}",
                oninput: move |e| content.set(e.value()),
            }
            p { id: "{content_hint_id}", class: "field-hint",
                "Markdown: headings, lists, links and code blocks render on the blog."
            }
            FieldError { id: content_error_id.clone(), text: content_error.clone() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::admin_shared::test_page::render_admin_route;
    use crate::components::ui::test_support::assert_sizes_from_tokens;

    fn render(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// Every `<label for="x">` has a control with `id="x"`.
    fn assert_labels_tied(html: &str) {
        let mut seen = 0;
        for part in html.split("<label").skip(1) {
            let head = part.split('>').next().unwrap_or_default();
            let target = head
                .split("for=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_else(|| panic!("label without for: {head}"));
            assert!(
                html.contains(&format!("id=\"{target}\"")),
                "no control for {target}: {html}"
            );
            seen += 1;
        }
        assert!(seen > 0, "{html}");
    }

    fn row(id: &str, title: &str, published: bool) -> Article {
        Article {
            id: id.into(),
            slug: format!("{id}-slug"),
            title: title.into(),
            content_markdown: "Body".into(),
            summary: None,
            cover_image_url: None,
            published,
            published_at: None,
            created_at: "2026-11-10T19:00:00Z".into(),
        }
    }

    #[test]
    fn page_has_one_h1_and_a_labelled_wait_in_the_wide_shell() {
        let html = render_admin_route("/admin/articles", Some(scuffed_types::OrgRole::Officer));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-articles-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Articles</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains(
                "<section id=\"admin-articles-list\" class=\"admin-list pager-target\" tabindex=\"-1\" aria-label=\"Articles\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading articles...</p>"),
            "{html}"
        );
        assert!(!html.contains('\u{2014}'), "{html}");
    }

    #[test]
    fn rows_say_draft_or_published_and_fold_the_rest() {
        fn view() -> Element {
            rsx! {
                ArticlesTable {
                    rows: vec![row("a1", "Tank basics", false), row("a2", "Scrim recap", true)],
                    can_delete: false,
                    on_edit: |_| {},
                    on_delete: |_| {},
                    on_set_published: |_| {},
                }
            }
        }
        let html = render(view);
        assert!(html.contains("data-table-scroll is-cards"), "{html}");
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--warn\">Draft</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--ok\">Published</span>"),
            "{html}"
        );
        assert!(
            html.contains("aria-label=\"Publish Tank basics\""),
            "{html}"
        );
        assert!(
            html.contains("aria-label=\"Unpublish Scrim recap\""),
            "{html}"
        );
        assert!(html.contains("id=\"article-a1-edit\""), "{html}");
        // Officers cannot delete: no Delete button at all.
        assert!(!html.contains(">Delete</button>"), "{html}");
    }

    #[test]
    fn editor_fields_are_labelled_with_always_rendered_status_lines() {
        fn view() -> Element {
            let title = use_signal(String::new);
            let slug = use_signal(String::new);
            let summary = use_signal(String::new);
            let cover = use_signal(String::new);
            let content = use_signal(String::new);
            rsx! {
                ArticleFields {
                    record_id: Some("article:a1".to_string()),
                    title,
                    slug,
                    summary,
                    cover,
                    content,
                    title_error: "Add a title.".to_string(),
                    slug_error: String::new(),
                    content_error: String::new(),
                    cover_uploading: false,
                    cover_status: String::new(),
                    cover_status_error: false,
                    on_cover_file_change: |_| {},
                }
            }
        }
        let html = render(view);
        assert_labels_tied(&html);
        assert_eq!(html.matches("<label").count(), 6, "{html}");
        assert!(
            html.contains(
                "<p id=\"article-article-a1-cover-status\" class=\"field-hint\" role=\"status\"></p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"field-error\" id=\"article-article-a1-slug-error\"></p>"),
            "{html}"
        );
        assert!(html.contains("Add a title.</p>"), "{html}");
        assert!(!html.contains("style="), "{html}");
    }

    #[test]
    fn editor_errors_and_pager_line() {
        let (t, s, c) = editor_errors(" ", "!!", "");
        assert!(!t.is_empty() && !s.is_empty() && !c.is_empty());
        let (t, s, c) = editor_errors("Tank basics", "Tank Basics", "Body");
        assert!(t.is_empty() && s.is_empty() && c.is_empty());
        assert_eq!(slugify("Tank Basics!"), "tank-basics");
        assert_eq!(page_meta(0, 1), "1 article");
        assert_eq!(page_meta(0, 12), "12 articles");
        assert_eq!(page_meta(0, 50), "Articles 1 to 50 · Page 1");
        assert_eq!(page_meta(1, 3), "Articles 51 to 53 · Page 2");
        assert_eq!(page_meta(2, 0), "No articles on page 3");
    }

    #[test]
    fn page_css_sizes_come_from_tokens() {
        assert_sizes_from_tokens(PAGE_CSS);
    }

    #[test]
    fn field_id_uses_record_id_and_field() {
        assert_eq!(
            article_field_id(Some("article:abc123"), "cover-file"),
            "article-article-abc123-cover-file"
        );
    }

    #[test]
    fn field_id_for_new_article() {
        assert_eq!(article_field_id(None, "cover-url"), "article-new-cover-url");
        assert_eq!(
            article_field_id(Some("  "), "cover-url"),
            "article-new-cover-url"
        );
    }

    #[test]
    fn field_ids_differ_per_record() {
        assert_ne!(
            article_field_id(Some("a1"), "cover-file"),
            article_field_id(Some("a2"), "cover-file")
        );
    }

    #[test]
    fn preview_allows_uploads_and_https() {
        assert_eq!(
            cover_preview_src(" /uploads/images/m1/x.png "),
            Some("/uploads/images/m1/x.png")
        );
        assert_eq!(
            cover_preview_src("https://cdn.example.com/c.webp"),
            Some("https://cdn.example.com/c.webp")
        );
        assert_eq!(
            cover_preview_src("HTTPS://cdn.example.com/c.webp"),
            Some("HTTPS://cdn.example.com/c.webp")
        );
    }

    #[test]
    fn preview_rejects_http_and_https_without_host() {
        for bad in [
            "http://cdn.example.com/c.webp",
            "HTTP://cdn.example.com/c.webp",
            "https://",
            "https:///a.png",
            "https://?x",
            "https://#x",
        ] {
            assert_eq!(cover_preview_src(bad), None, "{bad}");
        }
    }

    #[test]
    fn preview_rejects_dotdot_and_avatar_paths() {
        for bad in [
            "/uploads/images/m1/../x.png",
            "/uploads/images/../avatars/m/x.png",
            "/uploads/avatars/member_abc/a.png",
        ] {
            assert_eq!(cover_preview_src(bad), None, "{bad}");
        }
    }

    #[test]
    fn preview_rejects_other_values() {
        for bad in [
            "",
            "/uploads/",
            "/uploads/images/",
            "/uploads/images/a.png",
            "javascript:alert(1)",
            "data:image/png;base64,AA",
            "//evil.example/x.png",
            "/uploads/a b.png",
            "/uploads/other/m/x.png",
            "/other/x.png",
            "uploads/images/m1/x.png",
        ] {
            assert_eq!(cover_preview_src(bad), None, "{bad}");
        }
    }

    #[test]
    fn upload_failure_copy() {
        assert!(image_upload_failure_message(403).contains("officers"));
        assert!(image_upload_failure_message(413).contains("too large"));
        assert_eq!(image_upload_failure_message(500), "Upload failed: HTTP 500");
        for status in [400, 401, 403, 413, 500] {
            assert!(!image_upload_failure_message(status).contains('\u{2014}'));
        }
    }
}
