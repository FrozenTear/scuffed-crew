use dioxus::prelude::*;
use scuffed_types::{
    APPLICATION_RESUBMIT_COOLDOWN_SECS, Game, MAX_APPLICATION_MESSAGE_CHARS,
    MAX_APPLICATION_PREFERENCE_ITEMS, MAX_WITHDRAWN_APPLICATIONS, MIN_APPLICATION_MESSAGE_CHARS,
    WITHDRAWN_APPLICATION_WINDOW_DAYS, application_message_long_enough,
};
use serde::{Deserialize, Serialize};

use crate::components::ui::inline_confirm::{focus_element_by_id_soon, focus_field_by_id};
use crate::components::ui::{
    Card, ChipButton, FieldError, InlineConfirm, PageHeader, PageShell, PageWidth, Pill, PillTone,
    btn_class,
};
use crate::components::{Toast, is_http_status, list_cap_notice, retry_label, use_toast};
use crate::hooks::{use_api_list, use_api_when};
use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};
use crate::state::{loaded_site_settings, use_site_settings};
use crate::util::{FetchClass, classify_fetch, log_client_error};
use scuffed_api_client::{ApiClient, ClientError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyScreen {
    Loading,
    Error,
    Ready,
}

/// Auth still booting, or settings still in flight, stays on the loading line.
/// A failed settings load is its own screen so `/apply` cannot sit on
/// "Loading..." after the request has already failed.
fn apply_screen(auth_loading: bool, phase: FetchClass) -> ApplyScreen {
    if auth_loading || phase == FetchClass::Loading {
        ApplyScreen::Loading
    } else if phase == FetchClass::Error {
        ApplyScreen::Error
    } else {
        ApplyScreen::Ready
    }
}

/// Logged-in application slot. Outer `None` is still in flight. The middle
/// `None` is a failed fetch. The inner `None` is a successful "no application".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MineView {
    Pending,
    Failed,
    Form,
    Status,
}

fn mine_view<T>(
    state: UseResourceState,
    data: Option<Option<Option<&T>>>,
    error: Option<&str>,
) -> MineView {
    // A refetch is Pending while the previous value is still a failure.
    if state == UseResourceState::Pending {
        return MineView::Pending;
    }
    match data {
        None if error.is_none() => MineView::Pending,
        Some(Some(Some(_))) => MineView::Status,
        Some(Some(None)) if error.is_none() => MineView::Form,
        _ => MineView::Failed,
    }
}

/// A signed in member has nothing to apply for. Someone with a live
/// application (pending or trial) still sees its status and the Withdraw
/// button, so a trial member keeps that control.
fn already_member(is_member: bool, mine: MineView, status: Option<&str>) -> bool {
    if !is_member {
        return false;
    }
    match mine {
        MineView::Form => true,
        MineView::Status => !matches!(status, Some("pending" | "trial")),
        MineView::Pending | MineView::Failed => false,
    }
}

/// Pill tone and label for an application status, in the shared badge tones
/// (pending is warn, trial is info). A turned down application is a neutral
/// "Not accepted", not a red alarm.
fn status_pill(status: &str) -> (PillTone, String) {
    match status {
        "pending" => (PillTone::Warn, "Pending Review".to_string()),
        "trial" => (PillTone::Info, "Trial Period".to_string()),
        "accepted" => (PillTone::Ok, "Accepted".to_string()),
        "rejected" => (PillTone::Neutral, "Not accepted".to_string()),
        "withdrawn" => (PillTone::Neutral, "Withdrawn".to_string()),
        other => (PillTone::Neutral, other.to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotPhase {
    Loading,
    Failed,
    Ready,
}

/// A refetch is loading while the resource is `Pending`, including when the
/// last value was a failure (`Some(None)` with the error already cleared).
/// Otherwise an empty slot with no error is the first load, and a settled
/// error is the failure screen.
fn apply_fetch_phase(state: UseResourceState, slot_absent: bool, error: bool) -> SlotPhase {
    if state == UseResourceState::Pending || (slot_absent && !error) {
        SlotPhase::Loading
    } else if error {
        SlotPhase::Failed
    } else {
        SlotPhase::Ready
    }
}

/// Which card the Ready screen shows. One decision feeds both the body and the
/// "your application" status line, so the two cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyBodyView {
    Closed,
    SignIn,
    Member,
    Status,
    MinePending,
    MineFailed,
    Form,
}

fn apply_body_view(
    recruitment_open: bool,
    logged_in: bool,
    is_member: bool,
    has_status: bool,
    mine: MineView,
) -> ApplyBodyView {
    if !recruitment_open {
        ApplyBodyView::Closed
    } else if !logged_in {
        ApplyBodyView::SignIn
    } else if is_member {
        ApplyBodyView::Member
    } else if has_status {
        ApplyBodyView::Status
    } else {
        match mine {
            MineView::Pending => ApplyBodyView::MinePending,
            MineView::Failed => ApplyBodyView::MineFailed,
            MineView::Form | MineView::Status => ApplyBodyView::Form,
        }
    }
}

/// Retry control for a slot. `None` hides it, `Some(retrying)` keeps it mounted.
/// After a click it stays on screen, aria-disabled, while the refetch runs, so
/// focus does not drop to the page body.
fn retry_control(failed: bool, pending: bool, retrying: bool) -> Option<bool> {
    if failed {
        Some(false)
    } else if pending && retrying {
        Some(true)
    } else {
        None
    }
}

/// Text of the always-mounted page status line and the Retry for the
/// application fetch. The line carries every wait and failure of the page
/// itself: the settings load, a failed settings load, then the logged-in
/// application fetch. `body` is `None` until the Ready screen has settings.
/// The settings Retry is its own button on the error screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MineSlot {
    text: &'static str,
    retry: Option<bool>,
}

fn mine_slot(screen: ApplyScreen, body: Option<ApplyBodyView>, retrying: bool) -> MineSlot {
    let idle = |text| MineSlot { text, retry: None };
    match screen {
        ApplyScreen::Loading => return idle("Loading the page"),
        ApplyScreen::Error => return idle("Couldn't load site settings."),
        ApplyScreen::Ready => {}
    }
    let pending = body == Some(ApplyBodyView::MinePending);
    let failed = body == Some(ApplyBodyView::MineFailed);
    let text = if pending {
        "Loading your application"
    } else if failed {
        "Couldn't load your application."
    } else {
        ""
    };
    MineSlot {
        text,
        retry: retry_control(failed, pending, retrying),
    }
}

/// Text of the page-level games status line. `form` is false until the form
/// screen shows, so the region is mounted from first paint but stays empty;
/// a games fetch that fails early is announced as a text change once the form
/// appears, not lost in a region that mounts with the failure already in it.
///
/// `at_cap` is [`games_at_cap`] for the current choice. With the fetch idle
/// (Ready) the line says the limit is reached, and empties again once a game
/// is deselected. Loading and failure text always win over it.
fn games_slot(form: bool, phase: SlotPhase, retrying: bool, at_cap: bool) -> MineSlot {
    if !form {
        return MineSlot {
            text: "",
            retry: None,
        };
    }
    let text = match phase {
        SlotPhase::Loading => "Loading games",
        SlotPhase::Failed => "Couldn't load games.",
        SlotPhase::Ready if at_cap => GAMES_CAP_REACHED,
        SlotPhase::Ready => "",
    };
    MineSlot {
        text,
        retry: retry_control(
            phase == SlotPhase::Failed,
            phase == SlotPhase::Loading,
            retrying,
        ),
    }
}

/// Games grid rows as `(id, name, selected)`.
fn game_items(games: Vec<(String, String)>, selected: &[String]) -> Vec<(String, String, bool)> {
    games
        .into_iter()
        .map(|(id, name)| {
            let on = selected.contains(&id);
            (id, name, on)
        })
        .collect()
}

/// Select or deselect one game. A game past
/// [`MAX_APPLICATION_PREFERENCE_ITEMS`] is not added, since the server
/// refuses a longer list. Deselecting always works.
fn toggle_game(selected: &mut Vec<String>, id: &str) {
    if let Some(pos) = selected.iter().position(|x| x == id) {
        selected.remove(pos);
    } else if selected.len() < MAX_APPLICATION_PREFERENCE_ITEMS {
        selected.push(id.to_string());
    }
}

/// Accessible name for a Retry button. It starts with the visible text
/// ("Retry" or "Retrying...") so the label contains what is on screen
/// (WCAG 2.5.3 label in name), then says what is being loaded.
fn retry_aria_label(subject: &str, retrying: bool) -> String {
    format!("{} loading {subject}", retry_label(retrying))
}

/// Retry kept outside any status region. Mounted while in flight and marked
/// `aria-disabled` (not native `disabled`, which blurs the focused button and
/// drops focus to the page body); clicks are ignored until the refetch ends.
fn retry_button(
    class: &'static str,
    subject: &'static str,
    retrying: bool,
    on_retry: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            r#type: "button",
            class,
            aria_label: retry_aria_label(subject, retrying),
            aria_busy: if retrying { "true" },
            aria_disabled: if retrying { "true" },
            onclick: move |_| {
                if !retrying {
                    on_retry.call(());
                }
            },
            "{retry_label(retrying)}"
        }
    }
}

/// Always-mounted page status line (see [`mine_slot`]). Only its text
/// changes; Retry sits after it, outside the region.
#[component]
fn MineStatus(slot: MineSlot, on_retry: EventHandler<()>) -> Element {
    rsx! {
        p { class: "apply-status", role: "status", "{slot.text}" }
        if let Some(retrying) = slot.retry {
            div { class: "fetch-error-wrap",
                {retry_button("fetch-error__retry", "your application", retrying, on_retry)}
            }
        }
    }
}

/// DOM id of the games status line, referenced by the Games fieldset.
const GAMES_STATUS_ID: &str = "apply-games-status";

/// Always-mounted status line for the games fetch, rendered with the page
/// like [`MineStatus`]. Only its text changes; Retry sits after it, outside
/// the region.
#[component]
fn GamesStatus(slot: MineSlot, on_retry: EventHandler<()>) -> Element {
    rsx! {
        p { class: "apply-status", id: GAMES_STATUS_ID, role: "status", "{slot.text}" }
        if let Some(retrying) = slot.retry {
            div { class: "fetch-error-wrap",
                {retry_button("fetch-error__retry is-compact", "games", retrying, on_retry)}
            }
        }
    }
}

/// DOM id of the "choose at least one, at most 10" hint under the Games
/// legend.
const GAMES_HINT_ID: &str = "apply-games-hint";

/// The hint under the Games legend: the minimum and the cap, said before the
/// first click.
fn games_hint() -> String {
    format!("Choose at least one, at most {MAX_APPLICATION_PREFERENCE_ITEMS}.")
}

/// Games status line text once the cap is reached. A literal so
/// [`MineSlot::text`] stays `&'static str`; a test pins it to
/// [`MAX_APPLICATION_PREFERENCE_ITEMS`].
const GAMES_CAP_REACHED: &str = "Limit of 10 games reached";

/// Whether the games cap is reached, so no further game can be added.
fn games_at_cap(selected: &[String]) -> bool {
    selected.len() >= MAX_APPLICATION_PREFERENCE_ITEMS
}

/// Accessible name of a game chip that cannot be added at the cap. It starts
/// with the visible game name (WCAG 2.5.3) and says why.
fn game_at_cap_name(game: &str) -> String {
    format!("{game}, limit of {MAX_APPLICATION_PREFERENCE_ITEMS} games reached")
}

/// Ids that describe the Games group: its hint, the page-level status line
/// and, after a failed submit, its error message.
fn games_describedby(invalid: bool) -> String {
    let mut ids = vec![GAMES_HINT_ID, GAMES_STATUS_ID];
    if invalid {
        ids.push(ApplyField::Games.error_id());
    }
    ids.join(" ")
}

/// Games picker: a named group of toggle buttons. Its loading and error text
/// live in the page-level [`GamesStatus`] (mounted from first paint), which
/// the group references with `aria-describedby`. `games` is `(id, name)`
/// pairs. `children` follows the grid.
///
/// A group cannot carry `required` or `aria-invalid`, so the requirement is a
/// visible hint and the error is a described-by message. The group takes
/// focus from the error summary link.
///
/// At the cap ([`MAX_APPLICATION_PREFERENCE_ITEMS`] chosen) the unselected
/// chips are `aria-disabled`, look disabled and are named with the reason.
/// The selected chips stay toggles, so a deselect frees a slot and the others
/// come back.
#[component]
fn GamesField(
    phase: SlotPhase,
    games: Vec<(String, String)>,
    selected: Vec<String>,
    #[props(default = false)] invalid: bool,
    on_toggle: EventHandler<String>,
    children: Element,
) -> Element {
    let at_cap = games_at_cap(&selected);
    rsx! {
        fieldset {
            class: if invalid { "apply-field apply-fieldset is-error" } else { "apply-field apply-fieldset" },
            id: ApplyField::Games.id(),
            tabindex: "-1",
            aria_describedby: games_describedby(invalid),
            legend { class: "field-label",
                "{ApplyField::Games.label()}"
                {required_mark()}
            }
            p { class: "field-hint", id: GAMES_HINT_ID, "{games_hint()}" }
            if phase == SlotPhase::Ready {
                // The fieldset is the named group, so this row is a plain box.
                div { class: "chip-row",
                    for (id, name, is_selected) in game_items(games, &selected) {
                        ChipButton {
                            key: "{id}",
                            class: "chip",
                            name: (at_cap && !is_selected).then(|| game_at_cap_name(&name)),
                            label: name,
                            pressed: is_selected,
                            id: "apply-game-{id}",
                            unavailable: at_cap && !is_selected,
                            onclick: move |_| on_toggle.call(id.clone()),
                        }
                    }
                }
            }
            {children}
            if invalid {
                {field_error(ApplyField::Games)}
            }
        }
    }
}

