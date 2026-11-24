//! Tests against a loopback HTTP server: bearer and cleartext refusal,
//! status mapping, error bodies and `Retry-After`.

use super::tests::client;
use super::*;

#[test]
fn try_new_rejects_cleartext_non_loopback_and_allows_https_and_loopback() {
    assert!(
        SyncClient::try_new(SyncConfig {
            server_url: "http://example.com".into(),
            token: "t".into(),
        })
        .is_err()
    );
    assert!(
        SyncClient::try_new(SyncConfig {
            server_url: "https://example.com".into(),
            token: "t".into(),
        })
        .is_ok()
    );
    assert!(
        SyncClient::try_new(SyncConfig {
            server_url: "http://127.0.0.1:9".into(),
            token: "t".into(),
        })
        .is_ok()
    );
    assert!(
        SyncClient::try_new(SyncConfig {
            server_url: "http://[::1]:9".into(),
            token: "t".into(),
        })
        .is_ok()
    );
    assert!(
        SyncClient::try_new(SyncConfig {
            server_url: "http://localhost:9".into(),
            token: "t".into(),
        })
        .is_ok()
    );
}

#[tokio::test]
async fn cleartext_non_loopback_does_not_send_bearer() {
    // TEST-NET-3. If the guard did not return first, reqwest would sit
    // on the 30s client timeout (or a connect error). A fast policy
    // error that omits the token is the refusal.
    let started = std::time::Instant::now();
    let c = client("http://203.0.113.10/stats");
    let fetch_err = c.fetch_daemon_config().await.unwrap_err().to_string();
    let upload_err = c.upload_matches(&[], &[]).await.unwrap_err().to_string();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "refused in {:?}",
        started.elapsed()
    );
    for err in [&fetch_err, &upload_err] {
        assert!(
            err.contains("https"),
            "expected the scheme refusal, got {err}"
        );
        assert!(
            !err.contains("super-secret-token"),
            "token leaked into the error: {err}"
        );
        assert!(
            !err.contains("error sending request"),
            "request left the process: {err}"
        );
    }
}

#[tokio::test]
async fn loopback_http_sends_bearer_token() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();
    let server = std::thread::spawn(move || recv_one_http(listener));
    let c = client(&format!("http://127.0.0.1:{port}"));
    let _ = c.fetch_daemon_config().await;
    let req = server.join().expect("server thread");
    let head = req.to_ascii_lowercase();
    assert!(
        head.contains("authorization: bearer super-secret-token"),
        "loopback http is the local-dev exception and must still authenticate; got {req:?}"
    );
    assert!(
        head.contains("get /api/stats/daemon-config"),
        "unexpected request {req:?}"
    );
    assert!(
        head.contains(&format!(
            "user-agent: scuffed-stat-tracker/{}\r\n",
            env!("CARGO_PKG_VERSION")
        )),
        "every site request names the tracker version; got {req:?}"
    );
}

fn recv_one_http(listener: std::net::TcpListener) -> String {
    exchange_http(
        listener,
        b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n",
    )
}

