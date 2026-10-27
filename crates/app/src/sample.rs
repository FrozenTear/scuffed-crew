//! Small-sample rule for win rates (Robert, Oct 20; same rule as the tracker).
//!
//! - Under 3 games a hero folds away under "Other heroes".
//! - From 3 to 9 games only the record shows, e.g. `2W-1L`.
//! - From 10 games the win rate shows.
//! - An empty map, mode or season shows "No games", never 0%.
//!
//! Used by My Stats (`/stats`), member stats and member profiles.

use dioxus::prelude::*;

use crate::util::record_label;

/// Fewer games than this and a hero folds into "Other heroes".
pub const RECORD_MIN_GAMES: u32 = 3;
/// From this many games a win rate shows instead of the record.
pub const RATE_MIN_GAMES: u32 = 10;

/// Group name for heroes under [`RECORD_MIN_GAMES`].
pub const OTHER_HEROES: &str = "Other heroes";
/// Shown for an empty map, mode or season instead of 0%.
pub const NO_GAMES: &str = "No games";

/// Caption for hero lists that follow the rule.
pub const HERO_RULE_NOTE: &str =
    "Win rate from 10 games, record from 3. Heroes with fewer fold into Other heroes.";
/// Caption for map lists that follow the rule.
pub const MAP_RULE_NOTE: &str = "Win rate from 10 games, record below that.";

/// Games and results for one hero, map, mode or season.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub games: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

impl Tally {
    pub fn new(games: u32, wins: u32, losses: u32, draws: u32) -> Self {
        Self {
            games,
            wins,
            losses,
            draws,
        }
    }

    /// Under [`RECORD_MIN_GAMES`]: the hero goes under "Other heroes".
    pub fn folds_away(&self) -> bool {
        self.games < RECORD_MIN_GAMES
    }

    /// At least [`RATE_MIN_GAMES`]: a win rate may show.
    pub fn has_win_rate(&self) -> bool {
        self.games >= RATE_MIN_GAMES
    }

    /// Win rate in percent, only once the sample is big enough.
    pub fn win_rate_pct(&self) -> Option<f64> {
        if self.has_win_rate() {
            Some(f64::from(self.wins) / f64::from(self.games) * 100.0)
        } else {
            None
        }
    }

    pub fn record(&self) -> String {
        record_label(self.wins, self.losses, self.draws)
    }

    /// The one result text: "No games", the record, or the win rate.
    pub fn label(&self) -> String {
        if self.games == 0 {
            return NO_GAMES.to_string();
        }
        match self.win_rate_pct() {
            Some(pct) => format!("{pct:.1}%"),
            None => self.record(),
        }
    }

    /// Sum of several tallies, for the "Other heroes" row or a mode total.
    pub fn sum(tallies: impl IntoIterator<Item = Tally>) -> Tally {
        tallies.into_iter().fold(Tally::default(), |acc, t| Tally {
            games: acc.games + t.games,
            wins: acc.wins + t.wins,
            losses: acc.losses + t.losses,
            draws: acc.draws + t.draws,
        })
    }
}

/// Split `items` into heroes that keep their own row and heroes that fold
/// into "Other heroes". Order is kept on both sides.
pub fn fold_small<T: Copy>(items: &[T], tally: impl Fn(T) -> Tally) -> (Vec<T>, Vec<T>) {
    items
        .iter()
        .copied()
        .partition(|item| !tally(*item).folds_away())
}

/// Season summary tile: the win rate with the record under it from 10
/// games, only the record below that. An empty season never reaches here;
/// the pages show their own empty state.
pub fn season_tile(season: Tally) -> Element {
    let value = season.label();
    let (label, record) = if season.has_win_rate() {
        ("Win Rate", Some(season.record()))
    } else {
        ("Record", None)
    };
    rsx! {
        div { class: "stat-tile stat-tile-hero",
            div { class: "stat-tile-value", "{value}" }
            div { class: "stat-tile-label", "{label}" }
            if let Some(record) = record {
                div { class: "stat-tile-record", "{record}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn games(n: u32) -> Tally {
        let wins = n / 2 + n % 2;
        Tally::new(n, wins, n - wins, 0)
    }

    #[test]
    fn zero_games_folds_and_reads_no_games() {
        let t = games(0);
        assert!(t.folds_away());
        assert!(!t.has_win_rate());
        assert_eq!(t.win_rate_pct(), None);
        assert_eq!(t.label(), "No games");
    }

    #[test]
    fn two_games_folds_away() {
        let t = games(2);
        assert!(t.folds_away());
        assert!(!t.has_win_rate());
        assert_eq!(t.label(), "1W-1L");
    }

    #[test]
    fn three_games_shows_the_record_only() {
        let t = games(3);
        assert!(!t.folds_away());
        assert!(!t.has_win_rate());
        assert_eq!(t.label(), "2W-1L");
    }

    #[test]
    fn nine_games_still_shows_the_record_only() {
        let t = Tally::new(9, 5, 3, 1);
        assert!(!t.folds_away());
        assert!(!t.has_win_rate());
        assert_eq!(t.win_rate_pct(), None);
        assert_eq!(t.label(), "5W-3L-1D");
    }

    #[test]
    fn ten_games_gets_a_win_rate() {
        let t = Tally::new(10, 7, 3, 0);
        assert!(!t.folds_away());
        assert!(t.has_win_rate());
        assert_eq!(t.win_rate_pct(), Some(70.0));
        assert_eq!(t.label(), "70.0%");
    }

    #[test]
    fn sum_and_fold_keep_order() {
        let list = [games(12), games(1), games(4), games(2)];
        let (kept, folded) = fold_small(&list, |t| t);
        assert_eq!(kept, vec![games(12), games(4)]);
        assert_eq!(folded, vec![games(1), games(2)]);
        assert_eq!(Tally::sum(folded), Tally::new(3, 2, 1, 0));
    }

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn season_tile_shows_record_then_win_rate() {
        fn nine() -> Element {
            season_tile(Tally::new(9, 6, 3, 0))
        }
        fn ten() -> Element {
            season_tile(Tally::new(10, 6, 4, 0))
        }
        let small = render(nine);
        assert!(
            small.contains("6W-3L") && small.contains("Record"),
            "{small}"
        );
        assert!(!small.contains('%'), "{small}");
        let big = render(ten);
        assert!(big.contains("60.0%") && big.contains("Win Rate"), "{big}");
        assert!(big.contains("6W-4L"), "{big}");
    }

    #[test]
    fn notes_have_no_long_dashes() {
        for note in [HERO_RULE_NOTE, MAP_RULE_NOTE] {
            assert!(!note.contains('\u{2014}') && !note.contains('\u{2013}'));
        }
    }
}
