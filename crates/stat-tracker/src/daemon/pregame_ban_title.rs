//! The ban-results title check for the opt-in pre-game saves, on Vision's
//! title region: screen fractions of the 16:9 game area (x0, y0, x1, y1) =
//! (0.43, 0.05, 0.575, 0.12).
//!
//! Two ways to tell, one of them in use:
//!
//! * With no example installed, a shape rule on the grey region
//!   ([`SHAPE`]): a band of bright title text, centred and wide, on a dark
//!   ground, clear of the region's top and bottom edges. Vision measured
//!   it at 8 of 8 on real shots, also downscaled to 1080p and 720p.
//! * With examples in `{data_dir}/templates/pregame/ban_results/` (full
//!   screenshots of the ban-results screen, `.png` only, read when the
//!   tracker starts), a small grey cut of the region must match one of
//!   them by normalized correlation. Vision matched such cuts at 0.6 and
//!   was right on 2 of 2 real shots; the threshold here is stricter
//!   because two samples are no calibration. Examples REPLACE the shape
//!   rule: with any example loaded, the shape rule is not used at all.
//!
//! No real crop lives in the repo. Either way a frame that does not match
//! is never saved as the ban results, and a match is only the candidate:
//! it is confirmed by one more non-phase frame (`pregame_dump`).

/// The title region, as fractions of the 16:9 game area.
pub(crate) const BAN_RESULTS_TITLE: [f32; 4] = [0.43, 0.05, 0.575, 0.12];

/// The grey cut's size. The region is about 3.7:1 at 16:9; the cut is
/// squashed the same way for examples and frames, so they compare alike.
const CUT_W: u32 = 46;
const CUT_H: u32 = 10;

/// Normalized correlation of the cuts at or above this is a match.
/// Vision's 0.6, raised because only two real samples back it.
pub(crate) const BAN_TITLE_MATCH_MIN: f32 = 0.7;

/// A cut whose grey levels spread less than this (standard deviation, of
/// 255) is a flat area with no title on it, and matches nothing.
const CUT_SPREAD_MIN: f32 = 6.0;

/// At most this many example files are read, counted among the regular
/// `.png` files (by name) after the others are left out.
pub(crate) const MAX_EXAMPLES: usize = 16;

/// An example file larger than this is skipped unread.
pub(crate) const MAX_EXAMPLE_BYTES: u64 = 32 * 1024 * 1024;

/// An example wider or taller than this (8K) is skipped before decoding.
pub(crate) const MAX_EXAMPLE_SIDE: u32 = 8192;

/// An example with more pixels than this (8K UHD, 7680 x 4320) is skipped
/// before decoding; the side cap alone allows 8192 x 8192.
pub(crate) const MAX_EXAMPLE_PIXELS: u64 = 7680 * 4320;

/// The template-free shape rule. A STARTING RULE, from Vision's real shots:
/// two English positives (about 30% bright, 63% dark, 99% of the width),
/// the ban vote (2.6% bright, 42% width) and the hero select and roster (no
/// bright pixel) as negatives; 8 of 8 right, also downscaled to 1080p and
/// 720p. Untested on colourblind filters and on real 1080p captures, so
/// expect to retune it once saved frames exist. All its numbers live here.
pub(crate) struct ShapeRule {
    /// A grey level above this is bright (title text).
    pub(crate) bright_above: u8,
    /// A grey level below this is dark (the ground).
    pub(crate) dark_below: u8,
    /// Share of bright pixels, inclusive.
    pub(crate) bright_min: f32,
    pub(crate) bright_max: f32,
    /// At least this share of dark pixels.
    pub(crate) dark_min: f32,
    /// The bright pixels' column centroid, as a share of the width.
    pub(crate) centroid_min: f32,
    pub(crate) centroid_max: f32,
    /// Columns holding at least one bright pixel, counted, as a share of
    /// the width (a stray pixel near each edge adds two columns, not the
    /// whole width).
    pub(crate) span_min: f32,
    /// The top tenth and the bottom tenth of rows each hold at most this
    /// share of bright pixels.
    pub(crate) edge_bright_max: f32,
}

pub(crate) const SHAPE: ShapeRule = ShapeRule {
    bright_above: 200,
    dark_below: 60,
    bright_min: 0.12,
    bright_max: 0.55,
    dark_min: 0.40,
    centroid_min: 0.40,
    centroid_max: 0.60,
    span_min: 0.60,
    edge_bright_max: 0.05,
};

/// What the shape rule measures on the grey title region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ShapeMetrics {
    pub(crate) bright: f32,
    pub(crate) dark: f32,
    /// `None` with no bright pixel.
    pub(crate) centroid: Option<f32>,
    pub(crate) span: f32,
    pub(crate) top_bright: f32,
    pub(crate) bottom_bright: f32,
}

