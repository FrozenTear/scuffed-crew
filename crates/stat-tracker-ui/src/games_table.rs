//! The 0.6 Games tab under the top tabs (variant A): session headers with a
//! form strip and summary, then one table per session (or one flat table),
//! with columns that fold away on narrow windows. Layout rules and labels
//! live in [`crate::games_rows`]; this module only draws them. The filter
//! chips, the sortable headers and the row cells (amber E/D/A, portrait
//! and role letter, Playing, the marks) are ported from #555.

use iced::widget::{button, column, container, image, mouse_area, row, rule, space, text};
use iced::{
    Alignment, Background, Border, Color, Element, Fill, FillPortion, Length, Padding, Theme,
};

use crate::aggregate::group_by_session_gap;
use crate::app::{Message, TrackerApp};
use crate::games_rows::{self, Col, Grid, Grouping, Mark, TabState, Tick, Width};
use crate::icons::{self, Icon};
use crate::model::{Game, Outcome, Role, RoleFilter, Screen, game_card_in_progress, local_hm};
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_EXTRABOLD, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META, TEXT,
    TEXT_2, TEXT_3, WARN,
};
use crate::widgets;

/// Games tab messages.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    SetGrouping(Grouping),
    /// A column header: sort by it, or flip its direction.
    SortBy(Col),
    /// The role chip: one role, or all (`None`).
    PickRole(Option<Role>),
    ToggleCompact,
    ToggleFilters,
    /// Esc, or Enter: close the open row.
    CloseOpen,
    /// Open a game's saved scoreboard picture.
    OpenScoreboard(String),
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> iced::Task<Message> {
    let s = &mut app.games_tab;
    match msg {
        Msg::SetGrouping(g) => s.grouping = g,
        Msg::SortBy(col) => s.sort = s.sort.clicked(col),
        Msg::PickRole(role) => {
            app.roles = role.map_or_else(RoleFilter::default, |r| RoleFilter::default().toggle(r));
        }
        Msg::ToggleCompact => s.compact = !s.compact,
        Msg::ToggleFilters => s.filters_open = !s.filters_open,
        Msg::CloseOpen => {
            app.expanded = None;
            app.editing = false;
            app.confirm_delete = None;
        }
        Msg::OpenScoreboard(sid) => crate::games_panel::open_scoreboard(&app.data_dir, &sid),
    }
    iced::Task::none()
}

fn games_msg(m: Msg) -> Message {
    Message::Games(m)
}

