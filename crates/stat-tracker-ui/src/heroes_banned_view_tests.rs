//! Tests for the When banned view (`heroes_banned_view`): the words the
//! drawn page carries, read back through a headless layout.

use super::*;
use crate::heroes_page::Msg as HeroesMsg;
use crate::model::{Game, Outcome};
use iced::Rectangle;
use iced::advanced::Layout;
use iced::advanced::layout::Limits;
use iced::advanced::widget::operation::Operation;
use iced::advanced::widget::{Id, Tree};

fn headless() -> iced::Renderer {
    use iced::advanced::renderer::Headless;
    let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
        FONT_MEDIUM,
        iced::Pixels(SIZE_BODY),
        Some("tiny-skia"),
    ))
    .expect("the tiny-skia renderer needs no display");
    for bytes in [
        theme::FONT_BYTES_MEDIUM,
        theme::FONT_BYTES_SEMIBOLD,
        theme::FONT_BYTES_BOLD,
        theme::FONT_BYTES_EXTRABOLD,
    ] {
        iced::advanced::graphics::text::font_system()
            .write()
            .expect("font system lock")
            .load_font(std::borrow::Cow::Borrowed(bytes));
    }
    renderer
}

struct Collect(Vec<String>);

impl Operation<()> for Collect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        operate(self);
    }

    fn text(&mut self, _id: Option<&Id>, _bounds: Rectangle, text: &str) {
        self.0.push(text.to_string());
    }
}

/// Every text the element draws at `width`.
fn texts(mut element: Element<'_, Message>, width: f32) -> Vec<String> {
    let renderer = headless();
    let mut tree = Tree::new(&element);
    let node = element.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &Limits::new(iced::Size::ZERO, iced::Size::new(width, 4000.0)),
    );
    let mut collect = Collect(Vec::new());
    element
        .as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut collect);
    collect.0
}

fn has(texts: &[String], words: &str) -> bool {
    texts.iter().any(|t| t == words)
}

fn sample() -> Vec<Game> {
    crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
        crate::cli::FixtureKind::Sample,
    ))
}

/// `n` games where `hero` was banned (own team for the first `own`, enemy
/// for the rest), `wins` of them won.
fn banned(hero: &str, n: usize, own: usize, wins: usize) -> Vec<Game> {
    let base = sample().remove(0);
    (0..n)
        .map(|i| {
            let mut g = base.clone();
            g.session_id = format!("{hero}-{i}");
            let team = if i < own { "own" } else { "enemy" };
            g.bans_picks = Some(format!(
                r#"{{"bans":[{{"value":"{hero}","team":"{team}"}}],"bans_picks_recognizer":"cv-v1"}}"#
            ));
            g.outcome = if i < wins { Outcome::Win } else { Outcome::Loss };
            g
        })
        .collect()
}

fn app_with(games: Vec<Game>) -> (tempfile::TempDir, TrackerApp) {
    let (dir, mut app) = crate::fixtures::test_app();
    app.games = games;
    app.screen = crate::model::Screen::Heroes;
    (dir, app)
}

fn when_banned(app: &mut TrackerApp) {
    let _ = app.update(Message::HeroesPage(HeroesMsg::Mode(Mode::WhenBanned)));
}

fn page(app: &TrackerApp) -> Vec<String> {
    texts(crate::heroes_page::view(app, 1232.0), 1232.0)
}

#[test]
fn the_heading_carries_played_and_when_banned_and_starts_on_played() {
    let (_dir, mut app) = app_with(sample());
    assert_eq!(app.heroes_page.mode, Mode::Played);
    let words = texts(heading(&app), 1232.0);
    assert!(has(&words, "Heroes"), "{words:?}");
    assert!(
        has(&words, "Played") && has(&words, "When banned"),
        "{words:?}"
    );
    when_banned(&mut app);
    assert_eq!(app.heroes_page.mode, Mode::WhenBanned);
    app.screen = crate::model::Screen::Maps;
    let maps = texts(heading(&app), 1232.0);
    assert!(!has(&maps, "When banned"), "only on Heroes: {maps:?}");
}

