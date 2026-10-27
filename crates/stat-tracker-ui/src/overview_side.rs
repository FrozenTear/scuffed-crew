//! The Overview's right panel (spec p1): the season card ("49%" over its
//! games, a bar with the rate marked, "Last 20 55% ▲ +6 vs season" and its
//! sparkline, then how the rate is counted), "Needs a look · all seasons"
//! with Review games, and the Companion card with its switch.
//!
//! Small-sample rule ([`crate::small_sample`]): a win rate shows only
//! from 10 decided games; fewer show the record instead. Rank now and Peak wait for the rank reader.

use iced::widget::{Column, Row, button, column, container, row, text, toggler};
use iced::{Alignment, Background, Border, Element, Fill, Padding, Theme};

use crate::aggregate::Record;
use crate::app::{Message, TrackerApp};
use crate::model::{Game, Outcome, SeasonSel, game_card_in_progress};
use crate::session_bits::sparkline;
use crate::theme::{
    self, BORDER, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL,
    SIZE_META, TEXT, TEXT_2, TEXT_3,
};
use crate::widgets;

use crate::small_sample::{decided, rate};
/// The "Last 20" window.
pub const LAST_N: usize = 20;

pub fn view<'a>(app: &'a TrackerApp, shown: &[&Game]) -> Element<'a, Message> {
    column![
        season_card(app, shown),
        needs_look_card(app),
        companion_card(app)
    ]
    .spacing(crate::overview_page::GAP)
    .width(Fill)
    .into()
}

fn panel<'a>(content: Element<'a, Message>, warn: bool) -> Element<'a, Message> {
    container(content)
        .padding(Padding::from([16, 18]))
        .width(Fill)
        .style(move |t: &Theme| {
            let mut style = theme::surface_panel(t);
            if warn {
                style.border.color = iced::Color {
                    a: 0.6,
                    ..theme::WARN
                };
            }
            style
        })
        .into()
}

/// Wins, losses, draws and games of `games`.
pub fn record_of<'a>(games: impl IntoIterator<Item = &'a Game>) -> Record {
    let mut r = Record {
        games: 0,
        wins: 0,
        losses: 0,
        draws: 0,
    };
    for g in games {
        r.games += 1;
        match g.outcome {
            Outcome::Win => r.wins += 1,
            Outcome::Loss => r.losses += 1,
            Outcome::Draw => r.draws += 1,
            Outcome::Unknown => {}
        }
    }
    r
}

/// The rate, or the record when the sample is small.
pub fn headline(r: &Record) -> String {
    rate(r).map_or_else(|| r.wl_label(), |p| format!("{p}%"))
}

/// The newest [`LAST_N`] decided games, newest first.
pub fn last_n<'a>(shown: &[&'a Game]) -> Vec<&'a Game> {
    shown
        .iter()
        .copied()
        .filter(|g| g.outcome != Outcome::Unknown)
        .take(LAST_N)
        .collect()
}

/// "▲ +6 vs season" (or "▼ -3 vs season") when both rates show.
pub fn versus(last: &Record, season: &Record) -> Option<String> {
    let (a, b) = (rate(last)?, rate(season)?);
    let d = a - b;
    Some(match d.signum() {
        1 => format!("▲ +{d} vs season"),
        -1 => format!("▼ {d} vs season"),
        _ => "same as season".to_string(),
    })
}

/// "▲ +6 vs season" with the arrow in the icon font
/// ([`crate::overview_page::glyph`]).
fn versus_text(vs: &str) -> Element<'static, Message> {
    let mut chars = vs.chars();
    let first = chars.next().unwrap_or(' ');
    let rest: String = chars.collect();
    let color = match first {
        '▲' => theme::OK,
        '▼' => theme::DANGER,
        _ => TEXT_2,
    };
    if matches!(first, '▲' | '▼') {
        row![
            crate::overview_page::glyph(first)
                .size(SIZE_LABEL)
                .color(color),
            text(rest)
                .size(SIZE_LABEL)
                .font(FONT_SEMIBOLD)
                .color(TEXT_2),
        ]
        .align_y(Alignment::Center)
        .into()
    } else {
        text(vs.to_string())
            .size(SIZE_LABEL)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2)
            .into()
    }
}

