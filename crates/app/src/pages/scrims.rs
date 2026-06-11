use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use scuffed_api_client::ApiClient;

use crate::components::{Toast, use_toast};
use crate::hooks::use_api_list;
use crate::state::use_auth;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Scrim {
    id: String,
    team_id: String,
    game_id: String,
    #[allow(dead_code)]
    requested_by: String,
    opponent_name: Option<String>,
    scheduled_at: String,
    duration_minutes: u32,
    status: String,
    notes: Option<String>,
    #[allow(dead_code)]
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Team {
    id: String,
    name: String,
    #[allow(dead_code)]
    game_id: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Game {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct CreateScrimBody {
    team_id: String,
    game_id: String,
    scheduled_at: String,
    duration_minutes: u32,
    notes: Option<String>,
}

#[derive(Serialize)]
struct UpdateScrimBody {
    status: String,
    opponent_name: Option<String>,
}

const PAGE_CSS: &str = r#"
.scrims-page {
    padding: 3rem 2rem;
    max-width: 900px;
    margin: 0 auto;
}
.scrims-page-title {
    font-family: var(--font-head);
    font-size: 2.5rem;
    color: var(--text);
    letter-spacing: 3px;
    margin: 0 0 0.25rem;
}
.scrims-subtitle {
    color: var(--text-2);
    font-size: 0.9rem;
    margin: 0 0 2rem;
}
.scrims-section-title {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 1.2rem;
    color: var(--text);
    margin: 2rem 0 0.75rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
}
.scrims-section-title .count {
    background: var(--surface-2);
    color: var(--text-3);
    font-size: 0.7rem;
    padding: 0.1rem 0.5rem;
    border-radius: var(--radius-pill);
}
.scrims-list {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
}
.scrim-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.25rem 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
}
.scrim-card.open { border-left: 3px solid var(--ok); }
.scrim-card.confirmed { border-left: 3px solid var(--accent); }
.scrim-card.completed { border-left: 3px solid var(--text-3); }
.scrim-card.cancelled { border-left: 3px solid var(--danger); opacity: 0.6; }
.scrim-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
}
.scrim-team {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 1.05rem;
    color: var(--text);
}
.scrim-status {
    display: inline-block;
    padding: 0.15rem 0.6rem;
    border-radius: var(--radius-pill);
    font-size: 0.65rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
}
.scrim-status.open {
    background: color-mix(in srgb, var(--ok) 15%, transparent);
    color: var(--ok);
}
.scrim-status.confirmed {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
}
.scrim-status.completed {
    background: color-mix(in srgb, var(--text-2) 15%, transparent);
    color: var(--text-2);
}
.scrim-status.cancelled {
    background: color-mix(in srgb, var(--danger) 15%, transparent);
    color: var(--danger);
}
.scrim-details {
    display: flex;
    gap: 1.25rem;
    font-size: 0.8rem;
    color: var(--text-2);
    flex-wrap: wrap;
}
.scrim-details span { display: flex; align-items: center; gap: 0.25rem; }
.scrim-notes {
    font-size: 0.8rem;
    color: var(--text-3);
    font-style: italic;
}
.scrim-opponent {
    font-size: 0.8rem;
    color: var(--accent-soft);
}
.scrim-actions {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.25rem;
}
.scrim-btn {
    padding: 0.3rem 0.75rem;
    border-radius: 6px;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-2);
    cursor: pointer;
    transition: all 0.15s;
}
.scrim-btn:hover { border-color: var(--accent-soft); color: var(--text); }
.scrim-btn.confirm {
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--accent);
}
.scrim-btn.confirm:hover {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
}
.scrim-btn.cancel {
    border-color: color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--danger);
}
.scrim-btn.cancel:hover {
    background: color-mix(in srgb, var(--danger) 12%, transparent);
}
.scrim-btn.complete {
    border-color: color-mix(in srgb, var(--ok) 40%, transparent);
    color: var(--ok);
}
.scrim-btn.complete:hover {
    background: color-mix(in srgb, var(--ok) 12%, transparent);
}
.scrims-loading, .scrims-empty {
    color: var(--text-3);
    text-align: center;
    padding: 3rem 0;
}
.scrim-create-form {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.5rem;
    margin-bottom: 1.5rem;
}
.scrim-create-title {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 1.1rem;
    color: var(--text);
    margin: 0 0 1rem;
}
.scrim-form-row {
    display: flex;
    gap: 0.75rem;
    margin-bottom: 0.75rem;
    flex-wrap: wrap;
}
.scrim-form-field {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    flex: 1;
    min-width: 140px;
}
.scrim-form-label {
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-3);
}
.scrim-form-input, .scrim-form-select, .scrim-form-textarea {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 0.5rem 0.75rem;
    font-size: 0.85rem;
    font-family: inherit;
}
.scrim-form-input:focus, .scrim-form-select:focus, .scrim-form-textarea:focus {
    outline: none;
    border-color: var(--accent);
}
.scrim-form-textarea { resize: vertical; min-height: 60px; }
.scrim-form-submit {
    padding: 0.5rem 1.25rem;
    border-radius: 6px;
    font-size: 0.85rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    border: none;
    background: var(--accent);
    color: var(--accent-fg);
    cursor: pointer;
    transition: all 0.15s;
}
.scrim-form-submit:hover { filter: brightness(1.15); box-shadow: 0 0 20px var(--accent-soft); }
.scrim-form-submit:disabled { opacity: 0.5; cursor: not-allowed; }
@media (max-width: 768px) {
    .scrims-page { padding: 2rem 1rem; }
    .scrim-form-row { flex-direction: column; }
    .scrim-header { flex-direction: column; align-items: flex-start; }
}
"#;

