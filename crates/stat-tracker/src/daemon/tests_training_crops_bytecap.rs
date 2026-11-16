//! Byte-cap follow-up for `hero_icon`: a snapped card whose Best-encoded
//! PNG is over the 65,536-byte cap is downscaled to the 1080p card side
//! (171 px) so the stored PNG fits, and a refused crop does not mark the
//! hero offered, so a later Tab of the same game can retry.

use super::super::training_crops::{
    HERO_ICON_BYTE_FALLBACK_SIDE, HERO_ICON_BYTE_SHRINK_FLOOR, HeroIconEncodeMiss, card_side,
    encode_hero_icon, encode_png, keep_encoded_hero_icon, keep_hero_icon_result,
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
    assert_eq!(
        encoded, at_fallback,
        "the first fitting side is the Lanczos3 171 px encode, not another filter or side"
    );
    assert_eq!(
        sync::png_size(&encoded),
        Some((171, 171)),
        "the first shrink step is 171 px, not a later rung"
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
    assert_eq!(
        queued[0].png, at_fallback,
        "the stored PNG is the Lanczos3 171 px encode"
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

/// A pure-noise 228 px card is still over the cap at 171 and 155, so the
/// shrink loop must take the third step and land at
/// [`HERO_ICON_BYTE_SHRINK_FLOOR`] (139). Kills a mutant that gives up
/// after the first fallback side, or that downscales with Nearest instead
/// of Lanczos3.
#[test]
fn a_pure_noise_228_card_lands_at_the_shrink_floor() {
    let card = noise_card(228);
    let native = encode_png(&card).expect("the fixture encodes");
    assert!(
        native.len() > cap(),
        "the fixture itself must be over the cap, got {} bytes",
        native.len()
    );
    for side in [HERO_ICON_BYTE_FALLBACK_SIDE, 155] {
        let png = encode_png(&card.resize_exact(side, side, image::imageops::FilterType::Lanczos3))
            .expect("the mid-step copy encodes");
        assert!(
            png.len() > cap(),
            "side {side} must still be over the cap so the loop continues, got {} bytes",
            png.len()
        );
    }
    let at_floor = encode_png(&card.resize_exact(
        HERO_ICON_BYTE_SHRINK_FLOOR,
        HERO_ICON_BYTE_SHRINK_FLOOR,
        image::imageops::FilterType::Lanczos3,
    ))
    .expect("the 139 px copy encodes");
    assert!(
        at_floor.len() <= cap(),
        "the 139 px copy must fit, got {} bytes",
        at_floor.len()
    );

    let encoded = encode_hero_icon(&card).expect("the over-cap card shrinks to the floor");
    assert_eq!(
        encoded, at_floor,
        "the stored PNG is the Lanczos3 139 px encode"
    );
    assert_eq!(sync::png_size(&encoded), Some((139, 139)));
}

/// Incompressible RGB so Best PNG stays close to the raw pixel count:
/// 171 and 155 stay over the cap, 139 (raw 57,963 B) fits.
fn noise_card(side: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(side, side, |x, y| {
        let n = x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663) ^ x.wrapping_mul(y);
        Rgb([
            (n & 0xff) as u8,
            ((n >> 8) & 0xff) as u8,
            ((n >> 16) & 0xff) as u8,
        ])
    }))
}

#[test]
fn the_byte_fallback_side_is_the_1080p_card_side() {
    assert_eq!(card_side(1920, 1080), HERO_ICON_BYTE_FALLBACK_SIDE);
    assert_eq!(HERO_ICON_BYTE_FALLBACK_SIDE, 171);
    assert_eq!(HERO_ICON_BYTE_SHRINK_FLOOR, 139);
}

/// Grow a valid PNG to exactly `len` bytes by padding after the file
/// (the header, and so [`sync::png_size`], is unchanged).
fn pad_png(mut png: Vec<u8>, len: usize) -> Vec<u8> {
    assert!(
        png.len() <= len,
        "base PNG is already {} bytes, cannot pad to {len}",
        png.len()
    );
    png.resize(len, 0);
    png
}

/// A `hero_icon` of exactly 65,536 bytes is kept; one of 65,537 is refused
/// and does not mark the hero offered, so a later Tab can retry. Kills
/// `>` becoming `>=` in `check_crop`, and a cap of cap + 1.
#[tokio::test]
async fn a_hero_icon_at_exactly_the_byte_cap_is_kept_and_one_byte_over_is_refused() {
    let cap = cap();
    assert_eq!(cap, 65_536);
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let session = "tc-bytecap-exact";
    let meta = sync::hero_icon_meta(2560, 1440, "ana", 0.97, 0.5, "heroes-v2").unwrap();
    let at_cap = pad_png(encode_png(&noisy_card(64)).expect("base encodes"), cap);
    assert_eq!(at_cap.len(), 65_536);
    assert_eq!(sync::png_size(&at_cap), Some((64, 64)));
    keep_encoded_hero_icon(dir.path(), session, "ana", meta.clone(), at_cap.clone());
    let queued = sync::queued_crops(dir.path(), UploadKind::HeroIcon);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].png.len(), 65_536);
    assert_eq!(queued[0].png, at_cap);
    assert_eq!(sync::training_warning(dir.path()), None);

    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let session = "tc-bytecap-capplus1";
    let over = pad_png(encode_png(&noisy_card(64)).expect("base encodes"), cap + 1);
    assert_eq!(over.len(), 65_537);
    keep_encoded_hero_icon(dir.path(), session, "ana", meta, over);
    assert!(
        sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty(),
        "one byte over the cap must not be stored"
    );
    assert_eq!(sync::training_warning(dir.path()), None);
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
        "a cap+1 refusal must not mark the hero offered"
    );
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::HeroIcon).len(),
        1
    );
}

