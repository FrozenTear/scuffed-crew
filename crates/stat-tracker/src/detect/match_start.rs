//! Match-start phase screens: map vote, hero ban, hero select (OCR site P7).
//!
//! Each screen has a pixel gate (no OCR) and then a title check:
//!
//! | phase | pixel gate | ocr-v1 title check (Tesseract, top-centre h/4 crop) | `reader = "new"` |
//! |---|---|---|---|
//! | [`GamePhase::MapVote`] | navy ≥ 0.40 in the top quarter | `VOTE` and `MAP`; map names from the same text | words-v1 `vote_for_a_map` or `selecting_map` |
//! | [`GamePhase::HeroBan`] | red ≥ 0.003 and dark ≥ 0.30 in the top sixth | `BAN HERO`, `TEAM BAN SCORE` or `VOTE TO BAN` | words-v1 `ban_heroes` |
//! | [`GamePhase::HeroSelect`] | dark ≥ 0.50 in the top eighth and grid variance ≥ 2000 | `ASSEMBLE`, `PREFERRED HERO`, or `SELECT` and `HERO` | words-v1 `select_a_preferred_hero` |
//!
//! The order is vote, ban, select; the first confirmed detector wins. Under
//! `reader = "new"` ([`PhaseSource::Words`]) a detector with a loaded mask
//! is confirmed by its masks alone ([`crate::shadow::result::phrase`]): a
//! sure read of one of them is the phase, a suspect or absent read is not,
//! and no OCR runs. The map vote then names no maps.
//!
//! The unit is the detector, not the title. A detector with none of its
//! masks loaded keeps the OCR check, with all of its OCR phrases. A
//! detector with a mask loaded reads only the titles it has masks for: the
//! other titles in its OCR column are then not read at all (with words-v1:
//! `TEAM BAN SCORE`, `VOTE TO BAN`, `ASSEMBLE`; and `SELECTING MAP` on a
//! version 1 pack, which has no `selecting_map` mask). The pixel gates, the
//! stability deferral and the order are the same for both readers.

use image::{DynamicImage, RgbImage};

use super::GamePhase;
use super::stability::FrameStability;
use crate::shadow::result::phrase::{Phrase, PhraseRead};

/// Detect map-vote / hero-ban / hero-select phase (ocr-v1).
///
/// Converts the frame to RGB once; detectors share that buffer (P6).
pub fn detect_phase(img: &DynamicImage) -> GamePhase {
    let rgb = img.to_rgb8();
    detect_phase_with_rgb(img, &rgb)
}

/// Same as [`detect_phase`] when the caller already holds an RGB conversion
/// (e.g. poll tick that also runs the outcome detectors on the same frame).
pub fn detect_phase_with_rgb(img: &DynamicImage, rgb: &RgbImage) -> GamePhase {
    if let Some(phase) = detect_map_vote(img, rgb) {
        return phase;
    }
    if detect_hero_ban(img, rgb) {
        return GamePhase::HeroBan;
    }
    if detect_hero_select(img, rgb) {
        return GamePhase::HeroSelect;
    }
    GamePhase::Unknown
}

/// Who confirms a phase screen once its pixel gate passed (P7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhaseSource {
    /// Tesseract over the top-centre crop (`reader = "ocr-v1"`).
    #[default]
    Ocr,
    /// The words-v1 phrase masks (`reader = "new"`). Only a sure read is a
    /// signal; a suspect or absent read is not, and no OCR runs. A detector
    /// with none of its masks loaded keeps the OCR check.
    Words,
}

impl PhaseSource {
    /// The source for this reader setting (`reader = "new"` or not).
    pub fn for_reader(use_new_reader: bool) -> Self {
        if use_new_reader {
            PhaseSource::Words
        } else {
            PhaseSource::Ocr
        }
    }
}

/// Poll-tick phase detection (PR-A). Two differences from the one-shot path,
/// neither changing what a phase screen ultimately detects as:
///
/// 1. All three detectors confirm with OCR of the *identical* top-center crop
///    — the one-shot path can run that same Tesseract call up to three times
///    on a tick where several pixel gates pass (dark combat scenes routinely
///    pass the ban/select gates). Here the crop is recognized once and each
///    gate-passing detector checks its phrases against the shared text.
/// 2. The confirm is gated on temporal stability ([`FrameStability`]): phase
///    screens hold still for 10s+, combat frames never do, so the confirm
///    stops burning a Tesseract call on every dark mid-fight tick. First
///    sighting of a screen defers one tick — well inside every phase screen's
///    lifetime.
///
/// Pixel gates, thresholds, phrase checks, and priority order are shared with
/// the one-shot detectors — fixture parity holds by construction. `source`
/// says who confirms ([`PhaseSource`]); under [`PhaseSource::Words`] the
/// crop is recognized only for a gate-passing detector without a mask.
pub fn detect_phase_polled(
    img: &DynamicImage,
    rgb: &RgbImage,
    stability: &mut FrameStability,
    source: PhaseSource,
) -> GamePhase {
    detect_phase_polled_with(
        img,
        rgb,
        stability,
        source,
        crate::shadow::read_phrase,
        // A screen without a mask keeps the OCR check, while OCR is on.
        // With OCR off there is no check to fall back to: that screen is
        // not confirmed, and the read is not asked for. The missing mask is
        // what the switch reports (`ocr_switch::PackGroup::PhaseTitles`).
        |crop: &DynamicImage| {
            if crate::ocr::gate::is_off() {
                None
            } else {
                crate::ocr::recognize_sparse_region(crop).ok()
            }
        },
    )
}

