//! Two close Tabs of one game: the first to qualify claims the kind before
//! its save starts on the blocking thread, so the second saves nothing
//! even while the first is still saving. A save that keeps nothing
//! releases the claim (consent turned off mid-save, every cell over the
//! byte cap, or a hero with no `hero_icon` meta), so a later Tab retries.

use std::sync::atomic::Ordering;

use super::super::training_crops::{offer_key, offered};
use crate::daemon::{CROP_TASK_GATE, CROP_TASKS_IN_FLIGHT};

use super::*;

/// Wait until no blocking crop task is in flight, up to 10 s. `false` on
/// timeout.
pub(super) async fn crops_settled() -> bool {
    for _ in 0..2000 {
        if CROP_TASKS_IN_FLIGHT.load(Ordering::SeqCst) == 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    false
}

/// Offer each of `tabs` for `session` while every blocking crop task is
/// held at its start, then let them run and wait for all of them. Returns
/// how many offers started a task.
async fn offer_held(
    dir: &Path,
    session: &str,
    frame: &DynamicImage,
    tabs: Vec<TabCropSource<'_>>,
) -> usize {
    let gate = CROP_TASK_GATE.write().unwrap_or_else(|e| e.into_inner());
    let handles: Vec<_> = tabs
        .into_iter()
        .filter_map(|tab| archive_tab_training_crops(dir, session, frame, tab))
        .collect();
    drop(gate);
    let started = handles.len();
    for handle in handles {
        handle.await.unwrap();
    }
    started
}

#[tokio::test]
async fn two_tabs_offered_before_the_first_saves_keep_one_digit_set() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    // Held across both offers, so the first Tab's save has not started
    // when the second Tab is offered, however the threads are scheduled.
    let gate = CROP_TASK_GATE.write().unwrap_or_else(|e| e.into_inner());
    let first = archive_tab_training_crops(dir.path(), "race-digits", &frame, source(None, None));
    let second = archive_tab_training_crops(dir.path(), "race-digits", &frame, source(None, None));
    drop(gate);
    assert!(first.is_some(), "the first Tab claims the set");
    assert!(second.is_none(), "the second Tab finds the set claimed");
    first.unwrap().await.unwrap();
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::StatDigits).len(),
        6,
        "one set"
    );
}

#[tokio::test]
async fn two_tabs_offered_before_the_first_saves_keep_one_hero_icon() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let frame = tab_frame();
    let read = panel((1600, 200, 1800, 400), "ana", 0.97);
    let tabs = vec![
        source(Some(&read), Some("ana")),
        source(Some(&read), Some("ana")),
    ];
    let started = offer_held(dir.path(), "race-hero", &frame, tabs).await;
    assert_eq!(started, 1, "the second Tab finds the hero claimed");
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::HeroIcon).len(),
        1,
        "one icon"
    );
}

#[tokio::test]
async fn a_digit_set_lost_to_a_write_error_is_saved_by_a_later_tab() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    // The stat digits outbox cannot be made: a file sits where it goes.
    let outbox = dir.path().join("training_crops").join("outbox");
    std::fs::create_dir_all(&outbox).unwrap();
    std::fs::write(outbox.join("stat_digits"), b"not a folder").unwrap();
    assert!(run(dir.path(), "race-writefail", &frame, source(None, None)).await);
    assert!(sync::queued_crops(dir.path(), UploadKind::StatDigits).is_empty());
    let warning = sync::training_warning(dir.path()).expect("the member is told");
    assert!(warning.contains("digit"), "{warning}");
    // The write error released the claim: once the outbox can be written,
    // a later Tab of the same game saves the set, and only once.
    std::fs::remove_file(outbox.join("stat_digits")).unwrap();
    assert!(run(dir.path(), "race-writefail", &frame, source(None, None)).await);
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::StatDigits).len(),
        6
    );
    assert!(!run(dir.path(), "race-writefail", &frame, source(None, None)).await);
}

