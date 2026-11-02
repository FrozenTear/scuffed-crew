//! Tests for the layout check (`focus/layout.rs`): ratios only, so common
//! resolutions, ultrawide and windowed games pass, and only degenerate
//! frames fail. Sizes only, no pictures.

use super::focus::layout::*;

#[test]
fn common_frames_hold_the_game_area() {
    for (w, h) in [
        (1920, 1080),
        (2560, 1440),
        (3840, 2160),
        (1280, 720),
        (2560, 1080),
        (3440, 1440),
        (5120, 1440),
        (1920, 1200),
        (2560, 1600),
        (1024, 768),
    ] {
        assert_eq!(layout_ok_for_frame(w, h), LayoutVerdict::Fits, "{w}x{h}");
    }
}

#[test]
fn a_windowed_game_smaller_than_the_frame_or_offset_passes() {
    assert_eq!(
        layout_ok(1920, 1080, (100, 50, 1280, 720)),
        LayoutVerdict::Fits
    );
    assert_eq!(
        layout_ok(2560, 1440, (0, 0, 1600, 900)),
        LayoutVerdict::Fits
    );
    assert_eq!(
        layout_ok(3440, 1440, (1000, 300, 1920, 1080)),
        LayoutVerdict::Fits
    );
    // Odd sizes round within the tolerance.
    assert_eq!(
        layout_ok(1920, 1080, (0, 0, 1366, 768)),
        LayoutVerdict::Fits
    );
}

#[test]
fn degenerate_frames_fail() {
    assert_eq!(layout_ok_for_frame(0, 1080), LayoutVerdict::EmptyFrame);
    assert_eq!(layout_ok_for_frame(1920, 0), LayoutVerdict::EmptyFrame);
    assert_eq!(
        layout_ok(1920, 1080, (0, 0, 0, 0)),
        LayoutVerdict::EmptyFrame
    );
    assert_eq!(
        layout_ok(1920, 1080, (800, 500, 1280, 720)),
        LayoutVerdict::OutsideFrame
    );
    assert_eq!(
        layout_ok(1920, 1080, (0, 0, 1920, 1440)),
        LayoutVerdict::OutsideFrame
    );
    assert_eq!(
        layout_ok(1920, 1080, (0, 0, 1440, 1080)),
        LayoutVerdict::NotSixteenNine
    );
    assert_eq!(
        layout_ok(1920, 1080, (0, 0, 320, 180)),
        LayoutVerdict::TooSmall
    );
    // A frame too thin for any 16:9 area rounds to a rect that is not 16:9.
    assert_ne!(layout_ok_for_frame(1, 1000), LayoutVerdict::Fits);
}

#[test]
fn the_edge_passes_and_fails_with_the_frame() {
    let mut edge = LayoutEdge::default();
    assert!(edge.passes(2560, 1440));
    assert!(!edge.passes(0, 1440));
    assert!(!edge.passes(0, 1440));
    assert!(edge.passes(3440, 1440));
}
