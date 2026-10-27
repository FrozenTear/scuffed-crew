//! Small-sample rule for win rates (FrozenTear, Oct 20; same rule as the tracker).
//!
//! - Under 3 games a hero folds away under "Other heroes".
//! - From 3 games on, under 10 decided games (win, loss or draw) only the
//!   record shows, e.g. `2W-1L`, and the result reads "Too few games".
//!   Undecided games never count toward the 10 (ux.md section 17), so 12
//!   games with 9 decided still read "Too few games".
//! - From 10 decided games the win rate shows.
//! - An empty map, mode or season shows "No games", never 0%.
//! - The pooled "Other heroes" row always shows its record, never a win
//!   rate, because it mixes several heroes.
//!
//! Used by My Stats (`/stats`), member stats, member profiles, the members
//! list hero filter and the leaderboards.

use dioxus::prelude::*;

use crate::util::record_label;

/// Fewer games than this and a hero folds into "Other heroes".
pub const RECORD_MIN_GAMES: u32 = 3;
/// From this many decided games (win, loss or draw) a win rate shows
/// instead of the record. Undecided games never count.
pub const RATE_MIN_DECIDED: u32 = 10;

/// Group name for heroes under [`RECORD_MIN_GAMES`].
pub const OTHER_HEROES: &str = "Other heroes";
/// Shown for an empty map, mode or season instead of 0%.
pub const NO_GAMES: &str = "No games";
/// The one wording for a result under [`RATE_MIN_DECIDED`] decided games.
pub const TOO_FEW_GAMES: &str = "Too few games";

/// Text class for the pooled "Other heroes" result: always muted.
pub const OTHER_RESULT_CLASS: &str = "stats-wr muted";

/// Caption for hero lists that follow the rule.
pub const HERO_RULE_NOTE: &str = "Win rate from 10 decided games. Fewer is Too few games, with the record. Heroes under 3 games fold into Other heroes.";
/// Caption for map lists that follow the rule.
pub const MAP_RULE_NOTE: &str =
    "Win rate from 10 decided games. Fewer is Too few games, with the record.";

/// Caption for the members list and leaderboards.
pub const LIST_RULE_NOTE: &str =
    "Win rate from 10 decided games. Fewer is Too few games, with the record.";

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

    /// Wins, losses and draws. Undecided games are the rest of `games`.
    pub fn decided(&self) -> u32 {
        self.wins + self.losses + self.draws
    }

    /// At least [`RATE_MIN_DECIDED`] decided games: a win rate may show.
    pub fn has_win_rate(&self) -> bool {
        self.decided() >= RATE_MIN_DECIDED
    }

    /// Some games but under [`RATE_MIN_DECIDED`] decided: "Too few games".
    pub fn too_few(&self) -> bool {
        self.games > 0 && !self.has_win_rate()
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
/// "No games" for an empty pool. Never a win rate, even from 10 decided games,
/// because the pool mixes several heroes.
pub fn other_label(pool: Tally) -> String {
    if pool.games == 0 {
        NO_GAMES.to_string()
    } else {
        pool.record()
    }
}

/// Result text for a list row on the members list or the leaderboards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListResult {
    /// "No games", the record such as `5W-3L-1D`, or a whole percent.
    pub text: String,
    /// True once the win rate shows (10 or more decided games).
    pub is_rate: bool,
    /// True under [`RECORD_MIN_GAMES`]: show it muted.
    pub muted: bool,
}

/// Apply the rule to a list row with `games`, its `wins`, `losses` and
/// `draws`, and the server's 0.0 to 1.0 `winrate`.
///
/// Older servers send only `games` and `winrate`, so the three counts
/// default to 0. In that case the win count is rebuilt from the rate (wins
/// over games) and shows alone, such as `5W`, because losses and draws
/// cannot be told apart, and every game counts as decided (see
/// [`list_decided`]).
pub fn list_result(games: u32, wins: u32, losses: u32, draws: u32, winrate: f32) -> ListResult {
    let has_record = wins + losses + draws > 0;
    let tally = if has_record || games == 0 {
        Tally::new(games, wins, losses, draws)
    } else {
        let rebuilt = (f64::from(winrate) * f64::from(games))
            .round()
            .clamp(0.0, f64::from(games)) as u32;
        Tally::new(games, rebuilt, games - rebuilt, 0)
    };
    let muted = tally.folds_away();
    let (text, is_rate) = if games == 0 {
        (NO_GAMES.to_string(), false)
    } else {
        match tally.win_rate_pct() {
            Some(pct) => (format!("{pct:.0}%"), true),
            None if has_record => (tally.record(), false),
            None => (format!("{}W", tally.wins), false),
        }
    };
    ListResult {
        text,
        is_rate,
        muted,
    }
}

