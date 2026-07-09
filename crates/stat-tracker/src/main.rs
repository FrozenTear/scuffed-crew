use stat_tracker::{capture, config, detect, ocr, parse, setup, storage, sync};

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use surrealdb_types::Datetime as SurrealDatetime;
use tracing_subscriber::EnvFilter;

const SYNC_EVERY_N_CAPTURES: u32 = 5;

/// Window for two word-OCR outcome reads to confirm each other. Sized to span
/// the accolade → rank-screen transition under a starved poller (measured 45s
/// between the last accolade tick and the first rank-screen tick) while still
/// bounding how long a single stray read stays actionable.
const OUTCOME_CONFIRM_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// How many poll-tick frames `--dump-poll-frames` keeps (ring buffer on disk).
/// At a 4s poll interval this is ~10 minutes — enough that a defeat's
/// post-match sequence survives even if the next game is already underway
/// before the frames are copied out. Frames are a few MB each.
const POLL_DUMP_KEEP: usize = 150;

/// How many rejected-capture frames are kept in `<data_dir>/debug/rejected/`.
/// A rejected capture records nothing, so the frame is the only evidence for
/// diagnosing why ("it didn't record my game" is undebuggable otherwise).
const REJECTED_KEEP: usize = 30;

struct PidGuard(std::path::PathBuf);

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("scuffed_stat_tracker=info,stat_tracker=info,surrealdb=warn")
        }))
        .init();

    let collect_portraits = std::env::args().any(|a| a == "--collect-portraits");
    let dump_poll_frames = std::env::args().any(|a| a == "--dump-poll-frames");

    if std::env::args().any(|a| a == "--generate-tessdata") {
        match setup::ensure_koverwatch_tessdata() {
            Ok(()) => {
                println!("koverwatch.traineddata generated successfully.");
                return Ok(());
            }
            Err(e) => {
                eprintln!("tessdata generation failed: {e}");
                std::process::exit(1);
            }
        }
    }

    if std::env::args().any(|a| a == "--list-outputs") {
        match capture::wayshot::list_outputs() {
            Ok(outputs) => {
                println!("Available outputs:");
                for (i, name) in outputs.iter().enumerate() {
                    println!("  [{i}] {name}");
                }
                println!("\nSet capture_output in config.toml to select one.");
            }
            Err(e) => eprintln!("Failed to list outputs: {e}"),
        }
        return Ok(());
    }

    let mut config = config::Config::load()?;
    tracing::info!("Scuffed Stat Tracker starting");
    tracing::info!(data_dir = %config.data_dir.display(), "using data directory");

    // If player_name is not set locally, try fetching it from the server.
    // This is the "first run via GUI" path: user set their name in the web UI
    // and launched the daemon with just a token — no manual config editing needed.
    if config.player_name.is_none()
        && let Some(sync_cfg) = &config.sync
    {
        let client = sync::SyncClient::new(sync_cfg.clone());
        match client.fetch_daemon_config().await {
            Ok(remote) if remote.player_name.is_some() => {
                tracing::info!(
                    player_name = %remote.player_name.as_deref().unwrap_or(""),
                    "player_name fetched from server"
                );
                config.player_name = remote.player_name;
            }
            Ok(_) => {
                tracing::info!(
                    "server has no player_name configured — set it in the web UI under My Stats → Settings"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not fetch daemon config from server (continuing without player_name)");
            }
        }
    }

    std::fs::create_dir_all(&config.data_dir)?;

    let pid_path = config.data_dir.join("daemon.pid");
    if let Ok(existing_pid) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = existing_pid.trim().parse::<u32>()
            && std::fs::metadata(format!("/proc/{pid}")).is_ok()
        {
            tracing::error!(pid, "another daemon is already running — stop it first");
            return Err(format!("another daemon is already running (PID {pid})").into());
        }
        let _ = std::fs::remove_file(&pid_path);
    }
    std::fs::write(&pid_path, std::process::id().to_string())?;

    let _pid_guard = PidGuard(pid_path);

    // Tessdata generation is triggered manually via --generate-tessdata or the GUI button.
    // Don't run it at daemon startup — it can take minutes and blocks Tab capture.

    let backend = capture::detect_backend().await;
    tracing::info!(?backend, "capture backend selected");

    if let Ok(outputs) = capture::wayshot::list_outputs() {
        let selected = config.capture_output.as_deref().unwrap_or(&outputs[0]);
        tracing::info!(
            available = ?outputs,
            selected = %selected,
            "wayland outputs"
        );
    }

    let store = storage::LocalStore::open(&config.data_dir).await?;
    let count = store.match_count().await?;
    tracing::info!(stored_matches = count, "local store ready");

    // Initial snapshot so the GUI has current data from the moment the daemon
    // takes the store lock (refreshed after every mutation from here on).
    if let Err(e) = store.export_snapshot(&config.data_dir).await {
        tracing::warn!(error = %e, "failed to write initial live snapshot");
    }

    let portraits_path = detect::hero_portrait::portraits_dir(&config.data_dir);
    let portrait_matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(
        &portraits_path,
    ));

    let data_dir = config.data_dir.clone();

    let sync_client = config
        .sync
        .as_ref()
        .map(|s| sync::SyncClient::new(s.clone()));

    if config.auto_detect.enabled {
        tracing::info!(
            poll_secs = config.auto_detect.poll_interval_secs,
            cooldown_secs = config.auto_detect.cooldown_secs,
            "auto-detect mode enabled — polling for match end screens"
        );
    }
    if config.game_process_names.is_empty() {
        tracing::info!("game-process gate disabled (game_process_names is empty)");
    } else {
        tracing::info!(
            processes = ?config.game_process_names,
            "captures gated on game process — set game_process_names in config.toml if yours differs"
        );
    }
    if dump_poll_frames {
        tracing::info!(
            dir = %config.data_dir.join("debug").join("poll").display(),
            "poll-frame dumping enabled (keeps the last {POLL_DUMP_KEEP} frames)"
        );
    }
    tracing::info!("daemon ready — press Tab in-game to capture scoreboard");

    if collect_portraits {
        tracing::info!(
            "portrait collection mode enabled — will save portrait references when OCR identifies heroes"
        );
    }

    run_loop(
        &backend,
        &store,
        sync_client.as_ref(),
        config.player_name.as_deref(),
        config.capture_output.as_deref(),
        &config.auto_detect,
        &config.game_process_names,
        &portrait_matcher,
        collect_portraits,
        dump_poll_frames,
        &data_dir,
    )
    .await
}

