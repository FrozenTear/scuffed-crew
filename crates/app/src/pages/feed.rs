use dioxus::prelude::*;

use super::article_author::{ARTICLE_CSS, ARTICLE_CSS_ID, Load, is_retrying};
use crate::components::post::{FeedPost, PostCard, PostCompose};
use crate::components::ui::{EmptyCard, LoadError, PageHeader, PageShell, PageWidth, SkeletonBars};
use crate::util::{log_client_error, pluralize};
use scuffed_api_client::ApiClient;

const PAGE_CSS: &str = r#"
.feed-page .feed-list {
  display: grid;
  gap: var(--space-section);
  min-width: 0;
}
.feed-page .feed-clear { align-self: flex-start; }
"#;

/// Posts sent from this page that the server list does not hold yet, then the
/// server list. A post already in the first group is not repeated.
fn merge_posts(optimistic: &[FeedPost], server: &[FeedPost]) -> Vec<FeedPost> {
    let mut all = optimistic.to_vec();
    for post in server {
        if !all.iter().any(|p| p.id == post.id) {
            all.push(post.clone());
        }
    }
    all
}

/// The posts carrying `tag` (any case), or all of them with no tag set.
fn filter_posts<'a>(posts: &'a [FeedPost], tag: Option<&str>) -> Vec<&'a FeedPost> {
    posts
        .iter()
        .filter(|p| match tag {
            Some(tag) => p.hashtags.iter().any(|t| t.eq_ignore_ascii_case(tag)),
            None => true,
        })
        .collect()
}

/// A tag click: the same tag again takes the filter off.
fn toggle_tag(current: Option<&str>, clicked: &str) -> Option<String> {
    match current {
        Some(current) if current.eq_ignore_ascii_case(clicked) => None,
        _ => Some(clicked.to_string()),
    }
}

/// Text of the status line: the loading words during a load or a retry, the
/// filter summary while a tag is set, empty otherwise.
fn feed_status(loading: bool, shown: usize, tag: Option<&str>) -> String {
    match tag {
        _ if loading => "Loading posts".to_string(),
        Some(tag) => format!("{} tagged #{tag}", pluralize(shown, "post", "posts")),
        None => String::new(),
    }
}

#[component]
pub fn Feed() -> Element {
    let mut refresh = use_signal(|| 0u32);
    let posts_resource = use_resource(move || {
        let _ = refresh();
        async move {
            ApiClient::web()
                .fetch::<Vec<FeedPost>>("/api/nostr/feed")
                .await
                .map_err(|e| {
                    log_client_error("feed", &e);
                    e.to_string()
                })
        }
    });

    let me = use_resource(|| async {
        ApiClient::web()
            .fetch::<scuffed_types::MeResponse>("/api/auth/me")
            .await
            .ok()
    });

    let mut optimistic_posts = use_signal(Vec::<FeedPost>::new);
    let mut tag_filter = use_signal(|| Option::<String>::None);

    let is_member = me
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .and_then(|m| m.member.as_ref())
        .is_some();

    let load = Load::of(posts_resource.read().as_ref());
    let retrying = is_retrying(posts_resource.state()(), &load);

    rsx! {
        FeedView {
            load,
            optimistic: optimistic_posts(),
            tag: tag_filter(),
            is_member,
            retrying,
            on_retry: move |_| refresh += 1,
            on_post: move |post: FeedPost| optimistic_posts.write().insert(0, post),
            on_post_failed: move |id: String| optimistic_posts.write().retain(|p| p.id != id),
            on_tag: move |tag: String| {
                let next = toggle_tag(tag_filter.peek().as_deref(), &tag);
                tag_filter.set(next);
            },
            on_clear: move |_| tag_filter.set(None),
        }
    }
}

