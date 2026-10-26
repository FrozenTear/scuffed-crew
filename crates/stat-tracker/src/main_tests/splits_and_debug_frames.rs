//! Daemon tests: splits_and_debug_frames. See `mod.rs`.

use super::*;

#[test]
fn stat_regression_detects_real_game_boundaries() {
    // Colosseo end -> Neon Junction first capture (E11->0, DMG 2740->0;
    // deaths 0->0 is NOT strictly lower, two drops still suffice).
    assert!(regressed((11, 0, 2740), (0, 0, 0)));
    // Neon Junction end -> Dorado first capture: all three drop.
    assert!(regressed((34, 10, 11742), (1, 0, 254)));
}

#[test]
fn stat_regression_ignores_single_field_misreads() {
    // Garbage OCR row mid-game (E29->9 misread, deaths/damage rose):
    // one drop must not split the session.
    assert!(!regressed((29, 5, 9242), (9, 11, 61029)));
    // Inflated elims read (E91) settling back next capture, deaths and
    // damage unchanged: still only one drop.
    assert!(!regressed((91, 3, 9072), (15, 3, 9072)));
    // Normal mid-game progression never regresses.
    assert!(!regressed((5, 1, 1271), (8, 2, 5966)));
    // Identical re-capture of the same board (post-match Tab spam).
    assert!(!regressed((30, 5, 10352), (30, 5, 10352)));
}

#[test]
fn stat_regression_after_garbage_row_needs_the_gap_guard() {
    // A garbage row (E9 D11 DMG61029) followed by the next real capture
    // DOES look like a regression, the STAT_SPLIT_MIN_GAP guard is what
    // prevents this from splitting (captures were 86s apart, gap is 120s).
    assert!(regressed((9, 11, 61029), (3, 5, 10865)));
    assert!(STAT_SPLIT_MIN_GAP > Duration::from_secs(86));
}

#[test]
fn dup1_suspect_columns_do_not_vote_for_a_split() {
    // CG-2 latch: `accepted` sits inflated (E latched 40, DMG latched 35031),
    // so the real reads (E10, DMG5435) look like a two-column regression and
    // would split a NEW session mid-game (Rialto 2026-07-20). Here last_raw
    // also sits high (=accepted), so only the current-suspect strip protects.
    let prev = (40, 4, 35031);
    let cur = (10, 4, 5435);
    assert!(
        stats_regressed(prev, prev, cur, CLEAN3, CLEAN3),
        "with last_raw high and all clean the latched inflation fakes a boundary"
    );
    assert!(
        !stats_regressed(prev, prev, cur, (true, false, true), CLEAN3),
        "suspect E + DMG drops must not vote: no mid-game split"
    );
}

#[test]
fn dup1_genuine_new_game_still_splits_on_clean_resets() {
    // A real new game resets every counter and reads them CLEAN, so the guard
    // leaves the boundary fully detectable: E34->1, D10->0, DMG11742->254.
    assert!(
        regressed((34, 10, 11742), (1, 0, 254)),
        "clean reset reads still trip the split"
    );
    // Even if ONE reset column happened to be suspect, the other two clean
    // drops still carry the 2-of-3 vote.
    assert!(
        stats_regressed(
            (34, 10, 11742),
            (34, 10, 11742),
            (1, 0, 254),
            (false, false, true),
            CLEAN3
        ),
        "one suspect reset column does not block a genuine boundary"
    );
}

// --- F-CG2-1: raw-continuity split vote (latch-recovery reads are clean) ---

#[test]
fn fcg2_latch_recovery_reads_do_not_split_midgame() {
    // The Rialto-class residual: `accepted` is CG-2-latched high (E40,
    // DMG35031) while `last_raw` tracks reality (E10, DMG10311 from the last
    // real capture). After the ≥120s gap the first clean read (E10,
    // DMG10500) drops versus the latched `accepted` but is CONTINUOUS with
    // last_raw, the un-latch streak is still 0, yet this must NOT split.
    let prev_acc = (40, 4, 35031);
    let prev_raw = (10, 4, 10311);
    let cur = (10, 5, 10500);
    assert!(
        !stats_regressed(prev_acc, prev_raw, cur, CLEAN3, CLEAN3),
        "clean reads continuous with last_raw must not vote for a split"
    );
}

