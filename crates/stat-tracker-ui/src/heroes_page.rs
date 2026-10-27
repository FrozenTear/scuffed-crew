//! The 0.6 Heroes page (spec r1.1, png/A-heroes-*): the filter row, then
//! "ALL TIME · 17 HEROES WITH 3 OR MORE GAMES · WIN RATE FROM 10 GAMES"
//! with Sort (Games, Win rate, Recent) and Cards | List on the right.
//! Cards: heroes with 10 or more games show their win rate on the shared
//! bar ([`crate::winrate_bar`]); under "3 TO 9 GAMES · RECORD ONLY" the
//! rest show their record, one dash per game out of ten and how many more
//! a win rate needs; heroes under 3 games fold into "Other heroes", with
//! the games whose hero is unknown marked as in Needs a look. List is the
//! same heroes as a table. Five cards across, two below a 1100 px window.
//! The 0.5 sidebar layout keeps its own Heroes page ([`crate::heroes`]).

use std::collections::HashMap;

use chrono::{DateTime, Local, Utc};
use iced::widget::{Column, Row, button, column, container, row, space, text};
use iced::{Alignment, Element, Fill, Padding, Task, Theme};

use crate::aggregate::{HeroAgg, aggregate_filtered, filter_games};
use crate::app::{Message, TrackerApp};
use crate::model::{Game, display_hero_name};
use crate::overview_main::{portrait, role_label, warn_chip};
use crate::overview_side::{RATE_MIN, rate};
use crate::settings_rows::{Segment, hairline, segmented};
use crate::theme::{
    self, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META, TEXT,
    TEXT_2, TEXT_3,
};
use crate::widgets;

/// Fewest games for a card; fewer fold into "Other heroes".
pub const CARD_MIN: usize = 3;
/// Window width below which two cards go across instead of five.
const TWO_ACROSS_BELOW: f32 = 1100.0;
/// Space between cards.
const GAP: f32 = 12.0;

/// How the heroes are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeroSort {
    #[default]
    Games,
    WinRate,
    Recent,
}

/// A message from the Heroes page.
#[derive(Debug, Clone)]
pub enum Msg {
    Sort(HeroSort),
    /// Cards (`false`) or List (`true`).
    List(bool),
    /// Show or hide the heroes under 3 games.
    ToggleOthers,
}

/// What the Heroes page keeps between frames.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeroesPage {
    pub sort: HeroSort,
    pub list: bool,
    pub show_others: bool,
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    let page = &mut app.heroes_page;
    match msg {
        Msg::Sort(sort) => page.sort = sort,
        Msg::List(list) => page.list = list,
        Msg::ToggleOthers => page.show_others = !page.show_others,
    }
    Task::none()
}

/// A known hero: not blank, not "Unknown".
fn known(hero: &str) -> bool {
    let h = hero.trim();
    !h.is_empty() && !h.eq_ignore_ascii_case("unknown")
}

/// The heroes split by sample size, each tier sorted: win rate shown (10
/// or more games), record only (3 to 9, or 10 with none decided), and
/// under 3 games.
pub struct Tiers<'a> {
    pub rated: Vec<&'a HeroAgg>,
    pub record: Vec<&'a HeroAgg>,
    pub others: Vec<&'a HeroAgg>,
}

/// Newest game per hero, for Recent.
pub fn last_played(games: &[&Game]) -> HashMap<String, DateTime<Utc>> {
    let mut out: HashMap<String, DateTime<Utc>> = HashMap::new();
    for g in games {
        let at = out.entry(g.hero.clone()).or_insert(g.played_at);
        if g.played_at > *at {
            *at = g.played_at;
        }
    }
    out
}

/// Order heroes: Games (most first), Win rate (highest first, then the
/// record's balance), or Recent (newest first). Ties go by name.
pub fn sort_heroes(heroes: &mut [&HeroAgg], sort: HeroSort, last: &HashMap<String, DateTime<Utc>>) {
    heroes.sort_by(|a, b| {
        let by = match sort {
            HeroSort::Games => b.record.games.cmp(&a.record.games),
            HeroSort::WinRate => {
                let key = |h: &HeroAgg| {
                    (
                        rate(&h.record).unwrap_or(-1),
                        h.record.wins as i64 - h.record.losses as i64,
                    )
                };
                key(b).cmp(&key(a))
            }
            HeroSort::Recent => last.get(&b.hero).cmp(&last.get(&a.hero)),
        };
        by.then_with(|| a.hero.cmp(&b.hero))
    });
}

