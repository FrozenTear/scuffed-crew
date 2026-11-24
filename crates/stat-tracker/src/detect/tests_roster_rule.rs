//! Tests for the roster rule in `roster_screen.rs`. Synthetic frames only,
//! drawn in code by shares of the frame (all 16:9, so the shares are the
//! game area's).

use super::*;
use image::Rgb;

/// A dim map backdrop, low chroma and far from near-white.
const BACKDROP: [u8; 3] = [60, 70, 90];
/// The header bars: lowest channel 246, which sets the fade limit.
const BAR: [u8; 3] = [247, 246, 246];
/// Dark label text on the bars.
const LABEL: [u8; 3] = [30, 30, 40];
/// The player rows' bodies, low chroma, so only the strips are coloured.
const ROW_LEFT: [u8; 3] = [52, 56, 66];
const ROW_RIGHT: [u8; 3] = [58, 60, 70];
/// The role strips: chroma 123 and 251.
const STRIP_L: [u8; 3] = [107, 22, 145];
const STRIP_R: [u8; 3] = [255, 210, 4];
/// White text: player names, glyphs, labels.
const NAME: [u8; 3] = [235, 235, 235];

/// Header (top, bottom) as shares of the height.
const FIVE: (f32, f32) = (0.236, 0.278);
const SIX: (f32, f32) = (0.19, 0.235);
/// Where the columns end, for any team size.
const COLUMNS_END: f32 = 0.756;

const SIZES: [(u32, u32); 2] = [(1920, 1080), (2560, 1440)];
const LAYOUTS: [((f32, f32), u32); 2] = [(FIVE, 5), (SIX, 6)];

/// Fill the box (x0, y0, x1, y1), as shares of the frame, with `c`.
fn fill(img: &mut RgbImage, b: [f32; 4], c: [u8; 3]) {
    let (w, h) = img.dimensions();
    let (x0, y0) = ((b[0] * w as f32) as u32, (b[1] * h as f32) as u32);
    let (x1, y1) = ((b[2] * w as f32) as u32, (b[3] * h as f32) as u32);
    for y in y0..y1.min(h) {
        for x in x0..x1.min(w) {
            img.put_pixel(x, y, Rgb(c));
        }
    }
}

/// The white team header bars with their labels: the bars reach past the
/// sampled spans into the gap, as the real ones do.
fn bars(img: &mut RgbImage, header: (f32, f32), left: bool, right: bool) {
    let (top, bottom) = header;
    let (t0, t1) = (top + 0.3 * (bottom - top), top + 0.75 * (bottom - top));
    if left {
        fill(img, [0.0676, top, 0.4059, bottom], BAR);
        fill(img, [0.09, t0, 0.12, t1], LABEL);
        fill(img, [0.36, t0, 0.40, t1], [89, 3, 93]);
    }
    if right {
        fill(img, [0.5938, top, 0.9324, bottom], BAR);
        fill(img, [0.60, t0, 0.64, t1], [184, 151, 1]);
        fill(img, [0.66, t0, 0.69, t1], LABEL);
    }
}

/// The two columns of `rows` player rows under a header ending at
/// `header_bottom`, sharing one span (so 6 rows are thinner than 5), with
/// white names and, if `strips`, the coloured role strips on the inner
/// edges.
fn columns(img: &mut RgbImage, header_bottom: f32, rows: u32, strips: bool) {
    let top = header_bottom + 0.006;
    let pitch = (COLUMNS_END - top) / rows as f32;
    for r in 0..rows {
        let y0 = top + pitch * r as f32;
        let y1 = y0 + pitch - 0.004;
        fill(img, [0.0676, y0, 0.4059, y1], ROW_LEFT);
        fill(img, [0.5938, y0, 0.9324, y1], ROW_RIGHT);
        if strips {
            fill(img, [0.3801, y0, 0.4059, y1], STRIP_L);
            fill(img, [0.5938, y0, 0.6195, y1], STRIP_R);
        }
        let (n0, n1) = (y0 + pitch * 0.35, y0 + pitch * 0.55);
        fill(img, [0.12, n0, 0.22, n1], NAME);
        fill(img, [0.70, n0, 0.80, n1], NAME);
    }
}

