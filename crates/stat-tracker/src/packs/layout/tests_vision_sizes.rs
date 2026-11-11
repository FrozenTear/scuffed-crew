//! Tests for `vision_sizes`: the size each pack name declares, the support
//! signal read from a fake pack record, the ask and its clearing, answers
//! that survive a restart, the answers file's safety, and the hero icon
//! collection gate.

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

/// A pack record that is there but does not read.
fn unreadable_record(data_dir: &Path) {
    let path = super::super::versions_path(data_dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{not json").unwrap();
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
    // A height tag: the 16:9 size that tall.
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
fn an_unknown_tag_declares_nothing() {
    for name in [
        "heroes-v2-beta.tar",
        "heroes-v2-foo.tar",
        "heroes-v2-12.tar",
        "heroes-v2-12345.tar",
        "heroes-foo.tar",
        "heroes-.tar",
    ] {
        assert_eq!(declared_size(name), None, "{name}");
    }
}

#[test]
fn a_height_tag_rounds_the_width_to_whole_pixels() {
    // 768 * 16 / 9 is 1365.33: the declared size is 1365x768, so a
    // 1366x768 panel is not an exact match.
    assert_eq!(
        declared_size("heroes-v3-768.tar"),
        Some(ScreenSize::new(1365, 768))
    );
    assert_eq!(
        declared_size("heroes-v3-2160.tar"),
        Some(ScreenSize::new(3840, 2160))
    );
}

#[test]
fn the_shipped_packs_declare_1440p_and_1080p_only() {
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
    // The same run: the next Tab takes the ask down.
    assert_eq!(
        before.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        Some(Decision::Clear(UW))
    );
    assert_eq!(
        before.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        None
    );
    // A new run: quiet from the start.
    let mut after = Asker::default();
    assert_eq!(
        after.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        Some(Decision::Quiet)
    );
}

#[test]
fn a_no_clicked_while_the_ask_is_up_takes_it_down() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    let mut asker = Asker::default();
    let mut decide = || asker.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path()));
    assert_eq!(decide(), Some(Decision::Ask(UW)));
    assert_eq!(decide(), None, "still up, still due");
    record_answer(dir.path(), UW, Answer::Dismissed).unwrap();
    assert_eq!(decide(), Some(Decision::Clear(UW)));
    assert_eq!(decide(), None);
}

#[test]
fn a_reset_answers_file_turns_the_warning_into_the_ask_or_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    std::fs::write(asks_path(dir.path()), b"[").unwrap();
    let mut asker = Asker::default();
    let mut decide = |size: ScreenSize| {
        asker.decide(
            size,
            || support_in(dir.path(), size),
            || Asks::load(dir.path()),
        )
    };
    assert!(matches!(decide(UW), Some(Decision::Warn(size, _)) if size == UW));
    assert!(matches!(decide(WIDE_16_10), Some(Decision::Warn(..))));
    // Reset from the WIDE_16_10 warning: a no for that size only.
    reset_unreadable(dir.path(), WIDE_16_10).unwrap();
    assert_eq!(decide(UW), Some(Decision::Ask(UW)));
    assert_eq!(decide(WIDE_16_10), Some(Decision::Clear(WIDE_16_10)));
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
    unreadable_record(dir.path());
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
        // Settled quiet: the record is not read again in this run.
        let support_read = Cell::new(false);
        let again = asker.decide(
            supported,
            || {
                support_read.set(true);
                Support::Unsupported
            },
            || Asks::load(dir.path()),
        );
        assert_eq!(again, None);
        assert!(!support_read.get());
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
fn an_unanswered_ask_comes_back_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    let mut first = Asker::default();
    assert_eq!(
        first.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        Some(Decision::Ask(UW))
    );
    // Nothing was clicked, so nothing was written.
    assert!(!asks_path(dir.path()).exists());
    let mut restarted = Asker::default();
    assert_eq!(
        restarted.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        Some(Decision::Ask(UW))
    );
}

