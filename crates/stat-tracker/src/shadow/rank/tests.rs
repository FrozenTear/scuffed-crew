//! Tests for the rank-v1 reader on synthetic frames.
//!
//! No game art and no real screenshot is used. Every glyph here comes from a
//! 5x7 dot font defined below, drawn in flat colours at the positions the
//! reader looks at. Words (tags, the mode banner, the tier name) are stand-in
//! strings in the same dot font: the reader only cares that a word's shape
//! repeats, not what it says.
//!
//! The bench at the bottom (`bench_labelled_rank_screens`) is ignored: it
//! needs private labelled frames that never leave the bench box.

use std::sync::OnceLock;

use image::{Rgb, RgbImage};

use super::ink::{Plane, first_cluster, runs, tight};
use super::layout;
use super::templates::{Family, Learner, TemplateError, TemplateSet};
use super::*;

/// The game record wiring ([`super::game`]) on the same synthetic frames.
#[path = "tests_game.rs"]
mod game_wiring;

// ---- 5x7 dot font ----

fn rows_of(c: char) -> [&'static str; 7] {
    match c {
        '0' => [
            ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
        ],
        '1' => [
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
        '2' => [
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ],
        '3' => [
            "####.", "....#", "....#", ".###.", "....#", "....#", "####.",
        ],
        '4' => [
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ],
        '5' => [
            "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
        ],
        '6' => [
            "..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###.",
        ],
        '7' => [
            "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
        ],
        '8' => [
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ],
        '9' => [
            ".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##..",
        ],
        '%' => [
            "##..#", "##..#", "...#.", "..#..", ".#...", "#..##", "#..##",
        ],
        '+' => [
            ".....", "..#..", "..#..", "#####", "..#..", "..#..", ".....",
        ],
        '-' => [
            ".....", ".....", ".....", "#####", "#####", ".....", ".....",
        ],
        'H' => [
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        'E' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
        'T' => [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
        'L' => [
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ],
        'O' => [
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        '<' => [
            "....#", "...#.", "..#..", ".#...", "..#..", "...#.", "....#",
        ],
        '>' => [
            "#....", ".#...", "..#..", "...#.", "..#..", ".#...", "#....",
        ],
        _ => [
            "#####", "#####", "#####", "#####", "#####", "#####", "#####",
        ],
    }
}

/// Draw `text` with its top-left at `(x, y)`, each dot `s` pixels square,
/// one dot of space between characters and three for a space. Returns the
/// right edge.
fn draw(img: &mut RgbImage, text: &str, x: u32, y: u32, s: u32, col: Rgb<u8>) -> u32 {
    let mut cx = x;
    for c in text.chars() {
        if c == ' ' {
            cx += 3 * s;
            continue;
        }
        for (r, row) in rows_of(c).iter().enumerate() {
            for (k, b) in row.bytes().enumerate() {
                if b == b'#' {
                    for dy in 0..s {
                        for dx in 0..s {
                            img.put_pixel(cx + k as u32 * s + dx, y + r as u32 * s + dy, col);
                        }
                    }
                }
            }
        }
        cx += 6 * s;
    }
    cx - s
}

/// Width `draw` would use.
fn width(text: &str, s: u32) -> u32 {
    text.chars()
        .map(|c| if c == ' ' { 3 * s } else { 6 * s })
        .sum::<u32>()
        - s
}

/// Three chevrons (`c` is `<` or `>`) at two-pixel dots, each one dot
/// over the last so they cut as one glyph [`CHEVRONS_W`] wide.
fn chevrons(img: &mut RgbImage, c: char, x: u32, y: u32) {
    for k in 0..3 {
        draw(img, &c.to_string(), x + k * 8, y, 2, WHITE);
    }
}

const CHEVRONS_W: u32 = 26;

fn fill(img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, col: Rgb<u8>) {
    for y in y0..y1 {
        for x in x0..x1 {
            img.put_pixel(x, y, col);
        }
    }
}

const BG: Rgb<u8> = Rgb([40, 44, 60]);
const WHITE: Rgb<u8> = Rgb([250, 250, 250]);
const CYAN: Rgb<u8> = Rgb([28, 239, 239]);
const ORANGE: Rgb<u8> = Rgb([240, 120, 30]);
const YELLOW: Rgb<u8> = Rgb([252, 252, 4]);
const GREEN: Rgb<u8> = Rgb([0, 200, 0]);
const RED: Rgb<u8> = Rgb([208, 59, 97]);
const SR_GREEN: Rgb<u8> = Rgb([60, 230, 60]);

fn blank() -> RgbImage {
    RgbImage::from_pixel(FRAME_W, FRAME_H, BG)
}

/// What a synthetic rank screen shows.
#[derive(Clone)]
struct Scene {
    pct: i32,
    higher: u8,
    change: Option<i32>,
    sr: Option<u32>,
    sr_change: Option<i32>,
    tags: Vec<&'static str>,
    banner: Option<&'static str>,
    tier: Option<&'static str>,
    division: Option<u8>,
    /// Hero SR text grey level (250 = bright, about 80 = under a chat overlay).
    hero_level: u8,
    /// Role line above the emblem; `None` paints none (open queue).
    role: Option<Role>,
    /// Chevrons on the pill: `>>>` after a green pill's text, `<<<` before a
    /// red pill's minus.
    chevrons: bool,
    /// A rank icon left of the hero SR.
    sr_icon: Icon,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            pct: 31,
            higher: 96,
            change: Some(27),
            sr: Some(3597),
            sr_change: Some(12),
            tags: vec!["EH", "TOT"],
            banner: Some("HELLO 6V6"),
            tier: Some("HELLO"),
            division: Some(5),
            hero_level: 250,
            role: None,
            chevrons: false,
            sr_icon: Icon::None,
        }
    }
}

/// The rank icon left of the hero SR.
#[derive(Clone, Copy, PartialEq)]
enum Icon {
    None,
    /// Taller than the digits, its bottom below theirs.
    Tall,
    /// A disc of digit height (a block with a dark bar), as some tiers show.
    Level,
}

/// Where the pill text starts.
const PILL_X: u32 = 730;
/// The pill text's top row (two-pixel dots, 14 rows).
const PILL_Y: u32 = 1135;

/// Left edge of the pill's digits for `sc` (a red pill keeps room for its
/// chevrons and minus bar).
fn pill_digits_x(sc: &Scene) -> u32 {
    match (sc.change, sc.chevrons) {
        (Some(c), true) if c < 0 => PILL_X + CHEVRONS_W + 12 + 10,
        (Some(c), false) if c < 0 => PILL_X + 10,
        _ => PILL_X,
    }
}

/// Paint `sc` at the reader's positions (see `layout`).
fn paint(sc: &Scene) -> RgbImage {
    let mut img = blank();
    // the "RANK PROGRESS:" label, a stand-in word
    draw(&mut img, "TELLOH", 842, 1068, 3, WHITE);
    // progress percent: cyan, orange when negative
    let (col, text) = if sc.pct < 0 {
        (ORANGE, format!("-{}%", -sc.pct))
    } else {
        (CYAN, format!("{}%", sc.pct))
    };
    draw(&mut img, &text, 978, 1068, 3, col);
    draw(&mut img, &format!("{}%", sc.higher), 1730, 1082, 2, YELLOW);
    // the bar, white fill then a pill
    fill(&mut img, 640, 1122, 1920, 1162, Rgb([20, 20, 26]));
    if let Some(ch) = sc.change {
        let (col, text) = if ch < 0 {
            (RED, format!("{}%", -ch))
        } else {
            (GREEN, format!("+{ch}%"))
        };
        let x = pill_digits_x(sc);
        let trail = if ch > 0 && sc.chevrons {
            12 + CHEVRONS_W
        } else {
            0
        };
        fill(
            &mut img,
            700,
            1123,
            x + width(&text, 2) + trail + 30,
            1161,
            col,
        );
        let right = draw(&mut img, &text, x, PILL_Y, 2, WHITE);
        if ch < 0 {
            // a two-row minus bar at mid height touching the first digit,
            // as the game draws it ("<<< -29%")
            fill(&mut img, x - 10, PILL_Y + 6, x + 2, PILL_Y + 8, WHITE);
            if sc.chevrons {
                chevrons(&mut img, '<', PILL_X, PILL_Y);
            }
        } else if sc.chevrons {
            chevrons(&mut img, '>', right + 12, PILL_Y);
        }
    }
    // tags: an icon disc stand-in then a word
    let mut x = 640;
    for (k, t) in sc.tags.iter().enumerate() {
        let col = if k % 2 == 0 { RED } else { GREEN };
        fill(&mut img, x, 1186, x + 34, 1220, col);
        let right = draw(&mut img, t, x + 44, 1194, 2, WHITE);
        x = right + 40;
    }
    if let Some(b) = sc.banner {
        fill(&mut img, 1952, 120, 2540, 200, RED);
        draw(&mut img, b, 2068, 150, 3, WHITE);
    }
    if let Some(r) = sc.role {
        let right = draw(&mut img, &role_icon(r).to_string(), 1190, 290, 4, WHITE);
        draw(&mut img, "TEL", right + 20, 290, 4, WHITE);
    }
    if let Some(d) = sc.division {
        // the neighbour emblems' digits, centred near x 560 and x 2000
        let (l, r) = neighbours(d);
        draw(&mut img, &l.to_string(), 556, 1148, 2, WHITE);
        draw(&mut img, &r.to_string(), 1996, 1148, 2, WHITE);
    }
    if let (Some(t), Some(d)) = (sc.tier, sc.division) {
        let right = draw(&mut img, t, 1160, 900, 6, WHITE);
        draw(&mut img, &d.to_string(), right + 30, 900, 6, WHITE);
    }
    if let Some(sr) = sc.sr {
        let g = Rgb([sc.hero_level; 3]);
        // name line right-aligned with the SR digits at x 2333
        let name = "HELLO";
        draw(&mut img, name, 2333 - width(name, 2), 700, 2, g);
        let s = sr.to_string();
        let left = 2333 - width(&s, 3);
        draw(&mut img, &s, left, 724, 3, g);
        match sc.sr_icon {
            Icon::None => {}
            Icon::Tall => fill(&mut img, left - 30, 722, left - 12, 751, g),
            Icon::Level => {
                fill(&mut img, left - 30, 724, left - 12, 745, g);
                fill(&mut img, left - 26, 733, left - 16, 736, BG);
            }
        }
        if let Some(c) = sc.sr_change {
            let col = if c < 0 { Rgb([220, 30, 30]) } else { SR_GREEN };
            let t = c.abs().to_string();
            draw(&mut img, &t, 2333 - width(&t, 2), 752, 2, col);
        }
    }
    img
}

/// Stand-in role icons in the dot font.
fn role_icon(r: Role) -> char {
    match r {
        Role::Support => '+',
        Role::Damage => 'E',
        Role::Tank => 'O',
        Role::Open => 'L',
    }
}

/// Labels for `sc`, in the bench's JSON shape.
fn labels(sc: &Scene) -> Labels {
    Labels {
        role: Some(sc.role.unwrap_or(Role::Open)),
        role_rank: sc.tier.map(str::to_string),
        division: sc.division,
        progress_pct: Some(sc.pct),
        progress_change: sc.change,
        change_tags: Some(sc.tags.iter().map(|t| t.to_string()).collect()),
        higher_than_pct: Some(sc.higher),
        hero_sr: sc.sr,
        hero_sr_change: sc.sr_change,
        next_mode_hint: sc.banner.map(str::to_string),
    }
}

/// Scenes that between them show every digit in every numeric field.
fn training() -> Vec<Scene> {
    let d = Scene::default();
    vec![
        Scene {
            pct: 10,
            higher: 23,
            change: Some(45),
            sr: Some(6789),
            sr_change: Some(10),
            role: Some(Role::Support),
            chevrons: true,
            ..d.clone()
        },
        Scene {
            pct: -23,
            higher: 45,
            change: Some(-67),
            sr: Some(1234),
            sr_change: Some(-23),
            tags: vec!["TOT", "EH"],
            division: Some(4),
            role: Some(Role::Damage),
            chevrons: true,
            ..d.clone()
        },
        Scene {
            pct: 45,
            higher: 67,
            change: Some(89),
            sr: Some(5012),
            sr_change: Some(45),
            role: Some(Role::Tank),
            sr_icon: Icon::Tall,
            ..d.clone()
        },
        Scene {
            pct: 67,
            higher: 89,
            change: Some(-10),
            sr: Some(3456),
            sr_change: Some(-67),
            division: Some(1),
            banner: Some("TOT 5V5"),
            ..d.clone()
        },
        Scene {
            pct: 89,
            higher: 10,
            change: Some(23),
            sr: Some(7890),
            sr_change: Some(89),
            sr_icon: Icon::Level,
            ..d
        },
    ]
}

/// The pack learned from [`training`], built once for all tests.
fn trained() -> &'static TemplateSet {
    static SET: OnceLock<TemplateSet> = OnceLock::new();
    SET.get_or_init(|| {
        let mut l = Learner::new();
        for sc in training() {
            l.add_frame(&paint(&sc), &labels(&sc));
        }
        l.build()
    })
}

fn reader() -> &'static Reader {
    static READER: OnceLock<Reader> = OnceLock::new();
    READER.get_or_init(|| Reader::new(trained().clone()))
}

