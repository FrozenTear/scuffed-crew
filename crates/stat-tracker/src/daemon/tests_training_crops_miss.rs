//! End-to-end miss arms of `hero_icon_crop`: a FarEdge, OuterTop or
//! NearEdge frame with a normal-sized box (under the 256 px edge cap)
//! queues no `hero_icon` crop on the capture path. The existing NearEdge
//! capture test uses a 300 px box, which the edge cap would refuse even
//! if that arm fell back to the read's box.

use super::*;

/// A square box of `side` px at `(x, y)`, under the 256 px edge cap in
/// these tests (171 at 1080p, 228 at 1440p, 256 at 1620p).
fn square_box(x: u32, y: u32, side: u32) -> (usize, usize, usize, usize) {
    assert!(side <= 256, "the box itself must be under the edge cap");
    let (x, y, side) = (x as usize, y as usize, side as usize);
    (x, y, x + side, y + side)
}

/// Record one Tab through `handle_capture` and return whether a
/// `hero_icon` crop was queued. The frame must still carry
/// `tab_frame`'s stat labels so the digit crops can mark the archive
/// task done.
async fn capture_queued_hero_icon(
    frame: DynamicImage,
    box_px: (usize, usize, usize, usize),
    session: &str,
) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    set_consent(dir.path(), true, true);
    let mut analysis = panel_tab(box_px);
    analysis.frame = frame;
    let (recorded, rows) = record_tab(&ctx, session, analysis, sure_board()).await;
    assert!(recorded, "the Tab records whatever happens to its crops");
    assert_eq!(rows, 1);
    let digits = || sync::queued_crops(dir.path(), UploadKind::StatDigits).len() == 6;
    assert!(wait_for(digits).await, "digit crops still run");
    assert_eq!(sync::training_warning(dir.path()), None);
    !sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty()
}

/// `tab_frame` with the card's last two rows painted as gap, so the
/// bottom drop is 2 px early: Near edges still find the card, the far
/// edge check returns FarEdge.
fn tab_frame_card_two_px_short() -> DynamicImage {
    let mut frame = tab_frame();
    let (left, top, side) = TAB_CARD;
    let img = frame.as_mut_rgb8().unwrap();
    for y in top + side - 2..top + side {
        for x in left..left + side {
            img.put_pixel(x, y, Rgb([GAP_LUMA; 3]));
        }
    }
    frame
}

/// `tab_frame` with a lit band over the card's right outer columns
/// only, up to the card top. The mid-band and left outer still find the
/// card top, the far edges hold, and the right outer does not: OuterTop.
fn tab_frame_right_outer_band() -> DynamicImage {
    let mut frame = tab_frame();
    let (left, top, side) = TAB_CARD;
    let img = frame.as_mut_rgb8().unwrap();
    for y in top.saturating_sub(43)..top {
        for x in left + side * 3 / 4..left + side {
            img.put_pixel(x, y, Rgb([45; 3]));
        }
    }
    frame
}

#[tokio::test]
async fn a_nearedge_frame_with_a_normal_box_queues_no_hero_icon_crop() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let frame = tab_frame();
    let (left, top, side) = TAB_CARD;
    // 110 px left of and above the card, the same corner as the existing
    // miss test, but a 171 px box (the card's side), not 300 px.
    let (x, y) = (left - 110, top - 110);
    let box_px = square_box(x, y, side);
    assert_eq!(snap_to_card(&frame, (x, y)), Err(SnapMiss::NearEdge));
    assert!(
        !capture_queued_hero_icon(frame, box_px, "tc-miss-near").await,
        "NearEdge must not fall back to the read's box"
    );
}

#[tokio::test]
async fn a_faredge_frame_with_a_normal_box_queues_no_hero_icon_crop() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let frame = tab_frame_card_two_px_short();
    let (left, top, side) = TAB_CARD;
    let box_px = square_box(left - 10, top - 10, side);
    let got = snap_to_card(&frame, (left - 10, top - 10));
    assert!(
        matches!(got, Err(SnapMiss::FarEdge { cut, .. }) if cut == TAB_CARD),
        "{got:?}"
    );
    assert!(
        !capture_queued_hero_icon(frame, box_px, "tc-miss-far").await,
        "FarEdge must not keep the rejected cut"
    );
}

