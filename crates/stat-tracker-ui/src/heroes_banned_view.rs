//! The Heroes page's When banned mode, drawn (issue 779, UX Spec
//! `ban-winrate.md`). The model is [`crate::heroes_banned`].
//!
//! The heading row carries Played | When banned on the right. When banned
//! draws the Heroes filter row, the past-results caption, then the stats
//! line with Sort (Games, Win rate, Recent) and one table card in the Maps
//! chrome: Hero (portrait, role, name), Games, Decided (the count, muted,
//! then the record), Win rate (the shared bar and a whole percent from 10
//! decided games, else "Too few games (n of 10)") and Banned by ("Your
//! ban N", tinted as a tag only above 0, "Enemy N", then "Side not read
//! N" and "Lobby N" when there is one). Headers sort like Maps: the
//! active one in the accent with a chevron. No Best or Worst rail, so
//! nothing ranks as a pick. With no rows, one empty card says why, with
//! no Sort and no stats line. Below an 860 px window the columns narrow
//! and the hero name and the stats line wrap rather than clip.

use iced::widget::text::Wrapping;
use iced::widget::{Column, Row, button, column, container, row, space, text};
use iced::{Alignment, Background, Border, Element, Fill, Length, Padding, Theme};

use crate::app::{Message, TrackerApp};
use crate::heroes_banned::{
    self as model, BanRow, BanTable, BanUi, CAPTION, Mode, READER_OFF, SortKey,
};
use crate::heroes_page::Msg as PageMsg;
use crate::overview_main::{portrait, role_label};
use crate::settings_rows::{Segment, segmented};
use crate::tab_bits;
use crate::theme::{
    self, ACCENT, ACCENT_SOFT, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL,
    SIZE_META, TEXT, TEXT_2, TEXT_3,
};

/// The words of the own-team tag.
pub const YOUR_BAN: &str = "Your ban";
/// The words of the other side.
pub const ENEMY: &str = "Enemy";
/// A ban whose team was not read.
pub const SIDE_NOT_READ: &str = "Side not read";
/// A lobby ban that is not one of the four slots, so it has no side.
pub const LOBBY: &str = "Lobby";

/// Windows narrower than this get the compact columns, so the hero name
/// keeps room at the 720 px minimum.
const COMPACT_BELOW: f32 = 860.0;

const PORTRAIT: f32 = 32.0;
const HEAD_H: f32 = 36.0;
const PCT_W: f32 = 44.0;

fn msg(m: model::Msg) -> Message {
    Message::HeroesPage(PageMsg::Banned(m))
}

/// The page title, with Played | When banned on the right on Heroes once
/// the game list is in. Every other tab keeps the plain title.
pub fn heading(app: &TrackerApp) -> Element<'_, Message> {
    let title = crate::nav::page_heading(app.screen);
    if app.screen != crate::model::Screen::Heroes || crate::games_state::loading(app) {
        return title;
    }
    row![
        title,
        space().width(Fill),
        mode_switch(app.heroes_page.mode)
    ]
    .align_y(Alignment::Center)
    .into()
}

/// Played | When banned.
pub fn mode_switch(mode: Mode) -> Element<'static, Message> {
    let seg = |label, m| Segment {
        label,
        selected: mode == m,
        msg: Some(Message::HeroesPage(PageMsg::Mode(m))),
    };
    segmented(vec![
        seg("Played", Mode::Played),
        seg("When banned", Mode::WhenBanned),
    ])
}

/// Column widths at one content width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cols {
    pub games: f32,
    pub decided: f32,
    pub rate: f32,
    pub split: f32,
}

pub fn cols(width: f32) -> Cols {
    if tab_bits::has_rail(width) {
        Cols {
            games: 72.0,
            decided: 128.0,
            rate: 200.0,
            split: 240.0,
        }
    } else if crate::overview_page::window_for(width) < COMPACT_BELOW {
        Cols {
            games: 44.0,
            decided: 112.0,
            rate: 120.0,
            split: 140.0,
        }
    } else {
        Cols {
            games: 56.0,
            decided: 112.0,
            rate: 150.0,
            split: 200.0,
        }
    }
}

