//! The Overview's main column (spec p1): the latest session's card
//! ("TONIGHT · 5 games · 4-1", its sparkline, "Since 23:31 · 1 h 21 min"),
//! the newest game (mode, hero, role and time, the map, the result, the
//! six shared stat tiles from [`crate::stat_tiles`]), then "Earlier tonight" as
//! compact rows. "Check" and a row open the game on Games.
//!
//! A session shows its record, never a win rate (small-sample rule).

use chrono::Local;
use iced::widget::{Column, Row, button, column, container, image, row, space, text};
use iced::{Alignment, Background, Border, Element, Fill, Padding, Theme};

use crate::aggregate::SessionGroup;
use crate::app::{Message, TrackerApp};
use crate::icons::{self, Icon};
use crate::model::{Game, Outcome, Role, Screen, game_card_in_progress};
use crate::overview_page::Msg;
use crate::session_bits::{day_part, span_line, sparkline};
use crate::stat_tiles::unsure_now;
use crate::theme::{
    self, BORDER, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL,
    SIZE_META, TEXT, TEXT_2, TEXT_3,
};
use crate::widgets;

/// No game in the filters at all.
pub const EMPTY: &str = "No games yet. Press Tab in-game to capture the scoreboard.";

/// "TONIGHT" while the session is this evening's, else "LAST SESSION ·
/// Sunday night".
pub fn session_label(group: &SessionGroup, now: chrono::DateTime<chrono::Utc>) -> String {
    let Some(oldest) = group.games.last() else {
        return "Tonight".to_string();
    };
    let start = day_part(oldest.played_at.with_timezone(&Local));
    if start.0 == day_part(now.with_timezone(&Local)).0 {
        "Tonight".to_string()
    } else {
        format!("Last session · {} {}", start.0.format("%A"), start.1)
    }
}

/// "Since 23:31 · 1 h 21 min".
pub fn since_line(group: &SessionGroup) -> String {
    let (Some(oldest), Some(newest)) = (group.games.last(), group.games.first()) else {
        return String::new();
    };
    let minutes = (newest.played_at - oldest.played_at).num_minutes();
    format!(
        "Since {} · {}",
        oldest.played_at.with_timezone(&Local).format("%H:%M"),
        span_line(minutes)
    )
}

pub fn view<'a>(
    app: &'a TrackerApp,
    session: Option<&SessionGroup>,
    _width: f32,
) -> Element<'a, Message> {
    let Some(group) = session else {
        return widgets::empty_surface(EMPTY);
    };
    let live = app.live_game_session.as_deref();
    let mut col = Column::new().spacing(crate::overview_page::GAP).width(Fill);
    col = col.push(session_card(group, app.clock));
    if let Some(newest) = group.games.first() {
        let playing = game_card_in_progress(&newest.session_id, newest.outcome, live);
        col = col.push(newest_card(newest, playing));
    }
    if group.games.len() > 1 {
        let label = if session_label(group, app.clock) == "Tonight" {
            "Earlier tonight"
        } else {
            "Earlier in this session"
        };
        col = col.push(
            column![
                widgets::label_text(label),
                earlier_rows(&group.games[1..], live)
            ]
            .spacing(8),
        );
    }
    col.into()
}

fn panel(content: Element<'_, Message>) -> container::Container<'_, Message> {
    container(content)
        .padding(Padding::from([16, 20]))
        .width(Fill)
        .style(theme::surface_panel)
}

fn session_card(
    group: &SessionGroup,
    now: chrono::DateTime<chrono::Utc>,
) -> Element<'static, Message> {
    let title = format!(
        "{} · {}",
        group.record.games_label(),
        record_line(&group.record)
    );
    panel(
        column![
            widgets::label_text(&session_label(group, now)),
            row![
                text(title).size(20).font(FONT_BOLD).color(TEXT),
                space().width(24),
                sparkline(group.games.iter().rev()),
            ]
            .align_y(Alignment::Center),
            text(since_line(group))
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        ]
        .spacing(4)
        .into(),
    )
    .into()
}

/// "4-1", draws as "3-3-1", then "· 1 undecided" (no rate).
pub fn record_line(record: &crate::aggregate::Record) -> String {
    match record.undecided() {
        0 => record.wl_label(),
        n => format!("{} · {n} undecided", record.wl_label()),
    }
}