/// A full roster frame.
fn roster(w: u32, h: u32, header: (f32, f32), rows: u32) -> RgbImage {
    let mut img = RgbImage::from_pixel(w, h, Rgb(BACKDROP));
    bars(&mut img, header, true, true);
    columns(&mut img, header.1, rows, true);
    img
}

/// Every channel scaled by `f`, as a fade-in frame.
fn fade(img: &RgbImage, f: f32) -> RgbImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for c in p.0.iter_mut() {
            *c = (f32::from(*c) * f).round() as u8;
        }
    }
    out
}

/// A small deterministic pseudo random source.
struct Lcg(u64);

impl Lcg {
    fn step(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
}

/// Gameplay-like noise: coloured noise everywhere and white specks.
fn noise(w: u32, h: u32, seed: u64) -> RgbImage {
    let mut rng = Lcg(seed);
    let mut img = RgbImage::from_fn(w, h, |_, _| {
        let v = rng.step();
        Rgb([v as u8, (v >> 8) as u8, (v >> 16) as u8])
    });
    for _ in 0..600 {
        let (x, y) = (rng.step() % (w - 4), rng.step() % (h - 4));
        for dy in 0..4 {
            for dx in 0..4 {
                img.put_pixel(x + dx, y + dy, Rgb([250, 250, 250]));
            }
        }
    }
    img
}

#[test]
fn five_and_six_player_rosters_match_at_both_sizes() {
    for (w, h) in SIZES {
        for (header, rows) in LAYOUTS {
            let s = scores(&roster(w, h, header, rows));
            let at = format!("{w}x{h} {rows} rows");
            assert!(s.header_passes(), "{at}: {}", s.describe());
            assert!(s.strips_pass(), "{at}: {}", s.describe());
            assert!(s.is_roster(), "{at}: {}", s.describe());
            assert!(!s.near_miss(), "{at}: {}", s.describe());
            assert!(s.describe().ends_with("roster true"), "{at}");
        }
    }
}

#[test]
fn the_bans_label_above_a_5v5_header_does_not_spoil_it() {
    let mut img = roster(2560, 1440, FIVE, 5);
    fill(&mut img, [0.6668, 0.1785, 0.7605, 0.2042], NAME);
    assert!(is_roster_screen(&img), "{}", scores(&img).describe());
}

#[test]
fn a_downscaled_roster_matches_too() {
    use image::imageops::FilterType;
    for (header, rows) in LAYOUTS {
        let big = roster(1920, 1080, header, rows);
        let small = image::imageops::resize(&big, 1280, 720, FilterType::Triangle);
        assert!(
            is_roster_screen(&small),
            "{rows} rows: {}",
            scores(&small).describe()
        );
    }
}

#[test]
fn a_fading_roster_matches_down_to_about_0_61() {
    // The bars' lowest channel 246 scaled by f stays near-white while
    // 246 f >= 150: 0.7 gives 172, 0.55 gives 135. The strips keep their
    // chroma past that (123 f > 55 down to about 0.45).
    for (w, h) in SIZES {
        for (header, rows) in LAYOUTS {
            let full = roster(w, h, header, rows);
            for f in [1.0, 0.9, 0.8, 0.7] {
                let s = scores(&fade(&full, f));
                let at = format!("{w}x{h} {rows} rows at {f}");
                assert!(s.is_roster(), "{at}: {}", s.describe());
            }
            for f in [0.55, 0.5, 0.3] {
                let s = scores(&fade(&full, f));
                let at = format!("{w}x{h} {rows} rows at {f}");
                assert!(!s.header_passes(), "{at}: {}", s.describe());
                assert!(!s.is_roster(), "{at}: {}", s.describe());
            }
        }
    }
}

#[test]
fn gameplay_noise_is_no_roster() {
    for (seed, (w, h)) in [(1, (1920, 1080)), (7, (2560, 1440))] {
        let s = scores(&noise(w, h, seed));
        assert!(!s.is_roster(), "seed {seed}: {}", s.describe());
        assert!(!s.header_passes(), "seed {seed}: {}", s.describe());
    }
}

#[test]
fn a_bright_sky_edge_in_the_header_rows_is_no_roster() {
    // A bright sky ending inside the search rows leaves a band of the
    // right height at the top of the search; the area above it is white.
    let mut img = noise(1920, 1080, 3);
    fill(&mut img, [0.0, 0.0, 1.0, 0.20], [190, 200, 210]);
    let s = scores(&img);
    assert!(!s.header_passes(), "{}", s.describe());
    assert!(!s.is_roster(), "{}", s.describe());
}

#[test]
fn one_white_bar_is_no_roster() {
    for (header, rows) in LAYOUTS {
        // The left bar only.
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb(BACKDROP));
        bars(&mut img, header, true, false);
        columns(&mut img, header.1, rows, true);
        let s = scores(&img);
        assert!(!s.header_passes() && !s.is_roster(), "{}", s.describe());
        assert!(s.near_miss(), "{}", s.describe());
        // One bar across the whole width: no gap, no clean outside.
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb(BACKDROP));
        fill(&mut img, [0.0, header.0, 1.0, header.1], BAR);
        columns(&mut img, header.1, rows, true);
        let s = scores(&img);
        assert!(!s.header_passes() && !s.is_roster(), "{}", s.describe());
        assert!(s.strips_pass() && s.near_miss(), "{}", s.describe());
    }
}

