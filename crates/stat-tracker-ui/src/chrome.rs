//! The window chrome around the screens: the 0.6 top tabs or the 0.5
//! sidebar ([`crate::nav::NavStyle`]). Both wrap the same [`page_body`],
//! so the page padding and the scrolling are drawn one way for both
//! layouts. The sidebar puts the toast and the restart bar above the
//! screen; the top tabs float them at the bottom of the window, where the
//! spec puts its one toast (p4 "Report sent"), so nothing sits between the
//! bar and the page heading.

use iced::widget::{Column, column, container, responsive, row, stack};
use iced::{Element, Fill, Length, Padding};

use crate::app::{Message, TrackerApp};
use crate::model::Screen;
use crate::nav::NavStyle;
use crate::theme::{self, PAGE_PAD_X, PAGE_PAD_Y, SIDEBAR_WIDTH};
use crate::{restart_prompt, widgets};

/// The whole window: the chrome for the layout picked at start on the page
/// background, with each open dialog in `layers` stacked over it in order.
pub(crate) fn with_layers<'a, const N: usize>(
    app: &'a TrackerApp,
    layers: [Option<Element<'a, Message>>; N],
) -> Element<'a, Message> {
    let chrome = match app.nav_style {
        NavStyle::TopTabs => top_tabs(app),
        NavStyle::ClassicSidebar => sidebar(app),
    };
    let page = container(chrome)
        .width(Fill)
        .height(Fill)
        .style(theme::page_background);
    layers
        .into_iter()
        .chain([crate::what_leaves::layer(app)])
        .flatten()
        .fold(page.into(), |below, layer| {
            stack![below, layer].width(Fill).height(Fill).into()
        })
}

/// 0.5 layout: sidebar on the left, header and old Settings cards.
fn sidebar(app: &TrackerApp) -> Element<'_, Message> {
    // Sidebar is a fixed width. The remaining pane flexes with the window
    // (rev 3). Header chips live in that pane. Column counts come from
    // `responsive`. Settings keeps the header and scrolls the form in the
    // leftover height. Other screens scroll the whole pane.
    let content = column![widgets::app_header(app)];
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
        page_body(
            app,
            content,
            crate::settings::view,
            PAGE_PAD_X,
            true,
            Prompts::Inline
        ),
    ]
    .spacing(0)
    .width(Fill)
    .height(Fill)
    .into()
}

/// 0.6 layout: tabs across the top, then the page title, then on the stats
/// screens the season, role and Companion row, then the screen. The bar and
/// the page share one gutter ([`theme::page_gutter`]).
///
/// The control row is a stopgap: it is the 0.5 header row, not the spec's
/// filter chips (Season, queues, roles, Crew, Hero, Map; "Filters (n)" at
/// 1024), which land with the per-screen redesigns. Games has them already.
fn top_tabs(app: &TrackerApp) -> Element<'_, Message> {
    responsive(move |size| {
        let mut content = column![crate::nav::page_heading(app.screen)];
        // Settings has none; Overview and Games draw the spec's filter
        // chips themselves.
        if !matches!(
            app.screen,
            Screen::Settings | Screen::Overview | Screen::Games
        ) {
            content = content.push(widgets::header_controls(app));
        }
        let gutter = theme::page_gutter(size.width);
        let window = column![
            crate::nav::top_bar(app, size.width),
            page_body(
                app,
                content,
                crate::settings_page::view,
                gutter,
                false,
                Prompts::Floating
            ),
        ]
        .spacing(0)
        .width(Fill)
        .height(Fill);
        match floating_prompts(app, size.width - 2.0 * gutter) {
            Some(prompts) => stack![window, prompts].width(Fill).height(Fill).into(),
            None => window.into(),
        }
    })
    .width(Fill)
    .height(Fill)
    .into()
}

/// Where the toast and the restart bar go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prompts {
    /// Above the screen, under the header (the 0.5 sidebar).
    Inline,
    /// Over the window at the bottom ([`floating_prompts`]).
    Floating,
}

/// Widest the floating toast and restart bar get.
const FLOAT_MAX: f32 = 640.0;
/// Space between them and the window's bottom edge.
const FLOAT_GAP: f32 = 16.0;

/// The toast and the restart bar, centred at the bottom of the window,
/// at most [`FLOAT_MAX`] wide and never wider than the page (`page`).
/// Only they take clicks: the space around them passes them through.
fn floating_prompts(app: &TrackerApp, page: f32) -> Option<Element<'_, Message>> {
    let restart = app.restart_bar_visible();
    if app.toast.is_none() && !restart {
        return None;
    }
    let mut prompts = Column::new().spacing(8);
    if let Some(t) = &app.toast {
        prompts = prompts.push(widgets::toast_bar(t, app.toast_action.clone()));
    }
    if restart {
        prompts = prompts.push(restart_prompt::bar());
    }
    Some(
        container(container(prompts).max_width(FLOAT_MAX.min(page.max(0.0))))
            .padding(FLOAT_GAP)
            .width(Fill)
            .height(Fill)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Bottom)
            .into(),
    )
}

/// The pane right of the sidebar or under the tabs: `content` (what the
/// layout puts first: the header, or the title and controls), then with
/// [`Prompts::Inline`] the toast and the restart bar, then the screen,
/// padded `pad_x` at the sides. The toast and the restart bar never come
/// between a title and its controls.
/// The classic Settings (`settings_scrolls`) fills the height and scrolls
/// its own form; every other screen, the 0.6 Settings page included,
/// scrolls the whole pane, so the scrollbar sits in one place on every tab.
fn page_body<'a>(
    app: &'a TrackerApp,
    content: Column<'a, Message>,
    settings: fn(&'a TrackerApp, f32) -> Element<'a, Message>,
    pad_x: f32,
    settings_scrolls: bool,
    prompts: Prompts,
) -> Element<'a, Message> {
    let settings_screen = settings_scrolls && app.screen == Screen::Settings;
    let height = if settings_screen {
        Fill
    } else {
        Length::Shrink
    };
    let mut content = content.spacing(16).width(Fill).height(height);
    if prompts == Prompts::Inline {
        if let Some(t) = &app.toast {
            content = content.push(widgets::toast_bar(t, app.toast_action.clone()));
        }
        if app.restart_bar_visible() {
            content = content.push(restart_prompt::bar());
        }
    }
    content = content.push(
        responsive(move |size| match app.screen {
            Screen::Overview => crate::overview::view(app, size.width),
            Screen::Games => crate::games::view(app, size.width, size.width + 2.0 * pad_x),
            Screen::Heroes => crate::heroes::view(app, size.width),
            Screen::Maps => crate::maps::view(app, size.width),
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
            bottom: PAGE_PAD_Y,
            left: pad_x,
            right: pad_x,
        });
    if settings_screen {
        container(main).width(Fill).height(Fill).into()
    } else {
        iced::widget::scrollable(main)
            .width(Fill)
            .height(Fill)
            .into()
    }
}
