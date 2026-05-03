use dioxus::prelude::*;

use stat_tracker::config::Config;
use stat_tracker::storage::LocalStore;

#[component]
pub fn StatusPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let match_count = use_resource(move || {
        let data_dir = config().data_dir.clone();
        async move {
            match LocalStore::open(&data_dir).await {
                Ok(store) => store.match_count().await.unwrap_or(0),
                Err(_) => 0,
            }
        }
    });

    let outputs = use_signal(|| {
        stat_tracker::capture::wayshot::list_outputs().unwrap_or_default()
    });

    let selected_output = config().capture_output.clone().unwrap_or_else(|| {
        outputs().first().cloned().unwrap_or_else(|| "unknown".into())
    });

    let tessdata_installed = use_memo(|| {
        stat_tracker::setup::tessdata_dir()
            .join("koverwatch.traineddata")
            .exists()
    });

    let count = match &*match_count.read() {
        Some(c) => *c,
        None => 0,
    };

    rsx! {
        div { class: "panel",
            h2 { "Dashboard" }

            div { class: "card",
                h3 { "Capture" }
                div { class: "stat-row",
                    span { class: "label", "Active output" }
                    span { class: "value", "{selected_output}" }
                }
                div { class: "stat-row",
                    span { class: "label", "Available outputs" }
                    span { class: "value", "{outputs().len()}" }
                }
                div { class: "stat-row",
                    span { class: "label", "Backend" }
                    span { class: "value", "libwayshot (Wayland)" }
                }
            }

            div { class: "card",
                h3 { "OCR" }
                div { class: "stat-row",
                    span { class: "label", "Koverwatch tessdata" }
                    span { class: "value",
                        span {
                            class: if tessdata_installed() { "status-dot ok" } else { "status-dot err" },
                        }
                        if tessdata_installed() { "installed" } else { "missing" }
                    }
                }
            }

            div { class: "card",
                h3 { "Storage" }
                div { class: "stat-row",
                    span { class: "label", "Stored matches" }
                    span { class: "value", "{count}" }
                }
                div { class: "stat-row",
                    span { class: "label", "Database" }
                    span { class: "value", "{config().data_dir.display()}" }
                }
            }

            if config().sync.is_some() {
                div { class: "card",
                    h3 { "Sync" }
                    div { class: "stat-row",
                        span { class: "label", "Server" }
                        span { class: "value",
                            "{config().sync.as_ref().map(|s| s.server_url.as_str()).unwrap_or(\"-\")}"
                        }
                    }
                    div { class: "stat-row",
                        span { class: "label", "Status" }
                        span { class: "value",
                            span { class: "status-dot ok" }
                            "configured"
                        }
                    }
                }
            }
        }
    }
}