#[test]
fn fcg2_genuine_reset_drops_versus_both_and_splits() {
    // Same latched state, but a genuine new game resets every counter to ~0,
    // which drops versus BOTH accepted AND last_raw, so the boundary trips.
    let prev_acc = (40, 4, 35031);
    let prev_raw = (10, 4, 10311);
    let cur = (0, 0, 200);
    assert!(
        stats_regressed(prev_acc, prev_raw, cur, CLEAN3, CLEAN3),
        "a real reset drops versus last_raw too and still splits"
    );
}

#[test]
fn fcg2_suspect_prev_raw_falls_back_to_accepted_drop_alone() {
    // When last_raw for a column is itself suspect, it is not a trustworthy
    // continuity anchor, so that column falls back to the DUP-1 rule: an
    // accepted-drop alone may vote. Here E and DMG prev_raw are suspect and
    // the reads drop versus accepted → the two votes split (conservative).
    let prev_acc = (40, 4, 35031);
    let prev_raw = (10, 4, 10311); // values irrelevant on the suspect path
    let cur = (10, 5, 10500); // continuous with last_raw, but that is suspect
    assert!(
        stats_regressed(prev_acc, prev_raw, cur, CLEAN3, (true, false, true)),
        "suspect prev_raw → accepted-drop alone may vote (conservative fallback)"
    );
}

#[test]
fn pending_outcome_expires() {
    let now = test_now();

    let mut fresh = Some((detect::MatchOutcome::Defeat, now - Duration::from_secs(30)));
    assert_eq!(
        take_fresh_pending(&mut fresh, now),
        Some(detect::MatchOutcome::Defeat)
    );
    assert!(fresh.is_none());

    let mut stale = Some((detect::MatchOutcome::Defeat, now - Duration::from_secs(200)));
    assert_eq!(take_fresh_pending(&mut stale, now), None);
    assert!(stale.is_none());

    assert_eq!(take_fresh_pending(&mut None, now), None);
}

#[test]
fn poll_debug_hit_skips_mid_match_no_signal() {
    assert_eq!(poll_debug_hit(None, None, OUTCOME_CONFIRM_WINDOW), None);
    assert_eq!(
        poll_debug_hit(
            None,
            Some((detect::MatchOutcome::Victory, Duration::from_secs(5))),
            OUTCOME_CONFIRM_WINDOW
        ),
        None
    );
}

/// R9-1 (0.5.0-rc.9): no source confirms a result on one tick. The
/// colour flood that did is gone; every source needs a second agreeing
/// read inside the confirm window.
#[test]
fn one_read_of_any_source_is_never_a_confirm() {
    use detect::match_end::OutcomeSource as S;
    for source in [S::ResultWord, S::RankScreen, S::EndTitle] {
        for outcome in [detect::MatchOutcome::Victory, detect::MatchOutcome::Defeat] {
            assert_eq!(
                poll_debug_hit(Some((outcome, source)), None, OUTCOME_CONFIRM_WINDOW),
                Some((PollDebugHit::Streak, outcome)),
                "{source:?}"
            );
            let mut streak = None;
            let tick = resolve_word_tick(&mut streak, Some((outcome, source)), test_now(), None);
            assert_eq!(tick.signal, Some(outcome));
            assert!(!tick.signal_confirmed, "{source:?} {outcome:?}");
        }
    }
}