pub fn view<'a>(
    app: &'a TrackerApp,
    content_width: f32,
    heroes: &[String],
    maps: &[String],
) -> Element<'a, Message> {
    let state = app.games_tab;
    let grid = Grid::for_width(content_width);
    let mut shown = crate::games::shown_games(app);
    games_rows::sort_games(&mut shown, state.sort);

    let mut col = column![].spacing(16).width(Fill);
    col = col.push(controls(app, state));
    let chips = crate::filter_chips::bar(app, heroes, maps, state.filters_open);
    if state.filters_open || crate::filter_chips::active_count(app) > 0 {
        col = col.push(chips);
    }
    // The spec does not place "Retry rejected uploads" (UX Spec v2 open
    // question 5), so it waits under Filters instead of above the table.
    if state.filters_open && app.games.iter().any(|game| game.upload_reject.is_some()) {
        col = col.push(crate::games::retry_rejected_uploads_button());
    }
    if shown.is_empty() {
        let copy = if app.games.is_empty() {
            games_rows::EMPTY_NONE
        } else {
            games_rows::EMPTY_FILTERED
        };
        col = col.push(widgets::empty_surface(copy));
        return col.into();
    }

    let gap_secs = app.saved_config.session_window_secs;
    match state.grouping {
        Grouping::Sessions => {
            // Grouping walks newest first; the sort only orders rows inside.
            let mut newest = shown.clone();
            newest.sort_by_key(|g| std::cmp::Reverse(g.played_at));
            let gap = chrono::TimeDelta::seconds(gap_secs.max(1) as i64);
            let mut groups = group_by_session_gap(&newest, gap);
            if state.sort.oldest_first() {
                groups.reverse();
            }
            for group in groups {
                let head = games_rows::session_head(&group);
                col = col.push(session_header(head, games_rows::form_ticks(&group.games)));
                let mut ordered: Vec<&Game> = group.games.iter().collect();
                games_rows::sort_games(&mut ordered, state.sort);
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
        text(games_rows::filters_label(
            crate::filter_chips::active_count(app),
        ))
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
    let segmented = container(
        row![
            seg("Sessions", Grouping::Sessions),
            seg("Flat list", Grouping::Flat)
        ]
        .spacing(2),
    )
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
                container(space())
                    .width(4)
                    .height(h)
                    .style(theme::stripe_color(color)),
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

fn table<'a>(
    app: &'a TrackerApp,
    games: &[&Game],
    grid: Grid,
    state: TabState,
) -> Element<'a, Message> {
    let cols = games_rows::columns(grid);
    let live = app.live_game_session.as_deref();
    let mut body = column![header_row(&cols, state)].width(Fill);
    for g in games {
        body = body.push(line());
        let in_progress = game_card_in_progress(&g.session_id, g.outcome, live);
        let open = app.expanded.as_deref() == Some(g.session_id.as_str());
        body = body.push(game_row(g, &cols, state, in_progress, open));
        if open {
            body = body.push(crate::games_panel::panel(
                app,
                g,
                grid == Grid::Narrow,
                in_progress,
            ));
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
    let mut r = row![]
        .spacing(12)
        .height(games_rows::HEADER_H)
        .align_y(Alignment::Center);
    for &(c, w) in cols {
        let cell: Element<'static, Message> = if c.sortable() {
            let active = state.sort.col == c;
            let label = if active {
                format!("{} {}", c.label(), state.sort.arrow())
            } else {
                c.label().to_string()
            };
            button(
                text(label)
                    .size(SIZE_LABEL)
                    .font(FONT_BOLD)
                    .color(if active { ACCENT } else { TEXT_3 }),
            )
            .padding(0)
            .style(|_: &Theme, _| button::Style::default())
            .on_press(games_msg(Msg::SortBy(c)))
            .into()
        } else {
            text(c.label())
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(TEXT_3)
                .into()
        };
        r = r.push(container(cell).width(cell_len(w)));
    }
    // The chevron column.
    r = r.push(space().width(CHEVRON));
    container(r)
        .padding(Padding {
            right: 16.0,
            ..Padding::ZERO
        })
        .into()
}

fn role_letter(role: Role) -> &'static str {
    match role {
        Role::Tank => "T",
        Role::Damage => "D",
        Role::Support => "S",
        Role::Unknown => "",
    }
}

/// The chevron's column at the row's end.
const CHEVRON: f32 = 14.0;

/// One game row; the open one has an accent outline and a down chevron.
fn game_row(
    g: &Game,
    cols: &[(Col, Width)],
    state: TabState,
    in_progress: bool,
    open: bool,
) -> Element<'static, Message> {
    let h = state.row_height();
    let undecided = g.outcome == Outcome::Unknown && !in_progress;
    let main = if undecided { TEXT_3 } else { TEXT };
    let mut r = row![].spacing(12).height(h).align_y(Alignment::Center);
    for &(c, w) in cols {
        let cell: Element<'static, Message> = match c {
            Col::Stripe => container(space())
                .width(4)
                .height(h - 16.0)
                .style(theme::stripe_color(if in_progress {
                    ACCENT
                } else {
                    theme::outcome_color(g.outcome)
                }))
                .into(),
            Col::Result => result_cell(g, in_progress),
            Col::Map => {
                let name = if g.map_name.trim().is_empty() {
                    "No map".to_string()
                } else {
                    g.map_name.clone()
                };
                let mut m = row![text(name).size(SIZE_META).font(FONT_BOLD).color(main)]
                    .spacing(8)
                    .align_y(Alignment::Center);
                if let Some(mode) = games_rows::mode_chip(&g.map_name) {
                    m = m.push(small_chip(mode, TEXT_2, BORDER));
                }
                m.into()
            }
            Col::Hero => hero_cell(g),
            Col::Eda => text(games_rows::eda(g))
                .size(SIZE_META)
                .color(if games_rows::eda_unsure(g) {
                    WARN
                } else {
                    TEXT_2
                })
                .into(),
            Col::Rank | Col::Length | Col::Crew => text("-").size(SIZE_META).color(TEXT_3).into(),
            Col::Time => text(local_hm(g.played_at))
                .size(SIZE_META)
                .color(TEXT_2)
                .into(),
            Col::Unsure => marks_cell(g, in_progress),
        };
        r = r.push(container(cell).width(cell_len(w)).clip(true));
    }
    r = r.push(
        image(icons::handle(if open {
            Icon::FoldOpen
        } else {
            Icon::FoldClosed
        }))
        .width(CHEVRON)
        .height(CHEVRON),
    );
    let outline = if open { ACCENT } else { Color::TRANSPARENT };
    mouse_area(
        container(r)
            .padding(Padding {
                right: 16.0,
                ..Padding::ZERO
            })
            .style(move |_: &Theme| container::Style {
                border: Border {
                    color: outline,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }),
    )
    .on_press(Message::ToggleGame(g.session_id.clone()))
    .into()
}

