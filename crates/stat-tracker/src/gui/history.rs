use dioxus::prelude::*;

use stat_tracker::config::Config;
use stat_tracker::detect::MatchOutcome;
use stat_tracker::storage::{self, LocalStore, MatchEdit, PersonalMatch, StoreCommand};

use super::live_data;

/// Editable text state for the manual-correction form. One `String` per field
/// (parsed on save), prefilled from the game's effective (displayed) values.
#[derive(Clone, PartialEq, Default)]
struct EditForm {
    hero: String,
    role: String,
    map_name: String,
    elims: String,
    deaths: String,
    assists: String,
    damage: String,
    healing: String,
    mitigation: String,
}

impl EditForm {
    fn from_game(g: &PersonalMatch) -> Self {
        Self {
            hero: g.display_hero().to_string(),
            role: g.display_role().to_string(),
            map_name: g.display_map_name().to_string(),
            elims: g.display_elims().to_string(),
            deaths: g.display_deaths().to_string(),
            assists: g.display_assists().to_string(),
            damage: g.display_damage().to_string(),
            healing: g.display_healing().to_string(),
            mitigation: g.display_mitigation().to_string(),
        }
    }

    /// Build a [`MatchEdit`] holding only the fields that differ from the game's
    /// current effective values — an unchanged field is left as its OCR read.
    fn diff(&self, g: &PersonalMatch) -> MatchEdit {
        let mut e = MatchEdit::default();
        let txt = |cur: &str, disp: &str| {
            let t = cur.trim();
            (!t.is_empty() && t != disp).then(|| t.to_string())
        };
        e.hero = txt(&self.hero, g.display_hero());
        e.role = txt(&self.role, g.display_role());
        e.map_name = txt(&self.map_name, g.display_map_name());
        let num = |cur: &str, disp: u32| cur.trim().parse::<u32>().ok().filter(|v| *v != disp);
        e.elims = num(&self.elims, g.display_elims());
        e.deaths = num(&self.deaths, g.display_deaths());
        e.assists = num(&self.assists, g.display_assists());
        e.damage = num(&self.damage, g.display_damage());
        e.healing = num(&self.healing, g.display_healing());
        e.mitigation = num(&self.mitigation, g.display_mitigation());
        e
    }
}