/// When banned: the filter row (when there are games), the caption, then
/// the table or the empty card. The loading surface is the Heroes page's.
pub fn view(app: &TrackerApp, width: f32) -> Element<'_, Message> {
    let mut page = Column::new().spacing(16).width(Fill);
    if crate::overview_page::shows_filters(app) {
        let pool = crate::aggregate::filter_games(&app.games, &app.header_filter());
        page = page.push(crate::overview_page::chip_row_except(
            app,
            &[],
            &crate::aggregate::distinct_maps(&pool),
            crate::overview_page::chips_fold(width),
            crate::heroes_page::HIDDEN,
        ));
    }
    page = page.push(
        text(CAPTION)
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
    );
    let shown = crate::heroes_page::shown_games(app);
    let mut t = model::aggregate(&shown);
    let has_games =
        !crate::overview_page::settled_of(&app.games, app.live_game_session.as_deref()).is_empty();
    let reader_on = app.bans_picks_switch().0;
    if let Some((title, line)) = model::empty_copy(has_games, shown.len(), &t, reader_on) {
        return page.push(empty_card(title, line)).into();
    }
    let ui = app.heroes_page.banned;
    page = page.push(sort_line(
        &model::stats_line(&crate::overview_heroes::scope(app), &t),
        ui,
    ));
    if !reader_on {
        page = page.push(
            text(READER_OFF)
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        );
    }
    model::sort_rows(&mut t.rows, ui.sort, ui.ascending);
    page.push(table(&t, ui, cols(width))).into()
}

/// The stats line and Sort: Games, Win rate, Recent. The line wraps in
/// its share of the row when the window is too narrow for one line.
fn sort_line(line: &str, ui: BanUi) -> Element<'static, Message> {
    let seg = |label, key| Segment {
        label,
        selected: ui.sort == key,
        msg: Some(msg(model::Msg::Sort(key))),
    };
    row![
        container(crate::widgets::label_text(line)).width(Fill),
        text("Sort").size(SIZE_META).font(FONT_MEDIUM).color(TEXT_2),
        segmented(vec![
            seg("Games", SortKey::Games),
            seg("Win rate", SortKey::WinRate),
            seg("Recent", SortKey::Recent),
        ]),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn table(t: &BanTable, ui: BanUi, c: Cols) -> Element<'static, Message> {
    let mut col = Column::new().width(Fill).push(header(ui, c));
    for r in &t.rows {
        col = col.push(tab_bits::rule()).push(ban_line(r, c));
    }
    tab_bits::card(col.into(), false, 0.0)
}

fn header(ui: BanUi, c: Cols) -> Element<'static, Message> {
    let head = |label: &'static str, key: SortKey, w: Length| -> Element<'static, Message> {
        let on = ui.sort == key;
        let color = if on { ACCENT } else { TEXT_3 };
        let mut head = Row::new().spacing(4).align_y(Alignment::Center).push(
            text(label.to_uppercase())
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(color)
                .wrapping(Wrapping::None),
        );
        // The page font has no arrows: iced's icon font draws them.
        if on {
            let arrow = if ui.ascending { '▲' } else { '▼' };
            head = head.push(
                crate::overview_page::glyph(arrow)
                    .size(SIZE_LABEL)
                    .color(color),
            );
        }
        button(head)
            .padding(0)
            .width(w)
            .style(|_: &Theme, _| button::Style::default())
            .on_press(msg(model::Msg::Header(key)))
            .into()
    };
    row![
        head("Hero", SortKey::Hero, Fill),
        head("Games", SortKey::Games, Length::Fixed(c.games)),
        head("Decided", SortKey::Decided, Length::Fixed(c.decided)),
        head("Win rate", SortKey::WinRate, Length::Fixed(c.rate)),
        tab_bits::head_label("Banned by", c.split),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .height(HEAD_H)
    .padding(Padding::from([0, 16]))
    .into()
}

