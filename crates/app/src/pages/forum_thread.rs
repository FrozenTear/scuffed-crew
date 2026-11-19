//! One forum thread (`/forum/t/:id`): the first post, its replies and the
//! reply form. Layout shared with the board pages lives in `forum.rs`
//! ([`FORUM_CSS`]).

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, ArticleByline, AuthorNames, Load, author_label, is_retrying,
    load_status, use_author_names,
};
use super::forum::{
    FORUM_CSS, FORUM_CSS_ID, count_label, forum_post_error, required_mark, thread_api_path,
};
use super::not_found::{NotFoundLink, NotFoundPanel};
use crate::components::ui::inline_confirm::focus_field_by_id;
use crate::components::ui::{
    FieldError, LoadError, PageHeader, PageShell, PageWidth, Pager, Pill, btn_class, page_count,
    show_list_top, use_url_page,
};
use crate::components::{Toast, use_toast};
use crate::routes::Route;
use crate::state::auth::use_auth;
use crate::util::local_time_node;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumThreadData {
    title: String,
    /// Member id of the starter. Shown only as a display name.
    author_member_id: String,
    content: String,
    pinned: bool,
    locked: bool,
    created_at: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct BoardMeta {
    name: String,
    slug: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumReplyData {
    id: String,
    /// Member id of the poster. Shown only as a display name.
    author_member_id: String,
    content: String,
    created_at: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ThreadDetailResponse {
    thread: ForumThreadData,
    /// One page of replies (`limit` and `offset`).
    replies: Vec<ForumReplyData>,
    /// Replies on the whole thread.
    reply_count: u64,
    board: Option<BoardMeta>,
    parent_board: Option<BoardMeta>,
}

#[derive(Serialize)]
struct CreateReplyBody {
    content: String,
}

/// Replies per page. The detail endpoint pages replies on the server
/// (`limit`, `offset`, at most 100) and returns the thread's `reply_count`.
const REPLY_PAGE_SIZE: usize = 50;

/// The replies section. It takes focus after a page change.
const REPLIES_ID: &str = "forum-replies";
const REPLIES_HEADING_ID: &str = "forum-replies-heading";

const REPLY_FIELD_ID: &str = "forum-reply";
const REPLY_ERROR_ID: &str = "forum-reply-error";
const REPLY_ERROR: &str = "Write a reply before posting.";

#[component]
pub fn ForumThread(id: String) -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let mut refresh = use_signal(|| 0u32);
    let mut reply_content = use_signal(String::new);
    let mut reply_invalid = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    // The reply page lives in the address (`?page=2`), as on a board.
    let page = use_url_page();

    // Another thread (a link from one thread to the next reuses this
    // component): a half written reply and its error belong to the old one.
    use_effect(use_reactive((&id,), move |_| {
        reply_content.set(String::new());
        reply_invalid.set(false);
    }));

    let detail = use_resource(use_reactive((&id,), move |(tid,)| {
        let _ = refresh();
        let offset = page.get().saturating_sub(1) * REPLY_PAGE_SIZE;
        async move {
            if tid.is_empty() {
                return Err("HTTP error: 404".to_string());
            }
            ApiClient::web()
                .fetch::<ThreadDetailResponse>(&format!(
                    "{}?limit={REPLY_PAGE_SIZE}&offset={offset}",
                    thread_api_path(&tid)
                ))
                .await
                .map_err(|e| crate::client_error::logged_message("forum thread", &e))
        }
    }));
    // Posts carry only the author id; resolve the ones on screen.
    let names = use_author_names(move || {
        detail
            .read()
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|resp| {
                std::iter::once(resp.thread.author_member_id.clone())
                    .chain(resp.replies.iter().map(|r| r.author_member_id.clone()))
                    .collect()
            })
            .unwrap_or_default()
    });
    // A 401 or 403 keeps its reason; only a 404 is "Thread not found".
    let load = Load::of(detail.read().as_ref());
    let retrying = is_retrying(detail.state()(), &load);
    crate::head::use_page_title(tab_label(&load));

    let reply_id = id.clone();
    rsx! {
        ThreadView {
            load,
            names: Some(names()),
            logged_in: auth().is_logged_in(),
            retrying,
            reply: reply_content(),
            reply_invalid: reply_invalid(),
            posting: submitting(),
            page: page.get(),
            on_retry: move |_| refresh += 1,
            on_page: move |next| {
                page.go(next);
                show_list_top(REPLIES_ID);
            },
            on_reply_input: move |value| reply_content.set(value),
            on_reply_submit: move |_| {
                if submitting() {
                    return;
                }
                let content = reply_content();
                if content.trim().is_empty() {
                    reply_invalid.set(true);
                    focus_field_by_id(REPLY_FIELD_ID, ".forum-field");
                    return;
                }
                reply_invalid.set(false);
                // The new reply is the last one, so the view moves to the
                // last page once it is posted.
                let replies_after = detail
                    .read()
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .map_or(1, |resp| resp.reply_count as usize + 1);
                let last_page = page_count(replies_after, REPLY_PAGE_SIZE);
                let tid = reply_id.clone();
                submitting.set(true);
                spawn(async move {
                    let body = CreateReplyBody { content };
                    let url = format!("{}/replies", thread_api_path(&tid));
                    match ApiClient::web().post_json::<_, serde_json::Value>(&url, &body).await {
                        Ok(_) => {
                            toast.show(Toast::success("Reply posted"));
                            reply_content.set(String::new());
                            refresh += 1;
                            if page.get() != last_page {
                                page.go(last_page);
                            }
                        }
                        Err(e) => {
                            crate::util::log_client_error("forum reply", &e);
                            toast.show(Toast::error(forum_post_error("The reply was not posted.", &e)));
                        }
                    }
                    submitting.set(false);
                });
            },
        }
    }
}

/// The browser tab names the thread by its title once it has loaded, not by
/// the id in the address. `None` keeps the route's default label.
fn tab_label(load: &Load<ThreadDetailResponse>) -> Option<String> {
    match load {
        Load::Ready(resp) => Some(format!("{} | Forum", resp.thread.title.trim())),
        Load::Missing => Some("Thread not found | Forum".to_string()),
        Load::Loading | Load::Failed(_) => None,
    }
}

/// The trail over a thread: Forum, the parent board when the board is a
/// sub-board, the board, then the thread title as the current page.
fn thread_crumb(resp: Option<&ThreadDetailResponse>) -> Element {
    let boards = resp
        .into_iter()
        .flat_map(|resp| resp.parent_board.iter().chain(resp.board.iter()));
    rsx! {
        nav { class: "article-crumb", aria_label: "Breadcrumb",
            Link { to: Route::Forum {}, "Forum" }
            for board in boards {
                Fragment { key: "{board.slug}",
                    span { aria_hidden: "true", "/" }
                    Link { to: Route::ForumBoardPage { slug: board.slug.clone() }, "{board.name}" }
                }
            }
            if let Some(resp) = resp {
                span { aria_hidden: "true", "/" }
                span { aria_current: "page", "{resp.thread.title}" }
            }
        }
    }
}

/// The thread page for one fetch state. Every state has one H1, and the
/// status line is always in the tree; only its text changes.
#[component]
fn ThreadView(
    load: Load<ThreadDetailResponse>,
    names: Option<AuthorNames>,
    logged_in: bool,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    #[props(default)] reply: String,
    /// The last submit had no text.
    #[props(default = false)]
    reply_invalid: bool,
    #[props(default = false)] posting: bool,
    /// The reply page, counted from 1.
    #[props(default = 1)]
    page: usize,
    on_retry: EventHandler<()>,
    on_page: EventHandler<usize>,
    on_reply_input: EventHandler<String>,
    on_reply_submit: EventHandler<()>,
) -> Element {
    let status = load_status(&load, retrying, "Loading thread");
    let ready = match &load {
        Load::Ready(resp) => Some(resp),
        _ => None,
    };

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: FORUM_CSS_ID, css: FORUM_CSS }

        PageShell { width: PageWidth::Default, class: "forum-page forum-thread-page",
            // The not-found card has its own "Back to the forum" link.
            if load != Load::Missing {
                {thread_crumb(ready)}
            }
            {match &load {
                // The title is not known yet, so the page is named by its kind.
                Load::Loading => rsx! { PageHeader { title: "Forum thread" } },
                Load::Failed(_) => rsx! { PageHeader { title: "Couldn't load this thread" } },
                Load::Missing | Load::Ready(_) => rsx! {},
            }}
            p { class: "entry-status", role: "status", "{status}" }
            {match &load {
                Load::Loading => rsx! {},
                Load::Missing => rsx! {
                    NotFoundPanel {
                        title: "Thread not found",
                        message: "This thread does not exist or is not open to you.",
                        links: vec![
                            NotFoundLink { label: "Back to the forum", to: Route::Forum {} },
                            NotFoundLink { label: "Return home", to: Route::Home {} },
                        ],
                    }
                },
                Load::Failed(reason) => rsx! {
                    LoadError {
                        message: crate::client_error::reason_or_fallback(reason),
                        on_retry: load.retry_helps().then_some(on_retry),
                        retrying,
                    }
                },
                Load::Ready(resp) => rsx! {
                    PageHeader {
                        title: resp.thread.title.clone(),
                        meta: rsx! {
                            ArticleByline {
                                author: author_label(names.as_ref(), &resp.thread.author_member_id),
                                {local_time_node(&resp.thread.created_at)}
                                span { class: "entry-sep", aria_hidden: "true", "·" }
                                span { "{count_label(resp.reply_count, \"reply\", \"replies\")}" }
                                if resp.thread.pinned {
                                    Pill { "Pinned" }
                                }
                                if resp.thread.locked {
                                    Pill { "Locked" }
                                }
                            }
                        },
                    }
                    div { class: "forum-posts",
                        article { class: "forum-post", aria_label: "First post",
                            p { class: "forum-post-body", "{resp.thread.content}" }
                        }
                        section {
                            id: REPLIES_ID,
                            class: "forum-posts pager-target",
                            tabindex: "-1",
                            aria_labelledby: REPLIES_HEADING_ID,
                            h2 { id: REPLIES_HEADING_ID, class: "visually-hidden", "Replies" }
                            for r in resp.replies.iter() {
                                Fragment { key: "{r.id}", {render_reply(r, names.as_ref())} }
                            }
                            if resp.reply_count == 0 {
                                p { class: "forum-note", "No replies yet." }
                            } else if resp.replies.is_empty() {
                                // An address past the last page (`?page=9` of 2).
                                p { class: "forum-note", "No replies on this page." }
                            }
                            Pager {
                                id: "forum-reply-pager",
                                page,
                                total: resp.reply_count as usize,
                                page_size: REPLY_PAGE_SIZE,
                                noun: "replies",
                                noun_singular: "reply",
                                show_page_count: true,
                                on_change: move |next| on_page.call(next),
                            }
                        }
                    }
                    if resp.thread.locked {
                        p { class: "forum-note",
                            Pill { "Locked" }
                            span { "This thread is locked. No new replies can be posted." }
                        }
                    } else if logged_in {
                        ReplyForm {
                            content: reply.clone(),
                            invalid: reply_invalid,
                            posting,
                            on_input: move |value| on_reply_input.call(value),
                            on_submit: move |_| on_reply_submit.call(()),
                        }
                    }
                },
            }}
        }
    }
}

