use dioxus::prelude::*;

use crate::components::admin_shared::{
    after_delete_focus_ids, focus_after_delete_soon, focus_after_refresh_soon,
};
use serde::Deserialize;
use std::collections::HashMap;

use crate::client_error::two_sentences;
use crate::components::ui::inline_confirm::focus_element_by_id;
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
use crate::pages::tournament::{Participant, UNNAMED_ENTRANT, participant_names};
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
struct Team {
    id: String,
    name: String,
    /// The team's game (`game:abc` or `abc`). Missing reads as any game.
    #[serde(default)]
    game_id: Option<String>,
}

/// The teams the Add participant picker offers: every listed team that is
/// not entered yet (the server holds one row per team and tournament), and,
/// when the tournament has a game, only that game's teams. Issue 850.
fn team_choices(teams: &[Team], bracket: Option<&BracketData>, game_id: Option<&str>) -> Vec<Team> {
    let entered: Vec<&str> = bracket
        .map(|b| {
            b.participants
                .iter()
                .filter_map(|p| p.team_id.as_deref())
                .collect()
        })
        .unwrap_or_default();
    let game = game_id.map(bare_game_id).filter(|g| !g.is_empty());
    teams
        .iter()
        .filter(|t| !entered.contains(&t.id.as_str()))
        .filter(|t| match (game, t.game_id.as_deref().map(bare_game_id)) {
            (Some(want), Some(have)) if !have.is_empty() => want == have,
            _ => true,
        })
        .cloned()
        .collect()
}

/// The Add participant entrant hint, or why the teams did not load. The
/// default names the teams the picker leaves out (issue 850), so an admin
/// whose team is missing knows why and can type the name.
fn entrant_hint_text(teams_error: Option<&str>) -> String {
    match teams_error {
        Some(reason) => two_sentences(
            &load_error_text("the teams list", reason),
            "You can still type an outside team name.",
        ),
        None => "Pick one of your teams, type a name for a team from outside the org, or both. \
                 With both, the typed name is shown and the entry stays linked to your team. \
                 Teams already entered and teams of another game are not listed; to enter a team \
                 of another game, type its name."
            .to_string(),
    }
}

