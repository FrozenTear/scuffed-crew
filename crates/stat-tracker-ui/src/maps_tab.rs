//! The 0.6 Maps tab (spec r1.1 `maps-seasons.md`, variant A).
//!
//! One table card (placeholder tile, Map, Games, W-L-D, Undecided, the
//! win-rate bar and the printed %) and a rail with Best 3, Worst 3 and the
//! undecided games left out of every rate. Sort by Games (default), Win
//! rate or Recent, or click a header. The small-sample rule is
//! [`crate::small_sample`]: under 3 games a map folds under "Show", under
//! 10 decided games it shows its record and "Too few games", sorts after
//! the rated maps by win rate and never ranks as best or worst. Best and
//! Worst rank inside the active filter, so a season filter only counts
//! that season's games. The open game is left out until it has a result.
//!
//! Map tiles stay grey placeholders (40x24 px, the Maps icon) until
//! server-hosted art packs exist. Art is never bundled with the app.

use chrono::{DateTime, Utc};
use iced::widget::text::Wrapping;
use iced::widget::{Column, Row, button, column, container, image, row, space, text};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Padding, Theme};

use crate::aggregate::{GameFilter, Record, filter_games};
use crate::app::{Message, TrackerApp};
use crate::games_state;
use crate::icons::{self, Icon};
use crate::model::{Game, Outcome, Screen, SeasonSel};
use crate::overview_chips::Hidden;
use crate::small_sample::{RATE_MIN, Sample, TOO_FEW, rate, sample};
use crate::tab_bits::{self, PageMsg};
use crate::theme::SURFACE_2;
use crate::theme::{
    ACCENT, BORDER, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, SIZE_BODY, SIZE_LABEL,
    SIZE_META, TEXT, TEXT_2, TEXT_3, WARN,
};

/// What the table sorts by. The segment offers the first three; the
/// column headers offer every one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Games,
    WinRate,
    Recent,
    Map,
    Undecided,
}

impl SortKey {
    /// Names A to Z first; every other key starts high to low.
    fn starts_ascending(self) -> bool {
        self == SortKey::Map
    }
}

/// The tab's own state: the sort and whether the folded maps are open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapsUi {
    pub sort: SortKey,
    pub ascending: bool,
    pub show_folded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    /// The Games, Win rate, Recent segment: that key, its usual direction.
    Sort(SortKey),
    /// A column header: that key, or the other direction when it is the
    /// key already.
    Header(SortKey),
    ShowFolded(bool),
}

pub fn update(ui: &mut MapsUi, msg: Msg) {
    match msg {
        Msg::Sort(key) => {
            ui.sort = key;
            ui.ascending = key.starts_ascending();
        }
        Msg::Header(key) if key == ui.sort => ui.ascending = !ui.ascending,
        Msg::Header(key) => {
            ui.sort = key;
            ui.ascending = key.starts_ascending();
        }
        Msg::ShowFolded(open) => ui.show_folded = open,
    }
}

/// One map in the table.
#[derive(Debug, Clone, PartialEq)]
pub struct MapRow {
    pub name: String,
    pub record: Record,
    pub last_played: DateTime<Utc>,
    /// The printed win rate, `None` under [`RATE_MIN`] decided games.
    pub pct: Option<u32>,
    pub sample: Sample,
}

/// The display name a stored map counts under: the catalog spelling when
/// the core matcher knows it, the stored text otherwise, `None` when the
/// map was never read (those games are in Needs a look instead).
pub fn map_key(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("unknown") {
        return None;
    }
    Some(stat_tracker::parse::canonical_map(t).unwrap_or_else(|| t.to_string()))
}

