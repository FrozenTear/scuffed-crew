//! Training crops against a loopback server: the requests the tracker
//! sends, and what a pass does with each answer.

use scuffed_types::api::training_crops::{TrainingCropConsent, TrainingCropUploadMeta};

use crate::config::SyncConfig;

use super::tests::client;
use super::tests_training::{all_consent, consent, part_names, png, token};
use super::training_pass::{TRAINING_DELETE_AGAIN, TRAINING_DELETE_OWED};
use super::training_store::{pending_deletes, write_pending_deletes, write_training_warning};
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
    serve_hooked(listener, responses, |_| {})
}

/// [`serve_on`], calling `hook` with each request after it is read and
/// before it is answered. A request that never comes ends the server after
/// 3 seconds, so a missing call fails the test instead of hanging it.
fn serve_hooked(
    listener: std::net::TcpListener,
    responses: Vec<String>,
    hook: impl Fn(&Seen) + Send + 'static,
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
            let request = read_one(sock);
            hook(&request);
            seen.push(request);
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

/// Crop ids the site makes (32 lowercase hex).
const ID_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ID_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// A 201 answer that stored one crop per id in `ids`.
fn stored_ids(kind: &str, ids: &[&str]) -> String {
    let crops: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id, "kind": kind, "size_bytes": 1,
                "width": 8, "height": 8, "uploaded_at": "2026-11-10T18:04:05Z",
                "meta": {}, "thumbnail_url": "/x"
            })
        })
        .collect();
    serde_json::json!({ "kind": kind, "crops": crops }).to_string()
}

fn delete_line(id: &str) -> String {
    format!("DELETE /api/training-crops/{id} HTTP/1.1")
}

fn lines(seen: &[Seen]) -> Vec<String> {
    seen.iter().map(|s| s.line().to_string()).collect()
}

fn new_listener() -> std::net::TcpListener {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap()
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
    // Refused by the URL check before any connect: a connect to this
    // TEST-NET address would also fail as Transport, but with another text.
    let https_required = TrainingCallError::Transport(
        crate::config::SyncUrlReject::HttpsRequired
            .message()
            .to_string(),
    );
    let c = client("http://203.0.113.10");
    assert_eq!(
        c.fetch_training_consent().await.unwrap_err(),
        https_required
    );
    assert_eq!(
        c.delete_own_training_crops().await.unwrap_err(),
        https_required
    );
    let creds = c.credentials();
    let crop = queued_crop();
    assert_eq!(
        c.upload_training_crops(&creds, UploadKind::MapName, &[crop])
            .await
            .unwrap_err(),
        https_required
    );
    assert_eq!(
        c.delete_training_crop(&creds, ID_A).await.unwrap_err(),
        https_required
    );
}

