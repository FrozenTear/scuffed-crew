//! The `names-v1` pack: letter templates for [`super::read_row_name`].
//!
//! Installed layout, under `<data_dir>/templates/names-v1/`:
//!
//! | file | content |
//! |---|---|
//! | `manifest.json` | `name` `names-v1.tar`, `format` 1, `version`, `canvas` `[16, 28]`, `mark_canvas` `[16, 8]`, `glyphs`, `marks`, optional `params` (each falls back to [`Params::DEFAULT`]), and `files`: `size` and `sha256` of every file below |
//! | `glyphs.png` | 8-bit gray, 64 tiles a row, one 16x28 letter tile each, in `glyphs.txt` order |
//! | `glyphs.txt` | one line per tile: the letter, a tab, its width over the cap height |
//! | `marks.png` | 8-bit gray, 64 tiles a row, one 16x8 accent tile each |
//! | `marks.txt` | one line per mark tile: `U+XXXX` (combining mark), a tab, its part count |
//! | `compose.txt` | `base`, tab, `U+XXXX`, tab, the composed letter: the only accented letters the reader may give |
//! | `whiten.f32` | 448 x 448 little-endian f32, row-major: the whitening applied to a tile before matching |
//!
//! The pack holds single letters cut from boards, never a whole name.
//! Anything missing, unreadable or not matching its manifest refuses the
//! whole pack ([`PackError`]); the caller then reads names the ocr-v1 way.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const PACK_NAME: &str = "names-v1.tar";
pub const PACK_SUBDIR: &str = "names-v1";
pub const FORMAT: u64 = 1;
pub const CANVAS: (usize, usize) = (16, 28);
pub const MARK_CANVAS: (usize, usize) = (16, 8);
pub const DIM: usize = CANVAS.0 * CANVAS.1;
pub const MARK_DIM: usize = MARK_CANVAS.0 * MARK_CANVAS.1;
const TILES_PER_ROW: usize = 64;
const FILES: [&str; 6] = [
    "glyphs.png",
    "glyphs.txt",
    "marks.png",
    "marks.txt",
    "compose.txt",
    "whiten.f32",
];

/// Where the pack is installed under the tracker's data dir.
pub fn dir_in(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join(PACK_SUBDIR)
}

#[derive(Debug, Clone, PartialEq)]
pub enum PackError {
    /// No pack folder: names are read the ocr-v1 way.
    Missing,
    /// A pack folder that cannot be used. The text names what is wrong.
    Invalid(String),
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::Missing => f.write_str("no names-v1 pack"),
            PackError::Invalid(why) => write!(f, "names-v1 pack refused: {why}"),
        }
    }
}

impl std::error::Error for PackError {}

/// Reader settings carried by the pack, so they can change without a release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// Weight of the width term in a letter's score.
    pub rw: f32,
    /// Italic slant used to straighten a glyph before splitting or measuring a gap.
    pub slant: f64,
    /// Lowest cosine an accent may have to its nearest mark tile.
    pub mark_min: f32,
    /// Lowest lead of the best mark over the next one.
    pub mark_margin: f32,
    /// A gap (over the cap height) in `[space_lo, space_hi)` makes the row abstain;
    /// `space_hi` and wider is a space.
    pub space_lo: f64,
    pub space_hi: f64,
    /// Lowest confidence (smallest best-vs-second letter margin) a read may have.
    pub min_confidence: f32,
}

impl Params {
    /// What the reader uses when the pack does not say: a gap of 0.25 to
    /// 0.40 cap heights makes the row abstain, 0.40 and wider is a space,
    /// and a read needs a margin of at least 0.016 (on the labelled boards
    /// 0.0125 fills more names but reads some wrong; a wrong name is worse
    /// than none).
    pub const DEFAULT: Params = Params {
        rw: 0.15,
        slant: 0.22,
        mark_min: 0.75,
        mark_margin: 0.05,
        space_lo: 0.25,
        space_hi: 0.40,
        min_confidence: 0.016,
    };
}

