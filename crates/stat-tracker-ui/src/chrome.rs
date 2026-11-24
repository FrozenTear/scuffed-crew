//! The window chrome around the screens: the 0.6 top tabs or the 0.5
//! sidebar ([`crate::nav::NavStyle`]). Both wrap the same [`page_body`],
//! so the page padding and the scrolling are drawn one way for both
//! layouts. Both float the toast and the restart bar over the bottom of
//! the page ([`floating_prompts`]), where the spec puts its one toast (p4
//! "Report sent"), in a layer that is always there: the page under it
//! never moves or remounts when a toast or the bar comes or goes.
//!
//! The dialogs stack over the window, each in a [`crate::dialog_focus`]
//! layer that keeps the keyboard in the top one.

use iced::widget::{Column, Stack, column, container, responsive, row, space, stack};
use iced::{Element, Fill, Length, Padding};

use crate::app::{Message, TrackerApp};
use crate::dialog_focus::{self, Dialog, Role};
use crate::model::Screen;
use crate::nav::NavStyle;
use crate::theme::{self, PAGE_PAD_X, PAGE_PAD_Y, SIDEBAR_WIDTH};
use crate::{restart_prompt, widgets};

#[path = "prompt_clearance.rs"]
mod prompt_clearance;
use prompt_clearance::clearance;

/// The whole window ([`TrackerApp::view`]): the chrome, the status pill's
/// menu and confirm, What's new, the setup guide, Uninstall and Report a
/// problem, in that order from the bottom ([`with_layers`] puts What leaves
/// your PC on top). Each dialog closes on Esc from its layer, except the
/// guide: the app listens for its keys. The pill's menu and confirm keep
/// their place as empty layers when they are closed.
pub(crate) fn window(app: &TrackerApp) -> Element<'_, Message> {
    let notes = app
        .notes_layer()
        .map(|notes| Dialog::new(notes, Some(Message::DismissNotes)));
    let guide = app
        .setup
        .open
        .then(|| Dialog::new(crate::setup_guide::view(&app.setup), None));
    let uninstall = app.uninstall_dialog.as_ref().map(|dialog| {
        let view = crate::uninstall::view(dialog, app.uninstall_busy);
        Dialog::new(view, Some(Message::DismissUninstall))
    });
    let report = app.report.as_ref().map(|ui| {
        let close = Message::Report(crate::report::ReportMessage::Close);
        Dialog::new(crate::report::view(ui), Some(close))
    });
    with_layers(app, [notes, guide, uninstall, report])
}

/// A dialog of [`window`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopDialog {
    Notes,
    Guide,
    Uninstall,
    Report,
    WhatLeaves,
}

/// The open dialog on top of the window, the one with the keyboard, in the
/// order [`window`] stacks them; `None` with no dialog open. The pill's
/// menu and confirm sit under the dialogs and have their own key listener.
pub(crate) fn top_dialog(app: &TrackerApp) -> Option<TopDialog> {
    [
        (
            app.page_state.opened.what_leaves_open,
            TopDialog::WhatLeaves,
        ),
        (app.report.is_some(), TopDialog::Report),
        (app.uninstall_dialog.is_some(), TopDialog::Uninstall),
        (app.setup.open, TopDialog::Guide),
        (app.notes_dialog.is_some(), TopDialog::Notes),
    ]
    .into_iter()
    .find_map(|(open, dialog)| open.then_some(dialog))
}

/// The chrome for the layout picked at start on the page background, then
/// the status pill's menu and confirm, then each open dialog in `layers`.
/// A closed menu, confirm or dialog keeps its place as an empty layer
/// ([`empty_layer`]), and the page and every open dialog sit in a
/// [`dialog_focus`] layer whatever its role, so the page keeps its scroll
/// when a menu, confirm or dialog opens or closes. The top open dialog has
/// the keyboard; the page and the dialogs under it get none. The pill's
/// menu and confirm are ordinary layers: they have their own key listener
/// and do not cover the page the way a dialog does.
pub(crate) fn with_layers<'a, const N: usize>(
    app: &'a TrackerApp,
    layers: [Option<Dialog<'a>>; N],
) -> Element<'a, Message> {
    let chrome = match app.nav_style {
        NavStyle::TopTabs => top_tabs(app),
        NavStyle::ClassicSidebar => sidebar(app),
    };
    let page = container(chrome)
        .width(Fill)
        .height(Fill)
        .style(theme::page_background);
    let pill = [
        crate::status_pill::menu_layer(app),
        crate::status_pill::confirm_layer(app),
    ];
    let layers: Vec<_> = layers
        .into_iter()
        .chain([crate::what_leaves::layer(app)])
        .collect();
    let top = layers.iter().rposition(Option::is_some);
    let covered = |covered: bool| if covered { Role::Covered } else { Role::Live };
    let page = dialog_focus::layer(page, covered(top.is_some()), None);
    let pill = pill
        .into_iter()
        .map(|layer| layer.unwrap_or_else(empty_layer));
    let layers = layers
        .into_iter()
        .enumerate()
        .map(|(i, layer)| match layer {
            Some(dialog) if top == Some(i) => {
                dialog_focus::layer(dialog.content, Role::Top, dialog.on_escape)
            }
            Some(dialog) => dialog_focus::layer(dialog.content, Role::Covered, dialog.on_escape),
            None => empty_layer(),
        });
    Stack::with_children(std::iter::once(page).chain(pill).chain(layers))
        .width(Fill)
        .height(Fill)
        .into()
}

