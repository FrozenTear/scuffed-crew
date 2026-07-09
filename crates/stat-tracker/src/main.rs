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

    // Single wiring point for OCR debug dumps — the lib reads this switch
    // instead of re-loading config on first use.
    ocr::set_debug_ocr(config.debug_ocr_enabled());

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
        // `.first()`, not `[0]`: zero outputs (headless / compositor hiccup)
        // must not panic the daemon at startup.
        let selected = config
            .capture_output
            .as_deref()
            .or_else(|| outputs.first().map(String::as_str))
            .unwrap_or("<none>");
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
    /// The map actually being played, once a trusted read confirms it
    /// (top-bar label, accolade screen). Never set from the map vote.
    map: Option<String>,
    /// Canonicalized names seen on the map-vote screen. The winner is
    /// unknowable at vote time, so these are CANDIDATES only — they constrain
    /// later OCR reads (a read that isn't one of them is a misread) but are
    /// never stored as the played map themselves.
    map_candidates: Vec<String>,
    /// Whether the `match_session` row has been created (on the first capture).
    session_created: bool,
    /// When `outcome` was recorded. Drives the post-match grace window: Tab
    /// presses shortly after the outcome (the post-match scoreboard) still
    /// belong to this game; later ones belong to the next.
    outcome_recorded_at: Option<Instant>,
    /// When the game was opened (start screen seen or first Tab).
    opened_at: Instant,
    /// Last recorded evidence the game is still this game (capture stored,
    /// outcome/map recorded). Bounds how long an unfinished session can
    /// absorb captures — see [`UNFINISHED_SESSION_IDLE`].
    last_activity: Instant,
}

impl ActiveGame {
    fn open_now(
        session_id: String,
        outcome: detect::MatchOutcome,
        map_candidates: Vec<String>,
    ) -> Self {
        let now = Instant::now();
        ActiveGame {
            session_id,
            outcome_recorded_at: (outcome != detect::MatchOutcome::Unknown).then_some(now),
            outcome,
            map: None,
            map_candidates,
            session_created: false,
            opened_at: now,
            last_activity: now,
        }
    }

    fn finished(&self) -> bool {
        !matches!(self.outcome, detect::MatchOutcome::Unknown)
    }

    fn record_outcome(&mut self, outcome: detect::MatchOutcome) {
        self.outcome = outcome;
        self.outcome_recorded_at = Some(Instant::now());
        self.last_activity = Instant::now();
    }

    fn touch(&mut self) {
        self.last_activity = Instant::now();
    }
}

/// On-disk mirror of [`ActiveGame`] (wall-clock timestamps instead of
/// `Instant`s). The session state machine is otherwise memory-only, and daemon
/// restarts are routine — without this, a restart mid-game either split the
/// game (new session per Tab) or merged it into whatever came next.
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedGame {
    session_id: String,
    outcome: detect::MatchOutcome,
    #[serde(default)]
    map: Option<String>,
    #[serde(default)]
    map_candidates: Vec<String>,
    session_created: bool,
    opened_at: chrono::DateTime<Utc>,
    last_activity: chrono::DateTime<Utc>,
    outcome_recorded_at: Option<chrono::DateTime<Utc>>,
}

fn active_game_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("active_game.json")
}

/// Persist (or clear) the open-game skeleton. Fire-and-forget: losing this
/// file only degrades restart recovery, never the capture itself.
fn persist_active_game(data_dir: &std::path::Path, game: Option<&ActiveGame>) {
    let path = active_game_path(data_dir);
    let Some(g) = game else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    let now_i = Instant::now();
    let now_w = Utc::now();
    let to_wall = |i: Instant| {
        now_w - chrono::Duration::from_std(now_i.duration_since(i)).unwrap_or_default()
    };
    let persisted = PersistedGame {
        session_id: g.session_id.clone(),
        outcome: g.outcome,
        map: g.map.clone(),
        map_candidates: g.map_candidates.clone(),
        session_created: g.session_created,
        opened_at: to_wall(g.opened_at),
        last_activity: to_wall(g.last_activity),
        outcome_recorded_at: g.outcome_recorded_at.map(to_wall),
    };
    let write = || -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&persisted)?)?;
        std::fs::rename(&tmp, &path)
    };
    if let Err(e) = write() {
        tracing::debug!(error = %e, "failed to persist active game");
    }
}

