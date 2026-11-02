//! The layout check: can this frame hold the 16:9 playfield the crops
//! assume? Ratios only, no pixel sizes, so 1080p, 1440p, ultrawide and a
//! windowed game (a game rect smaller than the frame, or offset) all pass.
//! It turns down only frames where the crops would land outside or across
//! the picture: an empty frame, a rect outside it, a rect far from 16:9,
//! or one too small a share of the frame.
//!
//! It is separate from the focus check. Focus says Overwatch is the active
//! window; this says the captured frame can hold its playfield. A poll
//! reads only when both pass.

use stat_tracker::frame::geometry::game_rect_16_9;

/// How far a game rect's aspect may be from 16:9, as a share of it.
const ASPECT_TOLERANCE: f64 = 0.02;

/// The smallest game rect, as a share of the frame's area.
const MIN_AREA_SHARE: f64 = 1.0 / 16.0;

/// A game rect inside a frame: x, y, width, height.
pub(crate) type GameRect = (u32, u32, u32, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutVerdict {
    Fits,
    EmptyFrame,
    OutsideFrame,
    NotSixteenNine,
    TooSmall,
}

/// The check for a game rect `game` in a `frame_w` by `frame_h` frame.
pub(crate) fn layout_ok(frame_w: u32, frame_h: u32, game: GameRect) -> LayoutVerdict {
    let (x, y, w, h) = game;
    if frame_w == 0 || frame_h == 0 || w == 0 || h == 0 {
        return LayoutVerdict::EmptyFrame;
    }
    let right = u64::from(x) + u64::from(w);
    let bottom = u64::from(y) + u64::from(h);
    if right > u64::from(frame_w) || bottom > u64::from(frame_h) {
        return LayoutVerdict::OutsideFrame;
    }
    let aspect = f64::from(w) / f64::from(h);
    if (aspect / (16.0 / 9.0) - 1.0).abs() > ASPECT_TOLERANCE {
        return LayoutVerdict::NotSixteenNine;
    }
    let share = (f64::from(w) * f64::from(h)) / (f64::from(frame_w) * f64::from(frame_h));
    if share < MIN_AREA_SHARE {
        return LayoutVerdict::TooSmall;
    }
    LayoutVerdict::Fits
}

/// The check on a whole frame, with the game rect the crops use
/// ([`game_rect_16_9`]).
pub(crate) fn layout_ok_for_frame(frame_w: u32, frame_h: u32) -> LayoutVerdict {
    layout_ok(frame_w, frame_h, game_rect_16_9(frame_w, frame_h))
}

/// The poll's layout check with its log on edges only. True when the frame
/// may be read.
#[derive(Debug, Default)]
pub(crate) struct LayoutEdge {
    failing: bool,
}

impl LayoutEdge {
    pub(crate) fn passes(&mut self, frame_w: u32, frame_h: u32) -> bool {
        let verdict = layout_ok_for_frame(frame_w, frame_h);
        let fits = verdict == LayoutVerdict::Fits;
        if fits && self.failing {
            tracing::info!(target: "scuffed_stat_tracker", frame_w, frame_h, "layout check: the frame holds the game area again, reads go on");
        } else if !fits && !self.failing {
            tracing::info!(target: "scuffed_stat_tracker", frame_w, frame_h, ?verdict, "layout check: the frame cannot hold the 16:9 game area, reads skipped");
        }
        self.failing = !fits;
        fits
    }
}
