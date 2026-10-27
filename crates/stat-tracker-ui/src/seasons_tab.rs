//! The 0.6 Seasons tab (spec r1.1 `maps-seasons.md`, variant A).
//!
//! A current-season card (dates, days left, Win rate, Last 20 with its
//! form strip and the change against the season, Not counted), then the
//! All seasons table (seasons with games newest first, one muted row for
//! the seasons with none, All time as the total) and a rail with Needs a
//! look by season and where the season dates come from.
//!
//! The page lists every season, so it has no season filter; the role
//! filter still applies. The Rank tile shows only when the opt-in rank
//! reader is on. That reader is not in the tracker yet, so the tile is
//! absent for now and nothing stands in its place.

use chrono::{DateTime, Local, Utc};
use iced::widget::text::Wrapping;
use iced::widget::{Column, Row, column, container, row, space, text};
use iced::{Alignment, Background, Border, Element, Fill, Length, Padding, Theme};
use scuffed_types::Season;

use crate::aggregate::{GameFilter, Record, SeasonWindow, filter_games};
use crate::app::{Message, TrackerApp};
use crate::model::{Game, Outcome, RoleFilter, game_card_in_progress};
use crate::small_sample::{Sample, TOO_FEW, rate, sample};
use crate::tab_bits;
use crate::theme::SURFACE_2;
use crate::theme::{
    ACCENT, BG, BORDER, DANGER, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, OK,
    SIZE_BODY, SIZE_LABEL, SIZE_META, TEXT, TEXT_2, TEXT_3, WARN,
};

/// Decided games the form tile looks back over.
pub const LAST_N: usize = 20;

/// One row of the All seasons table.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonLine {
    /// `None` for All time.
    pub id: Option<String>,
    pub name: String,
    pub dates: String,
    pub is_current: bool,
    pub record: Record,
    pub pct: Option<u32>,
    pub sample: Sample,
}

/// The form tile: the last [`LAST_N`] decided games, shown only while they
/// are a part of the season, as on the Overview.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub pct: u32,
    /// Oldest first, as the strip reads left to right.
    pub marks: Vec<Outcome>,
    /// Points against the season's win rate, when the season has one.
    pub delta: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Current {
    pub line: SeasonLine,
    pub days_left: Option<i64>,
    pub form: Option<Form>,
}

/// One line of the Needs a look card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedLine {
    pub scope: String,
    pub why: &'static str,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub current: Option<Current>,
    pub lines: Vec<SeasonLine>,
    /// Started seasons with no games, newest first.
    pub empty: Vec<String>,
    pub all_time: SeasonLine,
    pub needs: Vec<NeedLine>,
    /// A role chip is on, so "no games" may only mean none for that role.
    pub filtered: bool,
}

