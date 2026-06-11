use dioxus::prelude::*;

use crate::components::poll::{PollCard, PollCreate, PollResultsData};
use crate::hooks::use_api;
use crate::state::use_auth;

const PAGE_CSS: &str = r#"
.polls-page {
    padding: 3rem 2rem;
    max-width: 700px;
    margin: 0 auto;
}
.polls-page-title {
    font-family: var(--font-head);
    font-size: 2.5rem;
    color: var(--text);
    letter-spacing: 3px;
    margin: 0 0 0.25rem;
}
.polls-subtitle {
    color: var(--text-2);
    font-size: 0.9rem;
    margin: 0 0 2rem;
}
.polls-list {
    display: flex;
    flex-direction: column;
    gap: 1rem;
}
.polls-loading, .polls-empty {
    color: var(--text-3);
    text-align: center;
    padding: 3rem 0;
}
@media (max-width: 768px) {
    .polls-page { padding: 2rem 1rem; }
}
"#;

#[component]
pub fn Polls() -> Element {
    let auth = use_auth();
    let mut polls = use_api::<Vec<PollResultsData>>("/api/polls");

    let is_logged_in = auth().is_logged_in();
    let is_officer = auth().is_officer_or_above();

    let on_change = move |_| {
        polls.refresh += 1;
    };

    rsx! {
        style { {PAGE_CSS} }

        main { class: "polls-page",
            h1 { class: "polls-page-title", "Polls" }
            p { class: "polls-subtitle", "Vote on community decisions and see what the crew thinks." }

            if is_officer {
                PollCreate { on_created: on_change }
            }

            {
                let data = polls.data.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "polls-loading", "Loading polls..." } },
                    Some(poll_list) => {
                        if poll_list.is_empty() {
                            rsx! {
                                p { class: "polls-empty", "No active polls right now." }
                            }
                        } else {
                            rsx! {
                                div { class: "polls-list",
                                    for poll_results in poll_list.iter() {
                                        PollCard {
                                            key: "{poll_results.poll.id}",
                                            results: poll_results.clone(),
                                            is_member: is_logged_in,
                                            is_officer: is_officer,
                                            on_change: on_change,
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
