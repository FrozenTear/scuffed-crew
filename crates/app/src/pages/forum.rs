//! Forum: the board index (`/forum`) and one board (`/forum/b/:slug`) with
//! its new thread form. The thread page is `forum_thread.rs`; it shares
//! [`FORUM_CSS`] and [`count_label`] from here.
//!
//! Cards, the breadcrumb, the status line and author names are the shared
//! reading-page pieces in `article_author.rs`; the pager, buttons, pills,
//! fields and the empty card come from `components/ui`.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, ArticleCrumb, AuthorNames, Load, author_byline, is_retrying,
    list_status, load_status, use_author_names,
};
use super::not_found::{NotFoundLink, NotFoundPanel};
use crate::components::ui::inline_confirm::{focus_element_by_id_soon, focus_field_by_id};
use crate::components::ui::{
    EmptyCard, FieldError, LoadError, PageHeader, PageShell, PageWidth, Pager, Pill, btn_class,
    page_count, show_list_top, use_url_page,
};
use crate::components::{Toast, use_toast};
use crate::routes::Route;
use crate::state::auth::use_auth;
use crate::util::{encode_query, local_time_node};
use scuffed_api_client::{ApiClient, ClientError};

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumCategory {
    id: String,
    name: String,
    slug: String,
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumBoard {
    id: String,
    name: String,
    slug: String,
    description: Option<String>,
    is_locked: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumBoardNode {
    #[serde(flatten)]
    board: ForumBoard,
    sub_boards: Vec<ForumBoard>,
    thread_count: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumCategoryNode {
    #[serde(flatten)]
    category: ForumCategory,
    boards: Vec<ForumBoardNode>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumThread {
    id: String,
    title: String,
    /// Member id of the starter. Shown only as a display name.
    author_member_id: String,
    pinned: bool,
    locked: bool,
    created_at: String,
    reply_count: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ThreadListResponse {
    /// One page of the board (`limit` and `offset`).
    threads: Vec<ForumThread>,
    /// Threads on the whole board.
    total: usize,
    board: Option<ForumBoard>,
}

#[derive(Serialize)]
struct CreateThreadBody {
    title: String,
    content: String,
    board: String,
}

/// Threads per page of a board. The list endpoint pages on the server
/// (`limit`, `offset`) and returns the board's `total`.
const THREAD_PAGE_SIZE: usize = 20;

/// The thread list section. It takes focus after a page change.
const THREAD_LIST_ID: &str = "forum-thread-list";

/// The button that opens the new thread form. Focus returns to it when the
/// form closes.
const NEW_THREAD_BUTTON_ID: &str = "forum-new-thread";
const NEW_THREAD_FORM_ID: &str = "forum-new-thread-form";
const NEW_THREAD_HEADING_ID: &str = "forum-new-thread-heading";

pub(crate) fn count_label(count: u64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

fn thread_count_label(count: u64) -> String {
    count_label(count, "thread", "threads")
}

/// API path of one thread. The id is percent-encoded like the board slug.
pub(crate) fn thread_api_path(thread_id: &str) -> String {
    format!("/api/forum/threads/{}", encode_query(thread_id))
}

/// What a failed thread or reply post says, in words: `lead` names the
/// action. The routes answer 403 for a locked board or thread and for a
/// board the viewer may not post to.
pub(crate) fn forum_post_error(lead: &str, err: &ClientError) -> String {
    let why = match err {
        ClientError::Http { status: 401, .. } => "Your session ended. Log in again.",
        ClientError::Http {
            status: 403, body, ..
        } if body.contains("locked") => "It is locked now. Reload the page.",
        ClientError::Http { status: 403, .. } => "Your account can't post here.",
        ClientError::Http { status: 404, .. } => "It no longer exists. Reload the page.",
        ClientError::Http { status: 400, .. } => "The server did not accept the text.",
        ClientError::Http { status: 429, .. } => "Too many posts in a row. Wait a moment.",
        // No answer, an unreadable answer and any other status: the one
        // mapping (QA on 748).
        other => crate::client_error::message_without_reason(other),
    };
    format!("{lead} {why}")
}

/// A description worth a line: trimmed, and `None` when blank.
fn shown_text(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|text| !text.is_empty())
}

/// `StyleOnce` id for [`FORUM_CSS`].
pub(crate) const FORUM_CSS_ID: &str = "scuffed-css-pages-forum-page-css";

/// Forum-only layout on top of the shared entry cards: board titles one
/// heading level down, the thread row's reply column, post cards and the two
/// forms. Tokens only.
pub(crate) const FORUM_CSS: &str = r#"
/* On the index a board title is an H3 under its category H2. Same recipe as
   the shared `.entry-row h2`, link cover and focus ring included. */
.forum-page .entry-row h3 {
  margin: 0 0 var(--space-2);
  font-family: var(--font-head);
  font-size: var(--text-lg);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
  color: var(--text);
  overflow-wrap: anywhere;
}
.forum-page .entry-row h3 .ui-pill {
  margin-inline-start: var(--space-2);
  vertical-align: middle;
}
.forum-page .entry-row h3 a {
  color: var(--accent-text);
  text-decoration: none;
}
.forum-page .entry-row h3 a::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
}
.forum-page .entry-row h3 a:hover { text-decoration: underline; }
.forum-page .entry-row:has(h3 a:hover) { border-color: var(--border-strong); }
.forum-page .entry-row h3 a:focus-visible { outline: none; }
.forum-page .entry-row h3 a:focus-visible::after {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
/* A category name and its description read as one heading group, a stack
   step above its boards. */
.forum-cat-head {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}
.forum-cat-desc {
  margin: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
.forum-sub-board { margin-inline-start: var(--space-5); }

/* A thread row: title and starter on the left, the reply count on the
   right. On a phone the count sits under the title. */
.forum-thread-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--space-3) var(--space-4);
  align-items: start;
}
.forum-thread-main { min-width: 0; }
/* The frame trims the row title to its cap height and sets the starter line
   right under it, so a row is one compact block. */
.forum-thread-row h2 {
  text-box-trim: trim-both;
  text-box-edge: cap alphabetic;
}
.forum-thread-row .entry-meta { margin-top: 0; }
/* The pager sits a page block under the list, as on the frame. */
.forum-thread-list { gap: var(--space-section); }
.forum-thread-row .forum-thread-stats { white-space: nowrap; }

/* A line of plain words under a title or a list ("This board is locked"). */
.forum-note {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin: var(--space-3) 0 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
.forum-posts .forum-note { margin: 0; }

/* A long trail (Forum / parent board / board / thread) wraps on a phone
   instead of pushing the page sideways. */
.forum-page .article-crumb { flex-wrap: wrap; row-gap: 0; }

/* The new thread form and the reply form: one card, fields a group apart. */
.forum-form {
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  min-width: 0;
  margin: 0;
  padding: var(--space-card-pad);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.forum-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
  margin: 0;
}
.forum-req {
  margin-inline-start: var(--space-1);
  color: var(--loss);
  font-weight: var(--font-label-weight);
}
.forum-form-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

/* The thread page: the first post and its replies, one card each. The
   frame sets each post card a page block apart. */
.forum-posts {
  display: grid;
  gap: var(--space-section);
  min-width: 0;
}
.forum-post {
  box-sizing: border-box;
  min-width: 0;
  padding: var(--space-card-pad);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.forum-post .article-byline { margin: 0 0 var(--space-3); }
/* Posts are plain text: line breaks are kept, long words wrap. */
.forum-post-body {
  margin: 0;
  max-width: var(--content-max-narrow);
  color: var(--text);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
@media (max-width: 820px) {
  .forum-thread-row { grid-template-columns: minmax(0, 1fr); }
  .forum-form-actions .btn { flex: 1 1 auto; }
}
"#;

#[component]
pub fn Forum() -> Element {
    let auth = use_auth();
    let mut refresh = use_signal(|| 0u32);
    let tree = use_resource(move || {
        let _ = refresh();
        async move {
            ApiClient::web()
                .fetch::<Vec<ForumCategoryNode>>("/api/forum/tree")
                .await
                .map_err(|e| crate::client_error::logged_message("forum tree", &e))
        }
    });
    // Only a 404 is missing: a 401 or 403 keeps the server's reason next to
    // the lead (QA round 3 on 748).
    let load = Load::of(tree.read().as_ref());
    let retrying = is_retrying(tree.state()(), &load);

    rsx! {
        ForumIndex {
            load,
            logged_in: auth().is_logged_in(),
            checking_session: auth().loading,
            retrying,
            on_retry: move |_| refresh += 1,
        }
    }
}

/// The board index for one fetch state. The status line is always in the
/// tree; only its text changes.
#[component]
fn ForumIndex(
    load: Load<Vec<ForumCategoryNode>>,
    logged_in: bool,
    /// The session is still loading: say nothing about logging in yet.
    #[props(default = false)]
    checking_session: bool,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    on_retry: EventHandler<()>,
) -> Element {
    let status = list_status(&load, retrying, false, "Loading boards", "boards");
    let lead = if logged_in || checking_session {
        "Boards for the crew, recruits, and guests."
    } else {
        "Boards for the crew, recruits, and guests. Log in to start a thread."
    };

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: FORUM_CSS_ID, css: FORUM_CSS }

        PageShell { width: PageWidth::Default, class: "forum-page",
            PageHeader { title: "Forum", lead }
            p { class: "entry-status", role: "status", "{status}" }
            {match &load {
                Load::Loading => rsx! {},
                Load::Failed(_) | Load::Missing => rsx! {
                    LoadError {
                        message: load.failure_line("Couldn't load the forum."),
                        on_retry: load.retry_helps().then_some(on_retry),
                        retrying,
                    }
                },
                Load::Ready(categories) if categories.is_empty() => rsx! {
                    EmptyCard {
                        title: "No boards yet",
                        message: "An officer can add boards in the admin area.",
                    }
                },
                Load::Ready(categories) => rsx! {
                    for cat in categories.iter() {
                        {render_category(cat)}
                    }
                },
            }}
        }
    }
}

/// One category: its name as an H2, then its boards as cards. A sub-board is
/// its own card, set in under its parent.
fn render_category(cat: &ForumCategoryNode) -> Element {
    let heading_id = format!("forum-cat-{}", cat.category.slug);
    rsx! {
        section {
            key: "{cat.category.id}",
            class: "ui-stack",
            aria_labelledby: "{heading_id}",
            div { class: "forum-cat-head",
                h2 { id: "{heading_id}", class: "ui-section-title", "{cat.category.name}" }
                if let Some(desc) = shown_text(cat.category.description.as_deref()) {
                    p { class: "forum-cat-desc", "{desc}" }
                }
            }
            if cat.boards.is_empty() {
                p { class: "forum-cat-desc", "No boards in this category yet." }
            } else {
                div { class: "entry-list",
                    for node in cat.boards.iter() {
                        {render_board_card(&node.board, thread_count_label(node.thread_count), false)}
                        for sub in node.sub_boards.iter() {
                            {render_board_card(sub, format!("Sub-board of {}", node.board.name), true)}
                        }
                    }
                }
            }
        }
    }
}

/// "Locked" is a word in a badge next to the name, so it does not rest on
/// colour.
fn render_board_card(board: &ForumBoard, meta: String, sub: bool) -> Element {
    rsx! {
        article {
            key: "{board.id}",
            class: if sub { "entry-row forum-sub-board" } else { "entry-row" },
            h3 {
                Link { to: Route::ForumBoardPage { slug: board.slug.clone() }, "{board.name}" }
                if board.is_locked {
                    Pill { "Locked" }
                }
            }
            if let Some(desc) = shown_text(board.description.as_deref()) {
                p { "{desc}" }
            }
            p { class: "entry-meta", "{meta}" }
        }
    }
}

/// A field of the new thread form. One value gives the control id, the error
/// id, the label and the error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadField {
    Title,
    Content,
}

impl ThreadField {
    const fn id(self) -> &'static str {
        match self {
            ThreadField::Title => "forum-thread-title",
            ThreadField::Content => "forum-thread-content",
        }
    }

    const fn error_id(self) -> &'static str {
        match self {
            ThreadField::Title => "forum-thread-title-error",
            ThreadField::Content => "forum-thread-content-error",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            ThreadField::Title => "Title",
            ThreadField::Content => "Post",
        }
    }

    const fn error(self) -> &'static str {
        match self {
            ThreadField::Title => "Enter a title for the thread.",
            ThreadField::Content => "Write the first post.",
        }
    }
}

/// The fields a submit would fail on, in form order. The server requires
/// both to be non-blank.
fn thread_errors(title: &str, content: &str) -> Vec<ThreadField> {
    let mut errors = Vec::new();
    if title.trim().is_empty() {
        errors.push(ThreadField::Title);
    }
    if content.trim().is_empty() {
        errors.push(ThreadField::Content);
    }
    errors
}

#[component]
pub fn ForumBoardPage(slug: String) -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    // The page lives in the address (`?page=2`), as on the blog list.
    let page = use_url_page();
    let mut composing = use_signal(|| false);
    let mut title = use_signal(String::new);
    let mut content = use_signal(String::new);
    let mut errors = use_signal(Vec::<ThreadField>::new);
    let mut posting = use_signal(|| false);
    let mut refresh = use_signal(|| 0u32);

    // Another board (a link from one board to the next reuses this
    // component): an open form and its text belong to the old one.
    use_effect(use_reactive((&slug,), move |_| {
        composing.set(false);
        title.set(String::new());
        content.set(String::new());
        errors.set(Vec::new());
    }));

    let list = use_resource(use_reactive((&slug,), move |(slug,)| {
        let _ = refresh();
        let offset = page.get().saturating_sub(1) * THREAD_PAGE_SIZE;
        async move {
            let url = format!(
                "/api/forum/threads?board={}&limit={THREAD_PAGE_SIZE}&offset={offset}",
                encode_query(&slug)
            );
            ApiClient::web()
                .fetch::<ThreadListResponse>(&url)
                .await
                .map_err(|e| crate::client_error::logged_message("forum threads", &e))
        }
    }));
    // Names for the starters of the threads on screen only.
    let names = use_author_names(move || {
        list.read()
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|r| {
                r.threads
                    .iter()
                    .map(|t| t.author_member_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    });

    // A 401 or 403 board keeps its reason; only a 404 is "Board not found".
    let load = Load::of(list.read().as_ref());
    let pending = list.state()() == UseResourceState::Pending;
    let retrying = is_retrying(list.state()(), &load);
    // A page change or a refresh keeps the old rows until the new ones land.
    let refreshing = pending && matches!(load, Load::Ready(_));
    crate::head::use_page_title(board_tab_label(&load));

    let submit_slug = slug.clone();
    let form = composing().then(|| {
        rsx! {
            NewThreadForm {
                title: title(),
                content: content(),
                errors: errors(),
                posting: posting(),
                on_title: move |value| title.set(value),
                on_content: move |value| content.set(value),
                on_cancel: move |_| {
                    composing.set(false);
                    errors.set(Vec::new());
                    focus_element_by_id_soon(NEW_THREAD_BUTTON_ID.to_string());
                },
                on_submit: move |_| {
                    if posting() {
                        return;
                    }
                    let t = title().trim().to_string();
                    let c = content().trim().to_string();
                    let found = thread_errors(&t, &c);
                    if let Some(first) = found.first().copied() {
                        errors.set(found);
                        focus_field_by_id(first.id(), ".forum-field");
                        return;
                    }
                    errors.set(Vec::new());
                    posting.set(true);
                    let slug = submit_slug.clone();
                    spawn(async move {
                        let body = CreateThreadBody {
                            title: t,
                            content: c,
                            board: slug,
                        };
                        match ApiClient::web()
                            .post_json::<_, serde_json::Value>("/api/forum/threads", &body)
                            .await
                        {
                            Ok(_) => {
                                title.set(String::new());
                                content.set(String::new());
                                composing.set(false);
                                refresh += 1;
                                // A new thread is at the top of the first page.
                                if page.get() != 1 {
                                    page.go(1);
                                }
                                toast.show(Toast::success("Thread created"));
                                focus_element_by_id_soon(NEW_THREAD_BUTTON_ID.to_string());
                            }
                            Err(e) => {
                                crate::util::log_client_error("forum thread", &e);
                                toast.show(Toast::error(forum_post_error(
                                    "The thread was not posted.",
                                    &e,
                                )));
                            }
                        }
                        posting.set(false);
                    });
                },
            }
        }
    });

    rsx! {
        BoardView {
            load,
            slug,
            names: Some(names()),
            page: page.get(),
            logged_in: auth().is_logged_in(),
            checking_session: auth().loading,
            retrying,
            refreshing,
            composing: composing(),
            form,
            on_retry: move |_| refresh += 1,
            on_page: move |next| {
                page.go(next);
                show_list_top(THREAD_LIST_ID);
            },
            on_compose: move |open: bool| {
                composing.set(open);
                if open {
                    focus_element_by_id_soon(ThreadField::Title.id().to_string());
                } else {
                    errors.set(Vec::new());
                }
            },
        }
    }
}

/// The browser tab names the board once it has loaded, not by the slug in the
/// address. `None` keeps the route's default label.
fn board_tab_label(load: &Load<ThreadListResponse>) -> Option<String> {
    match load {
        Load::Ready(resp) => resp
            .board
            .as_ref()
            .map(|board| format!("{} | Forum", board.name.trim())),
        Load::Missing => Some("Board not found | Forum".to_string()),
        Load::Loading | Load::Failed(_) => None,
    }
}

/// The board page for one fetch state. Every state has one H1, and the
/// status line is always in the tree; only its text changes.
#[component]
fn BoardView(
    load: Load<ThreadListResponse>,
    /// Names the board when the response carries no board row.
    slug: String,
    names: Option<AuthorNames>,
    page: usize,
    logged_in: bool,
    /// The session is still loading: no log-in link and no compose button yet.
    #[props(default = false)]
    checking_session: bool,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    /// Rows are on screen and a newer set is loading.
    #[props(default = false)]
    refreshing: bool,
    /// The new thread form is open.
    #[props(default = false)]
    composing: bool,
    /// The new thread form, between the header and the list.
    #[props(default)]
    form: Option<Element>,
    on_retry: EventHandler<()>,
    on_page: EventHandler<usize>,
    /// Open (`true`) or close the new thread form.
    on_compose: EventHandler<bool>,
) -> Element {
    let status = load_status(&load, retrying || refreshing, "Loading threads");
    let viewer = Viewer::of(logged_in, checking_session);
    let board = match &load {
        Load::Ready(resp) => resp.board.as_ref(),
        _ => None,
    };
    let board_name = match &load {
        Load::Ready(_) => Some(board.map_or_else(|| slug.clone(), |b| b.name.clone())),
        _ => None,
    };
    let locked = board.is_some_and(|b| b.is_locked);
    let total = match &load {
        Load::Ready(resp) => resp.total,
        _ => 0,
    };

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: FORUM_CSS_ID, css: FORUM_CSS }

        PageShell { width: PageWidth::Default, class: "forum-page",
            // The not-found card has its own "Back to the forum" link.
            if load != Load::Missing {
                ArticleCrumb { to: Route::Forum {}, section: "Forum", current: board_name.clone() }
            }
            {match &load {
                // The name is not known yet, so the page is named by its kind.
                Load::Loading => rsx! { PageHeader { title: "Forum board" } },
                Load::Failed(_) => rsx! { PageHeader { title: "Couldn't load this board" } },
                Load::Missing => rsx! {},
                Load::Ready(_) => rsx! {
                    PageHeader {
                        title: board_name.clone().unwrap_or_default(),
                        lead: shown_text(board.and_then(|b| b.description.as_deref())).map(str::to_string),
                        meta: locked.then(|| rsx! {
                            p { class: "forum-note",
                                Pill { "Locked" }
                                span { "No new threads can be started on this board." }
                            }
                        }),
                        actions: board_action(locked, viewer, composing, on_compose, true),
                    }
                },
            }}
            if load != Load::Missing {
                {form}
            }
            p { class: "entry-status", role: "status", "{status}" }
            if load == Load::Missing {
                NotFoundPanel {
                    title: "Board not found",
                    message: "This board does not exist or is not open to you.",
                    links: vec![
                        NotFoundLink { label: "Back to the forum", to: Route::Forum {} },
                        NotFoundLink { label: "Return home", to: Route::Home {} },
                    ],
                }
            } else {
                section {
                    id: THREAD_LIST_ID,
                    class: "ui-stack forum-thread-list pager-target",
                    tabindex: "-1",
                    aria_label: "Threads",
                    {match &load {
                        Load::Loading | Load::Missing => rsx! {},
                        Load::Failed(reason) => rsx! {
                            LoadError {
                                message: crate::client_error::reason_or_fallback(reason),
                                on_retry: load.retry_helps().then_some(on_retry),
                                retrying,
                            }
                        },
                        Load::Ready(resp) if resp.total == 0 => rsx! {
                            EmptyCard {
                                title: "No threads yet",
                                message: empty_board_message(locked, viewer),
                                // The open form is the action already.
                                actions: if composing {
                                    None
                                } else {
                                    board_action(locked, viewer, composing, on_compose, false)
                                },
                            }
                        },
                        // An address past the last page (`?page=9` of 3).
                        Load::Ready(resp) if resp.threads.is_empty() => rsx! {
                            EmptyCard {
                                title: "No threads on this page",
                                message: past_last_page_message(resp.total),
                                actions: rsx! {
                                    button {
                                        r#type: "button",
                                        class: btn_class(true),
                                        onclick: move |_| on_page.call(1),
                                        "Go to the first page"
                                    }
                                },
                            }
                        },
                        Load::Ready(resp) => rsx! {
                            div { class: "entry-list",
                                for t in resp.threads.iter() {
                                    {render_thread_row(t, names.as_ref())}
                                }
                            }
                        },
                    }}
                    Pager {
                        id: "forum-thread-pager",
                        page,
                        total,
                        page_size: THREAD_PAGE_SIZE,
                        noun: "threads",
                        noun_singular: "thread",
                        show_page_count: true,
                        on_change: move |next| on_page.call(next),
                    }
                }
            }
        }
    }
}

