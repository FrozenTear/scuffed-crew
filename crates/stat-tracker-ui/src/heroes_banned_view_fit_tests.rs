//! When banned view tests that read what the headless renderer laid out
//! or drew: the Banned by colours (from a screenshot), the tooltip's
//! words (from the open overlay), and the 720 px minimum (each text's
//! unclamped width against its box, headers and chevrons included).

use super::*;
use iced::advanced::renderer::Headless as _;
use iced::advanced::text::Paragraph as _;
use iced::advanced::widget::tree::Tag;
use iced::advanced::{Shell, clipboard, mouse};
use iced::{Color, Size};

/// 720 px window, 24 px gutters.
const CONTENT_720: f32 = 672.0;

type Para = <iced::Renderer as iced::advanced::text::Renderer>::Paragraph;
type TextState = iced::advanced::text::paragraph::Plain<Para>;

/// One laid-out text: its words, the width and height it needs on its
/// own (not clamped to its box), the box it was laid out in, and one
/// line's height.
#[derive(Debug, Clone)]
struct Fit {
    words: String,
    need: Size,
    room: Size,
    line: f32,
}

impl Fit {
    fn overflows(&self) -> bool {
        self.need.width > self.room.width + 0.5
    }

    fn one_line(&self) -> bool {
        self.need.height <= self.line + 0.5
    }
}

/// Every text state under `tree`, in tree order.
fn fits(tree: &Tree) -> Vec<Fit> {
    let mut out = Vec::new();
    if tree.tag == Tag::of::<TextState>() {
        let p = tree.state.downcast_ref::<TextState>();
        let raw = p.raw();
        out.push(Fit {
            words: p.content().to_string(),
            need: p.min_bounds(),
            room: raw.bounds(),
            line: raw.line_height().to_absolute(raw.size()).0,
        });
    }
    for child in &tree.children {
        out.extend(fits(child));
    }
    out
}

/// `element` laid out at `width`: every text with its laid-out bounds
/// (clamped to the box), and every text's [`Fit`].
fn lay_out(mut element: Element<'_, Message>, width: f32) -> (Vec<(String, Rectangle)>, Vec<Fit>) {
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
        &Limits::new(Size::ZERO, Size::new(width, 4000.0)),
    );
    let mut at = At(Vec::new());
    element
        .as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut at);
    (at.0, fits(&tree))
}

/// The one [`Fit`] with these words.
fn fit_of<'a>(all: &'a [Fit], words: &str) -> &'a Fit {
    let hits: Vec<&Fit> = all.iter().filter(|f| f.words == words).collect();
    assert_eq!(hits.len(), 1, "{words}: {all:?}");
    hits[0]
}

