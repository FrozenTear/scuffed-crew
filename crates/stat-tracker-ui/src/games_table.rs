//! The 0.6 Games tab under the top tabs (variant A): session headers with a
//! form strip and summary, then one table per session (or one flat table),
//! with columns that fold away on narrow windows. Layout rules and labels
//! live in [`crate::games_rows`]; this module only draws them: the sortable
//! headers and the row cells (amber E/D/A, portrait and role letter,
//! Playing, the marks). The filter chips live in [`crate::filter_chips`].

use iced::widget::{button, column, container, image, mouse_area, row, rule, space, text};
use iced::{
    Alignment, Background, Border, Color, Element, Fill, FillPortion, Length, Padding, Theme,
};

use crate::app::{Message, TrackerApp};
use crate::games_rows::{self, Col, Focus, Grid, Mark, TabState, Tick, UnsureItem, Width};
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
    /// A row clicked: open it, or close it when it is the open one.
    Toggle(String),
    /// Delete game: show the confirm strip, focus on Cancel.
    AskDelete(String),
    /// The strip's Cancel: close it, focus back on Delete game.
    CancelDelete,
    /// The strip's Delete game.
    Delete(String),
    /// Tab no widget took: Cancel and Delete game in the strip.
    Tab,
    /// Space no widget took: press the focused button.
    Space,
    /// The status box's Retry upload.
    Retry(String),
    /// "N games couldn't upload · Retry all" above the table, while any
    /// game is rejected.
    RetryAll,
    /// Delete no widget took: the open or focused row's delete confirm.
    DeleteKey,
    /// The Set map or Set hero picker.
    Picker(crate::games_picker::Msg),
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> iced::Task<Message> {
    // A picker that Edit, the delete confirm or another row replaced is
    // gone: it must not keep taking Enter and Delete.
    if !matches!(msg, Msg::Picker(crate::games_picker::Msg::Open(..)))
        && crate::games_picker::open_picker(app).is_none()
    {
        app.games_tab.picker = None;
    }
    // The one hidden-row check for a tab message (`before_message` skips
    // these). An open row left undrawn now is one in Edit: no key acts on it.
    let undrawn_edit = drop_hidden_open_row(app);
    if undrawn_edit && is_key(&msg) {
        return iced::Task::none();
    }
    match msg {
        Msg::SetGrouping(g) => app.games_tab.view.grouping = g,
        // The click acts on the sort the header shows: under 1100 px a Hero
        // or E/D/A sort reads as TIME, so a TIME click flips that.
        Msg::SortBy(col) => {
            let shown = app.games_tab.view.sort.visible_in(app.games_tab.grid.get());
            app.games_tab.view.sort = shown.clicked(col);
        }
        Msg::ToggleCompact => app.games_tab.view.compact = !app.games_tab.view.compact,
        Msg::ToggleFilters => app.games_tab.view.filters_open = !app.games_tab.view.filters_open,
        Msg::Open(sid) => {
            if app.expanded.as_deref() != Some(sid.as_str()) {
                return update(app, Msg::Toggle(sid));
            }
        }
        Msg::Toggle(sid) => {
            app.games_tab.focus = Focus::None;
            app.games_tab.picker = None;
            app.games_tab.deleted_note = None;
            return iced::Task::done(Message::ToggleGame(sid));
        }
        Msg::AskDelete(sid) => {
            app.games_tab.picker = None;
            app.games_tab.focus = Focus::Cancel;
            app.confirm_delete = Some(sid);
        }
        Msg::CancelDelete => cancel_delete(app),
        Msg::Delete(sid) => delete(app, &sid),
        // Enter goes to an open picker, so Tab leaves the focus ring where
        // it is rather than put it on Delete game.
        Msg::Tab if app.games_tab.picker.is_some() => {}
        Msg::Tab => {
            let focus = &mut app.games_tab.focus;
            match focus {
                Focus::Cancel => *focus = Focus::ConfirmDelete,
                Focus::ConfirmDelete => *focus = Focus::Cancel,
                // The panel's other controls are iced widgets with no
                // focus of their own, so Delete game is its one Tab stop.
                Focus::DeleteButton => *focus = Focus::None,
                _ if app.expanded.is_some() && !app.editing => *focus = Focus::DeleteButton,
                _ => {}
            }
        }
        Msg::Picker(m) => return crate::games_picker::update(app, m),
        Msg::Escape if app.games_tab.picker.is_some() => app.games_tab.picker = None,
        // Enter picks the highlighted name, also once a click took the
        // focus from the search box; Delete is the search box's.
        Msg::Enter if app.games_tab.picker.is_some() => {
            return crate::games_picker::update(app, crate::games_picker::Msg::Submit);
        }
        Msg::DeleteKey if app.games_tab.picker.is_some() => {}
        Msg::Escape if app.confirm_delete.is_some() => cancel_delete(app),
        Msg::Escape if app.editing => app.editing = false,
        Msg::Enter | Msg::Space if press_focused(app) => {}
        Msg::Space => {}
        Msg::Enter if app.editing => {}
        Msg::Escape | Msg::Enter if app.expanded.is_none() => app.games_tab.focus = Focus::None,
        Msg::Escape | Msg::Enter => {
            app.expanded = None;
            app.editing = false;
            app.confirm_delete = None;
            app.games_tab.focus = Focus::None;
        }
        Msg::DeleteKey => delete_key(app),
        Msg::Retry(sid) => crate::games_retry::retry(app, Some(&sid)),
        Msg::RetryAll => crate::games_retry::retry(app, None),
        Msg::OpenScoreboard(sid) => {
            if let Err(e) = crate::games_panel::open_scoreboard(&app.data_dir, &sid) {
                app.replace_toast(e);
            }
        }
    }
    iced::Task::none()
}

