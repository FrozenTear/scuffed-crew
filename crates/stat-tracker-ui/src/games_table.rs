//! The 0.6 Games tab under the top tabs (variant A): session headers with a
//! form strip and summary, then one table per session (or one flat table),
//! with columns that fold away on narrow windows. Layout rules and labels
//! live in [`crate::games_rows`]; this module only draws them.

use iced::widget::{button, column, container, mouse_area, row, rule, space, text};
use iced::{Alignment, Background, Border, Color, Element, Fill, FillPortion, Length, Padding, Theme};

use crate::aggregate::group_by_session_gap;
use crate::app::{Message, TrackerApp};
use crate::games_rows::{self, Col, Grid, Grouping, TabState, Tick, Unsure, Width};
use crate::model::{Game, Outcome, Role, game_card_in_progress, local_hm};
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_EXTRABOLD, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META, TEXT,
    TEXT_2, TEXT_3, WARN,
};
use crate::widgets;

/// Games tab messages.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    SetGrouping(Grouping),
    FlipTimeSort,
    ToggleCompact,
    ToggleFilters,
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> iced::Task<Message> {
    let s = &mut app.games_tab;
    match msg {
        Msg::SetGrouping(g) => s.grouping = g,
        Msg::FlipTimeSort => s.sort = s.sort.flipped(),
        Msg::ToggleCompact => s.compact = !s.compact,
        Msg::ToggleFilters => s.filters_open = !s.filters_open,
    }
    iced::Task::none()
}

fn games_msg(m: Msg) -> Message {
    Message::Games(m)
}

/// Hero, map and outcome filters that are on.
fn active_filters(app: &TrackerApp) -> usize {
    usize::from(app.filter_hero.is_some())
        + usize::from(app.filter_map.is_some())
        + usize::from(app.filter_outcome.is_some())
}

pub fn view<'a>(
    app: &'a TrackerApp,
    content_width: f32,
    filters: Element<'a, Message>,
) -> Element<'a, Message> {
    let state = app.games_tab;
    let grid = Grid::for_width(content_width);
    let shown = crate::games::shown_games(app);
    let shown = games_rows::sorted(&shown, state.sort);

    let mut col = column![].spacing(16).width(Fill);
    col = col.push(controls(app, state));
    if state.filters_open {
        col = col.push(filters);
    }
    if app.needs_look_filter {
        col = col.push(row![widgets::filter_chip(
            crate::games::NEEDS_LOOK_FILTER_LABEL.into(),
            true,
            Message::Nav(crate::nav::Msg::ClearNeedsLook),
        )]);
    }
    if app.games.iter().any(|game| game.upload_reject.is_some()) {
        col = col.push(crate::games::retry_rejected_uploads_button());
    }
    if shown.is_empty() {
        let copy = if app.games.is_empty() {
            "No games yet. Games show here after the tracker reads a scoreboard."
        } else {
            "No games match these filters"
        };
        col = col.push(widgets::empty_surface(copy));
        return col.into();
    }

    let gap_secs = app.saved_config.session_window_secs;
    match state.grouping {
        Grouping::Sessions => {
            // Grouping walks newest first; the sort only orders rows inside.
            let mut newest = shown.clone();
            newest.sort_by(|a, b| b.played_at.cmp(&a.played_at));
            let gap = chrono::TimeDelta::seconds(gap_secs.max(1) as i64);
            let mut groups = group_by_session_gap(&newest, gap);
            if state.sort == games_rows::TimeSort::OldestFirst {
                groups.reverse();
            }
            for group in groups {
                let head = games_rows::session_head(&group);
                col = col.push(session_header(
                    head,
                    games_rows::form_ticks(&group.games),
                ));
                let refs: Vec<&Game> = group.games.iter().collect();
                let ordered = games_rows::sorted(&refs, state.sort);
                col = col.push(table(app, &ordered, grid, state));
            }
        }
        Grouping::Flat => {
            col = col.push(table(app, &shown, grid, state));
        }
    }
    col = col.push(
        text(games_rows::split_note(gap_secs))
            .size(SIZE_LABEL)
            .color(TEXT_3),
    );
    col.into()
}

