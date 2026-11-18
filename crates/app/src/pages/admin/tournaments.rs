use dioxus::prelude::*;

use crate::components::admin_shared::{
    after_delete_focus_ids, focus_after_delete_soon, focus_after_refresh_soon,
};
use serde::Deserialize;
use std::collections::HashMap;

use crate::components::ui::{
    ChipButton, ChipRow, EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pager, Pill,
    PillTone, SkeletonBars, page_slice, show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, AdminDate, ConfirmDialog, DataTable, FormModal, RowActions, SaveState,
    StatusPill, Toast, admin_list_pending, list_cap_notice, load_error_text, status_label,
    use_toast,
};
use crate::hooks::{ModalController, use_api_list};
use scuffed_api_client::ApiClient;
use scuffed_types::api::{
    AddParticipantRequest, CreateTournamentRequest, MatchReportRequest, StatusChangeRequest,
};

// --- Types ---
// Local response types with simplified/string-typed fields for display.

#[derive(Debug, Clone, Deserialize)]
struct Tournament {
    id: String,
    name: String,
    format: String,
    /// API may send game_id; game_name is optional enrichment.
    #[serde(default)]
    game_name: Option<String>,
    #[serde(default)]
    game_id: Option<String>,
    status: String,
    /// Server field is max_teams.
    #[serde(default, alias = "max_teams")]
    max_participants: Option<u32>,
    starts_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
}

/// `game:abc` and `abc` are the same record.
fn bare_game_id(id: &str) -> &str {
    id.trim().strip_prefix("game:").unwrap_or(id.trim())
}

/// The tournament's game as a word: the API name, else the name of its
/// `game_id` in the games list. "No game" only when it has none.
fn tournament_game(t: &Tournament, games: &[Game]) -> String {
    if let Some(name) = t
        .game_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return name.to_string();
    }
    match t
        .game_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        None => "No game".to_string(),
        Some(id) => games
            .iter()
            .find(|g| bare_game_id(&g.id) == bare_game_id(id))
            .map(|g| g.name.clone())
            .unwrap_or_else(|| "Unknown game".to_string()),
    }
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
/// Must match server `TournamentStatus` (no "active"; use `in_progress`).
const STATUS_FILTERS: [&str; 6] = [
    "all",
    "draft",
    "registration",
    "in_progress",
    "completed",
    "archived",
];

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-tournaments-list";
const NEW_TOURNAMENT_ID: &str = "tournaments-new";
const BACK_ID: &str = "tournament-back";
const ADD_PARTICIPANT_ID: &str = "tournament-add-participant";
const NAME_ID: &str = "tournament-name";
const NAME_ERROR_ID: &str = "tournament-name-error";
const FORMAT_ID: &str = "tournament-format";
const GAME_ID: &str = "tournament-game";
const MAX_ID: &str = "tournament-max";
const DATE_ID: &str = "tournament-date";
const TIME_ID: &str = "tournament-time";
const TIME_HINT_ID: &str = "tournament-time-hint";
const PARTICIPANT_ID: &str = "participant-member";
const PARTICIPANT_ERROR_ID: &str = "participant-member-error";
const SCORE_A_ID: &str = "report-score-a";
const SCORE_B_ID: &str = "report-score-b";
const SCORES_ERROR_ID: &str = "report-scores-error";
const WINNER_ID: &str = "report-winner";
const WINNER_ERROR_ID: &str = "report-winner-error";
const REPLAYS_ID: &str = "report-replays";

/// Page CSS: the head row of a detail block and the round headings.
const PAGE_CSS: &str = r#"
.admin-tournaments-page .tournament-block-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
}
.admin-tournaments-page .tournament-round {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
}
.admin-tournaments-page .tournament-round + .tournament-round { margin-top: var(--space-5); }
.admin-tournaments-page .tournament-round h3 {
    margin: 0;
    color: var(--text-3);
    font-family: var(--font-head);
    font-size: var(--text-sm);
    font-weight: var(--font-label-weight);
    letter-spacing: var(--tracking-caps);
    text-transform: uppercase;
    line-height: var(--leading-ui);
}
.admin-tournaments-page .tournament-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-3) 0 0;
    color: var(--text-3);
    font-size: var(--text-md);
}
.admin-tournaments-page .bracket-side {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
}
.admin-tournaments-page .bracket-side.is-winner { font-weight: var(--font-weight-700); }
"#;

/// The word a format reads as. The stored value stays the option value.
fn format_label(format: &str) -> String {
    match format {
        "single_elim" => "Single elimination".to_string(),
        "double_elim" => "Double elimination".to_string(),
        "round_robin" => "Round robin".to_string(),
        "swiss" => "Swiss".to_string(),
        other => status_label(other),
    }
}

/// The chip word of a status filter.
fn filter_label(status: &str) -> String {
    if status == "all" {
        "All".to_string()
    } else {
        status_label(status)
    }
}

/// The tournaments a status chip keeps. "all" keeps all.
fn filter_tournaments(list: &[Tournament], status: &str) -> Vec<Tournament> {
    list.iter()
        .filter(|t| status == "all" || t.status == status)
        .cloned()
        .collect()
}

/// The next status of a tournament and the button word that moves it there
/// (server: draft, registration, in_progress, completed, archived).
fn next_status(status: &str) -> Option<(&'static str, &'static str)> {
    match status {
        "draft" => Some(("registration", "Open registration")),
        "registration" => Some(("in_progress", "Start tournament")),
        "in_progress" => Some(("completed", "Complete")),
        "completed" => Some(("archived", "Archive")),
        _ => None,
    }
}

/// Id of a row's status cell. A status change from the list waits for it to
/// show the new status, or to leave with its row, before moving focus.
fn status_cell_id(tournament_id: &str) -> String {
    format!("tournament-status-{tournament_id}")
}

/// Id of a status chip, from its filter value.
fn tournament_filter_id(filter: &str) -> String {
    format!("tournament-filter-{filter}")
}

