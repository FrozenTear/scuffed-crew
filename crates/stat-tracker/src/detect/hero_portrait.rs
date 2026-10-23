use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::{DynamicImage, RgbImage, imageops::FilterType};

// Board layout moved to `frame/layout.rs` (0.6 lane D V2). Re-exported so
// `detect::hero_portrait::scan_rows` and the other old paths still work.
use crate::frame::layout::detect_player_row_inner;
pub use crate::frame::layout::{
    PortraitRect, RowScan, ScoreboardPreflight, classify_scoreboard_preflight, detect_team_size,
    player_row_highlight, portrait_rect, preflight_scoreboard, resolve_player_row, scan_rows,
};

const PORTRAIT_SIZE: u32 = 32;

pub struct PortraitMatcher {
    references: HashMap<String, RgbImage>,
}

/// How [`load_portrait_pngs`] treats a directory.
#[derive(Clone, Copy)]
enum LoadMode {
    /// Every hero png. A later file with the same stem replaces the earlier one.
    Replace,
    /// A stem that is not loaded yet. A provisional stand-in is skipped.
    FillGaps,
}

impl PortraitMatcher {
    /// Load hero pngs from `dir` and nothing else.
    ///
    /// Tests and tools that already chose a directory use this. It does not
    /// unpack anything and it does not read a reader pack. Placeholder,
    /// skull, and empty icons are not heroes.
    pub fn load(dir: &Path) -> Self {
        let mut references = HashMap::new();
        load_portrait_pngs(&mut references, dir, LoadMode::Replace);
        tracing::info!(count = references.len(), "loaded hero portrait references");
        Self { references }
    }

    /// Load references from the hero packs installed under `data_dir`.
    ///
    /// Those are the same folders [`crate::shadow::heroes::HeroSet`] reads
    /// after pack sync: `templates/heroes` (heroes-v1), `templates/heroes-v2`,
    /// and `templates/heroes-v2-1080`. The 1440p set is loaded first. The
    /// 1080p set fills stems it does not already have. A crop saved under
    /// [`portraits_dir`] fills a stem the pack does not have, and only when
    /// a pack loaded.
    ///
    /// No pack means an empty matcher. [`Self::match_portrait`] then returns
    /// no hero. Files left in [`portraits_dir`] by an older build are not a
    /// substitute, so a missing pack is never a guess.
    pub fn load_installed(data_dir: &Path) -> Self {
        let local = portraits_dir(data_dir);
        if let Err(e) = invalidate_stale_seeds(&local) {
            tracing::warn!(error = %e, "could not move stale portrait seeds aside; will retry next start");
        }
        let Some(set) = crate::shadow::heroes::HeroSet::load_data_dir(data_dir) else {
            tracing::info!("no hero pack installed; portrait matcher will not name a hero");
            return Self {
                references: HashMap::new(),
            };
        };
        let mut references = HashMap::new();
        let mut dirs = Vec::new();
        if let Some(pack) = set.main_pack() {
            dirs.push(hero_pack_dir(data_dir, pack));
        }
        if let Some(pack) = set.pack_1080() {
            let dir = hero_pack_dir(data_dir, pack);
            if dirs.iter().all(|have| have != &dir) {
                dirs.push(dir);
            }
        }
        for dir in &dirs {
            let mode = if references.is_empty() {
                LoadMode::Replace
            } else {
                LoadMode::FillGaps
            };
            load_portrait_pngs(&mut references, dir, mode);
        }
        if !references.is_empty() {
            load_portrait_pngs(&mut references, &local, LoadMode::FillGaps);
        }
        tracing::info!(
            count = references.len(),
            "loaded hero portrait references from the reader pack"
        );
        Self { references }
    }

    pub fn is_empty(&self) -> bool {
        self.references.is_empty()
    }

    #[cfg(test)]
    fn contains(&self, stem: &str) -> bool {
        self.references.contains_key(stem)
    }

    #[cfg(test)]
    fn from_references(references: HashMap<String, RgbImage>) -> Self {
        Self { references }
    }

    pub fn match_portrait(&self, crop: &DynamicImage) -> Option<(String, f64)> {
        // No references (no hero pack, or none of its pngs loaded): name
        // nothing. The closest of an empty set is not a hero.
        if self.references.is_empty() {
            return None;
        }

        let candidate = crop
            .resize_exact(PORTRAIT_SIZE, PORTRAIT_SIZE, FilterType::Lanczos3)
            .to_rgb8();

        let mut best_name = None;
        let mut best_score = f64::MAX;

        for (name, reference) in &self.references {
            let score = mean_absolute_difference(&candidate, reference);
            if score < best_score {
                best_score = score;
                best_name = Some(name.clone());
            }
        }

        // Confidence: invert MAD to 0-1 range (0 = no match, 1 = perfect)
        // MAD ranges from 0 (identical) to 255 (maximum difference)
        let confidence = 1.0 - (best_score / 255.0);

        // Same floor as a portrait switch. A live Tab guessed Sierra or Tracer
        // at 0.77, and that guess overrode the new reader. Below 0.85 is not a hero.
        if !portrait_confidence_is_trusted(confidence) {
            tracing::debug!(best_score, confidence, "portrait match below threshold");
            return None;
        }

        best_name.map(|name| (name, confidence))
    }

    pub fn match_all_portraits(&self, scoreboard: &DynamicImage) -> Vec<(String, f64)> {
        let crops = extract_portrait_crops(scoreboard);
        crops
            .iter()
            .filter_map(|crop| self.match_portrait(crop))
            .collect()
    }

    pub fn match_player_portrait(
        &self,
        scoreboard: &DynamicImage,
        player_row_index: Option<usize>,
    ) -> Option<(String, f64)> {
        let crops = extract_portrait_crops(scoreboard);
        let idx = player_row_index.unwrap_or(0);
        crops.get(idx).and_then(|crop| self.match_portrait(crop))
    }

    pub fn match_player_hero(&self, scoreboard: &DynamicImage) -> Option<(String, f64, usize)> {
        let team_size = detect_team_size(scoreboard);
        self.match_player_hero_with_team_size(scoreboard, team_size)
    }

    /// Like [`Self::match_player_hero`] when the caller already detected team size
    /// (avoids a second full scoreboard saturation scan — P7).
    pub fn match_player_hero_with_team_size(
        &self,
        scoreboard: &DynamicImage,
        team_size: usize,
    ) -> Option<(String, f64, usize)> {
        let player_row = detect_player_row_inner(scoreboard, team_size);
        let crops = extract_portrait_crops_inner(scoreboard, team_size);

        if let Some(idx) = player_row
            && let Some(result) = crops.get(idx).and_then(|crop| self.match_portrait(crop))
        {
            tracing::info!(row = idx, hero = %result.0, confidence = result.1, "matched player portrait via highlighted row");
            return Some((result.0, result.1, idx));
        }

        // Log all team 1 portrait matches for debugging, but do NOT use them
        // as the result. Picking the highest-confidence teammate portrait caused
        // wrong-hero attribution when the highlighted row wasn't detected.
        tracing::debug!("highlighted row detection failed, falling back to OCR text");
        let mut all_matches: Vec<(usize, String, f64)> = Vec::new();
        for (i, crop) in crops.iter().enumerate().take(team_size) {
            if let Some((name, conf)) = self.match_portrait(crop) {
                all_matches.push((i, name, conf));
            }
        }
        if !all_matches.is_empty() {
            tracing::debug!(
                matches = ?all_matches.iter().map(|(i, n, c)| format!("row {i}: {n} ({c:.2})")).collect::<Vec<_>>(),
                "team 1 portrait matches (debug only, not used without highlighted row)"
            );
        }

        None
    }
}

