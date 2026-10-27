//! The open Games row (UX Spec v2, variant A): Map, Heroes (timeline and
//! split), Result and Time on the left; the six stat tiles with their
//! Read/set and "vs avg" lines in the middle; the upload status box and
//! the Edit, Report misread and Scoreboard picture buttons on the right.
//! Under 1100px wide the tiles go 3x2 and the status and buttons drop to
//! one row under both columns. The words and numbers come from
//! [`crate::games_detail`]; the tiles are [`crate::stat_tiles`]'s.

use iced::widget::{Row, button, column, container, image, row, space, text};
use iced::{Alignment, Background, Border, Color, Element, Fill, FillPortion, Padding, Theme};

use crate::app::{Message, TrackerApp};
use crate::games_detail::{self as detail, DeltaLine, Stat, Upload};
use crate::games_rows::{self, UnsureItem};
use crate::games_table::{Msg, pill_box, portrait, small_chip, warn_chip};
use crate::icons::{self, Icon};
use crate::model::{Game, Outcome};
use crate::report::ReportMessage;
use crate::stat_tiles::{self, TileExtras};
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL, SIZE_META, TEXT, TEXT_2,
    TEXT_3,
};
use crate::widgets;

/// The left column at 1366 (spec: 304px).
const LEFT_W: f32 = 304.0;
/// The tile column at 1024 (spec: 420px, 3x2).
const TILES_W_NARROW: f32 = 420.0;
/// Every v2-only line (deltas, Read/set, notes, status) is 12px.
const SMALL: f32 = 12.0;

/// The panel under the open row. `narrow` is the 1024 layout.
pub fn panel<'a>(
    app: &'a TrackerApp,
    g: &Game,
    narrow: bool,
    in_progress: bool,
) -> Element<'a, Message> {
    let has_kit = app.sessions_with_kit.contains(&g.session_id);
    // At 1024 the left column takes what the 420px tile column leaves.
    let left =
        container(left_column(g, in_progress)).width(if narrow { Fill } else { LEFT_W.into() });
    let editing = app.editing && app.edit.session_id == g.session_id;
    let stats = stats_column(app, g, narrow, in_progress);
    let status = status_box(g, in_progress, app.sync_on());
    let up = detail::upload(g, in_progress, app.sync_on());
    // Delete game's confirm strip replaces the actions; the status stays.
    let confirming = app.confirm_delete.as_deref() == Some(g.session_id.as_str());
    let strip = confirming.then(|| {
        crate::games_delete::strip(&g.session_id, up == Upload::Uploaded, &app.games_tab.focus)
    });
    let delete = crate::games_delete::button_for(
        &g.session_id,
        app.games_tab.focus == games_rows::Focus::DeleteButton,
    );
    let mut body = if narrow {
        let actions_row = match strip {
            // The status keeps the left, the strip takes the right (1:1.5).
            Some(strip) => row![
                container(status).width(FillPortion(2)),
                container(strip).width(FillPortion(3)),
            ]
            .spacing(12)
            .align_y(Alignment::Start),
            None => {
                // "Only on this computer" follows the buttons; Delete game
                // is the last cell, at least 12px plus the gap further on.
                let mut r = row![
                    container(status).width(Fill),
                    actions(g, has_kit, narrow, editing, in_progress),
                ]
                .spacing(12)
                .align_y(Alignment::Center);
                if has_kit {
                    r = r.push(local_only());
                }
                r.push(container(delete).padding(Padding {
                    left: 12.0,
                    ..Padding::ZERO
                }))
            }
        };
        column![
            row![left, container(stats).width(TILES_W_NARROW)].spacing(24),
            actions_row,
        ]
        .spacing(16)
    } else {
        let mut right = column![status].spacing(10);
        match strip {
            Some(strip) => right = right.push(strip),
            None => {
                right = right.push(actions(g, has_kit, narrow, editing, in_progress));
                // The picture is the only local-only thing, so the note
                // goes with it.
                if has_kit {
                    right = right.push(local_only());
                }
                // Delete game: right-aligned at the bottom, 16px more
                // than the column's gap above it.
                right = right.push(
                    container(row![space().width(Fill), delete]).padding(Padding {
                        top: 16.0,
                        ..Padding::ZERO
                    }),
                );
            }
        }
        column![
            row![
                left,
                // The right column fills the rest (spec: at least 232px;
                // about 300 at 1366).
                container(stats).width(FillPortion(2)),
                container(right).width(FillPortion(1)),
            ]
            .spacing(24)
        ]
        .spacing(16)
    };
    if editing {
        body = body.push(widgets::edit_form(&app.edit, &g.session_id));
    }
    body = body.push(footer(app.fixture.is_some()));
    // The 2px accent edge down the open panel (spec: "2px edge").
    container(
        row![
            container(space().width(2))
                .width(2)
                .height(Fill)
                .style(theme::stripe_color(ACCENT)),
            container(body).padding(Padding::from([16, 20])).width(Fill),
        ]
        .width(Fill),
    )
    .width(Fill)
    .clip(true)
    .into()
}

