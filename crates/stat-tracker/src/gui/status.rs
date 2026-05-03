use dioxus::prelude::*;

use stat_tracker::config::Config;
use stat_tracker::storage::LocalStore;

use super::daemon::DaemonCard;

#[component]
pub fn StatusPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut refresh_tick = use_signal(|| 0u32);

    let stats = use_resource(move || {
        let data_dir = config().data_dir.clone();
        let _tick = refresh_tick();
        async move { load_stats(&data_dir).await }
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

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            refresh_tick += 1;
        }
    });

    let (count, unsynced, last_capture) = match &*stats.read() {
        Some(s) => (s.total_matches, s.unsynced_count, s.last_capture_time.clone()),
        None => (0, 0, None),
    };

    let sync_configured = config().sync.is_some();

    rsx! {
        div { class: "panel",
            h2 { "Dashboard" }

            DaemonCard {}

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
                    span { class: "label", "Last capture" }
                    span { class: "value",
                        if let Some(ref t) = last_capture {
                            "{t}"
                        } else {
                            "—"
                        }
                    }
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

            div { class: "card",
                h3 { "Sync" }
                if sync_configured {
                    div { class: "stat-row",
                        span { class: "label", "Server" }
                        span { class: "value",
                            "{config().sync.as_ref().map(|s| s.server_url.as_str()).unwrap_or(\"-\")}"
                        }
                    }
                    div { class: "stat-row",
                        span { class: "label", "Unsynced matches" }
                        span { class: "value",
                            if unsynced > 0 {
                                span { class: "status-dot warn" }
                                "{unsynced} pending"
                            } else {
                                span { class: "status-dot ok" }
                                "all synced"
                            }
                        }
                    }
                } else {
                    div { class: "stat-row",
                        span { class: "label", "Status" }
                        span { class: "value text-dim", "not configured" }
                    }
                }
            }
        }
    }
}

struct DashboardStats {
    total_matches: usize,
    unsynced_count: usize,
    last_capture_time: Option<String>,
}

async fn load_stats(data_dir: &std::path::Path) -> DashboardStats {
    match LocalStore::open(data_dir).await {
        Ok(store) => {
            let total = store.match_count().await.unwrap_or(0);
            let unsynced = store
                .get_unsynced()
                .await
                .map(|v| v.len())
                .unwrap_or(0);
            let last = store.last_capture_time().await;
            DashboardStats {
                total_matches: total,
                unsynced_count: unsynced,
                last_capture_time: last,
            }
        }
        Err(_) => DashboardStats {
            total_matches: 0,
            unsynced_count: 0,
            last_capture_time: None,
        },
    }
}