/// How the numbers are counted, under the card.
pub fn counted_line(season: &Record, last: &Record) -> String {
    let mut line = format!("Win rate counts decided games: {}", season.wl_label());
    match season.undecided() {
        0 => line.push('.'),
        n => line.push_str(&format!(", plus {n} undecided, makes {}.", season.games)),
    }
    if last.games > 0 {
        line.push_str(&format!(
            " Last {} is {}, oldest left.",
            last.games,
            last.wl_label()
        ));
    }
    line
}

fn scope_label(app: &TrackerApp) -> String {
    match &app.season {
        SeasonSel::Season(id) => app
            .seasons
            .seasons
            .iter()
            .find(|s| &s.id == id)
            .map_or_else(|| "Season".to_string(), |s| s.name.clone()),
        SeasonSel::AllTime => "All time".to_string(),
    }
}

fn season_card<'a>(app: &'a TrackerApp, shown: &[&Game]) -> Element<'a, Message> {
    let season = record_of(shown.iter().copied());
    // The Last line only while it is a part of the season, not all of it.
    let mut recent = last_n(shown);
    if recent.len() >= decided(&season) {
        recent.clear();
    }
    let last = record_of(recent.iter().copied());
    let mut col = Column::new()
        .spacing(8)
        .push(widgets::label_text(&scope_label(app)));
    col = col.push(
        row![
            text(headline(&season))
                .size(32)
                .font(FONT_EXTRABOLD)
                .color(TEXT),
            text(season.games_label())
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        ]
        .spacing(10)
        .align_y(Alignment::End),
    );
    if let Some(p) = rate(&season) {
        col = col.push(crate::winrate_bar::bar(p as f32));
    }
    if last.games > 0 {
        let mut line = Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(
                text(format!("Last {}", last.games))
                    .size(SIZE_BODY)
                    .font(FONT_BOLD)
                    .color(TEXT),
            )
            .push(
                text(headline(&last))
                    .size(SIZE_BODY)
                    .font(FONT_BOLD)
                    .color(TEXT),
            );
        if let Some(vs) = versus(&last, &season) {
            line = line.push(
                container(versus_text(&vs))
                    .padding(Padding::from([2, 8]))
                    .style(|_: &Theme| container::Style {
                        background: Some(Background::Color(BORDER)),
                        border: Border {
                            radius: theme::RADIUS_CHIP.into(),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    }),
            );
        }
        col = col.push(line).push(sparkline(recent.iter().rev().copied()));
    }
    col = col.push(
        text(counted_line(&season, &last))
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    );
    panel(col.into(), false)
}

/// What the count is made of: "150 without an outcome (3 of them in
/// Season 5), 9 with an unknown hero".
pub fn needs_look_parts(app: &TrackerApp) -> String {
    let live = app.live_game_session.as_deref();
    let window = app.season_window();
    let mut no_outcome = 0;
    let mut no_outcome_here = 0;
    let mut no_hero = 0;
    let mut no_map = 0;
    for g in &app.games {
        let playing = game_card_in_progress(&g.session_id, g.outcome, live);
        if g.outcome == Outcome::Unknown && !playing {
            no_outcome += 1;
            if window.is_some_and(|w| w.contains(g.played_at)) {
                no_outcome_here += 1;
            }
        }
        if g.needs_review() {
            let fields = g.review_fields();
            if fields.contains(&"hero") {
                no_hero += 1;
            }
            if fields.iter().any(|f| *f == "map" || *f == "mode") {
                no_map += 1;
            }
        }
    }
    let mut parts = Vec::new();
    if no_outcome > 0 {
        let mut p = format!("{no_outcome} without an outcome");
        if window.is_some() {
            p.push_str(&format!(
                " ({no_outcome_here} of them in {})",
                scope_label(app)
            ));
        }
        parts.push(p);
    }
    if no_hero > 0 {
        parts.push(format!("{no_hero} with an unknown hero"));
    }
    if no_map > 0 {
        parts.push(format!("{no_map} with an unknown map"));
    }
    parts.join(", ")
}

fn needs_look_card(app: &TrackerApp) -> Element<'_, Message> {
    let count = crate::nav::needs_look_count(&app.games, app.live_game_session.as_deref());
    let mut col = Column::new()
        .spacing(8)
        .push(widgets::label_text("Needs a look · all seasons"));
    if count == 0 {
        col = col.push(
            text("Nothing needs a look. Every game has a result, a map and a hero.")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        );
        return panel(col.into(), false);
    }
    col = col
        .push(
            row![
                text(count.to_string())
                    .size(32)
                    .font(FONT_EXTRABOLD)
                    .color(theme::WARN),
                text(if count == 1 { "game" } else { "games" })
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3),
            ]
            .spacing(10)
            .align_y(Alignment::End),
        )
        .push(
            text(needs_look_parts(app))
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        )
        .push(
            button(
                container(
                    text("Review games")
                        .size(SIZE_META)
                        .font(FONT_BOLD)
                        .color(TEXT),
                )
                .center_x(Fill),
            )
            .padding(Padding::from([9, 14]))
            .width(Fill)
            .style(theme::chip(true))
            .on_press(crate::nav::chip_message()),
        );
    panel(col.into(), true)
}