#[test]
fn poll_debug_hit_first_word_is_streak() {
    assert_eq!(
        poll_debug_hit(
            word(detect::MatchOutcome::Victory),
            None,
            OUTCOME_CONFIRM_WINDOW
        ),
        Some((PollDebugHit::Streak, detect::MatchOutcome::Victory))
    );
    // Expired prior streak is a new first sighting, not a confirm.
    assert_eq!(
        poll_debug_hit(
            word(detect::MatchOutcome::Defeat),
            Some((
                detect::MatchOutcome::Defeat,
                OUTCOME_CONFIRM_WINDOW + Duration::from_secs(1)
            )),
            OUTCOME_CONFIRM_WINDOW
        ),
        Some((PollDebugHit::Streak, detect::MatchOutcome::Defeat))
    );
    // Disagreement restarts the streak.
    assert_eq!(
        poll_debug_hit(
            word(detect::MatchOutcome::Draw),
            Some((detect::MatchOutcome::Victory, Duration::from_secs(2))),
            OUTCOME_CONFIRM_WINDOW
        ),
        Some((PollDebugHit::Streak, detect::MatchOutcome::Draw))
    );
}

#[test]
fn poll_debug_hit_second_agreeing_word_is_confirm() {
    assert_eq!(
        poll_debug_hit(
            word(detect::MatchOutcome::Victory),
            Some((detect::MatchOutcome::Victory, Duration::from_secs(10))),
            OUTCOME_CONFIRM_WINDOW
        ),
        Some((PollDebugHit::Confirm, detect::MatchOutcome::Victory))
    );
    assert_eq!(
        poll_debug_hit(
            Some((
                detect::MatchOutcome::Defeat,
                detect::match_end::OutcomeSource::RankScreen
            )),
            Some((detect::MatchOutcome::Defeat, Duration::from_secs(1))),
            OUTCOME_CONFIRM_WINDOW
        ),
        Some((PollDebugHit::Confirm, detect::MatchOutcome::Defeat))
    );
    assert_eq!(
        poll_debug_hit(
            Some((
                detect::MatchOutcome::Draw,
                detect::match_end::OutcomeSource::EndTitle
            )),
            Some((detect::MatchOutcome::Draw, Duration::from_secs(50))),
            OUTCOME_CONFIRM_WINDOW
        ),
        Some((PollDebugHit::Confirm, detect::MatchOutcome::Draw))
    );
}

#[test]
fn poll_debug_prefix_is_greppable_by_kind_and_outcome() {
    assert_eq!(
        poll_debug_prefix(PollDebugHit::Confirm, detect::MatchOutcome::Victory),
        "poll_confirm_victory"
    );
    assert_eq!(
        poll_debug_prefix(PollDebugHit::Streak, detect::MatchOutcome::Defeat),
        "poll_streak_defeat"
    );
    assert_eq!(
        poll_debug_prefix(PollDebugHit::Confirm, detect::MatchOutcome::Draw),
        "poll_confirm_draw"
    );
}

#[tokio::test]
async fn a_rejected_capture_saves_a_map_crop_only_with_debug_on() {
    // R2 on #244: a rejected frame can be gameplay, where the map-label
    // box covers the kill feed (player names).
    let dir = tempfile::tempdir().unwrap();
    let mapcrops = dir.path().join("debug").join("mapcrops");
    for reason in ["teamsize", "noscoreboard", "noplayerrow", "unreadable"] {
        assert!(
            archive_rejected_map_label_crop(false, dir.path(), "sess-r", &labelled_frame(), reason)
                .is_none(),
            "{reason}"
        );
    }
    assert!(
        archive_rejected_map_label_crop(true, dir.path(), "sess-r", &labelled_frame(), "preflight")
            .is_none()
    );
    assert!(
        png_names(&mapcrops).is_empty(),
        "debug off saves no map crop"
    );

    archive_rejected_map_label_crop(true, dir.path(), "sess-r", &labelled_frame(), "unreadable")
        .expect("debug on saves the crop")
        .await
        .unwrap();
    assert_eq!(png_names(&mapcrops).len(), 1);

    // An accepted capture still saves one with debug off.
    archive_map_label_crop(dir.path(), "sess-a", &labelled_frame())
        .expect("accepted capture saves the crop")
        .await
        .unwrap();
    assert_eq!(png_names(&mapcrops).len(), 2);
}

