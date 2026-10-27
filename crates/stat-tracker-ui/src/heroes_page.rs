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

use chrono::{DateTime, Datelike, Local, Utc};
use iced::widget::{Column, Row, button, column, container, row, space, text};
use iced::{Alignment, Element, Fill, Padding, Task, Theme};

use crate::aggregate::{GameFilter, HeroAgg, aggregate_games, filter_games, hero_key};
use crate::app::{Message, TrackerApp};
use crate::model::{Game, display_hero_name};
use crate::overview_main::{portrait, role_label, warn_chip};
use crate::settings_rows::{Segment, hairline, segmented};
use crate::small_sample::{RATE_MIN, Sample, TOO_FEW, rate, sample};
use crate::theme::{
    self, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META, TEXT,
    TEXT_2, TEXT_3,
};
use crate::widgets;

/// Fewest games for a card; fewer fold into "Other heroes". The same
/// rule as the Overview strip.
pub use crate::overview_heroes::CARD_MIN;
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

/// Cards or List.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeroView {
    #[default]
    Cards,
    List,
}

/// A message from the Heroes page.
#[derive(Debug, Clone)]
pub enum Msg {
    Sort(HeroSort),
    View(HeroView),
    /// Show or hide the heroes under 3 games.
    ToggleOthers,
}

/// What the Heroes page keeps between frames.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeroesPage {
    pub sort: HeroSort,
    pub view: HeroView,
    pub show_others: bool,
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    let page = &mut app.heroes_page;
    match msg {
        Msg::Sort(sort) => page.sort = sort,
        Msg::View(view) => page.view = view,
        Msg::ToggleOthers => page.show_others = !page.show_others,
    }
    Task::none()
}

/// A known hero: not blank, not "Unknown".
fn known(hero: &str) -> bool {
    !stat_tracker::parse::hero_is_unknown_label(hero)
}

/// The games the Heroes stats count: the season, role and map filters.
/// The Games tab's outcome and hero filters stay on Games: a win-only
/// filter would show every hero at 100%, and a hero filter would leave one
/// card.
pub fn stats_filter(app: &TrackerApp) -> GameFilter {
    GameFilter {
        outcome: None,
        hero: None,
        ..crate::overview_page::filter(app)
    }
}

/// Window width below which cards use the smaller portrait and padding.
const ROOMY_FROM: f32 = 1440.0;

/// Portrait size and card padding: 48 px and 16 px from a 1440 px window,
/// 40 px and 12 by 16 px below it.
fn card_metrics(roomy: bool) -> (f32, Padding) {
    if roomy {
        (48.0, Padding::from(16))
    } else {
        (40.0, Padding::from([12, 16]))
    }
}

/// "Oct 18", or "Oct 18, 2025" when it is not this year.
pub fn played_label(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let at = at.with_timezone(&Local);
    if at.year() == now.with_timezone(&Local).year() {
        at.format("%b %-d").to_string()
    } else {
        at.format("%b %-d, %Y").to_string()
    }
}

/// The heroes split by [`crate::small_sample`], each tier sorted: win
/// rate (10 or more decided games), "Too few games" (3 or more games,
/// under 10 decided), and under 3 games in total.
pub struct Tiers<'a> {
    pub rated: Vec<&'a HeroAgg>,
    pub record: Vec<&'a HeroAgg>,
    pub others: Vec<&'a HeroAgg>,
}

/// Newest game per hero, for Recent.
pub fn last_played(games: &[&Game]) -> HashMap<String, DateTime<Utc>> {
    let mut out: HashMap<String, DateTime<Utc>> = HashMap::new();
    for g in games {
        let at = out.entry(hero_key(&g.hero)).or_insert(g.played_at);
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
        match sample(&h.record) {
            Sample::Rated(_) => t.rated.push(h),
            Sample::TooFew => t.record.push(h),
            Sample::Fold => t.others.push(h),
        }
    }
    for tier in [&mut t.rated, &mut t.record, &mut t.others] {
        sort_heroes(tier, sort, last);
    }
    t
}