/// One reply: who and when, then the text. The author stays out while the
/// name lookup is pending; the raw member id is never shown.
fn render_reply(r: &ForumReplyData, names: Option<&AuthorNames>) -> Element {
    rsx! {
        article { key: "{r.id}", class: "forum-post",
            ArticleByline { author: author_label(names, &r.author_member_id),
                {local_time_node(&r.created_at)}
            }
            p { class: "forum-post-body", "{r.content}" }
        }
    }
}

/// The reply form. Its heading is the field's visible label, so the label is
/// the accessible name. The error line is always in the tree, so a new error
/// is announced. The submit button is never natively disabled: while sending
/// it keeps focus and ignores a second press.
#[component]
fn ReplyForm(
    content: String,
    #[props(default = false)] invalid: bool,
    #[props(default = false)] posting: bool,
    on_input: EventHandler<String>,
    on_submit: EventHandler<()>,
) -> Element {
    let error_text = if invalid { REPLY_ERROR } else { "" };
    rsx! {
        form {
            class: "forum-form",
            novalidate: true,
            onsubmit: move |e: FormEvent| {
                e.prevent_default();
                on_submit.call(());
            },
            div { class: "forum-field",
                h2 { class: "ui-section-title",
                    label { r#for: REPLY_FIELD_ID,
                        "Post a reply"
                        {required_mark()}
                    }
                }
                textarea {
                    id: REPLY_FIELD_ID,
                    name: REPLY_FIELD_ID,
                    class: "field-input",
                    required: true,
                    aria_required: "true",
                    aria_invalid: if invalid { "true" },
                    aria_describedby: if invalid { REPLY_ERROR_ID },
                    value: "{content}",
                    oninput: move |evt: FormEvent| on_input.call(evt.value()),
                }
                FieldError {
                    id: REPLY_ERROR_ID,
                    text: error_text,
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
                        "Post reply"
                    }
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
    use crate::components::ui::test_support::render_routed;

    const OP: &str = "member:7f3a9c2e1b";
    const REPLIER: &str = "member:9d1c44a0e2";

    fn reply(n: usize) -> ForumReplyData {
        ForumReplyData {
            id: format!("forum_reply:{n}"),
            author_member_id: REPLIER.to_string(),
            content: format!("Reply {n}\nsecond line"),
            created_at: "2026-11-12T18:22:00Z".to_string(),
        }
    }

    fn detail(replies: Vec<ForumReplyData>, locked: bool) -> ThreadDetailResponse {
        ThreadDetailResponse {
            thread: ForumThreadData {
                title: "Friday customs, looking for a tank".to_string(),
                author_member_id: OP.to_string(),
                content: "We have four for Friday customs.".to_string(),
                pinned: false,
                locked,
                created_at: "2026-11-12T18:10:00Z".to_string(),
            },
            reply_count: replies.len() as u64,
            replies,
            board: Some(BoardMeta {
                name: "General".to_string(),
                slug: "general".to_string(),
            }),
            parent_board: None,
        }
    }

    fn names() -> AuthorNames {
        AuthorNames::from([
            (OP.to_string(), Some("Player02".to_string())),
            (REPLIER.to_string(), Some("Player One".to_string())),
        ])
    }

    #[derive(Clone)]
    struct Case {
        load: Load<ThreadDetailResponse>,
        names: Option<AuthorNames>,
        logged_in: bool,
        retrying: bool,
        reply_invalid: bool,
        posting: bool,
        page: usize,
    }

    impl Case {
        fn of(load: Load<ThreadDetailResponse>) -> Self {
            Case {
                load,
                names: Some(names()),
                logged_in: false,
                retrying: false,
                reply_invalid: false,
                posting: false,
                page: 1,
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
                    ThreadView {
                        load: case.load,
                        names: case.names,
                        logged_in: case.logged_in,
                        retrying: case.retrying,
                        reply: "hello",
                        reply_invalid: case.reply_invalid,
                        posting: case.posting,
                        page: case.page,
                        on_retry: move |_| {},
                        on_page: move |_| {},
                        on_reply_input: move |_| {},
                        on_reply_submit: move |_| {},
                    }
                }
            })
        }
    }

    #[test]
    fn thread_title_is_the_one_h1_under_a_breadcrumb_with_the_board() {
        let html = Case::of(Load::Ready(detail(vec![reply(1), reply(2)], false))).render();
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(
            html.contains("<h1>Friday customs, looking for a tank</h1>"),
            "{html}"
        );
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains(
                "<nav class=\"article-crumb\" aria-label=\"Breadcrumb\"><a href=\"/forum\">Forum</a><span aria-hidden=\"true\">/</span><a href=\"/forum/b/general\">General</a><span aria-hidden=\"true\">/</span><span aria-current=\"page\">Friday customs, looking for a tank</span></nav>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("aria-current=\"page\"").count(), 1, "{html}");
    }

    #[test]
    fn a_sub_board_thread_links_the_parent_board_first() {
        let mut resp = detail(Vec::new(), false);
        resp.parent_board = Some(BoardMeta {
            name: "Overwatch".to_string(),
            slug: "overwatch".to_string(),
        });
        let html = Case::of(Load::Ready(resp)).render();
        let parent = html
            .find("href=\"/forum/b/overwatch\"")
            .expect("parent link");
        let board = html.find("href=\"/forum/b/general\"").expect("board link");
        assert!(parent < board, "{html}");
    }

    #[test]
    fn posts_show_initials_display_names_and_times_never_a_raw_id() {
        let html = Case::of(Load::Ready(detail(vec![reply(1), reply(2)], false))).render();
        // The starter is in the header line, with the reply count.
        assert!(
            html.contains(
                "<p class=\"article-byline\"><span class=\"article-avatar\" aria-hidden=\"true\">P2</span><span class=\"entry-author\">Player02</span>"
            ),
            "{html}"
        );
        assert!(html.contains("<span>2 replies</span>"), "{html}");
        assert!(
            html.contains(
                "<article class=\"forum-post\" aria-label=\"First post\"><p class=\"forum-post-body\">We have four for Friday customs.</p></article>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<h2 id=\"forum-replies-heading\" class=\"visually-hidden\">Replies</h2>"
            ),
            "{html}"
        );
        assert_eq!(
            html.matches("<article class=\"forum-post\">").count(),
            2,
            "{html}"
        );
        assert!(
            html.contains(
                "<span class=\"article-avatar\" aria-hidden=\"true\">PO</span><span class=\"entry-author\">Player One</span>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<time datetime=\"2026-11-12T18:22:00"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"forum-post-body\">Reply 1\nsecond line</p>"),
            "{html}"
        );
        for id in [OP, REPLIER] {
            assert!(!html.contains(id), "raw author id leaked: {html}");
        }

        // A failed lookup reads as the fallback; a pending one shows no name.
        let mut failed = Case::of(Load::Ready(detail(vec![reply(1)], false)));
        failed.names = Some(AuthorNames::from([(OP.to_string(), None)]));
        let failed = failed.render();
        assert_eq!(
            failed
                .matches(&format!(
                    "<span class=\"entry-author\">{AUTHOR_FALLBACK}</span>"
                ))
                .count(),
            1,
            "{failed}"
        );
        assert!(
            !failed.contains(OP) && !failed.contains(REPLIER),
            "{failed}"
        );
    }

    #[test]
    fn pinned_and_locked_are_words_and_a_locked_thread_has_no_reply_form() {
        let mut resp = detail(vec![reply(1)], true);
        resp.thread.pinned = true;
        for case in [
            Case::of(Load::Ready(resp.clone())),
            Case::of(Load::Ready(resp)).member(),
        ] {
            let html = case.render();
            assert!(
                html.contains("<span class=\"ui-pill ui-pill--neutral\">Pinned</span>"),
                "{html}"
            );
            assert!(
                html.contains("<span>This thread is locked. No new replies can be posted.</span>"),
                "{html}"
            );
            assert!(!html.contains("<form"), "{html}");
            assert!(!html.contains("<textarea"), "{html}");
        }
    }

    #[test]
    fn only_a_signed_in_viewer_gets_the_reply_form() {
        let ready = Load::Ready(detail(Vec::new(), false));
        let guest = Case::of(ready.clone()).render();
        assert!(!guest.contains("<form"), "{guest}");
        assert!(
            guest.contains("<p class=\"forum-note\">No replies yet.</p>"),
            "{guest}"
        );
        assert!(guest.contains("<span>0 replies</span>"), "{guest}");

        let member = Case::of(ready).member().render();
        assert!(member.contains("<form class=\"forum-form\""), "{member}");
        assert_eq!(member.matches("<h1").count(), 1, "{member}");
    }

    #[test]
    fn a_long_thread_pages_its_replies_on_the_server_count() {
        let mut resp = detail((1..=REPLY_PAGE_SIZE).map(reply).collect(), false);
        resp.reply_count = 75;
        let html = Case::of(Load::Ready(resp)).render();
        assert_eq!(
            html.matches("<article class=\"forum-post\">").count(),
            REPLY_PAGE_SIZE,
            "{html}"
        );
        assert!(html.contains("<span>75 replies</span>"), "{html}");
        // The replies are one section, named by its heading, that takes
        // focus after a page change.
        assert!(
            html.contains(
                "<section id=\"forum-replies\" class=\"forum-posts pager-target\" tabindex=\"-1\" aria-labelledby=\"forum-replies-heading\"><h2 id=\"forum-replies-heading\" class=\"visually-hidden\">Replies</h2>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<p id=\"forum-reply-pager-meta\" class=\"pager-meta\" role=\"status\">1 to 50 of 75 replies · Page 1 of 2</p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<nav class=\"pager-pages\" aria-label=\"Pagination\">"),
            "{html}"
        );
        assert!(html.contains("id=\"forum-reply-pager-page-2\""), "{html}");
        assert!(!html.contains("Showing the first"), "{html}");
    }

    #[test]
    fn an_address_past_the_last_reply_page_says_so() {
        let mut resp = detail(Vec::new(), false);
        resp.reply_count = 75;
        let mut case = Case::of(Load::Ready(resp));
        case.page = 9;
        let html = case.render();
        assert!(
            html.contains("<p class=\"forum-note\">No replies on this page.</p>"),
            "{html}"
        );
        assert!(!html.contains("No replies yet."), "{html}");
        assert!(html.contains("id=\"forum-reply-pager-page-1\""), "{html}");
    }

    #[test]
    fn loading_failed_and_missing_states() {
        let loading = Case::of(Load::Loading).render();
        assert!(loading.contains("<h1>Forum thread</h1>"), "{loading}");
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading thread</p>"),
            "{loading}"
        );

        let failed = Case::of(Load::Failed(NET.to_string())).render();
        assert!(failed.contains("load this thread</h1>"), "{failed}");
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(failed.contains(">Retry</button>"), "{failed}");
        assert!(!failed.contains("Loading"), "{failed}");

        let mut retrying = Case::of(Load::Failed(NET.to_string()));
        retrying.retrying = true;
        let retrying = retrying.render();
        assert!(retrying.contains(">Loading thread</p>"), "{retrying}");
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");

        // QA round 3 on 748: a 403 thread keeps the server's reason (mapped
        // and logged once, read by `Load::of`), not the not-found card.
        let _ = crate::util::take_logged();
        let err = scuffed_api_client::ClientError::http(403, r#"{"error":"Members only"}"#, None);
        let slot: Result<ThreadDetailResponse, String> =
            Err(crate::client_error::logged_message("forum thread", &err));
        assert_eq!(crate::util::take_logged().len(), 1);
        let reason = crate::client_error::user_message(&err);
        let refused = Case::of(Load::of(Some(&slot))).render();
        assert!(refused.contains("load this thread</h1>"), "{refused}");
        assert!(refused.contains(&reason), "{refused}");
        assert!(refused.contains("Members only"), "{refused}");
        assert!(!refused.contains("Thread not found"), "{refused}");
        // Vision round 6 on 748: no Retry for a refusal, the same as the
        // strategy editor 403. Vision round 7: the box is an alert, as the
        // editor 403 is, so it is announced, and the status line stays empty
        // so it is said once.
        assert!(
            refused.contains("<div class=\"fetch-error-wrap\" role=\"alert\">"),
            "{refused}"
        );
        assert_eq!(refused.matches("role=\"alert\"").count(), 1, "{refused}");
        assert!(
            refused.contains("<p class=\"entry-status\" role=\"status\"></p>"),
            "{refused}"
        );
        assert!(!refused.contains("fetch-error__retry"), "{refused}");
        assert!(!refused.contains(">Retry</button>"), "{refused}");
        for (status, body) in [(401, r#"{"error":"Log in first"}"#), (403, ""), (401, "")] {
            let err = scuffed_api_client::ClientError::http(status, body, None);
            let slot: Result<ThreadDetailResponse, String> =
                Err(crate::client_error::logged_message("forum thread", &err));
            let html = Case::of(Load::of(Some(&slot))).render();
            assert!(html.contains("load this thread</h1>"), "{status}: {html}");
            assert!(
                html.contains(&crate::client_error::user_message(&err)),
                "{html}"
            );
            assert!(!html.contains(">Retry</button>"), "{status}: {html}");
            assert!(!html.contains("fetch-error__retry"), "{status}: {html}");
        }
        // A 5xx or an unreadable answer keeps Retry.
        for err in [
            scuffed_api_client::ClientError::http(500, r#"{"error":"Database down"}"#, None),
            scuffed_api_client::ClientError::Deserialize("expected value".into()),
        ] {
            let slot: Result<ThreadDetailResponse, String> =
                Err(crate::client_error::logged_message("forum thread", &err));
            let html = Case::of(Load::of(Some(&slot))).render();
            assert!(html.contains(">Retry</button>"), "{err:?}: {html}");
            assert!(
                html.contains("<div class=\"fetch-error-wrap\" role=\"alert\">"),
                "{err:?}: {html}"
            );
        }
        let _ = crate::util::take_logged();

        // A missing thread is the not-found card, not an error page.
        let missing = Case::of(Load::Missing).render();
        assert!(missing.contains("<h1>Thread not found</h1>"), "{missing}");
        assert!(missing.contains(">Back to the forum</a>"), "{missing}");
        assert!(!missing.contains("article-crumb"), "{missing}");
        assert!(!missing.contains("role=\"alert\""), "{missing}");

        for html in [&loading, &failed, &retrying, &missing] {
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(!html.contains("<main"), "{html}");
            assert!(
                html.contains("class=\"entry-status\" role=\"status\""),
                "{html}"
            );
        }
        for html in [&loading, &failed, &retrying] {
            assert!(
                html.contains(
                    "<nav class=\"article-crumb\" aria-label=\"Breadcrumb\"><a href=\"/forum\">Forum</a></nav>"
                ),
                "{html}"
            );
        }
    }

    #[test]
    fn tab_is_named_by_the_thread_title() {
        let ready = Load::Ready(detail(Vec::new(), false));
        assert_eq!(
            tab_label(&ready).as_deref(),
            Some("Friday customs, looking for a tank | Forum")
        );
        assert_eq!(
            tab_label(&Load::Missing).as_deref(),
            Some("Thread not found | Forum")
        );
        assert_eq!(tab_label(&Load::Loading), None);
    }

    #[test]
    fn reply_field_has_id_name_and_label() {
        let html = Case::of(Load::Ready(detail(Vec::new(), false)))
            .member()
            .render();
        assert!(html.contains("id=\"forum-reply\""), "{html}");
        assert!(html.contains("name=\"forum-reply\""), "{html}");
        assert!(
            html.contains(
                "<h2 class=\"ui-section-title\"><label for=\"forum-reply\">Post a reply<span class=\"forum-req\" aria-hidden=\"true\">*</span></label></h2>"
            ),
            "{html}"
        );
        assert!(html.contains("<textarea"), "{html}");
        assert!(html.contains("aria-required=\"true\""), "{html}");
        assert!(
            !html.contains("aria-label=\"Post") && !html.contains("placeholder"),
            "the visible label is the accessible name: {html}"
        );
        // The error line exists, empty, before there is an error.
        assert!(
            html.contains("<p class=\"field-error\" id=\"forum-reply-error\" role=\"alert\"></p>"),
            "{html}"
        );
        assert!(!html.contains("aria-invalid"), "{html}");
        assert!(
            html.contains("<button type=\"submit\" class=\"btn is-primary\">Post reply</button>"),
            "{html}"
        );
    }

    #[test]
    fn empty_reply_is_an_inline_error_and_posting_keeps_the_button_focusable() {
        let mut invalid = Case::of(Load::Ready(detail(Vec::new(), false))).member();
        invalid.reply_invalid = true;
        let html = invalid.render();
        assert!(
            html.contains("aria-invalid=\"true\" aria-describedby=\"forum-reply-error\""),
            "{html}"
        );
        assert!(
            html.contains("</span>Write a reply before posting.</p>"),
            "{html}"
        );

        let mut posting = Case::of(Load::Ready(detail(Vec::new(), false))).member();
        posting.posting = true;
        let html = posting.render();
        assert!(
            html.contains("aria-busy=\"true\" aria-disabled=\"true\">Posting...</button>"),
            "{html}"
        );
        assert!(!html.contains(" disabled"), "{html}");
    }

    #[test]
    fn thread_detail_parses_the_server_shape() {
        let resp: ThreadDetailResponse = serde_json::from_str(
            r#"{"thread":{"id":"forum_thread:1","title":"T","category":"general",
                "board_id":"forum_board:1","author_member_id":"member:1","content":"C",
                "pinned":false,"locked":true,"created_at":"2026-11-12T18:10:00Z",
                "is_active":true},
                "replies":[{"id":"forum_reply:1","thread_id":"forum_thread:1",
                "author_member_id":"member:2","content":"R",
                "created_at":"2026-11-12T18:22:00Z"}],
                "reply_count":1,
                "board":{"id":"forum_board:1","name":"General","slug":"general"},
                "category":{"name":"Community"},
                "parent_board":null}"#,
        )
        .unwrap();
        assert!(resp.thread.locked);
        assert_eq!(resp.replies.len(), 1);
        assert_eq!(resp.board.unwrap().slug, "general");
        assert_eq!(resp.parent_board, None);
    }
}
