//! Pre-game screens seen for one tick only (a quick screen, or the focus
//! gate pausing the poll right after the first tick), next to
//! `pregame_dump.rs`.

use super::pregame_dump::*;
use super::tests_pregame_dump::Run;

const VOTE: Seen = Seen::Phase(PregamePhase::MapVote);
const BAN: Seen = Seen::Phase(PregamePhase::HeroBan);
const SELECT: Seen = Seen::Phase(PregamePhase::HeroSelect);

#[test]
fn a_one_tick_phase_writes_only_its_first_frame() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(1_000, SELECT, 2);
    run.tick(2_000, Seen::Other, 3);
    assert_eq!(
        run.names(),
        [("mapvote_first.png", 1), ("heroselect_first.png", 2)]
    );
}

#[test]
fn a_one_tick_hero_ban_paused_by_the_focus_gate_still_writes_its_last_frame() {
    // One ban tick, the focus gate pauses the poll, and it resumes on the
    // roster: the ban's last frame (the same pixels as its first) is
    // written so the bans reader has a vote frame.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(28_000, Seen::Roster, 2);
    run.tick(29_500, Seen::Other, 3);
    let saved = run.names();
    assert!(saved.contains(&("heroban_first.png", 1)), "{saved:?}");
    assert!(saved.contains(&("heroban_last.png", 1)), "{saved:?}");
    assert!(
        run.shots
            .iter()
            .any(|(folder, n, _)| folder == "game1" && *n == "heroban_last.png")
    );
    // The other one-tick screens still write only their first frame.
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(1_500, Seen::Other, 2);
    assert_eq!(run.names(), [("mapvote_first.png", 1)]);
}