/// A 228 px card that is still over the cap at 171 px but fits at 155 px
/// must be stored at 155, the second shrink step. Kills a 32 px step
/// (171 then 139) and a floor that skips this rung.
#[test]
fn a_card_over_the_cap_at_171_is_stored_at_155() {
    let card = card_over_at_171_under_at_155();
    let at_155 = encode_png(&card.resize_exact(155, 155, image::imageops::FilterType::Lanczos3))
        .expect("the 155 px copy encodes");
    assert!(
        at_155.len() <= cap(),
        "the fixture's 155 px copy must fit, got {} bytes",
        at_155.len()
    );
    let encoded = encode_hero_icon(&card).expect("the card shrinks to 155");
    assert_eq!(
        encoded, at_155,
        "the stored PNG is the Lanczos3 155 px encode"
    );
    assert_eq!(sync::png_size(&encoded), Some((155, 155)));
}

/// A 171 px over-cap card must not stay at 171 or be upscaled; it steps
/// down the ladder and lands at 139. Pins the same-side skip: a 171 px
/// native miss goes to 155, then 139.
#[test]
fn a_171_px_over_cap_card_lands_at_139() {
    let card = noise_card(171);
    let native = encode_png(&card).expect("the 171 px fixture encodes");
    assert!(
        native.len() > cap(),
        "the 171 px fixture must be over the cap, got {} bytes",
        native.len()
    );
    let at_155 = encode_png(&card.resize_exact(155, 155, image::imageops::FilterType::Lanczos3))
        .expect("the 155 px copy encodes");
    assert!(
        at_155.len() > cap(),
        "155 must still be over so the loop reaches 139, got {} bytes",
        at_155.len()
    );
    let at_139 = encode_png(&card.resize_exact(139, 139, image::imageops::FilterType::Lanczos3))
        .expect("the 139 px copy encodes");
    assert!(
        at_139.len() <= cap(),
        "139 must fit, got {} bytes",
        at_139.len()
    );
    let encoded = encode_hero_icon(&card).expect("the 171 px card shrinks to 139");
    assert_eq!(encoded, at_139);
    assert_eq!(sync::png_size(&encoded), Some((139, 139)));
}