#[test]
fn a_header_with_no_colour_strips_is_a_near_miss() {
    for (header, rows) in LAYOUTS {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb(BACKDROP));
        bars(&mut img, header, true, true);
        columns(&mut img, header.1, rows, false);
        let s = scores(&img);
        assert!(s.header_passes() && !s.strips_pass(), "{}", s.describe());
        assert!(!s.is_roster() && s.near_miss(), "{}", s.describe());
    }
}

#[test]
fn loading_and_update_like_frames_are_no_roster() {
    // A dark frame with a centred line of white text-like glyphs.
    let mut loading = RgbImage::from_pixel(1920, 1080, Rgb([8, 10, 16]));
    for i in 0..12 {
        let x0 = 0.40 + 0.017 * i as f32;
        fill(&mut loading, [x0, 0.46, x0 + 0.011, 0.49], NAME);
    }
    // A dark frame with a wide light band low in the middle, and its text.
    let mut update = RgbImage::from_pixel(1920, 1080, Rgb([12, 14, 20]));
    fill(&mut update, [0.05, 0.52, 0.95, 0.58], [225, 225, 225]);
    fill(&mut update, [0.40, 0.54, 0.60, 0.56], [40, 40, 40]);
    for (name, img) in [("loading", loading), ("update", update)] {
        let s = scores(&img);
        assert!(!s.is_roster(), "{name}: {}", s.describe());
        assert!(!s.near_miss(), "{name}: {}", s.describe());
    }
}

#[test]
fn plain_frames_do_not_match_and_are_no_candidates() {
    for c in [
        [0, 0, 0],
        [255, 255, 255],
        [200, 200, 200],
        [128, 128, 128],
        [30, 40, 70],
        [200, 30, 30],
    ] {
        let s = scores(&RgbImage::from_pixel(1920, 1080, Rgb(c)));
        assert!(!s.is_roster(), "{c:?}: {}", s.describe());
        assert!(!s.near_miss(), "{c:?}: {}", s.describe());
    }
}

#[test]
fn a_default_score_is_no_near_miss_and_one_part_is() {
    assert!(!RosterScores::default().near_miss());
    let header_only = RosterScores {
        header: 0.6,
        ..Default::default()
    };
    assert!(header_only.near_miss() && !header_only.is_roster());
    let strips_only = RosterScores {
        strip_left: 0.9,
        strip_right: 0.9,
        ..Default::default()
    };
    assert!(strips_only.near_miss() && !strips_only.is_roster());
}
