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
        // shift on a letterboxed Tab, and the right drop catches none.
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
/// from 1 px above its top rather than dropped.
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