/// A card already smaller than 171 px and over the cap is not upscaled.
/// It starts at its own side and lands at 139. Kills dropping
/// `.min(image.width())` (which would first try 171).
#[test]
fn a_155_px_over_cap_card_is_not_upscaled_and_lands_at_139() {
    let card = noise_card(155);
    let native = encode_png(&card).expect("the 155 px fixture encodes");
    assert!(
        native.len() > cap(),
        "the 155 px fixture must be over the cap, got {} bytes",
        native.len()
    );
    let at_171 = encode_png(&card.resize_exact(171, 171, image::imageops::FilterType::Lanczos3))
        .expect("an upscale to 171 encodes");
    // If the shrink loop wrongly starts at 171, it may store this 171 px
    // copy when that encode fits. The own-side path must land at 139.
    let at_139 = encode_png(&card.resize_exact(139, 139, image::imageops::FilterType::Lanczos3))
        .expect("the 139 px copy encodes");
    assert!(
        at_139.len() <= cap(),
        "139 must fit, got {} bytes",
        at_139.len()
    );
    let encoded = encode_hero_icon(&card).expect("the 155 px card shrinks to 139");
    assert_eq!(encoded, at_139);
    assert_eq!(sync::png_size(&encoded), Some((139, 139)));
    assert_ne!(encoded, at_171, "must not store an upscale to 171");
}

/// Mix high-frequency noise into [`noisy_card`] until 171 is over the cap
/// and 155 still fits, so the shrink loop has to take the second step.
fn card_over_at_171_under_at_155() -> DynamicImage {
    let base = noisy_card(228).to_rgb8();
    let noise = noise_card(228).to_rgb8();
    for mix in 1..=32u16 {
        let img = image::RgbImage::from_fn(228, 228, |x, y| {
            let a = base.get_pixel(x, y).0;
            let b = noise.get_pixel(x, y).0;
            image::Rgb([
                ((u16::from(a[0]) * (32 - mix) + u16::from(b[0]) * mix) / 32) as u8,
                ((u16::from(a[1]) * (32 - mix) + u16::from(b[1]) * mix) / 32) as u8,
                ((u16::from(a[2]) * (32 - mix) + u16::from(b[2]) * mix) / 32) as u8,
            ])
        });
        let card = DynamicImage::ImageRgb8(img);
        let native = encode_png(&card).expect("mix encodes");
        if native.len() <= cap() {
            continue;
        }
        let at_171 =
            encode_png(&card.resize_exact(171, 171, image::imageops::FilterType::Lanczos3))
                .expect("171 encodes");
        let at_155 =
            encode_png(&card.resize_exact(155, 155, image::imageops::FilterType::Lanczos3))
                .expect("155 encodes");
        if at_171.len() > cap() && at_155.len() <= cap() {
            return card;
        }
    }
    panic!("no 228 px mix was over the cap at 171 and under at 155");
}

/// An encode failure warns and does not mark the hero offered, so a later
/// Tab can retry. Kills a silent Encode arm, or one that marks offered.
#[tokio::test]
async fn an_encode_miss_warns_and_does_not_mark_the_hero_offered() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let session = "tc-bytecap-encode-miss";
    let meta = sync::hero_icon_meta(1920, 1080, "ana", 0.97, 0.5, "heroes-v2").unwrap();
    keep_hero_icon_result(
        dir.path(),
        session,
        "ana",
        meta,
        Err(HeroIconEncodeMiss::Encode),
    );
    assert!(sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty());
    let warning = sync::training_warning(dir.path()).expect("an encode miss is an error");
    assert!(warning.contains("hero icons"), "{warning}");

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
        "an encode miss must not mark the hero offered"
    );
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::HeroIcon).len(),
        1
    );
}

/// An over-cap miss after shrinking does not warn and does not mark the
/// hero offered, so a later Tab can retry. Kills an OverCap arm that
/// warns or marks offered.
#[tokio::test]
async fn an_overcap_miss_does_not_warn_or_mark_the_hero_offered() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let session = "tc-bytecap-overcap-miss";
    let meta = sync::hero_icon_meta(1920, 1080, "ana", 0.97, 0.5, "heroes-v2").unwrap();
    keep_hero_icon_result(
        dir.path(),
        session,
        "ana",
        meta,
        Err(HeroIconEncodeMiss::OverCap),
    );
    assert!(sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty());
    assert_eq!(
        sync::training_warning(dir.path()),
        None,
        "an over-cap miss is not an error"
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
        "an over-cap miss must not mark the hero offered"
    );
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::HeroIcon).len(),
        1
    );
}