/// One map_name crop, not on disk.
fn queued_crop() -> QueuedCrop {
    QueuedCrop {
        id: "0000000000001-00000000".into(),
        meta: map_name_meta(&token(), 1920, 1080),
        png: png(64, 16, false),
    }
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

/// The dropped crops warning stays until a send works: through a quiet
/// pass, a pass that warns about something else, a pass that clears the
/// ids an earlier delete left, and the quiet pass after those.
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
            http(500, "", r#"{"error":"internal"}"#),
            http(404, "", r#"{"error":"crop_not_found"}"#),
            http(200, "", &consent_body(all_consent())),
            http(200, "", &consent_body(all_consent())),
        ],
    );
    let c = client(&url);
    let dropped = |what: &str| {
        let warning = training_warning(dir.path()).expect(what);
        assert!(warning.contains("map names"), "{what}: {warning}");
        assert_eq!(
            warning.matches("Waiting ones were dropped").count(),
            1,
            "{what}: {warning}"
        );
        warning
    };
    let first = run_training_pass(&c, dir.path()).await;
    assert_eq!(first.sent, 0);
    dropped("the pass that dropped them");
    // A later pass with nothing to send keeps it.
    let quiet = run_training_pass(&c, dir.path()).await;
    assert_eq!(quiet.sent, 0);
    dropped("a quiet pass");
    // A pass that warns about something else keeps it too.
    let failed = run_training_pass(&c, dir.path()).await;
    assert_eq!(failed.sent, 0);
    let warning = dropped("a pass with another warning");
    assert!(warning.contains("consent"), "{warning}");
    // Clearing the ids an earlier delete left does not end it.
    write_pending_deletes(dir.path(), &[ID_A.to_string()]);
    let cleared = run_training_pass(&c, dir.path()).await;
    assert_eq!(cleared.sent, 0);
    assert!(pending_deletes(dir.path()).is_empty());
    let warning = dropped("a pass that cleared what a delete left");
    assert!(!warning.contains("consent"), "{warning}");
    run_training_pass(&c, dir.path()).await;
    server.join().unwrap();
    dropped("the quiet pass after those");

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

/// The member's delete is marked after the pass picked its batch and
/// before the upload is stored, and the desktop app's clock reads a second
/// behind the tracker's. The marker is older than the upload, but it
/// changed, so every crop the upload stored is removed by id. (A rule that
/// compares the marker's time with the upload's skips this cleanup.)
#[tokio::test]
async fn a_delete_marked_between_the_batch_and_the_upload_removes_what_it_stored() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 2);
    let marker = dir.path().join("training_crops").join("deleted_at");
    let (url, server) = serve_hooked(
        new_listener(),
        vec![
            http(200, "", &consent_body(all_consent())),
            http(201, "", &stored_ids("map_name", &[ID_A, ID_B])),
            http(200, "", r#"{"deleted":1}"#),
            http(200, "", r#"{"deleted":1}"#),
            http(200, "", &consent_body(all_consent())),
        ],
        move |seen| {
            if seen.line().starts_with("POST") {
                let at = chrono::Utc::now().timestamp_millis() - 1_000;
                std::fs::write(&marker, at.to_string()).unwrap();
            }
        },
    );
    let c = client(&url);
    let report = run_training_pass(&c, dir.path()).await;
    assert_eq!(report.sent, 0, "nothing counts as sent");
    assert_eq!(report.warning, None);
    assert!(pending_deletes(dir.path()).is_empty());
    assert!(!has_queued_crops(dir.path()));
    // The next pass sees the same marker: no delete call.
    let report = run_training_pass(&c, dir.path()).await;
    assert_eq!(report.warning, None);
    let seen = lines(&server.join().unwrap());
    assert_eq!(
        seen,
        [
            "GET /api/training-crops/consent HTTP/1.1".to_string(),
            "POST /api/training-crops?kind=map_name HTTP/1.1".to_string(),
            delete_line(ID_A),
            delete_line(ID_B),
            "GET /api/training-crops/consent HTTP/1.1".to_string(),
        ]
    );
}

/// A by-id delete the site fails (500) is saved, tried first on each
/// later pass before any upload, and warned about until it works.
#[tokio::test]
async fn a_failed_delete_by_id_is_saved_retried_first_and_warned_until_gone() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 1);
    let listener = new_listener();
    let second = listener.try_clone().unwrap();
    let third = listener.try_clone().unwrap();
    let marker_dir = dir.path().to_path_buf();
    let (url, server) = serve_hooked(
        listener,
        vec![
            http(200, "", &consent_body(all_consent())),
            http(201, "", &stored_ids("map_name", &[ID_A])),
            http(500, "", r#"{"error":"internal"}"#),
        ],
        move |seen| {
            if seen.line().starts_with("POST") {
                // The member confirms the delete while the upload runs.
                mark_training_delete(&marker_dir).unwrap();
            }
        },
    );
    let c = client(&url);
    let report = run_training_pass(&c, dir.path()).await;
    assert_eq!(
        lines(&server.join().unwrap())[2],
        delete_line(ID_A),
        "removed by id at once"
    );
    assert_eq!(report.sent, 0);
    assert_eq!(report.retry, TrainingRetry::Backoff);
    assert_eq!(pending_deletes(dir.path()), [ID_A]);
    let status = read_training_status(dir.path()).expect("a warning");
    assert!(status.sticky);
    assert!(
        status
            .warning
            .as_deref()
            .unwrap()
            .starts_with(TRAINING_DELETE_OWED),
        "{status:?}"
    );

    // Still failing: the retry goes first, and a failure ends the pass
    // before the consent read or any upload.
    std::thread::sleep(std::time::Duration::from_millis(5));
    queue(dir.path(), UploadKind::MapName, 1);
    let (_, server) = serve_on(second, vec![http(500, "", r#"{"error":"internal"}"#)]);
    let report = run_training_pass(&c, dir.path()).await;
    assert_eq!(lines(&server.join().unwrap()), [delete_line(ID_A)]);
    assert_eq!(report.retry, TrainingRetry::Backoff);
    assert_eq!(pending_deletes(dir.path()), [ID_A]);
    assert!(has_queued_crops(dir.path()), "not sent before the delete");
    let warning = training_warning(dir.path()).expect("still shown");
    assert!(warning.starts_with(TRAINING_DELETE_OWED), "{warning}");

    // It works: the warning goes, and the waiting crop is sent after it.
    let (_, server) = serve_on(
        third,
        vec![
            http(200, "", r#"{"deleted":1}"#),
            http(200, "", &consent_body(all_consent())),
            http(201, "", &stored_ids("map_name", &[ID_B])),
        ],
    );
    let report = run_training_pass(&c, dir.path()).await;
    let seen = lines(&server.join().unwrap());
    assert_eq!(seen[0], delete_line(ID_A));
    assert!(seen[2].starts_with("POST"), "{seen:?}");
    assert_eq!(report.sent, 1);
    assert!(pending_deletes(dir.path()).is_empty());
    assert_eq!(training_warning(dir.path()), None);
    assert_eq!(read_training_status(dir.path()), None);
}

/// 404 `crop_not_found` on a by-id delete means it is already gone: done.
/// Any other 404 is not, and stays to try again.
#[tokio::test]
async fn crop_not_found_counts_as_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let listener = new_listener();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    // The ids belong to this site and token.
    adopt_training_owner(dir.path(), &url, "super-secret-token");
    write_pending_deletes(dir.path(), &[ID_A.to_string(), ID_B.to_string()]);
    let (_, server) = serve_on(
        listener,
        vec![
            http(404, "", r#"{"error":"crop_not_found"}"#),
            http(404, "", r#"{"error":"Not Found"}"#),
        ],
    );
    let report = run_training_pass(&client(&url), dir.path()).await;
    assert_eq!(
        lines(&server.join().unwrap()),
        [delete_line(ID_A), delete_line(ID_B)]
    );
    assert_eq!(
        pending_deletes(dir.path()),
        [ID_B],
        "only the gone one is done"
    );
    assert_eq!(report.retry, TrainingRetry::Backoff);

    let listener = new_listener();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    adopt_training_owner(dir.path(), &url, "super-secret-token");
    write_pending_deletes(dir.path(), &[ID_B.to_string()]);
    let (_, server) = serve_on(
        listener,
        vec![
            http(404, "", r#"{"error":"crop_not_found"}"#),
            http(200, "", &consent_body(TrainingCropConsent::default())),
        ],
    );
    let report = run_training_pass(&client(&url), dir.path()).await;
    server.join().unwrap();
    assert_eq!(report.retry, TrainingRetry::Normal);
    assert!(pending_deletes(dir.path()).is_empty());
    assert_eq!(training_warning(dir.path()), None);
}

/// Settings saves another website and token while an upload runs, and a
/// delete runs too. The by-id delete still goes to the site the crops went
/// to, nothing reaches the new site, and the old site's consent and
/// warning are not written under the new owner.
#[tokio::test]
async fn a_credential_change_mid_pass_sends_nothing_to_the_new_site() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 1);
    let new_site = new_listener();
    new_site.set_nonblocking(true).unwrap();
    let new_url = format!("http://127.0.0.1:{}", new_site.local_addr().unwrap().port());
    let listener = new_listener();
    let old_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let c = client(&old_url);
    let settings = c.clone();
    let files = dir.path().to_path_buf();
    let switched = new_url.clone();
    let (_, server) = serve_hooked(
        listener,
        vec![
            http(200, "", &consent_body(all_consent())),
            http(201, "", &stored_ids("map_name", &[ID_A])),
            http(500, "", r#"{"error":"internal"}"#),
        ],
        move |seen| {
            if seen.line().starts_with("POST") {
                settings.replace_credentials(SyncConfig {
                    server_url: switched.clone(),
                    token: "token-b".into(),
                });
                // The daemon's next tick takes the new owner.
                adopt_training_owner(&files, &switched, "token-b");
                mark_training_delete(&files).unwrap();
            }
        },
    );
    let report = run_training_pass(&c, dir.path()).await;
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 3, "{:?}", lines(&seen));
    assert_eq!(seen[2].line(), delete_line(ID_A), "to the old site");
    assert_eq!(
        seen[2].header("authorization").as_deref(),
        Some("Bearer super-secret-token"),
        "with the old token"
    );
    assert!(
        new_site.accept().is_err(),
        "nothing reached the new website"
    );
    assert!(report.warning.is_some(), "the by-id delete failed");
    // Nothing the pass learned is written under the new owner.
    assert_eq!(
        cached_training_consent(dir.path()),
        TrainingCropConsent::default()
    );
    assert_eq!(read_training_status(dir.path()), None);
    assert!(pending_deletes(dir.path()).is_empty());
}

/// The member's delete is marked after the pass picked its batch, and the
/// upload then gets no answer (the connection drops). The site may have
/// stored the batch after the delete, and the tracker has no ids for it: a
/// sticky warning asks the member to delete again, no id is made up, and
/// the warning stays, even after a later send works and after a delete
/// pressed again that failed, until a delete the member pressed again is
/// answered with success.
#[tokio::test]
async fn no_answer_to_a_send_during_a_delete_asks_to_delete_again() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 1);
    let listener = new_listener();
    let second = listener.try_clone().unwrap();
    let third = listener.try_clone().unwrap();
    let marker_dir = dir.path().to_path_buf();
    let (url, server) = serve_hooked(
        listener,
        vec![
            http(200, "", &consent_body(all_consent())),
            // Nothing comes back: the connection closes unanswered.
            String::new(),
        ],
        move |seen| {
            if seen.line().starts_with("POST") {
                mark_training_delete(&marker_dir).unwrap();
            }
        },
    );
    let c = client(&url);
    let report = run_training_pass(&c, dir.path()).await;
    let seen = lines(&server.join().unwrap());
    assert_eq!(seen.len(), 2, "no delete by id: {seen:?}");
    assert_eq!(report.sent, 0);
    assert_eq!(report.retry, TrainingRetry::Backoff);
    assert_eq!(report.warning.as_deref(), Some(TRAINING_DELETE_AGAIN));
    assert!(pending_deletes(dir.path()).is_empty(), "no invented ids");
    let status = read_training_status(dir.path()).expect("a warning");
    assert!(status.sticky);
    assert_eq!(status.warning.as_deref(), Some(TRAINING_DELETE_AGAIN));
    assert!(status.delete_again_at.is_some());

    // A later send that works does not hide it.
    std::thread::sleep(std::time::Duration::from_millis(5));
    queue(dir.path(), UploadKind::MapName, 1);
    let (_, server) = serve_on(
        second,
        vec![
            http(200, "", &consent_body(all_consent())),
            http(201, "", &stored_ids("map_name", &[ID_A])),
        ],
    );
    let report = run_training_pass(&c, dir.path()).await;
    server.join().unwrap();
    assert_eq!(report.sent, 1);
    assert_eq!(
        training_warning(dir.path()).as_deref(),
        Some(TRAINING_DELETE_AGAIN)
    );

    // A delete the website did not answer with success (offline, 429)
    // leaves the ask, on screen and after the next pass.
    std::thread::sleep(std::time::Duration::from_millis(5));
    mark_training_delete(dir.path()).unwrap();
    assert_eq!(
        training_warning(dir.path()).as_deref(),
        Some(TRAINING_DELETE_AGAIN)
    );
    let fourth = third.try_clone().unwrap();
    let (_, server) = serve_on(third, vec![http(200, "", &consent_body(all_consent()))]);
    let report = run_training_pass(&c, dir.path()).await;
    server.join().unwrap();
    assert_eq!(report.warning.as_deref(), Some(TRAINING_DELETE_AGAIN));
    assert_eq!(
        training_warning(dir.path()).as_deref(),
        Some(TRAINING_DELETE_AGAIN)
    );

    // The member deletes again and the website answers 200: the ask is
    // gone from the app's view at once, and the next pass clears it.
    std::thread::sleep(std::time::Duration::from_millis(5));
    let marker = mark_training_delete(dir.path()).unwrap();
    record_training_delete_ok(dir.path(), marker).unwrap();
    assert_eq!(training_warning(dir.path()), None);
    let (_, server) = serve_on(fourth, vec![http(200, "", &consent_body(all_consent()))]);
    let report = run_training_pass(&c, dir.path()).await;
    server.join().unwrap();
    assert_eq!(report.warning, None);
    assert_eq!(read_training_status(dir.path()), None);
}

/// An ask to delete again that sits with another warning: a delete that
/// worked leaves out only the ask, in the app's view.
#[test]
fn a_delete_that_worked_leaves_out_only_the_ask_to_delete_again() {
    let dir = tempfile::tempdir().unwrap();
    let asked = mark_training_delete(dir.path()).unwrap();
    let both = format!("{TRAINING_DELETE_AGAIN} {TRAINING_DELETE_OWED}");
    write_training_warning(dir.path(), Some(&both), true, false, None, Some(asked));
    // The website answered the delete the ask was left under, not a new one.
    record_training_delete_ok(dir.path(), asked).unwrap();
    assert_eq!(training_warning(dir.path()), Some(both));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let marker = mark_training_delete(dir.path()).unwrap();
    record_training_delete_ok(dir.path(), marker).unwrap();
    assert_eq!(
        training_warning(dir.path()).as_deref(),
        Some(TRAINING_DELETE_OWED)
    );
}

/// An upload that gets no answer when no delete ran meanwhile, though one
/// ran before the crop was kept: the plain "could not reach" warning, not
/// sticky and no ask to delete again, the crop stays queued, and the pass
/// backs off.
#[tokio::test]
async fn no_answer_to_a_send_with_no_delete_meanwhile_is_a_plain_warning() {
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    mark_training_delete(dir.path()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    queue(dir.path(), UploadKind::MapName, 1);
    let (url, server) = serve(vec![
        http(200, "", &consent_body(all_consent())),
        // Nothing comes back: the connection closes unanswered.
        String::new(),
    ]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    let seen = lines(&server.join().unwrap());
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen[1].starts_with("POST"), "{seen:?}");
    assert_eq!(report.sent, 0);
    assert_eq!(report.retry, TrainingRetry::Backoff);
    let warning = report.warning.expect("a warning");
    assert!(
        warning.contains("Could not reach the website."),
        "{warning}"
    );
    assert!(!warning.contains(TRAINING_DELETE_AGAIN), "{warning}");
    let status = read_training_status(dir.path()).expect("a warning");
    assert!(!status.sticky);
    assert_eq!(status.delete_again_at, None);
    assert_eq!(status.warning, Some(warning));
    assert_eq!(queued_crops(dir.path(), UploadKind::MapName).len(), 1);
}

/// A new token for the same website keeps the ids still to delete and
/// their warning, since the site lets any token of the member delete by
/// id; it still drops waiting crops and the cached consent. Another
/// website drops them all.
#[tokio::test]
async fn a_new_token_keeps_crops_still_to_delete_another_website_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let listener = new_listener();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    assert!(!adopt_training_owner(dir.path(), &url, "token-a"));
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 1);
    write_pending_deletes(dir.path(), &[ID_A.to_string()]);
    write_training_warning(
        dir.path(),
        Some(TRAINING_DELETE_OWED),
        true,
        false,
        None,
        None,
    );

    assert!(adopt_training_owner(dir.path(), &url, "token-b"));
    assert!(!has_queued_crops(dir.path()));
    assert_eq!(
        cached_training_consent(dir.path()),
        TrainingCropConsent::default()
    );
    assert_eq!(pending_deletes(dir.path()), [ID_A]);
    assert_eq!(
        training_warning(dir.path()).as_deref(),
        Some(TRAINING_DELETE_OWED)
    );

    // A pass with yet another token removes it, then clears the warning.
    let (_, server) = serve_on(
        listener,
        vec![
            http(200, "", r#"{"deleted":1}"#),
            http(200, "", &consent_body(TrainingCropConsent::default())),
        ],
    );
    let report = run_training_pass(&client(&url), dir.path()).await;
    let seen = server.join().unwrap();
    assert_eq!(seen[0].line(), delete_line(ID_A));
    assert_eq!(
        seen[0].header("authorization").as_deref(),
        Some("Bearer super-secret-token")
    );
    assert_eq!(report.warning, None);
    assert!(pending_deletes(dir.path()).is_empty());
    assert_eq!(read_training_status(dir.path()), None);

    // Another website drops the ids and the warning.
    write_pending_deletes(dir.path(), &[ID_B.to_string()]);
    write_training_warning(
        dir.path(),
        Some(TRAINING_DELETE_OWED),
        true,
        false,
        None,
        None,
    );
    assert!(adopt_training_owner(
        dir.path(),
        "https://site-b.example",
        "super-secret-token"
    ));
    assert!(pending_deletes(dir.path()).is_empty());
    assert_eq!(read_training_status(dir.path()), None);
}

/// A consent read that fails (offline, 5xx, a site without these routes)
/// warns only a member who uses training crops: a kind on, or crops
/// waiting. A member who never opted in sees nothing.
#[tokio::test]
async fn a_failed_consent_read_warns_only_when_training_crops_are_in_use() {
    // Never opted in, nothing waiting.
    let dir = tempfile::tempdir().unwrap();
    let (url, server) = serve(vec![http(500, "", r#"{"error":"internal"}"#)]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    server.join().unwrap();
    assert_eq!(report.retry, TrainingRetry::Backoff, "still backs off");
    assert_eq!(report.warning, None);
    assert_eq!(read_training_status(dir.path()), None);
    let (url, server) = serve(vec![http(404, "", r#"{"error":"Not Found"}"#)]);
    run_training_pass(&client(&url), dir.path()).await;
    server.join().unwrap();
    assert_eq!(read_training_status(dir.path()), None);

    // A kind on, nothing waiting.
    let dir = tempfile::tempdir().unwrap();
    consent(
        dir.path(),
        TrainingCropConsent {
            map_name: true,
            ..TrainingCropConsent::default()
        },
    );
    let (url, server) = serve(vec![http(500, "", r#"{"error":"internal"}"#)]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    server.join().unwrap();
    assert!(report.warning.as_deref().unwrap().contains("consent"));
    assert_eq!(training_warning(dir.path()), report.warning);

    // Every kind off now, but crops still wait.
    let dir = tempfile::tempdir().unwrap();
    consent(dir.path(), all_consent());
    queue(dir.path(), UploadKind::MapName, 1);
    consent(dir.path(), TrainingCropConsent::default());
    let (url, server) = serve(vec![http(503, "", r#"{"error":"hero_icons_disabled"}"#)]);
    let report = run_training_pass(&client(&url), dir.path()).await;
    server.join().unwrap();
    assert!(report.warning.is_some());
    assert!(training_warning(dir.path()).is_some());
}

/// A crop a Tab could not keep leaves its own warning, but never drops an
/// open ask to delete again: the next pass still shows that ask.
#[tokio::test]
async fn a_lost_tab_crop_keeps_the_ask_to_delete_again() {
    let dir = tempfile::tempdir().unwrap();
    let listener = new_listener();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    adopt_training_owner(dir.path(), &url, "super-secret-token");
    mark_training_delete(dir.path()).unwrap();
    let marker = super::training_store::training_delete_at(dir.path());
    super::training_store::write_training_warning(
        dir.path(),
        Some(TRAINING_DELETE_AGAIN),
        true,
        false,
        None,
        marker,
    );
    warn_crop_lost(dir.path(), UploadKind::StatDigits);
    let status = read_training_status(dir.path()).expect("a warning");
    let warning = status.warning.as_deref().unwrap();
    assert!(warning.contains("could not be kept"), "{warning}");
    assert!(
        warning.starts_with(TRAINING_DELETE_AGAIN),
        "the app still shows the ask: {warning}"
    );
    assert!(status.sticky);
    assert_eq!(status.delete_again_at, marker, "the ask is kept");

    let (_, server) = serve_on(
        listener,
        vec![http(200, "", &consent_body(TrainingCropConsent::default()))],
    );
    let report = run_training_pass(&client(&url), dir.path()).await;
    server.join().unwrap();
    assert_eq!(report.warning.as_deref(), Some(TRAINING_DELETE_AGAIN));
}

/// An ask a delete that worked has met is not carried into a lost crop
/// warning, and with no ask the warning is the lost crop line alone.
#[test]
fn a_lost_tab_crop_does_not_carry_a_met_ask_to_delete_again() {
    let dir = tempfile::tempdir().unwrap();
    warn_crop_lost(dir.path(), UploadKind::HeroIcon);
    let status = read_training_status(dir.path()).expect("a warning");
    assert!(
        status
            .warning
            .as_deref()
            .unwrap()
            .starts_with("Training crops: hero")
    );
    assert_eq!(status.delete_again_at, None);

    let asked = mark_training_delete(dir.path()).unwrap();
    super::training_store::write_training_warning(
        dir.path(),
        Some(TRAINING_DELETE_AGAIN),
        true,
        false,
        None,
        Some(asked),
    );
    std::thread::sleep(std::time::Duration::from_millis(5));
    let marker = mark_training_delete(dir.path()).unwrap();
    record_training_delete_ok(dir.path(), marker).unwrap();
    warn_crop_lost(dir.path(), UploadKind::StatDigits);
    let status = read_training_status(dir.path()).expect("a warning");
    let warning = status.warning.as_deref().unwrap();
    assert!(!warning.contains(TRAINING_DELETE_AGAIN), "{warning}");
    assert_eq!(status.delete_again_at, None);
}

/// A crop a Tab could not keep never drops the dropped crops warning a pass
/// left: the app still shows it and the next pass still carries it.
#[test]
fn a_lost_tab_crop_keeps_the_dropped_crops_warning() {
    let dir = tempfile::tempdir().unwrap();
    let dropped = "Training crops: the website refused 1 hero icon (bad). They were dropped.";
    super::training_store::write_training_warning(
        dir.path(),
        Some(dropped),
        true,
        false,
        Some(dropped),
        None,
    );
    warn_crop_lost(dir.path(), UploadKind::StatDigits);
    let status = read_training_status(dir.path()).expect("a warning");
    let warning = status.warning.as_deref().unwrap();
    assert!(warning.starts_with(dropped), "{warning}");
    assert!(warning.contains("could not be kept"), "{warning}");
    assert!(status.sticky);
    assert_eq!(status.dropped.as_deref(), Some(dropped), "still carried");
    assert_eq!(status.delete_again_at, None);
}
