//! Keyboard focus with a dialog open, on the production window
//! ([`TrackerApp::view`]) and its real dialogs. The page under a dialog
//! must get no keys, the dialog takes the focus and keeps Tab, and Esc
//! closes it and gives the page field its focus back
//! ([`crate::dialog_focus`]).

use super::tree_tests::headless;
use super::*;
use iced::advanced::layout::Limits;
use iced::advanced::widget::operation::{Focusable, Operation};
use iced::advanced::widget::{Id, Tree};
use iced::advanced::{Layout, Shell, clipboard};
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::{Event, Point, Rectangle, Size, mouse};

use crate::dialog_focus::DIALOG_ID;

const SIZE: Size = Size::new(1280.0, 860.0);

/// The window kept across frames as iced keeps it: one tree, diffed
/// against each new view of the app.
struct Ui {
    renderer: iced::Renderer,
    tree: Option<Tree>,
    view: View,
}

type View = for<'a> fn(&'a TrackerApp) -> Element<'a, Message>;

fn production(app: &TrackerApp) -> Element<'_, Message> {
    app.view(iced::window::Id::unique())
}

/// What one event did: the messages it published, whether a widget kept
/// it, and whether a focused text field asked for the input method (it
/// does on each redraw).
struct Sent {
    messages: Vec<Message>,
    captured: bool,
    typing: bool,
}

impl Ui {
    fn new() -> Self {
        Self {
            renderer: headless(),
            tree: None,
            view: production,
        }
    }

    fn with<R>(
        &mut self,
        app: &TrackerApp,
        f: impl FnOnce(&mut Element<'_, Message>, &mut Tree, Layout<'_>, &iced::Renderer) -> R,
    ) -> R {
        let mut element = (self.view)(app);
        let tree = match &mut self.tree {
            Some(tree) => {
                tree.diff(&element);
                tree
            }
            None => self.tree.insert(Tree::new(&element)),
        };
        let limits = Limits::new(Size::ZERO, SIZE);
        let node = element
            .as_widget_mut()
            .layout(tree, &self.renderer, &limits);
        f(&mut element, tree, Layout::new(&node), &self.renderer)
    }

    fn send(&mut self, app: &TrackerApp, event: Event) -> Sent {
        self.with(app, |element, tree, layout, renderer| {
            let mut messages = Vec::new();
            let mut shell = Shell::new(&mut messages);
            element.as_widget_mut().update(
                tree,
                &event,
                layout,
                mouse::Cursor::Unavailable,
                renderer,
                &mut clipboard::Null,
                &mut shell,
                &Rectangle::new(Point::ORIGIN, SIZE),
            );
            let captured = shell.is_event_captured();
            let typing = matches!(
                shell.input_method(),
                iced::advanced::input_method::InputMethod::Enabled { .. }
            );
            Sent {
                messages,
                captured,
                typing,
            }
        })
    }

    /// The first event after a view change, as iced sends one each frame.
    fn redraw(&mut self, app: &TrackerApp) -> Sent {
        let now = std::time::Instant::now();
        self.send(
            app,
            Event::Window(iced::window::Event::RedrawRequested(now)),
        )
    }

    fn operate(&mut self, app: &TrackerApp, operation: &mut dyn Operation) {
        self.with(app, |element, tree, layout, renderer| {
            element
                .as_widget_mut()
                .operate(tree, layout, renderer, operation);
        });
    }

    fn focuses(&mut self, app: &TrackerApp) -> Focuses {
        let mut focuses = Focuses::default();
        self.operate(app, &mut focuses);
        focuses
    }
}

fn press(key: Key, text: Option<&str>, modifiers: Modifiers) -> Event {
    Event::Keyboard(keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers,
        text: text.map(Into::into),
        repeat: false,
    })
}

fn type_x() -> Event {
    press(Key::Character("x".into()), Some("x"), Modifiers::empty())
}

fn named(named: Named) -> Event {
    press(Key::Named(named), None, Modifiers::empty())
}