#[test]
fn cards_and_list_show_on_played_only() {
    let (_dir, mut app) = app_with(sample());
    let played = page(&app);
    assert!(has(&played, "Cards") && has(&played, "List"), "{played:?}");
    when_banned(&mut app);
    let banned = page(&app);
    assert!(
        !has(&banned, "Cards") && !has(&banned, "List"),
        "{banned:?}"
    );
    assert!(has(&banned, CAPTION), "{banned:?}");
}

#[test]
fn no_games_at_all_is_no_bans_yet() {
    let (_dir, mut app) = app_with(Vec::new());
    when_banned(&mut app);
    let words = page(&app);
    assert!(has(&words, "No bans yet"), "{words:?}");
    assert!(has(
        &words,
        "They show here once the tracker saves a game with a ban read."
    ));
    assert!(!has(&words, "Sort"), "{words:?}");
}

#[test]
fn games_with_no_ban_read_say_how_many_and_name_the_reader_when_off() {
    let mut games = sample();
    for g in &mut games {
        g.bans_picks = None;
    }
    let n = games.len();
    let (_dir, mut app) = app_with(games);
    when_banned(&mut app);
    let words = page(&app);
    assert!(has(&words, "No bans read yet"), "{words:?}");
    let line = format!("{n} games in this view have no ban read. They are left out of this table.");
    assert!(has(&words, &line), "{words:?}");
    assert!(!has(&words, "Sort"), "no Sort with no rows");
    assert!(
        !words.iter().any(|w| w.contains("WITH A BAN READ")),
        "no stats line: {words:?}"
    );
    app.saved_config.bans_picks_reader = false;
    let off = page(&app);
    assert!(has(&off, &format!("{line} {READER_OFF}")), "{off:?}");
}

#[test]
fn filters_that_leave_no_games_say_so() {
    let (_dir, mut app) = app_with(banned("ana", 3, 1, 2));
    when_banned(&mut app);
    app.filter_map = Some("Nowhere".into());
    let words = page(&app);
    assert!(has(&words, "No games match these filters"), "{words:?}");
    assert!(has(
        &words,
        "Clear a chip above to see when a hero was banned."
    ));
    assert!(!has(&words, "Sort"));
}

#[test]
fn under_ten_decided_shows_too_few_and_no_percent() {
    let (_dir, mut app) = app_with(banned("ana", 9, 4, 5));
    when_banned(&mut app);
    let words = page(&app);
    assert!(has(&words, "Too few games (9 of 10)"), "{words:?}");
    assert!(!words.iter().any(|w| w.ends_with('%')), "{words:?}");
    assert!(
        has(&words, "Your ban 4") && has(&words, "Enemy 5"),
        "{words:?}"
    );
    assert!(has(&words, "Sort"));
    assert!(
        has(&words, "ALL TIME · 1 HERO · 9 GAMES WITH A BAN READ"),
        "{words:?}"
    );
    assert!(!words.iter().any(|w| w.starts_with(SIDE_NOT_READ)));
}

#[test]
fn ten_decided_shows_the_percent() {
    let mut games = banned("ana", 10, 3, 6);
    let mut none = sample().remove(0);
    none.session_id = "no-ban".into();
    none.bans_picks = None;
    games.push(none);
    let (_dir, mut app) = app_with(games);
    when_banned(&mut app);
    let words = page(&app);
    assert!(has(&words, "60%"), "{words:?}");
    assert!(!words.iter().any(|w| w.starts_with("Too few")), "{words:?}");
    assert!(has(&words, "Your ban 3") && has(&words, "Enemy 7"));
    assert!(
        has(
            &words,
            "ALL TIME · 1 HERO · 10 GAMES WITH A BAN READ · 1 GAME WITH NO BAN READ LEFT OUT"
        ),
        "{words:?}"
    );
    assert!(has(&words, "Ana") && has(&words, "SUPPORT"));
    // Both widths draw.
    let _ = texts(crate::heroes_page::view(&app, 952.0), 952.0);
}

