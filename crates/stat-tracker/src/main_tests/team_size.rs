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
