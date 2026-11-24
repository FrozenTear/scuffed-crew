use dioxus::prelude::*;
use scuffed_api_client::ApiClient;

use crate::components::ui::{Pill, PillTone};
use crate::components::{Toast, use_toast};

use super::{
    PollCloseDate, PollResultsData, is_closed, logged_poll_error, now_iso, percent, votes_text,
};

pub(crate) const CARD_CSS: &str = r#"
.poll-card {
  box-sizing: border-box;
  min-width: 0;
  padding: var(--space-section);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.poll-card-title {
  margin: 0;
  font-family: var(--font-head);
  font-weight: var(--font-head-weight);
  letter-spacing: var(--font-head-tracking);
  font-size: var(--text-xl);
  line-height: var(--leading-display);
  color: var(--text);
  overflow-wrap: anywhere;
}
.poll-card-title .ui-pill {
  margin-inline-start: var(--space-2);
  vertical-align: middle;
}
.poll-card-desc {
  margin: var(--space-2) 0 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
  overflow-wrap: anywhere;
}
.poll-card-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 0 var(--space-2);
  margin: var(--space-4) 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
/* The dot trails each item, so a wrapped line never starts with one. */
.poll-card-meta > :not(:last-child)::after {
  content: "·" / "";
  margin-inline-start: var(--space-2);
}
.poll-options {
  list-style: none;
  margin: 0;
  padding: 0;
}
.poll-option {
  display: grid;
  gap: var(--space-2);
  margin: 0 0 var(--space-3);
}
.poll-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-3);
  color: var(--text);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
