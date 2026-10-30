//! The loading screen (`ENTERING GAME`): black above and below a lit
//! strip. Pixel check only, no OCR.
//!
//! It is the shape the end-reel letterbox check in [`super::match_end`]
//! turns away as a loading slab: both 7% edge bands dark, a partly lit
//! middle and both shoulders black. The strip is a vertical navy gradient
//! (each row one colour, the colour drifting down the screen), so its
//! flatness is checked per row as well as across the whole middle (the
//! end-reel flat-UI test); either one passing is enough, so the per-row
//! test only adds the gradient. A dark scene (a spawn room, footage) has
//! rows of many colours and no dominant one. Used only by the
//! opt-in pre-game screen saves; no boundary or outcome reads it.
//!
//! The saves use the stricter [`is_entering_game`]: the same check plus
//! one line of white text centred on the strip, so a navy band with its
//! text off centre, wider than half the screen or with a bar far under it
//! fails, and so does a band with no text yet (the fade-in). There is no
//! minimum width: on real frames the `ENTERING GAME` text is about 0.17 of
//! the width and an `APPLYING UPDATE` band's about 0.19, too close to tell
//! apart. What tells them apart is the words: a band of exactly two words
//! whose second is at least [`WORD_RATIO_REJECT`] as wide as the first
//! (`APPLYING UPDATE`, about 0.74) fails; `ENTERING GAME` (about 0.61)
//! passes. Of three clusters, the end one across the wider gap is left
//! out first as a spinner, but only when it is spinner-sized (at most
//! [`SPINNER_WIDTH_MAX`] of the width, [`SPINNER_HEIGHT_MAX`] of the
//! height and roughly square, about 1.5 times the stand-in spinner);
//! otherwise the three are not two words. Anything else passes, and the
//! saves keep only the last frame that passes before the game starts. The
//! ratio is tuned on the English texts: on other clients only the game
//! start decides.

use image::RgbImage;

use super::match_end::{
    LETTERBOX_SHOULDER_BOT_Y_PM, LETTERBOX_SHOULDER_DARK_MAX, LETTERBOX_SHOULDER_H_PM,
    LETTERBOX_SHOULDER_TOP_Y_PM, dark_ratio, letterbox_mid_is_footage,
};

/// The middle band holds the lit strip: about 0.3 to 0.5 of it is dark on
/// the loading screen. A darker middle is a fade or a dark scene.
const MID_DARK_MAX: f32 = 0.65;

/// Edge bands at least this dark, as in the end-reel letterbox check.
const EDGE_DARK_MIN: f32 = 0.72;

/// A pixel is lit when its brightest channel reaches this (the dark test
/// of [`dark_ratio`] is below it).
const LIT_MIN: u8 = 36;

/// A lit pixel with every channel at least this is a white glyph of the
/// `ENTERING GAME` text and is left out of the row colour.
const GLYPH_MIN: u8 = 160;

/// A middle row counts when at least this share of its samples is lit
/// (the black padding around the strip is no row of it).
const ROW_LIT_MIN: f32 = 0.25;

/// A lit sample is the row's colour when each channel is within this of
/// the row's mean.
const ROW_TOLERANCE: i32 = 16;

/// A row is flat when at least this share of its lit samples is its
/// colour (glyph edges are the rest).
const ROW_FLAT_MIN: f32 = 0.80;

/// The middle is the loading strip when at least this share of its
/// counted rows is flat.
const FLAT_ROWS_MIN: f32 = 0.85;

/// The `ENTERING GAME` text, as shares of the 16:9 game area: one line of
/// white glyphs centred across the screen and centred down the strip at
/// half height, at most half the width (about 0.17 on real frames; no
/// minimum).
const TEXT_CENTRE_X_TOL: f32 = 0.05;
const TEXT_WIDTH_MAX: f32 = 0.50;
const TEXT_CENTRE_Y_MIN: f32 = 0.44;
const TEXT_CENTRE_Y_MAX: f32 = 0.56;
const TEXT_HEIGHT_MAX: f32 = 0.10;

/// Fewest glyph samples that make text (a few stray white pixels do not).
const TEXT_SAMPLES_MIN: u32 = 24;

/// Glyph columns of the text box at least this far apart (a share of the
/// game width, counted as the empty sampled columns between them) are in
/// different words; letters are closer.
const WORD_GAP: f32 = 0.0045;

/// A band of exactly two words whose second is at least this share of the
/// first's width is no `ENTERING GAME` (about 0.61) but `APPLYING UPDATE`
/// (about 0.74).
pub const WORD_RATIO_REJECT: f32 = 0.70;

