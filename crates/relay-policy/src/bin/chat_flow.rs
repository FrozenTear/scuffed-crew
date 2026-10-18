//! End-to-end check for the relay image.
//!
//! `seed` writes one active member, team, roster edge, and public team channel.
//! `check` then does what the site chat does:
//! 1. NIP-42 AUTH on the read gate.
//! 2. REQ group metadata with the same filter as `NostrFilter::group_metadata`.
//!    The app helper that sends this filter has no caller. An empty EOSE is
//!    expected. The group id is the team channel id, which is what
//!    `GET /api/teams/:id/channels` returns.
//! 3. REQ kind 9 with `#h` set to that id (`NostrFilter::group_chat`, limit 50).
//! 4. Publish one kind 9 with a single `h` tag straight to strfry, the same
//!    shape as `POST /api/chat/send-public`.
//! 5. Read that event back on the gate subscription.
//! 6. Refuse a pubkey that is not on the allowlist, a kind that is not
//!    allowed, and the next event once the rate limit is already full.
//!
//! The container must be started with `RELAY_POLICY_RATE_LIMIT=1` so step 6
//! trips on the second member event.

use std::time::{Duration, Instant};

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, SecretKey, Tag, TagKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

const CONTENT: &str = "hello from the site chat flow";

#[derive(Debug, Serialize, Deserialize)]
struct SeedState {
    secret_hex: String,
    pubkey: String,
    group_id: String,
    member_id: String,
    team_id: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("relay-chat-flow: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let command = std::env::args()
        .nth(1)
        .ok_or("usage: relay-chat-flow <seed|check>")?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    match command.as_str() {
        "seed" => rt.block_on(seed()),
        "check" => rt.block_on(check()),
        other => Err(format!("unknown command {other}")),
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn state_path() -> String {
    env_or("RELAY_CHAT_STATE", "/tmp/relay-chat-flow.json")
}

async fn connect_db() -> Result<scuffed_db::Database, String> {
    let url = env_or("SURREALDB_URL", "ws://127.0.0.1:8000");
    let user = env_or("SURREALDB_USER", "root");
    let password = env_or("SURREALDB_PASSWORD", "root");
    let mut last = String::from("no attempt");
    for attempt in 1..=40 {
        let config = scuffed_db::DbConfig {
            namespace: env_or("SURREALDB_NS", "scuffed_crew"),
            database: env_or("SURREALDB_DB", "main"),
        };
        match scuffed_db::Database::connect_plaintext(&url, &user, &password, config).await {
            Ok(db) => return Ok(db),
            Err(err) => {
                last = err.to_string();
                eprintln!("db connect attempt {attempt}: {last}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    Err(format!("surreal did not accept a connection: {last}"))
}

async fn seed() -> Result<(), String> {
    let db = connect_db().await?;
    scuffed_db::migrations::run_migrations(&db.client)
        .await
        .map_err(|e| format!("migrations: {e}"))?;

    let keys = Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let member = db
        .create_member("relay-ci-user", "Relay CI", scuffed_db::OrgRole::Member)
        .await
        .map_err(|e| format!("create member: {e}"))?;
    db.update_member_nostr_keys(&member.id, Some(&pubkey), Some("external"), None)
        .await
        .map_err(|e| format!("set pubkey: {e}"))?;

    let team = db
        .create_team("Relay CI Squad", "ow2", None, None, None)
        .await
        .map_err(|e| format!("create team: {e}"))?;
    db.add_to_roster(&member.id, &team.id, scuffed_db::TeamRole::Player)
        .await
        .map_err(|e| format!("roster: {e}"))?;
    // Same id the site uses for the public channel (`public_group_id`).
    let group_id = team.id.clone();
    db.create_team_channel(&team.id, &group_id, scuffed_db::GroupType::Public, "")
        .await
        .map_err(|e| format!("channel: {e}"))?;

    let state = SeedState {
        secret_hex: keys.secret_key().to_secret_hex(),
        pubkey,
        group_id,
        member_id: member.id,
        team_id: team.id,
    };
    let path = state_path();
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("write {path}: {e}"))?;
    println!(
        "seeded member {} pubkey {} group {}",
        state.member_id, state.pubkey, state.group_id
    );
    Ok(())
}

fn load_state() -> Result<SeedState, String> {
    let path = state_path();
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse state: {e}"))
}

fn member_keys(state: &SeedState) -> Result<Keys, String> {
    let sk = SecretKey::from_hex(&state.secret_hex).map_err(|e| format!("secret: {e}"))?;
    Ok(Keys::new(sk))
}

fn signed_event(keys: &Keys, kind: u16, content: &str, tags: Vec<Tag>) -> Result<Value, String> {
    let mut builder = EventBuilder::new(Kind::Custom(kind), content);
    for tag in tags {
        builder = builder.tag(tag);
    }
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| format!("sign kind {kind}: {e}"))?;
    Ok(event_value(&event))
}

fn event_value(event: &Event) -> Value {
    json!({
        "id": event.id.to_hex(),
        "pubkey": event.pubkey.to_hex(),
        "created_at": event.created_at.as_secs(),
        "kind": event.kind.as_u16() as u32,
        "tags": event
            .tags
            .iter()
            .map(|tag| tag.as_slice().iter().map(|part| part.to_string()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        "content": event.content.to_string(),
        "sig": event.sig.to_string(),
    })
}

fn h_tag(group_id: &str) -> Tag {
    Tag::custom(TagKind::custom("h"), vec![group_id.to_string()])
}

fn kind9(keys: &Keys, group_id: &str, content: &str) -> Result<Value, String> {
    signed_event(keys, 9, content, vec![h_tag(group_id)])
}

async fn send_text<W>(write: &mut W, text: &str) -> Result<(), String>
where
    W: Sink<Message> + Unpin,
    W::Error: std::fmt::Display,
{
    write
        .send(Message::Text(text.to_string().into()))
        .await
        .map_err(|e| format!("send: {e}"))
}

async fn recv_text<R, E>(read: &mut R) -> Result<String, String>
where
    R: Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("timed out waiting for a relay frame".into());
        }
        let next = tokio::time::timeout(left, read.next())
            .await
            .map_err(|_| "timed out waiting for a relay frame".to_string())?
            .ok_or("relay socket closed")?
            .map_err(|e| format!("relay frame: {e}"))?;
        match next {
            Message::Text(text) => return Ok(text.to_string()),
            Message::Close(_) => return Err("relay closed the socket".into()),
            _ => {}
        }
    }
}

fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("bad json {text}: {e}"))
}

fn frame_kind(value: &Value) -> Option<&str> {
    value.as_array()?.first()?.as_str()
}

fn frame_sub(value: &Value) -> Option<&str> {
    value.as_array()?.get(1)?.as_str()
}

async fn publish_result(url: &str, event: &Value) -> Result<(bool, String, String), String> {
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| format!("strfry connect: {e}"))?;
    let (mut write, mut read) = ws.split();
    let frame = json!(["EVENT", event]).to_string();
    send_text(&mut write, &frame).await?;
    let event_id = event
        .get("id")
        .and_then(|id| id.as_str())
        .unwrap_or("")
        .to_string();
    loop {
        let text = recv_text(&mut read).await?;
        let value = parse_json(&text)?;
        if frame_kind(&value) != Some("OK") {
            continue;
        }
        let accepted = value
            .as_array()
            .and_then(|a| a.get(2))
            .and_then(|v| v.as_bool())
            .ok_or_else(|| format!("OK frame missing accepted flag: {text}"))?;
        let message = value
            .as_array()
            .and_then(|a| a.get(3))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        println!("publish id={event_id} accepted={accepted} message={message}");
        return Ok((accepted, message, event_id));
    }
}

async fn expect_reject(url: &str, event: &Value, needle: &str, label: &str) -> Result<(), String> {
    let (accepted, message, id) = publish_result(url, event).await?;
    if accepted {
        return Err(format!("{label} event {id} was stored"));
    }
    if !message.contains(needle) {
        return Err(format!(
            "{label} rejection {message:?} does not contain {needle:?}"
        ));
    }
    println!("{label} refused ({needle})");
    Ok(())
}

async fn check() -> Result<(), String> {
    let state = load_state()?;
    let keys = member_keys(&state)?;
    if keys.public_key().to_hex() != state.pubkey {
        return Err("state pubkey does not match the secret".into());
    }
    let gate = env_or("RELAY_GATE_URL", "ws://127.0.0.1:17779");
    let strfry = env_or("RELAY_STRFRY_URL", "ws://127.0.0.1:17778");

    let (ws, _) = tokio_tungstenite::connect_async(&gate)
        .await
        .map_err(|e| format!("gate connect: {e}"))?;
    let (mut write, mut read) = ws.split();

    let challenge = loop {
        let text = recv_text(&mut read).await?;
        let value = parse_json(&text)?;
        if frame_kind(&value) == Some("AUTH") {
            let challenge = frame_sub(&value)
                .ok_or("AUTH frame has no challenge")?
                .to_string();
            break challenge;
        }
    };
    println!("gate AUTH challenge received");

    let auth = signed_event(
        &keys,
        22242,
        "",
        vec![
            Tag::custom(TagKind::custom("relay"), vec![gate.clone()]),
            Tag::custom(TagKind::custom("challenge"), vec![challenge]),
        ],
    )?;
    send_text(&mut write, &json!(["AUTH", auth]).to_string()).await?;
    loop {
        let text = recv_text(&mut read).await?;
        let value = parse_json(&text)?;
        if frame_kind(&value) != Some("OK") {
            continue;
        }
        let accepted = value
            .as_array()
            .and_then(|a| a.get(2))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let message = value
            .as_array()
            .and_then(|a| a.get(3))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !accepted {
            return Err(format!("NIP-42 AUTH rejected: {message}"));
        }
        println!("NIP-42 AUTH accepted for {}", state.pubkey);
        break;
    }

    // Same filter as NostrFilter::group_metadata.
    let meta = json!([
        "REQ",
        "meta",
        {"kinds": [39000, 39001, 39002], "#d": [state.group_id]}
    ]);
    send_text(&mut write, &meta.to_string()).await?;
    let mut meta_events = 0usize;
    loop {
        let text = recv_text(&mut read).await?;
        let value = parse_json(&text)?;
        match frame_kind(&value) {
            Some("EVENT") if frame_sub(&value) == Some("meta") => meta_events += 1,
            Some("EOSE") if frame_sub(&value) == Some("meta") => break,
            Some("CLOSED") => {
                return Err(format!("metadata query closed: {text}"));
            }
            _ => {}
        }
    }
    println!(
        "metadata query for group {} EOSE events={meta_events} (group id is the team channel row)",
        state.group_id
    );

    let sub = json!([
        "REQ",
        "group",
        {"kinds": [9], "#h": [state.group_id], "limit": 50}
    ]);
    send_text(&mut write, &sub.to_string()).await?;
    loop {
        let text = recv_text(&mut read).await?;
        let value = parse_json(&text)?;
        match frame_kind(&value) {
            Some("EOSE") if frame_sub(&value) == Some("group") => break,
            Some("CLOSED") => return Err(format!("kind 9 subscription closed: {text}")),
            _ => {}
        }
    }
    println!("subscribed to kind 9 #h={}", state.group_id);

    let event = kind9(&keys, &state.group_id, CONTENT)?;
    let (accepted, message, event_id) = publish_result(&strfry, &event).await?;
    if !accepted {
        return Err(format!(
            "kind 9 publish rejected: {message} (id {event_id})"
        ));
    }
    println!("kind 9 stored id={event_id}");

    loop {
        let text = recv_text(&mut read).await?;
        let value = parse_json(&text)?;
        if frame_kind(&value) != Some("EVENT") || frame_sub(&value) != Some("group") {
            continue;
        }
        let body = value
            .as_array()
            .and_then(|a| a.get(2))
            .ok_or("EVENT frame has no body")?;
        let id = body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id != event_id {
            continue;
        }
        let kind = body.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
        let content = body.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let h = body
            .get("tags")
            .and_then(|v| v.as_array())
            .and_then(|tags| {
                tags.iter().find_map(|tag| {
                    let parts = tag.as_array()?;
                    if parts.first()?.as_str()? == "h" {
                        parts.get(1)?.as_str().map(str::to_string)
                    } else {
                        None
                    }
                })
            });
        if kind != 9 || content != CONTENT || h.as_deref() != Some(state.group_id.as_str()) {
            return Err(format!("received event does not match the publish: {text}"));
        }
        println!("received kind 9 id={event_id} on the subscription");
        break;
    }

    let outsider = Keys::generate();
    let foreign = kind9(&outsider, &state.group_id, "not a member")?;
    expect_reject(&strfry, &foreign, "allowlist", "non-member").await?;

    // Kind 42 is not on the site whitelist. Kind numbers above 65535 do not
    // fit in a Nostr kind.
    let weird = signed_event(&keys, 42, "nope", vec![])?;
    expect_reject(&strfry, &weird, "not allowed", "disallowed kind").await?;

    let extra = kind9(&keys, &state.group_id, "over the limit")?;
    expect_reject(&strfry, &extra, "rate-limited", "rate limit").await?;

    println!("chat flow ok");
    Ok(())
}
