//! Daemon tests: night_sessions. See `mod.rs`.

use super::*;

#[tokio::test]
async fn a_fallback_board_yields_to_two_matching_clean_reads_and_a_later_ghost_is_rate_capped() {
    let mut night = Night::new().await;
    night.begin_on("Dorado", "Wrecking Ball");
    night
        .tab_once(
            night_counters(2, 40, 18, 450, 0, 2),
            "Wrecking Ball",
            None,
            Some("Dorado"),
            NIGHT_CLEAN,
        )
        .await;
    let latched = night.game().gate.expect("fallback stored");
    assert!(latched.low_trust);
    assert_eq!(latched.accepted.assists, 40);

    night.advance(Duration::from_secs(20));
    night
        .tab_once(
            night_counters(2, 0, 0, 1105, 259, 450),
            "Wrecking Ball",
            Some(0),
            Some("Dorado"),
            NIGHT_CLEAN,
        )
        .await;
    let once = night.game().gate.expect("first clean read stored");
    assert_eq!(
        once.accepted.assists, 0,
        "one clean read replaces the latch"
    );
    assert_eq!(once.accepted.deaths, 0);
    assert_eq!(once.accepted.mitigation, 450);
    assert!(!once.low_trust);

    night.advance(Duration::from_secs(20));
    night
        .tab_once(
            night_counters(2, 0, 0, 1105, 259, 450),
            "Wrecking Ball",
            Some(0),
            Some("Dorado"),
            NIGHT_CLEAN,
        )
        .await;
    let cleaned = night.game().gate.expect("second clean read stored");
    assert_eq!(cleaned.accepted.assists, 0);
    assert_eq!(cleaned.accepted.deaths, 0);
    assert_eq!(cleaned.accepted.damage, 1105);
    assert!(!cleaned.low_trust);

    night.advance(Duration::from_secs(20));
    night
        .tab_once(
            night_counters(91, 0, 0, 1105, 259, 450),
            "Wrecking Ball",
            Some(0),
            Some("Dorado"),
            NIGHT_CLEAN,
        )
        .await;
    let ghost = night.game().gate.expect("ghost stored");
    assert_eq!(ghost.accepted.elims, 2, "the rate cap still holds a ghost");
    assert!(!ghost.low_trust);
}

#[tokio::test]
async fn an_identified_row_can_still_be_an_untrusted_fallback() {
    // The row index is set, and the cells were unreadable, so the
    // numbers came from the text fallback. Trust does not follow the
    // row index.
    let mut night = Night::new().await;
    night.begin_on("Dorado", "Wrecking Ball");
    night.cells_trusted = Some(false);
    night
        .tab_once(
            night_counters(2, 40, 18, 450, 0, 2),
            "Wrecking Ball",
            Some(0),
            Some("Dorado"),
            NIGHT_CLEAN,
        )
        .await;
    let latched = night.game().gate.expect("fallback stored");
    assert!(latched.low_trust);
    assert!(latched.unconfirmed.iter().all(|&col| col));
    assert_eq!(latched.accepted.assists, 40);
}

#[tokio::test]
async fn multi_tick_vote_ban_and_select_keep_one_session_and_its_candidates() {
    let mut night = Night::new().await;
    let candidates = ["Junkertown", "Ilios"];
    night.screen(vote(&candidates)).await;
    let id = night.id();
    assert_eq!(night.game().map_candidates, vec!["Junkertown", "Ilios"]);
    night.screen(boundary::StartScreen::HeroBan).await;
    night.screen(boundary::StartScreen::HeroSelect).await;
    assert_eq!(night.id(), id, "a stable start screen must not re-split");
    assert_eq!(
        night.game().map_candidates,
        vec!["Junkertown", "Ilios"],
        "ban and select keep the vote candidates"
    );
    assert!(night.game().awaiting_first_board);
    let rows = night.elims(&id).await;
    assert!(rows.is_empty(), "no false unrecorded split wrote a board");
}