// Local minimal type for checking existing application status.
#[derive(Debug, Clone, Deserialize)]
struct Application {
    #[allow(dead_code)]
    id: String,
    status: String,
}

// Local request type (no shared equivalent for application submission).
#[derive(Serialize)]
struct ApplyBody {
    preferred_games: Vec<String>,
    preferred_roles: Vec<String>,
    message: Option<String>,
}

/// Main role choices. The role is optional: a picked one is sent as the one
/// entry of `preferred_roles`, no pick sends an empty list. The label is the
/// value, because the admin review page prints the stored text.
const MAIN_ROLES: [&str; 4] = ["Tank", "Damage", "Support", "Flex"];

/// DOM id of the optional main role select.
const ROLE_ID: &str = "apply-role";

/// Shortest "Why you want to join" text the form sends. A short message is
/// enough; the rule only stops an empty or one-word answer. The server runs
/// the same check from `scuffed_types` (trim, then count chars; short is a
/// 400), so the form and the server cannot drift.
const MIN_MESSAGE_CHARS: usize = MIN_APPLICATION_MESSAGE_CHARS;

/// The body `POST /api/applications` gets. The one builder the submit
/// handler and the tests share.
fn apply_body(games: Vec<String>, role: &str, message: String) -> ApplyBody {
    let preferred_roles = if MAIN_ROLES.contains(&role) {
        vec![role.to_string()]
    } else {
        Vec::new()
    };
    ApplyBody {
        preferred_games: games,
        preferred_roles,
        message: if message.trim().is_empty() {
            None
        } else {
            Some(message)
        },
    }
}

/// DOM id of the error summary, focused after a failed submit.
const SUMMARY_ID: &str = "apply-error-summary";
/// DOM id of the summary heading, which names the summary when it takes focus.
const SUMMARY_HEADING_ID: &str = "apply-error-summary-title";
/// DOM id of the "Log in to apply" heading, focused when the session ended
/// under the page (a withdrawn trial).
const SIGN_IN_HEADING_ID: &str = "apply-sign-in-heading";
/// DOM id of the status card heading, focused after a successful submit.
const STATUS_HEADING_ID: &str = "apply-status-heading";
/// DOM id of the always-rendered hint under "Why you want to join".
const MESSAGE_HINT_ID: &str = "apply-message-hint";
/// DOM id of the form's send status line, where a refused send (400) shows
/// the server's reason.
const SEND_STATUS_ID: &str = "apply-send-status";
/// Id stem of the withdraw confirm strip; its trigger is `{stem}-trigger`.
const WITHDRAW_ID: &str = "withdraw-application";
const WITHDRAW_TRIGGER_ID: &str = "withdraw-application-trigger";

/// A required field of the form. One value gives the control id, the error
/// message id, the label and the error text, so the summary link, the
/// `aria-describedby` target and the inline message cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyField {
    Games,
    Message,
}

impl ApplyField {
    /// Id of the control a summary link moves focus to.
    const fn id(self) -> &'static str {
        match self {
            ApplyField::Games => "apply-games",
            ApplyField::Message => "apply-message",
        }
    }

    /// Id of the inline error message.
    const fn error_id(self) -> &'static str {
        match self {
            ApplyField::Games => "apply-games-err",
            ApplyField::Message => "apply-message-err",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            ApplyField::Games => "Games",
            ApplyField::Message => "Why you want to join",
        }
    }

    const fn error(self) -> &'static str {
        match self {
            ApplyField::Games => "Choose at least one game.",
            ApplyField::Message => "Write at least 10 characters so we know why you want to join.",
        }
    }
}

/// Invalid fields in form order. Empty means the form can be sent. The main
/// role is optional, so it is never invalid.
fn validate(games: &[String], message: &str) -> Vec<ApplyField> {
    let mut invalid = Vec::new();
    if games.is_empty() {
        invalid.push(ApplyField::Games);
    }
    if !application_message_long_enough(message) {
        invalid.push(ApplyField::Message);
    }
    invalid
}

/// The errors of the last submit that still stand. A field drops out as soon
/// as its value is valid again, so `aria-invalid`, the inline message and the
/// summary row never outlive the mistake. A change never adds an error: new
/// ones only appear on the next submit.
fn remaining_errors(errors: &[ApplyField], games: &[String], message: &str) -> Vec<ApplyField> {
    let invalid = validate(games, message);
    errors
        .iter()
        .copied()
        .filter(|field| invalid.contains(field))
        .collect()
}

/// Ids that describe the message field: its hint and, after a failed submit,
/// its error message.
fn message_describedby(invalid: bool) -> String {
    if invalid {
        format!("{MESSAGE_HINT_ID} {}", ApplyField::Message.error_id())
    } else {
        MESSAGE_HINT_ID.to_string()
    }
}

/// Where the wait for the status card stands after a send or a withdraw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusFocus {
    Idle,
    /// The refetch has been asked for but has not started. The old value is
    /// still in the slot, so it must not be taken for the new one.
    AfterRefetch,
    /// The refetch is running; focus the heading when it lands.
    WhenLoaded,
}

/// One step of that wait: the next state, and whether to focus the status
/// heading now. A refetch that settles with no application ends the wait.
fn status_focus_step(wait: StatusFocus, pending: bool, has_app: bool) -> (StatusFocus, bool) {
    match wait {
        StatusFocus::Idle => (StatusFocus::Idle, false),
        StatusFocus::AfterRefetch if pending => (StatusFocus::WhenLoaded, false),
        StatusFocus::AfterRefetch => (StatusFocus::AfterRefetch, false),
        StatusFocus::WhenLoaded if pending => (StatusFocus::WhenLoaded, false),
        StatusFocus::WhenLoaded => (StatusFocus::Idle, has_app),
    }
}

/// The `error` text of a JSON error body, trimmed. `None` when the server
/// sent no reason.
fn server_reason(err: &ClientError) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(err.http_body()?).ok()?;
    let reason = value.get("error")?.as_str()?.trim();
    (!reason.is_empty()).then(|| reason.to_string())
}

/// Toast for a failed send or withdraw. `what` is the opening clause with no
/// full stop ("Couldn't send the application").
///
/// "Try again" is only said when trying again can work. A 403 (recruitment
/// closed, banned) or a 409 (already applied, already a member) comes back
/// the same on every retry, so the toast gives the server's reason instead.
/// A 401 means the session ended, and no answer at all is a network failure.
///
/// A 429 is either the per-user wait or cap, whose reason says when or why
/// ("You can apply again in 42 minutes"), or the per-IP limit, whose body is
/// the code `rate_limited`. Neither is fixed by trying again at once.
fn action_failure_text(what: &str, err: &ClientError) -> String {
    let with_reason = |reason: String| {
        let stop = if reason.ends_with(['.', '!', '?']) {
            ""
        } else {
            "."
        };
        format!("{what}: {reason}{stop}")
    };
    match err.http_status() {
        Some(400 | 403 | 404 | 409) => match server_reason(err) {
            Some(reason) => with_reason(reason),
            None => format!("{what}. The server refused it."),
        },
        Some(429) => match server_reason(err).filter(|reason| reason != "rate_limited") {
            Some(reason) => with_reason(reason),
            None => format!("{what}. Too many attempts. Wait a minute, then try again."),
        },
        Some(401) => format!("{what}. Your session has ended. Log in again."),
        Some(_) => format!("{what}. Try again."),
        // No answer or an unreadable answer: the one mapping, so an
        // unreadable answer to a send that may have gone through is not
        // blamed on the connection or retried blindly (QA on 748).
        None => crate::client_error::two_sentences(
            what,
            crate::client_error::message_without_reason(err),
        ),
    }
}

/// Where a failed send is reported.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SendFailure {
    /// A 400: the server refused the content (the message rule). Its reason
    /// goes in the form's send status line, next to the button.
    Inline(String),
    /// Anything else stays a toast.
    Toast(String),
}

fn send_failure(err: &ClientError) -> SendFailure {
    let text = action_failure_text("Couldn't send the application", err);
    if err.http_status() == Some(400) {
        SendFailure::Inline(text)
    } else {
        SendFailure::Toast(text)
    }
}

/// True when the application fetch failed because the session is gone. A
/// trial applicant who withdraws is logged out by the server, so the refetch
/// that follows comes back 401 and a Retry could never work.
fn session_ended(logged_in: bool, mine_error: Option<&str>) -> bool {
    logged_in && mine_error.is_some_and(|err| is_http_status(err, 401))
}

/// Body of the withdraw confirm. Withdrawing during a trial also ends the
/// trial and logs the applicant out, so the strip says so before the click.
///
/// Applying again is not promised: the server makes them wait
/// [`APPLICATION_RESUBMIT_COOLDOWN_SECS`], refuses past
/// [`MAX_WITHDRAWN_APPLICATIONS`] withdrawals in
/// [`WITHDRAWN_APPLICATION_WINDOW_DAYS`] days, and recruitment may be closed.
fn withdraw_confirm_body(status: &str) -> String {
    let minutes = APPLICATION_RESUBMIT_COOLDOWN_SECS / 60;
    let wait = if minutes == 60 {
        "an hour".to_string()
    } else {
        format!("{minutes} minutes")
    };
    let again = format!(
        "You have to wait {wait} before you can apply again, and only while recruitment is open. \
         After {MAX_WITHDRAWN_APPLICATIONS} withdrawals in {WITHDRAWN_APPLICATION_WINDOW_DAYS} \
         days you have to wait longer."
    );
    if status == "trial" {
        format!("This ends your trial and logs you out. {again}")
    } else {
        again
    }
}

/// Second line of the error summary.
fn summary_count_text(count: usize) -> String {
    if count == 1 {
        "Fix the 1 item below, then send it again.".to_string()
    } else {
        format!("Fix the {count} items below, then send it again.")
    }
}

/// Page title. The organisation name comes from site settings, so the title
/// is the short form until they load.
fn page_title(org_name: Option<&str>) -> String {
    match org_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("Apply to {name}"),
        None => "Apply".to_string(),
    }
}

/// Heading of the status card. Right after a send it is the confirmation.
fn status_heading(just_sent: bool, status: &str) -> &'static str {
    if just_sent && status == "pending" {
        "Application received"
    } else {
        "Application status"
    }
}

/// Games and the message are required and the main role is optional, so the
/// lead says so in words. The asterisks and the "* Required" key are the
/// visual form of the same fact.
const FORM_LEAD: &str = "Tell us what you play and why you want to join. Games and the message are required; your main role is optional.";

/// Page layout. Fields, labels, hints, error lines and buttons are the shared
/// ones from `components/ui`.
const APPLY_CSS: &str = r#"
    .apply-body { min-width: 0; }
    .apply-page .ui-card { padding: var(--space-section); }
    .apply-card-title { margin: 0 0 var(--space-2); font-family: var(--font-head); font-weight: var(--font-head-weight); font-size: var(--text-xl); line-height: var(--leading-display); color: var(--text); }
    .apply-card-desc { margin: 0; color: var(--text-3); font-size: var(--text-md); line-height: var(--leading-body); }
    .apply-status-row { margin: var(--space-3) 0; }
    .apply-actions { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); margin-top: var(--space-5); }
    .apply-page .inline-confirm { margin-top: var(--space-5); }
    /* The confirm strip's two buttons take the page's button type and sit
       where the page's other actions do, so the strip reads as part of the
       card it opens in. */
    .apply-page .inline-confirm__actions { justify-content: flex-start; }
    .apply-page .inline-confirm__hint { text-align: left; }
    .apply-page .inline-confirm__cancel,
    .apply-page .inline-confirm__go { min-height: var(--hit-target-min); padding: var(--space-2) var(--button-pad-x); font-family: var(--button-font); font-size: var(--button-font-size); font-weight: var(--button-font-weight); letter-spacing: var(--font-head-tracking); line-height: var(--leading-ui); border-radius: var(--button-radius); }
    .apply-page .inline-confirm__cancel { border-color: var(--border-strong); }
    .apply-page .inline-confirm__cancel:hover { border-color: var(--text); }
    .apply-status { margin: 0; color: var(--text-3); font-size: var(--text-sm); line-height: var(--leading-body); }
    .apply-status:not(:empty) { margin: 0 0 var(--space-3); }
    /* The send line only ever holds a refusal, so it takes the error ink the
       field errors use, not the muted status grey. */
    .apply-send-status { color: var(--loss); }

    .apply-form { margin: 0; }
    .apply-summary { margin: 0 0 var(--space-5); padding: var(--space-4); border: 1px solid var(--loss); border-radius: var(--radius-card); background: var(--loss-soft); color: var(--text); }
    .apply-summary h2 { margin: 0 0 var(--space-2); font-family: var(--font-head); font-size: var(--text-md); font-weight: var(--font-label-weight); line-height: var(--leading-ui); color: var(--loss); }
    .apply-summary p { margin: 0 0 var(--space-3); font-size: var(--text-md); line-height: var(--leading-body); }
    .apply-summary ul { margin: 0; padding-inline-start: var(--space-5); list-style: disc; }
    .apply-summary li { font-size: var(--text-md); line-height: var(--leading-body); }
    .apply-summary li + li { margin-top: var(--space-2); }
    .apply-summary a { color: var(--accent-text); text-decoration: underline; }
    .apply-legend { margin: 0 0 var(--space-5); color: var(--text-3); font-size: var(--text-sm); }
    .apply-req { color: var(--loss); font-weight: var(--font-label-weight); margin-inline-start: var(--space-1); }
    .apply-legend .apply-req { margin-inline-start: 0; }

    .apply-field { display: flex; flex-direction: column; gap: var(--space-2); margin: 0; min-width: 0; }
    .apply-field + .apply-field,
    .apply-field + .apply-actions { margin-top: var(--space-5); }
    .apply-fieldset { border: 0; padding: 0; }
    .apply-fieldset > legend { padding: 0; margin-bottom: var(--space-2); }
    .apply-fieldset.is-error .chip:not([aria-pressed="true"]) { border-color: var(--loss); }
    .apply-fieldset:focus-visible { outline: var(--focus-ring-width) solid var(--focus-ring); outline-offset: var(--focus-ring-offset); }

    @media (max-width: 820px) {
        .apply-form .apply-actions { flex-direction: column; align-items: stretch; }
        .apply-form .apply-actions .btn { width: 100%; }
        .apply-page .inline-confirm__actions { flex-direction: column; align-items: stretch; }
        /* 44px targets from block padding, not a min-height: a link that
           wraps to two lines keeps the same space around it as a short one,
           so neighbours never run together. */
        .apply-summary li + li { margin-top: 0; }
        .apply-summary a { display: block; padding-block: var(--space-3); }
    }
