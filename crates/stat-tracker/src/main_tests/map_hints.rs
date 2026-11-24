//! Daemon tests: map_hints. See `mod.rs`.

use super::*;

#[tokio::test]
async fn an_armed_different_map_end_screen_seals_the_old_hint() {
    let mut night = Night::new().await;
    let busan = arm_busan_defeat(&mut night).await;
    night.advance(Duration::from_secs(20));
    // Two agreeing word reads, the way the poller confirms. The first
    // must not replace Busan's hint.
    night
        .word(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_ne!(night.id(), busan);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
    assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.outcome == "defeat"),
        "B's confirmed result does not overwrite A, got {:?}",
        snaps
            .iter()
            .map(|row| row.outcome.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_single_different_map_word_does_not_replace_the_hint_before_a_vote() {
    let mut night = Night::new().await;
    let busan = arm_busan_defeat(&mut night).await;
    night
        .word_tick(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_eq!(night.id(), busan);
    assert_eq!(
        night.game().result_mark.map(|mark| mark.outcome),
        Some(detect::MatchOutcome::Defeat),
        "one unconfirmed word on the next map leaves A's hint"
    );
    night.advance(Duration::from_secs(150));
    night
        .screen(boundary::StartScreen::MapVote {
            candidates: vec!["Junkertown".into(), "Ilios".into()],
        })
        .await;
    assert_ne!(night.id(), busan);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Unknown);
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.outcome == "defeat"),
        "C's vote seals A's defeat, got {:?}",
        snaps
            .iter()
            .map(|row| row.outcome.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn an_armed_result_without_a_map_seals_onto_the_open_session() {
    let mut night = Night::new().await;
    let busan = arm_busan_defeat(&mut night).await;
    night
        .confirmed_without_map(detect::MatchOutcome::Victory)
        .await;
    assert_eq!(night.id(), busan);
    assert_eq!(
        night.game().outcome,
        detect::MatchOutcome::Victory,
        "a result with no accolade map still seals onto A"
    );
}

#[tokio::test]
async fn an_armed_end_title_without_a_map_does_not_replace_the_hint() {
    let mut night = Night::new().await;
    let busan = arm_busan_defeat(&mut night).await;
    night.word_tick(detect::MatchOutcome::Victory, None).await;
    assert_eq!(night.id(), busan);
    assert_eq!(
        night.game().result_mark.map(|mark| mark.outcome),
        Some(detect::MatchOutcome::Defeat),
        "an end title with no map leaves A's hint"
    );
    night.advance(Duration::from_secs(4));
    night
        .word_tick(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_ne!(night.id(), busan, "the confirming accolade opens B");
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.outcome == "defeat"),
        "A keeps its own result, got {:?}",
        snaps
            .iter()
            .map(|row| row.outcome.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn an_accolade_map_carries_into_the_rank_screen_confirmation() {
    let mut night = Night::new().await;
    let busan = arm_busan_defeat(&mut night).await;
    night
        .word_tick(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_eq!(night.id(), busan);
    assert_eq!(
        night.game().result_mark.map(|mark| mark.outcome),
        Some(detect::MatchOutcome::Defeat)
    );
    night.advance(Duration::from_secs(4));
    night.word_tick(detect::MatchOutcome::Victory, None).await;
    assert_ne!(
        night.id(),
        busan,
        "the rank screen confirms with the map carried from the accolade"
    );
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
    assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.outcome == "defeat"),
        "A keeps its own result, got {:?}",
        snaps
            .iter()
            .map(|row| row.outcome.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn an_unarmed_trusted_difference_splits_and_a_missing_name_seals() {
    let mut split = Night::new().await;
    split.begin_on("Busan", "Zenyatta");
    split
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    split.word_once(detect::MatchOutcome::Defeat).await;
    let busan = split.id();
    split.advance(Duration::from_secs(4));
    split
        .word(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_ne!(split.id(), busan);
    assert_eq!(split.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(split.game().map.as_deref(), Some("Junkertown"));
    let snaps = split.store.get_session_snapshots(&busan).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.outcome == "defeat"),
        "an unarmed trusted difference still leaves A's result, got {:?}",
        snaps
            .iter()
            .map(|row| row.outcome.clone())
            .collect::<Vec<_>>()
    );

    let mut seal = Night::new().await;
    seal.begin_on("Busan", "Zenyatta");
    seal.tab_once(
        night_counters(14, 22, 6, 2400, 9800, 400),
        "Zenyatta",
        Some(2),
        Some("Busan"),
        NIGHT_CLEAN,
    )
    .await;
    seal.word_once(detect::MatchOutcome::Defeat).await;
    let busan = seal.id();
    seal.word(detect::MatchOutcome::Victory, None).await;
    assert_eq!(
        seal.id(),
        busan,
        "a confirming word with no trusted name seals onto the open session"
    );
    assert_eq!(seal.game().outcome, detect::MatchOutcome::Victory);
}

#[tokio::test]
async fn a_text_fallback_sessions_own_accolade_replaces_the_map() {
    let mut night = Night::new().await;
    let dorado = text_fallback(&mut night, "Dorado").await;
    night
        .word_tick(detect::MatchOutcome::Victory, Some("Busan"))
        .await;
    assert_eq!(
        night.id(),
        dorado,
        "this session's own accolade does not split"
    );
    assert_eq!(night.game().map.as_deref(), Some("Busan"));
    assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
    let snaps = night.store.get_session_snapshots(&dorado).await.unwrap();
    assert!(
        !snaps.is_empty() && snaps.iter().all(|row| row.map_name == "Busan"),
        "the adopted accolade rewrites the snapshots, got {:?}",
        snaps
            .iter()
            .map(|row| row.map_name.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn an_armed_text_fallback_is_not_relabelled_by_the_next_map() {
    let mut night = Night::new().await;
    let busan = text_fallback(&mut night, "Busan").await;
    night.word_once(detect::MatchOutcome::Defeat).await;
    night.screen(boundary::StartScreen::HeroSelect).await;
    assert!(night.game().pending_boundary);
    night
        .word(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_eq!(night.id(), busan);
    assert_eq!(night.game().map.as_deref(), Some("Busan"));
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.map_name == "Busan"),
        "the next accolade must not rewrite A's snapshots, got {:?}",
        snaps
            .iter()
            .map(|row| row.map_name.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_text_fallback_hint_is_not_relabelled_by_the_next_result() {
    let mut night = Night::new().await;
    let busan = text_fallback(&mut night, "Busan").await;
    night.word_once(detect::MatchOutcome::Defeat).await;
    night
        .word_tick(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert!(night.game().text_fallback_locked);
    night.advance(Duration::from_secs(4));
    night.no_signal_tick().await;
    assert!(
        night.game().text_fallback_locked,
        "a tick with no result word keeps the text-fallback lock"
    );
    night.advance(Duration::from_secs(4));
    night
        .word(detect::MatchOutcome::Victory, Some("Junkertown"))
        .await;
    assert_eq!(night.id(), busan);
    assert_eq!(night.game().map.as_deref(), Some("Busan"));
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(snaps.iter().all(|row| row.map_name == "Busan"));
}

#[tokio::test]
async fn progressed_boards_clear_the_lock_so_the_accolade_can_correct_the_map() {
    let mut night = Night::new().await;
    let dorado = text_fallback(&mut night, "Dorado").await;
    night.word_once(detect::MatchOutcome::Defeat).await;
    night.word_once(detect::MatchOutcome::Victory).await;
    assert!(
        night.game().text_fallback_locked,
        "a different result replaces the hint and locks the text map"
    );
    night.advance(Duration::from_secs(70));
    night
        .tab_text(
            night_counters(16, 24, 7, 2800, 10400, 500),
            "Zenyatta",
            Some(2),
            "Dorado",
            NIGHT_CLEAN,
        )
        .await;
    assert!(
        night.game().result_mark.is_some(),
        "the first progressed board keeps the hint"
    );
    assert!(night.game().text_fallback_locked);
    night.advance(Duration::from_secs(60));
    night
        .tab_text(
            night_counters(18, 26, 8, 3200, 11000, 600),
            "Zenyatta",
            Some(2),
            "Dorado",
            NIGHT_CLEAN,
        )
        .await;
    assert!(
        night.game().result_mark.is_none(),
        "the second progressed board clears the hint"
    );
    assert!(
        !night.game().text_fallback_locked,
        "clearing the hint clears the text-fallback lock"
    );
    night.advance(Duration::from_secs(5 * 60));
    night
        .word(detect::MatchOutcome::Victory, Some("Busan"))
        .await;
    assert_eq!(night.id(), dorado);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(night.game().map.as_deref(), Some("Busan"));
    assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
    let snaps = night.store.get_session_snapshots(&dorado).await.unwrap();
    assert_eq!(
        snaps.len(),
        3,
        "the opening board and both progressed boards"
    );
    assert!(
        snaps.iter().all(|row| row.map_name == "Busan"),
        "the corrected accolade rewrites every snapshot, got {:?}",
        snaps
            .iter()
            .map(|row| row.map_name.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_finished_text_fallback_session_keeps_its_map() {
    let mut night = Night::new().await;
    let busan = text_fallback(&mut night, "Busan").await;
    night
        .confirmed_without_map(detect::MatchOutcome::Victory)
        .await;
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(night.game().map.as_deref(), Some("Busan"));
    night
        .word(detect::MatchOutcome::Defeat, Some("Junkertown"))
        .await;
    assert_eq!(night.id(), busan);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(night.game().map.as_deref(), Some("Busan"));
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(snaps.iter().all(|row| row.map_name == "Busan"));
}

#[tokio::test]
async fn a_word_with_no_open_game_still_notes_the_streak() {
    let mut night = Night::new().await;
    night
        .word_tick(detect::MatchOutcome::Victory, Some("Busan"))
        .await;
    assert!(night.st.active_game.is_none());
    assert_eq!(
        night
            .st
            .word_outcome_streak
            .as_ref()
            .map(|streak| streak.outcome),
        Some(detect::MatchOutcome::Victory)
    );
    night.advance(Duration::from_secs(4));
    night.word_tick(detect::MatchOutcome::Victory, None).await;
    assert_eq!(
        night.st.pending_outcome.map(|(outcome, _)| outcome),
        Some(detect::MatchOutcome::Victory)
    );
    assert_eq!(
        night
            .st
            .word_outcome_streak
            .as_ref()
            .and_then(|streak| streak.map.as_deref()),
        Some("Busan")
    );
}

#[tokio::test]
async fn an_accolade_adopts_the_map_source() {
    let mut night = Night::new().await;
    let g = ActiveGame::open_at(
        format!("{:016x}", rand_id()),
        detect::MatchOutcome::Unknown,
        Vec::new(),
        night.now,
    );
    night.st.active_game = Some(g);
    night.st.last_game_open = Some(night.now);
    night
        .word_tick(detect::MatchOutcome::Victory, Some("Busan"))
        .await;
    assert_eq!(night.game().map.as_deref(), Some("Busan"));
    assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
}

#[tokio::test]
async fn a_stat_split_session_keeps_the_top_bar_map_source() {
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
    night.word_once(detect::MatchOutcome::Defeat).await;
    let busan = night.id();
    night.advance(Duration::from_secs(150));
    night
        .tab_once(
            night_counters(16, 23, 7, 2800, 10000, 500),
            "Zenyatta",
            Some(2),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(night.id(), busan);
    assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
    assert_eq!(night.game().map_source, Some(boundary::MapSource::TopBar));
}

#[tokio::test]
async fn a_later_top_bar_upgrades_a_text_fallback_and_the_next_map_splits() {
    let mut night = Night::new().await;
    let mut g = ActiveGame::open_at(
        format!("{:016x}", rand_id()),
        detect::MatchOutcome::Unknown,
        Vec::new(),
        night.now,
    );
    g.hero_auth.accepted_hero = Some("Zenyatta".into());
    night.st.active_game = Some(g);
    night.st.last_game_open = Some(night.now);
    night
        .tab_text(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            "Busan",
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.game().map_source,
        Some(boundary::MapSource::TextFallback)
    );
    night.advance(Duration::from_secs(10));
    night
        .tab_once(
            night_counters(16, 23, 7, 2600, 10000, 450),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.game().map.as_deref(), Some("Busan"));
    assert_eq!(night.game().map_source, Some(boundary::MapSource::TopBar));
    night.word_once(detect::MatchOutcome::Defeat).await;
    let busan = night.id();
    night.advance(Duration::from_secs(150));
    night
        .tab_once(
            night_counters(18, 24, 8, 3000, 11000, 520),
            "Zenyatta",
            Some(2),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(
        night.id(),
        busan,
        "once the top bar agrees, a later different map still splits"
    );
}

#[tokio::test]
async fn a_different_map_inside_the_gap_stays_one_game() {
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
    night.word_once(detect::MatchOutcome::Defeat).await;
    let id = night.id();
    night.advance(Duration::from_secs(60));
    night
        .tab_once(
            night_counters(16, 23, 7, 2800, 10000, 500),
            "Zenyatta",
            Some(2),
            Some("Junkertown"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), id, "inside 120s a different top bar stays");
}

#[tokio::test]
async fn a_same_map_tab_after_the_gap_stays_one_game() {
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
    night.word_once(detect::MatchOutcome::Defeat).await;
    let id = night.id();
    night.advance(Duration::from_secs(150));
    night
        .tab_once(
            night_counters(16, 23, 7, 2800, 10000, 500),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), id);
}

/// F251-1: a text-fallback session is renamed by a later sure banner
/// read, rows included, and keeps the new trusted map.
/// F252-3: a member's map pick on a text-fallback session is never
/// renamed by a banner read, and its stored mode is left alone.
#[tokio::test]
async fn a_banner_read_never_renames_a_member_map_pick() {
    let mut night = Night::new().await;
    let id = text_fallback(&mut night, "Busan").await;
    night
        .store
        .apply_command(&storage::StoreCommand::EditMatch {
            session_id: id.clone(),
            edit: storage::MatchEdit {
                map_name: Some("King's Row".into()),
                ..Default::default()
            },
        })
        .await
        .unwrap();
    let before = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(
        before
            .iter()
            .all(|r| r.corrected_map_name.as_deref() == Some("King's Row"))
    );
    night.advance(Duration::from_secs(10));
    night
        .tab_banner(
            night_counters(16, 23, 7, 2600, 10000, 450),
            "Zenyatta",
            Some(2),
            "Ilios",
        )
        .await;
    assert_eq!(night.game().map.as_deref(), Some("Busan"));
    let after = night.store.get_session_snapshots(&id).await.unwrap();
    for row in &after[..before.len()] {
        assert_eq!(row.corrected_map_name.as_deref(), Some("King's Row"));
        assert_ne!(row.map_name, "Ilios");
    }
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(
            a.game_mode, b.game_mode,
            "the member's mode is not rewritten"
        );
    }
}

/// F252-2: a sure banner read after a gap longer than the splitter's
/// STAT_SPLIT_MIN_GAP does not rename the text-fallback session: game 1's
/// end may have been missed, and this Tab may be game 2.
#[tokio::test]
async fn a_banner_read_after_a_long_gap_does_not_rename() {
    let mut night = Night::new().await;
    let id = text_fallback(&mut night, "Busan").await;
    night.advance(STAT_SPLIT_MIN_GAP + Duration::from_secs(30));
    night
        .tab_banner(
            night_counters(16, 23, 7, 2600, 10000, 450),
            "Zenyatta",
            Some(2),
            "Ilios",
        )
        .await;
    let snaps = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(
        snaps.iter().all(|row| row.map_name == "Busan"),
        "{:?}",
        snaps.iter().map(|r| r.map_name.clone()).collect::<Vec<_>>()
    );
    if night.id() == id {
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
    }
}

/// F252-1: after a rename, the game's own accolade that still names the
/// replaced text-fallback map is this game's end screen: the outcome
/// lands on the renamed session, not on a new empty one.
#[tokio::test]
async fn the_accolade_of_a_renamed_game_seals_it() {
    let mut night = Night::new().await;
    let id = text_fallback(&mut night, "Busan").await;
    night.advance(Duration::from_secs(10));
    night
        .tab_banner(
            night_counters(16, 23, 7, 2600, 10000, 450),
            "Zenyatta",
            Some(2),
            "Ilios",
        )
        .await;
    assert_eq!(night.game().map.as_deref(), Some("Ilios"));
    night
        .word(detect::MatchOutcome::Victory, Some("Busan"))
        .await;
    assert_eq!(night.id(), id, "no split on the old name");
    assert_eq!(night.game().map.as_deref(), Some("Ilios"));
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    let snaps = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(snaps.iter().all(|row| row.map_name == "Ilios"));
}

#[tokio::test]
async fn a_sure_banner_read_replaces_a_text_fallback_session_map() {
    let mut night = Night::new().await;
    let id = text_fallback(&mut night, "Busan").await;
    night.advance(Duration::from_secs(10));
    night
        .tab_banner(
            night_counters(16, 23, 7, 2600, 10000, 450),
            "Zenyatta",
            Some(2),
            "Ilios",
        )
        .await;
    assert_eq!(night.id(), id, "same game, no split");
    assert_eq!(night.game().map.as_deref(), Some("Ilios"));
    assert_eq!(night.game().map_source, Some(boundary::MapSource::TopBar));
    let snaps = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(!snaps.is_empty());
    assert!(
        snaps
            .iter()
            .all(|row| row.map_name == "Ilios" && row.game_mode == "Control"),
        "{:?}",
        snaps.iter().map(|r| r.map_name.clone()).collect::<Vec<_>>()
    );
    // first trusted map wins from here on
    night.advance(Duration::from_secs(10));
    night
        .tab_banner(
            night_counters(18, 24, 8, 2800, 10400, 500),
            "Zenyatta",
            Some(2),
            "Nepal",
        )
        .await;
    assert_eq!(night.game().map.as_deref(), Some("Ilios"));
}

/// The other direction: a trusted session map is never replaced, the
/// top-bar OCR never replaces a text fallback (ocr-v1 unchanged), and a
/// banner read after a result does not rename the finished game.
#[tokio::test]
async fn a_banner_read_keeps_a_trusted_or_closed_session_map() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_banner(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            "Ilios",
        )
        .await;
    assert_eq!(
        night.game().map.as_deref(),
        Some("Busan"),
        "trusted top bar"
    );
    assert_eq!(night.game().map_source, Some(boundary::MapSource::TopBar));
    let snaps = night
        .store
        .get_session_snapshots(&night.id())
        .await
        .unwrap();
    assert!(!snaps.is_empty());
    assert!(
        snaps.iter().all(|row| row.map_name == "Busan"),
        "the rows keep the trusted map"
    );

    let mut night = Night::new().await;
    let id = text_fallback(&mut night, "Busan").await;
    night.advance(Duration::from_secs(10));
    night
        .tab_once(
            night_counters(16, 23, 7, 2600, 10000, 450),
            "Zenyatta",
            Some(2),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.game().map.as_deref(), Some("Busan"), "top-bar OCR");
    let snaps = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(snaps.iter().all(|row| row.map_name == "Busan"));

    let mut night = Night::new().await;
    let id = text_fallback(&mut night, "Busan").await;
    night.word_once(detect::MatchOutcome::Defeat).await;
    night.advance(Duration::from_secs(10));
    night
        .tab_banner(
            night_counters(16, 23, 7, 2600, 10000, 450),
            "Zenyatta",
            Some(2),
            "Ilios",
        )
        .await;
    assert_eq!(night.id(), id);
    assert_eq!(night.game().map.as_deref(), Some("Busan"), "after a result");
    let snaps = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(snaps.iter().all(|row| row.map_name == "Busan"));
}

#[tokio::test]
async fn a_text_fallback_map_does_not_split_on_a_later_top_bar() {
    let mut night = Night::new().await;
    let mut g = ActiveGame::open_at(
        format!("{:016x}", rand_id()),
        detect::MatchOutcome::Unknown,
        Vec::new(),
        night.now,
    );
    g.hero_auth.accepted_hero = Some("Zenyatta".into());
    night.st.active_game = Some(g);
    night.st.last_game_open = Some(night.now);
    night
        .tab_text(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            "Dorado",
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.game().map.as_deref(), Some("Dorado"));
    assert_eq!(
        night.game().map_source,
        Some(boundary::MapSource::TextFallback)
    );
    night.word_once(detect::MatchOutcome::Defeat).await;
    let id = night.id();
    night.advance(Duration::from_secs(150));
    night
        .tab_once(
            night_counters(16, 23, 7, 2800, 10000, 500),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.id(),
        id,
        "a text-fallback map does not make the post-match Tab a new game"
    );
}

#[tokio::test]
async fn an_unidentified_row_leaves_the_held_board_for_the_split() {
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
            night_counters(2, 1, 0, 200, 40, 80),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.game().deferred.map(|c| c.elims), Some(2));
    assert!(night.game().reset_streak >= 1);
    night.advance(Duration::from_secs(10));
    night
        .tab_once(
            night_counters(9, 4, 3, 1500, 400, 200),
            "Zenyatta",
            None,
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.game().deferred.map(|c| c.elims),
        Some(2),
        "an unidentified row leaves the held board"
    );
    assert!(night.game().reset_streak >= 1);
    let finished = night.id();
    night.advance(Duration::from_secs(60));
    night
        .tab_once(
            night_counters(1, 0, 0, 80, 10, 20),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(night.id(), finished);
    let elims = night.elims(&night.id()).await;
    assert!(
        elims.contains(&2),
        "the split writes the held board onto the new session, got {elims:?}"
    );
}

#[tokio::test]
async fn a_held_board_is_written_on_a_tab_opened_session() {
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
    night
        .word(detect::MatchOutcome::Victory, Some("Busan"))
        .await;
    night.advance(Duration::from_secs(60));
    night
        .tab_once(
            night_counters(2, 1, 0, 200, 40, 80),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert!(night.game().deferred.is_some());
    let finished = night.id();
    night.advance(Duration::from_secs(80));
    night
        .tab_once(
            night_counters(4, 2, 1, 900, 100, 200),
            "Ana",
            Some(0),
            Some("Ilios"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(night.id(), finished);
    let elims = night.elims(&night.id()).await;
    assert!(
        elims.contains(&2),
        "grace opens a session and writes the held board, got {elims:?}"
    );
    assert!(!night.elims(&finished).await.contains(&2));
}

#[tokio::test]
async fn a_rejected_frame_adopts_the_outcome_and_clears_the_arm() {
    let mut night = Night::new().await;
    let busan = arm_busan_defeat(&mut night).await;
    let report = CaptureReport {
        best_guess: None,
        recorded: false,
        outcome: detect::MatchOutcome::Victory,
        map: None,
        map_source: None,
        session_id: busan.clone(),
        split: false,
        armed_reset: false,
        ignore_row: true,
        reset_streak: night.game().reset_streak,
        reset_baseline: night.game().reset_baseline,
        baseline_row: night.game().baseline_row,
        refresh_baseline: false,
        clear_hint: false,
        count_progress: false,
        career_panel: false,
        held_counters: None,
        held_hero: None,
        gate_state: None,
        hero_auth: night.game().hero_auth.clone(),
        seal: None,
        close_reason: None,
        held_at: None,
        map_replaces_text_fallback: false,
        rejoin_carry: None,
        finished_game_tab: None,
    };
    apply_capture_report(
        &mut night.st,
        &night.store,
        night.dir.path(),
        &busan,
        Ok(report),
        night.now,
    )
    .await;
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert!(
        !night.game().pending_boundary,
        "adopting the rejected frame's outcome clears the armed boundary"
    );
}

/// MAIN-1: the end-reel poll decides the game while a Tab capture that
/// was requested before the result is still running. The capture's
/// writes do not reset the session to unknown, and its row, the newest
/// and most complete one, carries the result and is queued for upload.
#[tokio::test]
async fn a_result_decided_while_a_tab_capture_runs_survives_the_capture() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Ana");
    night
        .tab_once(
            night_counters(5, 2, 1, 2000, 300, 500),
            "Ana",
            Some(0),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    let busan = night.id();
    night.advance(Duration::from_secs(30));
    let req = night.tab_request().await;
    assert_eq!(req.game_outcome, detect::MatchOutcome::Unknown);
    night
        .confirmed_without_map(detect::MatchOutcome::Victory)
        .await;
    assert_eq!(night.id(), busan);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    night
        .tab_land(
            req,
            night_counters(9, 3, 2, 3500, 400, 800),
            "Ana",
            Some(0),
            (Some("Busan"), "Busan"),
            NIGHT_CLEAN,
            detect::MatchOutcome::Unknown,
        )
        .await;
    assert_eq!(night.id(), busan);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(
        snaps.iter().any(|row| row.elims == 9),
        "the late row is stored"
    );
    assert!(
        snaps.iter().all(|row| row.outcome == "victory"),
        "{:?}",
        snaps
            .iter()
            .map(|row| (row.elims, row.outcome.clone()))
            .collect::<Vec<_>>()
    );
    let sessions = night.store.get_all_sessions().await.unwrap();
    let stored = sessions
        .iter()
        .find(|s| s.session_id == busan)
        .expect("session row");
    assert_eq!(stored.final_outcome, "victory");
    let unsynced = night.store.get_unsynced().await.unwrap();
    assert!(
        unsynced
            .iter()
            .any(|row| row.session_id == busan && row.elims == 9),
        "the late row is not held back from upload"
    );
}

/// MAIN-1: the first Tab of a game is still running when the result is
/// decided. The session row does not exist yet, so the store has no
/// result to keep; the report re-seals the decided one.
#[tokio::test]
async fn a_first_capture_that_lands_after_the_result_is_re_sealed() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Ana");
    let busan = night.id();
    let req = night.tab_request().await;
    assert!(req.create_session);
    night
        .confirmed_without_map(detect::MatchOutcome::Victory)
        .await;
    assert_eq!(night.id(), busan);
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    night
        .tab_land(
            req,
            night_counters(9, 3, 2, 3500, 400, 800),
            "Ana",
            Some(0),
            (Some("Busan"), "Busan"),
            NIGHT_CLEAN,
            detect::MatchOutcome::Unknown,
        )
        .await;
    assert_eq!(night.id(), busan);
    let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
    assert!(!snaps.is_empty());
    assert!(
        snaps.iter().all(|row| row.outcome == "victory"),
        "{:?}",
        snaps
            .iter()
            .map(|row| (row.elims, row.outcome.clone()))
            .collect::<Vec<_>>()
    );
    let sessions = night.store.get_all_sessions().await.unwrap();
    assert_eq!(
        sessions
            .iter()
            .find(|s| s.session_id == busan)
            .map(|s| s.final_outcome.as_str()),
        Some("victory")
    );
}
