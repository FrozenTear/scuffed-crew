//! `hero_icon` and `stat_digits` Tab crops: their `meta`, and how
//! [`keep_tab_crops`] keeps, skips and warns.

use scuffed_types::api::training_crops::{TrainingCropConsent, TrainingCropUploadMeta};

use super::tests_training::{all_consent, consent, png};
use super::*;

#[test]
fn hero_icon_meta_has_the_hero_key_the_panel_scores_and_no_token() {
    let meta = hero_icon_meta(2560, 1440, "Soldier: 76", 0.97, 0.5, "heroes-v2").unwrap();
    meta.validate().unwrap();
    assert_eq!(meta.hero.as_deref(), Some("soldier-76"));
    assert_eq!(meta.screen.as_deref(), Some("tab"));
    assert_eq!(meta.slot_type.as_deref(), Some("career_panel"));
    assert_eq!(meta.icon_set_version.as_deref(), Some("heroes-v2"));
    assert_eq!(
        (meta.screen_width, meta.screen_height),
        (Some(2560), Some(1440))
    );
    let (score, runner_up, margin) = (
        meta.score.unwrap(),
        meta.runner_up.unwrap(),
        meta.margin.unwrap(),
    );
    assert!((score - 0.97).abs() < 1e-6, "{score}");
    assert!((margin - 0.5).abs() < 1e-6, "{margin}");
    assert!((runner_up - 0.47).abs() < 1e-6, "{runner_up}");
    assert!(meta.game_token.is_none(), "keep_tab_crops adds it");
    assert!(meta.found_spot.is_none() && meta.found_confidence.is_none());

    // No known hero, no meta: the server requires one for this kind.
    assert!(hero_icon_meta(2560, 1440, "Unknown", 0.97, 0.5, "heroes-v2").is_none());
    assert!(hero_icon_meta(2560, 1440, "Not A Hero", 0.97, 0.5, "heroes-v2").is_none());
    // A score that is not a number is left out, not sent.
    let odd = hero_icon_meta(2560, 1440, "ana", f32::NAN, 0.5, "heroes-v2").unwrap();
    assert_eq!((odd.score, odd.runner_up), (None, None));
    odd.validate().unwrap();
}

#[test]
fn stat_digits_meta_names_the_column_and_the_hero_when_known() {
    for (col, name) in STAT_DIGIT_COLUMNS.iter().enumerate() {
        let meta = stat_digits_meta(1920, 1080, col, "Ana").unwrap();
        meta.validate().unwrap();
        assert_eq!(meta.slot_type.as_deref(), Some(*name));
        assert_eq!(meta.hero.as_deref(), Some("ana"));
        assert_eq!(meta.screen.as_deref(), Some("tab"));
        assert_eq!(
            (meta.screen_width, meta.screen_height),
            (Some(1920), Some(1080))
        );
        assert!(meta.game_token.is_none() && meta.found_spot.is_none());
    }
    assert!(
        stat_digits_meta(1920, 1080, 0, "Unknown")
            .unwrap()
            .hero
            .is_none()
    );
    assert!(stat_digits_meta(1920, 1080, 6, "Ana").is_none());
}

fn hero_meta() -> TrainingCropUploadMeta {
    hero_icon_meta(1920, 1080, "ana", 0.97, 0.5, "heroes-v2").unwrap()
}

#[test]
fn tab_crops_are_kept_with_the_game_token_only_with_consent() {
    let dir = tempfile::tempdir().unwrap();
    // No consent read yet: nothing is read or written.
    let kept = keep_tab_crops(
        dir.path(),
        "s1",
        UploadKind::HeroIcon,
        vec![(hero_meta(), png(200, 200, false))],
    );
    assert_eq!(kept, Some(0));
    assert!(!dir.path().join("training_crops").exists());

    // stat_digits on, hero_icon off: the hero icon is still not kept.
    consent(
        dir.path(),
        TrainingCropConsent {
            stat_digits: true,
            ..TrainingCropConsent::default()
        },
    );
    let crops = vec![(hero_meta(), png(200, 200, false))];
    assert_eq!(
        keep_tab_crops(dir.path(), "s1", UploadKind::HeroIcon, crops),
        Some(0)
    );
    assert!(!has_queued_crops(dir.path()));

    consent(dir.path(), all_consent());
    let crops = vec![(hero_meta(), png(200, 200, false))];
    assert_eq!(
        keep_tab_crops(dir.path(), "s1", UploadKind::HeroIcon, crops),
        Some(1)
    );
    let queued = queued_crops(dir.path(), UploadKind::HeroIcon);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].meta.game_token, game_token_for(dir.path(), "s1"));
    assert_eq!(queued[0].meta.hero.as_deref(), Some("ana"));
    assert_eq!(training_warning(dir.path()), None);
}