"#;

/// Inline error under a field: the shared icon-then-text line.
fn field_error(field: ApplyField) -> Element {
    rsx! { FieldError { id: field.error_id(), text: field.error() } }
}

/// The asterisk next to a required label. Hidden from assistive tech, which
/// gets `required` from the control.
fn required_mark() -> Element {
    rsx! { span { class: "apply-req", aria_hidden: "true", "*" } }
}

/// The application form. `errors` is the result of the last submit, empty
/// until one fails, so the first paint shows no errors and no summary.
/// The submit button is never natively disabled: after errors the form can be
/// corrected and sent again, and while sending it keeps focus.
/// `children` goes after the submit button (the Cancel link).
#[component]
fn ApplyForm(
    games_phase: SlotPhase,
    games: Vec<(String, String)>,
    selected: Vec<String>,
    role: String,
    message: String,
    errors: Vec<ApplyField>,
    submitting: bool,
    /// Text of the send status line: the server's reason after a 400, empty
    /// otherwise. The line is always rendered; only this text changes.
    #[props(default)]
    send_status: String,
    on_toggle: EventHandler<String>,
    on_role: EventHandler<String>,
    on_message: EventHandler<String>,
    on_submit: EventHandler<()>,
    /// Shown under the games grid (the "Load more" notice).
    #[props(default)]
    games_notice: Option<Element>,
    children: Element,
) -> Element {
    let message_invalid = errors.contains(&ApplyField::Message);
    rsx! {
        form {
            class: "ui-card apply-form",
            novalidate: true,
            onsubmit: move |e: FormEvent| {
                e.prevent_default();
                on_submit.call(());
            },
            if !errors.is_empty() {
                // Not a live region: a failed submit moves focus here, which
                // reads it once. As `role="alert"` it would be read a second
                // time, and again each time a corrected field shortens it.
                div {
                    class: "apply-summary",
                    id: SUMMARY_ID,
                    role: "group",
                    tabindex: "-1",
                    aria_labelledby: SUMMARY_HEADING_ID,
                    h2 { id: SUMMARY_HEADING_ID, "The application could not be sent" }
                    p { "{summary_count_text(errors.len())}" }
                    ul {
                        for field in errors.iter().copied() {
                            li { key: "{field.id()}",
                                a {
                                    href: "#{field.id()}",
                                    onclick: move |e: MouseEvent| {
                                        e.prevent_default();
                                        // Focus the control and show the whole
                                        // field, error line included.
                                        focus_field_by_id(field.id(), ".apply-field");
                                    },
                                    "{field.label()}: {field.error()}"
                                }
                            }
                        }
                    }
                }
            }
            // A visual key for the asterisks. Assistive tech gets `required`
            // from each control and the lead's own words, so the key is hidden.
            p { class: "apply-legend", aria_hidden: "true",
                span { class: "apply-req", "*" }
                " Required"
            }

            // Games loading / error text lives in `GamesStatus` above the form.
            GamesField {
                phase: games_phase,
                games,
                selected,
                invalid: errors.contains(&ApplyField::Games),
                on_toggle,
                {games_notice}
            }

            // Optional: no asterisk, no `required`, never an error. The
            // label says so in words.
            div { class: "apply-field",
                label { class: "field-label", r#for: ROLE_ID, "Main role (optional)" }
                select {
                    class: "field-input",
                    id: ROLE_ID,
                    name: "role",
                    onchange: move |e: FormEvent| on_role.call(e.value()),
                    option { value: "", selected: role.is_empty(), "No main role" }
                    for name in MAIN_ROLES {
                        option { key: "{name}", value: name, selected: role == name, "{name}" }
                    }
                }
            }

            div { class: "apply-field",
                label { class: "field-label", r#for: ApplyField::Message.id(),
                    "{ApplyField::Message.label()}"
                    {required_mark()}
                }
                // The minimum and the maximum are said before the first
                // submit, not only after a failed one.
                p { class: "field-hint", id: MESSAGE_HINT_ID,
                    "At least {MIN_MESSAGE_CHARS} characters, at most {MAX_APPLICATION_MESSAGE_CHARS}."
                }
                textarea {
                    class: "field-input",
                    id: ApplyField::Message.id(),
                    name: "message",
                    required: true,
                    // The server refuses a longer message (400). The cap is
                    // shared with it through `scuffed_types`.
                    maxlength: "{MAX_APPLICATION_MESSAGE_CHARS}",
                    aria_required: "true",
                    aria_invalid: if message_invalid { "true" },
                    aria_describedby: message_describedby(message_invalid),
                    value: "{message}",
                    oninput: move |e: FormEvent| on_message.call(e.value()),
                }
                if message_invalid {
                    {field_error(ApplyField::Message)}
                }
            }

            // Always rendered with the form, so a refused send is announced
            // as a text change. Empty until the server refuses one.
            p { class: "apply-status apply-send-status", id: SEND_STATUS_ID, role: "status", "{send_status}" }

            div { class: "apply-actions",
                button {
                    r#type: "submit",
                    class: btn_class(true),
                    aria_busy: if submitting { "true" },
                    aria_disabled: if submitting { "true" },
                    if submitting {
                        "Sending..."
                    } else {
                        "Send application"
                    }
                }
                {children}
            }
        }
    }
}

#[component]
pub fn Apply() -> Element {
    let mut auth = use_auth();
    let mut toast = use_toast();

    let mut settings = use_site_settings();
    let resolved = settings.resolved.read();
    let settings_phase = classify_fetch(resolved.as_ref());
    let s = loaded_site_settings(resolved.as_ref());
    let mut games = use_api_list::<Game>("/api/games");
    // Only a signed-in viewer has an application to ask for. A guest gets the
    // "Log in to apply" card and sends no request.
    let mut my_app = use_api_when::<Option<Application>>("/api/applications/mine", move || {
        auth().is_logged_in()
    });

    let mut selected_games = use_signal(Vec::<String>::new);
    let mut role = use_signal(String::new);
    let mut message = use_signal(String::new);
    // Result of the last submit. Empty until a submit fails validation.
    let mut errors = use_signal(Vec::<ApplyField>::new);
    // The server's reason for the last refused (400) send. Empty otherwise.
    let mut send_status = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    // True from a successful send until the application changes again.
    let mut just_sent = use_signal(|| false);
    // A send or a withdraw succeeded and the status card has not taken focus
    // yet.
    let mut focus_status = use_signal(|| StatusFocus::Idle);
    let mut confirm_withdraw = use_signal(|| false);
    // Set by a Retry click; cleared once that refetch settles.
    let mut mine_retrying = use_signal(|| false);
    let mut games_retrying = use_signal(|| false);
    use_effect(move || {
        if my_app.data.state()() != UseResourceState::Pending {
            mine_retrying.set(false);
        }
    });
    use_effect(move || {
        if games.data.state()() != UseResourceState::Pending {
            games_retrying.set(false);
        }
    });
    // A send or a withdraw refetches the application, and the form or the
    // status card unmounts while that runs. Once the new application is on
    // screen, move focus to its heading so it does not stay on the page body.
    use_effect(move || {
        let pending = my_app.data.state()() == UseResourceState::Pending;
        let has_app = matches!(my_app.data.read().as_ref(), Some(Some(Some(_))));
        let (next, focus_now) = status_focus_step(focus_status(), pending, has_app);
        if next != focus_status() {
            focus_status.set(next);
        }
        if focus_now {
            focus_element_by_id_soon(STATUS_HEADING_ID.to_string());
        }
    });

    // The server ended the session (a withdrawn trial): drop the stale login
    // so the page shows "Log in to apply" instead of a Retry that cannot
    // work, and put focus on that card.
    use_effect(move || {
        let logged_in = auth().is_logged_in();
        if session_ended(logged_in, my_app.error.read().as_deref()) {
            auth.set(AuthState {
                user: None,
                loading: false,
            });
            focus_status.set(StatusFocus::Idle);
            focus_element_by_id_soon(SIGN_IN_HEADING_ID.to_string());
        }
    });

    let loading = auth().loading;
    let screen = apply_screen(loading, settings_phase);
    let org_name = s.as_ref().map(|x| x.org_name.clone());
    let mine_data = my_app.data.read();
    let mine_error = my_app.error.read();
    let mine = mine_view(
        my_app.data.state()(),
        mine_data
            .as_ref()
            .map(|outer| outer.as_ref().map(|inner| inner.as_ref())),
        mine_error.as_deref(),
    );
    let status_app = mine_data
        .as_ref()
        .and_then(|outer| outer.as_ref())
        .and_then(|inner| inner.as_ref())
        .filter(|_| mine == MineView::Status)
        .cloned();
    let body_view = s
        .as_ref()
        .filter(|_| screen == ApplyScreen::Ready)
        .map(|s| {
            let is_member = already_member(
                auth().is_org_member(),
                mine,
                status_app.as_ref().map(|a| a.status.as_str()),
            );
            apply_body_view(
                s.recruitment_open,
                auth().is_logged_in(),
                is_member,
                status_app.is_some(),
                mine,
            )
        });
    let slot = mine_slot(screen, body_view, mine_retrying());
    let games_data = games.data.read();
    let games_phase = apply_fetch_phase(
        games.data.state()(),
        games_data.as_ref().is_none(),
        games.error.read().is_some(),
    );
    let game_list: Vec<(String, String)> = games_data
        .as_ref()
        .and_then(|g| g.as_ref())
        .map(|g| g.iter().map(|g| (g.id.clone(), g.name.clone())).collect())
        .unwrap_or_default();
    drop(games_data);
    let games_status = games_slot(
        body_view == Some(ApplyBodyView::Form),
        games_phase,
        games_retrying(),
        games_at_cap(&selected_games.read()),
    );
    let lead = (body_view == Some(ApplyBodyView::Form)).then(|| FORM_LEAD.to_string());

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-apply-apply-css", css: APPLY_CSS }
        PageShell { width: PageWidth::Narrow, class: "apply-page",
            PageHeader { title: page_title(org_name.as_deref()), lead }

            div { class: "apply-body",
            // Mounted on every screen (Loading, Error, Ready) so a failed
            // application fetch is announced as a text change. Empty when idle.
            MineStatus {
                slot,
                on_retry: move |_| {
                    mine_retrying.set(true);
                    my_app.refresh += 1;
                },
            }
            // Mounted with the page so an early /api/games failure lands in a
            // region that already exists. Empty until the form shows.
            GamesStatus {
                slot: games_status,
                on_retry: move |_| {
                    games_retrying.set(true);
                    games.refresh += 1;
                },
            }

            // The Loading screen is the status line above; nothing else shows.
            if let (Some(s), Some(view)) = (s, body_view) {
                {
                    let org_name = s.org_name.clone();
                    match (view, status_app.clone()) {
                        (ApplyBodyView::Closed, _) => rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Recruitment closed" }
                                p { class: "apply-card-desc", "{s.recruitment_message}" }
                            }
                        },
                        (ApplyBodyView::SignIn, _) => rsx! {
                            Card {
                                h2 {
                                    class: "apply-card-title",
                                    id: SIGN_IN_HEADING_ID,
                                    tabindex: "-1",
                                    "Log in to apply"
                                }
                                p { class: "apply-card-desc", "Log in before you send an application." }
                                div { class: "apply-actions",
                                    Link { to: Route::Login {}, class: btn_class(true), "Log in" }
                                    // Only in debug builds: route is not registered in production
                                    if cfg!(debug_assertions) {
                                        a {
                                            href: "/api/dev/login",
                                            class: btn_class(false),
                                            "Dev login"
                                        }
                                    }
                                }
                            }
                        },
                        (ApplyBodyView::Member, _) => rsx! {
                            Card {
                                h2 { class: "apply-card-title", "You are already in {org_name}" }
                                p { class: "apply-card-desc", "Your membership is active, so there is nothing to apply for." }
                                div { class: "apply-actions",
                                    Link { to: Route::ProfileSettings {}, class: btn_class(true), "Open your profile" }
                                    Link { to: Route::Members {}, class: btn_class(false), "See the crew" }
                                }
                            }
                        },
                        (ApplyBodyView::Status, Some(app)) => {
                            let (status_tone, status_label) = status_pill(app.status.as_str());
                            let desc = match app.status.as_str() {
                                "pending" => {
                                    "Your application is being reviewed. We'll get back to you soon."
                                        .to_string()
                                }
                                "trial" => {
                                    "You're in your trial period. Show up, have fun, and be yourself."
                                        .to_string()
                                }
                                "accepted" => format!("Welcome aboard! You're a member of {org_name}."),
                                "rejected" => {
                                    "Unfortunately your application was not accepted at this time."
                                        .to_string()
                                }
                                "withdrawn" => {
                                    "You withdrew this application. You can re-apply later if recruitment is open."
                                        .to_string()
                                }
                                _ => String::new(),
                            };
                            let can_withdraw = app.status == "pending" || app.status == "trial";
                            let heading = status_heading(just_sent(), app.status.as_str());
                            let withdraw_label = if submitting() { "Withdrawing..." } else { "Withdraw" };
                            rsx! {
                                Card {
                                    h2 {
                                        class: "apply-card-title",
                                        id: STATUS_HEADING_ID,
                                        tabindex: "-1",
                                        "{heading}"
                                    }
                                    div { class: "apply-status-row",
                                        Pill { tone: status_tone, "{status_label}" }
                                    }
                                    p { class: "apply-card-desc", "{desc}" }
                                    if can_withdraw {
                                        if confirm_withdraw() {
                                            // The strip stays up, busy, until the request
                                            // settles: closing it on click would unmount the
                                            // focused button and drop focus to the page.
                                            InlineConfirm {
                                                id: WITHDRAW_ID.to_string(),
                                                title: "Withdraw your application?".to_string(),
                                                body: withdraw_confirm_body(app.status.as_str()),
                                                confirm_label: withdraw_label.to_string(),
                                                busy: submitting(),
                                                on_confirm: move |_| {
                                                    if submitting() {
                                                        return;
                                                    }
                                                    submitting.set(true);
                                                    spawn(async move {
                                                        match ApiClient::web()
                                                            .post_json_empty(
                                                                "/api/applications/mine/withdraw",
                                                                &serde_json::json!({}),
                                                            )
                                                            .await
                                                        {
                                                            Ok(_) => {
                                                                toast.show(Toast::success("Application withdrawn"));
                                                                just_sent.set(false);
                                                                // The card comes back without Withdraw;
                                                                // its heading takes focus then.
                                                                focus_status.set(StatusFocus::AfterRefetch);
                                                                my_app.refresh += 1;
                                                            }
                                                            Err(e) => {
                                                                log_client_error("withdraw application", &e);
                                                                toast.show(Toast::error(action_failure_text(
                                                                    "Couldn't withdraw the application",
                                                                    &e,
                                                                )));
                                                                // A refused withdraw means the status on
                                                                // screen is stale: show the current one.
                                                                if matches!(e.http_status(), Some(400 | 401 | 403 | 404 | 409)) {
                                                                    my_app.refresh += 1;
                                                                }
                                                                // Back to the button that opened the strip.
                                                                focus_element_by_id_soon(WITHDRAW_TRIGGER_ID.to_string());
                                                            }
                                                        }
                                                        confirm_withdraw.set(false);
                                                        submitting.set(false);
                                                    });
                                                },
                                                on_cancel: move |_| confirm_withdraw.set(false),
                                            }
                                        } else {
                                            div { class: "apply-actions",
                                                button {
                                                    r#type: "button",
                                                    id: WITHDRAW_TRIGGER_ID,
                                                    class: btn_class(false),
                                                    onclick: move |_| confirm_withdraw.set(true),
                                                    "Withdraw application"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // The status line and its Retry live in `MineStatus` above.
                        (ApplyBodyView::Status, None)
                        | (ApplyBodyView::MinePending | ApplyBodyView::MineFailed, _) => rsx! {},
                        (ApplyBodyView::Form, _) => {
                            rsx! {
                                ApplyForm {
                                    games_phase,
                                    games: game_list,
                                    selected: selected_games(),
                                    role: role(),
                                    message: message(),
                                    errors: errors(),
                                    submitting: submitting(),
                                    send_status: send_status(),
                                    // A corrected field loses its error at once, not
                                    // on the next submit.
                                    on_toggle: move |id: String| {
                                        toggle_game(&mut selected_games.write(), &id);
                                        let left = remaining_errors(&errors(), &selected_games(), &message());
                                        errors.set(left);
                                    },
                                    // The role is optional and has no error to clear.
                                    on_role: move |value: String| role.set(value),
                                    on_message: move |value: String| {
                                        message.set(value);
                                        let left = remaining_errors(&errors(), &selected_games(), &message());
                                        errors.set(left);
                                    },
                                    on_submit: move |_| {
                                        // The button stays focusable while a send runs.
                                        if submitting() {
                                            return;
                                        }
                                        let games = selected_games();
                                        let main_role = role();
                                        let msg = message();
                                        let invalid = validate(&games, &msg);
                                        let failed = !invalid.is_empty();
                                        errors.set(invalid);
                                        if failed {
                                            focus_element_by_id_soon(SUMMARY_ID.to_string());
                                            return;
                                        }
                                        submitting.set(true);
                                        send_status.set(String::new());
                                        spawn(async move {
                                            let body = apply_body(games, &main_role, msg);
                                            match ApiClient::web().post_json_empty("/api/applications", &body).await {
                                                Ok(_) => {
                                                    toast.show(Toast::success("Application received"));
                                                    just_sent.set(true);
                                                    focus_status.set(StatusFocus::AfterRefetch);
                                                    my_app.refresh += 1;
                                                }
                                                Err(e) => {
                                                    log_client_error("send application", &e);
                                                    match send_failure(&e) {
                                                        SendFailure::Inline(text) => send_status.set(text),
                                                        SendFailure::Toast(text) => toast.show(Toast::error(text)),
                                                    }
                                                }
                                            }
                                            submitting.set(false);
                                        });
                                    },
                                    games_notice: list_cap_notice(&games, "games"),
                                    Link { to: Route::Home {}, class: btn_class(false), "Cancel" }
                                }
                            }
                        }
                    }
                }
            } else if screen == ApplyScreen::Error {
                // Settings failed. The words are the status line above; Retry
                // sits outside any live region.
                div { class: "fetch-error-wrap",
                    button {
                        r#type: "button",
                        class: "fetch-error__retry",
                        aria_label: retry_aria_label("site settings", false),
                        onclick: move |_| settings.refresh += 1,
                        "{retry_label(false)}"
                    }
                }
            }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_types::application_message_chars;

    #[test]
    fn rejected_is_a_neutral_not_accepted_chip() {
        let (tone, label) = status_pill("rejected");
        assert_eq!(label, "Not accepted");
        assert!(tone == PillTone::Neutral);
        assert!(!label.contains("Rejected"));
        assert!(status_pill("pending").0 == PillTone::Warn);
        assert!(status_pill("accepted").0 == PillTone::Ok);
        assert_eq!(status_pill("mystery").1, "mystery");
    }

    #[test]
    fn members_are_told_they_are_already_in() {
        // Old rejected or withdrawn applications and the empty form both give way.
        assert!(already_member(true, MineView::Form, None));
        assert!(already_member(true, MineView::Status, Some("rejected")));
        assert!(already_member(true, MineView::Status, Some("withdrawn")));
        assert!(already_member(true, MineView::Status, Some("accepted")));
        // A live application keeps its status and Withdraw button.
        assert!(!already_member(true, MineView::Status, Some("pending")));
        assert!(!already_member(true, MineView::Status, Some("trial")));
        // No flash while the application is still loading, or when it failed.
        assert!(!already_member(true, MineView::Pending, None));
        assert!(!already_member(true, MineView::Failed, None));
        // Non members never see it.
        assert!(!already_member(false, MineView::Form, None));
        assert!(!already_member(false, MineView::Status, Some("rejected")));
    }

    #[test]
    fn settings_failure_is_not_an_endless_loading_state() {
        assert_eq!(apply_screen(false, FetchClass::Error), ApplyScreen::Error);
        assert_eq!(
            apply_screen(false, FetchClass::Loading),
            ApplyScreen::Loading
        );
        assert_eq!(apply_screen(true, FetchClass::Ready), ApplyScreen::Loading);
        assert_eq!(apply_screen(false, FetchClass::Ready), ApplyScreen::Ready);
        assert_ne!(apply_screen(false, FetchClass::Error), ApplyScreen::Loading);
    }

    #[test]
    fn logged_in_apply_waits_for_the_existing_application() {
        assert_eq!(
            mine_view(UseResourceState::Ready, None::<Option<Option<&()>>>, None),
            MineView::Pending
        );
        assert_eq!(
            mine_view(
                UseResourceState::Ready,
                Some(None::<Option<&()>>),
                Some("offline")
            ),
            MineView::Failed
        );
        // Deleting the Pending override leaves this as Failed.
        assert_eq!(
            mine_view(
                UseResourceState::Pending,
                Some(None::<Option<&()>>),
                Some("offline")
            ),
            MineView::Pending
        );
        assert_eq!(
            mine_view(UseResourceState::Ready, Some(Some(None::<&()>)), None),
            MineView::Form
        );
        assert_eq!(
            mine_view(UseResourceState::Ready, Some(Some(Some(&()))), None),
            MineView::Status
        );
        assert_ne!(
            mine_view(UseResourceState::Ready, None::<Option<Option<&()>>>, None),
            MineView::Form
        );
    }

    #[test]
    fn retry_in_flight_stays_loading_when_the_last_value_failed() {
        // `use_api_list` clears the error when a refetch starts and leaves
        // `Some(None)` in the slot. That used to paint an empty games grid.
        assert_eq!(
            apply_fetch_phase(UseResourceState::Pending, false, false),
            SlotPhase::Loading
        );
        assert_eq!(
            apply_fetch_phase(UseResourceState::Pending, false, true),
            SlotPhase::Loading
        );
        assert_eq!(
            apply_fetch_phase(UseResourceState::Ready, false, true),
            SlotPhase::Failed
        );
        assert_eq!(
            apply_fetch_phase(UseResourceState::Ready, true, false),
            SlotPhase::Loading
        );
        assert_eq!(
            apply_fetch_phase(UseResourceState::Ready, false, false),
            SlotPhase::Ready
        );
    }

    #[test]
    fn body_view_follows_the_page_decision_order() {
        use ApplyBodyView as V;
        // Closed and signed out win over everything else.
        assert_eq!(
            apply_body_view(false, true, true, true, MineView::Failed),
            V::Closed
        );
        assert_eq!(
            apply_body_view(true, false, false, false, MineView::Failed),
            V::SignIn
        );
        assert_eq!(
            apply_body_view(true, true, true, true, MineView::Status),
            V::Member
        );
        assert_eq!(
            apply_body_view(true, true, false, true, MineView::Status),
            V::Status
        );
        assert_eq!(
            apply_body_view(true, true, false, false, MineView::Pending),
            V::MinePending
        );
        assert_eq!(
            apply_body_view(true, true, false, false, MineView::Failed),
            V::MineFailed
        );
        assert_eq!(
            apply_body_view(true, true, false, false, MineView::Form),
            V::Form
        );
    }

    #[test]
    fn mine_slot_text_only_shows_for_the_application_fetch() {
        use ApplyBodyView as V;
        // Loading and settings-error screens speak through the same line.
        // Their Retry is not the application one.
        for retrying in [false, true] {
            assert_eq!(
                mine_slot(ApplyScreen::Loading, None, retrying),
                MineSlot {
                    text: "Loading the page",
                    retry: None
                }
            );
            assert_eq!(
                mine_slot(ApplyScreen::Error, None, retrying),
                MineSlot {
                    text: "Couldn't load site settings.",
                    retry: None
                }
            );
        }
        for view in [V::Closed, V::SignIn, V::Member, V::Status, V::Form] {
            assert_eq!(
                mine_slot(ApplyScreen::Ready, Some(view), false),
                MineSlot {
                    text: "",
                    retry: None
                }
            );
        }
        assert_eq!(
            mine_slot(ApplyScreen::Ready, Some(V::MinePending), false),
            MineSlot {
                text: "Loading your application",
                retry: None
            }
        );
        assert_eq!(
            mine_slot(ApplyScreen::Ready, Some(V::MineFailed), false),
            MineSlot {
                text: "Couldn't load your application.",
                retry: Some(false)
            }
        );
        // A Retry click keeps the button mounted and aria-disabled while in flight.
        assert_eq!(
            mine_slot(ApplyScreen::Ready, Some(V::MinePending), true),
            MineSlot {
                text: "Loading your application",
                retry: Some(true)
            }
        );
    }

    #[test]
    fn retry_stays_mounted_only_for_a_failure_or_its_refetch() {
        assert_eq!(retry_control(true, false, false), Some(false));
        assert_eq!(retry_control(true, false, true), Some(false));
        assert_eq!(retry_control(false, true, true), Some(true));
        assert_eq!(retry_control(false, true, false), None);
        assert_eq!(retry_control(false, false, true), None);
    }

    #[test]
    fn game_toggle_adds_then_removes() {
        let mut selected = vec!["ow".to_string()];
        toggle_game(&mut selected, "val");
        assert_eq!(selected, ["ow", "val"]);
        toggle_game(&mut selected, "ow");
        assert_eq!(selected, ["val"]);
    }

    #[test]
    fn an_eleventh_game_is_ignored_and_deselect_still_works_at_ten() {
        let mut selected: Vec<String> = Vec::new();
        for n in 0..MAX_APPLICATION_PREFERENCE_ITEMS {
            toggle_game(&mut selected, &format!("game-{n}"));
        }
        assert_eq!(MAX_APPLICATION_PREFERENCE_ITEMS, 10);
        assert_eq!(selected.len(), 10);
        let ten = selected.clone();

        // The eleventh is not added, however often it is tried.
        toggle_game(&mut selected, "game-10");
        toggle_game(&mut selected, "game-10");
        assert_eq!(selected, ten);

        // Deselect still works at ten, and frees a place.
        toggle_game(&mut selected, "game-3");
        assert_eq!(selected.len(), 9);
        assert!(!selected.contains(&"game-3".to_string()));
        toggle_game(&mut selected, "game-10");
        assert_eq!(selected.len(), 10);
        assert_eq!(selected.last().map(String::as_str), Some("game-10"));
        toggle_game(&mut selected, "game-3");
        assert_eq!(selected.len(), 10);
        assert!(!selected.contains(&"game-3".to_string()));
    }

    #[test]
    fn game_items_mark_the_selected_games() {
        let items = game_items(games(), &["game-val".to_string()]);
        assert_eq!(
            items,
            [
                ("game-ow".to_string(), "Overwatch".to_string(), false),
                ("game-val".to_string(), "Valorant".to_string(), true),
            ]
        );
    }

    #[test]
    fn games_slot_is_empty_until_the_form_shows() {
        for phase in [SlotPhase::Loading, SlotPhase::Failed, SlotPhase::Ready] {
            for retrying in [false, true] {
                assert_eq!(
                    games_slot(false, phase, retrying, false),
                    MineSlot {
                        text: "",
                        retry: None
                    }
                );
            }
        }
        assert_eq!(
            games_slot(true, SlotPhase::Ready, false, false),
            MineSlot {
                text: "",
                retry: None
            }
        );
        assert_eq!(
            games_slot(true, SlotPhase::Loading, false, false),
            MineSlot {
                text: "Loading games",
                retry: None
            }
        );
        assert_eq!(
            games_slot(true, SlotPhase::Failed, false, false),
            MineSlot {
                text: "Couldn't load games.",
                retry: Some(false)
            }
        );
        assert_eq!(
            games_slot(true, SlotPhase::Loading, true, false),
            MineSlot {
                text: "Loading games",
                retry: Some(true)
            }
        );
    }

    #[test]
    fn games_slot_says_the_cap_only_when_idle() {
        // The literal tracks the shared cap.
        assert_eq!(
            GAMES_CAP_REACHED,
            format!("Limit of {MAX_APPLICATION_PREFERENCE_ITEMS} games reached")
        );
        assert_eq!(GAMES_CAP_REACHED, "Limit of 10 games reached");

        // Set: picking the 10th game while the fetch is idle.
        let mut selected: Vec<String> = Vec::new();
        for n in 0..MAX_APPLICATION_PREFERENCE_ITEMS {
            assert_eq!(
                games_slot(true, SlotPhase::Ready, false, games_at_cap(&selected)).text,
                ""
            );
            toggle_game(&mut selected, &format!("game-{n}"));
        }
        assert_eq!(
            games_slot(true, SlotPhase::Ready, false, games_at_cap(&selected)),
            MineSlot {
                text: "Limit of 10 games reached",
                retry: None
            }
        );

        // Clear: a deselect frees a place and empties the line.
        toggle_game(&mut selected, "game-3");
        assert_eq!(selected.len(), MAX_APPLICATION_PREFERENCE_ITEMS - 1);
        assert_eq!(
            games_slot(true, SlotPhase::Ready, false, games_at_cap(&selected)),
            MineSlot {
                text: "",
                retry: None
            }
        );
        assert_eq!(
            games_slot(true, SlotPhase::Ready, false, games_at_cap(&[])),
            MineSlot {
                text: "",
                retry: None
            }
        );

        // Loading wins over the cap (for example a refetch), retry unchanged.
        for retrying in [false, true] {
            assert_eq!(
                games_slot(true, SlotPhase::Loading, retrying, true),
                games_slot(true, SlotPhase::Loading, retrying, false)
            );
        }
        assert_eq!(
            games_slot(true, SlotPhase::Loading, false, true),
            MineSlot {
                text: "Loading games",
                retry: None
            }
        );
        assert_eq!(
            games_slot(true, SlotPhase::Loading, true, true),
            MineSlot {
                text: "Loading games",
                retry: Some(true)
            }
        );

        // The failure wins over the cap, and Retry still shows.
        assert_eq!(
            games_slot(true, SlotPhase::Failed, false, true),
            MineSlot {
                text: "Couldn't load games.",
                retry: Some(false)
            }
        );

        // Before the form shows the line stays empty, even at the cap.
        for phase in [SlotPhase::Loading, SlotPhase::Failed, SlotPhase::Ready] {
            for retrying in [false, true] {
                assert_eq!(
                    games_slot(false, phase, retrying, true),
                    MineSlot {
                        text: "",
                        retry: None
                    }
                );
            }
        }
    }

    #[test]
    fn retry_accessible_name_contains_the_visible_text() {
        for subject in ["games", "your application"] {
            for retrying in [false, true] {
                let label = retry_aria_label(subject, retrying);
                assert!(label.starts_with(retry_label(retrying)), "{label}");
                assert!(label.ends_with(subject), "{label}");
            }
        }
        assert_eq!(retry_aria_label("games", false), "Retry loading games");
        assert_eq!(retry_aria_label("games", true), "Retrying... loading games");
    }

    /// The Retry button's `aria-label` contains its visible text.
    fn assert_retry_label_in_name(html: &str, visible: &str, label: &str) {
        let end = html
            .find(&format!(">{visible}</button>"))
            .unwrap_or_else(|| panic!("{visible} button in {html}"));
        let start = html[..end].rfind("<button").unwrap();
        let tag = &html[start..end];
        assert!(tag.contains(&format!(r#"aria-label="{label}""#)), "{html}");
        assert!(label.contains(visible), "{label}");
    }

    fn html_of(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    /// Exactly one status region holding `text`, with any Retry after it.
    fn assert_one_status(html: &str, text: &str) {
        assert_eq!(html.matches(r#"role="status""#).count(), 1, "{html}");
        let text = text.replace('\'', "&#39;");
        let region = format!(r#"role="status">{text}</p>"#);
        let at = html
            .find(&region)
            .unwrap_or_else(|| panic!("{region} in {html}"));
        if let Some(button) = html.find("<button") {
            assert!(button >= at + region.len(), "Retry inside status: {html}");
        }
    }

    #[test]
    fn mine_status_is_one_text_region_with_retry_outside() {
        fn loading() -> Element {
            let slot = mine_slot(ApplyScreen::Loading, None, false);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }
        fn settings_failed() -> Element {
            let slot = mine_slot(ApplyScreen::Error, None, false);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }
        fn pending() -> Element {
            let slot = mine_slot(ApplyScreen::Ready, Some(ApplyBodyView::MinePending), false);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }
        fn failed() -> Element {
            let slot = mine_slot(ApplyScreen::Ready, Some(ApplyBodyView::MineFailed), false);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }
        fn retrying() -> Element {
            let slot = mine_slot(ApplyScreen::Ready, Some(ApplyBodyView::MinePending), true);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }
        fn ready() -> Element {
            let slot = mine_slot(ApplyScreen::Ready, Some(ApplyBodyView::Form), false);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }

        let idle = html_of(ready);
        assert_one_status(&idle, "");
        assert!(!idle.contains("<button"), "{idle}");

        // The page-level wait and the settings failure use this same line.
        let html = html_of(loading);
        assert_one_status(&html, "Loading the page");
        assert!(!html.contains("<button"), "{html}");
        let html = html_of(settings_failed);
        assert_one_status(&html, "Couldn't load site settings.");
        assert!(!html.contains("<button"), "{html}");

        let html = html_of(pending);
        assert_one_status(&html, "Loading your application");
        assert!(!html.contains("<button"), "{html}");

        let html = html_of(failed);
        assert_one_status(&html, "Couldn't load your application.");
        assert!(html.contains(">Retry</button>"), "{html}");
        assert_retry_label_in_name(&html, "Retry", "Retry loading your application");
        assert!(!html.contains("disabled"), "{html}");

        let html = html_of(retrying);
        assert_one_status(&html, "Loading your application");
        assert!(html.contains(">Retrying...</button>"), "{html}");
        assert_retry_label_in_name(&html, "Retrying...", "Retrying... loading your application");
        assert!(!html.contains("Retry loading"), "{html}");
        assert_keeps_focus_while_busy(&html);
        assert!(html.contains(r#"aria-busy="true""#), "{html}");
    }

    /// Busy Retry is `aria-disabled`, never native `disabled`: browsers blur a
    /// natively disabled button, which drops focus to the page body.
    fn assert_keeps_focus_while_busy(html: &str) {
        assert!(html.contains(r#"aria-disabled="true""#), "{html}");
        assert!(!html.contains(" disabled"), "{html}");
    }

    fn games() -> Vec<(String, String)> {
        vec![
            ("game-ow".to_string(), "Overwatch".to_string()),
            ("game-val".to_string(), "Valorant".to_string()),
        ]
    }

    #[test]
    fn games_status_is_one_text_region_with_retry_outside() {
        fn before_form() -> Element {
            let slot = games_slot(false, SlotPhase::Failed, false, false);
            rsx! { GamesStatus { slot, on_retry: |_| {} } }
        }
        fn loading() -> Element {
            let slot = games_slot(true, SlotPhase::Loading, false, false);
            rsx! { GamesStatus { slot, on_retry: |_| {} } }
        }
        fn failed() -> Element {
            let slot = games_slot(true, SlotPhase::Failed, false, false);
            rsx! { GamesStatus { slot, on_retry: |_| {} } }
        }
        fn retrying() -> Element {
            let slot = games_slot(true, SlotPhase::Loading, true, false);
            rsx! { GamesStatus { slot, on_retry: |_| {} } }
        }
        fn ready() -> Element {
            let slot = games_slot(true, SlotPhase::Ready, false, false);
            rsx! { GamesStatus { slot, on_retry: |_| {} } }
        }

        // Mounted (empty) from first paint, even if games already failed.
        for idle in [html_of(before_form), html_of(ready)] {
            assert_one_status(&idle, "");
            assert!(idle.contains(r#"id="apply-games-status""#), "{idle}");
            assert!(!idle.contains("<button"), "{idle}");
        }

        let html = html_of(loading);
        assert_one_status(&html, "Loading games");
        assert!(!html.contains("<button"), "{html}");

        let html = html_of(failed);
        assert_one_status(&html, "Couldn't load games.");
        assert!(html.contains(">Retry</button>"), "{html}");
        assert_retry_label_in_name(&html, "Retry", "Retry loading games");
        assert!(!html.contains("disabled"), "{html}");

        let html = html_of(retrying);
        assert_one_status(&html, "Loading games");
        assert!(html.contains(">Retrying...</button>"), "{html}");
        assert_retry_label_in_name(&html, "Retrying...", "Retrying... loading games");
        assert!(!html.contains("Retry loading"), "{html}");
        assert_keeps_focus_while_busy(&html);
    }

    #[test]
    fn games_status_says_the_limit_is_reached_at_the_cap() {
        fn at_cap() -> Element {
            let slot = games_slot(true, SlotPhase::Ready, false, true);
            rsx! { GamesStatus { slot, on_retry: |_| {} } }
        }
        let html = html_of(at_cap);
        assert_one_status(&html, "Limit of 10 games reached");
        // The cap text sits in the element that carries the games status id.
        let at = html
            .find(r#"id="apply-games-status""#)
            .unwrap_or_else(|| panic!("games status id in {html}"));
        let end = at + html[at..].find("</p>").unwrap();
        assert!(
            html[at..end].ends_with(">Limit of 10 games reached"),
            "{html}"
        );
        assert!(!html.contains("<button"), "{html}");
    }

    #[test]
    fn games_field_is_a_named_group_described_by_the_status() {
        fn loading() -> Element {
            rsx! {
                GamesField {
                    phase: SlotPhase::Loading,
                    games: vec![],
                    selected: vec![],
                    on_toggle: |_| {},
                }
            }
        }
        fn ready() -> Element {
            rsx! {
                GamesField {
                    phase: SlotPhase::Ready,
                    games: games(),
                    selected: vec!["game-val".to_string()],
                    on_toggle: |_| {},
                }
            }
        }

        let html = html_of(loading);
        assert!(!html.contains(r#"role="status""#), "{html}");
        assert!(!html.contains("<button"), "{html}");
        assert!(
            html.contains(r#"aria-describedby="apply-games-hint apply-games-status""#),
            "{html}"
        );

        let html = html_of(ready);
        assert!(!html.contains(r#"role="status""#), "{html}");
        assert!(html.contains("<fieldset"), "{html}");
        assert!(
            html.contains(r#"<legend class="field-label">Games<span"#),
            "{html}"
        );
        assert!(
            html.contains(r#"aria-describedby="apply-games-hint apply-games-status""#),
            "{html}"
        );
        assert!(!html.contains("<label"), "{html}");
        assert!(!html.contains(">Retry"), "{html}");
        assert_eq!(html.matches(r#"aria-pressed="false""#).count(), 1, "{html}");
        assert_eq!(html.matches(r#"aria-pressed="true""#).count(), 1, "{html}");
        let button_for = |name: &str| {
            let end = html.find(&format!(">{name}</button>")).unwrap();
            let start = html[..end].rfind("<button").unwrap();
            html[start..end].to_string()
        };
        assert!(
            button_for("Valorant").contains(r#"aria-pressed="true""#),
            "{html}"
        );
        assert!(
            button_for("Overwatch").contains(r#"aria-pressed="false""#),
            "{html}"
        );
        assert!(
            button_for("Overwatch").contains(r#"type="button""#),
            "{html}"
        );
    }

    /// `count` games, `game-0` ("Game 0") upward.
    fn many_games(count: usize) -> Vec<(String, String)> {
        (0..count)
            .map(|n| (format!("game-{n}"), format!("Game {n}")))
            .collect()
    }

    /// The opening tag of the chip for `game-{n}`.
    fn game_chip_tag(html: &str, n: usize) -> String {
        tag_with_id(html, &format!("apply-game-game-{n}")).to_string()
    }

    #[test]
    fn games_hint_and_cap_name_take_the_number_from_the_shared_cap() {
        assert_eq!(MAX_APPLICATION_PREFERENCE_ITEMS, 10);
        assert_eq!(games_hint(), "Choose at least one, at most 10.");
        assert_eq!(
            game_at_cap_name("Overwatch"),
            "Overwatch, limit of 10 games reached"
        );
        let cap = MAX_APPLICATION_PREFERENCE_ITEMS.to_string();
        assert!(games_hint().contains(&cap));
        assert!(game_at_cap_name("Overwatch").contains(&cap));
        // The name starts with the visible label (WCAG 2.5.3).
        assert!(game_at_cap_name("Overwatch").starts_with("Overwatch"));

        fn field() -> Element {
            rsx! {
                GamesField {
                    phase: SlotPhase::Ready,
                    games: games(),
                    selected: vec![],
                    on_toggle: |_| {},
                }
            }
        }
        let html = html_of(field);
        assert!(
            html.contains(
                r#"<p class="field-hint" id="apply-games-hint">Choose at least one, at most 10.</p>"#
            ),
            "{html}"
        );
        // Below the cap no chip is disabled, selected or not.
        assert!(!html.contains("aria-disabled"), "{html}");
        assert!(!html.contains("is-disabled"), "{html}");
        assert!(!html.contains("limit of"), "{html}");
    }

    /// Site blocker on 825: at the cap an unselected chip must not look or
    /// act like a toggle that does nothing. It is `aria-disabled` (never
    /// native `disabled`, which drops focus), looks disabled and is named
    /// with the reason. Selected chips stay toggles.
    #[test]
    fn at_the_cap_unselected_game_chips_are_disabled_and_say_why() {
        fn at_cap() -> Element {
            // 12 games, the first 10 chosen: game-10 and game-11 are left.
            let selected = (0..MAX_APPLICATION_PREFERENCE_ITEMS)
                .map(|n| format!("game-{n}"))
                .collect::<Vec<_>>();
            rsx! {
                GamesField {
                    phase: SlotPhase::Ready,
                    games: many_games(12),
                    selected,
                    on_toggle: |_| {},
                }
            }
        }
        let html = html_of(at_cap);
        assert!(!html.contains(" disabled"), "{html}");
        assert_eq!(html.matches(r#"aria-pressed="true""#).count(), 10, "{html}");
        assert_eq!(html.matches(r#"aria-pressed="false""#).count(), 2, "{html}");
        assert_eq!(html.matches(r#"aria-disabled="true""#).count(), 2, "{html}");

        for n in 10..12 {
            let tag = game_chip_tag(&html, n);
            assert!(tag.starts_with("<button"), "{tag}");
            assert!(tag.contains(r#"type="button""#), "{tag}");
            assert!(tag.contains(r#"aria-disabled="true""#), "{tag}");
            assert!(tag.contains(r#"aria-pressed="false""#), "{tag}");
            assert!(tag.contains(r#"class="chip is-disabled""#), "{tag}");
            assert!(
                tag.contains(&format!(
                    r#"aria-label="Game {n}, limit of 10 games reached""#
                )),
                "{tag}"
            );
            // The visible label is still only the game name.
            assert!(html.contains(&format!(">Game {n}</button>")), "{html}");
        }
        for n in 0..10 {
            let tag = game_chip_tag(&html, n);
            assert!(tag.contains(r#"aria-pressed="true""#), "{tag}");
            assert!(tag.contains(r#"class="chip active""#), "{tag}");
            assert!(!tag.contains("aria-disabled"), "{tag}");
            assert!(!tag.contains("is-disabled"), "{tag}");
            assert!(!tag.contains("aria-label"), "{tag}");
        }
    }

    /// Deselecting one game at the cap frees a slot: the chips that were
    /// disabled are plain toggles again, and so is the one just deselected.
    #[test]
    fn deselecting_at_the_cap_enables_the_other_game_chips_again() {
        fn after_deselect() -> Element {
            let mut selected = (0..MAX_APPLICATION_PREFERENCE_ITEMS)
                .map(|n| format!("game-{n}"))
                .collect::<Vec<_>>();
            assert!(games_at_cap(&selected));
            // A click on a disabled chip never reaches `toggle_game`, and
            // `toggle_game` would refuse the add anyway.
            toggle_game(&mut selected, "game-10");
            assert!(games_at_cap(&selected));
            toggle_game(&mut selected, "game-3");
            assert!(!games_at_cap(&selected));
            rsx! {
                GamesField {
                    phase: SlotPhase::Ready,
                    games: many_games(12),
                    selected,
                    on_toggle: |_| {},
                }
            }
        }
        let html = html_of(after_deselect);
        assert!(!html.contains("aria-disabled"), "{html}");
        assert!(!html.contains("is-disabled"), "{html}");
        assert!(!html.contains("limit of"), "{html}");
        assert!(!html.contains(" disabled"), "{html}");
        assert_eq!(html.matches(r#"aria-pressed="true""#).count(), 9, "{html}");
        assert_eq!(html.matches(r#"aria-pressed="false""#).count(), 3, "{html}");
        for n in [3, 10, 11] {
            let tag = game_chip_tag(&html, n);
            assert!(tag.contains(r#"aria-pressed="false""#), "{tag}");
            assert!(tag.contains(r#"class="chip""#), "{tag}");
            assert!(!tag.contains("aria-label"), "{tag}");
        }
    }

    #[test]
    fn rejected_application_is_not_accepted_and_neutral() {
        let (tone, label) = status_pill("rejected");
        assert_eq!(label, "Not accepted");
        assert!(tone == PillTone::Neutral);
        assert!(status_pill("pending").0 == PillTone::Warn);
        assert_eq!(status_pill("pending").1, "Pending Review");
    }

    #[test]
    fn trial_and_pending_use_the_shared_badge_tones() {
        assert!(status_pill("trial").0 == PillTone::Info);
        assert_eq!(status_pill("trial").1, "Trial Period");
        assert!(status_pill("pending").0 == PillTone::Warn);
        assert!(status_pill("withdrawn").0 == PillTone::Neutral);
    }

    const LONG_MESSAGE: &str = "I play support most evenings and want a steady team.";

    #[test]
    fn validate_lists_invalid_fields_in_form_order() {
        let ow = vec!["game-ow".to_string()];
        assert!(LONG_MESSAGE.chars().count() >= MIN_MESSAGE_CHARS);
        assert_eq!(validate(&ow, LONG_MESSAGE), []);
        assert_eq!(
            validate(&[], "Hi"),
            [ApplyField::Games, ApplyField::Message]
        );
        assert_eq!(validate(&[], LONG_MESSAGE), [ApplyField::Games]);
        // Padding does not reach the minimum, and the count is characters.
        let padded = format!("Hi{}", " ".repeat(60));
        assert_eq!(validate(&ow, &padded), [ApplyField::Message]);
        assert_eq!(MIN_MESSAGE_CHARS, 10);
        assert_eq!(validate(&ow, &"a".repeat(9)), [ApplyField::Message]);
        assert_eq!(validate(&ow, &"a".repeat(10)), []);
        // Two bytes each: 5 of them are 10 bytes but only 5 characters.
        assert_eq!(validate(&ow, &"å".repeat(5)), [ApplyField::Message]);
        assert_eq!(validate(&ow, &"å".repeat(10)), []);
        // A short real answer passes; the old 40 character floor is gone.
        assert_eq!(validate(&ow, "I main Ana."), []);
        assert_eq!(validate(&ow, &"a".repeat(39)), []);
    }

    #[test]
    fn the_message_rule_is_the_servers_trim_then_count_chars() {
        let ow = vec!["game-ow".to_string()];
        // 9 characters once trimmed: refused, however much padding (spaces,
        // tabs, newlines) brings the raw length to 10 or more.
        for nine in [
            "  123456789  ",
            "\n\t123456789\n",
            " Hi I heal ",
            "ålesund99 ",
        ] {
            assert_eq!(application_message_chars(nine), 9, "{nine:?}");
            assert_eq!(validate(&ow, nine), [ApplyField::Message], "{nine:?}");
        }
        // Inner spaces count, like on the server.
        assert_eq!(application_message_chars("Hi, I heal"), 10);
        assert_eq!(validate(&ow, "Hi, I heal"), []);
        // 10 characters with multibyte letters is accepted: 10 chars, more
        // than 10 bytes. A byte count would pass 9 of them too, so check that
        // 9 such characters (still over 10 bytes) is refused.
        for ten in [
            "æøåÆØÅéüñß",
            " Smørbrødet ",
            "ゲームが好きです。!",
            "🎮🎮🎮🎮🎮 heal",
        ] {
            assert_eq!(application_message_chars(ten), 10, "{ten:?}");
            assert!(ten.trim().len() > 10, "{ten:?}");
            assert_eq!(validate(&ow, ten), [], "{ten:?}");
        }
        let nine_multibyte = "æøåÆØÅéüñ";
        assert!(nine_multibyte.len() > 10);
        assert_eq!(validate(&ow, nine_multibyte), [ApplyField::Message]);
    }

    #[test]
    fn the_main_role_is_optional() {
        let ow = vec!["game-ow".to_string()];
        // No role, or a value that is not a listed role, is not an error:
        // validation does not look at the role at all.
        assert_eq!(validate(&ow, LONG_MESSAGE), []);
        // The error summary can only name games and the message.
        let fields = [ApplyField::Games, ApplyField::Message];
        assert!(fields.iter().all(|f| f.id() != ROLE_ID));
        assert!(
            !FORM_LEAD.contains("Every field is required"),
            "{FORM_LEAD}"
        );
        assert!(FORM_LEAD.contains("main role is optional"), "{FORM_LEAD}");
    }

    #[test]
    fn field_ids_are_distinct_and_the_message_names_the_minimum() {
        let fields = [ApplyField::Games, ApplyField::Message];
        let mut ids: Vec<&str> = fields
            .iter()
            .flat_map(|f| [f.id(), f.error_id()])
            .chain([
                ROLE_ID,
                SEND_STATUS_ID,
                SUMMARY_ID,
                STATUS_HEADING_ID,
                GAMES_STATUS_ID,
                GAMES_HINT_ID,
                MESSAGE_HINT_ID,
                WITHDRAW_TRIGGER_ID,
                SUMMARY_HEADING_ID,
                SIGN_IN_HEADING_ID,
            ])
            .collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
        assert!(
            ApplyField::Message
                .error()
                .contains(&MIN_MESSAGE_CHARS.to_string())
        );
    }

    #[test]
    fn the_send_body_carries_the_main_role_as_the_one_preferred_role() {
        // `POST /api/applications` takes `preferred_roles: Vec<String>` and
        // stores it as sent. A chosen main role is the one entry; the rest of
        // the body is as before. Built by the submit handler's own builder.
        let body = apply_body(
            vec!["game-ow".to_string()],
            "Support",
            "I main Ana.".to_string(),
        );
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "preferred_games": ["game-ow"],
                "preferred_roles": ["Support"],
                "message": "I main Ana.",
            })
        );
    }

    #[test]
    fn no_main_role_sends_an_empty_preferred_roles_list() {
        // Nothing picked (the "No main role" option has the empty value):
        // the list is empty, not `[""]`, and the key is still sent.
        let body = apply_body(vec!["game-ow".to_string()], "", "Hi, I heal".to_string());
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "preferred_games": ["game-ow"],
                "preferred_roles": [],
                "message": "Hi, I heal",
            })
        );
        // A value that is not a listed role is not sent either.
        assert!(
            apply_body(vec![], "Healer", String::new())
                .preferred_roles
                .is_empty()
        );
        for role in MAIN_ROLES {
            assert_eq!(
                apply_body(vec![], role, String::new()).preferred_roles,
                [role]
            );
        }
    }

    #[test]
    fn a_400_on_send_shows_the_servers_text_in_the_form() {
        let short = ClientError::http(
            400,
            r#"{"error":"Message must be at least 10 characters"}"#,
            None,
        );
        assert_eq!(
            send_failure(&short),
            SendFailure::Inline(
                "Couldn't send the application: Message must be at least 10 characters."
                    .to_string()
            )
        );
        // A 400 with no usable reason still goes inline, without "try again".
        assert_eq!(
            send_failure(&ClientError::http(400, "", None)),
            SendFailure::Inline(
                "Couldn't send the application. The server refused it.".to_string()
            )
        );
        // Every other failure stays a toast, with the same words as before.
        for err in [
            ClientError::http(403, r#"{"error":"recruitment is currently closed"}"#, None),
            ClientError::http(
                409,
                r#"{"error":"You already have an open application (pending)"}"#,
                None,
            ),
            ClientError::http(401, "", None),
            ClientError::http(500, "", None),
            ClientError::Network("failed to fetch".to_string()),
        ] {
            assert_eq!(
                send_failure(&err),
                SendFailure::Toast(action_failure_text("Couldn't send the application", &err)),
                "{err:?}"
            );
        }
    }

    fn form_refused() -> Element {
        rsx! {
            ApplyForm {
                games_phase: SlotPhase::Ready,
                games: games(),
                selected: vec!["game-ow".to_string()],
                role: String::new(),
                message: "Hi, I heal".to_string(),
                errors: vec![],
                submitting: false,
                send_status: "Couldn't send the application: Message must be at least 10 characters.".to_string(),
                on_toggle: |_| {},
                on_role: |_| {},
                on_message: |_| {},
                on_submit: |_| {},
            }
        }
    }

    #[test]
    fn the_send_status_line_is_always_there_and_only_its_text_changes() {
        // Before any send: mounted, empty.
        let idle = html_of(form_empty);
        assert!(
            idle.contains(
                r#"<p class="apply-status apply-send-status" id="apply-send-status" role="status"></p>"#
            ),
            "{idle}"
        );
        // After a 400: the same element, now with the server's text, before
        // the submit button. No alert and no error summary.
        let html = html_of(form_refused);
        let line = r#"<p class="apply-status apply-send-status" id="apply-send-status" role="status">Couldn&#39;t send the application: Message must be at least 10 characters.</p>"#;
        let at = html
            .find(line)
            .unwrap_or_else(|| panic!("{line} in {html}"));
        assert!(
            at < html.find(">Send application</button>").unwrap(),
            "{html}"
        );
        assert_eq!(html.matches(r#"role="status""#).count(), 1, "{html}");
        assert!(!html.contains(r#"role="alert""#), "{html}");
        assert!(!html.contains(SUMMARY_ID), "{html}");
    }

    #[test]
    fn a_refused_send_shows_the_servers_reason_and_no_try_again() {
        let send = "Couldn't send the application";
        // 403: recruitment closed. A retry gets the same answer.
        let closed = ClientError::http(403, r#"{"error":"recruitment is currently closed"}"#, None);
        assert_eq!(
            action_failure_text(send, &closed),
            "Couldn't send the application: recruitment is currently closed."
        );
        // 409: already applied.
        let dup = ClientError::http(
            409,
            r#"{"error":"You already have an open application (pending)"}"#,
            None,
        );
        assert_eq!(
            action_failure_text(send, &dup),
            "Couldn't send the application: You already have an open application (pending)."
        );
        // A reason that already ends a sentence is not given a second stop.
        let stopped = ClientError::http(409, r#"{"error":"Already withdrawn."}"#, None);
        assert_eq!(
            action_failure_text("Couldn't withdraw the application", &stopped),
            "Couldn't withdraw the application: Already withdrawn."
        );
        // No usable reason: still no "try again".
        for body in ["", "<html>", r#"{"error":"  "}"#, r#"{"error":7}"#] {
            let bare = ClientError::http(403, body, None);
            assert_eq!(
                action_failure_text(send, &bare),
                "Couldn't send the application. The server refused it.",
                "{body}"
            );
        }
        for status in [400, 401, 403, 404, 409] {
            let err = ClientError::http(status, r#"{"error":"nope"}"#, None);
            let text = action_failure_text(send, &err).to_lowercase();
            assert!(!text.contains("try again"), "{status}: {text}");
            assert!(!text.contains("connection"), "{status}: {text}");
        }
        assert_eq!(
            action_failure_text(send, &ClientError::http(401, "", None)),
            "Couldn't send the application. Your session has ended. Log in again."
        );
    }

    #[test]
    fn a_rate_limited_send_or_withdraw_says_to_wait() {
        let send = "Couldn't send the application";
        // Per-user cooldown and withdrawn cap: the server's reason is shown.
        let cooldown = ClientError::http(
            429,
            r#"{"error":"You can apply again in 42 minutes"}"#,
            None,
        );
        assert_eq!(
            action_failure_text(send, &cooldown),
            "Couldn't send the application: You can apply again in 42 minutes."
        );
        let capped = ClientError::http(
            429,
            r#"{"error":"You have withdrawn 5 applications in the last 30 days. You can withdraw again in 3 days"}"#,
            None,
        );
        assert_eq!(
            action_failure_text("Couldn't withdraw the application", &capped),
            "Couldn't withdraw the application: You have withdrawn 5 applications in the last 30 days. You can withdraw again in 3 days."
        );
        // Per-IP limit: the body is a code, not a sentence.
        let limited = ClientError::http(
            429,
            r#"{"code":"rate_limited","error":"rate_limited","retry_after":42}"#,
            None,
        );
        let text = action_failure_text(send, &limited);
        assert_eq!(
            text,
            "Couldn't send the application. Too many attempts. Wait a minute, then try again."
        );
        assert!(!text.contains("rate_limited"), "{text}");
    }

    /// The textarea stops at the cap the server enforces.
    #[test]
    fn the_message_cap_is_the_shared_four_thousand() {
        assert_eq!(MAX_APPLICATION_MESSAGE_CHARS, 4000);
        // `maxlength` counts UTF-16 units, never fewer than chars, so text
        // the textarea lets through is within the server's char count.
        let astral = "🎮".repeat(2000);
        assert_eq!(astral.encode_utf16().count(), 4000);
        assert!(scuffed_types::application_message_within_max(&astral));
    }

    #[test]
    fn only_failures_a_retry_can_fix_say_try_again() {
        let send = "Couldn't send the application";
        // The server's text for a 500 is not shown; a retry may work.
        let down = ClientError::http(500, r#"{"error":"Internal server error"}"#, None);
        assert_eq!(
            action_failure_text(send, &down),
            "Couldn't send the application. Try again."
        );
        // No answer from the server is the only case that names the connection.
        let offline = ClientError::Network("failed to fetch".to_string());
        assert_eq!(
            action_failure_text(send, &offline),
            "Couldn't send the application. Could not reach the server. Check your connection and try again."
        );
        let unreadable = ClientError::Deserialize("expected value".to_string());
        assert_eq!(
            action_failure_text(send, &unreadable),
            format!(
                "Couldn't send the application. {}",
                crate::client_error::UNREADABLE_ANSWER_MESSAGE
            )
        );
        assert!(!action_failure_text(send, &unreadable).contains("connection"));
    }

    #[test]
    fn a_401_on_the_application_fetch_ends_the_session() {
        // Only for a viewer the page still takes as logged in, so the effect
        // that clears the login runs once.
        assert!(session_ended(true, Some("HTTP error 401: Unauthorized")));
        assert!(session_ended(true, Some("HTTP error: 401")));
        assert!(!session_ended(false, Some("HTTP error 401: Unauthorized")));
        assert!(!session_ended(
            true,
            Some("HTTP error 500: Internal server error")
        ));
        assert!(!session_ended(true, Some("Network error: failed to fetch")));
        assert!(!session_ended(true, None));
    }

    #[test]
    fn the_trial_withdraw_confirm_says_it_ends_the_trial() {
        let trial = withdraw_confirm_body("trial");
        assert!(trial.contains("ends your trial"), "{trial}");
        assert!(trial.contains("logs you out"), "{trial}");
        let pending = withdraw_confirm_body("pending");
        assert!(!pending.contains("trial"), "{pending}");
        assert_eq!(
            pending,
            "You have to wait an hour before you can apply again, and only while recruitment is open. After 5 withdrawals in 30 days you have to wait longer."
        );
        // Both variants name the cooldown, and neither promises a later
        // application without it.
        assert_eq!(
            trial,
            format!("This ends your trial and logs you out. {pending}")
        );
        for body in [&trial, &pending] {
            assert!(!body.contains("apply again later"), "{body}");
        }
    }

    #[test]
    fn summary_count_is_singular_for_one_item() {
        assert_eq!(
            summary_count_text(1),
            "Fix the 1 item below, then send it again."
        );
        assert_eq!(
            summary_count_text(2),
            "Fix the 2 items below, then send it again."
        );
    }

    #[test]
    fn title_uses_the_org_name_once_it_loads() {
        assert_eq!(page_title(None), "Apply");
        assert_eq!(page_title(Some("  ")), "Apply");
        assert_eq!(page_title(Some("Test Org")), "Apply to Test Org");
    }

    #[test]
    fn a_fresh_send_is_confirmed_as_received() {
        assert_eq!(status_heading(true, "pending"), "Application received");
        assert_eq!(status_heading(false, "pending"), "Application status");
        // Only the pending application that was just sent is a confirmation.
        assert_eq!(status_heading(true, "withdrawn"), "Application status");
        assert_eq!(status_heading(true, "trial"), "Application status");
    }

    /// Opening tag of the element with this id.
    fn tag_with_id<'a>(html: &'a str, id: &str) -> &'a str {
        let at = html
            .find(&format!(r#"id="{id}""#))
            .unwrap_or_else(|| panic!("#{id} in {html}"));
        let start = html[..at].rfind('<').unwrap();
        let end = at + html[at..].find('>').unwrap();
        &html[start..=end]
    }

    fn form_empty() -> Element {
        rsx! {
            ApplyForm {
                games_phase: SlotPhase::Ready,
                games: games(),
                selected: vec![],
                role: String::new(),
                message: String::new(),
                errors: vec![],
                submitting: false,
                on_toggle: |_| {},
                on_role: |_| {},
                on_message: |_| {},
                on_submit: |_| {},
                a { href: "/", "Cancel" }
            }
        }
    }

    fn form_failed() -> Element {
        rsx! {
            ApplyForm {
                games_phase: SlotPhase::Ready,
                games: games(),
                selected: vec![],
                role: String::new(),
                message: "Hi".to_string(),
                errors: vec![ApplyField::Games, ApplyField::Message],
                submitting: false,
                on_toggle: |_| {},
                on_role: |_| {},
                on_message: |_| {},
                on_submit: |_| {},
                a { href: "/", "Cancel" }
            }
        }
    }

    fn form_message_failed() -> Element {
        rsx! {
            ApplyForm {
                games_phase: SlotPhase::Ready,
                games: games(),
                selected: vec!["game-ow".to_string()],
                role: String::new(),
                message: "123456789".to_string(),
                errors: vec![ApplyField::Message],
                submitting: false,
                on_toggle: |_| {},
                on_role: |_| {},
                on_message: |_| {},
                on_submit: |_| {},
            }
        }
    }

    fn form_all_failed() -> Element {
        rsx! {
            ApplyForm {
                games_phase: SlotPhase::Ready,
                games: games(),
                selected: vec![],
                role: "Tank".to_string(),
                message: LONG_MESSAGE.to_string(),
                errors: vec![ApplyField::Games],
                submitting: false,
                on_toggle: |_| {},
                on_role: |_| {},
                on_message: |_| {},
                on_submit: |_| {},
            }
        }
    }

    fn form_sending() -> Element {
        rsx! {
            ApplyForm {
                games_phase: SlotPhase::Ready,
                games: games(),
                selected: vec!["game-ow".to_string()],
                role: "Support".to_string(),
                message: LONG_MESSAGE.to_string(),
                errors: vec![],
                submitting: true,
                on_toggle: |_| {},
                on_role: |_| {},
                on_message: |_| {},
                on_submit: |_| {},
            }
        }
    }

    #[test]
    fn empty_form_shows_required_fields_and_no_errors() {
        let html = html_of(form_empty);
        // No errors and no summary before the first submit.
        assert!(!html.contains(r#"role="alert""#), "{html}");
        assert!(!html.contains("aria-invalid"), "{html}");
        assert!(!html.contains("field-error"), "{html}");
        assert!(!html.contains("is-error"), "{html}");
        // The form does its own validation and never nests a heading level 1.
        assert!(
            tag_with_id(&html, "apply-role").starts_with("<select"),
            "{html}"
        );
        assert!(html.contains("<form"), "{html}");
        assert!(html.contains("novalidate"), "{html}");
        assert!(!html.contains("<h1"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        // "* Required" is a visual key for the asterisks. The whole line is
        // hidden from assistive tech, which gets `required` from each control.
        assert!(
            html.contains(
                r#"<p class="apply-legend" aria-hidden="true"><span class="apply-req">*</span> Required</p>"#
            ),
            "{html}"
        );
        // Two required fields (games, message), each with a visible
        // asterisk, plus the key's own.
        assert_eq!(html.matches(r#"class="apply-req""#).count(), 3, "{html}");
        let message = tag_with_id(&html, "apply-message");
        assert!(message.contains("required"), "{message}");
        assert!(message.contains(r#"aria-required="true""#), "{message}");
        assert!(message.contains(r#"class="field-input""#), "{message}");
        assert!(html.contains(r#"for="apply-message""#), "{html}");
        // The main role is optional: labelled so in words, no asterisk, not
        // required, nothing describing an error.
        let role = tag_with_id(&html, "apply-role");
        assert!(role.contains(r#"class="field-input""#), "{role}");
        assert!(!role.contains("required"), "{role}");
        assert!(!role.contains("aria-describedby"), "{role}");
        assert!(
            html.contains(
                r#"<label class="field-label" for="apply-role">Main role (optional)</label>"#
            ),
            "{html}"
        );
        assert!(!html.contains("apply-role-err"), "{html}");
        // The 10 character minimum and the 4000 maximum are said before the
        // first submit: a hint that is always there and always describes the
        // field.
        assert!(
            html.contains(
                r#"<p class="field-hint" id="apply-message-hint">At least 10 characters, at most 4000.</p>"#
            ),
            "{html}"
        );
        assert!(
            tag_with_id(&html, "apply-message")
                .contains(r#"aria-describedby="apply-message-hint""#),
            "{html}"
        );
        assert!(
            html.contains(
                r#"<p class="field-hint" id="apply-games-hint">Choose at least one, at most 10.</p>"#
            ),
            "{html}"
        );
        // Role choices, with the empty "No main role" choice selected.
        for role in MAIN_ROLES {
            assert!(html.contains(&format!(">{role}</option>")), "{html}");
        }
        let none = html.find(">No main role</option>").expect("empty choice");
        let option = &html[html[..none].rfind("<option").unwrap()..none];
        assert!(option.contains(r#"value="""#), "{option}");
        assert!(option.contains("selected"), "{option}");
        // Game buttons take ids from the game id, not a counter.
        assert!(html.contains(r#"id="apply-game-game-ow""#), "{html}");
        assert!(html.contains(r#"id="apply-game-game-val""#), "{html}");
        // A submit button that is not disabled, then the Cancel link.
        let submit = html.find(">Send application</button>").expect("submit");
        let tag = &html[html[..submit].rfind("<button").unwrap()..submit];
        assert!(tag.contains(r#"type="submit""#), "{tag}");
        assert!(!tag.contains("disabled"), "{tag}");
        assert!(html[submit..].contains(">Cancel</a>"), "{html}");
        assert!(!html.contains("Europe/Oslo"), "{html}");
    }

    #[test]
    fn failed_submit_marks_each_field_and_links_the_summary() {
        let html = html_of(form_failed);
        // One summary, at the top of the form, with a heading and a count.
        // It takes focus on a failed submit, so it is not a live region: as
        // an alert it would be read twice, and again as the list shrinks.
        assert!(!html.contains(r#"role="alert""#), "{html}");
        assert!(!html.contains("aria-live"), "{html}");
        assert_eq!(
            html.matches(r#"class="apply-summary""#).count(),
            1,
            "{html}"
        );
        let summary = tag_with_id(&html, SUMMARY_ID);
        assert!(summary.starts_with("<div"), "{summary}");
        assert!(summary.contains(r#"role="group""#), "{summary}");
        assert!(summary.contains(r#"tabindex="-1""#), "{summary}");
        assert!(
            summary.contains(r#"aria-labelledby="apply-error-summary-title""#),
            "{summary}"
        );
        assert!(
            html.contains(
                r#"<h2 id="apply-error-summary-title">The application could not be sent</h2>"#
            ),
            "{html}"
        );
        assert!(
            html.contains("<p>Fix the 2 items below, then send it again.</p>"),
            "{html}"
        );
        assert!(html.find(SUMMARY_ID).unwrap() < html.find("apply-legend").unwrap());
        // Summary links point at the invalid controls, in form order. The
        // optional role never has one.
        let games_link = html
            .find(r##"<a href="#apply-games">Games: Choose at least one game.</a>"##)
            .unwrap_or_else(|| panic!("games link in {html}"));
        let message_link = html
            .find(
                r##"<a href="#apply-message">Why you want to join: Write at least 10 characters so we know why you want to join.</a>"##,
            )
            .unwrap_or_else(|| panic!("message link in {html}"));
        assert!(games_link < message_link);
        assert_eq!(html.matches("<li>").count(), 2, "{html}");
        assert!(!html.contains(r##"href="#apply-role""##), "{html}");
        // The invalid message control is marked and described by its hint
        // (kept next to the error) and its message.
        let field = ApplyField::Message;
        let tag = tag_with_id(&html, field.id());
        assert!(tag.contains(r#"aria-invalid="true""#), "{tag}");
        assert!(
            tag.contains(r#"aria-describedby="apply-message-hint apply-message-err""#),
            "{tag}"
        );
        // The message is an icon, then text.
        let message = format!(
            r#"<p class="field-error" id="{}"><span class="field-error-icon" aria-hidden="true"></span>{}</p>"#,
            field.error_id(),
            field.error()
        );
        assert!(html.contains(&message), "{message} in {html}");
        // The invalid Games group carries its message too.
        assert!(
            html.contains(&format!(
                r#"<p class="field-error" id="apply-games-err"><span class="field-error-icon" aria-hidden="true"></span>{}</p>"#,
                ApplyField::Games.error()
            )),
            "{html}"
        );
        // The empty, optional role is left alone.
        let role = tag_with_id(&html, "apply-role");
        assert!(!role.contains("aria-invalid"), "{role}");
        assert!(!html.contains("apply-role-err"), "{html}");
        // With a game picked, only the message fails: the valid Games group
        // and the empty role are left alone.
        let one = html_of(form_message_failed);
        assert!(!one.contains("apply-games-err"), "{one}");
        assert!(!one.contains("is-error"), "{one}");
        assert!(!tag_with_id(&one, "apply-role").contains("aria-invalid"));
        assert!(
            one.contains("<p>Fix the 1 item below, then send it again.</p>"),
            "{one}"
        );
        assert_eq!(one.matches("<li>").count(), 1, "{one}");
        // Typed text is kept, and the button stays active for another try.
        assert!(
            html.contains(">Hi</textarea>") || html.contains(r#"value="Hi""#),
            "{html}"
        );
        let submit = html.find(">Send application</button>").expect("submit");
        let tag = &html[html[..submit].rfind("<button").unwrap()..submit];
        assert!(!tag.contains("disabled"), "{tag}");
    }

    #[test]
    fn games_error_is_described_and_the_group_takes_focus() {
        let html = html_of(form_all_failed);
        let group = tag_with_id(&html, "apply-games");
        assert!(group.starts_with("<fieldset"), "{group}");
        assert!(group.contains("is-error"), "{group}");
        assert!(group.contains(r#"tabindex="-1""#), "{group}");
        // A group has no aria-invalid; the error is a described-by message.
        assert!(!group.contains("aria-invalid"), "{group}");
        assert!(
            group.contains(
                r#"aria-describedby="apply-games-hint apply-games-status apply-games-err""#
            ),
            "{group}"
        );
        assert!(
            html.contains(r##"<a href="#apply-games">Games: Choose at least one game.</a>"##),
            "{html}"
        );
        assert!(
            html.contains("<p>Fix the 1 item below, then send it again.</p>"),
            "{html}"
        );
        assert!(html.contains(r#"id="apply-games-err""#), "{html}");
        // The chosen role is the selected option.
        let tank = html.find(">Tank</option>").unwrap();
        let option = &html[html[..tank].rfind("<option").unwrap()..tank];
        assert!(option.contains("selected"), "{option}");
        assert!(!tag_with_id(&html, "apply-role").contains("aria-invalid"));
    }

    #[test]
    fn sending_keeps_the_submit_button_focusable() {
        let html = html_of(form_sending);
        let submit = html.find(">Sending...</button>").expect("submit");
        let tag = &html[html[..submit].rfind("<button").unwrap()..submit];
        assert!(tag.contains(r#"aria-busy="true""#), "{tag}");
        assert_keeps_focus_while_busy(tag);
        assert!(!html.contains("Send application"), "{html}");
        assert!(!html.contains(r#"role="alert""#), "{html}");
    }

    #[test]
    fn a_corrected_field_loses_its_error_before_the_next_submit() {
        let all = [ApplyField::Games, ApplyField::Message];
        let ow = vec!["game-ow".to_string()];
        // Nothing fixed yet: every error stands.
        assert_eq!(remaining_errors(&all, &[], "Hi"), all);
        // Picking a game clears the group's error.
        assert_eq!(remaining_errors(&all, &ow, "Hi"), [ApplyField::Message]);
        // A long enough message clears only the message error.
        assert_eq!(
            remaining_errors(&all, &[], LONG_MESSAGE),
            [ApplyField::Games]
        );
        assert_eq!(remaining_errors(&all, &ow, LONG_MESSAGE), []);
        // A change never adds an error the last submit did not report.
        assert_eq!(remaining_errors(&[], &[], ""), []);
        assert_eq!(
            remaining_errors(&[ApplyField::Message], &[], LONG_MESSAGE),
            []
        );
    }

    /// After a send or a withdraw the heading takes focus only once the
    /// refetched application is on screen, never on the value from before.
    #[test]
    fn status_heading_takes_focus_only_after_the_refetch_lands() {
        use StatusFocus::{AfterRefetch, Idle, WhenLoaded};
        assert_eq!(status_focus_step(Idle, false, true), (Idle, false));
        assert_eq!(status_focus_step(Idle, true, true), (Idle, false));
        // Withdraw: the old application is still in the slot. Not yet.
        assert_eq!(
            status_focus_step(AfterRefetch, false, true),
            (AfterRefetch, false)
        );
        assert_eq!(
            status_focus_step(AfterRefetch, true, true),
            (WhenLoaded, false)
        );
        assert_eq!(
            status_focus_step(WhenLoaded, true, true),
            (WhenLoaded, false)
        );
        assert_eq!(status_focus_step(WhenLoaded, false, true), (Idle, true));
        // The refetch failed or found nothing: the wait ends without a jump.
        assert_eq!(status_focus_step(WhenLoaded, false, false), (Idle, false));
    }

    #[test]
    fn message_is_described_by_its_hint_and_then_its_error() {
        assert_eq!(message_describedby(false), "apply-message-hint");
        assert_eq!(
            message_describedby(true),
            "apply-message-hint apply-message-err"
        );
        assert!(FORM_LEAD.contains("Games and the message are required"));
        assert!(!FORM_LEAD.contains("asterisk"));
    }

    #[test]
    fn page_css_uses_tokens_and_keeps_targets_at_44() {
        use crate::components::ui::test_support::{
            assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl,
        };
        assert_sizes_from_tokens(APPLY_CSS);
        // Fields, buttons, Retry and Load more are the shared 44px ones, so
        // the page carries no copy of them.
        for gone in [
            ".ui-btn",
            ".ui-field",
            ".apply-label",
            ".apply-error",
            "fetch-error__retry",
            "list-cap-notice__more",
        ] {
            assert!(!APPLY_CSS.contains(gone), "{gone} is shared now");
        }
        // Groups sit one small step under their label, a larger step apart.
        let field = css_rule(APPLY_CSS, ".apply-field {");
        assert!(has_decl(&field, "gap: var(--space-2)"), "{field}");
        assert!(
            APPLY_CSS.contains(".apply-field + .apply-actions { margin-top: var(--space-5); }")
        );
        // Phone: the primary button fills the row.
        let phone = APPLY_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block")
            .1;
        assert!(
            has_decl(
                &css_rule(phone, ".apply-form .apply-actions .btn {"),
                "width: 100%"
            ),
            "{phone}"
        );
        // An invalid games group marks its unpressed chips.
        assert!(APPLY_CSS.contains(".apply-fieldset.is-error .chip:not([aria-pressed=\"true\"])"));
        // Text contrast in dark and light.
        assert_ink_clears_aa("--loss", "--surface", 4.5);
        assert_ink_clears_aa("--loss", "--loss-soft", 4.5);
        assert_ink_clears_aa("--accent-text", "--loss-soft", 4.5);
        assert_ink_clears_aa("--text", "--loss-soft", 4.5);
        assert_ink_clears_aa("--on-loss", "--loss", 4.5);
        assert_ink_clears_aa("--text-3", "--surface", 4.5);
    }

    /// Vision nit 5 on 696: the send line only holds a refusal, so it uses the
    /// field-error ink (`--loss`), not the muted `.apply-status` grey. The
    /// other status lines stay muted.
    #[test]
    fn the_send_refusal_line_uses_the_error_ink() {
        use crate::components::ui::test_support::{assert_ink_clears_aa, css_rule, has_decl};
        let send = css_rule(APPLY_CSS, ".apply-send-status {");
        assert!(has_decl(&send, "color: var(--loss)"), "{send}");
        let status = css_rule(APPLY_CSS, ".apply-status {");
        assert!(has_decl(&status, "color: var(--text-3)"), "{status}");
        // Later in the sheet than the shared rule, same specificity, so it wins.
        assert!(
            APPLY_CSS.find(".apply-send-status {").unwrap()
                > APPLY_CSS.find(".apply-status {").unwrap()
        );
        assert_ink_clears_aa("--loss", "--surface", 4.5);
    }
}
