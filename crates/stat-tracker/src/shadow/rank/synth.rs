//! Synthetic rank screens for tests. No game art.
//!
//! `paint`, `Scene` and `labels` are shared with the pack cutter
//! tests. The shapes are the same stand-in font the rank reader tests use.

use image::{Rgb, RgbImage};

use super::{FRAME_H, FRAME_W, Labels, Role, neighbours};

pub(crate) fn rows_of(c: char) -> [&'static str; 7] {
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
pub(crate) fn draw(img: &mut RgbImage, text: &str, x: u32, y: u32, s: u32, col: Rgb<u8>) -> u32 {
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
pub(crate) fn width(text: &str, s: u32) -> u32 {
    text.chars()
        .map(|c| if c == ' ' { 3 * s } else { 6 * s })
        .sum::<u32>()
        - s
}

/// Three chevrons (`c` is `<` or `>`) at two-pixel dots, each one dot
/// over the last so they cut as one glyph [`CHEVRONS_W`] wide.
pub(crate) fn chevrons(img: &mut RgbImage, c: char, x: u32, y: u32) {
    for k in 0..3 {
        draw(img, &c.to_string(), x + k * 8, y, 2, WHITE);
    }
}

pub(crate) const CHEVRONS_W: u32 = 26;

pub(crate) fn fill(img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, col: Rgb<u8>) {
    for y in y0..y1 {
        for x in x0..x1 {
            img.put_pixel(x, y, col);
        }
    }
}

pub(crate) const BG: Rgb<u8> = Rgb([40, 44, 60]);
pub(crate) const WHITE: Rgb<u8> = Rgb([250, 250, 250]);
pub(crate) const CYAN: Rgb<u8> = Rgb([28, 239, 239]);
pub(crate) const ORANGE: Rgb<u8> = Rgb([240, 120, 30]);
pub(crate) const YELLOW: Rgb<u8> = Rgb([252, 252, 4]);
pub(crate) const GREEN: Rgb<u8> = Rgb([0, 200, 0]);
pub(crate) const RED: Rgb<u8> = Rgb([208, 59, 97]);
pub(crate) const SR_GREEN: Rgb<u8> = Rgb([60, 230, 60]);

pub(crate) fn blank() -> RgbImage {
    RgbImage::from_pixel(FRAME_W, FRAME_H, BG)
}

/// What a synthetic rank screen shows.
#[derive(Clone)]
pub(crate) struct Scene {
    pub(crate) pct: i32,
    pub(crate) higher: u8,
    pub(crate) change: Option<i32>,
    pub(crate) sr: Option<u32>,
    pub(crate) sr_change: Option<i32>,
    pub(crate) tags: Vec<&'static str>,
    pub(crate) banner: Option<&'static str>,
    pub(crate) tier: Option<&'static str>,
    pub(crate) division: Option<u8>,
    /// Hero SR text grey level (250 = bright, about 80 = under a chat overlay).
    pub(crate) hero_level: u8,
    /// Role line above the emblem; `None` paints none (open queue).
    pub(crate) role: Option<Role>,
    /// Chevrons on the pill: `>>>` after a green pill's text, `<<<` before a
    /// red pill's minus.
    pub(crate) chevrons: bool,
    /// A rank icon left of the hero SR.
    pub(crate) sr_icon: Icon,
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
pub(crate) enum Icon {
    None,
    /// Taller than the digits, its bottom below theirs.
    Tall,
    /// A disc of digit height (a block with a dark bar), as some tiers show.
    Level,
}

/// Where the pill text starts.
pub(crate) const PILL_X: u32 = 730;
/// The pill text's top row (two-pixel dots, 14 rows).
pub(crate) const PILL_Y: u32 = 1135;

/// Left edge of the pill's digits for `sc` (a red pill keeps room for its
/// chevrons and minus bar).
pub(crate) fn pill_digits_x(sc: &Scene) -> u32 {
    match (sc.change, sc.chevrons) {
        (Some(c), true) if c < 0 => PILL_X + CHEVRONS_W + 12 + 10,
        (Some(c), false) if c < 0 => PILL_X + 10,
        _ => PILL_X,
    }
}

/// Paint `sc` at the reader's positions (see `layout`).
pub(crate) fn paint(sc: &Scene) -> RgbImage {
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
pub(crate) fn role_icon(r: Role) -> char {
    match r {
        Role::Support => '+',
        Role::Damage => 'E',
        Role::Tank => 'O',
        Role::Open => 'L',
    }
}

/// Labels for `sc`, in the bench's JSON shape.
pub(crate) fn labels(sc: &Scene) -> Labels {
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
pub(crate) fn training() -> Vec<Scene> {
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