#[test]
fn an_over_cap_or_heroless_crop_is_skipped_without_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    // 257 px on an edge: over the cap, skipped, never shrunk.
    let wide = vec![(hero_meta(), png(257, 200, false))];
    assert_eq!(
        keep_tab_crops(dir.path(), "s1", UploadKind::HeroIcon, wide),
        Some(0)
    );
    // Over 64 KiB: skipped.
    let heavy = vec![(hero_meta(), png(256, 256, true))];
    assert_eq!(
        keep_tab_crops(dir.path(), "s1", UploadKind::HeroIcon, heavy),
        Some(0)
    );
    // A hero icon without a hero: skipped.
    let heroless = TrainingCropUploadMeta {
        hero: None,
        ..hero_meta()
    };
    let crops = vec![(heroless, png(200, 200, false))];
    assert_eq!(
        keep_tab_crops(dir.path(), "s1", UploadKind::HeroIcon, crops),
        Some(0)
    );
    assert!(!has_queued_crops(dir.path()));
    assert_eq!(
        training_warning(dir.path()),
        None,
        "a cap skip is not an error"
    );

    // The good crops of one set are kept beside a skipped one.
    let cells = vec![
        (
            stat_digits_meta(1920, 1080, 0, "Ana").unwrap(),
            png(40, 50, false),
        ),
        (
            stat_digits_meta(1920, 1080, 1, "Ana").unwrap(),
            png(300, 50, false),
        ),
        (
            stat_digits_meta(1920, 1080, 3, "Ana").unwrap(),
            png(60, 50, false),
        ),
    ];
    assert_eq!(
        keep_tab_crops(dir.path(), "s1", UploadKind::StatDigits, cells),
        Some(2)
    );
    let mut slots: Vec<_> = queued_crops(dir.path(), UploadKind::StatDigits)
        .into_iter()
        .map(|c| c.meta.slot_type.unwrap())
        .collect();
    slots.sort();
    assert_eq!(slots, ["damage", "elims"]);
}

#[test]
fn a_consented_crop_lost_to_an_error_leaves_the_training_warning() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    // The outbox folder cannot be made: a file sits where it goes.
    let outbox = dir.path().join("training_crops").join("outbox");
    std::fs::create_dir_all(&outbox).unwrap();
    std::fs::write(outbox.join("hero_icon"), b"not a folder").unwrap();
    let crops = vec![(hero_meta(), png(200, 200, false))];
    assert_eq!(
        keep_tab_crops(dir.path(), "s1", UploadKind::HeroIcon, crops),
        None
    );
    let warning = training_warning(dir.path()).expect("a warning for the app");
    assert!(warning.contains("hero icons"), "{warning}");
    assert!(read_training_status(dir.path()).unwrap().sticky);

    // No game token (its file cannot be written): the same warning.
    let other = tempfile::tempdir().unwrap();
    consent(other.path(), all_consent());
    std::fs::create_dir_all(other.path().join("training_crops").join("game_tokens.json")).unwrap();
    let crops = vec![(
        stat_digits_meta(1920, 1080, 0, "Ana").unwrap(),
        png(40, 50, false),
    )];
    assert_eq!(
        keep_tab_crops(other.path(), "s1", UploadKind::StatDigits, crops),
        None
    );
    let warning = training_warning(other.path()).expect("a warning for the app");
    assert!(warning.contains("stat digits"), "{warning}");
    assert!(!has_queued_crops(other.path()));
}