/// The sub-line under a record card's W-L: "Too few games · 8 games ·
/// 1 undecided", the one wording every page uses below 10 decided games.
pub fn record_sub(h: &HeroAgg) -> String {
    let r = &h.record;
    let mut line = format!("{TOO_FEW} · {}", r.games_label());
    if r.undecided() > 0 {
        line.push_str(&format!(" · {} undecided", r.undecided()));
    }
    line
}

/// "ALL TIME · 1 HERO WITH 3 OR MORE GAMES · WIN RATE FROM 10 DECIDED GAMES".
pub fn header_line(scope: &str, carded: usize) -> String {
    let heroes = if carded == 1 { "hero" } else { "heroes" };
    format!(
        "{scope} · {carded} {heroes} with {CARD_MIN} or more games · win rate from {RATE_MIN} decided games"
    )
}

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let shown = filter_games(&app.games, &stats_filter(app));
    let stats = aggregate_games(&shown);
    let last = last_played(&shown);
    let page = app.heroes_page;
    let t = tiers(&stats.heroes, page.sort, &last);
    // All time and every role, like the top bar's Needs a look count.
    let unknown = app.games.iter().filter(|g| !known(&g.hero)).count();
    let roomy = crate::overview_page::window_for(content_width) >= ROOMY_FROM;

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
            .push(
                container(widgets::label_text(&header_line(
                    &crate::overview_heroes::scope(app),
                    carded,
                )))
                .width(Fill),
            )
            .push(text("Sort").size(SIZE_META).font(FONT_MEDIUM).color(TEXT_2))
            .push(sort_switch(page.sort))
            .push(view_switch(page.view)),
    );
    if shown.is_empty() {
        return col
            .push(widgets::empty_surface(empty_copy(&app.games)))
            .into();
    }
    if page.view == HeroView::List {
        let now = Utc::now();
        col = col.push(list(&list_groups(&t), &last, now));
        if !t.others.is_empty() || unknown > 0 {
            col = col.push(others_row(&t.others, unknown, page.show_others));
            if page.show_others && !t.others.is_empty() {
                col = col.push(list(&[(String::new(), t.others.clone())], &last, now));
            }
        }
    } else {
        let across = if crate::overview_page::window_for(content_width) < TWO_ACROSS_BELOW {
            2
        } else {
            5
        };
        if !t.rated.is_empty() {
            col = col.push(grid(&t.rated, across, roomy));
        }
        if !t.record.is_empty() {
            col = col.push(divider(TOO_FEW));
            col = col.push(grid(&t.record, across, roomy));
        }
        if !t.others.is_empty() || unknown > 0 {
            col = col.push(others_row(&t.others, unknown, page.show_others));
            if page.show_others && !t.others.is_empty() {
                col = col.push(grid(&t.others, across, roomy));
            }
        }
    }
    col.into()
}

/// The empty page: no games at all, or filters that leave none.
pub fn empty_copy(games: &[Game]) -> &'static str {
    if games.is_empty() {
        "No heroes yet. Heroes show here once the tracker saves a game."
    } else {
        "No games match these filters"
    }
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

