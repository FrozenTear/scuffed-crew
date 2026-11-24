//! Map names from OCR text: banner stripping, glyph folding, exact and
//! fuzzy matching against the catalog, the Watchpoint family, and the
//! Adlersbrunn alias.

use strsim::normalized_levenshtein;

use super::catalog::MAPS;

/// PvE Adlersbrunn (Junkenstein) has no enemy team. Do not store it as
/// Eichenwalde. A real Eichenwalde board, or an alias board with both
/// teams, is kept.
pub fn reject_pve_adlersbrunn(alias: bool, map_name: &str, both_teams: bool) -> bool {
    alias && map_name == "Eichenwalde" && !both_teams
}

/// Adlersbrunn is the Junkenstein event map. The both-teams gate excludes
/// the PvE board, which has no enemy team. Literal "Eichenwalde" is not
/// an alias. A one-letter misread counts when it is closer to Adlersbrunn
/// than to Eichenwalde and the same matcher would store Eichenwalde.
pub fn is_adlersbrunn_alias(text: &str) -> bool {
    let folded = normalize_ocr_glyphs(&text.to_lowercase());
    let eichenwalde = normalize_ocr_glyphs("eichenwalde");
    if folded.contains(&eichenwalde) {
        return false;
    }
    let adlersbrunn = normalize_ocr_glyphs("adlersbrunn");
    if folded.contains(&adlersbrunn) {
        return true;
    }
    if map_from_normalized(&folded, true).as_deref() != Some("Eichenwalde") {
        return false;
    }
    best_word_score(&folded, &adlersbrunn) > best_word_score(&folded, &eichenwalde)
}

fn best_word_score(text: &str, pattern: &str) -> f64 {
    text.split_whitespace()
        .map(|word| normalized_levenshtein(word, pattern))
        .fold(0.0_f64, f64::max)
}

/// Canonicalize a map identifier to its display name in the MAPS table —
/// e.g. the map-vote screen's "SHAMBALI" becomes "Shambali Monastery".
/// `None` when nothing matches: an uncanonicalizable name must not be stored,
/// or the same map fractures into several aggregate rows.
pub fn canonical_map(name: &str) -> Option<String> {
    match_map_in_text(name)
}

/// Mode word printed on the Tab banner, before the `|` and the map name.
/// None of these is a map key. A longer banner phrase still starts with one
/// of them (`PAYLOAD RACE`, `CAPTURE THE FLAG`).
const BANNER_MODE_WORDS: &[&str] = &[
    "CONTROL",
    "ESCORT",
    "HYBRID",
    "PUSH",
    "FLASHPOINT",
    "CLASH",
    "ASSAULT",
    "ELIMINATION",
    "DEATHMATCH",
    "PAYLOAD",
    "CAPTURE",
];

/// Map text from a Tab banner read.
///
/// The banner is `icon MODE | MAP TIME`. The map is the text after the first
/// real `|`. A leading `|` is icon junk (`| CONTROL | ILIOS`), and the mode
/// word after that split is dropped too. This split happens before glyph
/// folding, which would turn `|` into `i` and glue it onto the map
/// (`|ILIOS` becomes a word the matcher misses). A trailing clock
/// (`05:12`, or glued on as `ILIOS05:12`) is not part of the name. With no
/// bar, a mode word in the first few tokens is skipped the same way, and so
/// is the junk in front of it.
pub(super) fn tab_banner_map_text(text: &str) -> &str {
    let text = text.trim().trim_start_matches('|').trim();
    let map_side = if let Some((_, right)) = text.split_once('|') {
        strip_leading_mode(right.trim().trim_start_matches('|').trim())
    } else {
        strip_icon_and_mode(text)
    };
    strip_trailing_clock(map_side)
}

/// Drop a mode word that survived the bar split, and a bar glued to it.
fn strip_leading_mode(text: &str) -> &str {
    let Some(tok) = text.split_whitespace().next() else {
        return text;
    };
    if !mode_token(tok) {
        return text;
    }
    let Some(rel) = text.find(tok) else {
        return text;
    };
    let after = text[rel + tok.len()..]
        .trim_start()
        .trim_start_matches('|')
        .trim();
    if after.is_empty() { text } else { after }
}