// ---- the switch ----

#[test]
fn the_reader_is_off_unless_switched_on() {
    assert_eq!(RankSetting::default(), RankSetting::Off);
    assert!(!RankSetting::default().is_on());
    for on in ["rank-v1", "on", "1", "true", "YES", " On "] {
        assert_eq!(RankSetting::parse(on), RankSetting::RankV1, "{on}");
    }
    for off in ["off", "0", "false", "no", "", "maybe", "cv-v1"] {
        assert_eq!(RankSetting::parse(off), RankSetting::Off, "{off}");
    }
    assert_eq!(ENV, "STAT_TRACKER_RANK_RECOGNIZER");
    assert_eq!(RECOGNIZER_ID, "rank-v1");
}

// ---- pixel helpers ----

#[test]
fn runs_split_on_empty_columns_and_merge_small_gaps() {
    let prof = [0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0];
    assert_eq!(runs(&prof, 0.5, 1), vec![(1, 3), (5, 6), (7, 9)]);
    assert_eq!(runs(&prof, 0.5, 2), vec![(1, 3), (5, 9)]);
    assert_eq!(first_cluster(&runs(&prof, 0.5, 1), 2), Some((1, 3)));
    assert_eq!(first_cluster(&runs(&prof, 0.5, 1), 3), Some((1, 9)));
    assert_eq!(first_cluster(&[], 3), None);
}

