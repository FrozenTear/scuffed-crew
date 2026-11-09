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

use image::{Rgb, RgbImage};

use super::ink::{Plane, first_cluster, runs, tight};
use super::layout;
use super::templates::{Family, Learner, TemplateSet};
use super::*;

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
        }
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
        let w = width(&text, 2);
        fill(&mut img, 700, 1123, 700 + w + 60, 1161, col);
        draw(&mut img, &text, 730, 1135, 2, WHITE);
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
        draw(&mut img, &s, 2333 - width(&s, 3), 724, 3, g);
        if let Some(c) = sc.sr_change {
            let col = if c < 0 { Rgb([220, 30, 30]) } else { SR_GREEN };
            let t = c.abs().to_string();
            draw(&mut img, &t, 2333 - width(&t, 2), 752, 2, col);
        }
    }
    img
}

/// Labels for `sc`, in the bench's JSON shape.
fn labels(sc: &Scene) -> Labels {
    Labels {
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
            ..d.clone()
        },
        Scene {
            pct: 45,
            higher: 67,
            change: Some(89),
            sr: Some(5012),
            sr_change: Some(45),
            ..d.clone()
        },
        Scene {
            pct: 67,
            higher: 89,
            change: Some(-10),
            sr: Some(3456),
            sr_change: Some(-67),
            ..d.clone()
        },
        Scene {
            pct: 89,
            higher: 10,
            change: Some(23),
            sr: Some(7890),
            sr_change: Some(89),
            ..d
        },
    ]
}

fn trained() -> TemplateSet {
    let mut l = Learner::new();
    for sc in training() {
        l.add_frame(&paint(&sc), &labels(&sc));
    }
    l.build()
}

fn reader() -> Reader {
    Reader::new(trained())
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
    assert_eq!(back, set);
    assert!(TemplateSet::load_dir(&dir.path().join("missing")).is_err());
    assert!(Reader::load(&dir.path().join("nowhere")).is_err());
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
fn other_screens_and_sizes_are_refused() {
    let r = reader();
    assert!(r.read(&blank()).is_none(), "no RANK PROGRESS label");
    // blank 1080p has no gate label, so it is refused for content, not size
    assert!(r.read(&RgbImage::from_pixel(1920, 1080, BG)).is_none());
    // non-16:9 is refused for size
    assert!(r.read(&RgbImage::from_pixel(800, 600, BG)).is_none());
    let mut img = paint(&Scene::default());
    fill(&mut img, 830, 1060, 980, 1100, BG);
    assert!(r.read(&img).is_none(), "label gone");
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
    let rows: Vec<Row> = std::fs::read_to_string(&jsonl)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
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
