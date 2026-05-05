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
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("scuffed_stat_tracker=info,stat_tracker=info,surrealdb=warn")),
        )
        .init();

    let collect_portraits = std::env::args().any(|a| a == "--collect-portraits");

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

    let config = config::Config::load()?;
    tracing::info!("Scuffed Stat Tracker starting");
    tracing::info!(data_dir = %config.data_dir.display(), "using data directory");

    std::fs::create_dir_all(&config.data_dir)?;

    let pid_path = config.data_dir.join("daemon.pid");
    if let Ok(existing_pid) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = existing_pid.trim().parse::<u32>() {
            if std::fs::metadata(format!("/proc/{pid}")).is_ok() {
                tracing::error!(pid, "another daemon is already running — stop it first");
                return Err(format!("another daemon is already running (PID {pid})").into());
            }
        }
        let _ = std::fs::remove_file(&pid_path);
    }
    std::fs::write(&pid_path, std::process::id().to_string())?;

    let _pid_guard = PidGuard(pid_path);

    if let Err(e) = setup::ensure_koverwatch_tessdata() {
        tracing::warn!(error = %e, "koverwatch tessdata setup failed — falling back to eng");
    }

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
    let portrait_matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(&portraits_path));

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
        tracing::info!("portrait collection mode enabled — will save portrait references when OCR identifies heroes");
    }

    run_loop(
        &backend,
        &store,
        sync_client.as_ref(),
        config.player_name.as_deref(),
        config.capture_output.as_deref(),
        &config.auto_detect,
        config.session_window_secs,
        &portrait_matcher,
        collect_portraits,
        &data_dir,
    )
    .await
}