fn shift_tab() -> Event {
    press(Key::Named(Named::Tab), None, Modifiers::SHIFT)
}

/// Every focusable widget a focus operation sees, in order.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Focus {
    /// Inside the top dialog's layer.
    in_dialog: bool,
    /// The dialog layer itself.
    itself: bool,
    bounds: Rectangle,
    focused: bool,
}

#[derive(Debug, Default)]
struct Focuses {
    depth: usize,
    dialog_depth: Option<usize>,
    /// A top dialog layer was seen.
    dialog: bool,
    all: Vec<Focus>,
}

impl Focuses {
    fn focused(&self) -> Vec<Focus> {
        self.all.iter().filter(|f| f.focused).copied().collect()
    }

    fn in_dialog(&self) -> usize {
        self.all.iter().filter(|f| f.in_dialog && !f.itself).count()
    }
}

impl Operation for Focuses {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        self.depth += 1;
        operate(self);
        self.depth -= 1;
        if self.dialog_depth == Some(self.depth) {
            self.dialog_depth = None;
        }
    }

    fn container(&mut self, id: Option<&Id>, _bounds: Rectangle) {
        if id == Some(&Id::new(DIALOG_ID)) {
            self.dialog_depth = Some(self.depth);
            self.dialog = true;
        }
    }

    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        self.all.push(Focus {
            in_dialog: self.dialog_depth.is_some(),
            itself: id == Some(&Id::new(DIALOG_ID)),
            bounds,
            focused: state.is_focused(),
        });
    }
}

/// Focuses the `n`th focusable widget the operation reaches.
struct FocusNth {
    n: usize,
    current: usize,
}

impl Operation for FocusNth {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if self.current == self.n {
            state.focus();
        } else {
            state.unfocus();
        }
        self.current += 1;
    }
}

/// A Settings apply: what Enter in a Settings field sends.
fn saves(message: &Message) -> bool {
    let debug = format!("{message:?}");
    debug.contains("Commit(") || debug.contains("Apply(") || debug.contains("SaveSettings")
}

fn types_into_settings(sent: &Sent) -> bool {
    sent.messages
        .iter()
        .any(|m| matches!(m, Message::SettingsText(..)))
}

/// The 0.6 Settings page with fields that take typing (not a demo), and
/// its first field that takes typing and saves on Enter, focused: its
/// index among the focusable widgets.
fn settings_with_a_focused_field() -> (tempfile::TempDir, TrackerApp, Ui, usize) {
    settings_field_in(NavStyle::TopTabs)
}

const LAYOUTS: [NavStyle; 2] = [NavStyle::TopTabs, NavStyle::ClassicSidebar];

/// [`settings_with_a_focused_field`] in the layout `nav`. The classic
/// Settings form saves with its Save button, not on Enter, so there it is
/// the first field that takes typing.
fn settings_field_in(nav: NavStyle) -> (tempfile::TempDir, TrackerApp, Ui, usize) {
    let (dir, mut app) = crate::fixtures::test_app();
    super::tree_tests::close_launch_notes(&mut app);
    // Not a demo, so the fields take input. Nothing here applies a save.
    app.fixture = None;
    app.nav_style = nav;
    app.screen = Screen::Settings;
    app.toast = None;
    let mut ui = Ui::new();
    ui.redraw(&app);
    let total = ui.focuses(&app).all.len();
    let fields: Vec<(usize, bool)> = (0..total)
        .filter_map(|n| {
            ui.operate(&app, &mut FocusNth { n, current: 0 });
            let typed = ui.send(&app, type_x());
            let entered = ui.send(&app, named(Named::Enter));
            let saves = entered.messages.iter().any(saves);
            types_into_settings(&typed).then_some((n, saves))
        })
        .collect();
    let saving = fields.iter().find(|(_, saves)| *saves);
    let field = match nav {
        NavStyle::TopTabs => saving.expect("a Settings field that saves on Enter").0,
        NavStyle::ClassicSidebar => saving.or(fields.first()).expect("a Settings field").0,
    };
    ui.operate(
        &app,
        &mut FocusNth {
            n: field,
            current: 0,
        },
    );
    // Settle while live so last_id is this field, not the last field
    // the probe loop focused.
    ui.redraw(&app);
    let focused = ui.focuses(&app).focused();
    assert_eq!(focused.len(), 1, "{focused:?}");
    (dir, app, ui, field)
}

