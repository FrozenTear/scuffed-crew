use dioxus::prelude::*;

use stat_tracker::config::Config;
use stat_tracker::storage::LocalStore;

use super::daemon::DaemonCard;

#[component]
pub fn StatusPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut refresh_tick = use_signal(|| 0u32);
    let mut db_locked = use_signal(|| false);

    let stats = use_resource(move || {
        let data_dir = config().data_dir.clone();
        let _tick = refresh_tick();
        async move {
            match LocalStore::open(&data_dir).await {
                Ok(store) => {
                    db_locked.set(false);
                    load_stats(&store).await
                }
                Err(_) => {
                    db_locked.set(true);
                    None
                }
            }
        }
    });

    let outputs = use_signal(|| stat_tracker::capture::wayshot::list_outputs().unwrap_or_default());

    let selected_output = config().capture_output.clone().unwrap_or_else(|| {
        outputs()
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".into())
    });

    let tessdata_installed = use_resource(move || {
        let _tick = refresh_tick();
        async move {
            stat_tracker::setup::tessdata_dir()
                .join("koverwatch.traineddata")
                .exists()
        }
    });

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            refresh_tick += 1;
        }
    });

    let db_locked = db_locked();

    let (count, unsynced, last_capture) = match &*stats.read() {
        Some(Some(s)) => (
            s.total_matches,
            s.unsynced_count,
            s.last_capture_time.clone(),
        ),
        _ => (0, 0, None),
    };

    let sync_configured = config().sync.is_some();

    rsx! {
        div { class: "panel",
            h2 { "Dashboard" }

            DaemonCard {}

            if db_locked {
                div { class: "card card-warning",
                    h3 { "Database locked" }
                    p { "Stats database is in use by the running daemon. Live stats will appear here when the daemon stops, or check the database directly." }
                }
            }

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
                        if db_locked {
                            span { class: "text-dim", "locked" }
                        } else if let Some(ref t) = last_capture {
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
                        {
                            let data = tessdata_installed.read();
                            let installed = data.as_ref().copied().unwrap_or(false);
                            rsx! {
                                span { class: if installed { "status-dot ok" } else { "status-dot err" } }
                                if installed { "installed" } else { "missing — see Settings" }
                            }
                        }
                    }
                }
            }

            div { class: "card",
                h3 { "Storage" }
                div { class: "stat-row",
                    span { class: "label", "Stored matches" }
                    span { class: "value",
                        if db_locked {
                            span { class: "text-dim", "locked" }
                        } else {
                            "{count}"
                        }
                    }
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
                            if db_locked {
                                span { class: "text-dim", "locked" }
                            } else if unsynced > 0 {
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

async fn load_stats(store: &LocalStore) -> Option<DashboardStats> {
    let total = store.match_count().await.unwrap_or(0);
    let unsynced = store.get_unsynced().await.map(|v| v.len()).unwrap_or(0);
    let last = store.last_capture_time().await;
    Some(DashboardStats {
        total_matches: total,
        unsynced_count: unsynced,
        last_capture_time: last,
    })
}