/// Per-map records for `games`, in no set order, plus how many games have
/// no map.
pub fn map_rows(games: &[&Game]) -> (Vec<MapRow>, usize) {
    let mut rows: Vec<MapRow> = Vec::new();
    let mut no_map = 0;
    for g in games {
        let Some(name) = map_key(&g.map_name) else {
            no_map += 1;
            continue;
        };
        let at = match rows.iter().position(|r| r.name == name) {
            Some(i) => i,
            None => {
                rows.push(MapRow {
                    name,
                    record: Record {
                        games: 0,
                        wins: 0,
                        losses: 0,
                        draws: 0,
                    },
                    last_played: g.played_at,
                    pct: None,
                    sample: Sample::Fold,
                });
                rows.len() - 1
            }
        };
        let r = &mut rows[at];
        r.record.games += 1;
        match g.outcome {
            Outcome::Win => r.record.wins += 1,
            Outcome::Loss => r.record.losses += 1,
            Outcome::Draw => r.record.draws += 1,
            Outcome::Unknown => {}
        }
        r.last_played = r.last_played.max(g.played_at);
    }
    for r in &mut rows {
        r.pct = rate(&r.record).map(|p| p as u32);
        r.sample = sample(&r.record);
    }
    (rows, no_map)
}

/// Sort `rows` by `key`. Ties go to more games, then the name. By win
/// rate the maps without one always come after the rated ones, by games.
pub fn sort_rows(rows: &mut [MapRow], key: SortKey, ascending: bool) {
    use std::cmp::Ordering;
    let tie = |a: &MapRow, b: &MapRow| {
        b.record
            .games
            .cmp(&a.record.games)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    };
    let dir = |o: Ordering| if ascending { o } else { o.reverse() };
    rows.sort_by(|a, b| match key {
        SortKey::Games => dir(a.record.games.cmp(&b.record.games)).then_with(|| tie(a, b)),
        SortKey::Undecided => {
            dir(a.record.undecided().cmp(&b.record.undecided())).then_with(|| tie(a, b))
        }
        SortKey::Recent => dir(a.last_played.cmp(&b.last_played)).then_with(|| tie(a, b)),
        SortKey::Map => dir(a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        SortKey::WinRate => match (a.pct, b.pct) {
            (Some(x), Some(y)) => dir(x.cmp(&y)).then_with(|| tie(a, b)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => tie(a, b),
        },
    });
}

/// Rows shown, and rows folded into the "Show" row: the maps under 3
/// games in total ([`Sample::Fold`]).
pub fn split_fold(rows: Vec<MapRow>) -> (Vec<MapRow>, Vec<MapRow>) {
    rows.into_iter().partition(|r| r.sample != Sample::Fold)
}

/// Best and Worst: up to three each among the maps with a rate, never the
/// same map in both. With one rated map, Worst is empty.
pub fn best_worst(rows: &[MapRow]) -> (Vec<&MapRow>, Vec<&MapRow>) {
    let mut rated: Vec<&MapRow> = rows.iter().filter(|r| r.pct.is_some()).collect();
    rated.sort_by(|a, b| {
        b.pct
            .cmp(&a.pct)
            .then_with(|| b.record.games.cmp(&a.record.games))
            .then_with(|| a.name.cmp(&b.name))
    });
    let worst_n = (rated.len() / 2).min(3);
    let best_n = (rated.len() - worst_n).min(3);
    let best = rated[..best_n].to_vec();
    let worst = rated.iter().rev().take(worst_n).copied().collect();
    (best, worst)
}

/// Copy for an empty Best or Worst card.
pub fn rank_card_empty(best: bool, rated: usize) -> &'static str {
    match (best, rated) {
        (_, 0) => "Needs a map with 10 decided games.",
        (false, 1) => "Needs a second map with 10 decided games.",
        _ => "",
    }
}

/// "All time · 31 maps · 857 games · win rate from 10 decided games ·
/// click a header to sort", in the label's capitals.
pub fn subtitle(scope: &str, maps: usize, games: usize) -> String {
    let maps = if maps == 1 {
        "1 map".to_string()
    } else {
        format!("{maps} maps")
    };
    let games = if games == 1 {
        "1 game".to_string()
    } else {
        format!("{games} games")
    };
    format!(
        "{scope} · {maps} · {games} · win rate from {RATE_MIN} decided games · click a header to sort"
    )
}

/// "3 more maps under 3 games · Junkertown, Numbani, Watchpoint: Grímsvötn".
pub fn fold_line(folded: &[MapRow]) -> String {
    let names: Vec<&str> = folded.iter().map(|r| r.name.as_str()).collect();
    let n = folded.len();
    let under = crate::small_sample::FOLD_UNDER;
    let head = if n == 1 {
        format!("1 more map under {under} games")
    } else {
        format!("{n} more maps under {under} games")
    };
    format!("{head} · {}", names.join(", "))
}

/// Column widths at one page width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cols {
    pub games: f32,
    pub wld: f32,
    pub undecided: f32,
    pub bar: f32,
    /// Right rail width; `None` stacks the rail under the table.
    pub rail: Option<f32>,
    pub gap: f32,
}

