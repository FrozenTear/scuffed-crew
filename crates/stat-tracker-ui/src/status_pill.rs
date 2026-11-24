//! The status pill at the right of the 0.6 top bar (UX spec 1): the
//! tracker's state in words, and a menu with Start, Restart and Stop.
//!
//! The pill keeps only UI state ([`PillState`]): the verb in flight, the
//! last failed Start or Restart, the open menu, the mid-game confirm and
//! the pill's keyboard focus. The verbs run through
//! [`crate::daemon::run_verb`] as before; this module only records which
//! one runs, so the pill can say Starting..., Restarting... or
//! Stopping....
//!
//! The menu and the confirm are layers over the window
//! ([`crate::chrome::with_layers`]), always present, empty when closed,
//! so the page keeps its scroll when either opens or closes.
//!
//! iced 0.14 buttons take no keyboard focus, so the pill keeps its own:
//! Tab (when no field took it) lands on the pill and Tab again leaves it.
//! On the pill, Space or Enter opens the menu (on the unreadable pill they
//! open Settings, as a click does) and Down opens the menu in every state.
//! In the menu, Up and Down move, Enter or Space chooses, Escape closes.

use iced::widget::text::Wrapping;
use iced::widget::{
    Column, button, column, container, image, mouse_area, opaque, responsive, row, space, stack,
    text,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Padding, Shadow, Task, Theme};

use crate::app::{Message, TrackerApp};
use crate::daemon::{self, DaemonVerb};
use crate::nav::NavStyle;
use crate::nav_fit::{
    CHIP_GAP, CHIP_GLYPH_GAP, DOT_GAP, PILL_CHEVRON, PILL_PAD_X, QUESTION_BUTTON, SIZE_CHIP,
    SIZE_STATUS, StatusForm,
};
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL, SURFACE,
    TEXT, TEXT_2, WARN,
};
use crate::tracker_status::{self, HeaderStatus};

/// What the pill remembers between frames (spec 1.8).
#[derive(Debug, Clone, Default)]
pub struct PillState {
    /// The verb [`run`] started and that has not answered yet.
    pub in_flight: Option<DaemonVerb>,
    /// The last Start or Restart that failed, until the next verb.
    pub failure: Option<Failure>,
    /// The pill has the keyboard focus (Tab).
    pub focused: bool,
    /// The menu is open, with this item marked for the keyboard.
    pub menu: Option<usize>,
    /// The mid-game confirm is open.
    pub confirm: Option<Confirm>,
}

/// A failed Start or Restart, with the error [`daemon::run_verb`] gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub verb: DaemonVerb,
    pub reason: String,
}

/// "Stop the tracker?" or "Restart the tracker?" (spec 1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confirm {
    pub verb: DaemonVerb,
    /// The keyboard is on the Stop or Restart button. It starts on Keep
    /// tracking, the default.
    pub on_action: bool,
}

/// One menu item (spec 1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Start,
    Restart,
    Stop,
    /// The Error menu's link to Settings, Troubleshooting.
    Troubleshooting,
    /// The Error menu: the verb that failed, or Start if the tracker is down.
    Retry,
}

/// A message from the pill, its menu or its confirm.
#[derive(Debug, Clone)]
pub enum Msg {
    /// A click on the pill or its chevron: open or close the menu.
    Toggle,
    /// A click outside the open menu.
    Close,
    Choose(Item),
    /// Keep tracking, Escape or a click outside the confirm.
    Keep,
    /// Stop tracker or Restart tracker in the confirm.
    Go,
    Key(Key),
}

/// The keys the pill, its menu and its confirm answer to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Tab,
    Enter,
    Space,
    Up,
    Down,
    Escape,
}

/// The words of an item.
pub fn item_label(item: Item) -> &'static str {
    match item {
        Item::Start => "Start tracker",
        Item::Restart => "Restart tracker",
        Item::Stop => "Stop tracker",
        Item::Troubleshooting => "Troubleshooting",
        Item::Retry => "Retry",
    }
}

/// The tooltip line that says the pill has a menu (spec 1.6).
pub const MENU_HINT: &str = "Menu: start or stop the tracker";
/// The confirm's body, the same for Stop and Restart (spec 1.4).
pub const CONFIRM_BODY: &str = "The current game won't be recorded.";
/// The confirm's default button.
pub const KEEP_TRACKING: &str = "Keep tracking";

