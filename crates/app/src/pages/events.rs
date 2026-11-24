use std::collections::{HashMap, HashSet};

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ui::inline_confirm::focus_element_by_id;
use crate::components::ui::toggle::toggle_class;
use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pager, Pill, PillTone, SkeletonBars,
    btn_class, page_slice, show_list_top,
};
use crate::components::{Toast, list_cap_notice, use_toast};
use crate::hooks::use_api_list;
use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};
use crate::util::log_client_error;
use scuffed_api_client::ApiClient;

// --- Types ---

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Event {
    id: String,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
    duration_minutes: u32,
    is_recurring: bool,
    #[allow(dead_code)]
    team_id: Option<String>,
    #[allow(dead_code)]
    created_by: String,
    is_active: bool,
    /// False for an event only org members can see.
    is_public: bool,
}

#[derive(Serialize)]
struct RsvpRequest {
    status: &'static str,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct RsvpSummary {
    #[allow(dead_code)]
    event_id: String,
    yes_count: u32,
    maybe_count: u32,
    no_count: u32,
}

/// One answer to an event. The wire value is what the API stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rsvp {
    Yes,
    Maybe,
    No,
}

impl Rsvp {
    const ALL: [Rsvp; 3] = [Rsvp::Yes, Rsvp::Maybe, Rsvp::No];

    const fn wire(self) -> &'static str {
        match self {
            Rsvp::Yes => "yes",
            Rsvp::Maybe => "maybe",
            Rsvp::No => "no",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Rsvp::Yes => "Going",
            Rsvp::Maybe => "Maybe",
            Rsvp::No => "Can't make it",
        }
    }
}

/// Who is looking at the page. It picks the second action of the empty
/// state and whether the RSVP buttons show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Viewer {
    /// The session is still loading: no guess, so nothing flashes.
    Pending,
    /// Logged out, or an account with no member row.
    Guest,
    /// Recruit or member.
    Member,
    /// Officer or admin: they can schedule events.
    Officer,
}

impl Viewer {
    fn of(auth: &AuthState) -> Self {
        if auth.loading {
            Viewer::Pending
        } else if auth.is_officer_or_above() {
            Viewer::Officer
        } else if auth.is_org_member() {
            Viewer::Member
        } else {
            Viewer::Guest
        }
    }

    /// POST /api/events/:id/rsvp takes org members only.
    const fn can_rsvp(self) -> bool {
        matches!(self, Viewer::Member | Viewer::Officer)
    }
}

/// Events per page of the list.
const PAGE_SIZE: usize = 10;

const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// The schedule section. It takes focus after a page change.
const LIST_ID: &str = "events-schedule";

/// The list and its cards. The empty card, buttons, chips, pager and status
/// line come from `components/ui`.
const PAGE_CSS: &str = r#"
    .events-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }
    .event-card {
        background: var(--card-bg);
        color: var(--card-fg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
        padding: var(--card-padding);
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        min-width: 0;
    }
    .event-card-head {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-2) var(--space-3);
    }
    .event-card-title {
        margin: 0;
        min-width: 0;
        font-family: var(--font-head);
        font-size: var(--text-lg);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
        color: var(--text);
        overflow-wrap: anywhere;
    }
    .event-card-badges {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
    }
    .event-card-meta {
        margin: 0;
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-1) var(--space-3);
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    .event-card-meta time {
        color: var(--text);
        font-weight: var(--font-label-weight);
        font-variant-numeric: tabular-nums;
    }
    .event-rsvp {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-2) var(--space-4);
        padding-top: var(--space-3);
        border-top: 1px solid var(--border);
    }
    /* The answers are the card's only action. At rest they take the button
       edge and the full ink, so they do not read as the muted tags next to
       the title. The pressed look stays the shared chip's. */
    .event-rsvp .chip:not(.active) {
        border-color: var(--border-strong);
        color: var(--text);
    }
    .event-rsvp .chip:not(.active):hover {
        border-color: var(--text);
    }
    .event-rsvp-counts {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        font-variant-numeric: tabular-nums;
    }
"#;

fn format_duration(minutes: u32) -> String {
    if minutes >= 60 {
        let h = minutes / 60;
        let m = minutes % 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m}m")
        }
    } else {
        format!("{minutes}m")
    }
}

/// "Every Monday, 19:00" for a weekly event, "Monday, 19:00" for a single one.
fn when_text(event: &Event) -> String {
    let day = DAYS.get(event.day_of_week as usize).copied();
    match (day, event.is_recurring) {
        (Some(day), true) => format!("Every {day}, {}", event.time),
        (Some(day), false) => format!("{day}, {}", event.time),
        (None, _) => event.time.clone(),
    }
}