#[test]
fn tight_crops_to_strong_ink_only() {
    let mut p = Plane::new(6, 5);
    p.set(2, 1, 0.9);
    p.set(4, 3, 0.8);
    p.set(0, 0, 0.3); // faint, ignored
    let (b, sub) = tight(&p, 100, 200).expect("ink");
    assert_eq!(b, [102, 201, 105, 204]);
    assert_eq!((sub.w, sub.h), (3, 3));
    assert!(tight(&Plane::new(4, 4), 0, 0).is_none());
}

// ---- extraction ----

#[test]
fn percent_digits_split_into_one_glyph_each() {
    let img = paint(&Scene::default());
    let (gl, negative) = layout::pct_glyphs(&img);
    assert_eq!(gl.len(), 3, "3, 1, %");
    assert!(!negative);
    assert!(gl.windows(2).all(|w| w[0].bbox[2] <= w[1].bbox[0]));
    let img = paint(&Scene {
        pct: -24,
        ..Scene::default()
    });
    let (gl, negative) = layout::pct_glyphs(&img);
    assert_eq!(gl.len(), 4, "-, 2, 4, %");
    assert!(negative);
}

#[test]
fn the_pill_sign_comes_from_its_colour() {
    let img = paint(&Scene {
        change: Some(-31),
        ..Scene::default()
    });
    let (gl, sign) = layout::pill_glyphs(&img);
    assert_eq!(sign, Some(-1));
    assert_eq!(gl.len(), 3);
    let img = paint(&Scene::default());
    let (gl, sign) = layout::pill_glyphs(&img);
    assert_eq!(sign, Some(1));
    assert_eq!(gl.len(), 4, "+, 2, 7, %");
    let img = paint(&Scene {
        change: None,
        ..Scene::default()
    });
    assert!(layout::pill_glyphs(&img).0.is_empty());
}

#[test]
fn the_hero_entry_is_found_bright_or_dimmed() {
    for level in [250u8, 90] {
        let img = paint(&Scene {
            hero_level: level,
            ..Scene::default()
        });
        let e = layout::first_entry(&img).unwrap_or_else(|| panic!("entry at level {level}"));
        assert_eq!(layout::sr_glyphs(&img, &e).len(), 4, "level {level}");
        let (cg, sign) = layout::change_glyphs(&img, &e);
        assert_eq!(cg.len(), 2);
        assert_eq!(sign, Some(1));
    }
}

#[test]
fn tag_words_follow_their_icons() {
    let img = paint(&Scene::default());
    let words = layout::tag_words(&img);
    assert_eq!(words.len(), 2);
    assert!(words[0].bbox[2] < words[1].bbox[0]);
}

#[test]
fn the_tier_word_and_division_split_at_the_space() {
    let img = paint(&Scene::default());
    let (word, digit) = layout::rank_text(&img).expect("rank text");
    assert!(word.bbox[2] < digit.bbox[0]);
    assert!(digit.bbox[2] - digit.bbox[0] < word.bbox[2] - word.bbox[0]);
}

// ---- templates ----

#[test]
fn learning_covers_every_field_family() {
    let set = trained();
    for (f, want) in [
        (Family::Pct, "0123456789%-"),
        (Family::Higher, "0123456789%"),
        (Family::Sr, "0123456789"),
        (Family::Change, "0123456789"),
    ] {
        for c in want.chars() {
            assert!(set.has_class(f, &c.to_string()), "{f:?} {c}");
        }
    }
    assert!(set.has_class(Family::Pill, "+"));
    for r in Role::ICONS {
        assert!(set.has_class(Family::Role, r.as_str()), "{r:?}");
    }
    assert!(!set.has_class(Family::Role, Role::Open.as_str()));
    assert!(set.has_class(Family::Tag, "EH"));
    assert!(set.has_class(Family::Banner, "HELLO 6V6"));
    assert!(set.has_class(Family::Tier, "HELLO"));
    assert!(set.has_class(Family::Gate, GATE_CLASS));
}

#[test]
fn a_template_pack_round_trips_through_its_file() {
    let set = trained();
    let dir = tempfile::tempdir().unwrap();
    set.save_dir(dir.path()).unwrap();
    let back = TemplateSet::load_dir(dir.path()).unwrap();
    assert_eq!(&back, set);
    assert!(matches!(
        TemplateSet::load_dir(&dir.path().join("missing")),
        Err(TemplateError::Missing(_))
    ));
    assert!(Reader::load(&dir.path().join("nowhere")).is_err());
}

