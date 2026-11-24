//! The consent fetch reads at most `MAX_BODY_BYTES` of body. A declared
//! length over the cap is refused before the body is read, and a body with
//! no length that passes the cap is refused at the cap, without waiting for
//! the rest. The servers here never finish an over-cap body, so a fetch
//! that buffers the whole body before checking its size runs into the
//! deadline and fails.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use super::tests::client;
use super::visibility::MAX_BODY_BYTES;

/// How long a fetch may take here.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a server keeps the connection open after its reply when the
/// client does not hang up. Longer than [`DEADLINE`].
const HOLD: Duration = Duration::from_secs(10);

/// Serve one connection: read the request head, write `reply`, then keep
/// the connection open (a body that is not finished stays unfinished) until
/// the client hangs up or [`HOLD`] passes. Returns the request head.
fn serve_and_hold(reply: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let handle = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        sock.set_read_timeout(Some(Duration::from_secs(2))).ok();
        let mut head = Vec::new();
        let mut buf = [0u8; 4096];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            match sock.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => head.extend_from_slice(&buf[..n]),
            }
        }
        let head = String::from_utf8_lossy(&head).into_owned();
        if sock.write_all(&reply).is_err() {
            return head;
        }
        sock.set_read_timeout(Some(Duration::from_millis(100))).ok();
        let until = Instant::now() + HOLD;
        while Instant::now() < until {
            match sock.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
        }
        head
    });
    (url, handle)
}

/// A 200 JSON reply head with `extra` header lines (each ending `\r\n`).
fn head_with(extra: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{extra}\r\n").into_bytes()
}

/// `{"hero_icon":true}` padded with spaces to `len` bytes: only its size
/// can refuse it.
fn padded_yes(len: usize) -> Vec<u8> {
    let mut body = br#"{"hero_icon":true}"#.to_vec();
    body.resize(len, b' ');
    body
}

/// One chunk of a chunked body.
fn chunk(data: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

/// Fetch the consent from a server that sends `reply`, within [`DEADLINE`].
async fn fetch(reply: Vec<u8>) -> (Option<bool>, std::thread::JoinHandle<String>) {
    let (url, server) = serve_and_hold(reply);
    let got = tokio::time::timeout(DEADLINE, client(&url).fetch_hero_icon_consent())
        .await
        .expect("the fetch answers without waiting for the rest of the body");
    (got, server)
}

/// The control: a chunked body of exactly the cap, finished, is read.
#[tokio::test]
async fn a_chunked_body_at_the_cap_is_read() {
    let mut reply = head_with("Transfer-Encoding: chunked\r\n");
    reply.extend(chunk(&padded_yes(MAX_BODY_BYTES)));
    reply.extend_from_slice(b"0\r\n\r\n");
    let (got, server) = fetch(reply).await;
    assert_eq!(got, Some(true));
    let head = tokio::task::spawn_blocking(move || server.join().expect("server"))
        .await
        .expect("join")
        .to_ascii_lowercase();
    assert!(
        head.starts_with("get /api/training-crops/consent "),
        "{head}"
    );
}

/// A `Content-Length` over the cap is refused from the head alone: the
/// server sends only the first bytes of that body and never the rest.
#[tokio::test]
async fn a_declared_length_over_the_cap_is_refused_before_the_body() {
    for declared in [MAX_BODY_BYTES + 1, 1 << 30] {
        let mut reply = head_with(&format!("Content-Length: {declared}\r\n"));
        reply.extend_from_slice(br#"{"hero_icon":true"#);
        let (got, _server) = fetch(reply).await;
        assert_eq!(got, None, "{declared}");
    }
}

/// A body with no length (chunked, or read until the connection closes)
/// that passes the cap is refused at the cap. The server never finishes
/// it, so only a read that stops at the cap answers in time.
#[tokio::test]
async fn a_body_with_no_length_that_passes_the_cap_is_refused_at_the_cap() {
    let over = padded_yes(MAX_BODY_BYTES + 1);
    let mut chunked = head_with("Transfer-Encoding: chunked\r\n");
    chunked.extend(chunk(&over));
    let mut until_close = head_with("Connection: close\r\n");
    until_close.extend_from_slice(&over);
    for (what, reply) in [("chunked", chunked), ("until close", until_close)] {
        let (got, _server) = fetch(reply).await;
        assert_eq!(got, None, "{what}");
    }
}