/// `05:12` at the end of the map side, with or without a space before it.
fn strip_trailing_clock(text: &str) -> &str {
    let text = text.trim_end();
    let b = text.as_bytes();
    let n = b.len();
    if n < 4 || !b[n - 1].is_ascii_digit() || !b[n - 2].is_ascii_digit() || b[n - 3] != b':' {
        return text;
    }
    let mut start = n - 3;
    let mut hour_digits = 0u8;
    while start > 0 && b[start - 1].is_ascii_digit() && hour_digits < 2 {
        start -= 1;
        hour_digits += 1;
    }
    if hour_digits == 0 {
        return text;
    }
    text[..start].trim_end()
}

fn mode_token(tok: &str) -> bool {
    let bare = tok.trim_matches(|c: char| !c.is_ascii_alphabetic());
    BANNER_MODE_WORDS
        .iter()
        .any(|word| bare.eq_ignore_ascii_case(word))
}

/// Drop icon junk and a following mode word. The map name is what remains.
/// A string with no mode word is returned unchanged, so `King's Row` is kept.
fn strip_icon_and_mode(text: &str) -> &str {
    let mut offset = 0usize;
    for (seen, tok) in text.split_whitespace().enumerate() {
        if seen >= 4 {
            break;
        }
        let Some(rel) = text[offset..].find(tok) else {
            break;
        };
        let start = offset + rel;
        if mode_token(tok) {
            let after = text[start + tok.len()..].trim_start();
            if !after.is_empty() {
                return after;
            }
            break;
        }
        offset = start + tok.len();
    }
    text
}

/// Match a map name from arbitrary OCR text (e.g. the top-bar map label).
///
/// See [`tab_banner_map_text`]: only the map side of a Tab banner is matched,
/// case-insensitively, against the known map list.
pub fn match_map_in_text(text: &str) -> Option<String> {
    let label = tab_banner_map_text(text);
    let lines: Vec<&str> = label
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    find_map(&lines)
}

/// Exact substring match only. The accolade crop uses this so a fuzzy
/// near-miss in the gameplay HUD cannot become the session map.
pub fn exact_map_in_text(text: &str) -> Option<String> {
    // Join wrapped lines first. "New Junk\nCity" is one map name.
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = normalize_ocr_glyphs(&joined.to_lowercase());
    map_from_normalized(&text, false)
}

/// Fold OCR-ambiguous glyphs and Latin diacritics so a mangled map name still
/// matches. `1`, `|`, `l` collapse to `i`; `0` collapses to `o`. Precomposed
/// accents fold to ASCII (`í` to `i`, `ö` to `o`, and the other letters this
/// table already stores). Turkish `İ` (U+0130) and dotless `ı` (U+0131) fold
/// to `i`. Combining marks are dropped. Callers lowercase first; `İ` then
/// arrives as `i` plus a combining dot, which this fold deletes, leaving `i`.
/// The explicit `İ` arm still covers a caller that has not lowercased.
///
/// Applied to BOTH the OCR candidate text and the map patterns (see `find_map`,
/// `fuzzy_match_map`, and the vote reader in `detect::match_start`). The fold is
/// deliberately symmetric — normalizing both sides means it can never corrupt a
/// legit name into a non-match (e.g. `ilios`→`iiios` on both sides still matches,
/// `hollywood`→`hoiiywood` on both sides still matches). Named for the class of
/// misread it fixes: ILIOS (three capital I's) reads as `1LIOS`/`IL10S`/`|LIOS`.
pub fn normalize_ocr_glyphs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if ('\u{0300}'..='\u{036f}').contains(&c) {
            continue;
        }
        let c = match c {
            'á' | 'à' | 'ã' | 'â' | 'ä' | 'Á' | 'À' | 'Ã' | 'Â' | 'Ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' | 'İ' | 'ı' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
            'ç' | 'Ç' => 'c',
            'ñ' | 'Ñ' => 'n',
            'ý' | 'ÿ' | 'Ý' => 'y',
            other => other,
        };
        out.push(match c {
            '1' | '|' | 'l' => 'i',
            '0' => 'o',
            other => other,
        });
    }
    out
}

/// Fuzzy threshold for a map name of `len` non-space chars. Short names get a
/// touch more slack: a single wrong glyph in a 5-char name (BUSAN→BUSVN) is a
/// 0.80 score, which the strict long-name bar would reject. Kept close to the
/// long-name bar so player names / stat fragments still don't cross it — the
/// King's-Row false-positive that justified the strict bar is a substring trap
/// (guarded by dropping bare "king" from MAPS), not a fuzzy near-miss.
pub fn map_fuzzy_threshold(len: usize) -> f64 {
    if len <= 6 { 0.80 } else { 0.85 }
}