#[tokio::test]
async fn board_one_defeat_then_vote_seals_a_and_opens_b_with_no_tab() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_held(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
        )
        .await;
    night.advance(Duration::from_secs(8 * 60));
    night.word_once(detect::MatchOutcome::Defeat).await;
    let a = night.id();
    night.advance(Duration::from_secs(20));
    night.screen(vote(&["Junkertown", "Ilios"])).await;
    night.screen(boundary::StartScreen::HeroBan).await;
    night.screen(boundary::StartScreen::HeroSelect).await;
    assert_ne!(night.id(), a, "the vote opens B");
    assert_eq!(night.elims(&a).await, vec![14, 14, 14, 14]);
    let snaps = night.store.get_session_snapshots(&a).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.outcome == "defeat"),
        "A stays Defeat, got {:?}",
        snaps
            .iter()
            .map(|row| row.outcome.clone())
            .collect::<Vec<_>>()
    );
    night
        .word(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_ne!(night.id(), a);
    assert!(
        night.elims(&night.id()).await.is_empty(),
        "B was played with no Tab"
    );
}

#[tokio::test]
async fn one_read_on_b_then_c_does_not_seal_b_onto_a() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(8 * 60));
    night.word_once(detect::MatchOutcome::Defeat).await;
    let a = night.id();
    night.advance(Duration::from_secs(30));
    night.screen(vote(&["Junkertown"])).await;
    let b = night.id();
    assert_ne!(b, a);
    night.word_once(detect::MatchOutcome::Victory).await;
    night.advance(Duration::from_secs(20));
    night
        .tab_once(
            night_counters(6, 2, 1, 900, 200, 400),
            "Wrecking Ball",
            Some(0),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), b, "B's first Tab is B's board");
    night.advance(Duration::from_secs(130));
    night
        .tab_once(
            night_counters(2, 1, 0, 350, 60, 800),
            "Reinhardt",
            Some(1),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(night.id(), b, "C's gap closes B");
    assert_ne!(night.id(), a);
    let snaps = night.store.get_session_snapshots(&a).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.outcome == "defeat"),
        "C's gap must not seal B's word onto A"
    );
    let b_rows = night.store.get_session_snapshots(&b).await.unwrap();
    assert!(
        b_rows.iter().all(|row| row.outcome == "victory"),
        "B's one read is sealed on B"
    );
}

#[tokio::test]
async fn a_tab_on_b_does_not_move_bs_board_onto_c() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    // The debounce runs from when the session opened, the gap from the
    // last board. Open long enough for the vote, then take A's board.
    night.advance(Duration::from_secs(130));
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(20));
    night.word_once(detect::MatchOutcome::Defeat).await;
    let a = night.id();
    night.advance(Duration::from_secs(20));
    night.screen(vote(&["Junkertown"])).await;
    let b = night.id();
    night.advance(Duration::from_secs(20));
    night
        .tab_once(
            night_counters(3, 1, 1, 400, 80, 500),
            "Wrecking Ball",
            Some(0),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), b, "B's first Tab stays on B");
    assert_eq!(night.elims(&b).await, vec![3]);
    assert_eq!(night.elims(&a).await, vec![14]);
    night.advance(Duration::from_secs(130));
    night.screen(vote(&["Ilios"])).await;
    assert_ne!(night.id(), b);
    assert_eq!(night.elims(&a).await, vec![14]);
    assert_eq!(night.elims(&b).await, vec![3]);
}

#[tokio::test]
async fn fresh_misread_then_hero_swap_does_not_split() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    let id = night.id();
    night.advance(Duration::from_secs(60));
    night
        .tab_once(
            night_counters(2, 1, 0, 350, 60, 800),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), id);
    assert!(night.game().deferred.is_some(), "the misread is held");
    assert_eq!(night.elims(&id).await, vec![14]);
    night.advance(Duration::from_secs(30));
    night.screen(boundary::StartScreen::HeroSelect).await;
    assert_eq!(
        night.id(),
        id,
        "a hero swap after one misread does not split"
    );
    assert_eq!(night.elims(&id).await, vec![14]);
}

