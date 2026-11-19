use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use scuffed_api_client::{ApiClient, ClientError};

use super::article_author::{AuthorNames, author_label, short_date, use_author_names};
use crate::components::ui::inline_confirm::{focus_element_by_id_soon, focus_field_by_id};
use crate::components::ui::{
    EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pill, PillTone,
};
use crate::components::{Toast, list_cap_notice, member_pending, use_toast};
use crate::hooks::use_api_list;
use crate::state::use_auth;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Scrim {
    id: String,
    team_id: String,
    game_id: String,
    /// Member id of the poster. Shown only as a display name.
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
.scrim-form {
  box-sizing: border-box;
  min-width: 0;
  margin: 0;
  padding: var(--space-section);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.scrim-form-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-4);
}
.scrim-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
.scrim-field.is-wide { grid-column: 1 / -1; }
/* The error line is always mounted; the gap shows only with a message. */
.scrim-form-error:has(.field-error-icon) { margin-top: var(--space-4); }
.scrim-form-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin-top: var(--space-4);
}
.scrim-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-3);
  min-width: 0;
}
.scrim-card {
  box-sizing: border-box;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--space-3) var(--space-4);
  align-items: start;
  min-width: 0;
  padding: var(--space-card-pad);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
  scroll-margin-top: calc(var(--nav-height-used) + var(--space-section));
}
/* A card takes focus after its status changed and it moved to another
   section. */
.scrim-card:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.scrim-card-body { min-width: 0; }
.scrim-card-title {
  margin: 0 0 var(--space-1);
  font-family: var(--font-head);
  font-size: var(--text-lg);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
  color: var(--text);
  overflow-wrap: anywhere;
}
.scrim-card-title .ui-pill {
  margin-inline-start: var(--space-2);
  vertical-align: middle;
}
.scrim-card-notes,
.scrim-card-meta {
  margin: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
  overflow-wrap: anywhere;
}
.scrim-card-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 0 var(--space-2);
}
/* The dot trails each item, so a wrapped line never starts with one. */
.scrim-card-meta > :not(:last-child)::after {
  content: "·" / "";
  margin-inline-start: var(--space-2);
}
.scrim-card-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
@media (max-width: 820px) {
  .scrim-form-grid { grid-template-columns: minmax(0, 1fr); }
  .scrim-card { grid-template-columns: minmax(0, 1fr); }
  .scrim-card-actions .btn { flex: 1 1 auto; }
}
"#;

/// A status change a member can make from a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrimAction {
    Confirm,
    Complete,
    Cancel,
}

impl ScrimAction {
    /// The status the server stores for it.
    const fn status(self) -> &'static str {
        match self {
            ScrimAction::Confirm => "confirmed",
            ScrimAction::Complete => "completed",
            ScrimAction::Cancel => "cancelled",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            ScrimAction::Confirm => "Confirm",
            ScrimAction::Complete => "Complete",
            ScrimAction::Cancel => "Cancel",
        }
    }

    /// Toast after the change is saved.
    const fn done(self) -> &'static str {
        match self {
            ScrimAction::Confirm => "Scrim confirmed",
            ScrimAction::Complete => "Scrim completed",
            ScrimAction::Cancel => "Scrim cancelled",
        }
    }
}

/// The actions a card offers for a scrim in `status`.
fn actions_for(status: &str) -> &'static [ScrimAction] {
    match status {
        "open" => &[ScrimAction::Confirm, ScrimAction::Cancel],
        "confirmed" => &[ScrimAction::Complete, ScrimAction::Cancel],
        _ => &[],
    }
}

/// The status word next to a card title. An open request has none: its
/// section heading already says so. The word carries the state, the tone
/// only repeats it.
fn status_pill(status: &str) -> Option<(String, PillTone)> {
    match status {
        "open" => None,
        "confirmed" => Some(("Confirmed".to_string(), PillTone::Ok)),
        "completed" => Some(("Completed".to_string(), PillTone::Neutral)),
        "cancelled" => Some(("Cancelled".to_string(), PillTone::Danger)),
        other => Some((other.to_string(), PillTone::Neutral)),
    }
}

/// `2026-11-15T20:00:00Z` as "15 Nov 2026, 20:00 UTC". The form sends the
/// time it is given as UTC, so the card says UTC. `None` when the value is
/// not a timestamp.
fn scrim_when(iso: &str) -> Option<String> {
    let (_, date) = short_date(iso)?;
    let time = iso
        .get(11..16)
        .filter(|_| iso.as_bytes().get(10) == Some(&b'T'))?;
    Some(format!("{date}, {time} UTC"))
}

/// DOM id of a card: the record id, so it stays the same when the card moves
/// to another section.
fn scrim_card_id(scrim_id: &str) -> String {
    format!("scrim-card-{scrim_id}")
}

/// The name for `id`, or `None` while the list loads, after it failed, or
/// when the id is not in it. Nothing reads "Unknown".
fn name_of(pairs: &[(String, String)], id: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(pair_id, _)| pair_id == id)
        .map(|(_, name)| name.clone())
}