/// A loaded, checked pack.
pub struct NamePack {
    pub version: String,
    pub params: Params,
    /// Whitened, unit-length templates, `DIM` floats each.
    pub(crate) z: Vec<f32>,
    pub(crate) width: Vec<f32>,
    /// Class index of each template into `classes`.
    pub(crate) class: Vec<u16>,
    pub(crate) classes: Vec<char>,
    pub(crate) whiten: Vec<f32>,
    pub(crate) marks: Vec<(Vec<f32>, u8, char)>,
    pub(crate) compose: BTreeMap<(char, char), char>,
}

impl std::fmt::Debug for NamePack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamePack")
            .field("version", &self.version)
            .field("templates", &self.width.len())
            .field("classes", &self.classes.len())
            .field("marks", &self.marks.len())
            .finish()
    }
}

fn bad(why: impl Into<String>) -> PackError {
    PackError::Invalid(why.into())
}

fn tiles(png: &[u8], n: usize, (w, h): (usize, usize)) -> Result<Vec<Vec<u8>>, PackError> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| bad(format!("png: {e}")))?;
    let image::DynamicImage::ImageLuma8(img) = img else {
        return Err(bad("png is not 8-bit gray"));
    };
    let rows = n.div_ceil(TILES_PER_ROW);
    if img.width() as usize != TILES_PER_ROW * w || (img.height() as usize) < rows * h {
        return Err(bad("png size does not fit the tile count"));
    }
    let stride = img.width() as usize;
    let raw = img.as_raw();
    Ok((0..n)
        .map(|i| {
            let (r, c) = (i / TILES_PER_ROW, i % TILES_PER_ROW);
            let mut t = Vec::with_capacity(w * h);
            for y in 0..h {
                let at = (r * h + y) * stride + c * w;
                t.extend_from_slice(&raw[at..at + w]);
            }
            t
        })
        .collect())
}

fn mark_char(s: &str) -> Result<char, PackError> {
    let hex = s.strip_prefix("U+").ok_or_else(|| bad("mark code"))?;
    u32::from_str_radix(hex, 16)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| bad("mark code"))
}

fn one_char(s: &str) -> Result<char, PackError> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Ok(c),
        _ => Err(bad("expected one letter")),
    }
}

