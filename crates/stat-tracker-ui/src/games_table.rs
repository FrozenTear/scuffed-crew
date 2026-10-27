//! The 0.6 Games table (spec p2): one row per game with Result, Map (and
//! its mode), Hero, E/D/A and Time, then the row's marks ("⚠ 2 stats
//! unsure", "⚠ No outcome · Set"). Each column header sorts; unsure and
//! empty values always sort last. Below 1100 px of window the Hero and
//! E/D/A columns are left out, as the spec does.
//!
//! The spec's Rank, Length, Crew and queue (Comp, QP) are example data for
//! fields the tracker does not store yet, so they are not drawn.

use std::cmp::Ordering;

use iced::widget::{Column, Row, button, container, image, row, space, text};
use iced::{Alignment, Background, Border, Element, Fill, FillPortion, Padding, Theme};

use crate::app::Message;
use crate::games_page::Msg;
use crate::icons::{self, Icon};
use crate::model::{Game, Outcome, Role, Screen};
use crate::theme::{
    self, BORDER, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, PAGE_PAD_X, PAGE_PAD_X_NARROW, SIZE_LABEL,
    SIZE_META, TEXT, TEXT_2, TEXT_3, WIDE_GUTTER_MIN,
};

/// Window width below which Hero and E/D/A are left out (spec CSS).
const HIDE_HERO_BELOW: f32 = 1100.0;
/// Fixed column widths.
const W_RESULT: f32 = 104.0;
const W_EDA: f32 = 96.0;
const W_TIME: f32 = 64.0;
const W_MARKS: f32 = 190.0;
/// The outcome stripe at a row's left edge.
const STRIPE_W: f32 = 3.0;
/// Row padding.
const ROW_PAD_Y: f32 = 9.0;
const ROW_PAD_X: f32 = 14.0;
/// Hero portrait stand-in and its corner.
const PORTRAIT: f32 = 20.0;

/// A column the table can sort by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Col {
    Result,
    Map,
    Hero,
    Eda,
    #[default]
    Time,
}

impl Col {
    fn label(self) -> &'static str {
        match self {
            Col::Result => "Result",
            Col::Map => "Map",
            Col::Hero => "Hero",
            Col::Eda => "E/D/A",
            Col::Time => "Time",
        }
    }

    /// The direction a first click on this header sorts: newest, most
    /// eliminations and wins first; names A to Z.
    pub fn first_desc(self) -> bool {
        matches!(self, Col::Time | Col::Eda | Col::Result)
    }
}

/// The sort in effect: a column and its direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub col: Col,
    pub desc: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            col: Col::Time,
            desc: true,
        }
    }
}

impl Sort {
    /// A click on `col`'s header: the same column flips, another starts in
    /// its own first direction.
    pub fn clicked(self, col: Col) -> Self {
        if self.col == col {
            Self {
                col,
                desc: !self.desc,
            }
        } else {
            Self {
                col,
                desc: col.first_desc(),
            }
        }
    }
}

/// A row's sort value. `None` is unsure or empty, and always sorts last.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
enum Key {
    Num(i64),
    Text(String),
}

fn key(g: &Game, col: Col) -> Option<Key> {
    match col {
        Col::Result => {
            if g.field_unsure("result") {
                return None;
            }
            match g.outcome {
                Outcome::Win => Some(Key::Num(2)),
                Outcome::Draw => Some(Key::Num(1)),
                Outcome::Loss => Some(Key::Num(0)),
                Outcome::Unknown => None,
            }
        }
        Col::Map => {
            let map = g.map_name.trim();
            (!map.is_empty() && !g.field_unsure("map")).then(|| Key::Text(map.to_lowercase()))
        }
        Col::Hero => {
            let hero = g.hero.trim();
            (!hero.is_empty() && !hero.eq_ignore_ascii_case("unknown") && !g.field_unsure("hero"))
                .then(|| Key::Text(hero.to_lowercase()))
        }
        Col::Eda => (!["elims", "deaths", "assists"]
            .iter()
            .any(|f| unsure_now(g, f)))
        .then_some(Key::Num(i64::from(g.elims))),
        Col::Time => Some(Key::Num(g.played_at.timestamp())),
    }
}

