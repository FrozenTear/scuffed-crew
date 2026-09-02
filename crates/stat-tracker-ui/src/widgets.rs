use iced::widget::{Row, button, column, container, mouse_area, row, space, text, text_input};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::aggregate::{HeroAgg, MapAgg, Record};
use crate::app::{Message, TrackerApp};
use crate::model::{
    EditField, EditForm, Game, Outcome, Role, Screen, SeasonSel, display_hero_name,
};
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

pub fn app_header(app: &TrackerApp) -> Element<'_, Message> {
    row![
        text("Scuffed Tracker")
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        space().width(Fill),
        season_switch(&app.seasons.seasons, &app.season),
        role_chips(app.roles),
        status_stub(&app.live_status),
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .into()
}

pub fn sidebar(current: Screen) -> Element<'static, Message> {
    let mut col = column![].spacing(8);
    for screen in Screen::all() {
        col = col.push(
            button(
                text(screen.label())
                    .size(SIZE_BODY)
                    .font(FONT_SEMIBOLD)
                    .color(if screen == current { TEXT } else { TEXT_2 }),
            )
            .padding(Padding::from([10, 14]))
            .width(Fill)
            .style(theme::nav_btn(screen == current))
            .on_press(Message::Navigate(screen)),
        );
    }
    col.into()
}

pub fn role_chips(filter: crate::model::RoleFilter) -> Element<'static, Message> {
    let mut chips = Row::new().spacing(8).align_y(Alignment::Center);
    for role in Role::all_playable() {
        let on = filter.is_on(role);
        chips = chips.push(
            button(
                text(role.label())
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(if on { TEXT } else { TEXT_2 }),
            )
            .padding(Padding::from([6, 14]))
            .style(theme::role_chip(role, on))
            .on_press(Message::ToggleRole(role)),
        );
    }
    chips.into()
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

pub fn toast_bar(msg: &str) -> Element<'static, Message> {
    container(
        text(msg.to_string())
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([8, 14]))
    .width(Fill)
    .style(|_t| container::Style {
        background: Some(iced::Background::Color(theme::SURFACE)),
        border: iced::Border {
            color: theme::ACCENT,
            width: 1.0,
            radius: theme::inner_radius(),
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
        role_and_edited(game),
        text(game.map_name.clone())
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        text(format!(
            "{}  ·  {}",
            game.display_hero(),
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
    card_shell(
        game.role,
        game.outcome,
        inner.into(),
        theme::HEIGHT_FEATURED,
    )
}

pub fn compact_game_card(game: &Game) -> Element<'static, Message> {
    compact_game_card_inner(game, false)
}

pub fn compact_game_card_clickable(game: &Game, selected: bool) -> Element<'static, Message> {
    let sid = game.session_id.clone();
    mouse_area(compact_game_card_inner(game, selected))
        .on_press(Message::ToggleGame(sid))
        .into()
}

fn compact_game_card_inner(game: &Game, selected: bool) -> Element<'static, Message> {
    let body = column![
        role_and_edited(game),
        text(game.map_name.clone())
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
        text(game.display_hero())
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
    let card = card_shell(game.role, game.outcome, body.into(), theme::HEIGHT_COMPACT);
    if selected {
        container(card)
            .style(|_t| container::Style {
                border: iced::Border {
                    color: theme::ACCENT,
                    width: 2.0,
                    radius: theme::card_radius(),
                },
                ..container::Style::default()
            })
            .into()
    } else {
        card
    }
}

pub fn hero_card(hero: &HeroAgg) -> Element<'static, Message> {
    let wr = format!("{:.0}%", hero.record.win_rate_pct());
    let body = column![
        label_text(hero.role.label()),
        text(display_hero_name(&hero.hero))
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
        text(wr)
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        text(hero.record.games_label())
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
        win_bar(hero.record.win_rate()),
    ]
    .spacing(6);
    card_shell(hero.role, Outcome::Unknown, body.into(), theme::HEIGHT_HERO)
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
            text(record.games_label())
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
            col = col.push(map_row(m));
        }
    }
    container(col)
        .padding(PAD_INNER)
        .width(Fill)
        .style(theme::surface_panel)
        .into()
}

pub fn map_row(m: &MapAgg) -> Element<'static, Message> {
    column![
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
        text(format!(
            "{} · {}",
            m.record.games_label(),
            m.record.wl_label()
        ))
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_3),
        win_bar(m.record.win_rate()),
    ]
    .spacing(6)
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
        ]
        .spacing(8),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