/// The confirm's title and its action button's words.
pub fn confirm_words(verb: DaemonVerb) -> (&'static str, &'static str) {
    match verb {
        DaemonVerb::Stop => ("Stop the tracker?", "Stop tracker"),
        _ => ("Restart the tracker?", "Restart tracker"),
    }
}

/// "Applies 1 change" beside Restart tracker when saved changes wait for
/// a restart (spec 1.3).
pub fn applies_label(waiting: usize) -> String {
    if waiting == 1 {
        "Applies 1 change".to_string()
    } else {
        format!("Applies {waiting} changes")
    }
}

/// The menu items now, in order. None while a verb (or Start on login, or
/// the update restart) runs, in the demo and in the classic layout.
pub fn items(app: &TrackerApp) -> Vec<Item> {
    if app.nav_style != NavStyle::TopTabs
        || app.fixture.is_some()
        || app.daemon_busy
        || app.pill.in_flight.is_some()
    {
        return Vec::new();
    }
    if app.pill.failure.is_some() {
        vec![Item::Troubleshooting, Item::Retry]
    } else if app.daemon.running() {
        vec![Item::Restart, Item::Stop]
    } else {
        vec![Item::Start]
    }
}

/// What Retry runs: the verb that failed, or Start once the tracker is
/// down (spec 1.3).
pub fn retry_verb(app: &TrackerApp) -> DaemonVerb {
    match &app.pill.failure {
        Some(failure) if app.daemon.running() => failure.verb,
        _ => DaemonVerb::Start,
    }
}

/// A match is in progress: the game file is open, or the Games tab has a
/// live game. Overwatch open on a menu is not one (spec 1.4).
pub fn match_in_progress(app: &TrackerApp) -> bool {
    app.live_game_session.is_some() || stat_tracker::replace::game_file_in_progress(&app.data_dir)
}

/// Whether `verb` from the menu asks first: only Stop and Restart, only
/// mid-game, never in the demo.
pub fn asks_first(app: &TrackerApp, verb: DaemonVerb) -> bool {
    verb != DaemonVerb::Start && app.fixture.is_none() && match_in_progress(app)
}

/// [`Message::Daemon`]: start a verb, as before, and record it. Ignored
/// in the demo and while anything else runs.
pub(crate) fn run(app: &mut TrackerApp, verb: DaemonVerb) -> Task<Message> {
    if app.fixture.is_some() || app.daemon_busy {
        return Task::none();
    }
    app.daemon_busy = true;
    app.pill.in_flight = Some(verb);
    app.pill.failure = None;
    app.pill.menu = None;
    app.pill.confirm = None;
    if verb.applies_saved_config() {
        app.page_state.restart_started(&app.saved_config);
    }
    let data_dir = app.data_dir.clone();
    let installed = app.daemon.service_installed;
    Task::perform(
        daemon::run_verb(data_dir, verb, installed),
        Message::DaemonDone,
    )
}

/// [`Message::DaemonDone`]: refresh the view and show the toast, as
/// before. A failed Start or Restart the pill ran becomes its Error.
/// Start on login and the save path's restart run no pill verb, so they
/// only toast.
pub(crate) fn done(app: &mut TrackerApp, result: Result<String, String>) -> Task<Message> {
    app.daemon_busy = false;
    if let Some(verb) = app.pill.in_flight.take() {
        app.pill.failure = match &result {
            Err(reason) if verb != DaemonVerb::Stop => Some(Failure {
                verb,
                reason: reason.clone(),
            }),
            _ => None,
        };
    }
    app.page_state.daemon_done(result.is_ok());
    app.replace_toast(match result {
        Ok(m) => m,
        Err(e) => e,
    });
    app.daemon = daemon::refresh_view(&app.data_dir, &app.daemon);
    app.page_state
        .observe_tracker(app.daemon.pid, &app.saved_config);
    Task::perform(daemon::systemd_enabled(), Message::AutostartReady)
}

