use dioxus::prelude::*;

use stat_tracker::config::Config;
use stat_tracker::storage::{self, LocalStore, PersonalMatch, StoreCommand};

use super::live_data;

/// One row per game, grouped by day, with an expandable detail view: the
/// session's capture timeline plus manual outcome editing / deletion (applied
/// directly when the store is free, queued to the daemon otherwise).
#[component]
pub fn MatchesPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut refresh_tick = use_signal(|| 0u32);
    let mut db_locked = use_signal(|| false);
    let mut selected: Signal<Option<String>> = use_signal(|| None);
    let mut confirm_delete: Signal<Option<String>> = use_signal(|| None);
    let mut toast: Signal<Option<(String, bool)>> = use_signal(|| None);

    let rows = use_resource(move || {
        let data_dir = config().data_dir.clone();
        let _tick = refresh_tick();
        async move {
            let live = live_data::fetch_live_matches(&data_dir).await;
            db_locked.set(live.db_locked);
            live.matches
        }
    });

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            refresh_tick += 1;
        }
    });

    // Apply an edit directly when the store is free; queue it for the daemon
    // otherwise. Refreshes the view after the daemon has had time to apply.
    let send_command = move |cmd: StoreCommand, ok_msg: &'static str| {
        let data_dir = config().data_dir.clone();
        spawn(async move {
            let result = match LocalStore::open(&data_dir).await {
                Ok(store) => {
                    let r = store.apply_command(&cmd).await.map_err(|e| e.to_string());
                    if r.is_ok() {
                        let _ = store.export_snapshot(&data_dir).await;
                    }
                    r
                }
                Err(_) => storage::queue_command(&data_dir, &cmd).map_err(|e| e.to_string()),
            };
            match result {
                Ok(()) => toast.set(Some((ok_msg.to_string(), true))),
                Err(e) => toast.set(Some((format!("Edit failed: {e}"), false))),
            }
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            refresh_tick += 1;
            toast.set(None);
        });
    };

    // Recompute only when the rows resource or selection changes — not on every
    // unrelated parent re-render. One clone of the match list; detail filter
    // runs before `latest_per_game` consumes ownership.
    let derived = use_memo(move || {
        let selected_sid = selected();
        let binding = rows.read();
        let all: Vec<PersonalMatch> = binding.as_deref().unwrap_or(&[]).to_vec();

        let detail_snaps: Vec<PersonalMatch> = selected_sid
            .as_deref()
            .map(|sid| {
                let mut v: Vec<PersonalMatch> = all
                    .iter()
                    .filter(|m| m.session_id == sid)
                    .cloned()
                    .collect();
                v.reverse();
                v
            })
            .unwrap_or_default();

        let games = storage::latest_per_game(all);
        let game_count = games.len();
        let mut groups: Vec<(String, Vec<PersonalMatch>)> = Vec::new();
        for g in games {
            let dt: chrono::DateTime<chrono::Utc> = g.played_at.into();
            let day = dt
                .with_timezone(&chrono::Local)
                .format("%A %d %B")
                .to_string();
            match groups.last_mut() {
                Some((d, v)) if *d == day => v.push(g),
                _ => groups.push((day, vec![g])),
            }
        }
        (game_count, groups, detail_snaps)
    });
    let (game_count, groups, detail_snaps) = derived();
    let selected_sid = selected();

    rsx! {
        div { class: "panel panel-wide",
            h2 { "Matches" }

            if db_locked() {
                div { class: "card card-info",
                    p { class: "text-dim text-sm", "Live view from daemon snapshot (daemon is running)" }
                }
            }

            p { class: "match-count text-dim", "{game_count} games · click a game to inspect or correct it" }

            for (day, day_games) in groups {
                div { class: "day-header", "{day}" }
                div { class: "match-table",
                    div { class: "match-header",
                        span { class: "col-outcome", "Result" }
                        span { class: "col-hero", "Hero" }
                        span { class: "col-role", "Role" }
                        span { class: "col-map", "Map" }
                        span { class: "col-stat", "E" }
                        span { class: "col-stat", "D" }
                        span { class: "col-stat", "A" }
                        span { class: "col-stat", "DMG" }
                        span { class: "col-stat", "HLG" }
                        span { class: "col-stat", "MIT" }
                        span { class: "col-time", "Time" }
                    }
                    for g in day_games {
                        {
                            let sid = g.session_id.clone();
                            let is_selected = !sid.is_empty() && selected_sid.as_deref() == Some(sid.as_str());
                            let outcome_class = match g.outcome.as_str() {
                                "victory" | "win" => "win",
                                "defeat" | "loss" => "loss",
                                "draw" => "draw",
                                _ => "undecided",
                            };
                            let outcome_text_class = match outcome_class {
                                "win" => "outcome-win",
                                "loss" => "outcome-loss",
                                "draw" => "outcome-draw",
                                _ => "outcome-unknown",
                            };
                            let dt: chrono::DateTime<chrono::Utc> = g.played_at.into();
                            let time_str = dt.with_timezone(&chrono::Local).format("%H:%M").to_string();
                            let row_sid = sid.clone();
                            let editable = !sid.is_empty();
                            let sid_v = sid.clone();
                            let sid_d = sid.clone();
                            let sid_w = sid.clone();
                            let sid_del = sid.clone();
                            let sid_del2 = sid.clone();
                            let delete_pending = confirm_delete().as_deref() == Some(sid.as_str()) && !sid.is_empty();
                            rsx! {
                                div {
                                    class: if is_selected { "match-row {outcome_class} selected" } else { "match-row {outcome_class}" },
                                    onclick: move |_| {
                                        if row_sid.is_empty() { return; }
                                        if selected().as_deref() == Some(row_sid.as_str()) {
                                            selected.set(None);
                                        } else {
                                            selected.set(Some(row_sid.clone()));
                                        }
                                        confirm_delete.set(None);
                                    },
                                    span { class: "col-outcome {outcome_text_class}", "{g.outcome.to_uppercase()}" }
                                    span { class: "col-hero", "{g.hero}" }
                                    span { class: "col-role role-{g.role.to_lowercase()}", "{g.role}" }
                                    span { class: "col-map", if g.map_name.is_empty() { "—" } else { "{g.map_name}" } }
                                    span { class: "col-stat", "{g.elims}" }
                                    span { class: "col-stat", "{g.deaths}" }
                                    span { class: "col-stat", "{g.assists}" }
                                    span { class: "col-stat", "{g.damage}" }
                                    span { class: "col-stat", "{g.healing}" }
                                    span { class: "col-stat", "{g.mitigation}" }
                                    span { class: "col-time", "{time_str}" }
                                }
                                if is_selected && editable {
                                    div { class: "game-detail",
                                        div { class: "detail-actions",
                                            span { class: "detail-label", "Set outcome" }
                                            button {
                                                class: if g.outcome == "victory" { "btn btn-sm btn-outcome current" } else { "btn btn-sm btn-outcome" },
                                                onclick: move |e| {
                                                    e.stop_propagation();
                                                    send_command(StoreCommand::SetOutcome { session_id: sid_v.clone(), outcome: "victory".into() }, "Outcome set to victory");
                                                },
                                                "Victory"
                                            }
                                            button {
                                                class: if g.outcome == "defeat" { "btn btn-sm btn-outcome current" } else { "btn btn-sm btn-outcome" },
                                                onclick: move |e| {
                                                    e.stop_propagation();
                                                    send_command(StoreCommand::SetOutcome { session_id: sid_d.clone(), outcome: "defeat".into() }, "Outcome set to defeat");
                                                },
                                                "Defeat"
                                            }
                                            button {
                                                class: if g.outcome == "draw" { "btn btn-sm btn-outcome current" } else { "btn btn-sm btn-outcome" },
                                                onclick: move |e| {
                                                    e.stop_propagation();
                                                    send_command(StoreCommand::SetOutcome { session_id: sid_w.clone(), outcome: "draw".into() }, "Outcome set to draw");
                                                },
                                                "Draw"
                                            }
                                            span { class: "detail-spacer" }
                                            button {
                                                class: "btn btn-sm btn-danger",
                                                onclick: move |e| {
                                                    e.stop_propagation();
                                                    if delete_pending {
                                                        send_command(StoreCommand::DeleteSession { session_id: sid_del.clone() }, "Game deleted");
                                                        confirm_delete.set(None);
                                                        selected.set(None);
                                                    } else {
                                                        confirm_delete.set(Some(sid_del2.clone()));
                                                    }
                                                },
                                                if delete_pending { "Click again to confirm" } else { "Delete game" }
                                            }
                                        }
                                        div { class: "timeline-table",
                                            div { class: "timeline-header",
                                                span { class: "col-capture", "#" }
                                                span { class: "col-stat", "E" }
                                                span { class: "col-stat", "D" }
                                                span { class: "col-stat", "A" }
                                                span { class: "col-stat", "DMG" }
                                                span { class: "col-stat", "HLG" }
                                                span { class: "col-stat", "MIT" }
                                                span { class: "col-time", "Time" }
                                            }
                                            for (i, snap) in detail_snaps.iter().enumerate() {
                                                {
                                                    let dt: chrono::DateTime<chrono::Utc> = snap.played_at.into();
                                                    let t = dt.with_timezone(&chrono::Local).format("%H:%M:%S").to_string();
                                                    let n = i + 1;
                                                    rsx! {
                                                        div { class: "timeline-row",
                                                            span { class: "col-capture", "{n}" }
                                                            span { class: "col-stat", "{snap.elims}" }
                                                            span { class: "col-stat", "{snap.deaths}" }
                                                            span { class: "col-stat", "{snap.assists}" }
                                                            span { class: "col-stat", "{snap.damage}" }
                                                            span { class: "col-stat", "{snap.healing}" }
                                                            span { class: "col-stat", "{snap.mitigation}" }
                                                            span { class: "col-time", "{t}" }
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
            }

            if let Some((msg, ok)) = toast() {
                div { class: if ok { "toast success" } else { "toast error" }, "{msg}" }
            }
        }
    }
}
