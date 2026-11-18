use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pager, Pill, PillTone, page_slice,
    show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, AdminDate, DataTable, FormModal, RowActions, SaveState, Toast,
    admin_list_pending, admin_pending, list_cap_notice, status_label, use_toast,
};
use crate::hooks::{ModalController, use_api_list, use_api_list_with};
use crate::routes::Route;
use scuffed_api_client::ApiClient;
use scuffed_types::api::MatchPayload;

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
    #[serde(default)]
    score_us: Option<u32>,
    #[serde(default)]
    score_them: Option<u32>,
    map_name: Option<String>,
    match_type: String,
    #[serde(default)]
    played_at: Option<String>,
    #[serde(default)]
    scheduled_at: Option<String>,
    notes: Option<String>,
    #[serde(default)]
    is_public: bool,
    #[serde(default)]
    vod_url: Option<String>,
    #[serde(default)]
    replay_code: Option<String>,
}

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-matches-list";
const TEAM_SELECT_ID: &str = "matches-team";
const NEW_MATCH_ID: &str = "matches-new";
const OPPONENT_ID: &str = "match-opponent";
const SCORE_US_ID: &str = "match-score-us";
const SCORE_US_HINT_ID: &str = "match-score-us-hint";
const SCORE_THEM_ID: &str = "match-score-them";
const MAP_ID: &str = "match-map";
const TYPE_ID: &str = "match-type";
const PLAYED_ID: &str = "match-played-at";
const SCHEDULED_ID: &str = "match-scheduled-at";
const WHEN_ERROR_ID: &str = "match-when-error";
const VOD_ID: &str = "match-vod-url";
const VOD_HINT_ID: &str = "match-vod-url-hint";
const REPLAY_ID: &str = "match-replay-code";
const REPLAY_HINT_ID: &str = "match-replay-code-hint";
const NOTES_ID: &str = "match-notes";
const PUBLIC_ID: &str = "match-public";

/// Page CSS: the team picker sits at a readable width on desktop.
const PAGE_CSS: &str = r#"
.admin-matches-page .matches-team-picker {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: var(--container-sm);
}
"#;

/// Server expects RFC3339 `DateTime<Utc>`; date input is YYYY-MM-DD.
fn date_to_rfc3339(date: &str) -> Option<String> {
    let d = date.trim();
    if d.is_empty() {
        return None;
    }
    if d.contains('T') {
        if d.ends_with('Z') || d.contains('+') {
            return Some(d.to_string());
        }
        if d.len() == 16 {
            // YYYY-MM-DDTHH:MM
            return Some(format!("{d}:00Z"));
        }
        return Some(format!("{d}Z"));
    }
    Some(format!("{d}T00:00:00Z"))
}

/// The result of a match as a word and a tone: the tone is never the only
/// signal, the pill always says the word.
fn match_result(score_us: Option<u32>, score_them: Option<u32>) -> (&'static str, PillTone) {
    match (score_us, score_them) {
        (Some(us), Some(them)) if us > them => ("Win", PillTone::Ok),
        (Some(us), Some(them)) if us < them => ("Loss", PillTone::Danger),
        (Some(_), Some(_)) => ("Draw", PillTone::Neutral),
        _ => ("Scheduled", PillTone::Info),
    }
}

/// A match with no score is still to be played.
fn is_unplayed(m: &MatchResult) -> bool {
    m.score_us.is_none() || m.score_them.is_none()
}

/// Matches still to be played first (soonest first), then played ones
/// newest first, so the date column reads in order.
fn sorted_matches(all: &[MatchResult]) -> Vec<MatchResult> {
    let date = |m: &MatchResult| {
        m.played_at
            .clone()
            .or(m.scheduled_at.clone())
            .unwrap_or_default()
    };
    let mut upcoming: Vec<MatchResult> = all.iter().filter(|m| is_unplayed(m)).cloned().collect();
    let mut played: Vec<MatchResult> = all.iter().filter(|m| !is_unplayed(m)).cloned().collect();
    upcoming.sort_by_key(date);
    played.sort_by_key(|m| std::cmp::Reverse(date(m)));
    upcoming.extend(played);
    upcoming
}

