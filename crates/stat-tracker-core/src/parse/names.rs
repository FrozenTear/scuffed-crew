//! Finding the player's own row by name across a row's name windows.

use crate::model::RowOcrResult;
use strsim::normalized_levenshtein;

/// Fuzzy score a name read needs to count as the player's name.
const NAME_MATCH_MIN: f64 = 0.55;

/// The row that matched the configured player name, and which name window
/// matched it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NameMatch {
    /// Row index across both teams.
    pub row: usize,
    /// Index into `ocr::preprocess::name_windows` of the read that
    /// matched: a whole-word hit if there is one, else a substring hit,
    /// else the best fuzzy read (the lower index on a tie).
    pub window: usize,
    /// No other window's read on that row matched. On 5v5 the windows belong
    /// to different table layouts, so a sole match also names the layout.
    pub sole_window: bool,
    /// Index, in the name list the matcher was given, of the saved name that
    /// matched (0 for the one-name matchers). Blank names keep their place
    /// in that count.
    pub name: usize,
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
/// per table layout). The first row, in board order, with a whole-word hit
/// in any window wins outright, then the first row with a substring hit;
/// otherwise the best fuzzy row at or above 0.55. This ranking holds with
/// one saved name too: a board reading FROZENTEAR on row 0 and FROZEN on
/// row 1 is row 1 for "Frozen", where the first substring row used to win.
pub fn match_player_row_by_name(rows: &[RowOcrResult], player_name: &str) -> Option<NameMatch> {
    scored_match(rows, &player_name.to_lowercase()).map(|(found, _)| found)
}

/// One in-game name, or every saved name of the member
/// (`Config::own_player_names`), for the row matchers.
pub trait NameList {
    fn each_name(&self) -> impl Iterator<Item = &str>;
}

impl NameList for str {
    fn each_name(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self)
    }
}

impl NameList for [String] {
    fn each_name(&self) -> impl Iterator<Item = &str> {
        self.iter().map(String::as_str)
    }
}

impl NameList for Vec<String> {
    fn each_name(&self) -> impl Iterator<Item = &str> {
        self.iter().map(String::as_str)
    }
}

/// [`match_player_row_by_name`] over every name in `names`, ranked by
/// [`pick_name`] with no fuzzy margin: the one most specific hit, and
/// `None` when two saved names tie (for example each a whole word on its
/// own row). Each name is matched in lowercase, so "öster" and "Öster"
/// read the same row. Blank names are skipped.
pub fn match_player_row_by_names(
    rows: &[RowOcrResult],
    names: &(impl NameList + ?Sized),
) -> Option<NameMatch> {
    pick_name(rows, names, 0.0).map(|(found, _)| found)
}

/// The one saved name of `names` that `row`'s name reads belong to, ranked
/// by [`pick_name`] with a fuzzy margin of [`ROW_NAME_FUZZY_MARGIN`].
/// `None` when the row has no name read, no saved name matches it, or two
/// saved names match it equally well. Blank names are skipped.
pub fn best_name_for_row<'a>(
    row: &RowOcrResult,
    names: &'a (impl NameList + ?Sized),
) -> Option<&'a str> {
    pick_name(std::slice::from_ref(row), names, ROW_NAME_FUZZY_MARGIN).map(|(_, name)| name)
}

/// [`best_name_for_row`] as a match: `row` is row `row_idx` of its board,
/// and the match also carries the window and the saved name's index.
/// See [`any_name_reads_row`] to tell "no saved name reads here" from
/// "saved names read here but none wins".
pub fn match_row_by_names(
    row: &RowOcrResult,
    row_idx: usize,
    names: &(impl NameList + ?Sized),
) -> Option<NameMatch> {
    pick_name(std::slice::from_ref(row), names, ROW_NAME_FUZZY_MARGIN).map(|(found, _)| NameMatch {
        row: row_idx,
        ..found
    })
}