/// [`detect_phase_polled`] with the phrase read and the OCR as arguments
/// (tests). `words` is called only under [`PhaseSource::Words`], for a
/// gate-passing screen; `None` from it (no mask) leaves that screen to the
/// OCR. `ocr` runs at most once per tick, and only when a gate-passing
/// screen needs it.
fn detect_phase_polled_with(
    img: &DynamicImage,
    rgb: &RgbImage,
    stability: &mut FrameStability,
    source: PhaseSource,
    words: impl Fn(Phrase, &RgbImage) -> Option<PhraseRead>,
    ocr: impl FnOnce(&DynamicImage) -> Option<String>,
) -> GamePhase {
    let (w, h) = rgb.dimensions();

    let navy = navy_ratio(rgb);
    let vote_gate = navy >= 0.40;
    let (red_ratio, dark_ratio) = ban_ratios(rgb);
    let ban_gate = red_ratio >= BAN_RED_MIN && dark_ratio >= BAN_DARK_MIN;
    let select_dark = select_header_dark_ratio(rgb);
    // The grid-variance scan only matters when the header gate passed.
    let select_variance = if select_dark >= 0.50 {
        select_grid_variance(rgb)
    } else {
        0.0
    };
    let select_gate = select_dark >= 0.50 && select_variance >= 2000.0;

    if !vote_gate && !ban_gate && !select_gate {
        return GamePhase::Unknown;
    }

    let top_region = img.crop_imm(w / 4, 0, w / 2, h / 4);
    if !stability.check("phase_top", &top_region) {
        tracing::trace!(
            vote_gate,
            ban_gate,
            select_gate,
            "phase pixel gate passed but top region not stable: deferring confirm"
        );
        return GamePhase::Unknown;
    }

    // The words-v1 read of one gate-passing screen: `Some(confirmed)` when
    // its mask is loaded, `None` to leave it to the OCR.
    let read = |phrase: Phrase| {
        if source != PhaseSource::Words {
            return None;
        }
        let read = words(phrase, rgb)?;
        tracing::debug!(phrase = phrase.id(), ?read, "phase title (words-v1)");
        Some(read.is_signal())
    };
    // The crop's OCR text, recognized on first use only.
    let mut ocr = Some(ocr);
    let mut text: Option<Option<String>> = None;
    let mut upper = || -> Option<String> {
        text.get_or_insert_with(|| {
            ocr.take()
                .and_then(|f| f(&top_region))
                .map(|t| t.to_uppercase())
        })
        .clone()
    };

    if vote_gate {
        // Two titles of one screen: VOTE FOR A MAP while the vote is open,
        // SELECTING MAP once the map is drawn. A sure read of either is the
        // vote. The second is read only when the first is not sure, and the
        // OCR only when neither mask is loaded (a version 1 pack has the
        // first alone, and then decides alone).
        let vote = read(Phrase::VoteForAMap);
        let selecting = if vote == Some(true) {
            None
        } else {
            read(Phrase::SelectingMap)
        };
        match (vote, selecting) {
            (Some(true), _) | (_, Some(true)) => {
                tracing::info!(navy_ratio = navy, "map vote screen detected (words-v1)");
                return GamePhase::MapVote { maps: Vec::new() };
            }
            (Some(false), _) | (_, Some(false)) => {}
            (None, None) => {
                if let Some(phase) = upper().and_then(|t| confirm_map_vote(&t, navy)) {
                    return phase;
                }
            }
        }
    }
    if ban_gate {
        let confirmed = match read(Phrase::BanHeroes) {
            Some(sure) => {
                if sure {
                    tracing::info!(red_ratio, "hero ban screen detected (words-v1)");
                }
                sure
            }
            None => upper().is_some_and(|t| confirm_hero_ban(&t, red_ratio, dark_ratio)),
        };
        if confirmed {
            return GamePhase::HeroBan;
        }
    }
    if select_gate {
        let confirmed = match read(Phrase::SelectAPreferredHero) {
            Some(sure) => {
                if sure {
                    tracing::info!(
                        variance = select_variance,
                        "hero select screen detected (words-v1)"
                    );
                }
                sure
            }
            None => upper().is_some_and(|t| confirm_hero_select(&t, select_dark, select_variance)),
        };
        if confirmed {
            return GamePhase::HeroSelect;
        }
    }
    GamePhase::Unknown
}

/// Pixel-scan stride: ratio tests tolerate 1-in-2 sampling and cut CPU ~4×.
const SCAN_STRIDE: u32 = 2;

/// The raw pixel-gate measurements every phase detector thresholds on.
/// Exposed so `examples/phaseprobe.rs` (and fixture tests) can show HOW FAR
/// a frame is from each gate — the 2026-07-14 session-merge went undiagnosed
/// for a full evening because a failed gate produced no signal at all.
#[derive(Debug, Clone, Copy)]
pub struct GateMetrics {
    /// Dark-navy ratio in the top quarter (map-vote gate, needs ≥0.40).
    pub navy_ratio: f32,
    /// Red-accent ratio in the top sixth (hero-ban gate, needs ≥0.003 — only
    /// the "TEAM BAN SCORE" text is bright red on the 2026-07 screen).
    pub ban_red_ratio: f32,
    /// Dark ratio in the top sixth (hero-ban gate, needs ≥0.30).
    pub ban_dark_ratio: f32,
    /// Dark ratio in the top eighth (hero-select gate, needs ≥0.50).
    pub select_dark_header_ratio: f32,
    /// Color variance across the lower-half sample grid (hero-select gate,
    /// needs ≥2000).
    pub select_grid_variance: f64,
}

/// Measure every phase pixel gate on a frame without running any OCR.
pub fn gate_metrics(rgb: &RgbImage) -> GateMetrics {
    let (red, dark) = ban_ratios(rgb);
    GateMetrics {
        navy_ratio: navy_ratio(rgb),
        ban_red_ratio: red,
        ban_dark_ratio: dark,
        select_dark_header_ratio: select_header_dark_ratio(rgb),
        select_grid_variance: select_grid_variance(rgb),
    }
}

/// Dark-navy background ratio in the top quarter of the frame (map-vote gate).
fn navy_ratio(rgb: &RgbImage) -> f32 {
    let (w, h) = rgb.dimensions();
    let mut navy_count = 0u32;
    let mut total = 0u32;

    // Sample the top quarter and side margins (avoiding the map cards in center)
    for y in (0..(h / 4)).step_by(SCAN_STRIDE as usize) {
        for x in (0..w).step_by(SCAN_STRIDE as usize) {
            let pixel = rgb.get_pixel(x, y);
            let [r, g, b] = pixel.0;
            total += 1;
            // The 2026-07 vote screen background sits around (16,25,41)-(16,25,58):
            // far darker than the pre-patch navy (b never reaches 70). Blue only
            // needs to dominate red, not be bright.
            if r < 50 && g < 50 && b > 30 && (b as i32 - r as i32) > 12 {
                navy_count += 1;
            }
        }
    }

    if total == 0 {
        return 0.0;
    }
    navy_count as f32 / total as f32
}

fn detect_map_vote(img: &DynamicImage, rgb: &RgbImage) -> Option<GamePhase> {
    let (w, h) = rgb.dimensions();

    // Map vote screen in OW2 has a dark blue/navy background with slight gradient.
    // The center third of the screen contains 3 map preview cards.
    // Characteristic: dark navy pixels dominate the background.
    let navy_ratio = navy_ratio(rgb);
    if navy_ratio < 0.40 {
        return None;
    }

    // Confirm with OCR on the top portion. The "VOTE FOR A MAP" header bottoms
    // out around 0.17h, so crop h/4 to include it whole.
    let top_region = img.crop_imm(w / 4, 0, w / 2, h / 4);
    match crate::ocr::recognize_sparse_region(&top_region) {
        Ok(text) => confirm_map_vote(&text.to_uppercase(), navy_ratio),
        Err(_) => None,
    }
}

/// Phrase check for the vote screen's OCR text. Requires BOTH words — the
/// top-center crop can also catch chat/scoreboard text where one alone may
/// appear.
fn confirm_map_vote(upper: &str, navy_ratio: f32) -> Option<GamePhase> {
    if upper.contains("VOTE") && upper.contains("MAP") {
        let maps = extract_map_names(upper);
        tracing::info!(navy_ratio, maps = ?maps, "map vote screen detected");
        Some(GamePhase::MapVote { maps })
    } else {
        tracing::debug!(
            navy_ratio,
            text = %upper.chars().take(80).collect::<String>(),
            "map-vote pixel gate passed but OCR did not confirm"
        );
        None
    }
}

/// Hero-ban pixel gate: red-accent and dark ratios in the top sixth.
const BAN_RED_MIN: f32 = 0.003;
const BAN_DARK_MIN: f32 = 0.30;