/// "2 stats unsure", or `None`.
pub fn unsure_label(g: &Game) -> Option<String> {
    match g.suspect_fields.iter().filter(|f| unsure_now(g, f)).count() {
        0 => None,
        1 => Some("1 stat unsure".to_string()),
        n => Some(format!("{n} stats unsure")),
    }
}

fn newest_card(g: &Game, playing: bool) -> Element<'static, Message> {
    let mut meta = Row::new().spacing(8).align_y(Alignment::Center);
    if let Some(mode) = mode_of(&g.map_name) {
        meta = meta.push(chip(mode.to_string(), TEXT_2, BORDER));
    }
    meta = meta.push(portrait(16.0)).push(
        text(g.display_hero())
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
    );
    if let Some(label) = role_label(g.role) {
        meta = meta.push(
            text(label)
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(theme::role_color(g.role)),
        );
    }
    meta = meta.push(
        text(
            g.played_at
                .with_timezone(&Local)
                .format("%H:%M")
                .to_string(),
        )
        .size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .color(TEXT_2),
    );

    let mut result = Row::new().spacing(10).align_y(Alignment::Center);
    result = result.push(result_word(g.outcome, playing));
    if let Some(label) = unsure_label(g) {
        result = result.push(
            button(warn_chip(format!("{label} · Check")))
                .padding(0)
                .style(theme::quiet_btn(999.0))
                .on_press(Message::OverviewPage(Msg::OpenGame(g.session_id.clone()))),
        );
    }

    let stripe = if playing {
        theme::ACCENT
    } else {
        outcome_color(g.outcome)
    };
    container(
        row![
            container(space().width(3))
                .width(3)
                .height(Fill)
                .style(move |_: &Theme| container::Style {
                    background: Some(Background::Color(stripe)),
                    ..container::Style::default()
                }),
            column![
                meta.wrap(),
                text(g.display_map())
                    .size(28)
                    .font(FONT_EXTRABOLD)
                    .color(TEXT),
                result,
                crate::stat_tiles::row(g),
            ]
            .spacing(10)
            .padding(Padding::from([18, 20])),
        ]
        .height(iced::Length::Shrink),
    )
    .width(Fill)
    .clip(true)
    .style(theme::surface_panel)
    .into()
}

fn result_word(outcome: Outcome, playing: bool) -> Element<'static, Message> {
    if playing {
        return text("Playing")
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(theme::ACCENT)
            .into();
    }
    match outcome {
        Outcome::Win => row![
            image(icons::handle(Icon::Check)).width(14).height(14),
            text("WIN").size(SIZE_BODY).font(FONT_BOLD).color(theme::OK),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into(),
        Outcome::Loss => text("LOSS")
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(theme::DANGER)
            .into(),
        Outcome::Draw => text("DRAW")
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(TEXT_2)
            .into(),
        Outcome::Unknown => text("No outcome")
            .size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .color(TEXT_3)
            .into(),
    }
}

fn outcome_color(outcome: Outcome) -> iced::Color {
    match outcome {
        Outcome::Unknown => TEXT_3,
        other => theme::outcome_color(other),
    }
}

/// "TANK", "DAMAGE", "SUPPORT".
pub fn role_label(role: Role) -> Option<&'static str> {
    match role {
        Role::Tank => Some("TANK"),
        Role::Damage => Some("DAMAGE"),
        Role::Support => Some("SUPPORT"),
        Role::Unknown => None,
    }
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

