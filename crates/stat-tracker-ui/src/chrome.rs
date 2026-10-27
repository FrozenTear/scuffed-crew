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
        page_body(app, content, crate::settings::view),
    ]
    .spacing(0)
    .width(Fill)
    .height(Fill)
    .into()
}

/// 0.6 layout: tabs across the top, page controls under them on the
/// stats screens, and the new Settings page.
fn top_tabs(app: &TrackerApp) -> Element<'_, Message> {
    let bar = responsive(|size| crate::nav::top_bar(app, size.width))
        .width(Fill)
        .height(Length::Shrink);
    let mut content = column![];
    if app.screen != Screen::Settings {
        content = content.push(crate::nav::page_controls(app));
    }
    column![bar, page_body(app, content, crate::settings_page::view)]
        .spacing(0)
        .width(Fill)
        .height(Fill)
        .into()
}

/// The pane right of the sidebar or under the tabs: `content` (what the
/// layout puts first), the toast, the restart bar, then the screen, padded.
/// Settings fills the height and scrolls its own form (`settings`); the
/// other screens scroll the whole pane.
fn page_body<'a>(
    app: &'a TrackerApp,
    content: Column<'a, Message>,
    settings: fn(&'a TrackerApp, f32) -> Element<'a, Message>,
) -> Element<'a, Message> {
    let settings_screen = app.screen == Screen::Settings;
    let height = if settings_screen {
        Fill
    } else {
        Length::Shrink
    };
    let mut content = content.spacing(16).width(Fill).height(height);
    if let Some(t) = &app.toast {
        content = content.push(widgets::toast_bar(t, app.toast_action.clone()));
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
            left: PAGE_PAD_X,
            right: PAGE_PAD_X,
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