/// Recover the open game from a previous daemon run, if it is still plausibly
/// the current game (last activity within [`UNFINISHED_SESSION_IDLE`]).
/// Timestamps that predate the current boot (recovery across a reboot) are
/// treated as stale rather than clamped.
fn recover_active_game(data_dir: &std::path::Path) -> Option<ActiveGame> {
    let bytes = std::fs::read(active_game_path(data_dir)).ok()?;
    let p: PersistedGame = serde_json::from_slice(&bytes).ok()?;
    let to_instant =
        |w: chrono::DateTime<Utc>| Instant::now().checked_sub((Utc::now() - w).to_std().ok()?);
    let last_activity = to_instant(p.last_activity)?;
    if last_activity.elapsed() > UNFINISHED_SESSION_IDLE {
        return None;
    }
    Some(ActiveGame {
        session_id: p.session_id,
        outcome: p.outcome,
        map: p.map,
        map_candidates: p.map_candidates,
        session_created: p.session_created,
        opened_at: to_instant(p.opened_at)?,
        last_activity,
        // An unrecoverable timestamp behaves as "unstamped", which the grace
        // logic already treats as stale — the outcome can't leak forward.
        outcome_recorded_at: p.outcome_recorded_at.and_then(to_instant),
    })
}

/// How long after a game's outcome is recorded that Tab captures still belong
/// to it. The post-match scoreboard is typically inspected right after the
/// result screens; without this window each such Tab opened a duplicate
/// session for the same match and double-counted it. Past the window, the
/// finished result must not leak onto the next match's captures.
const POST_MATCH_GRACE: std::time::Duration = std::time::Duration::from_secs(75);

/// How long an unfinished session stays reusable with no recorded activity.
/// If the poller misses the outcome AND the next game's start screens (likely
/// under Tab starvation, or with auto-detect off), an unbounded session would
/// absorb every capture that follows — yesterday's unfinished game swallowing
/// today's first Tab. Sized comfortably above a long match plus queue time.
const UNFINISHED_SESSION_IDLE: std::time::Duration = std::time::Duration::from_secs(20 * 60);

/// Minimum age of a poller-opened game before a banner-color outcome read off
/// a Tab frame is trusted. The color-flood detector can false-positive on
/// heavy mid-fight red vignettes; a real banner can't appear this early into
/// a match. The result-header text path stays available regardless, and a
/// session freshly opened by the Tab itself (daemon joined mid/post match) is
/// exempt — there the banner is exactly the evidence being recovered.
const MIN_BANNER_SESSION_AGE: std::time::Duration = std::time::Duration::from_secs(300);