#[tokio::test]
async fn a_hero_icon_lost_to_a_write_error_is_saved_by_a_later_tab() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let frame = tab_frame();
    let read = panel((1600, 200, 1800, 400), "ana", 0.97);
    let outbox = dir.path().join("training_crops").join("outbox");
    std::fs::create_dir_all(&outbox).unwrap();
    std::fs::write(outbox.join("hero_icon"), b"not a folder").unwrap();
    let tab = || source(Some(&read), Some("ana"));
    assert!(run(dir.path(), "race-hero-writefail", &frame, tab()).await);
    assert!(sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty());
    std::fs::remove_file(outbox.join("hero_icon")).unwrap();
    assert!(run(dir.path(), "race-hero-writefail", &frame, tab()).await);
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::HeroIcon).len(),
        1
    );
    assert!(!run(dir.path(), "race-hero-writefail", &frame, tab()).await);
}

#[tokio::test]
async fn a_hero_icon_with_no_meta_releases_its_claim() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let session = "race-hero-nometa";
    let frame = tab_frame();
    // A hero the panel read accepts but the hero list does not know, so
    // the save finds no `hero_icon` meta for it and keeps nothing.
    let hero = "zorblax";
    assert!(sync::hero_icon_meta(1920, 1080, hero, 0.97, 0.5, "heroes-v2").is_none());
    let read = panel((1600, 200, 1800, 400), hero, 0.97);
    let tab = || source(Some(&read), Some(hero));
    assert!(
        run(dir.path(), session, &frame, tab()).await,
        "the Tab claims the hero"
    );
    assert!(sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty());
    assert_eq!(sync::training_warning(dir.path()), None);
    assert!(!offered(&offer_key(session, UploadKind::HeroIcon, hero)));
    // The claim was released: a later Tab of the same game starts again.
    assert!(
        run(dir.path(), session, &frame, tab()).await,
        "a later Tab retries"
    );
    assert!(sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty());
}

/// A 1920x1080 Tab with no header labels (and no chat): the stat columns
/// are not guessed, so no digit cell is cut.
fn unlabelled_tab_frame() -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(1920, 1080, |x, y| {
        if x < 336 {
            Rgb([40, 40, 40])
        } else {
            coord_pixel(x, y)
        }
    }))
}

#[tokio::test]
async fn a_tab_that_cuts_no_digit_cell_releases_its_claim() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let session = "race-nocells";
    // The Tab qualifies and claims the set, but its save cuts no cell.
    let unlabelled = unlabelled_tab_frame();
    let started = offer_held(dir.path(), session, &unlabelled, vec![source(None, None)]).await;
    assert_eq!(started, 1, "the Tab claims the set before its save");
    assert!(sync::queued_crops(dir.path(), UploadKind::StatDigits).is_empty());
    // The claim was released: a good Tab of the same game saves the set,
    // and only once.
    let frame = tab_frame();
    assert!(run(dir.path(), session, &frame, source(None, None)).await);
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::StatDigits).len(),
        6
    );
    assert!(!run(dir.path(), session, &frame, source(None, None)).await);
    assert_eq!(sync::training_warning(dir.path()), None);
}

#[tokio::test]
async fn a_digit_save_that_keeps_no_cell_releases_its_claim() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let session = "race-keptnone";
    let frame = tab_frame();
    // The Tab claims the set, then consent is turned off before its save
    // runs: keep_tab_crops keeps no cell (Some(0)).
    let gate = CROP_TASK_GATE.write().unwrap_or_else(|e| e.into_inner());
    let handle = archive_tab_training_crops(dir.path(), session, &frame, source(None, None));
    set_consent(dir.path(), false, false);
    drop(gate);
    handle.expect("the Tab claims the set").await.unwrap();
    assert!(sync::queued_crops(dir.path(), UploadKind::StatDigits).is_empty());
    // While consent stays off, a later Tab offers nothing.
    assert!(!run(dir.path(), session, &frame, source(None, None)).await);
    // The claim was released: once consent is back on, a later Tab of the
    // same game saves the set, and only once.
    set_consent(dir.path(), false, true);
    assert!(run(dir.path(), session, &frame, source(None, None)).await);
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::StatDigits).len(),
        6
    );
    assert!(!run(dir.path(), session, &frame, source(None, None)).await);
    assert_eq!(sync::training_warning(dir.path()), None);
}