/// Sort `games` in place. Unsure and empty values go last in both
/// directions; ties keep newest first.
pub fn sort_games(games: &mut [&Game], sort: Sort) {
    games.sort_by(|a, b| {
        let order = match (key(a, sort.col), key(b, sort.col)) {
            (Some(x), Some(y)) => {
                let o = x.partial_cmp(&y).unwrap_or(Ordering::Equal);
                if sort.desc { o.reverse() } else { o }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        order.then_with(|| b.played_at.cmp(&a.played_at))
    });
}

/// A field the reader was unsure of that no hand edit has fixed since.
fn unsure_now(g: &Game, field: &str) -> bool {
    g.field_unsure(field) && !g.edited_fields.iter().any(|e| e == field)
}

/// "2 stats unsure": the fields still unsure. `None` when there are none.
pub fn unsure_label(g: &Game) -> Option<String> {
    let n = g.suspect_fields.iter().filter(|f| unsure_now(g, f)).count();
    match n {
        0 => None,
        1 => Some("1 stat unsure".to_string()),
        n => Some(format!("{n} stats unsure")),
    }
}

/// The window width a page `content` px wide sits in (the page gutter is
/// wider from [`WIDE_GUTTER_MIN`] up).
pub fn window_for(content: f32) -> f32 {
    let wide = content + 2.0 * PAGE_PAD_X;
    if wide >= WIDE_GUTTER_MIN {
        wide
    } else {
        content + 2.0 * PAGE_PAD_X_NARROW
    }
}

/// The columns drawn at a page `content` px wide.
pub fn columns(content: f32) -> Vec<Col> {
    let mut cols = vec![Col::Result, Col::Map];
    if window_for(content) >= HIDE_HERO_BELOW {
        cols.extend([Col::Hero, Col::Eda]);
    }
    cols.push(Col::Time);
    cols
}

/// What a row needs from the page.
pub struct RowState<'a> {
    pub live: Option<&'a str>,
    pub expanded: Option<&'a str>,
}

/// The table: a header row and one row per game in `games` (already
/// sorted), each followed by its open detail card when it is expanded.
pub fn table<'a>(
    games: &[&Game],
    cols: &[Col],
    sort: Sort,
    state: &RowState<'_>,
    detail: impl Fn(&Game) -> Element<'a, Message>,
) -> Element<'a, Message> {
    let mut body = Column::new().spacing(0).width(Fill);
    body = body.push(header(cols, sort));
    for g in games {
        body = body.push(hairline());
        body = body.push(game_row(g, cols, state));
        if state.expanded == Some(g.session_id.as_str()) {
            body = body.push(container(detail(g)).padding(12).width(Fill));
        }
    }
    container(body)
        .width(Fill)
        .clip(true)
        .style(theme::surface_panel)
        .into()
}

fn width_of(col: Col) -> iced::Length {
    match col {
        Col::Result => W_RESULT.into(),
        Col::Map => FillPortion(3),
        Col::Hero => FillPortion(2),
        Col::Eda => W_EDA.into(),
        Col::Time => W_TIME.into(),
    }
}

fn header(cols: &[Col], sort: Sort) -> Element<'static, Message> {
    let mut r = Row::new()
        .spacing(12)
        .align_y(Alignment::Center)
        .padding(Padding {
            top: 10.0,
            bottom: 8.0,
            left: ROW_PAD_X + STRIPE_W,
            right: ROW_PAD_X,
        });
    for &col in cols {
        let active = sort.col == col;
        let label = if active {
            format!(
                "{} {}",
                col.label().to_ascii_uppercase(),
                if sort.desc { "↓" } else { "↑" }
            )
        } else {
            col.label().to_ascii_uppercase()
        };
        let cell = button(
            text(label)
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(if active { theme::ACCENT } else { TEXT_3 }),
        )
        .padding(0)
        .style(theme::quiet_btn(4.0))
        .on_press(Message::GamesPage(Msg::SortBy(col)));
        r = r.push(container(cell).width(width_of(col)));
    }
    r.push(space().width(W_MARKS)).into()
}

fn hairline() -> Element<'static, Message> {
    crate::settings_rows::hairline()
}

fn game_row(g: &Game, cols: &[Col], state: &RowState<'_>) -> Element<'static, Message> {
    let in_progress = crate::model::game_card_in_progress(&g.session_id, g.outcome, state.live);
    let mut cells = Row::new().spacing(12).align_y(Alignment::Center);
    for &col in cols {
        let cell: Element<'static, Message> = match col {
            Col::Result => result_cell(g, in_progress),
            Col::Map => map_cell(g),
            Col::Hero => hero_cell(g),
            Col::Eda => eda_cell(g),
            Col::Time => text(
                g.played_at
                    .with_timezone(&chrono::Local)
                    .format("%H:%M")
                    .to_string(),
            )
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT)
            .into(),
        };
        cells = cells.push(container(cell).width(width_of(col)));
    }
    cells = cells.push(
        container(marks(g, in_progress))
            .width(W_MARKS)
            .align_x(iced::alignment::Horizontal::Right),
    );
    let stripe_color = if in_progress {
        theme::ACCENT
    } else {
        outcome_color(g.outcome)
    };
    let stripe = container(space().width(STRIPE_W))
        .width(STRIPE_W)
        .height(20)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(stripe_color)),
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    let open = state.expanded == Some(g.session_id.as_str());
    button(
        row![stripe, cells]
            .spacing(ROW_PAD_X - STRIPE_W)
            .align_y(Alignment::Center),
    )
    .padding(Padding {
        top: ROW_PAD_Y,
        bottom: ROW_PAD_Y,
        left: 0.0,
        right: ROW_PAD_X,
    })
    .width(Fill)
    .style(move |_: &Theme, status| iced::widget::button::Style {
        background: (open || matches!(status, iced::widget::button::Status::Hovered))
            .then_some(Background::Color(BORDER)),
        text_color: TEXT,
        ..iced::widget::button::Style::default()
    })
    .on_press(Message::ToggleGame(g.session_id.clone()))
    .into()
}