/// Whether a Tab capture should open a fresh session instead of reusing the
/// active one.
fn should_start_fresh_session(game: Option<&ActiveGame>, now: Instant) -> bool {
    match game {
        // No game open — daemon started mid-match or the start screen was missed.
        None => true,
        // Mid-game capture — unless the session has been idle so long it
        // can't plausibly be the same game.
        Some(g) if !g.finished() => {
            now.duration_since(g.last_activity) > UNFINISHED_SESSION_IDLE
        }
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

/// The map to store on a capture snapshot.
///
/// Priority: the session's confirmed map > top-bar label OCR > the fuzzy
/// scoreboard-text read. Map-vote names never appear here — the vote winner
/// is unknowable at vote time (recording candidates as the played map was
/// wrong ~2/3 of the time with 2+ candidates) — but when candidates are known
/// they veto OCR reads that aren't among them: the played map must be one of
/// the voted maps, so a read outside the set is a misread.
fn resolve_map(
    session_map: Option<&str>,
    panel_read: Option<&str>,
    text_read: &str,
    candidates: &[String],
) -> String {
    if let Some(map) = session_map {
        return map.to_string();
    }
    let plausible = |m: &&str| candidates.is_empty() || candidates.iter().any(|c| c == m);
    let dropped = |m: &&str| {
        if !plausible(m) {
            tracing::debug!(
                read = %m,
                ?candidates,
                "map read is not a vote candidate — dropping as misread"
            );
        }
    };
    panel_read
        .inspect(dropped)
        .filter(plausible)
        .or_else(|| {
            Some(text_read)
                .filter(|m| !m.is_empty())
                .inspect(dropped)
                .filter(plausible)
        })
        .unwrap_or_default()
        .to_string()
}

/// Everything the blocking vision/OCR pass extracts from one Tab frame.
/// Replaces a 9-tuple whose adjacent same-typed fields were swap-prone.
struct FrameAnalysis {
    /// Outcome for this capture: the game's own if already known, else
    /// recovered from the frame (banner colors / result header text).
    outcome: detect::MatchOutcome,
    /// Full-image OCR (hero/map name lookup); the per-cell rows carry stats.
    ocr: Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>>,
    /// Column-calibrated per-cell OCR rows, one per scoreboard row.
    rows: Vec<ocr::RowOcrResult>,
    /// Portrait template match: (hero file stem, confidence).
    portrait_hero: Option<(String, f64)>,
    /// Career-panel hero title (most reliable source when present).
    career_hero: Option<String>,
    /// Top-bar map label OCR.
    map_from_panel: Option<String>,
    /// Cropped scoreboard (portrait auto-collection reads from it).
    scoreboard: image::DynamicImage,
    /// The player's row index, by name match or brightness highlight.
    player_row_idx: Option<usize>,
    /// Detected team size (5 or 6) — portrait geometry depends on it.
    team_size: usize,
    /// The full frame, kept for the rejected-capture archive.
    frame: image::DynamicImage,
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
    // screen, reused for every capture until the next game starts. Recovered
    // from the previous run when the daemon restarted mid-game (crash,
    // upgrade, systemd restart), so the restart neither splits the game into
    // two sessions nor loses its outcome/map context.
    let mut active_game: Option<ActiveGame> = recover_active_game(data_dir);
    if let Some(g) = &active_game {
        tracing::info!(
            session_id = %g.session_id,
            outcome = %g.outcome,
            "recovered open game from previous run"
        );
    }
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

    // Tab OCR also runs as a spawned task (single-flight), reporting back on
    // this channel. Awaited inline, one capture starved the poller for a
    // measured 45-70s — long enough to miss the ~3s VICTORY/DEFEAT banner and
    // the whole accolade screen, i.e. the outcome. The 400ms "let the game
    // render the scoreboard" wait sleeps inside the task too.
    let (capture_tx, mut capture_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, Result<CaptureReport, String>)>();
    let mut capture_task: Option<tokio::task::JoinHandle<()>> = None;

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

                        // Single-flight: OCR of the previous Tab may still be
                        // running (it takes seconds) — don't stack captures.
                        if capture_task.as_ref().is_some_and(|t| !t.is_finished()) {
                            tracing::debug!("Tab ignored — a capture is already in progress");
                            continue;
                        }
                        last_tab_capture = Some(Instant::now());

                        // Session choice: reuse the active game (mid-game, or
                        // post-match scoreboard within the grace window), or
                        // open a fresh one, inheriting a still-fresh outcome
                        // the poller saw before any game was open.
                        let opened_by_this_tab = should_start_fresh_session(active_game.as_ref(), Instant::now());
                        if opened_by_this_tab {
                            let inherited = take_fresh_pending(&mut pending_outcome, Instant::now());
                            active_game = Some(ActiveGame::open_now(
                                format!("{:016x}", rand_id()),
                                inherited.unwrap_or(detect::MatchOutcome::Unknown),
                                Vec::new(),
                            ));
                            last_game_open = Some(Instant::now());
                            // Word reads about the previous match must not
                            // confirm into this one.
                            word_outcome_streak = None;
                            persist_active_game(data_dir, active_game.as_ref());
                        }

                        let (sid, create, outcome, session_map, candidates, banner_ok) = {
                            let g = active_game.as_ref().expect("active_game set above");
                            // A banner-color outcome off a Tab frame is only
                            // plausible when the daemon just joined mid/post
                            // match (fresh Tab session) or the game is old
                            // enough to have actually ended. Mid-game heavy
                            // red vignettes otherwise fake a DEFEAT banner and
                            // split the real game.
                            let banner_ok = opened_by_this_tab
                                || g.opened_at.elapsed() >= MIN_BANNER_SESSION_AGE;
                            (g.session_id.clone(), !g.session_created, g.outcome, g.map.clone(), g.map_candidates.clone(), banner_ok)
                        };

                        let tx = capture_tx.clone();
                        let backend = *backend;
                        let store_task = store.clone();
                        let player_name = player_name.map(str::to_string);
                        let capture_output = capture_output.map(str::to_string);
                        let matcher = Arc::clone(portrait_matcher);
                        let data_dir_task = data_dir.to_path_buf();
                        capture_task = Some(tokio::spawn(async move {
                            // Wait for the game to render the scoreboard
                            // overlay after the Tab press.
                            tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;
                            let result = handle_capture(
                                &backend,
                                &store_task,
                                player_name.as_deref(),
                                capture_output.as_deref(),
                                &sid,
                                create,
                                outcome,
                                session_map.as_deref(),
                                &candidates,
                                banner_ok,
                                &matcher,
                                collect_portraits,
                                &data_dir_task,
                            )
                            .await
                            .map_err(|e| e.to_string());
                            let _ = tx.send((sid, result));
                        }));
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
                                drain_capture(capture_task.take()).await;
                                if let Some(client) = sync_client {
                                    try_sync(store, client, data_dir).await;
                                }
                                return Ok(());
                            }
                        }
                    }
                }
            }
            Some((sid, result)) = capture_rx.recv() => {
                match result {
                    Err(e) => tracing::error!(error = %e, "capture cycle failed"),
                    // Rejected by a trust gate — nothing was recorded, so
                    // the session must not be marked as created.
                    Ok(report) if !report.recorded => {}
                    Ok(report) => {
                        // Mutate the in-memory game only if it is still the
                        // game this capture was taken for — a start screen may
                        // have opened a new one while OCR was running. The
                        // store writes (keyed by session id) already happened
                        // inside the capture task and remain correct.
                        if let Some(g) = active_game.as_mut().filter(|g| g.session_id == sid) {
                            g.session_created = true;
                            g.touch();
                            // First trusted map discovery propagates
                            // to the whole session (one game, one map).
                            if g.map.is_none()
                                && let Some(map) = &report.map
                            {
                                g.map = Some(map.clone());
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
                                g.record_outcome(report.outcome);
                                tracing::info!(
                                    outcome = ?report.outcome,
                                    session_id = %g.session_id,
                                    "outcome recovered from captured frame — back-filling session"
                                );
                                if let Err(e) = store.set_session_outcome(&g.session_id, &g.outcome.to_string()).await {
                                    tracing::warn!(error = %e, "failed to back-fill session outcome");
                                }
                            }
                            persist_active_game(data_dir, Some(g));
                        }
                        capture_count += 1;
                        if let Some(client) = sync_client
                            && capture_count.is_multiple_of(SYNC_EVERY_N_CAPTURES)
                            && sync_task.as_ref().is_none_or(|t| t.is_finished()) {
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
                            // One RGBA→RGB conversion shared by banner + phase
                            // detectors (P6); title OCR still uses the original frame.
                            let rgb = img.to_rgb8();
                            let signal =
                                detect::match_end::detect_outcome_signal_with_rgb(&img, &rgb);
                            // The accolade screen also prints the map — read it
                            // while we're here; it recovers games where the
                            // in-game top-bar OCR missed all match.
                            let accolade_map = match &signal {
                                Some((_, detect::match_end::OutcomeSource::ResultWord)) => {
                                    detect::match_end::read_accolade_map(&img)
                                }
                                _ => None,
                            };
                            let phase = detect::match_start::detect_phase_with_rgb(&img, &rgb);
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
                                word_outcome_streak = Some((outcome, Instant::now()));
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
                                    g.record_outcome(outcome);
                                    tracing::info!(?outcome, session_id = %g.session_id, "auto-detect: outcome confirmed from post-match screens");
                                    if g.session_created {
                                        if let Err(e) = store.set_session_outcome(&g.session_id, &outcome.to_string()).await {
                                            tracing::warn!(error = %e, "failed to back-fill session outcome");
                                        }
                                        refresh_snapshot(store, data_dir).await;
                                    }
                                    persist_active_game(data_dir, Some(g));
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
                            && g.map.is_none()
                        {
                            // The accolade read is a dedicated-region OCR and
                            // trusted even alongside vote candidates (which
                            // used to block this recovery entirely) — but a
                            // contradiction is worth a trace.
                            if !g.map_candidates.is_empty() && !g.map_candidates.contains(&map) {
                                tracing::warn!(map = %map, candidates = ?g.map_candidates, "accolade map is not a vote candidate");
                            }
                            tracing::info!(map = %map, session_id = %g.session_id, "map recovered from accolade screen");
                            g.map = Some(map.clone());
                            g.touch();
                            if g.session_created {
                                if let Err(e) = store.set_session_map(&g.session_id, &map).await {
                                    tracing::warn!(error = %e, "failed to set session map");
                                }
                                refresh_snapshot(store, data_dir).await;
                            }
                            persist_active_game(data_dir, Some(g));
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
                                    // Vote names are screen-text constants
                                    // ("SHAMBALI") — canonicalize through the
                                    // MAPS table so candidate checks compare
                                    // display names with display names.
                                    let candidates: Vec<String> = maps
                                        .iter()
                                        .filter_map(|m| parse::canonical_map(m))
                                        .collect();
                                    tracing::info!(?candidates, session_id = %sid, "auto-detect: map vote — new game");
                                    active_game = Some(ActiveGame::open_now(sid, detect::MatchOutcome::Unknown, candidates));
                                    last_game_open = Some(Instant::now());
                                    // Evidence about the previous match must not
                                    // confirm into this one — and a pending
                                    // outcome from before any game was open
                                    // can't be this new game's result either.
                                    word_outcome_streak = None;
                                    pending_outcome = None;
                                    persist_active_game(data_dir, active_game.as_ref());
                                }
                            }
                            detect::GamePhase::HeroBan | detect::GamePhase::HeroSelect
                                if active_game.is_none() || game_finished =>
                            {
                                let sid = format!("{:016x}", rand_id());
                                tracing::info!(session_id = %sid, "auto-detect: hero select/ban — new game (map vote missed)");
                                active_game = Some(ActiveGame::open_now(sid, detect::MatchOutcome::Unknown, Vec::new()));
                                last_game_open = Some(Instant::now());
                                word_outcome_streak = None;
                                pending_outcome = None;
                                persist_active_game(data_dir, active_game.as_ref());
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
                                        g.record_outcome(outcome.parse().unwrap_or(detect::MatchOutcome::Unknown));
                                        persist_active_game(data_dir, Some(g));
                                    }
                            }
                            storage::StoreCommand::DeleteSession { session_id } => {
                                if active_game.as_ref().is_some_and(|g| g.session_id == *session_id) {
                                    active_game = None;
                                    persist_active_game(data_dir, None);
                                }
                            }
                        }
                        if let Err(e) = store.apply_command(cmd).await {
                            tracing::warn!(error = %e, "GUI command failed");
                        }
                    }
                    refresh_snapshot(store, data_dir).await;
                } else {
                    // Trailing edge of the snapshot debounce: a refresh that
                    // was deferred inside the window flushes here once due,
                    // so the GUI never waits on the *next* mutation.
                    flush_snapshot_if_due(store, data_dir).await;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                drain_capture(capture_task.take()).await;
                if let Some(client) = sync_client {
                    try_sync(store, client, data_dir).await;
                }
                flush_snapshot_if_dirty(store, data_dir).await;
                return Ok(());
            }
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM received — shutting down");
                drain_capture(capture_task.take()).await;
                if let Some(client) = sync_client {
                    try_sync(store, client, data_dir).await;
                }
                flush_snapshot_if_dirty(store, data_dir).await;
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
    session_map: Option<&str>,
    map_candidates: &[String],
    allow_banner_recovery: bool,
    portrait_matcher: &Arc<detect::hero_portrait::PortraitMatcher>,
    collect_portraits: bool,
    data_dir: &std::path::Path,
) -> Result<CaptureReport, Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Tab detected — capturing screen (hold Tab to keep scoreboard visible)");
    let img = capture::capture_screen_output(backend, capture_output).await?;

    let matcher = Arc::clone(portrait_matcher);
    // Clone player_name so the blocking closure can own it.
    let player_name_owned = player_name.map(|s| s.to_string());
    let session_map_known = session_map.is_some();
    let analysis = tokio::task::spawn_blocking(move || {
        // Outcome: prefer the open game's result (read off the accolade
        // screen by the poller); else color-flood detection (only when the
        // caller deems a banner plausible — see MIN_BANNER_SESSION_AGE); else
        // read the VICTORY/DEFEAT header text off this frame. The last step
        // recovers the case where the poller missed the screens and we're
        // sitting on a post-match scoreboard that prints the result header.
        let outcome = if matches!(game_outcome, detect::MatchOutcome::Unknown) {
            let o = if allow_banner_recovery {
                detect::match_end::detect_outcome(&img)
            } else {
                detect::MatchOutcome::Unknown
            };
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
        // Pass team_size into portrait match + cell OCR so neither re-detects
        // size or re-crops the full scoreboard (P7).
        let player_match = matcher.match_player_hero_with_team_size(&scoreboard, team_size);
        let portrait_match = player_match
            .as_ref()
            .map(|(name, conf, _)| (name.clone(), *conf));
        let brightness_row_idx = player_match.map(|(_, _, idx)| idx);

        let rows = ocr::recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size);

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

        // Full-board OCR exists only to supply raw text for hero/map name
        // lookup and the name-in-raw-text stats fallback. On the happy path —
        // player row found and parseable, hero from the career panel, map
        // already known — it is pure redundancy (adaptive preprocessing plus
        // up to three threshold sweeps), so run it lazily.
        let cells_parse =
            parse::parse_scoreboard_cells(&rows, row_idx, "", "unknown", None).is_some();
        let need_full_ocr = !cells_parse
            || career_hero.is_none()
            || (!session_map_known && map_from_panel.is_none());
        let ocr = if need_full_ocr {
            ocr::recognize(&img)
        } else {
            tracing::debug!("skipping full-board OCR — cell path supplied everything");
            Ok(ocr::OcrResult {
                raw_text: String::new(),
                confidence: 0,
            })
        };

        FrameAnalysis {
            outcome,
            ocr,
            rows,
            portrait_hero: portrait_match,
            career_hero,
            map_from_panel,
            scoreboard,
            player_row_idx: row_idx,
            team_size,
            frame: img,
        }
    })
    .await?;
    let FrameAnalysis {
        outcome,
        ocr,
        rows,
        portrait_hero,
        career_hero,
        map_from_panel,
        scoreboard: scoreboard_img,
        player_row_idx,
        team_size,
        frame: frame_img,
    } = analysis;
    let ocr_result = ocr?;

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

    let outcome_label = outcome.to_string();

    if let Some(mut parsed) = parse::parse_scoreboard_cells(
        &rows,
        player_row_idx,
        &ocr_result.raw_text,
        &outcome_label,
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
            // Shared geometry (5v5/6v6 + team gap) — an inlined 5v5-only copy
            // here used to mis-crop 6v6/team-2 references into the template
            // library.
            let dims = (scoreboard_img.width(), scoreboard_img.height());
            if let Some(r) = detect::hero_portrait::portrait_rect(
                dims,
                player_row_idx.unwrap_or(0),
                team_size,
            ) {
                let crop = scoreboard_img.crop_imm(r.x, r.y, r.w, r.h);
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

        parsed.map_name = resolve_map(
            session_map,
            map_from_panel.as_deref(),
            &parsed.map_name,
            map_candidates,
        );

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
                final_outcome: outcome_label.clone(),
            };
            // A failed create must abort the capture: pressing on would write
            // a snapshot the caller then marks "session created", and every
            // later outcome/map back-fill would update a session row that
            // doesn't exist. The Tab can simply be pressed again.
            store.create_session(&session).await.map_err(
                |e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("session create failed: {e}").into()
                },
            )?;
            tracing::info!(session_id = %session_id, "started new match session");
        } else if let Err(e) = store.append_capture(session_id, now, &outcome_label).await {
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

/// Let an in-flight capture task finish before shutdown's final sync, so its
/// snapshot is uploaded and no store write is torn by the runtime dropping it.
async fn drain_capture(task: Option<tokio::task::JoinHandle<()>>) {
    if let Some(t) = task
        && !t.is_finished()
    {
        tracing::info!("waiting for in-flight capture to finish");
        let _ = t.await;
    }
}

/// Minimum gap between full snapshot rewrites (P11). Capture/poll can mark the
/// export dirty faster than this; the next due flush (or a forced one after
/// sync / shutdown) actually rewrites the file.
const SNAPSHOT_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);