#[test]
fn a_side_not_read_shows_only_when_there_is_one() {
    let mut r = crate::heroes_banned::aggregate(&[]).rows;
    assert!(r.is_empty());
    let games = banned("ana", 1, 1, 1);
    let refs: Vec<&Game> = games.iter().collect();
    r = crate::heroes_banned::aggregate(&refs).rows;
    assert_eq!(split_words(&r[0]), ["Your ban 1", "Enemy 0"]);
    r[0].side_not_read = 2;
    assert_eq!(
        split_words(&r[0]),
        ["Your ban 1", "Enemy 0", "Side not read 2"]
    );
}

/// The words of [`split_parts`], in order.
fn split_words(r: &BanRow) -> Vec<String> {
    split_parts(r).into_iter().map(|p| p.words).collect()
}

/// The one row of `games`.
fn only_row(games: &[Game]) -> BanRow {
    let refs: Vec<&Game> = games.iter().collect();
    let mut rows = crate::heroes_banned::aggregate(&refs).rows;
    assert_eq!(rows.len(), 1);
    rows.remove(0)
}

fn part(words: &str, tag: bool) -> SplitPart {
    SplitPart {
        words: words.into(),
        tag,
    }
}

#[test]
fn your_ban_is_tinted_only_above_zero() {
    let own = only_row(&banned("ana", 2, 1, 1));
    assert_eq!(
        split_parts(&own),
        [part("Your ban 1", true), part("Enemy 1", false)]
    );
    let enemy = only_row(&banned("ana", 2, 0, 1));
    assert_eq!(
        split_parts(&enemy),
        [part("Your ban 0", false), part("Enemy 2", false)],
        "the words stay, plain like Enemy"
    );
}

/// A game where `hero` is only the lobby ban, the slots unread.
fn lobby_only(hero: &str, i: usize) -> Game {
    let mut g = sample().remove(0);
    g.session_id = format!("lobby-{hero}-{i}");
    g.bans_picks = Some(format!(
        r#"{{"bans":[{{"value":null,"team":null}}],"lobby_ban":{{"value":"{hero}"}},"bans_picks_recognizer":"cv-v1"}}"#
    ));
    g
}

#[test]
fn a_lobby_only_row_shows_only_the_lobby_count() {
    let row = only_row(&[lobby_only("sigma", 0), lobby_only("sigma", 1)]);
    assert_eq!((row.own, row.enemy, row.lobby), (0, 0, 2));
    assert_eq!(split_parts(&row), [part("Lobby 2", false)]);

    let mut unread = row.clone();
    unread.lobby = 0;
    unread.side_not_read = 1;
    assert_eq!(split_words(&unread), ["Side not read 1"]);

    let (_dir, mut app) = app_with(vec![lobby_only("sigma", 0)]);
    when_banned(&mut app);
    let words = page(&app);
    assert!(has(&words, "Sigma") && has(&words, "Lobby 1"), "{words:?}");
    assert!(
        !words
            .iter()
            .any(|w| w.starts_with(YOUR_BAN) || w.starts_with(ENEMY)),
        "{words:?}"
    );
}

#[test]
fn a_mixed_row_keeps_both_sides_and_adds_lobby_and_side_not_read() {
    let mut games = banned("sigma", 2, 0, 1);
    games.push(lobby_only("sigma", 0));
    let mut row = only_row(&games);
    assert_eq!((row.own, row.enemy, row.lobby), (0, 2, 1));
    row.side_not_read = 1;
    assert_eq!(
        split_parts(&row),
        [
            part("Your ban 0", false),
            part("Enemy 2", false),
            part("Side not read 1", false),
            part("Lobby 1", false),
        ]
    );
}

