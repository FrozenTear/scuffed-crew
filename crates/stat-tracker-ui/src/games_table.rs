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

use crate::app::{Message, TrackerApp};
use crate::games_rows::{self, Col, Grid, Mark, TabState, Tick, UnsureItem, Width};
use crate::icons::{self, Icon};
use crate::model::{Game, Outcome, Role, Screen, game_card_in_progress};
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_EXTRABOLD, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META, TEXT,
    TEXT_2, TEXT_3, WARN,
};
use crate::widgets;

/// Games tab messages.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    SetGrouping(games_rows::Grouping),
    /// A column header: sort by it, or flip its direction.
    SortBy(Col),
    ToggleCompact,
    ToggleFilters,
    /// Open a game (a mark that asks for something); an open one stays open.
    Open(String),
    /// Esc no widget took: cancel the edit, else close the open row.
    Escape,
    /// Enter no widget took: close the open row unless the edit form is up.
    Enter,
    /// Open a game's saved scoreboard picture.
    OpenScoreboard(String),
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> iced::Task<Message> {
    let s = &mut app.games_tab;
    match msg {
        Msg::SetGrouping(g) => s.grouping = g,
        Msg::SortBy(col) => s.sort = s.sort.clicked(col),
        Msg::ToggleCompact => s.compact = !s.compact,
        Msg::ToggleFilters => s.filters_open = !s.filters_open,
        Msg::Open(sid) => {
            if app.expanded.as_deref() != Some(sid.as_str()) {
                return iced::Task::done(Message::ToggleGame(sid));
            }
        }
        Msg::Escape if app.editing => app.editing = false,
        Msg::Enter if app.editing => {}
        Msg::Escape | Msg::Enter => {
            app.expanded = None;
            app.editing = false;
            app.confirm_delete = None;
        }
        Msg::OpenScoreboard(sid) => {
            if let Err(e) = crate::games_panel::open_scoreboard(&app.data_dir, &sid) {
                app.replace_toast(e);
            }
        }
    }
    iced::Task::none()
}

fn games_msg(m: Msg) -> Message {
    Message::Games(m)
}

/// The tab at `window_width` (the grid follows the window, as the spec's
/// breakpoints do; the gutter changes at 1440 too).
pub fn view<'a>(
    app: &'a TrackerApp,
    window_width: f32,
    heroes: &[String],
    maps: &[String],
) -> Element<'a, Message> {
    let state = app.games_tab;
    let grid = Grid::for_window(window_width);
    let shown = crate::games::shown_games(app);

    let mut col = column![].spacing(16).width(Fill);
    col = col.push(controls(app, state));
    let chips = crate::filter_chips::bar(app, heroes, maps, state.filters_open);
    if state.filters_open || crate::filter_chips::active_count(app) > 0 {
        col = col.push(chips);
    }
    // UX Spec v2 open question 5 does not place it yet; above the table it
    // is seen, where under Filters it was not.
    if app.games.iter().any(|game| game.upload_reject.is_some()) {
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
    let live = app.live_game_session.as_deref();
    let layout = games_rows::layout(&app.games, &shown, state, grid, gap_secs, live);
    for section in layout.sections {
        if let Some(head) = section.head {
            col = col.push(session_header(head, section.ticks));
        }
        col = col.push(table(app, &section.rows, grid, state));
    }
    if let Some(note) = games_rows::cap_note(layout.hidden) {
        col = col.push(text(note).size(SIZE_META).color(TEXT_2));
    }
    col = col.push(
        text(games_rows::list_note(state.grouping, gap_secs))
            .size(SIZE_LABEL)
            .color(TEXT_3),
    );
    col.into()
}

fn controls(app: &TrackerApp, state: TabState) -> Element<'static, Message> {
    let filters = button(
        row![
            text(games_rows::filters_label(
                crate::filter_chips::active_count(app),
            ))
            .size(SIZE_META)
            .font(FONT_SEMIBOLD),
            crate::settings_fixed::arrow().size(SIZE_META),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([6, 14]))
    .style(theme::chip(state.filters_open))
    .on_press(games_msg(Msg::ToggleFilters));
    let compact = button(text("Compact rows").size(SIZE_META).font(FONT_SEMIBOLD))
        .padding(Padding::from([6, 14]))
        .style(theme::chip(state.compact))
        .on_press(games_msg(Msg::ToggleCompact));
    let seg = |label: &'static str, g: games_rows::Grouping| {
        button(text(label).size(SIZE_META).font(FONT_SEMIBOLD))
            .padding(Padding::from([5, 12]))
            .style(theme::filter_segment(ACCENT, state.grouping == g))
            .on_press(games_msg(Msg::SetGrouping(g)))
    };
    let segmented = container(
        row![
            seg("Sessions", games_rows::Grouping::Sessions),
            seg("Flat list", games_rows::Grouping::Flat)
        ]
        .spacing(2),
    )
    .padding(2)
    .style(pill_box(BORDER));
    row![
        filters,
        space().width(Fill),
        widgets::overlay_toggle(app.companion_chip()),
        compact,
        segmented
    ]
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
    let mut body = column![header_row(&cols, state, grid)].width(Fill);
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