/// The page for one fetch state. The status line is always in the tree; only
/// its text changes.
#[component]
fn FeedView(
    load: Load<Vec<FeedPost>>,
    /// Posts sent from this page, newest first.
    optimistic: Vec<FeedPost>,
    tag: Option<String>,
    is_member: bool,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    on_retry: EventHandler<()>,
    on_post: EventHandler<FeedPost>,
    on_post_failed: EventHandler<String>,
    on_tag: EventHandler<String>,
    on_clear: EventHandler<()>,
) -> Element {
    let server: &[FeedPost] = match &load {
        Load::Ready(list) => list,
        _ => &[],
    };
    let all = merge_posts(&optimistic, server);
    let shown = filter_posts(&all, tag.as_deref());
    let loading = retrying || matches!(load, Load::Loading);
    let status = feed_status(loading, shown.len(), tag.as_deref());
    let ready = matches!(load, Load::Ready(_));

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-feed-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Default, class: "feed-page",
            PageHeader { title: "Feed", lead: "Short notes from the crew." }

            if is_member {
                PostCompose {
                    on_post: move |post| on_post.call(post),
                    on_failed: move |id| on_post_failed.call(id),
                }
            }

            section { id: "feed-list", class: "ui-stack", aria_label: "Posts",
                p { class: "entry-status", role: "status", "{status}" }
                if tag.is_some() {
                    button {
                        r#type: "button",
                        class: "btn is-ghost feed-clear",
                        onclick: move |_| on_clear.call(()),
                        "Clear tag filter"
                    }
                }
                if !shown.is_empty() {
                    div { class: "feed-list",
                        for post in shown.iter() {
                            PostCard {
                                key: "{post.id}",
                                post: (*post).clone(),
                                on_tag_click: move |tag| on_tag.call(tag),
                                active_tag: tag.clone(),
                            }
                        }
                    }
                }
                {match &load {
                    Load::Loading => rsx! { SkeletonBars { rows: 3 } },
                    Load::Failed | Load::Missing => rsx! {
                        LoadError {
                            message: "Couldn't load the feed. Check your connection and try again.",
                            on_retry: move |_| on_retry.call(()),
                            retrying,
                        }
                    },
                    Load::Ready(_) => rsx! {},
                }}
                if ready && shown.is_empty() {
                    if tag.is_some() {
                        EmptyCard {
                            title: "No posts with that tag",
                            message: "Clear the tag filter to see every post.",
                        }
                    } else if is_member {
                        EmptyCard {
                            title: "No posts yet",
                            message: "Be the first to share something with the crew.",
                        }
                    } else {
                        EmptyCard {
                            title: "No posts yet",
                            message: "Notes from the crew will show up here.",
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ToastProvider;

    fn post(n: usize, tags: &[&str]) -> FeedPost {
        FeedPost {
            id: format!("note{n}"),
            pubkey: format!("key{n}"),
            author_name: Some(format!("Player{n:02}")),
            content: format!("Update {n}"),
            hashtags: tags.iter().map(|t| t.to_string()).collect(),
            // 12 Nov 2026, 17:40 UTC.
            created_at: 1_794_505_200,
            reactions: Vec::new(),
            reply_count: 0,
        }
    }

    #[derive(Props, Clone, PartialEq)]
    struct ProbeProps {
        load: Load<Vec<FeedPost>>,
        optimistic: Vec<FeedPost>,
        tag: Option<String>,
        is_member: bool,
        retrying: bool,
    }

    #[allow(non_snake_case)]
    fn Probe(props: ProbeProps) -> Element {
        rsx! {
            ToastProvider {
                FeedView {
                    load: props.load,
                    optimistic: props.optimistic,
                    tag: props.tag,
                    is_member: props.is_member,
                    retrying: props.retrying,
                    on_retry: move |_| {},
                    on_post: move |_| {},
                    on_post_failed: move |_| {},
                    on_tag: move |_| {},
                    on_clear: move |_| {},
                }
            }
        }
    }

    fn render(props: ProbeProps) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, props);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn state(load: Load<Vec<FeedPost>>, is_member: bool) -> ProbeProps {
        ProbeProps {
            load,
            optimistic: Vec::new(),
            tag: None,
            is_member,
            retrying: false,
        }
    }

    /// The page body without the toast regions the probe adds around it.
    fn page(html: &str) -> &str {
        html.split_once("<div class=\"toast-container\"")
            .map_or(html, |(page, _)| page)
    }

    const STATUS_EMPTY: &str = "<p class=\"entry-status\" role=\"status\"></p>";

    #[test]
    fn merge_keeps_sent_posts_first_and_drops_repeats() {
        let sent = vec![post(9, &[]), post(1, &[])];
        let server = vec![post(1, &[]), post(2, &[])];
        let ids: Vec<String> = merge_posts(&sent, &server)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["note9", "note1", "note2"]);
    }

    #[test]
    fn tag_filter_ignores_case_and_toggles_off() {
        let posts = vec![post(1, &["LFG"]), post(2, &["customs"]), post(3, &[])];
        assert_eq!(filter_posts(&posts, None).len(), 3);
        let lfg = filter_posts(&posts, Some("lfg"));
        assert_eq!(lfg.len(), 1);
        assert_eq!(lfg[0].id, "note1");

        assert_eq!(toggle_tag(None, "lfg"), Some("lfg".to_string()));
        assert_eq!(toggle_tag(Some("LFG"), "lfg"), None);
        assert_eq!(
            toggle_tag(Some("lfg"), "customs"),
            Some("customs".to_string())
        );
    }

    #[test]
    fn status_text_per_state() {
        assert_eq!(feed_status(true, 0, None), "Loading posts");
        assert_eq!(feed_status(true, 2, Some("lfg")), "Loading posts");
        assert_eq!(feed_status(false, 2, None), "");
        assert_eq!(feed_status(false, 2, Some("lfg")), "2 posts tagged #lfg");
        assert_eq!(feed_status(false, 1, Some("lfg")), "1 post tagged #lfg");
        assert_eq!(feed_status(false, 0, Some("lfg")), "0 posts tagged #lfg");
    }

    #[test]
    fn member_gets_the_composer_then_the_posts() {
        let html = render(state(Load::Ready(vec![post(2, &[]), post(1, &[])]), true));
        let html = page(&html);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Feed</h1>"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        // No protocol labels.
        for word in ["Nostr", "Kind 1", "KIND 1"] {
            assert!(!html.contains(word), "{word}: {html}");
        }
        let composer = html.find("class=\"post-compose\"").expect("composer");
        let first = html
            .find("aria-label=\"Post by Player02\"")
            .expect("post 2");
        let second = html
            .find("aria-label=\"Post by Player01\"")
            .expect("post 1");
        assert!(composer < first && first < second, "{html}");
        assert!(html.contains(STATUS_EMPTY), "{html}");
        assert!(!html.contains("Clear tag filter"), "{html}");
        // Raw keys stay out of the page.
        assert!(!html.contains("key1"), "{html}");
    }

    #[test]
    fn guest_reads_the_feed_without_a_composer() {
        let html = render(state(Load::Ready(vec![post(1, &[])]), false));
        let html = page(&html);
        assert!(!html.contains("post-compose"), "{html}");
        assert!(html.contains("aria-label=\"Post by Player01\""), "{html}");
    }

    #[test]
    fn loading_failed_and_empty_states() {
        let loading = render(state(Load::Loading, true));
        let loading = page(&loading);
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading posts</p>"),
            "{loading}"
        );
        assert!(
            loading.contains("class=\"ui-skel-list\" aria-hidden=\"true\""),
            "{loading}"
        );
        assert!(!loading.contains("No posts yet"), "{loading}");

        let failed = render(state(Load::Failed, true));
        let failed = page(&failed);
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains("load the feed. Check your connection"),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");
        assert!(failed.contains(STATUS_EMPTY), "{failed}");
        // The composer stays usable while the list is down.
        assert!(failed.contains("class=\"post-compose\""), "{failed}");

        let mut retry = state(Load::Failed, true);
        retry.retrying = true;
        let retrying = render(retry);
        let retrying = page(&retrying);
        assert!(retrying.contains(">Loading posts</p>"), "{retrying}");
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
        assert!(!retrying.contains(" disabled"), "{retrying}");

        let empty = render(state(Load::Ready(Vec::new()), true));
        let empty = page(&empty);
        assert!(empty.contains("<h2>No posts yet</h2>"), "{empty}");
        assert!(empty.contains("Be the first to share"), "{empty}");

        let guest_empty = render(state(Load::Ready(Vec::new()), false));
        let guest_empty = page(&guest_empty);
        assert!(
            guest_empty.contains("Notes from the crew will show up here."),
            "{guest_empty}"
        );

        for html in [loading, failed, retrying, empty, guest_empty] {
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(!html.contains("<main"), "{html}");
            assert_eq!(
                html.matches("class=\"entry-status\" role=\"status\"")
                    .count(),
                1,
                "{html}"
            );
            assert!(
                html.contains("<section id=\"feed-list\" class=\"ui-stack\" aria-label=\"Posts\">"),
                "{html}"
            );
        }
    }

    #[test]
    fn a_sent_post_shows_while_the_list_is_loading_or_failed() {
        for load in [Load::Loading, Load::Failed] {
            let mut props = state(load, true);
            props.optimistic = vec![post(7, &[])];
            let html = render(props);
            let html = page(&html);
            assert!(html.contains("aria-label=\"Post by Player07\""), "{html}");
        }
    }

    #[test]
    fn tag_filter_names_the_tag_and_offers_a_way_out() {
        let posts = vec![post(1, &["lfg"]), post(2, &["customs"])];
        let mut props = state(Load::Ready(posts.clone()), false);
        props.tag = Some("lfg".to_string());
        let html = render(props);
        let html = page(&html);
        assert!(
            html.contains("<p class=\"entry-status\" role=\"status\">1 post tagged #lfg</p>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" class=\"btn is-ghost feed-clear\">Clear tag filter</button>"
            ),
            "{html}"
        );
        assert!(html.contains("Post by Player01"), "{html}");
        assert!(!html.contains("Post by Player02"), "{html}");
        assert!(html.contains("aria-pressed=\"true\""), "{html}");

        let mut props = state(Load::Ready(posts), false);
        props.tag = Some("scrims".to_string());
        let html = render(props);
        let html = page(&html);
        assert!(html.contains("<h2>No posts with that tag</h2>"), "{html}");
        assert!(html.contains(">Clear tag filter</button>"), "{html}");
        assert!(html.contains(">0 posts tagged #scrims</p>"), "{html}");
    }
}