/// Extract hero portrait crops from a scoreboard screenshot.
fn extract_portrait_crops(scoreboard: &DynamicImage) -> Vec<DynamicImage> {
    extract_portrait_crops_inner(scoreboard, detect_team_size(scoreboard))
}

fn extract_portrait_crops_inner(scoreboard: &DynamicImage, team_size: usize) -> Vec<DynamicImage> {
    let dims = (scoreboard.width(), scoreboard.height());
    let mut crops = Vec::with_capacity(team_size * 2);

    for row_idx in 0..(team_size * 2) {
        if let Some(r) = portrait_rect(dims, row_idx, team_size) {
            crops.push(scoreboard.crop_imm(r.x, r.y, r.w, r.h));
        }
    }

    crops
}

/// At 0.85 the portrait is trusted. Just below that it is not.
fn portrait_confidence_is_trusted(confidence: f64) -> bool {
    confidence >= crate::hero_auth::PORTRAIT_SWITCH_MIN_CONF
}

fn mean_absolute_difference(a: &RgbImage, b: &RgbImage) -> f64 {
    let (w, h) = a.dimensions();
    assert_eq!((w, h), b.dimensions());

    let mut total_diff: u64 = 0;
    let pixel_count = (w * h) as u64;

    for (pa, pb) in a.pixels().zip(b.pixels()) {
        let dr = (pa.0[0] as i32 - pb.0[0] as i32).unsigned_abs() as u64;
        let dg = (pa.0[1] as i32 - pb.0[1] as i32).unsigned_abs() as u64;
        let db = (pa.0[2] as i32 - pb.0[2] as i32).unsigned_abs() as u64;
        total_diff += dr + dg + db;
    }

    total_diff as f64 / (pixel_count * 3) as f64
}

/// Where a captured reference for `hero_name` lives: `<portrait_stem>.png`
/// under the local portraits directory. The matcher keys a pack file by
/// its own stem (`ana`, `wrecking-ball`).
pub fn portrait_reference_path(portraits_dir: &Path, hero_name: &str) -> PathBuf {
    portraits_dir.join(format!("{}.png", portrait_stem(hero_name)))
}

/// File stem for `hero_name`: lowercase ASCII, accents folded, punctuation
/// dropped, a run of spaces or underscores between two words becomes one `_`.
/// A space next to punctuation goes with it.
///
/// "D.Va" -> `dva`, "Soldier: 76" -> `soldier76`, "Torbjörn" -> `torbjorn`,
/// "Lúcio" -> `lucio`, "Wrecking Ball" -> `wrecking_ball`. A stem maps to
/// itself, so `portrait_stem(stem) != stem` marks a file no hero name can
/// produce. A letter with no ASCII fold is kept lowercased, so a name never
/// maps to an empty stem.
pub fn portrait_stem(hero_name: &str) -> String {
    let mut stem = String::with_capacity(hero_name.len());
    let mut space = false;
    let mut punct = false;
    for c in hero_name.chars().flat_map(char::to_lowercase) {
        if c.is_whitespace() || c == '_' {
            space = true;
            continue;
        }
        let folded = ascii_fold(c);
        if folded.is_empty() {
            punct = true;
            continue;
        }
        if space && !punct && !stem.is_empty() {
            stem.push('_');
        }
        space = false;
        punct = false;
        stem.push_str(&folded);
    }
    stem
}

/// `c` (already lowercase) as ASCII: itself when ASCII alphanumeric, its base
/// letter(s) when a folded Latin letter, itself when any other letter or
/// digit, and empty for punctuation and symbols.
fn ascii_fold(c: char) -> String {
    let base = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'č' => "c",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => "e",
        'ì' | 'í' | 'î' | 'ï' | 'ī' => "i",
        'ł' => "l",
        'ñ' | 'ń' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => "o",
        'œ' => "oe",
        'ś' | 'š' => "s",
        'ß' => "ss",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => "u",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        'þ' => "th",
        'ð' => "d",
        _ if c.is_alphanumeric() => return c.to_string(),
        _ => "",
    };
    base.to_string()
}

/// Save a portrait crop as a reference image.
/// The filename is the hero name (lowercase, no spaces).
///
/// Always overwrites. Callers that must keep a real crop or a user-supplied
/// file use [`save_captured_portrait`].
pub fn save_portrait_reference(
    portraits_dir: &Path,
    hero_name: &str,
    crop: &DynamicImage,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(portraits_dir)?;
    let path = portrait_reference_path(portraits_dir, hero_name);
    let resized = crop.resize_exact(PORTRAIT_SIZE, PORTRAIT_SIZE, FilterType::Lanczos3);
    let bytes = png_bytes(&resized)?;
    write_bytes_atomic(&path, &bytes)?;
    tracing::info!(hero = hero_name, path = %path.display(), "saved portrait reference");
    Ok(path)
}

/// Write `bytes` via a temp file in the same directory, then rename.
/// Rename on one filesystem is atomic, so a crash cannot leave a half-written
/// portrait in the slot the matcher will load.
fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portrait path has no file name",
        )
    })?;
    let tmp_path = path.with_file_name(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    if let Err(e) = std::fs::write(&tmp_path, bytes) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

fn png_bytes(img: &DynamicImage) -> Result<Vec<u8>, image::ImageError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

/// Whether this capture may write a portrait reference.
///
/// A save needs an identified player row (row 0 belongs to someone else when
/// the row was not found), a portrait grid known to fit the board layout
/// ([`portrait_seed_layout_known`]), a slot that is empty or still a
/// provisional stand-in, and either the career panel or `--collect-portraits`.
pub fn should_save_portrait_crop(
    career_panel: bool,
    collect_portraits: bool,
    slot_replaceable: bool,
    player_row_known: bool,
    layout_known: bool,
) -> bool {
    player_row_known && layout_known && slot_replaceable && (collect_portraits || career_panel)
}

/// Whether [`portrait_rect`] is known to fit this board, so its crop may seed
/// a hero reference.
///
/// 6v6 has one measured layout: always. 5v5 only when the own name matched in
/// the in-match no-perk window
/// ([`crate::ocr::preprocess::NAME_WINDOW_5V5_INGAME`]) and in no other
/// window. The 5v5 grid's x fits only that layout: one or two perks shift the
/// table 28 or 66 px left, so the square is cut or misplaced, and the centred
/// table puts the portrait near x 323. A saved crop is never overwritten, so a
/// misplaced one would stick. A row found only by the highlight says nothing
/// about the layout and never seeds on 5v5.
///
/// Name windows 0 (no perks, x 0.10-0.25) and 1 (perks, x 0.095-0.20)
/// overlap, so a short name on a no-perk board matches both, `sole_window` is
/// false, and that board never seeds. This fails safe: no seed is written and
/// the career panel stays the hero authority. The one wrong-seed path left is
/// a perk board where window 1's read fails and window 0's succeeds.
pub fn portrait_seed_layout_known(
    team_size: usize,
    name_match: Option<&crate::parse::NameMatch>,
) -> bool {
    if team_size >= 6 {
        return true;
    }
    name_match.is_some_and(|m| {
        m.window == crate::ocr::preprocess::NAME_WINDOW_5V5_INGAME && m.sole_window
    })
}

/// Write `crop` only when the slot is empty or still holds a provisional
/// stand-in. Returns `Ok(None)` when an existing file has any other bytes.
pub fn save_captured_portrait(
    portraits_dir: &Path,
    hero_name: &str,
    crop: &DynamicImage,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let path = portrait_reference_path(portraits_dir, hero_name);
    if !portrait_slot_is_replaceable(&path) {
        tracing::debug!(
            hero = hero_name,
            path = %path.display(),
            "portrait reference kept (bytes differ from the provisional stand-in)"
        );
        return Ok(None);
    }
    save_portrait_reference(portraits_dir, hero_name, crop).map(Some)
}

/// Get the portraits directory path within the data dir.
pub fn portraits_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("portraits")
}

