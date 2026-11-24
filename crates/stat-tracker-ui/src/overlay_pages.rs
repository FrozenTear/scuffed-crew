//! Companion pages (0.7.1, companion-spec.md sections 1 and 3, option 3).
//!
//! Plain functions over a small state: which page shows, whether the
//! safe-page hold is on, and whether the match has started. Tonight is the
//! stream page and the default.
//!
//! - Auto-advance (on unless "Hold the stream page" is on): Map vote when the
//!   three vote maps were read, then Picks and bans when the map locks. A
//!   stage whose maps were not read leaves the page on Tonight.
//! - Auto-return to Tonight at match start (the loading screen or the first
//!   live Tab), always, whatever the hold or the setting says. Match start
//!   also sets an in-match flag. While that flag is set, a later locked-map
//!   stage does not auto-advance (a hero select mid-match must not put
//!   Picks and bans up on stream). The next map-vote stage clears the flag,
//!   so the next game can auto-advance again.
//! - The safe-page key jumps to Tonight and holds there, suppressing
//!   auto-advance, until the next next-page press or the hold is cleared.
//!
//! Only a change of stage acts, so a page the player stepped to by key is
//! not pulled back on every tick of the same stage. [`Stage::None`] is a
//! no-op (the page and the last applied stage stay as they are): a missing
//! status blip must not override a key-picked page, must not reveal hints,
//! and must not re-arm auto-advance.
//!
//! How a caller maps C3's status `companion_stage` onto [`Stage`]:
//! - missing, unknown, or a cleared field: [`Stage::None`] (a no-op).
//! - `map_vote`: [`Stage::MapVote`], with `maps_read` from
//!   [`vote_maps_read`]. A second vote with other maps is still
//!   `MapVote { maps_read: true }`, so it is not a stage change and does
//!   nothing. A cancelled vote that becomes `None` leaves Map vote up.
//! - `hero_select` / `hero_ban`: [`Stage::MapLocked`]. C3 lock stages
//!   carry no map name, so a caller that only has that field treats the
//!   lock itself as `map_read: true`.
//! - `in_game`: [`Stage::MatchStart`].

use serde::{Deserialize, Serialize};

/// One companion page. The first four are the in-match pages, in dot order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    /// The stream page: no picks, bans, hints or death rates.
    #[default]
    Tonight,
    MapVote,
    PicksAndBans,
    Matchup,
    /// Before a match or between games only. Not one of the in-match dots.
    Tip,
}

impl Page {
    /// The in-match pages, in the order of the four dots.
    pub const IN_MATCH: [Page; 4] = [
        Page::Tonight,
        Page::MapVote,
        Page::PicksAndBans,
        Page::Matchup,
    ];

    /// The page name as the indicator shows it (spec section 3).
    pub fn name(self) -> &'static str {
        match self {
            Page::Tonight => "Tonight",
            Page::MapVote => "Map vote",
            Page::PicksAndBans => "Picks and bans",
            Page::Matchup => "Matchup",
            Page::Tip => "Tip",
        }
    }

    /// The page's dot (0 to 3), or `None` for Tip, which has no dots.
    pub fn dot(self) -> Option<usize> {
        Self::IN_MATCH.iter().position(|p| *p == self)
    }
}

/// What the tracker last said about the pre-game stage, as the page rules
/// see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// No pre-game screen known.
    #[default]
    None,
    /// The map vote. `maps_read`: the three candidate maps were read.
    MapVote { maps_read: bool },
    /// Hero select or hero ban: the map is locked. C3's lock stages carry
    /// no map name. `map_read` is for a caller that has one; a caller that
    /// only has the C3 field treats the lock itself as `map_read: true`.
    MapLocked { map_read: bool },
    /// The loading screen or the first live Tab of the match.
    MatchStart,
}

/// True when the vote read gave three map names.
pub fn vote_maps_read(maps: &[String]) -> bool {
    maps.len() == 3 && maps.iter().all(|m| !m.trim().is_empty())
}

