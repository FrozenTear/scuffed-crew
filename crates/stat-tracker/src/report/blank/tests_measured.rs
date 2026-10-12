//! Tests of the measured blanker on synthetic boards (`synth.rs`) and on the
//! in-repo post-match fixtures. No member frames.

use super::synth::{self, Layout};
use super::{board, portrait};

const LAYOUTS: [Layout; 5] = [
    Layout::L1,
    Layout::L2,
    Layout::L3,
    Layout::L4,
    Layout::PostMatch,
];

#[test]
fn synthetic_boards_measure_like_real_ones() {
    for layout in LAYOUTS {
        let img = synth::board(layout);
        let b = board::measure(&img, 6).unwrap_or_else(|e| panic!("{layout:?}: {}", e.0));
        let (l, r) = layout.edges();
        assert_eq!((b.l, b.r, b.hb), (l, r, synth::T1), "{layout:?}");
        assert_eq!(b.rows.len(), 12, "{layout:?}");
        for (i, row) in b.rows.iter().enumerate() {
            let (top, bot) = synth::row_span(i);
            assert_eq!((row.top, row.verified), (top, true), "{layout:?} row {i}");
            assert!(
                row.bot.abs_diff(bot) <= 1,
                "{layout:?} row {i}: {} vs {bot}",
                row.bot
            );
        }
    }
}

#[test]
fn synthetic_portraits_pass_the_bar_checks() {
    for layout in LAYOUTS {
        let img = synth::board(layout);
        let b = board::measure(&img, 6).unwrap();
        let rows: Vec<&board::Row> = b.rows.iter().collect();
        let pe = portrait::portrait_edge(&img, b.l, &rows, b.s).unwrap();
        assert_eq!(pe, b.l + 104, "{layout:?}");
        let x0 = b.l.saturating_sub(3);
        let x1 = (b.l + 36).min(img.width());
        let sal = portrait::salient_map(&img, x0, x1);
        for row in &b.rows {
            portrait::bar_colour(&img, b.l, row, b.s).unwrap();
            portrait::verdict(&img, &sal, x0, (x1 - x0) as usize, b.l, row, b.s)
                .unwrap_or_else(|e| panic!("{layout:?} row {}/{}: {e}", row.team, row.idx));
        }
    }
}