/// Content width of a 1440 px window (32 px gutters) and of a 1100 px one
/// (24 px gutters): the spec's two breakpoints.
const WIDE_FROM: f32 = 1376.0;
const RAIL_FROM: f32 = 1052.0;

/// From the 1440 px window up: the wider columns and the 320 px rail.
pub fn wide(width: f32) -> bool {
    width >= WIDE_FROM
}

/// From the 1100 px window up the rail sits beside the table.
pub fn has_rail(width: f32) -> bool {
    width >= RAIL_FROM
}

pub fn cols(width: f32) -> Cols {
    if wide(width) {
        Cols {
            games: 64.0,
            wld: 96.0,
            undecided: 96.0,
            bar: 200.0,
            rail: Some(320.0),
            gap: 24.0,
        }
    } else {
        Cols {
            games: 56.0,
            wld: 88.0,
            undecided: 88.0,
            bar: 160.0,
            rail: has_rail(width).then_some(288.0),
            gap: 16.0,
        }
    }
}

const ROW_H: f32 = 40.0;
const HEAD_H: f32 = 32.0;
const TILE: (f32, f32) = (40.0, 24.0);
const PCT_W: f32 = 48.0;

/// The chips Maps leaves out of its filter row, because [`maps_games`]
/// does not apply them: a map filter would leave one row, a result filter
/// would show every map at 100%, and Needs a look filters Games only.
pub const HIDDEN: Hidden = Hidden {
    season: false,
    hero: false,
    map: true,
    result: true,
    needs_look: true,
};

/// The games the Maps stats count: the season, role and hero filters, the
/// same set [`HIDDEN`] leaves in the chip row. The open game waits for its
/// result.
pub fn maps_games(app: &TrackerApp) -> Vec<&Game> {
    let filter = GameFilter {
        outcome: None,
        map: None,
        ..app.games_filter()
    };
    games_state::settled(
        filter_games(&app.games, &filter),
        app.live_game_session.as_deref(),
    )
}

pub fn view(app: &TrackerApp, width: f32) -> Element<'_, Message> {
    let games = maps_games(app);
    let (mut rows, no_map) = map_rows(&games);
    let ui = app.maps_ui;
    let c = cols(width);
    let scope = match &app.season {
        SeasonSel::Season(id) => app
            .seasons
            .seasons
            .iter()
            .find(|s| &s.id == id)
            .map_or_else(|| "Season".to_string(), |s| s.name.clone()),
        SeasonSel::AllTime => "All time".to_string(),
    };

    let mut page = Column::new().spacing(GRID_GAP).width(Fill);
    if games_state::loading(app) {
        return page
            .push(empty_card(games_state::LOADING, games_state::LOADING_LINE))
            .into();
    }
    if games_state::shows_filters(app) {
        let pool = filter_games(&app.games, &app.header_filter());
        page = page.push(crate::overview_page::chip_row_except(
            app,
            &crate::aggregate::distinct_heroes(&pool),
            &[],
            crate::overview_page::chips_fold(width),
            HIDDEN,
        ));
    }
    if app.games.is_empty() {
        return page
            .push(empty_card(
                "No maps yet",
                "Play a game with the tracker running and your maps show up here.",
            ))
            .into();
    }
    if rows.is_empty() {
        return page
            .push(sort_line(&scope, 0, 0, ui))
            .push(empty_card(
                "No maps in this filter",
                "Games with no map read are in Needs a look.",
            ))
            .into();
    }

    let total_games = rows.iter().map(|r| r.record.games).sum();
    let undecided: usize = rows.iter().map(|r| r.record.undecided()).sum();
    let rated = rows.iter().filter(|r| r.pct.is_some()).count();
    page = page.push(sort_line(&scope, rows.len(), total_games, ui));
    if matches!(app.season, SeasonSel::Season(_)) {
        page = page.push(
            text(format!("Showing {scope} · Needs a look counts all seasons"))
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        );
    }

    let (best, worst) = best_worst(&rows);
    let need = crate::nav::needs_look_count(&app.games, app.live_game_session.as_deref());
    let rail_cards = [
        rank_card(true, &best, rated),
        rank_card(false, &worst, rated),
        not_counted_card(undecided, no_map, need),
    ];

    sort_rows(&mut rows, ui.sort, ui.ascending);
    let (shown, folded) = split_fold(rows);
    let table = table(&shown, &folded, ui, c);

    let body: Element<'_, Message> = match c.rail {
        Some(rail_w) => {
            let mut rail = Column::new().spacing(16).width(rail_w);
            for card in rail_cards {
                rail = rail.push(card);
            }
            row![container(table).width(Fill), rail]
                .spacing(c.gap)
                .align_y(Alignment::Start)
                .into()
        }
        None => {
            let mut under = Row::new().spacing(16).width(Fill);
            for card in rail_cards {
                under = under.push(container(card).width(Length::FillPortion(1)));
            }
            column![table, under].spacing(16).into()
        }
    };
    page.push(body).into()
}

