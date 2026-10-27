//! Small pieces the 0.6 Maps and Seasons tabs share: their one message
//! type, cards, labels, rules and the amber "N undecided · Review" chip.
//!
//! Kept private to these two tabs on purpose. The shared filter chips
//! (#556) and stat tiles (#557) replace the overlapping bits once those
//! land.

use iced::widget::text::Wrapping;
use iced::widget::{button, container, image, row, space, text};
use iced::{Alignment, Background, Border, Color, Element, Padding, Task, Theme};

use crate::app::{Message, TrackerApp};
use crate::icons::{self, Icon};
use crate::model::SeasonSel;
use crate::theme::{self, ACCENT, BORDER, FONT_BOLD, SIZE_LABEL, SURFACE, TEXT_3, WARN};

/// Spec `--accent-soft`: the accent at 16%.
pub const ACCENT_SOFT: Color = Color { a: 0.16, ..ACCENT };

/// The amber outline on undecided and need-a-look items (never a fill).
pub const WARN_LINE: Color = Color { a: 0.45, ..WARN };

/// What the Maps and Seasons tabs send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageMsg {
    Maps(crate::maps_tab::Msg),
    /// "Review" on an undecided chip: Games with the Needs a look filter
    /// on, in that season (`None` is All time).
    Review(Option<String>),
}

pub fn update(app: &mut TrackerApp, msg: PageMsg) -> Task<Message> {
    match msg {
        PageMsg::Maps(msg) => {
            crate::maps_tab::update(&mut app.maps_ui, msg);
            Task::none()
        }
        PageMsg::Review(season) => {
            // Clears the other filters, picks All time, opens Games.
            let task = crate::nav::update(app, crate::nav::Msg::ShowNeedsLook);
            if let Some(id) = season {
                app.season = SeasonSel::Season(id);
                crate::seasons::persist_selection(&app.data_dir, &app.season);
            }
            task
        }
    }
}

/// A section label: small, bold, muted capitals.
pub fn label(s: &str) -> text::Text<'static> {
    text(s.to_uppercase())
        .size(SIZE_LABEL)
        .font(FONT_BOLD)
        .color(TEXT_3)
        .wrapping(Wrapping::None)
}

/// A table header that does not sort, `width` px wide.
pub fn head_label(s: &str, width: f32) -> Element<'static, Message> {
    container(label(s)).width(width).into()
}

/// A 1 px line between table rows.
pub fn rule() -> Element<'static, Message> {
    container(space().height(1))
        .width(iced::Fill)
        .height(1)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(BORDER)),
            ..container::Style::default()
        })
        .into()
}

/// A page card. `amber` outlines it for undecided and need-a-look items.
pub fn card(
    content: Element<'static, Message>,
    amber: bool,
    pad: f32,
) -> Element<'static, Message> {
    container(content)
        .padding(pad)
        .width(iced::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            text_color: Some(theme::TEXT),
            border: Border {
                color: if amber { WARN_LINE } else { BORDER },
                width: 1.0,
                radius: theme::RADIUS_CARD.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A text button with no fill until hovered.
pub fn plain_button(status: button::Status) -> button::Style {
    let hot = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hot.then_some(Background::Color(crate::wr_bar::SURFACE_2)),
        text_color: theme::TEXT,
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// "N undecided · Review" for `count` games, or `None` when every game
/// has an outcome.
pub fn review_text(count: usize) -> Option<String> {
    (count > 0).then(|| format!("{count} undecided · Review"))
}

/// The amber outline chip: warning sign, "N undecided · Review". Opens
/// Games on that season's games that need a look.
pub fn review_chip(count: usize, season: Option<String>) -> Element<'static, Message> {
    let Some(words) = review_text(count) else {
        return space().width(0).into();
    };
    button(
        row![
            image(icons::handle(Icon::Warn)).width(11).height(11),
            text(words)
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(WARN)
                .wrapping(Wrapping::None),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([2, 8]))
    .style(|_: &Theme, status| {
        let tint = match status {
            button::Status::Hovered => 0.12,
            button::Status::Pressed => 0.2,
            button::Status::Active | button::Status::Disabled => 0.0,
        };
        button::Style {
            background: (tint > 0.0).then_some(Background::Color(Color { a: tint, ..WARN })),
            text_color: WARN,
            border: Border {
                color: WARN_LINE,
                width: 1.0,
                radius: theme::RADIUS_CHIP.into(),
            },
            ..button::Style::default()
        }
    })
    .on_press(Message::Pages(PageMsg::Review(season)))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Screen;

    #[test]
    fn review_copy_and_absence() {
        assert_eq!(review_text(3).as_deref(), Some("3 undecided · Review"));
        assert_eq!(review_text(0), None);
        let _ = review_chip(0, None);
        let _ = review_chip(86, Some("s4".into()));
    }

    #[test]
    fn review_opens_games_on_that_season() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.filter_hero = Some("Ana".into());
        let _ = update(&mut app, PageMsg::Review(Some("season-4".into())));
        assert_eq!(app.screen, Screen::Games);
        assert!(app.needs_look_filter);
        assert_eq!(app.filter_hero, None);
        assert_eq!(app.season, SeasonSel::Season("season-4".into()));

        let _ = update(&mut app, PageMsg::Review(None));
        assert_eq!(app.season, SeasonSel::AllTime);
    }

    #[test]
    fn maps_messages_reach_the_tab_state() {
        let (_dir, mut app) = crate::fixtures::test_app();
        let _ = update(
            &mut app,
            PageMsg::Maps(crate::maps_tab::Msg::Sort(crate::maps_tab::SortKey::Recent)),
        );
        assert_eq!(app.maps_ui.sort, crate::maps_tab::SortKey::Recent);
    }
}