async fn run_loop(
    backend: &capture::CaptureBackend,
    store: &storage::LocalStore,
    sync_client: Option<&sync::SyncClient>,
    player_name: Option<&str>,
    capture_output: Option<&str>,
    auto_detect: &config::AutoDetectConfig,
    session_window_secs: u64,
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
    let cooldown_duration = std::time::Duration::from_secs(auto_detect.cooldown_secs);
    let mut last_auto_detect: Option<Instant> = None;

    // Pending outcome detected by the poller, consumed by the next Tab capture
    let mut pending_outcome: Option<detect::MatchOutcome> = None;
    // When true, the next capture starts a fresh session regardless of hero/time
    let mut new_match_pending = false;
    // Maps detected from vote screen
    let mut pending_maps: Vec<String> = Vec::new();

    let mut poll_timer = tokio::time::interval(poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            result = kbd.wait_tab() => {
                match result {
                    Ok(()) => {
                        // Wait for the game to render the scoreboard overlay after Tab press
                        tokio::time::sleep(tokio::time::Duration::from_millis(750)).await;

                        let outcome = pending_outcome.take();
                        if outcome.is_some() {
                            tracing::info!("Tab pressed — using pending outcome from auto-detect");
                        }
                        let force_new = new_match_pending;
                        let maps = std::mem::take(&mut pending_maps);
                        if let Err(e) = handle_capture(backend, store, player_name, capture_output, outcome, session_window_secs, force_new, &maps, portrait_matcher, collect_portraits, data_dir).await {
                            tracing::error!(error = %e, "capture cycle failed");
                        } else {
                            new_match_pending = false;
                            capture_count += 1;
                            if let Some(client) = sync_client {
                                if capture_count % SYNC_EVERY_N_CAPTURES == 0 {
                                    try_sync(store, client).await;
                                }
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
                let in_cooldown = last_auto_detect
                    .is_some_and(|t| t.elapsed() < cooldown_duration);
                if in_cooldown {
                    continue;
                }

                match capture::capture_screen_output(backend, capture_output).await {
                    Ok(img) => {
                        let (outcome, phase) = tokio::task::spawn_blocking(move || {
                            let outcome = detect::match_end::detect_outcome(&img);
                            let phase = detect::match_start::detect_phase(&img);
                            (outcome, phase)
                        }).await.unwrap_or((detect::MatchOutcome::Unknown, detect::GamePhase::Unknown));

                        match outcome {
                            detect::MatchOutcome::Victory | detect::MatchOutcome::Defeat => {
                                tracing::info!(?outcome, "auto-detect: match end detected — press Tab on scoreboard to capture stats");
                                last_auto_detect = Some(Instant::now());
                                pending_outcome = Some(outcome);
                            }
                            _ => {}
                        }

                        match phase {
                            detect::GamePhase::MapVote { maps } => {
                                if !new_match_pending {
                                    tracing::info!(?maps, "auto-detect: map vote — new match starting");
                                    new_match_pending = true;
                                    pending_maps = maps;
                                    last_auto_detect = Some(Instant::now());
                                }
                            }
                            detect::GamePhase::HeroBan => {
                                if !new_match_pending {
                                    tracing::info!("auto-detect: hero ban phase — new match starting");
                                    new_match_pending = true;
                                    last_auto_detect = Some(Instant::now());
                                }
                            }
                            detect::GamePhase::HeroSelect => {
                                if !new_match_pending {
                                    tracing::info!("auto-detect: hero select — new match starting");
                                    new_match_pending = true;
                                    last_auto_detect = Some(Instant::now());
                                }
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

async fn handle_capture(
    backend: &capture::CaptureBackend,
    store: &storage::LocalStore,
    player_name: Option<&str>,
    capture_output: Option<&str>,
    polled_outcome: Option<detect::MatchOutcome>,
    session_window_secs: u64,
    force_new_session: bool,
    detected_maps: &[String],
    portrait_matcher: &Arc<detect::hero_portrait::PortraitMatcher>,
    collect_portraits: bool,
    data_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Tab detected — capturing screen (hold Tab to keep scoreboard visible)");
    let img = capture::capture_screen_output(backend, capture_output).await?;

    let matcher = Arc::clone(portrait_matcher);
    let (outcome, ocr_result, portrait_hero, scoreboard_img, player_row_idx) = tokio::task::spawn_blocking(move || {
        let outcome = polled_outcome
            .unwrap_or_else(|| detect::match_end::detect_outcome(&img));
        let ocr = ocr::recognize(&img);

        // Try portrait matching — detect the player's highlighted row first
        let scoreboard = ocr::preprocess::crop_scoreboard(&img);
        let player_match = matcher.match_player_hero(&scoreboard);
        let portrait_match = player_match.as_ref().map(|(name, conf, _)| (name.clone(), *conf));
        let row_idx = player_match.map(|(_, _, idx)| idx);

        (outcome, ocr, portrait_match, scoreboard, row_idx)
    })
    .await?;
    let ocr_result = ocr_result?;

    tracing::info!(?outcome, "frame analysis");
    let preview_end = ocr_result.raw_text
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

    if ocr_result.confidence < 55 {
        tracing::warn!(
            confidence = ocr_result.confidence,
            "OCR confidence too low — skipping (debug images saved to data_dir/debug/)"
        );
        return Ok(());
    }

    let outcome_str = match outcome {
        detect::MatchOutcome::Victory => "victory",
        detect::MatchOutcome::Defeat => "defeat",
        detect::MatchOutcome::Draw => "draw",
        detect::MatchOutcome::Unknown => "unknown",
    };

    if let Some(mut parsed) = parse::parse_scoreboard(&ocr_result.raw_text, outcome_str, player_name, player_row_idx) {
        // Portrait matching is the primary hero detection method (more reliable than OCR text)
        if let Some((hero_name, confidence)) = &portrait_hero {
            tracing::info!(
                hero = %hero_name,
                confidence = confidence,
                source = "portrait",
                "hero identified via portrait template matching"
            );
            parsed.hero = hero_name.clone();
            parsed.role = parse::guess_role_public(&parsed.hero);
        }

        // Auto-collect portrait reference when hero is identified and collection is enabled
        if collect_portraits && parsed.hero != "Unknown" {
            let portraits_path = detect::hero_portrait::portraits_dir(data_dir);
            let (sw, sh) = (scoreboard_img.width(), scoreboard_img.height());
            let portrait_w = sw * 6 / 100;
            let portrait_x = sw * 1 / 100;
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

        // Populate map name: prefer auto-detected maps, then OCR-parsed map
        if !detected_maps.is_empty() && parsed.map_name.is_empty() {
            parsed.map_name = detected_maps[0].clone();
        }

        let session_id = if force_new_session {
            let sid = format!("{:016x}", rand_id());
            let session = storage::MatchSession {
                session_id: sid.clone(),
                hero: parsed.hero.clone(),
                map_name: parsed.map_name.clone(),
                role: parsed.role.clone(),
                started_at: now.clone(),
                last_capture_at: now.clone(),
                capture_count: 1,
                final_outcome: outcome_str.to_string(),
            };
            if let Err(e) = store.create_session(&session).await {
                tracing::warn!(error = %e, "failed to create session");
            }
            tracing::info!(session_id = %sid, "started new match session (phase-detected boundary)");
            sid
        } else {
            match store.find_active_session(&parsed.hero, session_window_secs).await {
                Ok(Some(session)) => {
                    // Inherit map name from session if this capture doesn't have one
                    if parsed.map_name.is_empty() && !session.map_name.is_empty() {
                        parsed.map_name = session.map_name.clone();
                    }
                    let new_count = session.capture_count + 1;
                    if let Err(e) = store.update_session(
                        &session.session_id,
                        now.clone(),
                        new_count,
                        outcome_str,
                    ).await {
                        tracing::warn!(error = %e, "failed to update session");
                    }
                    tracing::info!(
                        session_id = %session.session_id,
                        capture_num = new_count,
                        "appending to existing match session"
                    );
                    session.session_id
                }
                _ => {
                    let sid = format!("{:016x}", rand_id());
                    let session = storage::MatchSession {
                        session_id: sid.clone(),
                        hero: parsed.hero.clone(),
                        map_name: parsed.map_name.clone(),
                        role: parsed.role.clone(),
                        started_at: now.clone(),
                        last_capture_at: now.clone(),
                        capture_count: 1,
                        final_outcome: outcome_str.to_string(),
                    };
                    if let Err(e) = store.create_session(&session).await {
                        tracing::warn!(error = %e, "failed to create session");
                    }
                    tracing::info!(session_id = %sid, "started new match session");
                    sid
                }
            }
        };

        parsed.session_id = session_id;

        tracing::info!(
            hero = %parsed.hero,
            map = %parsed.map_name,
            elims = parsed.elims,
            deaths = parsed.deaths,
            "parsed scoreboard"
        );
        storage::append_match_log(data_dir, &parsed);
        store.insert_match(parsed).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("store insert failed: {e}").into()
        })?;
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
                    tracing::info!(inserted = resp.inserted, skipped = resp.skipped, "sync complete");
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
