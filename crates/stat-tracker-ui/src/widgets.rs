use iced::widget::{Row, button, column, container, row, space, text};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::aggregate::{HeroAgg, MapAgg, Record};
use crate::app::Message;
use crate::model::{Game, Outcome, Role, SeasonSel};
use crate::theme::{
    self, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, PAD_INNER, SIZE_BODY,
    SIZE_FEATURED, SIZE_LABEL, SIZE_META, SIZE_TITLE, STRIPE, TEXT, TEXT_2, TEXT_3,
};
use scuffed_types::Season;

pub fn label_text(s: &str) -> text::Text<'static> {
    text(s.to_ascii_uppercase())
        .size(SIZE_LABEL)
        .font(FONT_SEMIBOLD)
        .color(TEXT_3)
}

pub fn season_switch<'a>(seasons: &'a [Season], selected: &'a SeasonSel) -> Element<'a, Message> {
    if seasons.is_empty() {
        return space().width(0).into();
    }
    let mut chips = Row::new().spacing(8).align_y(Alignment::Center);
    chips = chips.push(season_chip(
        "All time".into(),
        matches!(selected, SeasonSel::AllTime),
        Message::SelectSeason(SeasonSel::AllTime),
    ));
    for s in seasons {
        let sel = matches!(selected, SeasonSel::Season(id) if id == &s.id);
        let label = if s.is_current {
            format!("{} · current", s.name)
        } else {
            s.name.clone()
        };
        chips = chips.push(season_chip(
            label,
            sel,
            Message::SelectSeason(SeasonSel::Season(s.id.clone())),
        ));
    }
    chips.into()
}

fn season_chip(label: String, selected: bool, msg: Message) -> Element<'static, Message> {
    button(
        text(label)
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(if selected { TEXT } else { TEXT_2 }),
    )
    .padding(Padding::from([6, 14]))
    .style(theme::chip(selected))
    .on_press(msg)
    .into()
}

pub fn status_stub<'a>(live: &'a str) -> Element<'a, Message> {
    container(
        row![
            container(space().width(8).height(8)).style(|_| container::Style {
                background: Some(iced::Background::Color(theme::OK)),
                border: iced::Border {
                    radius: 999.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            }),
            text(live).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_2),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([6, 12]))
    .style(|_t| container::Style {
        background: Some(iced::Background::Color(theme::SURFACE)),
        border: iced::Border {
            color: theme::BORDER,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        ..container::Style::default()
    })
    .into()
}

pub fn featured_game_card(game: &Game) -> Element<'static, Message> {
    let stats = if game.has_stat_line() {
        Some(stat_line(game))
    } else {
        None
    };
    let body = column![
        label_text(game.role.label()),
        text(game.map_name.clone())
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        text(format!(
            "{}  ·  {}",
            game.hero,
            game.played_at.format("%H:%M")
        ))
        .size(SIZE_BODY)
        .font(FONT_MEDIUM)
        .color(TEXT_2),
        outcome_label(game.outcome),
    ]
    .spacing(6);
    let mut inner = column![body].spacing(GRID_GAP);
    if let Some(stats) = stats {
        inner = inner.push(stats);
    }
    card_shell(game.role, game.outcome, inner.into())
}

pub fn compact_game_card(game: &Game) -> Element<'static, Message> {
    let body = column![
        label_text(game.role.label()),
        text(game.map_name.clone())
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
        text(game.hero.clone())
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
        text(game.played_at.format("%H:%M").to_string())
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
        outcome_label(game.outcome),
    ]
    .spacing(4);
    card_shell(game.role, game.outcome, body.into())
}

pub fn hero_card(hero: &HeroAgg) -> Element<'static, Message> {
    let wr = format!("{:.0}%", hero.record.win_rate_pct());
    let games = format!(
        "{} game{}",
        hero.record.games,
        if hero.record.games == 1 { "" } else { "s" }
    );
    let body = column![
        label_text(hero.role.label()),
        text(hero.hero.clone())
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
        text(wr)
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        text(games).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_2),
        win_bar(hero.record.win_rate()),
    ]
    .spacing(6);
    card_shell(hero.role, Outcome::Unknown, body.into())
}