fn outcome_color(outcome: Outcome) -> iced::Color {
    match outcome {
        Outcome::Unknown => TEXT_3,
        other => theme::outcome_color(other),
    }
}

fn result_cell(g: &Game, in_progress: bool) -> Element<'static, Message> {
    let (label, color) = if in_progress {
        ("Playing", theme::ACCENT)
    } else {
        match g.outcome {
            Outcome::Win => ("WIN", theme::OK),
            Outcome::Loss => ("LOSS", theme::DANGER),
            Outcome::Draw => ("DRAW", TEXT_2),
            Outcome::Unknown => ("Undecided", TEXT_3),
        }
    };
    text(label)
        .size(SIZE_META)
        .font(if g.outcome == Outcome::Unknown {
            FONT_SEMIBOLD
        } else {
            FONT_BOLD
        })
        .color(color)
        .into()
}

/// The map in bold, then its mode as a neutral chip (Control, Push, …).
fn map_cell(g: &Game) -> Element<'static, Message> {
    let mut r = Row::new().spacing(8).align_y(Alignment::Center).push(
        text(g.display_map()).size(SIZE_META).font(FONT_BOLD).color(
            if g.map_name.trim().is_empty() {
                TEXT_3
            } else {
                TEXT
            },
        ),
    );
    if let Some(mode) = mode_of(&g.map_name) {
        r = r.push(chip(mode, TEXT_2, BORDER));
    }
    r.into()
}

/// The map's game mode, or `None` for a blank or unknown map.
pub fn mode_of(map: &str) -> Option<&'static str> {
    let map = map.trim();
    if map.is_empty() {
        return None;
    }
    match scuffed_types::MapName::game_mode_label(map) {
        "Other" => None,
        mode => Some(mode),
    }
}

/// A grey silhouette for the portrait (no Blizzard art is shipped), the
/// hero, and the role's letter in its colour.
fn hero_cell(g: &Game) -> Element<'static, Message> {
    let portrait = container(
        image(icons::handle(Icon::Tab(Screen::Heroes, false)))
            .width(14)
            .height(14),
    )
    .width(PORTRAIT)
    .height(PORTRAIT)
    .center(PORTRAIT)
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(BORDER)),
        border: Border {
            radius: 5.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    let mut r = Row::new()
        .spacing(8)
        .align_y(Alignment::Center)
        .push(portrait)
        .push(
            text(g.display_hero())
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT),
        );
    if let Some(letter) = role_letter(g.role) {
        r = r.push(
            text(letter)
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(theme::role_color(g.role)),
        );
    }
    r.into()
}

/// "T", "D" or "S".
pub fn role_letter(role: Role) -> Option<&'static str> {
    match role {
        Role::Tank => Some("T"),
        Role::Damage => Some("D"),
        Role::Support => Some("S"),
        Role::Unknown => None,
    }
}

fn eda_cell(g: &Game) -> Element<'static, Message> {
    let unsure = ["elims", "deaths", "assists"]
        .iter()
        .any(|f| unsure_now(g, f));
    text(format!("{}/{}/{}", g.elims, g.deaths, g.assists))
        .size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .color(if unsure { theme::WARN } else { TEXT })
        .into()
}

/// The row's marks, right-aligned: what still needs a look, then an
/// upload the website refused.
fn marks(g: &Game, in_progress: bool) -> Element<'static, Message> {
    let mut r = Row::new().spacing(6).align_y(Alignment::Center);
    if g.outcome == Outcome::Unknown && !in_progress {
        r = r.push(warn_chip("No outcome · Set".to_string()));
    } else if g.needs_review() {
        r = r.push(warn_chip("Needs a look".to_string()));
    }
    if let Some(label) = unsure_label(g) {
        r = r.push(warn_chip(label));
    }
    if g.upload_reject.is_some() {
        r = r.push(chip(
            crate::model::UPLOAD_REJECTED_LABEL,
            theme::DANGER,
            theme::DANGER,
        ));
    }
    r.into()
}