/// Leaving the Games tab: close the open row, its edit and delete strip,
/// and take down the "Game deleted." line, the ring after a delete and an
/// open picker, so none of them comes back with the tab.
pub fn leave(app: &mut TrackerApp) {
    app.expanded = None;
    app.editing = false;
    app.confirm_delete = None;
    app.games_tab.deleted_note = None;
    clear_focus(app);
}

/// Drop the tab's focus and picker. Edit calls it too: a Delete game
/// focus kept into the form would let Enter open the delete strip over it.
pub fn clear_focus(app: &mut TrackerApp) {
    app.games_tab.focus = Focus::None;
    app.games_tab.picker = None;
}

/// Run before each app message: take a stale "Game deleted." down and,
/// on any message but the 1 s tick, run the undrawn-row check
/// ([`drop_hidden_open_row`]: a row the filters hid, one past the first
/// 500 rows, or a game that left the list). A tick can reload the list,
/// so a game may leave it or go past the cap on one, but no key comes
/// with a tick and the next message runs the check first, so skipping it
/// there is safe and saves building the list. A Games tab message skips
/// it too, as [`update`] runs it once for that message.
pub(crate) fn before_message(app: &mut TrackerApp, message: &Message) {
    crate::games_delete::drop_stale_note(app);
    if !matches!(message, Message::Tick | Message::Games(_)) {
        drop_hidden_open_row(app);
    }
}

/// Drop a row focus that is not drawn now, and close an open row that is
/// not drawn, so a key never acts on a row the player can't see. Only the
/// top tabs take these keys, so the classic Games page keeps its open
/// card. Drawn is [`row_order`]: a row the filters hid, a row past the
/// first 500 and a game that left the list (an untracked map after a
/// reload) are all off screen. An open Edit stays up on an undrawn row so
/// the typed text comes back with the row (or a save can say the game is
/// gone), but its delete strip, picker and delete focus go. Returns
/// whether that is the case: an Edit left open on an undrawn row, which
/// [`update`] gives no key. The row list is built only when a row is open
/// or focused.
pub(crate) fn drop_hidden_open_row(app: &mut TrackerApp) -> bool {
    let focused_row = matches!(app.games_tab.focus, Focus::Row(_));
    if app.nav_style != crate::nav::NavStyle::TopTabs || (app.expanded.is_none() && !focused_row) {
        return false;
    }
    let drawn = row_order(app);
    let hidden = |sid: &str| !drawn.iter().any(|s| s == sid);
    if matches!(&app.games_tab.focus, Focus::Row(sid) if hidden(sid)) {
        app.games_tab.focus = Focus::None;
    }
    if !app.expanded.as_deref().is_some_and(hidden) {
        return false;
    }
    if !app.editing {
        app.expanded = None;
    }
    app.confirm_delete = None;
    app.games_tab.picker = None;
    if matches!(
        app.games_tab.focus,
        Focus::DeleteButton | Focus::Cancel | Focus::ConfirmDelete
    ) {
        app.games_tab.focus = Focus::None;
    }
    app.editing
}