/// 0.5 layout: sidebar on the left, header and old Settings cards. The
/// toast and the restart bar float over the pane, as in the top tabs.
fn sidebar(app: &TrackerApp) -> Element<'_, Message> {
    // Sidebar is a fixed width. The remaining pane flexes with the window
    // (rev 3). Header chips live in that pane. Column counts come from
    // `responsive`. Settings keeps the header and scrolls the form in the
    // leftover height. Other screens scroll the whole pane.
    row![
        container(widgets::sidebar(app.screen))
            .width(SIDEBAR_WIDTH)
            .height(Fill)
            .padding(Padding {
                top: PAGE_PAD_Y,
                bottom: PAGE_PAD_Y,
                left: PAGE_PAD_X,
                right: 8.0,
            }),
        responsive(move |size| {
            let (pane, prompts) = sidebar_pane_parts(app, size.width);
            stack![pane, prompts.unwrap_or_else(empty_layer)]
                .width(Fill)
                .height(Fill)
                .into()
        })
        .width(Fill)
        .height(Fill),
    ]
    .spacing(0)
    .width(Fill)
    .height(Fill)
    .into()
}

/// The classic pane right of the sidebar for a `width` px pane, and the
/// floating toast and restart bar over it, if any. The prompts layer is
/// always there ([`top_tabs`] says why).
fn sidebar_pane_parts(
    app: &TrackerApp,
    width: f32,
) -> (Element<'_, Message>, Option<Element<'_, Message>>) {
    let content = with_vision_notice(app, column![widgets::app_header(app)]);
    let page = width - 2.0 * PAGE_PAD_X;
    let pane = page_body(app, content, crate::settings::view, PAGE_PAD_X, true, page);
    (pane, floating_prompts(app, page))
}

/// 0.6 layout: tabs across the top, then the page title, then on the stats
/// screens the season, role and Companion row, then the screen. The bar and
/// the page share one gutter ([`theme::page_gutter`]).
///
/// The control row is the 0.5 header row, kept only on pages that do not
/// draw the spec's filter chips yet; the redesigned pages draw the chips
/// themselves (Season, queues, roles, Crew, Hero, Map; "Filters (n)"
/// below a 1440 px window).
///
/// The prompts layer is always there, empty when there is no toast and no
/// restart bar ([`empty_layer`]), so the widget tree keeps its shape when a
/// toast comes and goes. iced matches widget state by position in the
/// tree: with the layer only there while a toast shows, the page scroll
/// got a fresh state and jumped to the top.
fn top_tabs(app: &TrackerApp) -> Element<'_, Message> {
    responsive(move |size| {
        let (window, prompts) = top_tabs_parts(app, size.width);
        stack![window, prompts.unwrap_or_else(empty_layer)]
            .width(Fill)
            .height(Fill)
            .into()
    })
    .width(Fill)
    .height(Fill)
    .into()
}

/// The layer in place of no prompts or a closed dialog: a zero-size
/// [`space`] that draws nothing and takes no events, so clicks reach the
/// page under it, as they do in the space around a real toast.
fn empty_layer<'a>() -> Element<'a, Message> {
    space().into()
}

