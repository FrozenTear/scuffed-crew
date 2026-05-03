use chrono::Utc;
use dioxus::prelude::*;
use serde::Serialize;

use scuffed_api_client::ApiClient;

use crate::components::{Toast, use_toast};

use super::{PollDetailResponse, PollSummary};

#[derive(Debug, Clone, Serialize)]
struct VotePollRequest {
    option_index: u32,
}

#[component]
pub fn PollCard(poll: PollSummary, can_vote: bool) -> Element {
    let mut toast = use_toast();
    let poll_id = poll.id.clone();
    let refresh = use_signal(|| 0u64);

    let detail = use_resource({
        let poll_id = poll_id.clone();
        move || {
            let _ = refresh();
            let path = format!("/api/polls/{poll_id}");
            async move {
                ApiClient::web()
                    .fetch::<PollDetailResponse>(&path)
                    .await
                    .ok()
            }
        }
    });

    let detail_data = detail
        .read()
        .as_ref()
        .and_then(|value| value.as_ref())
        .cloned();

    let active_poll = detail_data
        .as_ref()
        .map(|d| d.poll.clone())
        .unwrap_or_else(|| poll.clone());
    let results = detail_data.as_ref().map(|d| d.results.clone());
    let viewer_votes = detail_data
        .as_ref()
        .map(|d| d.viewer_votes.clone())
        .unwrap_or_default();

    let is_closed = active_poll
        .close_at
        .is_some_and(|close_at| close_at <= Utc::now());

    let close_label = active_poll
        .close_at
        .map(|close_at| close_at.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "No close date".to_string());

    let created_label = active_poll.created_at.format("%Y-%m-%d").to_string();

    let total_votes = results.as_ref().map(|r| r.total_votes).unwrap_or(0);

    rsx! {
        article { class: "poll-card",
            div { class: "poll-head",
                div {
                    h3 { class: "poll-title", "{active_poll.title}" }
                    if let Some(desc) = &active_poll.description {
                        p { class: "poll-desc", "{desc}" }
                    }
                }
                span {
                    class: if is_closed { "poll-badge closed" } else { "poll-badge" },
                    if is_closed { "Closed" } else { "Open" }
                }
            }

            div { class: "poll-meta",
                span { "Created {created_label}" }
                span { "•" }
                span { "Closes {close_label}" }
                if active_poll.allow_multiple {
                    span { "• Multi-select" }
                }
            }

            div { class: "poll-options",
                for (idx, option) in active_poll.options.iter().enumerate() {
                    {
                        let selected = viewer_votes.contains(&(idx as u32));
                        let (vote_count, percentage) = results
                            .as_ref()
                            .and_then(|r| r.options.iter().find(|o| o.option_index == idx as u32))
                            .map(|o| (o.vote_count, o.percentage))
                            .unwrap_or((0, 0.0));

                        let style = format!("width: {:.2}%;", percentage.clamp(0.0, 100.0));
                        let option_class = if selected {
                            "poll-option selected"
                        } else {
                            "poll-option"
                        };

                        let option_text = option.clone();
                        let poll_id = poll_id.clone();
                        let mut refresh = refresh;

                        rsx! {
                            div { key: "{active_poll.id}-{idx}", class: "{option_class}",
                                div { class: "poll-option-fill", style: "{style}" }
                                button {
                                    class: "poll-option-btn",
                                    disabled: !can_vote || is_closed,
                                    onclick: move |_| {
                                        if !can_vote {
                                            toast.show(Toast::error("Only signed-in members can vote."));
                                            return;
                                        }
                                        if is_closed {
                                            toast.show(Toast::error("This poll is closed."));
                                            return;
                                        }

                                        let selected_now = selected;
                                        let poll_id = poll_id.clone();
                                        spawn(async move {
                                            let client = ApiClient::web();
                                            let result = if selected_now {
                                                client
                                                    .delete(&format!("/api/polls/{poll_id}/vote/{idx}"))
                                                    .await
                                                    .map(|_| ())
                                            } else {
                                                client
                                                    .post_json::<_, PollDetailResponse>(
                                                        &format!("/api/polls/{poll_id}/vote"),
                                                        &VotePollRequest {
                                                            option_index: idx as u32,
                                                        },
                                                    )
                                                    .await
                                                    .map(|_| ())
                                            };

                                            match result {
                                                Ok(_) => {
                                                    refresh += 1;
                                                }
                                                Err(err) => {
                                                    toast.show(Toast::error(format!("Vote failed: {err}")));
                                                }
                                            }
                                        });
                                    },
                                    span { class: "poll-option-label", "{option_text}" }
                                    span { class: "poll-option-stats",
                                        span { "{vote_count}" }
                                        span { "{percentage.round()}%" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "poll-foot",
                if !can_vote {
                    span { "Sign in as a member to vote." }
                } else if is_closed {
                    span { "Voting closed." }
                } else {
                    span { "Tap an option to vote or unvote." }
                }
                span { class: "poll-total", "{total_votes} votes" }
            }
        }
    }
}