/// The keys no widget took, which [`update`] refuses while an Edit is
/// left open on an undrawn row: Esc can't drop its form, and Tab, Enter,
/// Space and Delete can't reach it.
fn is_key(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::Escape | Msg::Enter | Msg::Tab | Msg::Space | Msg::DeleteKey
    )
}

/// Enter or Space on the focused button or row. False when nothing that
/// takes them has the focus.
fn press_focused(app: &mut TrackerApp) -> bool {
    let open = app.expanded.clone();
    match (app.games_tab.focus.clone(), open) {
        (Focus::Cancel, _) if app.confirm_delete.is_some() => cancel_delete(app),
        (Focus::ConfirmDelete, _) if let Some(sid) = app.confirm_delete.clone() => {
            delete(app, &sid);
        }
        (Focus::DeleteButton, Some(sid)) => {
            app.games_tab.focus = Focus::Cancel;
            app.confirm_delete = Some(sid);
        }
        (Focus::Row(sid), None) => {
            app.games_tab.focus = Focus::None;
            app.games_tab.deleted_note = None;
            app.expanded = Some(sid);
            app.editing = false;
            // As a click (`ToggleGame`): a kit written since the last
            // snapshot shows in the panel.
            app.refresh_kit_sessions();
        }
        _ => return false,
    }
    true
}

/// Delete on the open row, or on the focused one (opened first): the
/// inline confirm with Cancel focused. It never deletes by itself, and it
/// does nothing over the edit form.
fn delete_key(app: &mut TrackerApp) {
    if app.editing {
        return;
    }
    let sid = match (&app.expanded, &app.games_tab.focus) {
        (Some(sid), _) | (None, Focus::Row(sid)) => sid.clone(),
        _ => return,
    };
    if app.expanded.is_none() {
        app.games_tab.deleted_note = None;
        app.expanded = Some(sid.clone());
        app.refresh_kit_sessions();
    }
    app.games_tab.picker = None;
    app.confirm_delete = Some(sid);
    app.games_tab.focus = Focus::Cancel;
}

fn cancel_delete(app: &mut TrackerApp) {
    app.confirm_delete = None;
    app.games_tab.focus = Focus::DeleteButton;
}

/// Queue the delete (its tombstone goes on the next sync), drop the row,
/// and move the focus to the row after it, or before it when it was last.
/// The line says the site copy goes when the strip did: both come from
/// [`crate::games_delete::reach`].
fn delete(app: &mut TrackerApp, sid: &str) {
    let next = games_rows::neighbour(&row_order(app), sid);
    let live = app.live_game_session.as_deref();
    let uploaded = app.games.iter().any(|g| {
        g.session_id == sid
            && crate::games_delete::reach(g, game_card_in_progress(&g.session_id, g.outcome, live))
                == crate::games_delete::Reach::Site
    });
    match crate::commands::delete_session(&app.data_dir, sid) {
        Ok(()) => {
            app.games.retain(|g| g.session_id != sid);
            app.games_tab.picker = None;
            app.expanded = None;
            app.editing = false;
            app.confirm_delete = None;
            app.games_tab.focus = next.map_or(Focus::None, Focus::Row);
            app.games_tab.deleted_note = Some(crate::games_delete::Deleted {
                sid: sid.to_string(),
                uploaded,
            });
            app.games_tab.deleted_filters = Some(crate::games_delete::filters_now(app));
            // The line goes over the empty state when no row is left.
            app.games_tab.deleted_empty_shown = crate::games::shown_games(app).is_empty();
            app.games_tab.pending_deletes = None;
            // A "Send this fix to help training" for the deleted game would
            // open a report on a game that is gone.
            if toast_targets(app, sid) {
                app.toast_action = None;
            }
        }
        Err(e) => app.replace_toast(format!("Could not delete game: {e}")),
    }
}

/// Whether the toast's action opens a report on game `sid`.
fn toast_targets(app: &TrackerApp, sid: &str) -> bool {
    matches!(
        &app.toast_action,
        Some(Message::Report(crate::report::ReportMessage::OpenGame {
            session_id: Some(s),
            ..
        })) if s == sid
    )
}