/// Whether `rgb` passes the hero-ban pixel gate (no OCR, no stability).
pub fn passes_ban_gate(rgb: &RgbImage) -> bool {
    let (red, dark) = ban_ratios(rgb);
    red >= BAN_RED_MIN && dark >= BAN_DARK_MIN
}

/// (red-accent, dark) ratios in the top sixth of the frame (hero-ban gates).
fn ban_ratios(rgb: &RgbImage) -> (f32, f32) {
    let (w, h) = rgb.dimensions();
    let header_h = h / 6;
    let mut red_accent = 0u32;
    let mut dark_count = 0u32;
    let mut total = 0u32;

    for y in (0..header_h).step_by(SCAN_STRIDE as usize) {
        for x in (0..w).step_by(SCAN_STRIDE as usize) {
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
        return (0.0, 0.0);
    }
    (
        red_accent as f32 / total as f32,
        dark_count as f32 / total as f32,
    )
}

fn detect_hero_ban(img: &DynamicImage, rgb: &RgbImage) -> bool {
    let (w, h) = rgb.dimensions();

    // Hero ban screen shows red "TEAM BAN SCORE" text and a "BAN HEROES n"
    // header over a dark background. The bright-red pixels are ONLY that text
    // (~0.5% of the top sixth on the 2026-07 screen), so the red gate is a
    // low bar — the OCR phrase check is what carries specificity.
    let (red_ratio, dark_ratio) = ban_ratios(rgb);
    if red_ratio < BAN_RED_MIN || dark_ratio < BAN_DARK_MIN {
        return false;
    }

    // Confirm with OCR. Crop h/4: "TEAM BAN SCORE" sits near the top edge and
    // "BAN HEROES n" bottoms out around 0.20h.
    let top_region = img.crop_imm(w / 4, 0, w / 2, h / 4);
    match crate::ocr::recognize_sparse_region(&top_region) {
        Ok(text) => confirm_hero_ban(&text.to_uppercase(), red_ratio, dark_ratio),
        Err(_) => false,
    }
}

/// Phrase check for the hero-ban screen's OCR text. Requires a phrase, not
/// bare "BAN" — the crop can catch scoreboard player names and chat.
fn confirm_hero_ban(upper: &str, red_ratio: f32, dark_ratio: f32) -> bool {
    if hero_ban_phrase(upper) {
        tracing::info!(red_ratio, "hero ban screen detected");
        true
    } else {
        tracing::debug!(
            red_ratio,
            dark_ratio,
            text = %upper.chars().take(80).collect::<String>(),
            "hero-ban pixel gate passed but OCR did not confirm"
        );
        false
    }
}

fn hero_ban_phrase(upper: &str) -> bool {
    upper.contains("BAN HERO") || upper.contains("TEAM BAN SCORE") || upper.contains("VOTE TO BAN")
}

/// Cheap Ban Heroes layout — not an outcome / end-reel wake.
///
/// Future: OCR should register the voted hero bans (parse the grid +
/// `YOUR VOTES` slots). This hook only answers "is this the ban UI?"
/// so callers can keep it off [`crate::detect::match_end::detect_end_reel`]
/// and off `end_reel_wake_until`. Not wired into `poll_slow_mode`.
///
/// Layout only (no Tesseract): dark chrome header + red accent and/or the
/// full-width `VOTE TO BAN HEROES` bar. Phrase OCR stays on
/// [`detect_hero_ban`] / [`detect_phase`] for the match-start path.
pub fn detect_ban_screen(_img: &DynamicImage, rgb: &RgbImage) -> bool {
    // `_img` reserved for future ban-grid OCR (register voted heroes).
    ban_screen_layout(rgb)
}

/// Pixel-only Ban Heroes gate used to veto letterbox end-reel wakes.
pub(crate) fn ban_screen_layout(rgb: &RgbImage) -> bool {
    let (red, dark) = ban_ratios(rgb);
    let banner = center_red_banner_ratio(rgb);
    // Strong full-width vote bar is enough (FrozenTear batch: Ban Heroes looks
    // letterboxed). Header red+dark is the existing hero-ban pixel gate;
    // require a weaker banner so a red-title-only header does not veto
    // unrelated screens.
    banner >= 0.10 || (red >= 0.003 && dark >= 0.30 && banner >= 0.04)
}

/// Red-accent fraction in a thin mid-frame band (the `VOTE TO BAN HEROES`
/// bar sits around y 44–56% and spans the playfield).
fn center_red_banner_ratio(rgb: &RgbImage) -> f32 {
    let (w, h) = rgb.dimensions();
    let y0 = h * 440 / 1000;
    let y1 = (h * 560 / 1000).max(y0 + 1);
    let mut red = 0u32;
    let mut total = 0u32;
    for y in (y0..y1.min(h)).step_by(SCAN_STRIDE as usize) {
        for x in (0..w).step_by(SCAN_STRIDE as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            if r > 140 && (i16::from(r) - i16::from(g)) > 50 && (i16::from(r) - i16::from(b)) > 50 {
                red += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        red as f32 / total as f32
    }
}

/// Dark ratio in the top eighth of the frame (hero-select header gate).
fn select_header_dark_ratio(rgb: &RgbImage) -> f32 {
    let (w, h) = rgb.dimensions();
    let header_h = h / 8;
    let mut dark_header = 0u32;
    let mut header_total = 0u32;

    for y in (0..header_h).step_by(SCAN_STRIDE as usize) {
        for x in (0..w).step_by(SCAN_STRIDE as usize) {
            let pixel = rgb.get_pixel(x, y);
            let [r, g, b] = pixel.0;
            header_total += 1;
            if r < 80 && g < 80 && b < 100 {
                dark_header += 1;
            }
        }
    }

    if header_total == 0 {
        return 0.0;
    }
    dark_header as f32 / header_total as f32
}

/// Color variance across a sample grid of the lower half (hero-grid gate).
fn select_grid_variance(rgb: &RgbImage) -> f64 {
    let (w, h) = rgb.dimensions();
    let bottom_start = h / 2;
    let step_x = (w / 50).max(1);
    let step_y = ((h - bottom_start) / 20).max(1);
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
        return 0.0;
    }

    // Hero grid should be colorful — high variance across the samples.
    let avg_r = colors.iter().map(|c| c[0] as f64).sum::<f64>() / colors.len() as f64;
    let avg_g = colors.iter().map(|c| c[1] as f64).sum::<f64>() / colors.len() as f64;
    let avg_b = colors.iter().map(|c| c[2] as f64).sum::<f64>() / colors.len() as f64;

    colors
        .iter()
        .map(|c| {
            let dr = c[0] as f64 - avg_r;
            let dg = c[1] as f64 - avg_g;
            let db = c[2] as f64 - avg_b;
            dr * dr + dg * dg + db * db
        })
        .sum::<f64>()
        / colors.len() as f64
}

fn detect_hero_select(img: &DynamicImage, rgb: &RgbImage) -> bool {
    let (w, h) = rgb.dimensions();

    // Hero select screen characteristics:
    // - Header shows "SELECT A PREFERRED HERO" (2026-07 ban patch) before the
    //   ban vote, and "ASSEMBLE YOUR TEAM" on the in-match hero select
    // - Has a bright, colorful hero grid in the lower 2/3
    // - Top banner area is relatively dark with text
    // - Bottom area has high color variance from hero portraits

    let dark_ratio = select_header_dark_ratio(rgb);
    if dark_ratio < 0.50 {
        return false;
    }

    let variance = select_grid_variance(rgb);
    if variance < 2000.0 {
        return false;
    }

    // Confirm with OCR. The title bottoms out around 0.20h — well below the
    // h/8 dark-header band — so crop h/4.
    let top_region = img.crop_imm(w / 4, 0, w / 2, h / 4);
    match crate::ocr::recognize_sparse_region(&top_region) {
        Ok(text) => confirm_hero_select(&text.to_uppercase(), dark_ratio, variance),
        Err(_) => false,
    }
}

/// Phrase check for the hero-select screen's OCR text. Scoreboard frames pass
/// both pixel gates and their top rows land in the crop, so bare "HERO" would
/// match player names/titles ("Unrelenting Hero"); require phrases instead.
/// The pre-patch "CHOOSE YOUR HERO" title is gone from the game, so a bare
/// "CHOOSE" (chat, a player name) is no longer a hero select.
fn confirm_hero_select(upper: &str, dark_ratio: f32, variance: f64) -> bool {
    let is_hero_select = upper.contains("ASSEMBLE")
        || upper.contains("PREFERRED HERO")
        || (upper.contains("SELECT") && upper.contains("HERO"));
    if is_hero_select {
        tracing::info!(variance, "hero select screen detected");
    } else {
        tracing::debug!(
            dark_ratio,
            variance,
            text = %upper.chars().take(80).collect::<String>(),
            "hero-select pixel gates passed but OCR did not confirm"
        );
    }
    is_hero_select
}

const MAP_NAMES: &[&str] = &[
    "CIRCUIT ROYAL",
    "DORADO",
    "HAVANA",
    "JUNKERTOWN",
    "RIALTO",
    "ROUTE 66",
    "SHAMBALI",
    // Bare "WATCHPOINT" is a prefix token, not a map. A second occurrence
    // beside a Grímsvötn card is a Gibraltar card (see below). "GRIMSVOTN"
    // is its own key.
    "WATCHPOINT",
    "GIBRALTAR",
    "GRIMSVOTN",
    "BLIZZARD WORLD",
    "EICHENWALDE",
    "ADLERSBRUNN",
    "HOLLYWOOD",
    // Not bare "KING"/"ROW" — they substring-match unrelated text
    // ("WRECKING", "BROWN"); apostrophe loss in OCR is covered by both forms.
    "KING'S ROW",
    "KINGS ROW",
    "MIDTOWN",
    "NUMBANI",
    "PARAISO",
    "PARAÍSO",
    "NEON JUNCTION",
    "ANTARCTIC",
    "BUSAN",
    "ILIOS",
    "LIJIANG",
    "NEPAL",
    "OASIS",
    "SAMOA",
    "COLOSSEO",
    "ESPERANCA",
    "ESPERANÇA",
    "NEW QUEEN",
    "RUNASAPI",
    "NEW JUNK",
    "SURAVASA",
    "HANAOKA",
    "THRONE",
    "ANUBIS",
    "AATLIS",
    "GUILLARD",
];

/// Pull map names off the vote screen's OCR text. `text` is the uppercased OCR
/// dump. Exact `contains` alone went 0-for-2 in the field (07-18) — even a clean
/// "ROUTE 66" missed — so this glyph-normalizes both sides (`1`/`|`/`l`→`i`,
/// `0`→`o`; shared with the scoreboard reader) and falls back to a per-length
/// fuzzy match. Returns the raw MAP_NAMES entries; the caller canonicalizes them
/// through the MAPS table (`parse::canonical_map`).
fn extract_map_names(text: &str) -> Vec<String> {
    let norm = crate::parse::normalize_ocr_glyphs(&text.to_lowercase());
    let words: Vec<&str> = norm.split_whitespace().collect();

    let mut found = Vec::new();
    for &name in MAP_NAMES {
        let name_norm = crate::parse::normalize_ocr_glyphs(&name.to_lowercase());

        // Exact (normalized) substring first.
        if norm.contains(&name_norm) {
            found.push(name.to_string());
            continue;
        }

        // Fuzzy per word / window, looser threshold for short names.
        let parts: Vec<&str> = name_norm.split_whitespace().collect();
        let threshold = crate::parse::map_fuzzy_threshold(
            name_norm.chars().filter(|c| !c.is_whitespace()).count(),
        );
        let matched = if parts.len() == 1 {
            words
                .iter()
                .any(|w| strsim::normalized_levenshtein(w, &name_norm) >= threshold)
        } else {
            words
                .windows(parts.len())
                .any(|win| strsim::normalized_levenshtein(&win.join(" "), &name_norm) >= threshold)
        };
        if matched {
            found.push(name.to_string());
        }
    }
    // One "WATCHPOINT" next to a Grímsvötn key is that card's prefix, so the
    // token is dropped. Two or more means the extra card is Gibraltar: emit
    // GIBRALTAR (canonical_map no longer trusts a bare WATCHPOINT) and drop
    // the raw prefix. A lone WATCHPOINT stays in the list; canonical_map
    // turns it into no candidate.
    reconcile_watchpoint_votes(&norm, &mut found);
    found
}

fn reconcile_watchpoint_votes(norm: &str, found: &mut Vec<String>) {
    let folded = |name: &str| crate::parse::normalize_ocr_glyphs(&name.to_lowercase());
    let grimsvotn_present = found.iter().any(|name| folded(name).contains("grimsvotn"));
    if !grimsvotn_present {
        return;
    }
    // A watchpoint token whose next word is Grímsvötn is that card's prefix.
    // Counting every token treated the same Grímsvötn card, read twice, as
    // a second map and added a phantom Gibraltar.
    let words: Vec<&str> = norm
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let mut leftover_watchpoint = 0usize;
    for (i, word) in words.iter().enumerate() {
        if *word != "watchpoint" {
            continue;
        }
        let next = words.get(i + 1).copied().unwrap_or("");
        let next_folded = folded(next).replace('6', "o");
        if !next_folded.contains("grimsvotn") {
            leftover_watchpoint += 1;
        }
    }
    found.retain(|name| folded(name) != "watchpoint");
    // Glyph fold turns `l` into `i`, so compare the folded token, not the
    // raw spelling. Otherwise a Gibraltar card already in the list is added
    // again.
    let gibraltar = folded("gibraltar");
    if leftover_watchpoint >= 1 && !found.iter().any(|name| folded(name) == gibraltar) {
        found.push("GIBRALTAR".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vote_reader_recovers_mangled_names() {
        // The vote screen OCR text is uppercased before extract_map_names.
        // Glyph-mangled ILIOS variants and a clean ROUTE 66 must all resolve.
        // (Mutation check: revert extract_map_names to `text.contains(name)`
        // and every ILIOS assertion here fails — plus the field-observed clean
        // "ROUTE 66" that the old exact path also missed.)
        assert!(extract_map_names("1LIOS").contains(&"ILIOS".to_string()));
        assert!(extract_map_names("IL10S").contains(&"ILIOS".to_string()));
        assert!(extract_map_names("|LIOS").contains(&"ILIOS".to_string()));
        assert!(extract_map_names("ROUTE 66").contains(&"ROUTE 66".to_string()));
        // A realistic multi-option vote line.
        let maps = extract_map_names("VOTE FOR A MAP  1LIOS   NEPAL   0ASIS");
        assert!(maps.contains(&"ILIOS".to_string()));
        assert!(maps.contains(&"NEPAL".to_string()));
        assert!(maps.contains(&"OASIS".to_string()));
    }

    #[test]
    fn vote_reader_splits_the_two_watchpoint_maps() {
        let gibraltar = extract_map_names("WATCHPOINT: GIBRALTAR");
        assert!(gibraltar.contains(&"GIBRALTAR".to_string()));
        assert!(!gibraltar.iter().any(|name| name == "GRIMSVOTN"));
        assert_eq!(
            crate::parse::canonical_map("GIBRALTAR").as_deref(),
            Some("Watchpoint: Gibraltar")
        );

        let grimsvotn = extract_map_names("WATCHPOINT: GRÍMSVÖTN");
        assert!(grimsvotn.contains(&"GRIMSVOTN".to_string()));
        assert!(!grimsvotn.iter().any(|name| name == "GIBRALTAR"));
        assert_eq!(
            crate::parse::canonical_map("GRIMSVOTN").as_deref(),
            Some("Watchpoint: Grímsvötn")
        );

        let bare = extract_map_names("WATCHPOINT");
        assert!(bare.contains(&"WATCHPOINT".to_string()));
        assert_eq!(
            crate::parse::canonical_map("WATCHPOINT").as_deref(),
            None,
            "a lone watchpoint token is not a Gibraltar candidate"
        );
        let prefixed = extract_map_names("WATCHPOINT: GRIMSVOTN");
        assert!(prefixed.contains(&"GRIMSVOTN".to_string()));
        assert!(
            !prefixed.iter().any(|name| name == "WATCHPOINT"),
            "one prefix beside Grímsvötn is that card: {prefixed:?}"
        );

        let both = extract_map_names("WATCHPOINT   WATCHPOINT: GRÍMSVÖTN");
        assert!(
            both.contains(&"GIBRALTAR".to_string()),
            "a second watchpoint card is Gibraltar: {both:?}"
        );
        assert!(both.contains(&"GRIMSVOTN".to_string()));
        assert!(!both.iter().any(|name| name == "WATCHPOINT"));
        assert_eq!(
            crate::parse::canonical_map("GIBRALTAR").as_deref(),
            Some("Watchpoint: Gibraltar")
        );

        let labeled = extract_map_names("WATCHPOINT: GIBRALTAR WATCHPOINT: GRIMSVOTN");
        assert!(labeled.contains(&"GIBRALTAR".to_string()));
        assert!(labeled.contains(&"GRIMSVOTN".to_string()));
        assert_eq!(
            labeled.iter().filter(|name| *name == "GIBRALTAR").count(),
            1,
            "an existing Gibraltar token is not added twice: {labeled:?}"
        );

        let twice = extract_map_names("WATCHPOINT: GRIMSVOTN WATCHPOINT: GRIMSVOTN");
        assert!(
            !twice.iter().any(|name| name == "GIBRALTAR"),
            "the same Grímsvötn card read twice is not a second map: {twice:?}"
        );
        assert!(twice.iter().any(|name| name == "GRIMSVOTN"));
    }

    #[test]
    fn vote_reader_keeps_halloween_aliases() {
        let adlers = extract_map_names("ADLERSBRUNN");
        assert!(adlers.contains(&"ADLERSBRUNN".to_string()));
        assert_eq!(
            crate::parse::canonical_map("ADLERSBRUNN").as_deref(),
            Some("Eichenwalde")
        );
        let hollywood = extract_map_names("HALLOWEEN HOLLYWOOD");
        assert!(hollywood.contains(&"HOLLYWOOD".to_string()));
        let lijiang = extract_map_names("HALLOWEEN LIJIANG");
        assert!(lijiang.contains(&"LIJIANG".to_string()));
        let guillard = extract_map_names("CHATEAU GUILLARD");
        assert!(guillard.contains(&"GUILLARD".to_string()));
        assert_eq!(
            crate::parse::canonical_map("GUILLARD").as_deref(),
            Some("Château Guillard")
        );
    }

    #[test]
    fn vote_reader_ignores_non_map_text() {
        // Header words and hero names on the vote screen must not read as maps.
        let maps = extract_map_names("VOTE FOR A MAP  REAPER  SOMBRA  DOCTRINE");
        assert!(maps.is_empty(), "unexpected maps: {maps:?}");
    }

    // ------------------------------------------------ polled path (P7)

    /// Top quarter navy: passes the map-vote gate only.
    fn vote_frame() -> RgbImage {
        let mut img = RgbImage::from_pixel(1280, 720, image::Rgb([90, 90, 90]));
        for y in 0..180 {
            for x in 0..1280 {
                img.put_pixel(x, y, image::Rgb([16, 25, 58]));
            }
        }
        img
    }

    /// Dark top sixth with a red title strip: passes the ban gate only.
    fn ban_frame() -> RgbImage {
        let mut img = RgbImage::from_pixel(1280, 720, image::Rgb([120, 120, 120]));
        for y in 0..120 {
            for x in 0..1280 {
                img.put_pixel(x, y, image::Rgb([10, 10, 20]));
            }
        }
        for y in 20..30 {
            for x in 560..720 {
                img.put_pixel(x, y, image::Rgb([200, 24, 24]));
            }
        }
        img
    }

    /// Colourful lower half under `top`: passes the select gate (and with a
    /// navy top, the vote gate too).
    fn select_frame(top: [u8; 3]) -> RgbImage {
        let mut img = RgbImage::from_pixel(1280, 720, image::Rgb(top));
        for y in 360..720u32 {
            for x in 0..1280u32 {
                let c = [
                    (x * 7 % 256) as u8,
                    (y * 13 % 256) as u8,
                    ((x + y) * 5 % 256) as u8,
                ];
                img.put_pixel(x, y, image::Rgb(c));
            }
        }
        img
    }

    /// Navy top with a red strip over a hero grid: passes all three gates.
    fn all_gates_frame() -> RgbImage {
        let mut img = select_frame([16, 25, 58]);
        for y in 20..30 {
            for x in 560..720 {
                img.put_pixel(x, y, image::Rgb([200, 24, 24]));
            }
        }
        img
    }

    /// Two polled ticks of one frame (the first only primes stability).
    fn polled(
        rgb: &RgbImage,
        source: PhaseSource,
        words: impl Fn(Phrase, &RgbImage) -> Option<PhraseRead>,
        ocr: impl Fn(&DynamicImage) -> Option<String>,
    ) -> GamePhase {
        let img = DynamicImage::ImageRgb8(rgb.clone());
        let mut stab = FrameStability::default();
        let first = detect_phase_polled_with(&img, rgb, &mut stab, source, &words, &ocr);
        assert_eq!(first, GamePhase::Unknown, "first sighting defers");
        detect_phase_polled_with(&img, rgb, &mut stab, source, &words, &ocr)
    }

    fn no_ocr(_: &DynamicImage) -> Option<String> {
        panic!("OCR ran although the phrase mask is loaded")
    }

    fn no_words(_: Phrase, _: &RgbImage) -> Option<PhraseRead> {
        panic!("the words-v1 masks were read under ocr-v1")
    }

    fn sure_for(want: Phrase) -> impl Fn(Phrase, &RgbImage) -> Option<PhraseRead> {
        move |p, _| {
            Some(if p == want {
                PhraseRead::Sure(0.95)
            } else {
                PhraseRead::Absent(0.3)
            })
        }
    }

    #[test]
    fn the_test_frames_pass_only_their_gates() {
        let m = gate_metrics(&vote_frame());
        assert!(m.navy_ratio >= 0.40 && m.ban_red_ratio < 0.003);
        assert!(m.select_grid_variance < 2000.0);
        let m = gate_metrics(&ban_frame());
        assert!(m.navy_ratio < 0.40 && m.ban_red_ratio >= 0.003 && m.ban_dark_ratio >= 0.30);
        assert!(m.select_grid_variance < 2000.0);
        let m = gate_metrics(&select_frame([10, 10, 10]));
        assert!(m.navy_ratio < 0.40 && m.ban_red_ratio < 0.003);
        assert!(m.select_dark_header_ratio >= 0.50 && m.select_grid_variance >= 2000.0);
        let m = gate_metrics(&select_frame([16, 25, 58]));
        assert!(m.navy_ratio >= 0.40 && m.select_grid_variance >= 2000.0);
    }

    #[test]
    fn under_words_a_sure_phrase_is_the_phase_and_no_ocr_runs() {
        let w = PhaseSource::Words;
        assert_eq!(
            polled(&vote_frame(), w, sure_for(Phrase::VoteForAMap), no_ocr),
            GamePhase::MapVote { maps: Vec::new() }
        );
        assert_eq!(
            polled(&ban_frame(), w, sure_for(Phrase::BanHeroes), no_ocr),
            GamePhase::HeroBan
        );
        assert_eq!(
            polled(
                &select_frame([10, 10, 10]),
                w,
                sure_for(Phrase::SelectAPreferredHero),
                no_ocr
            ),
            GamePhase::HeroSelect
        );
    }

    #[test]
    fn under_words_a_suspect_or_absent_phrase_is_no_phase_and_no_ocr_runs() {
        for read in [PhraseRead::Suspect(0.69), PhraseRead::Absent(0.2)] {
            let words = move |_: Phrase, _: &RgbImage| Some(read);
            for frame in [vote_frame(), ban_frame(), select_frame([10, 10, 10])] {
                assert_eq!(
                    polled(&frame, PhaseSource::Words, words, no_ocr),
                    GamePhase::Unknown,
                    "{read:?}"
                );
            }
        }
    }

    #[test]
    fn under_words_a_screen_without_a_mask_keeps_the_ocr_check() {
        // No mask for this screen's title: the OCR confirms it, as today.
        let unloaded = |missing: Phrase| {
            move |p: Phrase, _: &RgbImage| {
                let vote = |p| matches!(p, Phrase::VoteForAMap | Phrase::SelectingMap);
                // the vote detector has two masks: both are missing
                (p != missing && !(vote(p) && vote(missing))).then_some(PhraseRead::Absent(0.2))
            }
        };
        let text = |t: &'static str| move |_: &DynamicImage| Some(t.to_string());
        let w = PhaseSource::Words;
        assert_eq!(
            polled(
                &vote_frame(),
                w,
                unloaded(Phrase::VoteForAMap),
                text("vote for a map  ilios")
            ),
            GamePhase::MapVote {
                maps: vec!["ILIOS".to_string()]
            }
        );
        assert_eq!(
            polled(
                &ban_frame(),
                w,
                unloaded(Phrase::BanHeroes),
                text("BAN HEROES 20")
            ),
            GamePhase::HeroBan
        );
        assert_eq!(
            polled(
                &select_frame([10, 10, 10]),
                w,
                unloaded(Phrase::SelectAPreferredHero),
                text("ASSEMBLE YOUR TEAM")
            ),
            GamePhase::HeroSelect
        );
        // A loaded mask is never overruled by the OCR, sure or not.
        let absent = |_: Phrase, _: &RgbImage| Some(PhraseRead::Absent(0.2));
        assert_eq!(
            polled(
                &select_frame([10, 10, 10]),
                w,
                absent,
                text("SELECT A PREFERRED HERO")
            ),
            GamePhase::Unknown
        );
    }

    #[test]
    fn ocr_v1_never_reads_the_masks() {
        let o = PhaseSource::Ocr;
        let text = |t: &'static str| move |_: &DynamicImage| Some(t.to_string());
        assert_eq!(
            polled(&vote_frame(), o, no_words, text("VOTE FOR A MAP")),
            GamePhase::MapVote { maps: Vec::new() }
        );
        assert_eq!(
            polled(&ban_frame(), o, no_words, text("TEAM BAN SCORE")),
            GamePhase::HeroBan
        );
        assert_eq!(
            polled(
                &select_frame([10, 10, 10]),
                o,
                no_words,
                text("ASSEMBLE YOUR TEAM")
            ),
            GamePhase::HeroSelect
        );
        assert_eq!(
            polled(&vote_frame(), o, no_words, |_: &DynamicImage| None),
            GamePhase::Unknown
        );
        assert_eq!(PhaseSource::for_reader(false), PhaseSource::Ocr);
        assert_eq!(PhaseSource::for_reader(true), PhaseSource::Words);
        assert_eq!(PhaseSource::default(), PhaseSource::Ocr);
    }

    #[test]
    fn the_pixel_gates_still_gate_the_phrase_read() {
        let black = RgbImage::from_pixel(1280, 720, image::Rgb([0, 0, 0]));
        let sky = RgbImage::from_pixel(1280, 720, image::Rgb([150, 190, 230]));
        let read_any = |_: Phrase, _: &RgbImage| -> Option<PhraseRead> {
            panic!("a phrase was read with every gate shut")
        };
        for frame in [black, sky] {
            let img = DynamicImage::ImageRgb8(frame.clone());
            let mut stab = FrameStability::default();
            for _ in 0..2 {
                assert_eq!(
                    detect_phase_polled_with(
                        &img,
                        &frame,
                        &mut stab,
                        PhaseSource::Words,
                        read_any,
                        no_ocr
                    ),
                    GamePhase::Unknown
                );
            }
        }
        // Only the gate-passing screen's phrases are read.
        for (frame, open) in [
            (
                vote_frame(),
                vec![Phrase::VoteForAMap, Phrase::SelectingMap],
            ),
            (ban_frame(), vec![Phrase::BanHeroes]),
            (
                select_frame([10, 10, 10]),
                vec![Phrase::SelectAPreferredHero],
            ),
        ] {
            let only_open = move |p: Phrase, _: &RgbImage| {
                assert!(open.contains(&p), "a shut gate's phrase was read: {p:?}");
                Some(PhraseRead::Absent(0.2))
            };
            assert_eq!(
                polled(&frame, PhaseSource::Words, only_open, no_ocr),
                GamePhase::Unknown
            );
        }
    }

    #[test]
    fn under_words_the_order_is_vote_then_ban_then_select() {
        // Navy top over a hero grid passes the vote and the select gates.
        let both = select_frame([16, 25, 58]);
        let all_sure = |_: Phrase, _: &RgbImage| Some(PhraseRead::Sure(0.95));
        assert_eq!(
            polled(&both, PhaseSource::Words, all_sure, no_ocr),
            GamePhase::MapVote { maps: Vec::new() }
        );
        // The preferred-hero pick sits on the vote screen's navy: the vote
        // title is absent, so the select title decides.
        assert_eq!(
            polled(
                &both,
                PhaseSource::Words,
                sure_for(Phrase::SelectAPreferredHero),
                no_ocr
            ),
            GamePhase::HeroSelect
        );
    }

    /// F260-6: every position in the order, on one frame that passes all
    /// three gates.
    #[test]
    fn under_words_the_order_holds_with_every_gate_open() {
        let all = all_gates_frame();
        let m = gate_metrics(&all);
        assert!(m.navy_ratio >= 0.40);
        assert!(m.ban_red_ratio >= 0.003 && m.ban_dark_ratio >= 0.30);
        assert!(m.select_dark_header_ratio >= 0.50 && m.select_grid_variance >= 2000.0);
        let sure = |these: &'static [Phrase]| {
            move |p: Phrase, _: &RgbImage| {
                Some(if these.contains(&p) {
                    PhraseRead::Sure(0.95)
                } else {
                    PhraseRead::Absent(0.3)
                })
            }
        };
        let w = PhaseSource::Words;
        let vote = GamePhase::MapVote { maps: Vec::new() };
        use Phrase::{BanHeroes, SelectAPreferredHero, SelectingMap, VoteForAMap};
        // vote before ban and select, by either title
        assert_eq!(
            polled(
                &all,
                w,
                sure(&[VoteForAMap, BanHeroes, SelectAPreferredHero]),
                no_ocr
            ),
            vote
        );
        assert_eq!(
            polled(
                &all,
                w,
                sure(&[SelectingMap, BanHeroes, SelectAPreferredHero]),
                no_ocr
            ),
            vote
        );
        assert_eq!(
            polled(&all, w, sure(&[VoteForAMap, BanHeroes]), no_ocr),
            vote
        );
        assert_eq!(
            polled(&all, w, sure(&[VoteForAMap, SelectAPreferredHero]), no_ocr),
            vote
        );
        // ban before select
        assert_eq!(
            polled(&all, w, sure(&[BanHeroes, SelectAPreferredHero]), no_ocr),
            GamePhase::HeroBan
        );
        assert_eq!(
            polled(&all, w, sure(&[BanHeroes]), no_ocr),
            GamePhase::HeroBan
        );
        // select last
        assert_eq!(
            polled(&all, w, sure(&[SelectAPreferredHero]), no_ocr),
            GamePhase::HeroSelect
        );
        assert_eq!(polled(&all, w, sure(&[]), no_ocr), GamePhase::Unknown);
        // The same order under ocr-v1, from one text.
        let text = |t: &'static str| move |_: &DynamicImage| Some(t.to_string());
        let o = PhaseSource::Ocr;
        assert_eq!(
            polled(
                &all,
                o,
                no_words,
                text("VOTE FOR A MAP  BAN HEROES  ASSEMBLE")
            ),
            vote
        );
        assert_eq!(
            polled(&all, o, no_words, text("BAN HEROES  ASSEMBLE")),
            GamePhase::HeroBan
        );
        assert_eq!(
            polled(&all, o, no_words, text("ASSEMBLE")),
            GamePhase::HeroSelect
        );
    }

    /// F260-6: the OCR text is recognized once per tick and every detector
    /// that needs it gets the same text.
    #[test]
    fn two_detectors_share_one_ocr_text_on_a_tick() {
        use std::cell::Cell;
        let all = all_gates_frame();
        // The vote detector takes the text first and rejects it; the ban
        // and select detectors then check the same text.
        for (said, want) in [
            ("TEAM BAN SCORE", GamePhase::HeroBan),
            ("ASSEMBLE YOUR TEAM", GamePhase::HeroSelect),
            ("NOTHING HERE", GamePhase::Unknown),
        ] {
            let calls = Cell::new(0);
            let once = |_: &DynamicImage| {
                calls.set(calls.get() + 1);
                Some(said.to_string())
            };
            assert_eq!(
                polled(&all, PhaseSource::Ocr, no_words, once),
                want,
                "{said}"
            );
            // two ticks; the first defers before any OCR
            assert_eq!(calls.get(), 1, "{said}");
            // Under words with no mask at all: the same, shared the same way.
            let calls = Cell::new(0);
            let once = |_: &DynamicImage| {
                calls.set(calls.get() + 1);
                Some(said.to_string())
            };
            let none = |_: Phrase, _: &RgbImage| None;
            assert_eq!(polled(&all, PhaseSource::Words, none, once), want, "{said}");
            assert_eq!(calls.get(), 1, "{said}");
        }
        // No text (OCR failed): asked once, no phase for any detector.
        let calls = Cell::new(0);
        let fails = |_: &DynamicImage| {
            calls.set(calls.get() + 1);
            None
        };
        assert_eq!(
            polled(&all, PhaseSource::Ocr, no_words, fails),
            GamePhase::Unknown
        );
        assert_eq!(calls.get(), 1);
    }

    // ------------------------------------------- SELECTING MAP (O7b)

    /// A read per phrase; a phrase not listed has no mask.
    fn reads(list: &[(Phrase, PhraseRead)]) -> impl Fn(Phrase, &RgbImage) -> Option<PhraseRead> {
        let list = list.to_vec();
        move |p, _| list.iter().find(|(q, _)| *q == p).map(|(_, r)| *r)
    }

    #[test]
    fn under_words_a_sure_selecting_map_is_the_map_vote_and_no_ocr_runs() {
        let w = PhaseSource::Words;
        let vote = GamePhase::MapVote { maps: Vec::new() };
        let (sure, suspect, absent) = (
            PhraseRead::Sure(0.95),
            PhraseRead::Suspect(0.5),
            PhraseRead::Absent(0.2),
        );
        const V: Phrase = Phrase::VoteForAMap;
        const S: Phrase = Phrase::SelectingMap;
        // SELECTING MAP on screen: the vote title half-matches (suspect).
        assert_eq!(
            polled(&vote_frame(), w, reads(&[(V, suspect), (S, sure)]), no_ocr),
            vote
        );
        assert_eq!(
            polled(&vote_frame(), w, reads(&[(V, absent), (S, sure)]), no_ocr),
            vote
        );
        // A pack with the selecting mask alone still confirms by it.
        assert_eq!(polled(&vote_frame(), w, reads(&[(S, sure)]), no_ocr), vote);
        // VOTE FOR A MAP on screen: sure by the first title, as before.
        assert_eq!(
            polled(&vote_frame(), w, reads(&[(V, sure), (S, absent)]), no_ocr),
            vote
        );
    }

    #[test]
    fn under_words_a_suspect_or_absent_selecting_map_is_no_phase_and_no_ocr_runs() {
        let w = PhaseSource::Words;
        const V: Phrase = Phrase::VoteForAMap;
        const S: Phrase = Phrase::SelectingMap;
        const SUSPECT: PhraseRead = PhraseRead::Suspect(0.69);
        const ABSENT: PhraseRead = PhraseRead::Absent(0.2);
        for list in [
            &[(V, ABSENT), (S, SUSPECT)][..],
            &[(V, ABSENT), (S, ABSENT)],
            &[(V, SUSPECT), (S, SUSPECT)],
            // one mask loaded and not sure: the detector is loaded, no OCR
            &[(S, SUSPECT)],
            &[(S, ABSENT)],
            &[(V, SUSPECT)],
        ] {
            assert_eq!(
                polled(&vote_frame(), w, reads(list), no_ocr),
                GamePhase::Unknown,
                "{list:?}"
            );
        }
    }

    #[test]
    fn selecting_map_confirms_only_the_map_vote() {
        let w = PhaseSource::Words;
        // Sure on the ban and the select screens (their own titles absent):
        // not a ban, not a select, and the vote gate is shut so not a vote.
        let sel_only = sure_for(Phrase::SelectingMap);
        assert_eq!(
            polled(&ban_frame(), w, &sel_only, no_ocr),
            GamePhase::Unknown
        );
        assert_eq!(
            polled(&select_frame([10, 10, 10]), w, &sel_only, no_ocr),
            GamePhase::Unknown
        );
        // With every gate open it is the vote, never the ban or the select.
        assert_eq!(
            polled(&all_gates_frame(), w, &sel_only, no_ocr),
            GamePhase::MapVote { maps: Vec::new() }
        );
    }

    /// A words-v1 version 1 pack has no `selecting_map` mask: the vote
    /// detector behaves exactly as before (#260).
    #[test]
    fn a_pack_without_selecting_map_reads_the_vote_as_before() {
        use std::cell::RefCell;
        let w = PhaseSource::Words;
        const V: Phrase = Phrase::VoteForAMap;
        let asked = RefCell::new(Vec::new());
        let v1 = |read: PhraseRead| {
            let asked = &asked;
            move |p: Phrase, _: &RgbImage| {
                asked.borrow_mut().push(p);
                (p == V).then_some(read)
            }
        };
        // sure: the vote, and the missing mask is not even asked for
        assert_eq!(
            polled(&vote_frame(), w, v1(PhraseRead::Sure(0.95)), no_ocr),
            GamePhase::MapVote { maps: Vec::new() }
        );
        assert_eq!(*asked.borrow(), vec![V]);
        // not sure (SELECTING MAP reads 0.50 on the vote mask): no phase,
        // and no OCR although the selecting mask is missing
        for read in [PhraseRead::Suspect(0.5), PhraseRead::Absent(0.2)] {
            assert_eq!(
                polled(&vote_frame(), w, v1(read), no_ocr),
                GamePhase::Unknown
            );
        }
        // neither mask: the OCR check, with SELECTING MAP through "VOTES"
        let text = |t: &'static str| move |_: &DynamicImage| Some(t.to_string());
        let none = |_: Phrase, _: &RgbImage| None;
        assert_eq!(
            polled(&vote_frame(), w, none, text("SELECTING MAP  2 VOTES")),
            GamePhase::MapVote { maps: Vec::new() }
        );
    }

    #[test]
    fn under_words_the_first_sighting_still_defers() {
        let img = DynamicImage::ImageRgb8(vote_frame());
        let rgb = vote_frame();
        let mut stab = FrameStability::default();
        let never = |_: Phrase, _: &RgbImage| -> Option<PhraseRead> {
            panic!("the phrase was read before the screen held still")
        };
        assert_eq!(
            detect_phase_polled_with(&img, &rgb, &mut stab, PhaseSource::Words, never, no_ocr),
            GamePhase::Unknown
        );
    }

    #[test]
    fn hero_select_ocr_no_longer_takes_a_bare_choose() {
        assert!(!confirm_hero_select("CHOOSE YOUR HERO", 0.9, 5000.0));
        assert!(!confirm_hero_select("CHOOSE", 0.9, 5000.0));
        assert!(confirm_hero_select("ASSEMBLE YOUR TEAM", 0.9, 5000.0));
        assert!(confirm_hero_select("SELECT A PREFERRED HERO", 0.9, 5000.0));
    }

    #[test]
    fn hero_ban_phrase_accepts_vote_to_ban() {
        assert!(hero_ban_phrase("BAN HEROES 20"));
        assert!(hero_ban_phrase("TEAM BAN SCORE"));
        assert!(hero_ban_phrase("VOTE TO BAN HEROES"));
        assert!(!hero_ban_phrase("BAN"));
        assert!(!hero_ban_phrase("READY"));
        assert!(!hero_ban_phrase("ENTERING GAME"));
    }

    fn ban_heroes_layout_frame() -> RgbImage {
        use image::Rgb;
        let mut img = RgbImage::from_pixel(640, 360, Rgb([12, 16, 28]));
        for y in 0..40 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([8, 8, 12]));
                img.put_pixel(x, 359 - y, Rgb([8, 8, 12]));
            }
        }
        // Colorful hero-grid tiles in the mid band.
        for row in 0..3 {
            for col in 0..8 {
                let x0 = 40 + col * 72;
                let y0 = 70 + row * 70;
                let color = Rgb([
                    40 + (col as u8) * 24,
                    80 + (row as u8) * 40,
                    160u8.saturating_sub(col as u8 * 12),
                ]);
                for y in y0..y0 + 50 {
                    for x in x0..x0 + 60 {
                        if x < 640 && y < 360 {
                            img.put_pixel(x, y, color);
                        }
                    }
                }
            }
        }
        // Full-width VOTE TO BAN bar (y ~46–54%).
        for y in 166..196 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([180, 28, 32]));
            }
        }
        // Header red accent ("BAN HEROES 20").
        for y in 8..22 {
            for x in 250..390 {
                img.put_pixel(x, y, Rgb([200, 24, 24]));
            }
        }
        img
    }

    #[test]
    fn detect_ban_screen_fires_on_vote_bar_layout() {
        let rgb = ban_heroes_layout_frame();
        let img = DynamicImage::ImageRgb8(rgb.clone());
        assert!(
            ban_screen_layout(&rgb),
            "Ban Heroes chrome + red vote bar must open the hook"
        );
        assert!(
            detect_ban_screen(&img, &rgb),
            "public hook is the layout gate (OCR not required this PR)"
        );
    }

    #[test]
    fn detect_ban_screen_rejects_flat_loading_and_combat() {
        use image::Rgb;
        let black = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        assert!(!ban_screen_layout(&black));
        let sky = RgbImage::from_pixel(640, 360, Rgb([180, 200, 220]));
        assert!(!ban_screen_layout(&sky));
        // ENTERING GAME: dark bars + flat navy mid, no red vote bar.
        let mut entering = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 140..220 {
            for x in 0..640 {
                entering.put_pixel(x, y, Rgb([18, 24, 38]));
            }
        }
        assert!(!ban_screen_layout(&entering));
    }
}
