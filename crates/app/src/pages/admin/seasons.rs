//! Admin › Seasons: define the windows that "per season" stats are cut by.
//!
//! A season is a half-open `[starts_at, ends_at)` range over `played_at`, in
//! UTC. Games are never tagged with a season; deleting or editing one only
//! changes how existing matches are grouped.

use chrono::{DateTime, NaiveDateTime, Utc};
use dioxus::prelude::*;

use crate::components::admin_shared::{after_delete_focus_ids, focus_after_delete_soon};

use crate::components::ui::{
    EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pager, Pill, PillTone, page_slice,
    show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, AccessDenied, ConfirmDialog, DataTable, FormModal, RowActions, SaveState,
    Toast, admin_list_pending, admin_short_date, use_toast,
};
use crate::hooks::{ModalController, use_api};
use crate::state::use_auth;
use scuffed_api_client::ApiClient;
use scuffed_types::{
    Season,
    api::{CreateSeasonRequest, UpdateSeasonRequest},
};

const INPUT_FMT: &str = "%Y-%m-%dT%H:%M";

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-seasons-list";
const NEW_SEASON_ID: &str = "seasons-new";
const NAME_ID: &str = "season-name";
const NAME_ERROR_ID: &str = "season-name-error";
const STARTS_ID: &str = "season-starts";
const ENDS_ID: &str = "season-ends";
const ENDS_HINT_ID: &str = "season-ends-hint";
const RANGE_ERROR_ID: &str = "season-range-error";
const CURRENT_ID: &str = "season-current";

const LEAD: &str = "Seasons split every stats page into all time or one window. A game belongs to the season it was played in: on or after the start, and before the end (UTC). Editing or deleting a season regroups existing games; nothing is lost.";

fn to_input(dt: &DateTime<Utc>) -> String {
    dt.format(INPUT_FMT).to_string()
}

/// Parse a `datetime-local` value as UTC. Browsers may include seconds.
fn parse_input(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    NaiveDateTime::parse_from_str(s, INPUT_FMT)
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .ok()
        .map(|n| n.and_utc())
}

/// "10 Nov 2026, 19:00 UTC": the visible date of a season edge.
fn fmt_display(dt: &DateTime<Utc>) -> String {
    let iso = dt.to_rfc3339();
    let day = admin_short_date(&iso)
        .map(|(_, human)| human)
        .unwrap_or_else(|| dt.format("%Y-%m-%d").to_string());
    format!("{day}, {} UTC", dt.format("%H:%M"))
}

/// The name, the start and the end checked before a save. Each error names
/// the field it belongs to.
#[derive(Debug, PartialEq, Eq)]
enum SeasonFormError {
    Name,
    Missing,
    Order,
}

impl SeasonFormError {
    fn text(&self) -> &'static str {
        match self {
            SeasonFormError::Name => "Enter a name for the season.",
            SeasonFormError::Missing => "Set both the start and the end.",
            SeasonFormError::Order => "The end must be after the start.",
        }
    }
}

fn check_season_form(
    name: &str,
    starts: &str,
    ends: &str,
) -> Result<(String, DateTime<Utc>, DateTime<Utc>), SeasonFormError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(SeasonFormError::Name);
    }
    let (Some(starts_at), Some(ends_at)) = (parse_input(starts), parse_input(ends)) else {
        return Err(SeasonFormError::Missing);
    };
    if ends_at <= starts_at {
        return Err(SeasonFormError::Order);
    }
    Ok((name.to_string(), starts_at, ends_at))
}

/// Id of a row's Edit button, from the season id.
fn edit_button_id(season_id: &str) -> String {
    format!("season-edit-{season_id}")
}

/// Id of a row's Delete button, from the season id.
fn delete_button_id(season_id: &str) -> String {
    format!("season-delete-{season_id}")
}

#[component]
fn SeasonEdge(at: DateTime<Utc>) -> Element {
    let machine = at.to_rfc3339();
    let human = fmt_display(&at);
    rsx! {
        time { class: "date", datetime: "{machine}", "{human}" }
    }
}

/// Admin only. The role is checked before any request: an officer gets the
/// no-access page without first asking the API for seasons (a 403).
#[component]
pub fn AdminSeasons() -> Element {
    let auth = use_auth();
    if !auth().is_admin() {
        return rsx! {
            AccessDenied { message: "You need admin permissions to manage seasons.".to_string() }
        };
    }
    rsx! { SeasonsPage {} }
}