/// Bundled stand-in portraits the tracker may replace.
///
/// Each entry is `(file stem, sha256 hex of a bundled PNG)`. A stem may
/// appear more than once. Never remove a hash that has shipped: an older
/// install still has those bytes on disk, and dropping the digest would make
/// the tracker treat that stand-in as a real crop and keep it forever.
/// Append the new digest instead.
///
/// An on-disk file whose bytes hash to any listed digest for its stem is
/// treated as missing, so the first real in-game crop overwrites it. Any
/// other bytes — a real crop, or a file the user placed there — are left
/// alone.
///
/// Doctrine's old stand-in. The bytes are not shipped. The digest stays so
/// an install that still has that file treats the slot as empty and a
/// capture can replace it.
const PROVISIONAL_PORTRAITS: &[(&str, &str)] = &[(
    "doctrine",
    "6dd3449514c8b0a657c5ca07f2f89e97820ce76a608684029e36300cc7a862cb",
)];

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes);
    format!("{digest:x}")
}

fn digest_matches_provisional(table: &[(&str, &str)], stem: &str, digest: &str) -> bool {
    table
        .iter()
        .any(|(name, hash)| *name == stem && *hash == digest)
}

fn bytes_match_provisional(stem: &str, bytes: &[u8]) -> bool {
    digest_matches_provisional(PROVISIONAL_PORTRAITS, stem, &sha256_hex(bytes))
}

/// True when `path` is absent, or its bytes are a known provisional stand-in.
/// A file with any other bytes is not replaceable.
pub fn portrait_slot_is_replaceable(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    match std::fs::read(path) {
        Ok(bytes) => bytes_match_provisional(stem, &bytes),
        Err(_) => false,
    }
}

/// Version of the portrait grid that captured seeds are cut with.
///
/// - 1: alpha.5 and earlier. The 5v5 grid put "row 0" at y 120 (row 1's
///   portrait), so a 5v5 seed could hold a teammate's hero.
/// - 2: the measured 5v5 grid, and 5v5 seeds only from the no-perk in-match
///   layout ([`portrait_seed_layout_known`]).
///
/// Bump it whenever [`portrait_rect`] moves for a layout that may seed.
pub const SEED_GEOMETRY_VERSION: u32 = 2;

/// Marker in the portraits directory holding the [`SEED_GEOMETRY_VERSION`]
/// its captured seeds were cut with. Absent means version 1.
const SEED_GEOMETRY_MARKER: &str = ".seed-geometry";

/// Move portrait files the matcher must no longer load out of the reference
/// set. Nothing is deleted. Returns how many files moved.
///
/// - Once per [`SEED_GEOMETRY_VERSION`], while the marker is older: every
///   `*.png` that is not a provisional stand-in moves to
///   `invalidated-<old version>/`. Nothing is shipped in the binary to
///   compare against, so an older unpacked reference moves with the seeds.
///   The next qualifying capture can seed the slot again.
/// - On every call: a `*.png` whose stem is not a [`portrait_stem`] (alpha.5
///   wrote `soldier:_76.png` and `torbjörn.png`) moves to
///   `invalidated-<marker version>/`. No hero name maps to it any more.
///
/// A png that cannot be read is skipped with one WARN and stays where it is.
/// It does not stop the other moves or the marker. The marker is written
/// once every move succeeded.
pub fn invalidate_stale_seeds(portraits_dir: &Path) -> std::io::Result<usize> {
    invalidate_stale_seeds_with(portraits_dir, &[])
}

fn invalidate_stale_seeds_with(
    portraits_dir: &Path,
    bundled: &[(&str, &[u8])],
) -> std::io::Result<usize> {
    invalidate_stale_seeds_reading(portraits_dir, bundled, |p| std::fs::read(p))
}

fn invalidate_stale_seeds_reading(
    portraits_dir: &Path,
    bundled: &[(&str, &[u8])],
    read: impl Fn(&Path) -> std::io::Result<Vec<u8>>,
) -> std::io::Result<usize> {
    std::fs::create_dir_all(portraits_dir)?;
    let marker = portraits_dir.join(SEED_GEOMETRY_MARKER);
    let on_disk: u32 = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1);
    let stale_grid = on_disk < SEED_GEOMETRY_VERSION;

    let dest = portraits_dir.join(format!("invalidated-{on_disk}"));
    let mut moved = 0usize;
    for entry in std::fs::read_dir(portraits_dir)? {
        let path = entry?.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.starts_with('.') {
            continue;
        }
        let bad_stem = portrait_stem(stem) != stem;
        if !bad_stem {
            if !stale_grid {
                continue;
            }
            let bytes = match read(&path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "could not read portrait file during the seed migration; left in place"
                    );
                    continue;
                }
            };
            let is_bundled = bundled
                .iter()
                .any(|(name, data)| *name == stem && *data == bytes.as_slice());
            if is_bundled || bytes_match_provisional(stem, &bytes) {
                continue;
            }
        }
        std::fs::create_dir_all(&dest)?;
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let mut target = dest.join(file_name);
        let mut n = 1;
        while target.exists() {
            target = dest.join(format!("{stem}.{n}.png"));
            n += 1;
        }
        std::fs::rename(&path, &target)?;
        moved += 1;
    }

    if stale_grid {
        write_bytes_atomic(&marker, format!("{SEED_GEOMETRY_VERSION}\n").as_bytes())?;
    }
    if moved > 0 {
        tracing::info!(
            moved,
            from_version = on_disk,
            to_version = SEED_GEOMETRY_VERSION,
            dir = %dest.display(),
            "moved portrait seeds cut with an older portrait grid or under a hero-name stem no build writes aside; they re-seed from new captures"
        );
    }
    Ok(moved)
}

