//! Training crops against a loopback server: the requests the tracker
//! sends, and what a pass does with each answer.

use scuffed_types::api::training_crops::{TrainingCropConsent, TrainingCropUploadMeta};

use super::tests::client;
use super::tests_training::{all_consent, consent, part_names, png, token};
use super::*;

/// One request as the server saw it.
struct Seen {
    head: String,
    body: Vec<u8>,
}

impl Seen {
    fn line(&self) -> &str {
        self.head.lines().next().unwrap_or_default()
    }

    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    }
}

fn read_one(sock: &mut std::net::TcpStream) -> Seen {
    sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let mut got = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        match std::io::Read::read(sock, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
        if let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&got[..end]).to_string();
            let length = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if got.len() >= end + 4 + length {
                return Seen {
                    head,
                    body: got[end + 4..end + 4 + length].to_vec(),
                };
            }
        }
    }
    Seen {
        head: String::from_utf8_lossy(&got).to_string(),
        body: Vec::new(),
    }
}

/// Answer one connection per response, in order. The URL and the thread
/// that returns what was sent.
fn serve(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<Seen>>) {
    serve_on(
        std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
        responses,
    )
}

/// [`serve`] on `listener`, so two rounds can share one site address.
fn serve_on(
    listener: std::net::TcpListener,
    responses: Vec<String>,
) -> (String, std::thread::JoinHandle<Vec<Seen>>) {
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let thread = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for response in responses {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let mut sock = loop {
                match listener.accept() {
                    Ok((sock, _)) => break Some(sock),
                    Err(_) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break None,
                }
            };
            let Some(sock) = sock.as_mut() else {
                break;
            };
            sock.set_nonblocking(false).ok();
            seen.push(read_one(sock));
            let _ = std::io::Write::write_all(sock, response.as_bytes());
        }
        seen
    });
    (url, thread)
}

fn http(status: u16, extra: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn consent_body(c: TrainingCropConsent) -> String {
    serde_json::to_string(&c).unwrap()
}

fn stored(kind: &str, n: usize) -> String {
    let crop = serde_json::json!({
        "id": "3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c", "kind": kind, "size_bytes": 1,
        "width": 8, "height": 8, "uploaded_at": "2026-11-10T18:04:05Z",
        "meta": {}, "thumbnail_url": "/x"
    });
    serde_json::json!({ "kind": kind, "crops": vec![crop; n] }).to_string()
}

fn queue(dir: &std::path::Path, kind: UploadKind, n: usize) {
    for _ in 0..n {
        let meta = match kind {
            UploadKind::HeroIcon => TrainingCropUploadMeta {
                hero: hero_field("Ana"),
                game_token: Some(token()),
                ..TrainingCropUploadMeta::default()
            },
            _ => map_name_meta(&token(), 1920, 1080),
        };
        assert!(keep_training_crop(dir, kind, meta, png(64, 16, false)));
    }
}

fn on_disk(dir: &std::path::Path, kind: UploadKind) -> usize {
    queued_crops(dir, kind).len()
}

#[tokio::test]
async fn consent_read_sends_the_token_and_tracker_user_agent() {
    let (url, server) = serve(vec![http(
        200,
        "",
        r#"{"hero_icon":true,"stat_digits":false,"map_name":true}"#,
    )]);
    let got = client(&url).fetch_training_consent().await.unwrap();
    let seen = server.join().unwrap();
    assert!(got.hero_icon && got.map_name && !got.stat_digits);
    assert_eq!(seen[0].line(), "GET /api/training-crops/consent HTTP/1.1");
    assert_eq!(
        seen[0].header("authorization").as_deref(),
        Some("Bearer super-secret-token")
    );
    let agent = seen[0].header("user-agent").unwrap();
    assert!(
        agent.starts_with("scuffed-stat-tracker/"),
        "same User-Agent as stats sync: {agent}"
    );
    assert_eq!(agent, USER_AGENT);
}

#[tokio::test]
async fn a_pass_sends_only_consented_kinds_as_meta_and_crop_pairs() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 2);
    queue(dir.path(), UploadKind::StatDigits, 1);
    // The site now has stat_digits off.
    let now = TrainingCropConsent {
        map_name: true,
        ..TrainingCropConsent::default()
    };
    let (url, server) = serve(vec![
        http(200, "", &consent_body(now)),
        http(201, "", &stored("map_name", 2)),
    ]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 2, "consent read, then one map_name upload");
    assert_eq!(report.sent, 2);
    assert_eq!(report.retry, TrainingRetry::Normal);
    assert_eq!(report.warning, None);

    let post = &seen[1];
    assert_eq!(
        post.line(),
        "POST /api/training-crops?kind=map_name HTTP/1.1"
    );
    assert_eq!(
        post.header("authorization").as_deref(),
        Some("Bearer super-secret-token")
    );
    let content_type = post.header("content-type").unwrap();
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .expect("multipart");
    let (names, closed) = part_names(&post.body, boundary);
    assert_eq!(names, ["meta", "crop", "meta", "crop"]);
    assert!(closed);
    // Each meta part as the server reads it.
    let text = String::from_utf8_lossy(&post.body);
    for part in text.split(&format!("--{boundary}\r\n")).skip(1) {
        if !part.contains("name=\"meta\"") {
            continue;
        }
        let json = part.split("\r\n\r\n").nth(1).unwrap().trim_end();
        assert!(!json.contains("colorblind"), "{json}");
        assert!(!json.contains("null"), "{json}");
        let meta: TrainingCropUploadMeta = serde_json::from_str(json).unwrap();
        meta.validate().unwrap();
        assert!(game_token_ok(meta.game_token.as_deref().unwrap()));
        assert!(meta.found_spot.is_none() && meta.found_confidence.is_none());
    }
    for s in &seen {
        assert!(!s.head.contains("full_screen") && !s.head.contains("stat_digits"));
        assert!(!s.line().starts_with("PUT"));
    }
    // Sent crops leave the outbox; the non-consented kind is dropped, never sent.
    assert_eq!(on_disk(dir.path(), UploadKind::MapName), 0);
    assert_eq!(on_disk(dir.path(), UploadKind::StatDigits), 0);
    assert_eq!(cached_training_consent(dir.path()), now);
}

