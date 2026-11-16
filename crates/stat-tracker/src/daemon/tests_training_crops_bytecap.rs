//! Byte-cap follow-up for `hero_icon`: a snapped card whose Best-encoded
//! PNG is over the 65,536-byte cap is downscaled to the 1080p card side
//! (171 px) so the stored PNG fits, and a refused crop does not mark the
//! hero offered, so a later Tab of the same game can retry.

use super::super::training_crops::{
    HERO_ICON_BYTE_FALLBACK_SIDE, encode_hero_icon, encode_png, keep_encoded_hero_icon,
};
use super::*;

/// A 228 px card whose Best-encoded PNG is over the `hero_icon` byte cap,
/// while a Lanczos3 copy at [`HERO_ICON_BYTE_FALLBACK_SIDE`] fits, matching
/// the over-cap Practice Range cards at 1440p.
fn noisy_card(side: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(side, side, |x, y| {
        // Smooth portrait-like gradients (a 171 px Lanczos3 copy still
        // compresses under the cap) plus a fine grain that pushes the
        // 228 px Best encode over it, as on the over-cap Practice Range cards.
        let s = side.max(1);
        let gx = x * 255 / s;
        let gy = y * 255 / s;
        let wave = ((x * 5 + y * 3) / 4) % 64;
        let grain = (x.wrapping_mul(37) + y.wrapping_mul(101) + (x * y) % 17) % 64;
        let hash = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)) % 64;
        Rgb([
            (40 + gx / 3 + wave / 2 + grain / 2).min(255) as u8,
            (70 + gy / 4 + grain + hash / 4).min(255) as u8,
            (90 + gx / 5 + gy / 3 + wave / 3 + hash / 5).min(255) as u8,
        ])
    }))
}

fn cap() -> usize {
    UploadKind::HeroIcon.crop_kind().max_crop_bytes()
}

/// A noisy 228 px card that is over the cap at its own side is stored
/// under the cap at the 1080p fallback side. Kills a mutant that keeps
/// the native 228 px encode (over the cap) or that skips the downscale.
#[tokio::test]
async fn a_noisy_228_card_over_the_byte_cap_is_stored_at_the_1080p_side() {
    let card = noisy_card(228);
    let native = encode_png(&card).expect("the fixture encodes");
    assert!(
        native.len() > cap(),
        "the fixture itself must be over the cap, got {} bytes",
        native.len()
    );
    let fallback = card.resize_exact(
        HERO_ICON_BYTE_FALLBACK_SIDE,
        HERO_ICON_BYTE_FALLBACK_SIDE,
        image::imageops::FilterType::Lanczos3,
    );
    let at_fallback = encode_png(&fallback).expect("the 171 px copy encodes");
    assert!(
        at_fallback.len() <= cap(),
        "the fixture's 171 px copy must fit, got {} bytes",
        at_fallback.len()
    );

    let encoded = encode_hero_icon(&card).expect("the over-cap card shrinks to fit");
    assert!(
        encoded.len() <= cap(),
        "nothing over the cap is stored, got {} bytes",
        encoded.len()
    );
    assert_eq!(
        sync::png_size(&encoded),
        Some((HERO_ICON_BYTE_FALLBACK_SIDE, HERO_ICON_BYTE_FALLBACK_SIDE)),
        "expected the 1080p fallback side"
    );

    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let card_px = (1621, 209, 228);
    let pixels = card.to_rgb8();
    let frame = DynamicImage::ImageRgb8(RgbImage::from_fn(2560, 1440, |x, y| {
        if in_square(card_px, x, y) {
            *pixels.get_pixel(x - 1621, y - 209)
        } else {
            Rgb([GAP_LUMA; 3])
        }
    }));
    let read = panel((1598, 207, 1598 + 252, 207 + 252), "ana", 0.97);
    assert!(
        run(
            dir.path(),
            "tc-bytecap-shrink",
            &frame,
            source(Some(&read), Some("ana"))
        )
        .await
    );
    let queued = sync::queued_crops(dir.path(), UploadKind::HeroIcon);
    assert_eq!(queued.len(), 1);
    assert!(
        queued[0].png.len() <= cap(),
        "stored {} bytes, over the cap",
        queued[0].png.len()
    );
    assert_eq!(
        sync::png_size(&queued[0].png),
        Some((HERO_ICON_BYTE_FALLBACK_SIDE, HERO_ICON_BYTE_FALLBACK_SIDE))
    );
    assert_eq!(
        (queued[0].meta.screen_width, queued[0].meta.screen_height),
        (Some(2560), Some(1440)),
        "meta still records the frame size, not the stored side"
    );
    assert_eq!(sync::training_warning(dir.path()), None);
}

/// A cap refusal through the post-encode keep path does not mark the hero
/// offered, so a later Tab of the same game still queues. Kills a mutant
/// that treats `keep_tab_crops(...).is_some()` as success (`Some(0)` is a
/// refusal).
#[tokio::test]
async fn a_refused_hero_icon_is_not_marked_offered_so_the_next_tab_can_retry() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let session = "tc-bytecap-retry";
    let over = encode_png(&noisy_card(228)).expect("the over-cap fixture encodes");
    assert!(over.len() > cap(), "the refusal PNG must miss the cap");
    let meta = sync::hero_icon_meta(2560, 1440, "ana", 0.97, 0.5, "heroes-v2").unwrap();
    keep_encoded_hero_icon(dir.path(), session, "ana", meta, over);
    assert!(
        sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty(),
        "an over-cap PNG is not stored"
    );
    assert_eq!(
        sync::training_warning(dir.path()),
        None,
        "a cap refusal is not an error"
    );

    let frame = tab_frame();
    let read = panel((1600, 200, 1800, 400), "ana", 0.97);
    assert!(
        run(
            dir.path(),
            session,
            &frame,
            source(Some(&read), Some("ana"))
        )
        .await,
        "the next Tab of the same game must still be able to offer this hero"
    );
    let queued = sync::queued_crops(dir.path(), UploadKind::HeroIcon);
    assert_eq!(queued.len(), 1, "the retry queued a crop");
    assert!(queued[0].png.len() <= cap());
    assert_eq!(sync::png_size(&queued[0].png), Some((171, 171)));
}

/// A 1080p card already under the cap is stored as the native Best encode,
/// byte for byte: no downscale, no second pass. Kills a mutant that always
/// resizes to the fallback side.
#[tokio::test]
async fn a_1080p_card_under_the_cap_is_stored_as_the_native_best_encode() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let frame = tab_frame();
    let (left, top, side) = TAB_CARD;
    assert_eq!(side, HERO_ICON_BYTE_FALLBACK_SIDE);
    let raw = frame.crop_imm(left, top, side, side);
    let native = encode_png(&raw).expect("the 1080p card encodes");
    assert!(
        native.len() <= cap(),
        "the 1080p fixture must already fit, got {} bytes",
        native.len()
    );
    let read = panel((1600, 200, 1800, 400), "ana", 0.97);
    assert!(
        run(
            dir.path(),
            "tc-bytecap-1080",
            &frame,
            source(Some(&read), Some("ana"))
        )
        .await
    );
    let queued = sync::queued_crops(dir.path(), UploadKind::HeroIcon);
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].png, native,
        "1080p must be unchanged: the stored PNG is the native Best encode"
    );
    assert_eq!(decode(&queued[0].png), raw.to_rgb8());
    assert_eq!(
        (queued[0].meta.screen_width, queued[0].meta.screen_height),
        (Some(1920), Some(1080))
    );
}