pub fn empty_surface(copy: &str) -> Element<'static, Message> {
    container(
        text(copy.to_string())
            .size(SIZE_BODY)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

pub fn filter_chip(label: String, selected: bool, msg: Message) -> Element<'static, Message> {
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

pub fn expanded_game_card<'a>(
    game: &Game,
    editing: bool,
    edit: &'a EditForm,
    confirm_delete: bool,
) -> Element<'a, Message> {
    let sid = game.session_id.clone();
    let mut body = column![
        role_and_edited(game),
        text(game.map_name.clone())
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
        text(format!(
            "{}  ·  {}",
            game.display_hero(),
            game.played_at.format("%H:%M")
        ))
        .size(SIZE_BODY)
        .font(FONT_MEDIUM)
        .color(TEXT_2),
        outcome_label(game.outcome),
        stat_line(game),
        action_row(game, editing, confirm_delete),
    ]
    .spacing(10);

    let corr = game.corrections();
    if !corr.is_empty() {
        let mut block = column![label_text("Corrections")].spacing(4);
        for (label, ocr, fixed) in corr {
            block = block.push(
                text(format!("{label}: OCR {ocr} → {fixed}"))
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_2),
            );
        }
        body = body.push(block);
    }

    if game.show_timeline() {
        body = body.push(segment_list(game));
    }

    if editing {
        body = body.push(edit_form(edit, &sid));
    }

    // Column-hosted card: stripe uses the content's measured height (not a
    // Row shelf). Do not use height(Fill) on a stripe inside a shrink Row.
    container(
        row![
            container(space().width(STRIPE))
                .width(STRIPE)
                .style(theme::stripe(game.outcome)),
            container(body).padding(PAD_INNER).width(Fill),
        ]
        .spacing(0)
        .width(Fill),
    )
    .style(theme::role_card(game.role))
    .width(Fill)
    .clip(true)
    .into()
}

fn action_row(game: &Game, editing: bool, confirm_delete: bool) -> Element<'static, Message> {
    let sid = game.session_id.clone();
    let sid_del = sid.clone();
    row![
        label_text("Set outcome"),
        filter_chip(
            "Victory".into(),
            game.outcome == Outcome::Win,
            Message::SetOutcome {
                session_id: sid.clone(),
                outcome: Outcome::Win,
            },
        ),
        filter_chip(
            "Defeat".into(),
            game.outcome == Outcome::Loss,
            Message::SetOutcome {
                session_id: sid.clone(),
                outcome: Outcome::Loss,
            },
        ),
        filter_chip(
            "Draw".into(),
            game.outcome == Outcome::Draw,
            Message::SetOutcome {
                session_id: sid,
                outcome: Outcome::Draw,
            },
        ),
        space().width(Fill),
        button(
            text(if editing { "Cancel edit" } else { "Edit stats" })
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        )
        .padding(Padding::from([6, 14]))
        .style(theme::ghost_btn())
        .on_press(Message::ToggleEdit),
        button(
            text(if confirm_delete {
                "Click again to confirm"
            } else {
                "Delete session"
            })
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(if confirm_delete { TEXT } else { theme::DANGER }),
        )
        .padding(Padding::from([6, 14]))
        .style(theme::danger_btn(confirm_delete))
        .on_press(if confirm_delete {
            Message::DeleteSession(sid_del)
        } else {
            Message::ConfirmDelete(game.session_id.clone())
        }),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn segment_list(game: &Game) -> Element<'static, Message> {
    let mut col = column![label_text(
        "Hero timeline · confirm real swaps, dismiss misreads"
    )]
    .spacing(6);
    for seg in &game.segments {
        let sid = game.session_id.clone();
        let sid2 = sid.clone();
        col = col.push(
            row![
                text(seg.hero.clone())
                    .size(SIZE_BODY)
                    .font(FONT_SEMIBOLD)
                    .color(TEXT)
                    .width(Fill),
                text(seg.role.label())
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_2),
                text(format!("{} caps", seg.snapshots))
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3),
                text(seg.status_label())
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(TEXT_2),
                filter_chip(
                    "Confirm".into(),
                    seg.confirmed && !seg.dismissed,
                    Message::ResolveSegment {
                        session_id: sid,
                        segment: seg.index,
                        confirm: true,
                    },
                ),
                button(
                    text("Dismiss")
                        .size(SIZE_META)
                        .font(FONT_SEMIBOLD)
                        .color(TEXT),
                )
                .padding(Padding::from([6, 14]))
                .style(theme::danger_btn(seg.dismissed))
                .on_press(Message::ResolveSegment {
                    session_id: sid2,
                    segment: seg.index,
                    confirm: false,
                }),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    col.into()
}

fn edit_form<'a>(form: &'a EditForm, _sid: &str) -> Element<'a, Message> {
    column![
        label_text("Correct stats (blank / unchanged fields keep the OCR read)"),
        row![
            field_input("Hero", &form.hero, EditField::Hero),
            field_input("Role", &form.role, EditField::Role),
            field_input("Map", &form.map_name, EditField::Map),
        ]
        .spacing(8),
        row![
            field_input("Elims", &form.elims, EditField::Elims),
            field_input("Deaths", &form.deaths, EditField::Deaths),
            field_input("Assists", &form.assists, EditField::Assists),
        ]
        .spacing(8),
        row![
            field_input("Damage", &form.damage, EditField::Damage),
            field_input("Healing", &form.healing, EditField::Healing),
            field_input("Mitigation", &form.mitigation, EditField::Mitigation),
        ]
        .spacing(8),
        button(
            text("Save corrections")
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        )
        .padding(Padding::from([8, 16]))
        .style(theme::chip(true))
        .on_press(Message::SaveEdit),
    ]
    .spacing(10)
    .into()
}