fn hero_pack_dir(data_dir: &Path, pack: crate::shadow::heroes::HeroPack) -> PathBuf {
    use crate::shadow::heroes::{
        HeroPack, HeroTemplates, TEMPLATE_SUBDIR_V2, TEMPLATE_SUBDIR_V2_1080,
    };
    match pack {
        HeroPack::V1 => HeroTemplates::dir_in(data_dir),
        HeroPack::V2 => data_dir.join(TEMPLATE_SUBDIR_V2),
        HeroPack::V2At1080 => data_dir.join(TEMPLATE_SUBDIR_V2_1080),
    }
}

/// Read top-level `<stem>.png` files into `references`. `special/` is not
/// walked: those icons are placeholders, skulls, and empty slots.
fn load_portrait_pngs(references: &mut HashMap<String, RgbImage>, dir: &Path, mode: LoadMode) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.starts_with('.') || scuffed_types::is_placeholder_hero(stem) {
            continue;
        }
        if matches!(mode, LoadMode::FillGaps)
            && (references.contains_key(stem) || portrait_slot_is_replaceable(&path))
        {
            continue;
        }
        match image::open(&path) {
            Ok(img) => {
                let resized = img
                    .resize_exact(PORTRAIT_SIZE, PORTRAIT_SIZE, FilterType::Lanczos3)
                    .to_rgb8();
                references.insert(stem.to_string(), resized);
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to load portrait reference");
            }
        }
    }
}

#[cfg(test)]
mod reference_path_tests {
    use super::portrait_reference_path;
    use std::path::Path;

    /// The on-disk stem is what `PortraitMatcher::load` keys on, so a
    /// dotted/spaced hero name must map to exactly one predictable file
    /// (WL-5 review nit).
    #[test]
    fn stem_drops_dots_and_underscores_spaces() {
        let dir = Path::new("/tmp/portraits");
        assert_eq!(portrait_reference_path(dir, "D.Mon"), dir.join("dmon.png"));
        assert_eq!(portrait_reference_path(dir, "D.Va"), dir.join("dva.png"));
        assert_eq!(
            portrait_reference_path(dir, "Wrecking Ball"),
            dir.join("wrecking_ball.png")
        );
        assert_eq!(
            portrait_reference_path(dir, "Doctrine"),
            dir.join("doctrine.png")
        );
    }

    /// alpha.5 kept ':' and 'ö', so Soldier: 76 and Torbjörn seeded a second
    /// reference beside the bundled `soldier76` and `torbjorn` (#240).
    #[test]
    fn stem_folds_accents_and_drops_punctuation() {
        let dir = Path::new("/tmp/portraits");
        for (hero, stem) in [
            ("Soldier: 76", "soldier76"),
            ("Torbjörn", "torbjorn"),
            ("D.Va", "dva"),
            ("Lúcio", "lucio"),
            ("Jetpack Cat", "jetpack_cat"),
            ("Junker  Queen", "junker_queen"),
        ] {
            assert_eq!(super::portrait_stem(hero), stem, "{hero}");
            assert_eq!(
                portrait_reference_path(dir, hero),
                dir.join(format!("{stem}.png")),
                "{hero}"
            );
        }
    }

    /// Pack file stems stay the matcher's key. `portrait_stem` folds a
    /// display name; it is not applied to `wrecking-ball`.
    #[test]
    fn pack_file_stems_are_not_rewritten() {
        assert_eq!(super::portrait_stem("Wrecking Ball"), "wrecking_ball");
        assert_ne!(super::portrait_stem("wrecking-ball"), "wrecking-ball");
        assert_eq!(
            scuffed_types::canonical_hero("wrecking-ball"),
            "Wrecking Ball"
        );
    }
}

#[cfg(test)]
mod provisional_portrait_tests {
    use super::{
        digest_matches_provisional, portrait_slot_is_replaceable, save_captured_portrait,
        should_save_portrait_crop,
    };
    use image::{DynamicImage, Rgba, RgbaImage};