/// [`Message::Pill`].
pub(crate) fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    match msg {
        Msg::Toggle if app.pill.menu.is_some() => {
            app.pill.menu = None;
            Task::none()
        }
        Msg::Toggle => {
            open_menu(app);
            Task::none()
        }
        Msg::Close => {
            app.pill.menu = None;
            Task::none()
        }
        Msg::Choose(item) => choose(app, item),
        Msg::Keep => {
            app.pill.confirm = None;
            Task::none()
        }
        Msg::Go => match app.pill.confirm.take() {
            Some(confirm) => run(app, confirm.verb),
            None => Task::none(),
        },
        Msg::Key(key) => on_key(app, key),
    }
}

fn open_menu(app: &mut TrackerApp) {
    app.pill.menu = (!items(app).is_empty()).then_some(0);
}

/// An item, by click or key. One that is no longer offered (a verb
/// started meanwhile) does nothing.
fn choose(app: &mut TrackerApp, item: Item) -> Task<Message> {
    app.pill.menu = None;
    if !items(app).contains(&item) {
        return Task::none();
    }
    let verb = match item {
        Item::Troubleshooting => {
            let section = crate::settings_model::AdvancedSection::Troubleshooting;
            return app.update(Message::Nav(crate::nav::Msg::ShowAdvancedSection(section)));
        }
        Item::Start => DaemonVerb::Start,
        Item::Retry => retry_verb(app),
        Item::Restart => DaemonVerb::Restart,
        Item::Stop => DaemonVerb::Stop,
    };
    if matches!(item, Item::Restart | Item::Stop) && asks_first(app, verb) {
        app.pill.confirm = Some(Confirm {
            verb,
            on_action: false,
        });
        return Task::none();
    }
    run(app, verb)
}

fn on_key(app: &mut TrackerApp, key: Key) -> Task<Message> {
    if app.nav_style != NavStyle::TopTabs {
        return Task::none();
    }
    if let Some(confirm) = app.pill.confirm.as_mut() {
        return match key {
            Key::Tab => {
                confirm.on_action = !confirm.on_action;
                Task::none()
            }
            Key::Enter | Key::Space if confirm.on_action => update(app, Msg::Go),
            Key::Enter | Key::Space | Key::Escape => update(app, Msg::Keep),
            Key::Up | Key::Down => Task::none(),
        };
    }
    if let Some(at) = app.pill.menu {
        let list = items(app);
        if list.is_empty() {
            app.pill.menu = None;
            return Task::none();
        }
        let len = list.len();
        let at = at.min(len - 1);
        match key {
            Key::Up => app.pill.menu = Some((at + len - 1) % len),
            Key::Down => app.pill.menu = Some((at + 1) % len),
            Key::Enter | Key::Space => return choose(app, list[at]),
            Key::Escape | Key::Tab => app.pill.menu = None,
        }
        return Task::none();
    }
    if !app.pill.focused {
        // Tab from the page lands on the pill, the bar's one focus stop.
        app.pill.focused = key == Key::Tab;
        return Task::none();
    }
    match key {
        Key::Tab | Key::Escape => app.pill.focused = false,
        Key::Down => open_menu(app),
        Key::Enter | Key::Space
            if tracker_status::header_status_for(app) == HeaderStatus::FocusUnknown =>
        {
            return app.update(Message::Nav(crate::nav::Msg::ShowFocusSetting));
        }
        Key::Enter | Key::Space => open_menu(app),
        Key::Up => {}
    }
    Task::none()
}

/// The pill's menu, confirm or focus has the keys: Escape is theirs, not
/// the page's.
pub(crate) fn owns_keys(app: &TrackerApp) -> bool {
    app.pill.menu.is_some() || app.pill.confirm.is_some() || app.pill.focused
}

/// Push the pill's key listener (top tabs only). True when the pill owns
/// the keys ([`owns_keys`]), so the page's own Escape listeners stay off.
pub(crate) fn listen(app: &TrackerApp, subs: &mut Vec<iced::Subscription<Message>>) -> bool {
    if app.nav_style != NavStyle::TopTabs {
        return false;
    }
    subs.push(iced::event::listen_with(keys));
    owns_keys(app)
}

/// A key press no field took, as a pill key.
fn keys(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) = event
    else {
        return None;
    };
    key_message(&key, modifiers, status)
}

