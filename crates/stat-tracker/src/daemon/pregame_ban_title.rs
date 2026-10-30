//! The ban-results title check for the opt-in pre-game saves: a small grey
//! cut of the title region, matched against example screenshots of the
//! ban-results screen kept on this PC.
//!
//! The region is Vision's, as screen fractions of the 16:9 game area
//! (x0, y0, x1, y1) = (0.43, 0.05, 0.575, 0.12). Vision matched small grey
//! cuts of it against saved examples at 0.6 and was right on 2 of 2 real
//! shots. Only those 2 samples exist, and no real crop may live in the
//! repo, so the examples are read from
//! `{data_dir}/templates/pregame/ban_results/` (full screenshots of the
//! ban-results screen, any size, PNG) when the tracker starts. With no
//! example there nothing matches, and no `ban_results.png` is written: a
//! missing file beats a wrong one. The threshold is stricter than
//! Vision's 0.6 for the same reason: two samples are no calibration.

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

/// Where the examples are read from, under the data dir.
pub(crate) fn ban_title_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir
        .join("templates")
        .join("pregame")
        .join("ban_results")
}

/// The examples' grey cuts, zero-mean and unit length.
#[derive(Debug, Default)]
pub(crate) struct BanTitleExamples {
    cuts: Vec<Vec<f32>>,
}

impl BanTitleExamples {
    /// Read every PNG in `dir`; a file that does not decode, or whose cut is
    /// flat, is skipped. A missing folder is no examples.
    pub(crate) fn load_dir(dir: &std::path::Path) -> Self {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Self::default();
        };
        let mut paths: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("png"))
            })
            .collect();
        paths.sort();
        let frames = paths.iter().filter_map(|p| match image::open(p) {
            Ok(img) => Some(img.to_rgb8()),
            Err(e) => {
                tracing::debug!(target: "scuffed_stat_tracker", error = %e, path = %p.display(), "ban-results example not read");
                None
            }
        });
        Self::from_frames(frames)
    }

    pub(crate) fn from_frames(frames: impl IntoIterator<Item = image::RgbImage>) -> Self {
        Self {
            cuts: frames.into_iter().filter_map(|f| grey_cut(&f)).collect(),
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
        let cut = grey_cut(rgb)?;
        self.cuts
            .iter()
            .map(|ex| ex.iter().zip(&cut).map(|(a, b)| a * b).sum::<f32>())
            .reduce(f32::max)
    }

    /// Whether the ban-results title is on `rgb`.
    pub(crate) fn matches(&self, rgb: &image::RgbImage) -> bool {
        self.best_score(rgb)
            .is_some_and(|score| score >= BAN_TITLE_MATCH_MIN)
    }
}

/// The title region of the 16:9 game area, grey, at [`CUT_W`] x [`CUT_H`],
/// zero-mean and unit length; `None` when it is flat or empty.
fn grey_cut(rgb: &image::RgbImage) -> Option<Vec<f32>> {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = stat_tracker::frame::geometry::game_rect_16_9(w, h);
    let [fx0, fy0, fx1, fy1] = BAN_RESULTS_TITLE;
    let x0 = gx + (gw as f32 * fx0) as u32;
    let y0 = gy + (gh as f32 * fy0) as u32;
    let x1 = (gx + (gw as f32 * fx1) as u32).min(w);
    let y1 = (gy + (gh as f32 * fy1) as u32).min(h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let region = image::imageops::crop_imm(rgb, x0, y0, x1 - x0, y1 - y0).to_image();
    let grey = image::DynamicImage::ImageRgb8(region).to_luma8();
    let small = image::imageops::resize(&grey, CUT_W, CUT_H, image::imageops::FilterType::Triangle);
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