/// "3 going, 1 maybe, 0 can't make it", or a plain line when nobody answered.
fn rsvp_counts_text(summary: &RsvpSummary) -> String {
    if summary.yes_count + summary.maybe_count + summary.no_count == 0 {
        return "No RSVPs yet".to_string();
    }
    format!(
        "{} going, {} maybe, {} can't make it",
        summary.yes_count, summary.maybe_count, summary.no_count
    )
}

/// Active events in week order: Monday first, then by start time.
fn schedule(mut events: Vec<Event>) -> Vec<Event> {
    events.retain(|e| e.is_active);
    events.sort_by(|a, b| (a.day_of_week, &a.time).cmp(&(b.day_of_week, &b.time)));
    events
}

/// State of the list fetch, as the body takes it.
#[derive(Debug, Clone, PartialEq)]
enum EventsLoad {
    Loading,
    /// The mapped reason (`ApiResource::error`, logged by the hook).
    Failed(String),
    Ready(Vec<Event>),
}

impl EventsLoad {
    /// From a `use_api_list` slot: `None` in flight, `Some(None)` failed.
    /// `error` is the hook's mapped error text.
    fn of(slot: Option<&Option<Vec<Event>>>, error: Option<&str>) -> Self {
        match slot {
            None => EventsLoad::Loading,
            Some(None) => EventsLoad::Failed(error.unwrap_or_default().to_string()),
            Some(Some(list)) => EventsLoad::Ready(schedule(list.clone())),
        }
    }
}

/// Text of the page status line: loading words during the first load and
/// during a retry, empty otherwise.
fn events_status(load: &EventsLoad, retrying: bool) -> &'static str {
    if retrying || *load == EventsLoad::Loading {
        "Loading events"
    } else {
        ""
    }
}

#[component]
pub fn Events() -> Element {
    let auth = use_auth();
    let events = use_api_list::<Event>("/api/events");
    let viewer = Viewer::of(&auth());
    let mut refresh = events.refresh;

    // Cloned out so no read guard is held while the children render.
    let load = EventsLoad::of(events.data.read().as_ref(), events.error.read().as_deref());
    // `use_api_list` keeps the failed slot while a retry is in flight.
    let retrying =
        matches!(load, EventsLoad::Failed(_)) && events.data.state()() == UseResourceState::Pending;

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-events-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Default, class: "events-page",
            PageHeader {
                title: "Events",
                lead: "Weekly play nights, meetups and open sessions.",
            }
            EventsBody { load, viewer, retrying, on_retry: move |_| refresh += 1 }
            {list_cap_notice(&events, "events")}
        }
    }
}

/// Everything under the page header, for one fetch state. It is mounted in
/// every state, so the status line and the pager's range line exist before
/// their text changes. It also holds the page number and the answers given in
/// this visit, so a card that leaves the page and comes back keeps its
/// pressed chip.
#[component]
fn EventsBody(
    load: EventsLoad,
    viewer: Viewer,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    #[props(default)] on_retry: Option<EventHandler<()>>,
) -> Element {
    let mut page = use_signal(|| 1usize);
    let mut answers = use_signal(HashMap::<String, Rsvp>::new);
    let mut toast = use_toast();
    // Event ids with a save in flight, and how many saves each event has had
    // (a card refetches its counts when its number goes up).
    let mut saving = use_signal(HashSet::<String>::new);
    let mut saved = use_signal(HashMap::<String, u64>::new);
    // The save runs here, in the list, not in the card: a card unmounts when
    // the pager moves on, and a task tied to it would be dropped mid-save,
    // taking the toast and the revert of a failed answer with it.
    let save_rsvp = use_callback(move |(id, choice): (String, Rsvp)| {
        if saving.peek().contains(&id) {
            return;
        }
        let previous = answers.peek().get(&id).copied();
        answers.write().insert(id.clone(), choice);
        saving.write().insert(id.clone());
        spawn(async move {
            let body = RsvpRequest {
                status: choice.wire(),
            };
            let path = format!("/api/events/{}/rsvp", crate::util::encode_query(&id));
            match ApiClient::web()
                .post_json::<_, serde_json::Value>(&path, &body)
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success(format!("RSVP: {}", choice.label())));
                    *saved.write().entry(id.clone()).or_default() += 1;
                }
                Err(e) => {
                    log_client_error("event rsvp", &e);
                    toast.show(Toast::error(rsvp_failure_text(&e)));
                    match previous {
                        Some(answer) => answers.write().insert(id.clone(), answer),
                        None => answers.write().remove(&id),
                    };
                }
            }
            saving.write().remove(&id);
        });
    });
    let status = events_status(&load, retrying);
    let total = match &load {
        EventsLoad::Ready(events) => events.len(),
        _ => 0,
    };

    rsx! {
        p { class: "visually-hidden", role: "status", "{status}" }
        match &load {
            EventsLoad::Loading => rsx! { SkeletonBars { rows: 3 } },
            EventsLoad::Failed(reason) => rsx! {
                LoadError {
                    message: crate::client_error::join_lead("Couldn't load events.", reason),
                    on_retry,
                    retrying,
                }
            },
            EventsLoad::Ready(events) if events.is_empty() => rsx! {
                EventsEmpty { viewer }
            },
            EventsLoad::Ready(events) => rsx! {
                section {
                    id: LIST_ID,
                    class: "ui-stack pager-target",
                    tabindex: "-1",
                    aria_labelledby: "events-schedule-title",
                    h2 { id: "events-schedule-title", class: "ui-section-title", "Schedule" }
                    // `list-style: none` drops the list role in some browsers.
                    ul { class: "events-list", role: "list",
                        for event in page_slice(events, page(), PAGE_SIZE) {
                            li { key: "{event.id}",
                                EventCard {
                                    event: event.clone(),
                                    can_rsvp: viewer.can_rsvp(),
                                    selected: answers.read().get(&event.id).copied(),
                                    busy: saving.read().contains(&event.id),
                                    saved: saved.read().get(&event.id).copied().unwrap_or(0),
                                    on_choose: {
                                        let id = event.id.clone();
                                        move |choice: Rsvp| save_rsvp.call((id.clone(), choice))
                                    },
                                }
                            }
                        }
                    }
                }
            },
        }
        Pager {
            id: "events-pager",
            page: page(),
            total,
            page_size: PAGE_SIZE,
            noun: "events",
            noun_singular: "event",
            on_change: move |p| {
                page.set(p);
                show_list_top(LIST_ID);
            },
        }
    }
}

