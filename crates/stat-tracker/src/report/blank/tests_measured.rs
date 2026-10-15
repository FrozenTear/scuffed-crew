//! Tests of the measured blanker on synthetic boards (`synth.rs`), including
//! the committed vision-baseline boards. No member frames.

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
    let mut mem = super::PortraitMemory::replay();
    super::blank_crop_gated(
        earlier,
        Some(6),
        chat,
        &mut mem,
        super::CaptureTime::wall(t0()),
    );
    super::blank_crop_gated(
        img,
        Some(6),
        chat,
        &mut mem,
        super::CaptureTime::wall(t0() + chrono::Duration::minutes(2)),
    )
}

/// `img` with one world pixel changed: a later live frame of the same
/// board (a byte-identical crop is a frozen render, never a reference).
fn live(img: &RgbImage) -> RgbImage {
    let mut e = img.clone();
    let (x, y) = (img.width() - 1, 2);
    let p = e.get_pixel(x, y).0;
    e.put_pixel(x, y, Rgb(p.map(|v| v ^ 0x10)));
    e
}

/// Blank `img` after an earlier capture that shows the same overlay: it
/// stayed put, so only the edge gates can catch it.
fn blank_still(img: &RgbImage) -> super::BlankedBoard {
    blank_after(&live(img), img, super::Chat::None)
}

/// The generator's post-match boards, before they are written to test-data:
/// every row keeps all six numbers.
#[test]
fn generated_post_match_boards_keep_every_number() {
    for (name, img) in [
        ("1440", synth::baseline_postgame_1440()),
        ("1080", synth::baseline_postgame_1080()),
    ] {
        let detail =
            super::reader::read_board_detail(&image::DynamicImage::ImageRgb8(img.clone()), 6)
                .unwrap_or_else(|| panic!("{name}: no board"));
        for (r, row) in detail.rows.iter().enumerate() {
            for (k, cell) in row.iter().enumerate() {
                assert_eq!(
                    cell.reading.value,
                    Some(synth::POSTGAME_VALUES[r][k]),
                    "{name} row {r} col {k}"
                );
                assert!(!cell.suspect, "{name} row {r} col {k} suspect");
            }
        }
        let b = super::blank_crop(&img, Some(6), super::Chat::None);
        assert_eq!(b.board, Ok(()), "{name}");
        assert_eq!(b.digit_rows(), 12, "{name}:\n{}", verdicts(&b));
    }
}

/// The committed post-match boards (synthetic 1440p and 1080p): every row
/// keeps all six numbers, read back exactly.
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
        assert_eq!(b.portraits(), 11, "{layout:?}:\n{}", verdicts(&b));
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

// ---- attacks on the synthetic post-match board ----------------------------
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
    assert_eq!(b.portraits(), 10, "{}", verdicts(&b));
    assert_not_copied(&b, &img, &drawn, "bar stroke");
}

