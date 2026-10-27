//! The one small-sample rule every 0.6 page uses (ux.md section 17), next
//! to the shared bar ([`crate::winrate_bar`]): Overview, Heroes, Maps and
//! Seasons all ask this module, so the rule and its words cannot drift.
//!
//! - Under [`FOLD_UNDER`] games in total: folded into "Other".
//! - Under [`RATE_MIN`] decided games (win, loss or draw; undecided never
//!   count): the record and [`TOO_FEW`], no % and no bar.
//! - [`RATE_MIN`] or more decided games: the win rate.

use crate::aggregate::Record;

/// Fewer games than this, counting every game, fold into "Other".
pub const FOLD_UNDER: usize = 3;
/// Decided games a win rate needs.
pub const RATE_MIN: usize = 10;
/// The words in place of a win rate below [`RATE_MIN`] decided games.
pub const TOO_FEW: &str = "Too few games";

/// Which of the three treatments a record gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sample {
    /// Under [`FOLD_UNDER`] games: counted in "Other", not shown alone.
    Fold,
    /// The record and [`TOO_FEW`].
    TooFew,
    /// The win rate as a whole percent.
    Rated(i64),
}

/// Wins, losses and draws: the games with an outcome.
pub fn decided(r: &Record) -> usize {
    r.wins + r.losses + r.draws
}

/// The win rate as a whole percent, from [`RATE_MIN`] decided games.
pub fn rate(r: &Record) -> Option<i64> {
    (decided(r) >= RATE_MIN).then(|| (r.win_rate() * 100.0).round() as i64)
}

pub fn sample(r: &Record) -> Sample {
    if r.games < FOLD_UNDER {
        Sample::Fold
    } else {
        rate(r).map_or(Sample::TooFew, Sample::Rated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(games: usize, wins: usize, losses: usize, draws: usize) -> Record {
        Record {
            games,
            wins,
            losses,
            draws,
        }
    }

    #[test]
    fn under_three_games_fold_counting_every_game() {
        assert_eq!(sample(&rec(1, 1, 0, 0)), Sample::Fold);
        assert_eq!(sample(&rec(2, 0, 0, 0)), Sample::Fold);
        assert_eq!(sample(&rec(3, 0, 0, 0)), Sample::TooFew, "3 undecided");
        assert_eq!(sample(&rec(3, 2, 1, 0)), Sample::TooFew);
    }

    #[test]
    fn a_rate_needs_ten_decided_games() {
        assert_eq!(sample(&rec(9, 5, 4, 0)), Sample::TooFew);
        assert_eq!(sample(&rec(10, 6, 4, 0)), Sample::Rated(60));
        assert_eq!(sample(&rec(10, 5, 4, 1)), Sample::Rated(50), "draws count");
        // 40 games with 9 outcomes read is still too few.
        assert_eq!(sample(&rec(40, 5, 4, 0)), Sample::TooFew);
        assert_eq!(rate(&rec(40, 5, 4, 0)), None);
        assert_eq!(rate(&rec(88, 45, 41, 0)), Some(52));
        assert_eq!(decided(&rec(12, 5, 4, 1)), 10);
    }

    #[test]
    fn one_wording() {
        assert_eq!(TOO_FEW, "Too few games");
    }
}
