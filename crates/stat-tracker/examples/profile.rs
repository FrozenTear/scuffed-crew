//! Geometry measurement tool: dump the scoreboard's vertical row-band structure
//! and header column positions, with NO OCR. Resolution-independent — used to
//! design row/team-size and column detection.
//!
//! Usage: cargo run -p scuffed-stat-tracker --example profile -- <image.png>

use image::DynamicImage;
use stat_tracker::{detect, ocr};

fn saturation(r: u8, g: u8, b: u8) -> f64 {
    let max = r.max(g).max(b) as f64;
    let min = r.min(g).min(b) as f64;
    if max == 0.0 { 0.0 } else { (max - min) / max }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = std::env::args().nth(1).ok_or("usage: profile <image.png>")?;
    let img = image::open(&path)?;
    let board: DynamicImage = ocr::preprocess::crop_scoreboard(&img);
    let rgb = board.to_rgb8();
    let (w, h) = rgb.dimensions();
    println!("scoreboard crop: {w}x{h}");

    // --- Vertical band profile: mean saturation over the name strip (x 6%..55%,
    // avoiding the right-side career panel), per scanline. Colored team rows are
    // saturated; header / VS gap / background are not.
    let x0 = (w as f64 * 0.06) as u32;
    let x1 = (w as f64 * 0.55) as u32;
    let mut sat_rows: Vec<f64> = Vec::with_capacity(h as usize);
    for y in 0..h {
        let mut sum = 0.0;
        for x in x0..x1 {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            sum += saturation(r, g, b);
        }
        sat_rows.push(sum / (x1 - x0) as f64);
    }

    // Threshold into "colored row" vs not, then list contiguous bands.
    let thresh = 0.18;
    let mut bands: Vec<(u32, u32)> = Vec::new();
    let mut start: Option<u32> = None;
    for (y, &s) in sat_rows.iter().enumerate() {
        if s >= thresh {
            if start.is_none() { start = Some(y as u32); }
        } else if let Some(st) = start.take() {
            if y as u32 - st >= h / 50 { bands.push((st, y as u32)); }
        }
    }
    if let Some(st) = start { bands.push((st, h)); }

    println!("\ncolored bands (y px / y% / height%):");
    for (s, e) in &bands {
        println!(
            "  {s:>4}..{e:<4}  {:>5.1}%..{:<5.1}%  h={:.1}%",
            *s as f64 / h as f64 * 100.0,
            *e as f64 / h as f64 * 100.0,
            (*e - *s) as f64 / h as f64 * 100.0
        );
    }

    // Compact ASCII saturation profile, sampled every 1.5% of height.
    println!("\nvertical saturation profile (each line ~1.5% height):");
    let step = (h / 66).max(1);
    let mut y = 0;
    while y < h {
        let s = sat_rows[y as usize];
        let bar = "#".repeat((s * 40.0) as usize);
        println!("  y={:>5.1}% {:.2} {bar}", y as f64 / h as f64 * 100.0, s);
        y += step;
    }

    // --- Header column centers via existing dark-text detector for reference.
    let off = ocr::preprocess::detect_column_offset(&board);
    println!("\ndetect_column_offset returned: {off:.4} ({} px)", (off * w as f64) as i32);

    // --- Player-row brightness: mean at center thin slice, multiple x zones ---
    let team_size = detect::hero_portrait::detect_team_size(&board);
    {
        let rgb = board.to_rgb8();
        let (bw, bh) = rgb.dimensions();
        let row_height = match team_size { 6 => bh * 58 / 1000, _ => bh * 7 / 100 };
        let start_y = bh * 12 / 100;
        // Probe multiple x windows to find where the signal lives
        let zones: &[(&str, u32, u32)] = &[
            ("10-30%", bw*10/100, bw*30/100),
            ("34-50%", bw*34/100, bw*50/100),
            ("10-50%", bw*10/100, bw*50/100),
        ];
        // Thin-strip (center ±15% row_h) + brightness-filtered [15..90] at 34-50%.
        // Thin strip avoids separator borders; filter clips bright icons/title-text.
        let (x0_f, x1_f) = (bw * 34 / 100, (bw * 50 / 100).min(bw));
        println!("\nteam1 row THIN-STRIP + FILTERED [15..90] mean (34-50%) [center ±15% row_h]:");
        for row in 0..team_size as u32 {
            let center = start_y + row * row_height + row_height / 2;
            let half = (row_height * 15 / 100).max(4);
            let y0 = center.saturating_sub(half); let y1 = (center + half).min(bh);
            let mut total = 0u64; let mut count = 0u64; let mut all_count = 0u64;
            for y in y0..y1 { for x in x0_f..x1_f {
                let [r,g,b] = rgb.get_pixel(x,y).0;
                let br = (r as u32 + g as u32 + b as u32) / 3;
                all_count += 1;
                if br >= 15 && br <= 90 { total += br as u64; count += 1; }
            } }
            let fm = if count > 0 { total as f64 / count as f64 } else { 0.0 };
            println!("  row {row}: filtered_mean={fm:.1} kept={}/{all_count}", count);
        }
        for &(label, x0, x1) in zones {
            println!("\nteam1 row brightness {label} [center ±15% row_h]:");
            for row in 0..team_size as u32 {
                let center = start_y + row * row_height + row_height / 2;
                let half = (row_height * 15 / 100).max(4);
                let y0 = center.saturating_sub(half);
                let y1 = (center + half).min(bh);
                let mut total = 0u64; let mut count = 0u64;
                for y in y0..y1 { for x in x0..x1 {
                    let [r,g,b] = rgb.get_pixel(x,y).0;
                    total += (r as u64 + g as u64 + b as u64) / 3; count += 1;
                } }
                let mean = if count > 0 { total as f64 / count as f64 } else { 0.0 };
                println!("  row {row}: mean={mean:.1}");
            }
        }
    }

    // --- Prototype: row-pitch based team-size detection ---
    // Smooth the saturation profile, then find row-center dips (white name text
    // lowers saturation once per row) within team 1, and classify by median pitch.
    let win = (h as f64 * 0.01).max(2.0) as usize;
    let smooth: Vec<f64> = (0..sat_rows.len())
        .map(|i| {
            let lo = i.saturating_sub(win);
            let hi = (i + win + 1).min(sat_rows.len());
            sat_rows[lo..hi].iter().sum::<f64>() / (hi - lo) as f64
        })
        .collect();
    let y_lo = (h as f64 * 0.04) as usize;
    let y_hi = (h as f64 * 0.45) as usize;
    let sep = (h as f64 * 0.035) as usize; // min spacing between row centers
    let mut dips: Vec<usize> = Vec::new();
    let mut y = y_lo + 1;
    while y < y_hi - 1 {
        // local minimum over +-sep window
        let lo = y.saturating_sub(sep);
        let hi = (y + sep).min(smooth.len() - 1);
        let local_min = (lo..=hi).all(|j| smooth[y] <= smooth[j]);
        let region_max = (lo..=hi).map(|j| smooth[j]).fold(0.0_f64, f64::max);
        if local_min && region_max - smooth[y] > 0.04 {
            if dips.last().is_none_or(|&d| y - d >= sep) {
                dips.push(y);
            }
        }
        y += 1;
    }
    let pitches: Vec<f64> = dips.windows(2).map(|w| (w[1] - w[0]) as f64).collect();
    let median_pitch = if pitches.is_empty() {
        0.0
    } else {
        let mut p = pitches.clone();
        p.sort_by(|a, b| a.partial_cmp(b).unwrap());
        p[p.len() / 2] / h as f64
    };
    let team_size = if median_pitch > 0.0 && median_pitch < 0.078 { 6 } else { 5 };
    println!(
        "\nrow dips (y%): {:?}",
        dips.iter().map(|&d| (d as f64 / h as f64 * 100.0).round() / 1.0).collect::<Vec<_>>()
    );
    println!("median pitch: {:.2}% -> team_size = {team_size}", median_pitch * 100.0);

    Ok(())
}