/// Sigma: one game with an unread slot, one named only by the lobby ban,
/// so its cell is Your ban 0, Enemy 0, Side not read 1, Lobby 1.
fn sigma_unread_and_lobby() -> Vec<Game> {
    let mut unread = sample().remove(0);
    unread.session_id = "unread-sigma".into();
    unread.bans_picks =
        Some(r#"{"bans":[{"value":"sigma","team":null}],"bans_picks_recognizer":"cv-v1"}"#.into());
    vec![unread, lobby_only("sigma", 0)]
}

/// A row with every part shown, Your ban as the tag.
fn every_part() -> BanRow {
    let mut games = sigma_unread_and_lobby();
    games.extend(banned("sigma", 3, 1, 1));
    let r = only_row(&games);
    assert_eq!((r.own, r.enemy, r.side_not_read, r.lobby), (1, 2, 1, 1));
    r
}

fn rgb(c: Color) -> [f32; 3] {
    [c.r * 255.0, c.g * 255.0, c.b * 255.0]
}

#[test]
fn each_part_has_its_colour_token() {
    let parts = split_parts(&every_part());
    let colours: Vec<(&str, Color)> = parts
        .iter()
        .map(|p| (p.words.as_str(), part_color(p)))
        .collect();
    assert_eq!(
        colours,
        [
            ("Your ban 1", TEXT),
            ("Enemy 2", TEXT_2),
            ("Side not read 1", TEXT_3),
            ("Lobby 1", TEXT_3),
        ]
    );
    assert!(parts[0].tag, "Your ban 1 is the tag");
    let mut zero = every_part();
    zero.own = 0;
    assert_eq!(
        part_color(&split_parts(&zero)[0]),
        TEXT_2,
        "Your ban 0 is plain, like Enemy"
    );
}

/// How far, per 0-255 channel, a part's brightest pixel may sit from its
/// token. The nearest other token is 33 or more away on some channel
/// (TEXT to TEXT_2).
const TOKEN_TOLERANCE: f32 = 3.0;

#[test]
fn the_cell_is_drawn_in_each_parts_colour() {
    // Draw the cell on black: a glyph pixel is its colour times its
    // coverage, so no pixel is brighter than the colour, and the
    // brightest one (a well covered stem) is close to it.
    let row = every_part();
    let (w, h) = (600u32, 60u32);
    let mut cell = split(&row);
    let mut renderer = headless();
    let mut tree = Tree::new(&cell);
    let node = cell.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &Limits::new(Size::ZERO, Size::new(w as f32, h as f32)),
    );
    let mut bounds = Vec::new();
    {
        struct At<'a>(&'a mut Vec<(String, Rectangle)>);
        impl Operation<()> for At<'_> {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
                operate(self);
            }
            fn text(&mut self, _id: Option<&Id>, b: Rectangle, text: &str) {
                self.0.push((text.to_string(), b));
            }
        }
        cell.as_widget_mut().operate(
            &mut tree,
            Layout::new(&node),
            &renderer,
            &mut At(&mut bounds),
        );
    }
    let drawn: Vec<String> = bounds.iter().map(|(t, _)| t.clone()).collect();
    assert_eq!(drawn, split_words(&row), "in order");
    cell.as_widget().draw(
        &tree,
        &mut renderer,
        &iced::Theme::Dark,
        &iced::advanced::renderer::Style { text_color: TEXT },
        Layout::new(&node),
        mouse::Cursor::Unavailable,
        &Rectangle::with_size(Size::new(w as f32, h as f32)),
    );
    let px = renderer.screenshot(Size::new(w, h), 1.0, Color::BLACK);
    for (part, (words, b)) in split_parts(&row).iter().zip(&bounds) {
        let want = rgb(part_color(part));
        let mut brightest = [0.0f32; 3];
        for y in b.y.floor() as u32..(b.y + b.height).ceil() as u32 {
            for x in b.x.floor() as u32..(b.x + b.width).ceil() as u32 {
                let i = ((y * w + x) * 4) as usize;
                let p = [px[i] as f32, px[i + 1] as f32, px[i + 2] as f32];
                for c in 0..3 {
                    assert!(
                        p[c] <= want[c] + TOKEN_TOLERANCE,
                        "{words}: pixel {p:?} brighter than {want:?}"
                    );
                }
                if p.iter().sum::<f32>() > brightest.iter().sum::<f32>() {
                    brightest = p;
                }
            }
        }
        // A well covered stem is the token itself, within rounding, so a
        // part drawn in a neighbouring token (TEXT_2 for TEXT, or for
        // TEXT_3) is caught.
        for c in 0..3 {
            assert!(
                (brightest[c] - want[c]).abs() <= TOKEN_TOLERANCE,
                "{words}: brightest {brightest:?}, want {want:?}"
            );
        }
    }
}

#[test]
fn hovering_the_cell_opens_a_tooltip_reading_the_split_name() {
    let row = every_part();
    let mut cell = split(&row);
    let renderer = headless();
    let mut tree = Tree::new(&cell);
    let size = Size::new(600.0, 400.0);
    let node = cell
        .as_widget_mut()
        .layout(&mut tree, &renderer, &Limits::new(Size::ZERO, size));
    let viewport = Rectangle::with_size(size);
    let overlay_open = |cell: &mut Element<'_, Message>, tree: &mut Tree| {
        cell.as_widget_mut()
            .overlay(
                tree,
                Layout::new(&node),
                &renderer,
                &viewport,
                iced::Vector::ZERO,
            )
            .map(|mut o| {
                // Laying the overlay out lays out the tooltip's words.
                let _ = o.as_overlay_mut().layout(&renderer, size);
            })
            .is_some()
    };
    assert!(!overlay_open(&mut cell, &mut tree), "closed until hovered");
    let at = iced::Point::new(4.0, node.bounds().height / 2.0);
    let mut messages = Vec::new();
    cell.as_widget_mut().update(
        &mut tree,
        &iced::Event::Mouse(mouse::Event::CursorMoved { position: at }),
        Layout::new(&node),
        mouse::Cursor::Available(at),
        &renderer,
        &mut clipboard::Null,
        &mut Shell::new(&mut messages),
        &viewport,
    );
    assert!(
        overlay_open(&mut cell, &mut tree),
        "the tooltip opens on hover"
    );
    // A tooltip's tree is [the cell, the tip]: read the tip's words.
    assert_eq!(tree.children.len(), 2, "the cell is a tooltip");
    let tip: Vec<String> = fits(&tree.children[1])
        .into_iter()
        .map(|f| f.words)
        .collect();
    assert_eq!(
        tip,
        [split_name(&row)],
        "the tip is the spoken label, every part"
    );
    assert_eq!(tip[0], "Your ban 1, Enemy 2, Side not read 1, Lobby 1.");
}

