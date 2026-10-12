//! Port check against the Python prototypes: dumps every measurement of the
//! blanker on a directory of scoreboard crops, one JSON line per board.
//! Ignored; run with `SCUFFED_QA_FRAMES=<dir> SCUFFED_QA_OUT=<file>`. The
//! frames are member captures and stay on the member's machine.

use serde_json::json;

use super::board;
use super::portrait;

#[test]
#[ignore = "needs SCUFFED_QA_FRAMES and SCUFFED_QA_OUT"]
fn dump_board_measurements() {
    let dir = std::env::var("SCUFFED_QA_FRAMES").expect("SCUFFED_QA_FRAMES");
    let out = std::env::var("SCUFFED_QA_OUT").expect("SCUFFED_QA_OUT");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            n.ends_with(".png") && !n.ends_with("_digits.png") && !n.ends_with("_secret.png")
        })
        .collect();
    files.sort();
    let mut lines = String::new();
    for p in files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let img = image::open(&p).unwrap().to_rgb8();
        let rec = match board::measure(&img, 6) {
            Err(e) => json!({"file": name, "fail": e.0}),
            Ok(b) => {
                let bars: Vec<_> = b
                    .rows
                    .iter()
                    .map(|r| r.verified.then(|| portrait::bar_colour(&img, b.l, r, b.s)))
                    .collect();
                let with_bar: Vec<&board::Row> = b
                    .rows
                    .iter()
                    .zip(&bars)
                    .filter(|(_, c)| matches!(c, Some(Ok(_))))
                    .map(|(r, _)| r)
                    .collect();
                let pe = portrait::portrait_edge(&img, b.l, &with_bar, b.s);
                let sal_x0 = b.l.saturating_sub(3);
                let sal_x1 =
                    (b.l as i64 + board::pyround(36.0 * b.s)).min(img.width() as i64) as u32;
                let sal = portrait::salient_map(&img, sal_x0, sal_x1);
                let sal_w = (sal_x1 - sal_x0) as usize;
                let rows: Vec<_> = b
                    .rows
                    .iter()
                    .zip(&bars)
                    .map(|(r, bar)| {
                        let (bar_v, why) = match bar {
                            None => (None, Some("row not verified".to_string())),
                            Some(Ok(c)) => (Some(c.map(|v| v as i64)), None),
                            Some(Err(e)) => (None, Some(e.to_string())),
                        };
                        let portrait = match (bar, &pe) {
                            (Some(Ok(_)), Ok(_)) => Some(
                                match portrait::verdict(&img, &sal, sal_x0, sal_w, b.l, r, b.s) {
                                    Ok(()) => "ok".to_string(),
                                    Err(e) => e.to_string(),
                                },
                            ),
                            _ => None,
                        };
                        json!({"team": r.team, "idx": r.idx, "top": r.top, "bot": r.bot,
                               "verified": r.verified, "bar": bar_v, "why": why, "portrait": portrait})
                    })
                    .collect();
                json!({"file": name, "L": b.l, "R": b.r, "hb": b.hb,
                       "pe": pe.as_ref().ok(), "pe_fail": pe.as_ref().err().map(|e| e.0.clone()),
                       "rows": rows})
            }
        };
        lines.push_str(&rec.to_string());
        lines.push('\n');
    }
    std::fs::write(out, lines).unwrap();
}