/// Where focus goes after a status change from the list, once the list has
/// refreshed: the row's own View if the row stays (Complete removes its own
/// button on desktop), else the next row's View, else the previous row's
/// (a status filter moves the row out), else the active status chip.
fn after_status_focus_ids(keys: &[String], changed: &str, filter: &str) -> Vec<String> {
    let view = |k: &str| vec![view_button_id(k)];
    let chip = tournament_filter_id(filter);
    let mut ids = view(changed);
    ids.extend(after_delete_focus_ids(keys, changed, view, &[&chip]));
    ids
}

/// Id of a row's View button, from the tournament id. Dialogs opened from
/// the row return focus here, because it is on screen at every width.
fn view_button_id(tournament_id: &str) -> String {
    format!("tournament-view-{tournament_id}")
}

/// Id of a participant's Remove button, from the participant id.
fn remove_participant_id(participant_id: &str) -> String {
    format!("participant-remove-{participant_id}")
}

/// Id of a bracket match's Report button, from the match id.
fn report_button_id(match_id: &str) -> String {
    format!("bracket-report-{match_id}")
}

/// `starts_at` split for the date and time inputs:
/// "2026-03-11T18:00:00Z" becomes ("2026-03-11", "18:00").
fn split_starts_at(raw: &str) -> (String, String) {
    match raw.split_once('T') {
        Some((d, rest)) => (
            d.to_string(),
            rest.trim_end_matches('Z').chars().take(5).collect(),
        ),
        None => (String::new(), String::new()),
    }
}

