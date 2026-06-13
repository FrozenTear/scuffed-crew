use image::DynamicImage;

use super::GamePhase;

pub fn detect_phase(img: &DynamicImage) -> GamePhase {
    if let Some(phase) = detect_map_vote(img) {
        return phase;
    }
    if detect_hero_ban(img) {
        return GamePhase::HeroBan;
    }
    if detect_hero_select(img) {
        return GamePhase::HeroSelect;
    }
    GamePhase::Unknown
}

fn detect_map_vote(img: &DynamicImage) -> Option<GamePhase> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();

    // Map vote screen in OW2 has a dark blue/navy background with slight gradient.
    // The center third of the screen contains 3 map preview cards.
    // Top area often shows "MAP VOTE" or timer text.
    // Characteristic: dark navy pixels (R<60, G<60, B>80) dominate the background.
    let mut navy_count = 0u32;
    let mut total = 0u32;

    // Sample the top quarter and side margins (avoiding the map cards in center)
    for y in 0..(h / 4) {
        for x in 0..w {
            let pixel = rgb.get_pixel(x, y);
            let [r, g, b] = pixel.0;
            total += 1;
            if r < 70 && g < 70 && b > 70 && (b as i32 - r as i32) > 30 {
                navy_count += 1;
            }
        }
    }

    if total == 0 {
        return None;
    }

    let navy_ratio = navy_count as f32 / total as f32;
    if navy_ratio < 0.40 {
        return None;
    }

    // Confirm with OCR on the top portion looking for "VOTE" or "MAP"
    let top_region = img.crop_imm(w / 4, 0, w / 2, h / 6);
    match crate::ocr::recognize_region(&top_region) {
        Ok(text) => {
            let upper = text.to_uppercase();
            if upper.contains("VOTE") || upper.contains("MAP") {
                let maps = extract_map_names(&upper);
                tracing::info!(navy_ratio, maps = ?maps, "map vote screen detected");
                Some(GamePhase::MapVote { maps })
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

fn detect_hero_ban(img: &DynamicImage) -> bool {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();

    // Hero ban screen has a distinctive red/orange tint in the header area
    // and shows "BAN" text. The background is darker than normal gameplay.
    let header_h = h / 6;
    let mut red_accent = 0u32;
    let mut dark_count = 0u32;
    let mut total = 0u32;

    for y in 0..header_h {
        for x in 0..w {
            let pixel = rgb.get_pixel(x, y);
            let [r, g, b] = pixel.0;
            total += 1;
            if r > 150 && g < 80 && b < 80 {
                red_accent += 1;
            }
            if r < 50 && g < 50 && b < 70 {
                dark_count += 1;
            }
        }
    }

    if total == 0 {
        return false;
    }

    let red_ratio = red_accent as f32 / total as f32;
    let dark_ratio = dark_count as f32 / total as f32;

    // Need significant red accent (ban UI) combined with dark background
    if red_ratio < 0.05 || dark_ratio < 0.30 {
        return false;
    }

    // Confirm with OCR
    let top_region = img.crop_imm(w / 4, 0, w / 2, header_h);
    match crate::ocr::recognize_region(&top_region) {
        Ok(text) => {
            let upper = text.to_uppercase();
            if upper.contains("BAN") {
                tracing::info!(red_ratio, "hero ban screen detected");
                true
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

fn detect_hero_select(img: &DynamicImage) -> bool {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();

    // Hero select screen characteristics:
    // - Top portion shows "CHOOSE YOUR HERO" or "ASSEMBLE YOUR TEAM"
    // - Has a bright, colorful hero grid in the lower 2/3
    // - Top banner area is relatively dark with text
    // - Bottom area has high color variance from hero portraits

    let header_h = h / 8;
    let mut dark_header = 0u32;
    let mut header_total = 0u32;

    for y in 0..header_h {
        for x in 0..w {
            let pixel = rgb.get_pixel(x, y);
            let [r, g, b] = pixel.0;
            header_total += 1;
            if r < 80 && g < 80 && b < 100 {
                dark_header += 1;
            }
        }
    }

    if header_total == 0 {
        return false;
    }

    let dark_ratio = dark_header as f32 / header_total as f32;
    if dark_ratio < 0.50 {
        return false;
    }

    // Check for high color variance in the bottom half (hero grid)
    let bottom_start = h / 2;
    let step_x = w / 50;
    let step_y = (h - bottom_start) / 20;
    let mut colors: Vec<[u8; 3]> = Vec::new();

    for sy in 0..20 {
        for sx in 0..50 {
            let x = sx * step_x;
            let y = bottom_start + sy * step_y;
            if x < w && y < h {
                let pixel = rgb.get_pixel(x, y);
                colors.push(pixel.0);
            }
        }
    }

    if colors.len() < 100 {
        return false;
    }

    // Calculate color variance — hero grid should be colorful
    let avg_r = colors.iter().map(|c| c[0] as f64).sum::<f64>() / colors.len() as f64;
    let avg_g = colors.iter().map(|c| c[1] as f64).sum::<f64>() / colors.len() as f64;
    let avg_b = colors.iter().map(|c| c[2] as f64).sum::<f64>() / colors.len() as f64;

    let variance = colors
        .iter()
        .map(|c| {
            let dr = c[0] as f64 - avg_r;
            let dg = c[1] as f64 - avg_g;
            let db = c[2] as f64 - avg_b;
            dr * dr + dg * dg + db * db
        })
        .sum::<f64>()
        / colors.len() as f64;

    if variance < 2000.0 {
        return false;
    }

    // Confirm with OCR on header
    let top_region = img.crop_imm(w / 4, 0, w / 2, header_h);
    match crate::ocr::recognize_region(&top_region) {
        Ok(text) => {
            let upper = text.to_uppercase();
            let is_hero_select =
                upper.contains("CHOOSE") || upper.contains("HERO") || upper.contains("ASSEMBLE");
            if is_hero_select {
                tracing::info!(variance, "hero select screen detected");
            }
            is_hero_select
        }
        Err(_) => false,
    }
}

const MAP_NAMES: &[&str] = &[
    "CIRCUIT ROYAL",
    "DORADO",
    "HAVANA",
    "JUNKERTOWN",
    "RIALTO",
    "ROUTE 66",
    "SHAMBALI",
    "WATCHPOINT",
    "GIBRALTAR",
    "BLIZZARD WORLD",
    "EICHENWALDE",
    "HOLLYWOOD",
    // Not bare "KING"/"ROW" — they substring-match unrelated text
    // ("WRECKING", "BROWN"); apostrophe loss in OCR is covered by both forms.
    "KING'S ROW",
    "KINGS ROW",
    "MIDTOWN",
    "NUMBANI",
    "PARAISO",
    "ANTARCTIC",
    "BUSAN",
    "ILIOS",
    "LIJIANG",
    "NEPAL",
    "OASIS",
    "SAMOA",
    "COLOSSEO",
    "ESPERANCA",
    "NEW QUEEN",
    "RUNASAPI",
    "NEW JUNK",
    "SURAVASA",
    "HANAOKA",
    "THRONE",
    "ANUBIS",
    "AATLIS",
];

fn extract_map_names(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for &name in MAP_NAMES {
        if text.contains(name) {
            found.push(name.to_string());
        }
    }
    found
}