/// "No events scheduled" with two next steps. The first is the forum LFG
/// board for everyone. The second depends on the viewer: guests apply,
/// members post a scrim, officers schedule the event themselves.
#[component]
fn EventsEmpty(viewer: Viewer) -> Element {
    let rest = match viewer {
        Viewer::Pending => "In the meantime you can see who is looking for a group.",
        Viewer::Guest => {
            "In the meantime you can see who is looking for a group, or join the crew and propose a night."
        }
        Viewer::Member => {
            "In the meantime you can see who is looking for a group, or post a scrim to propose a night."
        }
        Viewer::Officer => {
            "In the meantime you can see who is looking for a group, or schedule a night yourself."
        }
    };

    rsx! {
        EmptyCard {
            title: "No events scheduled",
            message: "When a play night or meetup is posted, it will show up here. {rest}",
            actions: rsx! {
                Link {
                    to: Route::ForumBoardPage { slug: "lfg".to_string() },
                    class: btn_class(true),
                    "See the forum LFG board"
                }
                match viewer {
                    Viewer::Pending => rsx! {},
                    Viewer::Guest => rsx! {
                        Link { to: Route::Apply {}, class: btn_class(false),
                            "Join the crew to propose a night"
                        }
                    },
                    Viewer::Member => rsx! {
                        Link { to: Route::Scrims {}, class: btn_class(false),
                            "Post a scrim to propose a night"
                        }
                    },
                    Viewer::Officer => rsx! {
                        Link { to: Route::AdminSchedule {}, class: btn_class(false),
                            "Schedule an event"
                        }
                    },
                }
            },
        }
    }
}

/// One event with its live RSVP counts. `saved` counts this event's saved
/// answers; when it goes up only this card refetches its counts, not the
/// whole list. `selected` is the answer given in this visit and `busy` is a
/// save in flight; the list owns both, and runs the save.
#[component]
fn EventCard(
    event: Event,
    can_rsvp: bool,
    selected: Option<Rsvp>,
    busy: bool,
    saved: u64,
    on_choose: EventHandler<Rsvp>,
) -> Element {
    let event_id = event.id.clone();
    let summary = use_resource(use_reactive!(|(event_id, saved)| async move {
        let _ = saved;
        let path = format!(
            "/api/events/{}/rsvp-summary",
            crate::util::encode_query(&event_id)
        );
        ApiClient::web().fetch::<RsvpSummary>(&path).await.ok()
    }));
    let summary: Option<RsvpSummary> = summary.read().clone().flatten();

    rsx! {
        EventCardView {
            event,
            summary,
            can_rsvp,
            selected,
            busy,
            on_choose,
        }
    }
}

