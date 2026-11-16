use dioxus::prelude::*;
use serde::Deserialize;

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, ArticleByline, ArticleCrumb, Load, author_label, entry_date,
    is_retrying, load_status, use_author_names,
};
use super::not_found::{NotFoundLink, NotFoundPanel};
use crate::components::ui::inline_confirm::{
    focus_element_by_id_soon, focus_text_start_by_id_soon,
};
use crate::components::ui::{FieldError, LoadError, PageHeader, PageShell, PageWidth};
use crate::components::{Toast, is_http_status, use_toast};
use crate::routes::Route;
use crate::state::auth::use_auth;
use crate::util::{FetchClass, classify_fetch, local_time_node, log_client_error, pluralize};
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct WikiPageData {
    id: String,
    topic: String,
    title: String,
    content_markdown: String,
    /// Member who started the page. Shown only as a display name.
    author_member_id: String,
    updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct WikiRevisionData {
    id: String,
    edited_at: String,
    revision_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct WikiRevisionsResponse {
    data: Vec<WikiRevisionData>,
}

/// Edit form and revision history. The reading view itself is the shared
/// article chrome (`ARTICLE_CSS`) plus the shared prose (`MARKDOWN_CSS`);
/// fields, buttons and the error line come from `components/ui`.
const PAGE_CSS: &str = r#"
.wiki-edit-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-stack);
  min-width: 0;
}
.wiki-edit-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
.wiki-edit-field textarea.field-input {
  min-height: max(calc(var(--space-12) * 6), 50vh);
  font-family: var(--font-mono);
}
.wiki-edit-buttons {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
/* History closed: out of the layout, but its status line stays in the tree
   so opening it announces the result. */
.wiki-revisions.is-closed {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
.wiki-revisions h2 {
  margin: 0;
  /* Focused when History opens: land below the sticky top bar. */
  scroll-margin-top: calc(var(--nav-height-used) + var(--space-section));
  color: var(--text);
  font-family: var(--font-head);
  font-size: var(--text-lg);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
}
.wiki-revisions h2:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.wiki-revision-list {
  display: grid;
  gap: var(--space-2);
  margin: 0;
  padding: 0;
  list-style: none;
}
.wiki-revision-item {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border: 1px solid var(--card-border);
  border-radius: var(--radius-control);
  background: var(--card-bg);
  color: var(--text);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
.wiki-revision-note {
  min-width: 0;
  overflow-wrap: anywhere;
}
.wiki-revision-date {
  color: var(--text-3);
  font-size: var(--text-xs);
  white-space: nowrap;
}
"#;

const SAVE_FAILED_TEXT: &str = "Couldn't save the page. Check your connection and try again.";

/// What the revision history shows.
#[derive(Debug, Clone, PartialEq)]
enum RevisionsView {
    /// History is closed: nothing is shown and nothing is said.
    Closed,
    Loading,
    /// The fetch failed for a reason a retry may fix.
    Failed,
    /// No revisions (an empty list, or the server said 404).
    Empty,
    List(Vec<WikiRevisionData>),
}

fn revisions_view(
    open: bool,
    slot: Option<&Result<Option<WikiRevisionsResponse>, String>>,
) -> RevisionsView {
    if !open {
        return RevisionsView::Closed;
    }
    match classify_fetch(slot) {
        FetchClass::Loading => RevisionsView::Loading,
        FetchClass::Error => {
            let not_found = slot
                .and_then(|r| r.as_ref().err())
                .is_some_and(|err| is_http_status(err, 404));
            if not_found {
                RevisionsView::Empty
            } else {
                RevisionsView::Failed
            }
        }
        FetchClass::Ready => match slot.and_then(|r| r.as_ref().ok()).and_then(Option::as_ref) {
            // The fetch that ran while History was closed returns no list.
            None => RevisionsView::Loading,
            Some(resp) if resp.data.is_empty() => RevisionsView::Empty,
            Some(resp) => RevisionsView::List(resp.data.clone()),
        },
    }
}

/// Text of the history status line. It is always rendered; only this changes.
fn revisions_status(view: &RevisionsView) -> String {
    match view {
        RevisionsView::Closed | RevisionsView::Failed => String::new(),
        RevisionsView::Loading => "Loading revisions".to_string(),
        RevisionsView::Empty => "No revisions yet.".to_string(),
        RevisionsView::List(rows) => pluralize(rows.len(), "revision", "revisions"),
    }
}

#[component]
pub fn WikiPage(topic: String) -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let mut page_refresh = use_signal(|| 0u32);
    let topic_clone = topic.clone();
    let page_resource = use_resource(move || {
        let t = topic_clone.clone();
        let _ = page_refresh();
        async move {
            if t.is_empty() {
                return Err("HTTP error: 404".to_string());
            }
            ApiClient::web()
                .fetch::<WikiPageData>(&format!("/api/wiki/{t}"))
                .await
                .map_err(|e| e.to_string())
        }
    });

    let topic_for_revisions = topic.clone();
    let mut show_revisions = use_signal(|| false);
    let mut rev_refresh = use_signal(|| 0u32);
    let revisions = use_resource(move || {
        let t = topic_for_revisions.clone();
        let show = show_revisions();
        let _ = rev_refresh();
        async move {
            if !show {
                return Ok(None);
            }
            ApiClient::web()
                .fetch::<WikiRevisionsResponse>(&format!("/api/wiki/{t}/revisions"))
                .await
                .map(Some)
                .map_err(|e| e.to_string())
        }
    });

    // The page row carries only the id of the member who started it.
    let names = use_author_names(move || {
        page_resource
            .read()
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|page| vec![page.author_member_id.clone()])
            .unwrap_or_default()
    });

    let mut editing = use_signal(|| false);
    let mut edit_content = use_signal(String::new);
    let mut revision_note = use_signal(String::new);
    let mut save_error = use_signal(|| Option::<&'static str>::None);

    let load = Load::of(page_resource.read().as_ref());
    let retrying = is_retrying(page_resource.state()(), &load);
    let names = names();
    let status = load_status(&load, retrying, "Loading page");
    let current = match &load {
        Load::Ready(page) => Some(page.title.clone()),
        _ => None,
    };
    crate::head::use_page_title(match &load {
        Load::Ready(page) => Some(format!("{} | Wiki", page.title.trim())),
        Load::Missing => Some("Page not found | Wiki".to_string()),
        Load::Loading | Load::Failed => None,
    });
    let rev_view = revisions_view(show_revisions(), revisions.read().as_ref());
    let rev_retrying =
        revisions.state()() == UseResourceState::Pending && rev_view == RevisionsView::Failed;

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-wiki_page-page-css", css: PAGE_CSS }
        crate::styles::StyleOnce { id: crate::markdown::MARKDOWN_CSS_ID, css: crate::markdown::MARKDOWN_CSS }

        PageShell { width: PageWidth::Narrow, class: "wiki-detail",
            // The not-found card has its own "Back to the wiki" link.
            if load != Load::Missing {
                ArticleCrumb { to: Route::Wiki {}, section: "Wiki", current }
            }
            {wiki_state_head(&load)}
            p { class: "entry-status", role: "status", "{status}" }

            {match &load {
                Load::Loading => rsx! {},
                Load::Missing => wiki_missing(),
                Load::Failed => rsx! {
                    LoadError {
                        message: "Check your connection and try again.",
                        on_retry: move |_| page_refresh += 1,
                        retrying,
                    }
                },
                Load::Ready(page) => {
                    let content = page.content_markdown.clone();
                    let page_topic = page.topic.clone();
                    let ids = WikiIds::new(&page.id);
                    let save_text = save_error().unwrap_or_default();

                    rsx! {
                        WikiHead {
                            title: page.title.clone(),
                            author: author_label(Some(&names), &page.author_member_id),
                            updated_at: page.updated_at.clone(),
                            can_edit: auth().is_logged_in() && !editing(),
                            history_open: show_revisions(),
                            edit_id: ids.edit.clone(),
                            history_panel_id: ids.history_panel.clone(),
                            on_edit: {
                                let c = content.clone();
                                let content_id = ids.content.clone();
                                move |_| {
                                    edit_content.set(c.clone());
                                    revision_note.set(String::new());
                                    editing.set(true);
                                    // The Edit button unmounts; the form takes
                                    // focus, at the first line of the page.
                                    focus_text_start_by_id_soon(content_id.clone());
                                }
                            },
                            on_history: {
                                let title_id = ids.history_title.clone();
                                move |_| {
                                    let open = !show_revisions();
                                    show_revisions.set(open);
                                    // The panel is below the article, so opening it
                                    // moves focus (and the view) to its heading.
                                    // Closing leaves focus on the button.
                                    if open {
                                        focus_element_by_id_soon(title_id.clone());
                                    }
                                }
                            },
                        }

                        if editing() {
                            div { class: "wiki-edit-form",
                                div { class: "wiki-edit-field",
                                    label { class: "field-label", r#for: "{ids.content}",
                                        "Page content (markdown)"
                                    }
                                    textarea {
                                        id: "{ids.content}",
                                        class: "field-input",
                                        aria_invalid: if save_error().is_some() { "true" },
                                        aria_describedby: "{ids.save_error}",
                                        value: "{edit_content}",
                                        oninput: move |e| edit_content.set(e.value()),
                                    }
                                }
                                div { class: "wiki-edit-field",
                                    label { class: "field-label", r#for: "{ids.note}",
                                        "Revision note (optional)"
                                    }
                                    input {
                                        id: "{ids.note}",
                                        class: "field-input",
                                        r#type: "text",
                                        value: "{revision_note}",
                                        oninput: move |e| revision_note.set(e.value()),
                                    }
                                }
                                // Always rendered, so a failed save is announced.
                                FieldError { id: ids.save_error.clone(), text: save_text, role: "alert" }
                                div { class: "wiki-edit-buttons",
                                    button {
                                        r#type: "button",
                                        class: "btn is-primary",
                                        onclick: {
                                            let pt = page_topic.clone();
                                            let edit_id = ids.edit.clone();
                                            move |_| {
                                                let pt = pt.clone();
                                                let edit_id = edit_id.clone();
                                                let content = edit_content();
                                                let note = revision_note();
                                                let note_opt = if note.is_empty() { None } else { Some(note) };
                                                spawn(async move {
                                                    #[derive(serde::Serialize)]
                                                    struct UpdateBody {
                                                        content_markdown: String,
                                                        revision_note: Option<String>,
                                                    }
                                                    let body = UpdateBody {
                                                        content_markdown: content,
                                                        revision_note: note_opt,
                                                    };
                                                    match ApiClient::web()
                                                        .put_json::<_, WikiPageData>(
                                                            &format!("/api/wiki/{pt}"),
                                                            &body,
                                                        )
                                                        .await
                                                    {
                                                        Ok(_) => {
                                                            editing.set(false);
                                                            save_error.set(None);
                                                            page_refresh += 1;
                                                            rev_refresh += 1;
                                                            toast.show(Toast::success("Page saved."));
                                                            // The form unmounts; Edit is back.
                                                            focus_element_by_id_soon(edit_id);
                                                        }
                                                        Err(e) => {
                                                            log_client_error("wiki save", &e);
                                                            save_error.set(Some(SAVE_FAILED_TEXT));
                                                        }
                                                    }
                                                });
                                            }
                                        },
                                        "Save"
                                    }
                                    button {
                                        r#type: "button",
                                        class: "btn",
                                        onclick: {
                                            let edit_id = ids.edit.clone();
                                            move |_| {
                                                editing.set(false);
                                                save_error.set(None);
                                                focus_element_by_id_soon(edit_id.clone());
                                            }
                                        },
                                        "Cancel"
                                    }
                                }
                            }
                        } else {
                            div {
                                class: "md-body",
                                dangerous_inner_html: crate::markdown::render_markdown(&content),
                            }
                        }

                        WikiRevisions {
                            panel_id: ids.history_panel.clone(),
                            title_id: ids.history_title.clone(),
                            view: rev_view.clone(),
                            retrying: rev_retrying,
                            on_retry: move |_| rev_refresh += 1,
                        }
                    }
                }
            }}
        }
    }
}

/// Element ids of one wiki page's controls, all built from its record id.
struct WikiIds {
    content: String,
    note: String,
    save_error: String,
    edit: String,
    history_panel: String,
    history_title: String,
}

impl WikiIds {
    fn new(page_id: &str) -> Self {
        WikiIds {
            content: format!("wiki-{page_id}-content"),
            note: format!("wiki-{page_id}-note"),
            save_error: format!("wiki-{page_id}-save-error"),
            edit: format!("wiki-{page_id}-edit"),
            history_panel: format!("wiki-{page_id}-history-panel"),
            history_title: format!("wiki-{page_id}-history"),
        }
    }
}

/// The H1 while there is no page title to show: loading and a failed load.
/// A loaded page and a missing one bring their own.
fn wiki_state_head(load: &Load<WikiPageData>) -> Element {
    match load {
        Load::Loading => rsx! { PageHeader { title: "Wiki page" } },
        Load::Failed => rsx! { PageHeader { title: "Couldn't load this page" } },
        Load::Missing | Load::Ready(_) => rsx! {},
    }
}

/// Title, author line and the Edit and History controls.
///
/// The row only names who started the page, so the line reads
/// "Player03 · Updated 10 Nov 2026" and does not claim who made the update.
/// History keeps one label; `aria-expanded` carries its state.
#[component]
fn WikiHead(
    title: String,
    author: Option<String>,
    updated_at: String,
    can_edit: bool,
    history_open: bool,
    edit_id: String,
    history_panel_id: String,
    on_edit: EventHandler<()>,
    on_history: EventHandler<()>,
) -> Element {
    rsx! {
        PageHeader {
            title,
            meta: rsx! {
                ArticleByline { author,
                    span { "Updated" }
                    {entry_date(&updated_at)}
                }
            },
            actions: rsx! {
                if can_edit {
                    button {
                        r#type: "button",
                        id: "{edit_id}",
                        class: "btn is-primary",
                        onclick: move |_| on_edit.call(()),
                        "Edit"
                    }
                }
                button {
                    r#type: "button",
                    class: "btn",
                    aria_expanded: if history_open { "true" } else { "false" },
                    aria_controls: "{history_panel_id}",
                    onclick: move |_| on_history.call(()),
                    "History"
                }
            },
        }
    }
}

/// Revision history. The section and its status line are always rendered, so
/// "Loading revisions" and the result are announced as text changes; closed,
/// the section is out of the layout.
#[component]
fn WikiRevisions(
    panel_id: String,
    title_id: String,
    view: RevisionsView,
    #[props(default = false)] retrying: bool,
    on_retry: EventHandler<()>,
) -> Element {
    let open = view != RevisionsView::Closed;
    let status = revisions_status(&view);
    rsx! {
        section {
            id: "{panel_id}",
            class: if open { "wiki-revisions ui-stack" } else { "wiki-revisions is-closed" },
            aria_labelledby: open.then(|| title_id.clone()),
            if open {
                h2 { id: "{title_id}", tabindex: "-1", "Revision history" }
            }
            p { class: "entry-status", role: "status", "{status}" }
            {match &view {
                RevisionsView::Failed => rsx! {
                    LoadError {
                        message: "Couldn't load revisions. Check your connection and try again.",
                        on_retry: move |_| on_retry.call(()),
                        retrying,
                    }
                },
                RevisionsView::List(rows) => rsx! {
                    ul { class: "wiki-revision-list",
                        for rev in rows.iter() {
                            {render_revision(rev)}
                        }
                    }
                },
                RevisionsView::Closed | RevisionsView::Loading | RevisionsView::Empty => rsx! {},
            }}
        }
    }
}

/// The shared not-found card for a missing topic: the page's one H1 and a
/// way back.
fn wiki_missing() -> Element {
    rsx! {
        NotFoundPanel {
            title: "Page not found",
            message: "This wiki page does not exist or was removed.",
            links: vec![
                NotFoundLink { label: "Back to the wiki", to: Route::Wiki {} },
                NotFoundLink { label: "Return home", to: Route::Home {} },
            ],
        }
    }
}

fn render_revision(rev: &WikiRevisionData) -> Element {
    let note = rev.revision_note.as_deref().unwrap_or("No note");

    rsx! {
        li { key: "{rev.id}", class: "wiki-revision-item",
            span { class: "wiki-revision-note", "{note}" }
            span { class: "wiki-revision-date", {local_time_node(&rev.edited_at)} }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::article_author::test_support::render_routed;
    use super::*;
    use crate::components::ui::test_support::assert_sizes_from_tokens;

    #[derive(Props, Clone, PartialEq)]
    struct HeadProps {
        author: Option<String>,
        can_edit: bool,
        history_open: bool,
    }

    #[allow(non_snake_case)]
    fn HeadProbe(props: HeadProps) -> Element {
        rsx! {
            WikiHead {
                title: "Tank basics",
                author: props.author,
                updated_at: "2026-11-10T08:00:00Z",
                can_edit: props.can_edit,
                history_open: props.history_open,
                edit_id: "wiki-wiki_page:1-edit",
                history_panel_id: "wiki-wiki_page:1-history-panel",
                on_edit: move |_| {},
                on_history: move |_| {},
            }
        }
    }

    fn head(author: Option<&str>, can_edit: bool, history_open: bool) -> String {
        let mut dom = VirtualDom::new_with_props(
            HeadProbe,
            HeadProps {
                author: author.map(str::to_string),
                can_edit,
                history_open,
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn head_shows_title_author_name_and_update_date() {
        let html = head(Some("Player03"), false, false);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Tank basics</h1>"), "{html}");
        assert!(
            html.contains("<span class=\"article-avatar\" aria-hidden=\"true\">PL</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"entry-author\">Player03</span>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<span>Updated</span><time class=\"entry-date\" datetime=\"2026-11-10\">10 Nov 2026</time>"
            ),
            "{html}"
        );
    }

    #[test]
    fn author_stays_out_while_its_lookup_is_pending() {
        let html = head(None, false, false);
        assert!(!html.contains("article-avatar"), "{html}");
        assert!(!html.contains("entry-author"), "{html}");
        assert!(html.contains("10 Nov 2026"), "{html}");
    }

    #[test]
    fn edit_shows_only_when_allowed_and_history_reports_its_state() {
        let reader = head(Some("Player03"), false, false);
        assert!(!reader.contains(">Edit</button>"), "{reader}");
        assert!(reader.contains("aria-expanded=\"false\""), "{reader}");
        assert!(reader.contains(">History</button>"), "{reader}");

        let editor = head(Some("Player03"), true, true);
        // Edit has an id from the page record, so focus can return to it.
        assert!(
            editor.contains(
                "<button type=\"button\" id=\"wiki-wiki_page:1-edit\" class=\"btn is-primary\">Edit</button>"
            ),
            "{editor}"
        );
        // One label in both states: aria-expanded carries the state, and the
        // button names the section it opens.
        for html in [&reader, &editor] {
            assert!(html.contains(">History</button>"), "{html}");
            assert!(!html.contains("Hide"), "{html}");
            assert!(
                html.contains("aria-controls=\"wiki-wiki_page:1-history-panel\""),
                "{html}"
            );
        }
        assert!(editor.contains("aria-expanded=\"true\""), "{editor}");
        assert_eq!(editor.matches("type=\"button\"").count(), 2, "{editor}");
    }

    fn revision(n: usize) -> WikiRevisionData {
        WikiRevisionData {
            id: format!("wiki_revision:{n}"),
            edited_at: "2026-11-10T08:00:00Z".to_string(),
            revision_note: Some(format!("Edit {n}")),
        }
    }

    #[test]
    fn revisions_view_follows_the_fetch() {
        let list = Ok(Some(WikiRevisionsResponse {
            data: vec![revision(1)],
        }));
        let empty = Ok(Some(WikiRevisionsResponse { data: vec![] }));
        let skipped: Result<Option<WikiRevisionsResponse>, String> = Ok(None);
        let gone = Err("HTTP error: 404".to_string());
        let down = Err("HTTP error: 500".to_string());
        assert_eq!(revisions_view(false, Some(&list)), RevisionsView::Closed);
        assert_eq!(revisions_view(true, None), RevisionsView::Loading);
        assert_eq!(revisions_view(true, Some(&skipped)), RevisionsView::Loading);
        assert_eq!(revisions_view(true, Some(&empty)), RevisionsView::Empty);
        assert_eq!(revisions_view(true, Some(&gone)), RevisionsView::Empty);
        assert_eq!(revisions_view(true, Some(&down)), RevisionsView::Failed);
        assert_eq!(
            revisions_view(true, Some(&list)),
            RevisionsView::List(vec![revision(1)])
        );

        assert_eq!(revisions_status(&RevisionsView::Closed), "");
        assert_eq!(revisions_status(&RevisionsView::Failed), "");
        assert_eq!(
            revisions_status(&RevisionsView::Loading),
            "Loading revisions"
        );
        assert_eq!(revisions_status(&RevisionsView::Empty), "No revisions yet.");
        assert_eq!(
            revisions_status(&RevisionsView::List(vec![revision(1)])),
            "1 revision"
        );
        assert_eq!(
            revisions_status(&RevisionsView::List(vec![revision(1), revision(2)])),
            "2 revisions"
        );
    }

    #[derive(Props, Clone, PartialEq)]
    struct RevProps {
        view: RevisionsView,
        retrying: bool,
    }

    #[allow(non_snake_case)]
    fn RevProbe(props: RevProps) -> Element {
        rsx! {
            WikiRevisions {
                panel_id: "wiki-p1-history-panel",
                title_id: "wiki-p1-history",
                view: props.view,
                retrying: props.retrying,
                on_retry: move |_| {},
            }
        }
    }

    fn revisions(view: RevisionsView, retrying: bool) -> String {
        let mut dom = VirtualDom::new_with_props(RevProbe, RevProps { view, retrying });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn history_status_line_is_always_rendered_and_only_its_text_changes() {
        let closed = revisions(RevisionsView::Closed, false);
        assert_eq!(
            closed,
            "<section id=\"wiki-p1-history-panel\" class=\"wiki-revisions is-closed\">\
             <p class=\"entry-status\" role=\"status\"></p></section>"
        );

        let loading = revisions(RevisionsView::Loading, false);
        assert!(
            loading.contains(
                "<section id=\"wiki-p1-history-panel\" class=\"wiki-revisions ui-stack\" aria-labelledby=\"wiki-p1-history\">"
            ),
            "{loading}"
        );
        assert!(
            loading.contains("<h2 id=\"wiki-p1-history\" tabindex=\"-1\">Revision history</h2>"),
            "{loading}"
        );
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading revisions</p>"),
            "{loading}"
        );

        let list = revisions(RevisionsView::List(vec![revision(1), revision(2)]), false);
        assert!(
            list.contains("<p class=\"entry-status\" role=\"status\">2 revisions</p>"),
            "{list}"
        );
        assert_eq!(list.matches("<li class=\"wiki-revision-item\">").count(), 2);

        let failed = revisions(RevisionsView::Failed, true);
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(failed.contains(">Retrying...</button>"), "{failed}");
        assert!(failed.contains("aria-disabled=\"true\""), "{failed}");

        for html in [&closed, &loading, &list, &failed] {
            assert_eq!(html.matches("role=\"status\"").count(), 1, "{html}");
            assert_eq!(html.matches("id=\"wiki-p1-history-panel\"").count(), 1);
        }
        // Closed leaves the layout but stays in the tree.
        let rule =
            crate::components::ui::test_support::css_rule(PAGE_CSS, ".wiki-revisions.is-closed {");
        assert!(rule.contains("position: absolute"), "{rule}");
        assert!(!rule.contains("display"), "{rule}");
    }

    #[test]
    fn loading_and_failed_states_have_their_own_h1() {
        fn loading() -> Element {
            wiki_state_head(&Load::Loading)
        }
        fn failed() -> Element {
            wiki_state_head(&Load::Failed)
        }
        fn missing() -> Element {
            wiki_state_head(&Load::Missing)
        }
        let render = |app: fn() -> Element| {
            let mut dom = VirtualDom::new(app);
            dom.rebuild_in_place();
            dioxus_ssr::render(&dom)
        };
        assert!(render(loading).contains("<h1>Wiki page</h1>"));
        assert!(render(failed).contains("<h1>Couldn&#39;t load this page</h1>"));
        // Missing and loaded pages bring their own H1.
        assert!(!render(missing).contains("<h1"));
    }

    #[test]
    fn control_ids_come_from_the_page_record() {
        let ids = WikiIds::new("wiki_page:7");
        let all = [
            &ids.content,
            &ids.note,
            &ids.save_error,
            &ids.edit,
            &ids.history_panel,
            &ids.history_title,
        ];
        for id in all {
            assert!(id.starts_with("wiki-wiki_page:7-"), "{id}");
        }
        let mut unique: Vec<&String> = all.to_vec();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn missing_topic_is_a_not_found_page_with_a_way_back() {
        let html = render_routed(wiki_missing);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Page not found</h1>"), "{html}");
        // The shared not-found card, with a way back as its first action.
        assert!(html.contains("<section class=\"wall\">"), "{html}");
        assert!(
            html.contains("href=\"/wiki\" class=\"btn is-primary\">Back to the wiki</a>"),
            "{html}"
        );
        assert!(html.contains(">Return home</a>"), "{html}");
    }

    #[test]
    fn revision_rows_show_the_note_or_a_placeholder() {
        fn app() -> Element {
            let with_note = WikiRevisionData {
                id: "wiki_revision:1".to_string(),
                edited_at: "2026-11-10T08:00:00Z".to_string(),
                revision_note: Some("Fixed the cooldown".to_string()),
            };
            let without = WikiRevisionData {
                id: "wiki_revision:2".to_string(),
                edited_at: "2026-11-09T08:00:00Z".to_string(),
                revision_note: None,
            };
            rsx! {
                ul {
                    {render_revision(&with_note)}
                    {render_revision(&without)}
                }
            }
        }
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert_eq!(
            html.matches("<li class=\"wiki-revision-item\">").count(),
            2,
            "{html}"
        );
        assert!(html.contains(">Fixed the cooldown</span>"), "{html}");
        assert!(html.contains(">No note</span>"), "{html}");
        assert!(
            html.contains("<time datetime=\"2026-11-10T08:00:00Z\""),
            "{html}"
        );
    }

    #[test]
    fn edit_and_history_styles_use_tokens_only() {
        assert_sizes_from_tokens(PAGE_CSS);
    }
}