impl ShapeMetrics {
    /// Every condition of [`SHAPE`].
    pub(crate) fn passes(&self) -> bool {
        let r = &SHAPE;
        (r.bright_min..=r.bright_max).contains(&self.bright)
            && self.dark >= r.dark_min
            && self
                .centroid
                .is_some_and(|c| (r.centroid_min..=r.centroid_max).contains(&c))
            && self.span >= r.span_min
            && self.top_bright <= r.edge_bright_max
            && self.bottom_bright <= r.edge_bright_max
    }
}

/// Measure [`SHAPE`] on `rgb`'s title region, at full resolution; `None`
/// when the region is too small to have edge rows.
pub(crate) fn shape_metrics(rgb: &image::RgbImage) -> Option<ShapeMetrics> {
    let (x0, y0, rw, rh) = title_rect(rgb.width(), rgb.height())?;
    if rh < 10 || rw < 10 {
        return None;
    }
    let edge_rows = rh / 10;
    let mut cols = vec![0u32; rw as usize];
    let (mut bright, mut dark, mut top, mut bottom) = (0u32, 0u32, 0u32, 0u32);
    for dy in 0..rh {
        for dx in 0..rw {
            let [r, g, b] = rgb.get_pixel(x0 + dx, y0 + dy).0;
            let grey = luma(r, g, b);
            if grey > SHAPE.bright_above {
                bright += 1;
                cols[dx as usize] += 1;
                if dy < edge_rows {
                    top += 1;
                }
                if dy >= rh - edge_rows {
                    bottom += 1;
                }
            } else if grey < SHAPE.dark_below {
                dark += 1;
            }
        }
    }
    let total = (rw * rh) as f32;
    let edge_total = (rw * edge_rows) as f32;
    let centroid = (bright > 0).then(|| {
        let weighted: f64 = cols
            .iter()
            .enumerate()
            .map(|(x, &n)| (x as f64 + 0.5) * f64::from(n))
            .sum();
        (weighted / f64::from(bright) / f64::from(rw)) as f32
    });
    let span = cols.iter().filter(|&&n| n > 0).count() as f32 / rw as f32;
    Some(ShapeMetrics {
        bright: bright as f32 / total,
        dark: dark as f32 / total,
        centroid,
        span,
        top_bright: top as f32 / edge_total,
        bottom_bright: bottom as f32 / edge_total,
    })
}

/// The grey level of a pixel, with the Rec. 709 weights `image` uses.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b)) / 10000) as u8
}

/// Where the examples are read from, under the data dir.
pub(crate) fn ban_title_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir
        .join("templates")
        .join("pregame")
        .join("ban_results")
}

/// The ban-results check: the examples' grey cuts (zero-mean, unit
/// length), or the shape rule when there are none.
#[derive(Debug, Default)]
pub(crate) struct BanTitleExamples {
    cuts: Vec<Vec<f32>>,
}