/// The section label and the Games, Win rate, Recent segment.
fn sort_line(scope: &str, maps: usize, games: usize, ui: MapsUi) -> Element<'static, Message> {
    let mut seg = Row::new().spacing(0).align_y(Alignment::Center);
    for (key, label) in [
        (SortKey::Games, "Games"),
        (SortKey::WinRate, "Win rate"),
        (SortKey::Recent, "Recent"),
    ] {
        let on = ui.sort == key;
        seg = seg.push(
            button(
                text(label)
                    .size(12)
                    .font(FONT_SEMIBOLD)
                    .color(if on { TEXT } else { TEXT_2 }),
            )
            .padding(Padding::from([3, 12]))
            .style(move |_: &Theme, status| segment_style(on, status))
            .on_press(page_msg(Msg::Sort(key))),
        );
    }
    let seg = container(seg)
        .padding(2)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(crate::theme::SURFACE)),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: crate::theme::RADIUS_CHIP.into(),
            },
            ..container::Style::default()
        });
    row![
        tab_bits::label(&subtitle(scope, maps, games)),
        space().width(Fill),
        seg
    ]
    .align_y(Alignment::Center)
    .into()
}

fn segment_style(on: bool, status: button::Status) -> button::Style {
    let hot = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: if on {
            Some(Background::Color(tab_bits::ACCENT_SOFT))
        } else if hot {
            Some(Background::Color(SURFACE_2))
        } else {
            None
        },
        text_color: TEXT,
        border: Border {
            color: if on { ACCENT } else { Color::TRANSPARENT },
            width: 1.0,
            radius: crate::theme::RADIUS_CHIP.into(),
        },
        ..button::Style::default()
    }
}

fn page_msg(msg: Msg) -> Message {
    Message::Pages(PageMsg::Maps(msg))
}

fn table(shown: &[MapRow], folded: &[MapRow], ui: MapsUi, c: Cols) -> Element<'static, Message> {
    let mut col = Column::new().width(Fill).push(header(ui, c));
    for r in shown {
        col = col.push(tab_bits::rule()).push(map_line(r, c));
    }
    if !folded.is_empty() {
        if ui.show_folded {
            for r in folded {
                col = col.push(tab_bits::rule()).push(map_line(r, c));
            }
        }
        col = col.push(tab_bits::rule()).push(
            row![
                text(fold_line(folded))
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_2)
                    .width(Fill),
                button(
                    text(if ui.show_folded { "Hide" } else { "Show" })
                        .size(SIZE_META)
                        .font(FONT_BOLD)
                        .color(TEXT),
                )
                .padding(Padding::from([4, 8]))
                .style(|_: &Theme, status| tab_bits::plain_button(status))
                .on_press(page_msg(Msg::ShowFolded(!ui.show_folded))),
            ]
            .align_y(Alignment::Center)
            .height(ROW_H)
            .padding(Padding::from([0, 16])),
        );
    }
    tab_bits::card(col.into(), false, 0.0)
}