/// [`banned`], every game played `days` after the sample game.
fn banned_on(hero: &str, n: usize, own: usize, wins: usize, days: i64) -> Vec<Game> {
    let mut games = banned(hero, n, own, wins);
    for g in &mut games {
        g.played_at += chrono::Duration::days(days);
    }
    games
}

/// Where each name first shows in the drawn page, in drawn order.
fn drawn_order(words: &[String], heroes: &[&'static str]) -> Vec<&'static str> {
    let mut at: Vec<(usize, &'static str)> = heroes
        .iter()
        .map(|h| {
            let i = words
                .iter()
                .position(|w| w == h)
                .unwrap_or_else(|| panic!("{h} not drawn: {words:?}"));
            (i, *h)
        })
        .collect();
    at.sort();
    at.into_iter().map(|(_, h)| h).collect()
}

#[test]
fn the_drawn_table_follows_the_sort() {
    // Drawn order never matches the list order (Ana, Mercy, Orisa) by
    // chance: Orisa's games are the newest, then Ana's, then Mercy's.
    let mut games = banned_on("orisa", 12, 0, 3, 3);
    games.extend(banned_on("ana", 10, 0, 5, 2));
    games.extend(banned_on("mercy", 11, 0, 9, 1));
    games.rotate_left(12);
    let heroes = ["Ana", "Mercy", "Orisa"];
    let (_dir, mut app) = app_with(games);
    when_banned(&mut app);
    assert_eq!(
        drawn_order(&page(&app), &heroes),
        ["Orisa", "Mercy", "Ana"],
        "Games, high to low"
    );
    let sort = |app: &mut TrackerApp, key| {
        let _ = app.update(Message::HeroesPage(HeroesMsg::Banned(
            crate::heroes_banned::Msg::Sort(key),
        )));
    };
    sort(&mut app, SortKey::WinRate);
    assert_eq!(
        drawn_order(&page(&app), &heroes),
        ["Mercy", "Ana", "Orisa"],
        "82%, 50%, 25%"
    );
    sort(&mut app, SortKey::Recent);
    assert_eq!(drawn_order(&page(&app), &heroes), ["Orisa", "Ana", "Mercy"]);
}

#[test]
fn decided_is_its_own_count_next_to_games() {
    let mut games = banned("ana", 5, 1, 2);
    games[3].outcome = Outcome::Unknown;
    games[4].outcome = Outcome::Unknown;
    let (_dir, mut app) = app_with(games);
    when_banned(&mut app);
    let words = page(&app);
    let at = words.iter().position(|w| w == "Ana").expect("Ana drawn");
    assert_eq!(
        words[at + 1..at + 4],
        ["5", "3", "2-1"],
        "Games, Decided, record: {words:?}"
    );
}

#[test]
fn reader_off_with_rows_is_one_line_under_the_stats_line() {
    let (_dir, mut app) = app_with(banned("ana", 3, 1, 2));
    when_banned(&mut app);
    let on = page(&app);
    assert!(!on.iter().any(|w| w.contains(READER_OFF)), "{on:?}");

    app.saved_config.bans_picks_reader = false;
    let off = page(&app);
    let hits: Vec<usize> = off
        .iter()
        .enumerate()
        .filter(|(_, w)| w.contains(READER_OFF))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(hits.len(), 1, "once: {off:?}");
    assert_eq!(off[hits[0]], READER_OFF, "a line of its own: {off:?}");
    assert!(!has(&off, "No bans read yet"), "not the empty card");
    let stats = off
        .iter()
        .position(|w| w.starts_with("ALL TIME · 1 HERO"))
        .expect("stats line");
    let ana = off
        .iter()
        .position(|w| w == "Ana")
        .expect("table still drawn");
    assert!(stats < hits[0] && hits[0] < ana, "{off:?}");
    assert!(has(&off, "Your ban 1") && has(&off, "Sort"), "{off:?}");
}

/// Every text the element draws at `width`, with its laid-out bounds.
fn texts_at(mut element: Element<'_, Message>, width: f32) -> Vec<(String, Rectangle)> {
    struct At(Vec<(String, Rectangle)>);
    impl Operation<()> for At {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            operate(self);
        }
        fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
            self.0.push((text.to_string(), bounds));
        }
    }
    let renderer = headless();
    let mut tree = Tree::new(&element);
    let node = element.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &Limits::new(iced::Size::ZERO, iced::Size::new(width, 4000.0)),
    );
    let mut at = At(Vec::new());
    element
        .as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut at);
    at.0
}