#[tokio::test]
async fn ordinary_growth_is_not_the_short_band_and_uses_the_baseline_clock() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Ana");
    night
        .tab_once(
            night_counters(4, 3, 2, 1500, 400, 300),
            "Ana",
            Some(0),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    // E4 at 2:00, E9 at 5:00. Three minutes of ordinary 2-4x growth.
    night.advance(Duration::from_secs(2 * 60));
    night
        .tab_once(
            night_counters(4, 3, 2, 1500, 400, 300),
            "Ana",
            Some(0),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(3 * 60));
    night
        .tab_once(
            night_counters(9, 7, 4, 3800, 900, 800),
            "Ana",
            Some(0),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.game().reset_baseline.map(|gate| gate.accepted.elims),
        Some(9),
        "a long-age 2-4x row becomes the baseline"
    );
    // Fresh session for the baseline clock. The first Tab is past the gap,
    // so the gate accepts the raw row instead of latching it to the
    // previous game. A short 2-4x row keeps the baseline. A spike at
    // 6:00 is stored and still is not the baseline. E17 twenty seconds
    // later is ordinary growth from that baseline.
    night.screen(vote(&["Ilios"])).await;
    night.advance(Duration::from_secs(130));
    night
        .tab_once(
            night_counters(4, 3, 2, 1500, 400, 300),
            "Ana",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(30));
    night
        .tab_once(
            night_counters(10, 8, 6, 4000, 1000, 800),
            "Ana",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.game().reset_baseline.map(|gate| gate.accepted.elims),
        Some(4),
        "the short 2-4x band does not move the baseline"
    );
    night.advance(Duration::from_secs(6 * 60 - 30));
    night
        .tab_once(
            night_counters(90, 40, 40, 30000, 50000, 5000),
            "Ana",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.game().reset_baseline.map(|gate| gate.accepted.elims),
        Some(4),
        "the spike is stored and is not the baseline"
    );
    night.advance(Duration::from_secs(20));
    night
        .tab_once(
            night_counters(17, 8, 5, 6100, 1200, 900),
            "Ana",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.game().reset_baseline.map(|gate| gate.accepted.elims),
        Some(17),
        "E17 at 6:20 is normal growth when age is measured from the baseline"
    );
}

#[tokio::test]
async fn a_deferred_board_is_dropped_when_the_match_continues() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(60));
    night
        .tab_once(
            night_counters(2, 1, 0, 350, 60, 800),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert!(night.game().deferred.is_some());
    night.advance(Duration::from_secs(10));
    night
        .tab_once(
            night_counters(16, 23, 6, 2600, 10000, 420),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert!(night.game().deferred.is_none());
    assert_eq!(night.elims(&night.id()).await, vec![14, 16]);
}

#[tokio::test]
async fn a_carried_deferred_board_is_written_on_the_new_session() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(18, 7, 9, 6400, 11000, 800),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(30));
    night
        .word(detect::MatchOutcome::Defeat, Some("Busan"))
        .await;
    night.advance(Duration::from_secs(50));
    let first = night_counters(1, 3, 0, 220, 80, 400);
    night
        .tab_once(first, "Wrecking Ball", Some(2), Some("Busan"), NIGHT_CLEAN)
        .await;
    assert_eq!(night.game().deferred, Some(first));
    assert!(!night.game().deferred_imported);
    let held_on = night.id();
    night.advance(Duration::from_secs(20));
    night.screen(boundary::StartScreen::HeroSelect).await;
    let opened = night.id();
    assert_ne!(opened, held_on, "the confirmed game closes");
    assert!(
        night.game().deferred_imported,
        "the held board is carried onto the session the start screen opened"
    );
    assert_eq!(night.game().deferred, Some(first));
    let next = night_counters(6, 2, 1, 900, 200, 400);
    night
        .tab_once(
            next,
            "Wrecking Ball",
            Some(0),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), opened, "the carried board does not split again");
    let rows = night.elims(&opened).await;
    assert!(
        rows.contains(&1),
        "the carried board is written on the new session, got {rows:?}"
    );
    assert!(rows.contains(&6), "the new session's own board is stored");
    assert!(
        !night.elims(&held_on).await.contains(&1),
        "the held board did not land on the session that deferred it"
    );
}