#[tokio::test]
async fn accepted_and_rejected_rings_write_nothing_with_debug_off() {
    let dir = tempfile::tempdir().unwrap();
    assert!(save_accepted_frame(false, dir.path(), Arc::new(tiny_board())).is_none());
    assert!(save_rejected_frame(false, dir.path(), tiny_board(), "unreadable").is_none());
    assert!(
        !dir.path().join("debug").exists(),
        "debug off must not create debug/accepted or debug/rejected"
    );
}

#[tokio::test]
async fn accepted_and_rejected_rings_write_with_debug_on() {
    let dir = tempfile::tempdir().unwrap();
    save_accepted_frame(true, dir.path(), Arc::new(tiny_board()))
        .expect("debug on spawns the write")
        .await
        .unwrap();
    save_rejected_frame(true, dir.path(), tiny_board(), "unreadable")
        .expect("debug on spawns the write")
        .await
        .unwrap();
    let accepted = png_names(&dir.path().join("debug").join("accepted"));
    let rejected = png_names(&dir.path().join("debug").join("rejected"));
    assert!(
        accepted.len() == 1 && accepted[0].starts_with("accepted_"),
        "accepted={accepted:?}"
    );
    assert!(
        rejected.len() == 1 && rejected[0].starts_with("rejected_unreadable_"),
        "rejected={rejected:?}"
    );
}

/// debug/accepted stores the unpadded crop. A 5v5 recognition image is
/// taller (1014 px at 1440p, 761 px at 1080p), and replaying that height
/// moves every row. The saved board keeps the unpadded size and the
/// same top-left pixel as the frame at the crop origin.
#[tokio::test]
async fn accepted_debug_board_saves_the_unpadded_crop() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let src = daemon_source();
    let prod = norm(src.split("\nmod tests {").next().unwrap());
    assert!(norm(fn_source("handle_capture")).contains(
        "save_accepted_frame(ctx.debug_ocr, data_dir, debug_accepted_board(&frame_img))"
    ));
    assert!(
        prod.contains(
            "save_accepted_frame(ctx.debug_ocr, data_dir, debug_accepted_board(&frame_img))"
        ),
        "debug/accepted must save the unpadded crop"
    );
    assert!(
        !prod.contains("save_accepted_frame(ctx.debug_ocr, data_dir, scoreboard_img)"),
        "the padded recognition image must not be the debug board"
    );

    for (w, h, origin, size) in [
        (2560u32, 1440u32, (448u32, 216u32), (1664u32, 1007u32)),
        (1920, 1080, (336, 162), (1248, 756)),
    ] {
        let (ox, oy) = origin;
        let (cw, ch) = size;
        assert_eq!(
            stat_tracker::ocr::preprocess::crop_scoreboard_rect(w, h),
            (ox, oy, cw, ch),
            "{w}x{h}"
        );
        let padded = stat_tracker::ocr::preprocess::crop_scoreboard_rect_for(w, h, 5);
        assert!(padded.3 > ch, "{w}x{h} 5v5 pad missing");

        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([0, 0, 0]));
        img.put_pixel(ox, oy, image::Rgb([10, 20, 30]));
        img.put_pixel(ox + cw - 1, oy + ch - 1, image::Rgb([40, 50, 60]));
        // First row of the 5v5 pad, just under the unpadded crop.
        img.put_pixel(ox, oy + ch, image::Rgb([255, 0, 0]));
        let frame = image::DynamicImage::ImageRgb8(img);
        let board = debug_accepted_board(&frame);
        assert_eq!((board.width(), board.height()), (cw, ch), "{w}x{h}");
        assert_eq!(
            board.to_rgb8().get_pixel(0, 0).0,
            [10, 20, 30],
            "{w}x{h} origin"
        );
        assert_eq!(
            board.to_rgb8().get_pixel(cw - 1, ch - 1).0,
            [40, 50, 60],
            "{w}x{h} bottom"
        );

        let dir = tempfile::tempdir().unwrap();
        save_accepted_frame(true, dir.path(), std::sync::Arc::clone(&board))
            .expect("debug on spawns the write")
            .await
            .unwrap();
        let names = png_names(&dir.path().join("debug").join("accepted"));
        assert_eq!(names.len(), 1, "{w}x{h} {names:?}");
        let saved = image::open(dir.path().join("debug").join("accepted").join(&names[0]))
            .unwrap()
            .to_rgb8();
        assert_eq!((saved.width(), saved.height()), (cw, ch), "{w}x{h}");
        assert_eq!(
            saved.get_pixel(0, 0).0,
            [10, 20, 30],
            "{w}x{h} saved origin"
        );
        assert_eq!(
            saved.get_pixel(cw - 1, ch - 1).0,
            [40, 50, 60],
            "{w}x{h} saved bottom"
        );
        assert_ne!(
            saved.get_pixel(0, ch - 1).0,
            [255, 0, 0],
            "{w}x{h} saved board includes the 5v5 pad"
        );
    }
}