#[tokio::test]
async fn consent_off_on_upload_stops_that_kind_and_drops_its_crops() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::StatDigits, 1);
    queue(dir.path(), UploadKind::MapName, 1);
    let (url, server) = serve(vec![
        http(200, "", &consent_body(all_consent())),
        http(403, "", r#"{"error":"crop_consent_required"}"#),
        http(201, "", &stored("map_name", 1)),
    ]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 3, "the other kind still goes");
    assert_eq!(report.sent, 1);
    assert_eq!(report.warning, None, "consent off is not a failure");
    assert_eq!(on_disk(dir.path(), UploadKind::StatDigits), 0);
    assert!(!cached_training_consent(dir.path()).stat_digits);
    assert!(cached_training_consent(dir.path()).map_name);
    // Nothing more of that kind is kept until the site says it is on again.
    queue_refused(dir.path(), UploadKind::StatDigits);
}

fn queue_refused(dir: &std::path::Path, kind: UploadKind) {
    let meta = map_name_meta(&token(), 1920, 1080);
    assert!(!keep_training_crop(dir, kind, meta, png(64, 16, false)));
}

#[tokio::test]
async fn a_refused_token_leaves_a_warning_and_never_pauses_stats_sync() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 1);
    let (url, server) = serve(vec![http(401, "", r#"{"error":"Unauthorized"}"#)]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    server.join().unwrap();
    assert_eq!(report.retry, TrainingRetry::Pause);
    assert!(!report.consent_read);
    let warning = training_warning(dir.path()).unwrap();
    assert!(warning.contains("refused the sync token"), "{warning}");
    assert!(
        auth_pause_status(dir.path(), &url, "super-secret-token").is_none(),
        "the stats sync pause is untouched"
    );
    assert!(!dir.path().join(AUTH_PAUSE_FILE).exists());
    assert_eq!(
        on_disk(dir.path(), UploadKind::MapName),
        1,
        "kept for later"
    );
}

#[tokio::test]
async fn a_server_error_keeps_the_crops_and_warns() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::StatDigits, 1);
    queue(dir.path(), UploadKind::MapName, 1);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let again = listener.try_clone().unwrap();
    let (url, server) = serve_on(
        listener,
        vec![
            http(200, "", &consent_body(all_consent())),
            http(500, "", r#"{"error":"internal"}"#),
        ],
    );
    let report = run_training_pass(&client(&url), dir.path()).await;
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 2, "no more calls after a server error");
    assert_eq!(report.retry, TrainingRetry::Backoff);
    assert!(report.warning.as_deref().unwrap().contains("500"));
    assert_eq!(training_warning(dir.path()), report.warning);
    assert_eq!(on_disk(dir.path(), UploadKind::StatDigits), 1);
    assert_eq!(on_disk(dir.path(), UploadKind::MapName), 1);

    // The next pass that works, on the same site, clears the warning.
    let (url, server) = serve_on(
        again,
        vec![
            http(200, "", &consent_body(all_consent())),
            http(201, "", &stored("stat_digits", 1)),
            http(201, "", &stored("map_name", 1)),
        ],
    );
    let report = run_training_pass(&client(&url), dir.path()).await;
    server.join().unwrap();
    assert_eq!(report.sent, 2);
    assert_eq!(training_warning(dir.path()), None);
}

