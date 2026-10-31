//! The roster screen (both teams side by side, the bans top right, the map
//! banner bottom right), shown after the ban results and before the hero
//! select. Pixel check only, no OCR. Used only by the opt-in pre-game saves
//! (`debug_save_pregame_screens`); nothing else reads it.
//!
//! It looks only at the parts that stay put whatever the team size, all as
//! shares of the 16:9 game area, so 5v5 and 6v6 rosters and any resolution
//! measure alike. The player rows are never counted:
//!
//! * team headers: the white `TEAM 1` and `TEAM 2` bars over the two
//!   columns, level with each other, each a band of white rows of about
//!   0.047 of the height (searched from 0.18 to 0.34, so a header that moves
//!   a little still counts), with no white between them in the middle;
//! * team columns: the coloured role strip on the inner edge of each
//!   column (team 1's on its right, team 2's on its left), measured on the
//!   band just under the headers, which the first row fills for any team
//!   size;
//! * bans: the red bordered icons, or the white `BANNED HEROES` label;
//! * map banner: the white map name, or the orange logo ring.
//!
//! A frame is the roster when at least [`PARTS_MIN`] parts pass and one of
//! them is the headers or the bans. The columns and the map banner alone
//! are not enough: on replayed real frames, post-match screens (the end
//! summary, the accolades) passed on those two while failing the headers
//! and the bans. Loose on purpose: a few wrong
//! frames saved cost less than a missed roster, since the saves are
//! training data. Tuned on one real 5v5 frame at 2560x1440 (every part
//! passes it with a 2.4 to 3 times margin); no 6v6 frame or other
//! resolution is checked yet.

use image::RgbImage;

/// The header search rows, as shares of the height.
const HEADER_SCAN: (f32, f32) = (0.18, 0.34);
/// The header sample boxes across, clear of the `TEAM 1` / `TEAM 2` text.
const HEADER_LEFT: (f32, f32) = (0.075, 0.340);
const HEADER_RIGHT: (f32, f32) = (0.650, 0.925);
/// A row is white across a header box when at least this share of it is.
const HEADER_ROW_WHITE: f32 = 0.80;
/// A header's white band is this tall (shares of the height): the real one
/// is 0.047; a white frame's band is far taller.
const HEADER_HEIGHT: (f32, f32) = (0.025, 0.070);
/// The two headers' tops are at most this far apart.
const HEADER_LEVEL: f32 = 0.015;
/// Between the headers (x 0.44 to 0.56, at their middle) at most this share
/// is white: the `VS` gap, not one white bar across.
const HEADER_GAP: (f32, f32) = (0.44, 0.56);
const HEADER_GAP_WHITE_MAX: f32 = 0.5;

/// The role strips on the columns' inner edges.
const STRIP_LEFT: (f32, f32) = (0.382, 0.404);
const STRIP_RIGHT: (f32, f32) = (0.596, 0.618);
/// The strip band starts this far under the header's bottom and is this
/// tall; with no header found it is [`STRIP_FALLBACK`].
const STRIP_BELOW: f32 = 0.006;
const STRIP_HEIGHT: f32 = 0.044;
const STRIP_FALLBACK: (f32, f32) = (0.29, 0.33);
/// Each strip at least this share of strongly coloured pixels (any hue:
/// team colours vary, and so do colourblind filters).
const STRIP_COLOUR_MIN: f32 = 0.35;

/// The ban icons and the `BANNED HEROES` label (x0, y0, x1, y1).
const BAN_ICONS: [f32; 4] = [0.770, 0.160, 0.933, 0.224];
const BAN_LABEL: [f32; 4] = [0.663, 0.175, 0.765, 0.207];
const BAN_ICONS_RED_MIN: f32 = 0.03;

/// The map name and the logo ring of the map banner (x0, y0, x1, y1).
const MAP_NAME: [f32; 4] = [0.76, 0.885, 0.90, 0.945];
const MAP_LOGO: [f32; 4] = [0.875, 0.855, 0.972, 0.98];
const MAP_LOGO_ORANGE_MIN: f32 = 0.04;

/// Text boxes count with this share of white pixels; at the top end the
/// box is a plain white area, not text.
const TEXT_WHITE: (f32, f32) = (0.12, 0.90);

/// The fewest passing parts that make the roster, one of them the headers
/// or the bans.
pub const PARTS_MIN: usize = 2;

/// What the roster check measured, one score per part.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RosterScores {
    /// The white header bands' heights (shares of the height), left and
    /// right; 0 with none.
    pub header_left: f32,
    pub header_right: f32,
    /// The headers' tops differ by at most [`HEADER_LEVEL`].
    pub headers_level: bool,
    /// The white share between the headers.
    pub header_gap: f32,
    /// The coloured share of the left and right role strips.
    pub strip_left: f32,
    pub strip_right: f32,
    /// The red share of the ban icons, the white share of their label.
    pub ban_icons: f32,
    pub ban_label: f32,
    /// The white share of the map name, the orange share of the logo.
    pub map_name: f32,
    pub map_logo: f32,
}