fn header(ui: MapsUi, c: Cols) -> Element<'static, Message> {
    let head = |label: &'static str, key: SortKey, w: Length| -> Element<'static, Message> {
        let on = ui.sort == key;
        let arrow = match (on, ui.ascending) {
            (false, _) => "",
            (true, true) => " ▲",
            (true, false) => " ▼",
        };
        button(
            text(format!("{}{arrow}", label.to_uppercase()))
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(if on { ACCENT } else { TEXT_3 })
                .wrapping(Wrapping::None),
        )
        .padding(0)
        .width(w)
        .style(|_: &Theme, _| button::Style::default())
        .on_press(page_msg(Msg::Header(key)))
        .into()
    };
    row![
        space().width(TILE.0),
        head("Map", SortKey::Map, Fill),
        head("Games", SortKey::Games, Length::Fixed(c.games)),
        tab_bits::head_label("W-L-D", c.wld),
        head("Undecided", SortKey::Undecided, Length::Fixed(c.undecided)),
        head("Win rate", SortKey::WinRate, Length::Fixed(c.bar)),
        container(tab_bits::label("%"))
            .width(PCT_W)
            .align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .height(HEAD_H)
    .padding(Padding::from([0, 16]))
    .into()
}

/// One map: tile, name, games, record, undecided, bar and %. A folded
/// map (under 3 games) is muted so it never reads like a counted row.
fn map_line(r: &MapRow, c: Cols) -> Element<'static, Message> {
    let muted = r.sample == Sample::Fold;
    let (strong, soft) = if muted {
        (TEXT_3, TEXT_3)
    } else {
        (TEXT, TEXT_2)
    };
    let (bar, pct): (Element<'static, Message>, Element<'static, Message>) = match r.pct {
        Some(p) => (
            crate::winrate_bar::bar(p as f32),
            text(format!("{p}%"))
                .size(SIZE_BODY)
                .font(FONT_BOLD)
                .color(TEXT)
                .into(),
        ),
        None => (
            text(TOO_FEW)
                .size(12)
                .font(FONT_MEDIUM)
                .color(TEXT_3)
                .wrapping(Wrapping::None)
                .into(),
            space().width(0).into(),
        ),
    };
    let undecided = r.record.undecided();
    row![
        map_tile(),
        text(r.name.clone())
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(strong)
            .wrapping(Wrapping::None)
            .width(Fill),
        text(r.record.games.to_string())
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(strong)
            .width(c.games),
        text(r.record.wl_label())
            .size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .color(soft)
            .width(c.wld),
        text(undecided.to_string())
            .size(SIZE_BODY)
            .font(FONT_MEDIUM)
            .color(TEXT_3)
            .width(c.undecided),
        container(bar).width(c.bar),
        container(pct)
            .width(PCT_W)
            .align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .height(ROW_H)
    .padding(Padding::from([0, 16]))
    .into()
}

/// The fixed 40x24 px tile: grey with the Maps icon until an art pack
/// fills it with the art drawn as a cover image in the same box.
fn map_tile() -> Element<'static, Message> {
    container(
        image(icons::handle(Icon::Tab(Screen::Maps, false)))
            .width(14)
            .height(14),
    )
    .width(TILE.0)
    .height(TILE.1)
    .align_x(iced::alignment::Horizontal::Center)
    .align_y(iced::alignment::Vertical::Center)
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(SURFACE_2)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