/// (dirty, last successful export). Module-level so capture + sync tasks share it.
static SNAPSHOT_STATE: std::sync::Mutex<(bool, Option<std::time::Instant>)> =
    std::sync::Mutex::new((false, None));

/// Mark the live snapshot dirty and export if the debounce window has elapsed.
async fn refresh_snapshot(store: &storage::LocalStore, data_dir: &std::path::Path) {
    refresh_snapshot_inner(store, data_dir, false).await;
}

/// Export immediately (sync complete, shutdown). Resets dirty.
async fn refresh_snapshot_force(store: &storage::LocalStore, data_dir: &std::path::Path) {
    refresh_snapshot_inner(store, data_dir, true).await;
}

/// Flush a debounce-deferred snapshot once its window has passed. Called from
/// the cmd-timer tick so a dirty snapshot never waits on the next mutation.
async fn flush_snapshot_if_due(store: &storage::LocalStore, data_dir: &std::path::Path) {
    let due = {
        let state = SNAPSHOT_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.0 && state.1.is_none_or(|last| last.elapsed() >= SNAPSHOT_DEBOUNCE)
    };
    if due {
        refresh_snapshot_inner(store, data_dir, true).await;
    }
}

/// Shutdown path: export a trailing dirty snapshot regardless of the window —
/// the GUI must see the final state (try_sync only force-exports on success).
async fn flush_snapshot_if_dirty(store: &storage::LocalStore, data_dir: &std::path::Path) {
    let dirty = SNAPSHOT_STATE.lock().unwrap_or_else(|e| e.into_inner()).0;
    if dirty {
        refresh_snapshot_inner(store, data_dir, true).await;
    }
}

