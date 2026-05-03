use dioxus::prelude::*;

use crate::components::poll::{POLL_CSS, PollCard, PollCreate, PollSummary};
use crate::hooks::use_api;
use crate::state::auth::use_auth;

const PAGE_CSS: &str = r#"
    .polls-page {
        max-width: 900px;
        margin: 0 auto;
        padding: 3rem 2rem;
    }
    .polls-header {
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
        margin-bottom: 1.25rem;
    }
    .polls-title {
        margin: 0;
        font-family: 'Bebas Neue', sans-serif;
        letter-spacing: 0.08em;
        font-size: clamp(2rem, 4vw, 2.8rem);
        color: var(--text-bright);
    }
    .polls-subtitle {
        margin: 0;
        color: var(--text-secondary);
        font-size: 0.9rem;
        max-width: 680px;
        line-height: 1.6;
    }
    .polls-list {
        display: flex;
        flex-direction: column;
        gap: 0.9rem;
    }
    .polls-empty {
        color: var(--text-muted);
        text-align: center;
        padding: 2.2rem 1rem;
        border: 1px dashed var(--border);
        border-radius: 12px;
        background: var(--bg-card);
    }
"#;

#[component]
pub fn Polls() -> Element {
    let mut polls = use_api::<Vec<PollSummary>>("/api/polls");
    let auth = use_auth();

    let can_create = auth().is_officer_or_above();
    let can_vote = auth().user.as_ref().and_then(|user| user.role).is_some();

    rsx! {
        style { {POLL_CSS} }
        style { {PAGE_CSS} }

        main { class: "polls-page",
            header { class: "polls-header",
                h1 { class: "polls-title", "Crew Polls" }
                p {
                    class: "polls-subtitle",
                    "Vote on scheduling, formats, and team decisions. Officers can create polls; members can vote in active polls."
                }
            }

            if can_create {
                PollCreate {
                    on_created: move |_| {
                        polls.refresh += 1;
                    }
                }
            }

            {
                let data = polls.data.read();
                let list = data.as_ref().and_then(|value| value.as_ref());
                match list {
                    None => rsx! { p { class: "polls-empty", "Loading polls..." } },
                    Some(items) if items.is_empty() => rsx! {
                        p { class: "polls-empty", "No active polls right now." }
                    },
                    Some(items) => rsx! {
                        section { class: "polls-list",
                            for poll in items.iter() {
                                { let poll_item = poll.clone();
                                  rsx! {
                                      PollCard {
                                          key: "{poll_item.id}",
                                          poll: poll_item,
                                          can_vote,
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