/// The shown rows' session ids in the order they are drawn.
fn row_order(app: &TrackerApp) -> Vec<String> {
    let shown = crate::games::shown_games(app);
    layout(app, &shown, app.games_tab.grid.get())
        .sections
        .iter()
        .flat_map(|sec| sec.rows.iter().map(|g| g.session_id.clone()))
        .collect()
}

fn layout<'g>(app: &'g TrackerApp, shown: &[&'g Game], grid: Grid) -> games_rows::Layout<'g> {
    games_rows::layout(
        &app.games,
        shown,
        app.games_tab.view,
        grid,
        app.saved_config.session_window_secs,
        app.live_game_session.as_deref(),
    )
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
    let state = app.games_tab.view;
    let grid = Grid::for_window(window_width);
    app.games_tab.grid.set(grid);
    let shown = crate::games::shown_games(app);

    let mut col = column![].spacing(16).width(Fill);
    col = col.push(controls(app, state));
    let chips = crate::filter_chips::bar(app, heroes, maps, state.filters_open);
    if state.filters_open || crate::filter_chips::active_count(app) > 0 {
        col = col.push(chips);
    }
    // Any rejected game: one Retry all line (UX Spec, OQ5).
    if let Some(retry_all) = crate::games_retry::above_table(app) {
        col = col.push(retry_all);
    }
    if shown.is_empty() {
        // Deleting the last row the filters leave still says so.
        if let Some(note) = crate::games_delete::empty_note(app) {
            col = col.push(crate::games_delete::deleted_line(note));
        }
        let copy = if app.games.is_empty() {
            games_rows::EMPTY_NONE
        } else {
            games_rows::EMPTY_FILTERED
        };
        col = col.push(widgets::empty_surface(copy));
        return col.into();
    }

    let gap_secs = app.saved_config.session_window_secs;
    let layout = layout(app, &shown, grid);
    let order: Vec<&str> = layout
        .sections
        .iter()
        .flat_map(|sec| sec.rows.iter().map(|g| g.session_id.as_str()))
        .collect();
    // "Game deleted." tops the table of the row the focus moved to.
    let deleted = crate::games_delete::note_shows(app);
    // A note already drawn over the empty state must not land on row 0
    // when games come back under the same filters.
    let note_at = if app.games_tab.deleted_empty_shown {
        None
    } else {
        match &app.games_tab.focus {
            Focus::Row(sid) if deleted => order.iter().position(|s| *s == sid.as_str()),
            _ => None,
        }
        .or(deleted.then_some(0))
    };
    let copy = crate::games_delete::deleted_copy(app);
    let mut seen = 0;
    for section in layout.sections {
        let rows = section.rows.len();
        let note = note_at.is_some_and(|i| i >= seen && i < seen + rows);
        seen += rows;
        if let Some(head) = section.head {
            col = col.push(session_header(head, section.ticks));
        }
        col = col.push(table(app, &section.rows, grid, state, note.then_some(copy)));
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
    deleted_note: Option<&'static str>,
) -> Element<'a, Message> {
    let cols = games_rows::columns(grid);
    let live = app.live_game_session.as_deref();
    let mut body = column![].width(Fill);
    if let Some(copy) = deleted_note {
        body = body
            .push(crate::games_delete::deleted_line(copy))
            .push(line());
    }
    body = body.push(header_row(&cols, state, grid));
    for g in games {
        body = body.push(line());
        let in_progress = game_card_in_progress(&g.session_id, g.outcome, live);
        let open = app.expanded.as_deref() == Some(g.session_id.as_str());
        let focused = app.games_tab.focus == Focus::Row(g.session_id.clone());
        body = body.push(game_row(g, &cols, state, in_progress, open, focused));
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
    focused: bool,
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
                let (name, known) = crate::games_picker::map_label(g);
                let doubt = unsure(UnsureItem::Map);
                let color = match (doubt, known) {
                    (true, _) => WARN,
                    (false, false) => TEXT_2,
                    _ => main,
                };
                let font = if known {
                    FONT_BOLD
                } else {
                    iced::Font::DEFAULT
                };
                let mut m = row![text(name).size(SIZE_META).font(font).color(color)]
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
            Col::Length => match crate::games_detail::game_length(g).length_cell() {
                Some(len) => text(len).size(SIZE_META).color(TEXT_2).into(),
                None => text("-").size(SIZE_META).color(TEXT_3).into(),
            },
            // Not recorded by the tracker (the Sort note says so).
            Col::Rank | Col::Crew => text("-").size(SIZE_META).color(TEXT_3).into(),
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
    let outline = if open || focused {
        ACCENT
    } else {
        Color::TRANSPARENT
    };
    // The focus ring after a delete is 2px, the open row's outline 1px.
    let width = if focused { 2.0 } else { 1.0 };
    mouse_area(
        container(r)
            .padding(Padding {
                right: 16.0,
                ..Padding::ZERO
            })
            .style(move |_: &Theme| container::Style {
                border: Border {
                    color: outline,
                    width,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }),
    )
    .on_press(games_msg(Msg::Toggle(g.session_id.clone())))
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
    // A hero not read shows "Unknown" with no role letter (spec).
    let unknown = crate::games_picker::hero_unknown(&g.hero);
    let name = if unknown {
        "Unknown".to_string()
    } else {
        g.display_hero()
    };
    let mut r = row![
        portrait(20.0, 14.0),
        text(name)
            .size(SIZE_META)
            .color(if unsure { WARN } else { TEXT_2 }),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if !unknown {
        r = r.push(
            text(role_letter(g.role))
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(theme::role_color(g.role)),
        );
    }
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
    // "⚠ Upload rejected": the row-chip size, a 1px loss outline, no fill.
    let chip = if mark.is_warning() {
        warn_chip(mark.label())
    } else {
        icon_chip(mark.label(), Icon::WarnLoss, theme::DANGER)
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
    icon_chip(label, Icon::Warn, WARN)
}

/// An outline chip with a sign: label and outline in `color`, no fill.
fn icon_chip(label: String, icon: Icon, color: Color) -> Element<'static, Message> {
    container(
        row![
            image(icons::handle(icon)).width(11).height(11),
            text(label)
                .size(SIZE_LABEL)
                .font(FONT_SEMIBOLD)
                .color(color),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([1, 8]))
    .style(move |_: &Theme| container::Style {
        border: Border {
            color,
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

pub(crate) fn small_chip_owned(
    label: String,
    color: Color,
    border: Color,
) -> Element<'static, Message> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures_games::{NORMAL, UNSURE};
    use crate::games_rows::{Col, Sort};

    fn app() -> (tempfile::TempDir, TrackerApp) {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = crate::fixtures::test_cli(dir.path());
        cli.fixture = Some(crate::cli::FixtureKind::Games);
        let mut app = TrackerApp::new(cli).0;
        app.nav_style = crate::nav::NavStyle::TopTabs;
        (dir, app)
    }

    fn send(app: &mut TrackerApp, msg: Msg) {
        let _ = update(app, msg);
    }

    /// Under 1100 px a Hero sort reads as TIME ▼, so the first TIME click
    /// flips it to TIME ▲ instead of doing nothing.
    #[test]
    fn a_header_click_acts_on_the_sort_the_header_shows() {
        let (_dir, mut app) = app();
        app.games_tab.grid.set(Grid::Narrow);
        for hidden in [Col::Hero, Col::Eda, Col::Length] {
            app.games_tab.view.sort = Sort::default().clicked(hidden);
            assert_eq!(
                app.games_tab.view.sort.visible_in(Grid::Narrow),
                Sort::default()
            );
            send(&mut app, Msg::SortBy(Col::Time));
            assert_eq!(
                app.games_tab.view.sort,
                Sort {
                    col: Col::Time,
                    desc: false
                },
                "{hidden:?}"
            );
        }
        // Wide: the Hero sort is shown, so a TIME click starts TIME ▼.
        app.games_tab.grid.set(Grid::Wide);
        app.games_tab.view.sort = Sort::default().clicked(Col::Hero);
        send(&mut app, Msg::SortBy(Col::Time));
        assert_eq!(app.games_tab.view.sort, Sort::default());
    }

    /// A delete drops the toast's training report offer for that game; an
    /// offer for another game stays.
    #[test]
    fn a_delete_drops_the_report_offer_for_the_deleted_game() {
        let (_dir, mut app) = app();
        app.replace_toast("Stats corrected");
        app.arm_training_toast(NORMAL, crate::report::GameCategory::WrongStats);
        assert!(toast_targets(&app, NORMAL));
        send(&mut app, Msg::Delete(UNSURE.into()));
        assert!(toast_targets(&app, NORMAL), "another game's offer stays");
        send(&mut app, Msg::Delete(NORMAL.into()));
        assert!(app.toast_action.is_none());
    }

    /// Leaving the tab takes the ring after a delete and an open picker
    /// with it; coming back shows neither.
    #[test]
    fn leaving_the_tab_clears_the_focus_and_the_picker() {
        let (_dir, mut app) = app();
        app.expanded = Some(crate::fixtures_games::HELD.into());
        send(
            &mut app,
            Msg::Picker(crate::games_picker::Msg::Open(
                crate::fixtures_games::HELD.into(),
                crate::games_picker::Field::Map,
            )),
        );
        assert!(app.games_tab.picker.is_some());
        // As after a delete: the ring on a row.
        app.games_tab.focus = Focus::Row(UNSURE.into());
        let _ = app.update(Message::Navigate(Screen::Overview));
        let _ = app.update(Message::Navigate(Screen::Games));
        assert_eq!(app.games_tab.focus, Focus::None);
        assert!(app.games_tab.picker.is_none());
    }

    /// Edit takes the focus off Delete game, so Enter over the form never
    /// opens the delete strip.
    #[test]
    fn opening_edit_clears_the_delete_focus() {
        let (_dir, mut app) = app();
        app.expanded = Some(NORMAL.into());
        send(&mut app, Msg::Tab);
        assert_eq!(app.games_tab.focus, Focus::DeleteButton);
        let _ = app.update(Message::ToggleEdit);
        assert!(app.editing);
        assert_eq!(app.games_tab.focus, Focus::None);
        send(&mut app, Msg::Enter);
        assert_eq!(app.confirm_delete, None);
        assert!(app.editing);
    }

    /// A focused row a filter hid is dropped: Enter and Delete never open
    /// a row the player can't see.
    #[test]
    fn a_focused_row_the_filters_hide_takes_no_keys() {
        for key in [Msg::Enter, Msg::Space, Msg::DeleteKey] {
            let (_dir, mut app) = app();
            app.games_tab.focus = Focus::Row(UNSURE.into());
            app.filter_outcome = Some(Outcome::Loss);
            send(&mut app, key.clone());
            assert_eq!(app.expanded, None, "{key:?}");
            assert_eq!(app.confirm_delete, None, "{key:?}");
            assert_eq!(app.games_tab.focus, Focus::None, "{key:?}");
        }
        // Still shown: Enter opens it.
        {
            let (_dir, mut app) = app();
            app.games_tab.focus = Focus::Row(UNSURE.into());
            send(&mut app, Msg::Enter);
            assert_eq!(app.expanded.as_deref(), Some(UNSURE));
        }
        // An open row a filter hid later is closed: Delete, Tab and Enter
        // never act on a row the player can't see.
        for key in [Msg::Enter, Msg::Tab, Msg::DeleteKey] {
            let (_dir, mut app) = app();
            app.expanded = Some(UNSURE.into());
            app.filter_outcome = Some(Outcome::Loss);
            send(&mut app, key.clone());
            assert_eq!(app.expanded, None, "{key:?}");
            assert_eq!(app.confirm_delete, None, "{key:?}");
            assert_ne!(app.games_tab.focus, Focus::DeleteButton, "{key:?}");
            assert_ne!(app.games_tab.focus, Focus::Cancel, "{key:?}");
        }
    }

    /// `n` older copies of the normal game, so the list runs past the
    /// top tabs' row cap. Returns the oldest one's session id.
    fn with_many_games(app: &mut TrackerApp, n: usize) -> String {
        let base = app
            .games
            .iter()
            .find(|g| g.session_id == NORMAL)
            .unwrap()
            .clone();
        for i in 0..n {
            let mut g = base.clone();
            g.session_id = format!("sess-old-{i}");
            g.played_at = base.played_at - chrono::TimeDelta::hours(1 + i as i64);
            app.games.push(g);
        }
        format!("sess-old-{}", n - 1)
    }

    /// The classic layout lists every game, so an old card past the top
    /// tabs' 500 rows keeps its delete confirm and its Edit, with the
    /// typed text, across ticks and other messages.
    #[test]
    fn the_classic_layout_keeps_an_old_open_card_past_the_row_cap() {
        let (_dir, mut app) = app();
        app.nav_style = crate::nav::NavStyle::ClassicSidebar;
        let old = with_many_games(&mut app, games_rows::ROW_CAP + 20);
        assert!(!row_order(&app).contains(&old), "past the top tabs' cap");
        app.expanded = Some(old.clone());
        app.confirm_delete = Some(old.clone());
        for _ in 0..3 {
            let _ = app.update(Message::Tick);
        }
        let _ = app.update(Message::Navigate(Screen::Games));
        assert_eq!(app.expanded.as_deref(), Some(old.as_str()));
        assert_eq!(app.confirm_delete.as_deref(), Some(old.as_str()));
        app.confirm_delete = None;
        let _ = app.update(Message::ToggleEdit);
        let _ = app.update(Message::EditField(
            crate::model::EditField::Elims,
            "40".into(),
        ));
        for _ in 0..3 {
            let _ = app.update(Message::Tick);
        }
        let _ = app.update(Message::Navigate(Screen::Games));
        assert!(app.editing);
        assert_eq!(app.expanded.as_deref(), Some(old.as_str()));
        assert_eq!(app.edit.elims, "40");
    }

    /// On the top tabs a row past the first 500 is not drawn, so it takes
    /// no key: an armed confirm on it goes, the row closes and the game is
    /// never deleted, and a focus on it never opens it.
    #[test]
    fn the_top_tabs_refuse_keys_on_a_row_past_the_row_cap() {
        for key in [
            Msg::Tab,
            Msg::Enter,
            Msg::Space,
            Msg::DeleteKey,
            Msg::Escape,
        ] {
            let (_dir, mut app) = app();
            let old = with_many_games(&mut app, games_rows::ROW_CAP + 20);
            assert!(!row_order(&app).contains(&old), "past the cap");
            app.expanded = Some(old.clone());
            app.confirm_delete = Some(old.clone());
            app.games_tab.focus = Focus::ConfirmDelete;
            send(&mut app, key.clone());
            assert!(app.games.iter().any(|g| g.session_id == old), "{key:?}");
            assert!(crate::commands::queued(&app.data_dir).is_empty(), "{key:?}");
            assert_eq!(app.confirm_delete, None, "{key:?}");
            assert_eq!(app.expanded, None, "{key:?}");
            assert_eq!(app.games_tab.focus, Focus::None, "{key:?}");
            // A focus on the undrawn row is dropped, not opened.
            app.games_tab.focus = Focus::Row(old.clone());
            send(&mut app, key.clone());
            assert_eq!(app.expanded, None, "{key:?}");
            assert_eq!(app.confirm_delete, None, "{key:?}");
            assert_eq!(app.games_tab.focus, Focus::None, "{key:?}");
        }
    }

    /// A row a filter hid while Edit is open keeps the form and its typed
    /// text, but its delete strip goes and it takes no key: Tab then Enter
    /// or Space can't delete the game, and Esc can't drop the form. Once
    /// the row is back, the form is there and Esc works again.
    #[test]
    fn a_row_hidden_mid_edit_takes_no_keys() {
        for keys in [
            vec![Msg::Tab, Msg::Enter],
            vec![Msg::Tab, Msg::Space],
            vec![Msg::Enter],
            vec![Msg::Space],
            vec![Msg::Escape],
            vec![Msg::DeleteKey],
        ] {
            let (_dir, mut app) = app();
            app.expanded = Some(UNSURE.into());
            let _ = app.update(Message::ToggleEdit);
            let _ = app.update(Message::EditField(
                crate::model::EditField::Elims,
                "41".into(),
            ));
            // Delete game is still drawn under the form: its strip is up.
            send(&mut app, Msg::AskDelete(UNSURE.into()));
            assert_eq!(app.confirm_delete.as_deref(), Some(UNSURE));
            let _ = app.update(Message::FilterOutcome(Some(Outcome::Loss)));
            assert!(!row_order(&app).iter().any(|s| s == UNSURE));
            // The check runs before each message, so the next key finds
            // the strip gone.
            for key in &keys {
                let _ = app.update(Message::Games(key.clone()));
            }
            assert!(app.games.iter().any(|g| g.session_id == UNSURE), "{keys:?}");
            assert!(
                crate::commands::queued(&app.data_dir).is_empty(),
                "{keys:?}"
            );
            assert_eq!(app.confirm_delete, None, "{keys:?}");
            assert!(app.editing, "{keys:?}");
            assert_eq!(app.expanded.as_deref(), Some(UNSURE), "{keys:?}");
            assert_eq!(app.edit.elims, "41", "{keys:?}");
            let _ = app.update(Message::FilterOutcome(None));
            assert!(app.editing, "{keys:?}");
            assert_eq!(app.edit.elims, "41", "{keys:?}");
            send(&mut app, Msg::Escape);
            assert!(!app.editing, "{keys:?}");
        }
    }

    /// A game that left the list (an untracked map, then a reload) is not
    /// drawn, so its open row closes and Delete, Tab then Enter delete
    /// nothing. In Edit it stays open with the typed text, but takes no
    /// key, as a hidden row does.
    #[test]
    fn a_game_that_left_the_list_takes_no_keys() {
        let (_d1, mut a) = app();
        a.expanded = Some(UNSURE.into());
        a.games.retain(|g| g.session_id != UNSURE);
        for key in [Msg::DeleteKey, Msg::Tab, Msg::Enter] {
            let _ = a.update(Message::Games(key));
        }
        assert_eq!(a.expanded, None);
        assert_eq!(a.confirm_delete, None);
        assert!(a.games_tab.deleted_note.is_none(), "nothing deleted");
        assert!(crate::commands::queued(&a.data_dir).is_empty());
        // In Edit: the form and its text stay, and no key reaches it.
        let (_dir, mut app) = app();
        app.expanded = Some(UNSURE.into());
        let _ = app.update(Message::ToggleEdit);
        let _ = app.update(Message::EditField(
            crate::model::EditField::Elims,
            "42".into(),
        ));
        app.games.retain(|g| g.session_id != UNSURE);
        for key in [Msg::DeleteKey, Msg::Tab, Msg::Enter, Msg::Escape] {
            let _ = app.update(Message::Games(key));
        }
        assert!(app.editing);
        assert_eq!(app.expanded.as_deref(), Some(UNSURE));
        assert_eq!(app.edit.elims, "42");
        assert_eq!(app.confirm_delete, None);
        assert!(app.games_tab.deleted_note.is_none(), "nothing deleted");
        assert!(crate::commands::queued(&app.data_dir).is_empty());
    }

    /// The tick never runs the hidden-row check, so it neither closes an
    /// open row nor builds the list: a row a filter hid stays open until
    /// the next message from the player.
    #[test]
    fn the_tick_does_not_run_the_hidden_row_check() {
        let (_dir, mut app) = app();
        app.expanded = Some(UNSURE.into());
        app.games_tab.focus = Focus::Row(UNSURE.into());
        app.filter_outcome = Some(Outcome::Loss);
        for _ in 0..3 {
            let _ = app.update(Message::Tick);
        }
        assert_eq!(app.expanded.as_deref(), Some(UNSURE));
        assert_eq!(app.games_tab.focus, Focus::Row(UNSURE.into()));
        let _ = app.update(Message::Navigate(Screen::Games));
        assert_eq!(app.expanded, None);
        assert_eq!(app.games_tab.focus, Focus::None);
    }

    /// A filter change mid-Edit keeps the form and its typed text: under a
    /// filter that still shows the row, and under one that hides it until
    /// the row is back.
    #[test]
    fn a_filter_change_mid_edit_keeps_the_typed_text() {
        let (_dir, mut app) = app();
        app.expanded = Some(UNSURE.into());
        let _ = app.update(Message::ToggleEdit);
        assert!(app.editing);
        let _ = app.update(Message::EditField(
            crate::model::EditField::Elims,
            "41".into(),
        ));
        for f in [Some(Outcome::Win), Some(Outcome::Loss), None] {
            let _ = app.update(Message::FilterOutcome(f));
            let _ = app.update(Message::Tick);
            assert!(app.editing, "{f:?}");
            assert_eq!(app.expanded.as_deref(), Some(UNSURE), "{f:?}");
            assert_eq!(app.edit.elims, "41", "{f:?}");
        }
    }

    /// The delete marks whether it left the filters no row, so the view
    /// only reads it.
    #[test]
    fn the_delete_marks_an_empty_list() {
        let (_dir, mut app) = app();
        send(&mut app, Msg::Delete(UNSURE.into()));
        assert!(!app.games_tab.deleted_empty_shown, "rows are left");
        app.filter_map = Some("Circuit Royal".into());
        send(&mut app, Msg::Delete(NORMAL.into()));
        assert!(crate::games::shown_games(&app).is_empty());
        assert!(app.games_tab.deleted_empty_shown);
    }
}