async fn refresh_snapshot_inner(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    force: bool,
) {
    {
        let mut state = SNAPSHOT_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.0 = true;
        if !force
            && let Some(last) = state.1
            && last.elapsed() < SNAPSHOT_DEBOUNCE
        {
            return;
        }
    }
    match store.export_snapshot(data_dir).await {
        Ok(()) => {
            if let Ok(mut state) = SNAPSHOT_STATE.lock() {
                state.0 = false;
                state.1 = Some(std::time::Instant::now());
            }
        }
        Err(e) => tracing::debug!(error = %e, "live snapshot refresh failed"),
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
            // Sync flips `synced` flags — GUI must see that promptly.
            refresh_snapshot_force(store, data_dir).await;
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
            map: None,
            map_candidates: Vec::new(),
            session_created: true,
            outcome_recorded_at: recorded_secs_ago.map(|s| now - Duration::from_secs(s)),
            opened_at: now - Duration::from_secs(300),
            last_activity: now - Duration::from_secs(30),
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
    fn idle_unfinished_session_goes_stale() {
        // An unfinished session with recent activity is reusable...
        let now = test_now() + UNFINISHED_SESSION_IDLE * 2;
        let g = game(detect::MatchOutcome::Unknown, None, now);
        assert!(!should_start_fresh_session(Some(&g), now));
        // ...but one idle past the bound can't plausibly be the same game:
        // yesterday's session must not absorb today's first Tab.
        let mut stale = game(detect::MatchOutcome::Unknown, None, now);
        stale.last_activity = now - UNFINISHED_SESSION_IDLE - Duration::from_secs(1);
        assert!(should_start_fresh_session(Some(&stale), now));
    }

    #[test]
    fn active_game_persists_and_recovers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut g = ActiveGame::open_now("abc123".into(), detect::MatchOutcome::Unknown, vec![
            "Oasis".into(),
            "Busan".into(),
        ]);
        g.session_created = true;
        g.map = Some("Oasis".into());
        persist_active_game(dir.path(), Some(&g));

        let r = recover_active_game(dir.path()).expect("recent game recovers");
        assert_eq!(r.session_id, "abc123");
        assert_eq!(r.outcome, detect::MatchOutcome::Unknown);
        assert_eq!(r.map.as_deref(), Some("Oasis"));
        assert_eq!(r.map_candidates, vec!["Oasis".to_string(), "Busan".to_string()]);
        assert!(r.session_created);

        // Clearing removes the file — nothing to recover.
        persist_active_game(dir.path(), None);
        assert!(recover_active_game(dir.path()).is_none());
    }

    #[test]
    fn stale_persisted_game_is_not_recovered() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stale = PersistedGame {
            session_id: "old".into(),
            outcome: detect::MatchOutcome::Unknown,
            map: None,
            map_candidates: Vec::new(),
            session_created: true,
            opened_at: Utc::now() - chrono::Duration::hours(9),
            last_activity: Utc::now() - chrono::Duration::hours(8),
            outcome_recorded_at: None,
        };
        std::fs::write(
            active_game_path(dir.path()),
            serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();
        assert!(
            recover_active_game(dir.path()).is_none(),
            "yesterday's unfinished game must not swallow today's captures"
        );
    }

    #[test]
    fn vote_candidates_constrain_but_never_become_the_map() {
        let candidates = vec!["Oasis".to_string(), "Busan".to_string()];

        // No trusted read yet: candidates alone must NOT pick a map — the
        // vote winner is unknowable, and detected_maps[0] used to be wrong
        // ~2/3 of the time.
        assert_eq!(resolve_map(None, None, "", &candidates), "");

        // Session's confirmed map always wins.
        assert_eq!(resolve_map(Some("Busan"), Some("Oasis"), "Ilios", &candidates), "Busan");

        // Panel read accepted when it is a candidate...
        assert_eq!(resolve_map(None, Some("Busan"), "", &candidates), "Busan");
        // ...vetoed when it isn't (misread), falling back to a plausible
        // text read, or to nothing.
        assert_eq!(resolve_map(None, Some("Ilios"), "Oasis", &candidates), "Oasis");
        assert_eq!(resolve_map(None, Some("Ilios"), "Nepal", &candidates), "");

        // Without candidates, panel > text (unchanged behavior).
        assert_eq!(resolve_map(None, Some("Ilios"), "Nepal", &[]), "Ilios");
        assert_eq!(resolve_map(None, None, "Nepal", &[]), "Nepal");
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
