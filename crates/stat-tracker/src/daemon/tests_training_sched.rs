//! Training crop sending in the daemon: it runs beside stats sync, never
//! in its way, and a failure leaves a warning for the desktop app.

use crate::{main_tests::*, *};
use scuffed_types::api::training_crops::TrainingCropConsent;

/// A loopback site that answers each connection on its own thread, by the
/// request line: training crop routes get `training(line)`, stats upload
/// gets 200. Returns the base URL; the server stops after `connections`.
fn site(
    connections: usize,
    training: fn(&str) -> (std::time::Duration, String),
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let server = std::thread::spawn(move || {
        let mut threads = Vec::new();
        for _ in 0..connections {
            let Ok((mut sock, _)) = listener.accept() else {
                break;
            };
            threads.push(std::thread::spawn(move || {
                let head = read_request(&mut sock);
                let line = head.lines().next().unwrap_or_default().to_string();
                let (delay, response) = if line.contains("/api/training-crops") {
                    training(&line)
                } else {
                    let body = serde_json::to_string(&upload_ok()).unwrap();
                    (std::time::Duration::ZERO, http(200, &body))
                };
                std::thread::sleep(delay);
                let _ = std::io::Write::write_all(&mut sock, response.as_bytes());
                line
            }));
        }
        threads.into_iter().filter_map(|t| t.join().ok()).collect()
    });
    (url, server)
}

fn read_request(sock: &mut std::net::TcpStream) -> String {
    sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let mut got = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match std::io::Read::read(sock, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
        let text = String::from_utf8_lossy(&got).to_string();
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if got.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&got).to_string()
}

fn http(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(width, height));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// Cache map_name consent and queue one map_name crop.
fn queue_map_crop(dir: &std::path::Path) {
    let consent = TrainingCropConsent {
        map_name: true,
        ..TrainingCropConsent::default()
    };
    let path = dir.join("training_crops").join("consent.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&consent).unwrap()).unwrap();
    let token = sync::game_token_for(dir, "s1").unwrap();
    let meta = sync::map_name_meta(&token, 1920, 1080);
    assert!(sync::keep_training_crop(
        dir,
        sync::UploadKind::MapName,
        meta,
        png(400, 40)
    ));
}

/// The consent read works, then the upload is slow and fails with a 500.
fn slow_failing_upload(line: &str) -> (std::time::Duration, String) {
    if line.starts_with("GET /api/training-crops/consent") {
        let body = r#"{"hero_icon":false,"stat_digits":false,"map_name":true}"#;
        return (std::time::Duration::ZERO, http(200, body));
    }
    (
        std::time::Duration::from_secs(3),
        http(500, r#"{"error":"internal"}"#),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_training_upload_does_not_block_stats_sync_and_sets_the_warning() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("s1", "victory"))
        .await
        .unwrap();
    queue_map_crop(dir.path());
    // consent GET, training POST, stats upload.
    let (url, server) = site(3, slow_failing_upload);
    let client = sync::SyncClient::new(config::SyncConfig {
        server_url: url.clone(),
        token: "tok".into(),
    });
    let backoff = std::sync::Mutex::new(sync::SyncBackoff::default());

    let mut training = TrainingSched::default();
    let started = std::time::Instant::now();
    training.tick(
        Some(&client),
        stats_sync_paused(&backoff),
        dir.path(),
        Instant::now(),
    );
    assert!(
        started.elapsed() < std::time::Duration::from_millis(500),
        "the tick must not wait on the site"
    );
    let task = training.take_task().expect("a pass started");

    // Stats sync runs and finishes while the training upload still hangs.
    let attempt = try_sync(&store, &client, dir.path(), None).await;
    assert_eq!(attempt, sync::SyncAttempt::Uploaded);
    assert!(store.get_unsynced().await.unwrap().is_empty());
    assert!(
        !task.is_finished(),
        "stats sync finished first, beside the training pass"
    );
    apply_sync_backoff(&backoff, attempt);

    task.await.unwrap();
    let warning = sync::training_warning(dir.path()).expect("a visible warning");
    assert!(warning.contains("Training crops"), "{warning}");
    assert!(warning.contains("500"), "{warning}");
    // The crop waits for a later pass, and the training clock backs off.
    assert_eq!(
        sync::queued_crops(dir.path(), sync::UploadKind::MapName).len(),
        1
    );
    let left = training.clock.lock().unwrap().wait_left(Instant::now());
    assert!(left > TRAINING_PASS_EVERY, "backing off: {left:?}");
    // Stats sync state is untouched by the training failure.
    let clock = backoff.lock().unwrap();
    assert_eq!(clock.failures(), 0);
    assert!(!clock.auth_rejected());
    drop(clock);
    assert!(sync::auth_pause_status(dir.path(), &url, "tok").is_none());
    let lines = server.join().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("POST /api/stats/upload"))
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("POST /api/training-crops?kind=map_name"))
    );
}