/// The page on show, the safe-page hold, and whether the match has started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PageState {
    pub page: Page,
    /// The safe-page hold: Tonight stays up and auto-advance is off.
    pub held: bool,
    /// The stage last applied, so only a change acts. [`Stage::None`]
    /// never writes this field.
    pub stage: Stage,
    /// Set at [`Stage::MatchStart`], cleared on the next [`Stage::MapVote`].
    /// While set, [`Stage::MapLocked`] does not auto-advance.
    pub in_match: bool,
}

/// Apply a stage from the tracker. `hold_stream_page` is the Settings
/// switch: on, auto-advance is off. Auto-return at match start always runs.
/// [`Stage::None`] returns `state` unchanged.
pub fn on_stage(state: PageState, stage: Stage, hold_stream_page: bool) -> PageState {
    if stage == Stage::None || stage == state.stage {
        return state;
    }
    let in_match = match stage {
        Stage::MatchStart => true,
        Stage::MapVote { .. } => false,
        _ => state.in_match,
    };
    let advance = !hold_stream_page && !state.held;
    let page = match stage {
        Stage::MatchStart => Page::Tonight,
        Stage::MapVote { maps_read: true } if advance => Page::MapVote,
        Stage::MapLocked { map_read: true } if advance && !in_match => Page::PicksAndBans,
        // The vote page went up for maps that are no longer the question,
        // and the locked map was not read: back to Tonight.
        Stage::MapLocked { map_read: false } if state.page == Page::MapVote => Page::Tonight,
        _ => state.page,
    };
    PageState {
        page,
        held: state.held,
        stage,
        in_match,
    }
}

/// The next-page key: clears the hold and steps to the next in-match page,
/// from Matchup back round to Tonight. From Tip it goes to Tonight.
pub fn next_page(state: PageState) -> PageState {
    let page = match state.page.dot() {
        Some(i) => Page::IN_MATCH[(i + 1) % Page::IN_MATCH.len()],
        None => Page::Tonight,
    };
    PageState {
        page,
        held: false,
        ..state
    }
}

/// The previous-page key: steps back, from Tonight round to Matchup. While
/// the hold is on, Tonight stays (only next page or a clear lifts it). From
/// Tip it goes to Tonight.
pub fn previous_page(state: PageState) -> PageState {
    if state.held {
        return PageState {
            page: Page::Tonight,
            ..state
        };
    }
    let n = Page::IN_MATCH.len();
    let page = match state.page.dot() {
        Some(i) => Page::IN_MATCH[(i + n - 1) % n],
        None => Page::Tonight,
    };
    PageState { page, ..state }
}

/// The safe-page key: Tonight, held.
pub fn safe_page(state: PageState) -> PageState {
    PageState {
        page: Page::Tonight,
        held: true,
        ..state
    }
}

/// Clear the safe-page hold. The page stays where it is.
pub fn clear_hold(state: PageState) -> PageState {
    PageState {
        held: false,
        ..state
    }
}

/// What the page indicator shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indicator {
    /// The lit dot of four, or `None` on Tip (no dots, the word "Tip").
    pub dot: Option<usize>,
    pub name: &'static str,
    /// Show the "Held on the stream page" tag, and the safe-page line in
    /// the footer.
    pub held: bool,
}

/// The tag beside the page name while the hold is on.
pub const HELD_TAG: &str = "Held on the stream page";

/// The footer line while the hold is on, for a safe-page bind.
pub fn safe_page_footer(bind: &str) -> String {
    format!("Safe page with {bind}")
}

/// The safe-page bind the spec names (Settings can change it).
pub const DEFAULT_SAFE_BIND: &str = "Super+Shift+T";