/// The Companion's state in words, and the shortcut while it is on.
pub fn companion_line(app: &TrackerApp) -> String {
    let chip = app.companion_chip();
    if !chip.enabled {
        return "Off in Settings".to_string();
    }
    let bind = app
        .overlay_hotkey
        .enabled
        .then(|| app.overlay_hotkey.bind.clone());
    let state = if chip.showing {
        "Showing"
    } else if chip.game_running {
        "Hidden for this game"
    } else {
        "Waiting for a game"
    };
    match bind {
        Some(b) if chip.showing || !chip.game_running => format!("{state} · {b} hides it"),
        Some(b) => format!("{state} · {b} shows it"),
        None => state.to_string(),
    }
}

fn companion_card(app: &TrackerApp) -> Element<'_, Message> {
    let chip = app.companion_chip();
    panel(
        Row::new()
            .spacing(12)
            .align_y(Alignment::Center)
            .push(
                column![
                    text("Companion")
                        .size(SIZE_BODY)
                        .font(FONT_BOLD)
                        .color(TEXT),
                    text(companion_line(app))
                        .size(SIZE_LABEL)
                        .font(FONT_MEDIUM)
                        .color(TEXT_3),
                ]
                .spacing(2)
                .width(Fill),
            )
            .push(
                toggler(chip.showing)
                    .on_toggle_maybe(chip.enabled.then_some(|_| Message::ToggleOverlay))
                    .size(20)
                    .style(theme::companion_toggle),
            )
            .into(),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(w: usize, l: usize, d: usize, games: usize) -> Record {
        Record {
            games,
            wins: w,
            losses: l,
            draws: d,
        }
    }

    #[test]
    fn a_rate_needs_ten_decided_games() {
        assert_eq!(headline(&rec(2, 5, 0, 7)), "2-5");
        assert_eq!(rate(&rec(5, 4, 0, 9)), None, "9 games");
        assert_eq!(rate(&rec(5, 4, 0, 12)), None, "12 games, 9 decided");
        assert_eq!(rate(&rec(5, 4, 1, 12)), Some(50), "12 games, 10 decided");
        assert_eq!(rate(&rec(0, 0, 0, 12)), None, "none decided");
        assert_eq!(headline(&rec(52, 55, 0, 110)), "49%");
    }

    #[test]
    fn last_vs_season_needs_both_rates() {
        assert_eq!(
            versus(&rec(11, 9, 0, 20), &rec(52, 55, 0, 110)).as_deref(),
            Some("▲ +6 vs season")
        );
        assert_eq!(versus(&rec(3, 2, 0, 5), &rec(52, 55, 0, 110)), None);
    }

    #[test]
    fn the_counted_line_reads_like_the_spec() {
        assert_eq!(
            counted_line(&rec(52, 55, 0, 110), &rec(11, 9, 0, 20)),
            "Win rate counts decided games: 52-55, plus 3 undecided, makes 110. Last 20 is 11-9, oldest left."
        );
    }

    #[test]
    fn last_n_skips_undecided_games() {
        let mut games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        games[0].outcome = Outcome::Unknown;
        let refs: Vec<&Game> = games.iter().collect();
        let last = last_n(&refs);
        assert_eq!(last.len(), games.len() - 1);
        assert!(last.iter().all(|g| g.outcome != Outcome::Unknown));
    }

    #[test]
    fn the_cards_draw_on_an_empty_demo() {
        let (_dir, app) = crate::fixtures::test_app();
        assert!(companion_line(&app).len() > 3);
        let _ = view(&app, &[]);
        assert_eq!(needs_look_parts(&app), "");
    }

    #[test]
    fn the_counted_line_leaves_out_an_empty_last() {
        assert_eq!(
            counted_line(&rec(2, 1, 0, 3), &rec(0, 0, 0, 0)),
            "Win rate counts decided games: 2-1."
        );
    }
}