/// What a page part with no games says: `unfiltered` with no role chip
/// on, otherwise that the filters hide them (the same words as Games and
/// Heroes).
pub fn none_copy(filtered: bool, unfiltered: &'static str) -> &'static str {
    if filtered {
        "No games match these filters"
    } else {
        unfiltered
    }
}

fn window(s: &Season) -> SeasonWindow {
    SeasonWindow {
        starts_at: s.starts_at,
        ends_at: s.ends_at,
    }
}

/// "Oct 6 to Dec 8" (UTC, as the website sets them).
pub fn dates(s: &Season) -> String {
    format!(
        "{} to {}",
        s.starts_at.format("%b %-d"),
        s.ends_at.format("%b %-d")
    )
}

/// Whole days until `ends_at`, rounded up; `None` once it has ended.
pub fn days_left(ends_at: DateTime<Utc>, now: DateTime<Utc>) -> Option<i64> {
    let secs = (ends_at - now).num_seconds();
    (secs > 0).then(|| (secs + 86_399) / 86_400)
}

fn record_of(games: &[&Game]) -> Record {
    let mut r = Record {
        games: games.len(),
        wins: 0,
        losses: 0,
        draws: 0,
    };
    for g in games {
        match g.outcome {
            Outcome::Win => r.wins += 1,
            Outcome::Loss => r.losses += 1,
            Outcome::Draw => r.draws += 1,
            Outcome::Unknown => {}
        }
    }
    r
}

fn line(
    id: Option<String>,
    name: String,
    dates: String,
    current: bool,
    games: &[&Game],
) -> SeasonLine {
    let record = record_of(games);
    SeasonLine {
        id,
        name,
        dates,
        is_current: current,
        pct: rate(&record).map(|p| p as u32),
        sample: sample(&record),
        record,
    }
}

/// The form over the newest decided games in `games`, if there are at
/// least [`RATE_MIN`] of them.
pub fn form(games: &[&Game], season_pct: Option<u32>) -> Option<Form> {
    let mut decided: Vec<&Game> = games
        .iter()
        .copied()
        .filter(|g| g.outcome != Outcome::Unknown)
        .collect();
    // All of the season would only repeat its own rate.
    if decided.len() <= LAST_N {
        return None;
    }
    decided.sort_by_key(|g| std::cmp::Reverse(g.played_at));
    decided.truncate(LAST_N);
    let wins = decided.iter().filter(|g| g.outcome == Outcome::Win).count();
    let pct = (wins as f32 * 100.0 / decided.len() as f32).round() as u32;
    Some(Form {
        pct,
        marks: decided.iter().rev().map(|g| g.outcome).collect(),
        delta: season_pct.map(|s| i64::from(pct) - i64::from(s)),
    })
}

/// Needs a look split by why, adding up to the top bar's count: a game
/// with no outcome goes under its season; otherwise it is an unknown hero
/// or a missing map, under "Any season".
pub fn needs_lines(games: &[Game], seasons: &[Season], live: Option<&str>) -> Vec<NeedLine> {
    let mut by_season: Vec<(String, DateTime<Utc>, usize)> = Vec::new();
    let (mut no_season, mut hero, mut map) = (0, 0, 0);
    for g in games {
        let playing = game_card_in_progress(&g.session_id, g.outcome, live);
        if !g.needs_look(playing) {
            continue;
        }
        if g.outcome == Outcome::Unknown && !playing {
            match seasons.iter().find(|s| window(s).contains(g.played_at)) {
                Some(s) => match by_season.iter_mut().find(|(n, _, _)| n == &s.name) {
                    Some(entry) => entry.2 += 1,
                    None => by_season.push((s.name.clone(), s.starts_at, 1)),
                },
                None => no_season += 1,
            }
        } else if g.review_fields().contains(&"hero") {
            hero += 1;
        } else {
            map += 1;
        }
    }
    by_season.sort_by_key(|s| std::cmp::Reverse(s.1));
    let mut out: Vec<NeedLine> = by_season
        .into_iter()
        .map(|(scope, _, count)| NeedLine {
            scope,
            why: "no outcome",
            count,
        })
        .collect();
    for (scope, why, count) in [
        ("No season", "no outcome", no_season),
        ("Any season", "unknown hero", hero),
        ("Any season", "no map", map),
    ] {
        if count > 0 {
            out.push(NeedLine {
                scope: scope.into(),
                why,
                count,
            });
        }
    }
    out
}

/// The whole page's numbers. Role-filtered except Needs a look, which is
/// the top bar's count. The open game (`live`) waits for its result.
pub fn model(
    games: &[Game],
    seasons: &[Season],
    roles: RoleFilter,
    live: Option<&str>,
    now: DateTime<Utc>,
) -> Model {
    let base = GameFilter::from_header(None, roles);
    let all = crate::overview_page::settled_of(filter_games(games, &base), live);
    let mut ordered: Vec<&Season> = seasons.iter().collect();
    ordered.sort_by_key(|s| std::cmp::Reverse(s.starts_at));

    let mut lines = Vec::new();
    let mut empty = Vec::new();
    let mut current = None;
    for s in ordered {
        let w = window(s);
        let in_season: Vec<&Game> = all
            .iter()
            .copied()
            .filter(|g| w.contains(g.played_at))
            .collect();
        let l = line(
            Some(s.id.clone()),
            s.name.clone(),
            dates(s),
            s.is_current,
            &in_season,
        );
        if s.is_current {
            current = Some(Current {
                days_left: days_left(s.ends_at, now),
                form: form(&in_season, l.pct),
                line: l.clone(),
            });
        }
        if !in_season.is_empty() {
            lines.push(l);
        } else if s.starts_at <= now && !s.is_current {
            empty.push(s.name.clone());
        }
    }
    Model {
        current,
        lines,
        empty,
        all_time: line(
            None,
            "All time".into(),
            "Every recorded game".into(),
            false,
            &all,
        ),
        needs: needs_lines(games, seasons, live),
        filtered: !roles.unrestricted(),
    }
}

/// "Oct 6 to Dec 8 · 49 days left".
pub fn current_sub(c: &Current) -> String {
    match c.days_left {
        Some(1) => format!("{} · 1 day left", c.line.dates),
        Some(n) => format!("{} · {n} days left", c.line.dates),
        None => format!("{} · ended", c.line.dates),
    }
}

/// "▲ +6 vs season", "▼ -4 vs season" or "same as season", the
/// Overview's words ([`crate::overview_side::versus`]).
pub fn delta_text(delta: i64) -> (Option<char>, String) {
    match delta {
        d if d > 0 => (Some('▲'), format!("+{d} vs season")),
        d if d < 0 => (Some('▼'), format!("{d} vs season")),
        _ => (None, "same as season".into()),
    }
}

/// The Season dates card's words.
pub fn dates_note(seasons: &[Season], fetched_at: Option<DateTime<Utc>>) -> String {
    if seasons.is_empty() {
        return "No seasons yet, so every number is all time. Seasons are set on the website under /admin/seasons.".into();
    }
    match fetched_at {
        Some(at) => format!(
            "Set on the website under /admin/seasons. Last refreshed {}.",
            at.with_timezone(&Local).format("%b %-d, %H:%M")
        ),
        None => "Set on the website under /admin/seasons. Not refreshed yet.".into(),
    }
}

/// Column widths: the Maps breakpoints, a 7-column table.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cols {
    dates: f32,
    games: f32,
    wld: f32,
    bar: f32,
    review: f32,
    rail: Option<f32>,
    gap: f32,
    narrow: bool,
}