/// An end cluster of three is the spinner, left out of the words, only
/// when it is at most this wide (a share of the game width) and this tall
/// (of its height), and neither side is more than [`SPINNER_ASPECT_MAX`]
/// times the other. The stand-in spinner (`two_word_band`) is 0.03 wide
/// and as tall as the text, 0.067 (0.8 as wide as tall in pixels), so
/// these allow about 1.5 times it; the shortest real word, `GAME` in the
/// 0.173 wide `ENTERING GAME` at 0.61, is about 0.061 wide, and the
/// second word of the 0.186 wide `APPLYING UPDATE` at 0.74 about 0.074.
const SPINNER_WIDTH_MAX: f32 = 0.045;
const SPINNER_HEIGHT_MAX: f32 = 0.10;
const SPINNER_ASPECT_MAX: f32 = 1.6;

/// Glyph samples this close across (a share of the game width) on one
/// row, or on rows this close down (a share of its height) with spans this
/// close across, are one piece of text: letters and words join, a blob
/// elsewhere on the band does not.
const TEXT_JOIN_X: f32 = 0.03;
const TEXT_JOIN_Y: f32 = 0.02;

/// A piece with fewer glyph samples than this share of the largest piece
/// is a stray blob (a cursor, a small overlay) and is left out of the text
/// box. A progress bar is not: it is as large as text.
const TEXT_PIECE_MIN: f32 = 0.15;

/// [`is_loading_screen`] and the `ENTERING GAME` text where it sits: at
/// least [`TEXT_SAMPLES_MIN`] glyph samples in the middle band, spanning
/// one line centred across the screen and down the strip, at most half its
/// width, and not exactly two words with the second at least
/// [`WORD_RATIO_REJECT`] of the first. A navy band with no text (the
/// fade-in), its text elsewhere or the `APPLYING UPDATE` words is not the
/// loading screen. Only the pre-game saves use it; [`is_loading_screen`]
/// is unchanged.
pub fn is_entering_game(rgb: &RgbImage) -> bool {
    is_loading_screen(rgb) && entering_game_text(rgb)
}

/// The white text of the middle band, as shares of the 16:9 game area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBox {
    pub width: f32,
    pub height: f32,
    pub centre_x: f32,
    pub centre_y: f32,
    /// Glyph samples in the box's pieces (every second pixel each way).
    pub samples: u32,
    /// Words in the box ([`WORD_GAP`]), after a spinner: of exactly three,
    /// the end one across the wider gap (the words are closer to each
    /// other than to it) is left out when it is spinner-sized
    /// ([`SPINNER_WIDTH_MAX`]); otherwise all three count.
    pub words: usize,
    /// The second word's width over the first's, with exactly two words.
    pub word_ratio: Option<f32>,
}

/// One run of glyph samples on a sampled row.
struct Run {
    y: u32,
    x0: u32,
    x1: u32,
    n: u32,
}

