use std::collections::HashMap;
use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{
    CreateTournamentRequest, StatusChangeRequest, AddParticipantRequest, MatchReportRequest,
};
use crate::components::{
    DataTable, FormModal, ConfirmDialog, StatusPill, Toast, use_toast,
};
use crate::hooks::{use_api, ModalController};

// --- Types ---
// Local response types with simplified/string-typed fields for display.

#[derive(Debug, Clone, Deserialize)]
struct Tournament {
    id: String,
    name: String,
    format: String,
    game_name: Option<String>,
    status: String,
    max_participants: Option<u32>,
    starts_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BracketData {
    participants: Vec<Participant>,
    rounds: Vec<Round>,
    matches: Vec<BracketMatch>,
}

#[derive(Debug, Clone, Deserialize)]
struct Participant {
    id: String,
    name: String,
    seed: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct Round {
    id: String,
    round_number: u32,
    stage: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BracketMatch {
    id: String,
    round_id: String,
    bracket_position: u32,
    participant_a_id: Option<String>,
    participant_b_id: Option<String>,
    score_a: Option<u32>,
    score_b: Option<u32>,
    winner_id: Option<String>,
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
}

const FORMATS: [&str; 4] = ["single_elim", "double_elim", "round_robin", "swiss"];
const STATUS_FILTERS: [&str; 6] = ["all", "draft", "registration", "active", "completed", "in_progress"];

#[component]
pub fn AdminTournaments() -> Element {
    let mut tournaments = use_api::<Vec<Tournament>>("/api/tournaments");
    let mut games = use_api::<Vec<Game>>("/api/games");
    let mut members = use_api::<Vec<Member>>("/api/members");
    let mut toast = use_toast();

    // View toggle: None = list, Some(id) = detail
    let mut detail_id: Signal<Option<String>> = use_signal(|| None);

    // Status filter
    let mut status_filter = use_signal(|| "all".to_string());

    // Form modal state
    let mut modal = ModalController::<String>::new();
    let mut form_name = use_signal(String::new);
    let mut form_format = use_signal(|| "single_elim".to_string());
    let mut form_game_id: Signal<Option<String>> = use_signal(|| None);
    let mut form_max = use_signal(String::new);
    let mut form_date = use_signal(String::new);
    let mut form_time = use_signal(String::new);

    // Delete confirm
    let mut delete_modal = ModalController::<Tournament>::new();

    // Detail view state
    let mut detail_refresh = use_signal(|| 0u64);
    let mut bracket_data: Signal<Option<BracketData>> = use_signal(|| None);
    let mut detail_tournament: Signal<Option<Tournament>> = use_signal(|| None);
    let mut detail_loading = use_signal(|| false);

    // Add participant
    let mut add_part_modal = ModalController::<()>::new();
    let mut add_part_member_id = use_signal(String::new);

    // Remove participant confirm
    let mut remove_part_modal = ModalController::<Participant>::new();

    // Match report modal
    let mut match_modal = ModalController::<BracketMatch>::new();
    let mut match_score_a = use_signal(String::new);
    let mut match_score_b = use_signal(String::new);
    let mut match_winner = use_signal(String::new);
    let mut match_replays = use_signal(String::new);


    // Detail data loader
    let _detail_loader = use_resource(move || async move {
        let _ = detail_refresh();
        if let Some(id) = detail_id() {
            detail_loading.set(true);
            if let Ok(t) = ApiClient::web()
                .fetch::<Tournament>(&format!("/api/tournaments/{id}"))
                .await
            {
                detail_tournament.set(Some(t));
            }
            if let Ok(b) = ApiClient::web()
                .fetch::<BracketData>(&format!("/api/tournaments/{id}/bracket"))
                .await
            {
                bracket_data.set(Some(b));
            }
            detail_loading.set(false);
        }
    });

    // --- List view handlers ---

    let open_create = move |_| {
        form_name.set(String::new());
        form_format.set("single_elim".to_string());
        form_game_id.set(None);
        form_max.set(String::new());
        form_date.set(String::new());
        form_time.set(String::new());
        modal.show_empty();
    };

    let mut open_edit = move |t: Tournament| {
        form_name.set(t.name);
        form_format.set(t.format);
        form_game_id.set(t.game_name.map(|_| String::new()));
        form_max.set(t.max_participants.map(|n| n.to_string()).unwrap_or_default());
        // Split "2026-03-11T18:00:00Z" into date + time
        let raw = t.starts_at.unwrap_or_default();
        if let Some((d, rest)) = raw.split_once('T') {
            form_date.set(d.to_string());
            form_time.set(rest.trim_end_matches('Z').chars().take(5).collect());
        } else {
            form_date.set(String::new());
            form_time.set(String::new());
        }
        modal.show(t.id);
    };

    let on_close = move |_| modal.close();

    let on_submit = move |_| {
        let name = form_name().trim().to_string();
        if name.is_empty() {
            toast.show(Toast::error("Name is required."));
            return;
        }
        let max_str = form_max().trim().to_string();
        let max_val = if max_str.is_empty() { None } else { max_str.parse::<u32>().ok() };
        let date = form_date().trim().to_string();
        let time = form_time().trim().to_string();
        let starts_at = if date.is_empty() {
            None
        } else if time.is_empty() {
            Some(format!("{date}T00:00"))
        } else {
            Some(format!("{date}T{time}"))
        };
        let body = CreateTournamentRequest {
            name,
            format: form_format(),
            game_id: form_game_id(),
            max_participants: max_val,
            starts_at,
        };
        let edit_id = modal.get_target();
        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                client.put_json::<_, Tournament>(&format!("/api/tournaments/{id}"), &body).await
            } else {
                client.post_json::<_, Tournament>("/api/tournaments", &body).await
            };
            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Tournament saved."));
                    modal.close();
                    tournaments.refresh += 1;
                    games.refresh += 1;
                    members.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to save: {e}"))),
            }
        });
    };

    let mut open_delete = move |t: Tournament| {
        delete_modal.show(t);
    };

    let on_delete_confirm = move |_| {
        if let Some(t) = delete_modal.get_target() {
            let id = t.id.clone();
            delete_modal.close();
            spawn(async move {
                match ApiClient::web().delete(&format!("/api/tournaments/{id}")).await {
                    Ok(_) => {
                        toast.show(Toast::success("Tournament deleted."));
                        tournaments.refresh += 1;
                        games.refresh += 1;
                        members.refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Delete failed: {e}"))),
                }
            });
        }
    };

    let on_delete_cancel = move |_| {
        delete_modal.close();
    };

    let change_status = move |(id, new_status): (String, String)| {
        let body = StatusChangeRequest { status: new_status.clone() };
        spawn(async move {
            let result = ApiClient::web()
                .patch_json_empty(&format!("/api/tournaments/{id}"), &body)
                .await;
            match result {
                Ok(_) => {
                    toast.show(Toast::success(format!("Status changed to {new_status}.")));
                    tournaments.refresh += 1;
                    games.refresh += 1;
                    members.refresh += 1;
                    detail_refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Status change failed: {e}"))),
            }
        });
    };

    let mut view_detail = move |id: String| {
        detail_id.set(Some(id));
        bracket_data.set(None);
        detail_tournament.set(None);
        detail_refresh += 1;
    };

    let back_to_list = move |_| {
        detail_id.set(None);
        bracket_data.set(None);
        detail_tournament.set(None);
    };

    // --- Detail view handlers ---

    let on_add_part_open = move |_| {
        add_part_member_id.set(String::new());
        add_part_modal.show_empty();
    };

    let on_add_part_close = move |_| add_part_modal.close();

    let on_add_part_submit = move |_| {
        let mid = add_part_member_id().trim().to_string();
        if mid.is_empty() {
            return;
        }
        if let Some(id) = detail_id() {
            let body = AddParticipantRequest { member_id: mid };
            add_part_modal.start_submit();
            spawn(async move {
                let result = ApiClient::web()
                    .post_json_empty(&format!("/api/tournaments/{id}/participants"), &body)
                    .await;
                add_part_modal.end_submit();
                match result {
                    Ok(_) => {
                        toast.show(Toast::success("Participant added."));
                        add_part_modal.close();
                        detail_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
                }
            });
        }
    };

    let mut open_remove_part = move |p: Participant| {
        remove_part_modal.show(p);
    };

    let on_remove_part_confirm = move |_| {
        if let Some(p) = remove_part_modal.get_target() {
            if let Some(tid) = detail_id() {
                let pid = p.id.clone();
                remove_part_modal.close();
                spawn(async move {
                    match ApiClient::web()
                        .delete(&format!("/api/tournaments/{tid}/participants/{pid}"))
                        .await
                    {
                        Ok(_) => {
                            toast.show(Toast::success("Participant removed."));
                            detail_refresh += 1;
                        }
                        Err(e) => toast.show(Toast::error(format!("Remove failed: {e}"))),
                    }
                });
            }
        }
    };

    let on_remove_part_cancel = move |_| {
        remove_part_modal.close();
    };

    let generate_bracket = move |_| {
        if let Some(tid) = detail_id() {
            spawn(async move {
                match ApiClient::web()
                    .post_json_empty(&format!("/api/tournaments/{tid}/bracket/generate"), &())
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Bracket generated."));
                        detail_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Generate failed: {e}"))),
                }
            });
        }
    };

    let next_swiss_round = move |_| {
        if let Some(tid) = detail_id() {
            spawn(async move {
                match ApiClient::web()
                    .post_json_empty(&format!("/api/tournaments/{tid}/bracket/next-round"), &())
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Next Swiss round generated."));
                        detail_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
                }
            });
        }
    };

    let mut open_match_report = move |m: BracketMatch| {
        match_score_a.set(m.score_a.map(|n| n.to_string()).unwrap_or_default());
        match_score_b.set(m.score_b.map(|n| n.to_string()).unwrap_or_default());
        match_winner.set(m.winner_id.clone().unwrap_or_default());
        match_replays.set(String::new());
        match_modal.show(m);
    };

    let on_match_close = move |_| {
        match_modal.close();
    };

    let on_match_submit = move |_| {
        if let Some(m) = match_modal.get_target() {
            if let Some(tid) = detail_id() {
                let mid = m.id.clone();
                let sa = match_score_a().trim().to_string();
                let sb = match_score_b().trim().to_string();
                let winner_raw = match_winner().trim().to_string();
                let replays_raw = match_replays().trim().to_string();
                let body = MatchReportRequest {
                    score_a: if sa.is_empty() { None } else { sa.parse::<u32>().ok() },
                    score_b: if sb.is_empty() { None } else { sb.parse::<u32>().ok() },
                    winner_id: if winner_raw.is_empty() { None } else { Some(winner_raw) },
                    replay_codes: if replays_raw.is_empty() { None } else { Some(replays_raw) },
                };
                match_modal.start_submit();
                spawn(async move {
                    let result = ApiClient::web()
                        .put_json_empty(
                            &format!("/api/tournaments/{tid}/matches/{mid}"),
                            &body,
                        )
                        .await;
                    match_modal.end_submit();
                    match result {
                        Ok(_) => {
                            toast.show(Toast::success("Match result saved."));
                            match_modal.close();
                            detail_refresh += 1;
                        }
                        Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
                    }
                });
            }
        }
    };

    // --- Render ---

    rsx! {

        if detail_id().is_some() {
            // ==================== DETAIL VIEW ====================
            {
                if detail_loading() {
                    rsx! { p { class: "admin-loading", "Loading tournament..." } }
                } else if let Some(tournament) = detail_tournament() {
                    let bracket = bracket_data();
                    let participant_map: HashMap<String, String> = bracket
                        .as_ref()
                        .map(|b| {
                            b.participants
                                .iter()
                                .map(|p| (p.id.clone(), p.name.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let t_id = tournament.id.clone();
                    let t_id2 = tournament.id.clone();
                    let is_swiss = tournament.format == "swiss";
                    let is_active = tournament.status == "active";

                    rsx! {
                        div { class: "admin-toolbar",
                            div { style: "display:flex;align-items:center;gap:1rem;",
                                button {
                                    class: "row-btn",
                                    onclick: back_to_list,
                                    "\u{2190} Back"
                                }
                                h1 { "{tournament.name}" }
                                StatusPill { status: tournament.status.clone() }
                            }
                            div { style: "display:flex;gap:0.5rem;flex-wrap:wrap;",
                                // Status transitions
                                if tournament.status == "draft" {
                                    button {
                                        class: "row-btn primary",
                                        onclick: move |_| change_status((t_id.clone(), "registration".to_string())),
                                        "Open Registration"
                                    }
                                }
                                if tournament.status == "registration" {
                                    button {
                                        class: "row-btn primary",
                                        onclick: move |_| change_status((t_id2.clone(), "active".to_string())),
                                        "Start Tournament"
                                    }
                                }
                                if is_active {
                                    {
                                        let tid_complete = tournament.id.clone();
                                        rsx! {
                                            button {
                                                class: "row-btn danger",
                                                onclick: move |_| change_status((tid_complete.clone(), "completed".to_string())),
                                                "Complete"
                                            }
                                        }
                                    }
                                }
                                button {
                                    class: "btn-add",
                                    onclick: generate_bracket,
                                    "Generate Bracket"
                                }
                                if is_swiss && is_active {
                                    button {
                                        class: "btn-add",
                                        onclick: next_swiss_round,
                                        "Next Swiss Round"
                                    }
                                }
                            }
                        }

                        // Participants section
                        div { style: "margin-bottom:2rem;",
                            div { style: "display:flex;justify-content:space-between;align-items:center;margin-bottom:0.75rem;",
                                h2 {
                                    style: "font-family:'Rajdhani',sans-serif;font-size:1.1rem;font-weight:700;color:var(--text-bright);text-transform:uppercase;letter-spacing:0.04em;margin:0;",
                                    "Participants"
                                }
                                button {
                                    class: "btn-add",
                                    onclick: on_add_part_open,
                                    "+ Add"
                                }
                            }
                            {
                                match bracket.as_ref().map(|b| &b.participants) {
                                    Some(parts) if !parts.is_empty() => rsx! {
                                        DataTable { headers: vec!["Seed", "Name", "Actions"],
                                            for p in parts.iter() {
                                                {
                                                    let p_rm = p.clone();
                                                    rsx! {
                                                        tr { key: "{p.id}",
                                                            td { "#{p.seed}" }
                                                            td { "{p.name}" }
                                                            td {
                                                                button {
                                                                    class: "row-btn danger",
                                                                    onclick: move |_| open_remove_part(p_rm.clone()),
                                                                    "Remove"
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    _ => rsx! {
                                        p { class: "empty-state", "No participants yet." }
                                    },
                                }
                            }
                        }

                        // Rounds & matches section
                        div {
                            h2 {
                                style: "font-family:'Rajdhani',sans-serif;font-size:1.1rem;font-weight:700;color:var(--text-bright);text-transform:uppercase;letter-spacing:0.04em;margin-bottom:0.75rem;",
                                "Bracket"
                            }
                            {
                                let rounds = bracket.as_ref().map(|b| &b.rounds);
                                let matches = bracket.as_ref().map(|b| &b.matches);
                                match (rounds, matches) {
                                    (Some(rnds), Some(mtchs)) if !rnds.is_empty() => {
                                        rsx! {
                                            for round in rnds.iter() {
                                                {
                                                    let round_matches: Vec<BracketMatch> = mtchs
                                                        .iter()
                                                        .filter(|m| m.round_id == round.id)
                                                        .cloned()
                                                        .collect();
                                                    let pmap = participant_map.clone();
                                                    rsx! {
                                                        div { style: "margin-bottom:1.5rem;",
                                                            h3 {
                                                                style: "font-family:'Rajdhani',sans-serif;font-size:0.9rem;font-weight:600;color:var(--text-secondary);text-transform:uppercase;margin-bottom:0.5rem;",
                                                                "Round {round.round_number} \u{2014} {round.stage}"
                                                            }
                                                            DataTable { headers: vec!["#", "Player A", "Score", "Player B", "Status", "Actions"],
                                                                for bm in round_matches.iter() {
                                                                    {
                                                                        let a_name = bm.participant_a_id.as_ref()
                                                                            .and_then(|id| pmap.get(id))
                                                                            .cloned()
                                                                            .unwrap_or_else(|| "TBD".into());
                                                                        let b_name = bm.participant_b_id.as_ref()
                                                                            .and_then(|id| pmap.get(id))
                                                                            .cloned()
                                                                            .unwrap_or_else(|| "TBD".into());
                                                                        let score_display = match (bm.score_a, bm.score_b) {
                                                                            (Some(a), Some(b)) => format!("{a} - {b}"),
                                                                            _ => "\u{2014}".to_string(),
                                                                        };
                                                                        let bm_report = bm.clone();
                                                                        rsx! {
                                                                            tr { key: "{bm.id}",
                                                                                td { "{bm.bracket_position}" }
                                                                                td {
                                                                                    span {
                                                                                        style: if bm.winner_id == bm.participant_a_id && bm.winner_id.is_some() { "color:var(--accent);font-weight:700;" } else { "" },
                                                                                        "{a_name}"
                                                                                    }
                                                                                }
                                                                                td { "{score_display}" }
                                                                                td {
                                                                                    span {
                                                                                        style: if bm.winner_id == bm.participant_b_id && bm.winner_id.is_some() { "color:var(--accent);font-weight:700;" } else { "" },
                                                                                        "{b_name}"
                                                                                    }
                                                                                }
                                                                                td { StatusPill { status: bm.status.clone() } }
                                                                                td {
                                                                                    button {
                                                                                        class: "row-btn primary",
                                                                                        onclick: move |_| open_match_report(bm_report.clone()),
                                                                                        "Report"
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
                                    },
                                    _ => rsx! {
                                        p { class: "empty-state", "No bracket generated yet." }
                                    },
                                }
                            }
                        }
                    }
                } else {
                    rsx! {
                        p { class: "empty-state", "Tournament not found." }
                        button { class: "row-btn", onclick: back_to_list, "\u{2190} Back to List" }
                    }
                }
            }
        } else {
            // ==================== LIST VIEW ====================

            div { class: "admin-toolbar",
                h1 { "Tournaments" }
                div { style: "display:flex;gap:0.75rem;align-items:center;",
                    select {
                        class: "form-select",
                        value: "{status_filter}",
                        onchange: move |e| status_filter.set(e.value()),
                        for s in STATUS_FILTERS.iter() {
                            option { value: "{s}", "{s}" }
                        }
                    }
                    button { class: "btn-add", onclick: open_create, "+ Add Tournament" }
                }
            }

            {
                let data = tournaments.data.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                let filter = status_filter();
                match data {
                    None => rsx! { p { class: "admin-loading", "Loading..." } },
                    Some(list) => {
                        let filtered: Vec<&Tournament> = list
                            .iter()
                            .filter(|t| filter == "all" || t.status == filter)
                            .collect();
                        if filtered.is_empty() {
                            rsx! { p { class: "empty-state", "No tournaments found." } }
                        } else {
                            rsx! {
                                DataTable { headers: vec!["Name", "Format", "Game", "Status", "Max", "Starts", "Actions"],
                                    for t in filtered.iter() {
                                        {
                                            let t_edit = (*t).clone();
                                            let t_del = (*t).clone();
                                            let t_view_id = t.id.clone();
                                            let game_display = t.game_name.clone().unwrap_or_else(|| "\u{2014}".into());
                                            let max_display = t.max_participants.map(|n| n.to_string()).unwrap_or_else(|| "\u{2014}".into());
                                            let starts_display = t.starts_at.clone().unwrap_or_else(|| "\u{2014}".into());
                                            let tid_reg = t.id.clone();
                                            let tid_active = t.id.clone();
                                            let tid_complete = t.id.clone();
                                            let t_status = t.status.clone();
                                            rsx! {
                                                tr { key: "{t.id}",
                                                    td { "{t.name}" }
                                                    td { "{t.format}" }
                                                    td { "{game_display}" }
                                                    td { StatusPill { status: t.status.clone() } }
                                                    td { "{max_display}" }
                                                    td { "{starts_display}" }
                                                    td {
                                                        div { class: "row-actions",
                                                            button {
                                                                class: "row-btn",
                                                                onclick: move |_| open_edit(t_edit.clone()),
                                                                "Edit"
                                                            }
                                                            button {
                                                                class: "row-btn primary",
                                                                onclick: move |_| view_detail(t_view_id.clone()),
                                                                "View"
                                                            }
                                                            button {
                                                                class: "row-btn danger",
                                                                onclick: move |_| open_delete(t_del.clone()),
                                                                "Delete"
                                                            }
                                                            if t_status == "draft" {
                                                                button {
                                                                    class: "row-btn",
                                                                    onclick: move |_| change_status((tid_reg.clone(), "registration".to_string())),
                                                                    "\u{2192} Registration"
                                                                }
                                                            }
                                                            if t_status == "registration" {
                                                                button {
                                                                    class: "row-btn",
                                                                    onclick: move |_| change_status((tid_active.clone(), "active".to_string())),
                                                                    "\u{2192} Active"
                                                                }
                                                            }
                                                            if t_status == "active" {
                                                                button {
                                                                    class: "row-btn",
                                                                    onclick: move |_| change_status((tid_complete.clone(), "completed".to_string())),
                                                                    "\u{2192} Completed"
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
                    },
                }
            }
        }

        // --- Shared modals (always rendered) ---

        // Create/Edit tournament modal
        FormModal {
            title: if modal.get_target().is_some() { "Edit Tournament".to_string() } else { "Add Tournament".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,
            wide: true,

            div { class: "form-grid",
                div { class: "form-field span-full",
                    label { class: "form-label", "Name" }
                    input {
                        class: "form-input",
                        r#type: "text",
                        value: "{form_name}",
                        oninput: move |e| form_name.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Format" }
                    select {
                        class: "form-select",
                        value: "{form_format}",
                        onchange: move |e| form_format.set(e.value()),
                        for f in FORMATS.iter() {
                            option { value: "{f}", "{f}" }
                        }
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Game" }
                    select {
                        class: "form-select",
                        value: form_game_id().unwrap_or_default(),
                        onchange: move |e| {
                            let v = e.value();
                            form_game_id.set(if v.is_empty() { None } else { Some(v) });
                        },
                        option { value: "", "\u{2014} None \u{2014}" }
                        {
                            let g = games.data.read();
                            let g = g.as_ref().and_then(|d| d.as_ref());
                            match g {
                                Some(list) => rsx! {
                                    for game in list.iter() {
                                        option { value: "{game.id}", "{game.name}" }
                                    }
                                },
                                None => rsx! {},
                            }
                        }
                    }
                }
                div { class: "form-field span-full",
                    label { class: "form-label", "Max Participants" }
                    input {
                        class: "form-input",
                        r#type: "number",
                        placeholder: "e.g. 16",
                        value: "{form_max}",
                        oninput: move |e| form_max.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Date" }
                    input {
                        class: "form-input",
                        r#type: "date",
                        onchange: move |e| form_date.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Time" }
                    input {
                        class: "form-input",
                        r#type: "time",
                        onchange: move |e| form_time.set(e.value()),
                    }
                }
            }
        }

        // Delete confirm
        ConfirmDialog {
            title: "Delete Tournament".to_string(),
            message: format!(
                "Are you sure you want to delete \"{}\"? All bracket data will be lost.",
                delete_modal.get_target().map(|t| t.name).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            on_confirm: on_delete_confirm,
            on_cancel: on_delete_cancel,
        }

        // Add participant modal
        FormModal {
            title: "Add Participant".to_string(),
            open: add_part_modal.is_open(),
            submitting: add_part_modal.is_submitting(),
            on_close: on_add_part_close,
            on_submit: on_add_part_submit,

            div { class: "form-field",
                label { class: "form-label", "Member" }
                select {
                    class: "form-select",
                    value: "{add_part_member_id}",
                    onchange: move |e| add_part_member_id.set(e.value()),
                    option { value: "", "-- Select Member --" }
                    {
                        let m = members.data.read();
                        let m = m.as_ref().and_then(|d| d.as_ref());
                        match m {
                            Some(list) => rsx! {
                                for mem in list.iter() {
                                    option { value: "{mem.id}", "{mem.display_name}" }
                                }
                            },
                            None => rsx! {},
                        }
                    }
                }
            }
        }

        // Remove participant confirm
        ConfirmDialog {
            title: "Remove Participant".to_string(),
            message: format!(
                "Remove \"{}\" from this tournament?",
                remove_part_modal.get_target().map(|p| p.name).unwrap_or_default()
            ),
            open: remove_part_modal.is_open(),
            danger: true,
            on_confirm: on_remove_part_confirm,
            on_cancel: on_remove_part_cancel,
        }

        // Match report modal
        FormModal {
            title: "Report Match Result".to_string(),
            open: match_modal.is_open(),
            submitting: match_modal.is_submitting(),
            on_close: on_match_close,
            on_submit: on_match_submit,

            div { class: "form-field",
                label { class: "form-label", "Score A" }
                input {
                    class: "form-input",
                    r#type: "number",
                    value: "{match_score_a}",
                    oninput: move |e| match_score_a.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Score B" }
                input {
                    class: "form-input",
                    r#type: "number",
                    value: "{match_score_b}",
                    oninput: move |e| match_score_b.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Winner" }
                select {
                    class: "form-select",
                    value: "{match_winner}",
                    onchange: move |e| match_winner.set(e.value()),
                    option { value: "", "-- Select Winner --" }
                    {
                        if let Some(m) = match_modal.get_target() {
                            let bd = bracket_data();
                            let pmap: HashMap<String, String> = bd
                                .as_ref()
                                .map(|b| b.participants.iter().map(|p| (p.id.clone(), p.name.clone())).collect())
                                .unwrap_or_default();
                            let a_id = m.participant_a_id.clone().unwrap_or_default();
                            let b_id = m.participant_b_id.clone().unwrap_or_default();
                            let a_name = pmap.get(&a_id).cloned().unwrap_or_else(|| "Player A".into());
                            let b_name = pmap.get(&b_id).cloned().unwrap_or_else(|| "Player B".into());
                            rsx! {
                                if !a_id.is_empty() {
                                    option { value: "{a_id}", "{a_name}" }
                                }
                                if !b_id.is_empty() {
                                    option { value: "{b_id}", "{b_name}" }
                                }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Replay Codes (one per line)" }
                textarea {
                    class: "form-textarea",
                    value: "{match_replays}",
                    oninput: move |e| match_replays.set(e.value()),
                }
            }
        }
    }
}