#[test]
fn a_pack_of_another_shape_is_refused_on_load() {
    let bad = |edit: &dyn Fn(&mut TemplateSet)| {
        let mut set = trained().clone();
        edit(&mut set);
        let dir = tempfile::tempdir().unwrap();
        set.save_dir(dir.path()).unwrap();
        match TemplateSet::load_dir(dir.path()) {
            Err(TemplateError::Bad(_, e)) => e,
            other => panic!("loaded: {:?}", other.map(|_| ())),
        }
    };
    let e = bad(&|s| s.recognizer = "cv-v1".to_string());
    assert!(e.contains("recognizer"), "{e}");
    let e = bad(&|s| {
        let v = &mut s.families.get_mut(&Family::Sr).unwrap()[0].vector;
        v.truncate(100);
    });
    assert!(e.contains("want 576"), "{e}");
    let e = bad(&|s| s.families.get_mut(&Family::Pct).unwrap()[0].aspect = -1.0);
    assert!(e.contains("finite"), "{e}");
    let e = bad(&|s| {
        s.families.remove(&Family::Gate);
    });
    assert!(e.contains("gate"), "{e}");
    // a file that is there but cannot be read is not "missing"
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(super::templates::PACK_FILE)).unwrap();
    assert!(matches!(
        TemplateSet::load_dir(dir.path()),
        Err(TemplateError::Bad(..))
    ));
}

// ---- reading ----

#[test]
fn reads_every_field_of_an_unseen_screen() {
    let sc = Scene {
        pct: 58,
        higher: 47,
        change: Some(-36),
        sr: Some(2481),
        sr_change: Some(-90),
        tags: vec!["TOT", "EH"],
        division: Some(4),
        ..Scene::default()
    };
    let r = reader().read(&paint(&sc)).expect("a rank screen");
    assert_eq!(r.rank_recognizer, RECOGNIZER_ID);
    assert_eq!(r.progress_pct.value, Some(58));
    assert_eq!(r.higher_than_pct.value, Some(47));
    assert_eq!(r.progress_change.value, Some(-36));
    assert_eq!(r.hero_sr.value, Some(2481));
    assert_eq!(r.hero_sr_change.value, Some(-90));
    assert_eq!(
        r.change_tags.value,
        Some(vec!["TOT".to_string(), "EH".to_string()])
    );
    assert_eq!(r.next_mode_hint.value.as_deref(), Some("HELLO 6V6"));
    assert_eq!(r.role_rank.value.as_deref(), Some("HELLO"));
    assert_eq!(r.division.value, Some(4));
    for (name, c) in r.confidences() {
        assert!((0.0..=1.0).contains(&c) && c > 0.0, "{name} {c}");
    }
    assert!(r.progress_pct.bbox.is_some());
}

#[test]
fn a_negative_progress_reads_negative() {
    let sc = Scene {
        pct: -24,
        change: None,
        ..Scene::default()
    };
    let r = reader().read(&paint(&sc)).expect("a rank screen");
    assert_eq!(r.progress_pct.value, Some(-24));
    assert_eq!(r.progress_change.value, None);
    assert_eq!(r.progress_change.confidence, 0.0);
}

#[test]
fn dimmed_hero_text_still_reads() {
    let sc = Scene {
        hero_level: 90,
        ..Scene::default()
    };
    let r = reader().read(&paint(&sc)).expect("a rank screen");
    assert_eq!(r.hero_sr.value, Some(3597));
}

#[test]
fn what_is_not_shown_is_none() {
    let sc = Scene {
        sr: None,
        banner: None,
        tags: vec![],
        ..Scene::default()
    };
    let r = reader().read(&paint(&sc)).expect("a rank screen");
    assert_eq!(r.hero_sr.value, None);
    assert_eq!(r.hero_sr_change.value, None);
    assert_eq!(r.next_mode_hint.value, None);
    assert_eq!(r.change_tags.value, None);
}

#[test]
fn an_unknown_glyph_makes_the_field_none_not_a_guess() {
    // A solid block was never learned as a digit: the percent is unreadable.
    let mut img = paint(&Scene::default());
    fill(&mut img, 960, 1058, 1140, 1100, BG);
    draw(&mut img, "3#%", 978, 1068, 3, CYAN);
    let r = reader().read(&img).expect("a rank screen");
    assert_eq!(r.progress_pct.value, None);
    assert_eq!(r.progress_pct.confidence, 0.0);
    assert_eq!(r.higher_than_pct.value, Some(96));
}

#[test]
fn another_screen_is_not_a_rank_screen() {
    let r = reader();
    for img in [blank(), RgbImage::from_pixel(1920, 1080, BG)] {
        assert_eq!(r.read(&img).err(), Some(ReadError::NotRankScreen));
        assert_eq!(r.is_rank_screen(&img), Err(ReadError::NotRankScreen));
    }
    let mut img = paint(&Scene::default());
    fill(&mut img, 830, 1060, 980, 1100, BG);
    assert_eq!(
        r.read(&img).err(),
        Some(ReadError::NotRankScreen),
        "label gone"
    );
    // another white word where the label goes
    draw(&mut img, "HEH 6", 842, 1068, 3, WHITE);
    assert_eq!(r.is_rank_screen(&img), Err(ReadError::NotRankScreen));
    assert_eq!(r.read(&img).err(), Some(ReadError::NotRankScreen));
}

#[test]
fn unsupported_sizes_are_their_own_error() {
    use image::imageops::{self, FilterType};
    let r = reader();
    let painted = paint(&Scene::default());
    // a crop of the screen, other aspect ratios, untested heights
    let crop = imageops::crop_imm(&painted, 560, 820, 1440, 480).to_image();
    for img in [
        crop,
        RgbImage::from_pixel(800, 600, BG),
        RgbImage::from_pixel(1920, 1200, BG),
        RgbImage::from_pixel(2560, 1080, BG),
        imageops::resize(&painted, 1280, 720, FilterType::Triangle),
        imageops::resize(&painted, 3840, 2160, FilterType::Nearest),
    ] {
        let (width, height) = img.dimensions();
        let want = ReadError::Unsupported { width, height };
        assert_eq!(r.read(&img).err(), Some(want), "{width}x{height}");
        assert_eq!(r.is_rank_screen(&img), Err(want), "{width}x{height}");
        assert!(want.to_string().contains("16:9"));
    }
}

