//! Daemon tests: night_replays_quiet_close. See `mod.rs`.

use super::*;

#[tokio::test]
#[allow(clippy::type_complexity)]
async fn busan_defeat_and_junkertown_victory_stay_two_games() {
    // Boards from a 2026-10-05 night. The file labeled the later boards
    // with the earlier map because they had been merged; the later map
    // is Junkertown. No person's name belongs in this test.
    let njc: &[(u32, u32, u32, u32, u32, u32, &str)] = &[
        (6, 3, 2, 1703, 412, 954, "20:22:13Z"),
        (6, 4, 2, 2121, 459, 1161, "20:23:02Z"),
        (6, 4, 2, 2121, 459, 1161, "20:23:08Z"),
        (6, 4, 2, 2121, 459, 1161, "20:23:13Z"),
        (10, 4, 2, 3352, 509, 1629, "20:23:59Z"),
        (10, 4, 2, 4566, 759, 2221, "20:24:58Z"),
        (10, 4, 2, 5499, 1131, 2821, "20:26:00Z"),
        (1, 5, 2, 5554, 1131, 3146, "20:26:04Z"),
        (1, 5, 2, 5554, 1131, 3146, "20:26:08Z"),
        (1, 5, 2, 5554, 1181, 3146, "20:26:37Z"),
        (1, 5, 2, 5554, 1181, 3146, "20:26:43Z"),
        (13, 7, 2, 6657, 1432, 4188, "20:28:34Z"),
        (14, 8, 2, 6877, 1582, 4434, "20:30:03Z"),
        (18, 9, 4, 8224, 1765, 4726, "20:31:04Z"),
        (19, 9, 4, 8243, 1765, 4726, "20:31:11Z"),
        (19, 9, 4, 8243, 1765, 4726, "20:31:16Z"),
    ];
    let busan: &[(u32, u32, u32, u32, u32, u32, &str, &str)] = &[
        (3, 3, 0, 2905, 691, 0, "20:40:39Z", "Soldier: 76"),
        (4, 3, 0, 2905, 691, 1, "20:42:20Z", "Zenyatta"),
        (4, 4, 2, 3695, 691, 294, "20:43:14Z", "Zenyatta"),
        (4, 4, 5, 4409, 0, 294, "20:43:58Z", "Zenyatta"),
        (6, 4, 5, 4409, 0, 294, "20:44:43Z", "Zenyatta"),
        (6, 4, 6, 4409, 809, 9, "20:45:03Z", "Zenyatta"),
        (8, 4649, 7, 4409, 809, 9, "20:45:33Z", "Zenyatta"),
        (8, 4649, 7, 5, 809, 9, "20:45:44Z", "Zenyatta"),
    ];
    let junk: &[(u32, u32, u32, u32, u32, u32, &str)] = &[
        (10, 4649, 7, 2879, 921, 1679, "21:01:48Z"),
        (14, 4649, 7, 4572, 1044, 2202, "21:03:09Z"),
        (14, 3, 5, 4572, 1044, 2202, "21:03:14Z"),
        (14, 3, 5, 4572, 1044, 2202, "21:03:27Z"),
        (16, 3, 6, 5145, 1099, 2319, "21:05:08Z"),
        (16, 3, 7, 5229, 1149, 2319, "21:05:27Z"),
        (16, 4, 7, 5691, 1349, 2564, "21:06:06Z"),
        (7, 4, 7, 5691, 1349, 2564, "21:06:12Z"),
        (7, 4, 7, 5691, 1349, 2564, "21:06:17Z"),
        (7, 4, 7, 5691, 1349, 2564, "21:06:20Z"),
        (21, 5, 9, 6121, 1413, 2714, "21:07:23Z"),
        (23, 5, 9, 6792, 1463, 3006, "21:08:06Z"),
        (23, 5, 9, 6792, 1463, 3006, "21:08:38Z"),
    ];
    let mut night = Night::new().await;
    night.begin_on("New Junk City", "Wrecking Ball");
    let origin = night.now;
    let start = chrono::DateTime::parse_from_rfc3339("2026-10-05T20:22:13Z")
        .unwrap()
        .with_timezone(&Utc);
    let day = "2026-10-05";
    for (e, d, a, dmg, hlg, mit, stamp) in njc {
        jump_to(&mut night, origin, start, stamp, day);
        night
            .tab_once(
                night_counters(*e, *a, *d, *dmg, *hlg, *mit),
                "Wrecking Ball",
                Some(0),
                Some("New Junk City"),
                NIGHT_CLEAN,
            )
            .await;
    }
    let njc_id = night.id();
    jump_to(&mut night, origin, start, "20:34:07Z", day);
    night
        .word(detect::MatchOutcome::Victory, Some("New Junk City"))
        .await;
    assert_eq!(night.id(), njc_id);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    for (e, d, a, dmg, hlg, mit, stamp, hero) in busan {
        jump_to(&mut night, origin, start, stamp, day);
        night
            .tab_once(
                night_counters(*e, *a, *d, *dmg, *hlg, *mit),
                hero,
                Some(0),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
    }
    let busan_id = night.id();
    assert_ne!(
        busan_id, njc_id,
        "grace closes the finished New Junk City game"
    );
    jump_to(&mut night, origin, start, "20:46:15Z", day);
    night.word_once(detect::MatchOutcome::Defeat).await;
    assert_eq!(night.id(), busan_id);
    for (e, d, a, dmg, hlg, mit, stamp) in junk {
        jump_to(&mut night, origin, start, stamp, day);
        night
            .tab_once(
                night_counters(*e, *a, *d, *dmg, *hlg, *mit),
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
    }
    let junk_id = night.id();
    assert_ne!(
        junk_id, busan_id,
        "the gap seals Busan and opens Junkertown"
    );
    jump_to(&mut night, origin, start, "21:08:47Z", day);
    night
        .word(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_eq!(night.id(), junk_id);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    let busan_rows = night.store.get_session_snapshots(&busan_id).await.unwrap();
    assert!(
        busan_rows.iter().all(|row| row.outcome == "defeat"),
        "Busan closes as Defeat"
    );
    assert!(night.elims(&junk_id).await.contains(&23));
    assert!(!night.elims(&busan_id).await.contains(&23));
    assert!(night.elims(&njc_id).await.contains(&19));
    assert!(
        night
            .store
            .get_session_snapshots(&njc_id)
            .await
            .unwrap()
            .iter()
            .all(|row| row.outcome == "victory")
    );
}

#[tokio::test]
async fn neon_junction_side_swap_stays_one_session() {
    // One Hybrid match. A hero-select screen at the side swap used to
    // split it. Elims misreads (9→1, 15→7) are in the row list.
    let rows: &[(u32, u32, u32, u32, u32, u32, &str)] = &[
        (9, 3, 4, 4470, 716, 1880, "21:55:03Z"),
        (9, 4, 4, 5736, 871, 2374, "21:56:54Z"),
        (9, 4, 4, 5736, 871, 2374, "21:56:59Z"),
        (1, 4, 4, 5736, 871, 2374, "21:57:02Z"),
        (1, 4, 4, 5736, 871, 2374, "21:57:07Z"),
        (1, 4, 4, 5736, 871, 2374, "21:57:12Z"),
        (1, 4, 4, 5736, 871, 2374, "21:57:32Z"),
        (1, 4, 4, 5736, 871, 2374, "21:57:55Z"),
        (1, 4, 4, 5736, 871, 2374, "21:58:12Z"),
        (1, 4, 4, 5736, 871, 2374, "21:58:23Z"),
        (1, 4, 4, 5736, 871, 2374, "21:58:31Z"),
        (12, 5, 4, 6660, 1171, 2924, "21:59:39Z"),
        (12, 5, 4, 6660, 1171, 2924, "21:59:44Z"),
        (12, 5, 4, 6660, 1171, 2924, "21:59:51Z"),
        (15, 5, 4, 7646, 1271, 3318, "22:00:48Z"),
        (15, 6, 4, 7859, 1321, 3718, "22:01:27Z"),
        (15, 7, 4, 8793, 1371, 4118, "22:02:05Z"),
        (15, 7, 4, 8793, 1371, 4118, "22:02:11Z"),
        (15, 7, 4, 10131, 1584, 4639, "22:03:05Z"),
        (15, 8, 4, 10165, 1634, 4789, "22:03:15Z"),
        (7, 8, 4, 10165, 1634, 4789, "22:03:21Z"),
        (7, 8, 4, 10165, 1634, 4789, "22:03:28Z"),
        (7, 8, 4, 10310, 1667, 4862, "22:03:44Z"),
        (7, 8, 4, 10654, 1667, 4962, "22:04:04Z"),
        (7, 8, 4, 10965, 1667, 5099, "22:04:27Z"),
        (7, 8, 4, 10965, 1667, 5099, "22:04:39Z"),
        (20, 8, 6, 11867, 1809, 5756, "22:05:17Z"),
        (23, 9, 8, 12469, 1923, 5821, "22:05:59Z"),
        (24, 9, 8, 12915, 2046, 6833, "22:06:52Z"),
        (24, 9, 8, 12915, 2046, 6833, "22:06:59Z"),
        (24, 9, 8, 12915, 2046, 6833, "22:07:04Z"),
        (24, 9, 8, 12921, 2096, 6833, "22:07:10Z"),
        (28, 9, 9, 14579, 2246, 7458, "22:07:48Z"),
        (32, 10, 10, 15143, 2446, 7940, "22:08:31Z"),
        (32, 10, 10, 15143, 2446, 7940, "22:08:44Z"),
        (32, 10, 10, 15143, 2446, 7940, "22:08:48Z"),
    ];
    let mut night = Night::new().await;
    night.begin_on("Neon Junction", "Wrecking Ball");
    let origin = night.now;
    let start = chrono::DateTime::parse_from_rfc3339("2026-09-27T21:55:03Z")
        .unwrap()
        .with_timezone(&Utc);
    let id = night.id();
    for (e, d, a, dmg, hlg, mit, stamp) in rows {
        if *stamp == "22:05:59Z" {
            jump_to(&mut night, origin, start, "22:05:38Z", "2026-09-27");
            night.screen(boundary::StartScreen::HeroSelect).await;
        }
        jump_to(&mut night, origin, start, stamp, "2026-09-27");
        night
            .tab_once(
                night_counters(*e, *a, *d, *dmg, *hlg, *mit),
                "Wrecking Ball",
                Some(0),
                Some("Neon Junction"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.id(), id, "split at {stamp}");
    }
    assert!(night.elims(&id).await.contains(&32));
}

#[tokio::test]
async fn a_stomp_with_a_short_tab_gap_and_a_hero_select_stays_one_session() {
    let mut night = Night::new().await;
    night.begin_on("Colosseo", "Reinhardt");
    let id = night.id();
    let rows = [
        (0, 0, 0, 120, 40, 80),
        (0, 1, 1, 480, 60, 200),
        (1, 2, 1, 900, 80, 400),
        (1, 3, 2, 1400, 90, 700),
        (2, 4, 2, 2100, 110, 1100),
    ];
    for (i, (e, d, a, dmg, hlg, mit)) in rows.into_iter().enumerate() {
        if i == 3 {
            night.screen(boundary::StartScreen::HeroSelect).await;
        }
        night.advance(Duration::from_secs(40));
        night
            .tab_once(
                night_counters(e, a, d, dmg, hlg, mit),
                "Reinhardt",
                Some(0),
                Some("Colosseo"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.id(), id);
    }
    assert_eq!(night.elims(&id).await, vec![0, 0, 1, 1, 2]);
}

#[tokio::test]
async fn last_game_of_the_night_retires_at_three_minutes_and_syncs_once() {
    let mut night = Night::new().await;
    let id = finish_colosseo(&mut night).await;
    let activity = night.game().last_activity;
    let recorded = night.game().outcome_recorded_at.expect("result recorded");
    assert_eq!(activity, recorded);
    assert!(
        night.now < activity + Duration::from_secs(179),
        "the confirming word must not already consume the quiet window"
    );
    night
        .idle_through(activity + close_after() - Duration::from_secs(1))
        .await;
    assert_eq!(night.id(), id, "still the open game just before 180s");
    assert_eq!(night.sync_count(), 0, "no sync before the quiet window");
    assert_eq!(night.upload_count(), 0);
    night.idle_through(activity + close_after()).await;
    assert!(
        night.st.active_game.is_none(),
        "the last game is retired at 180s without another Tab"
    );
    assert_eq!(night.sync_count(), 1, "deleting the sync call must fail");
    assert_eq!(night.upload_count(), 1);
    night
        .idle_through(activity + Duration::from_secs(10 * 60))
        .await;
    assert!(
        night.st.active_game.is_none(),
        "the retired game stays closed through the rest of the night"
    );
    assert!(
        !active_game_path(night.dir.path()).exists(),
        "active_game.json is cleared with the session"
    );
    assert_eq!(night.sync_count(), 1, "deleting the sync call must fail");
    assert_eq!(
        night.upload_count(),
        1,
        "the unsynced rows upload exactly once"
    );
    let rows = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(
        !rows.is_empty(),
        "the scoreboard rows are still in the store"
    );
    assert!(rows.iter().all(|row| row.outcome == "defeat"));
    assert!(rows.iter().all(|row| row.synced));
    assert_eq!(night.elims(&id).await, vec![14]);
}

#[tokio::test]
async fn post_match_tab_inside_grace_delays_the_quiet_close() {
    let mut night = Night::new().await;
    let id = finish_colosseo(&mut night).await;
    let recorded = night.game().outcome_recorded_at.expect("result recorded");
    night.idle_through(recorded + Duration::from_secs(60)).await;
    assert_eq!(night.id(), id);
    assert_eq!(night.sync_count(), 0);
    night
        .tab_once(
            night_counters(16, 23, 6, 2600, 10000, 420),
            "Zenyatta",
            Some(2),
            Some("Colosseo"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.id(),
        id,
        "a Tab at +60s still lands on the finished game"
    );
    assert!(night.elims(&id).await.contains(&16));
    let activity = night.game().last_activity;
    assert!(activity >= recorded + Duration::from_secs(60));
    night.idle_through(recorded + close_after()).await;
    assert_eq!(
        night.id(),
        id,
        "180s after the result is not enough once a later Tab moved last activity"
    );
    assert_eq!(night.sync_count(), 0);
    night
        .idle_through(activity + close_after() - Duration::from_secs(1))
        .await;
    assert_eq!(night.id(), id);
    night.idle_through(activity + close_after()).await;
    assert!(night.st.active_game.is_none());
    assert_eq!(night.sync_count(), 1);
    assert_eq!(night.upload_count(), 1);
    let rows = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(rows.iter().all(|row| row.outcome == "defeat"));
    assert!(night.elims(&id).await.contains(&16));
}

#[tokio::test]
async fn tab_after_the_quiet_close_opens_a_new_session() {
    let mut night = Night::new().await;
    let id = finish_colosseo(&mut night).await;
    let activity = night.game().last_activity;
    night.idle_through(activity + close_after()).await;
    assert!(night.st.active_game.is_none());
    assert_eq!(night.upload_count(), 1);
    let before = night.elims(&id).await;
    night
        .idle_through(night.now + Duration::from_secs(5 * 60))
        .await;
    assert_eq!(
        night.upload_count(),
        1,
        "sitting after the close does not upload again"
    );
    night
        .tab_once(
            night_counters(2, 1, 0, 350, 60, 800),
            "Reinhardt",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(night.id(), id, "the Tab opens a new session");
    assert_eq!(
        night.elims(&id).await,
        before,
        "the retired game is not written again"
    );
    let old = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(old.iter().all(|row| row.outcome == "defeat"));
    night
        .word(detect::MatchOutcome::Victory, Some("Ilios"))
        .await;
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_ne!(night.id(), id);
    let old = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(
        old.iter().all(|row| row.outcome == "defeat"),
        "a later result is not sealed onto the retired game"
    );
}

#[tokio::test]
async fn unfinished_game_idle_for_21_minutes_stays_open() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Ana");
    night
        .tab_once(
            night_counters(8, 3, 2, 1800, 400, 100),
            "Ana",
            Some(0),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    let id = night.id();
    assert!(!night.game().finished());
    let activity = night.game().last_activity;
    night.idle_through(activity + UNFINISHED_SESSION_IDLE).await;
    assert_eq!(night.id(), id, "exactly 20 minutes is still this game");
    night
        .idle_through(activity + Duration::from_secs(21 * 60))
        .await;
    assert_eq!(
        night.id(),
        id,
        "the timer must not close an unfinished game"
    );
    assert_eq!(night.sync_count(), 0);
    assert!(active_game_path(night.dir.path()).exists());
    let rows = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(rows.iter().all(|row| row.outcome == "unknown"));
}

#[tokio::test]
async fn victory_after_22_minutes_idle_stays_on_the_same_session() {
    let mut night = Night::new().await;
    night.begin_on("Circuit Royal", "Ana");
    night
        .tab_once(
            night_counters(8, 3, 2, 1800, 400, 100),
            "Ana",
            Some(0),
            Some("Circuit Royal"),
            NIGHT_CLEAN,
        )
        .await;
    let id = night.id();
    let activity = night.game().last_activity;
    night
        .idle_through(activity + Duration::from_secs(22 * 60))
        .await;
    assert_eq!(night.id(), id);
    night
        .word(detect::MatchOutcome::Victory, Some("Circuit Royal"))
        .await;
    assert_eq!(night.id(), id);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    let recorded = night.game().last_activity;
    night.idle_through(recorded + close_after()).await;
    assert!(night.st.active_game.is_none());
    assert_eq!(night.upload_count(), 1);
    let rows = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(rows.iter().any(|row| row.elims == 8));
    assert!(rows.iter().all(|row| row.map_name == "Circuit Royal"));
    assert!(rows.iter().all(|row| row.outcome == "victory"));
}

#[tokio::test]
async fn quiet_close_carries_a_held_board_onto_the_next_session() {
    let mut night = Night::new().await;
    let id = finish_colosseo(&mut night).await;
    {
        let g = night.st.active_game.as_mut().expect("finished game");
        g.deferred = Some(night_counters(1, 0, 0, 100, 0, 0));
        g.deferred_hero = Some("Ana".into());
        g.deferred_at = Some(night.wall);
    }
    let activity = night.game().last_activity;
    night.idle_through(activity + close_after()).await;
    assert_eq!(night.upload_count(), 1);
    let opened = night.game();
    assert_ne!(opened.session_id, id);
    assert!(opened.deferred_imported);
    assert_eq!(opened.deferred.map(|c| c.elims), Some(1));
    let new_id = night.id();
    night
        .tab_once(
            night_counters(9, 1, 0, 400, 0, 0),
            "Ana",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert!(night.elims(&id).await.iter().all(|elims| *elims != 1));
    let held = night.elims(&new_id).await;
    assert!(
        held.contains(&1),
        "held board landed on the new session: {held:?}"
    );
    assert!(held.contains(&9), "{held:?}");
}

#[tokio::test]
async fn stored_game_mode_follows_the_session_map() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Ana");
    night
        .tab_text(
            night_counters(4, 1, 0, 800, 200, 0),
            "Ana",
            Some(0),
            "Dorado",
            NIGHT_CLEAN,
        )
        .await;
    let rows = night
        .store
        .get_session_snapshots(&night.id())
        .await
        .unwrap();
    assert_eq!(rows[0].map_name, "Busan");
    assert_eq!(rows[0].game_mode, "Control");
}

#[test]
fn end_screen_split_log_uses_the_pending_boundary_and_prints_none() {
    let open = boundary::OpenNew {
        reason: boundary::CloseReason::EndScreenMap,
        seal_outcome: Some(detect::MatchOutcome::Defeat),
        new_outcome: detect::MatchOutcome::Victory,
        new_map: Some("Ilios".into()),
        candidates: Vec::new(),
        new_result: None,
    };
    let split = end_screen_split_log(true, Some(&open), Some(detect::MatchOutcome::Defeat));
    assert!(split.contains("armed=true"), "{split}");
    assert!(split.contains("accolade_map=Ilios"), "{split}");
    assert!(split.contains("sealed_hint="), "{split}");
    assert!(!split.contains("sealed_hint=none"), "{split}");
    let mut missing = open;
    missing.new_map = None;
    let split = end_screen_split_log(false, Some(&missing), None);
    assert!(split.contains("accolade_map=none"), "{split}");
    assert!(split.contains("sealed_hint=none"), "{split}");
    assert!(split.contains("armed=false"), "{split}");
}

#[tokio::test]
async fn a_rejected_or_failed_tab_keeps_the_carried_board() {
    let mut night = Night::new().await;
    night.begin_on("Ilios", "Ana");
    {
        let g = night.st.active_game.as_mut().expect("game");
        g.deferred = Some(night_counters(1, 0, 0, 50, 0, 0));
        g.deferred_hero = Some("Ana".into());
        g.deferred_at = Some(night.wall);
        g.deferred_imported = true;
    }
    let sid = night.id();
    apply_capture_report(
        &mut night.st,
        &night.store,
        night.dir.path(),
        &sid,
        Ok(rejected_report(&sid)),
        night.now,
    )
    .await;
    assert!(night.game().deferred_imported);
    assert_eq!(night.game().deferred.map(|c| c.elims), Some(1));
    apply_capture_report(
        &mut night.st,
        &night.store,
        night.dir.path(),
        &sid,
        Err("capture cycle failed".into()),
        night.now,
    )
    .await;
    assert_eq!(night.game().deferred.map(|c| c.elims), Some(1));
    night
        .tab_once(
            night_counters(3, 1, 0, 200, 40, 0),
            "Ana",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    let elims = night.elims(&sid).await;
    assert_eq!(
        elims.iter().filter(|elims| **elims == 1).count(),
        1,
        "{elims:?}"
    );
    assert!(elims.contains(&3), "{elims:?}");
    assert!(night.game().deferred.is_none());
}

#[tokio::test]
async fn carried_board_is_not_stored_when_the_current_insert_fails() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let raw = night_counters(1, 0, 0, 10, 0, 0);
    let gate = capture_gate::apply_gate(None, raw, NIGHT_CLEAN, false);
    let staged = StagedCapture {
        target_session: "held-fail".into(),
        create_session: true,
        map_name: "Busan".into(),
        map_source: None,
        outcome: detect::MatchOutcome::Victory,
        outcome_label: "victory".into(),
        gate,
        carried: Some(raw),
        carried_hero: Some("Ana".into()),
        carried_at: Utc::now(),
        split: false,
        recorded_map: Some("Busan".into()),
        replaced_text_fallback: false,
    };
    write_staged_rows(
        &store,
        &staged,
        "Ana",
        "Support",
        SurrealDatetime::from(Utc::now()),
    )
    .await
    .unwrap();
    write_carried_after_current(&store, dir.path(), &staged, "Ana", false)
        .await
        .unwrap();
    let rows = store.get_session_snapshots("held-fail").await.unwrap();
    assert!(
        rows.is_empty(),
        "a refused current insert must not store the carried board: {rows:?}"
    );
    assert!(!should_write_carried(false, true));
    assert!(should_write_carried(true, true));
}

#[tokio::test]
async fn spawned_upload_joins_the_previous_task_before_it_runs() {
    let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let previous_order = std::sync::Arc::clone(&order);
    let previous = tokio::spawn(async move {
        rx.await.ok();
        previous_order.lock().unwrap().push("prev");
    });
    let upload_order = std::sync::Arc::clone(&order);
    let handle = spawn_after_previous(Some(previous), move || {
        let upload_order = std::sync::Arc::clone(&upload_order);
        async move {
            upload_order.lock().unwrap().push("upload");
        }
    });
    tokio::task::yield_now().await;
    assert!(
        order.lock().unwrap().is_empty(),
        "the upload must wait for the previous task"
    );
    tx.send(()).unwrap();
    handle.await.unwrap();
    assert_eq!(*order.lock().unwrap(), ["prev", "upload"]);
}

#[tokio::test]
async fn begin_daemon_session_returns_before_the_previous_sync_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let mut sync_task = Some(tokio::spawn(async move {
        let _ = rx.await;
    }));
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let st = begin_daemon_session(dir.path(), &mut sync_task, &backoff, &store, None).await;
    assert!(st.active_game.is_none());
    assert!(
        !sync_task.as_ref().unwrap().is_finished(),
        "startup must return while the previous upload is still running"
    );
    tx.send(()).unwrap();
    sync_task.take().unwrap().await.unwrap();
}