fn field_input<'a>(label: &'static str, value: &'a str, field: EditField) -> Element<'a, Message> {
    column![
        label_text(label),
        text_input(label, value)
            .on_input(move |v| Message::EditField(field, v))
            .padding(Padding::from([8, 10]))
            .size(SIZE_BODY)
            .style(theme::text_input_style)
            .width(Fill),
    ]
    .spacing(4)
    .width(Fill)
    .into()
}

fn role_and_edited(game: &Game) -> Element<'static, Message> {
    let mut r = row![label_text(game.role.label())]
        .spacing(8)
        .align_y(Alignment::Center);
    if game.edited {
        r = r.push(
            container(
                text("edited")
                    .size(SIZE_LABEL)
                    .font(FONT_BOLD)
                    .color(theme::WARN),
            )
            .padding(Padding::from([2, 8]))
            .style(|_t| container::Style {
                background: Some(iced::Background::Color(theme::BG)),
                border: iced::Border {
                    color: theme::WARN,
                    width: 1.0,
                    radius: theme::RADIUS_CHIP.into(),
                },
                ..container::Style::default()
            }),
        );
    }
    r.into()
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

/// Fill / rest portions for a 0–1 win rate. Portions go on the *containers*
/// (not Shrink-wrapped `space()` children) or the bar stays ~50/50.
pub fn win_bar_portions(rate: f32) -> (u16, u16) {
    let fill = (rate.clamp(0.0, 1.0) * 100.0).round() as u16;
    let rest = 100u16.saturating_sub(fill);
    (fill, rest)
}

fn win_bar(rate: f32) -> Element<'static, Message> {
    let (fill, rest) = win_bar_portions(rate);
    let mut parts = Row::new().spacing(0).width(Fill).height(6);
    if fill > 0 {
        parts = parts.push(
            container(space().height(6))
                .width(Length::FillPortion(fill))
                .height(6)
                .style(|_| container::Style {
                    background: Some(iced::Background::Color(theme::OK)),
                    border: iced::Border {
                        radius: 999.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
        );
    }
    if rest > 0 {
        parts = parts.push(
            container(space().height(6))
                .width(Length::FillPortion(rest))
                .height(6)
                .style(|_| container::Style {
                    background: Some(iced::Background::Color(theme::BORDER)),
                    border: iced::Border {
                        radius: 999.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
        );
    }
    if fill == 0 && rest == 0 {
        parts = parts.push(
            container(space().height(6))
                .width(Fill)
                .height(6)
                .style(|_| container::Style {
                    background: Some(iced::Background::Color(theme::BORDER)),
                    border: iced::Border {
                        radius: 999.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
        );
    }
    container(parts).width(Fill).into()
}

fn card_shell(
    role: Role,
    outcome: Outcome,
    content: Element<'static, Message>,
    height: f32,
) -> Element<'static, Message> {
    // Fixed height + explicit stripe height (not Fill). A Fill-height stripe
    // inside a shrink-height Row resolves to 0 — featured worked only because
    // it sat in a Column that already had a measured height.
    container(
        row![
            container(space().width(STRIPE).height(height))
                .width(STRIPE)
                .height(height)
                .style(theme::stripe(outcome)),
            container(content)
                .padding(PAD_INNER)
                .width(Fill)
                .height(height),
        ]
        .spacing(0)
        .width(Fill)
        .height(height),
    )
    .style(theme::role_card(role))
    .width(Fill)
    .height(height)
    .clip(true)
    .into()
}

#[cfg(test)]
mod tests {
    use super::win_bar_portions;

    #[test]
    fn win_bar_portions_match_rate() {
        assert_eq!(win_bar_portions(0.0), (0, 100));
        assert_eq!(win_bar_portions(0.18), (18, 82));
        assert_eq!(win_bar_portions(0.5), (50, 50));
        assert_eq!(win_bar_portions(0.7), (70, 30));
        assert_eq!(win_bar_portions(1.0), (100, 0));
    }
}