/// The game currently in progress. Opened when the poller sees a game-start
/// screen (map vote / hero select / ban) and reused for every Tab capture until
/// the next game starts — so captures taken across hero swaps all land in one
/// session. `outcome` is filled in when the poller reads the post-match screens
/// (or recovered from a captured frame), then back-filled onto the snapshots.
struct ActiveGame {
    session_id: String,
    outcome: detect::MatchOutcome,
    maps: Vec<String>,
    /// Whether the `match_session` row has been created (on the first capture).
    session_created: bool,
    /// When `outcome` was recorded. Drives the post-match grace window: Tab
    /// presses shortly after the outcome (the post-match scoreboard) still
    /// belong to this game; later ones belong to the next.
    outcome_recorded_at: Option<Instant>,
}

impl ActiveGame {
    fn finished(&self) -> bool {
        !matches!(self.outcome, detect::MatchOutcome::Unknown)
    }

    fn record_outcome(&mut self, outcome: detect::MatchOutcome) {
        self.outcome = outcome;
        self.outcome_recorded_at = Some(Instant::now());
    }
}

/// How long after a game's outcome is recorded that Tab captures still belong
/// to it. The post-match scoreboard is typically inspected right after the
/// result screens; without this window each such Tab opened a duplicate
/// session for the same match and double-counted it. Past the window, the
/// finished result must not leak onto the next match's captures.
const POST_MATCH_GRACE: std::time::Duration = std::time::Duration::from_secs(75);

/// Whether a Tab capture should open a fresh session instead of reusing the
/// active one.
fn should_start_fresh_session(game: Option<&ActiveGame>, now: Instant) -> bool {
    match game {
        // No game open — daemon started mid-match or the start screen was missed.
        None => true,
        // Mid-game capture.
        Some(g) if !g.finished() => false,
        // Finished: reuse within the grace window (post-match scoreboard of the
        // same match), start fresh after it. An unstamped outcome is treated as
        // stale so a finished result can never leak forward.
        Some(g) => g
            .outcome_recorded_at
            .is_none_or(|t| now.duration_since(t) > POST_MATCH_GRACE),
    }
}

/// How long an outcome seen with no game open stays applicable to the next
/// session that opens. Covers "daemon started during the post-match screens";
/// without the bound, an outcome from hours ago could stamp a future game.
const PENDING_OUTCOME_TTL: std::time::Duration = std::time::Duration::from_secs(90);

/// Take the pending outcome if it is still fresh; stale ones are discarded.
fn take_fresh_pending(
    pending: &mut Option<(detect::MatchOutcome, Instant)>,
    now: Instant,
) -> Option<detect::MatchOutcome> {
    let (outcome, seen_at) = pending.take()?;
    if now.duration_since(seen_at) <= PENDING_OUTCOME_TTL {
        Some(outcome)
    } else {
        tracing::debug!(?outcome, "discarding stale pending outcome");
        None
    }
}

/// What a Tab capture actually did, reported back to the session state machine.
struct CaptureReport {
    /// A snapshot row was written (and the session row created if this was the
    /// session's first capture). False = rejected by a trust gate.
    recorded: bool,
    /// Outcome stored on the snapshot — may have been recovered from the frame
    /// itself (banner colors / header text) when the game's outcome was still
    /// Unknown, in which case the caller back-fills it onto the session.
    outcome: detect::MatchOutcome,
    /// Map stored on the snapshot, if one was read — the caller adopts the
    /// first discovery onto the active game so the whole session shares it.
    map: Option<String>,
}