/// Human labels + "OCR read → corrected" pairs for a game's edited fields,
/// for the transparency detail. Reads directly off the `corrected_*` overlay
/// so it stays in sync with what was actually stored.
fn corrections(g: &PersonalMatch) -> Vec<(&'static str, String, String)> {
    let mut out: Vec<(&'static str, String, String)> = Vec::new();
    if let Some(v) = &g.corrected_hero {
        out.push(("Hero", g.hero.clone(), v.clone()));
    }
    if let Some(v) = &g.corrected_role {
        out.push(("Role", g.role.clone(), v.clone()));
    }
    if let Some(v) = &g.corrected_map_name {
        let ocr = if g.map_name.is_empty() {
            "—".into()
        } else {
            g.map_name.clone()
        };
        out.push(("Map", ocr, v.clone()));
    }
    if let Some(v) = &g.corrected_outcome {
        out.push(("Result", g.outcome.clone(), v.clone()));
    }
    if let Some(v) = g.corrected_elims {
        out.push(("Elims", g.elims.to_string(), v.to_string()));
    }
    if let Some(v) = g.corrected_deaths {
        out.push(("Deaths", g.deaths.to_string(), v.to_string()));
    }
    if let Some(v) = g.corrected_assists {
        out.push(("Assists", g.assists.to_string(), v.to_string()));
    }
    if let Some(v) = g.corrected_damage {
        out.push(("Damage", g.damage.to_string(), v.to_string()));
    }
    if let Some(v) = g.corrected_healing {
        out.push(("Healing", g.healing.to_string(), v.to_string()));
    }
    if let Some(v) = g.corrected_mitigation {
        out.push(("Mitigation", g.mitigation.to_string(), v.to_string()));
    }
    out
}

/// One row per game, grouped by day, with an expandable detail view: the
/// session's capture timeline, manual outcome editing / deletion, and a manual
/// stat-correction form. Edited games keep an "edited" badge and expose the
/// original OCR reads. Mutations apply directly when the store is free, else
/// queue to the daemon.
#[component]
pub fn MatchesPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut refresh_tick = use_signal(|| 0u32);
    let mut db_locked = use_signal(|| false);
    let mut selected: Signal<Option<String>> = use_signal(|| None);
    let mut confirm_delete: Signal<Option<String>> = use_signal(|| None);
    let mut editing: Signal<bool> = use_signal(|| false);
    let mut form: Signal<EditForm> = use_signal(EditForm::default);
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
                            let outcome = MatchOutcome::parse_lenient(g.display_outcome());
                            let outcome_class = outcome.row_class();
                            let outcome_text_class = outcome.text_class();
                            let dt: chrono::DateTime<chrono::Utc> = g.played_at.into();
                            let time_str = dt.with_timezone(&chrono::Local).format("%H:%M").to_string();
                            let row_sid = sid.clone();
                            let editable = !sid.is_empty();
                            let sid_v = sid.clone();
                            let sid_d = sid.clone();
                            let sid_w = sid.clone();
                            let sid_del = sid.clone();
                            let sid_del2 = sid.clone();
                            let sid_save = sid.clone();
                            let delete_pending = confirm_delete().as_deref() == Some(sid.as_str()) && !sid.is_empty();
                            let is_edited = g.is_edited();
                            let badge_title = if is_edited {
                                format!("Manually edited: {}", corrections(&g)
                                    .iter().map(|(l, _, _)| *l).collect::<Vec<_>>().join(", "))
                            } else {
                                String::new()
                            };
                            let corr = corrections(&g);
                            let g_open = g.clone();
                            let g_save = g.clone();
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
                                        editing.set(false);
                                    },
                                    span { class: "col-outcome {outcome_text_class}", "{g.display_outcome().to_uppercase()}" }
                                    span { class: "col-hero",
                                        span { class: "hero-name-text", "{g.display_hero()}" }
                                        if is_edited {
                                            span { class: "edit-badge", title: "{badge_title}", "edited" }
                                        }
                                    }
                                    span { class: "col-role role-{g.display_role().to_lowercase()}", "{g.display_role()}" }
                                    span { class: "col-map", if g.display_map_name().is_empty() { "—" } else { "{g.display_map_name()}" } }
                                    span { class: "col-stat", "{g.display_elims()}" }
                                    span { class: "col-stat", "{g.display_deaths()}" }
                                    span { class: "col-stat", "{g.display_assists()}" }
                                    span { class: "col-stat", "{g.display_damage()}" }
                                    span { class: "col-stat", "{g.display_healing()}" }
                                    span { class: "col-stat", "{g.display_mitigation()}" }
                                    span { class: "col-time", "{time_str}" }
                                }
                                if is_selected && editable {
                                    div { class: "game-detail",
                                        div { class: "detail-actions",
                                            span { class: "detail-label", "Set outcome" }
                                            button {
                                                class: if outcome == MatchOutcome::Victory { "btn btn-sm btn-outcome current" } else { "btn btn-sm btn-outcome" },
                                                onclick: move |e| {
                                                    e.stop_propagation();
                                                    send_command(StoreCommand::SetOutcome { session_id: sid_v.clone(), outcome: MatchOutcome::Victory.to_string() }, "Outcome set to victory");
                                                },
                                                "Victory"
                                            }
                                            button {
                                                class: if outcome == MatchOutcome::Defeat { "btn btn-sm btn-outcome current" } else { "btn btn-sm btn-outcome" },
                                                onclick: move |e| {
                                                    e.stop_propagation();
                                                    send_command(StoreCommand::SetOutcome { session_id: sid_d.clone(), outcome: MatchOutcome::Defeat.to_string() }, "Outcome set to defeat");
                                                },
                                                "Defeat"
                                            }
                                            button {
                                                class: if outcome == MatchOutcome::Draw { "btn btn-sm btn-outcome current" } else { "btn btn-sm btn-outcome" },
                                                onclick: move |e| {
                                                    e.stop_propagation();
                                                    send_command(StoreCommand::SetOutcome { session_id: sid_w.clone(), outcome: MatchOutcome::Draw.to_string() }, "Outcome set to draw");
                                                },
                                                "Draw"
                                            }
                                            span { class: "detail-spacer" }
                                            button {
                                                class: if editing() { "btn btn-sm btn-outline current" } else { "btn btn-sm btn-outline" },
                                                onclick: move |e| {
                                                    e.stop_propagation();
                                                    if editing() {
                                                        editing.set(false);
                                                    } else {
                                                        form.set(EditForm::from_game(&g_open));
                                                        editing.set(true);
                                                    }
                                                },
                                                if editing() { "Cancel edit" } else { "Edit stats" }
                                            }
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

                                        if !corr.is_empty() {
                                            div { class: "corrections",
                                                span { class: "detail-label", "Corrections" }
                                                for (label, ocr, fixed) in corr.iter() {
                                                    div { class: "correction-row",
                                                        span { class: "correction-field", "{label}" }
                                                        span { class: "correction-ocr", "OCR read {ocr}" }
                                                        span { class: "correction-arrow", "→" }
                                                        span { class: "correction-fixed", "{fixed}" }
                                                    }
                                                }
                                            }
                                        }

                                        if editing() {
                                            div { class: "edit-form",
                                                span { class: "detail-label", "Correct stats (blank / unchanged fields keep the OCR read)" }
                                                div { class: "edit-grid",
                                                    label { class: "edit-field",
                                                        span { "Hero" }
                                                        input { r#type: "text", value: "{form().hero}",
                                                            oninput: move |e| form.with_mut(|f| f.hero = e.value()) }
                                                    }
                                                    label { class: "edit-field",
                                                        span { "Role" }
                                                        select {
                                                            value: "{form().role}",
                                                            onchange: move |e| form.with_mut(|f| f.role = e.value()),
                                                            option { value: "Tank", "Tank" }
                                                            option { value: "Damage", "Damage" }
                                                            option { value: "Support", "Support" }
                                                        }
                                                    }
                                                    label { class: "edit-field",
                                                        span { "Map" }
                                                        input { r#type: "text", value: "{form().map_name}",
                                                            oninput: move |e| form.with_mut(|f| f.map_name = e.value()) }
                                                    }
                                                    label { class: "edit-field",
                                                        span { "Elims" }
                                                        input { r#type: "number", min: "0", value: "{form().elims}",
                                                            oninput: move |e| form.with_mut(|f| f.elims = e.value()) }
                                                    }
                                                    label { class: "edit-field",
                                                        span { "Deaths" }
                                                        input { r#type: "number", min: "0", value: "{form().deaths}",
                                                            oninput: move |e| form.with_mut(|f| f.deaths = e.value()) }
                                                    }
                                                    label { class: "edit-field",
                                                        span { "Assists" }
                                                        input { r#type: "number", min: "0", value: "{form().assists}",
                                                            oninput: move |e| form.with_mut(|f| f.assists = e.value()) }
                                                    }
                                                    label { class: "edit-field",
                                                        span { "Damage" }
                                                        input { r#type: "number", min: "0", value: "{form().damage}",
                                                            oninput: move |e| form.with_mut(|f| f.damage = e.value()) }
                                                    }
                                                    label { class: "edit-field",
                                                        span { "Healing" }
                                                        input { r#type: "number", min: "0", value: "{form().healing}",
                                                            oninput: move |e| form.with_mut(|f| f.healing = e.value()) }
                                                    }
                                                    label { class: "edit-field",
                                                        span { "Mitigation" }
                                                        input { r#type: "number", min: "0", value: "{form().mitigation}",
                                                            oninput: move |e| form.with_mut(|f| f.mitigation = e.value()) }
                                                    }
                                                }
                                                div { class: "edit-form-actions",
                                                    button {
                                                        class: "btn btn-sm btn-primary",
                                                        onclick: move |e| {
                                                            e.stop_propagation();
                                                            let edit = form().diff(&g_save);
                                                            if edit.is_empty() {
                                                                toast.set(Some(("No changes to save".to_string(), false)));
                                                            } else {
                                                                send_command(StoreCommand::EditMatch { session_id: sid_save.clone(), edit }, "Stats corrected");
                                                            }
                                                            editing.set(false);
                                                        },
                                                        "Save corrections"
                                                    }
                                                }
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
