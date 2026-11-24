//! App and overlay wiring tests for companion page keys (0.7.1 C5).

#![cfg(test)]

use crate::app::Message;
use crate::cli::{Cli, FixtureKind};
use crate::companion_keys::{
    NEXT_TRIGGER, PageCmd, load_page_state, save_page_state, trigger_path,
};
use crate::fixtures::test_app;
use crate::overlay::OverlayApp;
use crate::overlay_pages::{self, Page, PageState, Stage};

fn companion_cli(data_dir: &std::path::Path, fixture: Option<FixtureKind>) -> Cli {
    Cli {
        data_dir: data_dir.to_path_buf(),
        fixture,
        seasons_url: None,
        companion: true,
        help: false,
        version: false,
        replace: false,
    }
}

#[test]
fn companion_page_message_and_tick_write_the_page_file() {
    let (dir, mut app) = test_app();
    app.fixture = None;
    let _ = app.update(Message::CompanionPage(PageCmd::Next));
    let state = load_page_state(dir.path());
    assert_eq!(state.page, Page::MapVote);

    let next = trigger_path(dir.path(), NEXT_TRIGGER);
    std::fs::write(&next, b"").unwrap();
    let _ = app.update(Message::Tick);
    assert_eq!(load_page_state(dir.path()).page, Page::MapVote);

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&next, b"").unwrap();
    let _ = app.update(Message::Tick);
    assert_eq!(load_page_state(dir.path()).page, Page::PicksAndBans);

    let status = stat_tracker::ocr_status::status_path(dir.path());
    std::fs::write(
        &status,
        r#"{"companion_stage":{"kind":"in_game","maps":[],"since":3}}"#,
    )
    .unwrap();
    let _ = app.update(Message::Tick);
    assert_eq!(load_page_state(dir.path()).page, Page::Tonight);
}

#[test]
fn new_companion_starts_on_tonight_and_keeps_hold() {
    let dir = tempfile::tempdir().unwrap();
    let saved = PageState {
        page: Page::PicksAndBans,
        held: true,
        stage: Stage::MapLocked { map_read: false },
        in_match: true,
    };
    save_page_state(dir.path(), saved).unwrap();
    let app = OverlayApp::load(&companion_cli(dir.path(), Some(FixtureKind::Empty)));
    assert_eq!(app.model.pages.page, Page::Tonight);
    assert!(app.model.pages.held);
    assert_eq!(app.model.pages.stage, Stage::MapLocked { map_read: false });
    assert!(app.model.pages.in_match);
    assert_eq!(load_page_state(dir.path()).page, Page::Tonight);
}

#[test]
fn fixture_refresh_loads_the_page_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = OverlayApp::load(&companion_cli(dir.path(), Some(FixtureKind::Empty)));
    assert_eq!(app.model.pages.page, Page::Tonight);
    let stepped = overlay_pages::next_page(app.model.pages);
    save_page_state(dir.path(), stepped).unwrap();
    app.refresh();
    assert_eq!(app.model.pages.page, Page::MapVote);
}

#[test]
fn live_refresh_loads_the_page_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = OverlayApp::load(&companion_cli(dir.path(), None));
    let stepped = overlay_pages::next_page(PageState::default());
    save_page_state(dir.path(), stepped).unwrap();
    app.refresh();
    assert_eq!(app.model.pages.page, Page::MapVote);
}
