//! The stats visibility read: body parsing and the loopback fetch.

use std::io::{Read, Write};

use super::tests::client;
use super::*;

#[test]
fn each_known_value_parses() {
    for (raw, want) in [
        ("public", StatsVisibility::Public),
        ("members", StatsVisibility::Members),
        ("private", StatsVisibility::Private),
    ] {
        let body = format!(r#"{{"player_name":"FrozenTear","stats_visibility":"{raw}"}}"#);
        assert_eq!(parse_stats_visibility(200, &body), Some(want), "{raw}");
    }
}

#[test]
fn an_unknown_or_missing_value_is_no_answer() {
    for body in [
        r#"{"stats_visibility":"friends"}"#,
        r#"{"stats_visibility":"Public"}"#,
        r#"{"stats_visibility":" public"}"#,
        r#"{"stats_visibility":""}"#,
        r#"{"stats_visibility":null}"#,
        r#"{"stats_visibility":1}"#,
        r#"{"stats_visibility":["public"]}"#,
        r#"{"player_name":"FrozenTear"}"#,
        r#"{}"#,
        r#"["public"]"#,
        r#""public""#,
    ] {
        assert_eq!(parse_stats_visibility(200, body), None, "{body}");
    }
}

#[test]
fn an_error_status_or_bad_body_is_no_answer() {
    let ok = r#"{"stats_visibility":"public"}"#;
    for status in [301, 400, 401, 403, 404, 429, 500, 502, 503] {
        assert_eq!(parse_stats_visibility(status, ok), None, "{status}");
    }
    for body in ["", "not json", "{", "<html>public</html>", "\u{0}"] {
        assert_eq!(parse_stats_visibility(200, body), None, "{body:?}");
    }
    assert_eq!(
        parse_stats_visibility(204, ok),
        Some(StatsVisibility::Public)
    );
}

#[test]
fn an_oversized_body_is_no_answer() {
    let pad = "x".repeat(visibility::MAX_BODY_BYTES);
    let body = format!(r#"{{"stats_visibility":"public","pad":"{pad}"}}"#);
    assert_eq!(parse_stats_visibility(200, &body), None);
}

/// Serve `response` once and return the request head.
fn serve_once(response: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let handle = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .ok();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        while !got.windows(4).any(|w| w == b"\r\n\r\n") {
            match sock.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
        let _ = sock.write_all(&response);
        String::from_utf8_lossy(&got).into_owned()
    });
    (url, handle)
}

fn json_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[tokio::test]
async fn fetch_sends_the_token_to_the_daemon_config_route_and_reads_the_value() {
    let (url, server) = serve_once(json_response(
        "200 OK",
        r#"{"player_name":null,"stats_visibility":"members"}"#,
    ));
    let got = client(&url).fetch_stats_visibility().await;
    let head = server.join().expect("server").to_ascii_lowercase();
    assert_eq!(got, Some(StatsVisibility::Members));
    assert!(
        head.starts_with("get /api/stats/daemon-config "),
        "{head:?}"
    );
    assert!(
        head.contains("authorization: bearer super-secret-token"),
        "{head:?}"
    );
}

#[tokio::test]
async fn fetch_reads_each_value_from_daemon_config() {
    for (raw, want) in [
        ("public", StatsVisibility::Public),
        ("members", StatsVisibility::Members),
        ("private", StatsVisibility::Private),
    ] {
        let body = format!(r#"{{"player_name":"FrozenTear","stats_visibility":"{raw}"}}"#);
        let (url, server) = serve_once(json_response("200 OK", &body));
        assert_eq!(client(&url).fetch_stats_visibility().await, Some(want));
        server.join().expect("server");
    }
}

#[tokio::test]
async fn fetch_failures_are_no_answer() {
    for (status, body) in [
        ("401 Unauthorized", r#"{"error":"Unauthorized"}"#),
        ("403 Forbidden", r#"{"error":"Not an org member"}"#),
        ("500 Internal Server Error", r#"{"error":"Internal error"}"#),
        ("200 OK", "not json"),
        ("200 OK", r#"{"stats_visibility":"friends"}"#),
        ("200 OK", r#"{"player_name":"FrozenTear"}"#),
        (
            "200 OK",
            r#"{"player_name":"FrozenTear","stats_visibility":null}"#,
        ),
        ("200 OK", r#"{"player_name":null}"#),
    ] {
        let (url, server) = serve_once(json_response(status, body));
        assert_eq!(
            client(&url).fetch_stats_visibility().await,
            None,
            "{status} {body}"
        );
        server.join().expect("server");
    }
}

#[tokio::test]
async fn fetch_with_nothing_listening_is_no_answer() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    assert_eq!(
        client(&format!("http://127.0.0.1:{port}"))
            .fetch_stats_visibility()
            .await,
        None
    );
}

#[tokio::test]
async fn fetch_refuses_cleartext_to_another_host_without_sending() {
    let started = std::time::Instant::now();
    let got = client("http://203.0.113.10").fetch_stats_visibility().await;
    assert_eq!(got, None);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}