/// [`keys`] for one key press: none when a field already took it.
fn key_message(
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
    status: iced::event::Status,
) -> Option<Message> {
    if status == iced::event::Status::Captured {
        return None;
    }
    pill_key(key, modifiers).map(|key| Message::Pill(Msg::Key(key)))
}

/// The pill key for a key press, if any. Shift+Tab counts as Tab: the pill
/// is the only stop. Control, Alt and Super chords are not the pill's.
pub fn pill_key(key: &iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) -> Option<Key> {
    use iced::keyboard::Key as K;
    use iced::keyboard::key::Named;
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    match key {
        K::Named(Named::Tab) => Some(Key::Tab),
        K::Named(Named::Enter) => Some(Key::Enter),
        K::Named(Named::Space) => Some(Key::Space),
        K::Named(Named::ArrowUp) => Some(Key::Up),
        K::Named(Named::ArrowDown) => Some(Key::Down),
        K::Named(Named::Escape) => Some(Key::Escape),
        K::Character(c) if c.as_str() == " " => Some(Key::Space),
        _ => None,
    }
}

/// The pill (spec 1.3): a 1 px border in every state, a SURFACE fill on
/// hover, an ACCENT ring while the menu is open or the pill has the
/// keyboard. The unreadable state keeps its amber outline and opens
/// Settings; its chevron part opens the menu. Under 1100 px it is the "?"
/// button, which only opens Settings.
pub(crate) fn pill<'a>(
    app: &'a TrackerApp,
    status: HeaderStatus,
    form: StatusForm,
    label: String,
    time: Option<String>,
    chevron: bool,
) -> Element<'a, Message> {
    let ring = app.pill.menu.is_some() || app.pill.focused;
    let toggle = (!items(app).is_empty()).then_some(Message::Pill(Msg::Toggle));
    let settings = Message::Nav(crate::nav::Msg::ShowFocusSetting);
    let pad = |left: f32, right: f32| Padding {
        top: 5.0,
        bottom: 5.0,
        left,
        right,
    };
    if status == HeaderStatus::FocusUnknown {
        if form == StatusForm::Short {
            let mark = text(label)
                .size(SIZE_CHIP)
                .font(FONT_BOLD)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center);
            return button(container(mark).center_x(Fill).center_y(Fill))
                .width(QUESTION_BUTTON)
                .height(QUESTION_BUTTON)
                .padding(0)
                .style(pill_style(theme::WARN_LINE, ring, true))
                .on_press(settings)
                .into();
        }
        let words = button(status_words(status, label, time))
            .padding(pad(PILL_PAD_X, CHIP_GLYPH_GAP / 2.0))
            .style(pill_style(Color::TRANSPARENT, false, true))
            .on_press(settings);
        let mut parts = row![words].align_y(Alignment::Center);
        if chevron {
            parts = parts.push(
                button(chevron_icon())
                    .padding(pad(CHIP_GLYPH_GAP / 2.0, PILL_PAD_X))
                    .style(pill_style(Color::TRANSPARENT, false, true))
                    .on_press_maybe(toggle),
            );
        }
        let edge = if ring { ACCENT } else { theme::WARN_LINE };
        return container(parts)
            .style(move |_: &Theme| container::Style {
                border: Border {
                    color: edge,
                    width: 1.0,
                    radius: theme::RADIUS_CHIP.into(),
                },
                ..container::Style::default()
            })
            .into();
    }
    let mut content = row![status_words(status, label, time)]
        .spacing(CHIP_GLYPH_GAP)
        .align_y(Alignment::Center);
    if chevron {
        content = content.push(chevron_icon());
    }
    // Busy and the demo have no menu: no press, but no grey either.
    button(content)
        .padding(pad(PILL_PAD_X, PILL_PAD_X))
        .style(pill_style(BORDER, ring, false))
        .on_press_maybe(toggle)
        .into()
}