#[tokio::test]
async fn an_outertop_frame_with_a_normal_box_queues_no_hero_icon_crop() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let frame = tab_frame_right_outer_band();
    let (left, top, side) = TAB_CARD;
    let box_px = square_box(left - 10, top - 10, side);
    let got = snap_to_card(&frame, (left - 10, top - 10));
    assert!(
        matches!(got, Err(SnapMiss::OuterTop { cut, .. }) if cut == TAB_CARD),
        "{got:?}"
    );
    assert!(
        !capture_queued_hero_icon(frame, box_px, "tc-miss-outer").await,
        "OuterTop must not keep the rejected cut"
    );
}

/// A lit band over the right outer columns only: the left outer and the
/// mid-band still hold, the right outer does not.
#[test]
fn a_right_outer_band_alone_keeps_no_cut() {
    let (w, h) = (2560, 1440);
    let card = (1621, 209, 228);
    let (left, top, side) = card;
    let bare = card_frame(w, h, card, GAP_LUMA, 60);
    let frame = DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        if (top - 43..top).contains(&y) && (left + side * 3 / 4..left + side).contains(&x) {
            Rgb([45; 3])
        } else {
            *bare.as_rgb8().unwrap().get_pixel(x, y)
        }
    }));
    let got = snap_to_card(&frame, (left - 23, top - 2));
    // Left outer holds at the card top; the right outer steps at the
    // start of the lit band (top - 43).
    let tops = [Some(top), Some(top - 43)];
    assert_eq!(got, Err(SnapMiss::OuterTop { cut: card, tops }));
}

/// The outer-top 1 px tolerance: a right-outer step 1 px above the cut
/// still holds; 2 px above is OuterTop. The left outer stays on the card.
#[test]
fn an_outer_top_two_px_off_keeps_no_cut() {
    let (w, h) = (2560, 1440);
    let card = (1621, 209, 228);
    let (left, top, side) = card;
    let bare = card_frame(w, h, card, GAP_LUMA, 60);
    let paint = |early: u32| {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            let band = (left + side * 3 / 4..left + side).contains(&x);
            if band && (top - early..top).contains(&y) {
                Rgb([80; 3])
            } else {
                *bare.as_rgb8().unwrap().get_pixel(x, y)
            }
        }))
    };
    let corner = (left - 23, top - 2);
    assert_eq!(snap_to_card(&paint(1), corner), Ok(card));
    let got = snap_to_card(&paint(2), corner);
    assert!(
        matches!(got, Err(SnapMiss::OuterTop { cut, .. }) if cut == card),
        "{got:?}"
    );
}

/// The far-right 1 px tolerance, pinned on its own: the bottom drop
/// stays on the cut, and only the card's right edge moves.
#[test]
fn a_right_drop_two_px_past_the_cut_keeps_none() {
    let (w, h) = (2560, 1440);
    let card = (1621, 209, 228);
    let (left, top, side) = card;
    let frame = |extra: u32| {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            let on_card = in_square(card, x, y)
                || ((left + side..left + side + extra).contains(&x)
                    && (top..top + side).contains(&y));
            if on_card {
                card_pixel(x, y, 60)
            } else {
                Rgb([GAP_LUMA; 3])
            }
        }))
    };
    let corner = (left - 23, top - 2);
    assert_eq!(snap(&frame(1), corner), Some(card));
    let got = snap_to_card(&frame(2), corner);
    assert!(
        matches!(got, Err(SnapMiss::FarEdge { cut, .. }) if cut == card),
        "{got:?}"
    );
}

/// At game height 1620 the card side is exactly 256, the edge cap's
/// inclusive bound (`>` not `>=`). A 1624-high frame is already over.
#[tokio::test]
async fn a_card_at_the_256_px_edge_cap_is_still_kept() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let (w, h) = (2880, 1620);
    let scale = |v: f64| (v * f64::from(h) / 1440.0).round() as u32;
    let card = (scale(1621.0), scale(209.0), 256);
    let frame = card_frame(w, h, card, GAP_LUMA, 60);
    let (left, top, side) = card;
    let read = panel(square_box(left - 23, top - 2, side), "ana", 0.97);
    assert_eq!(snap(&frame, (left - 23, top - 2)), Some(card));
    assert!(
        run(
            dir.path(),
            "tc-cap-256",
            &frame,
            source(Some(&read), Some("ana"))
        )
        .await
    );
    let queued = sync::queued_crops(dir.path(), UploadKind::HeroIcon);
    assert_eq!(queued.len(), 1);
    assert_eq!(sync::png_size(&queued[0].png), Some((256, 256)));
    assert_eq!(sync::training_warning(dir.path()), None);
}
