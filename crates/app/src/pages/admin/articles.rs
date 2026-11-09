use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{ConfirmDialog, DataTable, FormModal, Toast, admin_pending, use_toast};
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

#[derive(Debug, Clone, Deserialize)]
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

/// Source for the cover preview: an uploaded `/uploads/...` path or an
/// `http(s)` URL. Anything else (empty, `javascript:`, `//host`) has no preview.
fn cover_preview_src(url: &str) -> Option<&str> {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("/uploads/") {
        let clean = !rest.is_empty()
            && !rest
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '\\');
        return clean.then_some(url);
    }
    crate::util::http_href(url)
}

/// Toast copy when `POST /api/upload/image` answers with an error status.
fn image_upload_failure_message(status: u16) -> String {
    match status {
        400 => "The server could not read that image. Try a different JPEG, PNG, WebP, or GIF."
            .to_string(),
        401 => "Your session has ended. Sign in again, then retry the upload.".to_string(),
        403 => "Only officers and admins can upload images.".to_string(),
        413 => "The image is too large, or your upload space is full.".to_string(),
        other => format!("Upload failed: HTTP {other}"),
    }
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
    // Bumped whenever the editor opens or closes, so a finished upload from an
    // earlier session of the editor does not overwrite the current cover field.
    let mut editor_session = use_signal(|| 0u64);

    let mut delete_modal = ModalController::<Article>::new();

    let open_create = move |_| {
        form_title.set(String::new());
        form_slug.set(String::new());
        form_content.set(String::new());
        form_summary.set(String::new());
        form_cover.set(String::new());
        form_id.set(None);
        cover_uploading.set(false);
        cover_status.set(String::new());
        editor_session += 1;
        modal.show_empty();
    };

    let mut open_edit = move |a: Article| {
        form_title.set(a.title);
        form_slug.set(a.slug.clone());
        form_content.set(a.content_markdown);
        form_summary.set(a.summary.unwrap_or_default());
        form_cover.set(a.cover_image_url.unwrap_or_default());
        form_id.set(Some(a.id));
        cover_uploading.set(false);
        cover_status.set(String::new());
        editor_session += 1;
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
    let cover_url_id = article_field_id(form_id().as_deref(), "cover-url");
    let cover_hint_id = article_field_id(form_id().as_deref(), "cover-hint");

    let on_cover_file_change = {
        let input_id = cover_file_id.clone();
        move |_e: Event<FormData>| {
            let Some(file) = selected_file(&input_id) else {
                return;
            };
            clear_file_input(&input_id);
            if let Err(msg) = validate_image_upload(&file.type_(), file.size()) {
                cover_status.set(msg.to_string());
                return;
            }
            let session = editor_session();
            cover_uploading.set(true);
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
                        toast.show(Toast::error(msg));
                    }
                }
            });
        }
    };

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

    let on_submit = move |_| {
        let title = form_title().trim().to_string();
        let slug = slugify(&form_slug());
        let content = form_content().trim().to_string();
        if title.is_empty() || slug.is_empty() || content.is_empty() {
            toast.show(Toast::error(
                "Title, a valid slug (letters/numbers/hyphens), and content are required.",
            ));
            return;
        }
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
                    toast.show(Toast::error(format!("Failed to save: {e}")));
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
                    toast.show(Toast::error(format!("Failed to delete: {e}")));
                }
            }
        });
    };

    let on_cancel_delete = move |_| {
        delete_modal.close();
    };

    let on_prev = move |_| {
        if page() > 0 {
            page -= 1;
        }
    };

    let on_next = {
        let articles_data = articles.data;
        move |_| {
            let data = articles_data.read();
            if let Some(Some(list)) = data.as_ref()
                && (list.len() as u32) >= PAGE_SIZE
            {
                page += 1;
            }
        }
    };

    rsx! {
        div { class: "admin-toolbar",
            h1 { "Articles" }
            button { class: "btn-add", onclick: open_create, "+ New Article" }
        }

        {
            let data = articles.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => admin_pending(&articles, "articles"),
                Some(list) if list.is_empty() && page() == 0 => rsx! {
                    p { class: "empty-state", "No articles yet." }
                },
                Some(list) => {
                    let current = page() + 1;
                    let has_next = (list.len() as u32) >= PAGE_SIZE;
                    rsx! {
                        if list.is_empty() {
                            p { class: "empty-state", "No articles on this page." }
                        } else {
                            DataTable { headers: vec!["Title", "Slug", "Status", "Created", "Actions"],
                                for a in list.iter() {
                                    {
                                        let ae = a.clone();
                                        let ad = a.clone();
                                        let ap = a.clone();
                                        let status_class = if a.published { "active" } else { "inactive" };
                                        let status_label = if a.published { "Published" } else { "Draft" };
                                        let date: String = a.created_at.chars().take(10).collect();
                                        rsx! {
                                            tr { key: "{a.id}",
                                                td { "{a.title}" }
                                                td { code { "{a.slug}" } }
                                                td { span { class: "status-pill {status_class}", "{status_label}" } }
                                                td { "{date}" }
                                                td {
                                                    div { class: "row-actions",
                                                        button {
                                                            class: "row-btn",
                                                            onclick: move |_| open_edit(ae.clone()),
                                                            "Edit"
                                                        }
                                                        {
                                                            let ap2 = ap.clone();
                                                            if ap2.published {
                                                                rsx! {
                                                                    button {
                                                                        class: "row-btn",
                                                                        onclick: {
                                                                            let slug = ap2.slug.clone();
                                                                            move |_| {
                                                                                let slug = slug.clone();
                                                                                spawn(async move {
                                                                                    let client = ApiClient::web();
                                                                                    match client.post_json_empty(&format!("/api/articles/{slug}/unpublish"), &serde_json::Value::Null).await {
                                                                                        Ok(_) => {
                                                                                            toast.show(Toast::success("Article unpublished."));
                                                                                            articles.refresh += 1;
                                                                                        }
                                                                                        Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
                                                                                    }
                                                                                });
                                                                            }
                                                                        },
                                                                        "Unpublish"
                                                                    }
                                                                }
                                                            } else {
                                                                rsx! {
                                                                    button {
                                                                        class: "row-btn",
                                                                        onclick: {
                                                                            let slug = ap2.slug.clone();
                                                                            move |_| {
                                                                                let slug = slug.clone();
                                                                                spawn(async move {
                                                                                    let client = ApiClient::web();
                                                                                    match client.post_json_empty(&format!("/api/articles/{slug}/publish"), &serde_json::Value::Null).await {
                                                                                        Ok(_) => {
                                                                                            toast.show(Toast::success("Article published!"));
                                                                                            articles.refresh += 1;
                                                                                        }
                                                                                        Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
                                                                                    }
                                                                                });
                                                                            }
                                                                        },
                                                                        "Publish"
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        if is_admin {
                                                            button {
                                                                class: "row-btn danger",
                                                                onclick: move |_| open_delete(ad.clone()),
                                                                "Delete"
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
                        if page() > 0 || has_next {
                            div { class: "pagination",
                                button {
                                    disabled: page() == 0,
                                    onclick: on_prev,
                                    "Previous"
                                }
                                span { class: "page-info", "Page {current}" }
                                button {
                                    disabled: !has_next,
                                    onclick: on_next,
                                    "Next"
                                }
                            }
                        }
                    }
                },
            }
        }

        FormModal {
            title: if modal.get_target().is_some() { "Edit Article".to_string() } else { "New Article".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,

            div { class: "form-field",
                label { class: "form-label", "Title" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{form_title}",
                    oninput: move |e| form_title.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Slug" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{form_slug}",
                    placeholder: "my-article-slug",
                    oninput: move |e| form_slug.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Summary (optional)" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{form_summary}",
                    oninput: move |e| form_summary.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", r#for: "{cover_url_id}", "Cover Image URL (optional)" }
                input {
                    id: "{cover_url_id}",
                    class: "form-input",
                    r#type: "text",
                    value: "{form_cover}",
                    oninput: move |e| form_cover.set(e.value()),
                }
                label { class: "form-label", r#for: "{cover_file_id}", style: "margin-top:0.6rem;",
                    "Upload cover image"
                }
                input {
                    id: "{cover_file_id}",
                    r#type: "file",
                    accept: IMAGE_UPLOAD_ACCEPT,
                    disabled: cover_uploading(),
                    aria_describedby: "{cover_hint_id}",
                    onchange: on_cover_file_change,
                }
                p { id: "{cover_hint_id}", class: "form-hint",
                    "JPEG, PNG, WebP, or GIF, up to 5 MB. The uploaded path fills the URL field above."
                }
                p { class: "form-hint", role: "status", "{cover_status}" }
                if let Some(src) = cover_preview_src(&form_cover()) {
                    img {
                        src: "{src}",
                        alt: "Cover image preview",
                        style: "display:block;max-width:100%;max-height:180px;margin-top:0.5rem;border-radius:8px;border:1px solid var(--border);object-fit:cover;",
                    }
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Content (Markdown)" }
                textarea {
                    class: "form-textarea",
                    style: "min-height: 300px; font-family: monospace;",
                    value: "{form_content}",
                    oninput: move |e| form_content.set(e.value()),
                }
            }
        }

        ConfirmDialog {
            title: "Delete Article".to_string(),
            message: format!(
                "Are you sure you want to delete \"{}\"? This cannot be undone.",
                delete_modal.get_target().map(|a| a.title).unwrap_or_default()
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
    fn preview_allows_uploads_and_http() {
        assert_eq!(
            cover_preview_src(" /uploads/images/m1/x.png "),
            Some("/uploads/images/m1/x.png")
        );
        assert_eq!(
            cover_preview_src("https://cdn.example.com/c.webp"),
            Some("https://cdn.example.com/c.webp")
        );
    }

    #[test]
    fn preview_rejects_other_values() {
        for bad in [
            "",
            "/uploads/",
            "javascript:alert(1)",
            "data:image/png;base64,AA",
            "//evil.example/x.png",
            "/uploads/a b.png",
            "/other/x.png",
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