fn controls(app: &TrackerApp, state: TabState) -> Element<'static, Message> {
    let filters = button(
        text(games_rows::filters_label(active_filters(app)))
            .size(SIZE_META)
            .font(FONT_SEMIBOLD),
    )
    .padding(Padding::from([6, 14]))
    .style(theme::chip(state.filters_open))
    .on_press(games_msg(Msg::ToggleFilters));
    let compact = button(text("Compact rows").size(SIZE_META).font(FONT_SEMIBOLD))
        .padding(Padding::from([6, 14]))
        .style(theme::chip(state.compact))
        .on_press(games_msg(Msg::ToggleCompact));
    let seg = |label: &'static str, g: Grouping| {
        button(text(label).size(SIZE_META).font(FONT_SEMIBOLD))
            .padding(Padding::from([5, 12]))
            .style(theme::filter_segment(ACCENT, state.grouping == g))
            .on_press(games_msg(Msg::SetGrouping(g)))
    };
    let segmented = container(row![seg("Sessions", Grouping::Sessions), seg("Flat list", Grouping::Flat)].spacing(2))
        .padding(2)
        .style(pill_box(BORDER));
    row![filters, space().width(Fill), compact, segmented]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Fill)
        .into()
}

fn session_header(head: games_rows::SessionHead, ticks: Vec<Tick>) -> Element<'static, Message> {
    row![
        text(head.title).size(15).font(FONT_EXTRABOLD).color(TEXT),
        form_strip(&ticks),
        space().width(Fill),
        text(head.summary).size(SIZE_META).color(TEXT_2),
    ]
    .spacing(12)
    .height(32)
    .align_y(Alignment::Center)
    .into()
}

/// 4 px ticks: 10 px up for a win, 10 px down for a loss, 4 px on the
/// midline for a draw or undecided game.
fn form_strip(ticks: &[Tick]) -> Element<'static, Message> {
    let mut r = row![].spacing(3).height(24).align_y(Alignment::Center);
    for t in ticks {
        let (top, h, color) = match t {
            Tick::Up => (2.0, 10.0, theme::OK),
            Tick::Down => (12.0, 10.0, theme::DANGER),
            Tick::Mid => (10.0, 4.0, TEXT_3),
        };
        r = r.push(
            column![
                space().height(top),
                container(space()).width(4).height(h).style(theme::stripe_color(color)),
            ]
            .width(4)
            .height(24),
        );
    }
    r.into()
}

fn cell_len(w: Width) -> Length {
    match w {
        Width::Px(px) => Length::Fixed(px),
        Width::Share(n) => FillPortion(n),
    }
}

fn table<'a>(app: &'a TrackerApp, games: &[&Game], grid: Grid, state: TabState) -> Element<'a, Message> {
    let cols = games_rows::columns(grid);
    let live = app.live_game_session.as_deref();
    let mut body = column![header_row(&cols, state)].width(Fill);
    for g in games {
        body = body.push(line());
        body = body.push(game_row(g, &cols, state));
        if app.expanded.as_deref() == Some(g.session_id.as_str()) {
            let confirm = app.confirm_delete.as_deref() == Some(g.session_id.as_str());
            let in_progress = game_card_in_progress(&g.session_id, g.outcome, live);
            body = body.push(container(widgets::expanded_game_card(
                g,
                app.editing && app.edit.session_id == g.session_id,
                &app.edit,
                confirm,
                in_progress,
                app.sessions_with_kit.contains(&g.session_id),
            ))
            .padding(12));
        }
    }
    container(body)
        .width(Fill)
        .clip(true)
        .style(theme::surface_panel)
        .into()
}

fn line() -> Element<'static, Message> {
    rule::horizontal(1)
        .style(|_: &Theme| rule::Style {
            color: BORDER,
            radius: 0.0.into(),
            fill_mode: rule::FillMode::Full,
            snap: true,
        })
        .into()
}

