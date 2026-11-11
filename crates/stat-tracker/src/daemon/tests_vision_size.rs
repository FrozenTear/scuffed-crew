//! Tests for the capture size notice in the daemon: the debug-build fake
//! size through `after_capture`, a game recorded at an undeclared or
//! uncheckable size (by hand and through the real capture task), and no
//! consent or crop request to a loopback server after a no.
//!
//! The tests that note a frame hold `INJECTED_CAPTURE_SERIAL`: the noted
//! size is process-wide, and so is the injected capture.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::vision_size::{
    FAKE_SIZE_ENV, after_capture_with, check_size, checked_size, fake_size_from, note_frame,
};
use crate::main_tests::{
    NIGHT_CLEAN, Night, game, night_counters, o8_ctx, r9_counters, r9_frame, session,
};
use crate::{INJECTED_CAPTURE, INJECTED_CAPTURE_SERIAL, start_tab_capture};
use stat_tracker::ocr_status::{VisionSizeStatus, publish_vision_size, read_status};
use stat_tracker::packs::layout::vision_sizes::{
    Answer, Asker, ScreenSize, may_collect_hero_icons, record_answer,
};
use stat_tracker::{boundary, capture, detect, sync};

const UW: ScreenSize = ScreenSize::new(3440, 1440);
const P1440: ScreenSize = ScreenSize::new(2560, 1440);

/// The pack record sync writes, with the packs shipped today.
fn shipped_packs(data_dir: &Path) {
    let templates = data_dir.join("templates");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(
        templates.join("pack-versions.json"),
        br#"{"heroes-v1.tar":"1","heroes-v2.tar":"2","heroes-v2-1080.tar":"1","maps-v1.tar":"1"}"#,
    )
    .unwrap();
}

fn published(data_dir: &Path) -> Option<VisionSizeStatus> {
    read_status(data_dir).and_then(|s| s.vision_size)
}

fn ask(size: ScreenSize, unreadable: bool) -> Option<VisionSizeStatus> {
    Some(VisionSizeStatus {
        width: size.width,
        height: size.height,
        unreadable,
    })
}

#[test]
fn the_fake_size_parses_and_wins_over_the_frame() {
    assert_eq!(FAKE_SIZE_ENV, "SCUFFED_DEV_FAKE_RESOLUTION");
    assert_eq!(fake_size_from(Some("3440x1440")), Some(UW));
    assert_eq!(fake_size_from(Some("big")), None);
    assert_eq!(fake_size_from(None), None);
    assert_eq!(checked_size(P1440, None), P1440);
    assert_eq!(checked_size(P1440, Some(UW)), UW);
}

/// The fake size through the same `after_capture` the capture task runs,
/// with a temp data dir (never the member's).
#[tokio::test]
async fn the_fake_size_asks_through_after_capture() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    shipped_packs(dir.path());
    // A real 1440p frame and no fake: a declared size, no notice.
    note_frame(P1440.width, P1440.height);
    after_capture_with(dir.path(), None);
    assert_eq!(published(dir.path()), None);
    // No frame noted since: nothing is decided.
    after_capture_with(dir.path(), Some(UW));
    assert_eq!(published(dir.path()), None);
    // The same frame with the fake size: the ask, for the fake size.
    note_frame(P1440.width, P1440.height);
    after_capture_with(dir.path(), fake_size_from(Some("3440x1440")));
    assert_eq!(published(dir.path()), ask(UW, false));
    // Nothing but the status file was written: no answer, no config.
    let mut left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["ocr_switch.json", "templates"]);
    publish_vision_size(dir.path(), None);
}