/// The top tabs window for a `width` px window, and the floating toast and
/// restart bar over it, if any.
fn top_tabs_parts(
    app: &TrackerApp,
    width: f32,
) -> (Column<'_, Message>, Option<Element<'_, Message>>) {
    let mut content = column![crate::nav::page_heading(app.screen)];
    // No Tracker health panel here: without a keyboard Tab is never
    // seen, so the reason and its fix sit under the title on every tab.
    if let Some(notice) = top_tabs_input_notice(app) {
        content = content.push(widgets::warn_line(notice));
    }
    content = with_vision_notice(app, content);
    // Settings has none; Overview, Games, Heroes, Maps and Seasons draw
    // the spec's filter chips themselves.
    if !matches!(
        app.screen,
        Screen::Settings
            | Screen::Overview
            | Screen::Games
            | Screen::Heroes
            | Screen::Maps
            | Screen::Seasons
    ) {
        content = content.push(widgets::header_controls(app));
    }
    let gutter = theme::page_gutter(width);
    let window = column![
        crate::nav::top_bar(app, width),
        page_body(
            app,
            content,
            crate::settings_page::view,
            gutter,
            false,
            width - 2.0 * gutter,
        ),
    ]
    .spacing(0)
    .width(Fill)
    .height(Fill);
    (window, floating_prompts(app, width - 2.0 * gutter))
}

/// The keyboard notice the top tabs layout draws under the page title.
/// The classic layout has it in Tracker health instead.
fn top_tabs_input_notice(app: &TrackerApp) -> Option<&str> {
    (app.nav_style == NavStyle::TopTabs)
        .then_some(app.ocr_switch.input_notice.as_deref())
        .flatten()
}

/// The capture size notice ([`crate::vision_size_notice`]) under the
/// Overview title, in both layouts. Never on Games or in a game row:
/// [`crate::vision_size_notice::view`] draws nothing off Overview.
fn with_vision_notice<'a>(
    app: &'a TrackerApp,
    content: Column<'a, Message>,
) -> Column<'a, Message> {
    match crate::vision_size_notice::view(app) {
        Some(notice) => content.push(notice),
        None => content,
    }
}

/// Widest the floating toast and restart bar get.
const FLOAT_MAX: f32 = 640.0;
/// Space between them and the window's bottom edge.
const FLOAT_GAP: f32 = 16.0;
/// Space between a toast and the restart bar when both show.
const PROMPTS_SPACING: f32 = 8.0;
/// One toast or restart bar: 1 px border, [8, 14] pad, and a line as tall
/// as a meta-size button with [4, 10] pad, with room for the font's line
/// box. A test lays out both bars and checks [`PROMPTS_INSET`] covers them.
const PROMPT_BAR: f32 = 48.0;
/// Floor of the reserved bottom padding in scrolled content: [`FLOAT_GAP`]
/// plus two one-line bars and the space between them. Always kept, even
/// when no prompt is showing, so a short toast does not reflow the page.
/// When the visible prompts (a wrapped toast, the restart bar, and the
/// gap) lay out taller than this, the padding grows to that height.
pub(crate) const PROMPTS_INSET: f32 = FLOAT_GAP + PROMPT_BAR + PROMPTS_SPACING + PROMPT_BAR;

/// The toast and the restart bar, centred at the bottom of the window (of
/// the pane in the classic layout), at most [`FLOAT_MAX`] wide and never
/// wider than the page (`page`). Only they take clicks: the space around
/// them passes them through.
fn prompt_column<'a>(
    toast: Option<&'a str>,
    action: Option<Message>,
    restart: bool,
    page: f32,
) -> Option<Element<'a, Message>> {
    if toast.is_none() && !restart {
        return None;
    }
    let mut prompts = Column::new().spacing(PROMPTS_SPACING);
    if let Some(t) = toast {
        prompts = prompts.push(widgets::toast_bar(t, action));
    }
    if restart {
        prompts = prompts.push(restart_prompt::bar());
    }
    Some(
        container(prompts)
            .max_width(FLOAT_MAX.min(page.max(0.0)))
            .into(),
    )
}

/// The visible prompts plus the bottom [`FLOAT_GAP`], for measuring the
/// reserved slot. An empty slot when nothing is showing, so the widget
/// tree under [`clearance`] keeps its shape.
fn prompts_measure_slot<'a>(
    toast: Option<&'a str>,
    action: Option<Message>,
    restart: bool,
    page: f32,
) -> Element<'a, Message> {
    match prompt_column(toast, action, restart, page) {
        Some(col) => container(col)
            .padding(Padding {
                bottom: FLOAT_GAP,
                ..Padding::ZERO
            })
            .into(),
        None => space().into(),
    }
}

fn floating_prompts(app: &TrackerApp, page: f32) -> Option<Element<'_, Message>> {
    let restart = app.restart_bar_visible();
    let col = prompt_column(
        app.toast.as_deref(),
        app.toast_action.clone(),
        restart,
        page,
    )?;
    Some(
        container(col)
            .padding(FLOAT_GAP)
            .width(Fill)
            .height(Fill)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Bottom)
            .into(),
    )
}

