//! WebSocket proxy in front of strfry.
//!
//! Browsers hit this process (host port 7777, mapped to the gate). strfry itself
//! stays on container port 7777, which is reachable on the Compose network as
//! `ws://strfry:7777` and is not published on the host. The site server keeps
//! publishing and subscribing there. Same-origin `/relay` is the gated path.

use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};

use futures_util::sink::Sink;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

use crate::read_policy::ReadDirectory;
use crate::session::{AuthVerifier, GateSession};

pub async fn serve(
    listen: std::net::SocketAddr,
    upstream: String,
    directory: Arc<RwLock<ReadDirectory>>,
    verifier: Arc<dyn AuthVerifier>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(listen).await?;
    tracing::info!(%listen, %upstream, "read gate listening");
    serve_listener(listener, upstream, directory, verifier).await
}

pub async fn serve_listener(
    listener: TcpListener,
    upstream: String,
    directory: Arc<RwLock<ReadDirectory>>,
    verifier: Arc<dyn AuthVerifier>,
) -> std::io::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let upstream = upstream.clone();
        let directory = Arc::clone(&directory);
        let verifier = Arc::clone(&verifier);
        tokio::spawn(async move {
            if let Err(err) = handle_conn(stream, &upstream, directory, verifier).await {
                tracing::debug!(%peer, error = %err, "read gate connection closed");
            }
        });
    }
}

async fn handle_conn(
    mut client: TcpStream,
    upstream: &str,
    directory: Arc<RwLock<ReadDirectory>>,
    verifier: Arc<dyn AuthVerifier>,
) -> Result<(), String> {
    let header = read_headers(&mut client).await?;
    let head = String::from_utf8_lossy(&header).to_ascii_lowercase();
    if !head.contains("upgrade: websocket") {
        proxy_http(&header, client, upstream).await?;
        return Ok(());
    }
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
    pump(client_ws, upstream_ws, directory, verifier).await
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

async fn proxy_http(header: &[u8], mut client: TcpStream, upstream: &str) -> Result<(), String> {
    use tokio::io::{copy_bidirectional, AsyncWriteExt};
    let addr = tcp_target(upstream)?;
    let mut upstream = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("upstream http {addr}: {e}"))?;
    upstream
        .write_all(header)
        .await
        .map_err(|e| format!("write http request: {e}"))?;
    copy_bidirectional(&mut client, &mut upstream)
        .await
        .map_err(|e| format!("http proxy: {e}"))?;
    Ok(())
}

pub fn tcp_target(upstream: &str) -> Result<String, String> {
    let without = upstream
        .strip_prefix("ws://")
        .or_else(|| upstream.strip_prefix("http://"))
        .ok_or_else(|| format!("upstream must be ws://, got {upstream}"))?;
    let hostport = without.split(['/', '?', '#']).next().unwrap_or(without);
    if hostport.is_empty() {
        return Err("upstream host is empty".into());
    }
    if hostport.contains(':') {
        Ok(hostport.to_string())
    } else {
        Ok(format!("{hostport}:80"))
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
                if !forward_client(msg, &mut session, verifier.as_ref(), &mut client_tx, &mut upstream_tx).await? {
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

async fn forward_client<C, U>(
    msg: Message,
    session: &mut GateSession,
    verifier: &dyn AuthVerifier,
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
            let outcome = session.on_client_text(&text, verifier);
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
    use crate::read_policy::{GroupAcl, MemberAcl, ReadDirectory, GIFT_WRAP_KIND};
    use crate::session::AuthVerifier;
    use serde_json::{json, Value};
    use std::collections::HashSet;
    use std::time::Duration;

    #[test]
    fn tcp_target_parses_ws_url() {
        assert_eq!(tcp_target("ws://127.0.0.1:7777").unwrap(), "127.0.0.1:7777");
        assert_eq!(tcp_target("ws://strfry:7777/path").unwrap(), "strfry:7777");
        assert!(tcp_target("wss://relay.example").is_err());
    }

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

    async fn read_text(
        ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
    ) -> Message {
        tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timeout")
            .expect("closed")
            .expect("frame")
    }

    fn text_of(msg: &Message) -> String {
        match msg {
            Message::Text(t) => t.to_string(),
            other => panic!("expected text, got {other:?}"),
        }
    }

    async fn spawn_upstream(send_on_req: Vec<Value>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(msg)) = ws.next().await {
                let Message::Text(text) = msg else { continue };
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
        format!("ws://{addr}")
    }

    async fn spawn_gate(upstream: String, directory: ReadDirectory) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let directory = Arc::new(RwLock::new(directory));
        let verifier: Arc<dyn AuthVerifier> = Arc::new(PubkeyVerifier);
        tokio::spawn(async move {
            let _ = serve_listener(listener, upstream, directory, verifier).await;
        });
        addr
    }

    #[tokio::test]
    async fn signed_out_req_gets_no_events() {
        let leak = json!([
            "EVENT",
            "s",
            {"id":"1","pubkey":"aa","kind":1,"tags":[],"content":"secret-leak","sig":"s","created_at":1}
        ]);
        let upstream = spawn_upstream(vec![leak]).await;
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
        let upstream = spawn_upstream(vec![gift, other]).await;
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
}
