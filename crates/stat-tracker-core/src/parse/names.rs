//! Finding the player's own row by name across a row's name windows.

use crate::model::RowOcrResult;
use strsim::normalized_levenshtein;

/// Fuzzy score a name read needs to count as the player's name.
const NAME_MATCH_MIN: f64 = 0.55;

/// The row that matched the configured player name, and which name window
/// matched it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameMatch {
    /// Row index across both teams.
    pub row: usize,
    /// Index into `ocr::preprocess::name_windows` of the read that
    /// matched: a substring hit if there is one, else the best fuzzy read
    /// (the lower index on a tie).
    pub window: usize,
    /// No other window's read on that row matched. On 5v5 the windows belong
    /// to different table layouts, so a sole match also names the layout.
    pub sole_window: bool,
}

/// Find which row (across all rows, both teams) best matches the configured
/// player name. Returns the row index.
///
/// Used for replay and post-match screens where the player may be on team 2,
/// so brightness-based team-1 scanning can't find them. The name cells from
/// `recognize_row` are noisy OCR, so we use a generous fuzzy threshold (0.55)
/// and pick the best score across all rows. See [`match_player_row_by_name`]
/// for the matching window.
pub fn find_player_row_by_name(rows: &[RowOcrResult], player_name: &str) -> Option<usize> {
    match_player_row_by_name(rows, player_name).map(|m| m.row)
}

/// Like [`find_player_row_by_name`], and also reports which name window
/// matched.
///
/// Each row scores as its best read over every name window (5v5 reads one
/// per table layout). The first row, in board order, with a substring hit in
/// any window wins outright; otherwise the best fuzzy row at or above 0.55.
pub fn match_player_row_by_name(rows: &[RowOcrResult], player_name: &str) -> Option<NameMatch> {
    let name_lower = player_name.to_lowercase();

    let mut best: Option<(usize, f64)> = None;

    for (i, row) in rows.iter().enumerate() {
        let scores = row_window_scores(row, &name_lower);
        if let Some(&(window, _)) = scores.iter().find(|(_, s)| s.is_substring()) {
            // `name` so the report log scrub drops the field (report-bundle §6).
            tracing::debug!(row = i, window, "player name found via substring in row");
            return Some(name_match(i, window, &scores));
        }
        let row_best = scores
            .iter()
            .filter_map(|(_, s)| s.fuzzy())
            .fold(None, |acc: Option<f64>, s| {
                Some(acc.map_or(s, |a| a.max(s)))
            });
        if let Some(score) = row_best
            && best.is_none_or(|(_, b)| score > b)
        {
            best = Some((i, score));
        }
    }

    let (row, score) = best?;
    if score < NAME_MATCH_MIN {
        return None;
    }
    let scores = row_window_scores(&rows[row], &name_lower);
    let window = scores
        .iter()
        .filter(|(_, s)| s.fuzzy() == Some(score))
        .map(|(w, _)| *w)
        .min()
        .unwrap_or(0);
    tracing::debug!(row, window, score, "player name fuzzy-matched in row");
    Some(name_match(row, window, &scores))
}

fn name_match(row: usize, window: usize, scores: &[(usize, NameRead)]) -> NameMatch {
    let sole_window = !scores
        .iter()
        .any(|(w, s)| *w != window && s.passes(NAME_MATCH_MIN));
    NameMatch {
        row,
        window,
        sole_window,
    }
}

/// `(window index, score)` for every non-empty name read of `row`. `name` is
/// window 0 and `name_alternates[k]` is window `k + 1`.
fn row_window_scores(row: &RowOcrResult, name_lower: &str) -> Vec<(usize, NameRead)> {
    std::iter::once(row.name.as_ref())
        .chain(row.name_alternates.iter().map(Option::as_ref))
        .enumerate()
        .filter_map(|(window, read)| {
            let read = read.filter(|c| !c.value.is_empty())?;
            Some((
                window,
                name_read_score(&read.value.to_lowercase(), name_lower),
            ))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum NameRead {
    /// The read contains the name (handles "FROZEN" inside "L7 mRoE FROZEN").
    Substring,
    /// Best fuzzy score of the read against the name.
    Fuzzy(f64),
    /// The read is too short to hold the name; not scored.
    TooShort,
}

impl NameRead {
    fn is_substring(self) -> bool {
        self == NameRead::Substring
    }

    fn fuzzy(self) -> Option<f64> {
        match self {
            NameRead::Fuzzy(s) => Some(s),
            _ => None,
        }
    }

    fn passes(self, min: f64) -> bool {
        match self {
            NameRead::Substring => true,
            NameRead::Fuzzy(s) => s >= min,
            NameRead::TooShort => false,
        }
    }
}

/// Score one lowercased name-cell read against the lowercased player name.
fn name_read_score(cell_text: &str, name_lower: &str) -> NameRead {
    if cell_text.contains(name_lower) {
        return NameRead::Substring;
    }

    // Fuzzy: slide a window the length of the player name over the cell text
    let name_chars: Vec<char> = name_lower.chars().collect();
    let cell_chars: Vec<char> = cell_text.chars().collect();
    let window = name_chars.len();
    if window == 0 || window > cell_chars.len() + 4 {
        return NameRead::TooShort;
    }
    // Also compare whole cell text against the name
    let score_whole = normalized_levenshtein(cell_text, name_lower);
    let score_window = if cell_chars.len() >= window {
        (0..=(cell_chars.len().saturating_sub(window)))
            .map(|s| {
                let slice: String = cell_chars[s..s + window].iter().collect();
                normalized_levenshtein(&slice, name_lower)
            })
            .fold(0.0f64, f64::max)
    } else {
        0.0
    };
    NameRead::Fuzzy(score_whole.max(score_window))
}
