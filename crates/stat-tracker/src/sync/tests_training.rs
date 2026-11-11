//! Training crops without HTTP: the kinds, caps, `meta`, multipart body
//! and the outbox.

use scuffed_types::api::training_crops::{TrainingCropConsent, TrainingCropUploadMeta};
use scuffed_types::{CropKind, HERO_ICON_MAX_REQUEST_BYTES};

use super::training_store::write_cached_consent;
use super::*;

/// A real PNG of `width` by `height`. `noisy` fills it with pixels that do
/// not compress, to reach the byte caps.
pub(super) fn png(width: u32, height: u32, noisy: bool) -> Vec<u8> {
    let mut seed: u32 = 0x9e37_79b9;
    let img = image::RgbaImage::from_fn(width, height, |_, _| {
        if !noisy {
            return image::Rgba([10, 20, 30, 255]);
        }
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        image::Rgba(seed.to_le_bytes())
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

pub(super) fn token() -> String {
    random_token().expect("random token")
}

pub(super) fn crop(meta: TrainingCropUploadMeta, png: Vec<u8>) -> QueuedCrop {
    QueuedCrop {
        id: "0000000000001-00000000".into(),
        meta,
        png,
    }
}

pub(super) fn all_consent() -> TrainingCropConsent {
    TrainingCropConsent {
        hero_icon: true,
        stat_digits: true,
        map_name: true,
    }
}

pub(super) fn consent(dir: &std::path::Path, value: TrainingCropConsent) {
    write_cached_consent(dir, value).unwrap();
}

#[test]
fn no_upload_kind_is_a_whole_screen() {
    assert_eq!(UploadKind::ALL.len(), 3);
    for kind in UploadKind::ALL {
        assert_ne!(kind.crop_kind(), CropKind::FullScreen);
        assert!(kind.crop_kind().accepts_uploads());
        assert_eq!(UploadKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(UploadKind::parse("full_screen"), None);
    assert_eq!(UploadKind::parse("colorblind_filter"), None);
}

#[test]
fn map_name_meta_has_a_random_token_the_size_and_nothing_else() {
    let t = token();
    assert!(game_token_ok(&t), "{t}");
    let meta = map_name_meta(&t, 2560, 1440);
    meta.validate().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&meta_json(&meta)).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "screen": "tab",
            "game_token": t,
            "screen_width": 2560,
            "screen_height": 1440,
        })
    );
    // A size the server would refuse is left out, not sent.
    let small = map_name_meta(&t, 200, 100);
    assert_eq!((small.screen_width, small.screen_height), (None, None));
    small.validate().unwrap();
}

#[test]
fn meta_json_has_no_nulls_and_no_colorblind_filter() {
    let meta = TrainingCropUploadMeta {
        hero: hero_field("Soldier: 76"),
        game_token: Some(token()),
        ..TrainingCropUploadMeta::default()
    };
    let text = String::from_utf8(meta_json(&meta)).unwrap();
    assert!(!text.contains("null"), "{text}");
    assert!(!text.contains("colorblind"), "{text}");
    assert!(!text.contains("found_spot"), "{text}");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["hero"], "soldier-76");
    assert!(game_token_ok(json["game_token"].as_str().unwrap()));
    // The server's own parser takes it.
    let back: TrainingCropUploadMeta = serde_json::from_str(&text).unwrap();
    assert_eq!(back, meta);
}

#[test]
fn hero_is_sent_as_the_hero_key_or_not_at_all() {
    assert_eq!(hero_field("Soldier: 76").as_deref(), Some("soldier-76"));
    assert_eq!(hero_field("Lúcio").as_deref(), Some("lucio"));
    assert_eq!(hero_field("ana").as_deref(), Some("ana"));
    assert_eq!(hero_field("Unknown"), None);
    assert_eq!(hero_field("Not A Hero"), None);
}

#[test]
fn game_tokens_are_random_per_game_and_stable_within_one() {
    let dir = tempfile::tempdir().unwrap();
    let a = game_token_for(dir.path(), "2026-11-10T18:04:05Z-s1").unwrap();
    let again = game_token_for(dir.path(), "2026-11-10T18:04:05Z-s1").unwrap();
    let b = game_token_for(dir.path(), "2026-11-10T18:30:00Z-s2").unwrap();
    assert_eq!(a, again, "both crops of a game share one token");
    assert_ne!(a, b);
    for t in [&a, &b] {
        assert!(game_token_ok(t), "{t}");
    }
    assert_ne!(token(), token());
    assert!(!game_token_ok("0123456789ABCDEF0123456789ABCDEF"));
    assert!(!game_token_ok("3f2a9c0e-6b1d-4e8f-9a7c-5b3d1e0f"));
}