#[test]
fn nothing_is_cut_at_the_720_px_minimum() {
    // 720 px window, 24 px gutters.
    const CONTENT_720: f32 = 672.0;
    let mut games = banned("wrecking-ball", 10, 3, 6);
    games.extend(banned("junker-queen", 4, 1, 2));
    let mut none = sample().remove(0);
    none.session_id = "no-ban".into();
    none.bans_picks = None;
    games.push(none);
    let (_dir, mut app) = app_with(games);
    when_banned(&mut app);
    let line = "ALL TIME · 2 HEROES · 14 GAMES WITH A BAN READ · 1 GAME WITH NO BAN READ LEFT OUT";
    let all = texts_at(crate::heroes_page::view(&app, CONTENT_720), CONTENT_720);
    let wide = texts_at(crate::heroes_page::view(&app, 10_000.0), 10_000.0);
    // Each text, whole, gets its one-line width or wraps to more lines,
    // and ends before the next one in its row starts: the stats line
    // before Sort, a hero name before its Games count.
    for (words, next) in [
        (line, "Sort"),
        ("Wrecking Ball", "10"),
        ("Junker Queen", "4"),
    ] {
        let i = all
            .iter()
            .position(|(t, _)| t == words)
            .unwrap_or_else(|| panic!("{words} not drawn whole: {all:?}"));
        let (at, after) = (all[i].1, all[i + 1].clone());
        let whole = wide.iter().find(|(t, _)| t == words).expect("drawn wide").1;
        assert!(
            at.width + 0.5 >= whole.width || at.height > whole.height + 1.0,
            "{words} cut at 720: {at:?}, needs {whole:?} on one line"
        );
        assert_eq!(after.0, next, "{all:?}");
        assert!(
            at.x + at.width <= after.1.x + 0.5,
            "{words} cut at 720: {at:?} runs into {after:?}"
        );
        assert!(at.x + at.width <= CONTENT_720 + 0.5, "{words}: {at:?}");
    }
}

#[test]
fn only_a_real_own_ban_is_drawn_as_the_tag() {
    let mut games = banned("ana", 2, 1, 1);
    games.extend(banned("mercy", 2, 0, 1));
    let (_dir, mut app) = app_with(games);
    when_banned(&mut app);
    let all = texts_at(crate::heroes_page::view(&app, 1200.0), 1200.0);
    let x = |words: &str| {
        all.iter()
            .find(|(t, _)| t == words)
            .unwrap_or_else(|| panic!("{words} not drawn: {all:?}"))
            .1
            .x
    };
    // Both cells start at the same x; the tag's padding moves its words.
    assert_eq!(x("Your ban 1") - x("Your ban 0"), 8.0, "{all:?}");
}

#[test]
fn sort_messages_reach_the_page_state() {
    let (_dir, mut app) = app_with(banned("ana", 3, 1, 2));
    let _ = app.update(Message::HeroesPage(HeroesMsg::Banned(
        crate::heroes_banned::Msg::Sort(SortKey::Recent),
    )));
    assert_eq!(app.heroes_page.banned.sort, SortKey::Recent);
    let _ = app.update(Message::HeroesPage(HeroesMsg::Banned(
        crate::heroes_banned::Msg::Header(SortKey::Recent),
    )));
    assert!(app.heroes_page.banned.ascending);
    when_banned(&mut app);
    for key in [
        SortKey::Games,
        SortKey::WinRate,
        SortKey::Hero,
        SortKey::Decided,
    ] {
        app.heroes_page.banned.sort = key;
        let _ = page(&app);
    }
}
