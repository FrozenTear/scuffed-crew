use dioxus::prelude::*;
use scuffed_api_client::ApiClient;

use crate::components::{Toast, use_toast};

use super::PollResultsData;

const CARD_CSS: &str = r#"
.poll-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.25rem 1.5rem;
    transition: border-color 0.15s;
}
.poll-card:hover {
    border-color: var(--accent-soft);
}
.poll-card-title {
    font-family: var(--font-display);
    font-size: 1.15rem;
    color: var(--text-bright);
    margin: 0 0 0.25rem;
}
.poll-card-desc {
    color: var(--text-secondary);
    font-size: 0.85rem;
    margin: 0 0 1rem;
}
.poll-options {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
}
.poll-option {
    position: relative;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.6rem 0.75rem;
    cursor: pointer;
    overflow: hidden;
    transition: border-color 0.15s;
}
.poll-option:hover {
    border-color: var(--accent-soft);
}
.poll-option.voted {
    border-color: #7c3aed;
}
.poll-option-bar {
    position: absolute;
    top: 0;
    left: 0;
    bottom: 0;
    background: #7c3aed18;
    border-radius: 6px;
    transition: width 0.3s ease;
}
.poll-option.voted .poll-option-bar {
    background: #7c3aed30;
}
.poll-option-content {
    position: relative;
    display: flex;
    justify-content: space-between;
    align-items: center;
    z-index: 1;
}
.poll-option-label {
    color: var(--text-bright);
    font-size: 0.9rem;
}
.poll-option-stats {
    color: var(--text-muted);
    font-size: 0.8rem;
    white-space: nowrap;
}
.poll-card-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 0.75rem;
    color: var(--text-muted);
    font-size: 0.75rem;
}
.poll-total-votes {
    color: var(--text-secondary);
}
.poll-badge-multiple {
    background: #7c3aed22;
    color: #a78bfa;
    padding: 0.15rem 0.5rem;
    border-radius: 999px;
    font-size: 0.65rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
}
.poll-delete-btn {
    background: none;
    border: 1px solid var(--border);
    color: var(--text-muted);
    border-radius: 4px;
    padding: 0.2rem 0.5rem;
    font-size: 0.7rem;
    cursor: pointer;
    transition: border-color 0.15s, color 0.15s;
}
.poll-delete-btn:hover {
    border-color: #ef4444;
    color: #ef4444;
}
"#;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
struct VoteBody {
    option_index: u32,
}

#[component]
pub fn PollCard(
    results: PollResultsData,
    is_member: bool,
    is_officer: bool,
    on_change: EventHandler<()>,
) -> Element {
    let toasts = use_toast();

    let poll_id = results.poll.id.clone();
    let total = results.total_votes;
    let my_votes = results.my_votes.clone();

    rsx! {
        style { {CARD_CSS} }

        article { class: "poll-card",
            h3 { class: "poll-card-title", "{results.poll.title}" }

            if let Some(ref desc) = results.poll.description {
                p { class: "poll-card-desc", "{desc}" }
            }

            div { class: "poll-options",
                for opt in results.votes.iter() {
                    {
                        let is_voted = my_votes.contains(&opt.option_index);
                        let pct = if total > 0 { (opt.count as f64 / total as f64 * 100.0) as u32 } else { 0 };
                        let width_pct = format!("{pct}%");
                        let class_str = if is_voted { "poll-option voted" } else { "poll-option" };
                        let poll_id = poll_id.clone();
                        let oidx = opt.option_index;
                        let on_change = on_change;

                        rsx! {
                            div {
                                class: "{class_str}",
                                onclick: move |_| {
                                    if !is_member { return; }
                                    let poll_id = poll_id.clone();
                                    let mut toasts = toasts;
                                    spawn(async move {
                                        let client = ApiClient::web();
                                        let res = if is_voted {
                                            client.delete(&format!("/api/polls/{poll_id}/vote/{oidx}")).await
                                        } else {
                                            client.post_json::<_, serde_json::Value>(
                                                &format!("/api/polls/{poll_id}/vote"),
                                                &VoteBody { option_index: oidx },
                                            ).await.map(|_| ())
                                        };
                                        match res {
                                            Ok(()) => {
                                                on_change.call(());
                                            }
                                            Err(e) => {
                                                toasts.show(Toast::error(format!("Vote failed: {e}")));
                                            }
                                        }
                                    });
                                },
                                div { class: "poll-option-bar", style: "width: {width_pct}" }
                                div { class: "poll-option-content",
                                    span { class: "poll-option-label",
                                        if is_voted { "✓ " } else { "" }
                                        "{opt.label}"
                                    }
                                    span { class: "poll-option-stats", "{opt.count} ({pct}%)" }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "poll-card-footer",
                span { class: "poll-total-votes",
                    {format!("{total} vote{}", if total != 1 { "s" } else { "" })}
                }

                if results.poll.allow_multiple {
                    span { class: "poll-badge-multiple", "Multiple choice" }
                }

                if is_officer {
                    {
                        let poll_id = poll_id.clone();
                        let on_change = on_change;
                        rsx! {
                            button {
                                class: "poll-delete-btn",
                                onclick: move |_| {
                                    let poll_id = poll_id.clone();
                                    let mut toasts = toasts;
                                    spawn(async move {
                                        match ApiClient::web().delete(&format!("/api/polls/{poll_id}")).await {
                                            Ok(()) => {
                                                toasts.show(Toast::success("Poll removed"));
                                                on_change.call(());
                                            }
                                            Err(e) => {
                                                toasts.show(Toast::error(format!("Delete failed: {e}")));
                                            }
                                        }
                                    });
                                },
                                "Remove"
                            }
                        }
                    }
                }
            }
        }
    }
}
