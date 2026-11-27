//! The pinned Settings bar (#776) in the window tree: the page pane keeps
//! its place, scroll and field focus when the bar and its leave confirm
//! come and go, and the bar sits under the pane and fits the window. Uses
//! the helpers of [`super`] (the #806 tree tests).

use super::*;

/// A live (not demo) app on the 0.6 Settings tab, taller than [`WINDOW`],
/// with nothing pending. Its page never writes the host's config.toml:
/// these tests save nothing, and the path points at the temp dir anyway.
fn settings_bar_app() -> (tempfile::TempDir, TrackerApp) {
    let (dir, mut app) = classic_app(Screen::Settings);
    app.nav_style = NavStyle::TopTabs;
    app.fixture = None;
    app.daemon.pid = None;
    app.overlay_hotkey.enabled = true;
    app.settings = app.form_from_saved();
    app.page_state = crate::settings_update::PageState::new(app.saved_config.clone(), None, true);
    app.page_state.config_path = Some(dir.path().join("config.toml"));
    assert!(!crate::settings_bar::shows(&app));
    (dir, app)
}

/// Typed text not saved yet: the bar shows.
fn show_settings_bar(app: &mut TrackerApp) {
    app.settings.cooldown_secs = "99".into();
    assert!(crate::settings_bar::shows(app));
}

fn hide_settings_bar(app: &mut TrackerApp) {
    app.settings = app.form_from_saved();
    assert!(!crate::settings_bar::shows(app));
}

/// `top_tabs` with the bar's slot only while the bar shows: the shape the
/// always-present slot avoids, kept to show the tests below can fail.
fn bar_only_when_shown(app: &TrackerApp) -> Element<'_, Message> {
    responsive(move |size| {
        let (window, prompts) = top_tabs_parts(app, size.width);
        let page = stack![window, prompts.unwrap_or_else(empty_layer)]
            .width(Fill)
            .height(Fill);
        if crate::settings_bar::shows(app) {
            iced::widget::column![page, crate::settings_bar::view(app, size.width)].into()
        } else {
            page.into()
        }
    })
    .width(Fill)
    .height(Fill)
    .into()
}

/// The cause the slot guards against: a bar that is only in the tree
/// while it shows moves the pane, and iced resets its scroll (#806).
#[test]
fn a_bar_that_comes_and_goes_in_the_tree_would_reset_the_scroll() {
    let (_dir, mut app) = settings_bar_app();
    let quiet = pane_path(&app, bar_only_when_shown);
    show_settings_bar(&mut app);
    let shown = pane_path(&app, bar_only_when_shown);
    assert_ne!(quiet, shown);
    hide_settings_bar(&mut app);
    let kept = offset_after(
        &mut app,
        bar_only_when_shown,
        bar_only_when_shown,
        show_settings_bar,
    );
    assert_eq!(kept, 0.0);
}

/// The bar's slot is always in the tree, so the pane keeps its place and
/// its scroll when the bar shows and when it hides again, in the whole
/// window with its layers too.
#[test]
fn the_settings_bar_keeps_the_pane_and_its_scroll() {
    for view in [current_top_tabs as View, whole_window as View] {
        let (_dir, mut app) = settings_bar_app();
        let quiet = pane_path(&app, view);
        show_settings_bar(&mut app);
        let shown = pane_path(&app, view);
        assert_eq!(
            quiet,
            shown,
            "{:?} vs {:?}",
            readable(&quiet),
            readable(&shown)
        );
        hide_settings_bar(&mut app);
        let kept = offset_after(&mut app, view, view, show_settings_bar);
        assert!(
            (kept - SCROLL_Y).abs() < 0.5,
            "the bar moved the scroll to {kept}"
        );
        let kept = offset_after(&mut app, view, view, hide_settings_bar);
        assert!(
            (kept - SCROLL_Y).abs() < 0.5,
            "hiding the bar moved the scroll to {kept}"
        );
        // The leave confirm is a layer that is always there too.
        show_settings_bar(&mut app);
        let kept = offset_after(&mut app, view, view, |app| {
            let _ = app.update(Message::Navigate(Screen::Games));
            assert!(app.page_state.bar.leave.is_some());
        });
        assert!(
            (kept - SCROLL_Y).abs() < 0.5,
            "the leave confirm moved the scroll to {kept}"
        );
    }
}

/// The ids of the focused widgets.
#[derive(Default)]
struct Focused(Vec<Option<Id>>);

impl Operation<()> for Focused {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        operate(self);
    }

    fn focusable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        state: &mut dyn operation::Focusable,
    ) {
        if state.is_focused() {
            self.0.push(id.cloned());
        }
    }
}

/// The focused widgets after typing into the focused shortcut field,
/// which shows the bar, with `view` before and after (spec 2.8: the bar
/// does not take the keyboard from a field the player types in).
fn focus_after_typing(view: View, next: View) -> Vec<Option<Id>> {
    use crate::settings::SettingsField;
    let (_dir, mut app) = settings_bar_app();
    let renderer = headless();
    let size = Size::new(1280.0, 860.0);
    let field = crate::settings_rows::field_id(SettingsField::OverlayHotkey);
    let mut tree;
    {
        let mut element = view(&app);
        tree = Tree::new(&element);
        let node = lay_out(&mut element, &mut tree, &renderer, size);
        let mut focus = operation::focusable::focus(field.clone());
        run(&mut element, &mut tree, &node, &renderer, &mut focus);
        let mut seen = Focused::default();
        run(&mut element, &mut tree, &node, &renderer, &mut seen);
        assert_eq!(seen.0, vec![Some(field.clone())], "the field took focus");
    }
    let _ = app.update(Message::SettingsText(
        SettingsField::OverlayHotkey,
        "Super+Shift+K".into(),
    ));
    assert!(crate::settings_bar::shows(&app));
    let mut element = next(&app);
    tree.diff(&element);
    let node = lay_out(&mut element, &mut tree, &renderer, size);
    let mut seen = Focused::default();
    run(&mut element, &mut tree, &node, &renderer, &mut seen);
    seen.0
}