    fn solid(color: Rgba<u8>) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, color))
    }

    /// The digest of the old Doctrine stand-in stays listed. Dropping it
    /// would freeze that file on disk as if it were a real crop.
    #[test]
    fn doctrine_stand_in_digest_stays_listed() {
        assert!(super::PROVISIONAL_PORTRAITS.iter().any(|(name, hash)| {
            *name == "doctrine"
                && *hash == "6dd3449514c8b0a657c5ca07f2f89e97820ce76a608684029e36300cc7a862cb"
        }));
    }

    #[test]
    fn a_missing_slot_is_replaced_and_a_user_file_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        assert!(portrait_slot_is_replaceable(
            &dir.path().join("doctrine.png")
        ));
        let saved =
            save_captured_portrait(dir.path(), "Doctrine", &solid(Rgba([0, 255, 0, 255]))).unwrap();
        assert!(saved.is_some());
        let path = dir.path().join("doctrine.png");
        let before = std::fs::read(&path).unwrap();
        assert!(!portrait_slot_is_replaceable(&path));
        let again =
            save_captured_portrait(dir.path(), "Doctrine", &solid(Rgba([0, 0, 255, 255]))).unwrap();
        assert!(again.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn capture_leaves_a_non_provisional_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doctrine.png");
        solid(Rgba([255, 0, 0, 255]))
            .save(&path)
            .expect("write user portrait");
        let before = std::fs::read(&path).unwrap();
        assert!(!portrait_slot_is_replaceable(&path));

        let saved =
            save_captured_portrait(dir.path(), "Doctrine", &solid(Rgba([0, 0, 255, 255]))).unwrap();
        assert!(saved.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// A stem may list several shipped digests. `.any` keeps every one of
    /// them replaceable; stopping at the first row would freeze the older file.
    #[test]
    fn a_stem_matches_every_listed_hash() {
        let table = &[("doctrine", "aaa"), ("doctrine", "bbb"), ("ana", "ccc")];
        assert!(digest_matches_provisional(table, "doctrine", "aaa"));
        assert!(digest_matches_provisional(table, "doctrine", "bbb"));
        assert!(!digest_matches_provisional(table, "doctrine", "ccc"));
        assert!(!digest_matches_provisional(table, "ana", "aaa"));
    }

    #[test]
    fn capture_decision_needs_a_known_row_and_a_replaceable_slot() {
        // Career panel, empty-or-stand-in slot, identified row.
        assert!(should_save_portrait_crop(true, false, true, true, true));
        // Row not identified: do not crop row 0, even with --collect-portraits.
        assert!(!should_save_portrait_crop(true, true, true, false, true));
        // Portrait/held/text guess, collection off.
        assert!(!should_save_portrait_crop(false, false, true, true, true));
        // Layout not known to fit the portrait grid (5v5 perks / centred).
        assert!(!should_save_portrait_crop(true, true, true, true, false));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doctrine.png");
        solid(Rgba([255, 0, 0, 255]))
            .save(&path)
            .expect("write real portrait");
        let replaceable = portrait_slot_is_replaceable(&path);
        assert!(!replaceable);
        assert!(!should_save_portrait_crop(
            true,
            true,
            replaceable,
            true,
            true
        ));
    }
}

#[cfg(test)]
mod seed_layout_tests {
    use super::{portrait_seed_layout_known, should_save_portrait_crop};
    use crate::ocr::preprocess::{
        NAME_WINDOW_5V5_CENTRED, NAME_WINDOW_5V5_INGAME, NAME_WINDOW_5V5_PERKS,
    };
    use crate::parse::NameMatch;

    fn m(window: usize, sole_window: bool) -> NameMatch {
        NameMatch {
            row: 0,
            window,
            sole_window,
        }
    }

    #[test]
    fn five_v_five_seeds_only_from_the_no_perk_in_match_window() {
        assert!(portrait_seed_layout_known(
            5,
            Some(&m(NAME_WINDOW_5V5_INGAME, true))
        ));
        // Perk and centred boards: the grid's x does not fit them.
        assert!(!portrait_seed_layout_known(
            5,
            Some(&m(NAME_WINDOW_5V5_PERKS, true))
        ));
        assert!(!portrait_seed_layout_known(
            5,
            Some(&m(NAME_WINDOW_5V5_CENTRED, true))
        ));
        // The no-perk window matched, but so did another: layout unknown.
        assert!(!portrait_seed_layout_known(
            5,
            Some(&m(NAME_WINDOW_5V5_INGAME, false))
        ));
        // Row from the highlight alone: layout unknown.
        assert!(!portrait_seed_layout_known(5, None));
    }

    #[test]
    fn six_v_six_seeding_is_unchanged() {
        assert!(portrait_seed_layout_known(6, None));
        assert!(portrait_seed_layout_known(6, Some(&m(0, true))));
    }

    #[test]
    fn a_perk_board_never_seeds_even_with_the_career_panel() {
        let layout = portrait_seed_layout_known(5, Some(&m(NAME_WINDOW_5V5_PERKS, true)));
        assert!(!should_save_portrait_crop(true, true, true, true, layout));
        let layout = portrait_seed_layout_known(5, Some(&m(NAME_WINDOW_5V5_INGAME, true)));
        assert!(should_save_portrait_crop(true, false, true, true, layout));
    }
}

#[cfg(test)]
mod stale_seed_tests {
    use super::{
        PortraitMatcher, SEED_GEOMETRY_MARKER, SEED_GEOMETRY_VERSION, invalidate_stale_seeds,
        invalidate_stale_seeds_reading, invalidate_stale_seeds_with,
    };

    fn marker(dir: &std::path::Path) -> String {
        std::fs::read_to_string(dir.join(SEED_GEOMETRY_MARKER)).unwrap()
    }

    #[test]
    fn fresh_dir_moves_nothing_and_writes_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let portraits = dir.path().join("portraits");
        assert_eq!(invalidate_stale_seeds(&portraits).unwrap(), 0);
        assert_eq!(marker(&portraits).trim(), SEED_GEOMETRY_VERSION.to_string());
    }

    #[test]
    fn a_stale_grid_moves_ordinary_pngs_because_nothing_is_shipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("ana.png"), b"old unpack").unwrap();
        std::fs::write(p.join("notes.txt"), b"keep").unwrap();
        assert_eq!(invalidate_stale_seeds(p).unwrap(), 1);
        assert!(!p.join("ana.png").exists());
        assert_eq!(
            std::fs::read(p.join("invalidated-1/ana.png")).unwrap(),
            b"old unpack"
        );
        assert!(p.join("notes.txt").exists());
        assert_eq!(marker(p).trim(), SEED_GEOMETRY_VERSION.to_string());
        // Once per version: a file written after the marker stays.
        std::fs::write(p.join("dmon.png"), b"new seed").unwrap();
        assert_eq!(invalidate_stale_seeds(p).unwrap(), 0);
        assert_eq!(std::fs::read(p.join("dmon.png")).unwrap(), b"new seed");
    }

    #[test]
    fn listed_reference_bytes_stay_while_seeds_move() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let known: &[(&str, &[u8])] = &[("ana", b"ref ana")];
        std::fs::write(p.join("ana.png"), b"ref ana").unwrap();
        std::fs::write(p.join("dmon.png"), b"seed one").unwrap();
        std::fs::write(p.join("mercy.png"), b"seed two").unwrap();
        assert_eq!(invalidate_stale_seeds_with(p, known).unwrap(), 2);
        assert_eq!(std::fs::read(p.join("ana.png")).unwrap(), b"ref ana");
        assert_eq!(
            std::fs::read(p.join("invalidated-1/dmon.png")).unwrap(),
            b"seed one"
        );
        assert_eq!(
            std::fs::read(p.join("invalidated-1/mercy.png")).unwrap(),
            b"seed two"
        );
    }

    #[test]
    fn an_older_marker_also_invalidates_and_names_its_version() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join(SEED_GEOMETRY_MARKER), "1\n").unwrap();
        std::fs::write(p.join("ana.png"), b"crop").unwrap();
        let fake: &[(&str, &[u8])] = &[("ana", b"bundled ana")];
        assert_eq!(invalidate_stale_seeds_with(p, fake).unwrap(), 1);
        assert!(p.join("invalidated-1/ana.png").exists());
        // A second stale file of the same name never overwrites the first.
        std::fs::write(p.join(SEED_GEOMETRY_MARKER), "1\n").unwrap();
        std::fs::write(p.join("ana.png"), b"crop two").unwrap();
        assert_eq!(invalidate_stale_seeds_with(p, fake).unwrap(), 1);
        assert_eq!(
            std::fs::read(p.join("invalidated-1/ana.png")).unwrap(),
            b"crop"
        );
        assert_eq!(
            std::fs::read(p.join("invalidated-1/ana.1.png")).unwrap(),
            b"crop two"
        );
    }

    #[test]
    fn loading_does_not_invent_a_reference() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("mercy.png"), b"not a png").unwrap();
        let matcher = PortraitMatcher::load(p);
        assert!(matcher.is_empty());
        assert_eq!(std::fs::read(p.join("mercy.png")).unwrap(), b"not a png");
    }

    /// alpha.5 bad stems (#240) move. A stem whose bytes are listed as a
    /// known reference stays; the public migration lists none.
    #[test]
    fn bad_stem_seeds_move_with_the_grid_migration() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("soldier76.png"), b"listed").unwrap();
        std::fs::write(p.join("soldier:_76.png"), b"seed").unwrap();
        std::fs::write(p.join("torbjörn.png"), b"seed").unwrap();
        let known: &[(&str, &[u8])] = &[("soldier76", b"listed")];
        assert_eq!(invalidate_stale_seeds_with(p, known).unwrap(), 2);
        assert!(p.join("invalidated-1/soldier:_76.png").exists());
        assert!(p.join("invalidated-1/torbjörn.png").exists());
        assert_eq!(std::fs::read(p.join("soldier76.png")).unwrap(), b"listed");
        assert_eq!(marker(p).trim(), SEED_GEOMETRY_VERSION.to_string());
    }

    /// A bad stem written after the marker is current (an older daemon still
    /// running) still moves aside on the next start; normal seeds stay.
    #[test]
    fn bad_stem_seeds_move_even_when_the_marker_is_current() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(
            p.join(SEED_GEOMETRY_MARKER),
            format!("{SEED_GEOMETRY_VERSION}\n"),
        )
        .unwrap();
        std::fs::write(p.join("dmon.png"), b"v2 seed").unwrap();
        std::fs::write(p.join("torbjörn.png"), b"v2 bad stem").unwrap();
        assert_eq!(invalidate_stale_seeds(p).unwrap(), 1);
        assert_eq!(std::fs::read(p.join("dmon.png")).unwrap(), b"v2 seed");
        assert!(!p.join("torbjörn.png").exists());
        let dest = format!("invalidated-{SEED_GEOMETRY_VERSION}/torbjörn.png");
        assert_eq!(std::fs::read(p.join(dest)).unwrap(), b"v2 bad stem");
        // Nothing left to move.
        assert_eq!(invalidate_stale_seeds(p).unwrap(), 0);
    }

    /// One unreadable png (#240 N1): one WARN, the file stays, the other
    /// seeds still move, and the marker is written so it does not repeat.
    #[test]
    fn an_unreadable_png_warns_once_and_the_marker_is_still_written() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("ana.png"), b"unreadable").unwrap();
        std::fs::write(p.join("mercy.png"), b"seed").unwrap();
        let (moved, log) = with_warn_log(|| {
            invalidate_stale_seeds_reading(p, &[], |path| {
                if path.file_name().is_some_and(|n| n == "ana.png") {
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                } else {
                    std::fs::read(path)
                }
            })
        });
        assert_eq!(moved.unwrap(), 1);
        assert_eq!(log.matches("WARN").count(), 1, "{log}");
        assert!(log.contains("ana.png"), "{log}");
        assert!(p.join("ana.png").exists());
        assert!(p.join("invalidated-1/mercy.png").exists());
        assert_eq!(marker(p).trim(), SEED_GEOMETRY_VERSION.to_string());
    }

    #[derive(Clone, Default)]
    struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }

    fn with_warn_log<T>(f: impl FnOnce() -> T) -> (T, String) {
        let buf = Buf::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .finish();
        let out = tracing::subscriber::with_default(sub, f);
        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        (out, text)
    }
}

