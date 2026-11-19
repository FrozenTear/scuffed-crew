//! Two close Tabs of one game: the first to qualify claims the kind before
//! its save starts on the blocking thread, so the second saves nothing
//! even while the first is still saving. A save that keeps nothing
//! releases the claim, so a later Tab retries.

use std::sync::atomic::Ordering;

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
    let tabs = vec![source(None, None), source(None, None)];
    let started = offer_held(dir.path(), "race-digits", &frame, tabs).await;
    assert_eq!(started, 1, "the second Tab finds the set claimed");
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::StatDigits).len(),
        6,
        "one set"
    );
}

#[tokio::test]
async fn two_tabs_offered_back_to_back_keep_one_digit_set() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    // Held across both offers, so the first Tab's save has not started
    // when the second Tab is offered, however the threads are scheduled.
    let gate = CROP_TASK_GATE.write().unwrap_or_else(|e| e.into_inner());
    let first = archive_tab_training_crops(dir.path(), "race-b2b", &frame, source(None, None));
    let second = archive_tab_training_crops(dir.path(), "race-b2b", &frame, source(None, None));
    drop(gate);
    assert!(first.is_some(), "the first Tab claims the set");
    assert!(second.is_none(), "the second Tab finds the set claimed");
    first.unwrap().await.unwrap();
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::StatDigits).len(),
        6
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
