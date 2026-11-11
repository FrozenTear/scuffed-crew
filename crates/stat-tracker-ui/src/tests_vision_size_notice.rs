//! Tests for the capture size notice: what is shown for the tracker's value,
//! the pid and screen filters, each action and what it writes, a failed
//! write, the answers file reset, and the copy.

use std::cell::RefCell;
use std::path::Path;

use super::*;
use stat_tracker::config::SyncConfig;
use stat_tracker::packs::layout::vision_sizes::asks_path;

const UW: ScreenSize = ScreenSize::new(3440, 1440);
const WIDE_16_10: ScreenSize = ScreenSize::new(2560, 1600);

fn live(size: ScreenSize, unreadable: bool) -> Option<VisionSizeStatus> {
    Some(VisionSizeStatus {
        width: size.width,
        height: size.height,
        unreadable,
    })
}

fn asked(size: ScreenSize) -> VisionNotice {
    VisionNotice {
        shown: Shown::Ask(size),
        ..VisionNotice::default()
    }
}

fn declined(dir: &Path, size: ScreenSize) -> bool {
    Asks::load(dir).unwrap().declined(size)
}

/// A pack record that is there but does not read.
fn unreadable_pack_record(dir: &Path) {
    let templates = dir.join("templates");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(templates.join("pack-versions.json"), b"{not json").unwrap();
}

fn sync() -> SyncConfig {
    SyncConfig {
        server_url: "https://crew.example".into(),
        token: "tok".into(),
    }
}

fn status_from(pid: u32, vision: Option<VisionSizeStatus>) -> OcrSwitchStatus {
    OcrSwitchStatus {
        pid,
        vision_size: vision,
        ..OcrSwitchStatus::default()
    }
}

#[test]
fn a_new_size_is_shown_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), None, true);
    assert_eq!(notice.shown, Shown::Nothing);
    observe_in(&mut notice, dir.path(), live(UW, false), true);
    assert_eq!(notice.shown, Shown::Ask(UW));
    assert!(!asks_path(dir.path()).exists(), "seeing it records nothing");
    assert!(view_for(notice.shown, None, None).is_some());
    // An unanswered ask is shown again by a restarted app.
    let mut reopened = VisionNotice::default();
    observe_in(&mut reopened, dir.path(), live(UW, false), true);
    assert_eq!(reopened.shown, Shown::Ask(UW));
}

#[test]
fn without_a_sync_server_there_is_no_ask_but_warnings_still_show() {
    let dir = tempfile::tempdir().unwrap();
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), live(UW, false), false);
    assert_eq!(notice.shown, Shown::Nothing);
    assert!(!asks_path(dir.path()).exists(), "no answer recorded");
    // The pack record warning.
    unreadable_pack_record(dir.path());
    observe_in(&mut notice, dir.path(), live(UW, true), false);
    assert_eq!(notice.shown, Shown::PackUnreadable(UW));
    // The answers file warning.
    std::fs::write(asks_path(dir.path()), b"[").unwrap();
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), live(UW, false), false);
    assert_eq!(notice.shown, Shown::AnswersUnreadable(UW));
}

#[test]
fn no_thanks_hides_it_for_that_size_only_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), live(UW, false), true);
    answer_in(&mut notice, dir.path(), UW, Answer::Dismissed);
    assert_eq!(notice.shown, Shown::Nothing);
    assert_eq!(notice.failed, None);
    // The same value on the next tick does not bring it back.
    observe_in(&mut notice, dir.path(), live(UW, false), true);
    assert_eq!(notice.shown, Shown::Nothing);
    // A restarted app reads the answer from disk.
    let mut reopened = VisionNotice::default();
    observe_in(&mut reopened, dir.path(), live(UW, false), true);
    assert_eq!(reopened.shown, Shown::Nothing);
    // Another size is still asked.
    observe_in(&mut reopened, dir.path(), live(WIDE_16_10, false), true);
    assert_eq!(reopened.shown, Shown::Ask(WIDE_16_10));
}