/// One banned hero.
fn ban_line(r: &BanRow, c: Cols) -> Element<'static, Message> {
    let mut names = column![].spacing(2);
    if let Some(label) = role_label(r.role) {
        names = names.push(
            text(label)
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(theme::role_color(r.role)),
        );
    }
    // Wraps at word breaks in a narrow window instead of clipping.
    names = names.push(text(r.hero).size(SIZE_BODY).font(FONT_BOLD).color(TEXT));
    let rate: Element<'static, Message> = match (r.rate(), r.too_few()) {
        (Some(p), _) => row![
            text(format!("{p}%"))
                .size(SIZE_BODY)
                .font(FONT_BOLD)
                .color(TEXT)
                .width(PCT_W),
            crate::winrate_bar::bar(p as f32),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into(),
        (None, words) => text(words.unwrap_or_default())
            .size(12)
            .font(FONT_MEDIUM)
            .color(TEXT_3)
            .into(),
    };
    row![
        container(
            row![portrait(PORTRAIT), names]
                .spacing(12)
                .align_y(Alignment::Center)
        )
        .width(Fill),
        text(r.record.games.to_string())
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(TEXT)
            .width(c.games),
        row![
            text(r.decided().to_string())
                .size(SIZE_BODY)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
            text(r.record.wl_label())
                .size(SIZE_BODY)
                .font(FONT_SEMIBOLD)
                .color(TEXT_2),
        ]
        .spacing(12)
        .width(c.decided),
        container(rate).width(c.rate),
        container(split(r)).width(c.split),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .padding(Padding::from([10, 16]))
    .into()
}

/// One part of the Banned by cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPart {
    pub words: String,
    /// Drawn as the accent-tinted Your ban tag: only when our team banned
    /// the hero at least once.
    pub tag: bool,
}

/// "Your ban N" and "Enemy N" when either side banned the hero (Your ban
/// tinted only above 0), then "Side not read N" and "Lobby N" when there
/// is one. A hero named only by unread sides or lobby bans shows just
/// those, not "Your ban 0" and "Enemy 0". Wraps inside the cell.
pub fn split_parts(r: &BanRow) -> Vec<SplitPart> {
    let part = |words: String, tag| SplitPart { words, tag };
    let mut parts = Vec::new();
    if r.own > 0 || r.enemy > 0 {
        parts.push(part(format!("{YOUR_BAN} {}", r.own), r.own > 0));
        parts.push(part(format!("{ENEMY} {}", r.enemy), false));
    }
    if r.side_not_read > 0 {
        parts.push(part(format!("{SIDE_NOT_READ} {}", r.side_not_read), false));
    }
    if r.lobby > 0 {
        parts.push(part(format!("{LOBBY} {}", r.lobby), false));
    }
    parts
}

fn split(r: &BanRow) -> Element<'static, Message> {
    let mut cell = Row::new().spacing(8).align_y(Alignment::Center);
    for part in split_parts(r) {
        let t = text(part.words)
            .size(12)
            .font(FONT_SEMIBOLD)
            .color(if part.tag { TEXT } else { TEXT_2 })
            .wrapping(Wrapping::None);
        cell = cell.push(if part.tag {
            container(t)
                .padding(Padding::from([2, 8]))
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(ACCENT_SOFT)),
                    border: Border {
                        radius: 6.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                })
                .into()
        } else {
            Element::from(t)
        });
    }
    cell.wrap().vertical_spacing(4).into()
}

/// The Maps-style empty card: a title and a line.
fn empty_card(title: &'static str, line: String) -> Element<'static, Message> {
    tab_bits::card(
        column![
            text(title).size(SIZE_BODY).font(FONT_BOLD).color(TEXT),
            text(line).size(12).font(FONT_MEDIUM).color(TEXT_2),
        ]
        .spacing(4)
        .into(),
        false,
        20.0,
    )
}

#[cfg(test)]
#[path = "heroes_banned_view_tests.rs"]
mod tests;