/// Card title when the team's name is not known.
const UNNAMED_SCRIM: &str = "Scrim request";

/// Why a scrim request or status change failed, in words. The routes answer
/// 403 to a member who is not on the team's roster and not an officer.
fn scrim_error_copy(err: &ClientError) -> &'static str {
    match err {
        ClientError::Http { status: 401, .. } => "Your session ended. Log in again.",
        ClientError::Http { status: 403, .. } => {
            "Only the team's roster or an officer can do that for this team."
        }
        ClientError::Http { status: 404, .. } => "That scrim no longer exists.",
        ClientError::Http { status: 400, .. } => "The server did not accept those details.",
        ClientError::Http { status: 429, .. } => {
            "Too many changes in a row. Wait a moment and try again."
        }
        // No answer, an unreadable answer and any other status: the one
        // mapping (QA on 748).
        other => crate::client_error::message_without_reason(other),
    }
}

/// Open, upcoming and past scrims, in the order the server sent them. A
/// status this page does not know stays with the open ones.
fn split_scrims(list: &[Scrim]) -> (Vec<Scrim>, Vec<Scrim>, Vec<Scrim>) {
    let (mut open, mut upcoming, mut past) = (Vec::new(), Vec::new(), Vec::new());
    for scrim in list {
        match scrim.status.as_str() {
            "confirmed" => upcoming.push(scrim.clone()),
            "completed" | "cancelled" => past.push(scrim.clone()),
            _ => open.push(scrim.clone()),
        }
    }
    (open, upcoming, past)
}