fn cols(width: f32) -> Cols {
    let m = crate::maps_tab::cols(width);
    let wide = crate::maps_tab::wide(width);
    Cols {
        dates: if wide { 136.0 } else { 128.0 },
        games: if wide { 64.0 } else { 56.0 },
        wld: if wide { 104.0 } else { 96.0 },
        bar: if wide { 180.0 } else { 140.0 },
        review: if wide { 184.0 } else { 160.0 },
        rail: m.rail,
        gap: m.gap,
        narrow: !crate::maps_tab::has_rail(width),
    }
}

const PCT_W: f32 = 48.0;

/// The chips Seasons leaves out: the page lists every season and counts
/// by role only, so the season, hero, map and result chips would show
/// without filtering anything. Needs a look shows on Games only.
pub const HIDDEN: crate::overview_chips::Hidden = crate::overview_chips::Hidden {
    season: true,
    hero: true,
    map: true,
    result: true,
};

pub fn view(app: &TrackerApp, width: f32) -> Element<'_, Message> {
    let m = model(
        &app.games,
        &app.seasons.seasons,
        app.roles,
        app.live_game_session.as_deref(),
        app.clock,
    );
    let c = cols(width);
    let mut page = Column::new().spacing(24).width(Fill);
    if crate::overview_page::shows_filters(app) {
        page = page.push(crate::overview_page::chip_row_except(
            app,
            &[],
            &[],
            crate::overview_page::chips_fold(width),
            HIDDEN,
        ));
    }
    if crate::games_state::loading(app) {
        return page
            .push(tab_bits::card(
                column![
                    text(crate::games_state::LOADING)
                        .size(SIZE_BODY)
                        .font(FONT_BOLD)
                        .color(TEXT),
                    text(crate::games_state::LOADING_LINE)
                        .size(12)
                        .font(FONT_MEDIUM)
                        .color(TEXT_2),
                ]
                .spacing(2)
                .into(),
                false,
                16.0,
            ))
            .into();
    }
    if let Some(cur) = &m.current {
        page = page.push(current_card(cur, c.narrow, m.filtered));
    }

    let mut table = column![tab_bits::label("All seasons")].spacing(8);
    if m.all_time.record.games == 0 {
        table = table.push(
            text(none_copy(
                m.filtered,
                "Older seasons show here once you have games in them.",
            ))
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
        );
    } else {
        table = table.push(season_table(&m, c));
    }
    let need_total: usize = m.needs.iter().map(|n| n.count).sum();
    let rail_cards = [
        needs_card(&m.needs, need_total),
        tab_bits::card(
            column![
                tab_bits::label("Season dates"),
                text(dates_note(&app.seasons.seasons, app.seasons.fetched_at))
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_2),
            ]
            .spacing(6)
            .into(),
            false,
            16.0,
        ),
    ];
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