/// Some saved name of `names` reads on `row` at or above the name-match
/// floor: a whole word, a substring, or a fuzzy score of at least 0.55.
/// Blank names are skipped. True also when [`match_row_by_names`] names
/// none of them (two close reads, or a tie): the row is then unresolved,
/// not a stranger's.
pub fn any_name_reads_row(row: &RowOcrResult, names: &(impl NameList + ?Sized)) -> bool {
    names.each_name().any(|name| {
        let lower = name.to_lowercase();
        !name.trim().is_empty() && scored_match(std::slice::from_ref(row), &lower).is_some()
    })
}

/// How far the best fuzzy score has to lead every other saved name's
/// before one row is named by a fuzzy read alone. "MAlN" scores Main 0.75
/// and Alt 0.67: no name.
pub const ROW_NAME_FUZZY_MARGIN: f64 = 0.15;

/// How one saved name was found on the rows, best first.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hit {
    /// The name is a whole word of a read ("ALPHA" in "[TAG] ALPHA").
    Word,
    /// The name is inside a read but runs into other letters or digits
    /// ("alpha" in "ALPHA2").
    Substring,
    /// No substring anywhere: the best fuzzy score, at or above the floor.
    Fuzzy(f64),
}

/// One saved name's hit: the match, the name, its lowercase form, the hit.
type NameHit<'a> = (NameMatch, &'a str, String, Hit);

/// The one saved name of `names` that matches `rows` best, and its match.
/// List order never decides:
///
/// 1. A whole-word hit beats a substring hit, which beats any fuzzy read.
/// 2. Among the hits of that kind on one row, a name that sits inside a
///    longer saved name that also hit gives way to it (Alpha to Alpha2
///    on "ALPHA2").
/// 3. Two hits still left (two names on different rows, or two unrelated
///    names on one row) are a tie: `None`.
/// 4. A substring hit (never a whole word) names nothing when a longer
///    saved name holding it also reads on that row at or above the floor,
///    even fuzzily: "ALPHAZ" may be Alpha2 misread, so it names neither
///    Alpha nor Alpha2. "ALPHA" and "[TAG] ALPHA" stay Alpha's.
/// 5. Fuzzy only: the best score, when it leads every other saved name's
///    by more than nothing and by at least `fuzzy_margin`; else `None`.
///
/// Blank names, and a name equal to an earlier one ignoring case, are
/// skipped.
fn pick_name<'a>(
    rows: &[RowOcrResult],
    names: &'a (impl NameList + ?Sized),
    fuzzy_margin: f64,
) -> Option<(NameMatch, &'a str)> {
    let mut seen: Vec<String> = Vec::new();
    let mut hits: Vec<NameHit<'a>> = Vec::new();
    for (idx, name) in names.each_name().enumerate() {
        let lower = name.to_lowercase();
        if name.trim().is_empty() || seen.contains(&lower) {
            continue;
        }
        seen.push(lower.clone());
        if let Some((found, hit)) = scored_match(rows, &lower) {
            hits.push((NameMatch { name: idx, ..found }, name, lower, hit));
        }
    }
    let kind = [Hit::Word, Hit::Substring]
        .into_iter()
        .find(|kind| hits.iter().any(|h| h.3 == *kind));
    if let Some(kind) = kind {
        let same: Vec<&NameHit<'a>> = hits.iter().filter(|h| h.3 == kind).collect();
        let inside_longer = |h: &NameHit<'a>| {
            same.iter().any(|other| {
                other.0.row == h.0.row && other.2.len() > h.2.len() && other.2.contains(&h.2)
            })
        };
        let specific: Vec<&&NameHit<'a>> = same.iter().filter(|h| !inside_longer(h)).collect();
        let longer_reads_row = |h: &NameHit<'a>| {
            seen.iter().any(|other| {
                other.len() > h.2.len()
                    && other.contains(&h.2)
                    && rows.get(h.0.row).is_some_and(|row| {
                        row_window_scores(row, other)
                            .iter()
                            .any(|(_, read)| read.passes(NAME_MATCH_MIN))
                    })
            })
        };
        return match specific.as_slice() {
            [only] if kind == Hit::Substring && longer_reads_row(only) => None,
            [only] => Some((only.0, only.1)),
            _ => None,
        };
    }
    let score = |h: &NameHit<'a>| match h.3 {
        Hit::Fuzzy(score) => score,
        Hit::Word | Hit::Substring => 0.0,
    };
    let (first, rest) = hits.split_first()?;
    let best = rest
        .iter()
        .fold(first, |a, b| if score(b) > score(a) { b } else { a });
    let leads = hits
        .iter()
        .filter(|other| !std::ptr::eq(*other, best))
        .all(|other| {
            let lead = score(best) - score(other);
            lead > 0.0 && lead >= fuzzy_margin
        });
    leads.then_some((best.0, best.1))
}

