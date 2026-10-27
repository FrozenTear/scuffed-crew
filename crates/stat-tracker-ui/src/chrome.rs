//! The window chrome around the screens: the 0.6 top tabs or the 0.5
//! sidebar ([`crate::nav::NavStyle`]). Both wrap the same [`page_body`],
//! so the toast, the restart bar, the page padding and the scrolling are
//! drawn one way for both layouts.

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
        page_body(app, content, crate::settings::view, PAGE_PAD_X, true, None),
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
/// 1024), which land with the per-screen redesigns.
fn top_tabs(app: &TrackerApp) -> Element<'_, Message> {
    responsive(move |size| {
        let mut content = column![crate::nav::page_heading(app.screen)];
        if app.screen != Screen::Settings {
            content = content.push(widgets::header_controls(app));
        }
        let gutter = theme::page_gutter(size.width);
        // On Settings the toast is as wide as the page column below it.
        let toast_max = (app.screen == Screen::Settings)
            .then(|| crate::settings_model::page_width(size.width - 2.0 * gutter));
        column![
            crate::nav::top_bar(app, size.width),
            page_body(
                app,
                content,
                crate::settings_page::view,
                gutter,
                false,
                toast_max
            ),
        ]
        .spacing(0)
        .width(Fill)
        .height(Fill)
        .into()
    })
    .width(Fill)
    .height(Fill)
    .into()
}

/// The pane right of the sidebar or under the tabs: `content` (what the
/// layout puts first: the header, or the title and controls), the toast,
/// the restart bar, then the screen, padded `pad_x` at the sides. The
/// toast and the restart bar never come between a title and its controls.
/// The classic Settings (`settings_scrolls`) fills the height and scrolls
/// its own form; every other screen, the 0.6 Settings page included,
/// scrolls the whole pane, so the scrollbar sits in one place on every tab.
fn page_body<'a>(
    app: &'a TrackerApp,
    content: Column<'a, Message>,
    settings: fn(&'a TrackerApp, f32) -> Element<'a, Message>,
    pad_x: f32,
    settings_scrolls: bool,
    toast_max: Option<f32>,
) -> Element<'a, Message> {
    let settings_screen = settings_scrolls && app.screen == Screen::Settings;
    let height = if settings_screen {
        Fill
    } else {
        Length::Shrink
    };
    let mut content = content.spacing(16).width(Fill).height(height);
    if let Some(t) = &app.toast {
        let bar = widgets::toast_bar(t, app.toast_action.clone());
        content = content.push(match toast_max {
            Some(max) => container(bar).width(Fill).max_width(max).into(),
            None => bar,
        });
    }
    if app.restart_bar_visible() {
        content = content.push(restart_prompt::bar());
    }
    content = content.push(
        responsive(move |size| match app.screen {
            Screen::Overview => crate::overview::view(app, size.width),
            Screen::Games => crate::games::view(app, size.width),
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
