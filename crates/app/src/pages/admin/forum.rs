use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ui::inline_confirm::focus_element_by_id;
use crate::components::ui::{
    EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pill, PillTone,
};
use crate::components::{FormStatus, SaveState, Toast, admin_pending, use_toast};
use crate::hooks::use_api;
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

// Fixed, page-unique ids of the two create forms.
const CAT_NAME_ID: &str = "forum-new-category-name";
const CAT_HINT_ID: &str = "forum-new-category-name-hint";
const CAT_ERROR_ID: &str = "forum-new-category-name-error";
const CAT_STATUS_ID: &str = "forum-new-category-status";
const BOARD_CAT_ID: &str = "forum-new-board-category";
const BOARD_CAT_ERROR_ID: &str = "forum-new-board-category-error";
const BOARD_PARENT_ID: &str = "forum-new-board-parent";
const BOARD_PARENT_HINT_ID: &str = "forum-new-board-parent-hint";
const BOARD_NAME_ID: &str = "forum-new-board-name";
const BOARD_NAME_ERROR_ID: &str = "forum-new-board-name-error";
const BOARD_STATUS_ID: &str = "forum-new-board-status";

/// Page CSS: the structure cards and the two create forms side by side.
const CSS: &str = r#"
    .forum-admin-tree {
        display: grid;
        gap: var(--space-stack);
        margin: 0;
        padding: 0;
        list-style: none;
    }
    .forum-admin-cat {
        border: 1px solid var(--border);
        border-radius: var(--radius-card);
        padding: var(--space-card-pad);
        background: var(--surface);
    }
    .forum-admin-cat h3 {
        margin: 0;
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-lg);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
    }
    .forum-admin-boards {
        margin: var(--space-3) 0 0;
        padding: 0;
        list-style: none;
    }
    .forum-admin-boards .forum-admin-boards {
        margin-top: 0;
        padding-left: var(--space-5);
        border-left: 1px solid var(--border);
    }
    .forum-admin-board {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-1) var(--space-3);
        min-height: var(--density-row);
        padding: var(--space-2) 0;
        border-top: 1px solid var(--table-row-border);
        color: var(--text);
        font-size: var(--text-md);
    }
    .forum-admin-board-name { font-weight: var(--font-label-weight); }
    .forum-admin-path {
        color: var(--text-3);
        font-family: var(--font-mono);
        font-size: var(--text-sm);
        overflow-wrap: anywhere;
    }
    .forum-admin-none {
        margin: var(--space-3) 0 0;
        color: var(--text-3);
        font-size: var(--text-sm);
    }
    .forum-admin-forms {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: var(--space-section);
        align-items: start;
        margin-top: var(--space-section);
    }
    @media (max-width: 820px) {
        .forum-admin-forms { grid-template-columns: 1fr; }
    }
"#;

/// Text of a required-field error. Empty when the value is there.
fn required(value: &str, message: &str) -> String {
    if value.trim().is_empty() {
        message.to_string()
    } else {
        String::new()
    }
}