fn header_row(cols: &[(Col, Width)], state: TabState) -> Element<'static, Message> {
    let mut r = row![].spacing(12).height(games_rows::HEADER_H).align_y(Alignment::Center);
    for &(c, w) in cols {
        let cell: Element<'static, Message> = match c {
            Col::Time => button(
                text(format!("TIME {}", state.sort.arrow()))
                    .size(SIZE_LABEL)
                    .font(FONT_BOLD)
                    .color(ACCENT),
            )
            .padding(0)
            .style(|_: &Theme, _| button::Style::default())
            .on_press(games_msg(Msg::FlipTimeSort))
            .into(),
            _ => text(c.label())
                .size(SIZE_LABEL)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3)
                .into(),
        };
        r = r.push(container(cell).width(cell_len(w)));
    }
    container(r).padding(Padding { right: 12.0, ..Padding::ZERO }).into()
}

fn role_letter(role: Role) -> &'static str {
    match role {
        Role::Tank => "T",
        Role::Damage => "D",
        Role::Support => "S",
        Role::Unknown => "",
    }
}

fn game_row(g: &Game, cols: &[(Col, Width)], state: TabState) -> Element<'static, Message> {
    let h = state.row_height();
    let undecided = g.outcome == Outcome::Unknown;
    let main = if undecided { TEXT_3 } else { TEXT };
    let mut r = row![].spacing(12).height(h).align_y(Alignment::Center);
    for &(c, w) in cols {
        let cell: Element<'static, Message> = match c {
            Col::Stripe => container(space())
                .width(4)
                .height(h - 16.0)
                .style(theme::stripe(g.outcome))
                .into(),
            Col::Result => text(games_rows::result_label(g.outcome))
                .size(SIZE_META)
                .font(if undecided { FONT_SEMIBOLD } else { FONT_BOLD })
                .color(theme::outcome_color(g.outcome))
                .into(),
            Col::Map => {
                let name = if g.map_name.trim().is_empty() {
                    "No map".to_string()
                } else {
                    g.map_name.clone()
                };
                let mut m = row![text(name).size(SIZE_META).font(FONT_SEMIBOLD).color(main)]
                    .spacing(8)
                    .align_y(Alignment::Center);
                if let Some(mode) = games_rows::mode_chip(&g.map_name) {
                    m = m.push(small_chip(mode, TEXT_2, BORDER));
                }
                m.into()
            }
            Col::Hero => row![
                text(g.display_hero()).size(SIZE_META).color(TEXT_2),
                text(role_letter(g.role))
                    .size(SIZE_LABEL)
                    .font(FONT_BOLD)
                    .color(theme::role_color(g.role)),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
            Col::Eda => text(games_rows::eda(g)).size(SIZE_META).color(TEXT_2).into(),
            Col::Rank | Col::Length | Col::Crew => text("-").size(SIZE_META).color(TEXT_3).into(),
            Col::Time => text(local_hm(g.played_at)).size(SIZE_META).color(TEXT_2).into(),
            Col::Unsure => match games_rows::unsure(g) {
                Some(u) => {
                    let chip = small_chip_owned(u.label(), WARN, WARN);
                    if u == Unsure::NoOutcome {
                        mouse_area(chip)
                            .on_press(Message::ToggleGame(g.session_id.clone()))
                            .into()
                    } else {
                        chip
                    }
                }
                None => space().into(),
            },
        };
        r = r.push(container(cell).width(cell_len(w)).clip(true));
    }
    mouse_area(container(r).padding(Padding { right: 12.0, ..Padding::ZERO }))
        .on_press(Message::ToggleGame(g.session_id.clone()))
        .into()
}

fn pill_box(border: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(theme::BG)),
        border: Border {
            color: border,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        ..container::Style::default()
    }
}

fn small_chip(label: &'static str, color: Color, border: Color) -> Element<'static, Message> {
    small_chip_owned(label.to_string(), color, border)
}

fn small_chip_owned(label: String, color: Color, border: Color) -> Element<'static, Message> {
    container(text(label).size(SIZE_LABEL).font(FONT_SEMIBOLD).color(color))
        .padding(Padding::from([1, 8]))
        .style(move |_: &Theme| container::Style {
            border: Border {
                color: border,
                width: 1.0,
                radius: theme::RADIUS_CHIP.into(),
            },
            ..container::Style::default()
        })
        .into()
}