fn rank_card(best: bool, picks: &[&MapRow], rated: usize) -> Element<'static, Message> {
    let (title, arrow, sub, color) = if best {
        (
            "Best maps · 10 or more decided games",
            "▲",
            "Highest win rate",
            crate::theme::OK,
        )
    } else {
        (
            "Worst maps · 10 or more decided games",
            "▼",
            "Lowest win rate",
            crate::theme::DANGER,
        )
    };
    let mut col = column![tab_bits::label(title)].spacing(8);
    if picks.is_empty() {
        col = col.push(
            text(rank_card_empty(best, rated))
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        );
        return tab_bits::card(col.into(), false, 16.0);
    }
    col = col.push(
        row![
            text(arrow).size(SIZE_LABEL).color(color),
            text(sub).size(12).font(FONT_MEDIUM).color(TEXT_2),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    );
    let mut lines = Column::new();
    for r in picks {
        let p = r.pct.unwrap_or(0);
        lines = lines.push(tab_bits::rule()).push(
            row![
                text(r.name.clone())
                    .size(SIZE_META)
                    .font(FONT_BOLD)
                    .color(TEXT)
                    .wrapping(Wrapping::None)
                    .width(Fill),
                text(format!("{p}%"))
                    .size(SIZE_META)
                    .font(FONT_BOLD)
                    .color(tab_bits::pct_color(p)),
            ]
            .align_y(Alignment::Center)
            .height(32),
        );
    }
    tab_bits::card(col.push(lines).into(), false, 16.0)
}

/// "N games with no map read are in Needs a look too.", or nothing.
pub fn no_map_line(no_map: usize) -> Option<String> {
    match no_map {
        0 => None,
        1 => Some("1 game with no map read is in Needs a look too.".into()),
        n => Some(format!(
            "{n} games with no map read are in Needs a look too."
        )),
    }
}

