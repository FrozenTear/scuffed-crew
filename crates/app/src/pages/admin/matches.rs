use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::MatchPayload;
use crate::components::{DataTable, FormModal, Toast, use_toast};
use crate::hooks::{use_api_list, use_api_list_with, ModalController};

// Local response types with String-typed fields for display.
#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MatchResult {
    id: String,
    opponent: String,
    score_us: u32,
    score_them: u32,
    map_name: Option<String>,
    match_type: String,
    played_at: String,
    notes: Option<String>,
}

#[component]
pub fn AdminMatches() -> Element {
    let teams = use_api_list::<Team>("/api/teams");
    let mut toast = use_toast();

    // Team selector
    let mut selected_team = use_signal(|| None::<String>);

    let mut matches = use_api_list_with::<MatchResult>(move || {
        match selected_team() {
            Some(id) => format!("/api/teams/{id}/matches"),
            None => String::new(),
        }
    });

    // Form modal state
    let mut modal = ModalController::<String>::new();

    // Form fields
    let mut f_opponent = use_signal(String::new);
    let mut f_score_us = use_signal(|| "0".to_string());
    let mut f_score_them = use_signal(|| "0".to_string());
    let mut f_map_name = use_signal(String::new);
    let mut f_match_type = use_signal(|| "scrim".to_string());
    let mut f_played_at = use_signal(String::new);
    let mut f_notes = use_signal(String::new);

    let open_create = move |_| {
        f_opponent.set(String::new());
        f_score_us.set("0".to_string());
        f_score_them.set("0".to_string());
        f_map_name.set(String::new());
        f_match_type.set("scrim".to_string());
        f_played_at.set(String::new());
        f_notes.set(String::new());
        modal.show_empty();
    };

    let mut open_edit = move |m: MatchResult| {
        f_opponent.set(m.opponent);
        f_score_us.set(m.score_us.to_string());
        f_score_them.set(m.score_them.to_string());
        f_map_name.set(m.map_name.unwrap_or_default());
        f_match_type.set(m.match_type);
        f_played_at.set(m.played_at.chars().take(10).collect::<String>());
        f_notes.set(m.notes.unwrap_or_default());
        modal.show(m.id);
    };

    let on_submit = move |_| {
        let team_id = match selected_team() {
            Some(id) => id,
            None => return,
        };
        let edit_id = modal.get_target();
        let opponent = f_opponent().clone();
        let score_us: u32 = f_score_us().parse().unwrap_or(0);
        let score_them: u32 = f_score_them().parse().unwrap_or(0);
        let map_name_val = f_map_name().clone();
        let match_type_val = f_match_type().clone();
        let played_at_val = f_played_at().clone();
        let notes_val = f_notes().clone();

        modal.start_submit();
        spawn(async move {
            let payload = MatchPayload {
                team_id,
                opponent,
                score_us,
                score_them,
                map_name: if map_name_val.is_empty() { None } else { Some(map_name_val) },
                match_type: match_type_val,
                played_at: played_at_val,
                notes: if notes_val.is_empty() { None } else { Some(notes_val) },
            };

            let result = match edit_id {
                Some(id) => {
                    let path = format!("/api/matches/{id}");
                    ApiClient::web().put_json::<_, MatchResult>(&path, &payload).await
                }
                None => {
                    ApiClient::web().post_json::<_, MatchResult>("/api/matches", &payload).await
                }
            };

            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Match saved"));
                    modal.close();
                    matches.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to save: {e}"))),
            }
        });
    };

    let modal_title = if modal.get_target().is_some() { "Edit Match" } else { "New Match" };

    rsx! {

        h1 { "Matches" }

        div { class: "admin-toolbar",
            select {
                value: "{selected_team().unwrap_or_default()}",
                onchange: move |e| {
                    let val = e.value();
                    selected_team.set(if val.is_empty() { None } else { Some(val) });
                },
                option { value: "", "-- Select Team --" }
                {
                    let data = teams.data.read();
                    let data = data.as_ref().and_then(|d| d.as_ref());
                    match data {
                        Some(list) => rsx! {
                            for t in list.iter() {
                                option { value: "{t.id}", "{t.name}" }
                            }
                        },
                        None => rsx! {},
                    }
                }
            }
            if selected_team().is_some() {
                button { class: "btn-add", onclick: open_create, "+ New Match" }
            }
        }

        if selected_team().is_none() {
            p { class: "empty-state", "Select a team to view matches." }
        } else {
            {
                let data = matches.data.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "admin-loading", "Loading..." } },
                    Some(list) if list.is_empty() => rsx! {
                        p { class: "empty-state", "No matches recorded for this team." }
                    },
                    Some(list) => rsx! {
                        DataTable { headers: vec!["Opponent", "Score", "Map", "Type", "Date", "Actions"],
                            for m in list.iter() {
                                {
                                    let mc = m.clone();
                                    let score = format!("{}-{}", m.score_us, m.score_them);
                                    let map = m.map_name.clone().unwrap_or("-".to_string());
                                    let date: String = m.played_at.chars().take(10).collect();
                                    rsx! {
                                        tr { key: "{m.id}",
                                            td { "{m.opponent}" }
                                            td { "{score}" }
                                            td { "{map}" }
                                            td { "{m.match_type}" }
                                            td { "{date}" }
                                            td {
                                                div { class: "row-actions",
                                                    button {
                                                        class: "row-btn",
                                                        onclick: move |_| open_edit(mc.clone()),
                                                        "Edit"
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

        FormModal {
            title: modal_title.to_string(),
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: move |_| modal.close(),
            on_submit: on_submit,

            div { class: "form-field",
                label { class: "form-label", "Opponent" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{f_opponent}",
                    oninput: move |e| f_opponent.set(e.value()),
                }
            }
            div { style: "display:flex; gap:1rem;",
                div { class: "form-field", style: "flex:1;",
                    label { class: "form-label", "Our Score" }
                    input {
                        class: "form-input",
                        r#type: "number",
                        value: "{f_score_us}",
                        oninput: move |e| f_score_us.set(e.value()),
                    }
                }
                div { class: "form-field", style: "flex:1;",
                    label { class: "form-label", "Their Score" }
                    input {
                        class: "form-input",
                        r#type: "number",
                        value: "{f_score_them}",
                        oninput: move |e| f_score_them.set(e.value()),
                    }
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Map (optional)" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{f_map_name}",
                    oninput: move |e| f_map_name.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Type" }
                select {
                    class: "form-select",
                    value: "{f_match_type}",
                    onchange: move |e| f_match_type.set(e.value()),
                    option { value: "scrim", "Scrim" }
                    option { value: "ranked", "Ranked" }
                    option { value: "tournament", "Tournament" }
                    option { value: "casual", "Casual" }
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Played At" }
                input {
                    class: "form-input",
                    r#type: "date",
                    value: "{f_played_at}",
                    oninput: move |e| f_played_at.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Notes (optional)" }
                textarea {
                    class: "form-textarea",
                    value: "{f_notes}",
                    oninput: move |e| f_notes.set(e.value()),
                }
            }
        }
    }
}