fn outcome_str(outcome: &detect::MatchOutcome) -> &'static str {
    match outcome {
        detect::MatchOutcome::Victory => "victory",
        detect::MatchOutcome::Defeat => "defeat",
        detect::MatchOutcome::Draw => "draw",
        detect::MatchOutcome::Unknown => "unknown",
    }
}

fn outcome_from_str(s: &str) -> detect::MatchOutcome {
    match s {
        "victory" => detect::MatchOutcome::Victory,
        "defeat" => detect::MatchOutcome::Defeat,
        "draw" => detect::MatchOutcome::Draw,
        _ => detect::MatchOutcome::Unknown,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    backend: &capture::CaptureBackend,
    store: &storage::LocalStore,
    sync_client: Option<&sync::SyncClient>,
    player_name: Option<&str>,
    capture_output: Option<&str>,
    auto_detect: &config::AutoDetectConfig,
    game_process_names: &[String],
    portrait_matcher: &Arc<detect::hero_portrait::PortraitMatcher>,
    collect_portraits: bool,
    dump_poll_frames: bool,
    data_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut game_gate = detect::game_running::GameProcessGate::new(game_process_names);
    // The GUI's Stop button (and systemd) send SIGTERM — shut down as
    // gracefully as Ctrl+C, with a final sync.
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut kbd = match detect::MultiKeyboardStream::open() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "evdev init failed — no keyboard detected");
            tracing::info!("press Ctrl+C to quit");
            tokio::signal::ctrl_c().await?;
            return Ok(());
        }
    };

    let mut capture_count: u32 = 0;
    let poll_interval = tokio::time::Duration::from_secs(auto_detect.poll_interval_secs);
    let new_game_debounce = std::time::Duration::from_secs(auto_detect.cooldown_secs);
    let mut last_game_open: Option<Instant> = None;
    let mut last_tab_capture: Option<Instant> = None;
    let tab_debounce = std::time::Duration::from_secs(3);

    // The game currently in progress — opened at the map-vote / hero-select
    // screen, reused for every capture until the next game starts.
    let mut active_game: Option<ActiveGame> = None;
    // Outcome detected by the poller while no game was open — applied to the
    // next session that opens, if still fresh (PENDING_OUTCOME_TTL).
    let mut pending_outcome: Option<(detect::MatchOutcome, Instant)> = None;
    // Last result-word OCR read, for confirmation: a word outcome is only
    // trusted once two reads agree within OUTCOME_CONFIRM_WINDOW, so a single
    // hallucinated OCR read can't finish the open game with a wrong outcome.
    // The reads may come from different screens (accolade → rank screen) and
    // need not be consecutive ticks: heavy Tab-capture OCR starves the poller
    // (a measured session had 45-70s tick gaps), so the accolade screen may get
    // only one tick. Garbage/transition frames between reads don't reset it.
    let mut word_outcome_streak: Option<(detect::MatchOutcome, Instant)> = None;

    // Periodic sync runs as a spawned task so a slow or hung server can't
    // stall Tab capture, polling, or shutdown. Single-flight: while one sync
    // is in the air, the next trigger is skipped (the following one picks up
    // whatever it missed). Shutdown paths still sync inline — bounded by the
    // client's HTTP timeout.
    let mut sync_task: Option<tokio::task::JoinHandle<()>> = None;

    let mut poll_timer = tokio::time::interval(poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // GUI command queue (manual outcome edits, session deletion) — checked on
    // its own timer so edits apply even while no game is running.
    let mut cmd_timer = tokio::time::interval(tokio::time::Duration::from_secs(3));
    cmd_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            result = kbd.wait_tab() => {
                match result {
                    Ok(()) => {
                        if !game_gate.is_running() {
                            tracing::debug!("Tab ignored — game process not running");
                            continue;
                        }

                        if let Some(last) = last_tab_capture
                            && last.elapsed() < tab_debounce {
                                tracing::debug!("Tab debounced — ignoring rapid press");
                                continue;
                            }

                        // Wait for the game to render the scoreboard overlay after Tab press
                        tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;
                        last_tab_capture = Some(Instant::now());

                        // Session choice: reuse the active game (mid-game, or
                        // post-match scoreboard within the grace window), or
                        // open a fresh one, inheriting a still-fresh outcome
                        // the poller saw before any game was open.
                        if should_start_fresh_session(active_game.as_ref(), Instant::now()) {
                            let inherited = take_fresh_pending(&mut pending_outcome, Instant::now());
                            active_game = Some(ActiveGame {
                                session_id: format!("{:016x}", rand_id()),
                                outcome_recorded_at: inherited.as_ref().map(|_| Instant::now()),
                                outcome: inherited.unwrap_or(detect::MatchOutcome::Unknown),
                                maps: Vec::new(),
                                session_created: false,
                            });
                            last_game_open = Some(Instant::now());
                        }

                        let (sid, create, outcome, maps) = {
                            let g = active_game.as_ref().expect("active_game set above");
                            (g.session_id.clone(), !g.session_created, g.outcome.clone(), g.maps.clone())
                        };

                        match handle_capture(backend, store, player_name, capture_output, &sid, create, outcome, &maps, portrait_matcher, collect_portraits, data_dir).await {
                            Err(e) => tracing::error!(error = %e, "capture cycle failed"),
                            // Rejected by a trust gate — nothing was recorded, so
                            // the session must not be marked as created.
                            Ok(report) if !report.recorded => {}
                            Ok(report) => {
                                if let Some(g) = active_game.as_mut() {
                                    g.session_created = true;
                                    // First map discovery propagates to the
                                    // whole session (one game, one map).
                                    if g.maps.is_empty()
                                        && let Some(map) = &report.map
                                    {
                                        g.maps.push(map.clone());
                                        if let Err(e) = store.set_session_map(&g.session_id, map).await {
                                            tracing::warn!(error = %e, "failed to set session map");
                                        }
                                    }
                                    // The capture recovered an outcome the game
                                    // didn't have yet (banner / header text on
                                    // the frame) — adopt it so the in-memory
                                    // state agrees with what was stored, and
                                    // back-fill earlier snapshots.
                                    if !g.finished()
                                        && !matches!(report.outcome, detect::MatchOutcome::Unknown)
                                    {
                                        g.record_outcome(report.outcome.clone());
                                        tracing::info!(
                                            outcome = ?report.outcome,
                                            session_id = %g.session_id,
                                            "outcome recovered from captured frame — back-filling session"
                                        );
                                        if let Err(e) = store.set_session_outcome(&g.session_id, outcome_str(&g.outcome)).await {
                                            tracing::warn!(error = %e, "failed to back-fill session outcome");
                                        }
                                    }
                                }
                                capture_count += 1;
                                if let Some(client) = sync_client
                                    && capture_count.is_multiple_of(SYNC_EVERY_N_CAPTURES)
                                    && !sync_task.as_ref().is_some_and(|t| !t.is_finished()) {
                                        let store = store.clone();
                                        let client = client.clone();
                                        let data_dir = data_dir.to_path_buf();
                                        sync_task = Some(tokio::spawn(async move {
                                            try_sync(&store, &client, &data_dir).await;
                                        }));
                                    }
                                refresh_snapshot(store, data_dir).await;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "keyboard devices lost — attempting to reopen");
                        match detect::MultiKeyboardStream::open() {
                            Ok(new_kbd) => {
                                kbd = new_kbd;
                                tracing::info!("keyboard monitoring reopened");
                            }
                            Err(e2) => {
                                tracing::error!(error = %e2, "failed to reopen keyboard — exiting");
                                if let Some(client) = sync_client {
                                    try_sync(store, client, data_dir).await;
                                }
                                return Ok(());
                            }
                        }
                    }
                }
            }
            _ = poll_timer.tick(), if auto_detect.enabled => {
                if !game_gate.is_running() {
                    word_outcome_streak = None;
                    continue;
                }

                match capture::capture_screen_output(backend, capture_output).await {
                    Ok(img) => {
                        let dump_dir = dump_poll_frames.then(|| data_dir.join("debug").join("poll"));
                        let (signal, phase, accolade_map) = tokio::task::spawn_blocking(move || {
                            if let Some(dir) = &dump_dir {
                                save_frame_ring(dir, "poll", &img, POLL_DUMP_KEEP);
                            }
                            let signal = detect::match_end::detect_outcome_signal(&img);
                            // The accolade screen also prints the map — read it
                            // while we're here; it recovers games where the
                            // in-game top-bar OCR missed all match.
                            let accolade_map = match &signal {
                                Some((_, detect::match_end::OutcomeSource::ResultWord)) => {
                                    detect::match_end::read_accolade_map(&img)
                                }
                                _ => None,
                            };
                            let phase = detect::match_start::detect_phase(&img);
                            (signal, phase, accolade_map)
                        }).await.unwrap_or((None, detect::GamePhase::Unknown, None));

                        // The banner color-flood is specific enough to act on
                        // immediately (and only lasts ~3s — a second tick may
                        // never come). A word-OCR outcome (accolade or rank
                        // screen) needs a second agreeing read within the
                        // confirmation window.
                        let confirmed = match signal {
                            Some((outcome, detect::match_end::OutcomeSource::Banner)) => {
                                Some(outcome)
                            }
                            Some((outcome, source)) => {
                                let agreed = word_outcome_streak
                                    .as_ref()
                                    .is_some_and(|(prev, t)| *prev == outcome && t.elapsed() <= OUTCOME_CONFIRM_WINDOW);
                                word_outcome_streak = Some((outcome.clone(), Instant::now()));
                                if agreed {
                                    Some(outcome)
                                } else {
                                    tracing::debug!(?outcome, ?source, "result word read — awaiting agreeing read");
                                    None
                                }
                            }
                            // No signal this tick (transition/garbage frame) —
                            // keep the streak; the window bounds its lifetime.
                            None => None,
                        };

                        // Post-match accolade screen → record the outcome on the
                        // open game. Idempotent: the screen shows for ~20s (several
                        // ticks) but only the first, while the outcome is still
                        // Unknown, writes it.
                        if let Some(outcome) = confirmed {
                            match active_game.as_mut() {
                                Some(g) if !g.finished() => {
                                    g.record_outcome(outcome.clone());
                                    tracing::info!(?outcome, session_id = %g.session_id, "auto-detect: outcome confirmed from post-match screens");
                                    if g.session_created {
                                        if let Err(e) = store.set_session_outcome(&g.session_id, outcome_str(&outcome)).await {
                                            tracing::warn!(error = %e, "failed to back-fill session outcome");
                                        }
                                        refresh_snapshot(store, data_dir).await;
                                    }
                                }
                                Some(_) => { /* outcome already recorded for this game */ }
                                None => {
                                    // No game open yet — applies to the next
                                    // session if one opens within the TTL.
                                    pending_outcome = Some((outcome, Instant::now()));
                                }
                            }
                        }

                        if let Some(map) = accolade_map
                            && let Some(g) = active_game.as_mut()
                            && g.maps.is_empty()
                        {
                            tracing::info!(map = %map, session_id = %g.session_id, "map recovered from accolade screen");
                            g.maps.push(map.clone());
                            if g.session_created {
                                if let Err(e) = store.set_session_map(&g.session_id, &map).await {
                                    tracing::warn!(error = %e, "failed to set session map");
                                }
                                refresh_snapshot(store, data_dir).await;
                            }
                        }

                        // Game-start screens open a new game. Map vote is the
                        // unambiguous boundary (debounced so a lingering screen
                        // across ticks opens only one game); hero select/ban only
                        // open a game when none is active (fallback if the map
                        // vote was missed).
                        // A game whose outcome has already been recorded is
                        // finished; the next start screen must begin a fresh game
                        // so the previous result can't carry over to it.
                        let game_finished = active_game.as_ref().is_some_and(ActiveGame::finished);
                        match phase {
                            detect::GamePhase::MapVote { maps } => {
                                let can_open = active_game.is_none()
                                    || game_finished
                                    || last_game_open.is_none_or(|t| t.elapsed() >= new_game_debounce);
                                if can_open {
                                    let sid = format!("{:016x}", rand_id());
                                    tracing::info!(?maps, session_id = %sid, "auto-detect: map vote — new game");
                                    active_game = Some(ActiveGame { session_id: sid, outcome: detect::MatchOutcome::Unknown, maps, session_created: false, outcome_recorded_at: None });
                                    last_game_open = Some(Instant::now());
                                    // Evidence about the previous match must not
                                    // confirm into this one.
                                    word_outcome_streak = None;
                                }
                            }
                            detect::GamePhase::HeroBan | detect::GamePhase::HeroSelect
                                if active_game.is_none() || game_finished =>
                            {
                                let sid = format!("{:016x}", rand_id());
                                tracing::info!(session_id = %sid, "auto-detect: hero select/ban — new game (map vote missed)");
                                active_game = Some(ActiveGame { session_id: sid, outcome: detect::MatchOutcome::Unknown, maps: Vec::new(), session_created: false, outcome_recorded_at: None });
                                last_game_open = Some(Instant::now());
                                word_outcome_streak = None;
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        tracing::trace!(error = %e, "poll capture failed (game may not be running)");
                    }
                }
            }
            _ = cmd_timer.tick() => {
                let cmds = storage::take_commands(data_dir);
                if !cmds.is_empty() {
                    for cmd in &cmds {
                        tracing::info!(?cmd, "applying GUI command");
                        // Keep the in-memory game consistent when the command
                        // targets the active session, so the poller can't
                        // overwrite a manual edit or resurrect a deleted game.
                        match cmd {
                            storage::StoreCommand::SetOutcome { session_id, outcome } => {
                                if let Some(g) = active_game.as_mut()
                                    && g.session_id == *session_id {
                                        g.record_outcome(outcome_from_str(outcome));
                                    }
                            }
                            storage::StoreCommand::DeleteSession { session_id } => {
                                if active_game.as_ref().is_some_and(|g| g.session_id == *session_id) {
                                    active_game = None;
                                }
                            }
                        }
                        if let Err(e) = store.apply_command(cmd).await {
                            tracing::warn!(error = %e, "GUI command failed");
                        }
                    }
                    refresh_snapshot(store, data_dir).await;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                if let Some(client) = sync_client {
                    try_sync(store, client, data_dir).await;
                }
                return Ok(());
            }
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM received — shutting down");
                if let Some(client) = sync_client {
                    try_sync(store, client, data_dir).await;
                }
                return Ok(());
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_capture(
    backend: &capture::CaptureBackend,
    store: &storage::LocalStore,
    player_name: Option<&str>,
    capture_output: Option<&str>,
    session_id: &str,
    create_session: bool,
    game_outcome: detect::MatchOutcome,
    detected_maps: &[String],
    portrait_matcher: &Arc<detect::hero_portrait::PortraitMatcher>,
    collect_portraits: bool,
    data_dir: &std::path::Path,
) -> Result<CaptureReport, Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Tab detected — capturing screen (hold Tab to keep scoreboard visible)");
    let img = capture::capture_screen_output(backend, capture_output).await?;

    let matcher = Arc::clone(portrait_matcher);
    // Clone player_name so the blocking closure can own it.
    let player_name_owned = player_name.map(|s| s.to_string());
    let (
        outcome,
        ocr_result,
        rows,
        portrait_hero,
        career_hero,
        map_from_panel,
        scoreboard_img,
        player_row_idx,
        frame_img,
    ) = tokio::task::spawn_blocking(move || {
        // Outcome: prefer the open game's result (read off the accolade
        // screen by the poller); else color-flood detection; else read the
        // VICTORY/DEFEAT header text off this frame. The last step recovers
        // the case where the poller missed the screens and we're sitting on
        // a post-match scoreboard that prints the result header.
        let outcome = if matches!(game_outcome, detect::MatchOutcome::Unknown) {
            let o = detect::match_end::detect_outcome(&img);
            if matches!(o, detect::MatchOutcome::Unknown) {
                detect::match_end::detect_outcome_text(&img)
            } else {
                o
            }
        } else {
            game_outcome
        };

        let scoreboard = ocr::preprocess::crop_scoreboard(&img);
        let team_size = detect::hero_portrait::detect_team_size(&scoreboard);
        let player_match = matcher.match_player_hero(&scoreboard);
        let portrait_match = player_match
            .as_ref()
            .map(|(name, conf, _)| (name.clone(), *conf));
        let brightness_row_idx = player_match.map(|(_, _, idx)| idx);

        let rows = ocr::recognize_scoreboard_cells_with_team_size(&img, Some(team_size));
        let ocr = ocr::recognize(&img);

        // Player row: if a player name is configured, scan ALL rows (both teams)
        // for a name match — this handles replays and post-match screens where the
        // player may be on team 2. Fall back to brightness-detected row otherwise.
        let row_idx = player_name_owned
            .as_deref()
            .and_then(|name| parse::find_player_row_by_name(&rows, name))
            .or(brightness_row_idx);

        // Career-panel hero title. Guard against garbage OCR (happens when there
        // is no career panel — replay, post-match — by requiring the result to
        // actually match a known hero name, which match_hero_in_text already does).
        let career_hero = ocr::recognize_region(&ocr::preprocess::crop_career_hero(&img))
            .ok()
            .and_then(|t| parse::match_hero_in_text(&t));
        let map_from_panel = ocr::recognize_region(&ocr::preprocess::crop_map_name(&img))
            .ok()
            .and_then(|t| parse::match_map_in_text(&t));

        (
            outcome,
            ocr,
            rows,
            portrait_match,
            career_hero,
            map_from_panel,
            scoreboard,
            row_idx,
            img,
        )
    })
    .await?;
    let ocr_result = ocr_result?;

    tracing::info!(?outcome, "frame analysis");
    let preview_end = ocr_result
        .raw_text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= 120)
        .last()
        .unwrap_or(0);
    tracing::info!(
        confidence = ocr_result.confidence,
        text_preview = &ocr_result.raw_text[..preview_end],
        "OCR result"
    );

    let player_row_conf = player_row_idx
        .and_then(|i| rows.get(i))
        .map(|r| r.mean_confidence);
    tracing::info!(
        ?player_row_idx,
        player_row_conf,
        rows = rows.len(),
        text_confidence = ocr_result.confidence,
        "scoreboard cell OCR complete"
    );

    // Trust gate: don't parse frames that don't look like a scoreboard (menus,
    // replay browser, desktop). Better to record nothing than to scrape stats
    // out of a random screen.
    if !parse::looks_like_scoreboard(&rows) {
        tracing::warn!(
            rows = rows.len(),
            "capture rejected — frame does not look like a scoreboard (saved to debug/rejected)"
        );
        save_rejected_frame(data_dir, frame_img, "noscoreboard");
        return Ok(CaptureReport {
            recorded: false,
            outcome,
            map: None,
        });
    }

    let outcome_label = outcome_str(&outcome);

    if let Some(mut parsed) = parse::parse_scoreboard_cells(
        &rows,
        player_row_idx,
        &ocr_result.raw_text,
        outcome_label,
        player_name,
    ) {
        // Hero priority: career-panel title (plain text, most reliable) >
        // portrait template match > scoreboard-text guess already in `parsed`.
        if let Some(hero_name) = &career_hero {
            tracing::info!(hero = %hero_name, source = "career_panel", "hero identified via career-panel title");
            parsed.hero = hero_name.clone();
            parsed.role = parse::guess_role_public(&parsed.hero);
        } else if let Some((hero_name, confidence)) = &portrait_hero {
            tracing::info!(
                hero = %hero_name,
                confidence = confidence,
                source = "portrait",
                "hero identified via portrait template matching"
            );
            // Portrait references are keyed by file stem ("wrecking_ball") —
            // canonicalize so they count together with career-panel reads.
            parsed.hero = parse::canonical_hero(hero_name);
            parsed.role = parse::guess_role_public(&parsed.hero);
        } else {
            tracing::info!(
                hero = %parsed.hero,
                source = "ocr_text",
                "hero identified via OCR text (career panel + portrait missed)"
            );
        }

        // Auto-collect portrait reference when hero is identified and collection is enabled
        if collect_portraits && parsed.hero != "Unknown" {
            let portraits_path = detect::hero_portrait::portraits_dir(data_dir);
            let (sw, sh) = (scoreboard_img.width(), scoreboard_img.height());
            let portrait_w = sw * 6 / 100;
            let portrait_x = sw / 100;
            let row_height = sh * 7 / 100;
            let start_y = sh * 12 / 100;
            let row_y = start_y + player_row_idx.unwrap_or(0) as u32 * row_height;
            if portrait_x + portrait_w <= sw && row_y + portrait_w <= sh {
                let crop = scoreboard_img.crop_imm(portrait_x, row_y, portrait_w, portrait_w);
                if let Err(e) = detect::hero_portrait::save_portrait_reference(
                    &portraits_path,
                    &parsed.hero,
                    &crop,
                ) {
                    tracing::debug!(error = %e, "portrait save failed (non-fatal)");
                }
            }
        }

        let now = SurrealDatetime::from(Utc::now());

        // Map priority: auto-detected map-vote > top-bar label OCR > whatever
        // the scoreboard-text fuzzy match found (least reliable, kept last).
        if !detected_maps.is_empty() {
            parsed.map_name = detected_maps[0].clone();
        } else if let Some(map) = &map_from_panel {
            parsed.map_name = map.clone();
        }

        // The session is owned by the active game (map-vote → accolade). The
        // first capture creates the session row; later captures (including hero
        // swaps and the post-match scoreboard) append to the same session.
        parsed.session_id = session_id.to_string();
        if create_session {
            let session = storage::MatchSession {
                session_id: session_id.to_string(),
                hero: parsed.hero.clone(),
                map_name: parsed.map_name.clone(),
                role: parsed.role.clone(),
                started_at: now,
                last_capture_at: now,
                capture_count: 1,
                final_outcome: outcome_label.to_string(),
            };
            if let Err(e) = store.create_session(&session).await {
                tracing::warn!(error = %e, "failed to create session");
            }
            tracing::info!(session_id = %session_id, "started new match session");
        } else if let Err(e) = store.append_capture(session_id, now, outcome_label).await {
            tracing::warn!(error = %e, "failed to append capture to session");
        }

        tracing::info!(
            hero = %parsed.hero,
            map = %parsed.map_name,
            elims = parsed.elims,
            deaths = parsed.deaths,
            "parsed scoreboard"
        );
        let recorded_map = (!parsed.map_name.is_empty()).then(|| parsed.map_name.clone());
        storage::append_match_log(data_dir, &parsed);
        store.insert_match(parsed).await.map_err(
            |e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("store insert failed: {e}").into()
            },
        )?;

        // Keep the session label on the majority hero across its snapshots —
        // a single capture can mislabel (career panel shows the spectated hero
        // while dead; portrait matching can misfire), and the label otherwise
        // froze on whatever the first capture read.
        if !create_session
            && let Ok(snaps) = store.get_session_snapshots(session_id).await
            && let Some(hero) = storage::majority_hero(&snaps)
        {
            let role = parse::guess_role_public(&hero);
            if let Err(e) = store.set_session_hero(session_id, &hero, &role).await {
                tracing::debug!(error = %e, "failed to refresh session hero");
            }
        }
        Ok(CaptureReport {
            recorded: true,
            outcome,
            map: recorded_map,
        })
    } else {
        // Scoreboard-shaped frame, but the player's row couldn't be positively
        // identified (no name match, no highlighted row) — recording another
        // row's stats would be worse than recording nothing.
        tracing::warn!(
            "capture rejected — player row not identified (saved to debug/rejected; \
             set player_name in config.toml if it is missing)"
        );
        save_rejected_frame(data_dir, frame_img, "noplayerrow");
        Ok(CaptureReport {
            recorded: false,
            outcome,
            map: None,
        })
    }
}

/// Save a debug frame into `dir` as `<prefix>_<timestamp>.png`, keeping at
/// most `keep` PNGs in the directory (oldest by mtime evicted). Each ring gets
/// a dedicated directory (`debug/poll`, `debug/rejected`), so every PNG there
/// participates in the same ring regardless of prefix.
fn save_frame_ring(dir: &std::path::Path, prefix: &str, img: &image::DynamicImage, keep: usize) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let name = format!(
        "{prefix}_{}.png",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    if let Err(e) = img.save(dir.join(&name)) {
        tracing::debug!(error = %e, "failed to save debug frame");
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut frames: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("png"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        if frames.len() > keep {
            frames.sort_by_key(|(t, _)| *t);
            for (_, old) in &frames[..frames.len() - keep] {
                let _ = std::fs::remove_file(old);
            }
        }
    }
}

/// Archive a frame whose capture was rejected by a trust gate, for diagnosis.
/// Runs the PNG encode off the async runtime; fire-and-forget.
fn save_rejected_frame(data_dir: &std::path::Path, img: image::DynamicImage, reason: &'static str) {
    let dir = data_dir.join("debug").join("rejected");
    tokio::task::spawn_blocking(move || {
        save_frame_ring(&dir, &format!("rejected_{reason}"), &img, REJECTED_KEEP);
    });
}

fn rand_id() -> u64 {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    seed ^ (std::process::id() as u64).wrapping_mul(0x517cc1b727220a95)
}

/// Refresh the GUI's live snapshot after a store mutation. Failures are
/// logged but never block the capture/poll path.
async fn refresh_snapshot(store: &storage::LocalStore, data_dir: &std::path::Path) {
    if let Err(e) = store.export_snapshot(data_dir).await {
        tracing::debug!(error = %e, "live snapshot refresh failed");
    }
}

async fn try_sync(
    store: &storage::LocalStore,
    client: &sync::SyncClient,
    data_dir: &std::path::Path,
) {
    // Errors are stringified immediately: `Box<dyn Error>` isn't `Send`, and
    // this future runs on a spawned task.
    let unsynced = match store.get_unsynced().await.map_err(|e| e.to_string()) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "failed to query unsynced matches");
            return;
        }
    };
    if unsynced.is_empty() {
        return;
    }

    // The server keeps one row per session, so only the final snapshot of
    // each session is worth sending — but every fetched row is marked synced
    // on success (the collapsed ones are represented by the snapshot that
    // was uploaded).
    let ids: Vec<_> = unsynced.iter().filter_map(|m| m.id.clone()).collect();
    let mut newest_first = unsynced;
    newest_first.reverse(); // get_unsynced is played_at ASC
    let to_upload = storage::latest_per_game(newest_first);
    tracing::info!(
        rows = ids.len(),
        games = to_upload.len(),
        "syncing unsynced matches"
    );
    match client.upload_matches(&to_upload).await.map_err(|e| e.to_string()) {
        Ok(resp) => {
            tracing::info!(
                inserted = resp.inserted,
                skipped = resp.skipped,
                "sync complete"
            );
            if let Err(e) = store.mark_synced(ids).await.map_err(|e| e.to_string()) {
                tracing::error!(error = %e, "failed to mark matches as synced");
            }
            refresh_snapshot(store, data_dir).await;
        }
        Err(e) => tracing::error!(error = %e, "sync upload failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Tests construct `now` in the future so subtracting ages can't underflow
    // the monotonic clock on a freshly-booted machine.
    fn test_now() -> Instant {
        Instant::now() + Duration::from_secs(600)
    }

    fn game(
        outcome: detect::MatchOutcome,
        recorded_secs_ago: Option<u64>,
        now: Instant,
    ) -> ActiveGame {
        ActiveGame {
            session_id: "test".into(),
            outcome,
            maps: Vec::new(),
            session_created: true,
            outcome_recorded_at: recorded_secs_ago.map(|s| now - Duration::from_secs(s)),
        }
    }

    #[test]
    fn tab_with_no_game_starts_fresh() {
        assert!(should_start_fresh_session(None, test_now()));
    }

    #[test]
    fn tab_mid_game_reuses_session() {
        let now = test_now();
        let g = game(detect::MatchOutcome::Unknown, None, now);
        assert!(!should_start_fresh_session(Some(&g), now));
    }

    #[test]
    fn tab_on_post_match_scoreboard_reuses_finished_session() {
        // Within the grace window the Tab capture is the post-match scoreboard
        // of the SAME match — a fresh session would double-count the game.
        let now = test_now();
        let g = game(detect::MatchOutcome::Defeat, Some(30), now);
        assert!(!should_start_fresh_session(Some(&g), now));
    }

    #[test]
    fn tab_long_after_finish_starts_fresh() {
        let now = test_now();
        let g = game(detect::MatchOutcome::Victory, Some(120), now);
        assert!(should_start_fresh_session(Some(&g), now));
        // An unstamped outcome is treated as stale: a finished result must
        // never leak onto the next match's captures.
        let g = game(detect::MatchOutcome::Victory, None, now);
        assert!(should_start_fresh_session(Some(&g), now));
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
}