/// Who is looking, as far as the board's one action cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Viewer {
    /// The session has not loaded yet.
    Checking,
    Guest,
    SignedIn,
}

impl Viewer {
    fn of(logged_in: bool, checking_session: bool) -> Self {
        match (logged_in, checking_session) {
            (true, _) => Viewer::SignedIn,
            (false, true) => Viewer::Checking,
            (false, false) => Viewer::Guest,
        }
    }
}

fn empty_board_message(locked: bool, viewer: Viewer) -> &'static str {
    match (locked, viewer) {
        (true, _) => "This board is locked, so no threads can be started.",
        (false, Viewer::SignedIn) => "Start the first one.",
        (false, Viewer::Guest) => "Log in to start the first one.",
        (false, Viewer::Checking) => "Threads started here show up in this list.",
    }
}

fn past_last_page_message(total: usize) -> String {
    let pages = page_count(total, THREAD_PAGE_SIZE);
    format!(
        "This board has {} on {}.",
        thread_count_label(total as u64),
        count_label(pages as u64, "page", "pages")
    )
}

/// The one action of a board: a guest gets the log-in link, a signed-in
/// viewer the button that opens the form, and a locked board gets none.
/// `in_header` marks the header button, which owns the id that focus returns
/// to and says whether the form is open.
fn board_action(
    locked: bool,
    viewer: Viewer,
    composing: bool,
    on_compose: EventHandler<bool>,
    in_header: bool,
) -> Option<Element> {
    if locked || viewer == Viewer::Checking {
        return None;
    }
    if viewer == Viewer::Guest {
        return Some(rsx! {
            Link { to: Route::Login {}, class: btn_class(true), "Log in to start a thread" }
        });
    }
    Some(if in_header {
        rsx! {
            button {
                r#type: "button",
                id: NEW_THREAD_BUTTON_ID,
                class: btn_class(true),
                aria_expanded: if composing { "true" } else { "false" },
                aria_controls: if composing { NEW_THREAD_FORM_ID },
                onclick: move |_| on_compose.call(!composing),
                "New thread"
            }
        }
    } else {
        rsx! {
            button {
                r#type: "button",
                class: btn_class(true),
                onclick: move |_| on_compose.call(true),
                "Start a thread"
            }
        }
    })
}