#[test]
fn stale_board_debug_dirs_are_reported_only_with_debug_off_and_never_removed() {
    let dir = tempfile::tempdir().unwrap();
    assert!(stale_board_debug_dirs(false, dir.path()).is_empty());
    let accepted = dir.path().join("debug").join("accepted");
    std::fs::create_dir_all(&accepted).unwrap();
    let keep = accepted.join("accepted_old.png");
    std::fs::write(&keep, b"x").unwrap();
    assert_eq!(
        stale_board_debug_dirs(false, dir.path()),
        vec![accepted.clone()]
    );
    assert!(stale_board_debug_dirs(true, dir.path()).is_empty());
    assert!(keep.exists(), "the user's files are left untouched");
}

#[test]
fn accepted_ring_keeps_the_newest_60_crops() {
    assert_eq!(ACCEPTED_KEEP, 60);
    let dir = tempfile::tempdir().unwrap();
    let img =
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([8, 16, 32])));
    // 70 older crops, one minute apart, oldest first.
    let base = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 3600);
    for i in 0..70u64 {
        let path = dir.path().join(format!("accepted_old_{i:03}.png"));
        img.save(&path).unwrap();
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(base + std::time::Duration::from_secs(60 * i))
            .unwrap();
    }
    save_frame_ring(dir.path(), "accepted", &img, ACCEPTED_KEEP);
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), ACCEPTED_KEEP, "names={names:?}");
    // The new crop and the 59 newest old ones stay; 000..=010 go.
    assert!(
        names.iter().any(|n| n.starts_with("accepted_2")),
        "{names:?}"
    );
    assert!(!names.contains(&"accepted_old_010.png".to_string()));
    assert!(names.contains(&"accepted_old_011.png".to_string()));
    assert!(names.contains(&"accepted_old_069.png".to_string()));
}

#[test]
fn save_frame_ring_uses_on_hit_prefix_and_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let img =
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([8, 16, 32])));
    let prefix = poll_debug_prefix(PollDebugHit::Confirm, detect::MatchOutcome::Victory);
    save_frame_ring(dir.path(), &prefix, &img, 2);
    save_frame_ring(
        dir.path(),
        &poll_debug_prefix(PollDebugHit::Streak, detect::MatchOutcome::Defeat),
        &img,
        2,
    );
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("poll_confirm_victory_") && n.ends_with(".png")),
        "names={names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("poll_streak_defeat_") && n.ends_with(".png")),
        "names={names:?}"
    );
    assert_eq!(names.len(), 2);

    // Bounded keep: a third write evicts the oldest PNG in the ring.
    save_frame_ring(dir.path(), "poll_confirm_draw", &img, 2);
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "names={names:?}");
}