/// The pane right of the sidebar or under the tabs: `content` (what the
/// layout puts first: the header, or the title and controls), then the
/// screen, padded `pad_x` at the sides and, at the bottom, at least
/// [`PROMPTS_INSET`] (or the laid-out height of the visible prompts when
/// a toast wraps taller) so the last control can scroll clear of the
/// floating toast and restart bar ([`floating_prompts`]). The floor is
/// always there, so a short toast does not reflow the page. The prompts
/// themselves are never in the scrolled content; they float over it.
/// The classic Settings (`settings_scrolls`) fills the height and scrolls
/// its own form; every other screen, the 0.6 Settings page included,
/// scrolls the whole pane, so the scrollbar sits in one place on every tab.
fn page_body<'a>(
    app: &'a TrackerApp,
    content: Column<'a, Message>,
    settings: fn(&'a TrackerApp, f32) -> Element<'a, Message>,
    pad_x: f32,
    settings_scrolls: bool,
    page: f32,
) -> Element<'a, Message> {
    let settings_screen = settings_scrolls && app.screen == Screen::Settings;
    let height = if settings_screen {
        Fill
    } else {
        Length::Shrink
    };
    let content = content.spacing(16).width(Fill).height(height).push(
        responsive(move |size| match app.screen {
            Screen::Overview => crate::overview::view(app, size.width),
            Screen::Games => crate::games::view(app, size.width, size.width + 2.0 * pad_x),
            Screen::Heroes => crate::heroes::view(app, size.width),
            // The 0.6 pages under the top tabs, the 0.5 cards in the sidebar.
            Screen::Maps if app.nav_style == NavStyle::TopTabs => {
                crate::maps_tab::view(app, size.width)
            }
            Screen::Maps => crate::maps::view(app, size.width),
            Screen::Seasons if app.nav_style == NavStyle::TopTabs => {
                crate::seasons_tab::view(app, size.width)
            }
            Screen::Seasons => crate::seasons::view(app, size.width),
            Screen::Settings => settings(app, size.width),
        })
        .width(Fill)
        .height(height),
    );

    let main = container(content)
        .width(Fill)
        .height(height)
        .padding(Padding {
            top: PAGE_PAD_Y,
            bottom: 0.0,
            left: pad_x,
            right: pad_x,
        });
    let main = clearance(
        main,
        prompts_measure_slot(
            app.toast.as_deref(),
            app.toast_action.clone(),
            app.restart_bar_visible(),
            page,
        ),
    );
    if settings_screen {
        container(main).width(Fill).height(Fill).into()
    } else {
        iced::widget::scrollable(main)
            .id(crate::focus_status::PANE_SCROLL_ID)
            .width(Fill)
            .height(Fill)
            .into()
    }
}

/// A toast and the restart bar, stacked as [`floating_prompts`] stacks
/// them, with only the bottom [`FLOAT_GAP`]. Used to measure the slot.
#[cfg(test)]
pub(crate) fn prompts_slot(toast: &'static str, page: f32) -> Element<'static, Message> {
    prompts_measure_slot(Some(toast), None, true, page)
}

/// A one-line toast and the restart bar, for checking the [`PROMPTS_INSET`]
/// floor.
#[cfg(test)]
pub(crate) fn both_prompts_slot() -> Element<'static, Message> {
    prompts_slot("Report sent", 640.0)
}

#[cfg(test)]
mod tests {
    use super::top_tabs_input_notice;
    use crate::nav::NavStyle;

    /// The default layout has no Tracker health panel, so it must draw the
    /// keyboard notice itself; the classic layout keeps it in the panel.
    #[test]
    fn the_top_tabs_layout_shows_the_keyboard_notice() {
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::fixtures::test_cli(dir.path());
        let mut app = crate::app::TrackerApp::new(cli).0;
        app.nav_style = NavStyle::TopTabs;
        assert_eq!(top_tabs_input_notice(&app), None);
        let notice = "Tab detection is off. Run: sudo usermod -aG input \"$USER\"";
        app.ocr_switch.input_notice = Some(notice.into());
        assert_eq!(top_tabs_input_notice(&app), Some(notice));
        app.nav_style = NavStyle::ClassicSidebar;
        assert_eq!(top_tabs_input_notice(&app), None);
    }
}

#[cfg(test)]
#[path = "chrome_tree_tests.rs"]
mod tree_tests;

#[cfg(test)]
#[path = "chrome_focus_tests.rs"]
mod focus_tests;
