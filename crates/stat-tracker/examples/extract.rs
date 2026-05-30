//! Dev tool: run the full capture-time extraction pipeline against a still image
//! and print what it gets, without needing the game or a live capture.
//!
//! Usage: cargo run -p scuffed-stat-tracker --example extract -- <image.png>

use std::sync::Arc;

use stat_tracker::{detect, ocr, parse};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let path = std::env::args()
        .nth(1)
        .ok_or("usage: extract <image.png> [team_size] [player_name]")?;
    // Optional 2nd arg: force team size (5 or 6) to isolate detection bugs.
    let team_override: Option<usize> = std::env::args().nth(2).and_then(|s| s.parse().ok());
    // Optional 3rd arg: player name — enables name-based row search across both teams.
    let player_name: Option<String> = std::env::args().nth(3);

    let img = image::open(&path)?;
    println!("image: {}x{}", img.width(), img.height());

    // Portraits: unpack bundled refs into the standard data dir.
    let data_dir = dirs::data_dir()
        .map(|d| d.join("scuffed-stat-tracker"))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(
        &detect::hero_portrait::portraits_dir(&data_dir),
    ));

    // --- mirror handle_capture's blocking section ---
    let scoreboard = ocr::preprocess::crop_scoreboard(&img);
    let detected_team = detect::hero_portrait::detect_team_size(&scoreboard);
    let team_size = team_override.unwrap_or(detected_team);
    println!("detected team size: {detected_team} (using: {team_size})");

    let color_outcome = detect::match_end::detect_outcome(&img);
    let text_outcome = detect::match_end::detect_outcome_text(&img);
    println!("outcome (color flood): {color_outcome:?}");
    println!("outcome (header text): {text_outcome:?}");

    let player_match = matcher.match_player_hero(&scoreboard);
    match &player_match {
        Some((hero, conf, idx)) => {
            println!("portrait: player row {idx} -> {hero} (conf {conf:.2})")
        }
        None => println!("portrait: no highlighted player row matched"),
    }
    let brightness_row_idx = player_match.as_ref().map(|(_, _, idx)| *idx);

    // Career-panel hero title (plain text) and top-bar map label — both read
    // from dedicated regions outside the scoreboard crop.
    let career_raw = ocr::recognize_region(&ocr::preprocess::crop_career_hero(&img)).unwrap_or_default();
    let map_raw = ocr::recognize_region(&ocr::preprocess::crop_map_name(&img)).unwrap_or_default();
    println!(
        "career hero: raw={:?} -> {:?}",
        career_raw.trim(),
        parse::match_hero_in_text(&career_raw)
    );
    println!(
        "map label:   raw={:?} -> {:?}",
        map_raw.trim(),
        parse::match_map_in_text(&map_raw)
    );

    {
        let board = ocr::preprocess::crop_scoreboard(&img);
        println!("scoreboard crop: {}x{}", board.width(), board.height());
    }
    let rows = ocr::recognize_scoreboard_cells_with_team_size(&img, Some(team_size));
    // Name-based row search across all rows (both teams) — handles replay/post-match.
    let player_row_idx = player_name
        .as_deref()
        .and_then(|n| parse::find_player_row_by_name(&rows, n))
        .or(brightness_row_idx);
    if let (Some(name), Some(idx)) = (&player_name, player_row_idx) {
        println!("player name '{name}' -> row {idx}");
    }
    println!("rows returned: {}", rows.len());
    println!("\nper-cell OCR rows (E A D DMG HLG MIT):");
    for (i, row) in rows.iter().enumerate() {
        let name = row.name.as_ref().map(|n| n.value.as_str()).unwrap_or("?");
        let cells: Vec<String> = row
            .stats
            .iter()
            .map(|c| {
                format!(
                    "{}({})",
                    if c.value.is_empty() { "_" } else { &c.value },
                    c.confidence
                )
            })
            .collect();
        let marker = if Some(i) == player_row_idx {
            " <- player"
        } else {
            ""
        };
        println!(
            "  row {i:>2}: name={name:<14} [{}] mean_conf={}{marker}",
            cells.join(" "),
            row.mean_confidence
        );
    }

    let ocr_result = ocr::recognize(&img)?;
    println!("\nfull-image OCR confidence: {}", ocr_result.confidence);

    let outcome = match color_outcome {
        detect::MatchOutcome::Unknown => text_outcome,
        o => o,
    };
    let outcome_str = match outcome {
        detect::MatchOutcome::Victory => "victory",
        detect::MatchOutcome::Defeat => "defeat",
        detect::MatchOutcome::Draw => "draw",
        detect::MatchOutcome::Unknown => "unknown",
    };

    println!("\n=== parsed result ===");
    match parse::parse_scoreboard_cells(
        &rows,
        player_row_idx,
        &ocr_result.raw_text,
        outcome_str,
        player_name.as_deref(),
    ) {
        Some(mut m) => {
            // Mirror handle_capture's hero/map priority so this reflects the daemon.
            if let Some(h) = parse::match_hero_in_text(&career_raw) {
                m.role = parse::guess_role_public(&h);
                m.hero = h;
            }
            if let Some(map) = parse::match_map_in_text(&map_raw) {
                m.map_name = map;
            }
            println!("hero:       {}", m.hero);
            println!("role:       {}", m.role);
            println!(
                "map:        {}",
                if m.map_name.is_empty() {
                    "(none)"
                } else {
                    &m.map_name
                }
            );
            println!("outcome:    {}", m.outcome);
            println!("elims:      {}", m.elims);
            println!("assists:    {}", m.assists);
            println!("deaths:     {}", m.deaths);
            println!("damage:     {}", m.damage);
            println!("healing:    {}", m.healing);
            println!("mitigation: {}", m.mitigation);
        }
        None => println!("parse returned None (no plausible stat row)"),
    }

    Ok(())
}