#[test]
fn dont_ask_again_covers_every_size_after_a_reload() {
    let dir = tempfile::tempdir().unwrap();
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), live(UW, false), true);
    answer_in(&mut notice, dir.path(), UW, Answer::Never);
    assert_eq!(notice.shown, Shown::Nothing);
    assert!(declined(dir.path(), WIDE_16_10));
    observe_in(&mut notice, dir.path(), live(WIDE_16_10, false), true);
    assert_eq!(notice.shown, Shown::Nothing);
    let mut reopened = VisionNotice::default();
    observe_in(&mut reopened, dir.path(), live(WIDE_16_10, false), true);
    assert_eq!(reopened.shown, Shown::Nothing);
}

/// Blocking 2: a no that is not saved keeps the notice up and says so.
/// Put back the unconditional hide and this fails: the notice is gone.
#[test]
fn a_failed_answer_write_keeps_the_notice_and_shows_an_error() {
    // The data dir is a regular file: nothing under it can be read or
    // written.
    let file = tempfile::NamedTempFile::new().unwrap();
    // The answers path is a directory: the file cannot be read or renamed
    // over.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(asks_path(dir.path())).unwrap();
    for data_dir in [file.path(), dir.path()] {
        for answer in [Answer::Dismissed, Answer::Never] {
            let mut notice = asked(UW);
            answer_in(&mut notice, data_dir, UW, answer);
            assert_eq!(notice.shown, Shown::Ask(UW), "{answer:?}");
            assert_eq!(notice.failed, Some(Failure::Answer), "{answer:?}");
            assert!(view_for(notice.shown, notice.failed, None).is_some());
        }
    }
    assert!(asks_path(dir.path()).is_dir());
    // A write that works hides it and clears the error.
    let good = tempfile::tempdir().unwrap();
    let mut notice = asked(UW);
    notice.failed = Some(Failure::Answer);
    answer_in(&mut notice, good.path(), UW, Answer::Dismissed);
    assert_eq!(notice.shown, Shown::Nothing);
    assert_eq!(notice.failed, None);
    assert!(declined(good.path(), UW));
}

#[test]
fn a_late_no_for_an_older_size_leaves_the_newer_ask_up() {
    let dir = tempfile::tempdir().unwrap();
    let mut notice = asked(WIDE_16_10);
    answer_in(&mut notice, dir.path(), UW, Answer::Dismissed);
    assert_eq!(notice.shown, Shown::Ask(WIDE_16_10));
    assert!(declined(dir.path(), UW), "the older no is still kept");
    assert!(!declined(dir.path(), WIDE_16_10));
    // Close for the older size leaves it up too.
    close_in(&mut notice, UW);
    assert_eq!(notice.shown, Shown::Ask(WIDE_16_10));
}

#[test]
fn close_hides_it_and_records_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), live(UW, false), true);
    close_in(&mut notice, UW);
    assert_eq!(notice.shown, Shown::Nothing);
    assert!(!asks_path(dir.path()).exists());
    // This run: the same value keeps it hidden.
    observe_in(&mut notice, dir.path(), live(UW, false), true);
    assert_eq!(notice.shown, Shown::Nothing);
    // After a restart it can come back.
    let mut reopened = VisionNotice::default();
    observe_in(&mut reopened, dir.path(), live(UW, false), true);
    assert_eq!(reopened.shown, Shown::Ask(UW));
}

#[test]
fn an_unreadable_pack_record_is_a_warning_with_no_actions() {
    let dir = tempfile::tempdir().unwrap();
    unreadable_pack_record(dir.path());
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), live(UW, true), true);
    assert_eq!(notice.shown, Shown::PackUnreadable(UW));
    assert!(!asks_path(dir.path()).exists());
}