#[test]
fn a_typed_field_keeps_the_keyboard_when_the_bar_shows() {
    use crate::settings::SettingsField;
    let field = crate::settings_rows::field_id(SettingsField::OverlayHotkey);
    assert_eq!(
        focus_after_typing(whole_window, whole_window),
        vec![Some(field)]
    );
    assert!(
        focus_after_typing(bar_only_when_shown, bar_only_when_shown).is_empty(),
        "the test can tell: a bar that comes and goes drops the focus"
    );
}

/// Every text in the laid-out tree, with its bounds.
#[derive(Default)]
struct Texts(Vec<(String, Rectangle)>);

impl Operation<()> for Texts {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        operate(self);
    }

    fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
        self.0.push((text.to_string(), bounds));
    }
}

/// The longest bar: three typed groups and a waiting change on a stopped
/// tracker (Discard, Save, Save and start), with a refused save's line.
fn longest_bar(app: &mut TrackerApp) -> String {
    app.settings.cooldown_secs = "soon".into();
    app.settings.sync_url = "http://crew.example".into();
    app.settings.overlay_hotkey = "Super+Shift+K".into();
    app.saved_config.debug_ocr = !app.saved_config.debug_ocr;
    app.settings.debug_ocr = app.saved_config.debug_ocr;
    crate::settings_bar::show_error(
        app,
        "Check every must be a whole number of seconds, 1 or more. Nothing was changed.",
    );
    let words = crate::settings_bar::words(3, 1);
    assert_eq!(words, "3 unsaved changes, 1 waiting for restart");
    words
}

/// The bar sits under the page pane, never over it, and fits the window:
/// at 720x480 (the window minimum), 1000x768 and 1280x860, the words stay
/// on one line, each button label is one line inside the window, and the
/// pane keeps most of its height. The edge of [`ONE_LINE_MIN`] is checked
/// on the bar alone.
#[test]
fn the_bar_sits_under_the_pane_and_fits_the_window() {
    use crate::settings_bar::{DISCARD, ONE_LINE_MIN, SAVE, SAVE_AND_START};
    let renderer = headless();
    let line = theme::SIZE_BODY * 1.6;
    for size in [
        Size::new(720.0, 480.0),
        Size::new(1000.0, 768.0),
        Size::new(1280.0, 860.0),
    ] {
        let (_dir, mut app) = settings_bar_app();
        let words = longest_bar(&mut app);
        let mut element = whole_window(&app);
        let mut tree = Tree::new(&element);
        let node = lay_out(&mut element, &mut tree, &renderer, size);
        let pane = find(&mut element, &mut tree, &node, &renderer, Target::Pane).expect("the pane");
        let mut texts = Texts::default();
        run(&mut element, &mut tree, &node, &renderer, &mut texts);
        let at = |needle: &str| {
            texts
                .0
                .iter()
                .rev()
                .find(|(t, _)| t == needle)
                .map(|(_, b)| *b)
                .unwrap_or_else(|| panic!("{needle:?} at {size:?}"))
        };
        let copy = at(&words);
        let pane_bottom = pane.bounds.y + pane.bounds.height;
        assert!(
            copy.y >= pane_bottom - 0.5,
            "{size:?}: the bar's words at {} overlap the pane ending {pane_bottom}",
            copy.y
        );
        assert!(copy.height < line, "{size:?}: the words wrap: {copy:?}");
        for label in [DISCARD, SAVE, SAVE_AND_START] {
            let b = at(label);
            assert!(b.height < line, "{size:?}: {label} wraps: {b:?}");
            assert!(
                b.x + b.width <= size.width + 0.5 && b.y + b.height <= size.height + 0.5,
                "{size:?}: {label} is outside the window: {b:?}"
            );
        }
        assert!(
            pane.bounds.height > size.height * 0.4,
            "{size:?}: the bar leaves the pane only {}",
            pane.bounds.height
        );
    }
    // The narrowest one-line bar: the same checks on the bar alone.
    let (_dir, mut app) = settings_bar_app();
    let words = longest_bar(&mut app);
    let width = ONE_LINE_MIN + 2.0 * theme::PAGE_PAD_X_NARROW;
    let mut bar = crate::settings_bar::view(&app, width);
    let mut tree = Tree::new(&bar);
    let node = lay_out(&mut bar, &mut tree, &renderer, Size::new(width, 400.0));
    let mut texts = Texts::default();
    run(&mut bar, &mut tree, &node, &renderer, &mut texts);
    let copy = texts.0.iter().find(|(t, _)| *t == words).expect("words").1;
    let primary = texts
        .0
        .iter()
        .find(|(t, _)| t == SAVE_AND_START)
        .expect("primary")
        .1;
    assert!(copy.height < line, "the words wrap at the edge: {copy:?}");
    assert!(
        primary.x + primary.width <= width + 0.5,
        "the primary runs past {width}: {primary:?}"
    );
    assert!(
        (primary.y - copy.y).abs() < line,
        "one line at the edge: words {copy:?}, primary {primary:?}"
    );
}