#[test]
fn png_size_reads_the_header() {
    assert_eq!(png_size(&png(40, 16, false)), Some((40, 16)));
    assert_eq!(png_size(b"GIF89a not a png at all"), None);
    assert_eq!(png_size(&[]), None);
}

#[test]
fn caps_are_checked_per_kind_before_sending() {
    let t = token();
    let meta = map_name_meta(&t, 1920, 1080);
    let ok = |kind, w, h| check_crop(kind, &crop(meta.clone(), png(w, h, false)));
    // map_name: 8 to 1536 wide, 8 to 256 tall.
    assert_eq!(ok(UploadKind::MapName, 1536, 256), Ok(()));
    assert_eq!(ok(UploadKind::MapName, 8, 8), Ok(()));
    for (w, h) in [(1537, 40), (400, 257), (7, 40), (400, 7)] {
        assert_eq!(
            ok(UploadKind::MapName, w, h),
            Err(CropRefusal::Edges {
                width: w,
                height: h
            })
        );
    }
    // stat_digits: 8 to 256 each.
    assert_eq!(ok(UploadKind::StatDigits, 256, 256), Ok(()));
    assert!(matches!(
        ok(UploadKind::StatDigits, 257, 16),
        Err(CropRefusal::Edges { .. })
    ));
    // Bytes: 64 KiB for stat digits and hero icons, 256 KiB for map names.
    let big = png(200, 200, true);
    assert!(
        big.len() > 64 * 1024 && big.len() <= 256 * 1024,
        "{}",
        big.len()
    );
    assert_eq!(
        check_crop(UploadKind::StatDigits, &crop(meta.clone(), big.clone())),
        Err(CropRefusal::TooLarge(big.len()))
    );
    assert_eq!(
        check_crop(UploadKind::MapName, &crop(meta.clone(), big)),
        Ok(())
    );
    let huge = png(1400, 256, true);
    assert!(huge.len() > 256 * 1024);
    assert_eq!(
        check_crop(UploadKind::MapName, &crop(meta.clone(), huge.clone())),
        Err(CropRefusal::TooLarge(huge.len()))
    );
    // Not a PNG.
    assert_eq!(
        check_crop(UploadKind::MapName, &crop(meta.clone(), b"nope".to_vec())),
        Err(CropRefusal::NotPng)
    );
    // A hero icon needs a hero; a bad token is refused like the server would.
    assert_eq!(
        check_crop(
            UploadKind::HeroIcon,
            &crop(meta.clone(), png(64, 64, false))
        ),
        Err(CropRefusal::NoHero)
    );
    let bad = TrainingCropUploadMeta {
        game_token: Some("not-a-token".into()),
        ..meta
    };
    assert_eq!(
        check_crop(UploadKind::MapName, &crop(bad, png(64, 64, false))),
        Err(CropRefusal::Meta("game_token"))
    );
}

#[test]
fn a_batch_holds_at_most_20_pairs_and_2_mib() {
    let meta = map_name_meta(&token(), 1920, 1080);
    let small: Vec<QueuedCrop> = (0..25)
        .map(|_| crop(meta.clone(), png(64, 16, false)))
        .collect();
    assert_eq!(batch_len(&small), 20);
    assert_eq!(batch_len(&[]), 0);

    let banner = png(1100, 40, true);
    assert!(banner.len() > 150 * 1024 && banner.len() <= 256 * 1024);
    let large: Vec<QueuedCrop> = (0..20)
        .map(|_| crop(meta.clone(), banner.clone()))
        .collect();
    let n = batch_len(&large);
    assert!((1..20).contains(&n), "{n}");
    let body = multipart_body("b", &large[..n]);
    assert!(body.len() <= HERO_ICON_MAX_REQUEST_BYTES, "{}", body.len());
    let one_more = multipart_body("b", &large[..n + 1]);
    assert!(one_more.len() > HERO_ICON_MAX_REQUEST_BYTES - 1024);
}

/// The part names of a multipart body, in order, and the closing line.
pub(super) fn part_names(body: &[u8], boundary: &str) -> (Vec<String>, bool) {
    let text = String::from_utf8_lossy(body);
    let names = text
        .split(&format!("--{boundary}\r\n"))
        .skip(1)
        .filter_map(|part| {
            let rest = part.split("name=\"").nth(1)?;
            Some(rest.split('"').next()?.to_string())
        })
        .collect();
    (names, text.ends_with(&format!("--{boundary}--\r\n")))
}