#[test]
fn the_gate_alone_gives_its_score() {
    let r = reader();
    let img = paint(&Scene::default());
    let score = r.is_rank_screen(&img).expect("rank screen");
    assert!((MIN_WORD_SCORE..=1.0).contains(&score), "{score}");
    assert!(r.read(&img).is_ok());
}

/// Every field of `r`: a `None` value has confidence 0 and no box, and a
/// value has a confidence in (0, 1].
fn field_invariants(r: &RankRead, at: &str) {
    fn check<T>(name: &str, f: &Field<T>, at: &str) {
        let c = f.confidence;
        match &f.value {
            None => assert!(c == 0.0 && f.bbox.is_none(), "{at} {name}: {c}"),
            Some(_) => assert!(c > 0.0 && c <= 1.0, "{at} {name}: {c}"),
        }
    }
    check("role", &r.role, at);
    check("role_rank", &r.role_rank, at);
    check("division", &r.division, at);
    check("progress_pct", &r.progress_pct, at);
    check("progress_change", &r.progress_change, at);
    check("change_tags", &r.change_tags, at);
    check("higher_than_pct", &r.higher_than_pct, at);
    check("hero_sr", &r.hero_sr, at);
    check("hero_sr_change", &r.hero_sr_change, at);
    check("next_mode_hint", &r.next_mode_hint, at);
}

/// Every size near the supported ones and the common unsupported ones, blank
/// and painted: nothing panics, and only the supported sizes get past the
/// size check.
#[test]
fn odd_sizes_never_panic() {
    use image::imageops::{self, FilterType};
    let r = reader();
    let painted = paint(&Scene {
        change: Some(-31),
        chevrons: true,
        sr_icon: Icon::Tall,
        role: Some(Role::Damage),
        ..Scene::default()
    });
    let mut sizes = vec![
        (0, 0),
        (1, 1),
        (16, 9),
        (1280, 720),
        (1366, 768),
        (1600, 900),
        (1920, 1080),
        (2560, 1440),
        (3840, 2160),
    ];
    // the edges of the height and aspect tolerances around 1080p and 1440p
    for h in [1058, 1059, 1080, 1101, 1102, 1411, 1412, 1440, 1468, 1469] {
        let w = h as f32 * 16.0 / 9.0;
        for a in [-0.021f32, -0.019, 0.0, 0.019, 0.021] {
            sizes.push(((w + a * h as f32).round() as u32, h));
        }
    }
    for (w, h) in sizes {
        let supported = layout::Scale::from_dims(w, h).is_some();
        let blank = RgbImage::from_pixel(w, h, BG);
        let mut imgs = vec![blank];
        // a painted copy only where the reader gets past the size check
        if supported {
            imgs.push(imageops::resize(&painted, w, h, FilterType::Nearest));
        }
        for img in imgs {
            let gate = r.is_rank_screen(&img);
            let read = r.read(&img);
            assert_eq!(
                matches!(gate, Err(ReadError::Unsupported { .. })),
                !supported,
                "{w}x{h}"
            );
            assert_eq!(gate.is_ok(), read.is_ok(), "{w}x{h}");
            if let Ok(read) = &read {
                field_invariants(read, &format!("{w}x{h}"));
            }
            Learner::new().add_frame(&img, &labels(&Scene::default()));
        }
    }
    // the sizes the reader is built for
    for (w, h) in [(1920, 1080), (2560, 1440), (1884, 1060), (2524, 1420)] {
        assert!(layout::Scale::from_dims(w, h).is_some(), "{w}x{h}");
    }
    for (w, h) in [(1280, 720), (3840, 2160), (2560, 1600), (3440, 1440)] {
        assert!(layout::Scale::from_dims(w, h).is_none(), "{w}x{h}");
    }
}

#[test]
fn a_1080p_downscale_of_the_synthetic_screen_still_reads() {
    use image::imageops::{self, FilterType};
    let sc = Scene {
        pct: 58,
        higher: 47,
        change: Some(27),
        sr: Some(3597),
        sr_change: Some(12),
        ..Scene::default()
    };
    // Learn from both native 1440p paints and Nearest-downscaled 1080p copies
    // so tiny change digits still have a matching template at that scale.
    let mut l = Learner::new();
    for tr in training() {
        let img = paint(&tr);
        l.add_frame(&img, &labels(&tr));
        let small = imageops::resize(&img, 1920, 1080, FilterType::Nearest);
        l.add_frame(&small, &labels(&tr));
    }
    let reader = Reader::new(l.build());
    let img1080 = imageops::resize(&paint(&sc), 1920, 1080, FilterType::Nearest);
    let r = reader.read(&img1080).expect("1080p rank screen");
    assert_eq!(r.progress_pct.value, Some(58));
    assert_eq!(r.higher_than_pct.value, Some(47));
    assert_eq!(r.progress_change.value, Some(27));
    assert_eq!(r.hero_sr.value, Some(3597));
    assert_eq!(r.hero_sr_change.value, Some(12));
    assert_eq!(r.role_rank.value.as_deref(), Some("HELLO"));
    assert_eq!(r.division.value, Some(5));
    assert_eq!(
        r.change_tags.value,
        Some(vec!["EH".to_string(), "TOT".to_string()])
    );
    assert_eq!(r.next_mode_hint.value.as_deref(), Some("HELLO 6V6"));
}

/// A 1080p frame made with a soft filter, as the game's own scaling is:
/// glyph edges blur and neighbours can touch. Whatever does not read cleanly
/// must be `None`, never another value.
#[test]
fn a_soft_1080p_downscale_reads_right_or_none() {
    use image::imageops::{self, FilterType};
    let soft = |img: &RgbImage| imageops::resize(img, 1920, 1080, FilterType::Triangle);
    let mut l = Learner::new();
    for tr in training() {
        l.add_frame(&soft(&paint(&tr)), &labels(&tr));
    }
    let reader = Reader::new(l.build());
    let sc = Scene {
        pct: 58,
        higher: 47,
        change: Some(-36),
        chevrons: true,
        sr: Some(2481),
        sr_change: Some(-90),
        sr_icon: Icon::Tall,
        role: Some(Role::Support),
        ..Scene::default()
    };
    let r = reader.read(&soft(&paint(&sc))).expect("1080p rank screen");
    let want = labels(&sc);
    let mut read = 0;
    for ((name, got), (_, want)) in r.values().into_iter().zip(want.values()) {
        if let Some(got) = got {
            assert_eq!(Some(got), want, "{name}");
            read += 1;
        }
    }
    assert!(read >= 6, "only {read} fields read: {r:?}");
}

