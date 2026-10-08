//! Local evaluation of the shadow digit matcher against hand labels.
//!
//! Usage: shadow_eval <frames_dir> <labels.json>
//!
//! Each PNG in `frames_dir` is either a full capture (cropped with
//! `crop_scoreboard`) or an already-cropped scoreboard (aspect below 16:9).
//! Labels: JSON array of {frame, team (1|2), team_row, field, value}; HERO rows
//! are ignored. Team size comes from the tracker's detector unless
//! SE_TEAM=5|6 forces it. Prints per-frame lines, errors, and totals.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use stat_tracker::shadow::digits::{FIELDS, read_board};
use stat_tracker::{detect, ocr};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: shadow_eval <frames_dir> <labels.json>");
        std::process::exit(2);
    }
    let force_team: Option<usize> = std::env::var("SE_TEAM").ok().and_then(|s| s.parse().ok());
    let labels: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&args[1]).expect("read labels"))
            .expect("parse labels");
    // (frame, team 0|1, team_row, field index) -> value
    let mut truth: HashMap<(String, usize, usize, usize), u64> = HashMap::new();
    for l in &labels {
        let field = l["field"].as_str().unwrap_or("");
        let Some(k) = FIELDS.iter().position(|f| *f == field) else {
            continue;
        };
        truth.insert(
            (
                l["frame"].as_str().unwrap().to_string(),
                l["team"].as_u64().unwrap() as usize - 1,
                l["team_row"].as_u64().unwrap() as usize,
                k,
            ),
            l["value"].as_u64().unwrap(),
        );
    }

    let mut paths: Vec<_> = std::fs::read_dir(&args[0])
        .expect("read frames dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    paths.sort();

    let (mut right, mut total, mut flagged, mut flagged_right, mut wrong_unflagged) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut ms: Vec<f64> = Vec::new();
    let mut errors = Vec::new();
    let mut team_mismatch = 0;
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let img = image::open(path).expect("open frame");
        let img = image::DynamicImage::ImageRgb8(img.to_rgb8());
        let board = if img.width() as f64 / img.height() as f64 > 1.72 {
            ocr::preprocess::crop_scoreboard(&img)
        } else {
            img
        };
        let detected = detect::hero_portrait::detect_team_size(&board);
        let team = force_team.unwrap_or(detected);
        let t0 = Instant::now();
        let read = read_board(&board, team, Duration::from_secs(5));
        let wall = t0.elapsed().as_secs_f64() * 1000.0;
        let read = match read {
            Ok(r) => r,
            Err(e) => {
                println!("{name}: ERROR {e}");
                continue;
            }
        };
        ms.push(wall);
        let label_rows = (0..12)
            .filter(|r| truth.contains_key(&(name.clone(), r / 6, r % 6, 0)))
            .count();
        if label_rows != 2 * team {
            team_mismatch += 1;
        }
        let (mut fr, mut ft, mut ff) = (0, 0, 0);
        for (r, row) in read.rows.iter().enumerate() {
            let (t, tr) = (r / team, r % team);
            for (k, cell) in row.cells.iter().enumerate() {
                let Some(&want) = truth.get(&(name.clone(), t, tr, k)) else {
                    continue;
                };
                let ok = cell.value.map(u64::from) == Some(want);
                ft += 1;
                fr += ok as usize;
                ff += cell.suspect as usize;
                flagged_right += (cell.suspect && ok) as usize;
                wrong_unflagged += (!cell.suspect && !ok) as usize;
                if !ok {
                    errors.push(format!(
                        "  {name} row {r} {}: want {want} got {:?} conf {:.3} suspect {}",
                        FIELDS[k], cell.value, cell.confidence, cell.suspect
                    ));
                }
            }
        }
        println!(
            "{name}: team {team} (detected {detected}) {fr}/{ft} right, {ff} flagged, {wall:.1} ms"
        );
        right += fr;
        total += ft;
        flagged += ff;
    }
    for e in &errors {
        println!("{e}");
    }
    ms.sort_by(|a, b| a.total_cmp(b));
    let median = ms.get(ms.len() / 2).copied().unwrap_or(0.0);
    let mean = ms.iter().sum::<f64>() / ms.len().max(1) as f64;
    println!(
        "TOTAL {right}/{total} = {:.2}% right; flagged {flagged} ({flagged_right} of them right); \
         wrong+unflagged {wrong_unflagged}; team-size mismatches {team_mismatch}; \
         ms/frame median {median:.1} mean {mean:.1} over {} frames",
        100.0 * right as f64 / total.max(1) as f64,
        ms.len()
    );
}
