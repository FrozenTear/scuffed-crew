//! Daemon tests: hero_authority. See `mod.rs`.

use super::*;

/// A confident hero on Tab 1, then a 5v5 Tab whose own-row read the
/// career panel contradicts (the authority abstains): the game keeps
/// Tab 1's hero and the second row flags `hero`.
#[tokio::test]
async fn a_disagreeing_5v5_tab_keeps_the_earlier_confident_hero() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (rows, session) = guard_game(vec![
        guard_tab(Some("zenyatta"), Some("zenyatta")),
        guard_tab(Some("baptiste"), None),
    ])
    .await;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.hero == "Zenyatta"), "{rows:?}");
    assert!(!rows[0].suspect_fields.iter().any(|f| f == "hero"));
    assert!(rows[1].suspect_fields.iter().any(|f| f == "hero"));
    assert_eq!(session.as_deref(), Some("Zenyatta"));
}

/// The first Tab's own-row hero is rejected (the authority abstains, the
/// panel names nothing to confirm it): that row stores Unknown with
/// `hero` flagged, never the rejected read. A later agreeing Tab fills
/// the game's hero in.
#[tokio::test]
async fn a_rejected_first_tab_stores_unknown_and_a_later_agreeing_tab_fills_the_hero() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    for first in [Some("baptiste"), None] {
        let (rows, session) = guard_game(vec![
            guard_tab(first, None),
            guard_tab(Some("zenyatta"), Some("zenyatta")),
        ])
        .await;
        assert_eq!(rows.len(), 2, "{first:?}");
        assert_eq!(rows[0].hero, "Unknown", "{first:?}");
        assert!(
            rows[0].suspect_fields.iter().any(|f| f == "hero"),
            "{first:?}"
        );
        assert_eq!(rows[1].hero, "Zenyatta", "{first:?}");
        assert!(
            !rows[1].suspect_fields.iter().any(|f| f == "hero"),
            "{first:?}"
        );
        assert_eq!(session.as_deref(), Some("Zenyatta"), "{first:?}");
    }
}

/// #289 with #288: a confident 5v5 hero on Tab 1, then the tracker
/// restarts and resumes the game from `active_game.json`, then a Tab
/// whose own-row read the career panel contradicts (the authority
/// abstains). The resumed game goes through the same authority merge as
/// a live one: it keeps Tab 1's hero and the new row flags `hero`.
#[tokio::test]
async fn a_resumed_5v5_game_keeps_the_earlier_confident_hero_when_the_next_tab_disagrees() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut now = test_now();
    // The local store is on disk and outlives the process, so one store
    // stands in for both runs. The in-memory session state is what a
    // restart loses: it is rebuilt from `active_game.json` below.
    let ctx = o8_ctx(dir.path(), true).await;
    {
        let mut active = game(detect::MatchOutcome::Unknown, None, now);
        active.session_id = "o8".into();
        active.map = Some("Busan".into());
        active.map_source = Some(boundary::MapSource::TopBar);
        active.session_created = false;
        let mut st = session(Some(active), now);
        let (analysis, board) = guard_tab(Some("zenyatta"), Some("zenyatta"));
        *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture { analysis, board });
        let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
        let report = handle_capture(&ctx, req).await.unwrap();
        assert!(report.recorded);
        apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, "o8", Ok(report), now).await;
        assert!(st.active_game.as_ref().unwrap().session_created);
        persist_active_game(dir.path(), st.active_game.as_ref());
    }

    // Restart: the session state comes back from disk only.
    let mut st = startup_session(dir.path());
    let resumed = st
        .active_game
        .as_ref()
        .expect("the same recent game resumes");
    assert_eq!(resumed.session_id, "o8");
    assert_eq!(
        resumed.hero_auth.accepted_hero.as_deref(),
        Some("Zenyatta"),
        "the confident hero survives the restart"
    );
    now = now.max(Instant::now()) + Duration::from_secs(20);
    let (analysis, board) = guard_tab(Some("baptiste"), None);
    *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture { analysis, board });
    let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
    let report = handle_capture(&ctx, req).await.unwrap();
    assert!(report.recorded);
    assert!(!report.split, "the resumed game must not split");
    apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, "o8", Ok(report), now).await;

    let rows = ctx.store.get_session_snapshots("o8").await.unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().all(|r| r.hero == "Zenyatta"), "{rows:?}");
    assert!(!rows[0].suspect_fields.iter().any(|f| f == "hero"));
    assert!(rows[1].suspect_fields.iter().any(|f| f == "hero"));
    let shown: Vec<String> = rows.iter().map(|m| m.display_hero().to_string()).collect();
    assert!(shown.iter().all(|h| h == "Zenyatta"), "{shown:?}");
}
