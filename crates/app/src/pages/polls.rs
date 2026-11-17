use dioxus::prelude::*;

use super::article_author::{AuthorNames, author_label, short_date, use_author_names};
use crate::components::member_gate::member_pending;
use crate::components::poll::{PollCard, PollCloseDate, PollCreate, PollResultsData};
use crate::components::ui::inline_confirm::focus_element_by_id_soon;
use crate::components::ui::{EmptyCard, PageHeader, PageShell, PageWidth};
use crate::components::{Toast, use_toast};
use crate::hooks::use_api;
use crate::state::use_auth;

/// The list section. It takes focus after a poll is removed, so focus is not
/// left on a button that is gone.
const LIST_ID: &str = "polls-list";

const PAGE_CSS: &str = r#"
.polls-list {
  display: grid;
  gap: var(--space-stack);
  min-width: 0;
}
"#;

/// The close date for a card: `2026-11-16T20:00:00Z` as "16 Nov 2026". The
/// raw text when the value is not a date.
fn close_date(close_at: Option<&str>) -> Option<PollCloseDate> {
    let close_at = close_at?;
    Some(match short_date(close_at) {
        Some((datetime, text)) => PollCloseDate { datetime, text },
        None => PollCloseDate {
            datetime: close_at.to_string(),
            text: close_at.to_string(),
        },
    })
}

#[component]
pub fn Polls() -> Element {
    let auth = use_auth();
    let mut polls = use_api::<Vec<PollResultsData>>("/api/polls");
    let mut toast = use_toast();

    let is_logged_in = auth().is_logged_in();
    let is_officer = auth().is_officer_or_above();

    // Names of the members who posted the polls on screen.
    let names = use_author_names(move || {
        polls
            .data
            .read()
            .as_ref()
            .and_then(|slot| slot.as_ref())
            .map(|list| list.iter().map(|p| p.poll.created_by.clone()).collect())
            .unwrap_or_default()
    });

    let data = polls.data.read();
    let data = data.as_ref().and_then(|d| d.as_ref());

    rsx! {
        PollsLayout {
            if is_officer {
                PollCreate { on_created: move |_| polls.refresh += 1 }
            }

            // Named only once it holds polls: the loading line and the walls
            // are not "Open polls".
            section {
                id: LIST_ID,
                class: "ui-stack pager-target",
                tabindex: "-1",
                aria_label: if data.is_some() { "Open polls" },
                {match data {
                    None => member_pending(&auth(), &polls, "polls"),
                    Some(list) => rsx! {
                        PollList {
                            polls: list.clone(),
                            names: names(),
                            is_member: is_logged_in,
                            is_officer,
                            on_change: move |_| polls.refresh += 1,
                            // The toast runs on the page: the card that asked
                            // is unmounted by the reload.
                            on_removed: move |_| {
                                toast.show(Toast::success("Poll removed"));
                                focus_element_by_id_soon(LIST_ID.to_string());
                                polls.refresh += 1;
                            },
                        }
                    },
                }}
            }
        }
    }
}

/// Page container and the page's one H1.
#[component]
fn PollsLayout(children: Element) -> Element {
    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-polls-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Default, class: "polls-page",
            PageHeader {
                title: "Polls",
                lead: "Vote on crew decisions. Counts update as votes come in.",
            }
            {children}
        }
    }
}