// ---- small glyphs ----

#[test]
fn two_touching_digits_are_split_and_read() {
    let r = reader();
    let sc = Scene {
        higher: 94,
        ..Scene::default()
    };
    let mut img = paint(&sc);
    // join the 9 and the 4 (drawn at x 1730, two-pixel dots) across their gap
    fill(&mut img, 1738, 1088, 1744, 1092, YELLOW);
    assert_eq!(layout::higher_glyphs(&img).len(), 2, "9 and 4 cut as one");
    let read = r.read(&img).expect("rank screen");
    assert_eq!(read.higher_than_pct.value, Some(94));
}

#[test]
fn shifted_vectors_start_with_the_plain_vector() {
    let mut p = Plane::new(6, 10);
    for y in 0..10 {
        p.set(2, y, 1.0);
        p.set(3, y, 1.0);
    }
    let vs = super::ink::shifted_vectors(&p, 24, 24);
    assert_eq!(vs.len(), 9);
    assert_eq!(vs[0], super::ink::vector(&p, 24, 24));
    assert!(vs.iter().skip(1).all(|v| v != &vs[0]));
}

#[test]
fn a_hero_name_dimmed_halfway_still_bounds_the_sr() {
    let r = reader();
    let mut img = paint(&Scene::default());
    // a longer name: "HEL HELLO", right aligned over the SR as before
    draw(&mut img, "HEL", 2234, 700, 2, WHITE);
    // a chat overlay edge at x 2250 crosses the name and dims everything
    // right of it, the SR and its change included
    for y in 690..780 {
        for x in 2250..2345 {
            let p = img.get_pixel_mut(x, y);
            for c in p.0.iter_mut() {
                *c = (*c as f32 * 0.3) as u8;
            }
        }
    }
    let read = r.read(&img).expect("rank screen");
    assert_eq!(read.hero_sr.value, Some(3597));
    assert_eq!(read.hero_sr_change.value, Some(12));
}

// ---- pill signs, chevrons and merged cuts ----

#[test]
fn chevrons_and_the_minus_bar_read_on_both_pills() {
    assert!(trained().has_class(Family::Pill, CHEV_LEFT));
    assert!(trained().has_class(Family::Pill, CHEV_RIGHT));
    for change in [27, -31] {
        let sc = Scene {
            change: Some(change),
            chevrons: true,
            ..Scene::default()
        };
        let read = reader().read(&paint(&sc)).expect("rank screen");
        assert_eq!(read.progress_change.value, Some(change), "{change}");
    }
    // the minus bar touching the first digit is cut off, the digits read
    let img = paint(&Scene {
        change: Some(-31),
        ..Scene::default()
    });
    let (gl, sign) = layout::pill_glyphs(&img);
    assert_eq!((gl.len(), sign), (3, Some(-1)), "3, 1, %");
    assert_eq!(gl[0].bbox[0], pill_digits_x(&Scene::default()) + 10);
}

/// The review's case: at 1080p the "+" and the first digit can touch. The
/// merged cut must not be read as the "+" and dropped, leaving "7%".
#[test]
fn a_digit_merged_into_the_plus_is_not_dropped() {
    let sc = Scene {
        change: Some(27),
        ..Scene::default()
    };
    let mut img = paint(&sc);
    // join the + (its arm ends at x 740) to the 2 (from x 742)
    fill(
        &mut img,
        PILL_X + 10,
        PILL_Y + 6,
        PILL_X + 12,
        PILL_Y + 8,
        WHITE,
    );
    assert_eq!(layout::pill_glyphs(&img).0.len(), 3, "+2, 7, %");
    let read = reader().read(&img).expect("rank screen");
    // the old reader dropped the cut as "+" and read 7 at full confidence
    assert_ne!(read.progress_change.value, Some(7));
    assert!(matches!(read.progress_change.value, None | Some(27)));
}

/// "<<<" merged with the minus and the first digit: the cut must not be read
/// as the chevrons and dropped, leaving "1%".
#[test]
fn a_digit_merged_into_the_chevrons_is_not_dropped() {
    let sc = Scene {
        change: Some(-31),
        chevrons: true,
        ..Scene::default()
    };
    let mut img = paint(&sc);
    let bar = pill_digits_x(&sc) - 10;
    fill(
        &mut img,
        PILL_X + CHEVRONS_W,
        PILL_Y + 6,
        bar,
        PILL_Y + 8,
        WHITE,
    );
    assert_eq!(layout::pill_glyphs(&img).0.len(), 3, "<<<-3, 1, %");
    let read = reader().read(&img).expect("rank screen");
    assert_ne!(read.progress_change.value, Some(-1));
    assert!(matches!(read.progress_change.value, None | Some(-31)));
}

// ---- hero SR icon ----

#[test]
fn the_sr_icon_left_of_a_short_sr_is_not_read_as_a_digit() {
    assert!(trained().has_class(Family::Sr, SR_ICON));
    for icon in [Icon::Tall, Icon::Level] {
        for sr in [987, 3597] {
            let sc = Scene {
                sr: Some(sr),
                sr_icon: icon,
                ..Scene::default()
            };
            let img = paint(&sc);
            let e = layout::first_entry(&img).expect("entry");
            // a tall icon is dropped by shape, a level one kept for the reader
            let cuts = layout::sr_glyphs(&img, &e).len();
            let extra = usize::from(icon == Icon::Level);
            assert_eq!(cuts, sr.to_string().len() + extra, "{sr}");
            let read = reader().read(&img).expect("rank screen");
            assert_eq!(read.hero_sr.value, Some(sr), "{sr}");
            assert_eq!(read.hero_sr_change.value, Some(12), "{sr}");
        }
    }
}

/// An icon of digit height in a shape the pack never saw is neither dropped
/// nor read as a digit: the SR is `None`.
#[test]
fn an_unknown_sr_icon_makes_the_sr_none() {
    let sc = Scene {
        sr: Some(987),
        ..Scene::default()
    };
    let mut img = paint(&sc);
    let left = 2333 - width("987", 3);
    // a hollow box
    fill(&mut img, left - 30, 724, left - 12, 745, WHITE);
    fill(&mut img, left - 27, 727, left - 15, 742, BG);
    let read = reader().read(&img).expect("rank screen");
    assert_eq!(read.hero_sr.value, None);
}

// ---- division ----