#[component]
fn EventCardView(
    event: Event,
    summary: Option<RsvpSummary>,
    can_rsvp: bool,
    selected: Option<Rsvp>,
    #[props(default = false)] busy: bool,
    on_choose: EventHandler<Rsvp>,
) -> Element {
    let when = when_text(&event);
    let duration = format_duration(event.duration_minutes);
    let counts = summary.as_ref().map(rsvp_counts_text);
    let title_id = format!("event-{}-title", event.id);

    rsx! {
        article { id: "event-{event.id}", class: "event-card", aria_labelledby: "{title_id}",
            div { class: "event-card-head",
                h3 { id: "{title_id}", class: "event-card-title", "{event.title}" }
                div { class: "event-card-badges",
                    if event.is_recurring {
                        Pill { tone: PillTone::Info, "Weekly" }
                    } else {
                        Pill { tone: PillTone::Neutral, "One-off" }
                    }
                    if !event.is_public {
                        Pill { tone: PillTone::Warn, "Members only" }
                    }
                }
            }
            p { class: "event-card-meta",
                time { datetime: "{event.time}", "{when}" }
                span { "{event.timezone}" }
                span { "Lasts {duration}" }
            }
            if counts.is_some() || can_rsvp {
                div { class: "event-rsvp",
                    if let Some(counts) = counts {
                        p { class: "event-rsvp-counts", "{counts}" }
                    }
                    if can_rsvp {
                        RsvpButtons {
                            event_id: event.id.clone(),
                            title: event.title.clone(),
                            selected,
                            busy,
                            on_choose,
                        }
                    }
                }
            }
        }
    }
}

/// Toast for a failed RSVP. Only a failure a retry can fix says "try again".
fn rsvp_failure_text(err: &scuffed_api_client::ClientError) -> String {
    let named = match err.http_status() {
        Some(401) => "Couldn't save your RSVP. Your session has ended. Log in again.",
        Some(403) => "Couldn't save your RSVP. Only crew members can RSVP.",
        Some(404) => "Couldn't save your RSVP. This event no longer exists.",
        Some(_) => "Couldn't save your RSVP. Try again in a moment.",
        // No answer or an unreadable answer: the one mapping (QA on 748).
        None => {
            return crate::client_error::join_lead(
                "Couldn't save your RSVP.",
                crate::client_error::message_without_reason(err),
            );
        }
    };
    named.to_string()
}

/// DOM id of one RSVP radio, from the event id and the answer.
fn rsvp_id(event_id: &str, choice: Rsvp) -> String {
    format!("event-{event_id}-rsvp-{}", choice.wire())
}

/// The one radio in the Tab order: the checked answer, or the first radio
/// when no answer is given yet. The others are reached with the arrow keys.
fn rsvp_tab_stop(selected: Option<Rsvp>) -> Rsvp {
    selected.unwrap_or(Rsvp::ALL[0])
}

/// The radio an arrow key, Home or End moves focus to, wrapping at the ends.
fn rsvp_key_target(from: Rsvp, key: &Key) -> Option<Rsvp> {
    let all = Rsvp::ALL;
    let at = all.iter().position(|choice| *choice == from)?;
    match key {
        Key::ArrowRight | Key::ArrowDown => Some(all[(at + 1) % all.len()]),
        Key::ArrowLeft | Key::ArrowUp => Some(all[(at + all.len() - 1) % all.len()]),
        Key::Home => Some(all[0]),
        Key::End => Some(all[all.len() - 1]),
        _ => None,
    }
}