fn verdicts(b: &super::BlankedBoard) -> String {
    b.rows
        .iter()
        .map(|r| {
            format!(
                "t{}r{} digits {:?} portrait {:?}",
                r.team, r.idx, r.digits, r.portrait
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Capture times for the portrait memory: an earlier capture two minutes
/// before the one blanked.
fn t0() -> chrono::DateTime<chrono::Utc> {
    "2026-10-12T20:00:00Z".parse().unwrap()
}

/// Blank `img` captured two minutes after `earlier`, the same row's earlier
/// capture.
fn blank_after(earlier: &RgbImage, img: &RgbImage, chat: super::Chat) -> super::BlankedBoard {
    let mut mem = super::PortraitMemory::new();
    super::blank_crop_seen(earlier, Some(6), chat, &mut mem, t0());
    super::blank_crop_seen(
        img,
        Some(6),
        chat,
        &mut mem,
        t0() + chrono::Duration::minutes(2),
    )
}

/// Blank `img` after an identical earlier capture: whatever is drawn on it
/// stayed put, so only the edge gates can catch it.
fn blank_still(img: &RgbImage) -> super::BlankedBoard {
    blank_after(img, img, super::Chat::None)
}

/// The in-repo post-match fixtures (real 1440p and 1080p boards, names
/// pre-filled): every row keeps all six numbers, read back exactly.
#[test]
fn real_post_match_fixtures_keep_every_number() {
    for name in [
        "native_postgame_table_names_blanked.png",
        "s075_postgame_table_names_blanked.png",
    ] {
        let img = super::reader::tests::fixture(name).to_rgb8();
        let b = super::blank_crop(&img, Some(6), super::Chat::None);
        assert_eq!(b.board, Ok(()), "{name}");
        assert_eq!(b.digit_rows(), 12, "{name}:\n{}", verdicts(&b));
    }
}

#[test]
fn synthetic_boards_keep_portraits_and_no_name() {
    for layout in LAYOUTS {
        let img = synth::board(layout);
        // the first capture of a game has no earlier capture: no portrait
        let first = super::blank_crop(&img, Some(6), super::Chat::None);
        assert_eq!(first.portraits(), 0, "{layout:?}");
        let b = blank_still(&img);
        assert_eq!(b.board, Ok(()), "{layout:?}");
        assert_eq!(b.rows.len(), 12, "{layout:?}");
        // synthetic digits are not real glyphs: the gates may drop them
        assert_eq!(b.portraits(), 12, "{layout:?}:\n{}", verdicts(&b));
        for (x0, y0, x1, y1) in synth::name_boxes(layout) {
            for y in y0..y1 {
                for x in x0..x1 {
                    assert_eq!(
                        *b.image.get_pixel(x, y),
                        super::FILL,
                        "{layout:?} name px {x},{y}"
                    );
                }
            }
        }
        // the header strip and the world are fill
        assert_eq!(*b.image.get_pixel(layout.edges().0 + 200, 5), super::FILL);
        assert_eq!(*b.image.get_pixel(1600, 500), super::FILL);
    }
}

// ---- attacks on a real board (the in-repo post-match fixture) -------------
//
// The native fixture measures L 367, R 1297, rows at 29 + 75 i and
// 566 + 75 i. Each test draws one attack, blanks the board, and checks that
// the layer that should catch it drops the right row or portrait and that no
// drawn pixel survives.

use image::{Rgb, RgbImage};

const PM_L: u32 = 367;
const PM_R: u32 = 1297;

fn post_match() -> RgbImage {
    super::reader::tests::fixture("native_postgame_table_names_blanked.png").to_rgb8()
}

/// Row `r` (0-11) of the fixture: `(top, bottom)`.
fn pm_row(r: usize) -> (u32, u32) {
    synth::row_span(r)
}

/// Column `k`'s digit centre on the fixture.
fn pm_col(k: usize) -> u32 {
    (f64::from(PM_R) - super::board::OFF[k]).round() as u32
}

/// Draw strokes and remember every pixel drawn.
fn stroke(
    img: &mut RgbImage,
    drawn: &mut Vec<(u32, u32)>,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    c: Rgb<u8>,
) {
    for y in y0..y1 {
        for x in x0..x1 {
            img.put_pixel(x, y, c);
            drawn.push((x, y));
        }
    }
}

/// A short white "name": letter-like strokes at digit height.
fn draw_name(img: &mut RgbImage, drawn: &mut Vec<(u32, u32)>, x: u32, top: u32) {
    let w = Rgb([240, 240, 240]);
    for j in 0..4u32 {
        let x0 = x + j * 9;
        stroke(img, drawn, x0, top, x0 + 3, top + 13, w);
        stroke(img, drawn, x0, top, x0 + 7, top + 3, w);
        stroke(img, drawn, x0, top + 6, x0 + 6, top + 8, w);
    }
}

fn blank(img: &RgbImage) -> super::BlankedBoard {
    super::blank_crop(img, Some(6), super::Chat::None)
}

fn assert_not_copied(b: &super::BlankedBoard, src: &RgbImage, drawn: &[(u32, u32)], what: &str) {
    for &(x, y) in drawn {
        assert_ne!(
            b.image.get_pixel(x, y),
            src.get_pixel(x, y),
            "{what}: drawn px {x},{y} copied"
        );
    }
}

#[test]
fn a_name_drawn_in_a_stat_cell_drops_that_row_only() {
    let mut img = post_match();
    let mut drawn = Vec::new();
    let (top, _) = pm_row(0);
    draw_name(&mut img, &mut drawn, pm_col(4) - 16, top + 30);
    let b = blank(&img);
    assert!(b.rows[0].digits.is_err(), "{}", verdicts(&b));
    assert_eq!(b.digit_rows(), 11, "{}", verdicts(&b));
    assert_not_copied(&b, &img, &drawn, "name in H");
}

#[test]
fn a_notice_across_the_columns_drops_that_row() {
    let mut img = post_match();
    let mut drawn = Vec::new();
    let (top, _) = pm_row(3);
    for x in (pm_col(3) - 40..pm_col(5) + 20).step_by(12) {
        stroke(
            &mut img,
            &mut drawn,
            x,
            top + 24,
            x + 4,
            top + 50,
            Rgb([235, 235, 235]),
        );
    }
    let b = blank(&img);
    assert!(b.rows[3].digits.is_err(), "{}", verdicts(&b));
    assert_not_copied(&b, &img, &drawn, "notice");
}

#[test]
fn a_grey_row_keeps_no_digits() {
    // QC1-9: a near-neutral tint means text behind the row is not tinted
    // away from the digits' white.
    let mut img = post_match();
    let (top, bot) = pm_row(5);
    for y in top..bot {
        for x in PM_L + 104..PM_R {
            let p = img.get_pixel(x, y).0;
            let g = ((u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2])) / 3) as u8;
            img.put_pixel(x, y, Rgb([g, g, g]));
        }
    }
    let b = blank(&img);
    assert!(b.rows[5].digits.is_err(), "{}", verdicts(&b));
}

#[test]
fn text_over_the_role_bar_drops_that_portrait_only() {
    // synthetic board: its portraits all pass (the fixture's placeholder
    // portraits sit on the row colour and lose their edge)
    let mut img = synth::board(Layout::L1);
    let (l, _) = Layout::L1.edges();
    let mut drawn = Vec::new();
    let (top, _) = synth::row_span(7);
    // a white stroke across the bar outside the role icon (above it)
    stroke(
        &mut img,
        &mut drawn,
        l + 3,
        top + 6,
        l + 29,
        top + 9,
        Rgb([245, 245, 245]),
    );
    let b = blank_still(&img);
    assert!(b.rows[7].portrait.is_err(), "{}", verdicts(&b));
    assert_eq!(b.portraits(), 11, "{}", verdicts(&b));
    assert_not_copied(&b, &img, &drawn, "bar stroke");
}

#[test]
fn a_menu_box_laid_across_a_portrait_drops_the_portraits_it_covers() {
    // a context-menu box starting inside the portrait with a coloured name
    // header, 300 px wide and 150 tall (judge forgery menu_hdrcol_optfar)
    let mut img = post_match();
    let mut drawn = Vec::new();
    let (top, _) = pm_row(1);
    let (bx0, by0) = (PM_L + 31 + 30, top + 12);
    for y in by0..by0 + 150 {
        for x in bx0..bx0 + 300 {
            let p = img.get_pixel(x, y).0;
            let q = std::array::from_fn(|c| {
                ((f64::from(p[c]) * 0.08 + [22.0, 22.0, 26.0][c] * 0.92).round()) as u8
            });
            img.put_pixel(x, y, Rgb(q));
        }
    }
    draw_name(&mut img, &mut drawn, bx0 + 8, by0 + 8);
    let b = blank(&img);
    assert!(b.rows[1].portrait.is_err(), "{}", verdicts(&b));
    assert!(b.rows[2].portrait.is_err(), "{}", verdicts(&b));
    assert_not_copied(&b, &img, &drawn, "menu header");
}

#[test]
fn touching_digits_joined_by_a_thick_stroke_are_not_kept() {
    // E "12" on row 0 with a solid bridge between the two glyphs: a bold
    // name glyph joins its neighbour like this, real digits never do
    let mut img = post_match();
    let (top, _) = pm_row(0);
    let cx = pm_col(0);
    let mut drawn = Vec::new();
    stroke(
        &mut img,
        &mut drawn,
        cx - 2,
        top + 30,
        cx + 2,
        top + 43,
        Rgb([250, 250, 250]),
    );
    let b = blank(&img);
    assert!(b.rows[0].digits.is_err(), "{}", verdicts(&b));
}

#[test]
fn chat_on_the_frame_drops_the_portraits_it_can_reach() {
    let clean = synth::frame(Layout::L1, &synth::VALUES);
    assert_eq!(super::chat_over_board(&clean.to_rgb8()), super::Chat::None);
    // the chat stays put between the two captures: the chat gate decides
    let twice = |f: &image::DynamicImage| {
        let mut mem = super::PortraitMemory::new();
        super::blanked_scoreboard_seen(f, Some(6), &mut mem, t0());
        super::blanked_scoreboard_seen(f, Some(6), &mut mem, t0() + chrono::Duration::minutes(2))
    };
    let b = twice(&clean);
    assert_eq!(b.portraits(), 12, "{}", verdicts(&b));
    // one chat line at the bottom of the chat box, left of the crop
    let mut f = clean.to_rgb8();
    let mut drawn = Vec::new();
    for x in (90..440).step_by(9) {
        stroke(&mut f, &mut drawn, x, 1286, x + 3, 1302, Rgb([124, 5, 124]));
    }
    let chat = super::chat_over_board(&f);
    assert!(matches!(chat, super::Chat::ReachesY(_)), "{chat:?}");
    let b = twice(&image::DynamicImage::ImageRgb8(f.clone()));
    let dropped: Vec<usize> = b
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.portrait.is_err())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(dropped, vec![9, 10, 11], "{}", verdicts(&b));
    // a stack of 12 lines reaches frame y 978 (crop y 762): with one line
    // of margin, team-2 rows 2-5 (indices 8-11) can be under it
    for k in 1..12u32 {
        let y = 1286 - 28 * k;
        for x in (90..440).step_by(9) {
            stroke(&mut f, &mut drawn, x, y, x + 3, y + 16, Rgb([124, 5, 124]));
        }
    }
    let b = twice(&image::DynamicImage::ImageRgb8(f));
    let dropped: Vec<usize> = b
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.portrait.is_err())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(dropped, vec![8, 9, 10, 11], "{}", verdicts(&b));
}