/// Where a season stands today, as a word and a tone.
fn season_status(season: &Season, now: DateTime<Utc>) -> (&'static str, PillTone) {
    if season.is_current {
        ("Current", PillTone::Ok)
    } else if now < season.starts_at {
        ("Upcoming", PillTone::Info)
    } else if now >= season.ends_at {
        ("Ended", PillTone::Neutral)
    } else {
        ("In progress", PillTone::Neutral)
    }
}

#[component]
fn SeasonsPage() -> Element {
    let mut seasons = use_api::<Vec<Season>>("/api/admin/seasons");
    let mut toast = use_toast();
    let mut page = use_signal(|| 1usize);

    let mut modal = ModalController::<String>::new();
    let mut delete_modal = ModalController::<Season>::new();
    let mut form_name = use_signal(String::new);
    let mut form_starts = use_signal(String::new);
    let mut form_ends = use_signal(String::new);
    let mut form_current = use_signal(|| false);
    let mut name_error = use_signal(String::new);
    let mut range_error = use_signal(String::new);
    let mut save_error = use_signal(|| None::<String>);
    let mut opener = use_signal(|| NEW_SEASON_ID.to_string());

    let mut clear_errors = move || {
        name_error.set(String::new());
        range_error.set(String::new());
        save_error.set(None);
    };

    let mut open_create = move || {
        form_name.set(String::new());
        form_starts.set(String::new());
        form_ends.set(String::new());
        form_current.set(false);
        clear_errors();
        opener.set(NEW_SEASON_ID.to_string());
        modal.show_empty();
    };

    let mut open_edit = move |s: Season| {
        form_name.set(s.name);
        form_starts.set(to_input(&s.starts_at));
        form_ends.set(to_input(&s.ends_at));
        form_current.set(s.is_current);
        clear_errors();
        opener.set(edit_button_id(&s.id));
        modal.show(s.id);
    };

    let mut open_delete = move |s: Season| {
        delete_modal.show(s);
    };

    let on_close = move |_| {
        modal.close();
    };

    let on_submit = move |_| {
        clear_errors();
        let (name, starts_at, ends_at) =
            match check_season_form(&form_name(), &form_starts(), &form_ends()) {
                Ok(valid) => valid,
                Err(err) => {
                    match err {
                        SeasonFormError::Name => name_error.set(err.text().to_string()),
                        _ => range_error.set(err.text().to_string()),
                    }
                    toast.show(Toast::error(err.text()));
                    return;
                }
            };
        let is_current = form_current();
        let edit_id = modal.get_target();

        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                let body = UpdateSeasonRequest {
                    name: Some(name),
                    starts_at: Some(starts_at),
                    ends_at: Some(ends_at),
                    is_current: Some(is_current),
                };
                client
                    .put_json::<_, Season>(&format!("/api/admin/seasons/{id}"), &body)
                    .await
            } else {
                let body = CreateSeasonRequest {
                    name,
                    starts_at,
                    ends_at,
                    is_current,
                };
                client
                    .post_json::<_, Season>("/api/admin/seasons", &body)
                    .await
            };

            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Season saved."));
                    modal.close();
                    seasons.refresh += 1;
                }
                Err(e) => {
                    save_error.set(Some(
                        SaveState::failed("Could not save the season", &e).text(),
                    ));
                    toast.show(Toast::error(format!("Failed to save season: {e}")));
                }
            }
        });
    };

    let on_confirm_delete = move |_| {
        let Some(target) = delete_modal.get_target() else {
            return;
        };
        // B9: once the row is gone, focus the next row's Edit, else the
        // previous row's, else New season.
        let keys: Vec<String> = seasons
            .data
            .peek()
            .as_ref()
            .and_then(|d| d.as_ref())
            .map(|all| all.iter().map(|s| s.id.clone()).collect())
            .unwrap_or_default();
        let focus_next = after_delete_focus_ids(
            &keys,
            &target.id,
            |k| vec![edit_button_id(k)],
            &[NEW_SEASON_ID],
        );
        delete_modal.close();
        spawn(async move {
            match ApiClient::web()
                .delete(&format!("/api/admin/seasons/{}", target.id))
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Season deleted."));
                    seasons.refresh += 1;
                    focus_after_delete_soon(edit_button_id(&target.id), focus_next);
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed to delete season: {e}")));
                }
            }
        });
    };

    let on_cancel_delete = move |_| {
        delete_modal.close();
    };

    let now = Utc::now();
    let list = {
        let data = seasons.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&seasons, "seasons"),
            Some(all) => {
                let total = all.len();
                let rows = page_slice(all, page(), ADMIN_PAGE_SIZE).to_vec();
                rsx! {
                    if all.is_empty() {
                        EmptyCard {
                            title: "No seasons yet",
                            message: "Stats show all time until a season exists. Add one to split the stats pages by season.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-primary",
                                    onclick: move |_| open_create(),
                                    "New season"
                                }
                            },
                        }
                    } else {
                        DataTable {
                            headers: vec!["Season", "Status", "Starts", "Ends", "Actions"],
                            num_cols: vec![2, 3],
                            cards: true,
                            label: "Seasons",
                            for season in rows.iter() {
                                Fragment { key: "{season.id}", {
                                    let e = season.clone();
                                    let d = season.clone();
                                    let name = season.name.clone();
                                    rsx! {
                                        tr { key: "{season.id}",
                                            td { class: "cell-who", "{season.name}" }
                                            td { class: "cell-badge",
                                                {
                                                    let (word, tone) = season_status(season, now);
                                                    rsx! { Pill { tone, "{word}" } }
                                                }
                                            }
                                            // Labelled lines on phone, so the two dates say
                                            // which is the start and which the end.
                                            td { class: "num", "data-label": "Starts", SeasonEdge { at: season.starts_at } }
                                            td { class: "num", "data-label": "Ends", SeasonEdge { at: season.ends_at } }
                                            td { class: "cell-actions",
                                                RowActions {
                                                    button {
                                                        r#type: "button",
                                                        class: "btn is-ghost",
                                                        id: edit_button_id(&season.id),
                                                        aria_label: "Edit {name}",
                                                        onclick: move |_| open_edit(e.clone()),
                                                        "Edit"
                                                    }
                                                    button {
                                                        r#type: "button",
                                                        class: "btn is-danger",
                                                        id: delete_button_id(&season.id),
                                                        aria_label: "Delete {name}",
                                                        onclick: move |_| open_delete(d.clone()),
                                                        "Delete"
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
                        id: "seasons-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "seasons",
                        noun_singular: "season",
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

    let delete_focus = delete_modal.get_target().map(|s| delete_button_id(&s.id));
    let range_invalid = !range_error().is_empty();
    let range_described = if range_invalid {
        format!("{ENDS_HINT_ID} {RANGE_ERROR_ID}")
    } else {
        ENDS_HINT_ID.to_string()
    };

    rsx! {
        PageShell { width: PageWidth::Wide, class: "admin-seasons-page",
            PageHeader {
                title: "Seasons",
                lead: LEAD.to_string(),
                actions: rsx! {
                    button {
                        r#type: "button",
                        class: "btn is-primary",
                        id: NEW_SEASON_ID,
                        onclick: move |_| open_create(),
                        "New season"
                    }
                },
            }

            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Seasons",
                {list}
            }
        }

        FormModal {
            title: if modal.get_target().is_some() { "Edit season".to_string() } else { "New season".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,
            submit_label: modal.get_target().is_none().then(|| "Create".to_string()),
            error: save_error(),
            restore_focus_id: Some(opener()),

            div { class: "form-field",
                label { class: "field-label", r#for: NAME_ID, "Name" }
                input {
                    class: "field-input",
                    id: NAME_ID,
                    r#type: "text",
                    placeholder: "Season 3",
                    value: "{form_name}",
                    aria_invalid: if !name_error().is_empty() { "true" },
                    aria_describedby: NAME_ERROR_ID,
                    oninput: move |e| form_name.set(e.value()),
                }
                FieldError { id: NAME_ERROR_ID.to_string(), text: name_error() }
            }
            div { class: "form-grid",
                div { class: "form-field",
                    label { class: "field-label", r#for: STARTS_ID, "Starts (UTC)" }
                    input {
                        class: "field-input",
                        id: STARTS_ID,
                        r#type: "datetime-local",
                        value: "{form_starts}",
                        aria_invalid: if range_invalid { "true" },
                        aria_describedby: RANGE_ERROR_ID,
                        oninput: move |e| form_starts.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: ENDS_ID, "Ends (UTC)" }
                    input {
                        class: "field-input",
                        id: ENDS_ID,
                        r#type: "datetime-local",
                        value: "{form_ends}",
                        aria_invalid: if range_invalid { "true" },
                        aria_describedby: "{range_described}",
                        oninput: move |e| form_ends.set(e.value()),
                    }
                    p { class: "field-hint", id: ENDS_HINT_ID, "A game played at this exact time belongs to the next season." }
                }
            }
            FieldError { id: RANGE_ERROR_ID.to_string(), text: range_error() }
            label { class: "admin-check-row", r#for: CURRENT_ID,
                input {
                    r#type: "checkbox",
                    id: CURRENT_ID,
                    checked: form_current(),
                    onchange: move |e| form_current.set(e.checked()),
                }
                "Current season (clears the flag on any other season)"
            }
        }

        ConfirmDialog {
            title: "Delete season".to_string(),
            message: format!(
                "Delete \"{}\"? Games are kept; they simply stop being grouped under this season.",
                delete_modal.get_target().map(|s| s.name).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            confirm_label: "Delete".to_string(),
            restore_focus_id: delete_focus,
            on_confirm: on_confirm_delete,
            on_cancel: on_cancel_delete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::admin_shared::test_page::render_admin_route;
    use scuffed_types::OrgRole;

    #[test]
    fn page_has_one_h1_and_a_labelled_wait_in_the_wide_shell() {
        let html = render_admin_route("/admin/seasons", Some(OrgRole::Admin));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-seasons-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Seasons</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains("role=\"status\">Loading seasons...</p>"),
            "{html}"
        );
    }

    #[test]
    fn officers_get_the_no_access_page_with_one_h1() {
        let html = render_admin_route("/admin/seasons", Some(OrgRole::Officer));
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(
            html.contains("You need admin permissions to manage seasons."),
            "{html}"
        );
        assert!(!html.contains("admin-seasons-page"), "{html}");
    }

    #[test]
    fn form_checks_name_then_both_edges_then_order() {
        assert_eq!(
            check_season_form(" ", "2026-01-01T00:00", "2026-02-01T00:00"),
            Err(SeasonFormError::Name)
        );
        assert_eq!(
            check_season_form("S1", "", "2026-02-01T00:00"),
            Err(SeasonFormError::Missing)
        );
        assert_eq!(
            check_season_form("S1", "2026-02-01T00:00", "2026-02-01T00:00"),
            Err(SeasonFormError::Order)
        );
        let (name, start, end) =
            check_season_form(" S1 ", "2026-01-01T00:00", "2026-02-01T00:00:30").unwrap();
        assert_eq!(name, "S1");
        assert!(end > start);
    }

    #[test]
    fn every_season_has_a_status_word() {
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let season = |current: bool| Season {
            id: "s1".into(),
            name: "Test Season".into(),
            starts_at: at("2026-12-01T00:00:00Z"),
            ends_at: at("2027-03-01T00:00:00Z"),
            is_current: current,
        };
        assert_eq!(
            season_status(&season(true), at("2026-11-17T00:00:00Z")).0,
            "Current"
        );
        assert_eq!(
            season_status(&season(false), at("2026-11-17T00:00:00Z")),
            ("Upcoming", PillTone::Info)
        );
        assert_eq!(
            season_status(&season(false), at("2027-03-02T00:00:00Z")).0,
            "Ended"
        );
        assert_eq!(
            season_status(&season(false), at("2027-01-02T00:00:00Z")).0,
            "In progress"
        );
    }

    #[test]
    fn edges_read_as_a_day_and_a_utc_time() {
        let at = parse_input("2026-11-10T19:05").unwrap();
        assert_eq!(fmt_display(&at), "10 Nov 2026, 19:05 UTC");
        assert_eq!(to_input(&at), "2026-11-10T19:05");
    }

    #[test]
    fn every_field_has_a_tied_label_and_no_inline_style() {
        let src = include_str!("seasons.rs");
        let prod = src.split("mod tests").next().unwrap();
        for id in ["NAME_ID", "STARTS_ID", "ENDS_ID", "CURRENT_ID"] {
            assert!(prod.contains(&format!("r#for: {id}")), "{id}");
            assert!(prod.contains(&format!("id: {id},")), "{id}");
        }
        assert!(!prod.contains("style:"));
        assert!(!prod.contains('\u{2014}'));
    }
}
