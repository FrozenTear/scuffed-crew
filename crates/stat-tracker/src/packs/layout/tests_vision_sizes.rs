//! Tests for `vision_sizes`: the size each pack name declares, the support
//! signal read from a fake pack record, the once-per-size ask, answers that
//! survive a restart, and the collection gate.

use std::cell::Cell;
use std::path::Path;

use super::*;

const UW: ScreenSize = ScreenSize::new(3440, 1440);
const WIDE_16_10: ScreenSize = ScreenSize::new(2560, 1600);
const P1080: ScreenSize = ScreenSize::new(1920, 1080);

/// A pack record the way sync writes it: pack name to version.
fn write_record(data_dir: &Path, names: &[&str]) {
    let record: serde_json::Map<String, serde_json::Value> = names
        .iter()
        .map(|name| ((*name).to_string(), serde_json::Value::from("1")))
        .collect();
    let path = super::super::versions_path(data_dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
}

/// The packs shipped today.
fn shipped(data_dir: &Path) {
    write_record(
        data_dir,
        &[
            "heroes-v1.tar",
            "heroes-v2.tar",
            "heroes-v2-1080.tar",
            "maps-v1.tar",
            "names-v1.tar",
            "bans-picks-v1.tar",
        ],
    );
}

#[test]
fn sizes_parse_and_print() {
    assert_eq!(ScreenSize::parse("3440x1440"), Some(UW));
    assert_eq!(ScreenSize::parse(" 2560X1600 "), Some(WIDE_16_10));
    assert_eq!(UW.to_string(), "3440x1440");
    for bad in [
        "", "x", "3440", "3440x", "0x1440", "3440x0", "-1x2", "1x2x3", "a1x2",
    ] {
        assert_eq!(ScreenSize::parse(bad), None, "{bad}");
    }
}

#[test]
fn each_hero_pack_name_declares_its_size() {
    assert_eq!(declared_size("heroes-v1.tar"), Some(BASE_SIZE));
    assert_eq!(declared_size("heroes-v2.tar"), Some(BASE_SIZE));
    assert_eq!(declared_size("heroes-v2-1080.tar"), Some(P1080));
    assert_eq!(declared_size("heroes-v2-1440.tar"), Some(BASE_SIZE));
    assert_eq!(declared_size("heroes-v3-3440x1440.tar"), Some(UW));
    assert_eq!(declared_size("heroes-v3-2560x1600"), Some(WIDE_16_10));
    // Not hero packs: no size.
    for other in [
        "maps-v1.tar",
        "names-v1.tar",
        "bans-picks-v1.tar",
        "words-v1.tar",
    ] {
        assert_eq!(declared_size(other), None, "{other}");
    }
}

#[test]
fn the_shipped_packs_cover_1440p_and_1080p_only() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    assert_eq!(support_in(dir.path(), BASE_SIZE), Support::Supported);
    assert_eq!(support_in(dir.path(), P1080), Support::Supported);
    assert_eq!(support_in(dir.path(), UW), Support::Unsupported);
    assert_eq!(support_in(dir.path(), WIDE_16_10), Support::Unsupported);
}

#[test]
fn a_new_pack_that_declares_a_size_stops_the_ask_for_it() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    let mut before = Asker::default();
    assert_eq!(
        before.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        Some(Decision::Ask(UW))
    );
    write_record(
        dir.path(),
        &[
            "heroes-v2.tar",
            "heroes-v2-1080.tar",
            "heroes-v3-3440x1440.tar",
        ],
    );
    assert_eq!(support_in(dir.path(), UW), Support::Supported);
    let mut after = Asker::default();
    assert_eq!(
        after.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        Some(Decision::Quiet)
    );
}

#[test]
fn no_record_or_no_hero_pack_is_not_known_yet_and_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(support_in(dir.path(), UW), Support::NotYetKnown);
    write_record(dir.path(), &["maps-v1.tar"]);
    assert_eq!(support_in(dir.path(), UW), Support::NotYetKnown);
    // Not settled: the next Tab, after the packs land, asks.
    let mut asker = Asker::default();
    let quiet = asker.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path()));
    assert_eq!(quiet, Some(Decision::Quiet));
    shipped(dir.path());
    let ask = asker.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path()));
    assert_eq!(ask, Some(Decision::Ask(UW)));
}

#[test]
fn an_unreadable_record_warns_and_never_asks() {
    let dir = tempfile::tempdir().unwrap();
    let path = super::super::versions_path(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{not json").unwrap();
    assert!(matches!(support_in(dir.path(), UW), Support::Unreadable(_)));
    let mut asker = Asker::default();
    let decision = asker.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path()));
    assert!(matches!(decision, Some(Decision::Warn(size, _)) if size == UW));
    // Once per run: the warning is not repeated.
    assert_eq!(
        asker.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        None
    );
}