/// A grey silhouette for a hero portrait (no Blizzard art is shipped).
pub fn portrait(size: f32) -> Element<'static, Message> {
    container(
        image(icons::handle(Icon::Tab(Screen::Heroes, false)))
            .width(size * 0.7)
            .height(size * 0.7),
    )
    .width(size)
    .height(size)
    .center(size)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(BORDER)),
        border: Border {
            radius: (size / 4.0).into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// An amber outline chip with the warning sign.
pub fn warn_chip(label: String) -> Element<'static, Message> {
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

pub fn chip(label: String, color: iced::Color, border: iced::Color) -> Element<'static, Message> {
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

/// Earlier games of the session: result, map and mode, E/D/A, time and the
/// unsure mark, each opening the game on Games.
fn earlier_rows(games: &[Game], live: Option<&str>) -> Element<'static, Message> {
    let mut col = Column::new().spacing(0).width(Fill);
    for (i, g) in games.iter().enumerate() {
        if i > 0 {
            col = col.push(crate::settings_rows::hairline());
        }
        let playing = game_card_in_progress(&g.session_id, g.outcome, live);
        let (word, color) = if playing {
            ("Playing", theme::ACCENT)
        } else {
            match g.outcome {
                Outcome::Win => ("WIN", theme::OK),
                Outcome::Loss => ("LOSS", theme::DANGER),
                Outcome::Draw => ("DRAW", TEXT_2),
                Outcome::Unknown => ("Undecided", TEXT_3),
            }
        };
        let mut map = Row::new().spacing(8).align_y(Alignment::Center).push(
            text(g.display_map())
                .size(SIZE_META)
                .font(FONT_BOLD)
                .color(TEXT),
        );
        if let Some(mode) = mode_of(&g.map_name) {
            map = map.push(chip(mode.to_string(), TEXT_2, BORDER));
        }
        let mut marks = Row::new();
        if let Some(label) = unsure_label(g) {
            marks = marks.push(warn_chip(label));
        }
        let stripe_color = if playing {
            theme::ACCENT
        } else {
            outcome_color(g.outcome)
        };
        let line = row![
            container(space().width(3))
                .width(3)
                .height(20)
                .style(move |_: &Theme| container::Style {
                    background: Some(Background::Color(stripe_color)),
                    ..container::Style::default()
                }),
            container(text(word).size(SIZE_META).font(FONT_BOLD).color(color)).width(90),
            container(map).width(Fill),
            container(
                text(format!("{}/{}/{}", g.elims, g.deaths, g.assists))
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(TEXT)
            )
            .width(80),
            container(
                text(
                    g.played_at
                        .with_timezone(&Local)
                        .format("%H:%M")
                        .to_string()
                )
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT)
            )
            .width(56),
            container(marks)
                .width(150)
                .align_x(iced::alignment::Horizontal::Right),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        col = col.push(
            button(line)
                .padding(Padding {
                    top: 9.0,
                    bottom: 9.0,
                    left: 0.0,
                    right: 14.0,
                })
                .width(Fill)
                .style(|_: &Theme, status| button::Style {
                    background: matches!(status, button::Status::Hovered)
                        .then_some(Background::Color(BORDER)),
                    text_color: TEXT,
                    ..button::Style::default()
                })
                .on_press(Message::OverviewPage(Msg::OpenGame(g.session_id.clone()))),
        );
    }
    container(col)
        .width(Fill)
        .clip(true)
        .style(theme::surface_panel)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::Record;

    fn sample() -> Vec<Game> {
        crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ))
    }

    #[test]
    fn a_session_record_never_shows_a_rate() {
        let r = Record {
            games: 5,
            wins: 3,
            losses: 1,
            draws: 0,
        };
        assert_eq!(record_line(&r), "3-1 · 1 undecided");
        assert!(!record_line(&r).contains('%'));
    }

    #[test]
    fn the_label_says_tonight_only_for_this_evening() {
        let games = sample();
        let refs: Vec<&Game> = games.iter().collect();
        let group =
            crate::aggregate::group_by_session_gap(&refs, chrono::TimeDelta::hours(3)).remove(0);
        let newest = group.games[0].played_at;
        assert_eq!(session_label(&group, newest), "Tonight");
        let week_later = newest + chrono::TimeDelta::days(7);
        assert!(session_label(&group, week_later).starts_with("Last session · "));
        assert!(since_line(&group).starts_with("Since "));
    }

    #[test]
    fn unsure_marks_follow_hand_edits() {
        let mut g = sample().remove(0);
        g.suspect_fields = vec!["damage".into(), "healing".into()];
        g.edited_fields.clear();
        assert_eq!(unsure_label(&g).as_deref(), Some("2 stats unsure"));
        g.edited_fields = vec!["damage".into()];
        assert!(!unsure_now(&g, "damage"));
        assert_eq!(unsure_label(&g).as_deref(), Some("1 stat unsure"));
    }

    #[test]
    fn numbers_and_modes_read_like_the_spec() {
        assert_eq!(mode_of("King's Row"), Some("Hybrid"));
        assert_eq!(mode_of(""), None);
        assert_eq!(role_label(Role::Support), Some("SUPPORT"));
    }
}