/// The dot (or the pause ring), the words in TEXT and the time in TEXT_2.
fn status_words(
    status: HeaderStatus,
    label: String,
    time: Option<String>,
) -> Element<'static, Message> {
    let color = tracker_status::header_status_color(status);
    let mark = if status == HeaderStatus::Paused {
        crate::nav::ring(color)
    } else {
        crate::nav::dot(color)
    };
    let mut words = row![
        text(label)
            .size(SIZE_STATUS)
            .font(FONT_SEMIBOLD)
            .color(TEXT)
            .wrapping(Wrapping::None)
    ]
    .spacing(crate::nav_fit::TIME_GAP)
    .align_y(Alignment::Center);
    if let Some(time) = time {
        words = words.push(
            text(time)
                .size(SIZE_STATUS)
                .font(FONT_SEMIBOLD)
                .color(TEXT_2)
                .wrapping(Wrapping::None),
        );
    }
    row![mark, words]
        .spacing(DOT_GAP)
        .align_y(Alignment::Center)
        .into()
}

fn chevron_icon() -> Element<'static, Message> {
    image(crate::icons::handle(crate::icons::Icon::FoldOpen))
        .width(PILL_CHEVRON)
        .height(PILL_CHEVRON)
        .into()
}

/// The pill's border `edge` (ACCENT when `ring`), a SURFACE fill on hover
/// (an amber tint on the unreadable pill), never greyed when it has no
/// press.
fn pill_style(
    edge: Color,
    ring: bool,
    amber: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let fill = match (hovered, amber) {
            (false, _) => None,
            (true, true) => Some(Background::Color(Color { a: 0.10, ..WARN })),
            (true, false) => Some(Background::Color(SURFACE)),
        };
        button::Style {
            background: fill,
            text_color: TEXT,
            border: Border {
                color: if ring { ACCENT } else { edge },
                width: 1.0,
                radius: theme::RADIUS_CHIP.into(),
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// The open menu's top edge under a one-line bar and under the first line
/// of a two-line bar: 4 px under the pill. The one-line bar is a tab tall
/// (20 + 18 + 16 + 2 px), the pill centred in it; the two-line bar puts
/// the pill in its first line, 12 px from the top.
pub const MENU_TOP_ONE_LINE: f32 = 46.0;
pub const MENU_TOP_TWO_LINES: f32 = 44.0;
/// The menu's width (spec 1.3: at most 280 px).
pub const MENU_WIDTH: f32 = 260.0;

/// The menu layer: a click-catcher over the window (a click outside closes
/// the menu) and the menu right-aligned under the pill. `None` while the
/// menu is closed.
pub(crate) fn menu_layer(app: &TrackerApp) -> Option<Element<'_, Message>> {
    let at = app.pill.menu?;
    if items(app).is_empty() {
        return None;
    }
    Some(
        responsive(move |size| {
            let catcher =
                mouse_area(space().width(Fill).height(Fill)).on_press(Message::Pill(Msg::Close));
            let (top, right) = menu_offset(app, size.width);
            let placed = container(opaque(menu_card(app, at)))
                .padding(Padding {
                    top,
                    right,
                    ..Padding::ZERO
                })
                .width(Fill)
                .height(Fill)
                .align_x(iced::alignment::Horizontal::Right)
                .align_y(iced::alignment::Vertical::Top);
            stack![catcher, placed].width(Fill).height(Fill).into()
        })
        .width(Fill)
        .height(Fill)
        .into(),
    )
}

/// Where the menu goes on a `width` px window: its top edge, and its right
/// edge in from the window's (the gutter, and the chip when it shows).
pub fn menu_offset(app: &TrackerApp, width: f32) -> (f32, f32) {
    let count = crate::nav::needs_look_count(&app.games, app.live_game_session.as_deref());
    let status = tracker_status::header_status_for(app);
    let last_game = tracker_status::last_game_when(&app.games, app.clock);
    let fit = crate::nav_fit::layout_kept(width, status, last_game.as_deref(), count);
    let chip = if count > 0 {
        CHIP_GAP + crate::nav_fit::chip_width(count, fit.compact_chip)
    } else {
        0.0
    };
    let top = if fit.one_line {
        MENU_TOP_ONE_LINE
    } else {
        MENU_TOP_TWO_LINES
    };
    (top, fit.gutter + chip)
}

/// The menu (spec 1.3): SURFACE, a 1 px BORDER, RADIUS_INNER. On Error the
/// reason comes first, then Troubleshooting and Retry.
fn menu_card(app: &TrackerApp, at: usize) -> Element<'_, Message> {
    let waiting = app.changes_waiting();
    let mut list = Column::new().spacing(2).width(Fill);
    if let Some(failure) = &app.pill.failure {
        list = list.push(
            container(
                text(failure.reason.as_str())
                    .size(SIZE_STATUS)
                    .font(FONT_MEDIUM)
                    .color(TEXT_2),
            )
            .padding(Padding::from([8, 12])),
        );
    }
    for (i, item) in items(app).into_iter().enumerate() {
        list = list.push(menu_item(item, i == at, waiting));
    }
    container(list)
        .padding(4)
        .width(MENU_WIDTH)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: theme::RADIUS_INNER.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// One item: 14 px semibold, 8 by 12 px padding. Restart is the primary
/// item (accent, "Applies N changes") while changes wait; Troubleshooting
/// is a link in ACCENT.
fn menu_item(item: Item, marked: bool, waiting: usize) -> Element<'static, Message> {
    let primary = item == Item::Restart && waiting > 0;
    let color = if primary || item == Item::Troubleshooting {
        ACCENT
    } else {
        TEXT
    };
    let mut line = row![
        text(item_label(item))
            .size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .color(color)
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if primary {
        line = line.push(space().width(Fill)).push(
            text(applies_label(waiting))
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        );
    }
    button(line)
        .width(Fill)
        .padding(Padding::from([8, 12]))
        .style(move |_: &Theme, status| button::Style {
            background: theme::hover_tint(status)
                .or_else(|| marked.then_some(Background::Color(Color { a: 0.07, ..TEXT }))),
            text_color: TEXT,
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            shadow: Shadow::default(),
            snap: false,
        })
        .on_press(Message::Pill(Msg::Choose(item)))
        .into()
}

/// The confirm layer (spec 1.4): the backdrop (a click on it keeps
/// tracking) and the card with Keep tracking and Stop or Restart.
pub(crate) fn confirm_layer(app: &TrackerApp) -> Option<Element<'_, Message>> {
    let confirm = app.pill.confirm?;
    let (title, action) = confirm_words(confirm.verb);
    let keep = button(
        text(KEEP_TRACKING)
            .size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([9, 18]))
    .style(with_ring(theme::ghost_btn(), !confirm.on_action))
    .on_press(Message::Pill(Msg::Keep));
    let go = button(text(action).size(SIZE_BODY).font(FONT_SEMIBOLD).color(TEXT))
        .padding(Padding::from([9, 18]));
    let go = if confirm.verb == DaemonVerb::Stop {
        go.style(with_ring(theme::danger_btn(true), confirm.on_action))
    } else {
        go.style(with_ring(
            crate::settings_page::primary_btn(true),
            confirm.on_action,
        ))
    };
    let card = container(
        column![
            text(title)
                .size(SIZE_BODY + 4.0)
                .font(FONT_BOLD)
                .color(TEXT),
            text(CONFIRM_BODY)
                .size(SIZE_BODY)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
            row![
                space().width(Fill),
                keep,
                go.on_press(Message::Pill(Msg::Go))
            ]
            .spacing(8),
        ]
        .spacing(12),
    )
    .padding(24)
    .max_width(420)
    .style(theme::surface_panel);
    let mut backdrop = theme::BG;
    backdrop.a = 0.88;
    let scrim = mouse_area(
        container(space())
            .width(Fill)
            .height(Fill)
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(backdrop)),
                ..container::Style::default()
            }),
    )
    .on_press(Message::Pill(Msg::Keep));
    let card = container(opaque(card)).padding(28).center(Fill);
    Some(stack![scrim, card].width(Fill).height(Fill).into())
}

/// `style` with a 2 px ACCENT ring when the button has the keyboard.
fn with_ring<S>(style: S, ring: bool) -> impl Fn(&Theme, button::Status) -> button::Style
where
    S: Fn(&Theme, button::Status) -> button::Style,
{
    move |theme_, status| {
        let mut out = style(theme_, status);
        if ring {
            out.border.color = ACCENT;
            out.border.width = 2.0;
        }
        out
    }
}

#[cfg(test)]
#[path = "status_pill_tests.rs"]
mod tests;