pub fn tiers<'a>(
    heroes: &'a [HeroAgg],
    sort: HeroSort,
    last: &HashMap<String, DateTime<Utc>>,
) -> Tiers<'a> {
    let mut t = Tiers {
        rated: Vec::new(),
        record: Vec::new(),
        others: Vec::new(),
    };
    for h in heroes.iter().filter(|h| known(&h.hero)) {
        if rate(&h.record).is_some() {
            t.rated.push(h);
        } else if h.record.games >= CARD_MIN {
            t.record.push(h);
        } else {
            t.others.push(h);
        }
    }
    for tier in [&mut t.rated, &mut t.record, &mut t.others] {
        sort_heroes(tier, sort, last);
    }
    t
}

/// "8 games · 1 undecided · 8 of 10 for a win rate".
pub fn record_sub(h: &HeroAgg) -> String {
    let r = &h.record;
    let mut line = r.games_label();
    if r.undecided() > 0 {
        line.push_str(&format!(" · {} undecided", r.undecided()));
    }
    if r.games < RATE_MIN {
        line.push_str(&format!(" · {} of {RATE_MIN} for a win rate", r.games));
    }
    line
}

fn scope(app: &TrackerApp) -> String {
    match &app.season {
        crate::model::SeasonSel::AllTime => "All time".to_string(),
        crate::model::SeasonSel::Season(id) => app
            .seasons
            .seasons
            .iter()
            .find(|s| &s.id == id)
            .map_or_else(|| "Season".to_string(), |s| s.name.clone()),
    }
}

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let filter = crate::overview_page::filter(app);
    let shown = filter_games(&app.games, &filter);
    let stats = aggregate_filtered(&app.games, &filter);
    let last = last_played(&shown);
    let page = app.heroes_page;
    let t = tiers(&stats.heroes, page.sort, &last);
    let unknown = shown.iter().filter(|g| !known(&g.hero)).count();

    let pool = filter_games(&app.games, &app.header_filter());
    let heroes = crate::aggregate::distinct_heroes(&pool);
    let maps = crate::aggregate::distinct_maps(&pool);
    let mut col = Column::new().spacing(16).width(Fill);
    col = col.push(crate::overview_page::chip_row(
        app,
        &heroes,
        &maps,
        crate::overview_page::chips_fold(content_width),
    ));
    let carded = t.rated.len() + t.record.len();
    col = col.push(
        Row::new()
            .spacing(12)
            .align_y(Alignment::Center)
            .push(container(widgets::label_text(&format!(
                "{} · {carded} heroes with {CARD_MIN} or more games · win rate from {RATE_MIN} games",
                scope(app)
            )))
            .width(Fill))
            .push(text("Sort").size(SIZE_META).font(FONT_MEDIUM).color(TEXT_2))
            .push(sort_switch(page.sort))
            .push(view_switch(page.list))
    );
    if shown.is_empty() {
        return col
            .push(widgets::empty_surface(
                "No heroes yet. Heroes show here once the tracker saves a game.",
            ))
            .into();
    }
    if page.list {
        let mut all: Vec<&HeroAgg> = t
            .rated
            .iter()
            .chain(&t.record)
            .chain(&t.others)
            .copied()
            .collect();
        sort_heroes(&mut all, page.sort, &last);
        col = col.push(list(&all, &last));
    } else {
        let across = if crate::overview_page::window_for(content_width) < TWO_ACROSS_BELOW {
            2
        } else {
            5
        };
        if !t.rated.is_empty() {
            col = col.push(grid(&t.rated, across));
        }
        if !t.record.is_empty() {
            col = col.push(divider(&format!(
                "{CARD_MIN} to {} games · record only",
                RATE_MIN - 1
            )));
            col = col.push(grid(&t.record, across));
        }
        if !t.others.is_empty() || unknown > 0 {
            col = col.push(others_row(&t.others, unknown, page.show_others));
            if page.show_others && !t.others.is_empty() {
                col = col.push(grid(&t.others, across));
            }
        }
    }
    col.into()
}

fn sort_switch(sort: HeroSort) -> Element<'static, Message> {
    let seg = |label, s| Segment {
        label,
        selected: sort == s,
        msg: Some(Message::HeroesPage(Msg::Sort(s))),
    };
    segmented(vec![
        seg("Games", HeroSort::Games),
        seg("Win rate", HeroSort::WinRate),
        seg("Recent", HeroSort::Recent),
    ])
}