/// A 7680x4320 Tab: the [`tab_frame_with`] header bar and the six
/// [`REAL_LABELS`] marks, four times the 1080p size, over a chat strip of
/// flat grey and, everywhere else, high-entropy noise. Every stat digit
/// cell it cuts is within the 256 px edge bound but its PNG is over the
/// 64 KiB `stat_digits` byte cap.
fn noisy_8k_tab_frame() -> DynamicImage {
    let (w, h) = (7680, 4320);
    let (bx, by, bw, bh) = ocr::preprocess::crop_scoreboard_rect(w, h);
    let header_end = by + (f64::from(bh) * 0.024) as u32;
    let (label_y0, label_y1) = (
        by + (f64::from(bh) * 0.008) as u32,
        by + (f64::from(bh) * 0.022) as u32,
    );
    let spans: Vec<(u32, u32)> = REAL_LABELS
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let width = label_width(i) * 4;
            let start = bx + (c * f64::from(bw)).round() as u32 - width / 2;
            (start, start + width)
        })
        .collect();
    let is_label = |x: u32, y: u32| {
        (label_y0..label_y1).contains(&y) && spans.iter().any(|&(s, e)| (s..e).contains(&x))
    };
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        if x < bx {
            Rgb([40, 40, 40])
        } else if (by..header_end).contains(&y) && (bx..bx + bw).contains(&x) {
            if is_label(x, y) {
                Rgb([16, 16, 16])
            } else {
                Rgb([232, 232, 232])
            }
        } else {
            // splitmix64 of the position: no pattern for the encoder.
            let mut z = (u64::from(y) << 32 | u64::from(x)).wrapping_add(0x9e37_79b9_7f4a_7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            Rgb([z as u8, (z >> 8) as u8, (z >> 16) as u8])
        }
    }))
}

/// The six stat cells [`noisy_8k_tab_frame`] cuts for row 1 of a 5v5
/// board, as the save cuts them, each as its encoded `stat_digits` crop.
fn noisy_8k_digit_crops(frame: &DynamicImage) -> Vec<sync::QueuedCrop> {
    let board = ocr::preprocess::crop_scoreboard(frame);
    let groups = ocr::preprocess::header_label_groups(&board);
    let columns = training_stat_columns(&groups, board.width()).expect("the six labels");
    let row = ocr::preprocess::crop_player_row(&board, 1, 5).expect("row 1");
    (0..sync::STAT_DIGIT_COLUMNS.len())
        .map(|col| {
            let cell = ocr::preprocess::crop_stat_cell(&row, col, &columns).expect("a cell");
            sync::QueuedCrop {
                id: String::new(),
                meta: sync::stat_digits_meta(7680, 4320, col, "Ana").unwrap(),
                png: encode_png(&cell).expect("the cell encodes"),
            }
        })
        .collect()
}

#[tokio::test]
async fn a_digit_save_whose_every_cell_is_over_the_byte_cap_releases_its_claim() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let session = "race-overcap";
    let noisy = noisy_8k_tab_frame();
    // The fixture: six cells, each within the edge bound and refused only
    // by the 64 KiB byte cap.
    let crops = noisy_8k_digit_crops(&noisy);
    assert_eq!(crops.len(), 6);
    for crop in &crops {
        assert_eq!(
            sync::check_crop(UploadKind::StatDigits, crop),
            Err(sync::CropRefusal::TooLarge(crop.png.len())),
            "{:?}",
            crop.meta.slot_type
        );
    }
    // The Tab claims the set before its save; the save cuts six cells and
    // keeps none (Some(0)), and nothing is lost, so no warning.
    let started = offer_held(dir.path(), session, &noisy, vec![source(None, None)]).await;
    assert_eq!(started, 1, "the Tab claims the set before its save");
    assert!(sync::queued_crops(dir.path(), UploadKind::StatDigits).is_empty());
    assert_eq!(sync::training_warning(dir.path()), None);
    // The claim was released: a good 1080p Tab of the same game saves the
    // set, and only once.
    let frame = tab_frame();
    assert!(run(dir.path(), session, &frame, source(None, None)).await);
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::StatDigits).len(),
        6
    );
    assert!(!run(dir.path(), session, &frame, source(None, None)).await);
}