/// The box around the white glyphs of the middle band, from its pieces
/// of text with stray small blobs left out ([`TEXT_PIECE_MIN`]); `None`
/// with fewer than [`TEXT_SAMPLES_MIN`] glyph samples in them. What
/// [`is_entering_game`] measures, public for checks on real frames.
pub fn text_box(rgb: &RgbImage) -> Option<TextBox> {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::frame::geometry::game_rect_16_9(w, h);
    if gw == 0 || gh == 0 {
        return None;
    }
    let y0 = gy + gh * 300 / 1000;
    let y1 = (y0 + gh * 400 / 1000).min(h);
    let x1 = (gx + gw).min(w);
    let join_x = (gw as f32 * TEXT_JOIN_X) as u32;
    let join_y = (gh as f32 * TEXT_JOIN_Y) as u32;
    let mut runs: Vec<Run> = Vec::new();
    // Each glyph sample's column, row and the run it is in.
    let mut columns: Vec<(u32, u32, usize)> = Vec::new();
    for y in (y0..y1).step_by(2) {
        let mut open: Option<Run> = None;
        for x in (gx..x1).step_by(2) {
            let p = rgb.get_pixel(x, y).0;
            if p[0].min(p[1]).min(p[2]) < GLYPH_MIN {
                continue;
            }
            match open.as_mut() {
                Some(r) if x - r.x1 <= join_x => {
                    r.x1 = x;
                    r.n += 1;
                }
                _ => {
                    runs.extend(open.take());
                    open = Some(Run {
                        y,
                        x0: x,
                        x1: x,
                        n: 1,
                    });
                }
            }
            // The open run is pushed next, at this index.
            columns.push((x, y, runs.len()));
        }
        runs.extend(open);
    }
    // Join runs into pieces (union-find); runs are in row order.
    let mut parent: Vec<usize> = (0..runs.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..runs.len() {
        for j in i + 1..runs.len() {
            if runs[j].y - runs[i].y > join_y {
                break;
            }
            let near = runs[j].x0 <= runs[i].x1 + join_x && runs[i].x0 <= runs[j].x1 + join_x;
            if runs[j].y > runs[i].y && near {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    // Per piece: samples and box (left, right, top, bottom).
    let mut pieces: std::collections::HashMap<usize, (u32, u32, u32, u32, u32)> =
        std::collections::HashMap::new();
    for (i, r) in runs.iter().enumerate() {
        let p = pieces
            .entry(root(&mut parent, i))
            .or_insert((0, u32::MAX, 0, u32::MAX, 0));
        p.0 += r.n;
        p.1 = p.1.min(r.x0);
        p.2 = p.2.max(r.x1);
        p.3 = p.3.min(r.y);
        p.4 = p.4.max(r.y);
    }
    let largest = pieces.values().map(|p| p.0).max()?;
    let (mut samples, mut left, mut right, mut top, mut bottom) = (0u32, u32::MAX, 0, u32::MAX, 0);
    let kept: std::collections::HashSet<usize> = pieces
        .iter()
        .filter(|(_, p)| p.0 as f32 >= largest as f32 * TEXT_PIECE_MIN)
        .map(|(root, _)| *root)
        .collect();
    for p in kept.iter().map(|root| &pieces[root]) {
        samples += p.0;
        left = left.min(p.1);
        right = right.max(p.2);
        top = top.min(p.3);
        bottom = bottom.max(p.4);
    }
    if samples < TEXT_SAMPLES_MIN {
        return None;
    }
    let mut lit: Vec<(u32, u32)> = columns
        .iter()
        .filter(|&&(_, _, run)| kept.contains(&root(&mut parent, run)))
        .map(|&(x, y, _)| (x, y))
        .collect();
    lit.sort_unstable();
    let words = words(&lit, gw, gh);
    let word_ratio = match words[..] {
        [(a0, a1), (b0, b1)] if a1 > a0 => Some((b1 - b0) as f32 / (a1 - a0) as f32),
        _ => None,
    };
    let share_x = |x: u32| (x - gx) as f32 / gw as f32;
    let share_y = |y: u32| (y - gy) as f32 / gh as f32;
    Some(TextBox {
        width: (right - left) as f32 / gw as f32,
        height: (bottom - top) as f32 / gh as f32,
        centre_x: (share_x(left) + share_x(right)) / 2.0,
        centre_y: (share_y(top) + share_y(bottom)) / 2.0,
        samples,
        words: words.len(),
        word_ratio,
    })
}

/// The words (first and last column) of the glyph samples `lit` (column,
/// row; sorted), sampled every second pixel in a `gw` x `gh` game area: a
/// new word starts where the empty sampled columns between two lit ones
/// span at least [`WORD_GAP`] of the width. Of exactly three, the end one
/// across the wider gap is the spinner and is left out when it is
/// spinner-sized ([`SPINNER_WIDTH_MAX`], [`SPINNER_HEIGHT_MAX`],
/// [`SPINNER_ASPECT_MAX`]); otherwise all three are kept, since a stray gap
/// in a word can leave a letter or two at the other end as small.
fn words(lit: &[(u32, u32)], gw: u32, gh: u32) -> Vec<(u32, u32)> {
    let gap = gw as f32 * WORD_GAP;
    // First and last column, top and bottom row.
    let mut words: Vec<(u32, u32, u32, u32)> = Vec::new();
    for &(x, y) in lit {
        match words.last_mut() {
            Some((_, end, top, bottom)) if ((x - *end).saturating_sub(2) as f32) < gap => {
                *end = x;
                *top = (*top).min(y);
                *bottom = (*bottom).max(y);
            }
            _ => words.push((x, x, y, y)),
        }
    }
    if let [(_, a1, ..), (b0, b1, ..), (c0, ..)] = words[..] {
        let spinner = |(x0, x1, y0, y1): (u32, u32, u32, u32)| {
            let (w, h) = ((x1 - x0) as f32, (y1 - y0) as f32);
            w <= gw as f32 * SPINNER_WIDTH_MAX
                && h <= gh as f32 * SPINNER_HEIGHT_MAX
                && w <= h * SPINNER_ASPECT_MAX
                && h <= w * SPINNER_ASPECT_MAX
        };
        // The end across the wider gap: the words are closer to each other
        // than to the spinner.
        let end = if b0 - a1 > c0 - b1 { 0 } else { 2 };
        if spinner(words[end]) {
            words.remove(end);
        }
    }
    words.into_iter().map(|(x0, x1, ..)| (x0, x1)).collect()
}

/// Whether the white glyphs of the middle band sit as the `ENTERING GAME`
/// text does, and are not the two `APPLYING UPDATE` words.
fn entering_game_text(rgb: &RgbImage) -> bool {
    text_box(rgb).is_some_and(|t| {
        (t.centre_x - 0.5).abs() <= TEXT_CENTRE_X_TOL
            && t.width <= TEXT_WIDTH_MAX
            && !t.word_ratio.is_some_and(|r| r >= WORD_RATIO_REJECT)
            && (TEXT_CENTRE_Y_MIN..=TEXT_CENTRE_Y_MAX).contains(&t.centre_y)
            && t.height <= TEXT_HEIGHT_MAX
    })
}

/// Whether `rgb` looks like the loading screen.
pub fn is_loading_screen(rgb: &RgbImage) -> bool {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::frame::geometry::game_rect_16_9(w, h);
    let band = (gh * 70 / 1000).max(1);
    if band * 2 >= gh {
        return false;
    }
    let top_dark = dark_ratio(rgb, gx, gy, gw, band);
    let bot_dark = dark_ratio(rgb, gx, gy + gh - band, gw, band);
    let mid_y = gy + gh * 300 / 1000;
    let mid_h = gh * 400 / 1000;
    let mid_dark = dark_ratio(rgb, gx, mid_y, gw, mid_h);
    if top_dark < EDGE_DARK_MIN || bot_dark < EDGE_DARK_MIN || mid_dark >= MID_DARK_MAX {
        return false;
    }
    shoulders_dark(rgb, gx, gy, gw, gh)
        && (rows_are_flat(rgb, gx, mid_y, gw, mid_h)
            || !letterbox_mid_is_footage(rgb, gx, mid_y, gw, mid_h))
}

/// Whether the lit rows of the band are each about one navy colour (white
/// glyphs left out). Rows may differ from each other: the strip is a
/// gradient. The row colour must be blue-dominant (b > r and b > g), so a
/// dim or faded grey or warm band, which the tolerance alone would call
/// flat, does not pass.
fn rows_are_flat(rgb: &RgbImage, x0: u32, y0: u32, cw: u32, ch: u32) -> bool {
    let (w, h) = rgb.dimensions();
    let x1 = (x0 + cw).min(w);
    let y1 = (y0 + ch).min(h);
    let mut lit: Vec<[u8; 3]> = Vec::new();
    let (mut counted, mut flat) = (0u32, 0u32);
    for y in (y0..y1).step_by(2) {
        lit.clear();
        let mut samples = 0u32;
        for x in (x0..x1).step_by(2) {
            samples += 1;
            let p = rgb.get_pixel(x, y).0;
            let max = p[0].max(p[1]).max(p[2]);
            let min = p[0].min(p[1]).min(p[2]);
            if max >= LIT_MIN && min < GLYPH_MIN {
                lit.push(p);
            }
        }
        if samples == 0 || (lit.len() as f32) < samples as f32 * ROW_LIT_MIN {
            continue;
        }
        counted += 1;
        let n = lit.len() as i64;
        let mean: [i32; 3] =
            std::array::from_fn(|c| (lit.iter().map(|p| i64::from(p[c])).sum::<i64>() / n) as i32);
        let near = lit
            .iter()
            .filter(|p| (0..3).all(|c| (i32::from(p[c]) - mean[c]).abs() <= ROW_TOLERANCE))
            .count();
        let navy = mean[2] > mean[0] && mean[2] > mean[1];
        if navy && near as f32 >= lit.len() as f32 * ROW_FLAT_MIN {
            flat += 1;
        }
    }
    counted > 0 && flat as f32 >= counted as f32 * FLAT_ROWS_MIN
}

/// The bands just inside the edge bands carry gameplay on a highlight reel
/// and are both black on the loading screen.
fn shoulders_dark(rgb: &RgbImage, gx: u32, gy: u32, gw: u32, gh: u32) -> bool {
    let sh = (gh * LETTERBOX_SHOULDER_H_PM / 1000).max(1);
    let top_y = gy + gh * LETTERBOX_SHOULDER_TOP_Y_PM / 1000;
    let bot_y = gy + gh * LETTERBOX_SHOULDER_BOT_Y_PM / 1000;
    dark_ratio(rgb, gx, top_y, gw, sh) >= LETTERBOX_SHOULDER_DARK_MAX
        && dark_ratio(rgb, gx, bot_y, gw, sh) >= LETTERBOX_SHOULDER_DARK_MAX
}

#[cfg(test)]
mod tests {
    use image::Rgb;

    use super::*;

    use crate::detect::tests_loading_stand_ins::*;

    #[test]
    fn an_applying_update_band_is_not_entering_game() {
        // Off-centre text with a bar, off-centre text alone, centred text
        // over a wide bar: each passes the plain loading check, and each
        // fails the text place or width.
        for (centred, bar) in [(false, true), (false, false), (true, true)] {
            let band = applying_update_band(640, 360, centred, bar);
            assert!(is_loading_screen(&band), "centred {centred}, bar {bar}");
            assert!(!is_entering_game(&band), "centred {centred}, bar {bar}");
        }
    }

    #[test]
    fn the_entering_game_slab_is_entering_game() {
        let slab = entering_game_slab(640, 360);
        assert!(is_loading_screen(&slab));
        assert!(is_entering_game(&slab));
        // The other ENTERING GAME stand-ins here pass the stricter check too.
        assert!(is_entering_game(&entering_game()));
        assert!(is_entering_game(&entering_game_frame()));
        assert!(is_entering_game(&navy_gradient()));
        // The same layout drawn at each size (not a rescaled 640x360).
        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
            assert!(is_entering_game(&entering_game_slab(w, h)), "{w}x{h}");
            for (centred, bar) in [(false, true), (false, false), (true, true)] {
                let band = applying_update_band(w, h, centred, bar);
                assert!(!is_entering_game(&band), "{w}x{h} {centred} {bar}");
            }
        }
    }

    #[test]
    fn a_stray_blob_on_the_band_leaves_the_text_box_alone() {
        // A cursor-sized white blob on the strip, far right of the text,
        // and one in the black bar above it: left out of the box, so the
        // slab still passes, at each size.
        for (w, h) in [(640, 360), (1920, 1080), (2560, 1440)] {
            let mut slab = entering_game_slab(w, h);
            let clean = text_box(&slab).expect("text");
            block(&mut slab, 590..596, 140..146);
            block(&mut slab, 60..66, 116..122);
            assert_eq!(text_box(&slab), Some(clean), "{w}x{h}");
            assert!(is_entering_game(&slab), "{w}x{h}");
        }
        // A bar as large as the text is no stray blob: it widens the box
        // past half the width.
        let mut slab = entering_game_slab(640, 360);
        block(&mut slab, 60..580, 200..204);
        assert!(text_box(&slab).expect("text").width > TEXT_WIDTH_MAX);
        assert!(!is_entering_game(&slab));
    }

    #[test]
    fn the_word_ratio_rejects_only_exactly_two_words_at_seventy_percent() {
        // ENTERING GAME (0.61) passes, APPLYING UPDATE (0.74) fails, and a
        // pair just below and just above 0.70, at each size.
        let cases = [
            (ENTERING_WORDS, true),
            (UPDATE_WORDS, false),
            ((0.18, 0.68), true),
            ((0.18, 0.72), false),
        ];
        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
            for (words, pass) in cases {
                let band = two_word_band(w, h, words, None);
                let t = text_box(&band).expect("text");
                assert_eq!(t.words, 2, "{w}x{h} {words:?}: {t:?}");
                let ratio = t.word_ratio.expect("two words");
                assert!((ratio - words.1).abs() < 0.015, "{w}x{h} {words:?}: {t:?}");
                assert!(is_loading_screen(&band));
                assert_eq!(is_entering_game(&band), pass, "{w}x{h} {words:?}: {t:?}");
            }
        }
    }

    #[test]
    fn a_spinner_beside_the_words_is_left_out_of_the_ratio() {
        // Two words plus the stand-in spinner on either side, at each size:
        // judged on the two words, so 0.61 and 0.68 pass, 0.72 and 0.74 fail.
        let cases = [
            (ENTERING_WORDS, true),
            (UPDATE_WORDS, false),
            ((0.18, 0.68), true),
            ((0.18, 0.72), false),
        ];
        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
            for spinner in [Some(true), Some(false)] {
                for (words, pass) in cases {
                    let band = two_word_band(w, h, words, spinner);
                    let t = text_box(&band).expect("text");
                    let at = format!("{w}x{h} {spinner:?} {words:?}: {t:?}");
                    assert_eq!(t.words, 2, "{at}");
                    assert!((t.word_ratio.unwrap() - words.1).abs() < 0.015, "{at}");
                    assert_eq!(is_entering_game(&band), pass, "{at}");
                }
            }
        }
    }

    /// The stand-ins' text height (rows 168 to 192 of 360).
    const TEXT_H: f32 = 24.0 / 360.0;

    /// The navy band with solid white parts `(gap before, width, height)`,
    /// shares of the frame, left to right: the line centred across it, each
    /// part centred down it.
    fn parts_band(w: u32, h: u32, parts: &[(f32, f32, f32)]) -> RgbImage {
        let mut img = navy_band(w, h);
        let line: f32 = parts.iter().map(|p| p.0 + p.1).sum();
        let mut x0 = 0.5 - line / 2.0;
        for &(gap, width, height) in parts {
            x0 += gap;
            let (a, b) = ((x0 * w as f32) as u32, ((x0 + width) * w as f32) as u32);
            let (c, d) = (
                ((0.5 - height / 2.0) * h as f32) as u32,
                ((0.5 + height / 2.0) * h as f32) as u32,
            );
            for y in c..d {
                for x in a..b {
                    img.put_pixel(x, y, Rgb(GLYPH));
                }
            }
            x0 += width;
        }
        img
    }

    #[test]
    fn entering_game_split_into_three_with_no_spinner_passes() {
        // ENTERING GAME 0.173 wide (0.10 and 0.061, 0.012 apart) with a
        // stray 0.008 gap splitting ENTERING into 0.05 and 0.042: three
        // text-sized clusters, no spinner. Dropping GAME (the end across
        // the wider gap) would judge 0.042 over 0.05 and reject it.
        let parts = [
            (0.0, 0.05, TEXT_H),
            (0.008, 0.042, TEXT_H),
            (0.012, 0.061, TEXT_H),
        ];
        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
            let band = parts_band(w, h, &parts);
            let t = text_box(&band).expect("text");
            assert_eq!((t.words, t.word_ratio), (3, None), "{w}x{h}: {t:?}");
            assert!((t.width - 0.173).abs() < 0.01, "{w}x{h}: {t:?}");
            assert!(is_loading_screen(&band), "{w}x{h}");
            assert!(is_entering_game(&band), "{w}x{h}: {t:?}");
        }
    }

    #[test]
    fn a_spinner_sized_end_across_the_narrower_gap_is_kept() {
        // ENTERING GAME with a stray 0.008 gap leaving EN (0.03 wide, as
        // the stand-in spinner) at the left: GAME, across the wider gap,
        // is no spinner, so the three count and the band passes. Dropping
        // EN would judge GAME over TERING (about 0.98) and reject it.
        let parts = [
            (0.0, 0.03, TEXT_H),
            (0.008, 0.062, TEXT_H),
            (0.012, 0.061, TEXT_H),
        ];
        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
            let band = parts_band(w, h, &parts);
            let t = text_box(&band).expect("text");
            assert_eq!((t.words, t.word_ratio), (3, None), "{w}x{h}: {t:?}");
            assert!(is_entering_game(&band), "{w}x{h}: {t:?}");
        }
    }

    #[test]
    fn only_a_spinner_sized_end_cluster_is_left_out() {
        // APPLYING UPDATE (0.10 and 0.074, 0.012 apart) with a block 0.025
        // to the right, just inside and just outside each bound at
        // 1920x1080 (`(width, height)` shares): inside it is the spinner and
        // the words are judged (rejected), outside it is a third word.
        let update = [(0.0, 0.10, TEXT_H), (0.012, 0.074, TEXT_H)];
        // In pairs: 0.045 wide at most, 0.10 tall at most, at most 1.6 as
        // tall as wide, at most 1.6 as wide as tall.
        let cases = [
            ((0.043, TEXT_H), true),
            ((0.048, TEXT_H), false),
            ((0.042, 0.097), true),
            ((0.042, 0.104), false),
            ((0.03, 0.082), true),
            ((0.03, 0.090), false),
            ((0.032, 0.04), true),
            ((0.040, 0.04), false),
        ];
        for ((sw, sh), spinner) in cases {
            let band = parts_band(1920, 1080, &[update[0], update[1], (0.025, sw, sh)]);
            let t = text_box(&band).expect("text");
            assert_eq!(t.words, if spinner { 2 } else { 3 }, "{sw} {sh}: {t:?}");
            if sh <= TEXT_HEIGHT_MAX {
                assert_eq!(is_entering_game(&band), !spinner, "{sw} {sh}: {t:?}");
            }
        }
        // The bounds in sampled pixels, 1920x1080: 86.4 wide, 108 tall,
        // each side at most 1.6 times the other.
        let cluster = |x0: u32, x1: u32, y0: u32, y1: u32| {
            (x0..=x1).step_by(2).flat_map(move |x| [(x, y0), (x, y1)])
        };
        let count = |sw: u32, sh: u32| {
            let mut lit: Vec<(u32, u32)> = cluster(700, 900, 500, 572)
                .chain(cluster(912, 1060, 500, 572))
                .chain(cluster(1090, 1090 + sw, 536 - sh / 2, 536 + sh / 2))
                .collect();
            lit.sort_unstable();
            words(&lit, 1920, 1080).len()
        };
        for ((sw, sh), n) in [
            ((86, 72), 2),
            ((88, 72), 3),
            ((80, 108), 2),
            ((80, 110), 3),
            ((60, 96), 2),
            ((60, 98), 3),
            ((80, 50), 2),
            ((82, 50), 3),
        ] {
            assert_eq!(count(sw, sh), n, "{sw} x {sh}");
        }
    }

    #[test]
    fn a_band_that_splits_into_one_or_many_words_is_not_judged_by_ratio() {
        // One solid word, and strokes that split into many: no ratio.
        let mut one = navy_band(1920, 1080);
        block(&mut one, 270..370, 168..192);
        let t = text_box(&one).expect("text");
        assert_eq!((t.words, t.word_ratio), (1, None));
        assert!(is_entering_game(&one));
        let many = centred_text_band(1920, 1080, 0.186);
        let t = text_box(&many).expect("text");
        assert!(t.words > 3 && t.word_ratio.is_none(), "{t:?}");
        assert!(is_entering_game(&many));
    }

    #[test]
    fn the_band_must_be_centred_one_line_and_at_most_half_the_width() {
        let pass = |xs: std::ops::Range<u32>, ys: std::ops::Range<u32>| {
            let mut band = navy_band(640, 360);
            glyphs(&mut band, xs, ys);
            is_entering_game(&band)
        };
        assert!(pass(266..374, 168..192), "centred, 0.17 wide");
        assert!(!pass(120..520, 168..192), "0.62 wide");
        assert!(!pass(304..412, 168..192), "off centre by 0.07");
        assert!(!pass(266..374, 150..192), "0.12 tall");
        assert!(!pass(266..374, 136..150), "centred at 0.39 down");
        assert!(!pass(266..374, 210..222), "centred at 0.6 down");
    }

    #[test]
    fn centred_text_as_wide_as_on_real_frames_passes() {
        // 0.17 to 0.186 wide, in strokes that do not split into two words:
        // passes at each size.
        for (w, h) in [(640, 360), (1280, 720), (1920, 1080), (2560, 1440)] {
            for share in [0.17, 0.173, 0.186] {
                let band = centred_text_band(w, h, share);
                let t = text_box(&band).expect("text");
                assert!((t.width - share).abs() < 0.02, "{w}x{h} {share}: {t:?}");
                assert!(is_entering_game(&band), "{w}x{h} {share}: {t:?}");
            }
        }
    }

    #[test]
    fn a_fade_with_no_or_faint_text_fails_the_glyph_floor() {
        // The fade-in: the navy strip with no text yet, or text not yet
        // white, or a few white specks below the glyph floor.
        for (w, h) in [(640, 360), (1920, 1080)] {
            let strip = navy_band(w, h);
            assert!(is_loading_screen(&strip), "{w}x{h}");
            assert!(text_box(&strip).is_none(), "{w}x{h}");
            assert!(!is_entering_game(&strip), "{w}x{h}");
            let mut faint = centred_text_band(w, h, 0.17);
            for p in faint.pixels_mut() {
                if p.0 == GLYPH {
                    p.0 = [120, 122, 130];
                }
            }
            assert!(!is_entering_game(&faint), "{w}x{h} faint");
        }
        // The glyph floor: 23 samples fail, 24 pass (one row of white
        // pixels on the sampled grid, every second pixel).
        for (n, pass) in [(23, false), (24, true)] {
            let mut row = navy_band(640, 360);
            for k in 0..n {
                row.put_pixel(298 + 2 * k, 180, Rgb(GLYPH));
            }
            // Below the floor there is no text box at all.
            assert_eq!(text_box(&row).map(|t| t.samples), pass.then_some(n));
            assert_eq!(is_entering_game(&row), pass, "{n} samples");
        }
    }

    #[test]
    fn a_navy_band_with_no_text_is_not_entering_game() {
        let mut slab = entering_game_slab(640, 360);
        for p in slab.pixels_mut() {
            if p.0 == GLYPH {
                p.0 = [30, 36, 60];
            }
        }
        assert!(!is_entering_game(&slab));
    }

    /// Black bars and a navy strip with white glyphs, like `ENTERING GAME`.
    fn entering_game() -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 136..224 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([24, 32, 52]));
            }
        }
        for y in 168..192 {
            for x in (200..440).filter(|x| x % 7 < 3) {
                img.put_pixel(x, y, Rgb([240, 240, 240]));
            }
        }
        img
    }

    /// Gameplay-like noise over the whole frame.
    fn footage() -> RgbImage {
        RgbImage::from_fn(640, 360, |x, y| {
            Rgb([
                (x * 7 + y * 3) as u8,
                ((x * 13) ^ (y * 5)) as u8,
                (y * 11 + x) as u8,
            ])
        })
    }

    /// `entering_game_frame` from the end-reel tests (`tests_match_end.rs`),
    /// built from the real frames 013904 / 223050: ~38% black bars and a
    /// ~24% navy strip that is a vertical gradient, white glyphs, and a thin
    /// HUD line in the top band.
    fn entering_game_frame() -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 136..224 {
            let t = (y - 136) as u8;
            let navy = Rgb([20 + t / 4, 28 + t / 5, 42 + t / 3]);
            for x in 0..640 {
                img.put_pixel(x, y, navy);
            }
        }
        for y in 168..192 {
            for x in 200..440 {
                if x % 7 < 3 {
                    img.put_pixel(x, y, Rgb([240, 240, 240]));
                }
            }
        }
        for x in 8..400 {
            img.put_pixel(x, 4, Rgb([220, 220, 220]));
        }
        img
    }

    /// A taller strip whose navy drifts from dark to bright down the
    /// screen, with glyphs.
    fn navy_gradient() -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 120..240 {
            let t = (y - 120) as u8;
            let navy = Rgb([12 + t / 4, 20 + t / 3, 40 + t / 2]);
            for x in 0..640 {
                img.put_pixel(x, y, navy);
            }
        }
        for y in 170..190 {
            for x in (220..420).filter(|x| x % 5 < 2) {
                img.put_pixel(x, y, Rgb([235, 238, 245]));
            }
        }
        img
    }

    #[test]
    fn the_entering_game_strip_is_the_loading_screen() {
        assert!(is_loading_screen(&entering_game()));
    }

    #[test]
    fn the_real_entering_game_gradient_is_the_loading_screen() {
        assert!(is_loading_screen(&entering_game_frame()));
    }

    #[test]
    fn a_vertical_navy_gradient_is_the_loading_screen() {
        assert!(is_loading_screen(&navy_gradient()));
    }

    #[test]
    fn a_flat_middle_with_a_grey_share_is_still_the_loading_screen() {
        // 84% navy and 16% light grey per sampled row: the grey pulls the
        // row mean more than 16 off the navy, so the per-row test fails,
        // and the whole-middle flat-UI test (one bin holds 84%) passes.
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 136..224 {
            for x in 0..640u32 {
                let p = if x % 50 < 7 {
                    Rgb([150, 150, 150])
                } else {
                    Rgb([24, 32, 52])
                };
                img.put_pixel(x, y, p);
            }
        }
        // The middle band of a 640x360 frame is y 108 for 144 rows.
        assert!(!rows_are_flat(&img, 0, 108, 640, 144));
        assert!(is_loading_screen(&img));
    }

    #[test]
    fn a_dim_faded_grey_or_warm_band_is_not() {
        // A fade: each row a soft horizontal ramp (28 levels), within the
        // per-row tolerance but across several colour bins, grey or warm
        // and not blue: only the blue-dominant rule turns it away.
        for (dr, dg) in [(0u32, 0u32), (20, 8)] {
            let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
            for y in 136..224u32 {
                for x in 0..640u32 {
                    let v = 40 + x * 28 / 640;
                    img.put_pixel(x, y, Rgb([(v + dr) as u8, (v + dg) as u8, v as u8]));
                }
            }
            let mut blue = img.clone();
            for p in blue.pixels_mut() {
                if p.0 != [0, 0, 0] {
                    let v = p.0[2];
                    p.0 = [v, v + 4, v + 24];
                }
            }
            assert!(!is_loading_screen(&img), "tint +{dr},+{dg}");
            // The same band turned blue is flat per row: the colour, not
            // the ramp, is what failed.
            assert!(rows_are_flat(&blue, 0, 108, 640, 144), "tint +{dr},+{dg}");
        }
        // The real gradient still passes through the per-row test alone.
        assert!(rows_are_flat(&entering_game_frame(), 0, 108, 640, 144));
        assert!(rows_are_flat(&navy_gradient(), 0, 108, 640, 144));
    }

    #[test]
    fn a_strip_of_noisy_rows_is_not() {
        // The loading shape (black bars and shoulders) around footage.
        let mut img = entering_game();
        for y in 136..224u32 {
            for x in 0..640u32 {
                let n = ((x.wrapping_mul(37) ^ y.wrapping_mul(17)) % 90) as u8;
                img.put_pixel(x, y, Rgb([40 + n, 90 - n / 2, 30 + n / 3]));
            }
        }
        assert!(!is_loading_screen(&img));
    }

    /// 7% edge bands black, as in the end-reel tests.
    fn bars(img: &mut RgbImage) {
        for y in 0..30 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([8, 8, 8]));
                img.put_pixel(x, 359 - y, Rgb([8, 8, 8]));
            }
        }
    }

    /// A cinematic letterbox over a flat mid colour (the end-reel tests'
    /// flat frame): the shoulders are lit.
    fn letterbox_flat() -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([40, 60, 90]));
        bars(&mut img);
        img
    }

    /// A letterboxed highlight reel (the end-reel tests' footage frame).
    fn letterbox_footage() -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([80, 90, 100]));
        for y in 30u32..330 {
            for x in 0u32..640 {
                let n = ((x.wrapping_mul(37) ^ y.wrapping_mul(17)) % 50) as u8;
                let pix = if (x / 80) % 3 == 0 {
                    Rgb([110 + n / 2, 70 + n / 3, 50 + n / 4])
                } else {
                    Rgb([50 + n / 3, 90 + n / 2, 80 + n / 3])
                };
                img.put_pixel(x, y, pix);
            }
        }
        bars(&mut img);
        img
    }

    #[test]
    fn letterboxed_frames_with_lit_shoulders_are_not() {
        assert!(!is_loading_screen(&letterbox_flat()));
        assert!(!is_loading_screen(&letterbox_footage()));
    }

    #[test]
    fn one_dark_shoulder_is_not_enough() {
        let mut img = entering_game();
        // Light the bottom shoulder (y 77% to 85%) only.
        for y in 277..306 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([90, 110, 70]));
            }
        }
        assert!(!is_loading_screen(&img));
    }

    #[test]
    fn a_middle_three_quarters_dark_is_not() {
        // A 36-row strip in the 144-row middle band: 0.75 dark, under the
        // old 0.85 cap and over the 0.65 one.
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 162..198 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([24, 32, 52]));
            }
        }
        for y in 174..186 {
            for x in (200..440).filter(|x| x % 7 < 3) {
                img.put_pixel(x, y, Rgb([240, 240, 240]));
            }
        }
        assert!(!is_loading_screen(&img));
    }

    #[test]
    fn a_mostly_dark_middle_is_not() {
        // A thin lit line in a black frame: a fade or a dark scene.
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 176..184 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([24, 32, 52]));
            }
        }
        assert!(!is_loading_screen(&img));
    }

    #[test]
    fn footage_and_black_are_not() {
        assert!(!is_loading_screen(&footage()));
        assert!(!is_loading_screen(&RgbImage::from_pixel(
            640,
            360,
            Rgb([0, 0, 0])
        )));
    }
}