/// A table at the 720 px minimum: two long hero names, Your ban, Enemy,
/// Side not read and Lobby, a percent and a Too few.
fn app_720() -> (tempfile::TempDir, TrackerApp) {
    let mut games = banned("wrecking-ball", 10, 3, 6);
    games.extend(banned("junker-queen", 4, 1, 2));
    games.extend(sigma_unread_and_lobby());
    let mut none = sample().remove(0);
    none.session_id = "no-ban".into();
    none.bans_picks = None;
    games.push(none);
    let (dir, mut app) = app_with(games);
    when_banned(&mut app);
    (dir, app)
}

#[test]
fn nothing_is_cut_at_the_720_px_minimum() {
    let (_dir, app) = app_720();
    let (all, fit) = lay_out(crate::heroes_page::view(&app, CONTENT_720), CONTENT_720);
    // Every text on the page, the header row's included, needs no more
    // width than its box gives it: a word or a one-line text wider than
    // its box would run past it.
    let cut: Vec<&Fit> = fit.iter().filter(|f| f.overflows()).collect();
    assert!(cut.is_empty(), "cut at 720: {cut:?}");
    // The headers, counts, records, percent and Banned by parts stay on
    // one line; the names and the stats line may wrap.
    for words in [
        "HERO",
        "GAMES",
        "DECIDED",
        "WIN RATE",
        "BANNED BY",
        "6-4",
        "60%",
        "2-2",
        "Your ban 3",
        "Enemy 7",
        "Side not read 1",
        "Lobby 1",
    ] {
        let f = fit_of(&fit, words);
        assert!(f.one_line(), "{words} wraps at 720: {f:?}");
    }
    for words in ["Wrecking Ball", "Junker Queen"] {
        assert!(!fit_of(&fit, words).overflows());
    }
    // Each text ends before the next one in its row starts.
    let line = "ALL TIME · 3 HEROES · 16 GAMES WITH A BAN READ · 1 GAME WITH NO BAN READ LEFT OUT";
    for (words, next) in [
        (line, "Sort"),
        ("Wrecking Ball", "10"),
        ("Junker Queen", "4"),
    ] {
        let i = all
            .iter()
            .position(|(t, _)| t == words)
            .unwrap_or_else(|| panic!("{words} not drawn whole: {all:?}"));
        let (at, after) = (all[i].1, &all[i + 1]);
        assert_eq!(after.0, next, "{all:?}");
        let need = fit_of(&fit, words).need.width;
        assert!(
            at.x + need <= after.1.x + 0.5,
            "{words} runs into {after:?}: at {at:?}, needs {need}"
        );
    }
}

/// The active header at the 720 px minimum: its label, gap and chevron
/// fit its column and end before the next header.
fn sorted_header_fits(key: SortKey, label: &str, next: &str, col: f32) {
    let (_dir, mut app) = app_720();
    app.heroes_page.banned.sort = key;
    let (all, fit) = lay_out(crate::heroes_page::view(&app, CONTENT_720), CONTENT_720);
    let i = all
        .iter()
        .position(|(t, _)| t == label)
        .unwrap_or_else(|| panic!("{label} not drawn: {all:?}"));
    let (head, chevron, after) = (&all[i], &all[i + 1], &all[i + 2]);
    assert_eq!(after.0, next, "label, chevron, next header: {all:?}");
    let label_w = fit_of(&fit, label).need.width;
    let chevron_fit = fit_of(&fit, &chevron.0);
    assert!(!fit_of(&fit, label).overflows(), "{label}: {fit:?}");
    assert!(!chevron_fit.overflows(), "{label} chevron: {chevron_fit:?}");
    let end = chevron.1.x + chevron_fit.need.width;
    assert!(
        label_w + 4.0 + chevron_fit.need.width <= col + 0.5,
        "{label} + chevron need {} of {col}",
        label_w + 4.0 + chevron_fit.need.width
    );
    assert!(
        end <= head.1.x + col + 0.5,
        "{label} chevron ends at {end}, past its column {}",
        head.1.x + col
    );
    assert!(
        end + 12.0 <= after.1.x + 0.5,
        "{label} chevron ends at {end}, crowding {next} at {}",
        after.1.x
    );
}

#[test]
fn the_sorted_games_header_fits_at_720() {
    assert_eq!(app_720().1.heroes_page.banned.sort, SortKey::Games);
    let c = cols(CONTENT_720);
    sorted_header_fits(SortKey::Games, "GAMES", "DECIDED", c.games);
}

#[test]
fn the_sorted_decided_header_fits_at_720() {
    let c = cols(CONTENT_720);
    sorted_header_fits(SortKey::Decided, "DECIDED", "WIN RATE", c.decided);
}

#[test]
fn the_compact_columns_fill_the_same_row_as_before() {
    // Games grew so its sorted header fits; Banned by gave the pixels.
    let c = cols(CONTENT_720);
    assert_eq!(c.games + c.decided + c.rate + c.split, 416.0, "{c:?}");
}