/// WIN, LOSS, DRAW, Undecided, or "Playing" in the accent while the game is
/// still on.
fn result_cell(g: &Game, in_progress: bool) -> Element<'static, Message> {
    let color = if in_progress {
        ACCENT
    } else {
        theme::outcome_color(g.outcome)
    };
    let undecided = g.outcome == Outcome::Unknown && !in_progress;
    text(games_rows::result_text(g.outcome, in_progress))
        .size(SIZE_META)
        .font(if undecided { FONT_SEMIBOLD } else { FONT_BOLD })
        .color(color)
        .into()
}

/// A grey silhouette for the portrait (no Blizzard art is shipped), the
/// hero, and the role's letter in its colour.
fn hero_cell(g: &Game) -> Element<'static, Message> {
    row![
        portrait(20.0, 14.0),
        text(g.display_hero()).size(SIZE_META).color(TEXT_2),
        text(role_letter(g.role))
            .size(SIZE_LABEL)
            .font(FONT_BOLD)
            .color(theme::role_color(g.role)),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

/// The grey placeholder portrait: a `size` square with a silhouette.
pub(crate) fn portrait(size: f32, icon: f32) -> Element<'static, Message> {
    container(
        image(icons::handle(Icon::Tab(Screen::Heroes, false)))
            .width(icon)
            .height(icon),
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

/// The row's marks: amber warning chips, then the red upload chip. The
/// no-outcome chip opens the game to set one.
fn marks_cell(g: &Game, in_progress: bool) -> Element<'static, Message> {
    let mut r = row![].spacing(6).align_y(Alignment::Center);
    for mark in games_rows::marks(g, in_progress) {
        let chip = if mark.is_warning() {
            warn_chip(mark.label())
        } else {
            small_chip_owned(mark.label(), theme::DANGER, theme::DANGER)
        };
        r = r.push(if mark == Mark::NoOutcome {
            mouse_area(chip)
                .on_press(Message::ToggleGame(g.session_id.clone()))
                .into()
        } else {
            chip
        });
    }
    r.into()
}

/// An amber outline chip with the warning sign.
pub(crate) fn warn_chip(label: String) -> Element<'static, Message> {
    container(
        row![
            image(icons::handle(Icon::Warn)).width(11).height(11),
            text(label).size(SIZE_LABEL).font(FONT_SEMIBOLD).color(WARN),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([1, 8]))
    .style(move |_: &Theme| container::Style {
        border: Border {
            color: WARN,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        ..container::Style::default()
    })
    .into()
}

pub(crate) fn pill_box(border: Color) -> impl Fn(&Theme) -> container::Style {
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

pub(crate) fn small_chip(
    label: &'static str,
    color: Color,
    border: Color,
) -> Element<'static, Message> {
    small_chip_owned(label.to_string(), color, border)
}

fn small_chip_owned(label: String, color: Color, border: Color) -> Element<'static, Message> {
    container(
        text(label)
            .size(SIZE_LABEL)
            .font(FONT_SEMIBOLD)
            .color(color),
    )
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