impl BanTitleExamples {
    /// Read the `.png` files in `dir` (any case): first only regular files
    /// are kept (no symlink, and no FIFO that would block), then the first
    /// [`MAX_EXAMPLES`] of them by name are read. Files over
    /// [`MAX_EXAMPLE_BYTES`], with a side over [`MAX_EXAMPLE_SIDE`] or with
    /// more than [`MAX_EXAMPLE_PIXELS`] are skipped before decoding, and
    /// only the title region is converted. Each skipped file is logged at
    /// info with why. A missing folder is no examples.
    pub(crate) fn load_dir(dir: &std::path::Path) -> Self {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Self::default();
        };
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let png = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("png"));
            if !png {
                continue;
            }
            // `DirEntry::file_type` does not follow symlinks.
            if entry.file_type().is_ok_and(|t| t.is_file()) {
                paths.push(path);
            } else {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    path = %path.display(),
                    why = "not a regular file",
                    "ban-results example not used"
                );
            }
        }
        paths.sort();
        if paths.len() > MAX_EXAMPLES {
            tracing::info!(
                target: "scuffed_stat_tracker",
                found = paths.len(),
                kept = MAX_EXAMPLES,
                "ban-results examples: only the first regular .png files by name are read"
            );
            paths.truncate(MAX_EXAMPLES);
        }
        let mut cuts = Vec::new();
        for path in &paths {
            match example_cut(path) {
                Ok(cut) => cuts.push(cut),
                Err(why) => tracing::info!(
                    target: "scuffed_stat_tracker",
                    path = %path.display(),
                    why,
                    "ban-results example not used"
                ),
            }
        }
        Self { cuts }
    }

    #[cfg(test)]
    pub(crate) fn from_frames(frames: impl IntoIterator<Item = image::RgbImage>) -> Self {
        Self {
            cuts: frames.into_iter().filter_map(|f| frame_cut(&f)).collect(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.cuts.len()
    }

    /// The best correlation of `rgb`'s cut with an example; `None` with no
    /// example or a flat cut.
    pub(crate) fn best_score(&self, rgb: &image::RgbImage) -> Option<f32> {
        if self.cuts.is_empty() {
            return None;
        }
        let cut = frame_cut(rgb)?;
        self.cuts
            .iter()
            .map(|ex| ex.iter().zip(&cut).map(|(a, b)| a * b).sum::<f32>())
            .reduce(f32::max)
    }

    /// Whether the ban-results title is on `rgb`: an example match when
    /// there are examples, the shape rule otherwise. The score or the
    /// shape is logged at debug, to tune the rule from real runs, and so
    /// are the numbers of a shape that passes: the title is looked for on
    /// every poll while the ban results wait, so a pass comes once a poll.
    pub(crate) fn matches(&self, rgb: &image::RgbImage) -> bool {
        if self.cuts.is_empty() {
            let shape = shape_metrics(rgb);
            let pass = shape.is_some_and(|m| m.passes());
            tracing::debug!(target: "scuffed_stat_tracker", ?shape, pass, "ban-results title shape");
            if let Some(m) = shape.filter(|_| pass) {
                tracing::debug!(
                    target: "scuffed_stat_tracker",
                    bright = m.bright,
                    dark = m.dark,
                    centroid = m.centroid,
                    span = m.span,
                    top = m.top_bright,
                    bottom = m.bottom_bright,
                    "ban-results title passed the shape rule"
                );
            }
            return pass;
        }
        let score = self.best_score(rgb);
        let pass = score.is_some_and(|s| s >= BAN_TITLE_MATCH_MIN);
        tracing::debug!(target: "scuffed_stat_tracker", ?score, pass, "ban-results title score");
        pass
    }
}

/// One example file's grey cut, or why it was not used.
pub(crate) fn example_cut(path: &std::path::Path) -> Result<Vec<f32>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > MAX_EXAMPLE_BYTES {
        return Err(format!("larger than {MAX_EXAMPLE_BYTES} bytes"));
    }
    let (w, h) = image::image_dimensions(path).map_err(|e| e.to_string())?;
    if w > MAX_EXAMPLE_SIDE || h > MAX_EXAMPLE_SIDE {
        return Err(format!("{w}x{h} is over {MAX_EXAMPLE_SIDE} px"));
    }
    if u64::from(w) * u64::from(h) > MAX_EXAMPLE_PIXELS {
        return Err(format!("{w}x{h} is over {MAX_EXAMPLE_PIXELS} pixels"));
    }
    let img = image::open(path).map_err(|e| e.to_string())?;
    let (x, y, rw, rh) = title_rect(w, h).ok_or("no title region")?;
    grey_cut(&img.crop_imm(x, y, rw, rh).to_luma8())
        .ok_or_else(|| "the title region is flat (no title on it)".into())
}

/// A frame's grey cut; `None` when the region is empty or flat.
fn frame_cut(rgb: &image::RgbImage) -> Option<Vec<f32>> {
    let (x, y, w, h) = title_rect(rgb.width(), rgb.height())?;
    let region = image::imageops::crop_imm(rgb, x, y, w, h).to_image();
    grey_cut(&image::DynamicImage::ImageRgb8(region).to_luma8())
}

/// The title region of a `w` x `h` frame's 16:9 game area, in pixels;
/// `None` when it is empty.
fn title_rect(w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let (gx, gy, gw, gh) = stat_tracker::frame::geometry::game_rect_16_9(w, h);
    let [fx0, fy0, fx1, fy1] = BAN_RESULTS_TITLE;
    let x0 = gx + (gw as f32 * fx0) as u32;
    let y0 = gy + (gh as f32 * fy0) as u32;
    let x1 = (gx + (gw as f32 * fx1) as u32).min(w);
    let y1 = (gy + (gh as f32 * fy1) as u32).min(h);
    (x1 > x0 && y1 > y0).then(|| (x0, y0, x1 - x0, y1 - y0))
}

/// A grey title region at [`CUT_W`] x [`CUT_H`], zero-mean and unit
/// length; `None` when it is flat.
fn grey_cut(grey: &image::GrayImage) -> Option<Vec<f32>> {
    let small = image::imageops::resize(grey, CUT_W, CUT_H, image::imageops::FilterType::Triangle);
    let px: Vec<f32> = small.pixels().map(|p| f32::from(p.0[0])).collect();
    let n = px.len() as f32;
    let mean = px.iter().sum::<f32>() / n;
    let centred: Vec<f32> = px.iter().map(|v| v - mean).collect();
    let norm = centred.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm / n.sqrt() < CUT_SPREAD_MIN {
        return None;
    }
    Some(centred.iter().map(|v| v / norm).collect())
}