#[test]
fn no_thanks_is_for_one_size_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    record_answer(dir.path(), UW, Answer::Dismissed).unwrap();
    // A new run: a new asker, the answers read from disk again.
    let mut asker = Asker::default();
    assert_eq!(
        asker.decide(UW, || support_in(dir.path(), UW), || Asks::load(dir.path())),
        Some(Decision::Quiet)
    );
    // Another size is still asked.
    assert_eq!(
        asker.decide(
            WIDE_16_10,
            || support_in(dir.path(), WIDE_16_10),
            || Asks::load(dir.path())
        ),
        Some(Decision::Ask(WIDE_16_10))
    );
}

#[test]
fn dont_ask_again_covers_every_size_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    record_answer(dir.path(), UW, Answer::Never).unwrap();
    let asks = Asks::load(dir.path()).unwrap();
    assert!(asks.never_again);
    assert!(asks.declined(UW) && asks.declined(WIDE_16_10));
    // A new run: neither size is asked.
    let mut asker = Asker::default();
    for size in [UW, WIDE_16_10] {
        assert_eq!(
            asker.decide(
                size,
                || support_in(dir.path(), size),
                || Asks::load(dir.path())
            ),
            Some(Decision::Quiet),
            "{size}"
        );
    }
}

#[test]
fn answers_add_up_and_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    record_answer(dir.path(), WIDE_16_10, Answer::Dismissed).unwrap();
    let asks = Asks::load(dir.path()).unwrap();
    assert!(asks.declined(WIDE_16_10));
    assert!(!asks.declined(UW), "No thanks is for one size");
    record_answer(dir.path(), UW, Answer::Never).unwrap();
    let asks = Asks::load(dir.path()).unwrap();
    assert!(asks.declined(UW) && asks.declined(WIDE_16_10) && asks.declined(P1080));
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

#[test]
fn unknown_fields_survive_a_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        asks_path(dir.path()),
        br#"{"asked":["3440x1440"],"later_field":{"a":1},"dismissed":["2560x1600"]}"#,
    )
    .unwrap();
    record_answer(dir.path(), UW, Answer::Dismissed).unwrap();
    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(asks_path(dir.path())).unwrap()).unwrap();
    assert_eq!(raw["asked"], serde_json::json!(["3440x1440"]));
    assert_eq!(raw["later_field"], serde_json::json!({"a": 1}));
    assert_eq!(
        raw["dismissed"],
        serde_json::json!(["2560x1600", "3440x1440"])
    );
    // An old `asked` entry is no no: only the clicked sizes are declined.
    let asks = Asks::load(dir.path()).unwrap();
    assert!(asks.declined(UW) && !asks.never_again);
}

/// Blocking 3: an answers file that does not read is never written over.
/// Put back `Asks::load(..).unwrap_or_default()` and this fails: the call
/// returns Ok and the bytes become a fresh file.
#[test]
fn record_answer_refuses_an_unreadable_file_and_leaves_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let corrupt: &[u8] = b"{\"never\":[\"2560x1600\"], oops";
    std::fs::write(asks_path(dir.path()), corrupt).unwrap();
    for answer in [Answer::Dismissed, Answer::Never] {
        assert!(record_answer(dir.path(), UW, answer).is_err(), "{answer:?}");
        assert_eq!(std::fs::read(asks_path(dir.path())).unwrap(), corrupt);
    }
    assert!(!temp_path(dir.path()).exists());
}

#[test]
fn a_failed_save_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    // The destination is a directory: the temp file is written, the rename
    // fails.
    std::fs::create_dir(asks_path(dir.path())).unwrap();
    let mut asks = Asks::default();
    asks.mark(UW, Answer::Dismissed);
    assert!(asks.save(dir.path()).is_err());
    assert!(!temp_path(dir.path()).exists(), "temp file removed");
    assert!(asks_path(dir.path()).is_dir());
}

#[test]
fn a_reset_keeps_the_old_bytes_and_writes_a_no() {
    let dir = tempfile::tempdir().unwrap();
    let corrupt: &[u8] = b"[[[";
    std::fs::write(asks_path(dir.path()), corrupt).unwrap();
    let backup = reset_unreadable(dir.path(), UW).unwrap().expect("a copy");
    assert_eq!(std::fs::read(&backup).unwrap(), corrupt);
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("vision-size-asks.json.unreadable-")
    );
    let asks = Asks::load(dir.path()).unwrap();
    assert!(asks.declined(UW));
    assert!(!asks.declined(WIDE_16_10) && !asks.never_again);
    // A file that reads is not replaced: the no is added to it.
    assert_eq!(reset_unreadable(dir.path(), WIDE_16_10).unwrap(), None);
    let asks = Asks::load(dir.path()).unwrap();
    assert!(asks.declined(UW) && asks.declined(WIDE_16_10));
}