#[cfg(test)]
mod portrait_floor_tests {
    use super::{PortraitMatcher, portrait_confidence_is_trusted};
    use image::{DynamicImage, Rgb, RgbImage};
    use std::collections::HashMap;

    fn solid(value: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 32, Rgb([value, value, value])))
    }

    #[test]
    fn portrait_matcher_rejects_a_low_confidence_guess() {
        let mut references = HashMap::new();
        references.insert("sierra".into(), solid(128).to_rgb8());
        let matcher = PortraitMatcher::from_references(references);
        let weak = solid(128 + 59);
        let confidence = 1.0 - (59.0 / 255.0);
        assert!(
            (0.70..0.85).contains(&confidence),
            "synthetic pair should sit near 0.77, got {confidence}"
        );
        assert!(
            matcher.match_portrait(&weak).is_none(),
            "0.77 must not override the new reader"
        );
        let trusted = matcher
            .match_portrait(&solid(128 + 30))
            .expect("about 0.88 is a portrait");
        assert_eq!(trusted.0, "sierra");
        assert!(trusted.1 >= 0.85, "{}", trusted.1);
        let same = matcher.match_portrait(&solid(128)).expect("identical crop");
        assert_eq!(same.0, "sierra");
        assert!(same.1 > 0.99, "{}", same.1);
    }

    #[test]
    fn portrait_matcher_accepts_the_floor_and_rejects_just_below() {
        assert!(portrait_confidence_is_trusted(0.85));
        assert!(!portrait_confidence_is_trusted(0.85_f64.next_down()));
        let mut references = HashMap::new();
        references.insert("sierra".into(), solid(128).to_rgb8());
        let matcher = PortraitMatcher::from_references(references);
        // A flat image's confidence is 1 - delta/255. Delta 38 is the first
        // step at or above 0.85. Delta 39 is the step just below.
        let at_floor = 1.0 - (38.0 / 255.0);
        let just_below = 1.0 - (39.0 / 255.0);
        assert!(at_floor >= 0.85, "{at_floor}");
        assert!(just_below < 0.85, "{just_below}");
        let accepted = matcher
            .match_portrait(&solid(128 + 38))
            .expect("the 0.85 step is a portrait");
        assert_eq!(accepted.0, "sierra");
        assert!((accepted.1 - at_floor).abs() < 1e-9, "{}", accepted.1);
        assert!(matcher.match_portrait(&solid(128 + 39)).is_none());
    }
}

#[cfg(test)]
mod player_row_fallback_tests {
    use super::{PortraitMatcher, resolve_player_row};
    use image::{DynamicImage, Rgb, RgbImage};
    use std::collections::HashMap;