#[test]
fn a_menu_box_laid_across_a_portrait_drops_the_portraits_it_covers() {
    // a context-menu box starting inside the portrait with a name header,
    // 300 px wide and 150 tall (judge forgery menu_hdrcol_optfar), opened
    // since the earlier capture: the rows it covers keep no portrait.
    // (Left open across both captures it passes the edge gates on this
    // board: the pinned-overlay limit of section 4.)
    let clean = synth::board(Layout::L1);
    let mut img = clean.clone();
    let mut drawn = Vec::new();
    let (l, _, top) = l1_row(1);
    let (bx0, by0) = (l + 31 + 30, top + 12);
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
    let b = blank_after(&clean, &img, super::Chat::None);
    assert!(b.rows[1].portrait.is_err(), "{}", verdicts(&b));
    assert!(b.rows[2].portrait.is_err(), "{}", verdicts(&b));
    // the box's last 12 px reach row 3; all others keep theirs but the
    // bottom row, whose lower separator the crop cuts
    assert!(b.rows[3].portrait.is_err(), "{}", verdicts(&b));
    assert_eq!(b.portraits(), 8, "{}", verdicts(&b));
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
        let mut mem = super::PortraitMemory::replay();
        // the earlier frame differs in one world pixel inside the crop
        let mut e = f.to_rgb8();
        let (cx, cy, cw, _) = crate::ocr::preprocess::crop_scoreboard_rect(e.width(), e.height());
        let p = e.get_pixel(cx + cw - 1, cy + 2).0;
        e.put_pixel(cx + cw - 1, cy + 2, Rgb(p.map(|v| v ^ 0x10)));
        let e = image::DynamicImage::ImageRgb8(e);
        super::blanked_scoreboard_gated(&e, Some(6), &mut mem, super::CaptureTime::wall(t0()));
        super::blanked_scoreboard_gated(
            f,
            Some(6),
            &mut mem,
            super::CaptureTime::wall(t0() + chrono::Duration::minutes(2)),
        )
    };
    let b = twice(&clean);
    assert_eq!(b.portraits(), 11, "{}", verdicts(&b));
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
    let b = blank_after(&live(&img), &img, super::Chat::Unknown);
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
    // a 6v6 board read as 5v5 matches no 5v5 layout, and a board the
    // header strip does not match (here: the header removed) fails as a
    // whole
    let img = post_match();
    assert!(
        super::blank_crop(&img, Some(5), super::Chat::None)
            .board
            .is_err()
    );
    assert!(
        super::blank_crop(&synth::board(Layout::L4), Some(5), super::Chat::None)
            .board
            .is_err()
    );
    assert!(
        super::blank_crop(&synth::board_5v5(5), Some(7), super::Chat::None)
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
    assert_eq!(b.portraits(), 10, "{}", verdicts(&b));
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
        assert_eq!(b.portraits(), 10, "row {row}: {}", verdicts(&b));
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
    assert_eq!(b.portraits(), 10, "{}", verdicts(&b));
    // an earlier capture under a minute old is not a reference
    let mut mem = super::PortraitMemory::replay();
    super::blank_crop_gated(
        &clean,
        Some(6),
        super::Chat::None,
        &mut mem,
        super::CaptureTime::wall(t0()),
    );
    let b = super::blank_crop_gated(
        &clean,
        Some(6),
        super::Chat::None,
        &mut mem,
        super::CaptureTime::wall(t0() + chrono::Duration::seconds(50)),
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
    let b = blank_after(&live(&big), &big, super::Chat::None);
    assert_eq!(b.board, Ok(()), "the 1.25x board must measure");
    assert!(
        b.rows.iter().all(|r| r.portrait.is_err()),
        "{}",
        verdicts(&b)
    );
    // the same board at the native scale keeps them
    assert_eq!(blank_still(&img).portraits(), 11);
}

// ---- review round (claude-3 pre-review of #236) ----------------------------

/// Portraits a board keeps when its earlier capture is a live copy.
fn kept(img: &RgbImage) -> usize {
    blank_still(img).portraits()
}

#[test]
fn the_bottom_row_whose_separator_the_crop_cuts_keeps_no_portrait() {
    // B4: on the 1007 px crop the last row's lower separator is cut off
    let b = blank_still(&synth::board(Layout::L1));
    assert!(b.rows[11].portrait.is_err(), "{}", verdicts(&b));
    assert_eq!(b.portraits(), 11, "{}", verdicts(&b));
}

#[test]
fn a_frozen_render_is_never_a_reference() {
    // B2: a byte-identical earlier crop proves nothing about an overlay
    let img = synth::board(Layout::L1);
    let b = blank_after(&img, &img, super::Chat::None);
    assert_eq!(b.portraits(), 0, "{}", verdicts(&b));
    assert!(kept(&img) > 0);
}

#[test]
fn a_clock_step_does_not_make_a_fresh_capture_a_reference() {
    // B1 / R3-2: the daemon's memory takes the age on the monotonic instant
    // of the frame grab too, not only on the stamped wall clock
    let img = synth::board(Layout::L1);
    let grab = std::time::Instant::now();
    let at = |wall_min: i64, mono_s: u64| super::CaptureTime {
        wall: t0() + chrono::Duration::minutes(wall_min),
        mono: grab + std::time::Duration::from_secs(mono_s),
    };
    let kept_after = |mono_s: u64| {
        let mut mem = super::PortraitMemory::new();
        super::blank_crop_gated(&live(&img), Some(6), super::Chat::None, &mut mem, at(0, 0));
        super::blank_crop_gated(&img, Some(6), super::Chat::None, &mut mem, at(5, mono_s))
    };
    // the wall clock jumped 5 minutes, the grabs were 30 s apart: no reference
    let b = kept_after(30);
    assert_eq!(b.portraits(), 0, "{}", verdicts(&b));
    // 70 s apart on the monotonic clock: a reference
    let b = kept_after(70);
    assert_eq!(b.portraits(), 11, "{}", verdicts(&b));
}

#[test]
fn fast_tab_presses_keep_a_reference_old_enough() {
    // K5: twelve captures 5 s apart overflow the 8 kept; the oldest stays
    // until a newer one is old enough, so the capture at 65 s has one
    let img = synth::board(Layout::L1);
    let mut mem = super::PortraitMemory::replay();
    for i in 0..12i64 {
        let mut f = img.clone();
        f.put_pixel(img.width() - 1, 2, Rgb([i as u8, 0, 0]));
        super::blank_crop_gated(
            &f,
            Some(6),
            super::Chat::None,
            &mut mem,
            super::CaptureTime::wall(t0() + chrono::Duration::seconds(5 * i)),
        );
    }
    let b = super::blank_crop_gated(
        &img,
        Some(6),
        super::Chat::None,
        &mut mem,
        super::CaptureTime::wall(t0() + chrono::Duration::seconds(65)),
    );
    assert_eq!(b.portraits(), 11, "{}", verdicts(&b));
}

#[test]
fn text_left_of_a_bar_at_the_crop_edge_drops_the_portrait() {
    // B3: the wide perk layout puts the bar at x 0, so nothing is left of
    // it in the crop; white text over the bar's first px still drops it
    let mut img = synth::board(Layout::L4);
    let (l, _) = Layout::L4.edges();
    assert!(l < 8, "L4 is the layout with the bar at the crop edge");
    let (top, _) = synth::row_span(3);
    let mut drawn = Vec::new();
    // the last letter's stroke ends in the bar's first 2 px, left of where
    // the bar's own stroke check starts
    stroke(
        &mut img,
        &mut drawn,
        l,
        top + 26,
        l + 2,
        top + 40,
        Rgb([240, 240, 240]),
    );
    let b = blank_still(&img);
    assert_eq!(
        b.rows[3].portrait,
        Err("text continues left of the bar".to_string()),
        "{}",
        verdicts(&b)
    );
    assert_not_copied(&b, &img, &drawn, "left text");
}

// ---- 5v5 boards (measured 2026-10-15) ---------------------------------------

#[test]
fn a_synthetic_5v5_board_measures_like_real_ones() {
    let img = synth::board_5v5(5);
    let b = board::measure(&img, 5).unwrap_or_else(|e| panic!("{}", e.0));
    assert_eq!((b.l, b.r, b.hb), (0, synth::R5, synth::T1_5));
    assert_eq!(b.rows.len(), 10);
    for (i, row) in b.rows.iter().enumerate() {
        let (top, _) = synth::row_span_5(i);
        assert!(row.verified, "row {i}");
        // the grid is fitted to +-1 px (2 px separators at 88/75 scale)
        assert!(row.top.abs_diff(top) <= 2, "row {i}: {} vs {top}", row.top);
    }
    // the name column starts after the portrait art, ends before the perks
    assert_eq!(b.name, (57, 385));
}

/// FrozenTear's 5v5 boards were dropped whole ("team size 5 has no measured
/// geometry"). A 5v5 frame now keeps every row with its name filled.
#[test]
fn a_5v5_frame_gets_a_picture_with_every_name_filled() {
    for width in [2560, 1920] {
        let frame = synth::tab_frame(synth::Teams::Five, width, 5);
        let out = super::blanked_scoreboard(&frame, Some(5));
        assert_eq!(out.board, Ok(()), "{width}");
        assert!(out.has_picture(), "{width}");
        assert_eq!(out.name_fill_rows, 10, "{width}");
        let k = f64::from(out.image.width()) / f64::from(synth::W);
        for (x0, y0, x1, y1) in synth::name_boxes_5v5() {
            let sc = |v: u32| (f64::from(v) * k).round() as u32;
            for y in sc(y0)..sc(y1) {
                for x in sc(x0)..sc(x1) {
                    assert_eq!(
                        *out.image.get_pixel(x, y),
                        super::FILL,
                        "{width} ({x}, {y})"
                    );
                }
            }
        }
        if width == 2560 {
            // the stats are kept as captured: E to MIT of every row
            let src = synth::board_5v5(5);
            for i in 0..10 {
                let (top, bot) = synth::row_span_5(i);
                for y in top + 3..bot - 3 {
                    for x in 560..1040 {
                        assert_eq!(out.image.get_pixel(x, y), src.get_pixel(x, y), "({x}, {y})");
                    }
                }
            }
        }
    }
}

#[test]
fn a_6v6_frame_still_passes_with_the_5v5_geometry_in() {
    for width in [2560, 1920] {
        let frame = synth::tab_frame(synth::Teams::Six, width, 6);
        let out = super::blanked_scoreboard(&frame, Some(6));
        assert_eq!(out.board, Ok(()), "{width}");
        assert!(out.has_picture(), "{width}");
    }
    let frame = synth::tab_frame(synth::Teams::Six, 2560, 6);
    let out = super::blanked_scoreboard(&frame, Some(6));
    for (x0, y0, x1, y1) in synth::name_boxes(Layout::L4) {
        for y in y0..y1 {
            for x in x0..x1 {
                assert_eq!(*out.image.get_pixel(x, y), super::FILL, "({x}, {y})");
            }
        }
    }
}

fn baseline_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-data/vision-baseline")
        .join(name)
}