fn current_card(cur: &Current, narrow: bool, filtered: bool) -> Element<'static, Message> {
    let head = column![
        tab_bits::label("Current season"),
        text(cur.line.name.clone())
            .size(24)
            .font(FONT_EXTRABOLD)
            .color(TEXT)
            .wrapping(Wrapping::None),
        text(current_sub(cur))
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
    ]
    .spacing(4);

    let inner: Element<'static, Message> = if cur.line.record.games == 0 {
        let empty = column![
            text(none_copy(filtered, "No games this season yet"))
                .size(SIZE_BODY)
                .font(FONT_BOLD)
                .color(TEXT),
            text(if filtered {
                "Clear the role chip to see every game this season."
            } else {
                "Play a game with the tracker running and it shows up here."
            })
            .size(12)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
        ]
        .spacing(2);
        if narrow {
            column![head, empty].spacing(16).into()
        } else {
            row![
                container(head).width(Length::FillPortion(11)),
                container(empty).width(Length::FillPortion(40))
            ]
            .spacing(16)
            .align_y(Alignment::Center)
            .into()
        }
    } else {
        let mut tiles: Vec<Element<'static, Message>> = vec![rate_tile(&cur.line)];
        if let Some(f) = &cur.form {
            tiles.push(form_tile(f));
        }
        tiles.push(not_counted_tile(&cur.line));
        if narrow {
            let mut grid = Column::new().spacing(12);
            let mut it = tiles.into_iter();
            while let Some(a) = it.next() {
                let b = it.next().unwrap_or_else(|| space().width(Fill).into());
                grid = grid.push(
                    row![
                        container(a).width(Length::FillPortion(1)),
                        container(b).width(Length::FillPortion(1))
                    ]
                    .spacing(12),
                );
            }
            column![head, grid].spacing(16).into()
        } else {
            let mut r = Row::new()
                .spacing(16)
                .align_y(Alignment::Start)
                .push(container(head).width(Length::FillPortion(11)));
            for t in tiles {
                r = r.push(container(t).width(Length::FillPortion(10)));
            }
            r.into()
        }
    };

    let stripe = container(space().width(4))
        .width(4)
        .height(Fill)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(ACCENT)),
            ..container::Style::default()
        });
    container(
        row![
            stripe,
            container(inner)
                .padding(Padding::from([20, 20]))
                .width(Fill)
        ]
        .height(Length::Shrink),
    )
    .width(Fill)
    .clip(true)
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(crate::theme::SURFACE)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: crate::theme::RADIUS_CARD.into(),
        },
        ..container::Style::default()
    })
    .into()
}

fn tile(body: Column<'static, Message>) -> Element<'static, Message> {
    container(body.spacing(4))
        .padding(12)
        .width(Fill)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(SURFACE_2)),
            border: Border {
                radius: crate::theme::RADIUS_INNER.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn big(value: String) -> Element<'static, Message> {
    text(value).size(32).font(FONT_BOLD).color(TEXT).into()
}

fn note(s: String) -> Element<'static, Message> {
    text(s).size(12).font(FONT_MEDIUM).color(TEXT_3).into()
}

/// "52-55 · 110 games", with draws when there are any ("4-5-1").
pub fn rate_tile_note(r: &Record) -> String {
    format!("{} · {}", r.wl_label(), r.games_label())
}

fn rate_tile(l: &SeasonLine) -> Element<'static, Message> {
    let mut col = column![tab_bits::label("Win rate")];
    let rec = rate_tile_note(&l.record);
    match l.pct {
        Some(p) => {
            col = col
                .push(big(format!("{p}%")))
                .push(container(crate::winrate_bar::bar(p as f32)).width(Fill))
                .push(note(rec));
        }
        None => {
            col = col
                .push(text(TOO_FEW).size(20).font(FONT_BOLD).color(TEXT_2))
                .push(note(rec));
        }
    }
    tile(col)
}