#[component]
pub fn AdminTournaments() -> Element {
    let mut tournaments = use_api_list::<Tournament>("/api/tournaments");
    let mut games = use_api_list::<Game>("/api/games");
    let mut members = use_api_list::<Member>("/api/members");
    let mut toast = use_toast();

    // View toggle: None = list, Some(id) = detail
    let mut detail_id: Signal<Option<String>> = use_signal(|| None);

    // Status filter and page of the list. Both live in memory.
    let mut status_filter = use_signal(|| "all".to_string());
    let mut page = use_signal(|| 1usize);

    // Form modal state
    let mut modal = ModalController::<String>::new();
    let mut form_name = use_signal(String::new);
    let mut form_format = use_signal(|| "single_elim".to_string());
    let mut form_game_id: Signal<Option<String>> = use_signal(|| None);
    let mut form_max = use_signal(String::new);
    let mut form_date = use_signal(String::new);
    let mut form_time = use_signal(String::new);
    let mut name_error = use_signal(String::new);
    let mut save_error = use_signal(|| None::<String>);
    let mut opener = use_signal(|| NEW_TOURNAMENT_ID.to_string());

    // Delete confirm
    let mut delete_modal = ModalController::<Tournament>::new();

    // Detail view state
    let mut detail_refresh = use_signal(|| 0u64);
    let mut bracket_data: Signal<Option<BracketData>> = use_signal(|| None);
    let mut detail_tournament: Signal<Option<Tournament>> = use_signal(|| None);
    let mut detail_loading = use_signal(|| false);
    // Why the tournament or its bracket did not load (the server's reason).
    let mut detail_error = use_signal(|| None::<String>);

    // Add participant
    let mut add_part_modal = ModalController::<()>::new();
    let mut add_part_member_id = use_signal(String::new);
    let mut add_part_member_error = use_signal(String::new);
    let mut add_part_error = use_signal(|| None::<String>);

    // Remove participant confirm
    let mut remove_part_modal = ModalController::<Participant>::new();

    // Match report modal
    let mut match_modal = ModalController::<BracketMatch>::new();
    let mut match_score_a = use_signal(String::new);
    let mut match_score_b = use_signal(String::new);
    let mut match_winner = use_signal(String::new);
    let mut match_replays = use_signal(String::new);
    let mut scores_error = use_signal(String::new);
    let mut winner_error = use_signal(String::new);
    let mut match_error = use_signal(|| None::<String>);

    // Detail data loader
    let _detail_loader = use_resource(move || async move {
        let _ = detail_refresh();
        if let Some(id) = detail_id() {
            detail_loading.set(true);
            detail_error.set(None);
            match ApiClient::web()
                .fetch::<Tournament>(&format!("/api/tournaments/{id}"))
                .await
            {
                Ok(t) => detail_tournament.set(Some(t)),
                Err(e) => detail_error.set(Some(crate::client_error::logged_message(
                    "tournament detail",
                    &e,
                ))),
            }
            match ApiClient::web()
                .fetch::<BracketData>(&format!("/api/tournaments/{id}/bracket"))
                .await
            {
                Ok(b) => bracket_data.set(Some(b)),
                Err(e) => {
                    if detail_error.peek().is_none() {
                        detail_error.set(Some(crate::client_error::logged_message(
                            "tournament bracket",
                            &e,
                        )));
                    }
                }
            }
            detail_loading.set(false);
        }
    });

    // --- List view handlers ---

    let mut open_create = move || {
        form_name.set(String::new());
        form_format.set("single_elim".to_string());
        form_game_id.set(None);
        form_max.set(String::new());
        form_date.set(String::new());
        form_time.set(String::new());
        name_error.set(String::new());
        save_error.set(None);
        opener.set(NEW_TOURNAMENT_ID.to_string());
        modal.show_empty();
    };

    let mut open_edit = move |t: Tournament| {
        name_error.set(String::new());
        save_error.set(None);
        opener.set(view_button_id(&t.id));
        form_name.set(t.name);
        form_format.set(t.format);
        // Preserve real game id (was wrongly mapping game_name to an empty string)
        form_game_id.set(t.game_id.clone());
        form_max.set(
            t.max_participants
                .map(|n| n.to_string())
                .unwrap_or_default(),
        );
        // Split "2026-03-11T18:00:00Z" into date + time
        let (date, time) = split_starts_at(&t.starts_at.unwrap_or_default());
        form_date.set(date);
        form_time.set(time);
        modal.show(t.id);
    };

    let on_close = move |_| modal.close();

    let on_submit = move |_| {
        let name = form_name().trim().to_string();
        save_error.set(None);
        if name.is_empty() {
            name_error.set("Enter a name for the tournament.".to_string());
            toast.show(Toast::error("Name is required."));
            return;
        }
        name_error.set(String::new());
        let max_str = form_max().trim().to_string();
        let max_val = if max_str.is_empty() {
            None
        } else {
            max_str.parse::<u32>().ok()
        };
        let date = form_date().trim().to_string();
        let time = form_time().trim().to_string();
        // Server deserializes starts_at as DateTime<Utc>: need full RFC3339.
        let starts_at = if date.is_empty() {
            None
        } else if time.is_empty() {
            Some(format!("{date}T00:00:00Z"))
        } else {
            let t = if time.len() == 5 {
                format!("{time}:00")
            } else {
                time
            };
            Some(format!("{date}T{t}Z"))
        };
        let body = CreateTournamentRequest {
            name,
            format: form_format(),
            game_id: form_game_id(),
            max_teams: max_val,
            starts_at,
        };
        let edit_id = modal.get_target();
        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                client
                    .put_json::<_, Tournament>(&format!("/api/tournaments/{id}"), &body)
                    .await
            } else {
                client
                    .post_json::<_, Tournament>("/api/tournaments", &body)
                    .await
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
                Err(e) => {
                    save_error.set(Some(
                        SaveState::failed("Could not save the tournament", &e).text(),
                    ));
                    toast.show(Toast::error_from("Failed to save", &e));
                }
            }
        });
    };

    let mut open_delete = move |t: Tournament| {
        delete_modal.show(t);
    };

    // The shown list's ids in order, for where focus goes after a row moves.
    let list_keys = move || -> Vec<String> {
        tournaments
            .data
            .peek()
            .as_ref()
            .and_then(|d| d.as_ref())
            .map(|all| {
                filter_tournaments(all, &status_filter.peek())
                    .iter()
                    .map(|t| t.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    };

    // Server has no DELETE route; archive via status transition.
    let on_delete_confirm = move |_| {
        if let Some(t) = delete_modal.get_target() {
            let id = t.id.clone();
            // B9: an archived row leaves a filtered list. Once it is gone,
            // focus the next row's View, else the previous row's, else New
            // tournament.
            let keys = list_keys();
            let focus_next = after_delete_focus_ids(
                &keys,
                &id,
                |k| vec![view_button_id(k)],
                &[NEW_TOURNAMENT_ID],
            );
            delete_modal.close();
            let body = StatusChangeRequest {
                status: "archived".into(),
            };
            spawn(async move {
                match ApiClient::web()
                    .patch_json_empty(&format!("/api/tournaments/{id}/status"), &body)
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Tournament archived."));
                        tournaments.refresh += 1;
                        focus_after_delete_soon(view_button_id(&id), focus_next);
                        games.refresh += 1;
                        members.refresh += 1;
                        detail_id.set(None);
                    }
                    Err(e) => toast.show(Toast::error_from("Archive failed", &e)),
                }
            });
        }
    };

    let on_delete_cancel = move |_| {
        delete_modal.close();
    };

    let change_status = move |(id, new_status): (String, String)| {
        let focus_next = after_status_focus_ids(&list_keys(), &id, &status_filter.peek());
        let body = StatusChangeRequest {
            status: new_status.clone(),
        };
        spawn(async move {
            let result = ApiClient::web()
                .patch_json_empty(&format!("/api/tournaments/{id}/status"), &body)
                .await;
            match result {
                Ok(_) => {
                    toast.show(Toast::success(format!(
                        "Status changed to {}.",
                        status_label(&new_status)
                    )));
                    tournaments.refresh += 1;
                    // From the list: focus lost with a removed button or row
                    // goes to the row's View once the row shows the new
                    // status, or to a neighbour once the row is gone.
                    focus_after_refresh_soon(
                        status_cell_id(&id),
                        status_label(&new_status),
                        focus_next,
                    );
                    games.refresh += 1;
                    members.refresh += 1;
                    detail_refresh += 1;
                }
                Err(e) => toast.show(Toast::error_from("Status change failed", &e)),
            }
        });
    };

    let mut view_detail = move |id: String| {
        detail_error.set(None);
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
        add_part_member_error.set(String::new());
        add_part_error.set(None);
        add_part_modal.show_empty();
    };

    let on_add_part_close = move |_| add_part_modal.close();

    let on_add_part_submit = move |_| {
        let mid = add_part_member_id().trim().to_string();
        add_part_error.set(None);
        if mid.is_empty() {
            add_part_member_error.set("Pick a member to add.".to_string());
            return;
        }
        add_part_member_error.set(String::new());
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
                    Err(e) => {
                        add_part_error.set(Some(
                            SaveState::failed("Could not add the participant", &e).text(),
                        ));
                        toast.show(Toast::error_from("Failed", &e));
                    }
                }
            });
        }
    };

    let mut open_remove_part = move |p: Participant| {
        remove_part_modal.show(p);
    };

    let on_remove_part_confirm = move |_| {
        if let Some(p) = remove_part_modal.get_target()
            && let Some(tid) = detail_id()
        {
            let pid = p.id.clone();
            // B9: once the row is gone, focus the next participant's Remove,
            // else the previous one's, else Add participant.
            let keys: Vec<String> = bracket_data
                .peek()
                .as_ref()
                .map(|b| b.participants.iter().map(|p| p.id.clone()).collect())
                .unwrap_or_default();
            let focus_next = after_delete_focus_ids(
                &keys,
                &pid,
                |k| vec![remove_participant_id(k)],
                &[ADD_PARTICIPANT_ID],
            );
            remove_part_modal.close();
            spawn(async move {
                match ApiClient::web()
                    .delete(&format!("/api/tournaments/{tid}/participants/{pid}"))
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Participant removed."));
                        detail_refresh += 1;
                        focus_after_delete_soon(remove_participant_id(&pid), focus_next);
                    }
                    Err(e) => toast.show(Toast::error_from("Remove failed", &e)),
                }
            });
        }
    };

    let on_remove_part_cancel = move |_| {
        remove_part_modal.close();
    };

    let generate_bracket = move |_| {
        if let Some(tid) = detail_id() {
            spawn(async move {
                match ApiClient::web()
                    .post_json_empty(&format!("/api/tournaments/{tid}/generate-bracket"), &())
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Bracket generated."));
                        detail_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error_from("Generate failed", &e)),
                }
            });
        }
    };

    let next_swiss_round = move |_| {
        if let Some(tid) = detail_id() {
            spawn(async move {
                match ApiClient::web()
                    .post_json_empty(&format!("/api/tournaments/{tid}/next-round"), &())
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Next Swiss round generated."));
                        detail_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error_from("Failed", &e)),
                }
            });
        }
    };

    let mut open_match_report = move |m: BracketMatch| {
        match_score_a.set(m.score_a.map(|n| n.to_string()).unwrap_or_default());
        match_score_b.set(m.score_b.map(|n| n.to_string()).unwrap_or_default());
        match_winner.set(m.winner_id.clone().unwrap_or_default());
        match_replays.set(String::new());
        scores_error.set(String::new());
        winner_error.set(String::new());
        match_error.set(None);
        match_modal.show(m);
    };

    let on_match_close = move |_| {
        match_modal.close();
    };

    let on_match_submit = move |_| {
        if let Some(m) = match_modal.get_target()
            && let Some(tid) = detail_id()
        {
            let mid = m.id.clone();
            let sa = match_score_a().trim().parse::<u32>();
            let sb = match_score_b().trim().parse::<u32>();
            let winner_raw = match_winner().trim().to_string();
            let replays_raw = match_replays().trim().to_string();
            match_error.set(None);
            let (Ok(score_a), Ok(score_b)) = (sa, sb) else {
                scores_error.set("Enter both scores as whole numbers.".to_string());
                toast.show(Toast::error("Scores must be numbers."));
                return;
            };
            scores_error.set(String::new());
            if winner_raw.is_empty() {
                winner_error.set("Pick the winner.".to_string());
                toast.show(Toast::error("Select a winner."));
                return;
            }
            winner_error.set(String::new());
            let body = MatchReportRequest {
                score_a,
                score_b,
                winner_id: winner_raw,
                notes: None,
                replay_codes: if replays_raw.is_empty() {
                    None
                } else {
                    Some(
                        replays_raw
                            .split([',', '\n'])
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect(),
                    )
                },
            };
            match_modal.start_submit();
            spawn(async move {
                let result = ApiClient::web()
                    .patch_json_empty(
                        &format!("/api/tournaments/{tid}/matches/{mid}/report"),
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
                    Err(e) => {
                        match_error.set(Some(
                            SaveState::failed("Could not save the result", &e).text(),
                        ));
                        toast.show(Toast::error_from("Failed", &e));
                    }
                }
            });
        }
    };

    // --- Render ---

    // Game names for the list and the detail header, from the games list the
    // form loads too. The API's game_name wins when it sends one.
    let game_list: Vec<Game> = games
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .cloned()
        .unwrap_or_default();

    let list = {
        let data = tournaments.data.read();
        let filter = status_filter();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&tournaments, "tournaments"),
            Some(all) => {
                let shown = filter_tournaments(all, &filter);
                let total = shown.len();
                let rows = page_slice(&shown, page(), ADMIN_PAGE_SIZE).to_vec();
                rsx! {
                    if all.is_empty() {
                        EmptyCard {
                            title: "No tournaments yet",
                            message: "Create a tournament, open registration, then generate the bracket once the participants are in.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-primary",
                                    onclick: move |_| open_create(),
                                    "New tournament"
                                }
                            },
                        }
                    } else if rows.is_empty() {
                        EmptyCard {
                            title: "No tournaments match these filters",
                            message: "Clear the filters to see every tournament, or pick another status.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-ghost",
                                    onclick: move |_| {
                                        status_filter.set("all".to_string());
                                        page.set(1);
                                    },
                                    "Clear filters"
                                }
                            },
                        }
                    } else {
                        DataTable {
                            headers: vec!["Tournament", "Status", "Format", "Game", "Max", "Starts", "Actions"],
                            num_cols: vec![4, 5],
                            cards: true,
                            label: "Tournaments",
                            for t in rows.iter() {
                                Fragment { key: "{t.id}", {
                                    let t_edit = t.clone();
                                    let t_del = t.clone();
                                    let t_view_id = t.id.clone();
                                    let tid_next = t.id.clone();
                                    let name = t.name.clone();
                                    let format = format_label(&t.format);
                                    let game_display = tournament_game(t, &game_list);
                                    let max_display = t
                                        .max_participants
                                        .map(|n| format!("{n} max"))
                                        .unwrap_or_else(|| "No limit".into());
                                    let advance = next_status(&t.status).filter(|_| t.status != "completed");
                                    let archivable = t.status != "archived";
                                    let more = rsx! {
                                        button {
                                            r#type: "button",
                                            class: "btn is-ghost",
                                            aria_label: "Edit {name}",
                                            onclick: move |_| open_edit(t_edit.clone()),
                                            "Edit"
                                        }
                                        if let Some((to, word)) = advance {
                                            button {
                                                r#type: "button",
                                                class: "btn is-ghost",
                                                aria_label: "{word}: {name}",
                                                onclick: move |_| change_status((tid_next.clone(), to.to_string())),
                                                "{word}"
                                            }
                                        }
                                        if archivable {
                                            button {
                                                r#type: "button",
                                                class: "btn is-danger",
                                                aria_label: "Archive {name}",
                                                onclick: move |_| open_delete(t_del.clone()),
                                                "Archive"
                                            }
                                        }
                                    };
                                    rsx! {
                                        tr { key: "{t.id}",
                                            td { class: "cell-who", "{t.name}" }
                                            td { class: "cell-badge", id: status_cell_id(&t.id), StatusPill { status: t.status.clone() } }
                                            td { class: "cell-meta", "{format}" }
                                            td { class: "cell-meta", "{game_display}" }
                                            td { class: "cell-meta num", "{max_display}" }
                                            td { class: "cell-meta num",
                                                match t.starts_at.clone() {
                                                    Some(iso) => rsx! { AdminDate { iso } },
                                                    None => rsx! { span { class: "date", "Date not set" } },
                                                }
                                            }
                                            td { class: "cell-actions",
                                                RowActions {
                                                    id: "tournament-actions-{t.id}",
                                                    label: "Actions for {name}",
                                                    inline_on_desktop: true,
                                                    more,
                                                    button {
                                                        r#type: "button",
                                                        class: "btn is-ghost",
                                                        id: view_button_id(&t.id),
                                                        aria_label: "View {name}",
                                                        onclick: move |_| view_detail(t_view_id.clone()),
                                                        "View"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } }
                            }
                        }
                    }
                    // Always rendered: its count line is the list's live line.
                    Pager {
                        id: "tournaments-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "tournaments",
                        noun_singular: "tournament",
                        show_page_count: true,
                        on_change: move |next| {
                            page.set(next);
                            show_list_top(LIST_ID);
                        },
                    }
                }
            }
        }
    };

    let back_button = rsx! {
        button {
            r#type: "button",
            class: "btn is-ghost",
            id: BACK_ID,
            onclick: back_to_list,
            "Back to tournaments"
        }
    };

    let detail = if detail_id().is_none() {
        rsx! {}
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
        let is_swiss = tournament.format == "swiss";
        let is_in_progress = tournament.status == "in_progress";
        let is_registration = tournament.status == "registration";
        let advance = next_status(&tournament.status);
        let format = format_label(&tournament.format);
        let game = tournament_game(&tournament, &game_list);
        let max = tournament
            .max_participants
            .map(|n| format!("{n} max"))
            .unwrap_or_else(|| "No limit".to_string());
        let refresh_error =
            detail_error().map(|reason| load_error_text("part of this tournament", &reason));
        rsx! {
            PageShell { width: PageWidth::Wide, class: "admin-tournaments-page",
                PageHeader {
                    title: tournament.name.clone(),
                    meta: rsx! {
                        p { class: "tournament-meta",
                            StatusPill { status: tournament.status.clone() }
                            span { "{format} \u{b7} {game} \u{b7} {max}" }
                        }
                    },
                    actions: rsx! {
                        {back_button}
                        // Status transitions (server: draft, registration,
                        // in_progress, completed, archived).
                        if let Some((to, word)) = advance {
                            button {
                                r#type: "button",
                                class: if to == "archived" { "btn is-ghost" } else { "btn is-primary" },
                                onclick: move |_| change_status((t_id.clone(), to.to_string())),
                                "{word}"
                            }
                        }
                        // Bracket only before/at registration (server requires bracket before start)
                        if tournament.status == "draft" || is_registration {
                            button {
                                r#type: "button",
                                class: "btn is-ghost",
                                onclick: generate_bracket,
                                "Generate bracket"
                            }
                        }
                        if is_swiss && is_in_progress {
                            button {
                                r#type: "button",
                                class: "btn is-ghost",
                                onclick: next_swiss_round,
                                "Next Swiss round"
                            }
                        }
                    },
                }

                if let Some(message) = refresh_error {
                    div { class: "fetch-error-wrap", role: "alert",
                        p { class: "fetch-error", "{message}" }
                        button {
                            r#type: "button",
                            class: "fetch-error__retry",
                            onclick: move |_| detail_refresh += 1,
                            "Retry"
                        }
                    }
                }

                section { class: "ui-stack", aria_labelledby: "tournament-participants-title",
                    div { class: "tournament-block-head",
                        h2 { class: "ui-section-title", id: "tournament-participants-title", "Participants" }
                        button {
                            r#type: "button",
                            class: "btn is-ghost",
                            id: ADD_PARTICIPANT_ID,
                            onclick: on_add_part_open,
                            "Add participant"
                        }
                    }
                    match bracket.as_ref().map(|b| &b.participants) {
                        Some(parts) if !parts.is_empty() => rsx! {
                            DataTable {
                                headers: vec!["Participant", "Seed", "Actions"],
                                num_cols: vec![1],
                                cards: true,
                                label: "Participants",
                                for p in parts.iter() {
                                    Fragment { key: "{p.id}", {
                                        let p_rm = p.clone();
                                        let pname = p.name.clone();
                                        rsx! {
                                            tr { key: "{p.id}",
                                                td { class: "cell-who", "{p.name}" }
                                                td { class: "num", "data-label": "Seed", "{p.seed}" }
                                                td { class: "cell-actions",
                                                    RowActions {
                                                        button {
                                                            r#type: "button",
                                                            class: "btn is-danger",
                                                            id: remove_participant_id(&p.id),
                                                            aria_label: "Remove {pname}",
                                                            onclick: move |_| open_remove_part(p_rm.clone()),
                                                            "Remove"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    } }
                                }
                            }
                        },
                        _ => rsx! {
                            p { class: "field-hint", "No participants yet. Use Add participant to sign members up." }
                        },
                    }
                }

                section { class: "ui-stack", aria_labelledby: "tournament-bracket-title",
                    h2 { class: "ui-section-title", id: "tournament-bracket-title", "Bracket" }
                    {
                        let rounds = bracket.as_ref().map(|b| &b.rounds);
                        let matches = bracket.as_ref().map(|b| &b.matches);
                        match (rounds, matches) {
                            (Some(rnds), Some(mtchs)) if !rnds.is_empty() => rsx! {
                                for round in rnds.iter() {
                                    Fragment { key: "{round.id}", {
                                        let round_matches: Vec<BracketMatch> = mtchs
                                            .iter()
                                            .filter(|m| m.round_id == round.id)
                                            .cloned()
                                            .collect();
                                        let pmap = participant_map.clone();
                                        let stage = status_label(&round.stage);
                                        let round_label = format!("Round {}", round.round_number);
                                        rsx! {
                                            div { key: "{round.id}", class: "tournament-round",
                                                h3 { "{round_label}, {stage}" }
                                                DataTable {
                                                    headers: vec!["Match", "Status", "Player A", "Score", "Player B", "Actions"],
                                                    num_cols: vec![3],
                                                    cards: true,
                                                    label: round_label.clone(),
                                                    for bm in round_matches.iter() {
                                                        Fragment { key: "{bm.id}", {
                                                            let a_name = bm.participant_a_id.as_ref()
                                                                .and_then(|id| pmap.get(id))
                                                                .cloned()
                                                                .unwrap_or_else(|| "To be decided".into());
                                                            let b_name = bm.participant_b_id.as_ref()
                                                                .and_then(|id| pmap.get(id))
                                                                .cloned()
                                                                .unwrap_or_else(|| "To be decided".into());
                                                            let score_display = match (bm.score_a, bm.score_b) {
                                                                (Some(a), Some(b)) => format!("{a} - {b}"),
                                                                _ => "No score".to_string(),
                                                            };
                                                            let a_won = bm.winner_id.is_some() && bm.winner_id == bm.participant_a_id;
                                                            let b_won = bm.winner_id.is_some() && bm.winner_id == bm.participant_b_id;
                                                            let bm_report = bm.clone();
                                                            rsx! {
                                                                tr { key: "{bm.id}",
                                                                    td { class: "cell-who", "Match {bm.bracket_position}" }
                                                                    td { class: "cell-badge", StatusPill { status: bm.status.clone() } }
                                                                    td { "data-label": "Player A",
                                                                        BracketSide { name: a_name.clone(), won: a_won }
                                                                    }
                                                                    td { class: "num", "data-label": "Score", "{score_display}" }
                                                                    td { "data-label": "Player B",
                                                                        BracketSide { name: b_name.clone(), won: b_won }
                                                                    }
                                                                    td { class: "cell-actions",
                                                                        RowActions {
                                                                            button {
                                                                                r#type: "button",
                                                                                class: "btn is-ghost",
                                                                                id: report_button_id(&bm.id),
                                                                                aria_label: "Report match {bm.bracket_position}: {a_name} against {b_name}",
                                                                                onclick: move |_| open_match_report(bm_report.clone()),
                                                                                "Report"
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        } }
                                                    }
                                                }
                                            }
                                        }
                                    } }
                                }
                            },
                            _ => rsx! {
                                p { class: "field-hint", "No bracket yet. Generate it once the participants are in, before the tournament starts." }
                            },
                        }
                    }
                }

                {list_cap_notice(&members, "members")}
            }
        }
    } else if detail_loading() || detail_error().is_none() {
        rsx! {
            PageShell { width: PageWidth::Wide, class: "admin-tournaments-page",
                PageHeader { title: "Tournament", actions: back_button }
                div { class: "admin-pending",
                    p { class: "admin-pending-text", role: "status", "Loading tournament..." }
                    SkeletonBars { rows: 3 }
                }
            }
        }
    } else {
        let message = load_error_text("this tournament", &detail_error().unwrap_or_default());
        rsx! {
            PageShell { width: PageWidth::Wide, class: "admin-tournaments-page",
                PageHeader { title: "Tournament", actions: back_button }
                div { class: "fetch-error-wrap", role: "alert",
                    p { class: "fetch-error", "{message}" }
                    button {
                        r#type: "button",
                        class: "fetch-error__retry",
                        onclick: move |_| detail_refresh += 1,
                        "Retry"
                    }
                }
            }
        }
    };

    let delete_focus = delete_modal.get_target().map(|t| view_button_id(&t.id));
    let remove_focus = remove_part_modal
        .get_target()
        .map(|p| remove_participant_id(&p.id));
    let report_focus = match_modal.get_target().map(|m| report_button_id(&m.id));
    let name_invalid = !name_error().is_empty();
    let part_invalid = !add_part_member_error().is_empty();
    let scores_invalid = !scores_error().is_empty();
    let winner_invalid = !winner_error().is_empty();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-tournaments-page-css", css: PAGE_CSS }

        if detail_id().is_some() {
            {detail}
        } else {
            PageShell { width: PageWidth::Wide, class: "admin-tournaments-page",
                PageHeader {
                    title: "Tournaments",
                    lead: "Filter by status. View a tournament to manage its participants and bracket.",
                    actions: rsx! {
                        button {
                            r#type: "button",
                            class: "btn is-primary",
                            id: NEW_TOURNAMENT_ID,
                            onclick: move |_| open_create(),
                            "New tournament"
                        }
                    },
                }

                ChipRow { label: "Status filters", scroll: true,
                    for s in STATUS_FILTERS {
                        ChipButton {
                            key: "{s}",
                            class: "chip",
                            id: tournament_filter_id(s),
                            label: filter_label(s),
                            pressed: status_filter() == s,
                            onclick: move |_| {
                                status_filter.set(s.to_string());
                                page.set(1);
                            },
                        }
                    }
                }

                section {
                    id: LIST_ID,
                    class: "admin-list pager-target",
                    tabindex: "-1",
                    aria_label: "Tournaments",
                    {list}
                }

                {list_cap_notice(&tournaments, "tournaments")}
                {list_cap_notice(&games, "games")}
                {list_cap_notice(&members, "members")}
            }
        }

        // --- Shared modals (always rendered) ---

        // Create/Edit tournament modal
        FormModal {
            title: if modal.get_target().is_some() { "Edit tournament".to_string() } else { "New tournament".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,
            wide: true,
            submit_label: modal.get_target().is_none().then(|| "Create".to_string()),
            error: save_error(),
            restore_focus_id: Some(opener()),

            div { class: "form-grid",
                div { class: "form-field span-full",
                    label { class: "field-label", r#for: NAME_ID, "Name" }
                    input {
                        class: "field-input",
                        id: NAME_ID,
                        r#type: "text",
                        value: "{form_name}",
                        aria_invalid: if name_invalid { "true" },
                        aria_describedby: NAME_ERROR_ID,
                        oninput: move |e| form_name.set(e.value()),
                    }
                    FieldError { id: NAME_ERROR_ID.to_string(), text: name_error() }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: FORMAT_ID, "Format" }
                    select {
                        class: "field-input",
                        id: FORMAT_ID,
                        value: "{form_format}",
                        onchange: move |e| form_format.set(e.value()),
                        for f in FORMATS.iter() {
                            option { key: "{f}", value: "{f}", "{format_label(f)}" }
                        }
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: GAME_ID, "Game (optional)" }
                    select {
                        class: "field-input",
                        id: GAME_ID,
                        value: form_game_id().unwrap_or_default(),
                        onchange: move |e| {
                            let v = e.value();
                            form_game_id.set(if v.is_empty() { None } else { Some(v) });
                        },
                        option { value: "", "No game" }
                        {
                            let g = games.data.read();
                            let g = g.as_ref().and_then(|d| d.as_ref());
                            match g {
                                Some(list) => rsx! {
                                    for game in list.iter() {
                                        option { key: "{game.id}", value: "{game.id}", "{game.name}" }
                                    }
                                },
                                None => rsx! {},
                            }
                        }
                    }
                }
                div { class: "form-field span-full",
                    label { class: "field-label", r#for: MAX_ID, "Max participants (optional)" }
                    input {
                        class: "field-input",
                        id: MAX_ID,
                        r#type: "number",
                        inputmode: "numeric",
                        placeholder: "16",
                        value: "{form_max}",
                        oninput: move |e| form_max.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: DATE_ID, "Start date (optional)" }
                    input {
                        class: "field-input",
                        id: DATE_ID,
                        r#type: "date",
                        value: "{form_date}",
                        onchange: move |e| form_date.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: TIME_ID, "Start time, UTC (optional)" }
                    input {
                        class: "field-input",
                        id: TIME_ID,
                        r#type: "time",
                        aria_describedby: TIME_HINT_ID,
                        value: "{form_time}",
                        onchange: move |e| form_time.set(e.value()),
                    }
                    p { class: "field-hint", id: TIME_HINT_ID, "Without a time, the tournament starts at 00:00 UTC." }
                }
            }
        }

        // Archive confirm (no hard-delete API)
        ConfirmDialog {
            title: "Archive tournament".to_string(),
            message: format!(
                "Archive tournament \"{}\"? It will no longer appear as active.",
                delete_modal.get_target().map(|t| t.name).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            confirm_label: "Archive".to_string(),
            restore_focus_id: delete_focus,
            on_confirm: on_delete_confirm,
            on_cancel: on_delete_cancel,
        }

        // Add participant modal
        FormModal {
            title: "Add participant".to_string(),
            open: add_part_modal.is_open(),
            submitting: add_part_modal.is_submitting(),
            on_close: on_add_part_close,
            on_submit: on_add_part_submit,
            submit_label: Some("Add".to_string()),
            error: add_part_error(),
            restore_focus_id: Some(ADD_PARTICIPANT_ID.to_string()),

            div { class: "form-field",
                label { class: "field-label", r#for: PARTICIPANT_ID, "Member" }
                select {
                    class: "field-input",
                    id: PARTICIPANT_ID,
                    value: "{add_part_member_id}",
                    aria_invalid: if part_invalid { "true" },
                    aria_describedby: PARTICIPANT_ERROR_ID,
                    onchange: move |e| add_part_member_id.set(e.value()),
                    option { value: "", "Pick a member" }
                    {
                        let m = members.data.read();
                        let m = m.as_ref().and_then(|d| d.as_ref());
                        match m {
                            Some(list) => rsx! {
                                for mem in list.iter() {
                                    option { key: "{mem.id}", value: "{mem.id}", "{mem.display_name}" }
                                }
                            },
                            None => rsx! {},
                        }
                    }
                }
                FieldError { id: PARTICIPANT_ERROR_ID.to_string(), text: add_part_member_error() }
            }
        }

        // Remove participant confirm
        ConfirmDialog {
            title: "Remove participant".to_string(),
            message: format!(
                "Remove \"{}\" from this tournament?",
                remove_part_modal.get_target().map(|p| p.name).unwrap_or_default()
            ),
            open: remove_part_modal.is_open(),
            danger: true,
            confirm_label: "Remove".to_string(),
            restore_focus_id: remove_focus,
            on_confirm: on_remove_part_confirm,
            on_cancel: on_remove_part_cancel,
        }

        // Match report modal
        FormModal {
            title: "Report match result".to_string(),
            open: match_modal.is_open(),
            submitting: match_modal.is_submitting(),
            on_close: on_match_close,
            on_submit: on_match_submit,
            submit_label: Some("Save result".to_string()),
            error: match_error(),
            restore_focus_id: report_focus,

            {
                let (a_name, b_name, a_id, b_id) = match match_modal.get_target() {
                    Some(m) => {
                        let bd = bracket_data();
                        let pmap: HashMap<String, String> = bd
                            .as_ref()
                            .map(|b| b.participants.iter().map(|p| (p.id.clone(), p.name.clone())).collect())
                            .unwrap_or_default();
                        let a_id = m.participant_a_id.clone().unwrap_or_default();
                        let b_id = m.participant_b_id.clone().unwrap_or_default();
                        let a_name = pmap.get(&a_id).cloned().unwrap_or_else(|| "Player A".into());
                        let b_name = pmap.get(&b_id).cloned().unwrap_or_else(|| "Player B".into());
                        (a_name, b_name, a_id, b_id)
                    }
                    None => ("Player A".to_string(), "Player B".to_string(), String::new(), String::new()),
                };
                rsx! {
                    div { class: "form-grid",
                        div { class: "form-field",
                            label { class: "field-label", r#for: SCORE_A_ID, "Score for {a_name}" }
                            input {
                                class: "field-input",
                                id: SCORE_A_ID,
                                r#type: "number",
                                inputmode: "numeric",
                                aria_invalid: if scores_invalid { "true" },
                                aria_describedby: SCORES_ERROR_ID,
                                value: "{match_score_a}",
                                oninput: move |e| match_score_a.set(e.value()),
                            }
                        }
                        div { class: "form-field",
                            label { class: "field-label", r#for: SCORE_B_ID, "Score for {b_name}" }
                            input {
                                class: "field-input",
                                id: SCORE_B_ID,
                                r#type: "number",
                                inputmode: "numeric",
                                aria_invalid: if scores_invalid { "true" },
                                aria_describedby: SCORES_ERROR_ID,
                                value: "{match_score_b}",
                                oninput: move |e| match_score_b.set(e.value()),
                            }
                        }
                        div { class: "span-full",
                            FieldError { id: SCORES_ERROR_ID.to_string(), text: scores_error() }
                        }
                    }
                    div { class: "form-field",
                        label { class: "field-label", r#for: WINNER_ID, "Winner" }
                        select {
                            class: "field-input",
                            id: WINNER_ID,
                            value: "{match_winner}",
                            aria_invalid: if winner_invalid { "true" },
                            aria_describedby: WINNER_ERROR_ID,
                            onchange: move |e| match_winner.set(e.value()),
                            option { value: "", "Pick the winner" }
                            if !a_id.is_empty() {
                                option { value: "{a_id}", "{a_name}" }
                            }
                            if !b_id.is_empty() {
                                option { value: "{b_id}", "{b_name}" }
                            }
                        }
                        FieldError { id: WINNER_ERROR_ID.to_string(), text: winner_error() }
                    }
                }
            }
            div { class: "form-field",
                label { class: "field-label", r#for: REPLAYS_ID, "Replay codes (optional, one per line)" }
                textarea {
                    class: "field-input",
                    id: REPLAYS_ID,
                    value: "{match_replays}",
                    oninput: move |e| match_replays.set(e.value()),
                }
            }
        }
    }
}

/// One side of a bracket match. The winner is bold and carries a "Won"
/// pill, so the result is never colour or weight alone.
#[component]
fn BracketSide(name: String, won: bool) -> Element {
    rsx! {
        span { class: if won { "bracket-side is-winner" } else { "bracket-side" },
            "{name}"
            if won {
                Pill { tone: PillTone::Ok, "Won" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::admin_shared::test_page::render_admin_route;
    use scuffed_types::OrgRole;

    fn tournament(id: &str, status: &str) -> Tournament {
        Tournament {
            id: id.into(),
            name: format!("Cup {id}"),
            format: "single_elim".into(),
            game_name: None,
            game_id: None,
            status: status.into(),
            max_participants: None,
            starts_at: None,
        }
    }

    #[test]
    fn the_game_comes_from_game_id_when_the_api_sends_no_name() {
        let games = vec![Game {
            id: "game:ow".into(),
            name: "Overwatch".into(),
        }];
        let mut t = tournament("t1", "draft");
        assert_eq!(tournament_game(&t, &games), "No game");
        t.game_id = Some("ow".into());
        assert_eq!(tournament_game(&t, &games), "Overwatch");
        t.game_id = Some("gone".into());
        assert_eq!(tournament_game(&t, &games), "Unknown game");
        t.game_name = Some("Marvel Rivals".into());
        assert_eq!(tournament_game(&t, &games), "Marvel Rivals");
    }

    #[test]
    fn list_has_one_h1_status_chips_and_a_labelled_wait() {
        let html = render_admin_route("/admin/tournaments", Some(OrgRole::Officer));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-tournaments-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Tournaments</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains(
                "<div class=\"chip-row is-scroll\" role=\"group\" aria-label=\"Status filters\">"
            ),
            "{html}"
        );
        assert_eq!(html.matches("id=\"tournament-filter-").count(), 6, "{html}");
        assert!(
            html.contains("id=\"tournament-filter-all\" aria-pressed=\"true\""),
            "{html}"
        );
        assert!(html.contains(">In progress</button>"), "{html}");
        assert!(
            html.contains("role=\"status\">Loading tournaments...</p>"),
            "{html}"
        );
    }

    #[test]
    fn filters_statuses_and_formats_read_as_words() {
        let list = vec![
            tournament("a", "draft"),
            tournament("b", "in_progress"),
            tournament("c", "archived"),
        ];
        assert_eq!(filter_tournaments(&list, "all").len(), 3);
        assert_eq!(filter_tournaments(&list, "in_progress")[0].id, "b");
        assert!(filter_tournaments(&list, "completed").is_empty());
        assert_eq!(filter_label("all"), "All");
        assert_eq!(filter_label("in_progress"), "In progress");
        assert_eq!(format_label("double_elim"), "Double elimination");
        assert_eq!(
            next_status("draft"),
            Some(("registration", "Open registration"))
        );
        assert_eq!(next_status("completed"), Some(("archived", "Archive")));
        assert_eq!(next_status("archived"), None);
        assert_eq!(
            split_starts_at("2026-03-11T18:00:00Z"),
            ("2026-03-11".to_string(), "18:00".to_string())
        );
        assert_eq!(split_starts_at(""), (String::new(), String::new()));
    }

    #[test]
    fn the_winner_is_named_in_words() {
        fn view() -> Element {
            rsx! {
                BracketSide { name: "Player01".to_string(), won: true }
                BracketSide { name: "Player02".to_string(), won: false }
            }
        }
        let html = crate::components::ui::test_support::render(view);
        assert!(html.contains("bracket-side is-winner"), "{html}");
        assert_eq!(html.matches(">Won</span>").count(), 1, "{html}");
        crate::components::ui::test_support::assert_sizes_from_tokens(PAGE_CSS);
    }

    #[test]
    fn every_field_has_a_tied_label_and_no_inline_style() {
        let src = include_str!("tournaments.rs");
        let prod = src.split("mod tests").next().unwrap();
        for id in [
            "NAME_ID",
            "FORMAT_ID",
            "GAME_ID",
            "MAX_ID",
            "DATE_ID",
            "TIME_ID",
            "PARTICIPANT_ID",
            "SCORE_A_ID",
            "SCORE_B_ID",
            "WINNER_ID",
            "REPLAYS_ID",
        ] {
            assert!(prod.contains(&format!("r#for: {id}")), "{id}");
            assert!(prod.contains(&format!("id: {id},")), "{id}");
        }
        assert!(!prod.contains("style:"));
        assert!(!prod.contains('\u{2014}'));
        assert!(
            !prod.contains('\u{2192}'),
            "no arrow glyphs in button words"
        );
    }

    #[test]
    fn a_status_change_from_the_list_hands_focus_on() {
        // Vision nit 3 on PR 706 round 4: Complete on desktop removed its
        // own button, and a status chip can move the row out.
        let keys: Vec<String> = ["a", "b", "c"].iter().map(|k| k.to_string()).collect();
        let v = view_button_id;
        assert_eq!(
            after_status_focus_ids(&keys, "b", "all"),
            [v("b"), v("c"), v("a"), tournament_filter_id("all")]
        );
        assert_eq!(
            after_status_focus_ids(&["only".to_string()], "only", "in_progress"),
            [v("only"), "tournament-filter-in_progress".to_string()]
        );
        let src = include_str!("tournaments.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        // The wait ends when the row's status cell reads the new status or
        // the row is gone.
        assert!(prod.contains("td { class: \"cell-badge\", id: status_cell_id(&t.id),"));
        assert!(prod.contains(
            "focus_after_refresh_soon(\n                        status_cell_id(&id),\n                        status_label(&new_status),\n                        focus_next,\n                    );"
        ));
        assert!(prod.contains("id: tournament_filter_id(s),"));
    }
}