#[tokio::test]
async fn rate_limit_and_disabled_store_wait_without_dropping() {
    for (status, body, want) in [
        (
            429,
            r#"{"code":"rate_limited","error":"rate_limited","retry_after":90}"#,
            TrainingRetry::After(std::time::Duration::from_secs(90)),
        ),
        (
            503,
            r#"{"error":"hero_icons_disabled"}"#,
            TrainingRetry::Pause,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        consent(dir.path(), all_consent());
        queue(dir.path(), UploadKind::MapName, 1);
        let extra = if status == 429 {
            "Retry-After: 90\r\n"
        } else {
            ""
        };
        let (url, server) = serve(vec![
            http(200, "", &consent_body(all_consent())),
            http(status, extra, body),
        ]);
        let report = run_training_pass(&client(&url), dir.path()).await;
        server.join().unwrap();
        assert_eq!(report.retry, want, "{status}");
        assert!(training_warning(dir.path()).is_some(), "{status}");
        assert_eq!(on_disk(dir.path(), UploadKind::MapName), 1, "{status}");
    }
}

#[tokio::test]
async fn limit_reached_and_refused_batches_are_dropped_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::HeroIcon, 1);
    queue(dir.path(), UploadKind::MapName, 2);
    let (url, server) = serve(vec![
        http(200, "", &consent_body(all_consent())),
        http(409, "", r#"{"error":"crop_limit_reached"}"#),
        http(400, "", r#"{"error":"invalid_crop_meta"}"#),
    ]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen[1].line().contains("kind=hero_icon"));
    assert_eq!(
        report.retry,
        TrainingRetry::Normal,
        "no point retrying these"
    );
    let warning = training_warning(dir.path()).unwrap();
    assert!(warning.contains("hero icons"), "{warning}");
    assert!(warning.contains("invalid_crop_meta"), "{warning}");
    assert_eq!(on_disk(dir.path(), UploadKind::HeroIcon), 0);
    assert_eq!(on_disk(dir.path(), UploadKind::MapName), 0);
}

#[tokio::test]
async fn an_over_cap_crop_is_dropped_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 1);
    // A crop that grew past the cap on disk after it was kept.
    let outbox = dir.path().join("training_crops/outbox/map_name");
    let id = queued_crops(dir.path(), UploadKind::MapName)[0].id.clone();
    std::fs::write(outbox.join(format!("{id}.png")), png(1600, 40, false)).unwrap();
    let (url, server) = serve(vec![http(200, "", &consent_body(all_consent()))]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 1, "only the consent read");
    assert_eq!(report.sent, 0);
    assert_eq!(on_disk(dir.path(), UploadKind::MapName), 0);
}

