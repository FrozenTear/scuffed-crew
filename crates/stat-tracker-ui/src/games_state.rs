//! What the 0.6 stats pages (Heroes, Maps, Seasons) count before they
//! draw: nothing while the tracker has not written its game list yet, and
//! never the open game, whose result is not in.

use crate::app::TrackerApp;
use crate::model::{Game, game_card_in_progress};

/// Title and line while the tracker has not written its game list yet
/// (no `live_snapshot.json`): neutral, not an empty state.
pub const LOADING: &str = "Waiting for your games";
pub const LOADING_LINE: &str = "They show up here once the tracker has saved its game list.";

/// No game list on disk yet. Fixtures always have one.
pub fn loading(app: &TrackerApp) -> bool {
    app.fixture.is_none() && app.snapshot_mtime.is_none()
}

/// `games` less the open game (`live`): it counts once it closes, so an
/// undecided count never includes it and matches Needs a look.
pub fn settled<'a>(mut games: Vec<&'a Game>, live: Option<&str>) -> Vec<&'a Game> {
    games.retain(|g| !game_card_in_progress(&g.session_id, g.outcome, live));
    games
}

/// Whether a page draws its filter row and header: not while loading and
/// not with no games at all (ux.md section 17).
pub fn shows_filters(app: &TrackerApp) -> bool {
    !loading(app) && !app.games.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Outcome;

    #[test]
    fn loading_only_before_the_first_game_list() {
        let (_dir, mut app) = crate::fixtures::test_app();
        assert!(!loading(&app), "a fixture has its list");
        assert!(!shows_filters(&app), "no games");
        app.fixture = None;
        app.snapshot_mtime = None;
        assert!(loading(&app));
        app.snapshot_mtime = Some(std::time::SystemTime::now());
        assert!(!loading(&app));
        app.games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        assert!(shows_filters(&app));
    }

    #[test]
    fn the_open_game_waits_for_its_result() {
        let mut games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        games.truncate(2);
        games[0].session_id = "live".into();
        games[0].outcome = Outcome::Unknown;
        games[1].session_id = "done".into();
        games[1].outcome = Outcome::Unknown;
        let refs: Vec<&Game> = games.iter().collect();
        let kept = settled(refs.clone(), Some("live"));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].session_id, "done");
        assert_eq!(settled(refs, None).len(), 2);
    }
}