#[test]
fn neighbour_emblems_show_the_divisions_below_and_above() {
    assert_eq!(neighbours(5), (1, 4));
    assert_eq!(neighbours(4), (5, 3));
    assert_eq!(neighbours(3), (4, 2));
    assert_eq!(neighbours(2), (3, 1));
    assert_eq!(neighbours(1), (2, 5));
}

fn vote(division: u8, confidence: f32) -> Vote {
    Vote {
        division,
        confidence,
        bbox: [0, 0, 1, 1],
    }
}

/// The bench failure: Platinum 3 read by a pack holding only 1, 4 and 5.
/// The big digit says 5 (score 0.77, confidence 0.84); the emblems show 4
/// and 2, which both mean 3.
#[test]
fn platinum_3_is_not_read_as_5_when_the_emblems_disagree() {
    let f = division_from_votes(
        Some(vote(5, 0.84)),
        Some(vote(3, 0.90)),
        Some(vote(3, 0.88)),
    );
    assert_eq!(f.value, Some(3));
    assert_eq!(f.confidence, 0.88);
}

#[test]
fn division_votes_need_agreement() {
    let all = division_from_votes(Some(vote(5, 0.9)), Some(vote(5, 0.7)), Some(vote(5, 0.8)));
    assert_eq!((all.value, all.confidence), (Some(5), 0.9));
    // the big digit alone reads as before
    assert_eq!(
        division_from_votes(Some(vote(4, 0.9)), None, None).value,
        Some(4)
    );
    // one emblem alone is too little
    assert_eq!(
        division_from_votes(None, Some(vote(4, 0.9)), None).value,
        None
    );
    // a two-way split is None, not a guess
    assert_eq!(
        division_from_votes(Some(vote(5, 0.9)), Some(vote(3, 0.9)), None).value,
        None
    );
    assert_eq!(
        division_from_votes(Some(vote(5, 0.9)), Some(vote(3, 0.9)), Some(vote(2, 0.9))).value,
        None
    );
    assert_eq!(division_from_votes(None, None, None).value, None);
}

#[test]
fn a_division_missing_from_the_pack_reads_from_its_emblems() {
    let set = trained();
    for d in ["1", "4", "5"] {
        assert!(set.has_class(Family::Division, d));
    }
    assert!(!set.has_class(Family::Division, "3"));
    for d in ["1", "2", "3", "4", "5"] {
        assert!(set.has_class(Family::Emblem, d), "emblem {d}");
    }
    let sc = Scene {
        division: Some(3),
        ..Scene::default()
    };
    let r = reader().read(&paint(&sc)).expect("rank screen");
    assert_eq!(r.division.value, Some(3));
}

#[test]
fn the_emblems_read_the_division_without_the_tier_line() {
    let sc = Scene {
        tier: None,
        division: Some(3),
        ..Scene::default()
    };
    let img = paint(&sc);
    assert!(layout::rank_text(&img).is_none());
    let r = reader().read(&img).expect("rank screen");
    assert_eq!(r.role_rank.value, None);
    assert_eq!(r.division.value, Some(3));
}

#[test]
fn only_divisions_1_to_5_are_learned() {
    let mut l = Learner::new();
    let sc = Scene {
        division: Some(7),
        ..Scene::default()
    };
    l.add_frame(&paint(&sc), &labels(&sc));
    let set = l.build();
    assert!(set.has_class(Family::Tier, "HELLO"));
    assert!(!set.has_class(Family::Division, "7"));
}

// ---- mode banner ----

/// A banner that matches two classes about equally is `None`, not the one
/// that wins by a hair. Here the pack holds two classes learned from the
/// same banner, so their lead over each other is nil.
#[test]
fn a_banner_without_a_clear_lead_is_none() {
    let mut l = Learner::new();
    for (k, tr) in training().into_iter().enumerate() {
        let lab = Labels {
            next_mode_hint: Some(["A", "B"][k % 2].to_string()),
            ..labels(&tr)
        };
        l.add_frame(
            &paint(&Scene {
                banner: Some("HELLO 6V6"),
                ..tr
            }),
            &lab,
        );
    }
    let r = Reader::new(l.build());
    let read = r.read(&paint(&Scene::default())).expect("rank screen");
    assert_eq!(read.next_mode_hint.value, None);
    // a known banner with a clear lead still reads
    let read = reader()
        .read(&paint(&Scene::default()))
        .expect("rank screen");
    assert_eq!(read.next_mode_hint.value.as_deref(), Some("HELLO 6V6"));
}

// ---- field ranges ----

#[test]
fn values_out_of_their_range_are_none() {
    let mut img = paint(&Scene::default());
    // "Higher ranked than 120%" and a progress of 150%
    fill(&mut img, 1715, 1076, 1800, 1104, BG);
    draw(&mut img, "120%", 1730, 1082, 2, YELLOW);
    fill(&mut img, 960, 1058, 1140, 1100, BG);
    draw(&mut img, "150%", 978, 1068, 3, CYAN);
    let r = reader().read(&img).expect("rank screen");
    assert_eq!(r.higher_than_pct.value, None);
    assert_eq!(r.progress_pct.value, None);
    assert_eq!(r.progress_change.value, Some(27));
}

// ---- role ----

#[test]
fn the_role_comes_from_its_icon() {
    let r = reader();
    for role in Role::ICONS {
        let sc = Scene {
            role: Some(role),
            ..Scene::default()
        };
        let read = r.read(&paint(&sc)).expect("rank screen");
        assert_eq!(read.role.value, Some(role), "{role:?}");
        assert!(read.role.confidence > OPEN_CONFIDENCE, "{role:?}");
        assert!(read.role.bbox.is_some());
    }
}

#[test]
fn no_role_line_is_open_queue_at_a_capped_confidence() {
    let read = reader()
        .read(&paint(&Scene::default()))
        .expect("rank screen");
    assert_eq!(read.role.value, Some(Role::Open));
    assert_eq!(read.role.confidence, OPEN_CONFIDENCE);
    assert!(read.role.bbox.is_none());
    // below the weakest icon read that is accepted
    assert!(OPEN_CONFIDENCE < confidence(MIN_SCORE, MIN_MARGIN));
}

/// A role line dimmed by an overlay (or still fading in) is not an empty
/// band: it is unreadable, not Open.
#[test]
fn a_dimmed_role_line_is_not_open_queue() {
    let mut img = paint(&Scene::default());
    let dim = Rgb([90, 90, 90]);
    let right = draw(&mut img, "+", 1190, 290, 4, dim);
    draw(&mut img, "TEL", right + 20, 290, 4, dim);
    assert_eq!(layout::role_icon(&img), layout::RoleLine::Unclear);
    let read = reader().read(&img).expect("rank screen");
    assert_eq!(read.role.value, None);
}