#[tokio::test]
async fn a_carried_board_is_stored_once_and_the_accolade_can_correct_the_map() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(18, 7, 9, 6400, 11000, 800),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(30));
    night
        .word(detect::MatchOutcome::Defeat, Some("Busan"))
        .await;
    night.advance(Duration::from_secs(50));
    let carried = night_counters(1, 3, 0, 220, 80, 400);
    night
        .tab_once(
            carried,
            "Wrecking Ball",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.game().deferred, Some(carried));
    night.advance(Duration::from_secs(20));
    night.screen(boundary::StartScreen::HeroSelect).await;
    let opened = night.id();
    assert!(
        night.game().deferred_imported,
        "the held board is carried onto the session the start screen opened"
    );
    night.advance(Duration::from_secs(10));
    night
        .tab_text(
            night_counters(3, 1, 0, 400, 100, 200),
            "Wrecking Ball",
            None,
            "Dorado",
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), opened);
    assert_eq!(night.game().map.as_deref(), Some("Dorado"));
    assert_eq!(
        night.game().map_source,
        Some(boundary::MapSource::TextFallback)
    );
    night.advance(Duration::from_secs(10));
    night
        .tab_text(
            night_counters(4, 2, 1, 500, 150, 250),
            "Wrecking Ball",
            None,
            "Dorado",
            NIGHT_CLEAN,
        )
        .await;
    let rows = night.elims(&opened).await;
    assert_eq!(
        rows.iter().filter(|elims| **elims == 1).count(),
        1,
        "the carried board is stored once, got {rows:?}"
    );
    assert!(
        rows.contains(&3) && rows.contains(&4),
        "each unidentified Tab is stored on its own, got {rows:?}"
    );
    // The hold is still set here unless the first store cleared it. A
    // later identified Tab would clear it by refreshing the baseline,
    // which would hide the leftover flag.
    night.advance(Duration::from_secs(8 * 60));
    night
        .word(detect::MatchOutcome::Victory, Some("Ilios"))
        .await;
    assert_eq!(night.id(), opened, "the accolade stays on this session");
    assert_eq!(night.game().map.as_deref(), Some("Ilios"));
    assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
    let snaps = night.store.get_session_snapshots(&opened).await.unwrap();
    assert!(
        !snaps.is_empty() && snaps.iter().all(|row| row.map_name == "Ilios"),
        "the accolade rewrites the map after the hold is cleared, got {:?}",
        snaps
            .iter()
            .map(|row| row.map_name.clone())
            .collect::<Vec<_>>()
    );
    night.advance(Duration::from_secs(10));
    night
        .tab_text(
            night_counters(5, 2, 1, 600, 180, 280),
            "Wrecking Ball",
            Some(0),
            "Ilios",
            NIGHT_CLEAN,
        )
        .await;
    let rows = night.elims(&opened).await;
    assert_eq!(
        rows.iter().filter(|elims| **elims == 1).count(),
        1,
        "the identified Tab does not store the carried board again, got {rows:?}"
    );
    assert!(
        rows.contains(&5),
        "the identified Tab is stored on its own, got {rows:?}"
    );
}

#[tokio::test]
async fn one_defeat_then_post_match_tab_then_vote_leaves_a_as_defeat() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(8, 3, 2, 1800, 4000, 100),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(30));
    night.word_once(detect::MatchOutcome::Defeat).await;
    night.advance(Duration::from_secs(20));
    night
        .tab_once(
            night_counters(10, 4, 2, 2200, 4600, 140),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.game().result_mark.map(|mark| mark.outcome),
        Some(detect::MatchOutcome::Defeat),
        "the first progressed board keeps the hint"
    );
    let a = night.id();
    night.advance(Duration::from_secs(130));
    night.screen(vote(&["Junkertown"])).await;
    assert_ne!(night.id(), a);
    let snaps = night.store.get_session_snapshots(&a).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.outcome == "defeat"),
        "the vote seals the hint the post-match Tab kept"
    );
}

