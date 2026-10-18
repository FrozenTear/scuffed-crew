//! WebSocket proxy in front of strfry.
//!
//! Browsers hit this process (host port 7777, mapped to the gate). strfry itself
//! stays on container port 7777, which is reachable on the Compose network as
//! `ws://strfry:7777` and is not published on the host. The site server keeps
//! publishing and subscribing there. Same-origin `/relay` is the gated path.
//!
//! HTTP is never spliced to strfry. A reused connection must not turn a plain
//! GET into a raw pipe that a later WebSocket upgrade can ride. NIP-11 is
//! answered here. Every other non-upgrade request is rejected and the socket
//! is closed.

use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};

use futures_util::sink::Sink;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

use crate::http_gate::{self, HttpClass};
use crate::limits::{self, GateLimits};
use crate::read_policy::ReadDirectory;
use crate::session::{AuthVerifier, GateSession, GiftBudget};

pub async fn serve(
    listen: SocketAddr,
    upstream: String,
    directory: Arc<RwLock<ReadDirectory>>,
    verifier: Arc<dyn AuthVerifier>,
    limits: GateLimits,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(listen).await?;
    tracing::info!(%listen, %upstream, "read gate listening");
    serve_listener(listener, upstream, directory, verifier, limits).await
}

pub async fn serve_listener(
    listener: TcpListener,
    upstream: String,
    directory: Arc<RwLock<ReadDirectory>>,
    verifier: Arc<dyn AuthVerifier>,
    limits: GateLimits,
) -> std::io::Result<()> {
    let gifts = Arc::new(GiftBudget::from_env());
    loop {
        let (stream, peer) = listener.accept().await?;
        let upstream = upstream.clone();
        let directory = Arc::clone(&directory);
        let verifier = Arc::clone(&verifier);
        let limits = limits.clone();
        let gifts = Arc::clone(&gifts);
        tokio::spawn(async move {
            if let Err(err) =
                handle_conn(stream, peer, &upstream, directory, verifier, limits, gifts).await
            {
                tracing::debug!(%peer, error = %err, "read gate connection closed");
            }
        });
    }
}

async fn handle_conn(
    mut client: TcpStream,
    peer: SocketAddr,
    upstream: &str,
    directory: Arc<RwLock<ReadDirectory>>,
    verifier: Arc<dyn AuthVerifier>,
    limits: GateLimits,
    gifts: Arc<GiftBudget>,
) -> Result<(), String> {
    let header = read_headers(&mut client).await?;
    let parsed = http_gate::parse_request(&header);
    let (class, forwarded) = match &parsed {
        Ok(request) => (request.class, request.forwarded_for.as_slice()),
        Err(err) => {
            tracing::debug!(error = %err, "rejected non-upgrade http");
            (HttpClass::Reject, &[][..])
        }
    };
    let client_ip = limits::resolve_client_ip(peer.ip(), forwarded, limits.trusted());
    let _admit = match limits.try_admit(client_ip) {
        Ok(admit) => admit,
        Err(_) => {
            tracing::info!(%client_ip, "rejected connection over the per-ip limit");
            let _ = write_and_close(&mut client, &http_gate::limited_response()).await;
            return Ok(());
        }
    };
    match class {
        HttpClass::Nip11 => {
            let _ = write_and_close(&mut client, &http_gate::nip11_response()).await;
            Ok(())
        }
        HttpClass::Reject => {
            let _ = write_and_close(&mut client, &http_gate::reject_response()).await;
            Ok(())
        }
        HttpClass::WebSocket => {
            let prefixed = Prefixed {
                prefix: header,
                pos: 0,
                inner: client,
            };
            let client_ws = tokio_tungstenite::accept_async(prefixed)
                .await
                .map_err(|e| format!("client websocket: {e}"))?;
            let (upstream_ws, _) = tokio_tungstenite::connect_async(upstream)
                .await
                .map_err(|e| format!("upstream websocket: {e}"))?;
            pump(
                client_ws,
                upstream_ws,
                directory,
                verifier,
                limits,
                gifts,
                client_ip,
            )
            .await
        }
    }
}