/// MAP, HEROES, RESULT, TIME. A map, hero or result read still unsure
/// gets "⚠ Unsure" beside its label, the detail view's per-item mark.
fn left_column(g: &Game, in_progress: bool) -> Element<'static, Message> {
    let items = games_rows::unsure_items(g, in_progress);
    let unsure = |item| items.contains(&item);
    column![
        block("MAP", unsure(UnsureItem::Map), map_line(g)),
        block("HEROES", unsure(UnsureItem::Hero), heroes(g, in_progress)),
        block("RESULT", unsure(UnsureItem::Result), result(g, in_progress)),
        block("TIME", false, time(g)),
    ]
    .spacing(14)
    .into()
}

fn block(
    label: &'static str,
    unsure: bool,
    body: Element<'static, Message>,
) -> Element<'static, Message> {
    let mut head = row![section_label(label)]
        .spacing(8)
        .align_y(Alignment::Center);
    if unsure {
        head = head.push(warn_chip("Unsure".into()));
    }
    column![head, body].spacing(8).into()
}

fn section_label(label: &'static str) -> Element<'static, Message> {
    text(label)
        .size(SIZE_LABEL)
        .font(FONT_BOLD)
        .color(TEXT_3)
        .into()
}

fn map_line(g: &Game) -> Element<'static, Message> {
    let name = if g.map_name.trim().is_empty() {
        "No map".to_string()
    } else {
        g.map_name.clone()
    };
    let mut r = row![text(name).size(SIZE_BODY).font(FONT_BOLD).color(TEXT)]
        .spacing(8)
        .align_y(Alignment::Center);
    if let Some(mode) = games_rows::mode_chip(&g.map_name) {
        r = r.push(small_chip(mode, TEXT_2, BORDER));
    }
    with_correction(r.into(), detail::read_vs_set_name(g, "map_name"))
}

/// `line` with "Read X, you set Y" under it for a hand fix.
fn with_correction(
    line: Element<'static, Message>,
    correction: Option<String>,
) -> Element<'static, Message> {
    match correction {
        Some(c) => column![line, text(c).size(SMALL).color(TEXT_2)]
            .spacing(4)
            .into(),
        None => line,
    }
}