fn not_counted_card(undecided: usize, no_map: usize, need: usize) -> Element<'static, Message> {
    let mut col = column![tab_bits::label("Not counted")].spacing(4);
    let no_map_text =
        no_map_line(no_map).map(|line| text(line).size(12).font(FONT_MEDIUM).color(TEXT_3));
    if undecided == 0 {
        col = col.push(
            text("Every game on these maps has an outcome.")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        );
        if let Some(line) = no_map_text {
            col = col.push(line);
        }
        return tab_bits::card(col.into(), no_map > 0, 16.0);
    }
    col = col
        .push(
            row![
                text(undecided.to_string())
                    .size(24)
                    .font(FONT_BOLD)
                    .color(WARN),
                text("undecided games on these maps")
                    .size(12)
                    .font(FONT_MEDIUM)
                    .color(TEXT_2),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .push(
            text(format!(
                "Left out of every win rate until they get an outcome. They are part of the {need} in Needs a look."
            ))
            .size(12)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
        );
    if let Some(line) = no_map_text {
        col = col.push(line);
    }
    tab_bits::card(col.into(), true, 16.0)
}

fn empty_card(title: &'static str, line: &'static str) -> Element<'static, Message> {
    tab_bits::card(
        row![
            map_tile(),
            column![
                text(title).size(SIZE_BODY).font(FONT_BOLD).color(TEXT),
                text(line).size(12).font(FONT_MEDIUM).color(TEXT_2),
            ]
            .spacing(2),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into(),
        false,
        16.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use scuffed_types::MapName;

    fn game(map: &str, outcome: Outcome, day: u32) -> Game {
        let mut g = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ))
        .remove(0);
        g.map_name = map.into();
        g.outcome = outcome;
        g.played_at = Utc.with_ymd_and_hms(2026, 10, day, 20, 0, 0).unwrap();
        g
    }

    fn row_of(name: &str, games: usize, w: usize, l: usize, d: usize) -> MapRow {
        let record = Record {
            games,
            wins: w,
            losses: l,
            draws: d,
        };
        MapRow {
            name: name.into(),
            pct: rate(&record).map(|p| p as u32),
            sample: sample(&record),
            record,
            last_played: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn rows_group_by_catalog_name_and_skip_missing_maps() {
        let games = [
            game("Paraíso", Outcome::Win, 1),
            game("Paraiso", Outcome::Loss, 2),
            game("", Outcome::Win, 3),
            game("Unknown", Outcome::Loss, 4),
            game("King's Row", Outcome::Unknown, 5),
        ];
        let refs: Vec<&Game> = games.iter().collect();
        let (rows, no_map) = map_rows(&refs);
        assert_eq!(no_map, 2, "no map read is Needs a look, not a row");
        assert_eq!(rows.len(), 2);
        let paraiso = rows.iter().find(|r| r.record.games == 2).expect("one row");
        assert_eq!((paraiso.record.wins, paraiso.record.losses), (1, 1));
        assert_eq!(paraiso.last_played.format("%d").to_string(), "02");
        assert!(
            stat_tracker::parse::known_map_names().contains(&paraiso.name.as_str()),
            "{:?} is a catalog name",
            paraiso.name
        );
        let kings = rows.iter().find(|r| r.name == "King's Row").expect("row");
        assert_eq!(kings.record.undecided(), 1);
        assert_eq!(kings.pct, None, "under 10 games");
    }

    #[test]
    fn suravasa_is_flashpoint() {
        assert_eq!(MapName::game_mode_label("Suravasa"), "Flashpoint");
        assert_eq!(
            stat_tracker::parse::map_mode("Suravasa"),
            Some("Flashpoint")
        );
    }

    #[test]
    fn sort_by_games_win_rate_recent_and_headers() {
        let mut rows = vec![
            row_of("Samoa", 12, 5, 5, 0),
            row_of("King's Row", 62, 28, 22, 1),
            row_of("Busan", 9, 9, 0, 0),
            row_of("Antarctic Peninsula", 23, 14, 4, 0),
        ];
        rows[2].last_played = Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap();
        let names = |rows: &[MapRow]| rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>();

        sort_rows(&mut rows, SortKey::Games, false);
        assert_eq!(
            names(&rows),
            ["King's Row", "Antarctic Peninsula", "Samoa", "Busan"]
        );
        sort_rows(&mut rows, SortKey::WinRate, false);
        assert_eq!(
            names(&rows),
            ["Antarctic Peninsula", "King's Row", "Samoa", "Busan"],
            "too few games last even with 9-0"
        );
        sort_rows(&mut rows, SortKey::WinRate, true);
        assert_eq!(names(&rows)[3], "Busan", "and last going up too");
        sort_rows(&mut rows, SortKey::Recent, false);
        assert_eq!(names(&rows)[0], "Busan");
        sort_rows(&mut rows, SortKey::Map, true);
        assert_eq!(names(&rows)[0], "Antarctic Peninsula");
    }

    #[test]
    fn header_click_flips_and_segment_resets() {
        let mut ui = MapsUi::default();
        assert_eq!((ui.sort, ui.ascending), (SortKey::Games, false));
        update(&mut ui, Msg::Header(SortKey::Games));
        assert!(ui.ascending);
        update(&mut ui, Msg::Header(SortKey::Map));
        assert_eq!((ui.sort, ui.ascending), (SortKey::Map, true));
        update(&mut ui, Msg::Sort(SortKey::WinRate));
        assert_eq!((ui.sort, ui.ascending), (SortKey::WinRate, false));
        update(&mut ui, Msg::ShowFolded(true));
        assert!(ui.show_folded);
    }

    #[test]
    fn under_three_games_fold_and_under_ten_decided_say_too_few() {
        let rows = vec![
            row_of("King's Row", 12, 6, 6, 0),
            row_of("Busan", 9, 5, 4, 0),
            row_of("Ilios", 3, 1, 2, 0),
            row_of("Numbani", 2, 1, 1, 0),
            row_of("Junkertown", 1, 1, 0, 0),
            row_of("Rialto", 30, 5, 4, 0),
        ];
        let by = |n: &str| rows.iter().find(|r| r.name == n).expect(n).sample;
        assert_eq!(by("King's Row"), Sample::Rated(50));
        assert_eq!(by("Busan"), Sample::TooFew, "9 games");
        assert_eq!(by("Ilios"), Sample::TooFew, "3 games");
        assert_eq!(by("Numbani"), Sample::Fold, "2 games");
        assert_eq!(by("Junkertown"), Sample::Fold, "1 game");
        assert_eq!(by("Rialto"), Sample::TooFew, "30 games, 9 decided");
        let (shown, folded) = split_fold(rows);
        assert_eq!(shown.len(), 4);
        assert_eq!(
            fold_line(&folded),
            "2 more maps under 3 games · Numbani, Junkertown"
        );
    }

    #[test]
    fn the_open_game_is_left_out_until_it_has_a_result() {
        let (_dir, mut app) = crate::fixtures::test_app();
        let mut live = game("Ilios", Outcome::Unknown, 20);
        live.session_id = "live".into();
        app.games = vec![live, game("Ilios", Outcome::Unknown, 19)];
        app.live_game_session = Some("live".into());
        let shown = maps_games(&app);
        assert_eq!(shown.len(), 1);
        let (rows, _) = map_rows(&shown);
        assert_eq!(rows[0].record.undecided(), 1);
        app.live_game_session = None;
        assert_eq!(maps_games(&app).len(), 2);
    }

    #[test]
    fn hidden_chips_are_the_filters_maps_ignores() {
        use crate::overview_chips::active_count_except;
        let (_dir, mut app) = crate::fixtures::test_app();
        app.games = vec![
            game("Ilios", Outcome::Win, 1),
            game("Busan", Outcome::Loss, 2),
        ];
        app.filter_map = Some("Ilios".into());
        app.filter_outcome = Some(Outcome::Win);
        app.needs_look_filter = true;
        assert_eq!(
            maps_games(&app).len(),
            2,
            "map, result and Needs a look ignored"
        );
        assert_eq!(
            active_count_except(&app, HIDDEN),
            0,
            "so none of them shows"
        );
        assert_eq!(
            active_count_except(&app, crate::overview_chips::Hidden::default()),
            3
        );
        app.filter_hero = Some("Nobody".into());
        assert!(maps_games(&app).is_empty(), "hero applies");
        assert_eq!(active_count_except(&app, HIDDEN), 1, "and shows");
    }

    #[test]
    fn games_with_no_map_are_named_in_not_counted() {
        assert_eq!(no_map_line(0), None);
        assert_eq!(
            no_map_line(1).as_deref(),
            Some("1 game with no map read is in Needs a look too.")
        );
        assert_eq!(
            no_map_line(26).as_deref(),
            Some("26 games with no map read are in Needs a look too.")
        );
    }

    #[test]
    fn best_and_worst_never_share_a_map() {
        let rows = vec![
            row_of("Antarctic Peninsula", 23, 14, 4, 0),
            row_of("Busan", 9, 9, 0, 0),
        ];
        let (best, worst) = best_worst(&rows);
        assert_eq!(best.len(), 1);
        assert!(worst.is_empty());
        assert_eq!(
            rank_card_empty(false, 1),
            "Needs a second map with 10 decided games."
        );
        assert_eq!(
            rank_card_empty(true, 0),
            "Needs a map with 10 decided games."
        );

        let spec = vec![
            row_of("Antarctic Peninsula", 23, 14, 4, 0),
            row_of("Dorado", 24, 12, 7, 0),
            row_of("Havana", 21, 12, 8, 0),
            row_of("King's Row", 62, 28, 22, 1),
            row_of("Midtown", 25, 6, 15, 0),
            row_of("New Queen Street", 14, 3, 8, 0),
            row_of("Runasapi", 13, 2, 8, 0),
        ];
        let (best, worst) = best_worst(&spec);
        let n = |v: &[&MapRow]| v.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        assert_eq!(n(&best), ["Antarctic Peninsula", "Dorado", "Havana"]);
        assert_eq!(n(&worst), ["Runasapi", "New Queen Street", "Midtown"]);
    }

    #[test]
    fn subtitle_and_layout_breakpoints() {
        assert_eq!(
            subtitle("All time", 31, 857),
            "All time · 31 maps · 857 games · win rate from 10 decided games · click a header to sort"
        );
        // 1600, 1366 and 1024 px windows less their gutters.
        assert_eq!(cols(1536.0).rail, Some(320.0));
        assert_eq!(cols(1318.0).rail, Some(288.0));
        assert_eq!(cols(976.0).rail, None);
        assert_eq!(cols(976.0).bar, 160.0);
        assert!(wide(1536.0) && !wide(1318.0));
        assert!(has_rail(1318.0) && !has_rail(976.0));
    }
}