#[test]
fn a_reset_keeps_the_old_bytes_and_a_no_and_hides_the_warning() {
    let dir = tempfile::tempdir().unwrap();
    let corrupt: &[u8] = b"{\"dismissed\": [";
    std::fs::write(asks_path(dir.path()), corrupt).unwrap();
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), live(UW, true), true);
    assert_eq!(notice.shown, Shown::AnswersUnreadable(UW));
    reset_in(&mut notice, dir.path(), UW);
    assert_eq!(notice.shown, Shown::Nothing);
    assert_eq!(notice.failed, None);
    assert!(declined(dir.path(), UW));
    let copies: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("vision-size-asks.json.unreadable-")
        })
        .collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(std::fs::read(&copies[0]).unwrap(), corrupt);
}

#[test]
fn a_failed_reset_keeps_the_warning_and_the_unreadable_file() {
    let dir = tempfile::tempdir().unwrap();
    // A directory where the file should be: it does not read, and a reset
    // cannot copy it.
    std::fs::create_dir(asks_path(dir.path())).unwrap();
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), live(UW, false), true);
    assert_eq!(notice.shown, Shown::AnswersUnreadable(UW));
    reset_in(&mut notice, dir.path(), UW);
    assert_eq!(notice.shown, Shown::AnswersUnreadable(UW));
    assert_eq!(notice.failed, Some(Failure::Reset));
    assert!(asks_path(dir.path()).is_dir(), "left in place");
    assert!(view_for(notice.shown, notice.failed, None).is_some());
}

#[test]
fn the_tracker_clearing_it_hides_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut notice = VisionNotice::default();
    observe_in(&mut notice, dir.path(), live(UW, false), true);
    observe_in(&mut notice, dir.path(), None, true);
    assert_eq!(notice.shown, Shown::Nothing);
}

#[test]
fn only_the_running_trackers_value_counts() {
    let status = status_from(7, live(UW, false));
    assert_eq!(live_vision_size(Some(&status), Some(7)), live(UW, false));
    assert_eq!(live_vision_size(Some(&status), Some(8)), None);
    assert_eq!(live_vision_size(Some(&status), None), None);
    assert_eq!(live_vision_size(None, Some(7)), None);

    let (_dir, mut app) = crate::fixtures::test_app();
    app.saved_config.sync = Some(sync());
    app.daemon.pid = Some(8);
    observe(&mut app, Some(&status));
    assert_eq!(
        app.overview_page.vision.shown,
        Shown::Nothing,
        "another run"
    );
    app.daemon.pid = Some(7);
    observe(&mut app, Some(&status));
    assert_eq!(app.overview_page.vision.shown, Shown::Ask(UW));
}

#[test]
fn observe_without_a_sync_server_asks_nothing() {
    let (_dir, mut app) = crate::fixtures::test_app();
    app.saved_config.sync = None;
    app.daemon.pid = Some(7);
    observe(&mut app, Some(&status_from(7, live(UW, false))));
    assert_eq!(app.overview_page.vision.shown, Shown::Nothing);
    assert!(!asks_path(&app.data_dir).exists());
}

#[test]
fn the_notice_is_drawn_only_on_overview() {
    let (_dir, mut app) = crate::fixtures::test_app();
    app.saved_config.sync = Some(sync());
    app.overview_page.vision = asked(UW);
    app.screen = Screen::Overview;
    assert_eq!(shown_on(&app), Shown::Ask(UW));
    assert!(view(&app).is_some());
    for other in [Screen::Games, Screen::Heroes] {
        app.screen = other;
        assert_eq!(shown_on(&app), Shown::Nothing, "{other:?}");
        assert!(view(&app).is_none(), "{other:?}");
    }
    // No sync server: no ask, even on Overview. A warning still shows.
    app.screen = Screen::Overview;
    app.saved_config.sync = None;
    assert_eq!(shown_on(&app), Shown::Nothing);
    assert!(view(&app).is_none());
    app.overview_page.vision.shown = Shown::PackUnreadable(UW);
    assert_eq!(shown_on(&app), Shown::PackUnreadable(UW));
    assert!(view(&app).is_some());
}