/// Row verdicts of the whole blanker (no chat on screen), one JSON line per
/// board: `SCUFFED_QA_FRAMES=<dir> SCUFFED_QA_OUT=<file>`; with
/// `SCUFFED_QA_IMAGES=<dir>` the blanked crops are written there too.
#[test]
#[ignore = "needs SCUFFED_QA_FRAMES and SCUFFED_QA_OUT"]
fn dump_blank_verdicts() {
    let dir = std::env::var("SCUFFED_QA_FRAMES").expect("SCUFFED_QA_FRAMES");
    let out = std::env::var("SCUFFED_QA_OUT").expect("SCUFFED_QA_OUT");
    let images = std::env::var("SCUFFED_QA_IMAGES").ok();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            n.ends_with(".png") && !n.ends_with("_digits.png") && !n.ends_with("_secret.png")
        })
        .collect();
    files.sort();
    let mut lines = String::new();
    for p in files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let img = image::open(&p).unwrap().to_rgb8();
        let t0 = std::time::Instant::now();
        // SCUFFED_QA_CHAT=unknown: blank as a crop with no frame (chat
        // could cover the bottom rows)
        let chat = match std::env::var("SCUFFED_QA_CHAT").as_deref() {
            Ok("unknown") => super::Chat::Unknown,
            _ => super::Chat::None,
        };
        // a 16:9 (or wider) full frame goes through the frame path, with
        // chat read from the frame
        let ratio = f64::from(img.width()) / f64::from(img.height().max(1));
        let b = if ratio > 1.7 {
            super::blanked_scoreboard(&image::DynamicImage::ImageRgb8(img.clone()), Some(6))
        } else {
            super::blank_crop(&img, Some(6), chat)
        };
        let ms = t0.elapsed().as_millis();
        if let Some(d) = &images {
            b.image.save(std::path::Path::new(d).join(&name)).unwrap();
        }
        let rows: Vec<_> = b
            .rows
            .iter()
            .map(|r| {
                json!({"team": r.team, "idx": r.idx,
                       "digits": r.digits.as_ref().err(), "portrait": r.portrait.as_ref().err()})
            })
            .collect();
        let rec = json!({"file": name, "board": b.board.as_ref().err(), "ms": ms, "rows": rows});
        lines.push_str(&rec.to_string());
        lines.push('\n');
    }
    std::fs::write(out, lines).unwrap();
}

/// Per-piece reader statistics for calibration, one JSON line per cell:
/// `SCUFFED_QA_FRAMES=<dir> SCUFFED_QA_OUT=<file>`.
#[test]
#[ignore = "needs SCUFFED_QA_FRAMES and SCUFFED_QA_OUT"]
fn dump_piece_stats() {
    let dir = std::env::var("SCUFFED_QA_FRAMES").expect("SCUFFED_QA_FRAMES");
    let out = std::env::var("SCUFFED_QA_OUT").expect("SCUFFED_QA_OUT");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            n.ends_with(".png") && !n.ends_with("_digits.png") && !n.ends_with("_secret.png")
        })
        .collect();
    files.sort();
    let mut lines = String::new();
    for p in files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let img = image::open(&p).unwrap().to_rgb8();
        let Ok(b) = board::measure(&img, 6) else {
            continue;
        };
        for mut rec in super::cells::piece_stats(&img, &b) {
            rec["file"] = json!(name);
            lines.push_str(&rec.to_string());
            lines.push('\n');
        }
    }
    std::fs::write(out, lines).unwrap();
}

/// Rim candidates of accepted cells, one line per board:
/// `SCUFFED_QA_FRAMES=<dir> SCUFFED_QA_OUT=<file>`.
#[test]
#[ignore = "needs SCUFFED_QA_FRAMES and SCUFFED_QA_OUT"]
fn dump_rim_stats() {
    let dir = std::env::var("SCUFFED_QA_FRAMES").expect("SCUFFED_QA_FRAMES");
    let out = std::env::var("SCUFFED_QA_OUT").expect("SCUFFED_QA_OUT");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            n.ends_with(".png") && !n.ends_with("_digits.png") && !n.ends_with("_secret.png")
        })
        .collect();
    files.sort();
    let mut lines = String::new();
    for p in files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let img = image::open(&p).unwrap().to_rgb8();
        let Ok(b) = board::measure(&img, 6) else {
            continue;
        };
        let rim: Vec<_> = super::cells::rim_stats(&img, &b)
            .into_iter()
            .map(|(x, y, t, off, kept)| {
                json!([x, y, t, if off.is_finite() { off } else { 999.0 }, kept])
            })
            .collect();
        lines.push_str(&json!({"file": name, "rim": rim}).to_string());
        lines.push('\n');
    }
    std::fs::write(out, lines).unwrap();
}