pub fn indicator(state: &PageState) -> Indicator {
    Indicator {
        dot: state.page.dot(),
        name: state.page.name(),
        held: state.held && state.page == Page::Tonight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOTE: Stage = Stage::MapVote { maps_read: true };
    const VOTE_UNREAD: Stage = Stage::MapVote { maps_read: false };
    const LOCKED: Stage = Stage::MapLocked { map_read: true };
    const LOCKED_UNREAD: Stage = Stage::MapLocked { map_read: false };

    fn run(stages: &[Stage], hold_stream_page: bool) -> PageState {
        stages.iter().fold(PageState::default(), |s, st| {
            on_stage(s, *st, hold_stream_page)
        })
    }

    #[test]
    fn tonight_is_the_default() {
        let s = PageState::default();
        assert_eq!(s.page, Page::Tonight);
        assert!(!s.held);
        assert_eq!(indicator(&s).name, "Tonight");
        assert_eq!(indicator(&s).dot, Some(0));
    }

    #[test]
    fn auto_advance_follows_vote_then_lock() {
        let s = run(&[VOTE], false);
        assert_eq!(s.page, Page::MapVote);
        let s = on_stage(s, LOCKED, false);
        assert_eq!(s.page, Page::PicksAndBans);
        // Hero select then hero ban: both are the locked map, no change.
        let s = on_stage(s, LOCKED, false);
        assert_eq!(s.page, Page::PicksAndBans);
    }

    #[test]
    fn auto_return_at_match_start_always() {
        let s = run(&[VOTE, LOCKED, Stage::MatchStart], false);
        assert_eq!(s.page, Page::Tonight);
        // From a page reached by key, with the setting on: still Tonight.
        let s = PageState {
            page: Page::Matchup,
            ..PageState::default()
        };
        assert_eq!(on_stage(s, Stage::MatchStart, true).page, Page::Tonight);
        // A hold stays on through match start.
        let held = safe_page(PageState::default());
        assert!(on_stage(held, Stage::MatchStart, false).held);
    }

    #[test]
    fn missing_maps_stay_on_tonight() {
        assert_eq!(run(&[VOTE_UNREAD], false).page, Page::Tonight);
        assert_eq!(
            run(&[VOTE_UNREAD, LOCKED_UNREAD], false).page,
            Page::Tonight
        );
        // Vote read, locked map not: the vote page does not stay up.
        assert_eq!(run(&[VOTE, LOCKED_UNREAD], false).page, Page::Tonight);
        // Vote not read, locked map read: Picks and bans.
        assert_eq!(run(&[VOTE_UNREAD, LOCKED], false).page, Page::PicksAndBans);
        assert!(vote_maps_read(&["A".into(), "B".into(), "C".into()]));
        assert!(!vote_maps_read(&["A".into(), "B".into()]));
        assert!(!vote_maps_read(&["A".into(), " ".into(), "C".into()]));
    }

    #[test]
    fn hold_the_stream_page_suppresses_auto_advance() {
        assert_eq!(run(&[VOTE], true).page, Page::Tonight);
        assert_eq!(run(&[VOTE, LOCKED], true).page, Page::Tonight);
        // Keys still work with the setting on.
        let s = next_page(run(&[VOTE], true));
        assert_eq!(s.page, Page::MapVote);
    }

    #[test]
    fn safe_page_holds_tonight_until_next_page() {
        let s = run(&[VOTE], false);
        let s = safe_page(s);
        assert_eq!(s.page, Page::Tonight);
        assert!(s.held);
        let ind = indicator(&s);
        assert!(ind.held);
        assert_eq!(ind.name, "Tonight");
        // Auto-advance is suppressed while held.
        let s = on_stage(s, LOCKED, false);
        assert_eq!(s.page, Page::Tonight);
        assert!(s.held);
        // Previous page does not lift the hold.
        let s = previous_page(s);
        assert_eq!(s.page, Page::Tonight);
        assert!(s.held);
        // Next page lifts it and steps on.
        let s = next_page(s);
        assert!(!s.held);
        assert_eq!(s.page, Page::MapVote);
        // A later real stage change (the next game's vote, then lock)
        // advances again. A None blip would not re-arm the last lock.
        let s = on_stage(s, VOTE, false);
        let s = on_stage(s, LOCKED, false);
        assert_eq!(s.page, Page::PicksAndBans);
    }

    #[test]
    fn clearing_the_hold_lets_auto_advance_run() {
        let s = clear_hold(safe_page(PageState::default()));
        assert!(!s.held);
        assert_eq!(s.page, Page::Tonight);
        assert_eq!(on_stage(s, VOTE, false).page, Page::MapVote);
    }

    #[test]
    fn the_same_stage_does_not_pull_a_page_back() {
        let s = run(&[VOTE], false);
        let s = previous_page(s);
        assert_eq!(s.page, Page::Tonight);
        assert_eq!(on_stage(s, VOTE, false).page, Page::Tonight);
    }

    #[test]
    fn next_and_previous_wrap_over_the_four_pages() {
        let mut s = PageState::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            s = next_page(s);
            seen.push(s.page);
        }
        assert_eq!(
            seen,
            [
                Page::MapVote,
                Page::PicksAndBans,
                Page::Matchup,
                Page::Tonight
            ]
        );
        assert_eq!(previous_page(PageState::default()).page, Page::Matchup);
        let tip = PageState {
            page: Page::Tip,
            ..PageState::default()
        };
        assert_eq!(next_page(tip).page, Page::Tonight);
        assert_eq!(previous_page(tip).page, Page::Tonight);
    }

    #[test]
    fn indicator_names_and_tip_has_no_dots() {
        let names: Vec<_> = Page::IN_MATCH.iter().map(|p| p.name()).collect();
        assert_eq!(names, ["Tonight", "Map vote", "Picks and bans", "Matchup"]);
        let tip = PageState {
            page: Page::Tip,
            ..PageState::default()
        };
        assert_eq!(indicator(&tip).dot, None);
        assert_eq!(indicator(&tip).name, "Tip");
        assert_eq!(HELD_TAG, "Held on the stream page");
        assert_eq!(
            safe_page_footer(DEFAULT_SAFE_BIND),
            "Safe page with Super+Shift+T"
        );
    }

    #[test]
    fn mid_match_lock_does_not_show_picks_and_bans() {
        let s = run(&[VOTE, LOCKED, Stage::MatchStart, LOCKED], false);
        assert_eq!(s.page, Page::Tonight);
        assert!(s.in_match);
        let s = run(&[Stage::MatchStart, LOCKED], false);
        assert_eq!(s.page, Page::Tonight);
        assert!(s.in_match);
    }

    #[test]
    fn next_game_vote_clears_in_match_and_advances() {
        let s = run(&[VOTE, LOCKED, Stage::MatchStart, VOTE], false);
        assert_eq!(s.page, Page::MapVote);
        assert!(!s.in_match);
        let s = on_stage(s, LOCKED, false);
        assert_eq!(s.page, Page::PicksAndBans);
    }

    #[test]
    fn none_blip_does_not_override_a_key_or_reveal_hints() {
        let voted = run(&[VOTE], false);
        assert_eq!(on_stage(voted, Stage::None, false), voted);
        let keyed = PageState {
            page: Page::Matchup,
            stage: VOTE,
            ..PageState::default()
        };
        assert_eq!(on_stage(keyed, Stage::None, false), keyed);
        let s = run(&[Stage::MatchStart], false);
        let s = on_stage(s, Stage::None, false);
        assert_eq!(s.stage, Stage::MatchStart);
        let s = on_stage(s, LOCKED, false);
        assert_eq!(s.page, Page::Tonight);
        assert!(s.in_match);
        // The lock is recorded, but in-match skips the hint page.
        assert_eq!(s.stage, LOCKED);
    }

    #[test]
    fn second_vote_and_cancelled_vote_leave_map_vote() {
        let s = run(&[VOTE], false);
        assert_eq!(s.page, Page::MapVote);
        // A second vote with other maps is the same Stage value (this
        // type does not hold the names), so it does nothing.
        assert_eq!(on_stage(s, VOTE, false), s);
        // A cancelled vote that becomes None is a no-op: Map vote stays.
        assert_eq!(on_stage(s, Stage::None, false).page, Page::MapVote);
    }
}
