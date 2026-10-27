//! The held-game picker (UX Spec, OQ5 follow-up): a game held for a
//! missing map or hero shows an amber "Set map" or "Set hero" on that
//! field in its open row. It opens an inline search box over the known
//! names only (the map list with mode chips, or the hero list), the read's
//! best match first. Save stays off until a name is picked; Cancel and Esc
//! close it. Save queues the same edit as the edit form, so the field gets
//! its EDITED badge, the row loses Needs a look, and the game waits to
//! upload once nothing else is missing.

use iced::widget::{
    Column, button, column, container, image, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Border, Color, Element, Fill, Padding, Theme};

use crate::app::{Message, TrackerApp};
use crate::games_detail::{Upload, upload};
use crate::games_panel::key_cap;
use crate::games_table::small_chip;
use crate::icons::{self, Icon};
use crate::model::Game;
use crate::theme::{
    self, ACCENT, BG, BORDER, FONT_BOLD, FONT_SEMIBOLD, SIZE_META, SURFACE, TEXT, TEXT_2, TEXT_3,
    WARN,
};

/// Every picker line is 12px, as the panel's other v2 lines.
const SMALL: f32 = 12.0;
/// The list's rows are 32px; six show, then it scrolls (spec).
const ROW_H: f32 = 32.0;
const ROWS_SHOWN: usize = 6;

/// Which field the picker sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Map,
    Hero,
}

impl Field {
    /// The `edited_fields` name the edit path records.
    pub fn edited_name(self) -> &'static str {
        match self {
            Field::Map => "map_name",
            Field::Hero => "hero",
        }
    }

    /// The `review_suspect_fields` name.
    fn review_name(self) -> &'static str {
        match self {
            Field::Map => "map",
            Field::Hero => "hero",
        }
    }

    pub fn set_label(self) -> &'static str {
        match self {
            Field::Map => "Set map",
            Field::Hero => "Set hero",
        }
    }
}

/// The open picker: one game, one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub sid: String,
    pub field: Field,
    pub query: String,
    pub picked: Option<String>,
    /// The highlighted row (Up and Down), an index into [`candidates`].
    pub active: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Open(String, Field),
    Query(String),
    Pick(String),
    /// Up or Down: move the highlight.
    Move(i32),
    /// Enter in the search box: pick the highlighted row.
    Submit,
    Save,
    /// Cancel, or Esc (the search box takes Esc, so it comes through here).
    Cancel,
}

/// The search box's id, focused when the picker opens.
pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("games-picker-search")
}

/// Whether the uploader holds the game for this field (Needs a look).
/// It follows [`upload`]: an ended game with no result is held too, even
/// with its row marked synced (the daemon does that without sending it).
pub fn held_for(g: &Game, field: Field) -> bool {
    matches!(upload(g, false, true), Upload::Held(_))
        && g.review_fields().contains(&field.review_name())
}

/// The picker of the open row, while neither Edit nor the delete confirm
/// has taken its place. Any other picker is stale.
pub fn open_picker(app: &TrackerApp) -> Option<&Picker> {
    app.games_tab.picker.as_ref().filter(|p| {
        app.expanded.as_deref() == Some(p.sid.as_str())
            && !app.editing
            && app.confirm_delete.is_none()
    })
}

/// Opening Edit closes the picker; the form starts from the game.
pub fn edit_form(tab: &mut crate::games_rows::GamesTab, g: &Game) -> crate::model::EditForm {
    tab.picker = None;
    crate::model::EditForm::from_game(g)
}

/// Lower case with the accents of the hero and map names folded, so
/// "lucio" finds Lúcio and "esperanca" finds Esperança.
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// The map as the row and panel show it: "No map" (and `false`) for a
/// map that is empty or not a known one, as a game held for its map.
pub fn map_label(g: &Game) -> (String, bool) {
    let name = g.map_name.trim();
    if name.is_empty() || !stat_tracker::parse::map_is_known(name) {
        ("No map".into(), false)
    } else {
        (name.to_string(), true)
    }
}

/// "Unknown" for a hero not read (the model shows it as "Unknown hero").
pub fn hero_unknown(name: &str) -> bool {
    stat_tracker::parse::hero_is_unknown_label(name) || name.trim() == "Unknown hero"
}

/// What the reader read for the field ("" when nothing).
pub fn read(g: &Game, field: Field) -> &str {
    let (ocr, shown) = match field {
        Field::Map => (&g.ocr.map_name, &g.map_name),
        Field::Hero => (&g.ocr.hero, &g.hero),
    };
    let r = if ocr.trim().is_empty() { shown } else { ocr };
    r.trim()
}

