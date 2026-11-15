//! The `hero_icon` card snap on a letterboxed Tab, and over a black game
//! world that the letterbox black check cannot tell from a bar.

use super::*;

/// `frame`, a [`range_frame`] of `card`, with every row above `top + clip`
/// (`clip` px at 1440p) at luma `bar`: a letterbox bar over the card's top.
fn letterbox_frame(
    frame: DynamicImage,
    card: (u32, u32, u32),
    u: u32,
    clip: f64,
    bar: u8,
) -> DynamicImage {
    let mut img = frame.to_rgb8();
    let end = card.1 + (clip * f64::from(u) / 1440.0).round() as u32;
    for (_, y, p) in img.enumerate_pixels_mut() {
        if y < end {
            *p = Rgb([bar; 3]);
        }
    }
    DynamicImage::ImageRgb8(img)
}

#[test]
fn a_letterboxed_card_top_still_cuts_the_whole_card() {
    // On 29 accepted live 1440p Tabs a black bar covered every row down to
    // y 215: the card's top 7 px were lost, its found top was 216, and its
    // bottom gap stayed at 436. The cut is the card's own square (1604,
    // 209, 228 at 1440p) with those rows black, from the bottom gap and
    // CARD_SIDE: not FarEdge, and not the visible 220 px square at 216,
    // which would clip the card's right.
    assert_eq!(range_card(1440), (1604, 209, 228));
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, side) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
        let corner = (left - scale(23.0), top - scale(2.0));
        let boxed = |patch, clip, bar| {
            let frame = letterbox_frame(range_frame(w, h, card, u, patch), card, u, clip, bar);
            snap_to_card(&frame, corner)
        };
        // Up to CARD_CLIP_MAX (16 px at 1440p, 12 at 1080p), under a bar
        // of at most CARD_BLACK_MAX_LUMA (4).
        for (clip, bar) in [(7.0, 0), (16.0, 0), (7.0, 4)] {
            let got = boxed(RangePatch::Bare, clip, bar);
            assert_eq!(got, Ok(card), "{name} clip {clip} bar {bar}");
        }
        // Past the cap, or under a bar that is not black: the found top's
        // cut, whose bottom is past the gap.
        for (clip, bar) in [(17.0, 0), (7.0, 5), (7.0, 16)] {
            let cut = (left, top + scale(clip), side);
            let want = Err(SnapMiss::FarEdge {
                cut,
                right: None,
                bottom: Some(top + side - 1),
            });
            assert_eq!(
                boxed(RangePatch::Bare, clip, bar),
                want,
                "{name} clip {clip} bar {bar}"
            );
        }
        // A residual: beside the navy panel both outer bands see the bar and
        // step at the found top, so the outer-top guard catches no left
        // shift on a letterboxed Tab, and the right drop catches none. A
        // lit patch left of the card that reaches the bar (tall, or on the
        // card's rows) is kept shifted at any width the left search
        // reaches, and a partial one under about 45 px; a wider partial
        // one is still OuterTop.
        let shifted = Ok((left - scale(30.0), top, side));
        assert_eq!(
            boxed(RangePatch::LitLeftPartial, 7.0, 0),
            shifted,
            "{name} 30 px left"
        );
    }
}

/// Over a black game world (luma 0, as on some Practice Range Tabs) the
/// black check cannot tell the world from a letterbox bar. A cut shifted
/// onto a 60 px lit patch left of the card (45 px at 1080p), over part of
/// its rows, sees black down to the patch's top over its outer-left band,
/// but the cut was not moved, so it is still OuterTop. A residual: a card
/// painted 2 px short beside the navy panel, with black above it, is cut
/// from 1 px above its top rather than dropped. That is the 1 px case of a
/// wider one: with black above the card, a bottom drop k px early (up to
/// CARD_CLIP_MAX) moves the cut up k px. The measured bottoms (a drop of
/// at least 18 on the 55 Practice Range Tabs) make this unlikely.
#[test]
fn a_black_game_world_is_not_a_letterbox_bar() {
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, side) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
        let corner = (left - scale(23.0), top - scale(2.0));
        let world = |patch, width| range_frame_around(w, h, card, u, patch, (24, 0), width);
        let got = snap_to_card(&world(RangePatch::LitLeftPartial, 60.0), corner);
        let shifted = (left - scale(60.0), top, side);
        assert!(
            matches!(got, Err(SnapMiss::OuterTop { cut, .. }) if cut == shifted),
            "{name}: {got:?}"
        );
        let mut short = world(RangePatch::Bare, 30.0).to_rgb8();
        let y = top + side - 2;
        for x in left..left + side {
            short.put_pixel(x, y, Rgb([17 + ((x + y) % 5) as u8; 3]));
        }
        let got = snap_to_card(&DynamicImage::ImageRgb8(short), corner);
        assert_eq!(got, Ok((left, top - 1, side)), "{name} 2 px short");
    }
}

/// The two halves of the outer exception on a moved cut. Its outer band
/// must step within 1 px of the found top: a 7 px bar over a black world
/// (luma 0) and a 60 px lit patch left of the card, over part of its
/// rows, moves the shifted cut up, but its outer left steps at the patch's
/// top, so it is still OuterTop. And the rows above that step must be
/// letterbox-black: a bar over the card whose outer-left columns are not
/// black (luma 5 and up) is OuterTop at the card's own cut.
#[test]
fn a_moved_cut_needs_its_outer_step_at_the_found_top_past_black_rows() {
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, side) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
        let corner = (left - scale(23.0), top - scale(2.0));
        let world = range_frame_around(w, h, card, u, RangePatch::LitLeftPartial, (24, 0), 60.0);
        let got = snap_to_card(&letterbox_frame(world, card, u, 7.0, 0), corner);
        let want = Err(SnapMiss::OuterTop {
            cut: (left - scale(60.0), top, side),
            tops: [Some(top + scale(20.0)), Some(top + scale(7.0))],
        });
        assert_eq!(got, want, "{name} 60 px partial over a black world");
        let end = top + scale(7.0);
        let bare = range_frame(w, h, card, u, RangePatch::Bare);
        let boxed = letterbox_frame(bare, card, u, 7.0, 0);
        for outer in [5, 10, 16] {
            let mut img = boxed.to_rgb8();
            for y in 0..end {
                for x in left..left + side / 4 {
                    img.put_pixel(x, y, Rgb([outer; 3]));
                }
            }
            let got = snap_to_card(&DynamicImage::ImageRgb8(img), corner);
            let want = Err(SnapMiss::OuterTop {
                cut: card,
                tops: [Some(end), Some(end)],
            });
            assert_eq!(got, want, "{name} outer left at {outer}");
        }
    }
}
