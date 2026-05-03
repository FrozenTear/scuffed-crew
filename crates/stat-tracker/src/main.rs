use stat_tracker::{capture, config, detect, ocr, parse, setup, storage, sync};

use std::time::Instant;

use tracing_subscriber::EnvFilter;

const SYNC_EVERY_N_CAPTURES: u32 = 5;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

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

    if let Err(e) = setup::ensure_koverwatch_tessdata() {
        tracing::warn!(error = %e, "koverwatch tessdata setup failed — falling back to eng");
    }

    std::fs::create_dir_all(&config.data_dir)?;

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

    run_loop(
        &backend,
        &store,
        sync_client.as_ref(),
        config.player_name.as_deref(),
        config.capture_output.as_deref(),
        &config.auto_detect,
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
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = match detect::open_keyboard_stream() {
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

    let mut poll_timer = tokio::time::interval(poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            result = detect::wait_next_tab(&mut stream) => {
                match result {
                    Ok(()) => {
                        let outcome = pending_outcome.take();
                        if outcome.is_some() {
                            tracing::info!("Tab pressed — using pending outcome from auto-detect");
                        }
                        if let Err(e) = handle_capture(backend, store, player_name, capture_output, outcome).await {
                            tracing::error!(error = %e, "capture cycle failed");
                        } else {
                            capture_count += 1;
                            if let Some(client) = sync_client {
                                if capture_count % SYNC_EVERY_N_CAPTURES == 0 {
                                    try_sync(store, client).await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "evdev stream error — attempting to reopen");
                        match detect::open_keyboard_stream() {
                            Ok(new_stream) => {
                                stream = new_stream;
                                tracing::info!("keyboard stream reopened");
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
                        let outcome = tokio::task::spawn_blocking(move || {
                            detect::match_end::detect_outcome(&img)
                        }).await.unwrap_or(detect::MatchOutcome::Unknown);
                        match outcome {
                            detect::MatchOutcome::Victory | detect::MatchOutcome::Defeat => {
                                tracing::info!(?outcome, "auto-detect: match end detected");
                                last_auto_detect = Some(Instant::now());
                                pending_outcome = Some(outcome);
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Tab detected — capturing screen");
    let img = capture::capture_screen_output(backend, capture_output).await?;

    let (outcome, ocr_result) = tokio::task::spawn_blocking(move || {
        let outcome = polled_outcome
            .unwrap_or_else(|| detect::match_end::detect_outcome(&img));
        let ocr = ocr::recognize(&img);
        (outcome, ocr)
    })
    .await?;
    let ocr_result = ocr_result?;

    tracing::info!(?outcome, "frame analysis");
    tracing::info!(
        confidence = ocr_result.confidence,
        text_preview = &ocr_result.raw_text[..ocr_result.raw_text.len().min(120)],
        "OCR result"
    );

    let outcome_str = match outcome {
        detect::MatchOutcome::Victory => "victory",
        detect::MatchOutcome::Defeat => "defeat",
        detect::MatchOutcome::Draw => "draw",
        detect::MatchOutcome::Unknown => "unknown",
    };

    if let Some(parsed) = parse::parse_scoreboard(&ocr_result.raw_text, outcome_str, player_name) {
        tracing::info!(
            hero = %parsed.hero,
            elims = parsed.elims,
            deaths = parsed.deaths,
            "parsed scoreboard"
        );
        store.insert_match(parsed).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("store insert failed: {e}").into()
        })?;
    } else {
        tracing::warn!("could not parse scoreboard from OCR text");
    }

    Ok(())
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