#[test]
fn a_crop_without_its_frame_drops_the_chat_reach_portraits() {
    let img = synth::board(Layout::L1);
    let b = blank_after(&img, &img, super::Chat::Unknown);
    let dropped: Vec<usize> = b
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.portrait.is_err())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(dropped, vec![9, 10, 11], "{}", verdicts(&b));
    // digits do not depend on chat
    let pm = super::blank_crop(&post_match(), Some(6), super::Chat::Unknown);
    assert_eq!(pm.digit_rows(), 12);
}

#[test]
fn unmeasured_layouts_keep_nothing() {
    // 5v5 has no measured geometry yet; a board the header strip does not
    // match (here: the header removed) fails as a whole
    let img = post_match();
    assert!(
        super::blank_crop(&img, Some(5), super::Chat::None)
            .board
            .is_err()
    );
    let mut flat = img.clone();
    for y in 0..32 {
        for x in 0..flat.width() {
            flat.put_pixel(x, y, Rgb([40, 40, 40]));
        }
    }
    let b = super::blank_crop(&flat, Some(6), super::Chat::None);
    assert!(b.board.is_err());
    assert!(b.image.pixels().all(|p| *p == super::FILL));
}

// ---- the portrait content check (blank table 10) ---------------------------
//
// Attack round 2: a dark context menu opened inside a portrait, a menu
// caught mid fade-in, and coloured text across the portrait edge all passed
// the edge gates and were copied with the portrait. Each is drawn on a
// synthetic board whose earlier capture is clean.