/// The match for one lowercased name, and how it was found: the first row
/// with a whole-word hit, else the first row with a substring hit, else the
/// best fuzzy row at or above the floor. Every matcher here ranks through
/// this, the one-name matchers included, so a whole-word row beats an
/// earlier substring row for one saved name as well as for several.
fn scored_match(rows: &[RowOcrResult], name_lower: &str) -> Option<(NameMatch, Hit)> {
    let mut best: Option<(usize, f64)> = None;

    let kinds = [
        (Hit::Word, NameRead::WholeWord),
        (Hit::Substring, NameRead::Substring),
    ];
    for (kind, wanted) in kinds {
        for (i, row) in rows.iter().enumerate() {
            let scores = row_window_scores(row, name_lower);
            if let Some(&(window, _)) = scores.iter().find(|(_, s)| *s == wanted) {
                // `name` so the report log scrub drops the field (report-bundle §6).
                tracing::debug!(
                    row = i,
                    window,
                    ?kind,
                    "player name found via substring in row"
                );
                return Some((name_match(i, window, &scores), kind));
            }
        }
    }
    for (i, row) in rows.iter().enumerate() {
        let scores = row_window_scores(row, name_lower);
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
    let scores = row_window_scores(&rows[row], name_lower);
    let window = scores
        .iter()
        .filter(|(_, s)| s.fuzzy() == Some(score))
        .map(|(w, _)| *w)
        .min()
        .unwrap_or(0);
    tracing::debug!(row, window, score, "player name fuzzy-matched in row");
    Some((name_match(row, window, &scores), Hit::Fuzzy(score)))
}

fn name_match(row: usize, window: usize, scores: &[(usize, NameRead)]) -> NameMatch {
    let sole_window = !scores
        .iter()
        .any(|(w, s)| *w != window && s.passes(NAME_MATCH_MIN));
    NameMatch {
        row,
        window,
        sole_window,
        name: 0,
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
    /// The read holds the name as a whole word, no letter or digit right
    /// before or after it (handles "FROZEN" inside "L7 mRoE FROZEN").
    WholeWord,
    /// The read holds the name inside a longer word ("alpha" in
    /// "alpha2").
    Substring,
    /// Best fuzzy score of the read against the name.
    Fuzzy(f64),
    /// The read is too short to hold the name; not scored.
    TooShort,
}

impl NameRead {
    fn fuzzy(self) -> Option<f64> {
        match self {
            NameRead::Fuzzy(s) => Some(s),
            _ => None,
        }
    }

    fn passes(self, min: f64) -> bool {
        match self {
            NameRead::WholeWord | NameRead::Substring => true,
            NameRead::Fuzzy(s) => s >= min,
            NameRead::TooShort => false,
        }
    }
}

/// Score one lowercased name-cell read against the lowercased player name.
fn name_read_score(cell_text: &str, name_lower: &str) -> NameRead {
    if cell_text.contains(name_lower) {
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        let whole = cell_text.match_indices(name_lower).any(|(at, _)| {
            !word(cell_text[..at].chars().next_back())
                && !word(cell_text[at + name_lower.len()..].chars().next())
        });
        return if whole {
            NameRead::WholeWord
        } else {
            NameRead::Substring
        };
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