fn view_switch(view: HeroView) -> Element<'static, Message> {
    let seg = |label, v| Segment {
        label,
        selected: view == v,
        msg: Some(Message::HeroesPage(Msg::View(v))),
    };
    segmented(vec![
        seg("Cards", HeroView::Cards),
        seg("List", HeroView::List),
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

fn grid(heroes: &[&HeroAgg], across: usize, roomy: bool) -> Element<'static, Message> {
    let mut col = Column::new().spacing(GAP).width(Fill);
    for chunk in heroes.chunks(across) {
        let mut r = Row::new().spacing(GAP).width(Fill);
        for h in chunk {
            r = r.push(card(h, roomy));
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
fn card(h: &HeroAgg, roomy: bool) -> Element<'static, Message> {
    let (portrait_px, pad) = card_metrics(roomy);
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
            head(h, portrait_px),
            text(value).size(24).font(FONT_EXTRABOLD).color(TEXT),
            meter,
            text(sub).size(SIZE_LABEL).font(FONT_MEDIUM).color(TEXT_3),
        ]
        .spacing(10),
    )
    .padding(pad)
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

/// The List view's groups: "10 or more games · win rate" and "Too few
/// games", the same tiers as the cards. Heroes under 3 games stay folded
/// under the "Other heroes" row.
pub fn list_groups<'a>(t: &Tiers<'a>) -> Vec<(String, Vec<&'a HeroAgg>)> {
    let mut groups = Vec::new();
    if !t.rated.is_empty() {
        groups.push((
            format!("{RATE_MIN} or more games · win rate"),
            t.rated.clone(),
        ));
    }
    if !t.record.is_empty() {
        groups.push((TOO_FEW.to_string(), t.record.clone()));
    }
    groups
}

/// The List view: one row per hero with its role, games, record, win rate
/// (the shared bar, or "Too few games" under 10) and when it was last
/// played, under a group row per tier (`label` empty for none).
fn list(
    groups: &[(String, Vec<&HeroAgg>)],
    last: &HashMap<String, DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Element<'static, Message> {
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
    for (group, heroes) in groups {
        if !group.is_empty() {
            col = col
                .push(hairline())
                .push(container(label(group)).padding(Padding::from([8, 16])));
        }
        for h in heroes {
            col = col.push(hairline()).push(list_row(h, last, now));
        }
    }
    container(col)
        .width(Fill)
        .clip(true)
        .style(theme::surface_panel)
        .into()
}

fn list_row(
    h: &HeroAgg,
    last: &HashMap<String, DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Element<'static, Message> {
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
        None => text(TOO_FEW)
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3)
            .into(),
    };
    let when = last
        .get(&h.hero)
        .map(|at| played_label(*at, now))
        .unwrap_or_default();
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
            container(text(when).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_2)).width(110),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 16]))
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
    fn record_cards_say_too_few_games_with_count_and_undecided() {
        let mut h = hero("Sierra", 8, 3, 4);
        assert_eq!(record_sub(&h), "Too few games · 8 games · 1 undecided");
        h.record.wins = 4;
        assert_eq!(record_sub(&h), "Too few games · 8 games");
        assert_eq!(h.record.wl_label(), "4-4", "the card's W-L");
    }

    #[test]
    fn a_rate_needs_ten_decided_games_and_folding_counts_every_game() {
        let heroes = vec![
            hero("Wrecking Ball", 40, 5, 4),
            hero("Sierra", 12, 5, 4),
            hero("Domina", 3, 0, 0),
            hero("Kiriko", 11, 6, 4),
            hero("Orisa", 2, 2, 0),
        ];
        let t = tiers(&heroes, HeroSort::Games, &HashMap::new());
        assert_eq!(names(&t.rated), ["Kiriko"], "10 decided of 11");
        assert_eq!(
            names(&t.record),
            ["Wrecking Ball", "Sierra", "Domina"],
            "under 10 decided, by games"
        );
        assert_eq!(
            names(&t.others),
            ["Orisa"],
            "2 games fold even when decided"
        );
        assert_eq!(
            header_line("All time", 1),
            "All time · 1 hero with 3 or more games · win rate from 10 decided games"
        );
        assert!(header_line("Season 5", 3).contains("3 heroes"));
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
    fn the_rate_starts_at_exactly_ten_games() {
        let nine = hero("Sierra", 9, 5, 4);
        let ten = hero("Kiriko", 10, 6, 4);
        let even = hero("Juno", 10, 5, 5);
        assert_eq!(rate(&nine.record), None);
        assert_eq!(record_sub(&nine), "Too few games · 9 games");
        assert_eq!(rate(&ten.record), Some(60));
        assert_eq!(rate(&even.record), Some(50));
        assert_eq!(
            crate::winrate_bar::fill_px(50.0, 200.0),
            0.0,
            "exactly 50% is the bare tick"
        );
        let heroes = vec![nine, ten, even];
        let t = tiers(&heroes, HeroSort::Games, &HashMap::new());
        assert_eq!(names(&t.rated), ["Juno", "Kiriko"]);
        assert_eq!(names(&t.record), ["Sierra"]);
    }

    #[test]
    fn stats_ignore_the_games_outcome_and_hero_filters() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.filter_outcome = Some(crate::model::Outcome::Win);
        app.filter_hero = Some("Ana".into());
        app.filter_map = Some("Ilios".into());
        let f = stats_filter(&app);
        assert_eq!(f.outcome, None);
        assert_eq!(f.hero, None);
        assert_eq!(f.map.as_deref(), Some("Ilios"), "map still applies");
    }

    #[test]
    fn one_hero_whatever_the_stored_spelling() {
        let mut games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        games.truncate(3);
        games[0].hero = "Soldier: 76".into();
        games[1].hero = "soldier_76".into();
        games[2].hero = "SOLDIER-76".into();
        let stats = crate::aggregate::aggregate(&games, None, None);
        assert_eq!(stats.heroes.len(), 1);
        assert_eq!(stats.heroes[0].hero, "Soldier: 76");
        assert_eq!(stats.heroes[0].record.games, 3);
        let refs: Vec<&Game> = games.iter().collect();
        assert_eq!(last_played(&refs).len(), 1);
        assert!(last_played(&refs).contains_key("Soldier: 76"));
        assert_eq!(hero_key("Unknown"), "Unknown");
        assert_eq!(hero_key(""), "");
    }

    #[test]
    fn empty_page_says_why() {
        assert_eq!(
            empty_copy(&[]),
            "No heroes yet. Heroes show here once the tracker saves a game."
        );
        let games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        assert_eq!(empty_copy(&games), "No games match these filters");
    }

    #[test]
    fn list_keeps_the_tiers_and_folds_small_heroes() {
        let heroes = vec![
            hero("Wrecking Ball", 624, 330, 290),
            hero("Sierra", 8, 3, 4),
            hero("Orisa", 2, 1, 1),
        ];
        let t = tiers(&heroes, HeroSort::Games, &HashMap::new());
        let groups = list_groups(&t);
        let flat: Vec<(String, Vec<String>)> = groups
            .iter()
            .map(|(g, hs)| (g.clone(), names(hs)))
            .collect();
        assert_eq!(
            flat,
            [
                (
                    "10 or more games · win rate".to_string(),
                    vec!["Wrecking Ball".to_string()]
                ),
                ("Too few games".to_string(), vec!["Sierra".to_string()]),
            ],
            "Orisa stays under Other heroes"
        );
    }

    #[test]
    fn cards_shrink_below_1440_and_old_dates_carry_the_year() {
        assert_eq!(card_metrics(true), (48.0, Padding::from(16)));
        assert_eq!(card_metrics(false), (40.0, Padding::from([12, 16])));
        let now = Utc.with_ymd_and_hms(2026, 10, 27, 12, 0, 0).unwrap();
        let this_year = Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap();
        let last_year = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
        assert_eq!(played_label(this_year, now), "Jun 15");
        assert_eq!(played_label(last_year, now), "Jun 15, 2025");
    }

    #[test]
    fn page_state_follows_its_messages() {
        let (_dir, mut app) = crate::fixtures::test_app();
        let _ = app.update(Message::HeroesPage(Msg::Sort(HeroSort::Recent)));
        let _ = app.update(Message::HeroesPage(Msg::View(HeroView::List)));
        let _ = app.update(Message::HeroesPage(Msg::ToggleOthers));
        assert_eq!(app.heroes_page.sort, HeroSort::Recent);
        assert_eq!(app.heroes_page.view, HeroView::List);
        assert!(app.heroes_page.show_others);
    }

    #[test]
    fn cards_and_list_draw_sample_games_at_both_widths() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        let shown = filter_games(&app.games, &stats_filter(&app));
        assert!(!shown.is_empty(), "past the empty state");
        assert!(!aggregate_games(&shown).heroes.is_empty());
        for view_kind in [HeroView::Cards, HeroView::List] {
            let _ = app.update(Message::HeroesPage(Msg::View(view_kind)));
            for others in [false, true] {
                app.heroes_page.show_others = others;
                let _ = view(&app, 1300.0);
                let _ = view(&app, 900.0);
            }
        }
    }
}