#[component]
pub fn Scrims() -> Element {
    let auth = use_auth();
    let mut scrims = use_api_list::<Scrim>("/api/scrims");
    let teams = use_api_list::<Team>("/api/teams");
    let games = use_api_list::<Game>("/api/games");

    let is_logged_in = auth().is_logged_in();

    let on_change = move |_: ()| {
        scrims.refresh += 1;
    };

    // Split scrims into sections
    let scrim_data = scrims.data.read();
    let scrim_list = scrim_data.as_ref().and_then(|d| d.as_ref());

    let (open, confirmed, past) = match scrim_list {
        Some(list) => {
            let mut open = Vec::new();
            let mut confirmed = Vec::new();
            let mut past = Vec::new();
            for s in list.iter() {
                match s.status.as_str() {
                    "open" => open.push(s.clone()),
                    "confirmed" => confirmed.push(s.clone()),
                    "completed" | "cancelled" => past.push(s.clone()),
                    _ => open.push(s.clone()),
                }
            }
            (Some(open), Some(confirmed), Some(past))
        }
        None => (None, None, None),
    };

    let teams_data = teams.data.read();
    let teams_list = teams_data.as_ref().and_then(|d| d.as_ref());
    let games_data = games.data.read();
    let games_list = games_data.as_ref().and_then(|d| d.as_ref());

    rsx! {
        style { {PAGE_CSS} }

        main { class: "scrims-page",
            h1 { class: "scrims-page-title", "Scrim Board" }
            p { class: "scrims-subtitle", "Request, schedule, and track practice matches." }

            if is_logged_in {
                if let (Some(t_list), Some(g_list)) = (teams_list, games_list) {
                    ScrimCreateForm {
                        teams: t_list.clone(),
                        games: g_list.clone(),
                        on_created: on_change,
                    }
                }
            }

            {match &open {
                None => rsx! { p { class: "scrims-loading", "Loading scrims..." } },
                Some(list) => {
                    let team_lookup = teams_list.cloned().unwrap_or_default();
                    let game_lookup = games_list.cloned().unwrap_or_default();
                    rsx! {
                        ScrimSection {
                            title: "Open",
                            scrims: list.clone(),
                            teams: team_lookup.clone(),
                            games: game_lookup.clone(),
                            show_actions: is_logged_in,
                            on_change: on_change,
                        }
                        ScrimSection {
                            title: "Upcoming",
                            scrims: confirmed.clone().unwrap_or_default(),
                            teams: team_lookup.clone(),
                            games: game_lookup.clone(),
                            show_actions: is_logged_in,
                            on_change: on_change,
                        }
                        ScrimSection {
                            title: "Past",
                            scrims: past.clone().unwrap_or_default(),
                            teams: team_lookup,
                            games: game_lookup,
                            show_actions: false,
                            on_change: on_change,
                        }
                    }
                }
            }}
        }
    }
}

#[component]
fn ScrimSection(
    title: String,
    scrims: Vec<Scrim>,
    teams: Vec<Team>,
    games: Vec<Game>,
    show_actions: bool,
    on_change: EventHandler<()>,
) -> Element {
    rsx! {
        h2 { class: "scrims-section-title",
            "{title}"
            span { class: "count", "{scrims.len()}" }
        }
        if scrims.is_empty() {
            p { class: "scrims-empty", "No {title.to_lowercase()} scrims." }
        } else {
            div { class: "scrims-list",
                for scrim in scrims.iter() {
                    ScrimCard {
                        key: "{scrim.id}",
                        scrim: scrim.clone(),
                        teams: teams.clone(),
                        games: games.clone(),
                        show_actions: show_actions,
                        on_change: on_change,
                    }
                }
            }
        }
    }
}