#[component]
pub fn AdminForum() -> Element {
    let mut toast = use_toast();
    let mut tree = use_api::<Vec<ForumCategoryNode>>("/api/forum/tree");

    let mut cat_name = use_signal(String::new);
    let mut cat_error = use_signal(String::new);
    let mut cat_state = use_signal(SaveState::default);
    let mut board_name = use_signal(String::new);
    let board_cat = use_signal(String::new);
    let board_parent = use_signal(String::new);
    let mut board_cat_error = use_signal(String::new);
    let mut board_name_error = use_signal(String::new);
    let mut board_state = use_signal(SaveState::default);

    let add_category = move |_| {
        if cat_state() == SaveState::Saving {
            return;
        }
        let name = cat_name().trim().to_string();
        cat_error.set(required(&name, "Add a name for the category."));
        if name.is_empty() {
            cat_state.set(SaveState::Idle);
            toast.show(Toast::error("Name required"));
            return;
        }
        let slug = slugify(&name);
        cat_state.set(SaveState::Saving);
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
                    cat_state.set(SaveState::Saved("Category created.".to_string()));
                    tree.refresh += 1;
                    toast.show(Toast::success("Category created"));
                }
                Err(e) => {
                    cat_state.set(SaveState::failed("Could not create the category", &e));
                    toast.show(Toast::error(e.to_string()));
                }
            }
        });
    };

    let add_board = move |_| {
        if board_state() == SaveState::Saving {
            return;
        }
        let category_id = board_cat();
        let parent = board_parent();
        let name = board_name().trim().to_string();
        board_cat_error.set(required(&category_id, "Pick the category for the board."));
        board_name_error.set(required(&name, "Add a name for the board."));
        if category_id.is_empty() || name.is_empty() {
            board_state.set(SaveState::Idle);
            toast.show(Toast::error("Category and name required"));
            return;
        }
        let slug = slugify(&name);
        let parent_board_id = if parent.is_empty() {
            None
        } else {
            Some(parent)
        };
        board_state.set(SaveState::Saving);
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
                    board_state.set(SaveState::Saved("Board created.".to_string()));
                    tree.refresh += 1;
                    toast.show(Toast::success("Board created"));
                }
                Err(e) => {
                    board_state.set(SaveState::failed("Could not create the board", &e));
                    toast.show(Toast::error(e.to_string()));
                }
            }
        });
    };

    let body = {
        let data = tree.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_pending(&tree, "the forum structure"),
            Some(cats) => {
                let cats = cats.clone();
                let options = cats.clone();
                rsx! {
                    section { class: "forum-admin-structure", aria_labelledby: "forum-structure-title",
                        h2 { id: "forum-structure-title", class: "visually-hidden", "Current structure" }
                        if cats.is_empty() {
                            EmptyCard {
                                title: "No forum categories yet",
                                message: "Members cannot post until there is a category with a board in it. Add a category, then a board inside it.",
                                actions: rsx! {
                                    button {
                                        r#type: "button",
                                        class: "btn is-primary",
                                        onclick: move |_| focus_element_by_id(CAT_NAME_ID),
                                        "Add a category"
                                    }
                                },
                            }
                        } else {
                            ForumTree { cats }
                        }
                    }
                    div { class: "forum-admin-forms",
                        section { class: "form-section", aria_labelledby: "forum-new-category-title",
                            h2 { id: "forum-new-category-title", "Add a category" }
                            p { class: "form-section-lead", "A category groups boards on the forum page." }
                            div { class: "form-section-card form-stack",
                                CategoryFields { name: cat_name, error: cat_error() }
                                div { class: "admin-form-actions",
                                    button {
                                        r#type: "button",
                                        class: "btn is-primary",
                                        aria_disabled: if cat_state() == SaveState::Saving { "true" },
                                        aria_describedby: CAT_STATUS_ID,
                                        onclick: add_category,
                                        "Add category"
                                    }
                                    FormStatus { state: cat_state(), id: CAT_STATUS_ID.to_string() }
                                }
                            }
                        }
                        section { class: "form-section", aria_labelledby: "forum-new-board-title",
                            h2 { id: "forum-new-board-title", "Add a board or sub-board" }
                            p { class: "form-section-lead",
                                "Pick a parent board to make a sub-board. Example: Games, then Overwatch, then Strategy."
                            }
                            div { class: "form-section-card form-stack",
                                BoardFields {
                                    cats: options,
                                    category: board_cat,
                                    parent: board_parent,
                                    name: board_name,
                                    category_error: board_cat_error(),
                                    name_error: board_name_error(),
                                }
                                div { class: "admin-form-actions",
                                    button {
                                        r#type: "button",
                                        class: "btn is-primary",
                                        aria_disabled: if board_state() == SaveState::Saving { "true" },
                                        aria_describedby: BOARD_STATUS_ID,
                                        onclick: add_board,
                                        "Add board"
                                    }
                                    FormStatus { state: board_state(), id: BOARD_STATUS_ID.to_string() }
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-forum-css", css: CSS }
        PageShell { width: PageWidth::Wide, class: "admin-forum-page",
            PageHeader {
                title: "Forum structure",
                lead: "Categories hold boards, and a board can hold sub-boards. Each board gets its own address under /forum/b/.",
            }
            {body}
        }
    }
}

/// The current structure: one card per category, its boards and sub-boards
/// as nested lists with their addresses and a worded Locked or Hidden badge.
#[component]
fn ForumTree(cats: Vec<ForumCategoryNode>) -> Element {
    rsx! {
        ul { class: "forum-admin-tree",
            for cat in cats {
                li { key: "{cat.category.id}", class: "forum-admin-cat",
                    h3 { "{cat.category.name}" }
                    if cat.boards.is_empty() {
                        p { class: "forum-admin-none", "No boards yet. Add one below." }
                    } else {
                        ul { class: "forum-admin-boards",
                            for node in cat.boards.iter() {
                                li { key: "{node.board.id}",
                                    BoardLine { board: node.board.clone() }
                                    if !node.sub_boards.is_empty() {
                                        ul { class: "forum-admin-boards", aria_label: "Sub-boards of {node.board.name}",
                                            for sub in node.sub_boards.iter() {
                                                li { key: "{sub.id}", BoardLine { board: sub.clone() } }
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
    }
}

#[component]
fn BoardLine(board: ForumBoard) -> Element {
    rsx! {
        div { class: "forum-admin-board",
            span { class: "forum-admin-board-name", "{board.name}" }
            code { class: "forum-admin-path", "/forum/b/{board.slug}" }
            if board.is_locked {
                Pill { tone: PillTone::Warn, "Locked" }
            }
            if !board.is_active {
                Pill { tone: PillTone::Neutral, "Hidden" }
            }
        }
    }
}

#[component]
fn CategoryFields(name: Signal<String>, error: String) -> Element {
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: CAT_NAME_ID, "Name" }
            input {
                id: CAT_NAME_ID,
                class: "field-input",
                r#type: "text",
                placeholder: "Games",
                value: "{name}",
                aria_invalid: if !error.is_empty() { "true" },
                aria_describedby: "{CAT_HINT_ID} {CAT_ERROR_ID}",
                oninput: move |e| name.set(e.value()),
            }
            p { id: CAT_HINT_ID, class: "field-hint", "The address is made from the name." }
            FieldError { id: CAT_ERROR_ID.to_string(), text: error.clone() }
        }
    }
}

#[component]
fn BoardFields(
    cats: Vec<ForumCategoryNode>,
    category: Signal<String>,
    parent: Signal<String>,
    name: Signal<String>,
    category_error: String,
    name_error: String,
) -> Element {
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: BOARD_CAT_ID, "Category" }
            select {
                id: BOARD_CAT_ID,
                class: "field-input",
                aria_invalid: if !category_error.is_empty() { "true" },
                aria_describedby: BOARD_CAT_ERROR_ID,
                onchange: move |e| category.set(e.value()),
                option { value: "", "Choose a category" }
                for cat in cats.iter() {
                    option {
                        value: "{cat.category.id}",
                        selected: category() == cat.category.id,
                        "{cat.category.name}"
                    }
                }
            }
            FieldError { id: BOARD_CAT_ERROR_ID.to_string(), text: category_error.clone() }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: BOARD_PARENT_ID, "Parent board" }
            select {
                id: BOARD_PARENT_ID,
                class: "field-input",
                aria_describedby: BOARD_PARENT_HINT_ID,
                onchange: move |e| parent.set(e.value()),
                option { value: "", "None (a top-level board)" }
                for cat in cats.iter() {
                    for node in cat.boards.iter() {
                        option {
                            value: "{node.board.id}",
                            selected: parent() == node.board.id,
                            "{cat.category.name} / {node.board.name}"
                        }
                    }
                }
            }
            p { id: BOARD_PARENT_HINT_ID, class: "field-hint",
                "Leave it at None for a board directly in the category."
            }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: BOARD_NAME_ID, "Name" }
            input {
                id: BOARD_NAME_ID,
                class: "field-input",
                r#type: "text",
                placeholder: "Overwatch",
                value: "{name}",
                aria_invalid: if !name_error.is_empty() { "true" },
                aria_describedby: BOARD_NAME_ERROR_ID,
                oninput: move |e| name.set(e.value()),
            }
            FieldError { id: BOARD_NAME_ERROR_ID.to_string(), text: name_error.clone() }
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

    fn board(id: &str, name: &str, locked: bool) -> ForumBoard {
        ForumBoard {
            id: id.into(),
            category_id: "c1".into(),
            parent_board_id: None,
            name: name.into(),
            slug: name.to_lowercase(),
            description: None,
            sort_order: 0,
            is_locked: locked,
            is_active: true,
        }
    }

    fn sample_tree() -> Vec<ForumCategoryNode> {
        vec![ForumCategoryNode {
            category: ForumCategory {
                id: "c1".into(),
                name: "Games".into(),
                slug: "games".into(),
                description: None,
                sort_order: 10,
            },
            boards: vec![ForumBoardNode {
                board: board("b1", "Overwatch", false),
                sub_boards: vec![board("b2", "Strategy", true)],
                thread_count: 3,
            }],
        }]
    }

    #[test]
    fn page_has_one_h1_and_a_labelled_wait_in_the_wide_shell() {
        let html = render_admin_route("/admin/forum", Some(scuffed_types::OrgRole::Officer));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-forum-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Forum structure</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains("role=\"status\">Loading the forum structure...</p>"),
            "{html}"
        );
        assert!(!html.contains('\u{2014}'), "{html}");
    }

    #[test]
    fn tree_nests_sub_boards_and_words_the_lock() {
        fn view() -> Element {
            rsx! { ForumTree { cats: sample_tree() } }
        }
        let html = render(view);
        assert!(html.contains("<h3>Games</h3>"), "{html}");
        assert!(html.contains(">/forum/b/overwatch</code>"), "{html}");
        assert!(
            html.contains("aria-label=\"Sub-boards of Overwatch\""),
            "{html}"
        );
        assert_eq!(
            html.matches("<span class=\"ui-pill ui-pill--warn\">Locked</span>")
                .count(),
            1,
            "{html}"
        );
    }

    #[test]
    fn create_fields_are_labelled_and_errors_are_always_rendered() {
        fn view() -> Element {
            let cat_name = use_signal(String::new);
            let category = use_signal(String::new);
            let parent = use_signal(String::new);
            let name = use_signal(String::new);
            rsx! {
                CategoryFields { name: cat_name, error: String::new() }
                BoardFields {
                    cats: sample_tree(),
                    category,
                    parent,
                    name,
                    category_error: "Pick the category for the board.".to_string(),
                    name_error: String::new(),
                }
            }
        }
        let html = render(view);
        for id in [CAT_NAME_ID, BOARD_CAT_ID, BOARD_PARENT_ID, BOARD_NAME_ID] {
            assert!(
                html.contains(&format!("<label class=\"field-label\" for=\"{id}\"")),
                "{html}"
            );
            assert_eq!(html.matches(&format!("id=\"{id}\"")).count(), 1, "{html}");
        }
        assert!(
            html.contains(&format!(
                "<p class=\"field-error\" id=\"{CAT_ERROR_ID}\"></p>"
            )),
            "{html}"
        );
        assert!(
            html.contains("Pick the category for the board.</p>"),
            "{html}"
        );
        assert!(html.contains(">Games / Overwatch</option>"), "{html}");
        assert!(!html.contains("style="), "{html}");
    }

    #[test]
    fn page_css_sizes_come_from_tokens() {
        assert_sizes_from_tokens(CSS);
        assert_eq!(slugify("  Hero Talk! "), "hero-talk");
        assert_eq!(required(" ", "x"), "x");
    }
}