/// The committed boards are the generator's pixels. A real capture in their
/// place fails here.
#[test]
fn committed_baseline_boards_match_the_generator() {
    for (name, want) in [
        (
            "native_postgame_table_names_blanked.png",
            synth::baseline_postgame_1440(),
        ),
        (
            "s075_postgame_table_names_blanked.png",
            synth::baseline_postgame_1080(),
        ),
        ("s075_tab_board_stats_only.png", synth::baseline_tab_1080()),
    ] {
        let path = baseline_path(name);
        let got = image::open(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .to_rgb8();
        assert_eq!(got.dimensions(), want.dimensions(), "{name}");
        assert_eq!(
            got.as_raw(),
            want.as_raw(),
            "{name} is not the synthetic board"
        );
    }
}

/// Rewrite the three baseline boards from the generator. Not part of `cargo test`.
#[test]
#[ignore = "writes test-data/vision-baseline boards"]
fn write_synthetic_baseline_boards() {
    for (name, img) in [
        (
            "native_postgame_table_names_blanked.png",
            synth::baseline_postgame_1440(),
        ),
        (
            "s075_postgame_table_names_blanked.png",
            synth::baseline_postgame_1080(),
        ),
        ("s075_tab_board_stats_only.png", synth::baseline_tab_1080()),
    ] {
        img.save(baseline_path(name)).unwrap();
    }
}

/// A half-drawn 5v5 board never shows a name, whatever it keeps.
#[test]
fn a_half_drawn_5v5_board_shows_no_name() {
    for drawn in 0..5 {
        let frame = synth::tab_frame(synth::Teams::Five, 2560, drawn);
        let out = super::blanked_scoreboard(&frame, Some(5));
        for (row, (x0, y0, x1, y1)) in synth::name_boxes_5v5().into_iter().enumerate() {
            if row % 5 >= drawn {
                continue;
            }
            for y in y0..y1 {
                for x in x0..x1 {
                    assert_eq!(
                        *out.image.get_pixel(x, y),
                        super::FILL,
                        "{drawn} ({x}, {y})"
                    );
                }
            }
        }
    }
}