fn find_name<'a>(list: &'a [Team], id: &str) -> &'a str {
    list.iter()
        .find(|t| t.id == id)
        .map(|t| t.name.as_str())
        .unwrap_or("Unknown")
}

fn find_game_name<'a>(list: &'a [Game], id: &str) -> &'a str {
    list.iter()
        .find(|g| g.id == id)
        .map(|g| g.name.as_str())
        .unwrap_or("Unknown")
}

#[component]
fn ScrimCard(
    scrim: Scrim,
    teams: Vec<Team>,
    games: Vec<Game>,
    show_actions: bool,
    on_change: EventHandler<()>,
) -> Element {
    let status = scrim.status.clone();
    let card_class = format!("scrim-card {status}");
    let status_class = format!("scrim-status {status}");
    let team_name = find_name(&teams, &scrim.team_id).to_string();
    let game_name = find_game_name(&games, &scrim.game_id).to_string();
    let date: String = scrim.scheduled_at.chars().take(16).collect();
    let date = date.replace('T', " ");

    rsx! {
        div { class: "{card_class}",
            div { class: "scrim-header",
                span { class: "scrim-team", "{team_name}" }
                span { class: "{status_class}", "{status}" }
            }
            div { class: "scrim-details",
                span { "Game: {game_name}" }
                span { "When: {date}" }
                span { "Duration: {scrim.duration_minutes}m" }
            }
            if let Some(ref opp) = scrim.opponent_name {
                p { class: "scrim-opponent", "vs {opp}" }
            }
            if let Some(ref notes) = scrim.notes {
                p { class: "scrim-notes", "{notes}" }
            }
            if show_actions && status == "open" {
                div { class: "scrim-actions",
                    { let scrim_id = scrim.id.clone();
                      let on_change = on_change;
                      rsx! {
                        button {
                            class: "scrim-btn confirm",
                            onclick: move |_| {
                                let sid = scrim_id.clone();
                                let on_change = on_change;
                                spawn(async move {
                                    let mut toast = use_toast();
                                    let body = UpdateScrimBody {
                                        status: "confirmed".to_string(),
                                        opponent_name: None,
                                    };
                                    match ApiClient::web().patch_json::<_, serde_json::Value>(&format!("/api/scrims/{sid}"), &body).await {
                                        Ok(_) => {
                                            toast.show(Toast::success("Scrim confirmed"));
                                            on_change.call(());
                                        }
                                        Err(e) => toast.show(Toast::error(format!("Error: {e}"))),
                                    }
                                });
                            },
                            "Confirm"
                        }
                    }}
                    { let scrim_id = scrim.id.clone();
                      let on_change = on_change;
                      rsx! {
                        button {
                            class: "scrim-btn cancel",
                            onclick: move |_| {
                                let sid = scrim_id.clone();
                                let on_change = on_change;
                                spawn(async move {
                                    let mut toast = use_toast();
                                    let body = UpdateScrimBody {
                                        status: "cancelled".to_string(),
                                        opponent_name: None,
                                    };
                                    match ApiClient::web().patch_json::<_, serde_json::Value>(&format!("/api/scrims/{sid}"), &body).await {
                                        Ok(_) => {
                                            toast.show(Toast::success("Scrim cancelled"));
                                            on_change.call(());
                                        }
                                        Err(e) => toast.show(Toast::error(format!("Error: {e}"))),
                                    }
                                });
                            },
                            "Cancel"
                        }
                    }}
                }
            }
            if show_actions && status == "confirmed" {
                div { class: "scrim-actions",
                    { let scrim_id = scrim.id.clone();
                      let on_change = on_change;
                      rsx! {
                        button {
                            class: "scrim-btn complete",
                            onclick: move |_| {
                                let sid = scrim_id.clone();
                                let on_change = on_change;
                                spawn(async move {
                                    let mut toast = use_toast();
                                    let body = UpdateScrimBody {
                                        status: "completed".to_string(),
                                        opponent_name: None,
                                    };
                                    match ApiClient::web().patch_json::<_, serde_json::Value>(&format!("/api/scrims/{sid}"), &body).await {
                                        Ok(_) => {
                                            toast.show(Toast::success("Scrim completed"));
                                            on_change.call(());
                                        }
                                        Err(e) => toast.show(Toast::error(format!("Error: {e}"))),
                                    }
                                });
                            },
                            "Complete"
                        }
                    }}
                    { let scrim_id = scrim.id.clone();
                      let on_change = on_change;
                      rsx! {
                        button {
                            class: "scrim-btn cancel",
                            onclick: move |_| {
                                let sid = scrim_id.clone();
                                let on_change = on_change;
                                spawn(async move {
                                    let mut toast = use_toast();
                                    let body = UpdateScrimBody {
                                        status: "cancelled".to_string(),
                                        opponent_name: None,
                                    };
                                    match ApiClient::web().patch_json::<_, serde_json::Value>(&format!("/api/scrims/{sid}"), &body).await {
                                        Ok(_) => {
                                            toast.show(Toast::success("Scrim cancelled"));
                                            on_change.call(());
                                        }
                                        Err(e) => toast.show(Toast::error(format!("Error: {e}"))),
                                    }
                                });
                            },
                            "Cancel"
                        }
                    }}
                }
            }
        }
    }
}