#[tokio::test]
async fn a_vote_session_keeps_its_first_tab_and_its_candidates() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    let a = night.id();
    night.advance(Duration::from_secs(130));
    night.screen(vote(&["Junkertown", "Ilios"])).await;
    let b = night.id();
    assert_ne!(b, a);
    assert!(night.game().awaiting_first_board);
    night.advance(Duration::from_secs(130));
    night
        .tab_once(
            night_counters(2, 1, 0, 350, 60, 800),
            "Wrecking Ball",
            Some(0),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.id(),
        b,
        "the first Tab stays on the session the vote opened"
    );
    assert_eq!(night.elims(&b).await, vec![2]);
    assert_eq!(night.elims(&a).await, vec![14]);
    assert_eq!(
        night.game().map_candidates,
        vec!["Junkertown".to_string(), "Ilios".to_string()]
    );
    let unrecorded = std::fs::read_to_string(
        night
            .dir
            .path()
            .join("debug")
            .join("unrecorded_games.jsonl"),
    )
    .unwrap_or_default();
    assert!(
        !unrecorded.contains(&b),
        "the vote session was recorded: {unrecorded}"
    );
}

#[tokio::test]
async fn a_normal_night_is_one_session_per_game() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_held(
            night_counters(10, 4, 3, 2000, 800, 100),
            "Zenyatta",
            Some(0),
            Some("Busan"),
        )
        .await;
    night.advance(Duration::from_secs(8 * 60));
    night
        .word(detect::MatchOutcome::Victory, Some("Busan"))
        .await;
    let busan = night.id();
    night.advance(Duration::from_secs(20));
    night.screen(vote(&["Junkertown", "Ilios"])).await;
    assert_ne!(night.id(), busan);
    assert_eq!(night.game().map_candidates, vec!["Junkertown", "Ilios"]);
    let vote_session = night.id();
    night.advance(Duration::from_secs(150));
    night.screen(boundary::StartScreen::HeroSelect).await;
    night.advance(Duration::from_secs(150));
    night.screen(boundary::StartScreen::HeroSelect).await;
    assert_eq!(
        night.id(),
        vote_session,
        "a hero swap before the first Tab, after the debounce, stays"
    );
    assert_eq!(night.game().map_candidates, vec!["Junkertown", "Ilios"]);
    night.advance(Duration::from_secs(30));
    night
        .tab_held(
            night_counters(4, 6, 2, 900, 3000, 200),
            "Ana",
            Some(1),
            Some("Junkertown"),
        )
        .await;
    let junkertown = night.id();
    assert_eq!(
        junkertown, vote_session,
        "the vote session keeps its first Tab"
    );
    assert_eq!(night.game().map_candidates, vec!["Junkertown", "Ilios"]);
    let unrecorded = std::fs::read_to_string(
        night
            .dir
            .path()
            .join("debug")
            .join("unrecorded_games.jsonl"),
    )
    .unwrap_or_default();
    assert!(
        !unrecorded.contains(&vote_session),
        "the vote session was recorded: {unrecorded}"
    );
    assert_ne!(junkertown, busan);
    night.advance(Duration::from_secs(8 * 60));
    night
        .word(detect::MatchOutcome::Defeat, Some("Junkertown"))
        .await;
    night.advance(Duration::from_secs(20));
    night.screen(boundary::StartScreen::HeroSelect).await;
    night.advance(Duration::from_secs(30));
    night
        .tab_held(
            night_counters(6, 2, 1, 1500, 200, 4000),
            "Reinhardt",
            Some(0),
            Some("Ilios"),
        )
        .await;
    let ilios = night.id();
    assert_ne!(ilios, junkertown);
    night.advance(Duration::from_secs(8 * 60));
    night
        .word(detect::MatchOutcome::Victory, Some("Ilios"))
        .await;
    assert_eq!(night.id(), ilios);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(night.game().map.as_deref(), Some("Ilios"));
    let busan_rows = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(busan_rows.iter().all(|row| row.outcome == "victory"));
    let junk_rows = night
        .store
        .get_session_snapshots(&junkertown)
        .await
        .unwrap();
    assert!(junk_rows.iter().all(|row| row.outcome == "defeat"));
    assert!(night.elims(&ilios).await.contains(&6));
}