/// Id of a row's Edit button, from the match id.
fn match_edit_button_id(match_id: &str) -> String {
    format!("match-edit-{match_id}")
}

#[component]
pub fn AdminMatches() -> Element {
    let teams = use_api_list::<Team>("/api/teams");
    let mut toast = use_toast();
    let mut page = use_signal(|| 1usize);

    // Team selector
    let mut selected_team = use_signal(|| None::<String>);

    let mut matches = use_api_list_with::<MatchResult>(move || match selected_team() {
        Some(id) => format!("/api/teams/{id}/matches"),
        None => String::new(),
    });

    // Form modal state
    let mut modal = ModalController::<String>::new();
    let mut when_error = use_signal(String::new);
    let mut save_error = use_signal(|| None::<String>);
    let mut opener = use_signal(|| NEW_MATCH_ID.to_string());

    // Form fields
    let mut f_opponent = use_signal(String::new);
    let mut f_score_us = use_signal(String::new);
    let mut f_score_them = use_signal(String::new);
    let mut f_map_name = use_signal(String::new);
    let mut f_match_type = use_signal(|| "scrim".to_string());
    let mut f_played_at = use_signal(String::new);
    let mut f_scheduled_at = use_signal(String::new);
    let mut f_notes = use_signal(String::new);
    let mut f_public = use_signal(|| false);
    let mut f_vod_url = use_signal(String::new);
    let mut f_replay_code = use_signal(String::new);

    let open_create = move |_| {
        f_opponent.set(String::new());
        f_score_us.set(String::new());
        f_score_them.set(String::new());
        f_map_name.set(String::new());
        f_match_type.set("scrim".to_string());
        f_played_at.set(String::new());
        f_scheduled_at.set(String::new());
        f_notes.set(String::new());
        f_public.set(false);
        f_vod_url.set(String::new());
        f_replay_code.set(String::new());
        when_error.set(String::new());
        save_error.set(None);
        opener.set(NEW_MATCH_ID.to_string());
        modal.show_empty();
    };

    let mut open_edit = move |m: MatchResult| {
        opener.set(match_edit_button_id(&m.id));
        f_opponent.set(m.opponent);
        f_score_us.set(m.score_us.map(|s| s.to_string()).unwrap_or_default());
        f_score_them.set(m.score_them.map(|s| s.to_string()).unwrap_or_default());
        f_map_name.set(m.map_name.unwrap_or_default());
        f_match_type.set(m.match_type);
        f_played_at.set(
            m.played_at
                .map(|s| s.chars().take(10).collect::<String>())
                .unwrap_or_default(),
        );
        f_scheduled_at.set(
            m.scheduled_at
                .map(|s| s.chars().take(10).collect::<String>())
                .unwrap_or_default(),
        );
        f_notes.set(m.notes.unwrap_or_default());
        f_public.set(m.is_public);
        f_vod_url.set(m.vod_url.unwrap_or_default());
        f_replay_code.set(m.replay_code.unwrap_or_default());
        when_error.set(String::new());
        save_error.set(None);
        modal.show(m.id);
    };

    let on_submit = move |_| {
        let team_id = match selected_team() {
            Some(id) => id,
            None => return,
        };
        let edit_id = modal.get_target();
        let opponent = f_opponent().clone();
        let score_us: Option<u32> = {
            let s = f_score_us();
            if s.trim().is_empty() {
                None
            } else {
                Some(s.parse().unwrap_or(0))
            }
        };
        let score_them: Option<u32> = {
            let s = f_score_them();
            if s.trim().is_empty() {
                None
            } else {
                Some(s.parse().unwrap_or(0))
            }
        };
        let map_name_val = f_map_name().clone();
        let match_type_val = f_match_type().clone();
        let played_at_val = date_to_rfc3339(&f_played_at());
        let scheduled_at_val = date_to_rfc3339(&f_scheduled_at());
        let notes_val = f_notes().clone();
        let is_public = f_public();
        let vod_url_val = f_vod_url().clone();
        let replay_code_val = f_replay_code().clone();
        save_error.set(None);

        if played_at_val.is_none()
            && scheduled_at_val.is_none()
            && score_us.is_none()
            && score_them.is_none()
        {
            when_error.set("Set a played date, a scheduled date, or the scores.".to_string());
            toast.show(Toast::error("Provide played-at, scheduled-at, or scores."));
            return;
        }
        when_error.set(String::new());

        modal.start_submit();
        spawn(async move {
            let payload = MatchPayload {
                team_id,
                opponent,
                score_us,
                score_them,
                map_name: if map_name_val.is_empty() {
                    None
                } else {
                    Some(map_name_val)
                },
                game_mode: None,
                match_type: match_type_val,
                played_at: played_at_val,
                scheduled_at: scheduled_at_val,
                notes: if notes_val.is_empty() {
                    None
                } else {
                    Some(notes_val)
                },
                is_public,
                vod_url: if vod_url_val.is_empty() {
                    None
                } else {
                    Some(vod_url_val)
                },
                replay_code: if replay_code_val.is_empty() {
                    None
                } else {
                    Some(replay_code_val)
                },
            };

            let result = match edit_id {
                Some(id) => {
                    let path = format!("/api/matches/{id}");
                    ApiClient::web()
                        .put_json::<_, MatchResult>(&path, &payload)
                        .await
                }
                None => {
                    ApiClient::web()
                        .post_json::<_, MatchResult>("/api/matches", &payload)
                        .await
                }
            };

            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Match saved"));
                    modal.close();
                    matches.refresh += 1;
                }
                Err(e) => {
                    save_error.set(Some(
                        SaveState::failed("Could not save the match", &e).text(),
                    ));
                    toast.show(Toast::error(format!("Failed to save: {e}")));
                }
            }
        });
    };

    let modal_title = if modal.get_target().is_some() {
        "Edit match"
    } else {
        "New match"
    };

    let team_name = {
        let data = teams.data.read();
        let selected = selected_team();
        data.as_ref()
            .and_then(|d| d.as_ref())
            .and_then(|list| {
                list.iter()
                    .find(|t| Some(&t.id) == selected.as_ref())
                    .map(|t| t.name.clone())
            })
            .unwrap_or_else(|| "this team".to_string())
    };

    let list = if selected_team().is_none() {
        let data = teams.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_pending(&teams, "teams"),
            Some(list) if list.is_empty() => rsx! {
                EmptyCard {
                    title: "No teams yet",
                    message: "Matches belong to a team. Create a team first, then record its matches here.",
                    actions: rsx! {
                        Link { to: Route::AdminTeams {}, class: "btn is-ghost", "Go to Teams" }
                    },
                }
            },
            Some(_) => rsx! {
                EmptyCard {
                    title: "Pick a team",
                    message: "Choose a team above to see its matches and record new ones.",
                }
            },
        }
    } else {
        let data = matches.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&matches, "matches"),
            Some(all) => {
                let total = all.len();
                let ordered = sorted_matches(all);
                let rows = page_slice(&ordered, page(), ADMIN_PAGE_SIZE).to_vec();
                rsx! {
                    if all.is_empty() {
                        EmptyCard {
                            title: "No matches yet",
                            message: format!("No match is recorded for {team_name}. Add a scrim, an official match or a tournament game."),
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-primary",
                                    onclick: open_create,
                                    "New match"
                                }
                            },
                        }
                    } else {
                        DataTable {
                            headers: vec!["Opponent", "Result", "Score", "Map", "Type", "Visibility", "Date", "Actions"],
                            num_cols: vec![2, 6],
                            cards: true,
                            label: "Matches",
                            for m in rows.iter() {
                                Fragment { key: "{m.id}", {
                                    let mc = m.clone();
                                    // A scheduled match says so once (its pill); no "No
                                    // score" or "No map" noise in its card.
                                    let score = match (m.score_us, m.score_them) {
                                        (Some(u), Some(t)) => format!("{u}-{t}"),
                                        _ => String::new(),
                                    };
                                    let (result, tone) = match_result(m.score_us, m.score_them);
                                    let map = m.map_name.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
                                        if is_unplayed(m) { String::new() } else { "No map".to_string() }
                                    });
                                    let date = m.played_at.clone().or(m.scheduled_at.clone());
                                    let kind = status_label(&m.match_type);
                                    let public_label = if m.is_public { "Public" } else { "Members only" };
                                    let opponent = m.opponent.clone();
                                    rsx! {
                                        tr { key: "{m.id}",
                                            td { class: "cell-who", "{m.opponent}" }
                                            td { class: "cell-badge", Pill { tone, "{result}" } }
                                            td { class: "cell-meta num", "{score}" }
                                            td { class: "cell-meta", "{map}" }
                                            td { class: "cell-meta", "{kind}" }
                                            td { class: "cell-meta", "{public_label}" }
                                            td { class: "cell-meta num",
                                                match date {
                                                    Some(iso) => rsx! { AdminDate { iso } },
                                                    None => rsx! { span { class: "date", "Date not set" } },
                                                }
                                            }
                                            td { class: "cell-actions",
                                                RowActions {
                                                    button {
                                                        r#type: "button",
                                                        class: "btn is-ghost",
                                                        id: match_edit_button_id(&m.id),
                                                        aria_label: "Edit match against {opponent}",
                                                        onclick: move |_| open_edit(mc.clone()),
                                                        "Edit"
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
                        id: "matches-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "matches",
                        noun_singular: "match",
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

    let when_invalid = !when_error().is_empty();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-matches-page-css", css: PAGE_CSS }
        PageShell { width: PageWidth::Wide, class: "admin-matches-page",
            PageHeader {
                title: "Matches",
                lead: "Pick a team to see its matches, then record results or schedule the next one.",
                actions: selected_team().is_some().then(|| rsx! {
                    button {
                        r#type: "button",
                        class: "btn is-primary",
                        id: NEW_MATCH_ID,
                        onclick: open_create,
                        "New match"
                    }
                }),
            }

            div { class: "matches-team-picker",
                label { class: "field-label", r#for: TEAM_SELECT_ID, "Team" }
                select {
                    class: "field-input",
                    id: TEAM_SELECT_ID,
                    value: "{selected_team().unwrap_or_default()}",
                    onchange: move |e| {
                        let val = e.value();
                        selected_team.set(if val.is_empty() { None } else { Some(val) });
                        page.set(1);
                    },
                    option { value: "", "Pick a team" }
                    {
                        let data = teams.data.read();
                        let data = data.as_ref().and_then(|d| d.as_ref());
                        match data {
                            Some(list) => rsx! {
                                for t in list.iter() {
                                    option { key: "{t.id}", value: "{t.id}", "{t.name}" }
                                }
                            },
                            None => rsx! {},
                        }
                    }
                }
            }

            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Matches",
                {list}
            }

            {list_cap_notice(&teams, "teams")}
            {list_cap_notice(&matches, "matches")}
        }

        FormModal {
            title: modal_title.to_string(),
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: move |_| modal.close(),
            on_submit: on_submit,
            wide: true,
            submit_label: modal.get_target().is_none().then(|| "Create".to_string()),
            error: save_error(),
            restore_focus_id: Some(opener()),

            div { class: "form-grid",
                div { class: "form-field span-full",
                    label { class: "field-label", r#for: OPPONENT_ID, "Opponent" }
                    input {
                        class: "field-input",
                        id: OPPONENT_ID,
                        r#type: "text",
                        value: "{f_opponent}",
                        oninput: move |e| f_opponent.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: SCORE_US_ID, "Our score" }
                    input {
                        class: "field-input",
                        id: SCORE_US_ID,
                        r#type: "number",
                        inputmode: "numeric",
                        aria_describedby: SCORE_US_HINT_ID,
                        value: "{f_score_us}",
                        oninput: move |e| f_score_us.set(e.value()),
                    }
                    p { class: "field-hint", id: SCORE_US_HINT_ID, "Leave both scores empty for a match that is still to come." }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: SCORE_THEM_ID, "Their score" }
                    input {
                        class: "field-input",
                        id: SCORE_THEM_ID,
                        r#type: "number",
                        inputmode: "numeric",
                        value: "{f_score_them}",
                        oninput: move |e| f_score_them.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: MAP_ID, "Map (optional)" }
                    input {
                        class: "field-input",
                        id: MAP_ID,
                        r#type: "text",
                        value: "{f_map_name}",
                        oninput: move |e| f_map_name.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: TYPE_ID, "Type" }
                    select {
                        class: "field-input",
                        id: TYPE_ID,
                        value: "{f_match_type}",
                        onchange: move |e| f_match_type.set(e.value()),
                        // Must match scuffed_db::MatchType / schema ASSERT
                        option { value: "scrim", "Scrim" }
                        option { value: "official", "Official" }
                        option { value: "tournament", "Tournament" }
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: PLAYED_ID, "Played on" }
                    input {
                        class: "field-input",
                        id: PLAYED_ID,
                        r#type: "date",
                        aria_invalid: if when_invalid { "true" },
                        aria_describedby: WHEN_ERROR_ID,
                        value: "{f_played_at}",
                        oninput: move |e| f_played_at.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: SCHEDULED_ID, "Scheduled for" }
                    input {
                        class: "field-input",
                        id: SCHEDULED_ID,
                        r#type: "date",
                        aria_invalid: if when_invalid { "true" },
                        aria_describedby: WHEN_ERROR_ID,
                        value: "{f_scheduled_at}",
                        oninput: move |e| f_scheduled_at.set(e.value()),
                    }
                }
                div { class: "span-full",
                    FieldError { id: WHEN_ERROR_ID.to_string(), text: when_error() }
                }
                div { class: "form-field span-full",
                    label { class: "field-label", r#for: VOD_ID, "VOD link (optional)" }
                    input {
                        class: "field-input",
                        id: VOD_ID,
                        r#type: "url",
                        aria_describedby: VOD_HINT_ID,
                        value: "{f_vod_url}",
                        oninput: move |e| f_vod_url.set(e.value()),
                        placeholder: "https://www.youtube.com/watch?v=",
                    }
                    p { class: "field-hint", id: VOD_HINT_ID, "An https link to YouTube or Twitch." }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: REPLAY_ID, "Replay code (optional)" }
                    input {
                        class: "field-input",
                        id: REPLAY_ID,
                        r#type: "text",
                        aria_describedby: REPLAY_HINT_ID,
                        value: "{f_replay_code}",
                        oninput: move |e| f_replay_code.set(e.value()),
                        maxlength: "16",
                    }
                    p { class: "field-hint", id: REPLAY_HINT_ID, "Up to 16 letters and digits." }
                }
                div { class: "form-field span-full",
                    label { class: "field-label", r#for: NOTES_ID, "Notes (optional)" }
                    textarea {
                        class: "field-input",
                        id: NOTES_ID,
                        value: "{f_notes}",
                        oninput: move |e| f_notes.set(e.value()),
                    }
                }
            }
            label { class: "admin-check-row", r#for: PUBLIC_ID,
                input {
                    r#type: "checkbox",
                    id: PUBLIC_ID,
                    checked: f_public(),
                    onchange: move |e| f_public.set(e.checked()),
                }
                "Public (show on the team page)"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(
        id: &str,
        score: Option<(u32, u32)>,
        played: Option<&str>,
        scheduled: Option<&str>,
    ) -> MatchResult {
        MatchResult {
            id: id.into(),
            opponent: format!("Team {id}"),
            score_us: score.map(|s| s.0),
            score_them: score.map(|s| s.1),
            map_name: None,
            match_type: "scrim".into(),
            played_at: played.map(Into::into),
            scheduled_at: scheduled.map(Into::into),
            notes: None,
            is_public: true,
            vod_url: None,
            replay_code: None,
        }
    }

    #[test]
    fn upcoming_matches_come_first_then_played_newest_first() {
        let all = vec![
            game("old", Some((2, 1)), Some("2026-11-01T19:00:00Z"), None),
            game("new", Some((0, 2)), Some("2026-11-12T19:00:00Z"), None),
            game("later", None, None, Some("2026-11-30T19:00:00Z")),
            game("soon", None, None, Some("2026-11-20T19:00:00Z")),
        ];
        let ids: Vec<String> = sorted_matches(&all).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["soon", "later", "new", "old"]);
    }

    /// This sheet sets no type, so the shared check (which needs a
    /// font-size) does not apply: no rem, no px past a hairline, no hex.
    fn assert_page_css_on_tokens(css: &str) {
        for word in css.split(|c: char| c.is_whitespace() || ";:(),".contains(c)) {
            assert!(!word.ends_with("rem"), "{word}");
            assert!(!word.ends_with("px") || word == "1px", "{word}");
            assert!(!word.starts_with('#'), "{word}");
        }
        assert!(
            !css.contains("font-size"),
            "type goes through assert_sizes_from_tokens"
        );
    }
    use crate::components::admin_shared::test_page::render_admin_route;
    use scuffed_types::OrgRole;

    #[test]
    fn page_has_one_h1_a_labelled_team_picker_and_a_labelled_wait() {
        let html = render_admin_route("/admin/matches", Some(OrgRole::Officer));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-matches-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Matches</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(html.contains("for=\"matches-team\""), "{html}");
        assert!(html.contains("id=\"matches-team\""), "{html}");
        // No team picked: no New match button yet, and the teams load first.
        assert!(!html.contains("id=\"matches-new\""), "{html}");
        assert!(
            html.contains("role=\"status\">Loading teams...</p>"),
            "{html}"
        );
    }

    #[test]
    fn results_are_words_and_dates_become_rfc3339() {
        assert_eq!(match_result(Some(3), Some(1)).0, "Win");
        assert_eq!(match_result(Some(1), Some(3)).0, "Loss");
        assert_eq!(match_result(Some(2), Some(2)).0, "Draw");
        assert_eq!(match_result(None, Some(2)).0, "Scheduled");
        assert_eq!(date_to_rfc3339(""), None);
        assert_eq!(
            date_to_rfc3339("2026-11-10").as_deref(),
            Some("2026-11-10T00:00:00Z")
        );
        assert_eq!(
            date_to_rfc3339("2026-11-10T19:00").as_deref(),
            Some("2026-11-10T19:00:00Z")
        );
        assert_page_css_on_tokens(PAGE_CSS);
    }

    #[test]
    fn every_field_has_a_tied_label_and_no_inline_style() {
        let src = include_str!("matches.rs");
        let prod = src.split("mod tests").next().unwrap();
        for id in [
            "TEAM_SELECT_ID",
            "OPPONENT_ID",
            "SCORE_US_ID",
            "SCORE_THEM_ID",
            "MAP_ID",
            "TYPE_ID",
            "PLAYED_ID",
            "SCHEDULED_ID",
            "VOD_ID",
            "REPLAY_ID",
            "NOTES_ID",
            "PUBLIC_ID",
        ] {
            assert!(prod.contains(&format!("r#for: {id}")), "{id}");
            assert!(prod.contains(&format!("id: {id},")), "{id}");
        }
        assert!(!prod.contains("style:"));
        assert!(!prod.contains('\u{2014}'));
    }
}