type Open = fn(&mut TrackerApp);

/// Every dialog in the window, opened the way the app opens it.
const DIALOGS: [(&str, Open); 5] = [
    ("What's new", |app| {
        let _ = app.update(Message::OpenAbout);
    }),
    ("the setup guide", |app| app.setup.open = true),
    ("Uninstall", |app| {
        app.uninstall_dialog = Some(crate::uninstall::open_dialog(
            std::path::Path::new("/nonexistent/scuffed-stat-tracker-gui"),
            std::path::Path::new("/nonexistent/home"),
            None,
            std::path::Path::new("/nonexistent/data"),
        ));
    }),
    ("Report a problem", |app| {
        let open = crate::report::ReportMessage::Open(None);
        let _ = app.update(Message::Report(open));
    }),
    ("What leaves your PC", |app| {
        let open = crate::settings_open::OpenMsg::WhatLeaves(true);
        let _ = app.update(Message::SettingsPage(crate::settings_update::Msg::Open(
            open,
        )));
    }),
];

/// Every dialog in each layout.
fn each_case() -> impl Iterator<Item = (String, NavStyle, Open)> {
    LAYOUTS.into_iter().flat_map(|nav| {
        DIALOGS
            .into_iter()
            .map(move |(name, open)| (format!("{nav:?}, {name}"), nav, open))
    })
}

/// With a Settings field focused, no key typed while a dialog is open
/// reaches it: typing changes nothing on the page.
#[test]
fn typing_with_a_dialog_open_never_reaches_the_page_field() {
    for (name, nav, open) in each_case() {
        let (_dir, mut app, mut ui, _) = settings_field_in(nav);
        assert!(
            types_into_settings(&ui.send(&app, type_x())),
            "{name}: the field takes typing before the dialog opens"
        );
        open(&mut app);
        ui.redraw(&app);
        let space = press(Key::Named(Named::Space), Some(" "), Modifiers::empty());
        for event in [type_x(), named(Named::Backspace), space] {
            let sent = ui.send(&app, event);
            assert!(
                !types_into_settings(&sent),
                "{name}: a key reached the page field: {:?}",
                sent.messages
            );
        }
    }
}

/// Enter with a dialog open does not save Settings from the hidden field.
#[test]
fn enter_with_a_dialog_open_does_not_apply_settings() {
    for (name, nav, open) in each_case() {
        let (_dir, mut app, mut ui, _) = settings_field_in(nav);
        open(&mut app);
        for _ in 0..2 {
            let sent = ui.send(&app, named(Named::Enter));
            assert!(
                !sent.messages.iter().any(saves),
                "{name}: Enter applied Settings: {:?}",
                sent.messages
            );
        }
    }
}

/// On open the dialog has the focus: its first focusable widget, or the
/// dialog itself when it has none, and nothing on the page. The first Tab
/// and Shift+Tab stay inside it, and Tab cycles round its own widgets.
#[test]
fn the_dialog_takes_the_focus_and_keeps_tab() {
    for (name, open) in DIALOGS {
        let (_dir, mut app, mut ui, _) = settings_with_a_focused_field();
        open(&mut app);
        let first = ui.send(&app, named(Named::Tab));
        assert!(first.captured, "{name}: the dialog keeps Tab");
        let inside = |focuses: &Focuses| {
            let focused = focuses.focused();
            assert!(focuses.dialog, "{name}: the dialog is open");
            assert_eq!(focused.len(), 1, "{name}: one focus: {focused:?}");
            assert!(focused[0].in_dialog, "{name}: {focused:?}");
            focused[0]
        };
        let after_tab = inside(&ui.focuses(&app));
        let widgets = ui.focuses(&app).in_dialog();
        eprintln!("{name}: {widgets} focusable widgets, after Tab {after_tab:?}");
        assert_eq!(after_tab.itself, widgets == 0, "{name}");
        // Round the dialog's widgets and back to the same one.
        for _ in 0..widgets.max(1) {
            assert!(ui.send(&app, named(Named::Tab)).captured, "{name}");
            inside(&ui.focuses(&app));
        }
        assert_eq!(inside(&ui.focuses(&app)), after_tab, "{name}: Tab cycles");
        assert!(ui.send(&app, shift_tab()).captured, "{name}");
        inside(&ui.focuses(&app));
    }
}