impl RosterScores {
    pub fn headers(&self) -> bool {
        let tall = |h: f32| (HEADER_HEIGHT.0..=HEADER_HEIGHT.1).contains(&h);
        tall(self.header_left)
            && tall(self.header_right)
            && self.headers_level
            && self.header_gap < HEADER_GAP_WHITE_MAX
    }

    pub fn columns(&self) -> bool {
        self.strip_left >= STRIP_COLOUR_MIN && self.strip_right >= STRIP_COLOUR_MIN
    }

    pub fn bans(&self) -> bool {
        self.ban_icons >= BAN_ICONS_RED_MIN || text(self.ban_label)
    }

    pub fn map(&self) -> bool {
        text(self.map_name) || self.map_logo >= MAP_LOGO_ORANGE_MIN
    }

    /// How many of the four parts pass.
    pub fn passing(&self) -> usize {
        [self.headers(), self.columns(), self.bans(), self.map()]
            .into_iter()
            .filter(|p| *p)
            .count()
    }

    /// At least [`PARTS_MIN`] parts, one of them the headers or the bans.
    pub fn is_roster(&self) -> bool {
        self.passing() >= PARTS_MIN && (self.headers() || self.bans())
    }

    /// Some part passed but the frame is no roster: a turned-down
    /// candidate worth a look. A frame with no part is no candidate.
    pub fn near_miss(&self) -> bool {
        self.passing() > 0 && !self.is_roster()
    }

    /// The scores and the verdict, rounded, for a debug line.
    pub fn describe(&self) -> String {
        format!(
            "headers {} (left {:.3}, right {:.3}, level {}, gap {:.2}), columns {} (strips {:.2}, {:.2}), bans {} (icons red {:.3}, label {:.2}), map {} (name {:.2}, logo {:.3}), {}/4 parts, roster {}",
            self.headers(),
            self.header_left,
            self.header_right,
            self.headers_level,
            self.header_gap,
            self.columns(),
            self.strip_left,
            self.strip_right,
            self.bans(),
            self.ban_icons,
            self.ban_label,
            self.map(),
            self.map_name,
            self.map_logo,
            self.passing(),
            self.is_roster(),
        )
    }
}

fn text(white: f32) -> bool {
    (TEXT_WHITE.0..TEXT_WHITE.1).contains(&white)
}

fn white(p: [u8; 3]) -> bool {
    let (min, max) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
    min >= 200 && max - min <= 35
}

fn coloured(p: [u8; 3]) -> bool {
    let (min, max) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
    max >= 80 && max - min >= 50
}

fn red(p: [u8; 3]) -> bool {
    let [r, g, b] = p;
    r >= 160 && g <= 90 && b <= 90 && r - g.max(b) >= 90
}

fn orange(p: [u8; 3]) -> bool {
    let [r, g, b] = p;
    r >= 190 && (70..=175).contains(&g) && b <= 90 && r - b >= 130
}

/// The 16:9 game area and share-to-pixel maps.
struct Area {
    gx: u32,
    gy: u32,
    gw: u32,
    gh: u32,
}

impl Area {
    fn x(&self, share: f32) -> u32 {
        self.gx + (share.clamp(0.0, 1.0) * self.gw as f32) as u32
    }

    fn y(&self, share: f32) -> u32 {
        self.gy + (share.clamp(0.0, 1.0) * self.gh as f32) as u32
    }

    /// The share of sampled pixels (every second each way) of the box
    /// (x0, y0, x1, y1) for which `hit` holds; 0 for an empty box.
    fn share(&self, rgb: &RgbImage, b: [f32; 4], hit: fn([u8; 3]) -> bool) -> f32 {
        let (x0, x1) = (self.x(b[0]), self.x(b[2]).min(rgb.width()));
        let (y0, y1) = (self.y(b[1]), self.y(b[3]).min(rgb.height()));
        let (mut n, mut total) = (0u32, 0u32);
        for y in (y0..y1).step_by(2) {
            for x in (x0..x1).step_by(2) {
                total += 1;
                n += u32::from(hit(rgb.get_pixel(x, y).0));
            }
        }
        if total == 0 {
            0.0
        } else {
            n as f32 / total as f32
        }
    }