#[test]
fn the_body_is_meta_then_crop_for_each_pair() {
    let meta = map_name_meta(&token(), 1920, 1080);
    let crops = vec![
        crop(meta.clone(), png(64, 16, false)),
        crop(meta.clone(), png(80, 16, false)),
    ];
    let body = multipart_body("BOUND", &crops);
    let (names, closed) = part_names(&body, "BOUND");
    assert_eq!(names, ["meta", "crop", "meta", "crop"]);
    assert!(closed);
    let text = String::from_utf8_lossy(&body);
    assert_eq!(text.matches("Content-Type: application/json").count(), 2);
    assert_eq!(text.matches("Content-Type: image/png").count(), 2);
    assert!(!text.contains("full_screen"));
    // The PNG bytes are in the body as they are.
    let png0 = &crops[0].png;
    assert!(body.windows(png0.len()).any(|w| w == png0.as_slice()));
}

#[test]
fn nothing_is_kept_without_consent() {
    let dir = tempfile::tempdir().unwrap();
    let meta = map_name_meta(&token(), 1920, 1080);
    assert!(!keep_training_crop(
        dir.path(),
        UploadKind::MapName,
        meta.clone(),
        png(400, 40, false)
    ));
    assert!(!dir.path().join("training_crops").join("outbox").exists());

    // Consent for another kind does not count.
    consent(
        dir.path(),
        TrainingCropConsent {
            stat_digits: true,
            ..TrainingCropConsent::default()
        },
    );
    assert!(!keep_training_crop(
        dir.path(),
        UploadKind::MapName,
        meta.clone(),
        png(400, 40, false)
    ));
    let file = dir.path().join("map.png");
    std::fs::write(&file, png(400, 40, false)).unwrap();
    assert!(!keep_map_name_crop(dir.path(), "s1", &file, 1920, 1080));
    assert!(!has_queued_crops(dir.path()));
}

#[test]
fn the_outbox_keeps_consented_crops_private_and_capped() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    let file = dir.path().join("map.png");
    std::fs::write(&file, png(400, 40, false)).unwrap();
    assert!(keep_map_name_crop(dir.path(), "s1", &file, 1920, 1080));
    let queued = queued_crops(dir.path(), UploadKind::MapName);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].png, std::fs::read(&file).unwrap());
    assert_eq!(
        queued[0].meta.game_token,
        game_token_for(dir.path(), "s1"),
        "the game's token"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let outbox = dir.path().join("training_crops/outbox/map_name");
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&outbox), 0o700);
        for entry in std::fs::read_dir(&outbox).unwrap() {
            assert_eq!(mode(&entry.unwrap().path()), 0o600);
        }
    }
    // An over-cap crop is not kept.
    assert!(!keep_training_crop(
        dir.path(),
        UploadKind::MapName,
        map_name_meta(&token(), 1920, 1080),
        png(2000, 40, false)
    ));
    // Each crop carries its own token here, so the order they were kept in
    // is known. Ids sort by the millisecond they were kept: a short sleep
    // keeps two crops out of one millisecond.
    let mut kept_order = vec![game_token_for(dir.path(), "s1").unwrap()];
    for _ in 0..OUTBOX_KEEP_PER_KIND + 5 {
        std::thread::sleep(std::time::Duration::from_millis(2));
        let t = token();
        assert!(keep_training_crop(
            dir.path(),
            UploadKind::MapName,
            map_name_meta(&t, 1920, 1080),
            png(400, 40, false)
        ));
        kept_order.push(t);
    }
    let queued = queued_crops(dir.path(), UploadKind::MapName);
    assert_eq!(queued.len(), OUTBOX_KEEP_PER_KIND);
    let left: Vec<String> = queued
        .iter()
        .map(|c| c.meta.game_token.clone().unwrap())
        .collect();
    let pruned = kept_order.len() - OUTBOX_KEEP_PER_KIND;
    assert_eq!(pruned, 6);
    assert_eq!(
        left,
        kept_order[pruned..],
        "the first crops kept are the ones pruned, the rest wait in order"
    );

    clear_training_outbox(dir.path());
    assert!(!has_queued_crops(dir.path()));
    assert_eq!(cached_training_consent(dir.path()), all_consent());
    forget_training_crops(dir.path());
    assert_eq!(
        cached_training_consent(dir.path()),
        TrainingCropConsent::default()
    );
}

#[test]
fn the_app_has_no_consent_put() {
    // Consent is set on the site with a session only; a token PUT is 401.
    // No training crop code may send one, so a 401 there can never be read
    // as a dead token.
    for (name, source) in [
        ("training_client.rs", include_str!("training_client.rs")),
        ("training_pass.rs", include_str!("training_pass.rs")),
        ("training_store.rs", include_str!("training_store.rs")),
        ("training.rs", include_str!("training.rs")),
    ] {
        assert!(!source.contains(".put("), "{name}");
        assert!(!source.contains("Method::PUT"), "{name}");
    }
}