/// The first focus goes to the first focusable widget in the dialog, or
/// the dialog itself, as soon as it opens (before any key).
#[test]
fn the_dialog_focuses_its_first_widget_on_open() {
    for (name, open) in DIALOGS {
        let (_dir, mut app, mut ui, _) = settings_with_a_focused_field();
        open(&mut app);
        ui.redraw(&app);
        let focuses = ui.focuses(&app);
        let focused = focuses.focused();
        assert_eq!(focused.len(), 1, "{name}: {focused:?}");
        assert!(
            focuses.all.iter().all(|f| f.in_dialog),
            "{name}: focus operations do not see the page under the dialog"
        );
        let first = focuses.all.iter().find(|f| f.in_dialog && !f.itself);
        match first {
            Some(first) => assert_eq!(focused[0], *first, "{name}"),
            None => assert!(focused[0].itself, "{name}: {focused:?}"),
        }
    }
}

/// Esc closes each dialog (from its layer, or the guide's key listener)
/// and the page field that had the focus has it again: typing reaches it.
#[test]
fn esc_closes_the_dialog_and_gives_the_page_field_its_focus_back() {
    for (name, nav, open) in each_case() {
        let (_dir, mut app, mut ui, field) = settings_field_in(nav);
        let page_field = ui.focuses(&app).all[field];
        open(&mut app);
        ui.redraw(&app);
        let esc = ui.send(&app, named(Named::Escape));
        let mut messages = esc.messages;
        if messages.is_empty() {
            // The guide's Esc goes on to the app's key listener.
            assert!(!esc.captured, "{name}: Esc kept but nothing sent");
            let status = iced::event::Status::Ignored;
            let key = Key::Named(Named::Escape);
            messages.extend(crate::app::guide_shortcut(
                status,
                app.setup.open,
                false,
                &key,
            ));
        }
        assert!(!messages.is_empty(), "{name}: Esc does nothing");
        for message in messages {
            let _ = app.update(message);
        }
        ui.redraw(&app);
        let focuses = ui.focuses(&app);
        assert!(!focuses.dialog, "{name}: Esc closed the dialog");
        assert_eq!(
            focuses.focused(),
            vec![page_field],
            "{name}: the page field has the focus again"
        );
        assert!(
            types_into_settings(&ui.send(&app, type_x())),
            "{name}: typing reaches the page field again"
        );
    }
}

/// The production chrome with a dialog of three fields open over it.
fn three_fields(app: &TrackerApp) -> Element<'_, Message> {
    use iced::widget::text_input;
    let fields = iced::widget::column![
        text_input("first", ""),
        text_input("second", ""),
        text_input("third", ""),
    ];
    with_layers(app, [Some(Dialog::new(fields, None))])
}

