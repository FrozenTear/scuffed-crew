//! Chat on screen, read from the full frame.
//!
//! The chat box stacks lines upward from a fixed bottom line in the lower
//! left of the playfield, below the scoreboard crop. Each line starts at the
//! box's left edge, left of the crop, so the full frame shows how far up the
//! stack reaches even where the lines run over the board. Chat drawn in a
//! role bar's own colour is invisible to the portrait gate, so portraits the
//! stack can reach are dropped when chat shows; with no frame (a crop alone)
//! every row the chat box could reach drops its portrait.
//!
//! Measured on the member's 14 full Tab frames of 2026-10-11/12 (2560x1440):
//! the bottom band x 60-448, y 1250-1340 holds 0-2 stroke px (a channel
//! more than [`STROKE`] = 40 from its 5x5 median) without chat and
//! 282-2,071 with it; at 30 two chat-free Tab frames hold 91 and 93. The
//! bottom line sits at y 1258-1308.

use image::RgbImage;

use crate::ocr::preprocess::{crop_scoreboard_rect, game_rect_16_9};

/// The chat strip left of the scoreboard crop, in 2560x1440 playfield px:
/// x range, the bottom band, and the line pitch the stack is walked in.
const X: (f64, f64) = (60.0, 448.0);
const BOTTOM: (f64, f64) = (1250.0, 1340.0);
const LINE: f64 = 28.0;
/// A stroke: a channel more than this from its 5x5 median.
pub(super) const STROKE: i32 = 40;
/// Stroke px (x scale^2) in a band that mean text.
pub(super) const TEXT_PX: f64 = 12.0;
/// Rows whose bottom lies below this share of the crop height are within
/// the chat box's reach (about seven lines above the bottom line).
pub(super) const REACH: f64 = 0.85;

/// What the frame says about chat over the board.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Chat {
    /// Not known (no frame): every row the chat box could reach is treated
    /// as covered.
    Unknown,
    /// No chat on screen.
    None,
    /// Chat shows; its stack reaches up to this crop row (0 = the top).
    ReachesY(u32),
}

impl Chat {
    /// Whether a crop row ending at `bot` may be under chat.
    pub(crate) fn covers(self, bot: u32, crop_h: u32) -> bool {
        match self {
            Chat::None => false,
            Chat::Unknown => f64::from(bot) > REACH * f64::from(crop_h),
            Chat::ReachesY(y) => bot > y || f64::from(bot) > REACH * f64::from(crop_h),
        }
    }
}

/// Stroke px in `[x0, x1) x [y0, y1)` of the frame, against 5x5 medians taken
/// inside that rectangle (edges replicated).
fn strokes(img: &RgbImage, x0: u32, x1: u32, y0: u32, y1: u32) -> usize {
    let (x1, y1) = (x1.min(img.width()), y1.min(img.height()));
    if x1 <= x0 || y1 <= y0 {
        return 0;
    }
    let mut n = 0;
    let mut win = [0u8; 25];
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.get_pixel(x, y).0;
            for (c, &pc) in p.iter().enumerate() {
                let mut k = 0;
                for dy in -2i64..=2 {
                    for dx in -2i64..=2 {
                        let sx = (i64::from(x) + dx).clamp(i64::from(x0), i64::from(x1) - 1) as u32;
                        let sy = (i64::from(y) + dy).clamp(i64::from(y0), i64::from(y1) - 1) as u32;
                        win[k] = img.get_pixel(sx, sy).0[c];
                        k += 1;
                    }
                }
                win.sort_unstable();
                if (i32::from(pc) - i32::from(win[12])).abs() > STROKE {
                    n += 1;
                    break;
                }
            }
        }
    }
    n
}

/// Chat over the scoreboard crop of a full frame.
pub fn chat_over_board(frame: &RgbImage) -> Chat {
    let (w, h) = frame.dimensions();
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let (sx, sy) = (f64::from(gw) / 2560.0, f64::from(gh) / 1440.0);
    let px = |v: f64| gx + (v * sx).round() as u32;
    let py = |v: f64| gy + (v * sy).round() as u32;
    let min_px = TEXT_PX * sx * sy;
    let (x0, x1) = (px(X.0), px(X.1));
    if (strokes(frame, x0, x1, py(BOTTOM.0), py(BOTTOM.1)) as f64) <= min_px {
        return Chat::None;
    }
    // Walk the stack up line by line from the bottom band; it ends at the
    // first empty line. Up to the crop's top means every row may be covered.
    let (_, crop_y, _, _) = crop_scoreboard_rect(w, h);
    let mut top = BOTTOM.0;
    loop {
        let next = top - LINE;
        let (a, b) = (py(next), py(top));
        if b <= crop_y.max(gy) || (strokes(frame, x0, x1, a, b) as f64) <= min_px {
            break;
        }
        top = next;
    }
    // one line of margin above the highest line found
    let reach = py(top - LINE);
    Chat::ReachesY(reach.saturating_sub(crop_y))
}
