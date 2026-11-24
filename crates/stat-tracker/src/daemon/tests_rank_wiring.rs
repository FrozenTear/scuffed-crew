//! Source seams of the rank update wiring: the caller builds the feed
//! from config `rank_reader` (env override inside), hands it to the poll
//! loop and settles it after the loop. A literal in either place would
//! pass every behavior test, so the seams are pinned here.

#[test]
fn the_caller_builds_the_feed_and_settles_it_after_the_loop() {
    let lifecycle: String = include_str!("lifecycle.rs").split_whitespace().collect();
    let at = |needle: &str| {
        assert_eq!(lifecycle.matches(needle).count(), 1, "{needle} once");
        lifecycle.find(needle).expect("counted above")
    };
    let made =
        at("letmutrank_feed=daemon::rank::RankFeed::for_config(config.rank_reader,&ctx.data_dir);");
    let looped = at("letresult=run_loop(");
    let handed = at("&mutpregame_dump,&mutrank_feed,)");
    let settle = at("finish_or_abort_sync(");
    let flush = at("daemon::rank::finish_on_shutdown(feed,&ctx.store,pregame_dump.as_mut())");
    let pregame = at("dump.finish(within).await");
    let close = lifecycle
        .rfind("close_store_before_exit(ctx).await")
        .expect("store close");
    // built before the loop and handed to it; after the loop, settled and
    // stored before the pre-game flush takes the saver, and before the
    // store closes
    assert!(made < looped && looped < handed && handed < settle);
    assert!(settle < flush && flush < pregame && pregame < close);
    // bounded by the shutdown deadline
    assert!(lifecycle[flush..pregame].contains("tokio::time::timeout(within,flush)"));
    // a stored rank marks the snapshot dirty and then forces the write: the
    // loop's own forced shutdown write has run already, and the marking
    // alone writes only with a GUI running and the debounce passed
    let wrote = at("flush_snapshot_if_dirty(&ctx.store,&ctx.data_dir).await;");
    let marked = lifecycle[flush..]
        .find("refresh_snapshot(&ctx.store,&ctx.data_dir).await;")
        .map(|i| flush + i)
        .expect("the snapshot is marked after the rank is stored");
    assert!(flush < marked && marked < wrote && wrote < pregame);
}

/// Config `rank_reader` reaches both readers at start: the queue title feed
/// and the rank feed `run` builds and hands to `run_loop`. A literal in
/// either place would pass every behavior test, so the seam is pinned here.
#[test]
fn the_daemon_hands_config_rank_reader_to_both_readers() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let run = norm(crate::main_tests::source_text::fn_source("run"));
    let at = |needle: &str| {
        run.find(needle)
            .unwrap_or_else(|| panic!("run must contain {needle:?}"))
    };
    let queue = at(
        "let queue_title = daemon::queue_title::QueueTitleFeed::for_config(config.rank_reader, \
         &ctx.data_dir);",
    );
    let readers =
        at("PregameDump::with_readers( ctx.save_pregame_screens, bans_picks, queue_title,");
    let rank = at(
        "let mut rank_feed = daemon::rank::RankFeed::for_config(config.rank_reader, \
         &ctx.data_dir);",
    );
    let call = at(
        "let result = run_loop( Arc::clone(&ctx), early_sigterm, &mut sync_task, &mut \
         restart_to, &mut pregame_dump, &mut rank_feed, ) .await;",
    );
    assert!(queue < readers && readers < rank && rank < call);
    assert_eq!(run.matches("QueueTitleFeed::for_config(").count(), 1);
    assert_eq!(run.matches("RankFeed::for_config(").count(), 1);
    assert!(!run.contains("RankFeed::from_env("));
    assert_eq!(run.matches("run_loop(").count(), 1);
    // `run_loop` takes that feed as its last parameter and builds none of
    // its own, so no other value can stand in for the config switch
    let run_loop = norm(crate::main_tests::source_text::fn_source("run_loop"));
    assert!(
        run_loop
            .contains("rank_feed: &mut Option<daemon::rank::RankFeed>, ) -> anyhow::Result<()> {"),
        "rank_feed is run_loop's last parameter"
    );
    assert!(!run_loop.contains("rank_reader: bool"));
    assert!(!run_loop.contains("RankFeed::for_config("));
    assert!(!run_loop.contains("RankFeed::from_env("));
}
