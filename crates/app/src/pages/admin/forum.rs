use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{Toast, use_toast};
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumCategory {
    id: String,
    name: String,
    slug: String,
    description: Option<String>,
    sort_order: i32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumBoard {
    id: String,
    category_id: String,
    parent_board_id: Option<String>,
    name: String,
    slug: String,
    description: Option<String>,
    sort_order: i32,
    is_locked: bool,
    is_active: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumBoardNode {
    #[serde(flatten)]
    board: ForumBoard,
    sub_boards: Vec<ForumBoard>,
    #[allow(dead_code)]
    thread_count: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumCategoryNode {
    #[serde(flatten)]
    category: ForumCategory,
    boards: Vec<ForumBoardNode>,
}

#[derive(Serialize)]
struct CreateCategoryBody {
    name: String,
    slug: String,
    description: Option<String>,
    sort_order: i32,
}

#[derive(Serialize)]
struct CreateBoardBody {
    category_id: String,
    parent_board_id: Option<String>,
    name: String,
    slug: String,
    description: Option<String>,
    sort_order: i32,
}

fn slugify(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

const CSS: &str = r#"
    .af-page { padding: 1.5rem; max-width: 900px; }
    .af-title { font-family: var(--font-head); font-size: 1.5rem; margin: 0 0 1rem; color: var(--text); }
    .af-section { border: 1px solid var(--border); border-radius: 8px; padding: 1rem; margin-bottom: 1rem; background: var(--surface); }
    .af-cat { font-weight: 700; color: var(--accent); margin-bottom: 0.5rem; }
    .af-board { padding: 0.35rem 0 0.35rem 0.5rem; color: var(--text); font-size: 0.9rem; }
    .af-sub { padding-left: 1.5rem; color: var(--text-2); font-size: 0.85rem; }
    .af-form { display: grid; gap: 0.5rem; margin-top: 0.75rem; }
    .af-form input, .af-form select {
        background: var(--bg); border: 1px solid var(--border); color: var(--text);
        padding: 0.45rem 0.6rem; border-radius: 6px;
    }
    .af-form button {
        justify-self: start; background: var(--accent); color: var(--accent-fg);
        border: none; border-radius: 6px; padding: 0.45rem 0.9rem; font-weight: 600; cursor: pointer;
    }
    .af-hint { color: var(--text-3); font-size: 0.8rem; margin-bottom: 1rem; }
"#;

#[component]
pub fn AdminForum() -> Element {
    let mut toast = use_toast();
    let mut refresh = use_signal(|| 0u32);

    let tree = use_resource(move || {
        let _ = refresh();
        async move {
            ApiClient::web()
                .fetch::<Vec<ForumCategoryNode>>("/api/forum/tree")
                .await
                .ok()
        }
    });

    let mut cat_name = use_signal(String::new);
    let mut board_name = use_signal(String::new);
    let mut board_cat = use_signal(String::new);
    let mut board_parent = use_signal(String::new);

    rsx! {
        style { {CSS} }
        div { class: "af-page",
            h1 { class: "af-title", "Forum structure" }
            p { class: "af-hint",
                "Categories → boards → optional sub-boards. Example: Games / Overwatch / Strategy."
            }

            match tree.value()() {
                Some(Some(cats)) => rsx! {
                    for cat in cats.iter() {
                        div { class: "af-section",
                            div { class: "af-cat", "{cat.category.name} ({cat.category.slug})" }
                            for node in cat.boards.iter() {
                                div { class: "af-board",
                                    "• {node.board.name} — /forum/b/{node.board.slug}"
                                    if node.board.is_locked { " [locked]" }
                                }
                                for sub in node.sub_boards.iter() {
                                    div { class: "af-sub",
                                        "↳ {sub.name} — /forum/b/{sub.slug}"
                                    }
                                }
                            }
                        }
                    }

                    // Create category
                    div { class: "af-section",
                        div { class: "af-cat", "New category" }
                        div { class: "af-form",
                            input {
                                placeholder: "Name (e.g. Games)",
                                value: "{cat_name}",
                                oninput: move |e| cat_name.set(e.value()),
                            }
                            button {
                                onclick: move |_| {
                                    let name = cat_name().trim().to_string();
                                    if name.is_empty() {
                                        toast.show(Toast::error("Name required"));
                                        return;
                                    }
                                    let slug = slugify(&name);
                                    spawn(async move {
                                        let body = CreateCategoryBody {
                                            name,
                                            slug,
                                            description: None,
                                            sort_order: 10,
                                        };
                                        match ApiClient::web()
                                            .post_json::<_, serde_json::Value>("/api/forum/categories", &body)
                                            .await
                                        {
                                            Ok(_) => {
                                                cat_name.set(String::new());
                                                refresh.set(refresh() + 1);
                                                toast.show(Toast::success("Category created"));
                                            }
                                            Err(e) => toast.show(Toast::error(e.to_string())),
                                        }
                                    });
                                },
                                "Add category"
                            }
                        }
                    }

                    // Create board / sub-board
                    div { class: "af-section",
                        div { class: "af-cat", "New board or sub-board" }
                        div { class: "af-form",
                            select {
                                onchange: move |e| board_cat.set(e.value()),
                                option { value: "", "— Category —" }
                                for cat in cats.iter() {
                                    option {
                                        value: "{cat.category.id}",
                                        selected: board_cat() == cat.category.id,
                                        "{cat.category.name}"
                                    }
                                }
                            }
                            select {
                                onchange: move |e| board_parent.set(e.value()),
                                option { value: "", "— Top-level board (no parent) —" }
                                for cat in cats.iter() {
                                    for node in cat.boards.iter() {
                                        option {
                                            value: "{node.board.id}",
                                            selected: board_parent() == node.board.id,
                                            "{cat.category.name} / {node.board.name}"
                                        }
                                    }
                                }
                            }
                            input {
                                placeholder: "Board name (e.g. Overwatch or Strategy)",
                                value: "{board_name}",
                                oninput: move |e| board_name.set(e.value()),
                            }
                            button {
                                onclick: move |_| {
                                    let category_id = board_cat();
                                    let parent = board_parent();
                                    let name = board_name().trim().to_string();
                                    if category_id.is_empty() || name.is_empty() {
                                        toast.show(Toast::error("Category and name required"));
                                        return;
                                    }
                                    let slug = slugify(&name);
                                    let parent_board_id = if parent.is_empty() {
                                        None
                                    } else {
                                        Some(parent)
                                    };
                                    // If parent set, use parent's category from selection
                                    spawn(async move {
                                        let body = CreateBoardBody {
                                            category_id,
                                            parent_board_id,
                                            name,
                                            slug,
                                            description: None,
                                            sort_order: 0,
                                        };
                                        match ApiClient::web()
                                            .post_json::<_, serde_json::Value>("/api/forum/boards", &body)
                                            .await
                                        {
                                            Ok(_) => {
                                                board_name.set(String::new());
                                                refresh.set(refresh() + 1);
                                                toast.show(Toast::success("Board created"));
                                            }
                                            Err(e) => toast.show(Toast::error(e.to_string())),
                                        }
                                    });
                                },
                                "Add board"
                            }
                        }
                    }
                },
                Some(None) => rsx! { p { "Failed to load tree (are you logged in as officer?)" } },
                None => rsx! { p { "Loading…" } },
            }
        }
    }
}
