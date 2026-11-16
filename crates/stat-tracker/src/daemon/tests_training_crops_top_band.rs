//! The `hero_icon` card snap under a black row band over the card's top:
//! a black bar, or a black game world above a card whose bottom drop lands
//! early. Both keep no cut ([`SnapMiss::FarEdge`]), by design: the cut is
//! never moved up to the top the bottom drop implies. The bar models a
//! frame rebuilt from a debug board crop, not a capture.

use super::*;

/// `frame`, a [`range_frame`] of `card`, with every row above `top + clip`
/// (`clip` px at 1440p) at luma `bar`: a bar over the card's top.
fn bar_frame(
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

/// A bar over the card's top `clip` px (at 1440p) keeps no cut, black or
/// not, short or tall: the found top is the bar's edge and the cut from
/// there runs past the card's bottom gap. A cut shifted 30 px left onto a
/// lit patch beside the card, under a 7 px black bar, keeps none either:
/// FarEdge at the shifted cut from the bar's edge, on the same drop.
#[test]
fn a_black_bar_over_the_card_top_keeps_no_cut() {
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, side) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
        let corner = (left - scale(23.0), top - scale(2.0));
        let boxed = |patch, clip, bar| {
            let frame = bar_frame(range_frame(w, h, card, u, patch), card, u, clip, bar);
            snap_to_card(&frame, corner)
        };
        for (clip, bar) in [
            (1.0, 0),
            (7.0, 0),
            (16.0, 0),
            (7.0, 4),
            (17.0, 0),
            (7.0, 5),
            (7.0, 16),
        ] {
            let want = Err(SnapMiss::FarEdge {
                cut: (left, top + scale(clip), side),
                right: None,
                bottom: Some(top + side - 1),
            });
            assert_eq!(
                boxed(RangePatch::Bare, clip, bar),
                want,
                "{name} clip {clip} bar {bar}"
            );
        }
        let want = Err(SnapMiss::FarEdge {
            cut: (left - scale(30.0), top + scale(7.0), side),
            right: None,
            bottom: Some(top + side - 1),
        });
        assert_eq!(
            boxed(RangePatch::LitLeftPartial, 7.0, 0),
            want,
            "{name} 30 px left"
        );
    }
}

/// Over a black game world (luma 0, as on some Practice Range Tabs) a cut
/// shifted onto a 60 px lit patch left of the card (45 px at 1080p), over
/// part of its rows, is OuterTop, and a card painted 2 px short (its
/// bottom drop 1 px early) is FarEdge at the card's own cut.
#[test]
fn a_black_game_world_keeps_no_shifted_or_short_cut() {
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, side) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
        let corner = (left - scale(23.0), top - scale(2.0));
        let world = |patch, width| range_frame_around(w, h, card, u, patch, (24, 0), width);
        let got = snap_to_card(&world(RangePatch::LitLeftPartial, 60.0), corner);
        let shifted = (left - scale(60.0), top, side);
        let want = Err(SnapMiss::OuterTop {
            cut: shifted,
            tops: [Some(top + scale(20.0)), Some(top)],
        });
        assert_eq!(got, want, "{name} 60 px partial");
        let mut short = world(RangePatch::Bare, 30.0).to_rgb8();
        let y = top + side - 2;
        for x in left..left + side {
            short.put_pixel(x, y, Rgb([17 + ((x + y) % 5) as u8; 3]));
        }
        let got = snap_to_card(&DynamicImage::ImageRgb8(short), corner);
        let want = Err(SnapMiss::FarEdge {
            cut: card,
            right: None,
            bottom: Some(top + side - 2),
        });
        assert_eq!(got, want, "{name} 2 px short");
    }
}

/// Over a black world (luma 0 above and left of the card) with the card
/// painted `k` px shorter than the range card (1, 8 and 16 px at 1440p;
/// 1, 6 and 12 at 1080p; `k + 1` px short of the cut side, so 1 is the
/// 2 px short case above), the bottom drop lands `k` px early and the
/// snap is FarEdge at the card's own cut, never the cut moved up `k` px.
#[test]
fn a_short_card_over_a_black_world_keeps_no_cut_moved_up() {
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, side) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
        let corner = (left - scale(23.0), top - scale(2.0));
        let bottom = top + side - 1;
        for k in [1.0, 8.0, 16.0] {
            let k = scale(k);
            let mut img =
                range_frame_around(w, h, card, u, RangePatch::Bare, (24, 0), 30.0).to_rgb8();
            for y in bottom - k..bottom {
                for x in left..left + side {
                    img.put_pixel(x, y, Rgb([17 + ((x + y) % 5) as u8; 3]));
                }
            }
            let got = snap_to_card(&DynamicImage::ImageRgb8(img), corner);
            let want = Err(SnapMiss::FarEdge {
                cut: card,
                right: None,
                bottom: Some(bottom - k),
            });
            assert_eq!(got, want, "{name} {k} px short");
        }
    }
}
