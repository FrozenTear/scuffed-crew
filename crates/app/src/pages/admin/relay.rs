use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{AccessDenied, StatusPill, SummaryCard, admin_pending};
use crate::hooks::use_api;
use crate::state::use_auth;

#[derive(Debug, Clone, Deserialize)]
struct RelayHealth {
    configured: bool,
    reachable: bool,
    relay_url: Option<String>,
    #[serde(default)]
    extra_relay_urls: Vec<String>,
    forum_backend: String,
}

fn forum_is_local(backend: &str) -> bool {
    backend.eq_ignore_ascii_case("local") || backend.is_empty()
}

#[component]
pub fn AdminRelay() -> Element {
    let auth = use_auth();
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

    if !auth().is_admin() {
        return rsx! {
            AccessDenied { message: "You need admin permissions to view relay status.".to_string() }
        };
    }

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
                None => admin_pending(&health, "relay status"),
                Some(h) => {
                    let local_forum = forum_is_local(&h.forum_backend);
                    // F-AUI-003: empty URL + "configured" was lying; show clear labels.
                    let configured_label = if h.configured { "Yes" } else { "No" };
                    let status_label = if !h.configured {
                        if local_forum {
                            "Not needed"
                        } else {
                            "Not set"
                        }
                    } else if h.reachable {
                        "Online"
                    } else {
                        "Offline"
                    };
                    let status_pill = if !h.configured {
                        "inactive"
                    } else if h.reachable {
                        "active"
                    } else {
                        "inactive"
                    };
                    let url_display = h
                        .relay_url
                        .clone()
                        .filter(|u| !u.trim().is_empty())
                        .unwrap_or_else(|| {
                            if local_forum {
                                "Not set (optional for local forum)".into()
                            } else {
                                "Not configured".into()
                            }
                        });
                    let unconfigured_help = if local_forum {
                        "Forum runs locally — a Nostr relay is optional. Set NOSTR_RELAY_URL only when you need relay-backed features (profile publish, DMs, Nostr forum)."
                    } else {
                        "Forum is set to Nostr but no primary relay URL is configured. Set the NOSTR_RELAY_URL environment variable (non-empty ws:// or wss:// URL)."
                    };
                    let offline_help = if local_forum {
                        "Primary relay is configured but not reachable. Local forum still works; fix the URL or network if you need relay features."
                    } else {
                        "Primary relay is configured but not reachable. Check the URL, firewall, and that the relay process is running."
                    };

                    rsx! {
                        div { class: "summary-cards",
                            SummaryCard {
                                value: configured_label.to_string(),
                                label: "Relay Configured",
                            }
                            SummaryCard {
                                value: status_label.to_string(),
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
                                            code { "{url_display}" }
                                        }
                                    }
                                    tr {
                                        td { style: "font-weight: 600;", "Status" }
                                        td {
                                            StatusPill {
                                                status: status_pill.to_string(),
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
                                    style: "margin-top: 1rem; color: var(--text-3); font-size: 0.85rem;",
                                    "{unconfigured_help}"
                                }
                            } else if !h.reachable {
                                p {
                                    style: "margin-top: 1rem; color: var(--text-3); font-size: 0.85rem;",
                                    "{offline_help}"
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
                                    style: "margin-top: 0.5rem; color: var(--text-3); font-size: 0.85rem;",
                                    "Configure extra relays in Settings > Forum > Extra Relay URLs."
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}