#[test]
fn a_failed_reset_leaves_the_unreadable_bytes_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let corrupt: &[u8] = b"[[[";
    std::fs::write(asks_path(dir.path()), corrupt).unwrap();
    // The temp file cannot be made: the new file is never written.
    std::fs::create_dir(temp_path(dir.path())).unwrap();
    assert!(reset_unreadable(dir.path(), UW).is_err());
    assert_eq!(std::fs::read(asks_path(dir.path())).unwrap(), corrupt);
    assert!(Asks::load(dir.path()).is_err());
}

/// A fetch that counts its calls and would say yes.
fn counting_yes(calls: &Cell<u32>) -> impl FnOnce() -> std::future::Ready<Option<bool>> + '_ {
    move || {
        calls.set(calls.get() + 1);
        std::future::ready(Some(true))
    }
}

/// Blocking 1: an unreadable pack record is no yes, and no request is
/// made, even with consent on and no no on file. Drop the support check
/// from the gate and this fails: it fetches once and returns true.
#[tokio::test]
async fn an_unreadable_pack_record_collects_nothing_and_fetches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    unreadable_record(dir.path());
    assert!(!asks_path(dir.path()).exists(), "no no on file");
    let calls = Cell::new(0);
    assert!(!may_collect_hero_icons(dir.path(), UW, counting_yes(&calls)).await);
    assert_eq!(calls.get(), 0, "no consent request");
}

#[tokio::test]
async fn a_size_that_is_not_known_or_declared_collects_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Cell::new(0);
    // No pack record yet.
    assert!(!may_collect_hero_icons(dir.path(), UW, counting_yes(&calls)).await);
    // A declared size is not this gate's to open.
    shipped(dir.path());
    assert!(!may_collect_hero_icons(dir.path(), BASE_SIZE, counting_yes(&calls)).await);
    assert_eq!(calls.get(), 0);
}

#[tokio::test]
async fn a_no_reads_no_consent_and_collects_nothing() {
    for (answered, asked) in [
        (Answer::Dismissed, UW),
        (Answer::Never, UW),
        // "Don't ask again" for one size is a no for another.
        (Answer::Never, WIDE_16_10),
    ] {
        let dir = tempfile::tempdir().unwrap();
        shipped(dir.path());
        record_answer(dir.path(), UW, answered).unwrap();
        let calls = Cell::new(0);
        assert!(
            !may_collect_hero_icons(dir.path(), asked, counting_yes(&calls)).await,
            "{answered:?} {asked}"
        );
        assert_eq!(calls.get(), 0, "{answered:?} {asked}: no consent request");
    }
}

#[tokio::test]
async fn only_hero_icon_consent_on_is_a_yes() {
    let dir = tempfile::tempdir().unwrap();
    shipped(dir.path());
    let calls = Cell::new(0);
    assert!(may_collect_hero_icons(dir.path(), UW, counting_yes(&calls)).await);
    assert_eq!(calls.get(), 1);
    assert!(!may_collect_hero_icons(dir.path(), UW, || std::future::ready(Some(false))).await);
    assert!(!may_collect_hero_icons(dir.path(), UW, || std::future::ready(None)).await);
    // "No thanks" for another size is not a no for this one.
    record_answer(dir.path(), WIDE_16_10, Answer::Dismissed).unwrap();
    assert!(may_collect_hero_icons(dir.path(), UW, counting_yes(&calls)).await);
    // An answers file that does not read is a no, with no request.
    std::fs::write(asks_path(dir.path()), b"{").unwrap();
    let calls = Cell::new(0);
    assert!(!may_collect_hero_icons(dir.path(), UW, counting_yes(&calls)).await);
    assert_eq!(calls.get(), 0);
}
