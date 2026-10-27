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
use crate::theme::{
    self, BORDER, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL,
    SIZE_META, TEXT, TEXT_2, TEXT_3,
};
use crate::widgets;

/// No game stored at all.
pub const EMPTY: &str = "No games yet. Press Tab in-game to capture the scoreboard.";
/// Games stored, none left by the filters (as Games says it).
pub const EMPTY_FILTERED: &str = "No games match these filters";

/// The main column's line with no session: the first-run line only
/// before the first game, else the filters' line.
pub fn empty_text(no_games: bool) -> &'static str {
    if no_games { EMPTY } else { EMPTY_FILTERED }
}

/// The session started this evening (or after midnight of it): its
/// oldest game is on the same evening as `now` ([`day_part`]).
pub fn is_tonight(group: &SessionGroup, now: chrono::DateTime<chrono::Utc>) -> bool {
    group.games.last().is_none_or(|oldest| {
        day_part(oldest.played_at.with_timezone(&Local)).0 == day_part(now.with_timezone(&Local)).0
    })
}

/// "TONIGHT" while the session is this evening's ([`is_tonight`]), else
/// "LAST SESSION · Sunday night".
pub fn session_label(group: &SessionGroup, now: chrono::DateTime<chrono::Utc>) -> String {
    match group.games.last() {
        Some(oldest) if !is_tonight(group, now) => {
            let start = day_part(oldest.played_at.with_timezone(&Local));
            format!("Last session · {} {}", start.0.format("%A"), start.1)
        }
        _ => "Tonight".to_string(),
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

pub fn view<'a>(app: &'a TrackerApp, session: Option<&SessionGroup>) -> Element<'a, Message> {
    let Some(group) = session else {
        return widgets::empty_surface(empty_text(app.games.is_empty()));
    };
    let live = app.live_game_session.as_deref();
    let mut col = Column::new().spacing(crate::overview_page::GAP).width(Fill);
    col = col.push(session_card(group, app.clock, live));
    if let Some(newest) = group.games.first() {
        let playing = game_card_in_progress(&newest.session_id, newest.outcome, live);
        col = col.push(newest_card(newest, playing));
    }
    if group.games.len() > 1 {
        let label = if is_tonight(group, app.clock) {
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

/// The session card's title: every game of the session, then the record
/// of `settled` (its games less the one being played), so that game is
/// "Playing", not an undecided game ("5 games · 4-0 · Playing"). At the
/// first game there is no record yet ("1 game · Playing").
pub fn session_title(group: &SessionGroup, settled: &[&Game]) -> String {
    let mut title = group.record.games_label();
    if !settled.is_empty() {
        let record = crate::overview_side::record_of(settled.iter().copied());
        title.push_str(&format!(" · {}", record_line(&record)));
    }
    if settled.len() < group.games.len() {
        title.push_str(" · Playing");
    }
    title
}

fn session_card(
    group: &SessionGroup,
    now: chrono::DateTime<chrono::Utc>,
    live: Option<&str>,
) -> Element<'static, Message> {
    let settled = crate::overview_page::settled_of(group.games.iter(), live);
    let title = session_title(group, &settled);
    panel(
        column![
            widgets::label_text(&session_label(group, now)),
            row![
                text(title).size(20).font(FONT_BOLD).color(TEXT),
                space().width(24),
                // Oldest first, leaving out the game being played.
                sparkline(settled.iter().rev().copied()),
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
    if let Some(label) = g.unsure_label() {
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
        if let Some(label) = g.unsure_label() {
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
    fn the_game_being_played_is_playing_not_undecided() {
        let mut games = sample();
        games[0].outcome = Outcome::Unknown;
        let live = games[0].session_id.clone();
        let refs: Vec<&Game> = games.iter().collect();
        let group =
            crate::aggregate::group_by_session_gap(&refs, chrono::TimeDelta::hours(3)).remove(0);
        let settled = crate::overview_page::settled_of(group.games.iter(), Some(&live));
        let with_live = session_title(&group, &settled);
        assert!(with_live.ends_with(" · Playing"), "{with_live}");
        assert!(!with_live.contains("undecided"), "{with_live}");
        // Once its game closes without a result it is undecided.
        let all: Vec<&Game> = group.games.iter().collect();
        let closed = session_title(&group, &all);
        assert!(closed.contains("1 undecided"), "{closed}");
        assert!(!closed.contains("Playing"), "{closed}");
    }

    #[test]
    fn the_first_game_being_played_has_no_record_yet() {
        let mut games = sample();
        games[0].outcome = Outcome::Unknown;
        let live = games[0].session_id.clone();
        let refs: Vec<&Game> = games.iter().take(1).collect();
        let group =
            crate::aggregate::group_by_session_gap(&refs, chrono::TimeDelta::hours(3)).remove(0);
        let settled = crate::overview_page::settled_of(group.games.iter(), Some(&live));
        assert_eq!(session_title(&group, &settled), "1 game · Playing");
    }

    #[test]
    fn the_empty_column_says_first_run_only_before_any_game() {
        assert_eq!(empty_text(true), EMPTY);
        assert_eq!(empty_text(false), EMPTY_FILTERED);
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
        assert!(is_tonight(&group, newest));
        let week_later = newest + chrono::TimeDelta::days(7);
        assert!(session_label(&group, week_later).starts_with("Last session · "));
        assert!(since_line(&group).starts_with("Since "));
    }

    #[test]
    fn unsure_marks_follow_hand_edits() {
        let mut g = sample().remove(0);
        g.suspect_fields = vec!["dmg".into(), "h".into()];
        g.edited_fields.clear();
        assert_eq!(g.unsure_label().as_deref(), Some("2 stats unsure"));
        g.edited_fields = vec!["damage".into()];
        assert!(!crate::stat_tiles::unsure_now(&g, "damage"));
        assert_eq!(g.unsure_label().as_deref(), Some("1 stat unsure"));
    }

    /// The Overview counts unsure stats with `Game::unsure_label`, as the
    /// Games tab does: an unsure map, mode, hero or result is not a stat.
    #[test]
    fn only_stats_count_as_stats_unsure() {
        let mut g = sample().remove(0);
        g.suspect_fields = ["map", "mode", "hero", "result", "dmg"]
            .map(String::from)
            .to_vec();
        g.edited_fields.clear();
        assert_eq!(crate::stat_tiles::still_unsure(&g).len(), 5);
        assert_eq!(g.unsure_label().as_deref(), Some("1 stat unsure"));
    }

    #[test]
    fn numbers_and_modes_read_like_the_spec() {
        assert_eq!(mode_of("King's Row"), Some("Hybrid"));
        assert_eq!(mode_of(""), None);
        assert_eq!(role_label(Role::Support), Some("SUPPORT"));
    }
}