fn token_refused(_: &str) -> (std::time::Duration, String) {
    (
        std::time::Duration::ZERO,
        http(401, r#"{"error":"Unauthorized"}"#),
    )
}

#[tokio::test]
async fn a_refused_token_on_training_routes_never_pauses_stats_sync() {
    let dir = tempfile::tempdir().unwrap();
    queue_map_crop(dir.path());
    let (url, server) = site(1, token_refused);
    let client = sync::SyncClient::new(config::SyncConfig {
        server_url: url.clone(),
        token: "tok".into(),
    });
    let mut training = TrainingSched::default();
    training.tick(Some(&client), false, dir.path(), Instant::now());
    training.take_task().unwrap().await.unwrap();
    server.join().unwrap();
    assert!(sync::training_warning(dir.path()).is_some());
    assert!(
        sync::auth_pause_status(dir.path(), &url, "tok").is_none(),
        "only stats sync decides that the token is dead"
    );
    let left = training.clock.lock().unwrap().wait_left(Instant::now());
    assert!(left > TRAINING_BACKOFF_CAP - std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn no_pass_while_stats_sync_is_paused_and_sync_off_forgets_the_crops() {
    let dir = tempfile::tempdir().unwrap();
    queue_map_crop(dir.path());
    let client = sync::SyncClient::new(config::SyncConfig {
        server_url: "http://127.0.0.1:9".into(),
        token: "tok".into(),
    });
    let mut training = TrainingSched::default();
    training.tick(Some(&client), true, dir.path(), Instant::now());
    assert!(training.take_task().is_none(), "paused: no pass");
    assert!(sync::has_queued_crops(dir.path()));

    let status = dir.path().join("training_crops").join("status.json");
    std::fs::write(&status, r#"{"warning":"old"}"#).unwrap();
    training.tick(None, false, dir.path(), Instant::now());
    assert!(training.take_task().is_none());
    assert!(
        !sync::has_queued_crops(dir.path()),
        "sync off: nothing kept"
    );
    assert_eq!(sync::training_warning(dir.path()), None);
    assert_eq!(
        sync::cached_training_consent(dir.path()),
        TrainingCropConsent::default()
    );
}

#[test]
fn the_clock_waits_and_reads_consent_again_now_and_then() {
    let now = Instant::now();
    let mut clock = TrainingClock::default();
    assert!(clock.due(now, false), "the first look reads the consent");
    let ok = sync::TrainingPassReport {
        sent: 0,
        consent_read: true,
        retry: sync::TrainingRetry::Normal,
        warning: None,
    };
    clock.observe(&ok, now);
    assert!(
        !clock.due(now + TRAINING_PASS_EVERY, false),
        "nothing waits"
    );
    assert!(clock.due(now + TRAINING_PASS_EVERY, true));
    assert!(clock.due(now + TRAINING_CONSENT_EVERY, false));

    let limited = sync::TrainingPassReport {
        retry: sync::TrainingRetry::After(std::time::Duration::from_secs(600)),
        consent_read: false,
        ..ok.clone()
    };
    clock.observe(&limited, now);
    assert!(!clock.due(now + std::time::Duration::from_secs(599), true));
    assert!(clock.due(now + std::time::Duration::from_secs(600), true));

    let down = sync::TrainingPassReport {
        retry: sync::TrainingRetry::Backoff,
        ..limited.clone()
    };
    let mut waits = Vec::new();
    for _ in 0..12 {
        clock.observe(&down, now);
        waits.push(clock.wait_left(now));
    }
    assert_eq!(waits[0], TRAINING_BACKOFF_BASE);
    assert_eq!(waits[1], TRAINING_BACKOFF_BASE * 2);
    assert!(waits.windows(2).all(|w| w[0] <= w[1]), "never shrinks");
    assert_eq!(*waits.last().unwrap(), TRAINING_BACKOFF_CAP);
}

#[tokio::test]
async fn an_accepted_tab_offers_its_map_crop_only_with_consent() {
    let dir = tempfile::tempdir().unwrap();
    // No consent read yet: the debug map crop is saved, nothing is queued.
    archive_map_label_crop(dir.path(), "sess-a", &labelled_frame())
        .expect("saves the crop")
        .await
        .unwrap();
    assert!(!sync::has_queued_crops(dir.path()));
    assert!(!dir.path().join("training_crops").join("outbox").exists());

    // map_name consent on: the next game's first crop is queued.
    let consent = TrainingCropConsent {
        map_name: true,
        ..TrainingCropConsent::default()
    };
    let path = dir.path().join("training_crops").join("consent.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&consent).unwrap()).unwrap();
    archive_map_label_crop(dir.path(), "sess-b", &labelled_frame())
        .expect("saves the crop")
        .await
        .unwrap();
    let queued = sync::queued_crops(dir.path(), sync::UploadKind::MapName);
    assert_eq!(queued.len(), 1);
    let meta = &queued[0].meta;
    assert!(sync::game_token_ok(meta.game_token.as_deref().unwrap()));
    assert_eq!(
        (meta.screen_width, meta.screen_height),
        (Some(1920), Some(1080))
    );
    assert!(meta.found_spot.is_none() && meta.found_confidence.is_none());
    assert!(meta.hero.is_none());

    // A rejected capture never feeds the outbox, even with debug and consent on.
    archive_rejected_map_label_crop(true, dir.path(), "sess-r", &labelled_frame(), "unreadable")
        .expect("debug on saves the crop")
        .await
        .unwrap();
    assert_eq!(
        sync::queued_crops(dir.path(), sync::UploadKind::MapName).len(),
        1
    );
}
