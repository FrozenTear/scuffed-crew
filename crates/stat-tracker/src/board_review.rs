//! Review hold for a game's scoreboard reads.
//!
//! A game whose only Tab reads are dimmed/unsure boards, or all-zero
//! pre-round boards, is saved on this machine and left out of auto-upload.
//! One later clean, non-zero read in the same game clears the hold.

use crate::reader_apply::OWN_ROW_FIELDS;
use crate::storage::PersonalMatch;

/// One scoreboard read, reduced to the own-row stats and the unsure mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreboardRead {
    /// Own-row stats in order: elims, assists, deaths, damage, healing, mitigation.
    pub stats: [u32; 6],
    /// Every own-row field was marked unsure. A dimmed header does this.
    pub unsure: bool,
}

impl ScoreboardRead {
    pub fn from_match(row: &PersonalMatch) -> Self {
        Self {
            stats: [
                row.elims,
                row.assists,
                row.deaths,
                row.damage,
                row.healing,
                row.mitigation,
            ],
            unsure: own_row_marked_unsure(&row.suspect_fields),
        }
    }

    /// Dimmed/unsure, or a pre-round board with every own-row stat at 0.
    pub fn is_weak(self) -> bool {
        self.unsure || self.stats.iter().all(|stat| *stat == 0)
    }
}

fn names_field(stored: &str, flat: &str) -> bool {
    if stored == flat {
        return true;
    }
    let Some((row, field)) = stored.split_once('.') else {
        return false;
    };
    let Some(digits) = row.strip_prefix('r') else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) && field == flat
}

/// True when every own-row field is listed, the mark a dimmed board gets.
pub fn own_row_marked_unsure(fields: &[String]) -> bool {
    OWN_ROW_FIELDS
        .iter()
        .all(|name| fields.iter().any(|stored| names_field(stored, name)))
}

/// Hold the game when it has reads and every one of them is weak.
///
/// An empty list is not a hold. Any clean, non-zero read returns false,
/// whatever order the reads arrived in.
pub fn game_review_hold(reads: impl IntoIterator<Item = ScoreboardRead>) -> bool {
    let mut seen = false;
    for read in reads {
        seen = true;
        if !read.is_weak() {
            return false;
        }
    }
    seen
}

pub fn game_review_hold_matches(rows: &[PersonalMatch]) -> bool {
    game_review_hold(rows.iter().map(ScoreboardRead::from_match))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(stats: [u32; 6], unsure: bool) -> ScoreboardRead {
        ScoreboardRead { stats, unsure }
    }

    #[test]
    fn dimmed_only_is_held() {
        let dimmed = read([4, 1, 2, 900, 100, 0], true);
        assert!(game_review_hold([dimmed]));
        assert!(game_review_hold([dimmed, dimmed]));
    }

    #[test]
    fn all_zero_only_is_held() {
        let zero = read([0; 6], false);
        assert!(zero.is_weak());
        assert!(game_review_hold([zero]));
        assert!(game_review_hold([zero, zero]));
    }

    #[test]
    fn dimmed_then_clean_is_normal() {
        let dimmed = read([4, 1, 2, 900, 100, 0], true);
        let clean = read([10, 4, 2, 4000, 8000, 100], false);
        assert!(!clean.is_weak());
        assert!(!game_review_hold([dimmed, clean]));
    }

    #[test]
    fn zero_then_clean_is_normal() {
        let zero = read([0; 6], false);
        let clean = read([2, 0, 1, 400, 50, 0], false);
        assert!(!game_review_hold([zero, clean]));
    }

    #[test]
    fn clean_only_is_unchanged() {
        let clean = read([10, 4, 2, 4000, 8000, 100], false);
        assert!(!game_review_hold([clean]));
        assert!(!game_review_hold([clean, clean]));
    }

    #[test]
    fn dimmed_and_zero_with_no_clean_read_is_held() {
        let dimmed = read([1, 0, 0, 50, 0, 0], true);
        let zero = read([0; 6], false);
        assert!(game_review_hold([dimmed, zero]));
        assert!(game_review_hold([zero, dimmed]));
    }
}