fn view_switch(list: bool) -> Element<'static, Message> {
    segmented(vec![
        Segment {
            label: "Cards",
            selected: !list,
            msg: Some(Message::HeroesPage(Msg::List(false))),
        },
        Segment {
            label: "List",
            selected: list,
            msg: Some(Message::HeroesPage(Msg::List(true))),
        },
    ])
}

/// "3 TO 9 GAMES · RECORD ONLY" with a rule to the edge.
fn divider(label: &str) -> Element<'static, Message> {
    row![
        widgets::label_text(label),
        container(hairline()).width(Fill)
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn grid(heroes: &[&HeroAgg], across: usize) -> Element<'static, Message> {
    let mut col = Column::new().spacing(GAP).width(Fill);
    for chunk in heroes.chunks(across) {
        let mut r = Row::new().spacing(GAP).width(Fill);
        for h in chunk {
            r = r.push(card(h));
        }
        for _ in chunk.len()..across {
            r = r.push(space().width(Fill));
        }
        col = col.push(r);
    }
    col.into()
}

fn head(h: &HeroAgg, size: f32) -> Element<'static, Message> {
    let mut names = column![].spacing(2);
    if let Some(label) = role_label(h.role) {
        names = names.push(
            text(label)
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(theme::role_color(h.role)),
        );
    }
    names = names.push(
        text(display_hero_name(&h.hero))
            .size(15)
            .font(FONT_BOLD)
            .color(TEXT),
    );
    row![portrait(size), names]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
}

/// A hero card: its rate on the shared bar from 10 games, else its record
/// with one dash per game out of ten.
fn card(h: &HeroAgg) -> Element<'static, Message> {
    let (value, meter, sub) = match rate(&h.record) {
        Some(p) => (
            format!("{p}%"),
            crate::winrate_bar::bar(p as f32),
            h.record.games_label(),
        ),
        None => (
            h.record.wl_label(),
            crate::overview_heroes::dashes(h.record.games),
            record_sub(h),
        ),
    };
    container(
        column![
            head(h, 44.0),
            text(value).size(24).font(FONT_EXTRABOLD).color(TEXT),
            meter,
            text(sub).size(SIZE_LABEL).font(FONT_MEDIUM).color(TEXT_3),
        ]
        .spacing(10),
    )
    .padding(Padding::from([14, 16]))
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

/// "Other heroes (13) · under 3 games each, 16 games", the unknown-hero
/// mark, and Show or Hide.
fn others_row(others: &[&HeroAgg], unknown: usize, open: bool) -> Element<'static, Message> {
    let games: usize = others.iter().map(|h| h.record.games).sum();
    let mut r = Row::new().spacing(16).align_y(Alignment::Center);
    if !others.is_empty() {
        r = r.push(
            text(format!(
                "Other heroes ({}) · under {CARD_MIN} games each, {games} {}",
                others.len(),
                if games == 1 { "game" } else { "games" }
            ))
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
        );
    }
    if unknown > 0 {
        r = r.push(warn_chip(format!(
            "{unknown} {} with unknown hero · in Needs a look",
            if unknown == 1 { "game" } else { "games" }
        )));
    }
    r = r.push(space().width(Fill));
    if !others.is_empty() {
        r = r.push(
            button(
                text(if open { "Hide" } else { "Show" })
                    .size(SIZE_META)
                    .font(FONT_BOLD)
                    .color(TEXT),
            )
            .padding(Padding::from([4, 8]))
            .style(theme::quiet_btn(6.0))
            .on_press(Message::HeroesPage(Msg::ToggleOthers)),
        );
    }
    container(r)
        .padding(Padding::from([8, 16]))
        .width(Fill)
        .style(|t: &Theme| {
            let mut s = theme::surface_panel(t);
            s.background = None;
            s
        })
        .into()
}

