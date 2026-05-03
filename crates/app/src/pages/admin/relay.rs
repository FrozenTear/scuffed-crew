use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{SummaryCard, StatusPill};
use crate::hooks::use_api;

#[derive(Debug, Clone, Deserialize)]
struct RelayHealth {
    configured: bool,
    reachable: bool,
    relay_url: Option<String>,
    #[serde(default)]
    extra_relay_urls: Vec<String>,
    forum_backend: String,
}

#[component]
pub fn AdminRelay() -> Element {
    let mut health = use_api::<RelayHealth>("/api/nostr/health");

    let relay_count = {
        let data = health.data.read();
        let data = data.as_ref().and_then(|d| d.as_ref());
        match data {
            Some(h) => {
                let primary = if h.configured { 1 } else { 0 };
                primary + h.extra_relay_urls.len()
            }
            None => 0,
        }
    };

    rsx! {
        div { class: "admin-toolbar",
            h1 { "Relay Status" }
            button {
                class: "btn-add",
                onclick: move |_| health.refresh += 1,
                "Refresh"
            }
        }

        {
            let data = health.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(h) => rsx! {
                    div { class: "summary-cards",
                        SummaryCard {
                            value: (if h.configured { "Yes" } else { "No" }).to_string(),
                            label: "Relay Configured",
                        }
                        SummaryCard {
                            value: (if h.reachable { "Online" } else { "Offline" }).to_string(),
                            label: "Relay Status",
                        }
                        SummaryCard {
                            value: relay_count.to_string(),
                            label: "Total Relays",
                        }
                        SummaryCard {
                            value: h.forum_backend.clone(),
                            label: "Forum Backend",
                        }
                    }

                    div { class: "form-section",
                        h2 { "Primary Relay" }
                        table { class: "data-table",
                            tbody {
                                tr {
                                    td { style: "font-weight: 600; width: 180px;", "URL" }
                                    td {
                                        code {
                                            {h.relay_url.clone().unwrap_or_else(|| "Not configured".into())}
                                        }
                                    }
                                }
                                tr {
                                    td { style: "font-weight: 600;", "Status" }
                                    td {
                                        StatusPill {
                                            status: (if h.reachable { "active" } else { "inactive" }).to_string(),
                                        }
                                    }
                                }
                                tr {
                                    td { style: "font-weight: 600;", "Forum Mode" }
                                    td {
                                        StatusPill {
                                            status: h.forum_backend.clone(),
                                        }
                                    }
                                }
                            }
                        }

                        if !h.configured {
                            p {
                                style: "margin-top: 1rem; color: var(--text-muted); font-size: 0.85rem;",
                                "Set the NOSTR_RELAY_URL environment variable to connect to a relay."
                            }
                        }
                    }

                    if !h.extra_relay_urls.is_empty() {
                        div { class: "form-section",
                            h2 { "Extra Relays" }
                            table { class: "data-table",
                                thead {
                                    tr {
                                        th { "URL" }
                                    }
                                }
                                tbody {
                                    for url in h.extra_relay_urls.iter() {
                                        tr { key: "{url}",
                                            td { code { "{url}" } }
                                        }
                                    }
                                }
                            }
                            p {
                                style: "margin-top: 0.5rem; color: var(--text-muted); font-size: 0.85rem;",
                                "Configure extra relays in Settings > Forum > Extra Relay URLs."
                            }
                        }
                    }
                },
            }
        }
    }
}
