use dioxus::prelude::*;

use stat_tracker::config::Config;
use stat_tracker::storage::LocalStore;

#[derive(Clone, PartialEq, Props)]
struct MatchDisplayProps {
    outcome: String,
    hero: String,
    role: String,
    map_name: String,
    elims: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    healing: u32,
    mitigation: u32,
    time_str: String,
}

#[component]
pub fn HistoryPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut refresh_tick = use_signal(|| 0u32);
    let mut db_locked = use_signal(|| false);

    let matches = use_resource(move || {
        let data_dir = config().data_dir.clone();
        let _tick = refresh_tick();
        async move {
            match LocalStore::open(&data_dir).await {
                Ok(store) => {
                    db_locked.set(false);
                    store.get_all_matches().await.unwrap_or_default()
                }
                Err(_) => {
                    db_locked.set(true);
                    stat_tracker::storage::read_match_log(&data_dir)
                }
            }
        }
    });

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            refresh_tick += 1;
        }
    });

    let db_locked = db_locked();

    rsx! {
        div { class: "panel panel-wide",
            h2 { "Match History" }

            if db_locked {
                div { class: "card card-info",
                    p { class: "text-dim text-sm", "Reading from log file (daemon is running)" }
                }
            }

            match &*matches.read() {
                Some(m) if m.is_empty() => rsx! {
                    div { class: "card",
                        p { class: "text-dim", "No matches recorded yet. Play some games with the daemon running!" }
                    }
                },
                Some(m) => rsx! {
                    div { class: "match-count text-dim", "{m.len()} matches" }
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
                        for pm in m.iter() {
                            {
                                let dt: chrono::DateTime<chrono::Utc> = pm.played_at.clone().into();
                                let local = dt.with_timezone(&chrono::Local);
                                let time_str = local.format("%m/%d %H:%M").to_string();
                                rsx! {
                                    MatchRow {
                                        outcome: pm.outcome.clone(),
                                        hero: pm.hero.clone(),
                                        role: pm.role.clone(),
                                        map_name: pm.map_name.clone(),
                                        elims: pm.elims,
                                        deaths: pm.deaths,
                                        assists: pm.assists,
                                        damage: pm.damage,
                                        healing: pm.healing,
                                        mitigation: pm.mitigation,
                                        time_str: time_str,
                                    }
                                }
                            }
                        }
                    }
                },
                None => rsx! {
                    div { class: "card",
                        p { class: "text-dim", "Loading..." }
                    }
                },
            }
        }
    }
}

#[allow(non_snake_case)]
fn MatchRow(props: MatchDisplayProps) -> Element {
    let outcome_class = match props.outcome.to_lowercase().as_str() {
        "win" => "outcome-win",
        "loss" => "outcome-loss",
        _ => "outcome-draw",
    };

    let role_class = match props.role.to_lowercase().as_str() {
        "tank" => "role-tank",
        "damage" => "role-damage",
        "support" => "role-support",
        _ => "",
    };

    rsx! {
        div { class: "match-row",
            span { class: "col-outcome {outcome_class}", "{props.outcome}" }
            span { class: "col-hero", "{props.hero}" }
            span { class: "col-role {role_class}", "{props.role}" }
            span { class: "col-map", "{props.map_name}" }
            span { class: "col-stat", "{props.elims}" }
            span { class: "col-stat", "{props.deaths}" }
            span { class: "col-stat", "{props.assists}" }
            span { class: "col-stat", "{props.damage}" }
            span { class: "col-stat", "{props.healing}" }
            span { class: "col-stat", "{props.mitigation}" }
            span { class: "col-time text-dim", "{props.time_str}" }
        }
    }
}