/// Decided games for a list row: wins, losses and draws. An older server
/// sends no counts, so all of `games` counts as decided there, which keeps
/// the old 10 game floor for those rows.
pub fn list_decided(games: u32, wins: u32, losses: u32, draws: u32) -> u32 {
    match wins + losses + draws {
        0 => games,
        decided => decided,
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
/// decided games, below that the record labelled "Too few games". An empty season never reaches here;
/// the pages show their own empty state.
pub fn season_tile(season: Tally) -> Element {
    let value = season.label();
    let (label, record) = if season.has_win_rate() {
        ("Win Rate", Some(season.record()))
    } else {
        (TOO_FEW_GAMES, None)
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
    fn nine_decided_is_too_few_and_ten_decided_gets_a_win_rate() {
        let nine = Tally::new(9, 6, 2, 1);
        assert_eq!(nine.decided(), 9);
        assert!(!nine.has_win_rate() && nine.too_few());
        assert_eq!(nine.label(), "6W-2L-1D");

        let ten = Tally::new(10, 6, 2, 2);
        assert_eq!(ten.decided(), 10);
        assert!(ten.has_win_rate() && !ten.too_few());
        assert_eq!(ten.label(), "60.0%");
    }

    #[test]
    fn ten_plus_games_with_nine_decided_is_too_few() {
        let t = Tally::new(14, 6, 3, 0);
        assert!(!t.folds_away());
        assert_eq!(t.decided(), 9);
        assert!(!t.has_win_rate());
        assert!(t.too_few());
        assert_eq!(t.win_rate_pct(), None);
        assert_eq!(t.label(), "6W-3L");

        let r = list_result(14, 6, 3, 0, 6.0 / 14.0);
        assert_eq!(r.text, "6W-3L");
        assert!(!r.is_rate && !r.muted);
    }

    #[test]
    fn fold_counts_all_games_not_decided() {
        assert!(Tally::new(2, 0, 0, 0).folds_away());
        assert!(!Tally::new(3, 0, 0, 0).folds_away());
        assert!(Tally::new(3, 0, 0, 0).too_few());
        assert!(!Tally::default().too_few());
    }

    #[test]
    fn list_decided_falls_back_to_games_for_an_older_server() {
        assert_eq!(list_decided(12, 6, 3, 0), 9);
        assert_eq!(list_decided(12, 0, 0, 0), 12);
        assert_eq!(list_decided(0, 0, 0, 0), 0);
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
        assert_eq!(pool.decided(), 11);
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

    #[test]
    fn list_result_follows_the_rule() {
        let none = list_result(0, 0, 0, 0, 0.0);
        assert_eq!(none.text, "No games");
        assert!(none.muted && !none.is_rate);

        let two = list_result(2, 1, 1, 0, 0.5);
        assert_eq!(two.text, "1W-1L");
        assert!(two.muted && !two.is_rate);

        let three = list_result(3, 2, 0, 1, 2.0 / 3.0);
        assert_eq!(three.text, "2W-0L-1D");
        assert!(!three.muted && !three.is_rate);

        let nine = list_result(9, 5, 3, 1, 5.0 / 9.0);
        assert_eq!(nine.text, "5W-3L-1D");
        assert!(!nine.muted && !nine.is_rate);
        assert!(!nine.text.contains('%'), "{}", nine.text);

        let ten = list_result(10, 7, 2, 1, 0.7);
        assert_eq!(ten.text, "70%");
        assert!(!ten.muted && ten.is_rate);

        let undecided = list_result(11, 5, 3, 1, 5.0 / 11.0);
        assert_eq!(undecided.text, "5W-3L-1D");
        assert!(!undecided.muted && !undecided.is_rate);
    }

    #[test]
    fn list_result_draws_are_not_losses() {
        assert_eq!(list_result(4, 2, 1, 1, 0.5).text, "2W-1L-1D");
        assert_eq!(list_result(4, 2, 2, 0, 0.5).text, "2W-2L");
        assert_eq!(list_result(5, 0, 0, 5, 0.0).text, "0W-0L-5D");
    }

    #[test]
    fn list_result_falls_back_to_wins_from_an_older_server() {
        assert_eq!(list_result(7, 0, 0, 0, 4.0 / 7.0).text, "4W");
        assert_eq!(list_result(12, 0, 0, 0, 7.0 / 12.0).text, "58%");
        assert_eq!(list_result(5, 0, 0, 0, 1.5).text, "5W");
        assert_eq!(list_result(5, 0, 0, 0, -0.2).text, "0W");
        assert!(list_result(2, 0, 0, 0, 0.5).muted);
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
        fn undecided() -> Element {
            season_tile(Tally::new(12, 6, 3, 0))
        }
        let small = render(nine);
        assert!(
            small.contains("6W-3L") && small.contains("Too few games"),
            "{small}"
        );
        assert!(!small.contains('%'), "{small}");
        let open = render(undecided);
        assert!(open.contains("Too few games"), "{open}");
        assert!(!open.contains('%'), "{open}");
        let big = render(ten);
        assert!(big.contains("60.0%") && big.contains("Win Rate"), "{big}");
        assert!(big.contains("6W-4L"), "{big}");
    }

    #[test]
    fn notes_have_no_long_dashes() {
        for note in [HERO_RULE_NOTE, MAP_RULE_NOTE, LIST_RULE_NOTE, TOO_FEW_GAMES] {
            assert!(!note.contains('\u{2014}') && !note.contains('\u{2013}'));
        }
    }
}