async fn write_and_close(client: &mut TcpStream, bytes: &[u8]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    client
        .write_all(bytes)
        .await
        .map_err(|e| format!("write http: {e}"))?;
    client
        .shutdown()
        .await
        .map_err(|e| format!("close http: {e}"))?;
    Ok(())
}

async fn read_headers(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut header = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("read headers: {e}"))?;
        if n == 0 {
            return Err("client closed before headers".into());
        }
        header.extend_from_slice(&buf[..n]);
        if header.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(header);
        }
        if header.len() > 16 * 1024 {
            return Err("headers too large".into());
        }
    }
}

fn random_challenge() -> String {
    let mut bytes = [0u8; 16];
    let mut file = std::fs::File::open("/dev/urandom").expect("urandom");
    std::io::Read::read_exact(&mut file, &mut bytes).expect("urandom");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn pump<C, U>(
    client_ws: tokio_tungstenite::WebSocketStream<C>,
    upstream_ws: tokio_tungstenite::WebSocketStream<U>,
    directory: Arc<RwLock<ReadDirectory>>,
    verifier: Arc<dyn AuthVerifier>,
    limits: GateLimits,
    gifts: Arc<GiftBudget>,
    client_ip: IpAddr,
) -> Result<(), String>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: AsyncRead + AsyncWrite + Unpin,
{
    let (mut client_tx, mut client_rx) = client_ws.split();
    let (mut upstream_tx, mut upstream_rx) = upstream_ws.split();
    let mut session = GateSession::new(random_challenge());
    client_tx
        .send(Message::Text(session.challenge_message().into()))
        .await
        .map_err(|e| format!("send challenge: {e}"))?;

    loop {
        tokio::select! {
            client_msg = client_rx.next() => {
                let Some(msg) = client_msg else { break };
                let msg = msg.map_err(|e| format!("client frame: {e}"))?;
                if counts_toward_message_limit(&msg) && !limits.allow_message(client_ip) {
                    tracing::info!(%client_ip, "closing connection over the per-ip message limit");
                    let _ = client_tx
                        .send(Message::Text(
                            r#"["NOTICE","rate-limited: too many messages"]"#.into(),
                        ))
                        .await;
                    break;
                }
                if !forward_client(msg, &mut session, verifier.as_ref(), &directory, &gifts, &mut client_tx, &mut upstream_tx).await? {
                    break;
                }
            }
            relay_msg = upstream_rx.next() => {
                let Some(msg) = relay_msg else { break };
                let msg = msg.map_err(|e| format!("upstream frame: {e}"))?;
                if !forward_relay(msg, &session, &directory, &mut client_tx, &mut upstream_tx).await? {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn counts_toward_message_limit(msg: &Message) -> bool {
    matches!(
        msg,
        Message::Text(_) | Message::Binary(_) | Message::Ping(_)
    )
}

async fn forward_client<C, U>(
    msg: Message,
    session: &mut GateSession,
    verifier: &dyn AuthVerifier,
    directory: &RwLock<ReadDirectory>,
    gifts: &GiftBudget,
    client_tx: &mut C,
    upstream_tx: &mut U,
) -> Result<bool, String>
where
    C: Sink<Message> + Unpin,
    U: Sink<Message> + Unpin,
    <C as Sink<Message>>::Error: std::fmt::Display,
    <U as Sink<Message>>::Error: std::fmt::Display,
{
    match msg {
        Message::Text(text) => {
            let outcome = {
                let dir = directory.read().unwrap_or_else(|err| err.into_inner());
                session.on_client_text(&text, verifier, &dir, gifts)
            };
            for reply in outcome.to_client {
                client_tx
                    .send(Message::Text(reply.into()))
                    .await
                    .map_err(|e| format!("reply: {e}"))?;
            }
            for forward in outcome.to_relay {
                upstream_tx
                    .send(Message::Text(forward.into()))
                    .await
                    .map_err(|e| format!("forward client: {e}"))?;
            }
            Ok(true)
        }
        Message::Ping(payload) => {
            client_tx
                .send(Message::Pong(payload))
                .await
                .map_err(|e| format!("pong: {e}"))?;
            Ok(true)
        }
        Message::Pong(_) => Ok(true),
        Message::Close(_) => Ok(false),
        Message::Binary(_) => {
            // Negentropy and other binary frames would skip the text filter.
            tracing::debug!("dropped binary client frame");
            Ok(true)
        }
        Message::Frame(_) => Ok(true),
    }
}

async fn forward_relay<C, U>(
    msg: Message,
    session: &GateSession,
    directory: &RwLock<ReadDirectory>,
    client_tx: &mut C,
    upstream_tx: &mut U,
) -> Result<bool, String>
where
    C: Sink<Message> + Unpin,
    U: Sink<Message> + Unpin,
    <C as Sink<Message>>::Error: std::fmt::Display,
    <U as Sink<Message>>::Error: std::fmt::Display,
{
    match msg {
        Message::Text(text) => {
            let forward = {
                let dir = directory.read().unwrap_or_else(|err| err.into_inner());
                session.on_relay_text(&text, &dir)
            };
            if let Some(forward) = forward {
                client_tx
                    .send(Message::Text(forward.into()))
                    .await
                    .map_err(|e| format!("forward relay: {e}"))?;
            }
            Ok(true)
        }
        Message::Ping(payload) => {
            upstream_tx
                .send(Message::Pong(payload))
                .await
                .map_err(|e| format!("upstream pong: {e}"))?;
            Ok(true)
        }
        Message::Pong(_) => Ok(true),
        Message::Close(_) => Ok(false),
        Message::Binary(_) => {
            tracing::debug!("dropped binary relay frame");
            Ok(true)
        }
        Message::Frame(_) => Ok(true),
    }
}

struct Prefixed<S> {
    prefix: Vec<u8>,
    pos: usize,
    inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for Prefixed<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.pos < self.prefix.len() {
            let n = std::cmp::min(buf.remaining(), self.prefix.len() - self.pos);
            buf.put_slice(&self.prefix[self.pos..self.pos + n]);
            self.pos += n;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Prefixed<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::GateLimits;
    use crate::read_policy::{GroupAcl, MemberAcl, ReadDirectory, GIFT_WRAP_KIND};
    use crate::session::AuthVerifier;
    use serde_json::{json, Value};
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct PubkeyVerifier;

    impl AuthVerifier for PubkeyVerifier {
        fn verify_auth(&self, event: &Value, challenge: &str) -> Result<String, String> {
            let ok = event
                .get("tags")
                .and_then(|t| t.as_array())
                .is_some_and(|tags| {
                    tags.iter().any(|tag| {
                        tag.get(0).and_then(|v| v.as_str()) == Some("challenge")
                            && tag.get(1).and_then(|v| v.as_str()) == Some(challenge)
                    })
                });
            if !ok {
                return Err("challenge mismatch".into());
            }
            event
                .get("pubkey")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| "missing pubkey".into())
        }
    }

    fn pk(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    async fn read_text<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>) -> Message
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timeout")
            .expect("closed")
            .expect("frame")
    }

    fn header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
    }

    async fn read_http_response(stream: &mut TcpStream) -> (String, Vec<u8>) {
        let mut collected = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
                .await
                .expect("response timeout")
                .expect("read");
            assert!(n > 0, "connection closed before headers");
            collected.extend_from_slice(&buf[..n]);
            if let Some(end) = header_end(&collected) {
                let headers = String::from_utf8_lossy(&collected[..end]).into_owned();
                let rest = collected[end..].to_vec();
                return (headers, rest);
            }
            assert!(collected.len() < 16 * 1024, "headers never ended");
        }
    }

    fn text_of(msg: &Message) -> String {
        match msg {
            Message::Text(t) => t.to_string(),
            other => panic!("expected text, got {other:?}"),
        }
    }

    async fn spawn_upstream(send_on_req: Vec<Value>) -> (String, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen_task = Arc::clone(&seen);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(msg)) = ws.next().await {
                let Message::Text(text) = msg else { continue };
                seen_task.lock().unwrap().push(text.to_string());
                let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if value.get(0).and_then(|v| v.as_str()) == Some("REQ") {
                    let sub = value.get(1).and_then(|v| v.as_str()).unwrap_or("s");
                    for mut frame in send_on_req.clone() {
                        if let Some(slot) = frame.get_mut(1) {
                            *slot = json!(sub);
                        }
                        ws.send(Message::Text(frame.to_string().into()))
                            .await
                            .unwrap();
                    }
                    ws.send(Message::Text(json!(["EOSE", sub]).to_string().into()))
                        .await
                        .unwrap();
                }
            }
        });
        (format!("ws://{addr}"), seen)
    }

    async fn spawn_gate(upstream: String, directory: ReadDirectory) -> std::net::SocketAddr {
        spawn_gate_with(upstream, directory, GateLimits::generous()).await
    }

    async fn spawn_gate_with(
        upstream: String,
        directory: ReadDirectory,
        limits: GateLimits,
    ) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let directory = Arc::new(RwLock::new(directory));
        let verifier: Arc<dyn AuthVerifier> = Arc::new(PubkeyVerifier);
        tokio::spawn(async move {
            let _ = serve_listener(listener, upstream, directory, verifier, limits).await;
        });
        addr
    }

    /// Completes WebSocket handshakes and holds them open. Connection-cap tests
    /// need the gate to finish `accept_async` and stay alive so the slot is held.
    async fn spawn_holding_upstream() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    while ws.next().await.is_some() {}
                });
            }
        });
        format!("ws://{addr}")
    }

    /// Accepts TCP and records how many connections arrived and the first bytes.
    async fn spawn_byte_sink() -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<u8>>>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits_task = Arc::clone(&hits);
        let bytes_task = Arc::clone(&bytes);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                hits_task.fetch_add(1, Ordering::SeqCst);
                let mut buf = vec![0u8; 4096];
                if let Ok(n) = stream.read(&mut buf).await {
                    bytes_task.lock().unwrap().extend_from_slice(&buf[..n]);
                }
            }
        });
        (format!("ws://{addr}"), hits, bytes)
    }

    async fn exchange(addr: SocketAddr, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
            .await
            .expect("response timeout")
            .unwrap();
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    #[tokio::test]
    async fn signed_out_req_gets_no_events() {
        let leak = json!([
            "EVENT",
            "s",
            {"id":"1","pubkey":"aa","kind":1,"tags":[],"content":"secret-leak","sig":"s","created_at":1}
        ]);
        let (upstream, _) = spawn_upstream(vec![leak]).await;
        let gate = spawn_gate(upstream, ReadDirectory::default()).await;
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{gate}"))
            .await
            .unwrap();
        let challenge = text_of(&read_text(&mut ws).await);
        assert!(challenge.contains("AUTH"), "{challenge}");
        ws.send(Message::Text(
            r#"["REQ","sub",{"kinds":[1]}]"#.to_string().into(),
        ))
        .await
        .unwrap();
        let reply = text_of(&read_text(&mut ws).await);
        assert!(reply.contains("CLOSED"), "{reply}");
        assert!(reply.contains("auth-required"), "{reply}");
        assert!(!reply.contains("secret-leak"), "{reply}");
    }

    #[tokio::test]
    async fn authed_member_receives_own_gift_wrap_and_not_other_team_chat() {
        let me = pk(0x11);
        let directory = ReadDirectory::from_parts(
            [MemberAcl {
                pubkey: me.clone(),
                member_id: "m".into(),
                officer: false,
                teams: HashSet::from(["team-a".into()]),
            }],
            [(
                "team-a".into(),
                GroupAcl {
                    team_id: "team-a".into(),
                    officer_only: false,
                },
            )],
        );
        let gift = json!([
            "EVENT",
            "s",
            {"id":"g","pubkey":"ee","kind":GIFT_WRAP_KIND,"tags":[["p", me]],"content":"dm","sig":"s","created_at":1}
        ]);
        let other = json!([
            "EVENT",
            "s",
            {"id":"t","pubkey":"ee","kind":9,"tags":[["h","team-b"]],"content":"other-team-secret","sig":"s","created_at":1}
        ]);
        let (upstream, _) = spawn_upstream(vec![gift, other]).await;
        let gate = spawn_gate(upstream, directory).await;
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{gate}"))
            .await
            .unwrap();
        let challenge_frame: Value =
            serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
        let challenge = challenge_frame[1].as_str().unwrap();
        let auth = json!([
            "AUTH",
            {"id":"auth1","pubkey": me, "kind": 22242, "tags":[["challenge", challenge]]}
        ]);
        ws.send(Message::Text(auth.to_string().into()))
            .await
            .unwrap();
        let ok: Value = serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
        assert_eq!(ok[0], "OK");
        assert_eq!(ok[2], true);

        ws.send(Message::Text(
            r#"["REQ","feed",{"kinds":[1059,9]}]"#.to_string().into(),
        ))
        .await
        .unwrap();

        let mut saw_gift = false;
        let mut saw_eose = false;
        while !saw_eose {
            let frame: Value = serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
            match frame[0].as_str() {
                Some("EVENT") => {
                    assert_eq!(frame[2]["kind"], GIFT_WRAP_KIND);
                    assert!(!frame.to_string().contains("other-team-secret"));
                    saw_gift = true;
                }
                Some("EOSE") => saw_eose = true,
                other => panic!("unexpected {other:?} in {frame}"),
            }
        }
        assert!(saw_gift);
    }

    #[tokio::test]
    async fn plain_request_then_upgrade_on_same_connection_never_reaches_strfry() {
        let (upstream, hits, bytes) = spawn_byte_sink().await;
        let gate = spawn_gate(upstream, ReadDirectory::default()).await;
        let mut stream = TcpStream::connect(gate).await.unwrap();
        // Pipelined on one TCP connection, the way a reused Caddy connection
        // would deliver a later visitor's upgrade after a plain GET.
        let pipelined = b"\
GET / HTTP/1.1\r\n\
Host: relay.example\r\n\
\r\n\
GET / HTTP/1.1\r\n\
Host: relay.example\r\n\
Connection: Upgrade\r\n\
Upgrade: websocket\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n";
        stream.write_all(pipelined).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
            .await
            .expect("response timeout")
            .unwrap();
        let resp = String::from_utf8_lossy(&buf[..n]);
        assert!(resp.starts_with("HTTP/1.1 400"), "{resp}");
        assert!(
            resp.to_ascii_lowercase().contains("connection: close"),
            "{resp}"
        );
        assert!(!resp.contains("101"), "{resp}");
        let extra = tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf)).await;
        if let Ok(Ok(n)) = extra {
            let more = String::from_utf8_lossy(&buf[..n]);
            assert!(!more.contains("101"), "{more}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "strfry accepted a raw connection: {}",
            String::from_utf8_lossy(&bytes.lock().unwrap())
        );
        assert!(bytes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn nip11_is_answered_by_the_gate_and_does_not_dial_strfry() {
        let (upstream, hits, _) = spawn_byte_sink().await;
        let gate = spawn_gate(upstream, ReadDirectory::default()).await;
        let resp = exchange(
            gate,
            b"GET / HTTP/1.1\r\nHost: relay.example\r\nAccept: application/nostr+json\r\n\r\n",
        )
        .await;
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
        assert!(resp.contains("application/nostr+json"), "{resp}");
        assert!(resp.contains("auth_required"), "{resp}");
        assert!(
            resp.to_ascii_lowercase().contains("connection: close"),
            "{resp}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn upgrade_without_space_requires_auth_before_req_reaches_strfry() {
        let (upstream, seen) = spawn_upstream(vec![]).await;
        let gate = spawn_gate(upstream, ReadDirectory::default()).await;
        let mut stream = TcpStream::connect(gate).await.unwrap();
        let req = "\
GET / HTTP/1.1\r\n\
Host: relay.example\r\n\
Connection:Upgrade\r\n\
Upgrade:websocket\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n";
        assert!(
            !req.to_ascii_lowercase().contains("upgrade: websocket"),
            "header has no space after the colon"
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let (headers, rest) = read_http_response(&mut stream).await;
        assert!(headers.starts_with("HTTP/1.1 101"), "{headers}");
        let prefixed = Prefixed {
            prefix: rest,
            pos: 0,
            inner: stream,
        };
        let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            prefixed,
            tokio_tungstenite::tungstenite::protocol::Role::Client,
            None,
        )
        .await;
        let challenge = text_of(&read_text(&mut ws).await);
        assert!(challenge.contains("AUTH"), "{challenge}");
        ws.send(Message::Text(
            r#"["REQ","sub",{"kinds":[1]}]"#.to_string().into(),
        ))
        .await
        .unwrap();
        let reply = text_of(&read_text(&mut ws).await);
        assert!(reply.contains("CLOSED"), "{reply}");
        assert!(reply.contains("auth-required"), "{reply}");
        tokio::time::sleep(Duration::from_millis(50)).await;
        let forwarded = seen.lock().unwrap().clone();
        assert!(
            forwarded.iter().all(|frame| !frame.contains("REQ")),
            "unauthenticated REQ reached strfry: {forwarded:?}"
        );
    }

    #[tokio::test]
    async fn group_metadata_listing_loads_for_an_authed_member() {
        // Same filter as NostrFilter::group_metadata, which the app uses to
        // list a group. GroupManager::create_group in
        // crates/chat/src/nostr/groups.rs publishes kind 39000 with a `d` tag
        // and no `h` tag (EventBuilder::build_group_metadata).
        let me = pk(0x11);
        let directory = ReadDirectory::from_parts(
            [MemberAcl {
                pubkey: me.clone(),
                member_id: "m".into(),
                officer: false,
                teams: HashSet::from(["team-a".into()]),
            }],
            [
                (
                    "team-a".into(),
                    GroupAcl {
                        team_id: "team-a".into(),
                        officer_only: false,
                    },
                ),
                (
                    "team-a-officers".into(),
                    GroupAcl {
                        team_id: "team-a".into(),
                        officer_only: true,
                    },
                ),
                (
                    "team-b".into(),
                    GroupAcl {
                        team_id: "team-b".into(),
                        officer_only: false,
                    },
                ),
            ],
        );
        let frames = vec![
            meta_event(39000, "team-a", "alpha-name"),
            meta_event(39001, "team-a", "alpha-admins"),
            meta_event(39002, "team-a", "alpha-members"),
            meta_event(39000, "team-b", "other-group-secret"),
            meta_event(39002, "team-a-officers", "officer-roster-secret"),
        ];
        let (upstream, seen) = spawn_upstream(frames).await;
        let gate = spawn_gate(upstream, directory).await;
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{gate}"))
            .await
            .unwrap();
        let challenge_frame: Value =
            serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
        let challenge = challenge_frame[1].as_str().unwrap();
        ws.send(Message::Text(
            json!([
                "AUTH",
                {"id":"auth1","pubkey": me, "kind": 22242, "tags":[["challenge", challenge]]}
            ])
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
        let ok: Value = serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
        assert_eq!(ok[2], true, "{ok}");

        let listing = json!([
            "REQ",
            "meta",
            {"kinds":[39000, 39001, 39002], "#d":["team-a"]}
        ]);
        ws.send(Message::Text(listing.to_string().into()))
            .await
            .unwrap();

        let mut kinds = Vec::new();
        let mut saw_eose = false;
        while !saw_eose {
            let frame: Value = serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
            match frame[0].as_str() {
                Some("EVENT") => {
                    let text = frame.to_string();
                    assert!(!text.contains("other-group-secret"), "{text}");
                    assert!(!text.contains("officer-roster-secret"), "{text}");
                    assert_eq!(frame[2]["tags"][0][1], "team-a");
                    kinds.push(frame[2]["kind"].as_u64().unwrap());
                }
                Some("EOSE") => saw_eose = true,
                other => panic!("unexpected {other:?} in {frame}"),
            }
        }
        kinds.sort();
        assert_eq!(kinds, vec![39000, 39001, 39002]);
        let forwarded = seen.lock().unwrap().clone();
        assert!(
            forwarded.iter().any(|frame| frame.contains("meta")),
            "group listing REQ was not forwarded to strfry: {forwarded:?}"
        );
    }

    #[tokio::test]
    async fn non_member_does_not_receive_other_group_or_officer_metadata() {
        // Broad REQ of kinds 39000-39003, with no `#d` filter. A gate that
        // handed metadata to every signed-in member would deliver team-a and
        // the officer group. This client is a member of team-b only.
        let outsider = pk(0x33);
        let directory = ReadDirectory::from_parts(
            [MemberAcl {
                pubkey: outsider.clone(),
                member_id: "outsider".into(),
                officer: true,
                teams: HashSet::from(["team-b".into()]),
            }],
            [
                (
                    "team-a".into(),
                    GroupAcl {
                        team_id: "team-a".into(),
                        officer_only: false,
                    },
                ),
                (
                    "team-a-officers".into(),
                    GroupAcl {
                        team_id: "team-a".into(),
                        officer_only: true,
                    },
                ),
                (
                    "team-b".into(),
                    GroupAcl {
                        team_id: "team-b".into(),
                        officer_only: false,
                    },
                ),
            ],
        );
        let frames = vec![
            meta_event(39000, "team-a", "alpha-meta"),
            meta_event(39001, "team-a", "alpha-admins"),
            meta_event(39002, "team-a", "alpha-members"),
            meta_event(39003, "team-a", "alpha-roles"),
            meta_event(39000, "team-a-officers", "officer-meta"),
            meta_event(39001, "team-a-officers", "officer-admins"),
            meta_event(39002, "team-a-officers", "officer-roster"),
            meta_event(39003, "team-a-officers", "officer-roles"),
            meta_event(39000, "team-b", "bravo-meta"),
        ];
        let (upstream, _) = spawn_upstream(frames).await;
        let gate = spawn_gate(upstream, directory).await;
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{gate}"))
            .await
            .unwrap();
        let challenge_frame: Value =
            serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
        let challenge = challenge_frame[1].as_str().unwrap();
        ws.send(Message::Text(
            json!([
                "AUTH",
                {"id":"auth1","pubkey": outsider, "kind": 22242, "tags":[["challenge", challenge]]}
            ])
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
        let ok: Value = serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
        assert_eq!(ok[2], true, "{ok}");

        ws.send(Message::Text(
            json!(["REQ", "meta", {"kinds": [39000, 39001, 39002, 39003]}])
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

        let mut saw = Vec::new();
        let mut saw_eose = false;
        while !saw_eose {
            let frame: Value = serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
            match frame[0].as_str() {
                Some("EVENT") => {
                    let text = frame.to_string();
                    assert!(!text.contains("alpha-"), "{text}");
                    assert!(!text.contains("officer-"), "{text}");
                    assert!(!text.contains("team-a"), "{text}");
                    saw.push(text);
                }
                Some("EOSE") => saw_eose = true,
                other => panic!("unexpected {other:?} in {frame}"),
            }
        }
        assert_eq!(saw.len(), 1, "{saw:?}");
        assert!(saw[0].contains("bravo-meta"), "{saw:?}");
        assert!(saw[0].contains("team-b"), "{saw:?}");
    }

    fn meta_event(kind: u64, group: &str, marker: &str) -> Value {
        json!([
            "EVENT",
            "s",
            {
                "id": format!("m{kind}{group}"),
                "pubkey": "ee",
                "kind": kind,
                "tags": [["d", group], ["name", marker]],
                "content": "",
                "sig": "s",
                "created_at": 1
            }
        ])
    }

    async fn send_upgrade(addr: SocketAddr, forwarded: Option<&str>) -> (String, TcpStream) {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let xff = match forwarded {
            Some(ip) => format!("X-Forwarded-For: {ip}\r\n"),
            None => String::new(),
        };
        let req = format!(
            "GET / HTTP/1.1\r\nHost: relay.example\r\n{xff}Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let (headers, _) = read_http_response(&mut stream).await;
        (headers, stream)
    }

    #[tokio::test]
    async fn connection_cap_follows_forwarded_for_from_a_trusted_proxy() {
        let upstream = spawn_holding_upstream().await;
        let limits = GateLimits::build(
            1,
            100,
            100,
            Duration::from_secs(60),
            crate::limits::loopback_nets(),
        );
        let gate = spawn_gate_with(upstream, ReadDirectory::default(), limits).await;
        let (first, _hold_a) = send_upgrade(gate, Some("203.0.113.5")).await;
        assert!(first.starts_with("HTTP/1.1 101"), "{first}");
        let (second, _hold_b) = send_upgrade(gate, Some("203.0.113.6")).await;
        assert!(second.starts_with("HTTP/1.1 101"), "{second}");
        let (third, _) = send_upgrade(gate, Some("203.0.113.5")).await;
        assert!(third.starts_with("HTTP/1.1 429"), "{third}");
        assert!(
            third.to_ascii_lowercase().contains("connection: close"),
            "{third}"
        );
    }

    #[tokio::test]
    async fn forwarded_for_from_an_untrusted_peer_does_not_split_the_cap() {
        let upstream = spawn_holding_upstream().await;
        let trusted = vec!["192.0.2.0/24".parse().unwrap()];
        let limits = GateLimits::build(1, 100, 100, Duration::from_secs(60), trusted);
        let gate = spawn_gate_with(upstream, ReadDirectory::default(), limits).await;
        let (first, _hold) = send_upgrade(gate, Some("203.0.113.5")).await;
        assert!(first.starts_with("HTTP/1.1 101"), "{first}");
        let (second, _) = send_upgrade(gate, Some("198.51.100.8")).await;
        assert!(
            second.starts_with("HTTP/1.1 429"),
            "spoofed X-Forwarded-For must not open a second slot: {second}"
        );
    }

    #[tokio::test]
    async fn message_rate_stops_forwarding_before_strfry() {
        let (upstream, seen) = spawn_upstream(vec![]).await;
        let limits = GateLimits::build(
            10,
            100,
            2,
            Duration::from_secs(60),
            crate::limits::loopback_nets(),
        );
        let gate = spawn_gate_with(upstream, ReadDirectory::default(), limits).await;
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{gate}"))
            .await
            .unwrap();
        let challenge_frame: Value =
            serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
        let challenge = challenge_frame[1].as_str().unwrap();
        let me = pk(0x11);
        ws.send(Message::Text(
            json!([
                "AUTH",
                {"id":"auth1","pubkey": me, "kind": 22242, "tags":[["challenge", challenge]]}
            ])
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
        let ok: Value = serde_json::from_str(&text_of(&read_text(&mut ws).await)).unwrap();
        assert_eq!(ok[2], true, "{ok}");
        ws.send(Message::Text(
            r#"["REQ","one",{"kinds":[1]}]"#.to_string().into(),
        ))
        .await
        .unwrap();
        ws.send(Message::Text(
            r#"["REQ","two",{"kinds":[1]}]"#.to_string().into(),
        ))
        .await
        .unwrap();
        let mut notice = String::new();
        for _ in 0..4 {
            notice = text_of(&read_text(&mut ws).await);
            if notice.contains("rate-limited") {
                break;
            }
        }
        assert!(notice.contains("rate-limited"), "{notice}");
        tokio::time::sleep(Duration::from_millis(80)).await;
        let forwarded = seen.lock().unwrap().clone();
        let reqs: Vec<_> = forwarded
            .iter()
            .filter(|frame| frame.contains("REQ"))
            .cloned()
            .collect();
        assert_eq!(reqs.len(), 1, "{forwarded:?}");
        assert!(reqs[0].contains("\"one\""), "{reqs:?}");
    }
}