impl NamePack {
    /// Load and check the pack in `dir` (the installed folder itself).
    pub fn load(dir: &Path) -> Result<NamePack, PackError> {
        if !dir.is_dir() {
            return Err(PackError::Missing);
        }
        let raw = std::fs::read(dir.join("manifest.json")).map_err(|_| bad("no manifest.json"))?;
        let m: serde_json::Value =
            serde_json::from_slice(&raw).map_err(|_| bad("manifest.json is not JSON"))?;
        if m["name"] != PACK_NAME || m["format"].as_u64() != Some(FORMAT) {
            return Err(bad("not a names-v1 format 1 pack"));
        }
        if m["canvas"] != serde_json::json!([CANVAS.0, CANVAS.1])
            || m["mark_canvas"] != serde_json::json!([MARK_CANVAS.0, MARK_CANVAS.1])
        {
            return Err(bad("tile size"));
        }
        let mut files = BTreeMap::new();
        for f in FILES {
            let want = &m["files"][f];
            let bytes = std::fs::read(dir.join(f)).map_err(|_| bad(format!("missing {f}")))?;
            if want["size"].as_u64() != Some(bytes.len() as u64) {
                return Err(bad(format!("{f} size")));
            }
            let hash = format!("{:x}", Sha256::digest(&bytes));
            if want["sha256"].as_str() != Some(hash.as_str()) {
                return Err(bad(format!("{f} sha256")));
            }
            files.insert(f, bytes);
        }
        // Every setting has a default; a pack may carry its own.
        let p = &m["params"];
        let num = |k: &str, default: f64| match &p[k] {
            serde_json::Value::Null => Ok(default),
            v => v
                .as_f64()
                .filter(|x| x.is_finite())
                .ok_or_else(|| bad(format!("params.{k}"))),
        };
        let d = Params::DEFAULT;
        let params = Params {
            rw: num("rw", d.rw.into())? as f32,
            slant: num("slant", d.slant)?,
            mark_min: num("mark_min", d.mark_min.into())? as f32,
            mark_margin: num("mark_margin", d.mark_margin.into())? as f32,
            space_lo: num("space_lo", d.space_lo)?,
            space_hi: num("space_hi", d.space_hi)?,
            min_confidence: num("min_confidence", d.min_confidence.into())? as f32,
        };
        if params.space_lo > params.space_hi {
            return Err(bad("params.space_lo above params.space_hi"));
        }
        let text = |f: &str| {
            std::str::from_utf8(&files[f])
                .map(str::to_owned)
                .map_err(|_| bad(format!("{f} is not UTF-8")))
        };
        // letters
        let mut labels = Vec::new();
        let mut width = Vec::new();
        for line in text("glyphs.txt")?.lines() {
            let (c, r) = line
                .split_once('\t')
                .ok_or_else(|| bad("glyphs.txt line"))?;
            labels.push(one_char(c)?);
            width.push(r.parse::<f32>().map_err(|_| bad("glyphs.txt width"))?);
        }
        if m["glyphs"].as_u64() != Some(labels.len() as u64) || labels.is_empty() {
            return Err(bad("glyph count"));
        }
        let wb = &files["whiten.f32"];
        if wb.len() != DIM * DIM * 4 {
            return Err(bad("whiten.f32 size"));
        }
        let whiten: Vec<f32> = wb
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        if whiten.iter().any(|v| !v.is_finite()) {
            return Err(bad("whiten.f32 not finite"));
        }
        let mut classes: Vec<char> = labels.clone();
        classes.sort_unstable();
        classes.dedup();
        if classes.len() < 2 || classes.len() > u16::MAX as usize {
            return Err(bad("class count"));
        }
        let class = labels
            .iter()
            .map(|c| classes.binary_search(c).unwrap() as u16)
            .collect();
        let mut z = Vec::with_capacity(labels.len() * DIM);
        for t in tiles(&files["glyphs.png"], labels.len(), CANVAS)? {
            z.extend(super::reader::whitened(&super::reader::unit(&t), &whiten));
        }
        // marks
        let mut mark_meta = Vec::new();
        for line in text("marks.txt")?.lines() {
            let (c, n) = line.split_once('\t').ok_or_else(|| bad("marks.txt line"))?;
            mark_meta.push((
                mark_char(c)?,
                n.parse::<u8>().map_err(|_| bad("marks.txt parts"))?,
            ));
        }
        if m["marks"].as_u64() != Some(mark_meta.len() as u64) {
            return Err(bad("mark count"));
        }
        let marks = if mark_meta.is_empty() {
            Vec::new()
        } else {
            tiles(&files["marks.png"], mark_meta.len(), MARK_CANVAS)?
                .iter()
                .zip(&mark_meta)
                .map(|(t, &(c, n))| (super::reader::unit(t), n, c))
                .collect()
        };
        let mut compose = BTreeMap::new();
        for line in text("compose.txt")?.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let [b, mk, c] = f[..] else {
                return Err(bad("compose.txt line"));
            };
            compose.insert((one_char(b)?, mark_char(mk)?), one_char(c)?);
        }
        Ok(NamePack {
            version: m["version"].as_str().unwrap_or("").to_owned(),
            params,
            z,
            width,
            class,
            classes,
            whiten,
            marks,
            compose,
        })
    }

    /// Bytes held by the loaded pack (templates, whitening, marks).
    pub fn resident_bytes(&self) -> usize {
        self.z.len() * 4
            + self.width.len() * 4
            + self.class.len() * 2
            + self.whiten.len() * 4
            + self.marks.iter().map(|m| m.0.len() * 4 + 8).sum::<usize>()
            + self.compose.len() * 12
    }

    pub fn templates(&self) -> usize {
        self.width.len()
    }
}