/// Every name the field accepts: the known maps or the hero list.
fn known(field: Field) -> Vec<&'static str> {
    match field {
        Field::Map => {
            let mut names: Vec<_> = stat_tracker::parse::known_map_names()
                .into_iter()
                .filter(|m| stat_tracker::parse::map_is_known(m))
                .collect();
            names.sort_unstable();
            names
        }
        // The names the reader stores (Lúcio, Torbjörn), so one hero
        // never shows under two names.
        Field::Hero => scuffed_types::HEROES
            .iter()
            .map(|h| scuffed_types::hero_key_to_name(h).unwrap_or(h))
            .collect(),
    }
}

/// The known name the read is closest to, if any.
pub fn best_match(field: Field, read: &str) -> Option<&'static str> {
    if read.is_empty() || stat_tracker::parse::hero_is_unknown_label(read) {
        return None;
    }
    let guess = match field {
        Field::Map => stat_tracker::parse::canonical_map(read),
        Field::Hero => stat_tracker::parse::match_hero_in_text(read),
    }?;
    let guess = match field {
        Field::Hero => scuffed_types::hero_key_to_name(&guess).map_or(guess, str::to_string),
        Field::Map => guess,
    };
    known(field).into_iter().find(|n| fold(n) == fold(&guess))
}

/// The list for `query`: known names containing it (any case), the read's
/// best match first.
pub fn candidates(field: Field, query: &str, read: &str) -> Vec<&'static str> {
    let q = fold(query.trim());
    let mut names: Vec<_> = known(field)
        .into_iter()
        .filter(|n| fold(n).contains(&q))
        .collect();
    if let Some(best) = best_match(field, read)
        && let Some(i) = names.iter().position(|n| *n == best)
    {
        let best = names.remove(i);
        names.insert(0, best);
    }
    names
}

pub fn can_save(p: &Picker) -> bool {
    p.picked.is_some()
}

/// The line over Save: what [`upload`] says of the game once the pick is
/// saved. Before a pick, any name the field accepts stands in for it.
pub fn release_line(
    g: &Game,
    field: Field,
    picked: Option<&str>,
    in_progress: bool,
    sync_on: bool,
) -> String {
    if field == Field::Map
        && let Some(map) = picked
        && !stat_tracker::parse::stats_row_is_tracked(
            map,
            &stat_tracker::parse::stored_game_mode(map),
        )
    {
        return "Games on this map stay on this computer: saving takes it off the Games tab."
            .into();
    }
    let mut after = g.clone();
    let stand_in = match field {
        Field::Map => "Ilios",
        Field::Hero => "Ana",
    };
    apply(&mut after, field, picked.unwrap_or(stand_in));
    match upload(&after, in_progress, sync_on) {
        Upload::Held(what) => format!("Still held after saving: {what} not set."),
        Upload::Waiting => "Saving releases this game: it uploads when the server answers.".into(),
        Upload::InProgress => "Saving releases this game: it uploads once it ends.".into(),
        Upload::SyncOff => {
            "Saving releases this game. It stays on this computer: no website is linked.".into()
        }
        Upload::Undecided => {
            "Saving releases this game: it uploads once you set the result.".into()
        }
        Upload::Uploaded | Upload::Rejected(_) => match field {
            Field::Map => "Saving sets the map.".into(),
            Field::Hero => "Saving sets the hero.".into(),
        },
    }
}