#[test]
fn an_unknown_role_icon_is_none_not_a_guess() {
    let mut img = paint(&Scene::default());
    // a solid block icon no template was learned from
    let right = draw(&mut img, "#", 1190, 290, 4, WHITE);
    draw(&mut img, "TEL", right + 20, 290, 4, WHITE);
    let read = reader().read(&img).expect("rank screen");
    assert_eq!(read.role.value, None);
    assert_eq!(read.role.confidence, 0.0);
}

#[test]
fn ink_in_the_role_band_that_is_not_a_line_is_none() {
    let mut img = paint(&Scene::default());
    // a glow blob too tall to be a role line
    fill(&mut img, 1200, 240, 1360, 320, WHITE);
    let read = reader().read(&img).expect("rank screen");
    assert_eq!(read.role.value, None);
}

#[test]
fn role_labels_read_from_their_own_json_shape() {
    let l: Labels = serde_json::from_str(r#"{"role": "Support"}"#).unwrap();
    assert_eq!(l.role, Some(Role::Support));
    let l: Labels = serde_json::from_str(r#"{"division": 3}"#).unwrap();
    assert_eq!(l.role, None);
}

// ---- local bench (never in CI) ----

/// Local bench on labelled rank screens: set `SCUFFED_RANK_BENCH` to the
/// labels JSONL (one object per game: `game`, `file`, `readable` and the
/// field labels) and `SCUFFED_RANK_FRAMES` to the folder holding
/// `<game>/<file>`. Templates are learned leave-one-day-out: a frame is read
/// with templates built only from frames of other capture days, so no frame
/// is read by templates cut from itself (or from a same-session duplicate).
/// `SCUFFED_RANK_BENCH_OUT` gets one JSON line per frame with every read;
/// `SCUFFED_RANK_PACK_OUT` gets a pack learned from all readable frames.
#[test]
#[ignore = "needs local labelled rank screens"]
fn bench_labelled_rank_screens() {
    use std::collections::BTreeMap;
    let (Ok(jsonl), Ok(frames)) = (
        std::env::var("SCUFFED_RANK_BENCH"),
        std::env::var("SCUFFED_RANK_FRAMES"),
    ) else {
        return;
    };
    #[derive(serde::Deserialize)]
    struct Row {
        game: String,
        file: String,
        readable: bool,
        #[serde(flatten)]
        labels: Labels,
    }
    let mut rows: Vec<Row> = std::fs::read_to_string(&jsonl)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // roles live in a separate file (`game`, `role`), merged by game
    if let Ok(p) = std::env::var("SCUFFED_RANK_ROLE_LABELS") {
        #[derive(serde::Deserialize)]
        struct RoleRow {
            game: String,
            role: Option<Role>,
        }
        let roles: Vec<RoleRow> = std::fs::read_to_string(p)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for r in &mut rows {
            r.labels.role = roles.iter().find(|x| x.game == r.game).and_then(|x| x.role);
        }
    }
    let imgs: Vec<RgbImage> = rows
        .iter()
        .map(|r| {
            image::open(std::path::Path::new(&frames).join(&r.game).join(&r.file))
                .unwrap()
                .to_rgb8()
        })
        .collect();
    let day = |r: &Row| r.game[..8].to_string();
    let mut days: Vec<String> = rows.iter().map(day).collect();
    days.sort();
    days.dedup();
    // field -> [correct, wrong, unreadable, label null]
    let mut stats: BTreeMap<&str, [usize; 4]> = BTreeMap::new();
    let mut gate = [0usize; 2];
    let mut out = String::new();
    for d in &days {
        let mut l = Learner::new();
        for (r, img) in rows.iter().zip(&imgs) {
            if r.readable && day(r) != *d {
                l.add_frame(img, &r.labels);
            }
        }
        let reader = Reader::new(l.build());
        for (r, img) in rows.iter().zip(&imgs) {
            if day(r) != *d {
                continue;
            }
            let read = reader.read(img);
            if let Err(e @ ReadError::Unsupported { .. }) = read {
                eprintln!("UNSUPPORTED {} {}: {e}", r.game, r.file);
            }
            let read = read.ok();
            gate[usize::from(read.is_some() != r.readable)] += 1;
            out.push_str(
                &serde_json::json!({"game": r.game, "file": r.file, "fold": d, "read": read})
                    .to_string(),
            );
            out.push('\n');
            if !r.readable {
                continue;
            }
            let got = read.as_ref().map(RankRead::values).unwrap_or_default();
            for (name, want) in r.labels.values() {
                let st = stats.entry(name).or_default();
                let g = got
                    .iter()
                    .find(|(n, _)| *n == name)
                    .and_then(|(_, v)| v.clone());
                if want.is_none() {
                    st[3] += 1;
                }
                match g {
                    None => st[2] += 1,
                    Some(v) if Some(&v) == want.as_ref() => st[0] += 1,
                    Some(v) => {
                        st[1] += 1;
                        eprintln!("WRONG {} {name}: want {want:?} got {v}", r.game);
                    }
                }
            }
        }
    }
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let readable = rows.iter().filter(|r| r.readable).count();
    eprintln!(
        "set {jsonl}: {} frames ({} rank screens), {} day folds, HEAD {head}",
        rows.len(),
        readable,
        days.len()
    );
    eprintln!(
        "screen gate: right {} wrong {} (of {})",
        gate[0],
        gate[1],
        rows.len()
    );
    eprintln!("field: correct / wrong / unreadable (of which label null)");
    for name in Labels::NAMES {
        let s = stats.get(name).copied().unwrap_or_default();
        eprintln!("{name}: {} / {} / {} ({})", s[0], s[1], s[2], s[3]);
    }
    if let Ok(p) = std::env::var("SCUFFED_RANK_BENCH_OUT") {
        std::fs::write(p, out).unwrap();
    }
    if let Ok(p) = std::env::var("SCUFFED_RANK_PACK_OUT") {
        let mut l = Learner::new();
        for (r, img) in rows.iter().zip(&imgs) {
            if r.readable {
                l.add_frame(img, &r.labels);
            }
        }
        l.build().save_dir(std::path::Path::new(&p)).unwrap();
    }
}
