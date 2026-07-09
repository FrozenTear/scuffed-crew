use dioxus::prelude::*;

use stat_tracker::capture::CaptureBackend;
use stat_tracker::config::Config;

use super::daemon::DaemonCard;
use super::live_data;

#[component]
pub fn StatusPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut refresh_tick = use_signal(|| 0u32);
    let mut db_locked = use_signal(|| false);

    let stats = use_resource(move || {
        let data_dir = config().data_dir.clone();
        let _tick = refresh_tick();
        async move {
            let live = live_data::fetch_live_matches(&data_dir).await;
            // Snapshot-only path is still "live" data; only flag locked when we
            // have nothing at all (old daemon, no snapshot).
            if live.matches.is_empty() && live.db_locked {
                db_locked.set(true);
                None
            } else {
                db_locked.set(false);
                Some(stats_from_rows(live.matches))
            }
        }
    });

    let outputs = use_resource(move || async move {
        tokio::task::spawn_blocking(|| {
            stat_tracker::capture::wayshot::list_outputs().unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    });

    let backend_label = use_resource(move || async move {
        match stat_tracker::capture::detect_backend().await {
            CaptureBackend::Wayshot => "libwayshot (Wayland)".to_string(),
            CaptureBackend::Portal => "XDG Desktop Portal".to_string(),
            CaptureBackend::None => "none available".to_string(),
        }
    });

    let selected_output = config().capture_output.clone().unwrap_or_else(|| {
        outputs
            .read()
            .as_ref()
            .and_then(|o| o.first().cloned())
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

    let (count, unsynced, last_capture, recent) = match &*stats.read() {
        Some(Some(s)) => (
            s.total_matches,
            s.unsynced_count,
            s.last_capture_time.clone(),
            s.recent.clone(),
        ),
        _ => (0, 0, None, Vec::new()),
    };

    let sync_configured = config().sync.is_some();

    rsx! {
        div { class: "panel",
            h2 { "Dashboard" }

            DaemonCard {}

            if !recent.is_empty() {
                div { class: "card",
                    h3 { "Recent Games" }
                    div { class: "recent-list",
                        for g in recent.iter() {
                            {
                                let outcome_class = match g.outcome.as_str() {
                                    "victory" | "win" => "outcome-win",
                                    "defeat" | "loss" => "outcome-loss",
                                    "draw" => "outcome-draw",
                                    _ => "outcome-unknown",
                                };
                                let dt: chrono::DateTime<chrono::Utc> = g.played_at.into();
                                let when = dt.with_timezone(&chrono::Local).format("%a %H:%M").to_string();
                                let map = if g.map_name.is_empty() { "—".to_string() } else { g.map_name.clone() };
                                rsx! {
                                    div { class: "recent-row",
                                        span { class: "recent-outcome {outcome_class}", "{g.outcome.to_uppercase()}" }
                                        span { class: "recent-hero", "{g.hero}" }
                                        span { class: "recent-map text-dim", "{map}" }
                                        span { class: "recent-time text-dim", "{when}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if db_locked {
                div { class: "card card-warning",
                    h3 { "Database locked" }
                    p { "Stats database is in use by the running daemon, which hasn't written a live snapshot yet (older build?). Stats will appear when the daemon stops or after its first capture." }
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
                    span { class: "value",
                        {
                            let n = outputs.read().as_ref().map(|o| o.len()).unwrap_or(0);
                            rsx! { "{n}" }
                        }
                    }
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
                    span { class: "value",
                        {
                            let label = backend_label
                                .read()
                                .as_ref()
                                .cloned()
                                .unwrap_or_else(|| "…".into());
                            rsx! { "{label}" }
                        }
                    }
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
                    span { class: "label", "Games recorded" }
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
                        span { class: "label", "Unsynced captures" }
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
    /// The most recent games (final snapshot per session), newest first.
    recent: Vec<stat_tracker::storage::PersonalMatch>,
}

/// Dashboard numbers from the raw snapshot rows (newest first): games are
/// counted per session, sync state per row.
fn stats_from_rows(rows: Vec<stat_tracker::storage::PersonalMatch>) -> DashboardStats {
    let unsynced = rows.iter().filter(|m| !m.synced).count();
    let last = rows.first().map(|m| {
        let dt: chrono::DateTime<chrono::Utc> = m.played_at.into();
        dt.with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    });
    let games = stat_tracker::storage::latest_per_game(rows);
    DashboardStats {
        total_matches: games.len(),
        unsynced_count: unsynced,
        last_capture_time: last,
        recent: games.into_iter().take(5).collect(),
    }
}