/// Last 20: rate, the strip of marks and the change against the season.
fn form_tile(f: &Form) -> Element<'static, Message> {
    let mut col = column![
        tab_bits::label(&format!("Last {}", f.marks.len())),
        big(format!("{}%", f.pct)),
        crate::session_bits::outcome_sparkline(f.marks.iter().copied()),
    ];
    if let Some(d) = f.delta {
        let (arrow, words) = delta_text(d);
        let arrow_color = if d > 0 { OK } else { DANGER };
        let mut chip = Row::new().spacing(4).align_y(Alignment::Center);
        if let Some(a) = arrow {
            chip = chip.push(
                crate::overview_page::glyph(a)
                    .size(SIZE_LABEL)
                    .color(arrow_color),
            );
        }
        col = col.push(
            container(chip.push(text(words).size(12).font(FONT_BOLD).color(TEXT)))
                .padding(Padding::from([2, 8]))
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(BG)),
                    border: Border {
                        radius: crate::theme::RADIUS_CHIP.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }),
        );
    }
    tile(col)
}

fn not_counted_tile(l: &SeasonLine) -> Element<'static, Message> {
    let u = l.record.undecided();
    let col = if u == 0 {
        column![
            tab_bits::label("Not counted"),
            note("Every game this season has an outcome".into()),
        ]
    } else {
        column![
            tab_bits::label("Not counted"),
            container(tab_bits::review_chip(u, l.id.clone())).padding(Padding::from([8, 0])),
            note("Left out of the win rate until they get an outcome".into()),
        ]
    };
    tile(col)
}

fn season_table(m: &Model, c: Cols) -> Element<'static, Message> {
    let header = row![
        container(tab_bits::label("Season")).width(Fill),
        tab_bits::head_label("Dates", c.dates),
        tab_bits::head_label("Games", c.games),
        tab_bits::head_label("W-L-D", c.wld),
        tab_bits::head_label("Win rate", c.bar),
        container(tab_bits::label("%"))
            .width(PCT_W)
            .align_x(iced::alignment::Horizontal::Right),
        tab_bits::head_label("Not counted", c.review),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .height(32)
    .padding(Padding::from([0, 16]));

    let mut col = Column::new().width(Fill).push(header);
    for l in &m.lines {
        col = col.push(tab_bits::rule()).push(season_row(l, c, false));
    }
    if !m.empty.is_empty() {
        col = col.push(tab_bits::rule()).push(
            row![
                text(m.empty.join(" · "))
                    .size(SIZE_BODY)
                    .font(FONT_SEMIBOLD)
                    .color(TEXT_3)
                    .wrapping(Wrapping::None)
                    .width(Fill),
                text(none_copy(m.filtered, "No games recorded"))
                    .size(SIZE_BODY)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3)
                    .width(Length::Fixed(
                        c.dates + c.games + c.wld + c.bar + PCT_W + c.review + 60.0
                    )),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .height(40)
            .padding(Padding::from([0, 16])),
        );
    }
    col = col
        .push(tab_bits::rule())
        .push(tab_bits::rule())
        .push(season_row(&m.all_time, c, true));
    tab_bits::card(col.into(), false, 0.0)
}

/// One season. A season under 3 games is muted, so it never reads like a
/// counted row.
fn season_row(l: &SeasonLine, c: Cols, total: bool) -> Element<'static, Message> {
    let muted = l.sample == Sample::Fold;
    let (strong, soft) = if muted {
        (TEXT_3, TEXT_3)
    } else {
        (TEXT, TEXT_2)
    };
    let mut name = Row::new().spacing(8).align_y(Alignment::Center).push(
        text(l.name.clone())
            .size(SIZE_BODY)
            .font(if total { FONT_EXTRABOLD } else { FONT_BOLD })
            .color(strong)
            .wrapping(Wrapping::None),
    );
    if l.is_current {
        name = name.push(now_chip());
    }
    let (bar, pct): (Element<'static, Message>, Element<'static, Message>) = match l.pct {
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
    row![
        container(name).width(Fill),
        text(l.dates.clone())
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(soft)
            .wrapping(Wrapping::None)
            .width(c.dates),
        text(l.record.games.to_string())
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(strong)
            .width(c.games),
        text(format!(
            "{}-{}-{}",
            l.record.wins, l.record.losses, l.record.draws
        ))
        .size(SIZE_BODY)
        .font(FONT_SEMIBOLD)
        .color(soft)
        .width(c.wld),
        container(bar).width(c.bar),
        container(pct)
            .width(PCT_W)
            .align_x(iced::alignment::Horizontal::Right),
        container(tab_bits::review_chip(l.record.undecided(), l.id.clone())).width(c.review),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .height(44)
    .padding(Padding::from([0, 16]))
    .into()
}

fn now_chip() -> Element<'static, Message> {
    container(
        text("Now")
            .size(SIZE_LABEL)
            .font(FONT_EXTRABOLD)
            .color(ACCENT),
    )
    .padding(Padding::from([1, 8]))
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(tab_bits::ACCENT_SOFT)),
        border: Border {
            color: iced::Color { a: 0.5, ..ACCENT },
            width: 1.0,
            radius: crate::theme::RADIUS_CHIP.into(),
        },
        ..container::Style::default()
    })
    .into()
}