/// Set the field on the shown game as the daemon will.
fn apply(g: &mut Game, field: Field, name: &str) {
    match field {
        Field::Map => g.map_name = name.to_string(),
        Field::Hero => {
            g.hero = name.to_string();
            g.role = crate::model::Role::parse(&stat_tracker::parse::guess_role_public(name));
            for s in g.segments.iter_mut().filter(|s| hero_unknown(&s.hero)) {
                s.hero = name.to_string();
            }
        }
    }
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> iced::Task<Message> {
    match msg {
        // The Set buttons are hidden while Edit is open; a stray Open
        // leaves the form and what was typed alone.
        Msg::Open(..) if app.editing => {}
        Msg::Open(sid, field) => {
            app.confirm_delete = None;
            app.games_tab.focus = crate::games_rows::Focus::None;
            app.games_tab.picker = Some(Picker {
                sid,
                field,
                query: String::new(),
                picked: None,
                active: 0,
            });
            return iced::widget::operation::focus(input_id());
        }
        Msg::Query(q) => {
            if let Some(p) = &mut app.games_tab.picker {
                p.query = q;
                p.active = 0;
            }
        }
        Msg::Move(by) => {
            if let Some((p, g)) = open_with_game(app) {
                let n = candidates(p.field, &p.query, read(&g, p.field)).len();
                let at = (p.active as i64 + by as i64).clamp(0, n.saturating_sub(1) as i64);
                if let Some(p) = &mut app.games_tab.picker {
                    p.active = at as usize;
                }
            }
        }
        Msg::Submit => {
            if let Some((p, g)) = open_with_game(app) {
                let names = candidates(p.field, &p.query, read(&g, p.field));
                if let Some(name) = names.get(p.active) {
                    return update(app, Msg::Pick(name.to_string()));
                }
            }
        }
        // A click also moves the highlight to the clicked row, so Enter,
        // Up and Down go on from the name that was picked.
        Msg::Pick(name) => {
            let at = open_with_game(app).and_then(|(p, g)| {
                candidates(p.field, &p.query, read(&g, p.field))
                    .iter()
                    .position(|n| *n == name)
            });
            if let Some(p) = &mut app.games_tab.picker
                && known(p.field).contains(&name.as_str())
            {
                p.picked = Some(name);
                if let Some(at) = at {
                    p.active = at;
                }
            }
        }
        Msg::Cancel => app.games_tab.picker = None,
        Msg::Save => save(app),
    }
    iced::Task::none()
}

/// The open picker and its game.
fn open_with_game(app: &TrackerApp) -> Option<(Picker, Game)> {
    let p = app.games_tab.picker.clone()?;
    let g = app.games.iter().find(|g| g.session_id == p.sid)?.clone();
    Some((p, g))
}

/// Queue the edit, then show it at once: the field set and EDITED, the
/// hold gone when nothing else is missing.
fn save(app: &mut TrackerApp) {
    let Some(p) = app.games_tab.picker.clone() else {
        return;
    };
    let Some(name) = p.picked.clone() else {
        return;
    };
    let queued = match p.field {
        Field::Map => crate::commands::pick_map(&app.data_dir, &p.sid, &name),
        Field::Hero => crate::commands::pick_hero(&app.data_dir, &p.sid, &name),
    };
    if let Err(e) = queued {
        app.replace_toast(format!("Could not save: {e}"));
        return;
    }
    app.games_tab.picker = None;
    if let Some(g) = app.games.iter_mut().find(|g| g.session_id == p.sid) {
        apply(g, p.field, &name);
        // Only the field gets EDITED (spec): no STATS chip.
        let f = p.field.edited_name().to_string();
        if !g.edited_fields.contains(&f) {
            g.edited_fields.push(f);
        }
    }
    // As the classic picks: a held game is the misread the reader most
    // needs, so the toast offers a training report.
    let (saved, category) = match p.field {
        Field::Map => ("Map saved", crate::report::GameCategory::WrongMap),
        Field::Hero => ("Hero saved", crate::report::GameCategory::WrongHero),
    };
    app.replace_toast(saved);
    app.arm_training_toast(&p.sid, category);
}

/// The amber outlined "⚠ Set map" / "⚠ Set hero" on a held field.
pub fn set_button(sid: &str, field: Field) -> Element<'static, Message> {
    button(
        row![
            image(icons::handle(Icon::Warn)).width(11).height(11),
            text(field.set_label())
                .size(SMALL)
                .font(FONT_SEMIBOLD)
                .color(WARN),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([4, 10]))
    .style(|_: &Theme, status| button::Style {
        background: theme::hover_tint(status),
        text_color: WARN,
        border: Border {
            color: WARN,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..button::Style::default()
    })
    .on_press(pick_msg(Msg::Open(sid.to_string(), field)))
    .into()
}

/// The "EDITED" badge on a hand-set map or hero, as the stat tiles'.
pub fn edited_badge() -> Element<'static, Message> {
    container(text("EDITED").size(11).font(FONT_BOLD).color(TEXT))
        .padding(Padding::from([0, 4]))
        .style(|_: &Theme| container::Style {
            border: Border {
                color: ACCENT,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn pick_msg(m: Msg) -> Message {
    Message::Games(crate::games_table::Msg::Picker(m))
}

/// The top line: the read's best match, else the raw read, as unsure.
pub fn read_line(g: &Game, field: Field) -> String {
    let raw = read(g, field);
    if raw.is_empty() || stat_tracker::parse::hero_is_unknown_label(raw) {
        return "Read: nothing".into();
    }
    let shown = best_match(field, raw).unwrap_or(raw);
    format!("Read: {shown} (unsure)")
}

/// The read line with the read name in bold: "Read: **Suravasa** (unsure)".
fn read_spans(line: String) -> Element<'static, Message> {
    let body = line.strip_prefix("Read: ").unwrap_or(&line).to_string();
    let (name, tail) = match body.strip_suffix(" (unsure)") {
        Some(n) => (n.to_string(), " (unsure)"),
        None => (body.clone(), ""),
    };
    let span = |t: String, bold: bool, color: Color| {
        iced::widget::span(t)
            .size(SMALL)
            .color(color)
            .font(if bold { FONT_BOLD } else { iced::Font::DEFAULT })
    };
    let bold = !tail.is_empty();
    let spans: Vec<iced::widget::text::Span<'static, ()>> = vec![
        span("Read: ".into(), false, TEXT_2),
        span(name, bold, if bold { TEXT } else { TEXT_2 }),
        span(tail.into(), false, TEXT_2),
    ];
    iced::widget::rich_text(spans).into()
}

/// The list's box: BG fill, 1px border, radius 6.
fn list_box(_: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}

/// The no-match line.
pub fn no_match(field: Field, query: &str) -> String {
    let what = match field {
        Field::Map => "map",
        Field::Hero => "hero",
    };
    format!("No {what} called '{}'", query.trim())
}

/// Where the typed part sits in `name`, as a byte range of `name`. It
/// matches as [`candidates`] does (any case, accents folded), so "lucio"
/// marks "Lúcio" whole. Folding can change a character's length, so the
/// match is mapped back one character at a time.
fn typed_range(name: &str, query: &str) -> Option<(usize, usize)> {
    let q = fold(query.trim());
    if q.is_empty() {
        return None;
    }
    // Each character's start in `name` and in the folded name.
    let mut starts = Vec::new();
    let mut folded = String::new();
    for (i, c) in name.char_indices() {
        starts.push((i, folded.len()));
        folded.push_str(&fold(c.encode_utf8(&mut [0; 4])));
    }
    starts.push((name.len(), folded.len()));
    let at = folded.find(&q)?;
    let start = starts.iter().find(|(_, f)| *f == at)?.0;
    let end = starts.iter().find(|(_, f)| *f == at + q.len())?.0;
    Some((start, end))
}

/// A name with the typed part in bold.
fn name_spans(name: &'static str, query: &str) -> Element<'static, Message> {
    let at = typed_range(name, query);
    let span = |t: &'static str, bold: bool| {
        iced::widget::span(t)
            .size(SIZE_META)
            .color(TEXT)
            .font(if bold { FONT_BOLD } else { iced::Font::DEFAULT })
    };
    let spans: Vec<iced::widget::text::Span<'static, ()>> = match at {
        Some((i, j)) => vec![
            span(&name[..i], false),
            span(&name[i..j], true),
            span(&name[j..], false),
        ],
        None => vec![span(name, false)],
    };
    iced::widget::rich_text(spans).into()
}

/// The open picker in place of its field.
pub fn view(g: &Game, p: &Picker, in_progress: bool, sync_on: bool) -> Element<'static, Message> {
    let raw = read(g, p.field);
    let current = best_match(p.field, raw);
    let placeholder = match p.field {
        Field::Map => "Search maps",
        Field::Hero => "Search heroes",
    };
    let search = text_input(placeholder, &p.query)
        .id(input_id())
        .on_input(|q| pick_msg(Msg::Query(q)))
        .on_submit(pick_msg(Msg::Submit))
        .size(SIZE_META)
        .padding(Padding::from([8, 10]))
        .style(search_style);

    let names = candidates(p.field, &p.query, raw);
    let list: Element<'static, Message> = if names.is_empty() {
        // The no-match line sits in the list's own box (spec frame).
        container(text(no_match(p.field, &p.query)).size(SMALL).color(TEXT_2))
            .padding(Padding::from([8, 10]))
            .width(Fill)
            .style(list_box)
            .into()
    } else {
        let mut list = Column::new().spacing(0).width(Fill);
        for (i, name) in names.into_iter().enumerate() {
            list = list.push(option(p, name, i == p.active, current == Some(name)));
        }
        let shown = list_height(list_len(p, raw));
        container(scrollable(list).height(shown))
            .padding(1)
            .style(list_box)
            .into()
    };

    let save = button(text("Save").size(SIZE_META).font(FONT_SEMIBOLD))
        .padding(Padding::from([7, 14]))
        .style(|_: &Theme, status| {
            let on = status != button::Status::Disabled;
            button::Style {
                background: Some(iced::Background::Color(if on { ACCENT } else { SURFACE })),
                text_color: if on { TEXT } else { TEXT_3 },
                border: Border {
                    color: if on { ACCENT } else { BORDER },
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..button::Style::default()
            }
        })
        .on_press_maybe(can_save(p).then(|| pick_msg(Msg::Save)));
    let cancel = button(
        text("Cancel")
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([7, 14]))
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
    .on_press(pick_msg(Msg::Cancel));
    let keys = row![
        key_cap("↑"),
        key_cap("↓"),
        text("move ·").size(SMALL).color(TEXT_3),
        key_cap("Enter"),
        text("picks ·").size(SMALL).color(TEXT_3),
        key_cap("Esc"),
        text("cancels").size(SMALL).color(TEXT_3),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    container(
        column![
            read_spans(read_line(g, p.field)),
            search,
            list,
            text(release_line(
                g,
                p.field,
                p.picked.as_deref(),
                in_progress,
                sync_on
            ))
            .size(SMALL)
            .color(TEXT_2),
            row![space().width(Fill), cancel, save]
                .spacing(8)
                .align_y(Alignment::Center),
            keys,
        ]
        .spacing(8),
    )
    .padding(10)
    .width(Fill)
    .style(|_: &Theme| container::Style {
        background: Some(iced::Background::Color(SURFACE)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// Rows a list of `n` takes: all of them up to six, then it scrolls.
fn list_height(n: usize) -> f32 {
    ROW_H * n.clamp(1, ROWS_SHOWN) as f32
}

fn list_len(p: &Picker, raw: &str) -> usize {
    candidates(p.field, &p.query, raw).len()
}

/// One row: the name (typed part bold), the map's mode chip or the hero's
/// role, "Current read" on the read's match, a tick on the pick.
fn option(
    p: &Picker,
    name: &'static str,
    active: bool,
    current: bool,
) -> Element<'static, Message> {
    let picked = p.picked.as_deref() == Some(name);
    let mut line = row![name_spans(name, &p.query)]
        .spacing(8)
        .align_y(Alignment::Center);
    match p.field {
        Field::Map => {
            if let Some(mode) = crate::games_rows::mode_chip(name) {
                line = line.push(small_chip(mode, TEXT_2, BORDER));
            }
        }
        Field::Hero => {
            let role = stat_tracker::parse::guess_role_public(name);
            line = line.push(text(role).size(SMALL).color(TEXT_3));
        }
    }
    line = line.push(space().width(Fill));
    if current {
        line = line.push(text("Current read").size(SMALL).color(TEXT_3));
    }
    if picked {
        line = line.push(image(icons::handle(Icon::Tick)).width(12).height(12));
    }
    // The highlighted row: a 2px accent edge on a lifted fill.
    let edge = container(space().width(2))
        .width(2)
        .height(Fill)
        .style(theme::stripe_color(if active {
            ACCENT
        } else {
            iced::Color::TRANSPARENT
        }));
    let line = container(line.width(Fill).height(Fill))
        .padding(Padding::from([0, 8]))
        .center_y(Fill);
    button(row![edge, line].width(Fill).height(Fill))
        .width(Fill)
        .height(ROW_H)
        .padding(0)
        .style(move |_: &Theme, status| button::Style {
            background: if active {
                Some(iced::Background::Color(SURFACE))
            } else {
                theme::hover_tint(status)
            },
            text_color: TEXT,
            ..button::Style::default()
        })
        .on_press(pick_msg(Msg::Pick(name.to_string())))
        .into()
}

/// The search box: the accent edge while it has the focus.
fn search_style(
    theme: &Theme,
    status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    let mut s = theme::text_input_style(theme, status);
    if matches!(status, iced::widget::text_input::Status::Focused { .. }) {
        s.border.color = ACCENT;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures_games::{HELD, NORMAL};

    fn app() -> (tempfile::TempDir, TrackerApp) {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = crate::fixtures::test_cli(dir.path());
        cli.fixture = Some(crate::cli::FixtureKind::Games);
        let mut app = TrackerApp::new(cli).0;
        app.nav_style = crate::nav::NavStyle::TopTabs;
        (dir, app)
    }

    fn game<'a>(app: &'a TrackerApp, sid: &str) -> &'a Game {
        app.games.iter().find(|g| g.session_id == sid).unwrap()
    }

    fn open(app: &mut TrackerApp, field: Field) {
        app.expanded = Some(HELD.into());
        let _ = update(app, Msg::Open(HELD.into(), field));
    }

    fn table(app: &mut TrackerApp, msg: crate::games_table::Msg) {
        let _ = crate::games_table::update(app, msg);
    }

    #[test]
    fn the_held_game_gets_set_controls_and_others_do_not() {
        let (_dir, app) = app();
        let g = game(&app, HELD);
        assert!(held_for(g, Field::Map) && held_for(g, Field::Hero));
        assert_eq!(
            upload(g, false, true).detail(""),
            "The map and hero are missing. This game stays on this computer until you set them."
        );
        let n = game(&app, NORMAL);
        assert!(!held_for(n, Field::Map) && !held_for(n, Field::Hero));
    }

    /// Typing filters the known names (any case); the read's match is first.
    #[test]
    fn the_list_filters_and_puts_the_current_read_first() {
        assert_eq!(candidates(Field::Map, "sur", ""), vec!["Suravasa"]);
        assert_eq!(candidates(Field::Map, "SUR", ""), vec!["Suravasa"]);
        let all = candidates(Field::Map, "", "Ilio5");
        assert_eq!(all.first(), Some(&"Ilios"));
        assert!(all.len() > 20);
        assert_eq!(candidates(Field::Hero, "", "Unknown"), known(Field::Hero));
        assert_eq!(candidates(Field::Hero, "lucio", ""), vec!["Lúcio"]);
        assert_eq!(candidates(Field::Hero, "wrec", ""), vec!["Wrecking Ball"]);
        assert_eq!(candidates(Field::Map, "xyz", ""), Vec::<&str>::new());
        assert_eq!(no_match(Field::Map, "xyz "), "No map called 'xyz'");
        assert_eq!(no_match(Field::Hero, "q"), "No hero called 'q'");
    }

    #[test]
    fn the_read_line_names_the_read_or_nothing() {
        let (_dir, app) = app();
        let g = game(&app, HELD);
        assert_eq!(read_line(g, Field::Hero), "Read: nothing");
        assert_eq!(read_line(g, Field::Map), "Read: Ilios (unsure)");
    }

    /// Only a known name can be picked; Save stays off until one is.
    #[test]
    fn only_known_names_pick_and_save_waits_for_a_pick() {
        let (_dir, mut app) = app();
        open(&mut app, Field::Map);
        fn app_picker(app: &TrackerApp) -> Picker {
            app.games_tab.picker.clone().unwrap()
        }
        assert!(!can_save(&app_picker(&app)));
        let _ = update(&mut app, Msg::Pick("Ilio5".into()));
        assert!(!can_save(&app_picker(&app)));
        let _ = update(&mut app, Msg::Save);
        assert!(
            app.games_tab.picker.is_some(),
            "Save without a pick does nothing"
        );
        assert!(crate::commands::queued(&app.data_dir).is_empty());
        let _ = update(&mut app, Msg::Pick("Ilios".into()));
        assert!(can_save(&app_picker(&app)));
    }

    /// Up, Down and Enter pick from the list.
    #[test]
    fn the_keys_move_and_pick() {
        let (_dir, mut app) = app();
        open(&mut app, Field::Hero);
        let _ = update(&mut app, Msg::Move(-1));
        assert_eq!(app.games_tab.picker.as_ref().unwrap().active, 0);
        let _ = update(&mut app, Msg::Move(1));
        let _ = update(&mut app, Msg::Submit);
        let p = app.games_tab.picker.clone().unwrap();
        assert_eq!(p.picked.as_deref(), Some(known(Field::Hero)[1]));
        let _ = update(&mut app, Msg::Query("zzz".into()));
        let _ = update(&mut app, Msg::Submit);
        assert_eq!(
            app.games_tab.picker.as_ref().unwrap().picked.as_deref(),
            Some(known(Field::Hero)[1])
        );
    }

    /// Esc (taken by the search box) and Cancel close it unsaved.
    #[test]
    fn esc_and_cancel_close_without_saving() {
        let (_dir, mut app) = app();
        open(&mut app, Field::Map);
        let _ = update(&mut app, Msg::Pick("Ilios".into()));
        let _ = crate::games_table::update(&mut app, crate::games_table::Msg::Picker(Msg::Cancel));
        assert!(app.games_tab.picker.is_none());
        open(&mut app, Field::Map);
        let _ = crate::games_table::update(&mut app, crate::games_table::Msg::Escape);
        assert!(app.games_tab.picker.is_none());
        assert!(crate::commands::queued(&app.data_dir).is_empty());
        assert!(held_for(game(&app, HELD), Field::Map));
    }

    /// With the hero still unknown, saving the map leaves the game held.
    #[test]
    fn saving_one_field_says_the_other_still_holds_it() {
        let (_dir, mut app) = app();
        open(&mut app, Field::Map);
        assert_eq!(
            release_line(game(&app, HELD), Field::Map, None, false, true),
            "Still held after saving: hero not set."
        );
        let _ = update(&mut app, Msg::Pick("Ilios".into()));
        let _ = update(&mut app, Msg::Save);
        let g = game(&app, HELD);
        assert_eq!(g.map_name, "Ilios");
        assert!(g.edited_fields.iter().any(|f| f == "map_name"));
        assert_eq!(upload(g, false, true), Upload::Held("hero"));
        assert_eq!(crate::commands::queued(&app.data_dir).len(), 1);
    }

    /// Setting the last missing field releases the game: EDITED, no
    /// Needs a look, Waiting to upload.
    #[test]
    fn saving_the_last_field_releases_the_game() {
        let (_dir, mut app) = app();
        open(&mut app, Field::Map);
        let _ = update(&mut app, Msg::Pick("Ilios".into()));
        let _ = update(&mut app, Msg::Save);
        open(&mut app, Field::Hero);
        assert_eq!(
            release_line(game(&app, HELD), Field::Hero, None, false, true),
            "Saving releases this game: it uploads when the server answers."
        );
        let _ = update(&mut app, Msg::Pick("Mei".into()));
        let _ = update(&mut app, Msg::Save);
        let g = game(&app, HELD);
        assert_eq!(g.hero, "Mei");
        assert!(g.edited_fields.iter().any(|f| f == "hero"));
        assert!(!g.needs_review());
        assert_eq!(crate::games_rows::mark(g, false), None);
        assert_eq!(upload(g, false, true), Upload::Waiting);
        assert!(app.games_tab.picker.is_none());
        // As the classic picks, the toast offers a training report.
        assert_eq!(app.toast.as_deref(), Some("Hero saved"));
        assert!(matches!(
            app.toast_action,
            Some(Message::Report(crate::report::ReportMessage::OpenGame {
                category: Some(crate::report::GameCategory::WrongHero),
                training: true,
                ..
            }))
        ));
    }

    /// The delete confirm closes the picker, so Enter reaches Cancel; after
    /// a delete Enter opens the next row; Enter in an open picker picks.
    #[test]
    fn a_picker_never_keeps_enter_or_delete_from_the_row() {
        use crate::games_rows::Focus;
        use crate::games_table::Msg as T;
        let (_dir, mut app) = app();
        open(&mut app, Field::Map);
        table(&mut app, T::AskDelete(HELD.into()));
        assert!(app.games_tab.picker.is_none());
        table(&mut app, T::Enter);
        assert!(app.confirm_delete.is_none(), "Enter pressed Cancel");
        assert_eq!(app.games_tab.focus, Focus::DeleteButton);

        open(&mut app, Field::Map);
        table(&mut app, T::Delete(HELD.into()));
        assert!(app.games_tab.picker.is_none());
        let Focus::Row(next) = app.games_tab.focus.clone() else {
            panic!("{:?}", app.games_tab.focus)
        };
        table(&mut app, T::Enter);
        assert_eq!(app.expanded.as_deref(), Some(next.as_str()));

        // Enter in the picker picks the highlighted name (the read's match),
        // also with the focus off the search box.
        let (_dir2, mut app) = super::tests::app();
        open(&mut app, Field::Map);
        table(&mut app, T::Enter);
        let p = app.games_tab.picker.clone().unwrap();
        assert_eq!(p.picked.as_deref(), Some("Ilios"));
        table(&mut app, T::DeleteKey);
        assert!(app.confirm_delete.is_none(), "Delete is the search box's");

        // Opening Edit closes it; one left on another row is dropped by the
        // next key, and Delete opens the confirm.
        let _ = app.update(Message::ToggleEdit);
        assert!(app.games_tab.picker.is_none());
        let _ = app.update(Message::ToggleEdit);
        open(&mut app, Field::Map);
        app.expanded = Some(NORMAL.into());
        table(&mut app, T::DeleteKey);
        assert!(app.games_tab.picker.is_none());
        assert_eq!(app.confirm_delete.as_deref(), Some(NORMAL));
    }

    /// An ended game with no result that the uploader holds for its map
    /// or hero gets Set, even with its row marked synced unsent.
    #[test]
    fn a_held_game_with_no_result_gets_set() {
        let (_dir, app) = app();
        let mut g = game(&app, HELD).clone();
        g.outcome = crate::model::Outcome::Unknown;
        g.synced = true;
        assert!(held_for(&g, Field::Map) && held_for(&g, Field::Hero));
        g.outcome = crate::model::Outcome::Win;
        assert!(!held_for(&g, Field::Map), "an uploaded game is not held");
    }

    /// The release line says what the game does once the pick is saved.
    #[test]
    fn the_release_line_follows_the_game_after_the_pick() {
        let (_dir, app) = app();
        let mut g = game(&app, HELD).clone();
        g.hero = "Mei".into();
        g.outcome = crate::model::Outcome::Win;
        let line = |g: &Game, picked, live, sync| release_line(g, Field::Map, picked, live, sync);
        assert_eq!(
            line(&g, None, false, true),
            "Saving releases this game: it uploads when the server answers."
        );
        assert_eq!(
            line(&g, Some("Ilios"), false, false),
            "Saving releases this game. It stays on this computer: no website is linked."
        );
        assert_eq!(
            line(&g, Some("Ilios"), true, true),
            "Saving releases this game: it uploads once it ends."
        );
        assert_eq!(
            line(&g, Some("Arena Victoriae"), false, true),
            "Games on this map stay on this computer: saving takes it off the Games tab."
        );
        g.outcome = crate::model::Outcome::Unknown;
        assert_eq!(
            line(&g, Some("Ilios"), false, true),
            "Saving releases this game: it uploads once you set the result."
        );
    }

    /// Enter after a click saves the clicked name, not the row the keys
    /// last lit, and the arrows go on from it.
    #[test]
    fn enter_after_a_click_keeps_the_clicked_name() {
        use crate::games_table::Msg as T;
        let (_dir, mut app) = app();
        open(&mut app, Field::Map);
        let names = candidates(Field::Map, "", read(game(&app, HELD), Field::Map));
        let clicked = names[3].to_string();
        let _ = update(&mut app, Msg::Pick(clicked.clone()));
        assert_eq!(app.games_tab.picker.as_ref().unwrap().active, 3);
        table(&mut app, T::Enter);
        let p = app.games_tab.picker.clone().unwrap();
        assert_eq!(p.picked.as_deref(), Some(clicked.as_str()));
        let _ = update(&mut app, Msg::Move(1));
        assert_eq!(app.games_tab.picker.as_ref().unwrap().active, 4);
        let _ = update(&mut app, Msg::Save);
        assert_eq!(game(&app, HELD).map_name, clicked);
    }

    /// The bold part covers what was typed, accents or not.
    #[test]
    fn the_typed_part_is_found_through_accents() {
        assert_eq!(typed_range("Lúcio", "lucio"), Some((0, "Lúcio".len())));
        assert_eq!(typed_range("Lúcio", "úc"), Some((1, 4)));
        assert_eq!(typed_range("Torbjörn", "jorn"), Some((4, "Torbjörn".len())));
        assert_eq!(
            typed_range("Esperança", "anca"),
            Some((5, "Esperança".len()))
        );
        assert_eq!(typed_range("Ilios", " LI "), Some((1, 3)));
        assert_eq!(typed_range("Ilios", ""), None);
        assert_eq!(typed_range("Ilios", "zz"), None);
    }

    /// With a picker open, Tab leaves the focus alone, so Enter still
    /// goes to the picker.
    #[test]
    fn tab_leaves_an_open_picker_in_charge() {
        use crate::games_rows::Focus;
        use crate::games_table::Msg as T;
        let (_dir, mut app) = app();
        open(&mut app, Field::Map);
        table(&mut app, T::Tab);
        assert_eq!(app.games_tab.focus, Focus::None);
        table(&mut app, T::Enter);
        assert!(app.games_tab.picker.as_ref().unwrap().picked.is_some());
    }

    /// An uploaded game's line names the field being set.
    #[test]
    fn an_uploaded_game_says_which_field_saving_sets() {
        let (_dir, app) = app();
        let mut g = game(&app, NORMAL).clone();
        g.synced = true;
        assert_eq!(
            release_line(&g, Field::Map, Some("Ilios"), false, true),
            "Saving sets the map."
        );
        assert_eq!(
            release_line(&g, Field::Hero, Some("Ana"), false, true),
            "Saving sets the hero."
        );
    }

    /// Set is hidden while Edit is open, and a stray Open keeps the form
    /// and what was typed.
    #[test]
    fn set_leaves_an_open_edit_form_alone() {
        let (_dir, mut app) = app();
        app.expanded = Some(HELD.into());
        let _ = app.update(Message::ToggleEdit);
        app.edit.elims = "99".into();
        let _ = update(&mut app, Msg::Open(HELD.into(), Field::Map));
        assert!(app.editing);
        assert_eq!(app.edit.elims, "99");
        assert!(app.games_tab.picker.is_none());
    }
}