/// Tab and Shift+Tab cycle round the dialog's own fields, round from the
/// last to the first and back, and never land on the page field.
#[test]
fn tab_cycles_round_the_dialog_fields_only() {
    let (_dir, app, mut ui, _) = settings_with_a_focused_field();
    ui.view = three_fields;
    ui.redraw(&app);
    let fields: Vec<Rectangle> = ui.focuses(&app).all.iter().map(|f| f.bounds).collect();
    assert_eq!(fields.len(), 3, "only the dialog's fields are seen");
    let focused = |ui: &mut Ui| {
        let focused = ui.focuses(&app).focused();
        assert_eq!(focused.len(), 1, "{focused:?}");
        assert!(focused[0].in_dialog && !focused[0].itself);
        fields.iter().position(|b| *b == focused[0].bounds)
    };
    assert_eq!(focused(&mut ui), Some(0), "the first field on open");
    for expected in [1, 2, 0, 1] {
        assert!(ui.send(&app, named(Named::Tab)).captured);
        assert_eq!(focused(&mut ui), Some(expected));
    }
    for expected in [0, 2, 1] {
        assert!(ui.send(&app, shift_tab()).captured);
        assert_eq!(focused(&mut ui), Some(expected));
    }
}

/// The covered page really lets go of the focus, not only of the keys: its
/// field stops asking for the input method (the caret and the IME) while
/// a dialog with no field of its own is open, and asks again after.
#[test]
fn the_covered_page_lets_go_of_the_focus() {
    for nav in LAYOUTS {
        let (_dir, mut app, mut ui, _) = settings_field_in(nav);
        assert!(ui.redraw(&app).typing, "{nav:?}: the page field is focused");
        let _ = app.update(Message::OpenAbout);
        ui.redraw(&app);
        assert!(!ui.redraw(&app).typing, "{nav:?}: the page let go");
        let _ = app.update(Message::DismissNotes);
        ui.redraw(&app);
        assert!(ui.redraw(&app).typing, "{nav:?}: the page has it back");
    }
}

/// One focused widget, in the top dialog: whether it is the dialog itself.
fn dialog_focus(ui: &mut Ui, app: &TrackerApp) -> bool {
    let focused = ui.focuses(app).focused();
    assert_eq!(focused.len(), 1, "{focused:?}");
    assert!(focused[0].in_dialog, "{focused:?}");
    focused[0].itself
}

/// A dialog over a dialog: the lower one gets no keys, and when the top
/// one closes on Esc the lower one has its focus back; when that closes,
/// the page field has its focus back.
#[test]
fn stacked_dialogs_hand_the_focus_back_down() {
    let (_dir, mut app, mut ui, field) = settings_with_a_focused_field();
    let page_field = ui.focuses(&app).all[field];
    let _ = app.update(Message::OpenAbout);
    ui.redraw(&app);
    assert!(dialog_focus(&mut ui, &app), "What's new has itself");
    let open = crate::report::ReportMessage::Open(None);
    let _ = app.update(Message::Report(open));
    ui.redraw(&app);
    assert!(!dialog_focus(&mut ui, &app), "the report field has it");
    let esc = ui.send(&app, named(Named::Escape));
    let close = Message::Report(crate::report::ReportMessage::Close);
    assert_eq!(format!("{:?}", esc.messages), format!("{:?}", [close]));
    for message in esc.messages {
        let _ = app.update(message);
    }
    ui.redraw(&app);
    assert!(dialog_focus(&mut ui, &app), "What's new has its focus back");
    let esc = ui.send(&app, named(Named::Escape));
    assert_eq!(format!("{:?}", esc.messages), "[DismissNotes]");
    let _ = app.update(Message::DismissNotes);
    ui.redraw(&app);
    assert_eq!(ui.focuses(&app).focused(), vec![page_field]);
}

/// The crash prompt has nothing to focus; Yes turns it into the report
/// form in the same layer, and the form's field then gets the focus.
#[test]
fn the_report_form_takes_the_focus_from_the_crash_prompt() {
    let (_dir, mut app, mut ui, _) = settings_with_a_focused_field();
    use crate::report::{ReportDialog, ReportUi};
    app.report = Some(ReportUi::CrashPrompt { last_seen: None });
    ui.redraw(&app);
    assert!(dialog_focus(&mut ui, &app), "the prompt has itself");
    // What Yes does (`ReportMessage::CrashYes`), without its clipboard and
    // log side effects, which need a desktop session.
    let crash = Some(stat_tracker::report::LogOnlyCategory::Crash);
    app.report = Some(ReportUi::Dialog(Box::new(ReportDialog::new(crash, false))));
    ui.redraw(&app);
    assert!(!dialog_focus(&mut ui, &app), "the form's field has it");
    let typed = ui.send(&app, type_x());
    let debug = format!("{:?}", typed.messages);
    assert!(debug.contains("Report(Text("), "{debug}");
}