#[component]
pub fn Scrims() -> Element {
    let auth = use_auth();
    let mut scrims = use_api_list::<Scrim>("/api/scrims");
    let teams = use_api_list::<Team>("/api/teams");
    let games = use_api_list::<Game>("/api/games");
    let mut toast = use_toast();
    // Id of the scrim whose status change is being saved.
    let mut busy = use_signal(|| Option::<String>::None);
    // Id of the scrim that just changed status. Its card is mounted again in
    // another section once the list reloads, so focus goes back to it then.
    let mut moved = use_signal(|| Option::<String>::None);

    let is_logged_in = auth().is_logged_in();
    let is_org_member = auth().is_org_member();

    // Names of the members who posted the scrims on screen.
    let names = use_author_names(move || {
        scrims
            .data
            .read()
            .as_ref()
            .and_then(|slot| slot.as_ref())
            .map(|list| list.iter().map(|s| s.requested_by.clone()).collect())
            .unwrap_or_default()
    });

    use_effect(move || {
        // Runs when the list changes.
        let _ = scrims.data.read();
        let Some(id) = moved.peek().clone() else {
            return;
        };
        moved.set(None);
        focus_element_by_id_soon(scrim_card_id(&id));
    });

    // The request and its toast run on the page, not on the card: the card
    // is unmounted when it moves section, and its tasks would go with it.
    let on_action = use_callback(move |(scrim_id, action): (String, ScrimAction)| {
        if busy.peek().is_some() {
            return;
        }
        busy.set(Some(scrim_id.clone()));
        spawn(async move {
            let body = UpdateScrimBody {
                status: action.status().to_string(),
                opponent_name: None,
            };
            match ApiClient::web()
                .patch_json::<_, serde_json::Value>(&format!("/api/scrims/{scrim_id}"), &body)
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success(action.done()));
                    moved.set(Some(scrim_id));
                    scrims.refresh += 1;
                }
                Err(e) => {
                    crate::util::log_client_error("scrim status", &e);
                    toast.show(Toast::error(scrim_error_copy(&e)));
                }
            }
            busy.set(None);
        });
    });

    let scrim_data = scrims.data.read();
    let scrim_list = scrim_data.as_ref().and_then(|d| d.as_ref());
    let teams_data = teams.data.read();
    let teams_list = teams_data.as_ref().and_then(|d| d.as_ref());
    let games_data = games.data.read();
    let games_list = games_data.as_ref().and_then(|d| d.as_ref());

    let team_pairs: Vec<(String, String)> = teams_list
        .map(|list| {
            list.iter()
                .map(|t| (t.id.clone(), t.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let game_pairs: Vec<(String, String)> = games_list
        .map(|list| {
            list.iter()
                .map(|g| (g.id.clone(), g.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let can_post = is_org_member && teams_list.is_some() && games_list.is_some();

    rsx! {
        ScrimsLayout {
            if can_post {
                ScrimCreateForm {
                    teams: team_pairs.clone(),
                    games: game_pairs.clone(),
                    on_created: move |_| scrims.refresh += 1,
                }
            }

            {match scrim_list {
                None => member_pending(&auth(), &scrims, "scrims"),
                Some(list) => rsx! {
                    ScrimBoard {
                        scrims: list.clone(),
                        teams: team_pairs,
                        games: game_pairs,
                        names: names(),
                        can_act: is_logged_in,
                        can_post,
                        busy: busy(),
                        on_action,
                    }
                },
            }}
            {list_cap_notice(&scrims, "scrims")}
            {list_cap_notice(&teams, "teams")}
            {list_cap_notice(&games, "games")}
        }
    }
}

/// Page container and the page's one H1.
#[component]
fn ScrimsLayout(children: Element) -> Element {
    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-scrims-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Default, class: "scrims-page",
            PageHeader {
                title: "Scrims",
                lead: "Request, schedule and track practice matches. Times are UTC.",
            }
            {children}
        }
    }
}

/// The loaded board. "Open requests" is always there, with an empty card
/// when nothing is open. "Upcoming" and "Past" show only when they have rows.
#[component]
fn ScrimBoard(
    scrims: Vec<Scrim>,
    /// `(id, name)` of every team and game, for the card titles.
    teams: Vec<(String, String)>,
    games: Vec<(String, String)>,
    names: AuthorNames,
    can_act: bool,
    /// The post form is on the page, so the empty card can point at it.
    can_post: bool,
    /// Id of the scrim being saved.
    busy: Option<String>,
    on_action: EventHandler<(String, ScrimAction)>,
) -> Element {
    let (open, upcoming, past) = split_scrims(&scrims);
    let empty_message = if can_post {
        "Post one above."
    } else {
        "Check back soon."
    };
    let sections = [
        ("scrims-open", "Open requests", open, can_act),
        ("scrims-upcoming", "Upcoming", upcoming, can_act),
        ("scrims-past", "Past", past, false),
    ];

    rsx! {
        for (id, title, list, with_actions) in sections {
            // With nothing open, the empty card is the section: its H2 says
            // so, rather than a second H2 under "Open requests".
            if id == "scrims-open" && list.is_empty() {
                EmptyCard {
                    title: "No open requests",
                    message: empty_message.to_string(),
                }
            } else if !list.is_empty() {
                section { class: "ui-stack", aria_labelledby: "{id}-title",
                    h2 { class: "ui-section-title", id: "{id}-title", "{title}" }
                    ul { class: "scrim-list",
                        for scrim in list {
                            ScrimCard {
                                key: "{scrim.id}",
                                team_name: name_of(&teams, &scrim.team_id),
                                game_name: name_of(&games, &scrim.game_id),
                                poster: author_label(Some(&names), &scrim.requested_by),
                                with_actions,
                                busy: busy.as_deref() == Some(scrim.id.as_str()),
                                scrim: scrim.clone(),
                                on_action,
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ScrimCard(
    scrim: Scrim,
    /// `None` until `/api/teams` loads, or when it failed.
    team_name: Option<String>,
    game_name: Option<String>,
    /// Display name of the poster, once its lookup has finished.
    poster: Option<String>,
    with_actions: bool,
    /// A status change of this scrim is being saved.
    busy: bool,
    on_action: EventHandler<(String, ScrimAction)>,
) -> Element {
    let when = scrim_when(&scrim.scheduled_at);
    let when_text = when.clone().unwrap_or_else(|| scrim.scheduled_at.clone());
    let actions: &[ScrimAction] = if with_actions {
        actions_for(&scrim.status)
    } else {
        &[]
    };
    let notes = scrim.notes.as_deref().filter(|n| !n.trim().is_empty());
    let team_name = team_name.unwrap_or_else(|| UNNAMED_SCRIM.to_string());

    rsx! {
        li { class: "scrim-card", id: scrim_card_id(&scrim.id), tabindex: "-1",
            div { class: "scrim-card-body",
                h3 { class: "scrim-card-title",
                    span { "{team_name}" }
                    if let Some((word, tone)) = status_pill(&scrim.status) {
                        Pill { tone, "{word}" }
                    }
                }
                if let Some(notes) = notes {
                    p { class: "scrim-card-notes", "{notes}" }
                }
                p { class: "scrim-card-meta",
                    if let Some(poster) = poster {
                        span { "Posted by {poster}" }
                    }
                    if when.is_some() {
                        time { datetime: "{scrim.scheduled_at}", "{when_text}" }
                    } else {
                        span { "{when_text}" }
                    }
                    if let Some(game_name) = game_name {
                        span { "{game_name}" }
                    }
                    span { "{scrim.duration_minutes} min" }
                    if let Some(ref opponent) = scrim.opponent_name {
                        span { "vs {opponent}" }
                    }
                }
            }
            if !actions.is_empty() {
                div { class: "scrim-card-actions",
                    for action in actions.iter().copied() {
                        {
                            let scrim_id = scrim.id.clone();
                            rsx! {
                                // Busy keeps the button focusable; the page
                                // drops a second click while a save runs.
                                button {
                                    r#type: "button",
                                    class: "btn is-ghost",
                                    aria_label: "{action.label()}: {team_name}, {when_text}",
                                    aria_disabled: if busy { "true" },
                                    onclick: move |_| on_action.call((scrim_id.clone(), action)),
                                    "{action.label()}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A field of the post form that must be filled in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrimField {
    Team,
    Game,
    When,
}

impl ScrimField {
    const fn id(self) -> &'static str {
        match self {
            ScrimField::Team => "scrim-team",
            ScrimField::Game => "scrim-game",
            ScrimField::When => "scrim-scheduled-at",
        }
    }

    const fn error_id(self) -> &'static str {
        match self {
            ScrimField::Team => "scrim-team-err",
            ScrimField::Game => "scrim-game-err",
            ScrimField::When => "scrim-scheduled-at-err",
        }
    }

    const fn error(self) -> &'static str {
        match self {
            ScrimField::Team => "Choose a team.",
            ScrimField::Game => "Choose a game.",
            ScrimField::When => "Choose a date and time.",
        }
    }
}

/// The required fields that are still empty, in form order.
fn form_errors(team_id: &str, game_id: &str, scheduled_at: &str) -> Vec<ScrimField> {
    [
        (ScrimField::When, scheduled_at),
        (ScrimField::Team, team_id),
        (ScrimField::Game, game_id),
    ]
    .into_iter()
    .filter(|(_, value)| value.trim().is_empty())
    .map(|(field, _)| field)
    .collect()
}

/// The error line of a field. Always in the tree, empty while the field is
/// fine, so a new error is announced.
fn field_error(field: ScrimField, errors: &[ScrimField]) -> Element {
    let text = if errors.contains(&field) {
        field.error()
    } else {
        ""
    };
    rsx! { FieldError { id: field.error_id(), text: text.to_string(), role: "alert" } }
}

/// The post form. The submit button is never natively disabled: missing
/// fields get an error line and focus, and while posting it keeps focus.
#[component]
fn ScrimCreateForm(
    /// `(id, name)` of the teams and games to pick from.
    teams: Vec<(String, String)>,
    games: Vec<(String, String)>,
    on_created: EventHandler<()>,
) -> Element {
    let mut team_id = use_signal(String::new);
    let mut game_id = use_signal(String::new);
    let mut scheduled_at = use_signal(String::new);
    let mut duration = use_signal(|| "90".to_string());
    let mut notes = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    // Result of the last submit. Empty until one fails.
    let mut errors = use_signal(Vec::<ScrimField>::new);
    // Why the server refused the last post.
    let mut post_error = use_signal(String::new);
    let mut toast = use_toast();

    let invalid = errors();
    let team_invalid = invalid.contains(&ScrimField::Team);
    let game_invalid = invalid.contains(&ScrimField::Game);
    let when_invalid = invalid.contains(&ScrimField::When);

    rsx! {
        form {
            class: "scrim-form",
            novalidate: true,
            aria_labelledby: "scrim-form-title",
            onsubmit: move |e: FormEvent| {
                e.prevent_default();
                if submitting() {
                    return;
                }
                let tid = team_id();
                let gid = game_id();
                let sched = scheduled_at();
                let dur_str = duration();
                let n = notes();

                let missing = form_errors(&tid, &gid, &sched);
                let first = missing.first().copied();
                errors.set(missing);
                post_error.set(String::new());
                if let Some(first) = first {
                    focus_field_by_id(first.id(), ".scrim-field");
                    return;
                }

                submitting.set(true);
                spawn(async move {
                    let dur: u32 = dur_str.parse().unwrap_or(90);
                    let scheduled_iso = format!("{sched}:00Z");
                    let body = CreateScrimBody {
                        team_id: tid,
                        game_id: gid,
                        scheduled_at: scheduled_iso,
                        duration_minutes: dur,
                        notes: if n.is_empty() { None } else { Some(n) },
                    };
                    match ApiClient::web()
                        .post_json::<_, serde_json::Value>("/api/scrims", &body)
                        .await
                    {
                        Ok(_) => {
                            toast.show(Toast::success("Scrim requested"));
                            on_created.call(());
                        }
                        Err(e) => {
                            crate::util::log_client_error("scrim request", &e);
                            post_error
                                .set(format!("The request was not posted. {}", scrim_error_copy(&e)));
                        }
                    }
                    submitting.set(false);
                });
            },
            h2 { class: "ui-section-title scrim-form-title", id: "scrim-form-title", "Post a scrim" }
            div { class: "scrim-form-grid",
                div { class: "scrim-field",
                    label { class: "field-label", r#for: ScrimField::When.id(), "Date and time (UTC)" }
                    input {
                        class: "field-input",
                        id: ScrimField::When.id(),
                        name: "scrim-scheduled-at",
                        r#type: "datetime-local",
                        required: true,
                        aria_required: "true",
                        aria_invalid: if when_invalid { "true" },
                        aria_describedby: if when_invalid { ScrimField::When.error_id() },
                        value: "{scheduled_at}",
                        onchange: move |e| scheduled_at.set(e.value()),
                    }
                    {field_error(ScrimField::When, &invalid)}
                }
                div { class: "scrim-field",
                    label { class: "field-label", r#for: "scrim-duration", "Duration in minutes" }
                    input {
                        class: "field-input",
                        id: "scrim-duration",
                        name: "scrim-duration",
                        r#type: "number",
                        value: "{duration}",
                        min: "15",
                        max: "480",
                        onchange: move |e| duration.set(e.value()),
                    }
                }
                div { class: "scrim-field",
                    label { class: "field-label", r#for: ScrimField::Team.id(), "Team" }
                    select {
                        class: "field-input",
                        id: ScrimField::Team.id(),
                        name: "scrim-team",
                        required: true,
                        aria_required: "true",
                        aria_invalid: if team_invalid { "true" },
                        aria_describedby: if team_invalid { ScrimField::Team.error_id() },
                        onchange: move |e| team_id.set(e.value()),
                        option { value: "", selected: team_id().is_empty(), "Select a team" }
                        for (id, name) in teams.iter() {
                            option { key: "{id}", value: "{id}", selected: team_id() == *id, "{name}" }
                        }
                    }
                    {field_error(ScrimField::Team, &invalid)}
                }
                div { class: "scrim-field",
                    label { class: "field-label", r#for: ScrimField::Game.id(), "Game" }
                    select {
                        class: "field-input",
                        id: ScrimField::Game.id(),
                        name: "scrim-game",
                        required: true,
                        aria_required: "true",
                        aria_invalid: if game_invalid { "true" },
                        aria_describedby: if game_invalid { ScrimField::Game.error_id() },
                        onchange: move |e| game_id.set(e.value()),
                        option { value: "", selected: game_id().is_empty(), "Select a game" }
                        for (id, name) in games.iter() {
                            option { key: "{id}", value: "{id}", selected: game_id() == *id, "{name}" }
                        }
                    }
                    {field_error(ScrimField::Game, &invalid)}
                }
                div { class: "scrim-field is-wide",
                    label { class: "field-label", r#for: "scrim-notes", "Notes (optional)" }
                    input {
                        class: "field-input",
                        id: "scrim-notes",
                        name: "scrim-notes",
                        r#type: "text",
                        value: "{notes}",
                        onchange: move |e| notes.set(e.value()),
                    }
                }
            }
            div { class: "scrim-form-error",
                FieldError { id: "scrim-form-err", text: post_error(), role: "alert" }
            }
            div { class: "scrim-form-actions",
                button {
                    r#type: "submit",
                    class: "btn is-primary",
                    aria_busy: if submitting() { "true" },
                    aria_disabled: if submitting() { "true" },
                    if submitting() { "Posting..." } else { "Post request" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ToastProvider;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

    const POSTER: &str = "member:9d1c44a0e2";

    fn scrim(n: usize, status: &str) -> Scrim {
        Scrim {
            id: format!("scrim:{n}"),
            team_id: "team:1".to_string(),
            game_id: "game:1".to_string(),
            requested_by: POSTER.to_string(),
            opponent_name: None,
            scheduled_at: "2026-11-15T20:00:00Z".to_string(),
            duration_minutes: 90,
            status: status.to_string(),
            notes: Some(format!("Notes {n}")),
            created_at: "2026-11-08T10:00:00Z".to_string(),
            updated_at: "2026-11-08T10:00:00Z".to_string(),
        }
    }

    #[derive(Props, Clone, PartialEq)]
    struct ProbeProps {
        scrims: Vec<Scrim>,
        can_act: bool,
        busy: Option<String>,
        #[props(default)]
        form: bool,
    }

    #[allow(non_snake_case)]
    fn Probe(props: ProbeProps) -> Element {
        let teams = vec![("team:1".to_string(), "Squad Alpha".to_string())];
        let games = vec![("game:1".to_string(), "Overwatch".to_string())];
        let names = AuthorNames::from([(POSTER.to_string(), Some("Player01".to_string()))]);
        rsx! {
            ToastProvider {
                ScrimsLayout {
                    if props.form {
                        ScrimCreateForm {
                            teams: teams.clone(),
                            games: games.clone(),
                            on_created: move |_| {},
                        }
                    }
                    ScrimBoard {
                        scrims: props.scrims,
                        teams,
                        games,
                        names,
                        can_act: props.can_act,
                        can_post: props.form,
                        busy: props.busy,
                        on_action: move |_| {},
                    }
                }
            }
        }
    }

    fn render(props: ProbeProps) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, props);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[derive(Props, Clone, PartialEq)]
    struct NamesProbeProps {
        scrims: Vec<Scrim>,
        teams: Vec<(String, String)>,
        games: Vec<(String, String)>,
    }

    #[allow(non_snake_case)]
    fn NamesProbe(props: NamesProbeProps) -> Element {
        rsx! {
            ScrimBoard {
                scrims: props.scrims,
                teams: props.teams,
                games: props.games,
                names: AuthorNames::new(),
                can_act: true,
                can_post: false,
                busy: None,
                on_action: move |_| {},
            }
        }
    }

    fn render_with_names(
        scrims: Vec<Scrim>,
        teams: Vec<(String, String)>,
        games: Vec<(String, String)>,
    ) -> String {
        let mut dom = VirtualDom::new_with_props(
            NamesProbe,
            NamesProbeProps {
                scrims,
                teams,
                games,
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn board(scrims: Vec<Scrim>, can_act: bool) -> String {
        render(ProbeProps {
            scrims,
            can_act,
            busy: None,
            form: false,
        })
    }

    #[test]
    fn page_has_one_h1_and_no_main() {
        for html in [
            board(Vec::new(), true),
            board(vec![scrim(1, "open"), scrim(2, "confirmed")], true),
        ] {
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(html.contains("<h1>Scrims</h1>"), "{html}");
            assert!(!html.contains("<main"), "{html}");
            assert!(html.contains("class=\"ui-shell scrims-page\""), "{html}");
        }
    }

    #[test]
    fn open_card_shows_team_notes_poster_time_and_named_actions() {
        let html = board(vec![scrim(1, "open")], true);
        assert!(
            html.contains(
                "<section class=\"ui-stack\" aria-labelledby=\"scrims-open-title\"><h2 class=\"ui-section-title\" id=\"scrims-open-title\">Open requests</h2>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<li class=\"scrim-card\" id=\"scrim-card-scrim:1\" tabindex=\"-1\">"),
            "{html}"
        );
        assert!(
            html.contains("<h3 class=\"scrim-card-title\"><span>Squad Alpha</span></h3>"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"scrim-card-notes\">Notes 1</p>"),
            "{html}"
        );
        assert!(html.contains("<span>Posted by Player01</span>"), "{html}");
        assert!(html.contains("<span>Overwatch</span>"), "{html}");
        assert!(
            html.contains("<time datetime=\"2026-11-15T20:00:00Z\">15 Nov 2026, 20:00 UTC</time>"),
            "{html}"
        );
        assert!(html.contains("<span>90 min</span>"), "{html}");
        assert!(!html.contains(POSTER), "raw member id leaked: {html}");
        // Each action is a button with a name that says which scrim.
        assert!(
            html.contains(
                "<button type=\"button\" class=\"btn is-ghost\" aria-label=\"Confirm: Squad Alpha, 15 Nov 2026, 20:00 UTC\">Confirm</button>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "aria-label=\"Cancel: Squad Alpha, 15 Nov 2026, 20:00 UTC\">Cancel</button>"
            ),
            "{html}"
        );
        // Only the open section shows when the others are empty.
        assert_eq!(html.matches("<h2").count(), 1, "{html}");
        assert!(!html.contains("<a "), "{html}");
    }

    #[test]
    fn status_is_a_word_and_sections_follow_status() {
        let html = board(
            vec![
                scrim(1, "open"),
                scrim(2, "confirmed"),
                scrim(3, "completed"),
                scrim(4, "cancelled"),
            ],
            true,
        );
        for title in ["Open requests", "Upcoming", "Past"] {
            assert!(html.contains(&format!(">{title}</h2>")), "{title}: {html}");
        }
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--ok\">Confirmed</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--neutral\">Completed</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--danger\">Cancelled</span>"),
            "{html}"
        );
        // Open: Confirm and Cancel. Confirmed: Complete and Cancel. Past: none.
        assert_eq!(html.matches(">Confirm</button>").count(), 1, "{html}");
        assert!(!html.contains(">Accept</button>"), "{html}");
        assert_eq!(html.matches(">Complete</button>").count(), 1, "{html}");
        assert_eq!(html.matches(">Cancel</button>").count(), 2, "{html}");
        // Ids come from the record, so they are unique.
        for n in 1..=4 {
            assert_eq!(
                html.matches(&format!("id=\"scrim-card-scrim:{n}\""))
                    .count(),
                1,
                "{html}"
            );
        }
    }

    #[test]
    fn copy_only_promises_what_the_api_allows() {
        let html = board(vec![scrim(1, "open")], true);
        assert!(
            html.contains("Request, schedule and track practice matches. Times are UTC."),
            "{html}"
        );
        assert!(!html.contains("pick up"), "no opponent flow exists: {html}");
        assert_eq!(ScrimAction::Confirm.status(), "confirmed");
        assert_eq!(ScrimAction::Confirm.label(), "Confirm");
    }

    #[test]
    fn a_card_never_reads_unknown_while_names_load() {
        let html = render_with_names(vec![scrim(1, "open")], Vec::new(), Vec::new());
        assert!(!html.contains("Unknown"), "{html}");
        assert!(
            html.contains("<h3 class=\"scrim-card-title\"><span>Scrim request</span></h3>"),
            "{html}"
        );
        assert!(
            html.contains("aria-label=\"Confirm: Scrim request, 15 Nov 2026, 20:00 UTC\""),
            "{html}"
        );
        assert!(!html.contains("<span>Overwatch</span>"), "{html}");
        assert_eq!(name_of(&[], "team:1"), None);
    }

    #[test]
    fn a_refused_change_says_why_in_words() {
        let http = |status| {
            ClientError::http(
                status,
                r#"{"error":"Must be on the team roster or an officer"}"#,
                None,
            )
        };
        assert_eq!(
            scrim_error_copy(&http(403)),
            "Only the team's roster or an officer can do that for this team."
        );
        assert_eq!(scrim_error_copy(&http(404)), "That scrim no longer exists.");
        assert_eq!(
            scrim_error_copy(&http(401)),
            "Your session ended. Log in again."
        );
        assert!(scrim_error_copy(&ClientError::Network("offline".into())).contains("connection"));
        for status in [400, 401, 403, 404, 429, 500] {
            let copy = scrim_error_copy(&http(status));
            assert!(!copy.contains("HTTP"), "{copy}");
            assert!(!copy.contains("connection"), "{status}: {copy}");
        }
        let prod = include_str!("scrims.rs").split("mod tests").next().unwrap();
        assert!(
            !prod.contains("Error: {e}"),
            "a raw client error reaches a toast"
        );
        assert!(
            !prod.contains("posted. {e}"),
            "a raw client error reaches the form"
        );
    }

    #[test]
    fn viewer_who_cannot_act_gets_no_buttons() {
        let html = board(vec![scrim(1, "open"), scrim(2, "confirmed")], false);
        assert!(!html.contains("<button"), "{html}");
        assert!(!html.contains("scrim-card-actions"), "{html}");
    }

    #[test]
    fn busy_card_keeps_its_buttons_focusable() {
        let html = render(ProbeProps {
            scrims: vec![scrim(1, "open"), scrim(2, "open")],
            can_act: true,
            busy: Some("scrim:1".to_string()),
            form: false,
        });
        assert_eq!(html.matches("aria-disabled=\"true\"").count(), 2, "{html}");
        assert!(!html.contains(" disabled"), "{html}");
    }

    #[test]
    fn empty_board_says_so_and_points_at_the_form() {
        let html = render(ProbeProps {
            scrims: Vec::new(),
            can_act: true,
            busy: None,
            form: true,
        });
        assert!(html.contains("<h2>No open requests</h2>"), "{html}");
        assert!(html.contains("<p>Post one above.</p>"), "{html}");
        assert!(
            !html.contains(">Open requests</h2>"),
            "the empty card is the section, not an H2 under another H2: {html}"
        );
        assert!(!html.contains("Upcoming"), "{html}");
        assert!(!html.contains(">Past<"), "{html}");

        let no_form = board(Vec::new(), true);
        assert!(no_form.contains("<p>Check back soon.</p>"), "{no_form}");

        // Past rows alone still leave the open section with its empty card.
        let past_only = board(vec![scrim(1, "completed")], true);
        assert!(
            past_only.contains("<h2>No open requests</h2>"),
            "{past_only}"
        );
        assert!(!past_only.contains(">Open requests</h2>"), "{past_only}");
        assert!(past_only.contains(">Past</h2>"), "{past_only}");
    }

    #[test]
    fn form_controls_are_labelled_and_error_lines_are_always_mounted() {
        let html = render(ProbeProps {
            scrims: Vec::new(),
            can_act: true,
            busy: None,
            form: true,
        });
        assert!(html.contains("<form class=\"scrim-form\""), "{html}");
        assert!(
            html.contains("aria-labelledby=\"scrim-form-title\""),
            "{html}"
        );
        assert!(
            html.contains(
                "<h2 class=\"ui-section-title scrim-form-title\" id=\"scrim-form-title\">Post a scrim</h2>"
            ),
            "{html}"
        );
        for (id, label) in [
            ("scrim-team", "Team"),
            ("scrim-game", "Game"),
            ("scrim-scheduled-at", "Date and time (UTC)"),
            ("scrim-duration", "Duration in minutes"),
            ("scrim-notes", "Notes (optional)"),
        ] {
            assert!(
                html.contains(&format!(
                    "<label class=\"field-label\" for=\"{id}\">{label}</label>"
                )),
                "{id}: {html}"
            );
            assert_eq!(html.matches(&format!("id=\"{id}\"")).count(), 1, "{id}");
        }
        for id in [
            "scrim-team-err",
            "scrim-game-err",
            "scrim-scheduled-at-err",
            "scrim-form-err",
        ] {
            assert!(
                html.contains(&format!(
                    "<p class=\"field-error\" id=\"{id}\" role=\"alert\"></p>"
                )),
                "{id}: {html}"
            );
        }
        assert!(!html.contains("aria-invalid"), "{html}");
        assert!(
            html.contains("<button type=\"submit\" class=\"btn is-primary\">Post request</button>"),
            "{html}"
        );
        assert!(html.contains(">Squad Alpha</option>"), "{html}");
        assert!(html.contains(">Overwatch</option>"), "{html}");
        // The frame leads with the date and time; notes close the form.
        let at = |id: &str| {
            html.find(&format!("for=\"{id}\""))
                .unwrap_or_else(|| panic!("{id}"))
        };
        let order = [
            "scrim-scheduled-at",
            "scrim-duration",
            "scrim-team",
            "scrim-game",
            "scrim-notes",
        ];
        assert!(order.windows(2).all(|w| at(w[0]) < at(w[1])), "{html}");
    }

    #[test]
    fn meta_line_puts_the_time_after_the_poster_as_in_the_frame() {
        let html = board(vec![scrim(1, "open")], true);
        let poster = html.find("Posted by Player01").expect("poster");
        let time = html.find("<time ").expect("time");
        let game = html.find("<span>Overwatch</span>").expect("game");
        assert!(poster < time && time < game, "{html}");
    }

    /// The page in the real router and shell, as a guest and as a member:
    /// one H1 and only the shell's `<main>` on the wall and loading states.
    #[test]
    fn routed_page_has_one_h1_and_one_main_in_every_gate_state() {
        use crate::components::member_gate::test_route::render_route;
        use scuffed_types::OrgRole;
        for role in [None, Some(OrgRole::Member)] {
            let html = render_route("/scrims", role);
            assert_eq!(html.matches("<h1").count(), 1, "{role:?}: {html}");
            assert!(html.contains(">Scrims</h1>"), "{role:?}: {html}");
            assert_eq!(html.matches("<main").count(), 1, "{role:?}: {html}");
        }
    }

    #[test]
    fn missing_required_fields_are_reported_in_form_order() {
        // Date and time comes first, as in the frame.
        assert_eq!(
            form_errors("", "", " "),
            vec![ScrimField::When, ScrimField::Team, ScrimField::Game]
        );
        assert_eq!(form_errors("team:1", "game:1", ""), vec![ScrimField::When]);
        assert!(form_errors("team:1", "game:1", "2026-11-15T20:00").is_empty());
        assert_eq!(ScrimField::When.error(), "Choose a date and time.");
    }

    #[test]
    fn time_is_day_month_year_and_utc() {
        assert_eq!(
            scrim_when("2026-11-15T20:00:00Z").as_deref(),
            Some("15 Nov 2026, 20:00 UTC")
        );
        assert_eq!(scrim_when("soon"), None);
        assert_eq!(scrim_when("2026-11-15"), None);
    }

    #[test]
    fn unknown_status_stays_open_with_its_word() {
        let (open, upcoming, past) = split_scrims(&[scrim(1, "pending"), scrim(2, "cancelled")]);
        assert_eq!((open.len(), upcoming.len(), past.len()), (1, 0, 1));
        assert_eq!(
            status_pill("pending"),
            Some(("pending".to_string(), PillTone::Neutral))
        );
        assert_eq!(status_pill("open"), None);
        assert!(actions_for("pending").is_empty());
    }

    #[test]
    fn css_is_on_tokens_and_stacks_on_phone() {
        assert_sizes_from_tokens(PAGE_CSS);
        let card = css_rule(PAGE_CSS, ".scrim-card {");
        for decl in [
            "padding: var(--space-card-pad)",
            "background: var(--card-bg)",
            "grid-template-columns: minmax(0, 1fr) auto",
        ] {
            assert!(has_decl(&card, decl), "missing {decl}: {card}");
        }
        let phone = PAGE_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block")
            .1;
        assert!(
            has_decl(
                &css_rule(phone, ".scrim-card {"),
                "grid-template-columns: minmax(0, 1fr)"
            ),
            "{phone}"
        );
        // The action buttons take the row on phone, at the shared 44px.
        assert!(
            has_decl(
                &css_rule(phone, ".scrim-card-actions .btn {"),
                "flex: 1 1 auto"
            ),
            "{phone}"
        );
        assert!(
            PAGE_CSS.contains(".scrim-card:focus-visible {\n  outline: var(--focus-ring-width) solid var(--focus-ring);"),
            "{PAGE_CSS}"
        );
    }

    #[test]
    fn separator_dots_are_silent_and_the_error_gap_needs_a_message() {
        let dot = css_rule(PAGE_CSS, ".scrim-card-meta > :not(:last-child)::after {");
        assert!(
            has_decl(&dot, "content: \"·\" / \"\""),
            "the dot has empty alt text, so it is not read out: {dot}"
        );
        // The wrapper always holds the mounted error line, so :empty never
        // matches it; the gap keys on the icon a message brings.
        assert!(
            !PAGE_CSS.contains(".scrim-form-error:not(:empty)"),
            "{PAGE_CSS}"
        );
        let gap = css_rule(PAGE_CSS, ".scrim-form-error:has(.field-error-icon) {");
        assert!(has_decl(&gap, "margin-top: var(--space-4)"), "{gap}");
    }
}
