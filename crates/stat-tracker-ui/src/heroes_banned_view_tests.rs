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