    /// The longest band of rows in [`HEADER_SCAN`] whose span `xs` is at
    /// least [`HEADER_ROW_WHITE`] white: (top, bottom) as shares of the
    /// height, `None` with no such row.
    fn white_band(&self, rgb: &RgbImage, xs: (f32, f32)) -> Option<(f32, f32)> {
        let (x0, x1) = (self.x(xs.0), self.x(xs.1).min(rgb.width()));
        let (y0, y1) = (
            self.y(HEADER_SCAN.0),
            self.y(HEADER_SCAN.1).min(rgb.height()),
        );
        let mut best: Option<(u32, u32)> = None;
        let mut start: Option<u32> = None;
        for y in y0..y1 {
            let (mut n, mut total) = (0u32, 0u32);
            for x in (x0..x1).step_by(2) {
                total += 1;
                n += u32::from(white(rgb.get_pixel(x, y).0));
            }
            if total > 0 && n as f32 >= total as f32 * HEADER_ROW_WHITE {
                let top = *start.get_or_insert(y);
                if best.is_none_or(|(t, b)| y - top > b - t) {
                    best = Some((top, y));
                }
            } else {
                start = None;
            }
        }
        let share = |y: u32| (y - self.gy) as f32 / self.gh as f32;
        // A band of one row is one row high, not none.
        best.map(|(t, b)| (share(t), share(b + 1)))
    }
}

/// Measure the roster parts on `rgb`.
pub fn scores(rgb: &RgbImage) -> RosterScores {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::frame::geometry::game_rect_16_9(w, h);
    if gw < 16 || gh < 16 {
        return RosterScores::default();
    }
    let area = Area { gx, gy, gw, gh };
    let left = area.white_band(rgb, HEADER_LEFT);
    let right = area.white_band(rgb, HEADER_RIGHT);
    let height = |band: Option<(f32, f32)>| band.map_or(0.0, |(t, b)| b - t);
    let headers_level = match (left, right) {
        (Some((a, _)), Some((b, _))) => (a - b).abs() <= HEADER_LEVEL,
        _ => false,
    };
    let header_gap = match left.or(right) {
        Some((t, b)) => {
            let mid = (t + b) / 2.0;
            area.share(
                rgb,
                [HEADER_GAP.0, mid - 0.01, HEADER_GAP.1, mid + 0.01],
                white,
            )
        }
        None => 0.0,
    };
    let (band_top, band_bottom) = match left.or(right) {
        Some((_, bottom)) => (bottom + STRIP_BELOW, bottom + STRIP_BELOW + STRIP_HEIGHT),
        None => STRIP_FALLBACK,
    };
    let strip = |xs: (f32, f32)| area.share(rgb, [xs.0, band_top, xs.1, band_bottom], coloured);
    RosterScores {
        header_left: height(left),
        header_right: height(right),
        headers_level,
        header_gap,
        strip_left: strip(STRIP_LEFT),
        strip_right: strip(STRIP_RIGHT),
        ban_icons: area.share(rgb, BAN_ICONS, red),
        ban_label: area.share(rgb, BAN_LABEL, white),
        map_name: area.share(rgb, MAP_NAME, white),
        map_logo: area.share(rgb, MAP_LOGO, orange),
    }
}