/// Lead the best fuzzy map must hold over the next different map. A smaller
/// gap is a tie, and the read stays unknown. Exact whole-word hits do not
/// use this: Temple of Anubis and Throne of Anubis still resolve by table
/// order when the key is present.
pub(super) const MAP_FUZZY_MARGIN: f64 = 0.05;

const GRIMSVOTN_NAME: &str = "Watchpoint: Grímsvötn";
pub(super) const GIBRALTAR_NAME: &str = "Watchpoint: Gibraltar";

/// What a "watchpoint" token decided. Absent means the text never said it,
/// so the ordinary table still runs. Named is one of the two maps. Undecided
/// means the prefix was there and neither name won: the bare key must not
/// become Gibraltar, and the rest of the table may still match.
enum WatchpointFamily {
    Absent,
    Named(&'static str),
    Undecided,
}

fn map_from_normalized(text: &str, allow_fuzzy: bool) -> Option<String> {
    if let WatchpointFamily::Named(name) = resolve_watchpoint(text) {
        return Some(name.to_string());
    }
    for &(display_name, pattern) in MAPS {
        // Whole word or phrase. A short key must not fire inside a longer
        // word: "paris" inside "comparison", "petra" inside "competra",
        // "practice" inside "inpractice".
        if contains_map_key(text, pattern) {
            return Some(display_name.to_string());
        }
    }
    if allow_fuzzy {
        fuzzy_match_map(text)
    } else {
        None
    }
}

/// The word or two words after "watchpoint", apostrophes removed.
fn watchpoint_tails(words: &[String]) -> Vec<String> {
    let mut tails = Vec::new();
    for (i, word) in words.iter().enumerate() {
        if word != "watchpoint" {
            continue;
        }
        let Some(next) = words.get(i + 1) else {
            continue;
        };
        tails.push(next.clone());
        if let Some(after) = words.get(i + 2) {
            tails.push(format!("{next}{after}"));
        }
    }
    tails
}

fn score_watchpoint_tail(tail: &str) -> (f64, f64) {
    let grim_pattern = "grimsvotn";
    // `l` folds to `i`, so the Gibraltar pattern is "gibraitar", not the
    // spelling "gibraltar". Comparing the unfolded spelling made one-letter
    // misreads miss on the accolade path.
    let gib_pattern = normalize_ocr_glyphs("gibraltar");
    let tail = normalize_ocr_glyphs(tail);
    let folded_six = tail.replace('6', "o");
    let mut grim = normalized_levenshtein(&tail, grim_pattern);
    if folded_six != tail {
        grim = grim.max(normalized_levenshtein(&folded_six, grim_pattern));
    }
    if tail.contains(grim_pattern) || folded_six.contains(grim_pattern) {
        grim = 1.0;
    }
    let gib = if tail.contains(&gib_pattern) || folded_six.contains(&gib_pattern) {
        1.0
    } else {
        normalized_levenshtein(&tail, &gib_pattern)
            .max(normalized_levenshtein(&folded_six, &gib_pattern))
    };
    (grim, gib)
}

/// Bare or ambiguous "watchpoint" is not Gibraltar. A following word is
/// fuzzy-matched against Grímsvötn and Gibraltar; one side has to clear the
/// long-name threshold and lead the other by [`MAP_FUZZY_MARGIN`]. `6` folds
/// to `o` only here, so Route 66 is left alone. The next two tokens are also
/// joined, so "GRIMS VOTN" can still hit Grímsvötn.
fn resolve_watchpoint(text: &str) -> WatchpointFamily {
    // Fold first so an accented letter stays inside its word. Splitting on
    // every non-ASCII byte used to break "Grímsvötn" into two tokens.
    let text = normalize_ocr_glyphs(&text.to_lowercase());
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|word| !word.is_empty())
        .map(|word| word.replace('\'', ""))
        .filter(|word| !word.is_empty())
        .collect();
    let saw_prefix = words.iter().any(|word| word == "watchpoint");
    if !saw_prefix {
        return WatchpointFamily::Absent;
    }
    let stripped = text.replace('\'', "");
    if stripped.contains("grimsvotn") {
        return WatchpointFamily::Named(GRIMSVOTN_NAME);
    }
    let tails = watchpoint_tails(&words);
    if tails.is_empty() {
        return WatchpointFamily::Undecided;
    }
    let mut best_grim: f64 = 0.0;
    let mut best_gib: f64 = 0.0;
    for tail in &tails {
        let (grim, gib) = score_watchpoint_tail(tail);
        best_grim = best_grim.max(grim);
        best_gib = best_gib.max(gib);
    }
    let threshold = map_fuzzy_threshold("grimsvotn".chars().count());
    if best_grim >= threshold && best_grim - best_gib >= MAP_FUZZY_MARGIN {
        WatchpointFamily::Named(GRIMSVOTN_NAME)
    } else if best_gib >= threshold && best_gib - best_grim >= MAP_FUZZY_MARGIN {
        WatchpointFamily::Named(GIBRALTAR_NAME)
    } else {
        WatchpointFamily::Undecided
    }
}