#[tokio::test]
async fn delete_mine_calls_the_route_and_returns_the_count() {
    let (url, server) = serve(vec![
        http(200, "", r#"{"deleted":17}"#),
        http(500, "", r#"{"error":"internal"}"#),
    ]);
    let c = client(&url);
    assert_eq!(c.delete_own_training_crops().await, Ok(17));
    let err = c.delete_own_training_crops().await.unwrap_err();
    let seen = server.join().unwrap();
    assert_eq!(seen[0].line(), "DELETE /api/training-crops/mine HTTP/1.1");
    assert_eq!(
        seen[0].header("authorization").as_deref(),
        Some("Bearer super-secret-token")
    );
    assert_eq!(err.status(), Some(500));
    assert!(err.plain_text().contains("500"));
}

#[tokio::test]
async fn cleartext_off_loopback_sends_nothing() {
    let c = client("http://203.0.113.10");
    let err = c.fetch_training_consent().await.unwrap_err();
    assert!(matches!(err, TrainingCallError::Transport(_)));
    assert!(c.delete_own_training_crops().await.is_err());
}

#[tokio::test]
async fn crops_kept_for_another_website_or_token_are_never_sent() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!adopt_training_owner(
        dir.path(),
        "https://site-a.example",
        "token-a"
    ));
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 3);
    let (url, server) = serve(vec![http(200, "", &consent_body(all_consent()))]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 1, "only the new site's consent read");
    assert_eq!(report.sent, 0);
    assert!(!has_queued_crops(dir.path()));
    // The same pair again drops nothing.
    queue(dir.path(), UploadKind::MapName, 1);
    assert!(!adopt_training_owner(
        dir.path(),
        &url,
        "super-secret-token"
    ));
    assert!(has_queued_crops(dir.path()));
    // Another token for the same site does.
    assert!(adopt_training_owner(dir.path(), &url, "other-token"));
    assert!(!has_queued_crops(dir.path()));
    assert_eq!(
        cached_training_consent(dir.path()),
        TrainingCropConsent::default()
    );
}

#[tokio::test]
async fn crops_kept_before_a_delete_are_dropped_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 2);
    std::thread::sleep(std::time::Duration::from_millis(5));
    mark_training_delete(dir.path()).unwrap();
    let (url, server) = serve(vec![http(200, "", &consent_body(all_consent()))]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    assert_eq!(server.join().unwrap().len(), 1, "no upload");
    assert_eq!(report.sent, 0);
    assert!(!has_queued_crops(dir.path()));
}

#[tokio::test]
async fn crops_stored_while_a_delete_runs_are_removed_again() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 1);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let marker_dir = dir.path().to_path_buf();
    let server = std::thread::spawn(move || {
        let mut lines = Vec::new();
        let replies = [
            http(200, "", &consent_body(all_consent())),
            http(201, "", &stored("map_name", 1)),
            http(200, "", r#"{"deleted":1}"#),
        ];
        for reply in replies {
            let (mut sock, _) = listener.accept().unwrap();
            let seen = read_one(&mut sock);
            if seen.line().starts_with("POST") {
                // The member confirms the delete while the upload runs.
                mark_training_delete(&marker_dir).unwrap();
            }
            lines.push(seen.line().to_string());
            let _ = std::io::Write::write_all(&mut sock, reply.as_bytes());
        }
        lines
    });
    let report = run_training_pass(&client(&url), dir.path()).await;
    let lines = server.join().unwrap();
    assert_eq!(report.sent, 0, "nothing counts as sent");
    assert_eq!(
        lines[2],
        "DELETE /api/training-crops/3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c HTTP/1.1"
    );
}

#[tokio::test]
async fn a_dropped_crops_warning_stays_until_a_send_works() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 1);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let again = listener.try_clone().unwrap();
    let (url, server) = serve_on(
        listener,
        vec![
            http(200, "", &consent_body(all_consent())),
            http(409, "", r#"{"error":"crop_limit_reached"}"#),
            http(200, "", &consent_body(all_consent())),
        ],
    );
    let c = client(&url);
    run_training_pass(&c, dir.path()).await;
    // A later pass with nothing to send keeps it.
    run_training_pass(&c, dir.path()).await;
    server.join().unwrap();
    let warning = training_warning(dir.path()).expect("still shown");
    assert!(warning.contains("map names"), "{warning}");

    queue(dir.path(), UploadKind::MapName, 1);
    let (url, server) = serve_on(
        again,
        vec![
            http(200, "", &consent_body(all_consent())),
            http(201, "", &stored("map_name", 1)),
        ],
    );
    let report = run_training_pass(&client(&url), dir.path()).await;
    server.join().unwrap();
    assert_eq!(report.sent, 1, "same site: the crop was kept and sent");
    assert_eq!(training_warning(dir.path()), None);
}