fn needs_card(needs: &[NeedLine], total: usize) -> Element<'static, Message> {
    let mut col = column![tab_bits::label("Needs a look · by season")].spacing(4);
    if total == 0 {
        col = col.push(
            text("Nothing needs a look.")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        );
        return tab_bits::card(col.into(), false, 16.0);
    }
    col = col.push(
        row![
            text(total.to_string()).size(24).font(FONT_BOLD).color(WARN),
            text(if total == 1 { "game" } else { "games" })
                .size(12)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );
    for n in needs {
        col = col.push(tab_bits::rule()).push(
            row![
                text(n.scope.clone())
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_2)
                    .wrapping(Wrapping::None)
                    .width(Fill),
                text(n.why).size(SIZE_LABEL).font(FONT_MEDIUM).color(TEXT_3),
                container(
                    text(n.count.to_string())
                        .size(SIZE_META)
                        .font(FONT_BOLD)
                        .color(TEXT)
                )
                .width(32)
                .align_x(iced::alignment::Horizontal::Right),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .height(32),
        );
    }
    tab_bits::card(col.into(), true, 16.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, m, d, 12, 0, 0).unwrap()
    }

    fn season(id: &str, name: &str, from: DateTime<Utc>, to: DateTime<Utc>, cur: bool) -> Season {
        Season {
            id: id.into(),
            name: name.into(),
            starts_at: from,
            ends_at: to,
            is_current: cur,
        }
    }

    fn seasons() -> Vec<Season> {
        let day = |m, d| Utc.with_ymd_and_hms(2026, m, d, 0, 0, 0).unwrap();
        vec![
            season("s1", "Season 1", day(2, 1), day(4, 1), false),
            season("s2", "Season 2", day(4, 1), day(6, 16), false),
            season("s3", "Season 3", day(6, 16), day(8, 11), false),
            season("s4", "Season 4", day(8, 11), day(10, 6), false),
            season("s5", "Season 5", day(10, 6), day(12, 8), true),
        ]
    }

    fn game(outcome: Outcome, when: DateTime<Utc>) -> Game {
        let mut g = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ))
        .remove(0);
        g.outcome = outcome;
        g.played_at = when;
        g.session_id = format!("s-{}", when.timestamp());
        g.map_name = "King's Row".into();
        g.hero = "Ana".into();
        g
    }

    #[test]
    fn dates_and_days_left_read_like_the_spec() {
        let s = &seasons()[4];
        assert_eq!(dates(s), "Oct 6 to Dec 8");
        assert_eq!(dates(&seasons()[3]), "Aug 11 to Oct 6");
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 0, 40, 0).unwrap();
        assert_eq!(days_left(s.ends_at, now), Some(49));
        assert_eq!(days_left(s.ends_at, at(12, 9)), None);
    }

    #[test]
    fn a_role_chip_that_hides_every_game_says_so() {
        use crate::model::Role;
        let mut games = vec![game(Outcome::Win, at(10, 8)), game(Outcome::Loss, at(9, 1))];
        for g in &mut games {
            g.role = Role::Support;
        }
        let all = model(&games, &seasons(), RoleFilter::default(), None, at(10, 20));
        assert!(!all.filtered);
        assert_eq!(
            none_copy(all.filtered, "No games recorded"),
            "No games recorded"
        );
        let tank = RoleFilter::default().toggle(Role::Tank);
        let m = model(&games, &seasons(), tank, None, at(10, 20));
        assert!(m.filtered);
        assert!(m.lines.is_empty());
        assert_eq!(m.current.expect("current").line.record.games, 0);
        assert_eq!(
            none_copy(m.filtered, "No games this season yet"),
            "No games match these filters"
        );
    }

    #[test]
    fn model_lists_seasons_with_games_and_folds_the_rest() {
        let mut games = Vec::new();
        for i in 0..12 {
            let o = if i % 2 == 0 {
                Outcome::Win
            } else {
                Outcome::Loss
            };
            games.push(game(o, at(10, 8 + i)));
        }
        games.push(game(Outcome::Unknown, at(10, 7)));
        for i in 0..5 {
            games.push(game(Outcome::Loss, at(9, 1 + i)));
        }
        let m = model(&games, &seasons(), RoleFilter::default(), None, at(10, 20));
        let names: Vec<&str> = m.lines.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Season 5", "Season 4"]);
        assert_eq!(m.empty, ["Season 3", "Season 2", "Season 1"]);
        let s5 = &m.lines[0];
        assert!(s5.is_current);
        assert_eq!(s5.record.games, 13);
        assert_eq!(s5.record.undecided(), 1);
        assert_eq!(s5.pct, Some(50));
        assert_eq!(m.lines[1].pct, None, "5 games is too few");
        assert_eq!(m.all_time.record.games, 18);
        assert_eq!(m.all_time.pct, Some(35));

        let cur = m.current.expect("current season");
        assert_eq!(current_sub(&cur), "Oct 6 to Dec 8 · 49 days left");
        assert_eq!(cur.form, None, "12 decided games are the whole season");
    }

    #[test]
    fn form_takes_the_newest_twenty_decided() {
        let mut games = Vec::new();
        for i in 0..10 {
            games.push(game(Outcome::Loss, at(10, 7 + i)));
        }
        for i in 0..20 {
            games.push(game(Outcome::Win, at(11, 1 + i)));
        }
        games.push(game(Outcome::Unknown, at(11, 25)));
        let refs: Vec<&Game> = games.iter().collect();
        let f = form(&refs, Some(49)).expect("form");
        assert_eq!(f.marks.len(), LAST_N);
        assert_eq!(f.pct, 100);
        assert_eq!(f.delta, Some(51));
        assert_eq!(f.marks.first(), Some(&Outcome::Win), "oldest first");
        assert_eq!(form(&refs[..20], None), None, "20 decided is all of it");
        assert!(form(&refs[..21], None).is_some(), "21 decided");
        assert_eq!(delta_text(6), (Some('▲'), "+6 vs season".to_string()));
        assert_eq!(delta_text(-4), (Some('▼'), "-4 vs season".to_string()));
        assert_eq!(delta_text(0), (None, "same as season".to_string()));
    }

    #[test]
    fn needs_a_look_splits_by_why_and_adds_up() {
        let mut games = vec![
            game(Outcome::Unknown, at(10, 10)),
            game(Outcome::Unknown, at(10, 11)),
            game(Outcome::Unknown, at(9, 1)),
            game(Outcome::Unknown, at(1, 5)),
            game(Outcome::Win, at(10, 12)),
            game(Outcome::Win, at(10, 13)),
            game(Outcome::Win, at(10, 14)),
        ];
        games[4].hero = String::new();
        games[5].map_name = String::new();
        // The open game's missing result is not counted yet.
        games[1].session_id = "live".into();
        let live = Some("live");
        let lines = needs_lines(&games, &seasons(), live);
        let flat: Vec<(String, &str, usize)> = lines
            .iter()
            .map(|n| (n.scope.clone(), n.why, n.count))
            .collect();
        assert_eq!(
            flat,
            [
                ("Season 5".to_string(), "no outcome", 1),
                ("Season 4".to_string(), "no outcome", 1),
                ("No season".to_string(), "no outcome", 1),
                ("Any season".to_string(), "unknown hero", 1),
                ("Any season".to_string(), "no map", 1),
            ]
        );
        let sum: usize = lines.iter().map(|n| n.count).sum();
        assert_eq!(sum, crate::nav::needs_look_count(&games, live));
    }

    #[test]
    fn role_filter_applies_to_the_table() {
        let mut games = vec![game(Outcome::Win, at(10, 10))];
        games[0].role = crate::model::Role::Support;
        let tanks = RoleFilter::default().toggle(crate::model::Role::Tank);
        let m = model(&games, &seasons(), tanks, None, at(10, 20));
        assert_eq!(m.all_time.record.games, 0);
        assert!(m.lines.is_empty());
        assert_eq!(m.current.expect("still shown").line.record.games, 0);
    }

    #[test]
    fn seasons_under_three_games_are_muted_and_under_ten_decided_too_few() {
        let mut games = Vec::new();
        // Season 5: 9 games. Season 4: 2 games. Season 3: 3 games.
        for i in 0..9 {
            games.push(game(Outcome::Win, at(10, 7 + i)));
        }
        games.push(game(Outcome::Loss, at(9, 1)));
        games.push(game(Outcome::Loss, at(9, 2)));
        for i in 0..3 {
            games.push(game(Outcome::Win, at(7, 1 + i)));
        }
        // Season 2: 1 game.
        games.push(game(Outcome::Win, at(5, 1)));
        let m = model(&games, &seasons(), RoleFilter::default(), None, at(10, 20));
        let by = |n: &str| m.lines.iter().find(|l| l.name == n).expect(n).sample;
        assert_eq!(by("Season 5"), Sample::TooFew, "9 games");
        assert_eq!(by("Season 4"), Sample::Fold, "2 games");
        assert_eq!(by("Season 3"), Sample::TooFew, "3 games");
        assert_eq!(by("Season 2"), Sample::Fold, "1 game");
        assert_eq!(m.all_time.sample, Sample::Rated(87), "13 of 15");
    }

    #[test]
    fn the_open_game_is_left_out_until_it_has_a_result() {
        let mut games = vec![
            game(Outcome::Unknown, at(10, 19)),
            game(Outcome::Unknown, at(10, 18)),
        ];
        games[0].session_id = "live".into();
        let m = model(
            &games,
            &seasons(),
            RoleFilter::default(),
            Some("live"),
            at(10, 20),
        );
        assert_eq!(m.all_time.record.games, 1);
        assert_eq!(m.all_time.record.undecided(), 1);
        let m = model(&games, &seasons(), RoleFilter::default(), None, at(10, 20));
        assert_eq!(m.all_time.record.undecided(), 2);
    }

    #[test]
    fn the_rate_tile_keeps_draws() {
        let r = Record {
            games: 12,
            wins: 4,
            losses: 5,
            draws: 1,
        };
        assert_eq!(rate_tile_note(&r), "4-5-1 · 12 games");
        let r = Record {
            games: 110,
            wins: 52,
            losses: 55,
            draws: 0,
        };
        assert_eq!(rate_tile_note(&r), "52-55 · 110 games");
    }

    #[test]
    fn only_the_role_chip_shows_on_seasons() {
        use crate::overview_chips::active_count_except;
        let (_dir, mut app) = crate::fixtures::test_app();
        app.seasons.seasons = seasons();
        app.season = crate::model::SeasonSel::Season("s4".into());
        app.filter_hero = Some("Ana".into());
        app.filter_map = Some("Ilios".into());
        app.filter_outcome = Some(Outcome::Win);
        app.needs_look_filter = true;
        app.screen = crate::model::Screen::Seasons;
        assert_eq!(active_count_except(&app, HIDDEN), 0);
        app.roles = RoleFilter::default().toggle(crate::model::Role::Tank);
        assert_eq!(active_count_except(&app, HIDDEN), 1);
    }

    #[test]
    fn dates_card_copy() {
        assert!(dates_note(&[], None).starts_with("No seasons yet"));
        assert_eq!(
            dates_note(&seasons(), None),
            "Set on the website under /admin/seasons. Not refreshed yet."
        );
        assert!(
            dates_note(&seasons(), Some(at(10, 20)))
                .starts_with("Set on the website under /admin/seasons. Last refreshed Oct 20, ")
        );
    }

    #[test]
    fn views_build_at_every_width() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.seasons.seasons = seasons();
        for i in 0..12 {
            app.games.push(game(Outcome::Win, at(10, 8 + i)));
        }
        app.games.push(game(Outcome::Unknown, at(9, 1)));
        for w in [1536.0, 1318.0, 976.0] {
            let _ = view(&app, w);
            let _ = crate::maps_tab::view(&app, w);
        }
        app.games.clear();
        let _ = view(&app, 976.0);
        let _ = crate::maps_tab::view(&app, 976.0);
    }
}