/// The production chrome with a guide-like dialog over it: a field and
/// no Esc message of its own.
fn field_without_escape(app: &TrackerApp) -> Element<'_, Message> {
    let field = iced::widget::text_input("a field", "").on_input(|_| Message::Tick);
    with_layers(app, [Some(Dialog::new(field, None))])
}

/// A dialog that leaves Esc to the app (the setup guide) gives it to its
/// focused field first, as before: the field takes it and lets go of the
/// focus, so the guide's key listener (which skips a captured key) does
/// not close the guide from inside a field.
#[test]
fn esc_in_a_field_of_the_guide_leaves_the_field_first() {
    let (_dir, app, mut ui, _) = settings_with_a_focused_field();
    ui.view = field_without_escape;
    ui.redraw(&app);
    assert!(!dialog_focus(&mut ui, &app), "the field has the focus");
    let esc = ui.send(&app, named(Named::Escape));
    assert!(esc.messages.is_empty(), "{:?}", esc.messages);
    assert!(esc.captured, "the field took Esc");
    let key = Key::Named(Named::Escape);
    let status = iced::event::Status::Captured;
    assert!(crate::app::guide_shortcut(status, true, false, &key).is_none());
    let esc = ui.send(&app, named(Named::Escape));
    assert!(
        !esc.captured,
        "a second Esc goes on to the guide's listener"
    );
}

/// A counted page of fields, with a dialog over it when [`DIALOG_OPEN`]
/// is set. The layers are the same [`dialog_focus::layer`] widgets
/// [`with_layers`] uses; the page is only the fields, so a dropped
/// field is the only change in the focusable count.
fn page_of_fields(_app: &TrackerApp) -> Element<'_, Message> {
    use iced::widget::{container, space, text, text_input};
    let ids = PAGE_IDS.with(|c| c.borrow().clone());
    let mut fields = iced::widget::Column::new();
    for id in &ids {
        fields = fields.push(text_input(id.as_str(), "").id(Id::from(id.clone())));
    }
    let page = container(fields).width(Fill).height(Fill);
    let dialog = DIALOG_OPEN
        .get()
        .then(|| Dialog::new(text("a dialog"), Some(Message::DismissToast)));
    let role = if dialog.is_some() {
        Role::Covered
    } else {
        Role::Live
    };
    let page = dialog_focus::layer(page, role, None);
    let top = match dialog {
        Some(dialog) => dialog_focus::layer(dialog.content, Role::Top, dialog.on_escape),
        None => space().into(),
    };
    Stack::with_children([page, top])
        .width(Fill)
        .height(Fill)
        .into()
}