/// "Pinned" and "Locked" are words in badges next to the title, so they do
/// not rest on colour or position.
fn render_thread_row(t: &ForumThread, names: Option<&AuthorNames>) -> Element {
    rsx! {
        article { key: "{t.id}", class: "entry-row forum-thread-row",
            div { class: "forum-thread-main",
                h2 {
                    Link { to: Route::ForumThread { id: t.id.clone() }, "{t.title}" }
                    if t.pinned {
                        Pill { "Pinned" }
                    }
                    if t.locked {
                        Pill { "Locked" }
                    }
                }
                p { class: "entry-meta",
                    {author_byline(names, &t.author_member_id, "Started by")}
                    {local_time_node(&t.created_at)}
                }
            }
            p { class: "forum-thread-stats", "{count_label(t.reply_count, \"reply\", \"replies\")}" }
        }
    }
}

/// The asterisk next to a required label. Hidden from assistive tech, which
/// gets `required` from the control.
pub(crate) fn required_mark() -> Element {
    rsx! { span { class: "forum-req", aria_hidden: "true", "*" } }
}

/// The new thread form. `errors` is the result of the last submit, empty
/// until one fails. The error lines are always in the tree, so a new error is
/// announced. The submit button is never natively disabled: while sending it
/// keeps focus and ignores a second press.
#[component]
fn NewThreadForm(
    title: String,
    content: String,
    errors: Vec<ThreadField>,
    posting: bool,
    on_title: EventHandler<String>,
    on_content: EventHandler<String>,
    on_submit: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    let title_invalid = errors.contains(&ThreadField::Title);
    let content_invalid = errors.contains(&ThreadField::Content);
    let error_text = |field: ThreadField, invalid: bool| {
        if invalid {
            field.error().to_string()
        } else {
            String::new()
        }
    };
    rsx! {
        form {
            id: NEW_THREAD_FORM_ID,
            class: "forum-form",
            novalidate: true,
            aria_labelledby: NEW_THREAD_HEADING_ID,
            onsubmit: move |e: FormEvent| {
                e.prevent_default();
                on_submit.call(());
            },
            h2 { id: NEW_THREAD_HEADING_ID, class: "ui-section-title", "New thread" }
            div { class: "forum-field",
                label { class: "field-label", r#for: ThreadField::Title.id(),
                    "{ThreadField::Title.label()}"
                    {required_mark()}
                }
                input {
                    r#type: "text",
                    class: "field-input",
                    id: ThreadField::Title.id(),
                    name: "title",
                    required: true,
                    aria_required: "true",
                    aria_invalid: if title_invalid { "true" },
                    aria_describedby: if title_invalid { ThreadField::Title.error_id() },
                    value: "{title}",
                    oninput: move |e: FormEvent| on_title.call(e.value()),
                }
                FieldError {
                    id: ThreadField::Title.error_id(),
                    text: error_text(ThreadField::Title, title_invalid),
                    role: "alert",
                }
            }
            div { class: "forum-field",
                label { class: "field-label", r#for: ThreadField::Content.id(),
                    "{ThreadField::Content.label()}"
                    {required_mark()}
                }
                textarea {
                    class: "field-input",
                    id: ThreadField::Content.id(),
                    name: "content",
                    required: true,
                    aria_required: "true",
                    aria_invalid: if content_invalid { "true" },
                    aria_describedby: if content_invalid { ThreadField::Content.error_id() },
                    value: "{content}",
                    oninput: move |e: FormEvent| on_content.call(e.value()),
                }
                FieldError {
                    id: ThreadField::Content.error_id(),
                    text: error_text(ThreadField::Content, content_invalid),
                    role: "alert",
                }
            }
            div { class: "forum-form-actions",
                button {
                    r#type: "submit",
                    class: btn_class(true),
                    aria_busy: if posting { "true" },
                    aria_disabled: if posting { "true" },
                    if posting {
                        "Posting..."
                    } else {
                        "Post thread"
                    }
                }
                button {
                    r#type: "button",
                    class: btn_class(false),
                    onclick: move |_| on_cancel.call(()),
                    "Cancel"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    const NET: &str = crate::client_error::NETWORK_ERROR_MESSAGE;
    use super::super::article_author::AUTHOR_FALLBACK;
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, render_routed};

    const AUTHOR: &str = "member:7f3a9c2e1b";

    fn board(n: usize, name: &str, locked: bool) -> ForumBoard {
        ForumBoard {
            id: format!("forum_board:{n}"),
            name: name.to_string(),
            slug: name.to_lowercase().replace(' ', "-"),
            description: Some(format!("About {name}.")),
            is_locked: locked,
        }
    }

    fn tree() -> Vec<ForumCategoryNode> {
        vec![ForumCategoryNode {
            category: ForumCategory {
                id: "forum_category:1".to_string(),
                name: "Community".to_string(),
                slug: "community".to_string(),
                description: Some("Open boards.".to_string()),
            },
            boards: vec![
                ForumBoardNode {
                    board: board(1, "General", false),
                    sub_boards: vec![board(3, "Introductions", false)],
                    thread_count: 12,
                },
                ForumBoardNode {
                    board: board(2, "Staff", true),
                    sub_boards: Vec::new(),
                    thread_count: 1,
                },
            ],
        }]
    }

    fn render_index(load: Load<Vec<ForumCategoryNode>>, logged_in: bool, retrying: bool) -> String {
        render_routed(move || {
            rsx! {
                ForumIndex {
                    load: load.clone(),
                    logged_in,
                    retrying,
                    on_retry: move |_| {},
                }
            }
        })
    }

    fn thread(n: usize, pinned: bool, locked: bool, replies: u64) -> ForumThread {
        ForumThread {
            id: format!("forum_thread:{n}"),
            title: format!("Thread {n}"),
            author_member_id: AUTHOR.to_string(),
            pinned,
            locked,
            created_at: "2026-11-12T18:10:00Z".to_string(),
            reply_count: replies,
        }
    }

    fn names() -> AuthorNames {
        AuthorNames::from([(AUTHOR.to_string(), Some("Player06".to_string()))])
    }

    fn listing(threads: Vec<ForumThread>, total: usize, locked: bool) -> ThreadListResponse {
        ThreadListResponse {
            threads,
            total,
            board: Some(board(1, "General", locked)),
        }
    }

    #[derive(Clone)]
    struct BoardCase {
        load: Load<ThreadListResponse>,
        names: Option<AuthorNames>,
        page: usize,
        logged_in: bool,
        checking: bool,
        retrying: bool,
        composing: bool,
    }

    impl BoardCase {
        fn of(load: Load<ThreadListResponse>) -> Self {
            BoardCase {
                load,
                names: Some(names()),
                page: 1,
                logged_in: false,
                checking: false,
                retrying: false,
                composing: false,
            }
        }

        fn member(mut self) -> Self {
            self.logged_in = true;
            self
        }

        fn render(self) -> String {
            render_routed(move || {
                let case = self.clone();
                rsx! {
                    BoardView {
                        load: case.load,
                        slug: "general",
                        names: case.names,
                        page: case.page,
                        logged_in: case.logged_in,
                        checking_session: case.checking,
                        retrying: case.retrying,
                        composing: case.composing,
                        on_retry: move |_| {},
                        on_page: move |_| {},
                        on_compose: move |_| {},
                    }
                }
            })
        }
    }

    fn render_form(errors: Vec<ThreadField>, posting: bool) -> String {
        render_routed(move || {
            rsx! {
                NewThreadForm {
                    title: "Friday customs",
                    content: "",
                    errors: errors.clone(),
                    posting,
                    on_title: move |_| {},
                    on_content: move |_| {},
                    on_submit: move |_| {},
                    on_cancel: move |_| {},
                }
            }
        })
    }

    #[test]
    fn one_thread_is_singular_and_other_counts_are_plural() {
        assert_eq!(thread_count_label(0), "0 threads");
        assert_eq!(thread_count_label(1), "1 thread");
        assert_eq!(thread_count_label(2), "2 threads");
        assert_eq!(count_label(1, "reply", "replies"), "1 reply");
        assert_eq!(count_label(3, "reply", "replies"), "3 replies");
    }

    #[test]
    fn index_lists_boards_as_linked_cards_under_their_category() {
        let html = render_index(Load::Ready(tree()), false, false);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Forum</h1>"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains(
                "<section class=\"ui-stack\" aria-labelledby=\"forum-cat-community\"><div class=\"forum-cat-head\"><h2 id=\"forum-cat-community\" class=\"ui-section-title\">Community</h2><p class=\"forum-cat-desc\">Open boards.</p></div>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<article class=\"entry-row\"><h3><a href=\"/forum/b/general\">General</a></h3><p>About General.</p><p class=\"entry-meta\">12 threads</p></article>"
            ),
            "{html}"
        );
        // A sub-board is its own card and says whose it is.
        assert!(
            html.contains(
                "<article class=\"entry-row forum-sub-board\"><h3><a href=\"/forum/b/introductions\">Introductions</a></h3>"
            ),
            "{html}"
        );
        assert!(html.contains(">Sub-board of General</p>"), "{html}");
        // Locked is a word in a badge, and one thread is singular.
        assert!(
            html.contains(
                "<h3><a href=\"/forum/b/staff\">Staff</a><span class=\"ui-pill ui-pill--neutral\">Locked</span></h3>"
            ),
            "{html}"
        );
        assert!(html.contains(">1 thread</p>"), "{html}");
        assert_eq!(html.matches(">Locked<").count(), 1, "{html}");
        // No button sits inside a link.
        assert!(!html.contains("<button"), "{html}");
    }

    #[test]
    fn index_lead_only_asks_a_guest_to_log_in() {
        let guest = render_index(Load::Ready(tree()), false, false);
        assert!(
            guest.contains("recruits, and guests. Log in to start a thread.</p>"),
            "{guest}"
        );
        let member = render_index(Load::Ready(tree()), true, false);
        assert!(member.contains("recruits, and guests.</p>"), "{member}");
        assert!(!member.contains("Log in"), "{member}");
    }

    #[test]
    fn index_empty_loading_and_failed_states() {
        let empty = render_index(Load::Ready(Vec::new()), false, false);
        assert!(empty.contains("<h2>No boards yet</h2>"), "{empty}");

        let loading = render_index(Load::Loading, false, false);
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading boards</p>"),
            "{loading}"
        );
        assert!(!loading.contains("No boards yet"), "{loading}");

        // QA on 748: a refusal shows the server's reason, not the connection.
        for reason in ["HTTP error 403: Members only", "HTTP error 409: Board busy"] {
            let refused = render_index(Load::Failed(reason.to_string()), false, false);
            assert!(
                refused.contains(&format!("Couldn&#39;t load the forum. {reason}")),
                "{refused}"
            );
            assert!(!refused.contains("connection"), "{refused}");
        }
        let failed = render_index(Load::Failed(NET.to_string()), false, false);
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains(
                "Couldn&#39;t load the forum. Could not reach the server. Check your connection and try again."
            ),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");
        assert!(!failed.contains("Loading"), "{failed}");

        let retrying = render_index(Load::Failed(NET.to_string()), false, true);
        assert!(retrying.contains(">Loading boards</p>"), "{retrying}");
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");

        for html in [&empty, &loading, &failed, &retrying] {
            assert!(
                html.contains("class=\"entry-status\" role=\"status\""),
                "{html}"
            );
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(!html.contains("<main"), "{html}");
        }
    }

    #[test]
    fn board_has_its_name_as_the_one_h1_under_a_breadcrumb() {
        let html = BoardCase::of(Load::Ready(listing(
            vec![thread(1, false, false, 5)],
            1,
            false,
        )))
        .render();
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>General</h1>"), "{html}");
        assert!(
            html.contains("<p class=\"page-lead\">About General.</p>"),
            "{html}"
        );
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains(
                "<nav class=\"article-crumb\" aria-label=\"Breadcrumb\"><a href=\"/forum\">Forum</a><span aria-hidden=\"true\">/</span><span aria-current=\"page\">General</span></nav>"
            ),
            "{html}"
        );
    }

    #[test]
    fn board_without_a_board_row_is_named_by_its_slug() {
        let mut resp = listing(Vec::new(), 0, false);
        resp.board = None;
        let html = BoardCase::of(Load::Ready(resp)).render();
        assert!(html.contains("<h1>general</h1>"), "{html}");
    }

    #[test]
    fn guest_gets_the_log_in_link_and_a_member_the_new_thread_button() {
        let ready = Load::Ready(listing(vec![thread(1, false, false, 5)], 1, false));
        let guest = BoardCase::of(ready.clone()).render();
        assert!(
            guest.contains(
                "<div class=\"page-header-actions\"><a href=\"/login\" class=\"btn is-primary\">Log in to start a thread</a></div>"
            ),
            "{guest}"
        );
        assert!(!guest.contains("<button"), "{guest}");

        let member = BoardCase::of(ready.clone()).member().render();
        assert!(
            member.contains(
                "<button type=\"button\" id=\"forum-new-thread\" class=\"btn is-primary\" aria-expanded=\"false\">New thread</button>"
            ),
            "{member}"
        );
        assert!(!member.contains("Log in"), "{member}");

        let mut open = BoardCase::of(ready).member();
        open.composing = true;
        let open = open.render();
        assert!(
            open.contains("aria-expanded=\"true\" aria-controls=\"forum-new-thread-form\""),
            "{open}"
        );
    }

    #[test]
    fn locked_board_says_so_in_words_and_offers_no_action() {
        for case in [
            BoardCase::of(Load::Ready(listing(Vec::new(), 0, true))),
            BoardCase::of(Load::Ready(listing(Vec::new(), 0, true))).member(),
        ] {
            let html = case.render();
            assert!(
                html.contains(
                    "<p class=\"forum-note\"><span class=\"ui-pill ui-pill--neutral\">Locked</span><span>No new threads can be started on this board.</span></p>"
                ),
                "{html}"
            );
            assert!(!html.contains("page-header-actions"), "{html}");
            assert!(!html.contains("empty-card-actions"), "{html}");
            assert!(!html.contains("New thread"), "{html}");
            assert!(
                html.contains("This board is locked, so no threads"),
                "{html}"
            );
        }
    }

    #[test]
    fn thread_row_has_badges_the_starter_name_the_time_and_the_reply_count() {
        let html = BoardCase::of(Load::Ready(listing(
            vec![thread(1, true, true, 12), thread(2, false, false, 1)],
            2,
            false,
        )))
        .render();
        assert!(
            html.contains(
                "<article class=\"entry-row forum-thread-row\"><div class=\"forum-thread-main\"><h2><a href=\"/forum/t/forum_thread:1\">Thread 1</a><span class=\"ui-pill ui-pill--neutral\">Pinned</span><span class=\"ui-pill ui-pill--neutral\">Locked</span></h2>"
            ),
            "{html}"
        );
        assert!(html.contains(">Started by Player06</span>"), "{html}");
        assert!(
            html.contains("<time datetime=\"2026-11-12T18:10:00"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"forum-thread-stats\">12 replies</p>"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"forum-thread-stats\">1 reply</p>"),
            "{html}"
        );
        assert_eq!(html.matches(">Pinned<").count(), 1, "{html}");
        assert!(!html.contains(AUTHOR), "raw author id leaked: {html}");
        // The link is the title only: no button or second link inside it.
        assert_eq!(html.matches("href=\"/forum/t/").count(), 2, "{html}");

        // A failed lookup reads as the fallback, a pending one as nothing.
        let mut failed = BoardCase::of(Load::Ready(listing(
            vec![thread(1, false, false, 0)],
            1,
            false,
        )));
        failed.names = Some(AuthorNames::from([(AUTHOR.to_string(), None)]));
        let failed = failed.render();
        assert!(
            failed.contains(&format!(">Started by {AUTHOR_FALLBACK}</span>")),
            "{failed}"
        );
        let mut pending = BoardCase::of(Load::Ready(listing(
            vec![thread(1, false, false, 0)],
            1,
            false,
        )));
        pending.names = Some(AuthorNames::new());
        let pending = pending.render();
        assert!(!pending.contains("Started by"), "{pending}");
        assert!(!pending.contains(AUTHOR), "{pending}");
    }

    #[test]
    fn long_board_is_paged_on_the_server_total() {
        let threads: Vec<ForumThread> = (1..=THREAD_PAGE_SIZE)
            .map(|n| thread(n, false, false, 0))
            .collect();
        let html = BoardCase::of(Load::Ready(listing(threads, 45, false))).render();
        assert_eq!(
            html.matches("class=\"entry-row forum-thread-row\"").count(),
            THREAD_PAGE_SIZE,
            "{html}"
        );
        assert!(
            html.contains(
                "<p id=\"forum-thread-pager-meta\" class=\"pager-meta\" role=\"status\">1 to 20 of 45 threads · Page 1 of 3</p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<nav class=\"pager-pages\" aria-label=\"Pagination\">"),
            "{html}"
        );
        assert!(html.contains("aria-current=\"page\""), "{html}");
        assert!(html.contains("id=\"forum-thread-pager-page-3\""), "{html}");
    }

    #[test]
    fn an_address_past_the_last_page_offers_the_first_page() {
        let mut case = BoardCase::of(Load::Ready(listing(Vec::new(), 45, false)));
        case.page = 9;
        let html = case.render();
        assert!(html.contains("<h2>No threads on this page</h2>"), "{html}");
        assert!(
            html.contains("This board has 45 threads on 3 pages."),
            "{html}"
        );
        assert!(html.contains(">Go to the first page</button>"), "{html}");
        assert!(!html.contains("No threads yet"), "{html}");
    }

    #[test]
    fn empty_board_uses_the_empty_card_with_the_same_action() {
        let ready = Load::Ready(listing(Vec::new(), 0, false));
        let guest = BoardCase::of(ready.clone()).render();
        assert!(guest.contains("<h2>No threads yet</h2>"), "{guest}");
        assert!(guest.contains("Log in to start the first one."), "{guest}");
        assert!(
            guest.contains(
                "<div class=\"empty-card-actions\"><a href=\"/login\" class=\"btn is-primary\">Log in to start a thread</a></div>"
            ),
            "{guest}"
        );

        let member = BoardCase::of(ready).member().render();
        assert!(
            member.contains(
                "<div class=\"empty-card-actions\"><button type=\"button\" class=\"btn is-primary\">Start a thread</button></div>"
            ),
            "{member}"
        );
        // One element owns the id that focus returns to.
        assert_eq!(
            member.matches("id=\"forum-new-thread\"").count(),
            1,
            "{member}"
        );

        // With the form open, the empty card offers no second way to open it.
        let mut open = BoardCase::of(Load::Ready(listing(Vec::new(), 0, false))).member();
        open.composing = true;
        let open = open.render();
        assert!(open.contains("<h2>No threads yet</h2>"), "{open}");
        assert!(!open.contains("empty-card-actions"), "{open}");
        assert!(!open.contains("Start a thread"), "{open}");
    }

    /// A fetch failure the way the forum resources keep it: mapped and
    /// logged once by `logged_message`, then read by `Load::of`.
    fn mapped_load<T: Clone>(status: u16, body: &str) -> (Load<T>, String) {
        let _ = crate::util::take_logged();
        let err = ClientError::http(status, body, None);
        let slot: Result<T, String> =
            Err(crate::client_error::logged_message("forum threads", &err));
        let load = Load::of(Some(&slot));
        let logged = crate::util::take_logged();
        assert_eq!(logged.len(), 1, "{status}: {logged:?}");
        (load, crate::client_error::user_message(&err))
    }

    /// QA round 3 on 748: a 401 or 403 board showed only "Couldn't load the
    /// forum." because the forum mapped them to the not-found view. Only a
    /// 404 is missing now; a refusal keeps the server's reason on the
    /// index, the board and (through the same `Load::of`) the thread page.
    #[test]
    fn a_refused_forum_load_shows_the_reason_through_the_mapping() {
        for status in [401, 403] {
            let body = r#"{"error":"Members only"}"#;
            let (load, reason) = mapped_load::<Vec<ForumCategoryNode>>(status, body);
            assert!(reason.contains("Members only"), "{reason}");
            assert_eq!(load, Load::Failed(reason.clone()), "{status}");
            let index = render_index(load, false, false);
            assert!(
                index.contains(&format!("Couldn&#39;t load the forum. {reason}")),
                "{index}"
            );
            // Vision round 6 on 748: a retry gets the same refusal, so there
            // is no Retry, the same as the strategy editor 403. The error box
            // is still rendered, as a status rather than an alert.
            assert!(
                index.contains("<div class=\"fetch-error-wrap\" role=\"status\">"),
                "{index}"
            );
            assert!(!index.contains("role=\"alert\""), "{index}");
            assert!(!index.contains("fetch-error__retry"), "{index}");
            assert!(!index.contains(">Retry</button>"), "{index}");
            assert!(!index.contains("not found"), "{index}");

            let (load, _) = mapped_load::<ThreadListResponse>(status, body);
            assert!(!load.retry_helps(), "{status}");
            let board = BoardCase::of(load).render();
            assert!(board.contains("Couldn&#39;t load this board"), "{board}");
            assert!(board.contains(&reason), "{board}");
            assert!(!board.contains("Board not found"), "{board}");
            assert!(
                board.contains("<div class=\"fetch-error-wrap\" role=\"status\">"),
                "{board}"
            );
            assert!(!board.contains("fetch-error__retry"), "{board}");
            assert!(!board.contains(">Retry</button>"), "{board}");

            // A reason-less refusal ("HTTP error: 403") has no Retry either.
            let (load, _) = mapped_load::<ThreadListResponse>(status, "");
            assert!(!load.retry_helps(), "{status}");
            let board = BoardCase::of(load).render();
            assert!(board.contains(&format!("HTTP error: {status}")), "{board}");
            assert!(!board.contains(">Retry</button>"), "{board}");
        }

        // Anything a retry can change keeps Retry: no answer, a 5xx (with and
        // without a reason), an unreadable answer, and other 4xx answers.
        let _ = crate::util::take_logged();
        let retryable = [
            ClientError::Network("TypeError: Failed to fetch".into()),
            ClientError::http(500, r#"{"error":"Database down"}"#, None),
            ClientError::http(503, "", None),
            ClientError::Deserialize("expected value at line 1".into()),
            ClientError::http(409, r#"{"error":"Board busy"}"#, None),
        ];
        for err in retryable {
            let slot: Result<Vec<ForumCategoryNode>, String> =
                Err(crate::client_error::logged_message("forum tree", &err));
            let load = Load::of(Some(&slot));
            assert!(load.retry_helps(), "{err:?}");
            let index = render_index(load, false, false);
            assert!(index.contains(">Retry</button>"), "{err:?}: {index}");
            assert!(
                index.contains("<div class=\"fetch-error-wrap\" role=\"alert\">"),
                "{err:?}: {index}"
            );

            let slot: Result<ThreadListResponse, String> =
                Err(crate::client_error::logged_message("forum threads", &err));
            let board = BoardCase::of(Load::of(Some(&slot))).render();
            assert!(board.contains(">Retry</button>"), "{err:?}: {board}");
        }
        let _ = crate::util::take_logged();
        // Only a failed load can refuse a retry.
        assert!(Load::<u8>::Loading.retry_helps());
        assert!(Load::<u8>::Missing.retry_helps());
        assert!(Load::Ready(3u8).retry_helps());

        // A 404 is still the not-found panel, with nothing to retry.
        let (load, _) = mapped_load::<Vec<ForumCategoryNode>>(404, r#"{"error":"Not found"}"#);
        assert_eq!(load, Load::Missing);
        let (load, _) = mapped_load::<ThreadListResponse>(404, r#"{"error":"Not found"}"#);
        assert_eq!(load, Load::Missing);
        let board = BoardCase::of(load).render();
        assert!(board.contains("Board not found"), "{board}");
        assert!(!board.contains("role=\"alert\""), "{board}");

        // A reason-less 403 still says something past the lead.
        let (load, reason) = mapped_load::<Vec<ForumCategoryNode>>(403, "");
        assert_eq!(reason, "HTTP error: 403");
        let index = render_index(load, false, false);
        assert!(
            index.contains("Couldn&#39;t load the forum. HTTP error: 403"),
            "{index}"
        );
        assert!(!index.contains(">Retry</button>"), "{index}");

        assert_eq!(Load::of(Some(&Ok::<u8, String>(3))), Load::Ready(3));
        assert_eq!(Load::<u8>::of(None), Load::Loading);

        // All three forum fetches read their slot through `Load::of`, which
        // maps only a 404 to missing; no forum-only 401/403 mapping is back.
        let board = include_str!("forum.rs").split("mod tests").next().unwrap();
        assert!(board.contains("Load::of(tree.read().as_ref())"));
        assert!(board.contains("Load::of(list.read().as_ref())"));
        let thread = include_str!("forum_thread.rs")
            .split("mod tests")
            .next()
            .unwrap();
        assert!(thread.contains("Load::of(detail.read().as_ref())"));
        // Every forum LoadError takes its Retry from `retry_helps` (index,
        // board, thread), never an unconditional handler.
        assert_eq!(
            board
                .matches("on_retry: load.retry_helps().then_some(on_retry),")
                .count(),
            2
        );
        assert_eq!(
            thread
                .matches("on_retry: load.retry_helps().then_some(on_retry),")
                .count(),
            1
        );
        for src in [board, thread] {
            assert!(!src.contains("on_retry: move |_| on_retry.call(())"));
            assert!(!src.contains("forum_load"));
            assert!(!src.contains("is_http_status(err, 401)"));
            assert!(!src.contains("is_http_status(err, 403)"));
        }
    }

    #[test]
    fn no_log_in_prompt_flashes_while_the_session_loads() {
        let mut case = BoardCase::of(Load::Ready(listing(Vec::new(), 0, false)));
        case.checking = true;
        let html = case.render();
        assert!(!html.contains("Log in"), "{html}");
        assert!(!html.contains("Start a thread"), "{html}");
        assert!(html.contains("<h2>No threads yet</h2>"), "{html}");

        let guest = BoardCase::of(Load::Ready(listing(Vec::new(), 0, false))).render();
        assert!(guest.contains(">Log in to start a thread</a>"), "{guest}");

        let index = render_routed(|| {
            rsx! {
                ForumIndex {
                    load: Load::Ready(Vec::new()),
                    logged_in: false,
                    checking_session: true,
                    on_retry: move |_| {},
                }
            }
        });
        assert!(!index.contains("Log in"), "{index}");
        assert_eq!(Viewer::of(false, true), Viewer::Checking);
        assert_eq!(Viewer::of(true, true), Viewer::SignedIn);
        assert_eq!(Viewer::of(false, false), Viewer::Guest);
    }

    #[test]
    fn post_errors_say_why_and_thread_ids_are_encoded() {
        let http = |status, body: &str| ClientError::http(status, body, None);
        assert_eq!(
            forum_post_error(
                "The reply was not posted.",
                &http(403, r#"{"error":"Thread is locked"}"#)
            ),
            "The reply was not posted. It is locked now. Reload the page."
        );
        assert_eq!(
            forum_post_error(
                "The thread was not posted.",
                &http(403, r#"{"error":"No access"}"#)
            ),
            "The thread was not posted. Your account can't post here."
        );
        assert_eq!(
            forum_post_error("x", &ClientError::Network("offline".into())),
            "x Could not reach the server. Check your connection and try again."
        );
        for status in [400, 401, 403, 404, 429, 500] {
            let copy = forum_post_error("x", &http(status, r#"{"error":"raw"}"#));
            assert!(!copy.contains("raw") && !copy.contains("HTTP"), "{copy}");
        }
        assert_eq!(
            thread_api_path("forum_thread:abc/../x?y"),
            "/api/forum/threads/forum_thread%3Aabc%2F..%2Fx%3Fy"
        );
        let thread = include_str!("forum_thread.rs")
            .split("mod tests")
            .next()
            .unwrap();
        assert!(
            !thread.contains("/api/forum/threads/{tid}"),
            "unencoded id in a path"
        );
        assert!(!thread.contains("Failed: {e}"));
        let board = include_str!("forum.rs").split("mod tests").next().unwrap();
        assert!(!board.contains("Toast::error(e.to_string())"));
    }

    #[test]
    fn board_loading_failed_and_missing_states() {
        let loading = BoardCase::of(Load::Loading).render();
        assert!(loading.contains("<h1>Forum board</h1>"), "{loading}");
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading threads</p>"),
            "{loading}"
        );
        assert!(!loading.contains("No threads yet"), "{loading}");

        let failed = BoardCase::of(Load::Failed(NET.to_string())).render();
        assert!(
            failed.contains("<h1>Couldn&#39;t load this board</h1>")
                || failed.contains("<h1>Couldn't load this board</h1>"),
            "{failed}"
        );
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(failed.contains(">Retry</button>"), "{failed}");
        assert!(!failed.contains("Loading"), "{failed}");

        let mut retrying = BoardCase::of(Load::Failed(NET.to_string()));
        retrying.retrying = true;
        let retrying = retrying.render();
        assert!(retrying.contains(">Loading threads</p>"), "{retrying}");
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");

        for html in [&loading, &failed, &retrying] {
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(!html.contains("<main"), "{html}");
            assert!(
                html.contains("class=\"entry-status\" role=\"status\""),
                "{html}"
            );
            // The pager's range line exists before it has anything to say.
            assert!(html.contains("id=\"forum-thread-pager-meta\""), "{html}");
            assert!(
                html.contains(
                    "<section id=\"forum-thread-list\" class=\"ui-stack forum-thread-list pager-target\" tabindex=\"-1\" aria-label=\"Threads\">"
                ),
                "{html}"
            );
            assert!(html.contains("<a href=\"/forum\">Forum</a>"), "{html}");
        }

        // A missing or private board is the not-found card, not an error.
        let missing = BoardCase::of(Load::Missing).render();
        assert_eq!(missing.matches("<h1").count(), 1, "{missing}");
        assert!(missing.contains("<h1>Board not found</h1>"), "{missing}");
        assert!(missing.contains(">Back to the forum</a>"), "{missing}");
        assert!(!missing.contains("article-crumb"), "{missing}");
        assert!(!missing.contains("role=\"alert\""), "{missing}");
        assert!(
            missing.contains("class=\"entry-status\" role=\"status\""),
            "{missing}"
        );
    }

    #[test]
    fn tab_is_named_by_the_board_not_the_slug() {
        let ready = Load::Ready(listing(Vec::new(), 0, false));
        assert_eq!(board_tab_label(&ready).as_deref(), Some("General | Forum"));
        assert_eq!(
            board_tab_label(&Load::Missing).as_deref(),
            Some("Board not found | Forum")
        );
        assert_eq!(board_tab_label(&Load::Loading), None);
        assert_eq!(board_tab_label(&Load::Failed(NET.to_string())), None);
    }

    #[test]
    fn new_thread_form_has_labelled_required_fields_and_no_errors_at_first() {
        let html = render_form(Vec::new(), false);
        assert!(
            html.contains(
                "<form id=\"forum-new-thread-form\" class=\"forum-form\" novalidate=true aria-labelledby=\"forum-new-thread-heading\">"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<h2 id=\"forum-new-thread-heading\" class=\"ui-section-title\">New thread</h2>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<label class=\"field-label\" for=\"forum-thread-title\">Title<span class=\"forum-req\" aria-hidden=\"true\">*</span></label>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<label class=\"field-label\" for=\"forum-thread-content\">Post<span"),
            "{html}"
        );
        for id in ["forum-thread-title", "forum-thread-content"] {
            let start = html.find(&format!(" id=\"{id}\"")).expect(id);
            let tag = &html[html[..start].rfind('<').unwrap()..];
            let tag = &tag[..tag.find('>').unwrap()];
            assert!(tag.contains("class=\"field-input\""), "{tag}");
            // The native attribute itself, not the "required" inside
            // aria-required.
            assert!(
                tag.split_whitespace()
                    .any(|attr| attr == "required" || attr.starts_with("required=")),
                "{tag}"
            );
            assert!(tag.contains("aria-required=\"true\""), "{tag}");
            assert!(!tag.contains("aria-invalid"), "{tag}");
            assert!(!tag.contains("aria-describedby"), "{tag}");
            assert!(!tag.contains("placeholder"), "{tag}");
        }
        // The error lines exist, empty, before there is an error.
        assert!(
            html.contains(
                "<p class=\"field-error\" id=\"forum-thread-title-error\" role=\"alert\"></p>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<p class=\"field-error\" id=\"forum-thread-content-error\" role=\"alert\"></p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<button type=\"submit\" class=\"btn is-primary\">Post thread</button>"),
            "{html}"
        );
        assert!(
            html.contains("<button type=\"button\" class=\"btn is-ghost\">Cancel</button>"),
            "{html}"
        );
    }

    #[test]
    fn new_thread_form_marks_the_invalid_field_and_stays_usable_while_posting() {
        let html = render_form(vec![ThreadField::Content], false);
        assert!(
            html.contains("aria-invalid=\"true\" aria-describedby=\"forum-thread-content-error\""),
            "{html}"
        );
        assert!(
            html.contains(
                "<p class=\"field-error\" id=\"forum-thread-content-error\" role=\"alert\"><span class=\"field-error-icon\" aria-hidden=\"true\"></span>Write the first post.</p>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("aria-invalid").count(), 1, "{html}");

        let posting = render_form(Vec::new(), true);
        assert!(
            posting.contains("aria-busy=\"true\" aria-disabled=\"true\">Posting...</button>"),
            "{posting}"
        );
        assert!(!posting.contains(" disabled"), "{posting}");
    }

    #[test]
    fn blank_title_or_post_is_an_error_in_form_order() {
        assert!(thread_errors("Title", "Body").is_empty());
        assert_eq!(thread_errors("  ", "Body"), [ThreadField::Title]);
        assert_eq!(thread_errors("Title", "\n"), [ThreadField::Content]);
        assert_eq!(
            thread_errors("", ""),
            [ThreadField::Title, ThreadField::Content]
        );
    }

    #[test]
    fn thread_list_response_parses_the_server_shape() {
        let resp: ThreadListResponse = serde_json::from_str(
            r#"{"threads":[{"id":"forum_thread:1","title":"T","category":"general",
                "board_id":"forum_board:1","author_member_id":"member:1","content":"C",
                "pinned":true,"locked":false,"created_at":"2026-11-12T18:10:00Z",
                "is_active":true,"reply_count":2}],
                "total":41,
                "board":{"id":"forum_board:1","category_id":"forum_category:1",
                "parent_board_id":null,"name":"General","slug":"general",
                "description":null,"sort_order":0,"is_locked":false}}"#,
        )
        .unwrap();
        assert_eq!(resp.total, 41);
        assert_eq!(resp.threads[0].reply_count, 2);
        assert!(resp.threads[0].pinned);
        assert_eq!(resp.board.unwrap().name, "General");
    }

    #[test]
    fn forum_css_is_on_tokens_and_stacks_the_row_on_a_phone() {
        assert_sizes_from_tokens(FORUM_CSS);
        let row = css_rule(FORUM_CSS, ".forum-thread-row {");
        assert!(
            row.contains("grid-template-columns: minmax(0, 1fr) auto"),
            "{row}"
        );
        let phone = FORUM_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block")
            .1;
        assert!(
            phone.contains(".forum-thread-row { grid-template-columns: minmax(0, 1fr); }"),
            "{phone}"
        );
        for marker in [".forum-form {", ".forum-post {"] {
            let rule = css_rule(FORUM_CSS, marker);
            assert!(
                rule.contains("padding: var(--space-card-pad)"),
                "{marker}: {rule}"
            );
        }
    }
}