#[test]
fn each_message_does_what_its_button_says() {
    let (_dir, mut app) = crate::fixtures::test_app();
    app.saved_config.sync = Some(sync());
    let dir = app.data_dir.clone();
    let opened = RefCell::new(Vec::new());
    let open = |url: &str| opened.borrow_mut().push(url.to_string());

    // Open website profile: the link, nothing recorded, the ask stays.
    app.overview_page.vision = asked(UW);
    let url = profile_url("https://crew.example").unwrap();
    update_with(&mut app, Msg::OpenProfile(url.clone()), open);
    assert_eq!(*opened.borrow(), [url]);
    assert_eq!(app.overview_page.vision.shown, Shown::Ask(UW));
    assert!(!asks_path(&dir).exists());

    // Close: hidden, nothing recorded.
    update_with(&mut app, Msg::Close(UW), |_| panic!("no link"));
    assert_eq!(app.overview_page.vision.shown, Shown::Nothing);
    assert!(!asks_path(&dir).exists());

    // No thanks: hidden, a no for that size only.
    app.overview_page.vision = asked(UW);
    update_with(&mut app, Msg::NoThanks(UW), |_| panic!("no link"));
    assert_eq!(app.overview_page.vision.shown, Shown::Nothing);
    let asks = Asks::load(&dir).unwrap();
    assert!(asks.declined(UW) && !asks.declined(WIDE_16_10));

    // Don't ask again: hidden, a no for every size.
    app.overview_page.vision = asked(WIDE_16_10);
    update_with(&mut app, Msg::Never(WIDE_16_10), |_| panic!("no link"));
    assert_eq!(app.overview_page.vision.shown, Shown::Nothing);
    assert!(Asks::load(&dir).unwrap().never_again);

    // Reset: the unreadable file is replaced, a copy kept.
    std::fs::write(asks_path(&dir), b"[").unwrap();
    app.overview_page.vision = VisionNotice {
        shown: Shown::AnswersUnreadable(UW),
        ..VisionNotice::default()
    };
    update_with(&mut app, Msg::Reset(UW), |_| panic!("no link"));
    assert_eq!(app.overview_page.vision.shown, Shown::Nothing);
    assert!(declined(&dir, UW));
}

#[test]
fn the_profile_link_is_the_sites_profile_page() {
    for (server, want) in [
        ("https://crew.example", "https://crew.example/profile/edit"),
        ("https://crew.example/", "https://crew.example/profile/edit"),
        (
            "  https://crew.example/sync/api/  ",
            "https://crew.example/profile/edit",
        ),
        (
            "https://crew.example/?team=1#top",
            "https://crew.example/profile/edit",
        ),
        (
            "https://crew.example:8443/x?y=z#w",
            "https://crew.example:8443/profile/edit",
        ),
        (
            "http://127.0.0.1:3030",
            "http://127.0.0.1:3030/profile/edit",
        ),
    ] {
        assert_eq!(profile_url(server).as_deref(), Some(want), "{server}");
    }
    for bad in [
        "",
        "  ",
        "crew.example",
        "not a url",
        "mailto:a@b.example",
        "file:///tmp",
    ] {
        assert_eq!(profile_url(bad), None, "{bad}");
    }
}

#[test]
fn the_copy_is_short_plain_and_has_no_dashes() {
    for line in [
        ask_title(UW),
        pack_unreadable_text(UW),
        answers_unreadable_text(UW),
        ASK_BODY.to_string(),
        OPEN_PROFILE.to_string(),
        NO_THANKS.to_string(),
        NEVER.to_string(),
        CLOSE.to_string(),
        RESET.to_string(),
        RESET_HINT.to_string(),
        ANSWER_NOT_SAVED.to_string(),
        RESET_FAILED.to_string(),
    ] {
        assert!(!line.contains('\u{2014}'), "{line}");
        assert!(!line.contains('\u{2013}'), "{line}");
        assert!(!line.trim().is_empty());
    }
    assert!(ask_title(UW).contains("3440x1440"));
    assert!(ASK_BODY.contains("still record"));
    assert!(!ASK_BODY.contains("OCR"));
    assert!(pack_unreadable_text(UW).contains("still record"));
    assert!(answers_unreadable_text(UW).contains("still record"));
    assert!(answers_unreadable_text(UW).contains(ASKS_FILE));
}