/// The RSVP answers as a radio group in the chip look. An answer can be
/// changed but not taken back (the API has no "no answer"), which is what a
/// radio group says; a toggle button would promise it can be switched off.
///
/// One Tab stop; the arrow keys, Home and End move focus between the radios.
/// Moving focus does not answer: every answer is a save on the server, so it
/// takes Space, Enter or a click. While a save runs the radios stay
/// focusable and ignore clicks; the list puts the previous answer back if
/// the save fails.
#[component]
fn RsvpButtons(
    event_id: String,
    title: String,
    selected: Option<Rsvp>,
    #[props(default = false)] busy: bool,
    on_choose: EventHandler<Rsvp>,
) -> Element {
    let tab_stop = rsvp_tab_stop(selected);

    rsx! {
        div { class: "chip-row", role: "radiogroup", aria_label: "Your RSVP for {title}",
            for choice in Rsvp::ALL {
                button {
                    key: "{choice.wire()}",
                    r#type: "button",
                    id: rsvp_id(&event_id, choice),
                    role: "radio",
                    aria_checked: if selected == Some(choice) { "true" } else { "false" },
                    aria_disabled: if busy { "true" },
                    tabindex: if choice == tab_stop { "0" } else { "-1" },
                    class: toggle_class("chip", selected == Some(choice)),
                    onclick: move |_| {
                        if !busy && selected != Some(choice) {
                            on_choose.call(choice);
                        }
                    },
                    onkeydown: {
                        let eid = event_id.clone();
                        move |e: KeyboardEvent| {
                            if let Some(next) = rsvp_key_target(choice, &e.key()) {
                                e.prevent_default();
                                focus_element_by_id(&rsvp_id(&eid, next));
                            }
                        }
                    },
                    "{choice.label()}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::{Cell, RefCell};

    use crate::components::ToastProvider;
    use crate::components::ui::test_support::assert_sizes_from_tokens;
    use scuffed_types::{OrgRole, UserInfo};

    /// What the one test route renders.
    #[derive(Clone, PartialEq)]
    enum Show {
        Page,
        Empty,
        List(Vec<Event>),
        Failed(bool),
        Card(Event, Option<RsvpSummary>, Option<Rsvp>),
    }

    thread_local! {
        static SHOW: RefCell<Show> = const { RefCell::new(Show::Page) };
        static VIEWER: Cell<Viewer> = const { Cell::new(Viewer::Guest) };
    }

    /// The page links with `Link`, which needs a router above it.
    #[derive(Clone, Debug, PartialEq, Routable)]
    enum TestRoute {
        #[route("/")]
        Scene {},
    }

    #[component]
    fn Scene() -> Element {
        let viewer = VIEWER.with(Cell::get);
        match SHOW.with(|slot| slot.borrow().clone()) {
            Show::Page => rsx! { Events {} },
            Show::Empty => rsx! { EventsBody { load: EventsLoad::Ready(Vec::new()), viewer } },
            Show::List(events) => rsx! { EventsBody { load: EventsLoad::Ready(events), viewer } },
            Show::Failed(retrying) => rsx! {
                EventsBody {
                    load: EventsLoad::Failed(crate::client_error::NETWORK_ERROR_MESSAGE.to_string()),
                    viewer,
                    retrying,
                    on_retry: move |_| {},
                }
            },
            Show::Card(event, summary, selected) => rsx! {
                EventCardView {
                    event,
                    summary,
                    can_rsvp: viewer.can_rsvp(),
                    selected,
                    on_choose: move |_| {},
                }
            },
        }
    }

    fn auth_for(viewer: Viewer) -> AuthState {
        let role = match viewer {
            Viewer::Pending | Viewer::Guest => None,
            Viewer::Member => Some(OrgRole::Member),
            Viewer::Officer => Some(OrgRole::Officer),
        };
        AuthState {
            user: role.map(|role| UserInfo {
                id: "member-1".into(),
                username: "Player01".into(),
                avatar_url: None,
                role: Some(role),
            }),
            loading: viewer == Viewer::Pending,
        }
    }

    fn app() -> Element {
        let history =
            use_hook(|| std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path("/")));
        let auth = use_signal(|| auth_for(VIEWER.with(Cell::get)));
        use_context_provider(|| auth);
        rsx! {
            ToastProvider {
                dioxus::router::components::HistoryProvider {
                    history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                    Router::<TestRoute> {}
                }
            }
        }
    }

    fn render(show: Show, viewer: Viewer) -> String {
        SHOW.with(|slot| *slot.borrow_mut() = show);
        VIEWER.with(|slot| slot.set(viewer));
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn event(id: &str, title: &str, day: u8, time: &str) -> Event {
        Event {
            id: id.to_string(),
            title: title.to_string(),
            day_of_week: day,
            time: time.to_string(),
            timezone: "Europe/Oslo".to_string(),
            duration_minutes: 120,
            is_recurring: true,
            team_id: None,
            created_by: "member-1".to_string(),
            is_active: true,
            is_public: true,
        }
    }

    fn summary(yes: u32, maybe: u32, no: u32) -> RsvpSummary {
        RsvpSummary {
            event_id: "e1".to_string(),
            yes_count: yes,
            maybe_count: maybe,
            no_count: no,
        }
    }

    /// Text of every `<a>` with its href, in document order.
    fn links(html: &str) -> Vec<(String, String)> {
        html.split("<a ")
            .skip(1)
            .map(|chunk| {
                let href = chunk
                    .split("href=\"")
                    .nth(1)
                    .and_then(|rest| rest.split('"').next())
                    .unwrap_or_default();
                let text = chunk
                    .split_once('>')
                    .and_then(|(_, rest)| rest.split("</a>").next())
                    .unwrap_or_default();
                (href.to_string(), text.to_string())
            })
            .collect()
    }

    #[test]
    fn page_css_uses_tokens_only() {
        assert_sizes_from_tokens(PAGE_CSS);
    }

    #[test]
    fn page_has_one_h1_no_main_and_a_loading_status() {
        let html = render(Show::Page, Viewer::Guest);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Events</h1>"), "{html}");
        assert!(!html.contains("<main"), "the shell owns main: {html}");
        assert!(
            html.contains("<div class=\"ui-shell events-page\">"),
            "default content width: {html}"
        );
        assert!(
            html.contains("<p class=\"visually-hidden\" role=\"status\">Loading events</p>"),
            "{html}"
        );
        // The pager's range line exists before there is a list.
        assert!(html.contains("id=\"events-pager-meta\""), "{html}");
        assert!(
            html.contains("<div class=\"ui-skel-list\" aria-hidden=\"true\">"),
            "{html}"
        );
        assert!(
            html.contains("Weekly play nights, meetups and open sessions."),
            "{html}"
        );
    }

    #[test]
    fn empty_state_offers_guests_the_lfg_board_and_apply() {
        let html = render(Show::Empty, Viewer::Guest);
        assert!(
            html.contains("<section class=\"empty-card\"><h2>No events scheduled</h2>"),
            "{html}"
        );
        assert!(
            html.contains("href=\"/forum/b/lfg\" class=\"btn is-primary\""),
            "{html}"
        );
        assert!(
            html.contains("href=\"/apply\" class=\"btn is-ghost\""),
            "{html}"
        );
        assert!(
            html.contains("join the crew and propose a night."),
            "{html}"
        );
        assert_eq!(
            links(&html),
            vec![
                (
                    "/forum/b/lfg".to_string(),
                    "See the forum LFG board".to_string()
                ),
                (
                    "/apply".to_string(),
                    "Join the crew to propose a night".to_string()
                ),
            ],
            "{html}"
        );
        assert!(!html.contains("<button"), "{html}");
        assert!(!html.contains("events-list"), "{html}");
    }

    #[test]
    fn empty_state_second_action_follows_the_viewer() {
        let member = links(&render(Show::Empty, Viewer::Member));
        assert_eq!(
            member[1],
            (
                "/scrims".to_string(),
                "Post a scrim to propose a night".to_string()
            )
        );
        let officer = links(&render(Show::Empty, Viewer::Officer));
        assert_eq!(
            officer[1],
            (
                "/admin/schedule".to_string(),
                "Schedule an event".to_string()
            )
        );
        for links in [&member, &officer] {
            assert_eq!(links.len(), 2);
            assert_eq!(links[0].0, "/forum/b/lfg");
        }
    }

    #[test]
    fn empty_state_does_not_guess_while_the_session_loads() {
        let html = render(Show::Empty, Viewer::Pending);
        let links = links(&html);
        assert_eq!(links.len(), 1, "{html}");
        assert_eq!(links[0].0, "/forum/b/lfg");
        assert!(!html.contains("join the crew"), "{html}");
    }

    #[test]
    fn empty_list_keeps_the_pager_status_region() {
        let html = render(Show::Empty, Viewer::Guest);
        assert!(html.contains("id=\"events-pager-meta\""), "{html}");
        assert!(!html.contains("id=\"events-pager-next\""), "{html}");
    }

    #[test]
    fn failed_load_keeps_the_status_and_pager_lines_and_retry_keeps_focus() {
        let failed = render(Show::Failed(false), Viewer::Guest);
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains(
                "Couldn&#39;t load events. Could not reach the server. Check your connection and try again."
            ),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");
        assert!(
            failed.contains("<p class=\"visually-hidden\" role=\"status\"></p>"),
            "{failed}"
        );

        let retrying = render(Show::Failed(true), Viewer::Guest);
        assert!(
            retrying.contains("<p class=\"visually-hidden\" role=\"status\">Loading events</p>"),
            "{retrying}"
        );
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
        assert!(retrying.contains("aria-disabled=\"true\""), "{retrying}");
        assert!(!retrying.contains(" disabled"), "{retrying}");

        for html in [&failed, &retrying] {
            assert!(html.contains("id=\"events-pager-meta\""), "{html}");
        }
        assert_eq!(events_status(&EventsLoad::Loading, false), "Loading events");
        assert_eq!(events_status(&EventsLoad::Ready(Vec::new()), false), "");
        assert_eq!(EventsLoad::of(None, None), EventsLoad::Loading);
        assert_eq!(
            EventsLoad::of(Some(&None), Some("HTTP error 403: Members only")),
            EventsLoad::Failed("HTTP error 403: Members only".to_string())
        );
    }

    #[test]
    fn viewer_follows_the_session() {
        for viewer in [
            Viewer::Pending,
            Viewer::Guest,
            Viewer::Member,
            Viewer::Officer,
        ] {
            assert_eq!(Viewer::of(&auth_for(viewer)), viewer);
        }
        let no_member_row = AuthState {
            user: Some(UserInfo {
                id: "user-1".into(),
                username: "Player02".into(),
                avatar_url: None,
                role: None,
            }),
            loading: false,
        };
        assert_eq!(Viewer::of(&no_member_row), Viewer::Guest);
        assert!(!Viewer::Guest.can_rsvp());
        assert!(!Viewer::Pending.can_rsvp());
        assert!(Viewer::Member.can_rsvp());
        assert!(Viewer::Officer.can_rsvp());
    }

    #[test]
    fn schedule_is_week_ordered_and_drops_inactive_events() {
        let mut off = event("e4", "Old night", 0, "18:00");
        off.is_active = false;
        let sorted = schedule(vec![
            event("e3", "Friday scrim", 4, "20:00"),
            event("e2", "Late Monday", 0, "21:00"),
            off,
            event("e1", "Early Monday", 0, "19:00"),
        ]);
        let ids: Vec<&str> = sorted.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["e1", "e2", "e3"]);
    }

    #[test]
    fn card_shows_the_time_in_a_time_element_and_status_as_words() {
        let weekly = event("e1", "Play night", 0, "19:00");
        let html = render(Show::Card(weekly, None, None), Viewer::Guest);
        assert!(
            html.contains(
                "<article id=\"event-e1\" class=\"event-card\" aria-labelledby=\"event-e1-title\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("<h3 id=\"event-e1-title\" class=\"event-card-title\">Play night</h3>"),
            "{html}"
        );
        assert!(
            html.contains("<time datetime=\"19:00\">Every Monday, 19:00</time>"),
            "{html}"
        );
        assert!(html.contains("<span>Europe/Oslo</span>"), "{html}");
        assert!(html.contains("<span>Lasts 2h</span>"), "{html}");
        assert!(html.contains(">Weekly</span>"), "{html}");
        assert!(!html.contains("Members only"), "{html}");
        assert!(!html.contains("event-rsvp"), "nothing to show yet: {html}");

        let mut single = event("e2", "Meetup", 5, "14:30");
        single.is_recurring = false;
        single.is_public = false;
        let html = render(Show::Card(single, None, None), Viewer::Member);
        assert!(
            html.contains("<time datetime=\"14:30\">Saturday, 14:30</time>"),
            "{html}"
        );
        assert!(html.contains(">One-off</span>"), "{html}");
        assert!(html.contains(">Members only</span>"), "{html}");
    }

    #[test]
    fn rsvp_counts_are_words_not_dots() {
        let html = render(
            Show::Card(
                event("e1", "Play night", 0, "19:00"),
                Some(summary(3, 1, 0)),
                None,
            ),
            Viewer::Guest,
        );
        assert!(
            html.contains("<p class=\"event-rsvp-counts\">3 going, 1 maybe, 0 can"),
            "{html}"
        );
        assert_eq!(
            rsvp_counts_text(&summary(3, 1, 0)),
            "3 going, 1 maybe, 0 can't make it"
        );
        assert!(!html.contains("<button"), "guests cannot RSVP: {html}");
        assert_eq!(rsvp_counts_text(&summary(0, 0, 0)), "No RSVPs yet");
    }

    #[test]
    fn members_get_a_named_radio_group_with_ids_from_the_event() {
        let html = render(
            Show::Card(event("e7", "Play night", 0, "19:00"), None, None),
            Viewer::Member,
        );
        // An answer can be changed but not removed, so the three chips are
        // radios in a radio group, not toggle buttons.
        assert!(
            html.contains(
                "<div class=\"chip-row\" role=\"radiogroup\" aria-label=\"Your RSVP for Play night\">"
            ),
            "{html}"
        );
        assert!(!html.contains("aria-pressed"), "{html}");
        // No answer yet: none is checked and the first radio is the Tab stop.
        for (wire, label, tabindex) in [("yes", "Going", "0"), ("maybe", "Maybe", "-1")] {
            assert!(
                html.contains(&format!(
                    "<button type=\"button\" id=\"event-e7-rsvp-{wire}\" role=\"radio\" aria-checked=\"false\" tabindex=\"{tabindex}\" class=\"chip\">{label}</button>"
                )),
                "{wire}: {html}"
            );
        }
        assert!(html.contains("id=\"event-e7-rsvp-no\""), "{html}");
        assert_eq!(html.matches("<button").count(), 3, "{html}");
        assert_eq!(html.matches("role=\"radio\"").count(), 3, "{html}");
        assert_eq!(html.matches("aria-checked=\"false\"").count(), 3, "{html}");
        assert_eq!(html.matches("tabindex=\"0\"").count(), 1, "{html}");
        assert!(!html.contains("aria-disabled"), "{html}");
        assert!(links(&html).is_empty(), "no button sits in a link: {html}");
    }

    #[test]
    fn rsvp_radios_have_one_tab_stop_and_arrow_keys_wrap() {
        assert_eq!(rsvp_tab_stop(None), Rsvp::Yes);
        for choice in Rsvp::ALL {
            assert_eq!(rsvp_tab_stop(Some(choice)), choice);
        }
        for key in [Key::ArrowRight, Key::ArrowDown] {
            assert_eq!(rsvp_key_target(Rsvp::Yes, &key), Some(Rsvp::Maybe));
            assert_eq!(rsvp_key_target(Rsvp::Maybe, &key), Some(Rsvp::No));
            assert_eq!(rsvp_key_target(Rsvp::No, &key), Some(Rsvp::Yes));
        }
        for key in [Key::ArrowLeft, Key::ArrowUp] {
            assert_eq!(rsvp_key_target(Rsvp::Yes, &key), Some(Rsvp::No));
            assert_eq!(rsvp_key_target(Rsvp::No, &key), Some(Rsvp::Maybe));
        }
        assert_eq!(rsvp_key_target(Rsvp::No, &Key::Home), Some(Rsvp::Yes));
        assert_eq!(rsvp_key_target(Rsvp::Yes, &Key::End), Some(Rsvp::No));
        // Tab, Enter and Space are left to the browser.
        for key in [Key::Tab, Key::Enter, Key::Character(" ".into())] {
            assert_eq!(rsvp_key_target(Rsvp::Maybe, &key), None);
        }
        assert_eq!(rsvp_id("e7", Rsvp::No), "event-e7-rsvp-no");
    }

    #[test]
    fn a_failed_rsvp_names_its_cause() {
        use scuffed_api_client::ClientError;
        let http = |status| ClientError::http(status, "", None);
        assert_eq!(
            rsvp_failure_text(&http(403)),
            "Couldn't save your RSVP. Only crew members can RSVP."
        );
        for status in [401, 403, 404] {
            let text = rsvp_failure_text(&http(status)).to_lowercase();
            assert!(!text.contains("try again"), "{status}: {text}");
            assert!(!text.contains("connection"), "{status}: {text}");
        }
        assert_eq!(
            rsvp_failure_text(&http(500)),
            "Couldn't save your RSVP. Try again in a moment."
        );
        assert_eq!(
            rsvp_failure_text(&ClientError::Network("offline".into())),
            "Couldn't save your RSVP. Could not reach the server. Check your connection and try again."
        );
        let unreadable = rsvp_failure_text(&ClientError::Deserialize("expected value".into()));
        assert_eq!(
            unreadable,
            format!(
                "Couldn't save your RSVP. {}",
                crate::client_error::UNREADABLE_ANSWER_MESSAGE
            )
        );
        assert!(!unreadable.contains("connection"), "{unreadable}");
    }

    /// The answer lives with the list, not the card, so a card that leaves
    /// the page and comes back still shows it.
    #[test]
    fn the_given_answer_is_pressed_from_the_list_state() {
        let html = render(
            Show::Card(
                event("e7", "Play night", 0, "19:00"),
                None,
                Some(Rsvp::Maybe),
            ),
            Viewer::Member,
        );
        assert!(
            html.contains(
                "<button type=\"button\" id=\"event-e7-rsvp-maybe\" role=\"radio\" aria-checked=\"true\" tabindex=\"0\" class=\"chip active\">Maybe</button>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("aria-checked=\"true\"").count(), 1, "{html}");
        assert_eq!(html.matches("aria-checked=\"false\"").count(), 2, "{html}");
        // The checked radio is the group's one Tab stop.
        assert_eq!(html.matches("tabindex=\"0\"").count(), 1, "{html}");
        assert_eq!(html.matches("tabindex=\"-1\"").count(), 2, "{html}");
    }

    #[test]
    fn long_list_shows_one_page_and_the_pager() {
        let events: Vec<Event> = (0..12u8)
            .map(|i| event(&format!("e{i}"), &format!("Night {i}"), i % 7, "19:00"))
            .collect();
        let html = render(Show::List(schedule(events)), Viewer::Guest);
        assert!(
            html.contains(
                "<h2 id=\"events-schedule-title\" class=\"ui-section-title\">Schedule</h2>"
            ),
            "{html}"
        );
        // The schedule takes focus after a page change.
        assert!(
            html.contains(
                "<section id=\"events-schedule\" class=\"ui-stack pager-target\" tabindex=\"-1\" aria-labelledby=\"events-schedule-title\">"
            ),
            "{html}"
        );
        assert_eq!(html.matches("<article").count(), PAGE_SIZE, "{html}");
        assert_eq!(html.matches("<li").count(), PAGE_SIZE, "{html}");
        assert!(html.contains("1 to 10 of 12 events"), "{html}");
        assert!(html.contains("id=\"events-pager-next\""), "{html}");
        assert!(!html.contains("No events scheduled"), "{html}");
        assert_eq!(html.matches("<h1").count(), 0, "the header owns the h1");
    }

    #[test]
    fn one_event_reads_as_singular() {
        let html = render(
            Show::List(vec![event("e1", "Play night", 0, "19:00")]),
            Viewer::Guest,
        );
        assert!(html.contains(">1 event<"), "{html}");
    }

    #[test]
    fn event_reads_the_api_shape() {
        let parsed: Event = serde_json::from_str(
            r#"{"id":"e1","title":"Play night","day_of_week":2,"time":"19:00",
                "timezone":"Europe/Oslo","duration_minutes":90,"is_recurring":false,
                "team_id":null,"created_by":"member-1","is_active":true,"is_public":false}"#,
        )
        .unwrap();
        assert_eq!(when_text(&parsed), "Wednesday, 19:00");
        assert!(!parsed.is_public);
        assert_eq!(format_duration(parsed.duration_minutes), "1h 30m");
        assert_eq!(format_duration(45), "45m");
    }

    #[test]
    fn copy_has_no_long_dashes() {
        let src = include_str!("events.rs");
        for dash in ['\u{2013}', '\u{2014}'] {
            assert!(!src.contains(dash), "found {dash:?}");
        }
    }
}
