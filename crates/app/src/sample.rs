//! Small-sample rule for win rates (FrozenTear, Oct 20; same rule as the tracker).
//!
//! - Under 3 games a hero folds away under "Other heroes".
//! - From 3 to 9 games only the record shows, e.g. `2W-1L`.
//! - From 10 games the win rate shows.
//! - An empty map, mode or season shows "No games", never 0%.
//! - The pooled "Other heroes" row always shows its record, never a win
//!   rate, because it mixes several heroes.
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

/// Text class for the pooled "Other heroes" result: always muted.
pub const OTHER_RESULT_CLASS: &str = "stats-wr muted";

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

/// Result text for the pooled "Other heroes" row or card: the record, or
/// "No games" for an empty pool. Never a win rate, even from 10 games,
/// because the pool mixes several heroes.
pub fn other_label(pool: Tally) -> String {
    if pool.games == 0 {
        NO_GAMES.to_string()
    } else {
        pool.record()
    }
}

/// One ranked entry and its win rate in percent.
pub type Ranked<T> = (T, f64);

/// Best and worst entry from `ranked`, sorted best first by win rate.
/// Worst is `None` when it would show the same rounded % as Best, which
/// also covers a single ranked entry.
pub fn best_and_worst<T: Copy>(ranked: &[Ranked<T>]) -> (Option<Ranked<T>>, Option<Ranked<T>>) {
    let best = ranked.first().copied();
    let worst = ranked.last().copied().filter(|&(_, worst_wr)| match best {
        Some((_, best_wr)) => format!("{best_wr:.0}") != format!("{worst_wr:.0}"),
        None => false,
    });
    (best, worst)
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

    #[test]
    fn other_heroes_pool_of_ten_plus_shows_the_record() {
        let pool = Tally::sum([games(2), games(2), games(2), games(2), games(2), games(1)]);
        assert_eq!(pool.games, 11);
        assert!(pool.has_win_rate());
        let text = other_label(pool);
        assert_eq!(text, "6W-5L");
        assert!(!text.contains('%'), "{text}");
        assert_eq!(other_label(Tally::new(12, 6, 4, 2)), "6W-4L-2D");
        assert_eq!(other_label(Tally::default()), "No games");
        assert!(OTHER_RESULT_CLASS.contains("muted"));
    }

    #[test]
    fn worst_hides_when_it_ties_best() {
        assert_eq!(best_and_worst::<u8>(&[]), (None, None));
        assert_eq!(best_and_worst(&[(1, 60.0)]), (Some((1, 60.0)), None));
        assert_eq!(
            best_and_worst(&[(1, 50.0), (2, 50.0)]),
            (Some((1, 50.0)), None)
        );
        assert_eq!(
            best_and_worst(&[(1, 50.0), (2, 50.0), (3, 50.0)]),
            (Some((1, 50.0)), None)
        );
        assert_eq!(
            best_and_worst(&[(1, 70.0), (2, 55.0), (3, 40.0)]),
            (Some((1, 70.0)), Some((3, 40.0)))
        );
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