.poll-option-label {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1) var(--space-2);
  min-width: 0;
  overflow-wrap: anywhere;
}
.poll-option-stats {
  flex: 0 0 auto;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
.poll-track {
  height: var(--space-4);
  background: var(--surface-sunken);
  border-radius: var(--radius-pill);
  overflow: hidden;
}
.poll-fill {
  height: 100%;
  background: var(--accent-solid);
}
.poll-status {
  margin: 0 0 var(--space-3);
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
.poll-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}
.poll-actions .btn { overflow-wrap: anywhere; }
"#;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
struct VoteBody {
    option_index: u32,
}

/// One poll: the question, its counts as text plus a bar, and the actions.
///
/// A member gets one button per option, in the poll's own option order (the
/// results above are sorted by count and move as votes come in; the buttons
/// stay put, so focus stays on the one that was pressed). A closed poll
/// shows results only.
#[component]
pub fn PollCard(
    results: PollResultsData,
    /// Display name of the member who posted it, once the page knows it.
    #[props(default)]
    author: Option<String>,
    /// The close date, formatted by the page.
    #[props(default)]
    closes: Option<PollCloseDate>,
    is_member: bool,
    is_officer: bool,
    on_change: EventHandler<()>,
    /// The poll was removed. The page says so and reloads: this card is
    /// about to unmount, so it cannot.
    on_removed: EventHandler<()>,
) -> Element {
    let toasts = use_toast();
    // A vote or a removal is being saved.
    let mut busy = use_signal(|| false);
    // What the last vote did. Spoken through the status line below.
    let mut status = use_signal(String::new);

    let poll = &results.poll;
    let poll_id = poll.id.clone();
    let total = results.total_votes;
    let closed = is_closed(poll, now_iso().as_deref());
    let can_vote = is_member && !closed;
    let title_id = format!("poll-{poll_id}-title");

    let mut ballot: Vec<(u32, String, bool)> = results
        .votes
        .iter()
        .map(|opt| {
            (
                opt.option_index,
                opt.label.clone(),
                results.my_votes.contains(&opt.option_index),
            )
        })
        .collect();
    ballot.sort_by_key(|(index, _, _)| *index);

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-poll-card-card-css", css: CARD_CSS }

        article { class: "poll-card", aria_labelledby: "{title_id}",
            h2 { class: "poll-card-title", id: "{title_id}",
                span { "{poll.title}" }
                if closed {
                    Pill { "Closed" }
                }
                if poll.allow_multiple {
                    Pill { "Multiple choice" }
                }
            }

            if let Some(ref desc) = poll.description {
                p { class: "poll-card-desc", "{desc}" }
            }

            p { class: "poll-card-meta",
                if let Some(author) = author {
                    span { "Posted by {author}" }
                }
                span { "{votes_text(total)}" }
                if let Some(closes) = closes {
                    span {
                        if closed { "closed " } else { "closes " }
                        time { datetime: "{closes.datetime}", "{closes.text}" }
                    }
                }
            }

            ul { class: "poll-options",
                for opt in results.votes.iter() {
                    {
                        let pct = percent(opt.count, total);
                        let voted = results.my_votes.contains(&opt.option_index);
                        rsx! {
                            li { key: "{opt.option_index}", class: "poll-option",
                                div { class: "poll-head",
                                    span { class: "poll-option-label",
                                        span { "{opt.label}" }
                                        // A word, so the viewer's pick does
                                        // not rest on colour.
                                        if voted {
                                            Pill { tone: PillTone::Accent, "Your vote" }
                                        }
                                    }
                                    span { class: "poll-option-stats", "{votes_text(opt.count)}, {pct}%" }
                                }
                                // The count and percent are in the text above.
                                div { class: "poll-track", aria_hidden: "true",
                                    div { class: "poll-fill", style: "width: {pct}%" }
                                }
                            }
                        }
                    }
                }
            }

            // Always in the tree so the next text is announced. With nothing
            // to say it takes the shared visually hidden class.
            p {
                class: if status().is_empty() { "poll-status visually-hidden" } else { "poll-status" },
                role: "status",
                "{status}"
            }

            if can_vote || is_officer {
                div { class: "poll-actions",
                    if can_vote {
                        for (index, label, voted) in ballot {
                            {
                                let poll_id = poll_id.clone();
                                let name = if voted {
                                    format!("Remove vote for {label}")
                                } else {
                                    format!("Vote {label}")
                                };
                                rsx! {
                                    // Busy keeps the button focusable and
                                    // ignores the click.
                                    button {
                                        key: "{index}",
                                        r#type: "button",
                                        class: if voted { "btn is-ghost" } else { "btn is-primary" },
                                        aria_disabled: if busy() { "true" },
                                        onclick: move |_| {
                                            if busy() {
                                                return;
                                            }
                                            busy.set(true);
                                            let poll_id = poll_id.clone();
                                            let label = label.clone();
                                            let mut toasts = toasts;
                                            spawn(async move {
                                                let client = ApiClient::web();
                                                let res = if voted {
                                                    client.delete(&format!("/api/polls/{poll_id}/vote/{index}")).await
                                                } else {
                                                    client.post_json::<_, serde_json::Value>(
                                                        &format!("/api/polls/{poll_id}/vote"),
                                                        &VoteBody { option_index: index },
                                                    ).await.map(|_| ())
                                                };
                                                match res {
                                                    Ok(()) => {
                                                        status.set(if voted {
                                                            format!("Vote removed from {label}.")
                                                        } else {
                                                            format!("Vote counted for {label}.")
                                                        });
                                                        on_change.call(());
                                                    }
                                                    Err(e) => {
                                                        status.set(String::new());
                                                        let lead = if voted { "Couldn't remove the vote." } else { "Vote failed." };
                                                        toasts.show(Toast::error(logged_poll_error(lead, &e)));
                                                    }
                                                }
                                                busy.set(false);
                                            });
                                        },
                                        "{name}"
                                    }
                                }
                            }
                        }
                    }

                    if is_officer {
                        {
                            let poll_id = poll_id.clone();
                            rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-ghost",
                                    aria_label: "Remove poll: {poll.title}",
                                    aria_disabled: if busy() { "true" },
                                    onclick: move |_| {
                                        if busy() {
                                            return;
                                        }
                                        busy.set(true);
                                        let poll_id = poll_id.clone();
                                        let mut toasts = toasts;
                                        spawn(async move {
                                            match ApiClient::web().delete(&format!("/api/polls/{poll_id}")).await {
                                                Ok(()) => on_removed.call(()),
                                                Err(e) => {
                                                    toasts.show(Toast::error(logged_poll_error("Couldn't remove the poll.", &e)));
                                                }
                                            }
                                            busy.set(false);
                                        });
                                    },
                                    "Remove poll"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{PollData, PollOptionResultData};
    use super::*;
    use crate::components::ToastProvider;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

    /// The poll in the frame, as the server sends it: results sorted by
    /// count, the viewer's pick on the middle option.
    fn results(is_active: bool, my_votes: Vec<u32>) -> PollResultsData {
        let labels = ["19:00", "20:00", "21:00"];
        let option = |index: u32, count: u32| PollOptionResultData {
            option_index: index,
            label: labels[index as usize].to_string(),
            count,
        };
        PollResultsData {
            poll: PollData {
                id: "poll:7".to_string(),
                title: "What time should we run Friday customs?".to_string(),
                description: None,
                options: labels.iter().map(|l| l.to_string()).collect(),
                close_at: Some("2026-11-16T20:00:00Z".to_string()),
                allow_multiple: false,
                created_by: "member:9d1c44a0e2".to_string(),
                created_at: "2026-11-08T10:00:00Z".to_string(),
                is_active,
            },
            votes: vec![option(1, 12), option(0, 8), option(2, 3)],
            total_votes: 23,
            my_votes,
        }
    }

    #[derive(Props, Clone, PartialEq)]
    struct ProbeProps {
        results: PollResultsData,
        is_member: bool,
        is_officer: bool,
    }

    #[allow(non_snake_case)]
    fn Probe(props: ProbeProps) -> Element {
        rsx! {
            ToastProvider {
                PollCard {
                    results: props.results,
                    author: "Player06".to_string(),
                    closes: PollCloseDate {
                        datetime: "2026-11-16".to_string(),
                        text: "16 Nov 2026".to_string(),
                    },
                    is_member: props.is_member,
                    is_officer: props.is_officer,
                    on_change: move |_| {},
                    on_removed: move |_| {},
                }
            }
        }
    }

    fn render(results: PollResultsData, is_member: bool, is_officer: bool) -> String {
        let props = ProbeProps {
            results,
            is_member,
            is_officer,
        };
        let mut dom = VirtualDom::new_with_props(Probe, props);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn option_has_label_count_percent_and_a_hidden_bar() {
        let html = render(results(true, Vec::new()), true, false);
        assert!(
            html.contains("<article class=\"poll-card\" aria-labelledby=\"poll-poll:7-title\">"),
            "{html}"
        );
        assert!(
            html.contains(
                "<h2 class=\"poll-card-title\" id=\"poll-poll:7-title\"><span>What time should we run Friday customs?</span></h2>"
            ),
            "{html}"
        );
        assert!(html.contains("<span>Posted by Player06</span>"), "{html}");
        assert!(html.contains("<span>23 votes</span>"), "{html}");
        assert!(
            html.contains("closes <time datetime=\"2026-11-16\">16 Nov 2026</time>"),
            "{html}"
        );
        for (label, stats, width) in [
            ("19:00", "8 votes, 35%", "35%"),
            ("20:00", "12 votes, 52%", "52%"),
            ("21:00", "3 votes, 13%", "13%"),
        ] {
            assert!(
                html.contains(&format!(
                    "<span class=\"poll-option-label\"><span>{label}</span></span><span class=\"poll-option-stats\">{stats}</span>"
                )),
                "{label}: {html}"
            );
            assert!(
                html.contains(&format!(
                    "<div class=\"poll-track\" aria-hidden=\"true\"><div class=\"poll-fill\" style=\"width: {width}\"></div></div>"
                )),
                "{label}: {html}"
            );
        }
        assert!(!html.contains("<h1"), "{html}");
        assert!(!html.contains("<main"), "{html}");
    }

    #[test]
    fn member_gets_a_named_vote_button_per_option_in_option_order() {
        let html = render(results(true, Vec::new()), true, false);
        let buttons: Vec<usize> = ["Vote 19:00", "Vote 20:00", "Vote 21:00"]
            .iter()
            .map(|name| {
                html.find(&format!(
                    "<button type=\"button\" class=\"btn is-primary\">{name}</button>"
                ))
                .unwrap_or_else(|| panic!("missing {name}: {html}"))
            })
            .collect();
        assert!(buttons.windows(2).all(|pair| pair[0] < pair[1]), "{html}");
        // The results keep the server order: most votes first.
        assert!(
            html.find("<span>20:00</span>").unwrap() < html.find("<span>19:00</span>").unwrap(),
            "{html}"
        );
        assert!(!html.contains("Remove poll"), "{html}");
        assert!(!html.contains("aria-disabled"), "{html}");
        // The status line is in the tree before there is anything to say.
        assert!(
            html.contains("<p class=\"poll-status visually-hidden\" role=\"status\"></p>"),
            "{html}"
        );
    }

    #[test]
    fn own_vote_is_a_word_and_its_button_removes_it() {
        let html = render(results(true, vec![1]), true, false);
        assert!(
            html.contains(
                "<span>20:00</span><span class=\"ui-pill ui-pill--accent\">Your vote</span>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("Your vote").count(), 1, "{html}");
        assert!(
            html.contains(
                "<button type=\"button\" class=\"btn is-ghost\">Remove vote for 20:00</button>"
            ),
            "{html}"
        );
        assert!(html.contains(">Vote 19:00</button>"), "{html}");
        assert!(!html.contains(">Vote 20:00</button>"), "{html}");
    }

    #[test]
    fn closed_poll_shows_results_only() {
        let html = render(results(false, vec![1]), true, false);
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--neutral\">Closed</span>"),
            "{html}"
        );
        assert!(
            html.contains("closed <time datetime=\"2026-11-16\">16 Nov 2026</time>"),
            "{html}"
        );
        assert!(html.contains("12 votes, 52%"), "{html}");
        assert!(!html.contains("<button"), "{html}");
        assert!(!html.contains("poll-actions"), "{html}");
    }

    #[test]
    fn non_member_cannot_vote_and_officer_can_remove() {
        let guest = render(results(true, Vec::new()), false, false);
        assert!(!guest.contains("<button"), "{guest}");

        let officer = render(results(true, Vec::new()), true, true);
        assert!(
            officer.contains(
                "<button type=\"button\" class=\"btn is-ghost\" aria-label=\"Remove poll: What time should we run Friday customs?\">Remove poll</button>"
            ),
            "{officer}"
        );
        assert_eq!(officer.matches("<button").count(), 4, "{officer}");
    }

    #[test]
    fn zero_votes_and_multiple_choice() {
        let mut data = results(true, Vec::new());
        data.total_votes = 0;
        for opt in &mut data.votes {
            opt.count = 0;
        }
        data.poll.allow_multiple = true;
        let html = render(data, true, false);
        assert_eq!(html.matches("0 votes, 0%").count(), 3, "{html}");
        assert_eq!(html.matches("style=\"width: 0%\"").count(), 3, "{html}");
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--neutral\">Multiple choice</span>"),
            "{html}"
        );
    }

    #[test]
    fn css_is_on_tokens() {
        assert_sizes_from_tokens(CARD_CSS);
        let dot = css_rule(CARD_CSS, ".poll-card-meta > :not(:last-child)::after {");
        assert!(
            has_decl(&dot, "content: \"·\" / \"\""),
            "the separator has empty alt text, so it is not read out: {dot}"
        );
        let card = css_rule(CARD_CSS, ".poll-card {");
        for decl in [
            "padding: var(--space-section)",
            "background: var(--card-bg)",
        ] {
            assert!(has_decl(&card, decl), "missing {decl}: {card}");
        }
        let track = css_rule(CARD_CSS, ".poll-track {");
        assert!(
            has_decl(&track, "background: var(--surface-sunken)"),
            "{track}"
        );
        assert!(
            has_decl(
                &css_rule(CARD_CSS, ".poll-fill {"),
                "background: var(--accent-solid)"
            ),
            "{CARD_CSS}"
        );
    }
}