/// The loaded list, or the empty card.
#[component]
fn PollList(
    polls: Vec<PollResultsData>,
    names: AuthorNames,
    is_member: bool,
    is_officer: bool,
    on_change: EventHandler<()>,
    on_removed: EventHandler<()>,
) -> Element {
    if polls.is_empty() {
        let message = if is_officer {
            "Create one above."
        } else {
            "Check back soon."
        };
        return rsx! {
            EmptyCard {
                title: "No open polls",
                message: message.to_string(),
            }
        };
    }
    rsx! {
        div { class: "polls-list",
            for results in polls {
                PollCard {
                    key: "{results.poll.id}",
                    author: author_label(Some(&names), &results.poll.created_by),
                    closes: close_date(results.poll.close_at.as_deref()),
                    results: results.clone(),
                    is_member,
                    is_officer,
                    on_change,
                    on_removed,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ToastProvider;
    use crate::components::poll::{PollData, PollOptionResultData};
    use crate::components::ui::test_support::assert_sizes_from_tokens;

    const AUTHOR: &str = "member:9d1c44a0e2";

    fn poll(n: usize) -> PollResultsData {
        let option = |index: u32, count: u32| PollOptionResultData {
            option_index: index,
            label: format!("Option {index}"),
            count,
        };
        PollResultsData {
            poll: PollData {
                id: format!("poll:{n}"),
                title: format!("Question {n}"),
                description: Some(format!("Context {n}")),
                options: vec!["Option 0".to_string(), "Option 1".to_string()],
                close_at: Some("2026-11-16T20:00:00Z".to_string()),
                allow_multiple: false,
                created_by: AUTHOR.to_string(),
                created_at: "2026-11-08T10:00:00Z".to_string(),
                is_active: true,
            },
            votes: vec![option(0, 3), option(1, 1)],
            total_votes: 4,
            my_votes: Vec::new(),
        }
    }

    #[derive(Props, Clone, PartialEq)]
    struct ProbeProps {
        polls: Vec<PollResultsData>,
        is_officer: bool,
    }

    #[allow(non_snake_case)]
    fn Probe(props: ProbeProps) -> Element {
        let names = AuthorNames::from([(AUTHOR.to_string(), Some("Player06".to_string()))]);
        rsx! {
            ToastProvider {
                PollsLayout {
                    if props.is_officer {
                        PollCreate { on_created: move |_| {} }
                    }
                    section {
                        id: LIST_ID,
                        class: "ui-stack pager-target",
                        tabindex: "-1",
                        aria_label: "Open polls",
                        PollList {
                            polls: props.polls,
                            names,
                            is_member: true,
                            is_officer: props.is_officer,
                            on_change: move |_| {},
                            on_removed: move |_| {},
                        }
                    }
                }
            }
        }
    }

    fn render(polls: Vec<PollResultsData>, is_officer: bool) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, ProbeProps { polls, is_officer });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn page_has_one_h1_no_main_and_a_card_per_poll() {
        let html = render(vec![poll(1), poll(2)], false);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Polls</h1>"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        assert!(html.contains("class=\"ui-shell polls-page\""), "{html}");
        assert_eq!(html.matches("<article class=\"poll-card\"").count(), 2);
        for n in 1..=2 {
            assert_eq!(
                html.matches(&format!("id=\"poll-poll:{n}-title\"")).count(),
                1,
                "{html}"
            );
            assert!(
                html.contains(&format!("<p class=\"poll-card-desc\">Context {n}</p>")),
                "{html}"
            );
        }
        // The poster is a display name, never the member id.
        assert_eq!(html.matches("Posted by Player06").count(), 2, "{html}");
        assert!(!html.contains(AUTHOR), "{html}");
        assert!(
            html.contains("closes <time datetime=\"2026-11-16\">16 Nov 2026</time>"),
            "{html}"
        );
        assert!(html.contains("3 votes, 75%"), "{html}");
        assert!(html.contains("1 vote, 25%"), "{html}");
        // A member is not offered the officer actions.
        assert!(!html.contains("Create a poll"), "{html}");
        assert!(!html.contains("Remove poll"), "{html}");
        assert!(!html.contains("<a "), "{html}");
    }

    #[test]
    fn empty_list_says_so() {
        let member = render(Vec::new(), false);
        assert!(member.contains("<h2>No open polls</h2>"), "{member}");
        assert!(member.contains("<p>Check back soon.</p>"), "{member}");
        assert_eq!(member.matches("<h1").count(), 1, "{member}");
        assert!(
            member.contains(
                "<section id=\"polls-list\" class=\"ui-stack pager-target\" tabindex=\"-1\" aria-label=\"Open polls\">"
            ),
            "{member}"
        );

        let officer = render(Vec::new(), true);
        assert!(officer.contains("<p>Create one above.</p>"), "{officer}");
        assert!(officer.contains(">Create a poll</h2>"), "{officer}");
        assert_eq!(officer.matches("<h1").count(), 1, "{officer}");
    }

    #[test]
    fn officer_gets_the_form_and_a_named_remove_per_poll() {
        let html = render(vec![poll(1), poll(2)], true);
        assert!(html.contains(">Create a poll</h2>"), "{html}");
        for n in 1..=2 {
            assert!(
                html.contains(&format!("aria-label=\"Remove poll: Question {n}\"")),
                "{html}"
            );
        }
    }

    /// The page in the real router and shell: one H1 and only the shell's
    /// `<main>` for a guest (wall), a member (loading) and an officer (form).
    #[test]
    fn routed_page_has_one_h1_and_one_main_in_every_gate_state() {
        use crate::components::member_gate::test_route::render_route;
        use scuffed_types::OrgRole;
        for role in [None, Some(OrgRole::Member), Some(OrgRole::Officer)] {
            let html = render_route("/polls", role);
            assert_eq!(html.matches("<h1").count(), 1, "{role:?}: {html}");
            assert!(html.contains(">Polls</h1>"), "{role:?}: {html}");
            assert_eq!(html.matches("<main").count(), 1, "{role:?}: {html}");
            // Not loaded: the list section has no name of its own.
            assert!(
                html.contains(
                    "<section id=\"polls-list\" class=\"ui-stack pager-target\" tabindex=\"-1\">"
                ),
                "{role:?}: {html}"
            );
        }
        let officer = render_route("/polls", Some(OrgRole::Officer));
        assert!(officer.contains(">Create a poll</h2>"), "{officer}");
    }

    #[test]
    fn close_date_is_day_month_year() {
        assert_eq!(
            close_date(Some("2026-11-16T20:00:00Z")),
            Some(PollCloseDate {
                datetime: "2026-11-16".to_string(),
                text: "16 Nov 2026".to_string(),
            })
        );
        assert_eq!(close_date(None), None);
        assert_eq!(
            close_date(Some("soon")).map(|d| d.text),
            Some("soon".to_string())
        );
    }

    #[test]
    fn css_is_on_tokens() {
        // The page sheet has no type of its own; the card sheets carry it.
        for decl in ["gap: var(--space-stack)", "min-width: 0"] {
            assert!(PAGE_CSS.contains(decl), "{PAGE_CSS}");
        }
        assert_sizes_from_tokens(crate::components::poll::card::CARD_CSS);
        assert_sizes_from_tokens(crate::components::poll::create::CREATE_CSS);
    }
}