/// Whether `rgb` looks like the roster screen ([`RosterScores::is_roster`]).
pub fn is_roster_screen(rgb: &RgbImage) -> bool {
    scores(rgb).is_roster()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 1280;
    const H: u32 = 720;

    fn fill(img: &mut RgbImage, b: [f32; 4], c: [u8; 3]) {
        let (x0, y0) = ((b[0] * W as f32) as u32, (b[1] * H as f32) as u32);
        let (x1, y1) = ((b[2] * W as f32) as u32, (b[3] * H as f32) as u32);
        for y in y0..y1.min(H) {
            for x in x0..x1.min(W) {
                img.put_pixel(x, y, image::Rgb(c));
            }
        }
    }

    /// A roster-like frame: a dim map backdrop, the two white headers, the
    /// columns with `rows` player rows each (the rows share the same span,
    /// so 6 rows are thinner than 5), role strips on the inner edges, the
    /// bans and the map banner.
    fn roster(rows: u32, bans: bool, map: bool) -> RgbImage {
        let mut img = RgbImage::from_pixel(W, H, image::Rgb([60, 70, 90]));
        fill(&mut img, [0.0676, 0.2361, 0.4059, 0.2819], [247, 246, 246]);
        fill(&mut img, [0.5938, 0.2361, 0.9324, 0.2819], [247, 246, 246]);
        // Dark label text on the headers, past the sample boxes.
        fill(&mut img, [0.36, 0.25, 0.40, 0.27], [89, 3, 93]);
        fill(&mut img, [0.60, 0.25, 0.64, 0.27], [184, 151, 1]);
        let (top, span) = (0.284, 0.472);
        let pitch = span / rows as f32;
        for r in 0..rows {
            let y0 = top + pitch * r as f32;
            let y1 = y0 + pitch - 0.004;
            fill(&mut img, [0.0676, y0, 0.3801, y1], [120, 60, 160]);
            fill(&mut img, [0.3801, y0, 0.4059, y1], [107, 22, 145]);
            fill(&mut img, [0.5938, y0, 0.6195, y1], [255, 210, 4]);
            fill(&mut img, [0.6195, y0, 0.9324, y1], [70, 140, 170]);
        }
        if bans {
            fill(&mut img, [0.6668, 0.1785, 0.7605, 0.2042], [250, 250, 250]);
            for i in 0..4 {
                let x0 = 0.7715 + 0.0422 * i as f32;
                fill(&mut img, [x0, 0.1611, x0 + 0.0344, 0.2229], [200, 30, 30]);
                fill(
                    &mut img,
                    [x0 + 0.004, 0.168, x0 + 0.030, 0.216],
                    [90, 60, 50],
                );
            }
        }
        if map {
            fill(&mut img, [0.7688, 0.8889, 0.8945, 0.9410], [60, 70, 90]);
            for i in 0..6 {
                let x0 = 0.7688 + 0.021 * i as f32;
                fill(&mut img, [x0, 0.8889, x0 + 0.012, 0.9410], [250, 250, 250]);
            }
            fill(&mut img, [0.905, 0.87, 0.97, 0.88], [240, 120, 20]);
            fill(&mut img, [0.905, 0.96, 0.97, 0.97], [240, 120, 20]);
        }
        img
    }

    #[test]
    fn five_and_six_player_rosters_both_match() {
        for rows in [5, 6] {
            let s = scores(&roster(rows, true, true));
            assert!(
                s.headers() && s.columns() && s.bans() && s.map(),
                "{rows}: {s:?}"
            );
            assert!(s.is_roster(), "{rows}: {}", s.describe());
        }
    }

    #[test]
    fn a_roster_with_no_bans_or_no_map_still_matches() {
        assert!(is_roster_screen(&roster(6, false, true)));
        assert!(is_roster_screen(&roster(5, true, false)));
        // Only the team columns: still two parts.
        assert!(is_roster_screen(&roster(6, false, false)));
    }

    #[test]
    fn plain_frames_do_not_match_and_are_no_candidates() {
        for c in [[0, 0, 0], [128, 128, 128], [255, 255, 255], [30, 40, 70]] {
            let s = scores(&RgbImage::from_pixel(W, H, image::Rgb(c)));
            assert!(!s.is_roster(), "{c:?}: {}", s.describe());
            assert!(!s.near_miss(), "{c:?}: {}", s.describe());
        }
    }

    #[test]
    fn the_map_banner_alone_is_a_near_miss_not_a_roster() {
        let mut img = RgbImage::from_pixel(W, H, image::Rgb([60, 70, 90]));
        fill(&mut img, [0.905, 0.87, 0.97, 0.88], [240, 120, 20]);
        let s = scores(&img);
        assert!(
            s.map() && !s.is_roster() && s.near_miss(),
            "{}",
            s.describe()
        );
        // Bans and map but no team part: two parts, one of them the bans.
        let mut img = roster(5, true, true);
        // The headers and columns painted over.
        fill(&mut img, [0.0, 0.23, 1.0, 0.76], [60, 70, 90]);
        let s = scores(&img);
        assert!(
            !s.headers() && !s.columns() && s.bans() && s.map(),
            "{}",
            s.describe()
        );
        assert!(s.is_roster(), "{}", s.describe());
    }

    #[test]
    fn columns_and_map_alone_are_no_roster() {
        // The post-match screens that passed the old rule: column-like
        // strips and the map banner, no headers, no bans.
        let mut img = roster(5, false, true);
        // The headers painted over, above the first row.
        fill(&mut img, [0.0, 0.18, 1.0, 0.284], [60, 70, 90]);
        let s = scores(&img);
        assert!(
            !s.headers() && s.columns() && !s.bans() && s.map(),
            "{}",
            s.describe()
        );
        assert!(!s.is_roster(), "{}", s.describe());
        assert!(s.near_miss(), "{}", s.describe());
        // A Dorado-like frame (headers, columns, bans, map) still passes.
        let s = scores(&roster(5, true, true));
        assert_eq!(s.passing(), 4, "{}", s.describe());
        assert!(s.is_roster(), "{}", s.describe());
    }

    #[test]
    fn a_lower_resolution_roster_matches_too() {
        let big = roster(5, true, true);
        let small = image::imageops::resize(&big, 854, 480, image::imageops::FilterType::Triangle);
        assert!(is_roster_screen(&small), "{}", scores(&small).describe());
    }
}