thread_local! {
    static PAGE_IDS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static DIALOG_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn set_page(ids: &[&str], dialog: bool) {
    PAGE_IDS.with(|c| {
        *c.borrow_mut() = ids.iter().map(|s| (*s).to_string()).collect();
    });
    DIALOG_OPEN.set(dialog);
}

/// If the focused field is still on the page, Esc gives it back even when
/// another field was dropped (the same widget, by id). If the focused
/// field itself is gone, or the page swaps in a different field at the
/// same count, Esc must not move the focus to a different field.
#[test]
fn esc_does_not_focus_the_wrong_field_when_the_page_changes() {
    let (_dir, app) = crate::fixtures::test_app();
    let mut ui = Ui::new();
    ui.view = page_of_fields;

    // Three fields, the middle one focused. Drop the first: the middle
    // field is still there, at a new index, and gets the focus back.
    set_page(&["field 0", "field 1", "field 2"], false);
    ui.redraw(&app);
    ui.operate(&app, &mut FocusNth { n: 1, current: 0 });
    ui.redraw(&app);
    assert_eq!(ui.focuses(&app).focused().len(), 1);
    set_page(&["field 0", "field 1", "field 2"], true);
    ui.redraw(&app);
    set_page(&["field 1", "field 2"], true);
    ui.redraw(&app);
    set_page(&["field 1", "field 2"], false);
    ui.redraw(&app);
    let focuses = ui.focuses(&app);
    let focused = focuses.focused();
    // field 1 is now the first of the two remaining fields. A restore by
    // the old index would have given up (the count changed).
    assert_eq!(
        focused.len(),
        1,
        "the same field should have the focus back: {focused:?}"
    );
    assert_eq!(focused[0].bounds, focuses.all[0].bounds);

    // The opener itself is removed (the last of three). Esc still must
    // not pick another field.
    set_page(&["field 0", "field 1", "field 2"], false);
    ui.redraw(&app);
    ui.operate(&app, &mut FocusNth { n: 2, current: 0 });
    ui.redraw(&app);
    assert_eq!(ui.focuses(&app).focused().len(), 1);
    set_page(&["field 0", "field 1", "field 2"], true);
    ui.redraw(&app);
    set_page(&["field 0", "field 1"], true);
    ui.redraw(&app);
    set_page(&["field 0", "field 1"], false);
    ui.redraw(&app);
    let focused = ui.focuses(&app).focused();
    assert!(
        focused.is_empty(),
        "a field has the focus after the opener was dropped: {focused:?}"
    );
}

/// Same number of fields, but a different widget in the focused slot:
/// restore is by id, so the new field must not get the focus.
#[test]
fn esc_does_not_focus_a_new_field_at_the_same_index() {
    let (_dir, app) = crate::fixtures::test_app();
    let mut ui = Ui::new();
    ui.view = page_of_fields;

    set_page(&["alpha", "beta"], false);
    ui.redraw(&app);
    ui.operate(&app, &mut FocusNth { n: 0, current: 0 });
    ui.redraw(&app);
    assert_eq!(ui.focuses(&app).focused().len(), 1);
    set_page(&["alpha", "beta"], true);
    ui.redraw(&app);
    set_page(&["gamma", "beta"], true);
    ui.redraw(&app);
    set_page(&["gamma", "beta"], false);
    ui.redraw(&app);
    let focused = ui.focuses(&app).focused();
    assert!(
        focused.is_empty(),
        "a new field at the old index got the focus: {focused:?}"
    );
}

struct UnfocusAll;

impl Operation for UnfocusAll {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(
        &mut self,
        _id: Option<&iced::widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        state.unfocus();
    }
}

/// A click on a page button blurs the field before the dialog opens. Esc
/// still returns focus to the field that was focused before that click.
#[test]
fn esc_returns_focus_to_the_field_blurred_by_the_opening_click() {
    for nav in LAYOUTS {
        let (_dir, mut app, mut ui, field) = settings_field_in(nav);
        let page_field = ui.focuses(&app).all[field];
        ui.operate(&app, &mut UnfocusAll);
        ui.redraw(&app);
        assert!(
            ui.focuses(&app).focused().is_empty(),
            "{nav:?}: the click has already blurred the field"
        );
        let open = crate::settings_open::OpenMsg::WhatLeaves(true);
        let _ = app.update(Message::SettingsPage(crate::settings_update::Msg::Open(
            open,
        )));
        ui.redraw(&app);
        let esc = ui.send(&app, named(Named::Escape));
        assert!(!esc.messages.is_empty(), "{nav:?}: Esc does nothing");
        for message in esc.messages {
            let _ = app.update(message);
        }
        ui.redraw(&app);
        let focused = ui.focuses(&app).focused();
        assert_eq!(
            focused,
            vec![page_field],
            "{nav:?}: the field focused before the click should have it back"
        );
        assert!(
            types_into_settings(&ui.send(&app, type_x())),
            "{nav:?}: typing should reach the field again"
        );
    }
}
