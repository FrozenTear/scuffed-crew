//! Tests for the capture size notice in the daemon: the dev-only fake
//! size, a game recorded at an unsupported or uncheckable size, and no
//! consent or crop request to a loopback server after a no.

use std::path::Path;

use super::vision_size::{FAKE_SIZE_ENV, check_size, checked_size, fake_size_from};
use crate::main_tests::{NIGHT_CLEAN, Night, night_counters};
use stat_tracker::ocr_status::{VisionSizeStatus, publish_vision_size, read_status};
use stat_tracker::packs::layout::vision_sizes::{
    Answer, Asker, ScreenSize, may_collect, record_answer,
};

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
fn the_dev_fake_size_triggers_the_notice_and_leaves_the_frame_alone() {
    assert_eq!(FAKE_SIZE_ENV, "SCUFFED_DEV_FAKE_RESOLUTION");
    assert_eq!(fake_size_from(Some("3440x1440")), Some(UW));
    assert_eq!(fake_size_from(Some("big")), None);
    assert_eq!(fake_size_from(None), None);

    let dir = tempfile::tempdir().unwrap();
    shipped_packs(dir.path());
    // A real 1440p frame: supported, no notice.
    let frame = image::DynamicImage::new_rgb8(P1440.width, P1440.height);
    let before = frame.as_bytes().to_vec();
    let real = ScreenSize::new(frame.width(), frame.height());
    check_size(&mut Asker::default(), dir.path(), checked_size(real, None));
    assert_eq!(published(dir.path()), None);
    // The fake size asks, and the frame is the same frame.
    check_size(
        &mut Asker::default(),
        dir.path(),
        checked_size(real, fake_size_from(Some("3440x1440"))),
    );
    assert_eq!(published(dir.path()), ask(UW, false));
    assert_eq!((frame.width(), frame.height()), (2560, 1440));
    assert_eq!(frame.as_bytes(), &before[..]);
    // Nothing went to the config dir: only the data dir was written.
    assert!(!dir.path().join("config.toml").exists());
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
        night.advance(std::time::Duration::from_secs(60));
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
    // A warning is no yes: nothing may be collected at that size.
    assert!(!may_collect(night.dir.path(), UW, || std::future::ready(None)).await);
    publish_vision_size(night.dir.path(), None);
}

fn loopback_client(listener: &std::net::TcpListener) -> stat_tracker::sync::SyncClient {
    let port = listener.local_addr().unwrap().port();
    stat_tracker::sync::SyncClient::try_new(stat_tracker::config::SyncConfig {
        server_url: format!("http://127.0.0.1:{port}"),
        token: "tok-vision-size".into(),
    })
    .unwrap()
}

#[tokio::test]
async fn a_no_sends_no_consent_or_crop_request_to_the_server() {
    for answer in [Answer::Dismissed, Answer::Never] {
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let client = loopback_client(&listener);
        record_answer(dir.path(), UW, answer).unwrap();
        assert!(
            !may_collect(dir.path(), UW, || client.fetch_hero_icon_consent()).await,
            "{answer:?}"
        );
        match listener.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("{answer:?}: the server got a connection: {other:?}"),
        }
    }
}

/// The control for the test above: with no no on file, the gate asks the
/// mock server once, on the read-only consent route, and its answer counts.
#[tokio::test]
async fn without_a_no_the_gate_reads_consent_once() {
    for (consent, want) in [(true, true), (false, false)] {
        let dir = tempfile::tempdir().unwrap();
        record_answer(dir.path(), UW, Answer::Asked).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = loopback_client(&listener);
        let body = format!(r#"{{"hero_icon":{consent},"stat_digits":false,"map_name":false}}"#);
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
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
            String::from_utf8_lossy(&got).into_owned()
        });
        let allowed = may_collect(dir.path(), UW, || client.fetch_hero_icon_consent()).await;
        let req = server.join().unwrap().to_ascii_lowercase();
        assert_eq!(allowed, want);
        assert!(req.starts_with("get /api/training-crops/consent "), "{req}");
        assert!(
            req.contains("authorization: bearer tok-vision-size"),
            "{req}"
        );
    }
}