#[tokio::test]
async fn an_unsupported_size_still_records_the_game() {
    let mut night = Night::new().await;
    shipped_packs(night.dir.path());
    let mut asker = Asker::default();
    night.begin_on("Busan", "Zenyatta");
    for elims in [10, 12] {
        night
            .tab_once(
                night_counters(elims, 5, 2, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        // The order the capture task runs them in: capture, then the check.
        check_size(&mut asker, night.dir.path(), UW);
        night.advance(Duration::from_secs(60));
    }
    let elims = night.elims(&night.id()).await;
    assert!(elims.contains(&12), "the game is recorded, got {elims:?}");
    assert_eq!(published(night.dir.path()), ask(UW, false));
    publish_vision_size(night.dir.path(), None);
}

#[tokio::test]
async fn an_unreadable_signal_still_records_the_game_and_warns() {
    let mut night = Night::new().await;
    let templates = night.dir.path().join("templates");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(templates.join("pack-versions.json"), b"{not json").unwrap();
    let mut asker = Asker::default();
    // A warning from an earlier Tab is up while this game is played.
    check_size(&mut asker, night.dir.path(), UW);
    assert_eq!(
        published(night.dir.path()),
        ask(UW, true),
        "visible warning"
    );
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(7, 3, 1, 1800, 0, 900),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    check_size(&mut asker, night.dir.path(), UW);
    let elims = night.elims(&night.id()).await;
    assert!(elims.contains(&7), "the game is recorded, got {elims:?}");
    assert_eq!(published(night.dir.path()), ask(UW, true));
    // A warning is no yes: nothing may be collected at that size, and no
    // consent is read, even when it would say yes.
    let calls = std::cell::Cell::new(0);
    let fetch = || {
        calls.set(calls.get() + 1);
        std::future::ready(Some(true))
    };
    assert!(!may_collect_hero_icons(night.dir.path(), UW, fetch).await);
    assert_eq!(calls.get(), 0);
    publish_vision_size(night.dir.path(), None);
}

/// The real capture task (`start_tab_capture`, through `handle_capture`)
/// with a 3440x1440 frame noted the way the grab notes it: the Tab's
/// report reaches `capture_tx`, and the ask is published after it.
#[tokio::test]
async fn the_capture_task_sends_the_tab_at_an_unsupported_size() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    shipped_packs(dir.path());
    let ctx = Arc::new(o8_ctx(dir.path(), true).await);
    // Now, not `test_now`: the task waits until the press plus the draw
    // wait before it captures.
    let t0 = Instant::now();
    let mut active = game(detect::MatchOutcome::Unknown, None, t0);
    active.session_id = "uw".into();
    active.map = Some("Busan".into());
    active.map_source = Some(boundary::MapSource::TopBar);
    active.session_created = false;
    active.opened_at = t0;
    let mut st = session(Some(active), t0);
    let mut sync_task = None;
    let backoff = Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    // What the grab notes for this frame, and the board the frame reads.
    note_frame(UW.width, UW.height);
    *INJECTED_CAPTURE.lock().unwrap() = Some(r9_frame(r9_counters(9, 4, 3000), "Busan"));
    let (task, _flight) = start_tab_capture(
        &mut st,
        &ctx,
        &mut sync_task,
        &backoff,
        &tx,
        t0,
        capture::stream_key_hold(t0),
    )
    .await;
    let (sid, result) = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("the Tab report in time")
        .expect("the channel is open");
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the task ends")
        .expect("the task did not panic");
    assert_eq!(sid, "uw");
    let report = result.expect("the capture");
    assert!(report.recorded, "the game is recorded");
    assert_eq!(published(dir.path()), ask(UW, false));
    publish_vision_size(dir.path(), None);
}

fn loopback_client(listener: &std::net::TcpListener) -> stat_tracker::sync::SyncClient {
    let port = listener.local_addr().unwrap().port();
    stat_tracker::sync::SyncClient::try_new(stat_tracker::config::SyncConfig {
        server_url: format!("http://127.0.0.1:{port}"),
        token: "tok-vision-size".into(),
    })
    .unwrap()
}

/// The longest any loopback test here waits for a connection or the gate.
const DEADLINE: Duration = Duration::from_secs(5);

#[tokio::test]
async fn a_no_sends_no_consent_or_crop_request_to_the_server() {
    for answer in [Answer::Dismissed, Answer::Never] {
        let dir = tempfile::tempdir().unwrap();
        shipped_packs(dir.path());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let client = loopback_client(&listener);
        record_answer(dir.path(), UW, answer).unwrap();
        let allowed = tokio::time::timeout(
            DEADLINE,
            may_collect_hero_icons(dir.path(), UW, || client.fetch_hero_icon_consent()),
        )
        .await
        .expect("the gate answers without the network");
        assert!(!allowed, "{answer:?}");
        match listener.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("{answer:?}: the server got a connection: {other:?}"),
        }
    }
}

/// Accept one connection before `DEADLINE`, answer `body` with a 200, and
/// return the request head. `None` when nothing connected in time.
fn serve_once(listener: std::net::TcpListener, body: String) -> Option<String> {
    listener.set_nonblocking(true).unwrap();
    let until = Instant::now() + DEADLINE;
    let mut sock = loop {
        match listener.accept() {
            Ok((sock, _)) => break sock,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    };
    sock.set_nonblocking(false).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    while !got.windows(4).any(|w| w == b"\r\n\r\n") {
        match std::io::Read::read(&mut sock, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    std::io::Write::write_all(&mut sock, reply.as_bytes()).unwrap();
    Some(String::from_utf8_lossy(&got).into_owned())
}

/// The control for the test above: with no no on file, at a size the
/// shipped packs do not declare, the gate asks the mock server once, on
/// the read-only consent route, and its answer counts.
#[tokio::test]
async fn without_a_no_the_gate_reads_consent_once() {
    for (consent, want) in [(true, true), (false, false)] {
        let dir = tempfile::tempdir().unwrap();
        shipped_packs(dir.path());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = loopback_client(&listener);
        let body = format!(r#"{{"hero_icon":{consent},"stat_digits":false,"map_name":false}}"#);
        let server = std::thread::spawn(move || serve_once(listener, body));
        let allowed = tokio::time::timeout(
            DEADLINE,
            may_collect_hero_icons(dir.path(), UW, || client.fetch_hero_icon_consent()),
        )
        .await
        .expect("the gate answers in time");
        let req = server
            .join()
            .unwrap()
            .expect("the gate connected")
            .to_ascii_lowercase();
        assert_eq!(allowed, want);
        assert!(req.starts_with("get /api/training-crops/consent "), "{req}");
        assert!(
            req.contains("authorization: bearer tok-vision-size"),
            "{req}"
        );
    }
}