#[test]
fn an_unreadable_answers_file_warns_and_never_asks() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    std::fs::write(asks_path(dir.path()), b"[").unwrap();
    let mut asker = Asker::default();
    let decision = asker.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path()));
    assert!(matches!(decision, Some(Decision::Warn(size, _)) if size == UW));
}

#[test]
fn the_first_unsupported_size_is_asked_and_a_supported_size_never_is() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    let mut asker = Asker::default();
    for supported in [BASE_SIZE, P1080] {
        let asks_read = Cell::new(false);
        let decision = asker.decide(
            supported,
            || support_in(dir.path(), supported),
            || {
                asks_read.set(true);
                Asks::load(dir.path())
            },
        );
        assert_eq!(decision, Some(Decision::Quiet), "{supported}");
        assert!(!asks_read.get(), "a supported size reads no answers");
    }
    assert_eq!(
        asker.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        Some(Decision::Ask(UW))
    );
}

#[test]
fn the_same_size_is_asked_once_and_a_different_one_once_too() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    let mut asker = Asker::default();
    let mut decide = |size: ScreenSize| {
        asker.decide(
            size,
            || support_in(dir.path(), size),
            || Asks::load(dir.path()),
        )
    };
    assert_eq!(decide(UW), Some(Decision::Ask(UW)));
    // A second game at the same size: nothing new.
    assert_eq!(decide(UW), None);
    assert_eq!(decide(WIDE_16_10), Some(Decision::Ask(WIDE_16_10)));
    assert_eq!(decide(WIDE_16_10), None);
}

#[test]
fn shown_closed_and_dont_ask_again_survive_a_restart() {
    for answer in [Answer::Asked, Answer::Dismissed, Answer::Never] {
        let dir = tempfile::tempdir().unwrap();
        shipped(dir.path());
        record_answer(dir.path(), UW, answer).unwrap();
        // A new run: a new asker, the answers read from disk again.
        let mut asker = Asker::default();
        assert_eq!(
            asker.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
            Some(Decision::Quiet),
            "{answer:?}"
        );
        // Another size is still asked once.
        assert_eq!(
            asker.decide(
                WIDE_16_10,
                || support_in(dir.path(), WIDE_16_10),
                || Asks::load(dir.path())
            ),
            Some(Decision::Ask(WIDE_16_10)),
            "{answer:?}"
        );
    }
}

#[test]
fn answers_add_up_and_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    record_answer(dir.path(), UW, Answer::Asked).unwrap();
    record_answer(dir.path(), UW, Answer::Never).unwrap();
    record_answer(dir.path(), WIDE_16_10, Answer::Dismissed).unwrap();
    let asks = Asks::load(dir.path()).unwrap();
    assert!(asks.was_asked(UW) && asks.declined(UW));
    assert!(asks.declined(WIDE_16_10));
    assert!(!asks.was_asked(P1080));
    let mut left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [std::ffi::OsString::from(ASKS_FILE)],
        "no temp file left"
    );
}

#[tokio::test]
async fn a_no_reads_no_consent_and_collects_nothing() {
    for answer in [Answer::Dismissed, Answer::Never] {
        let dir = tempfile::tempdir().unwrap();
        record_answer(dir.path(), UW, answer).unwrap();
        let calls = Cell::new(0);
        let fetch = || {
            calls.set(calls.get() + 1);
            std::future::ready(Some(true))
        };
        assert!(!may_collect(dir.path(), UW, fetch).await, "{answer:?}");
        assert_eq!(calls.get(), 0, "{answer:?}: no consent request");
    }
}

#[tokio::test]
async fn only_hero_icon_consent_on_is_a_yes() {
    let dir = tempfile::tempdir().unwrap();
    record_answer(dir.path(), UW, Answer::Asked).unwrap();
    assert!(may_collect(dir.path(), UW, || std::future::ready(Some(true))).await);
    assert!(!may_collect(dir.path(), UW, || std::future::ready(Some(false))).await);
    assert!(!may_collect(dir.path(), UW, || std::future::ready(None)).await);
    // An answers file that does not read is a no, with no request.
    std::fs::write(asks_path(dir.path()), b"{").unwrap();
    let calls = Cell::new(0);
    let fetch = || {
        calls.set(calls.get() + 1);
        std::future::ready(Some(true))
    };
    assert!(!may_collect(dir.path(), UW, fetch).await);
    assert_eq!(calls.get(), 0);
}
