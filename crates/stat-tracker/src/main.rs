use stat_tracker::{capture, config, detect, ocr, parse, setup, storage, sync};

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use surrealdb_types::Datetime as SurrealDatetime;
use tracing_subscriber::EnvFilter;

const SYNC_EVERY_N_CAPTURES: u32 = 5;

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
    if config.player_name.is_none() {
        if let Some(sync_cfg) = &config.sync {
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
                    tracing::info!("server has no player_name configured — set it in the web UI under My Stats → Settings");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "could not fetch daemon config from server (continuing without player_name)");
                }
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
        &portrait_matcher,
        collect_portraits,
        &data_dir,
    )
    .await
}

/// The game currently in progress. Opened when the poller sees a game-start
/// screen (map vote / hero select / ban) and reused for every Tab capture until
/// the next game starts — so captures taken across hero swaps all land in one
/// session. `outcome` is filled in when the poller reads the post-match accolade
/// screen, then back-filled onto the captured snapshots.
struct ActiveGame {
    session_id: String,
    outcome: detect::MatchOutcome,
    maps: Vec<String>,
    /// Whether the `match_session` row has been created (on the first capture).
    session_created: bool,
}

fn outcome_str(outcome: &detect::MatchOutcome) -> &'static str {
    match outcome {
        detect::MatchOutcome::Victory => "victory",
        detect::MatchOutcome::Defeat => "defeat",
        detect::MatchOutcome::Draw => "draw",
        detect::MatchOutcome::Unknown => "unknown",
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
    portrait_matcher: &Arc<detect::hero_portrait::PortraitMatcher>,
    collect_portraits: bool,
    data_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
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
    // next game that opens (e.g. daemon started after the match began).
    let mut pending_outcome: Option<detect::MatchOutcome> = None;

    let mut poll_timer = tokio::time::interval(poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            result = kbd.wait_tab() => {
                match result {
                    Ok(()) => {
                        if let Some(last) = last_tab_capture
                            && last.elapsed() < tab_debounce {
                                tracing::debug!("Tab debounced — ignoring rapid press");
                                continue;
                            }

                        // Wait for the game to render the scoreboard overlay after Tab press
                        tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;
                        last_tab_capture = Some(Instant::now());

                        // No game open (daemon started mid-match, or the start
                        // screen was missed) — open one now, inheriting any
                        // outcome the poller already saw.
                        if active_game.is_none() {
                            active_game = Some(ActiveGame {
                                session_id: format!("{:016x}", rand_id()),
                                outcome: pending_outcome.take().unwrap_or(detect::MatchOutcome::Unknown),
                                maps: Vec::new(),
                                session_created: false,
                            });
                            last_game_open = Some(Instant::now());
                        }

                        let (sid, create, outcome, maps) = {
                            let g = active_game.as_ref().expect("active_game set above");
                            (g.session_id.clone(), !g.session_created, g.outcome.clone(), g.maps.clone())
                        };

                        if let Err(e) = handle_capture(backend, store, player_name, capture_output, &sid, create, outcome, &maps, portrait_matcher, collect_portraits, data_dir).await {
                            tracing::error!(error = %e, "capture cycle failed");
                        } else {
                            if let Some(g) = active_game.as_mut() {
                                g.session_created = true;
                            }
                            capture_count += 1;
                            if let Some(client) = sync_client
                                && capture_count.is_multiple_of(SYNC_EVERY_N_CAPTURES) {
                                    try_sync(store, client).await;
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
                                    try_sync(store, client).await;
                                }
                                return Ok(());
                            }
                        }
                    }
                }
            }
            _ = poll_timer.tick(), if auto_detect.enabled => {
                match capture::capture_screen_output(backend, capture_output).await {
                    Ok(img) => {
                        let (outcome, phase) = tokio::task::spawn_blocking(move || {
                            let outcome = detect::match_end::detect_outcome(&img);
                            let phase = detect::match_start::detect_phase(&img);
                            (outcome, phase)
                        }).await.unwrap_or((detect::MatchOutcome::Unknown, detect::GamePhase::Unknown));

                        // Post-match accolade screen → record the outcome on the
                        // open game. Idempotent: the screen shows for ~20s (several
                        // ticks) but only the first, while the outcome is still
                        // Unknown, writes it.
                        if matches!(outcome, detect::MatchOutcome::Victory | detect::MatchOutcome::Defeat | detect::MatchOutcome::Draw) {
                            match active_game.as_mut() {
                                Some(g) if matches!(g.outcome, detect::MatchOutcome::Unknown) => {
                                    g.outcome = outcome.clone();
                                    tracing::info!(?outcome, session_id = %g.session_id, "auto-detect: outcome read from accolade screen");
                                    if g.session_created
                                        && let Err(e) = store.set_session_outcome(&g.session_id, outcome_str(&outcome)).await {
                                            tracing::warn!(error = %e, "failed to back-fill session outcome");
                                        }
                                }
                                Some(_) => { /* outcome already recorded for this game */ }
                                None => {
                                    // No game open yet — apply to the next capture.
                                    pending_outcome = Some(outcome);
                                }
                            }
                        }

                        // Game-start screens open a new game. Map vote is the
                        // unambiguous boundary (debounced so a lingering screen
                        // across ticks opens only one game); hero select/ban only
                        // open a game when none is active (fallback if the map
                        // vote was missed).
                        match phase {
                            detect::GamePhase::MapVote { maps } => {
                                let can_open = active_game.is_none()
                                    || last_game_open.is_none_or(|t| t.elapsed() >= new_game_debounce);
                                if can_open {
                                    let sid = format!("{:016x}", rand_id());
                                    tracing::info!(?maps, session_id = %sid, "auto-detect: map vote — new game");
                                    active_game = Some(ActiveGame { session_id: sid, outcome: detect::MatchOutcome::Unknown, maps, session_created: false });
                                    last_game_open = Some(Instant::now());
                                }
                            }
                            detect::GamePhase::HeroBan | detect::GamePhase::HeroSelect
                                if active_game.is_none() =>
                            {
                                let sid = format!("{:016x}", rand_id());
                                tracing::info!(session_id = %sid, "auto-detect: hero select/ban — new game (map vote missed)");
                                active_game = Some(ActiveGame { session_id: sid, outcome: detect::MatchOutcome::Unknown, maps: Vec::new(), session_created: false });
                                last_game_open = Some(Instant::now());
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        tracing::trace!(error = %e, "poll capture failed (game may not be running)");
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                if let Some(client) = sync_client {
                    try_sync(store, client).await;
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Tab detected — capturing screen (hold Tab to keep scoreboard visible)");
    let img = capture::capture_screen_output(backend, capture_output).await?;

    let matcher = Arc::clone(portrait_matcher);
    // Clone player_name so the blocking closure can own it.
    let player_name_owned = player_name.map(|s| s.to_string());
    let (outcome, ocr_result, rows, portrait_hero, career_hero, map_from_panel, scoreboard_img, player_row_idx) =
        tokio::task::spawn_blocking(move || {
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

            (outcome, ocr, rows, portrait_match, career_hero, map_from_panel, scoreboard, row_idx)
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
            parsed.hero = hero_name.clone();
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
        storage::append_match_log(data_dir, &parsed);
        store.insert_match(parsed).await.map_err(
            |e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("store insert failed: {e}").into()
            },
        )?;
    } else {
        tracing::warn!("could not parse scoreboard from OCR text");
    }

    Ok(())
}

fn rand_id() -> u64 {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    seed ^ (std::process::id() as u64).wrapping_mul(0x517cc1b727220a95)
}

async fn try_sync(store: &storage::LocalStore, client: &sync::SyncClient) {
    match store.get_unsynced().await {
        Ok(unsynced) if !unsynced.is_empty() => {
            tracing::info!(count = unsynced.len(), "syncing unsynced matches");
            match client.upload_matches(&unsynced).await {
                Ok(resp) => {
                    tracing::info!(
                        inserted = resp.inserted,
                        skipped = resp.skipped,
                        "sync complete"
                    );
                    if let Err(e) = store.mark_synced(unsynced.len()).await {
                        tracing::error!(error = %e, "failed to mark matches as synced");
                    }
                }
                Err(e) => tracing::error!(error = %e, "sync upload failed"),
            }
        }
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "failed to query unsynced matches"),
    }
}