/// The List view: one row per hero with its role, games, record, win rate
/// (the shared bar, or "Too few games" under 10) and when it was last
/// played.
fn list(heroes: &[&HeroAgg], last: &HashMap<String, DateTime<Utc>>) -> Element<'static, Message> {
    let label = |s: &str| widgets::label_text(s);
    let mut col = Column::new().spacing(0).width(Fill);
    col = col.push(
        container(
            row![
                container(label("Hero")).width(iced::Length::FillPortion(3)),
                container(label("Games")).width(70),
                container(label("Record")).width(90),
                container(label("Win rate")).width(iced::Length::FillPortion(2)),
                container(label("Last played")).width(110),
            ]
            .spacing(12),
        )
        .padding(Padding::from([10, 16])),
    );
    for h in heroes {
        let rate_cell: Element<'static, Message> = match rate(&h.record) {
            Some(p) => row![
                text(format!("{p}%"))
                    .size(SIZE_META)
                    .font(FONT_BOLD)
                    .color(TEXT)
                    .width(44),
                crate::winrate_bar::bar(p as f32),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
            None => text("Too few games")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3)
                .into(),
        };
        let when = last
            .get(&h.hero)
            .map(|at| at.with_timezone(&Local).format("%b %-d").to_string())
            .unwrap_or_default();
        col = col.push(hairline()).push(
            container(
                row![
                    container(head(h, 28.0)).width(iced::Length::FillPortion(3)),
                    container(
                        text(h.record.games.to_string())
                            .size(SIZE_META)
                            .font(FONT_SEMIBOLD)
                            .color(TEXT)
                    )
                    .width(70),
                    container(
                        text(h.record.wl_label())
                            .size(SIZE_META)
                            .font(FONT_SEMIBOLD)
                            .color(TEXT)
                    )
                    .width(90),
                    container(rate_cell).width(iced::Length::FillPortion(2)),
                    container(text(when).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_2))
                        .width(110),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([8, 16])),
        );
    }
    container(col)
        .width(Fill)
        .clip(true)
        .style(theme::surface_panel)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::Record;
    use crate::model::Role;
    use chrono::TimeZone;

    fn hero(name: &str, games: usize, wins: usize, losses: usize) -> HeroAgg {
        HeroAgg {
            hero: name.into(),
            role: Role::Tank,
            record: Record {
                games,
                wins,
                losses,
                draws: 0,
            },
        }
    }

    fn names(hs: &[&HeroAgg]) -> Vec<String> {
        hs.iter().map(|h| h.hero.clone()).collect()
    }

    #[test]
    fn heroes_split_at_ten_and_three_games() {
        let heroes = vec![
            hero("Wrecking Ball", 624, 330, 290),
            hero("Zarya", 12, 0, 12),
            hero("Sierra", 8, 3, 4),
            hero("Domina", 3, 0, 3),
            hero("Orisa", 2, 1, 1),
            hero("Unknown", 9, 0, 0),
            hero("Undecided", 10, 0, 0),
        ];
        let t = tiers(&heroes, HeroSort::Games, &HashMap::new());
        assert_eq!(names(&t.rated), ["Wrecking Ball", "Zarya"]);
        assert_eq!(names(&t.record), ["Undecided", "Sierra", "Domina"]);
        assert_eq!(names(&t.others), ["Orisa"]);
    }

    #[test]
    fn record_cards_say_how_many_more_games_a_rate_needs() {
        let mut h = hero("Sierra", 8, 3, 4);
        assert_eq!(
            record_sub(&h),
            "8 games · 1 undecided · 8 of 10 for a win rate"
        );
        h.record.wins = 4;
        assert_eq!(record_sub(&h), "8 games · 8 of 10 for a win rate");
    }

    #[test]
    fn sorts_by_games_rate_or_recent() {
        let heroes = [
            hero("A", 20, 15, 5),
            hero("B", 50, 20, 30),
            hero("C", 12, 6, 6),
        ];
        let mut hs: Vec<&HeroAgg> = heroes.iter().collect();
        sort_heroes(&mut hs, HeroSort::Games, &HashMap::new());
        assert_eq!(names(&hs), ["B", "A", "C"]);
        sort_heroes(&mut hs, HeroSort::WinRate, &HashMap::new());
        assert_eq!(names(&hs), ["A", "C", "B"]);
        let at = |m| Utc.with_ymd_and_hms(2026, 10, 18, 22, m, 0).unwrap();
        let last = HashMap::from([("A".to_string(), at(1)), ("C".to_string(), at(9))]);
        sort_heroes(&mut hs, HeroSort::Recent, &last);
        assert_eq!(names(&hs), ["C", "A", "B"]);
    }

    #[test]
    fn page_state_follows_its_messages() {
        let (_dir, mut app) = crate::fixtures::test_app();
        let _ = app.update(Message::HeroesPage(Msg::Sort(HeroSort::Recent)));
        let _ = app.update(Message::HeroesPage(Msg::List(true)));
        let _ = app.update(Message::HeroesPage(Msg::ToggleOthers));
        assert_eq!(app.heroes_page.sort, HeroSort::Recent);
        assert!(app.heroes_page.list && app.heroes_page.show_others);
        let _ = view(&app, 1300.0);
        let _ = view(&app, 900.0);
    }
}