#[tokio::test]
async fn busan_board_one_defeat_then_junkertown_wrecking_ball() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(8 * 60));
    night.word_once(detect::MatchOutcome::Defeat).await;
    let busan = night.id();
    night.advance(Duration::from_secs(60));
    night
        .tab_once(
            night_counters(4, 3, 2, 1500, 400, 300),
            "Wrecking Ball",
            Some(0),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    let junk = night.id();
    assert_ne!(junk, busan, "the nine-minute gap splits with the gap on");
    let busan_rows = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(busan_rows.iter().all(|row| row.outcome == "defeat"));
    assert_eq!(night.elims(&busan).await, vec![14]);
    night.advance(Duration::from_secs(3 * 60));
    night
        .tab_once(
            night_counters(9, 7, 4, 3800, 900, 800),
            "Wrecking Ball",
            Some(0),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.game().reset_baseline.map(|gate| gate.accepted.elims),
        Some(9),
        "a long-age 2-4x row becomes the baseline"
    );
    night.advance(Duration::from_secs(3 * 60));
    let final_board = night_counters(23, 9, 5, 6792, 1463, 3006);
    night
        .tab_once(
            final_board,
            "Wrecking Ball",
            Some(0),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert!(
        night.elims(&junk).await.contains(&23),
        "the final board is stored"
    );
    assert_eq!(
        night.game().gate.map(|gate| gate.accepted),
        Some(final_board)
    );
    night.advance(Duration::from_secs(10 * 60));
    let over = night_counters(100, 20, 12, 14000, 3000, 6000);
    night
        .tab_once(
            over,
            "Wrecking Ball",
            Some(0),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert!(
        night.elims(&junk).await.contains(&100),
        "a long-age row past 4x is stored"
    );
    night
        .word(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
    assert_eq!(
        night.game().hero_auth.accepted_hero.as_deref(),
        Some("Wrecking Ball")
    );
}

#[tokio::test]
async fn defeat_word_with_no_board_and_no_start_screen_then_junkertown() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night.word_once(detect::MatchOutcome::Defeat).await;
    let busan = night.id();
    night.advance(Duration::from_secs(9 * 60));
    night
        .tab_once(
            night_counters(2, 1, 0, 350, 60, 800),
            "Wrecking Ball",
            Some(0),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(
        night.id(),
        busan,
        "a hinted session with no board splits when the first Tab names another map"
    );
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(
        snaps.is_empty() || snaps.iter().all(|row| row.outcome == "defeat"),
        "the hint is sealed on the session that had no board"
    );
    assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
}

#[tokio::test]
async fn a_same_map_tab_after_a_boardless_hint_stays() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night.word_once(detect::MatchOutcome::Defeat).await;
    let busan = night.id();
    night.advance(Duration::from_secs(9 * 60));
    night
        .tab_once(
            night_counters(8, 3, 2, 1800, 400, 100),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.id(),
        busan,
        "a late Tab of the same map is this match"
    );
    assert_eq!(night.elims(&busan).await, vec![8]);
}

#[tokio::test]
async fn grace_then_idle_each_open_a_fresh_session() {
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
    night
        .word(detect::MatchOutcome::Victory, Some("Busan"))
        .await;
    let finished = night.id();
    night.advance(Duration::from_secs(80));
    night
        .tab_once(
            night_counters(2, 1, 0, 400, 80, 200),
            "Ana",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(
        night.id(),
        finished,
        "past the 75s grace the next Tab is a new session"
    );
    let unfinished = night.id();
    night.advance(Duration::from_secs(21 * 60));
    night
        .tab_once(
            night_counters(4, 2, 1, 900, 100, 300),
            "Ana",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(
        night.id(),
        unfinished,
        "an unfinished session idle for 20 minutes does not absorb the next Tab"
    );
}

#[tokio::test]
async fn a_ban_after_a_held_misread_does_not_split() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    let id = night.id();
    night.advance(Duration::from_secs(60));
    night
        .tab_once(
            night_counters(2, 1, 0, 350, 60, 800),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert!(night.game().deferred.is_some());
    night.screen(boundary::StartScreen::HeroBan).await;
    assert_eq!(
        night.id(),
        id,
        "a ban after one held misread does not split"
    );
    assert_eq!(night.elims(&id).await, vec![14]);
}