/// An amber outline chip with the warning sign.
fn warn_chip(label: String) -> Element<'static, Message> {
    container(
        row![
            image(icons::handle(Icon::Warn)).width(11).height(11),
            text(label)
                .size(SIZE_LABEL)
                .font(FONT_SEMIBOLD)
                .color(theme::WARN),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([2, 9]))
    .style(outline(theme::WARN))
    .into()
}

fn chip(label: &'static str, color: iced::Color, border: iced::Color) -> Element<'static, Message> {
    container(
        text(label)
            .size(SIZE_LABEL)
            .font(FONT_SEMIBOLD)
            .color(color),
    )
    .padding(Padding::from([2, 9]))
    .style(outline(border))
    .into()
}

fn outline(color: iced::Color) -> impl Fn(&Theme) -> container::Style {
    move |_: &Theme| container::Style {
        border: Border {
            color: iced::Color { a: 0.6, ..color },
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn game(id: &str, minute: u32, outcome: Outcome, elims: u32) -> Game {
        let sample = crate::fixtures::snapshot(crate::cli::FixtureKind::Sample);
        let mut g = Game::from_match(&sample.matches[0]);
        g.session_id = id.into();
        g.outcome = outcome;
        g.elims = elims;
        g.suspect_fields.clear();
        g.edited_fields.clear();
        g.played_at = Utc.with_ymd_and_hms(2026, 10, 18, 22, minute, 0).unwrap();
        g
    }

    fn ids(games: &[&Game]) -> Vec<String> {
        games.iter().map(|g| g.session_id.clone()).collect()
    }

    #[test]
    fn time_sorts_newest_first_by_default() {
        let (a, b, c) = (
            game("a", 1, Outcome::Win, 1),
            game("b", 3, Outcome::Loss, 2),
            game("c", 2, Outcome::Win, 3),
        );
        let mut rows = vec![&a, &b, &c];
        sort_games(&mut rows, Sort::default());
        assert_eq!(ids(&rows), ["b", "c", "a"]);
    }

    #[test]
    fn unsure_and_empty_values_sort_last_both_ways() {
        let a = game("a", 1, Outcome::Win, 30);
        let mut b = game("b", 2, Outcome::Win, 40);
        b.suspect_fields = vec!["elims".into()];
        let c = game("c", 3, Outcome::Win, 10);
        let undecided = game("u", 4, Outcome::Unknown, 5);
        for desc in [true, false] {
            let mut rows = vec![&a, &b, &c];
            sort_games(
                &mut rows,
                Sort {
                    col: Col::Eda,
                    desc,
                },
            );
            assert_eq!(rows.last().unwrap().session_id, "b", "desc {desc}");
            let mut rows = vec![&undecided, &a, &b];
            sort_games(
                &mut rows,
                Sort {
                    col: Col::Result,
                    desc,
                },
            );
            assert_eq!(rows.last().unwrap().session_id, "u", "desc {desc}");
        }
        // A hand edit settles the unsure value.
        b.edited_fields = vec!["elims".into()];
        let mut rows = vec![&a, &b, &c];
        sort_games(
            &mut rows,
            Sort {
                col: Col::Eda,
                desc: true,
            },
        );
        assert_eq!(ids(&rows), ["b", "a", "c"]);
    }

    #[test]
    fn a_header_click_flips_or_starts_in_the_columns_own_direction() {
        let s = Sort::default().clicked(Col::Time);
        assert_eq!(
            s,
            Sort {
                col: Col::Time,
                desc: false
            }
        );
        assert_eq!(
            s.clicked(Col::Map),
            Sort {
                col: Col::Map,
                desc: false
            }
        );
        assert_eq!(
            s.clicked(Col::Eda),
            Sort {
                col: Col::Eda,
                desc: true
            }
        );
    }

    #[test]
    fn narrow_windows_drop_hero_and_eda() {
        assert_eq!(columns(1600.0 - 64.0).len(), 5);
        assert_eq!(columns(1100.0 - 48.0).len(), 5);
        assert_eq!(columns(1099.0 - 48.0), [Col::Result, Col::Map, Col::Time]);
    }

    #[test]
    fn unsure_count_skips_fixed_fields() {
        let mut g = game("a", 1, Outcome::Win, 1);
        g.suspect_fields = vec!["damage".into(), "healing".into()];
        assert_eq!(unsure_label(&g).as_deref(), Some("2 stats unsure"));
        g.edited_fields = vec!["damage".into()];
        assert_eq!(unsure_label(&g).as_deref(), Some("1 stat unsure"));
        g.edited_fields.push("healing".into());
        assert_eq!(unsure_label(&g), None);
    }

    #[test]
    fn modes_come_from_known_maps_only() {
        assert_eq!(mode_of("King's Row"), Some("Hybrid"));
        assert_eq!(mode_of(""), None);
        assert_eq!(mode_of("Not A Map"), None);
    }
}
