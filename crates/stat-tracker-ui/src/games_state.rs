//! What the 0.6 stats pages (Overview, Heroes, Maps and Seasons) show
//! before the tracker has written its game list: a neutral waiting card,
//! not an empty state. The open game and the filter row use
//! [`crate::overview_page::settled_of`] and
//! [`crate::overview_page::shows_filters`].

use crate::app::TrackerApp;

/// Title and line while the tracker has not written its game list yet
/// (no `live_snapshot.json`): neutral, not an empty state.
pub const LOADING: &str = "Waiting for your games";
pub const LOADING_LINE: &str = "They show up here once the tracker has saved its game list.";

/// No game list on disk yet. Fixtures always have one.
pub fn loading(app: &TrackerApp) -> bool {
    app.fixture.is_none() && app.snapshot_mtime.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_only_before_the_first_game_list() {
        let (_dir, mut app) = crate::fixtures::test_app();
        assert!(!loading(&app), "a fixture has its list");
        app.fixture = None;
        app.snapshot_mtime = None;
        assert!(loading(&app));
        assert!(!crate::overview_page::shows_filters(&app));
        app.games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        assert!(
            !crate::overview_page::shows_filters(&app),
            "no filter row while loading"
        );
        app.snapshot_mtime = Some(std::time::SystemTime::now());
        assert!(!loading(&app));
        assert!(crate::overview_page::shows_filters(&app));
    }
}