/// Row `r` of the L1 synthetic board: `(L, pe, top)`.
fn l1_row(r: usize) -> (u32, u32, u32) {
    let (l, _) = Layout::L1.edges();
    (l, l + 104, synth::row_span(r).0)
}

/// Alpha-blend a box of colour `c` at opacity `op` (no drawn px recorded:
/// the box itself carries no name).
fn shade(img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, c: [f64; 3], op: f64) {
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.get_pixel(x, y).0;
            img.put_pixel(
                x,
                y,
                Rgb(std::array::from_fn(|k| {
                    (f64::from(p[k]) * (1.0 - op) + c[k] * op).round() as u8
                })),
            );
        }
    }
}

fn portrait_dropped(b: &super::BlankedBoard, row: usize, src: &RgbImage, drawn: &[(u32, u32)]) {
    assert!(b.rows[row].portrait.is_err(), "{}", verdicts(b));
    assert_not_copied(b, src, drawn, "overlay");
}

#[test]
fn a_dark_menu_opened_inside_a_portrait_keeps_nothing_of_it() {
    // CO-1 / G1: a box over 35% of the row height from 44 px inside the
    // portrait, the player name in white starting inside the portrait
    let clean = synth::board(Layout::L1);
    let mut img = clean.clone();
    let (_, pe, top) = l1_row(2);
    shade(
        &mut img,
        pe - 44,
        top + 24,
        pe + 220,
        top + 50,
        [22.0, 22.0, 26.0],
        0.92,
    );
    let mut drawn = Vec::new();
    draw_name(&mut img, &mut drawn, pe - 40, top + 31);
    let b = blank_after(&clean, &img, super::Chat::None);
    portrait_dropped(&b, 2, &img, &drawn);
    assert_eq!(b.portraits(), 11, "{}", verdicts(&b));
}

#[test]
fn a_menu_caught_mid_fade_keeps_nothing_of_it() {
    // P-R1: the menu at 50% of its fade, white text at half strength
    let clean = synth::board(Layout::L1);
    let mut img = clean.clone();
    let (_, pe, top) = l1_row(8);
    shade(
        &mut img,
        pe - 74,
        top + 12,
        pe + 226,
        top + 70,
        [22.0, 22.0, 26.0],
        0.46,
    );
    let mut drawn = Vec::new();
    let mut ink = clean.clone();
    draw_name(&mut ink, &mut drawn, pe - 66, top + 20);
    for &(x, y) in &drawn {
        let p = img.get_pixel(x, y).0;
        img.put_pixel(x, y, Rgb(p.map(|v| ((u16::from(v) + 240) / 2) as u8)));
    }
    let b = blank_after(&clean, &img, super::Chat::None);
    portrait_dropped(&b, 8, &img, &drawn);
}