/// The main hero, the 8px timeline bar, the split, a prompt under each
/// unconfirmed one-snapshot swap, and an Undo for each dismissed run. No
/// prompts while the game is still on: the runs are not final.
fn heroes(g: &Game, in_progress: bool) -> Element<'static, Message> {
    let split = detail::hero_split(g);
    let head = row![
        portrait(24.0, 16.0),
        text(split.main.clone())
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(TEXT),
        text(g.role.label().to_uppercase())
            .size(SIZE_LABEL)
            .font(FONT_BOLD)
            .color(theme::role_color(g.role)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let mut bar = Row::new().spacing(2).height(8).width(Fill);
    for part in &split.bar {
        bar = bar.push(bar_part(part));
    }

    let mut legend = Row::new().spacing(10).align_y(Alignment::Center);
    for p in &split.split {
        let bar_part = split.bar.iter().find(|b| b.hero == p.hero);
        let main = bar_part.is_some_and(|b| b.main);
        let swatch = container(space())
            .width(8)
            .height(8)
            .style(swatch_style(main, p.unconfirmed));
        let mut item = row![
            swatch,
            text(format!("{} {}%", p.hero, p.percent))
                .size(SMALL)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        if p.unconfirmed {
            item = item.push(text("unconfirmed").size(SMALL).color(TEXT_3));
        }
        legend = legend.push(item);
    }

    // The whole split as one line on hover.
    let bar = iced::widget::tooltip(
        bar,
        container(text(split.line()).size(SMALL).color(TEXT))
            .padding(Padding::from([4, 8]))
            .style(pill_box(BORDER)),
        iced::widget::tooltip::Position::Bottom,
    );
    let mut col = column![
        with_correction(head.into(), detail::read_vs_set_name(g, "hero")),
        bar,
        legend
    ]
    .spacing(8);
    if in_progress {
        return col.into();
    }
    let small_btn = |label: &'static str, index: u32, confirm: bool| {
        button(text(label).size(SMALL).font(FONT_SEMIBOLD).color(TEXT))
            .padding(Padding::from([3, 10]))
            .style(theme::quiet_btn(6.0))
            .on_press(Message::ResolveSegment {
                session_id: g.session_id.clone(),
                segment: index,
                confirm,
            })
    };
    for (index, prompt) in split.swaps() {
        col = col.push(
            column![
                text(prompt).size(SMALL).color(TEXT_2),
                row![
                    small_btn("Confirm", index, true),
                    small_btn("Dismiss", index, false)
                ]
                .spacing(4),
            ]
            .spacing(4),
        );
    }
    for (index, line) in detail::dismissed_runs(g) {
        col = col.push(
            row![
                text(line).size(SMALL).color(TEXT_3),
                small_btn("Undo", index, true)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    col.into()
}

/// One run in the bar: the main hero solid accent, another accent at 45%,
/// an unconfirmed swap a muted outline with a dashed centre line.
fn bar_part(part: &detail::BarPart) -> Element<'static, Message> {
    let portion = FillPortion(part.snapshots.clamp(1, u16::MAX as u32) as u16);
    if part.unconfirmed {
        let mut dashes = Row::new().spacing(2).align_y(Alignment::Center);
        for _ in 0..12 {
            dashes = dashes.push(
                container(space())
                    .width(3)
                    .height(1)
                    .style(theme::stripe_color(TEXT_3)),
            );
        }
        return container(dashes)
            .width(portion)
            .height(8)
            .center_y(8)
            .clip(true)
            .style(|_: &Theme| container::Style {
                border: Border {
                    color: TEXT_3,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..container::Style::default()
            })
            .into();
    }
    let color = if part.main {
        ACCENT
    } else {
        Color { a: 0.45, ..ACCENT }
    };
    container(space())
        .width(portion)
        .height(8)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn swatch_style(main: bool, unconfirmed: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        if unconfirmed {
            container::Style {
                border: Border {
                    color: TEXT_3,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..container::Style::default()
            }
        } else {
            let color = if main {
                ACCENT
            } else {
                Color { a: 0.45, ..ACCENT }
            };
            container::Style {
                background: Some(Background::Color(color)),
                border: Border {
                    radius: 2.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }
        }
    }
}

/// Win / Loss / Draw, or "Undecided" with the no-outcome chip and "Pick
/// one and it counts." under the same control. "Playing" while it is on.
fn result(g: &Game, in_progress: bool) -> Element<'static, Message> {
    if in_progress {
        return text("Playing")
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(ACCENT)
            .into();
    }
    let sid = g.session_id.clone();
    let seg = |label: &'static str, o: Outcome| {
        button(text(label).size(SIZE_META).font(FONT_SEMIBOLD))
            .padding(Padding::from([4, 12]))
            .style(theme::filter_segment(ACCENT, g.outcome == o))
            .on_press(Message::SetOutcome {
                session_id: sid.clone(),
                outcome: o,
            })
    };
    let control = container(
        row![
            seg("Win", Outcome::Win),
            seg("Loss", Outcome::Loss),
            seg("Draw", Outcome::Draw),
        ]
        .spacing(2),
    )
    .padding(2)
    .style(pill_box(BORDER));
    if g.outcome != Outcome::Unknown {
        return control.into();
    }
    column![
        row![
            text("Undecided")
                .size(SIZE_BODY)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3),
            warn_chip("No outcome · Set".into()),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        row![
            control,
            text("Pick one and it counts.").size(SMALL).color(TEXT_2),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(8)
    .into()
}

/// "Tracked 9:14" with its note. Never a Length line (alpha.1).
fn time(g: &Game) -> Element<'static, Message> {
    let t = detail::tracked(g);
    let mut col = column![text(t.line()).size(SIZE_BODY).font(FONT_BOLD).color(TEXT)].spacing(4);
    if let Some(note) = t.note() {
        col = col.push(text(note).size(SMALL).color(TEXT_3));
    }
    col.into()
}

/// STATS with the edited and unsure chips, the six tiles, and the one
/// caption. While the game is on the tiles are the latest read, so the
/// caption says that instead of comparing with the averages.
fn stats_column<'a>(
    app: &'a TrackerApp,
    g: &Game,
    narrow: bool,
    in_progress: bool,
) -> Element<'a, Message> {
    let mut head = row![section_label("STATS")]
        .spacing(10)
        .align_y(Alignment::Center);
    if g.edited {
        head = head.push(small_chip("Edited", theme::WARN, theme::WARN));
    }
    if let Some(label) = g.unsure_label() {
        head = head.push(
            button(warn_chip(format!("{label} · Check")))
                .padding(0)
                .style(|_: &Theme, _| button::Style::default())
                .on_press(Message::ToggleEdit),
        );
    }
    let avg = if in_progress {
        detail::Averages {
            hero: g.hero.clone(),
            games: 0,
            window: 0,
            means: None,
        }
    } else {
        detail::averages(g, &app.games)
    };
    let tiles: Vec<Element<'static, Message>> = Stat::ALL
        .iter()
        .map(|&s| tile(g, s, &avg, narrow))
        .collect();
    let caption = if in_progress {
        crate::model::NOT_FINAL_NOTE.to_string()
    } else {
        avg.caption()
    };
    let grid: Element<'static, Message> = if narrow {
        let mut it = tiles.into_iter();
        let mut rows = column![].spacing(8);
        for _ in 0..2 {
            let mut r = Row::new().spacing(8).width(Fill);
            for t in it.by_ref().take(3) {
                r = r.push(t);
            }
            rows = rows.push(r);
        }
        rows.into()
    } else {
        Row::with_children(tiles).spacing(8).width(Fill).into()
    };
    column![head, grid, text(caption).size(SMALL).color(TEXT_3),]
        .spacing(10)
        .into()
}

/// One tile: the corrected value, an EDITED badge and "Read 22, you set
/// 24" for a hand fix, then the delta or "Check first".
fn tile(g: &Game, stat: Stat, avg: &detail::Averages, narrow: bool) -> Element<'static, Message> {
    let unsure = g.unsure_now(stat.flat());
    let mut extras = TileExtras {
        fill: true,
        ..TileExtras::default()
    };
    if let Some((read, set)) = detail::read_vs_set(g, stat) {
        // At 1366 the hand-fix tile grows to keep "Read 22, you set 24" on
        // one line; the 1024 grid keeps all six equal.
        extras.fill = narrow;
        extras.badge = Some("EDITED");
        extras
            .lines
            .push((detail::read_vs_set_line(stat, read, set), TEXT_2));
    }
    // The delta is neutral (for D a minus is good); "Check first" muted.
    let delta = detail::delta_line(g, stat, avg);
    if let Some(line) = delta.text() {
        let color = if delta == DeltaLine::CheckFirst {
            TEXT_3
        } else {
            TEXT_2
        };
        extras.lines.push((line, color));
    }
    stat_tiles::tile_with(
        stat.label(),
        detail::tile_value(stat, stat.value(g)),
        unsure,
        extras,
    )
}

/// The upload and reader status box. Only a rejected upload is coloured,
/// always with the sign and the words, and it has its own Retry. A game
/// held for a missing map or hero reads "Needs a look" in amber.
fn status_box(g: &Game, in_progress: bool, sync_on: bool) -> Element<'static, Message> {
    let up = detail::upload(g, in_progress, sync_on);
    let rejected = up.rejected();
    let (icon, title_color) = match up {
        Upload::Uploaded => (Icon::Upload, TEXT),
        Upload::Waiting | Upload::InProgress | Upload::SyncOff => (Icon::Clock, TEXT),
        Upload::Held(_) => (Icon::Warn, theme::WARN),
        Upload::Rejected(_) => (Icon::WarnLoss, theme::DANGER),
    };
    let title = text(up.title())
        .size(SMALL)
        .font(FONT_BOLD)
        .color(title_color);
    let detail_line = text(up.detail(&g.recognizer)).size(SMALL).color(TEXT_2);
    let body: Element<'static, Message> = match up {
        // "Uploaded · Read by cv-v1" on one line.
        Upload::Uploaded => row![title, text("·").size(SMALL).color(TEXT_3), detail_line]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        Upload::Rejected(_) => column![
            title,
            detail_line,
            button(text("Retry").size(SMALL).font(FONT_SEMIBOLD).color(TEXT))
                .padding(Padding::from([3, 10]))
                .style(theme::quiet_btn(6.0))
                .on_press(Message::RetryUpload(g.session_id.clone())),
        ]
        .spacing(6)
        .into(),
        _ => column![title, detail_line].spacing(2).into(),
    };
    let border = if rejected { theme::DANGER } else { BORDER };
    container(
        row![image(icons::handle(icon)).width(12).height(12), body]
            .spacing(8)
            .align_y(Alignment::Start),
    )
    .padding(Padding::from([8, 12]))
    .width(Fill)
    .style(move |_: &Theme| container::Style {
        border: Border {
            color: border,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// An outlined button with an icon: full width in the right column, sized
/// to its label in the 1024 action row.
/// `msg` None draws it disabled.
fn outline_btn(
    icon: Icon,
    label: &'static str,
    full: bool,
    msg: Option<Message>,
) -> Element<'static, Message> {
    let color = if msg.is_some() { TEXT } else { TEXT_3 };
    let b = button(
        row![
            image(icons::handle(icon)).width(12).height(12),
            text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(color),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([7, 12]))
    .style(|_: &Theme, status| button::Style {
        background: theme::hover_tint(status),
        text_color: TEXT,
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..button::Style::default()
    })
    .on_press_maybe(msg);
    if full { b.width(Fill).into() } else { b.into() }
}

/// Edit ("Editing" while the form is open; pressing it closes the form),
/// Report misread, and Scoreboard picture only when one is saved. Report
/// waits for a saved picture while the game is on, as the classic card did.
fn actions(
    g: &Game,
    has_kit: bool,
    narrow: bool,
    editing: bool,
    in_progress: bool,
) -> Element<'static, Message> {
    let sid = g.session_id.clone();
    let full = !narrow;
    let report = (!in_progress || has_kit).then(|| {
        Message::Report(ReportMessage::OpenGame {
            session_id: Some(sid.clone()),
            category: None,
            training: false,
            choices: Vec::new(),
        })
    });
    let edit = if editing { "Editing" } else { "Edit" };
    let mut items = vec![
        outline_btn(Icon::Pencil, edit, full, Some(Message::ToggleEdit)),
        outline_btn(Icon::Flag, "Report misread", full, report),
    ];
    if has_kit {
        items.push(outline_btn(
            Icon::Picture,
            "Scoreboard picture",
            full,
            Some(Message::Games(Msg::OpenScoreboard(sid))),
        ));
    }
    if narrow {
        Row::with_children(items).spacing(8).into()
    } else {
        iced::widget::Column::with_children(items).spacing(8).into()
    }
}

fn local_only() -> Element<'static, Message> {
    row![
        image(icons::handle(Icon::Lock)).width(12).height(12),
        text("Only on this computer")
            .size(SMALL)
            .color(TEXT_3)
            .wrapping(iced::widget::text::Wrapping::None),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

/// The Example data chip (fixture only) and the key hint.
fn footer(example: bool) -> Element<'static, Message> {
    let mut r = row![].spacing(10).align_y(Alignment::Center);
    if example {
        r = r.push(key_cap("Example data")).push(
            text("FrozenTear · sample numbers")
                .size(SMALL)
                .color(TEXT_3),
        );
    }
    r.push(space().width(Fill))
        .push(key_cap("Esc"))
        .push(text("or").size(SMALL).color(TEXT_3))
        .push(key_cap("Enter"))
        .push(
            text("closes · one game open at a time")
                .size(SMALL)
                .color(TEXT_3),
        )
        .into()
}

pub(crate) fn key_cap(label: &'static str) -> Element<'static, Message> {
    container(text(label).size(SIZE_LABEL).font(FONT_BOLD).color(TEXT_2))
        .padding(Padding::from([1, 6]))
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(BORDER)),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// Whether Esc, Enter, Tab and Space go to the Games tab: only under the
/// top tabs, while a row is open or has the focus after a delete (the
/// classic card has its own keys).
pub fn listens(app: &TrackerApp) -> bool {
    let row_focus = matches!(app.games_tab.focus, games_rows::Focus::Row(_));
    (app.expanded.is_some() || row_focus) && app.nav_style == crate::nav::NavStyle::TopTabs
}

/// Esc, Enter, Tab or Space that no widget took: Esc cancels the delete
/// strip or an edit, else closes the row; Enter and Space press the
/// focused button; Enter else closes the row
/// ([`crate::games_table::update`]).
pub fn close_keys(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    key_msg(&key, status).map(Message::Games)
}

fn key_msg(key: &iced::keyboard::Key, status: iced::event::Status) -> Option<Msg> {
    use iced::keyboard::{Key, key::Named};
    if status != iced::event::Status::Ignored {
        return None;
    }
    match key {
        Key::Named(Named::Escape) => Some(Msg::Escape),
        Key::Named(Named::Enter) => Some(Msg::Enter),
        Key::Named(Named::Tab) => Some(Msg::Tab),
        Key::Named(Named::Space) => Some(Msg::Space),
        _ => None,
    }
}

/// The kit's scoreboard picture, opened in the desktop's image viewer.
/// The error is the toast's words.
pub fn open_scoreboard(data_dir: &std::path::Path, sid: &str) -> Result<(), String> {
    use stat_tracker::report::kit::{SCOREBOARD_PNG, session_dir};
    let path = session_dir(data_dir, sid)
        .map(|dir| dir.join(SCOREBOARD_PNG))
        .filter(|p| p.is_file())
        .ok_or_else(|| "No scoreboard picture saved for this game".to_string())?;
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not open the scoreboard picture: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::event::Status;
    use iced::keyboard::{Key, key::Named};

    #[test]
    fn the_keys_count_only_when_nothing_took_them() {
        let esc = Key::Named(Named::Escape);
        let enter = Key::Named(Named::Enter);
        assert_eq!(key_msg(&esc, Status::Ignored), Some(Msg::Escape));
        assert_eq!(key_msg(&esc, Status::Captured), None);
        assert_eq!(key_msg(&enter, Status::Ignored), Some(Msg::Enter));
        assert_eq!(key_msg(&enter, Status::Captured), None);
        let tab = Key::Named(Named::Tab);
        assert_eq!(key_msg(&tab, Status::Ignored), Some(Msg::Tab));
        assert_eq!(key_msg(&tab, Status::Captured), None);
        let space = Key::Named(Named::Space);
        assert_eq!(key_msg(&space, Status::Ignored), Some(Msg::Space));
        assert_eq!(key_msg(&Key::Character("d".into()), Status::Ignored), None);
    }

    /// Esc cancels the edit first, then closes; Enter never closes over
    /// the form; the classic layout keeps its own keys.
    #[test]
    fn esc_cancels_the_edit_before_closing() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = crate::fixtures::test_cli(dir.path());
        cli.fixture = Some(crate::cli::FixtureKind::Games);
        let mut app = crate::app::TrackerApp::new(cli).0;
        app.nav_style = crate::nav::NavStyle::TopTabs;
        app.expanded = Some(crate::fixtures_games::NORMAL.into());
        app.editing = true;
        assert!(listens(&app));
        let _ = crate::games_table::update(&mut app, Msg::Enter);
        assert!(app.editing && app.expanded.is_some());
        let _ = crate::games_table::update(&mut app, Msg::Escape);
        assert!(!app.editing && app.expanded.is_some());
        let _ = crate::games_table::update(&mut app, Msg::Escape);
        assert_eq!(app.expanded, None);
        app.expanded = Some(crate::fixtures_games::NORMAL.into());
        let _ = crate::games_table::update(&mut app, Msg::Enter);
        assert_eq!(app.expanded, None);
        app.expanded = Some(crate::fixtures_games::NORMAL.into());
        app.nav_style = crate::nav::NavStyle::ClassicSidebar;
        assert!(!listens(&app));
    }

    #[test]
    fn a_missing_scoreboard_picture_says_so() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            open_scoreboard(dir.path(), "nope"),
            Err("No scoreboard picture saved for this game".into())
        );
    }

    #[test]
    fn every_fixture_state_draws_at_both_widths() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = crate::fixtures::test_cli(dir.path());
        cli.fixture = Some(crate::cli::FixtureKind::Games);
        let mut app = crate::app::TrackerApp::new(cli).0;
        assert!(
            app.sessions_with_kit
                .contains(crate::fixtures_games::NORMAL)
        );
        let games = app.games.clone();
        for sid in [
            crate::fixtures_games::NORMAL,
            crate::fixtures_games::UNSURE,
            crate::fixtures_games::UNDECIDED,
        ] {
            let g = games.iter().find(|g| g.session_id == sid).unwrap();
            for narrow in [false, true] {
                for in_progress in [false, true] {
                    app.confirm_delete = None;
                    let _ = panel(&app, g, narrow, in_progress);
                    // The Delete game confirm strip in place of the actions.
                    app.confirm_delete = Some(sid.into());
                    let _ = panel(&app, g, narrow, in_progress);
                }
            }
        }
    }
}