/// `pattern` is a map key. It matches only when every character of the key
/// is bounded by a non-alphanumeric edge, so a shorter key cannot hide
/// inside a longer word.
fn contains_map_key(text: &str, pattern: &str) -> bool {
    let pattern = normalize_ocr_glyphs(pattern);
    if pattern.is_empty() {
        return false;
    }
    let mut rest = text;
    while let Some(at) = rest.find(&pattern) {
        let before_ok = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after = at + pattern.len();
        let after_ok = rest[after..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        let step = rest[at..].chars().next().map(char::len_utf8).unwrap_or(1);
        rest = &rest[at + step..];
    }
    false
}

pub(super) fn find_map(lines: &[&str]) -> Option<String> {
    let text = normalize_ocr_glyphs(&lines.join(" ").to_lowercase());
    map_from_normalized(&text, true)
}

/// Score for an OCR tail cut off the end of a long map name (`GRiMsVO` for
/// Grímsvötn). Short keys stay on the whole-word rule. The score is the
/// average of a full match and the covered fraction, so two missing letters
/// on a 9-letter name clear the long-name floor and a stub does not.
fn prefix_truncation_score(word: &str, pattern: &str) -> f64 {
    let n = pattern.chars().count();
    let k = word.chars().count();
    if n <= 6 || k < 6 || k >= n || !pattern.starts_with(word) {
        return 0.0;
    }
    (n + k) as f64 / (2.0 * n as f64)
}

/// Closest map at or above [`map_fuzzy_threshold`], when it leads the next
/// different display name by [`MAP_FUZZY_MARGIN`]. Below the floor, or inside
/// that margin, the map is unknown. Aliases of one display name (Lijiang's
/// `liang` / `lulang` keys) only raise that name's score.
fn fuzzy_match_map(text: &str) -> Option<String> {
    // `text` is expected to already be glyph-normalized by the caller.
    let text = normalize_ocr_glyphs(text);
    let words: Vec<&str> = text.split_whitespace().collect();

    let mut best_map: Option<&str> = None;
    let mut best_score: f64 = 0.0;
    let mut second_score: f64 = 0.0;

    for &(display_name, pattern) in MAPS {
        let pattern = normalize_ocr_glyphs(pattern);
        let pattern_parts: Vec<&str> = pattern.split_whitespace().collect();
        let threshold = map_fuzzy_threshold(pattern.chars().filter(|c| !c.is_whitespace()).count());
        let mut pattern_best = 0.0;

        if pattern_parts.len() == 1 {
            for &word in &words {
                let token = word.trim_matches(|c: char| !c.is_alphanumeric());
                if token.is_empty() {
                    continue;
                }
                let score = normalized_levenshtein(token, &pattern)
                    .max(prefix_truncation_score(token, &pattern));
                if score > pattern_best {
                    pattern_best = score;
                }
            }
        } else {
            for window in words.windows(pattern_parts.len()) {
                let candidate = window.join(" ");
                let score = normalized_levenshtein(&candidate, &pattern);
                if score > pattern_best {
                    pattern_best = score;
                }
            }
        }

        if pattern_best < threshold {
            continue;
        }
        // A second key for the map already in front only improves its score.
        if best_map == Some(display_name) {
            if pattern_best > best_score {
                best_score = pattern_best;
            }
            continue;
        }
        if best_map.is_none() || pattern_best > best_score {
            if best_map.is_some() {
                second_score = second_score.max(best_score);
            }
            best_score = pattern_best;
            best_map = Some(display_name);
        } else if pattern_best > second_score {
            second_score = pattern_best;
        }
    }

    let map_name = best_map?;
    if best_score - second_score < MAP_FUZZY_MARGIN {
        tracing::debug!(
            map = map_name,
            score = best_score,
            runner_up = second_score,
            "fuzzy map match held: top scores are too close"
        );
        return None;
    }

    tracing::debug!(map = map_name, score = best_score, "fuzzy matched map name");
    Some(map_name.to_string())
}