#[test]
fn coloured_text_across_the_portrait_edge_keeps_nothing_of_it() {
    // P-R2: orange, grey 140 and chat-blue names from inside the portrait
    // across its right edge, no box
    for (row, c) in [
        (1, [255u8, 140, 30]),
        (4, [140, 140, 140]),
        (9, [90, 170, 255]),
    ] {
        let clean = synth::board(Layout::L1);
        let mut img = clean.clone();
        let (_, pe, top) = l1_row(row);
        let mut drawn = Vec::new();
        let mut ink = clean.clone();
        draw_name(&mut ink, &mut drawn, pe - 30, top + 28);
        for &(x, y) in &drawn {
            img.put_pixel(x, y, Rgb(c));
        }
        let b = blank_after(&clean, &img, super::Chat::None);
        portrait_dropped(&b, row, &img, &drawn);
        assert_eq!(b.portraits(), 11, "row {row}: {}", verdicts(&b));
    }
}

#[test]
fn a_hero_swap_or_no_earlier_capture_keeps_no_portrait() {
    let clean = synth::board(Layout::L1);
    let mut other = clean.clone();
    let (l, pe, top) = l1_row(3);
    for y in top + 4..top + 69 {
        for x in l + 31..pe {
            let p = other.get_pixel(x, y).0;
            other.put_pixel(x, y, Rgb([p[2], p[0], p[1]]));
            if (x + y) % 7 == 0 {
                other.put_pixel(x, y, Rgb([250, 250, 250]));
            }
        }
    }
    let b = blank_after(&clean, &other, super::Chat::None);
    assert!(b.rows[3].portrait.is_err(), "{}", verdicts(&b));
    assert_eq!(b.portraits(), 11, "{}", verdicts(&b));
    // an earlier capture under a minute old is not a reference
    let mut mem = super::PortraitMemory::new();
    super::blank_crop_seen(&clean, Some(6), super::Chat::None, &mut mem, t0());
    let b = super::blank_crop_seen(
        &clean,
        Some(6),
        super::Chat::None,
        &mut mem,
        t0() + chrono::Duration::seconds(50),
    );
    assert_eq!(b.portraits(), 0, "{}", verdicts(&b));
}

#[test]
fn a_box_that_stays_over_the_portrait_column_hides_its_separators() {
    // P-V1 drawn on both captures: a tooltip over three portrait squares
    let mut img = synth::board(Layout::L1);
    let (l, pe, top) = l1_row(2);
    let (_, _, top4) = l1_row(4);
    shade(
        &mut img,
        l + 35,
        top + 8,
        pe - 4,
        top4 + 65,
        [22.0, 22.0, 26.0],
        0.95,
    );
    let mut drawn = Vec::new();
    for k in 0..6 {
        draw_name(&mut img, &mut drawn, l + 38, top + 20 + 30 * k);
    }
    let b = blank_still(&img);
    for row in 2..=4 {
        assert!(b.rows[row].portrait.is_err(), "row {row}: {}", verdicts(&b));
    }
    assert_not_copied(&b, &img, &drawn, "tooltip");
}

#[test]
fn a_panel_that_stays_over_the_role_bar_hides_the_icon() {
    // P-L1 / P-B1 drawn on both captures: a box in the bar's own colour over
    // the bar and most of the portrait, text over the portrait square
    let mut img = synth::board(Layout::L1);
    let (l, pe, top) = l1_row(1);
    let bar = synth::BAR_T1.0.map(f64::from);
    shade(&mut img, l, top + 6, pe - 8, top + 67, bar, 1.0);
    let mut drawn = Vec::new();
    draw_name(&mut img, &mut drawn, l + 36, top + 30);
    let b = blank_still(&img);
    portrait_dropped(&b, 1, &img, &drawn);
}

#[test]
fn portraits_keep_only_at_the_measured_scales() {
    // a 1.25x board (1600p) measures, but its portraits were never checked
    let img = synth::board(Layout::L1);
    let (w, h) = img.dimensions();
    let big = image::imageops::resize(
        &img,
        w * 5 / 4,
        h * 5 / 4,
        image::imageops::FilterType::Triangle,
    );
    let b = blank_after(&big, &big, super::Chat::None);
    if b.board.is_ok() {
        assert_eq!(b.portraits(), 0, "{}", verdicts(&b));
    }
}