#[component]
fn ScrimCreateForm(teams: Vec<Team>, games: Vec<Game>, on_created: EventHandler<()>) -> Element {
    let mut team_id = use_signal(String::new);
    let mut game_id = use_signal(String::new);
    let mut scheduled_at = use_signal(String::new);
    let mut duration = use_signal(|| "90".to_string());
    let mut notes = use_signal(String::new);
    let mut submitting = use_signal(|| false);

    rsx! {
        div { class: "scrim-create-form",
            h3 { class: "scrim-create-title", "Request a Scrim" }
            div { class: "scrim-form-row",
                div { class: "scrim-form-field",
                    label { class: "scrim-form-label", "Team" }
                    select {
                        class: "scrim-form-select",
                        value: "{team_id}",
                        onchange: move |e| team_id.set(e.value()),
                        option { value: "", "Select team..." }
                        for t in teams.iter() {
                            option { value: "{t.id}", "{t.name}" }
                        }
                    }
                }
                div { class: "scrim-form-field",
                    label { class: "scrim-form-label", "Game" }
                    select {
                        class: "scrim-form-select",
                        value: "{game_id}",
                        onchange: move |e| game_id.set(e.value()),
                        option { value: "", "Select game..." }
                        for g in games.iter() {
                            option { value: "{g.id}", "{g.name}" }
                        }
                    }
                }
            }
            div { class: "scrim-form-row",
                div { class: "scrim-form-field",
                    label { class: "scrim-form-label", "Scheduled Date & Time" }
                    input {
                        class: "scrim-form-input",
                        r#type: "datetime-local",
                        value: "{scheduled_at}",
                        onchange: move |e| scheduled_at.set(e.value()),
                    }
                }
                div { class: "scrim-form-field",
                    label { class: "scrim-form-label", "Duration (min)" }
                    input {
                        class: "scrim-form-input",
                        r#type: "number",
                        value: "{duration}",
                        min: "15",
                        max: "480",
                        onchange: move |e| duration.set(e.value()),
                    }
                }
            }
            div { class: "scrim-form-row",
                div { class: "scrim-form-field",
                    label { class: "scrim-form-label", "Notes (optional)" }
                    textarea {
                        class: "scrim-form-textarea",
                        placeholder: "Any details about the scrim...",
                        value: "{notes}",
                        onchange: move |e| notes.set(e.value()),
                    }
                }
            }
            button {
                class: "scrim-form-submit",
                disabled: submitting(),
                onclick: move |_| {
                    let tid = team_id().clone();
                    let gid = game_id().clone();
                    let sched = scheduled_at().clone();
                    let dur_str = duration().clone();
                    let n = notes().clone();
                    let on_created = on_created;

                    if tid.is_empty() || gid.is_empty() || sched.is_empty() {
                        let mut toast = use_toast();
                        toast.show(Toast::error("Please fill in team, game, and date/time"));
                        return;
                    }

                    submitting.set(true);
                    spawn(async move {
                        let mut toast = use_toast();
                        let dur: u32 = dur_str.parse().unwrap_or(90);
                        let scheduled_iso = format!("{sched}:00Z");
                        let body = CreateScrimBody {
                            team_id: tid,
                            game_id: gid,
                            scheduled_at: scheduled_iso,
                            duration_minutes: dur,
                            notes: if n.is_empty() { None } else { Some(n) },
                        };
                        match ApiClient::web().post_json::<_, serde_json::Value>("/api/scrims", &body).await {
                            Ok(_) => {
                                toast.show(Toast::success("Scrim requested"));
                                on_created.call(());
                            }
                            Err(e) => toast.show(Toast::error(format!("Error: {e}"))),
                        }
                        submitting.set(false);
                    });
                },
                if submitting() { "Creating..." } else { "Request Scrim" }
            }
        }
    }
}