pub fn season_panel(
    record: &Record,
    all_time: &Record,
    has_season: bool,
) -> Element<'static, Message> {
    let delta = if has_season && all_time.win_rate() > 0.0 {
        let d = (record.win_rate() - all_time.win_rate()) * 100.0;
        format!("{d:+.0} vs all time")
    } else {
        "All time".into()
    };
    container(
        column![
            label_text("Season"),
            text(format!("{:.0}%", record.win_rate_pct()))
                .size(SIZE_FEATURED)
                .font(FONT_EXTRABOLD)
                .color(TEXT),
            text(format!(
                "{}–{}–{}",
                record.wins, record.losses, record.draws
            ))
            .size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2),
            text(format!("{} games", record.games))
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
            text(delta).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_3),
            win_bar(record.win_rate()),
        ]
        .spacing(8),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

pub fn maps_panel(maps: &[MapAgg]) -> Element<'static, Message> {
    let mut col = column![label_text("Maps")].spacing(10);
    if maps.is_empty() {
        col = col.push(
            text("No maps in this window")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    } else {
        for m in maps.iter().take(4) {
            col = col.push(
                row![
                    text(m.map_name.clone())
                        .size(SIZE_BODY)
                        .font(FONT_SEMIBOLD)
                        .color(TEXT)
                        .width(Fill),
                    text(format!("{:.0}%", m.record.win_rate_pct()))
                        .size(SIZE_BODY)
                        .font(FONT_BOLD)
                        .color(TEXT_2),
                ]
                .align_y(Alignment::Center),
            );
        }
    }
    container(col)
        .padding(PAD_INNER)
        .width(Fill)
        .style(theme::surface_panel)
        .into()
}

pub fn health_panel(status: &str) -> Element<'static, Message> {
    container(
        column![
            label_text("Tracker health"),
            text(status.to_string())
                .size(SIZE_BODY)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
            text("Companion overlay — P3")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
            text("Read-only · no StoreCommand writes")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        ]
        .spacing(8),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

fn outcome_label(outcome: Outcome) -> Element<'static, Message> {
    text(outcome.label().to_ascii_uppercase())
        .size(SIZE_LABEL)
        .font(FONT_BOLD)
        .color(theme::outcome_color(outcome))
        .into()
}

fn stat_line(game: &Game) -> Element<'static, Message> {
    row![
        stat_box("E", game.elims),
        stat_box("D", game.deaths),
        stat_box("A", game.assists),
        stat_box("DMG", game.damage),
        stat_box("HEAL", game.healing),
        stat_box("MIT", game.mitigation),
    ]
    .spacing(8)
    .into()
}

fn stat_box(label: &'static str, value: u32) -> Element<'static, Message> {
    container(
        column![
            text(label)
                .size(SIZE_LABEL)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3),
            text(format_stat(value))
                .size(SIZE_BODY)
                .font(FONT_BOLD)
                .color(TEXT),
        ]
        .spacing(2)
        .align_x(Alignment::Center),
    )
    .padding(Padding::from([8, 10]))
    .style(theme::stat_box)
    .into()
}

fn format_stat(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.0)
    } else {
        n.to_string()
    }
}

fn win_bar(rate: f32) -> Element<'static, Message> {
    let fill = (rate.clamp(0.0, 1.0) * 100.0).round() as u16;
    let rest = 100u16.saturating_sub(fill);
    container(
        row![
            container(space().height(6).width(Length::FillPortion(fill.max(1))))
                .height(6)
                .style(|_| container::Style {
                    background: Some(iced::Background::Color(theme::OK)),
                    border: iced::Border {
                        radius: 999.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
            container(space().height(6).width(Length::FillPortion(rest.max(1))))
                .height(6)
                .style(|_| container::Style {
                    background: Some(iced::Background::Color(theme::BORDER)),
                    border: iced::Border {
                        radius: 999.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
        ]
        .spacing(0),
    )
    .width(Fill)
    .into()
}

fn card_shell(
    role: Role,
    outcome: Outcome,
    content: Element<'static, Message>,
) -> Element<'static, Message> {
    container(
        row![
            container(space())
                .width(STRIPE)
                .height(Fill)
                .style(theme::stripe(outcome)),
            container(content).padding(PAD_INNER).width(Fill),
        ]
        .spacing(0),
    )
    .style(theme::role_card(role))
    .width(Fill)
    .into()
}
