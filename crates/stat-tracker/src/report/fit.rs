//! What a report leaves out to stay within the site's limits
//! (`report-bundle.md` sections 3 and 8), and the preview line for each.
//!
//! Section 8 allows at most [`super::bundle::MAX_FILES`] files besides the
//! manifest. The count is checked before the zip is built. Over it, the
//! rank crops go first, then glyph crops two at a time, those from the name
//! seen on the latest frame first. The log, the scoreboard, the outcome
//! crop and an own-name crop are never left out to fit the count.

use super::bundle::ReportError;

/// Why a file was left out of a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftOutReason {
    /// The file alone is over its own size cap (a rank crop over 2 MiB).
    OverSizeLimit,
    /// The file would push the report over the 10 MB limit.
    ReportTooBig,
    /// The report already holds as many files as the site takes.
    TooManyFiles,
    /// The saved picture could not be read, or is too large on a side.
    NotReadable,
}

impl LeftOutReason {
    const fn text(self) -> &'static str {
        match self {
            Self::OverSizeLimit => "over the size limit.",
            Self::ReportTooBig => "the report would be over 10 MB.",
            Self::TooManyFiles => "too many files for one report.",
            Self::NotReadable => "the picture could not be read.",
        }
    }
}

/// One file the report would have carried but left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    /// The zip path the file would have had.
    pub path: String,
    pub reason: LeftOutReason,
}

impl LeftOut {
    /// The one plain line the preview shows for this file, such as
    /// "Rank screen left out: over the size limit."
    pub fn preview_line(&self) -> String {
        let what = match self.path.as_str() {
            "crops/rank.png" => "Rank screen",
            "crops/rank_banner.png" => "Rank banner",
            _ => "A name letter",
        };
        format!("{what} left out: {}", self.reason.text())
    }
}

/// What a file is, for the section 8 file count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountKind {
    /// The log, the scoreboard, the outcome crop or an own-name crop:
    /// never left out to fit the count.
    Kept,
    /// `crops/rank.png` or `crops/rank_banner.png`: left out first.
    Rank,
    /// A glyph crop. `name` tells apart the names the glyphs were cut from
    /// (in memory only, never written). `last_seen_frame` is the latest
    /// frame that name was seen on; a higher number is a later frame.
    Glyph { name: u32, last_seen_frame: u64 },
}

/// The files to leave out so that `kinds` fits `max`, as indices into
/// `kinds`, in the order they are left out. Empty when it already fits.
///
/// Rank crops go first (in their given order), then glyph crops two at a
/// time: by name, the name with the latest `last_seen_frame` first (ties
/// by the lower `name`), each name's glyphs in their given order. A
/// [`CountKind::Kept`] file is never picked. Errs only when the kept files
/// alone are over `max`, which a report never has.
pub fn count_drops(kinds: &[CountKind], max: usize) -> Result<Vec<usize>, ReportError> {
    let mut over = kinds.len().saturating_sub(max);
    if over == 0 {
        return Ok(Vec::new());
    }
    let mut order: Vec<usize> = (0..kinds.len())
        .filter(|&i| kinds[i] == CountKind::Rank)
        .collect();
    let mut glyphs: Vec<(u64, u32, usize)> = kinds
        .iter()
        .enumerate()
        .filter_map(|(i, k)| match *k {
            CountKind::Glyph {
                name,
                last_seen_frame,
            } => Some((last_seen_frame, name, i)),
            _ => None,
        })
        .collect();
    // Latest frame first; then lower name; then the given order.
    glyphs.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let rank_count = order.len();
    order.extend(glyphs.into_iter().map(|(_, _, i)| i));

    let mut drops = Vec::new();
    let mut at = 0;
    while over > 0 && at < order.len() {
        // Rank crops one at a time, glyph crops two at a time.
        let step = if at < rank_count { 1 } else { 2 };
        for &i in order.iter().skip(at).take(step) {
            drops.push(i);
        }
        over = over.saturating_sub(step.min(order.len() - at));
        at += step;
    }
    if over > 0 {
        return Err(ReportError::InvalidInput("a report holds at most 30 files"));
    }
    Ok(drops)
}