fn header_row(cols: &[(Col, Width)], state: TabState, grid: Grid) -> Element<'static, Message> {
    let sort = state.sort.visible_in(grid);
    let mut r = row![]
        .spacing(12)
        .height(games_rows::HEADER_H)
        .align_y(Alignment::Center);
    for &(c, w) in cols {
        let cell: Element<'static, Message> = if c.sortable() {
            let active = sort.col == c;
            let color = if active { ACCENT } else { TEXT_3 };
            let mut label = row![
                text(c.label())
                    .size(SIZE_LABEL)
                    .font(FONT_BOLD)
                    .color(color)
            ]
            .spacing(4)
            .align_y(Alignment::Center);
            if active {
                label = label.push(
                    crate::settings_fixed::sort_arrow(sort.desc)
                        .size(SIZE_LABEL)
                        .color(color),
                );
            }
            button(label)
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
    let items = games_rows::unsure_items(g, in_progress);
    let unsure = |item| items.contains(&item);
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
            Col::Result => result_cell(g, in_progress, unsure(UnsureItem::Result)),
            Col::Map => {
                let name = if g.map_name.trim().is_empty() {
                    "No map".to_string()
                } else {
                    g.map_name.clone()
                };
                let doubt = unsure(UnsureItem::Map);
                let mut m = row![text(name).size(SIZE_META).font(FONT_BOLD).color(if doubt {
                    WARN
                } else {
                    main
                })]
                .spacing(8)
                .align_y(Alignment::Center);
                if let Some(mode) = games_rows::mode_chip(&g.map_name) {
                    m = m.push(small_chip(mode, TEXT_2, BORDER));
                }
                with_warn(m.into(), doubt)
            }
            Col::Hero => hero_cell(g, unsure(UnsureItem::Hero)),
            Col::Eda => text(games_rows::eda(g))
                .size(SIZE_META)
                .color(if games_rows::eda_unsure(g) {
                    WARN
                } else {
                    TEXT_2
                })
                .into(),
            Col::Rank | Col::Length | Col::Crew => text("-").size(SIZE_META).color(TEXT_3).into(),
            Col::Time => {
                let (date, hm) = games_rows::time_cell_in(g, state.grouping, &chrono::Local);
                let hm = text(hm).size(SIZE_META).color(TEXT_2);
                match date {
                    Some(d) => column![text(d).size(SIZE_LABEL).color(TEXT_3), hm].into(),
                    None => hm.into(),
                }
            }
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
/// The result read in doubt keeps its colour and gets the warning sign.
fn result_cell(g: &Game, in_progress: bool, unsure: bool) -> Element<'static, Message> {
    let color = if in_progress {
        ACCENT
    } else {
        theme::outcome_color(g.outcome)
    };
    let undecided = g.outcome == Outcome::Unknown && !in_progress;
    with_warn(
        text(games_rows::result_text(g.outcome, in_progress))
            .size(SIZE_META)
            .font(if undecided { FONT_SEMIBOLD } else { FONT_BOLD })
            .color(color)
            .into(),
        unsure,
    )
}

/// `cell` with the amber warning sign after it when its read is unsure
/// (the reason a row sorts last on that column).
fn with_warn(cell: Element<'static, Message>, unsure: bool) -> Element<'static, Message> {
    if !unsure {
        return cell;
    }
    row![cell, image(icons::handle(Icon::Warn)).width(12).height(12)]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
}

/// A grey silhouette for the portrait (no Blizzard art is shipped), the
/// hero, and the role's letter in its colour. An unsure hero is amber.
fn hero_cell(g: &Game, unsure: bool) -> Element<'static, Message> {
    let r = row![
        portrait(20.0, 14.0),
        text(g.display_hero())
            .size(SIZE_META)
            .color(if unsure { WARN } else { TEXT_2 }),
        text(role_letter(g.role))
            .size(SIZE_LABEL)
            .font(FONT_BOLD)
            .color(theme::role_color(g.role)),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    with_warn(r.into(), unsure)
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

/// The row's one mark ([`games_rows::mark`]). The no-outcome chip opens
/// the game to set one; on an open row it does nothing more.
fn marks_cell(g: &Game, in_progress: bool) -> Element<'static, Message> {
    let Some(mark) = games_rows::mark(g, in_progress) else {
        return space().into();
    };
    let chip = if mark.is_warning() {
        warn_chip(mark.label())
    } else {
        small_chip_owned(mark.label(), theme::DANGER, theme::DANGER)
    };
    if mark == Mark::NoOutcome {
        mouse_area(chip)
            .on_press(games_msg(Msg::Open(g.session_id.clone())))
            .into()
    } else {
        chip
    }
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
