//! The poll tick's pre-game want ([`super::event_loop::pregame_tick_want`]):
//! the match skip is set from the session before the want, on every tick.

use super::event_loop::pregame_tick_want;
use super::pregame_dump::*;
use crate::*;
use std::time::Duration;

/// A session game with a stored scoreboard read at `at` and no result.
fn board_game(at: Instant) -> ActiveGame {
    let mut game = ActiveGame::open_at("s1".into(), detect::MatchOutcome::Unknown, Vec::new(), at);
    game.session_created = true;
    game
}

#[test]
fn each_tick_sets_the_match_skip_from_the_session_before_its_want() {
    let dir = tempfile::tempdir().unwrap();
    let (writer, _queue) = PregameWriter::stalled();
    let mut dump = PregameDump::with_writer(
        dir.path().join("pregame"),
        Duration::from_millis(1_500),
        writer,
    );
    let start = Instant::now();
    let game = board_game(start);
    // The first tick of a match in progress, no folder open: the skip is
    // already on in this tick's want. Taken before the skip is set, the
    // want would still look for the roster and loading screen.
    let (want, _) = pregame_tick_want(Some(&mut dump), Some(&game), start, false).expect("on");
    assert_eq!(want, PregameWant::default());
    // With no game in the session the same tick looks again.
    let (want, _) = pregame_tick_want(Some(&mut dump), None, start, false).expect("on");
    assert!(want.loading && want.roster, "{want:?}");
    // Back to the match: off again, set from the session on this tick.
    let t1 = start + Duration::from_secs(1);
    let (want, _) = pregame_tick_want(Some(&mut dump), Some(&game), t1, false).expect("on");
    assert_eq!(want, PregameWant::default());
    // The window after the last scoreboard read passed: on, on that tick.
    let late = start + MATCH_SKIP_IDLE + Duration::from_secs(1);
    let (want, _) = pregame_tick_want(Some(&mut dump), Some(&game), late, false).expect("on");
    assert!(want.loading && want.roster, "{want:?}");
    // A Tab-busy tick hands nothing over.
    assert!(pregame_tick_want(Some(&mut dump), Some(&game), t1, true).is_none());
    // The switch off: nothing.
    assert!(pregame_tick_want(None, Some(&game), start, false).is_none());
}
