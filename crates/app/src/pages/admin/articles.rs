use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{ConfirmDialog, DataTable, FormModal, Toast, use_toast};
use crate::hooks::{ModalController, use_api};
use scuffed_api_client::ApiClient;

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

#[component]
pub fn AdminArticles() -> Element {
    let mut articles = use_api::<Vec<Article>>("/api/articles/admin/all");
    let mut toast = use_toast();

    let mut modal = ModalController::<String>::new();
    let mut form_title = use_signal(String::new);
    let mut form_slug = use_signal(String::new);
    let mut form_content = use_signal(String::new);
    let mut form_summary = use_signal(String::new);
    let mut form_cover = use_signal(String::new);

    let mut delete_modal = ModalController::<Article>::new();

    let open_create = move |_| {
        form_title.set(String::new());
        form_slug.set(String::new());
        form_content.set(String::new());
        form_summary.set(String::new());
        form_cover.set(String::new());
        modal.show_empty();
    };

    let mut open_edit = move |a: Article| {
        form_title.set(a.title);
        form_slug.set(a.slug.clone());
        form_content.set(a.content_markdown);
        form_summary.set(a.summary.unwrap_or_default());
        form_cover.set(a.cover_image_url.unwrap_or_default());
        modal.show(a.slug);
    };

    let mut open_delete = move |a: Article| {
        delete_modal.show(a);
    };

    let on_close = move |_| {
        modal.close();
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
            } else if (c == '-' || c == '_' || c.is_whitespace()) && !prev_dash && !out.is_empty()
            {
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

    rsx! {
        div { class: "admin-toolbar",
            h1 { "Articles" }
            button { class: "btn-add", onclick: open_create, "+ New Article" }
        }

        {
            let data = articles.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No articles yet." }
                },
                Some(list) => rsx! {
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
                label { class: "form-label", "Cover Image URL (optional)" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{form_cover}",
                    oninput: move |e| form_cover.set(e.value()),
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