fn exchange_http(listener: std::net::TcpListener, response: &[u8]) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    listener.set_nonblocking(true).expect("nonblocking");
    loop {
        match listener.accept() {
            Ok((mut sock, _)) => {
                sock.set_nonblocking(false).ok();
                sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .ok();
                let mut buf = [0u8; 4096];
                let mut got = Vec::new();
                loop {
                    match std::io::Read::read(&mut sock, &mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            got.extend_from_slice(&buf[..n]);
                            if got.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let _ = std::io::Write::write_all(&mut sock, response);
                return String::from_utf8_lossy(&got).into_owned();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() > deadline {
                    return String::new();
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => return String::new(),
        }
    }
}

#[tokio::test]
async fn upload_reads_retry_after_on_429_and_503() {
    let secs = upload_rejected(429, "120").await;
    assert_eq!(secs.status, Some(429));
    assert_eq!(secs.retry_after, Some(std::time::Duration::from_secs(120)));
    assert!(matches!(
        secs.attempt(),
        SyncAttempt::RateLimited {
            retry_after: Some(d)
        } if d == std::time::Duration::from_secs(120)
    ));

    let when = chrono::Utc::now() + chrono::TimeDelta::seconds(90);
    let header = when.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let dated = upload_rejected(503, &header).await;
    assert_eq!(dated.status, Some(503));
    let wait = dated.retry_after.expect("HTTP-date Retry-After");
    assert!(
        (80..=100).contains(&wait.as_secs()),
        "expected about 90s from the HTTP-date, got {wait:?}"
    );
    assert!(matches!(
        dated.attempt(),
        SyncAttempt::ServerError {
            retry_after: Some(_)
        }
    ));
}

async fn upload_rejected(status: u16, retry_after: &str) -> SyncUploadError {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();
    let response = format!(
        "HTTP/1.1 {status} no\r\nRetry-After: {retry_after}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let response_bytes = response.into_bytes();
    let server = std::thread::spawn(move || exchange_http(listener, &response_bytes));
    let c = client(&format!("http://127.0.0.1:{port}"));
    let err = c.upload_matches(&[], &[]).await.expect_err("rejected");
    let _ = server.join().expect("server thread");
    err
}

#[tokio::test]
async fn upload_401_and_403_pause_without_growing_the_failure_streak() {
    let t0 = std::time::Instant::now();
    for status in [401_u16, 403] {
        let err = upload_rejected(status, "30").await;
        assert_eq!(err.status, Some(status));
        assert!(
            err.retry_after.is_none(),
            "401/403 must ignore Retry-After, got {:?}",
            err.retry_after
        );
        assert!(matches!(err.attempt(), SyncAttempt::AuthRejected));
        let mut backoff = SyncBackoff::default();
        backoff.observe(err.attempt(), t0);
        assert!(backoff.auth_rejected());
        assert_eq!(
            backoff.failures(),
            0,
            "auth rejection is not a server-error streak"
        );
        assert!(!backoff.should_attempt(t0));
        assert!(
            !backoff.should_attempt(t0 + SYNC_BACKOFF_CAP + std::time::Duration::from_secs(1)),
            "a rejected token must not become due again on the backoff clock"
        );
    }
}

#[tokio::test]
async fn daemon_config_401_is_the_same_auth_rejection() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let server = std::thread::spawn(move || {
        exchange_http(
            listener,
            b"HTTP/1.1 401 no\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
    });
    let c = client(&format!("http://127.0.0.1:{port}"));
    let err = c.fetch_daemon_config().await.expect_err("401");
    let _ = server.join().expect("server");
    assert_eq!(err.status, Some(401));
    assert!(matches!(err.attempt(), SyncAttempt::AuthRejected));
}

#[tokio::test]
async fn upload_500_stays_a_server_error_and_ignores_retry_after() {
    let err = upload_rejected(500, "30").await;
    assert!(matches!(
        err.attempt(),
        SyncAttempt::ServerError { retry_after: None }
    ));
    let t0 = std::time::Instant::now();
    let mut backoff = SyncBackoff::default();
    backoff.observe(err.attempt(), t0);
    assert!(!backoff.auth_rejected());
    assert_eq!(backoff.failures(), 1);
    assert_eq!(backoff.retry_after(t0), SYNC_BACKOFF_BASE);
}

#[test]
fn server_error_text_prefers_the_json_error_field() {
    assert_eq!(
        server_error_text(
            "Upload",
            400,
            r#"{"error":"matches[0]: hero is not allowed"}"#
        ),
        "matches[0]: hero is not allowed"
    );
    assert_eq!(
        server_error_text("Upload", 422, "  plain refusal  "),
        "plain refusal"
    );
    assert_eq!(
        server_error_text("Upload", 500, ""),
        "Upload failed (HTTP 500)"
    );
}

#[tokio::test]
async fn upload_400_keeps_the_server_error_field() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let body = r#"{"error":"matches[0]: hero is not allowed"}"#;
    let response = format!(
        "HTTP/1.1 400 no\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response_bytes = response.into_bytes();
    let server = std::thread::spawn(move || exchange_http(listener, &response_bytes));
    let c = client(&format!("http://127.0.0.1:{port}"));
    let err = c.upload_matches(&[], &[]).await.expect_err("400");
    let _ = server.join().expect("server");
    assert_eq!(err.status, Some(400));
    assert_eq!(err.message, "matches[0]: hero is not allowed");
    assert!(err.is_payload_reject());
    assert!(matches!(
        err.attempt(),
        SyncAttempt::ServerError { retry_after: None }
    ));
}

#[tokio::test]
async fn upload_413_batch_too_large_carries_the_server_caps() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let body = r#"{"code":"batch_too_large","error":"batch too large","max_matches":50,"max_deleted_sessions":100}"#;
    let response = format!(
        "HTTP/1.1 413 no\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response_bytes = response.into_bytes();
    let server = std::thread::spawn(move || exchange_http(listener, &response_bytes));
    let c = client(&format!("http://127.0.0.1:{port}"));
    let err = c.upload_matches(&[], &[]).await.expect_err("413");
    let _ = server.join().expect("server");
    assert!(err.is_batch_too_large());
    assert_eq!(err.max_matches, Some(50));
    assert_eq!(err.max_deleted_sessions, Some(100));
    assert!(matches!(err.attempt(), SyncAttempt::BatchTooLarge));
    let t0 = std::time::Instant::now();
    let mut backoff = SyncBackoff::default();
    backoff.observe(err.attempt(), t0);
    assert_eq!(
        backoff.failures(),
        1,
        "a 413 the pass cannot shrink backs off"
    );
    assert!(!backoff.should_attempt(t0));
    assert_eq!(backoff.retry_after(t0), SYNC_BACKOFF_BASE);
    assert!(!backoff.auth_rejected());
}

#[tokio::test]
async fn upload_401_token_expired_pauses_without_a_failure_streak() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let body = r#"{"code":"token_expired","error":"token expired"}"#;
    let response = format!(
        "HTTP/1.1 401 no\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response_bytes = response.into_bytes();
    let server = std::thread::spawn(move || exchange_http(listener, &response_bytes));
    let c = client(&format!("http://127.0.0.1:{port}"));
    let err = c.upload_matches(&[], &[]).await.expect_err("401");
    let _ = server.join().expect("server");
    assert!(err.is_token_expired());
    assert!(matches!(err.attempt(), SyncAttempt::TokenExpired));
    let t0 = std::time::Instant::now();
    let mut backoff = SyncBackoff::default();
    backoff.observe(err.attempt(), t0);
    assert!(backoff.auth_rejected());
    assert_eq!(backoff.failures(), 0);
    assert!(!backoff.should_attempt(t0));
}

#[tokio::test]
async fn upload_409_stats_delete_pending_pauses_until_a_new_token() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let body = r#"{"code":"stats_delete_pending"}"#;
    let response = format!(
        "HTTP/1.1 409 no\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response_bytes = response.into_bytes();
    let server = std::thread::spawn(move || exchange_http(listener, &response_bytes));
    let c = client(&format!("http://127.0.0.1:{port}"));
    let err = c.upload_matches(&[], &[]).await.expect_err("409");
    let _ = server.join().expect("server");
    assert_eq!(err.code.as_deref(), Some("stats_delete_pending"));
    assert!(c.take_delete_scheduled_at().is_none());
    assert!(err.is_stats_delete_pending());
    assert!(matches!(err.attempt(), SyncAttempt::StatsDeletePending));
    let t0 = std::time::Instant::now();
    let mut backoff = SyncBackoff::default();
    backoff.observe(err.attempt(), t0);
    assert_eq!(backoff.failures(), 0);
    assert!(!backoff.auth_rejected());
    assert!(backoff.delete_pending());
    assert!(!backoff.should_attempt(t0));
    assert_eq!(backoff.retry_after(t0), std::time::Duration::ZERO);
    let later = t0 + std::time::Duration::from_secs(24 * 60 * 60);
    assert!(
        !backoff.should_attempt(later),
        "a stats delete does not retry on its own"
    );
    backoff.clear_auth_rejected();
    assert!(!backoff.delete_pending());
    assert!(backoff.should_attempt(t0));
}

#[tokio::test]
async fn upload_429_rate_limited_keeps_retry_after() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let body = r#"{"error":"rate_limited","retry_after":30}"#;
    let response = format!(
        "HTTP/1.1 429 no\r\nRetry-After: 45\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response_bytes = response.into_bytes();
    let server = std::thread::spawn(move || exchange_http(listener, &response_bytes));
    let c = client(&format!("http://127.0.0.1:{port}"));
    let err = c.upload_matches(&[], &[]).await.expect_err("429");
    let _ = server.join().expect("server");
    assert_eq!(err.code.as_deref(), Some("rate_limited"));
    assert_eq!(err.retry_after, Some(std::time::Duration::from_secs(45)));
    assert!(matches!(
        err.attempt(),
        SyncAttempt::RateLimited {
            retry_after: Some(wait)
        } if wait == std::time::Duration::from_secs(45)
    ));
}

/// Serve one canned response per connection, in order, and return each
/// request head. Stops at the first accept that does not come within the
/// deadline, so a request that never arrives shows as a missing entry.
fn serve_http(listener: std::net::TcpListener, responses: Vec<Vec<u8>>) -> Vec<String> {
    let mut heads = Vec::new();
    for response in responses {
        let head = exchange_http(listener.try_clone().expect("clone listener"), &response);
        if head.is_empty() {
            break;
        }
        heads.push(head);
    }
    heads
}

fn redirect_to(location: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

#[test]
fn same_origin_needs_scheme_host_and_port() {
    let url = |raw: &str| reqwest::Url::parse(raw).expect("url");
    let base = url("https://crew.example/api/stats/upload");
    assert!(super::client::same_origin(
        &base,
        &url("https://crew.example:443/other/path?x=1")
    ));
    assert!(super::client::same_origin(
        &base,
        &url("https://CREW.example/api/stats/upload")
    ));
    for other in [
        "http://crew.example/api/stats/upload",
        "https://crew.example:8443/api/stats/upload",
        "https://www.crew.example/api/stats/upload",
        "https://evil.example/api/stats/upload",
    ] {
        assert!(!super::client::same_origin(&base, &url(other)), "{other}");
    }
}

#[tokio::test]
async fn redirect_to_another_port_is_refused_before_it_is_sent() {
    let site = std::net::TcpListener::bind("127.0.0.1:0").expect("bind site");
    let site_port = site.local_addr().expect("addr").port();
    let elsewhere = std::net::TcpListener::bind("127.0.0.1:0").expect("bind elsewhere");
    let elsewhere_port = elsewhere.local_addr().expect("addr").port();
    let target = format!("http://127.0.0.1:{elsewhere_port}/moved-path?code=hidden");
    let site_server = std::thread::spawn(move || exchange_http(site, &redirect_to(&target)));
    let elsewhere_server = std::thread::spawn(move || recv_one_http(elsewhere));

    let c = client(&format!("http://127.0.0.1:{site_port}"));
    let err = c.fetch_daemon_config().await.expect_err("redirect refused");
    let first = site_server.join().expect("site");
    let leaked = elsewhere_server.join().expect("elsewhere");

    assert!(
        first
            .to_ascii_lowercase()
            .contains("authorization: bearer super-secret-token"),
        "the configured server still gets the token: {first:?}"
    );
    assert!(
        leaked.is_empty(),
        "nothing may reach the redirect target, got {leaked:?}"
    );
    assert!(
        err.message.contains(&format!(
            "the site redirected sync to http://127.0.0.1:{elsewhere_port}; \
             if that is your site's new address, set the Website URL in Settings to it"
        )),
        "the member is told why and where: {}",
        err.message
    );
    assert!(
        !err.message.contains("moved-path") && !err.message.contains("hidden"),
        "only the target's origin is named: {}",
        err.message
    );
    assert!(
        !err.message.contains("super-secret-token"),
        "{}",
        err.message
    );
    assert_eq!(err.status, None, "a refused redirect is not a site status");
    assert!(
        !matches!(err.attempt(), SyncAttempt::AuthRejected),
        "a refused redirect must not pause sync as a rejected token"
    );
}

#[tokio::test]
async fn redirect_to_another_host_or_scheme_is_refused() {
    for target in [
        // Same port, other host name: still another origin.
        "http://localhost:{port}/api/stats/upload",
        // Same host and port, other scheme.
        "https://127.0.0.1:{port}/api/stats/upload",
        "https://203.0.113.10/api/stats/upload",
    ] {
        let site = std::net::TcpListener::bind("127.0.0.1:0").expect("bind site");
        let port = site.local_addr().expect("addr").port();
        let location = target.replace("{port}", &port.to_string());
        let responses = vec![redirect_to(&location), redirect_to(&location)];
        let server = std::thread::spawn(move || serve_http(site, responses));
        let c = client(&format!("http://127.0.0.1:{port}"));
        let err = c.upload_matches(&[], &[]).await.expect_err("refused");
        let heads = server.join().expect("site");
        assert_eq!(heads.len(), 1, "{location}: only the first request is sent");
        let origin = reqwest::Url::parse(&location)
            .expect("url")
            .origin()
            .ascii_serialization();
        assert!(
            err.message.contains(&format!(
                "the site redirected sync to {origin}; \
                 if that is your site's new address, set the Website URL in Settings to it"
            )),
            "{location}: {}",
            err.message
        );
    }
}

#[tokio::test]
async fn same_site_redirect_is_followed_with_the_token() {
    let site = std::net::TcpListener::bind("127.0.0.1:0").expect("bind site");
    let port = site.local_addr().expect("addr").port();
    let responses = vec![
        redirect_to("/api/stats/daemon-config/"),
        b"HTTP/1.1 401 no\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    ];
    let server = std::thread::spawn(move || serve_http(site, responses));
    let c = client(&format!("http://127.0.0.1:{port}"));
    let err = c
        .fetch_daemon_config()
        .await
        .expect_err("401 after the hop");
    let heads = server.join().expect("site");
    assert_eq!(heads.len(), 2, "the same-site hop is followed: {heads:?}");
    let second = heads[1].to_ascii_lowercase();
    assert!(
        second.contains("get /api/stats/daemon-config/ "),
        "{second:?}"
    );
    assert!(
        second.contains("authorization: bearer super-secret-token"),
        "{second:?}"
    );
    assert_eq!(err.status, Some(401));
}

/// `is_off_site_redirect` holds only for the policy's off-site refusal:
/// the desktop app shows `REDIRECT_OFF_SITE` for it, and its generic
/// "Could not reach the site" for a redirect loop or a closed port.
#[tokio::test]
async fn only_an_off_site_redirect_is_reported_as_one() {
    let http = reqwest::Client::builder()
        .redirect(super::token_safe_redirects())
        .build()
        .expect("client");

    let site = std::net::TcpListener::bind("127.0.0.1:0").expect("bind site");
    let port = site.local_addr().expect("addr").port();
    let server = std::thread::spawn(move || {
        serve_http(
            site,
            vec![redirect_to("https://203.0.113.10:8443/elsewhere?k=v")],
        )
    });
    let err = http
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .await
        .expect_err("refused");
    server.join().expect("site");
    assert!(super::is_off_site_redirect(&err), "{err:?}");
    // The desktop app's advice names the target's origin, not its path.
    let message =
        super::off_site_redirect_message(&err, super::WebsiteUrlField::Settings).expect("off-site");
    assert_eq!(
        message,
        "The site redirected to https://203.0.113.10:8443. \
         If that is your site's new address, set the Website URL in Settings to it."
    );
    assert!(!message.contains("elsewhere") && !message.contains("k=v"));

    // Ten same-site hops: refused as too many, not as off-site.
    let site = std::net::TcpListener::bind("127.0.0.1:0").expect("bind site");
    let port = site.local_addr().expect("addr").port();
    let server = std::thread::spawn(move || serve_http(site, vec![redirect_to("/again"); 10]));
    let err = http
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .await
        .expect_err("too many");
    let heads = server.join().expect("site");
    assert!(
        heads.iter().all(|head| !head.is_empty()),
        "ten hops were sent"
    );
    assert!(err.is_redirect(), "{err:?}");
    assert!(!super::is_off_site_redirect(&err), "{err:?}");
    for field in [
        super::WebsiteUrlField::Settings,
        super::WebsiteUrlField::Above,
    ] {
        assert_eq!(super::off_site_redirect_message(&err, field), None);
    }

    let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = closed.local_addr().expect("addr").port();
    drop(closed);
    let err = http
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .await
        .expect_err("closed port");
    assert!(!super::is_off_site_redirect(&err), "{err:?}");
}

/// Without a known target the advice keeps the "a different address" text.
/// The setup guide says "above": its Website URL field is on the same page.
#[test]
fn redirect_advice_without_a_target_keeps_the_old_text() {
    use super::WebsiteUrlField::{Above, Settings};
    assert_eq!(
        super::client::redirect_off_site_text(None, Settings),
        super::REDIRECT_OFF_SITE
    );
    assert_eq!(
        super::client::redirect_off_site_text(None, Above),
        super::REDIRECT_OFF_SITE_ABOVE
    );
    assert_eq!(
        super::REDIRECT_OFF_SITE_ABOVE,
        "The site redirected to a different address. \
         Set the Website URL above to the address the site uses."
    );
    assert_eq!(
        super::client::redirect_off_site_text(Some("https://www.example.com"), Settings),
        "The site redirected to https://www.example.com. \
         If that is your site's new address, set the Website URL in Settings to it."
    );
    assert_eq!(
        super::client::redirect_off_site_text(Some("https://www.example.com"), Above),
        "The site redirected to https://www.example.com. \
         If that is your site's new address, set the Website URL above to it."
    );
}

/// The origin a redirect message names: no userinfo, path or query, IPv6
/// in brackets, IDN in punycode, a default port left out. An over-long or
/// opaque origin is not named.
#[test]
fn redirect_origin_is_only_scheme_host_and_port() {
    let origin = |raw: &str| super::client::redirect_origin(&reqwest::Url::parse(raw).expect(raw));
    let some = |text: &str| Some(text.to_string());
    assert_eq!(
        origin("https://user:pass@site.example/p?token=x#f"),
        some("https://site.example")
    );
    assert_eq!(origin("http://[::1]:8080/x"), some("http://[::1]:8080"));
    assert_eq!(
        origin("https://bücher.example/"),
        some("https://xn--bcher-kva.example")
    );
    assert_eq!(
        origin("https://site.example:443/"),
        some("https://site.example")
    );
    assert_eq!(
        origin("http://site.example:80/"),
        some("http://site.example")
    );
    assert_eq!(
        origin("https://site.example:8443/"),
        some("https://site.example:8443")
    );
    assert_eq!(origin("data:text/plain,hello"), None);

    let max = super::client::REDIRECT_ORIGIN_MAX_CHARS;
    // "https://" + host == the cap: still named.
    let at_cap = format!(
        "https://{}.example/",
        "a".repeat(max - "https://.example".len())
    );
    assert_eq!(origin(&at_cap).map(|text| text.len()), Some(max));
    let long = format!("https://{}.example/", "a".repeat(60_000));
    assert_eq!(origin(&long), None, "an over-long origin is not named");
}

/// The named messages carry no em or en dash, like the fixed copy.
#[test]
fn redirect_advice_has_no_dash() {
    use super::WebsiteUrlField::{Above, Settings};
    for target in [
        None,
        Some("https://www.example.com"),
        Some("http://[::1]:8080"),
    ] {
        for field in [Settings, Above] {
            let text = super::client::redirect_off_site_text(target, field);
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "{text}"
            );
        }
    }
}