/// Display name of each participant, keyed by participant id: the same
/// naming as the public tournament page.
fn participant_name_map(
    bracket: Option<&BracketData>,
    teams: Option<&Vec<Team>>,
) -> HashMap<String, String> {
    let team_names: HashMap<String, String> = teams
        .map(|list| {
            list.iter()
                .map(|t| (t.id.clone(), t.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    bracket
        .map(|b| participant_names(&b.participants, &team_names))
        .unwrap_or_default()
}

/// Name of each participants table row, keyed by participant id, for the
/// Remove button and its dialog. A name that repeats (two "Unnamed entrant"
/// rows, or the same outside name twice) gets its row number, so every row
/// has its own name. A unique name that happens to read like a numbered one
/// gets its row number too, so the result is unique for every row.
fn participant_row_labels(
    parts: &[Participant],
    names: &HashMap<String, String>,
) -> HashMap<String, String> {
    let base: Vec<String> = parts
        .iter()
        .map(|p| {
            names
                .get(&p.id)
                .cloned()
                .unwrap_or_else(|| UNNAMED_ENTRANT.to_string())
        })
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for name in &base {
        *counts.entry(name.as_str()).or_default() += 1;
    }
    let mut numbered: Vec<bool> = base.iter().map(|n| counts[n.as_str()] > 1).collect();
    loop {
        let labels: Vec<String> = base
            .iter()
            .zip(&numbered)
            .enumerate()
            .map(|(i, (name, &num))| {
                if num {
                    format!("{name}, row {}", i + 1)
                } else {
                    name.clone()
                }
            })
            .collect();
        // Numbered labels end in distinct row numbers, so only a plain
        // label can clash with one; number it and check again.
        let taken: std::collections::HashSet<&str> = labels
            .iter()
            .zip(&numbered)
            .filter(|(_, num)| **num)
            .map(|(l, _)| l.as_str())
            .collect();
        let clashes: Vec<usize> = (0..labels.len())
            .filter(|&i| !numbered[i] && taken.contains(labels[i].as_str()))
            .collect();
        if clashes.is_empty() {
            return parts.iter().map(|p| p.id.clone()).zip(labels).collect();
        }
        for i in clashes {
            numbered[i] = true;
        }
    }
}

/// What is wrong with the Add participant form, every field in one pass.
#[derive(Debug, Clone, Default, PartialEq)]
struct AddParticipantErrors {
    entrant: Option<&'static str>,
    seed: Option<&'static str>,
}

/// The Add participant form as the body the server takes: a picked team
/// (`team_id`), a typed name (`external_name`), or both (the team is the
/// entrant and the typed name is the name shown for it), and a seed only
/// when one was typed. A form with errors reports all of them.
fn add_participant_body(
    team_id: &str,
    external_name: &str,
    seed: &str,
) -> Result<AddParticipantRequest, AddParticipantErrors> {
    let team_id = team_id.trim();
    let external_name = external_name.trim();
    let seed = seed.trim();
    let mut errors = AddParticipantErrors::default();
    let team_id = Some(team_id.to_string()).filter(|t| !t.is_empty());
    let external_name = Some(external_name.to_string()).filter(|n| !n.is_empty());
    if team_id.is_none() && external_name.is_none() {
        errors.entrant = Some("Pick a team or type a name.");
    }
    let seed = if seed.is_empty() {
        None
    } else {
        match seed.parse::<u32>() {
            Ok(n) if n >= 1 => Some(n),
            _ => {
                errors.seed = Some(SEED_ERROR);
                None
            }
        }
    };
    if errors != AddParticipantErrors::default() {
        return Err(errors);
    }
    Ok(AddParticipantRequest {
        team_id,
        external_name,
        seed,
    })
}

/// The seed field error: what the field takes, and that blank is fine.
const SEED_ERROR: &str = "Seed must be a whole number, 1 or higher, or blank for no seed.";

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
const PARTICIPANT_TEAM_ID: &str = "participant-team";
const PARTICIPANT_NAME_ID: &str = "participant-name";
const PARTICIPANT_HINT_ID: &str = "participant-entrant-hint";
const PARTICIPANT_ERROR_ID: &str = "participant-entrant-error";
const PARTICIPANT_SEED_ID: &str = "participant-seed";
const PARTICIPANT_SEED_HINT_ID: &str = "participant-seed-hint";
const PARTICIPANT_SEED_ERROR_ID: &str = "participant-seed-error";
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

/// The Seed cell: the number, or a word when the participant has none.
fn seed_label(seed: Option<u32>) -> String {
    seed.map(|n| n.to_string())
        .unwrap_or_else(|| "Unseeded".to_string())
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
    let mut teams = use_api_list::<Team>("/api/teams");
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
    let mut add_part_team_id = use_signal(String::new);
    let mut add_part_name = use_signal(String::new);
    let mut add_part_seed = use_signal(String::new);
    let mut add_part_entrant_error = use_signal(String::new);
    let mut add_part_seed_error = use_signal(String::new);
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
                    teams.refresh += 1;
                }
                Err(e) => {
                    save_error.set(Some(
                        SaveState::failed("Could not save the tournament", &e).text(),
                    ));
                    toast.show(Toast::error_logged_elsewhere("Failed to save", &e));
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
                        teams.refresh += 1;
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
                    teams.refresh += 1;
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
        add_part_team_id.set(String::new());
        add_part_name.set(String::new());
        add_part_seed.set(String::new());
        add_part_entrant_error.set(String::new());
        add_part_seed_error.set(String::new());
        add_part_error.set(None);
        add_part_modal.show_empty();
    };

    let on_add_part_close = move |_| add_part_modal.close();

    let on_add_part_submit = move |_| {
        add_part_error.set(None);
        add_part_entrant_error.set(String::new());
        add_part_seed_error.set(String::new());
        let body =
            match add_participant_body(&add_part_team_id(), &add_part_name(), &add_part_seed()) {
                Ok(body) => body,
                Err(errors) => {
                    // Both lines at once, a toast like the sibling forms,
                    // and focus on the first field to fix.
                    add_part_entrant_error.set(errors.entrant.unwrap_or_default().to_string());
                    add_part_seed_error.set(errors.seed.unwrap_or_default().to_string());
                    toast.show(Toast::error("Check the participant fields."));
                    focus_element_by_id(if errors.entrant.is_some() {
                        PARTICIPANT_TEAM_ID
                    } else {
                        PARTICIPANT_SEED_ID
                    });
                    return;
                }
            };
        if let Some(id) = detail_id() {
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
                        // One line for both, logged once by `SaveState::failed`.
                        let line = SaveState::failed("Could not add the participant", &e).text();
                        add_part_error.set(Some(line.clone()));
                        toast.show(Toast::error(line));
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
                    Err(e) => {
                        toast.show(Toast::error_from("Could not generate the next round.", &e))
                    }
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
                        // One line for both, logged once by `SaveState::failed`.
                        let line = SaveState::failed("Could not save the result", &e).text();
                        match_error.set(Some(line.clone()));
                        toast.show(Toast::error(line));
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

    // Participant id to display name, for the table, the bracket and both
    // participant dialogs.
    let participant_map: HashMap<String, String> = {
        let bracket = bracket_data.read();
        let teams = teams.data.read();
        participant_name_map(bracket.as_ref(), teams.as_ref().and_then(|d| d.as_ref()))
    };
    // Participant id to a name unique to its row, for the Remove button and
    // its dialog (QA Q3: two "Unnamed entrant" rows must not share a name).
    let row_labels: HashMap<String, String> = bracket_data
        .read()
        .as_ref()
        .map(|b| participant_row_labels(&b.participants, &participant_map))
        .unwrap_or_default();
    // Issue 850: teams already entered, or of another game, are not offered.
    let team_options: Vec<Team> = {
        let bracket = bracket_data.read();
        let tournament = detail_tournament.read();
        let teams = teams.data.read();
        team_choices(
            teams
                .as_ref()
                .and_then(|d| d.as_deref())
                .unwrap_or_default(),
            bracket.as_ref(),
            tournament.as_ref().and_then(|t| t.game_id.as_deref()),
        )
    };
    // The entrant hint, or why the teams did not load: always rendered.
    let entrant_hint = entrant_hint_text(teams.error.read().as_deref());

    let detail = if detail_id().is_none() {
        rsx! {}
    } else if let Some(tournament) = detail_tournament() {
        let bracket = bracket_data();
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
                                        let pname = participant_map
                                            .get(&p.id)
                                            .cloned()
                                            .unwrap_or_else(|| UNNAMED_ENTRANT.to_string());
                                        let row_label = row_labels
                                            .get(&p.id)
                                            .cloned()
                                            .unwrap_or_else(|| pname.clone());
                                        let seed = seed_label(p.seed);
                                        rsx! {
                                            tr { key: "{p.id}",
                                                td { class: "cell-who", "{pname}" }
                                                td { class: "num", "data-label": "Seed", "{seed}" }
                                                td { class: "cell-actions",
                                                    RowActions {
                                                        button {
                                                            r#type: "button",
                                                            class: "btn is-danger",
                                                            id: remove_participant_id(&p.id),
                                                            aria_label: "Remove {row_label}",
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
                        Some(_) => rsx! {
                            p { class: "field-hint", "No participants yet. Use Add participant to enter a team." }
                        },
                        // Not loaded: the load line or the error above says
                        // why, so no claim that the list is empty.
                        None if detail_loading() => rsx! {
                            p { class: "field-hint", "Loading participants..." }
                        },
                        None => rsx! {},
                    }
                }

                section { class: "ui-stack", aria_labelledby: "tournament-bracket-title",
                    h2 { class: "ui-section-title", id: "tournament-bracket-title", "Bracket" }
                    match bracket.as_ref().map(|b| (&b.rounds, &b.matches)) {
                        Some((rnds, mtchs)) if !rnds.is_empty() => rsx! {
                            for round in rnds.iter() {
                                Fragment { key: "{round.id}", {
                                    let round_matches: Vec<BracketMatch> = mtchs
                                        .iter()
                                        .filter(|m| m.round_id == round.id)
                                        .cloned()
                                        .collect();
                                    let pmap = &participant_map;
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
                        Some(_) => rsx! {
                            p { class: "field-hint", "No bracket yet. Generate it once the participants are in, before the tournament starts." }
                        },
                        // Not loaded: the load line or the error above says
                        // why, so no claim that there is no bracket.
                        None if detail_loading() => rsx! {
                            p { class: "field-hint", "Loading bracket..." }
                        },
                        None => rsx! {},
                    }
                }

                {list_cap_notice(&teams, "teams")}
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
    let part_invalid = !add_part_entrant_error().is_empty();
    let seed_invalid = !add_part_seed_error().is_empty();
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
                {list_cap_notice(&teams, "teams")}
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

            // One entrant: a team, an outside name, or a team shown under
            // a typed name.
            fieldset { class: "admin-check-group", aria_describedby: PARTICIPANT_HINT_ID,
                legend { class: "field-label", "Entrant" }
                p { class: "field-hint", id: PARTICIPANT_HINT_ID, "{entrant_hint}" }
                div { class: "form-field",
                    label { class: "field-label", r#for: PARTICIPANT_TEAM_ID, "Team" }
                    select {
                        class: "field-input",
                        id: PARTICIPANT_TEAM_ID,
                        value: "{add_part_team_id}",
                        aria_invalid: if part_invalid { "true" },
                        aria_describedby: PARTICIPANT_ERROR_ID,
                        onchange: move |e| add_part_team_id.set(e.value()),
                        option { value: "", "No team, use the name below" }
                        for team in team_options.iter() {
                            option { key: "{team.id}", value: "{team.id}", "{team.name}" }
                        }
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: PARTICIPANT_NAME_ID, "Name to show" }
                    input {
                        class: "field-input",
                        id: PARTICIPANT_NAME_ID,
                        r#type: "text",
                        aria_invalid: if part_invalid { "true" },
                        aria_describedby: PARTICIPANT_ERROR_ID,
                        value: "{add_part_name}",
                        oninput: move |e| add_part_name.set(e.value()),
                    }
                    FieldError { id: PARTICIPANT_ERROR_ID.to_string(), text: add_part_entrant_error() }
                }
            }
            div { class: "form-field",
                label { class: "field-label", r#for: PARTICIPANT_SEED_ID, "Seed (optional)" }
                input {
                    class: "field-input",
                    id: PARTICIPANT_SEED_ID,
                    // Text, not number: a number input reads unparsable
                    // text back as "", which would add the entrant unseeded
                    // with no error. The validator sees what was typed.
                    r#type: "text",
                    inputmode: "numeric",
                    aria_invalid: if seed_invalid { "true" },
                    aria_describedby: "{PARTICIPANT_SEED_HINT_ID} {PARTICIPANT_SEED_ERROR_ID}",
                    value: "{add_part_seed}",
                    oninput: move |e| add_part_seed.set(e.value()),
                }
                p { class: "field-hint", id: PARTICIPANT_SEED_HINT_ID, "Leave blank for no seed." }
                FieldError { id: PARTICIPANT_SEED_ERROR_ID.to_string(), text: add_part_seed_error() }
            }
        }

        // Remove participant confirm
        ConfirmDialog {
            title: "Remove participant".to_string(),
            message: format!(
                "Remove \"{}\" from this tournament?",
                remove_part_modal
                    .get_target()
                    .map(|p| {
                        row_labels
                            .get(&p.id)
                            .cloned()
                            .unwrap_or_else(|| UNNAMED_ENTRANT.to_string())
                    })
                    .unwrap_or_default()
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
                        let pmap = &participant_map;
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
            "PARTICIPANT_TEAM_ID",
            "PARTICIPANT_NAME_ID",
            "PARTICIPANT_SEED_ID",
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

    #[test]
    fn add_participant_sends_what_the_server_takes() {
        // Issue 689: the server reads team_id, external_name and seed, and
        // answers 400 when both team_id and external_name are missing.
        let team = add_participant_body(" team:alpha ", "", "").unwrap();
        assert_eq!(
            serde_json::to_value(&team).unwrap(),
            serde_json::json!({ "team_id": "team:alpha" })
        );
        let outside = add_participant_body("", "  Rival Squad ", " 3 ").unwrap();
        assert_eq!(
            serde_json::to_value(&outside).unwrap(),
            serde_json::json!({ "external_name": "Rival Squad", "seed": 3 })
        );
        let entrant_only = |text| AddParticipantErrors {
            entrant: Some(text),
            seed: None,
        };
        let seed_word = SEED_ERROR;
        assert!(seed_word.contains("1 or higher"), "{seed_word}");
        assert!(seed_word.contains("blank for no seed"), "{seed_word}");
        assert_eq!(
            add_participant_body("", "  ", "1"),
            Err(entrant_only("Pick a team or type a name."))
        );
        // Round 4 (owner decision): a team and a typed name together send
        // both; the team stays linked and the typed name is the one shown.
        // This replaces the old "not both" refusal this test pinned.
        let both = add_participant_body(" team:alpha ", " Alpha Event Roster ", "").unwrap();
        assert_eq!(
            serde_json::to_value(&both).unwrap(),
            serde_json::json!({ "team_id": "team:alpha", "external_name": "Alpha Event Roster" })
        );
        // A blank or whitespace seed is unseeded: the field is left out.
        for blank in ["", "   "] {
            let body = add_participant_body("team:alpha", "", blank).unwrap();
            assert_eq!(body.seed, None, "{blank:?}");
            assert!(
                serde_json::to_value(&body).unwrap().get("seed").is_none(),
                "{blank:?}"
            );
        }
        // Any whole number from 1 up to the largest u32 is taken.
        for (typed, want) in [("1", 1), (" 42 ", 42), ("4294967295", u32::MAX)] {
            let body = add_participant_body("", "Rival Squad", typed).unwrap();
            assert_eq!(body.seed, Some(want), "{typed}");
            assert_eq!(
                serde_json::to_value(&body).unwrap()["seed"],
                serde_json::json!(want),
                "{typed}"
            );
        }
        for bad in ["0", "-1", "1.5", "first", "1e", "-", "4294967296"] {
            assert_eq!(
                add_participant_body("team:alpha", "", bad),
                Err(AddParticipantErrors {
                    entrant: None,
                    seed: Some(seed_word),
                }),
                "{bad}"
            );
        }
        // Review on PR 828 nit 3: both errors in one pass.
        assert_eq!(
            add_participant_body("", "", "first"),
            Err(AddParticipantErrors {
                entrant: Some("Pick a team or type a name."),
                seed: Some(seed_word),
            })
        );
        let src = include_str!("tournaments.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(!prod.contains("member_id"));
        assert!(!prod.contains("/api/members"));
    }

    #[test]
    fn the_add_participant_form_groups_the_entrant_and_takes_typed_seeds() {
        // Review on PR 828 nits 2, 3, 4, 6 and 9.
        let src = include_str!("tournaments.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        // One named group with a tied hint, which also carries a teams
        // load error, around Team and Name to show.
        assert!(prod.contains(
            "fieldset { class: \"admin-check-group\", aria_describedby: PARTICIPANT_HINT_ID,\n                legend { class: \"field-label\", \"Entrant\" }\n                p { class: \"field-hint\", id: PARTICIPANT_HINT_ID, \"{entrant_hint}\" }"
        ));
        assert!(
            prod.contains("let entrant_hint = entrant_hint_text(teams.error.read().as_deref());")
        );
        // The seed is a text box, so text a number box would drop as ""
        // reaches the validator.
        let seed = prod
            .split("id: PARTICIPANT_SEED_ID,")
            .nth(1)
            .and_then(|rest| rest.split("value:").next())
            .expect("the seed input");
        assert!(seed.contains("r#type: \"text\","), "{seed}");
        assert!(seed.contains("inputmode: \"numeric\","), "{seed}");
        assert!(!seed.contains("\"number\""), "{seed}");
        // A failed check is heard: a toast and focus on the first field.
        assert!(prod.contains("toast.show(Toast::error(\"Check the participant fields.\"));"));
        assert!(prod.contains("focus_element_by_id(if errors.entrant.is_some() {"));
        // Both dialogs borrow the one name map.
        assert!(!prod.contains("dialog_names"));
        assert!(!prod.contains("participant_map.clone()"));
        // The picker lists the filtered teams only.
        assert!(prod.contains("for team in team_options.iter() {"));
    }

    #[test]
    fn the_team_picker_leaves_out_entered_teams_and_other_games() {
        // Issue 850: a team already entered hits the server's unique index.
        let team = |id: &str, game: Option<&str>| Team {
            id: id.into(),
            name: id.to_uppercase(),
            game_id: game.map(str::to_string),
        };
        let teams = vec![
            team("team:alpha", Some("game:ow")),
            team("team:beta", Some("ow")),
            team("team:gamma", Some("game:rivals")),
            team("team:delta", None),
            // The server's `Team.game_id` is a plain string: no game reads "".
            team("team:epsilon", Some("")),
        ];
        let participant = |id: &str, team_id: Option<&str>| {
            serde_json::json!({
                "id": id,
                "team_id": team_id,
                "external_name": if team_id.is_none() { Some("Rival Squad") } else { None },
                "seed": null,
                "status": "registered"
            })
        };
        let bracket: BracketData = serde_json::from_value(serde_json::json!({
            "participants": [participant("p1", Some("team:alpha")), participant("p2", None)],
            "rounds": [],
            "matches": []
        }))
        .unwrap();
        let ids = |list: Vec<Team>| list.into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(
            ids(team_choices(&teams, Some(&bracket), None)),
            ["team:beta", "team:gamma", "team:delta", "team:epsilon"]
        );
        // The tournament's game keeps that game's teams (either id form) and
        // teams whose game is not known.
        assert_eq!(
            ids(team_choices(&teams, Some(&bracket), Some("ow"))),
            ["team:beta", "team:delta", "team:epsilon"]
        );
        assert_eq!(
            ids(team_choices(&teams, None, Some("game:ow"))),
            ["team:alpha", "team:beta", "team:delta", "team:epsilon"]
        );
        assert_eq!(ids(team_choices(&teams, None, Some(" "))).len(), 5);
    }

    #[test]
    fn every_participant_row_has_its_own_remove_name() {
        // QA Q3: two nameless entrants (no team and no name, or a blank
        // name) and two with the same outside name must not share a name.
        let entrant = |id: &str, team_id: Option<&str>, external: Option<&str>| {
            serde_json::json!({
                "id": id,
                "team_id": team_id,
                "external_name": external,
                "seed": null,
                "status": "registered"
            })
        };
        let bracket: BracketData = serde_json::from_value(serde_json::json!({
            "participants": [
                entrant("p1", None, None),
                entrant("p2", Some("team:alpha"), None),
                entrant("p3", None, Some("   ")),
                entrant("p4", None, Some("Guests")),
                entrant("p5", None, Some("Guests")),
                // A unique name that reads like a numbered one.
                entrant("p6", None, Some("Guests, row 4")),
            ],
            "rounds": [],
            "matches": []
        }))
        .unwrap();
        // No teams list: the team entrant reads "Unlisted team".
        let names = participant_name_map(Some(&bracket), None);
        let labels = participant_row_labels(&bracket.participants, &names);
        let label = |id: &str| labels.get(id).map(String::as_str).unwrap();
        assert_eq!(label("p1"), "Unnamed entrant, row 1");
        assert_eq!(label("p3"), "Unnamed entrant, row 3");
        // A name that is unique stays as it is.
        assert_eq!(label("p2"), "Unlisted team");
        assert_eq!(label("p4"), "Guests, row 4");
        assert_eq!(label("p5"), "Guests, row 5");
        assert_eq!(label("p6"), "Guests, row 4, row 6");
        let distinct: std::collections::HashSet<&String> = labels.values().collect();
        assert_eq!(distinct.len(), bracket.participants.len());
        assert_eq!(labels.len(), bracket.participants.len());

        // The Remove button and its dialog both use the row name.
        let src = include_str!("tournaments.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(prod.contains("aria_label: \"Remove {row_label}\","));
        assert!(!prod.contains("aria_label: \"Remove {pname}\""));
        let dialog = prod
            .split("\"Remove \\\"{}\\\" from this tournament?\",")
            .nth(1)
            .and_then(|rest| rest.split("open:").next())
            .expect("the Remove dialog message");
        assert!(dialog.contains("row_labels"), "{dialog}");
    }

    #[test]
    fn a_team_with_a_typed_name_shows_the_typed_name() {
        // Round 4: with a team and a typed name, the typed name is what the
        // table and the Remove button and dialog use.
        let bracket: BracketData = serde_json::from_value(serde_json::json!({
            "participants": [
                {
                    "id": "p1",
                    "team_id": "team:alpha",
                    "external_name": "Alpha Event Roster",
                    "seed": null,
                    "status": "registered"
                },
                {
                    "id": "p2",
                    "team_id": "team:alpha2",
                    "external_name": null,
                    "seed": 2,
                    "status": "registered"
                }
            ],
            "rounds": [],
            "matches": []
        }))
        .unwrap();
        let teams = vec![
            Team {
                id: "team:alpha".into(),
                name: "Squad Alpha".into(),
                game_id: None,
            },
            Team {
                id: "team:alpha2".into(),
                name: "Squad Alpha Two".into(),
                game_id: None,
            },
        ];
        let names = participant_name_map(Some(&bracket), Some(&teams));
        assert_eq!(names["p1"], "Alpha Event Roster");
        assert_eq!(names["p2"], "Squad Alpha Two");
        let labels = participant_row_labels(&bracket.participants, &names);
        assert_eq!(labels["p1"], "Alpha Event Roster");
        assert_eq!(labels["p2"], "Squad Alpha Two");
        // A linked team counts as entered, so the picker hides it.
        let ids: Vec<String> = team_choices(&teams, Some(&bracket), None)
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(ids.is_empty(), "{ids:?}");
    }

    #[test]
    fn the_add_participant_form_allows_both_and_an_optional_seed() {
        // Round 4: no "not both" copy is left anywhere in the page code,
        // and the labels and hints say what the fields take.
        let src = include_str!("tournaments.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        let lower = prod.to_lowercase();
        assert!(!lower.contains("not both"));
        assert!(!lower.contains("never both"));
        let hint = entrant_hint_text(None);
        assert!(hint.contains("or both"), "{hint}");
        assert!(
            hint.contains("the typed name is shown and the entry stays linked to your team"),
            "{hint}"
        );
        assert!(prod.contains(
            "label { class: \"field-label\", r#for: PARTICIPANT_NAME_ID, \"Name to show\" }"
        ));
        assert!(prod.contains(
            "label { class: \"field-label\", r#for: PARTICIPANT_SEED_ID, \"Seed (optional)\" }"
        ));
        // The seed hint is always rendered and tied, next to the error line.
        let seed = prod
            .split("id: PARTICIPANT_SEED_ID,")
            .nth(1)
            .and_then(|rest| rest.split("// Remove participant confirm").next())
            .expect("the seed field");
        assert!(
            seed.contains(
                "aria_describedby: \"{PARTICIPANT_SEED_HINT_ID} {PARTICIPANT_SEED_ERROR_ID}\","
            ),
            "{seed}"
        );
        assert!(
            seed.contains(
                "p { class: \"field-hint\", id: PARTICIPANT_SEED_HINT_ID, \"Leave blank for no seed.\" }"
            ),
            "{seed}"
        );
        assert!(seed.contains("FieldError { id: PARTICIPANT_SEED_ERROR_ID.to_string(),"));
    }

    #[test]
    fn the_entrant_hint_names_the_hidden_teams_and_joins_a_load_error() {
        // Review on PR 828 round 3, new nits 1 and 3.
        let hint = entrant_hint_text(None);
        assert!(hint.starts_with("Pick one of your teams"), "{hint}");
        assert!(
            hint.contains("Teams already entered and teams of another game are not listed"),
            "{hint}"
        );
        assert!(hint.contains("type its name"), "{hint}");
        // A reason with no full stop does not run into the next sentence.
        assert_eq!(
            entrant_hint_text(Some("HTTP error 500: Internal error")),
            "Could not load the teams list. HTTP error 500: Internal error. \
             You can still type an outside team name."
        );
        assert_eq!(
            entrant_hint_text(Some("Offline.")),
            "Could not load the teams list. Offline. You can still type an outside team name."
        );
        assert!(!entrant_hint_text(Some("")).contains(".."));
    }

    #[test]
    fn participants_and_bracket_claim_empty_only_once_loaded() {
        // Review on PR 828 round 2 nit 7 and round 3 new nit 5: the empty
        // line shows for a loaded, empty bracket only; a loading line while
        // it loads; nothing after a failed load (the alert says why).
        let src = include_str!("tournaments.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        let section = |title: &str| {
            prod.split(&format!("aria_labelledby: \"{title}\""))
                .nth(1)
                .and_then(|rest| rest.split("section {").next())
                .unwrap_or_else(|| panic!("the {title} section"))
                .to_string()
        };
        let parts = section("tournament-participants-title");
        assert!(parts.contains("match bracket.as_ref().map(|b| &b.participants) {"));
        assert!(parts.contains(
            "Some(_) => rsx! {\n                            p { class: \"field-hint\", \"No participants yet."
        ));
        assert!(parts.contains(
            "None if detail_loading() => rsx! {\n                            p { class: \"field-hint\", \"Loading participants...\" }"
        ));
        assert!(parts.contains("None => rsx! {},"));
        let bracket = section("tournament-bracket-title");
        assert!(bracket.contains("match bracket.as_ref().map(|b| (&b.rounds, &b.matches)) {"));
        assert!(bracket.contains("Some((rnds, mtchs)) if !rnds.is_empty() => rsx! {"));
        assert!(bracket.contains(
            "Some(_) => rsx! {\n                            p { class: \"field-hint\", \"No bracket yet."
        ));
        assert!(bracket.contains(
            "None if detail_loading() => rsx! {\n                            p { class: \"field-hint\", \"Loading bracket...\" }"
        ));
        assert!(bracket.contains("None => rsx! {},"));
        assert!(!bracket.contains("_ => rsx! {"));
        assert_eq!(prod.matches("\"No bracket yet.").count(), 1);
    }

    #[test]
    fn a_bracket_with_participants_decodes_and_names_them() {
        // The server's `TournamentBracket`, field for field (db/src/types.rs):
        // `Tournament`, `TournamentParticipant`, `TournamentRound` and
        // `TournamentMatch`.
        let json = serde_json::json!({
            "tournament": {
                "id": "t1",
                "name": "Winter Cup",
                "game_id": null,
                "format": "single_elim",
                "status": "registration",
                "max_teams": 8,
                "best_of": 3,
                "swiss_rounds": null,
                "is_external": false,
                "is_open": true,
                "external_url": null,
                "rules": null,
                "description": null,
                "starts_at": null,
                "ends_at": null,
                "created_by": "member:1",
                "created_at": "2026-11-01T18:00:00Z",
                "updated_at": "2026-11-01T18:00:00Z"
            },
            "participants": [
                {
                    "id": "p1",
                    "tournament_id": "t1",
                    "team_id": "team:alpha",
                    "external_name": null,
                    "seed": 1,
                    "group_label": null,
                    "status": "registered",
                    "created_at": "2026-11-02T18:00:00Z"
                },
                {
                    "id": "p2",
                    "tournament_id": "t1",
                    "team_id": null,
                    "external_name": "Rival Squad",
                    "seed": null,
                    "group_label": null,
                    "status": "checked_in",
                    "created_at": "2026-11-02T18:05:00Z"
                },
                {
                    "id": "p3",
                    "tournament_id": "t1",
                    "team_id": "team:gone",
                    "external_name": null,
                    "seed": null,
                    "group_label": null,
                    "status": "registered",
                    "created_at": "2026-11-02T18:06:00Z"
                }
            ],
            "rounds": [
                {
                    "id": "r1",
                    "tournament_id": "t1",
                    "round_number": 1,
                    "stage": "winners",
                    "status": "in_progress",
                    "created_at": "2026-11-03T18:00:00Z"
                }
            ],
            "matches": [
                {
                    "id": "m1",
                    "tournament_id": "t1",
                    "round_id": "r1",
                    "bracket_position": 1,
                    "participant_a_id": "p1",
                    "participant_b_id": "p2",
                    "score_a": 2,
                    "score_b": 1,
                    "winner_id": "p1",
                    "status": "completed",
                    "scheduled_at": null,
                    "completed_at": "2026-11-03T19:00:00Z",
                    "match_result_id": null,
                    "next_match_id": null,
                    "next_match_slot": null,
                    "loser_next_match_id": null,
                    "loser_next_match_slot": null,
                    "notes": null,
                    "replay_codes": ["A1B2C3"]
                }
            ]
        });
        let bracket: BracketData =
            serde_json::from_value(json).expect("the server's shape decodes");
        assert_eq!(bracket.participants.len(), 3);
        assert_eq!(bracket.rounds[0].round_number, 1);
        assert_eq!(bracket.rounds[0].stage, "winners");
        let m = &bracket.matches[0];
        assert_eq!(m.round_id, "r1");
        assert_eq!(m.participant_a_id.as_deref(), Some("p1"));
        assert_eq!((m.score_a, m.score_b), (Some(2), Some(1)));
        assert_eq!(m.winner_id.as_deref(), Some("p1"));
        assert_eq!(m.status, "completed");
        assert_eq!(seed_label(bracket.participants[0].seed), "1");
        assert_eq!(seed_label(bracket.participants[1].seed), "Unseeded");

        let teams = vec![Team {
            id: "team:alpha".into(),
            name: "Alpha".into(),
            game_id: None,
        }];
        let names = participant_name_map(Some(&bracket), Some(&teams));
        assert_eq!(names["p1"], "Alpha");
        assert_eq!(names["p2"], "Rival Squad");
        // A team the list does not hold reads a word, not its record key
        // (review on PR 828 nit 6; this used to pin the raw key).
        assert_eq!(names["p3"], crate::pages::tournament::UNLISTED_TEAM);
        assert!(participant_name_map(None, Some(&teams)).is_empty());
    }
}