    /// A 5v5 board, flat dark grey, with one team-1 row slot lighter, as the
    /// game draws the player's own row.
    fn board_with_highlight(row: usize) -> DynamicImage {
        let (w, h, team_size) = (1600u32, 900u32, 5usize);
        let mut img = RgbImage::from_pixel(w, h, Rgb([40, 40, 40]));
        let slot = crate::ocr::preprocess::row_slot_height(h, team_size);
        let top = (h as f64 * 0.025) as u32 + row as u32 * slot;
        for y in top..top + slot {
            for x in 0..w {
                img.put_pixel(x, y, Rgb([62, 62, 62]));
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    /// alpha.7 O9: the own-row seeding gate needs how far the highlighted
    /// row leads the next brightest one. The row is `resolve_player_row`'s.
    #[test]
    fn the_highlight_reports_its_lead_over_the_next_brightest_row() {
        let board = board_with_highlight(2);
        let (row, lead) = super::player_row_highlight(&board, 5).expect("highlight");
        assert_eq!(Some(row), resolve_player_row(None, &board, 5));
        assert_eq!(row, 2);
        assert!((lead - 62.0 / 40.0).abs() < 0.01, "lead {lead}");
        // A second lit row (a cursor hover): the same row, a small lead.
        let mut two = board.to_rgb8();
        let slot = crate::ocr::preprocess::row_slot_height(900, 5);
        let top = (900.0 * 0.025) as u32 + 4 * slot;
        for y in top..top + slot {
            for x in 0..1600 {
                two.put_pixel(x, y, Rgb([58, 58, 58]));
            }
        }
        let two = DynamicImage::ImageRgb8(two);
        let (row, lead) = super::player_row_highlight(&two, 5).expect("highlight");
        assert_eq!(Some(row), resolve_player_row(None, &two, 5));
        assert_eq!(row, 2);
        assert!((lead - 62.0 / 58.0).abs() < 0.01, "lead {lead}");
        assert!(lead < crate::detect::own_row::SEED_HIGHLIGHT_LEAD);
        // No lit row: no highlight, as before.
        let flat = DynamicImage::ImageRgb8(RgbImage::from_pixel(1600, 900, Rgb([40, 40, 40])));
        assert_eq!(super::player_row_highlight(&flat, 5), None);
        assert_eq!(resolve_player_row(None, &flat, 5), None);
    }

    #[test]
    fn highlighted_row_survives_a_portrait_below_the_threshold() {
        let board = board_with_highlight(2);
        // The only reference is pure white, so every portrait crop scores far
        // below the trust floor and the portrait path returns nothing.
        let matcher = PortraitMatcher::from_references(HashMap::from([(
            "white".to_string(),
            RgbImage::from_pixel(
                super::PORTRAIT_SIZE,
                super::PORTRAIT_SIZE,
                Rgb([255, 255, 255]),
            ),
        )]));
        let portrait = matcher.match_player_hero_with_team_size(&board, 5);
        assert!(
            portrait.is_none(),
            "portrait should be untrusted: {portrait:?}"
        );

        let row = resolve_player_row(portrait.map(|(_, _, idx)| idx), &board, 5);
        assert_eq!(row, Some(2));
    }

    #[test]
    fn a_trusted_portrait_row_wins_over_the_highlight() {
        let board = board_with_highlight(2);
        assert_eq!(resolve_player_row(Some(4), &board, 5), Some(4));
    }

    #[test]
    fn no_highlight_and_no_portrait_is_no_row() {
        let flat = DynamicImage::ImageRgb8(RgbImage::from_pixel(1600, 900, Rgb([40, 40, 40])));
        assert_eq!(resolve_player_row(None, &flat, 5), None);
    }
}

#[cfg(test)]
mod installed_pack_tests {
    use super::PortraitMatcher;
    use crate::shadow::heroes::test_fixtures::portrait;
    use image::DynamicImage;

    fn save(dir: &std::path::Path, name: &str, id: u32) {
        std::fs::create_dir_all(dir).unwrap();
        portrait(id, 64, Some([20, 20, 30]))
            .save(dir.join(name))
            .unwrap();
    }

    #[test]
    fn a_missing_pack_names_no_hero_even_when_local_pngs_exist() {
        let dir = tempfile::tempdir().unwrap();
        save(&dir.path().join("portraits"), "ana.png", 1);
        let matcher = PortraitMatcher::load_installed(dir.path());
        assert!(matcher.is_empty());
        let crop = DynamicImage::ImageRgba8(portrait(1, 64, Some([20, 20, 30])));
        assert!(matcher.match_portrait(&crop).is_none());
        assert!(matcher.match_player_hero(&crop).is_none());
    }

    #[test]
    fn the_installed_pack_is_the_reference_and_specials_are_not_heroes() {
        let dir = tempfile::tempdir().unwrap();
        let heroes = dir.path().join("templates/heroes");
        save(&heroes, "ana.png", 7);
        save(&heroes, "placeholder.png", 9);
        save(&heroes.join("special"), "skull_01.png", 3);
        let matcher = PortraitMatcher::load_installed(dir.path());
        assert!(matcher.contains("ana"));
        assert!(!matcher.contains("placeholder"));
        assert!(!matcher.contains("skull_01"));
        let crop = DynamicImage::ImageRgba8(portrait(7, 64, Some([20, 20, 30])));
        let (name, conf) = matcher.match_portrait(&crop).expect("pack ana");
        assert_eq!(name, "ana");
        assert!(conf >= 0.85, "{conf}");
    }

    #[test]
    fn a_local_crop_fills_a_stem_the_pack_lacks_and_does_not_replace_one() {
        let dir = tempfile::tempdir().unwrap();
        let heroes = dir.path().join("templates/heroes");
        save(&heroes, "ana.png", 7);
        let portraits = dir.path().join("portraits");
        save(&portraits, "ana.png", 99);
        save(&portraits, "mercy.png", 4);
        // A current geometry marker: without it these files are stale seeds
        // and load moves them aside before the fill.
        std::fs::write(
            portraits.join(super::SEED_GEOMETRY_MARKER),
            format!("{}\n", super::SEED_GEOMETRY_VERSION),
        )
        .unwrap();
        let matcher = PortraitMatcher::load_installed(dir.path());
        assert!(matcher.contains("ana"));
        assert!(matcher.contains("mercy"));
        let pack_ana = DynamicImage::ImageRgba8(portrait(7, 64, Some([20, 20, 30])));
        let (name, _) = matcher.match_portrait(&pack_ana).expect("pack ana wins");
        assert_eq!(name, "ana");
        let local_ana = DynamicImage::ImageRgba8(portrait(99, 64, Some([20, 20, 30])));
        assert!(
            matcher.match_portrait(&local_ana).is_none(),
            "the local ana crop must not override the pack"
        );
    }

    #[test]
    fn the_1080_pack_fills_only_stems_the_main_pack_lacks() {
        let dir = tempfile::tempdir().unwrap();
        save(&dir.path().join("templates/heroes-v2"), "ana.png", 7);
        save(&dir.path().join("templates/heroes-v2-1080"), "ana.png", 99);
        save(&dir.path().join("templates/heroes-v2-1080"), "mercy.png", 4);
        // v2 is used only when it covers v1. With no v1, a v2 set that loads
        // is used as it is.
        let matcher = PortraitMatcher::load_installed(dir.path());
        assert!(matcher.contains("ana"));
        assert!(matcher.contains("mercy"));
        let main = DynamicImage::ImageRgba8(portrait(7, 64, Some([20, 20, 30])));
        assert_eq!(matcher.match_portrait(&main).unwrap().0, "ana");
    }
}

/// Pack parity check for the portrait move. Ignored in CI because the hero
/// pack and real boards are not in the repo. Run locally with:
///
/// ```text
/// SCUFFED_HERO_PACK_DIR=<extracted heroes-v1 pack>
/// SCUFFED_OLD_PORTRAITS_DIR=<portraits/ from the commit before the move>
/// SCUFFED_PROOF_BOARDS_DIR=<scoreboard PNGs>
/// cargo test -p scuffed-stat-tracker pack_parity -- --ignored --nocapture
/// ```
#[cfg(test)]
mod pack_parity_tests {
    use super::*;

    /// Pack files use `soldier-76`, the old bundle used `soldier76`.
    fn norm(stem: &str) -> String {
        stem.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
    }

    fn env_dir(key: &str) -> PathBuf {
        PathBuf::from(std::env::var(key).unwrap_or_else(|_| panic!("set {key}")))
    }

    fn installed_matcher() -> PortraitMatcher {
        let pack = env_dir("SCUFFED_HERO_PACK_DIR");
        let data = tempfile::tempdir().unwrap();
        let dest = crate::shadow::heroes::HeroTemplates::dir_in(data.path());
        std::fs::create_dir_all(&dest).unwrap();
        copy_tree(&pack, &dest);
        let matcher = PortraitMatcher::load_installed(data.path());
        assert!(!matcher.is_empty(), "pack did not load");
        matcher
    }

    fn copy_tree(from: &Path, to: &Path) {
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let p = e.path();
            let t = to.join(e.file_name());
            if p.is_dir() {
                std::fs::create_dir_all(&t).unwrap();
                copy_tree(&p, &t);
            } else {
                std::fs::copy(&p, &t).unwrap();
            }
        }
    }

    /// Every pack portrait, composited into hero slots on the committed
    /// made-up boards at the size the app crops, must come back as itself at
    /// or above [`crate::hero_auth::PORTRAIT_SWITCH_MIN_CONF`]. Skips when
    /// `SCUFFED_HERO_PACK_DIR` is unset (CI has no pack). Nothing is written.
    #[test]
    fn pack_portraits_read_back_from_board_slots() {
        let Ok(pack) = std::env::var("SCUFFED_HERO_PACK_DIR") else {
            eprintln!("SCUFFED_HERO_PACK_DIR not set; skipping pack slot check");
            return;
        };
        let matcher = installed_matcher();
        let mut heroes: Vec<(String, DynamicImage)> = std::fs::read_dir(&pack)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
            .filter_map(|p| {
                let stem = p.file_stem()?.to_str()?.to_string();
                if stem.starts_with('.') || scuffed_types::is_placeholder_hero(&stem) {
                    return None;
                }
                Some((stem, image::open(&p).unwrap()))
            })
            .collect();
        heroes.sort_by(|a, b| a.0.cmp(&b.0));
        // heroes-v1 carries 54 heroes. A short pack is a broken install.
        assert_eq!(
            heroes.len(),
            54,
            "pack at {pack} has {} heroes, want 54",
            heroes.len()
        );
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data/vision-baseline");
        let load = |f: &str| image::open(base.join(f)).unwrap();
        // 5v5: the synthetic board from `report/blank/synth`, generated here
        // (nothing committed), and its 0.75 downscale for 1080p.
        let five_1440 = DynamicImage::ImageRgb8(crate::report::blank::synth::board_5v5(5));
        let five_1080 = five_1440.resize_exact(1248, 756, FilterType::Lanczos3);
        let boards: Vec<(&str, &str, DynamicImage, Option<usize>)> = vec![
            (
                "1440",
                "native_postgame_table_names_blanked.png",
                load("native_postgame_table_names_blanked.png"),
                None,
            ),
            (
                "1080",
                "s075_postgame_table_names_blanked.png",
                load("s075_postgame_table_names_blanked.png"),
                None,
            ),
            (
                "1080",
                "s075_tab_board_stats_only.png",
                load("s075_tab_board_stats_only.png"),
                None,
            ),
            ("1440", "synth board_5v5", five_1440, Some(5)),
            ("1080", "synth board_5v5 x0.75", five_1080, Some(5)),
        ];
        let mut failures = Vec::new();
        for (res, file, board, expect) in boards {
            let detected = detect_team_size(&board);
            // Use the app's detection; if it disagrees with the expected
            // layout, force the expected one through the same slot code.
            let team_size = expect.unwrap_or(detected);
            let forced = team_size != detected;
            let rows: Vec<usize> = if team_size == 5 {
                // Row 9's square ends at y 1014 on a 1007-tall crop, so
                // `portrait_rect` refuses it, as it does in the app.
                vec![0, 4, 5, 8]
            } else {
                vec![0, team_size]
            };
            let (mut pass, mut total, mut min_conf) = (0, 0, f64::MAX);
            for (stem, art) in &heroes {
                for &row in &rows {
                    let r = portrait_rect((board.width(), board.height()), row, team_size)
                        .expect("slot fits the board");
                    let mut composed = board.to_rgba8();
                    let icon = art.resize_exact(r.w, r.h, FilterType::Lanczos3).to_rgba8();
                    image::imageops::overlay(&mut composed, &icon, r.x as i64, r.y as i64);
                    let crop = DynamicImage::ImageRgba8(composed).crop_imm(r.x, r.y, r.w, r.h);
                    total += 1;
                    match matcher.match_portrait(&crop) {
                        Some((got, conf)) if norm(&got) == norm(stem) => {
                            pass += 1;
                            min_conf = min_conf.min(conf);
                        }
                        other => failures.push(format!("{res} {file} row {row} {stem}: {other:?}")),
                    }
                }
            }
            println!(
                "{res} {file} team {team_size} (detected {detected}, forced {forced}) rows {rows:?}: {pass}/{total} pass, min conf {min_conf:.4}"
            );
            if pass > 0 {
                assert!(
                    min_conf >= crate::hero_auth::PORTRAIT_SWITCH_MIN_CONF,
                    "{res} {file}: min conf {min_conf} below the switch floor"
                );
            }
        }
        assert!(failures.is_empty(), "failures: {failures:#?}");
    }

    #[test]
    #[ignore = "needs the hero pack and old portraits on disk"]
    fn pack_parity_every_moved_portrait_loads_from_the_pack() {
        let old = PortraitMatcher::load(&env_dir("SCUFFED_OLD_PORTRAITS_DIR"));
        let new = installed_matcher();
        let have: std::collections::HashSet<String> =
            new.references.keys().map(|k| norm(k)).collect();
        let mut missing: Vec<_> = old
            .references
            .keys()
            .map(|k| norm(k))
            .filter(|k| !have.contains(k))
            .collect();
        missing.sort();
        println!(
            "old {} stems, pack {} stems",
            old.references.len(),
            have.len()
        );
        assert_eq!(old.references.len(), 52);
        assert!(missing.is_empty(), "not in pack: {missing:?}");
    }

    /// Nearest reference per portrait slot, before the trust threshold, so
    /// the comparison does not pass just because both sides return nothing.
    fn nearest_per_slot(m: &PortraitMatcher, board: &DynamicImage) -> Vec<String> {
        extract_portrait_crops(board)
            .iter()
            .map(|crop| {
                let c = crop
                    .resize_exact(PORTRAIT_SIZE, PORTRAIT_SIZE, FilterType::Lanczos3)
                    .to_rgb8();
                m.references
                    .iter()
                    .map(|(k, r)| (norm(k), mean_absolute_difference(&c, r)))
                    .min_by(|x, y| x.1.total_cmp(&y.1))
                    .map(|(k, _)| k)
                    .unwrap_or_default()
            })
            .collect()
    }

    #[test]
    #[ignore = "needs the hero pack, old portraits and proof boards on disk"]
    fn pack_parity_proof_boards_read_the_same() {
        let old = PortraitMatcher::load(&env_dir("SCUFFED_OLD_PORTRAITS_DIR"));
        let new = installed_matcher();
        let mut boards: Vec<_> = std::fs::read_dir(env_dir("SCUFFED_PROOF_BOARDS_DIR"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
            .collect();
        boards.sort();
        assert!(!boards.is_empty());
        let mut diffs = Vec::new();
        for b in &boards {
            let img = image::open(b).unwrap();
            // What callers see: trusted matches and the player hero.
            let trusted = |m: &PortraitMatcher| {
                let all: Vec<String> = m
                    .match_all_portraits(&img)
                    .iter()
                    .map(|(h, _)| norm(h))
                    .collect();
                let me = m.match_player_hero(&img).map(|(h, _, r)| (norm(&h), r));
                (all, me)
            };
            let (a, n) = (trusted(&old), trusted(&new));
            // Below the trust threshold, for information only.
            let (na, nn) = (nearest_per_slot(&old, &img), nearest_per_slot(&new, &img));
            let same_nearest = na.iter().zip(&nn).filter(|(x, y)| x == y).count();
            println!(
                "{}: trusted old {a:?} pack {n:?}; untrusted nearest agree {same_nearest}/{}",
                b.file_name().unwrap().to_string_lossy(),
                na.len()
            );
            if a != n {
                diffs.push(b.clone());
            }
        }
        println!("{} boards, {} differ", boards.len(), diffs.len());
        assert!(diffs.is_empty(), "differ: {diffs:?}");
    }
}
