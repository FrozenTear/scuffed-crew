//! Stored `PersonalMatch::team_size` from a Tab through `handle_capture`.

use super::*;

/// A 5-a-side Tab and a 6-a-side Tab each store that size on the row.
#[tokio::test]
async fn handle_capture_stores_five_or_six_a_side() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    for size in [5_u8, 6] {
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let mut analysis = o8_analysis(None, Some("zenyatta"));
        analysis.team_size = usize::from(size);
        let mut board = o8_board(Some("zenyatta"));
        board.team_size = Some(usize::from(size));
        let session = format!("size-{size}");
        let report = o9_tab(&ctx, &session, "Busan", analysis, Some(board)).await;
        assert!(report.recorded, "size {size} was not stored");
        let rows = ctx.store.get_session_snapshots(&session).await.unwrap();
        assert_eq!(rows.len(), 1, "size {size}");
        assert_eq!(rows[0].team_size, Some(size), "size {size}");
    }
}

/// Role Queue 6v6 (#654), end to end on this PC: a 6-a-side (12-player)
/// Tab and a 5-a-side Tab, each with the bare `ROLE QUEUE` title and a
/// Tank rank screen. The 12-player game's rank is filed under role queue
/// 6v6, the 10-player game's under role queue 5v5, and the newest rank of
/// each queue is kept side by side: the 6v6 Tank rank, though newer, does
/// not replace the 5v5 Tank rank.
#[tokio::test]
async fn a_12_player_role_queue_rank_is_kept_apart_from_the_5v5_one() {
    use scuffed_types::api::{RankQueue, RankRole};
    use stat_tracker::games::model::Game;
    use stat_tracker::games::rank_slots::latest_rank_per_slot;
    use stat_tracker::model::{QueueTitle, RankText, RankUpdate};

    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let mut games = Vec::new();
    for (size, tier, want) in [
        (5_u8, "DIAMOND", RankQueue::Role5v5),
        (6, "GOLD", RankQueue::Role6v6),
    ] {
        // A fresh daemon per game, as in the test above.
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let mut analysis = o8_analysis(None, Some("zenyatta"));
        analysis.team_size = usize::from(size);
        let mut board = o8_board(Some("zenyatta"));
        board.team_size = Some(usize::from(size));
        let session = format!("rq-{size}");
        let report = o9_tab(&ctx, &session, "Busan", analysis, Some(board)).await;
        assert!(report.recorded, "size {size} was not stored");
        let title = QueueTitle {
            value: Some("ROLE QUEUE".into()),
            confidence: 0.93,
            ..QueueTitle::default()
        };
        assert!(
            ctx.store
                .attach_queue_title(&session, &title)
                .await
                .unwrap()
        );
        let rank = RankUpdate {
            role: Some(RankText {
                value: "Tank".into(),
                confidence: 0.9,
            }),
            role_rank: Some(RankText {
                value: tier.into(),
                confidence: 0.9,
            }),
            ..RankUpdate::default()
        };
        assert!(ctx.store.attach_rank_update(&session, &rank).await.unwrap());
        let rows = ctx.store.get_session_snapshots(&session).await.unwrap();
        let game = Game::from_match(rows.last().unwrap());
        let slot = game.rank_slot.expect("a Competitive rank slot");
        assert_eq!(slot.queue, want, "size {size}");
        assert_eq!(slot.role, Some(RankRole::Tank), "size {size}");
        games.push(game);
    }
    // The 6v6 game is newer than the 5v5 one.
    games[1].played_at = games[0].played_at + chrono::Duration::minutes(20);
    let latest = latest_rank_per_slot(&games);
    let tiers: Vec<_> = latest
        .iter()
        .map(|g| {
            let r = g.rank_update.as_ref().unwrap();
            (
                g.rank_slot.unwrap().queue,
                r.role_rank.as_ref().unwrap().value.clone(),
            )
        })
        .collect();
    assert_eq!(
        tiers,
        vec![
            (RankQueue::Role6v6, "GOLD".to_string()),
            (RankQueue::Role5v5, "DIAMOND".to_string()),
        ]
    );
}

fn role_queue_tank_gold(row: &mut storage::PersonalMatch) {
    use stat_tracker::model::{QueueTitle, RankText, RankUpdate};
    row.queue_title = Some(QueueTitle {
        value: Some("ROLE QUEUE".into()),
        confidence: 0.93,
        ..QueueTitle::default()
    });
    let text = |v: &str| RankText {
        value: v.into(),
        confidence: 0.9,
    };
    row.rank_update = Some(RankUpdate {
        role: Some(text("Tank")),
        role_rank: Some(text("GOLD")),
        ..RankUpdate::default()
    });
}

/// Q3: a held board stores no team size it did not read. On a game with
/// no other row it has none (its queue stays unknown, never a guessed
/// 5v5); after a Tab that read 6 a side it takes that read size through
/// the storage carry, not a size of its own.
#[tokio::test]
async fn a_held_board_stores_no_team_size_it_did_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let held = Counters {
        elims: 2,
        assists: 1,
        deaths: 0,
        damage: 400,
        healing: 80,
        mitigation: 900,
    };
    let at = Utc::now() - chrono::Duration::seconds(90);
    for (session, earlier) in [("held-alone", None), ("held-after-6", Some(6_u8))] {
        if let Some(size) = earlier {
            let mut tab = tracked_row(session, -120);
            tab.team_size = Some(size);
            store.insert_match(tab).await.unwrap();
        }
        store_held_board(
            &store,
            dir.path(),
            session,
            "Ana",
            "Busan",
            "unknown",
            held,
            at,
        )
        .await
        .unwrap();
        let rows = store.get_session_snapshots(session).await.unwrap();
        let held_row = rows.iter().find(|r| r.elims == 2).expect("the held row");
        assert_eq!(held_row.team_size, earlier, "{session}");
    }
}

/// Q2 on the upload side: of a game's unsynced Tabs (sizes 5, then 6, then
/// one with no size, which carries the 6), the newest is the one sent, so
/// the upload says `role_6v6`, the same as the local slot of that row.
#[tokio::test]
async fn the_newest_tab_decides_the_queue_that_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    for (i, size) in [Some(5_u8), Some(6), None].into_iter().enumerate() {
        let mut row = tracked_row("q2-sent", i as i64 * 60);
        row.elims = 10 + i as u32;
        row.team_size = size;
        role_queue_tank_gold(&mut row);
        store.insert_match(row).await.unwrap();
    }
    let rows = store.get_session_snapshots("q2-sent").await.unwrap();
    let sizes: Vec<_> = rows.iter().map(|r| r.team_size).collect();
    assert_eq!(sizes, [Some(5), Some(6), Some(6)], "the newest carries 6");
    let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = std::sync::Arc::clone(&sent);
    try_sync_with(&store, dir.path(), None, move |matches, _| {
        let seen = std::sync::Arc::clone(&seen);
        async move {
            seen.lock().unwrap().extend(matches);
            Ok(upload_ok())
        }
    })
    .await;
    let sent = sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "one row per game");
    assert_eq!(sent[0].elims, 12, "the newest Tab is sent");
    let body = serde_json::to_value(stat_tracker::sync::upload_request(&sent, &[])).unwrap();
    assert_eq!(body["matches"][0]["rank"]["queue"]["value"], "role_6v6");
    let newest = rows.iter().max_by_key(|r| r.elims).unwrap();
    let slot = stat_tracker::sync::rank_slot(newest).unwrap();
    assert_eq!(slot.queue, scuffed_types::api::RankQueue::Role6v6);
}
