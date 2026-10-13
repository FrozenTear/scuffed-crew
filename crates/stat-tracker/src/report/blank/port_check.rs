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
///
/// The blanker runs as it ships; `SCUFFED_QA_PORTRAITS=on` runs the
/// portrait gates as with `COPY_PORTRAITS` on. Portraits need an earlier
/// capture (`SCUFFED_QA_MEMORY`):
/// * unset: none, so no portrait is kept;
/// * `seq`: the files are captures of one member in time order (the
///   `YYYYMMDD_HHMMSS` in their names) and share one memory;
/// * `self`: each file is checked against itself captured two minutes earlier
///   (an overlay that stayed put);
/// * any other value: a JSON file mapping a file name to the image of the
///   earlier capture (two minutes earlier).
#[test]
#[ignore = "needs SCUFFED_QA_FRAMES and SCUFFED_QA_OUT"]
fn dump_blank_verdicts() {
    use chrono::{DateTime, Duration, NaiveDateTime, Utc};
    let dir = std::env::var("SCUFFED_QA_FRAMES").expect("SCUFFED_QA_FRAMES");
    let out = std::env::var("SCUFFED_QA_OUT").expect("SCUFFED_QA_OUT");
    let images = std::env::var("SCUFFED_QA_IMAGES").ok();
    let mode = std::env::var("SCUFFED_QA_MEMORY").unwrap_or_default();
    let refs: serde_json::Map<String, serde_json::Value> = match mode.as_str() {
        "" | "seq" | "self" => Default::default(),
        path => serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap(),
    };
    let stamp = |name: &str| -> Option<DateTime<Utc>> {
        let b = name.as_bytes();
        (0..(b.len() + 1).saturating_sub(15)).find_map(|i| {
            let t = std::str::from_utf8(&b[i..i + 15]).ok()?;
            NaiveDateTime::parse_from_str(t, "%Y%m%d_%H%M%S")
                .ok()
                .map(|n| n.and_utc())
        })
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            n.ends_with(".png") && !n.ends_with("_digits.png") && !n.ends_with("_secret.png")
        })
        .collect();
    files.sort();
    if mode == "seq" {
        files.sort_by_key(|p| stamp(&p.file_name().unwrap().to_string_lossy()));
    }
    // SCUFFED_QA_CHAT=unknown: blank as a crop with no frame (chat
    // could cover the bottom rows)
    let chat = match std::env::var("SCUFFED_QA_CHAT").as_deref() {
        Ok("unknown") => super::Chat::Unknown,
        _ => super::Chat::None,
    };
    // a 16:9 (or wider) full frame goes through the frame path, with chat
    // read from the frame
    // SCUFFED_QA_PORTRAITS=on: the portrait gates as they run with
    // `COPY_PORTRAITS` on; otherwise the blanker as it ships
    let gated = std::env::var("SCUFFED_QA_PORTRAITS").as_deref() == Ok("on");
    let run = |img: &image::RgbImage, mem: &mut super::PortraitMemory, at: DateTime<Utc>| {
        let at = super::CaptureTime::wall(at);
        let ratio = f64::from(img.width()) / f64::from(img.height().max(1));
        let frame = || image::DynamicImage::ImageRgb8(img.clone());
        match (ratio > 1.7, gated) {
            (true, true) => super::blanked_scoreboard_gated(&frame(), Some(6), mem, at),
            (true, false) => super::blanked_scoreboard_seen(&frame(), Some(6), mem, at),
            (false, true) => super::blank_crop_gated(img, Some(6), chat, mem, at),
            (false, false) => super::blank_crop_seen(img, Some(6), chat, mem, at),
        }
    };
    let t0 = DateTime::<Utc>::UNIX_EPOCH + Duration::days(20_000);
    let mut shared = super::PortraitMemory::replay();
    let mut lines = String::new();
    for (i, p) in files.into_iter().enumerate() {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let img = image::open(&p).unwrap().to_rgb8();
        let at = match mode.as_str() {
            "seq" => stamp(&name).unwrap_or(t0 + Duration::minutes(2 * i as i64)),
            _ => t0,
        };
        let mut own = super::PortraitMemory::replay();
        let mem = if mode == "seq" { &mut shared } else { &mut own };
        if mode == "self" {
            // the same board a live frame earlier: one crop pixel differs
            // (a byte-identical crop is a frozen render, never a reference)
            let mut e = img.clone();
            let ratio = f64::from(img.width()) / f64::from(img.height().max(1));
            let (x, y) = if ratio > 1.7 {
                let (cx, cy, cw, _) =
                    crate::ocr::preprocess::crop_scoreboard_rect(img.width(), img.height());
                (cx + cw - 1, cy + 2)
            } else {
                (img.width() - 1, 2)
            };
            let p = e.get_pixel(x, y).0;
            e.put_pixel(x, y, image::Rgb(p.map(|v| v ^ 0x10)));
            run(&e, mem, at - Duration::minutes(2));
        } else if let Some(r) = refs.get(&name).and_then(|v| v.as_str()) {
            run(
                &image::open(r).unwrap().to_rgb8(),
                mem,
                at - Duration::minutes(2),
            );
        }
        let t = std::time::Instant::now();
        let b = run(&img, mem, at);
        let ms = t.elapsed().as_millis();
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
