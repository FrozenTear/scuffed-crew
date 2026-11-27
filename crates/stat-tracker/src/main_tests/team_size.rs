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
