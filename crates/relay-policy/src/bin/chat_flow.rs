//! End-to-end check for the relay image.
//!
//! `seed` writes one active member, team, roster edge, and the public plus
//! officer `team_channel` rows `ensure_team_channel_rows` writes. `check`
//! then does what the site chat does:
//! 1. NIP-42 AUTH on the read gate.
//! 2. Read channels with `get_team_channels`, the query behind
//!    `GET /api/teams/:id/channels`, and keep the public row a member sees.
//! 3. REQ kind 9 with `#h` set to that id (`NostrFilter::group_chat`, limit 50).
//! 4. REQ kind 1059 with `#p` set to the member (`NostrFilter::gift_wraps`).
//! 5. Publish one kind 9 with a single `h` tag straight to strfry, the same
//!    shape as `POST /api/chat/send-public`, and read it back.
//! 6. Publish one kind 1059 from an ephemeral key with a `p` tag for that
//!    member, the outer shape of `EncryptionService::build_gift_wraps`, and
//!    read it back on the `#p` subscription.
//! 7. Refuse a kind 9 whose `h` tag is another team's channel.
//! 8. Refuse a plain member's kind 9 on the officer channel, and accept an
//!    officer's kind 9 on that channel.
//! 9. Refuse an anonymous kind 9 and kind 1059 on the gate (`auth-required`).
//! 10. Accept a kind 1059 from an authenticated member connection on the gate.
//! 11. After the relay is up, add a member and a team channel, wait for the
//!     LIVE reload, and accept that member's kind 9.
//! 12. Refuse a pubkey that is not on the allowlist, a kind the site never
//!     publishes, and the next kind 9 once the member rate limit is full.
//!
//! The container must be started with `RELAY_POLICY_RATE_LIMIT=1`. Kind 1059
//! uses a per-sender `gift:` bucket, so the wrap still stores after the kind
//! 9 fills the member bucket. The second kind 9 is the rate-limit case.

use std::time::{Duration, Instant};

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, SecretKey, Tag, TagKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

const CONTENT: &str = "hello from the site chat flow";
const WRAP_CONTENT: &str = "gift wrap from the site chat flow";
/// Not a site kind, not NIP-28 channel chat (42), and outside the ephemeral
/// band 20000-29999 that the policy allows.
const UNRELATED_KIND: u16 = 12345;

#[derive(Debug, Serialize, Deserialize)]
struct SeedState {
    secret_hex: String,
    pubkey: String,
    group_id: String,
    member_id: String,
    team_id: String,
    other_team_id: String,
    officer_secret_hex: String,
    officer_group_id: String,
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
        .ok_or("usage: relay-chat-flow <seed|check|relay-login|index-perms>")?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    match command.as_str() {
        "seed" => rt.block_on(seed()),
        "check" => rt.block_on(check()),
        "relay-login" => rt.block_on(relay_login()),
        "index-perms" => rt.block_on(index_perms()),
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

/// Sign in as the relay record user against the server this process already
/// reached as root. Proves the login, and that the secret column and
/// `audit_log` stay hidden, on a real SurrealDB rather than `mem://`.
async fn relay_login() -> Result<(), String> {
    let password = env_or("SURREALDB_RELAY_PASSWORD", "relay-ci-record-login");
    let password = password.trim();
    if password.is_empty() || password == "root" {
        return Err(
            "relay-login needs SURREALDB_RELAY_PASSWORD set to a non-root value".to_string(),
        );
    }
    let db = connect_db().await?;
    scuffed_db::migrations::run_migrations(&db.client)
        .await
        .map_err(|e| format!("migrations: {e}"))?;
    scuffed_db::ensure_relay_access(&db.client, "scuffed_relay", password, true)
        .await
        .map_err(|e| format!("ensure relay access: {e}"))?;

    let secret = "relay-login-secret-marker";
    let reason = "relay-login-reason-marker";
    let audit = "relay-login-audit-marker";
    db.client
        .query(
            "CREATE member:relaylogin SET user_id = 'user:relaylogin', org_role = 'member', \
             display_name = 'RelayLogin', is_active = true, nostr_pubkey = 'cd', \
             nostr_secret_key_encrypted = { marker: $secret };
             CREATE moderation_action:relaylogin SET member_id = 'relaylogin', \
             action_type = 'ban', reason = $reason, issued_by = 'officer-1', is_active = true;
             CREATE audit_log SET actor_id = 'officer-1', action = $audit, \
             target_type = 'member', target_id = 'relaylogin'",
        )
        .bind(("secret", secret))
        .bind(("reason", reason))
        .bind(("audit", audit))
        .await
        .map_err(|e| format!("seed hidden rows: {e}"))?
        .check()
        .map_err(|e| format!("seed hidden rows: {e}"))?;

    let url = env_or("SURREALDB_URL", "ws://127.0.0.1:8000");
    let ns = env_or("SURREALDB_NS", "scuffed_crew");
    let database = env_or("SURREALDB_DB", "main");
    let relay =
        scuffed_db::Database::connect_relay_record(&url, &ns, &database, "scuffed_relay", password)
            .await
            .map_err(|e| format!("relay sign-in: {e}"))?;

    let member = relay
        .client
        .query("SELECT * FROM member:relaylogin")
        .await
        .map_err(|e| format!("relay member select: {e}"))?;
    let member_dbg = format!("{member:?}");
    if member_dbg.contains(secret) {
        return Err("relay login read nostr_secret_key_encrypted".to_string());
    }
    let actions = relay
        .client
        .query("SELECT * FROM moderation_action:relaylogin")
        .await
        .map_err(|e| format!("relay moderation select: {e}"))?;
    let actions_dbg = format!("{actions:?}");
    if actions_dbg.contains(reason) {
        return Err("relay login read moderation reason".to_string());
    }
    let audits = relay
        .client
        .query("SELECT * FROM audit_log")
        .await
        .map_err(|e| format!("relay audit select: {e}"))?;
    let audits_dbg = format!("{audits:?}");
    if audits_dbg.contains(audit) {
        return Err("relay login read audit_log".to_string());
    }
    println!("relay record login kept hidden fields and audit_log hidden");
    Ok(())
}

/// Indexed selects against the real server. SurrealDB 3.0.5 can serve a
/// SELECT from a BTree index without field permissions. This fails if a
/// hidden value comes back from `member` or `moderation_action`. It also
/// fails if the relay projection is missing the pubkey, the roster pair, or
/// the team channel, or if those copies contain a hidden value.
async fn index_perms() -> Result<(), String> {
    let password = env_or("SURREALDB_RELAY_PASSWORD", "relay-ci-record-login");
    let password = password.trim();
    if password.is_empty() || password == "root" {
        return Err(
            "index-perms needs SURREALDB_RELAY_PASSWORD set to a non-root value".to_string(),
        );
    }
    let db = connect_db().await?;
    scuffed_db::migrations::run_migrations(&db.client)
        .await
        .map_err(|e| format!("migrations: {e}"))?;
    scuffed_db::ensure_relay_access(&db.client, "scuffed_relay", password, true)
        .await
        .map_err(|e| format!("ensure relay access: {e}"))?;

    let secret = "index-secret-marker";
    let reason = "index-reason-marker";
    let issued_by = "index-officer-marker";
    let display_name = "IndexLeakName";
    let name_key = "indexleakname";
    let user_id = "user:indexleak";
    db.client
        .query(
            "CREATE member:indexleak SET user_id = $user_id, org_role = 'member', \
             display_name = $display_name, display_name_key = $name_key, is_active = true, \
             nostr_pubkey = 'ab', nostr_secret_key_encrypted = { marker: $secret };
             CREATE team:indexleak SET name = 'IndexSquad', game_id = 'ow2', is_active = true;
             RELATE member:indexleak->plays_on->team:indexleak \
                 SET team_role = 'player', is_active = true;
             CREATE team_channel:indexleak SET team_id = 'indexleak', \
                 group_id = 'indexleak-public', group_type = 'public', \
                 relay_url = 'wss://relay.example', is_active = true",
        )
        .bind(("secret", secret))
        .bind(("display_name", display_name))
        .bind(("name_key", name_key))
        .bind(("user_id", user_id))
        .await
        .map_err(|e| format!("seed indexed rows: {e}"))?
        .check()
        .map_err(|e| format!("seed indexed rows: {e}"))?;

    let url = env_or("SURREALDB_URL", "ws://127.0.0.1:8000");
    let ns = env_or("SURREALDB_NS", "scuffed_crew");
    let database = env_or("SURREALDB_DB", "main");
    let relay =
        scuffed_db::Database::connect_relay_record(&url, &ns, &database, "scuffed_relay", password)
            .await
            .map_err(|e| format!("relay sign-in: {e}"))?;

    let hidden = [secret, reason, issued_by, display_name, name_key, user_id];
    let member_queries = [
        (
            "member_user_idx",
            "SELECT * FROM member WHERE user_id = 'user:indexleak'",
        ),
        (
            "member_display_name_key_idx",
            "SELECT * FROM member WHERE display_name_key = 'indexleakname'",
        ),
        (
            "member_active_name_idx",
            "SELECT * FROM member WHERE is_active = true AND display_name = 'IndexLeakName'",
        ),
        (
            "member_nostr_pubkey_idx",
            "SELECT * FROM member WHERE nostr_pubkey = 'ab'",
        ),
    ];
    let mut leaked = Vec::new();
    for (label, sql) in member_queries {
        if indexed_select_leaks(&relay, label, sql, &hidden).await? {
            leaked.push(label);
        }
    }

    let keys = relay
        .list_relay_member_keys()
        .await
        .map_err(|e| format!("relay members: {e}"))?;
    reject_hidden("relay member projection", &format!("{keys:?}"), &hidden)?;
    if !keys
        .iter()
        .any(|row| row.id == "indexleak" && row.nostr_pubkey.as_deref() == Some("ab"))
    {
        return Err("relay member projection did not include the indexed pubkey".to_string());
    }
    let pairs = relay
        .list_active_roster_pairs()
        .await
        .map_err(|e| format!("relay roster: {e}"))?;
    reject_hidden("relay roster projection", &format!("{pairs:?}"), &hidden)?;
    if !pairs
        .iter()
        .any(|(member_id, team_id)| member_id == "indexleak" && team_id == "indexleak")
    {
        return Err("relay roster projection did not include the active pair".to_string());
    }
    let channels = relay
        .list_all_team_channels()
        .await
        .map_err(|e| format!("relay channels: {e}"))?;
    reject_hidden(
        "relay channel projection",
        &format!("{channels:?}"),
        &hidden,
    )?;
    if !channels
        .iter()
        .any(|channel| channel.group_id == "indexleak-public" && channel.team_id == "indexleak")
    {
        return Err("relay channel projection did not include the team channel".to_string());
    }

    db.client
        .query("UPDATE member:indexleak SET is_active = false")
        .await
        .map_err(|e| format!("deactivate member: {e}"))?
        .check()
        .map_err(|e| format!("deactivate member: {e}"))?;
    let pairs = relay
        .list_active_roster_pairs()
        .await
        .map_err(|e| format!("relay roster after deactivate: {e}"))?;
    if pairs.iter().any(|(member_id, _)| member_id == "indexleak") {
        return Err("relay roster still listed the member after is_active was cleared".to_string());
    }
    db.client
        .query("UPDATE member:indexleak SET is_active = true")
        .await
        .map_err(|e| format!("reactivate member: {e}"))?
        .check()
        .map_err(|e| format!("reactivate member: {e}"))?;
    let pairs = relay
        .list_active_roster_pairs()
        .await
        .map_err(|e| format!("relay roster after reactivate: {e}"))?;
    if !pairs
        .iter()
        .any(|(member_id, team_id)| member_id == "indexleak" && team_id == "indexleak")
    {
        return Err("relay roster did not restore the member after reactivation".to_string());
    }

    db.client
        .query(
            "CREATE moderation_action:indexleak SET member_id = 'indexleak', \
             action_type = 'ban', reason = $reason, issued_by = $issued_by, is_active = true",
        )
        .bind(("reason", reason))
        .bind(("issued_by", issued_by))
        .await
        .map_err(|e| format!("seed moderation: {e}"))?
        .check()
        .map_err(|e| format!("seed moderation: {e}"))?;
    let mod_queries = [
        (
            "mod_member_idx",
            "SELECT * FROM moderation_action WHERE member_id = 'indexleak'",
        ),
        (
            "mod_active_idx",
            "SELECT * FROM moderation_action WHERE member_id = 'indexleak' AND action_type = 'ban' AND is_active = true",
        ),
    ];
    for (label, sql) in mod_queries {
        if indexed_select_leaks(&relay, label, sql, &hidden).await? {
            leaked.push(label);
        }
    }
    let pairs = relay
        .list_active_roster_pairs()
        .await
        .map_err(|e| format!("relay roster after ban: {e}"))?;
    if pairs.iter().any(|(member_id, _)| member_id == "indexleak") {
        return Err("relay roster still listed the banned member".to_string());
    }

    if !leaked.is_empty() {
        return Err(format!(
            "indexed select returned hidden fields: {}",
            leaked.join(", ")
        ));
    }
    println!("indexed selects did not return hidden fields");
    Ok(())
}

fn reject_hidden(label: &str, rendered: &str, hidden: &[&str]) -> Result<(), String> {
    if hidden.iter().any(|marker| rendered.contains(marker)) {
        return Err(format!("{label} included a hidden field"));
    }
    Ok(())
}

/// True when the taken result contains a hidden marker. A denied query is
/// not a leak. The check uses the result value, not the response debug,
/// because the SQL text itself names some of the markers.
async fn indexed_select_leaks(
    relay: &scuffed_db::Database,
    label: &str,
    sql: &str,
    hidden: &[&str],
) -> Result<bool, String> {
    let mut response = match relay.client.query(sql).await {
        Ok(response) => response,
        Err(err) => {
            reject_hidden(label, &err.to_string(), hidden)?;
            eprintln!("{label}: denied");
            return Ok(false);
        }
    };
    match response.take::<surrealdb::types::Value>(0) {
        Ok(value) => {
            let rendered = format!("{value:?}");
            if hidden.iter().any(|marker| rendered.contains(marker)) {
                eprintln!("{label}: hidden fields returned");
                Ok(true)
            } else {
                eprintln!("{label}: no hidden field");
                Ok(false)
            }
        }
        Err(err) => {
            reject_hidden(label, &err.to_string(), hidden)?;
            eprintln!("{label}: denied");
            Ok(false)
        }
    }
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
    // Same rows as `ensure_team_channel_rows`: public id is the team id,
    // officer id is `{team_id}-officers`.
    let group_id = team.id.clone();
    let officer_id = format!("{group_id}-officers");
    db.create_team_channel(&team.id, &group_id, scuffed_db::GroupType::Public, "")
        .await
        .map_err(|e| format!("public channel: {e}"))?;
    db.create_team_channel(&team.id, &officer_id, scuffed_db::GroupType::Officer, "")
        .await
        .map_err(|e| format!("officer channel: {e}"))?;
    // Second team. The seeded member is not on its roster, so an `h` tag
    // for its public channel must be refused.
    let other = db
        .create_team("Relay CI Other", "ow2", None, None, None)
        .await
        .map_err(|e| format!("create other team: {e}"))?;
    db.create_team_channel(&other.id, &other.id, scuffed_db::GroupType::Public, "")
        .await
        .map_err(|e| format!("other channel: {e}"))?;

    let officer_keys = Keys::generate();
    let officer_pubkey = officer_keys.public_key().to_hex();
    let officer = db
        .create_member(
            "relay-ci-officer",
            "Relay CI Officer",
            scuffed_db::OrgRole::Officer,
        )
        .await
        .map_err(|e| format!("create officer: {e}"))?;
    db.update_member_nostr_keys(&officer.id, Some(&officer_pubkey), Some("external"), None)
        .await
        .map_err(|e| format!("officer pubkey: {e}"))?;
    db.add_to_roster(&officer.id, &team.id, scuffed_db::TeamRole::Coach)
        .await
        .map_err(|e| format!("officer roster: {e}"))?;

    let state = SeedState {
        secret_hex: keys.secret_key().to_secret_hex(),
        pubkey,
        group_id,
        member_id: member.id,
        team_id: team.id,
        other_team_id: other.id,
        officer_secret_hex: officer_keys.secret_key().to_secret_hex(),
        officer_group_id: officer_id,
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
    keys_from_hex(&state.secret_hex)
}

fn keys_from_hex(secret_hex: &str) -> Result<Keys, String> {
    let sk = SecretKey::from_hex(secret_hex).map_err(|e| format!("secret: {e}"))?;
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

fn p_tag(pubkey: &str) -> Tag {
    Tag::custom(TagKind::custom("p"), vec![pubkey.to_string()])
}

/// Outer NIP-59 gift wrap: ephemeral author, kind 1059, one `p` tag.
fn gift_wrap(recipient: &str) -> Result<Value, String> {
    let ephemeral = Keys::generate();
    signed_event(&ephemeral, 1059, WRAP_CONTENT, vec![p_tag(recipient)])
}

/// Public channel a non-officer sees from `GET /api/teams/:id/channels`.
async fn public_group_id(team_id: &str) -> Result<String, String> {
    let db = connect_db().await?;
    let mut channels = db
        .get_team_channels(team_id)
        .await
        .map_err(|e| format!("get_team_channels: {e}"))?;
    channels.retain(|channel| channel.group_type != scuffed_db::GroupType::Officer);
    channels
        .into_iter()
        .find(|channel| channel.group_type == scuffed_db::GroupType::Public)
        .map(|channel| channel.group_id)
        .ok_or_else(|| format!("no public channel for team {team_id}"))
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

async fn subscribe_until_eose<W, R, E>(
    write: &mut W,
    read: &mut R,
    sub: &str,
    filter: Value,
) -> Result<(), String>
where
    W: Sink<Message> + Unpin,
    W::Error: std::fmt::Display,
    R: Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    send_text(write, &json!(["REQ", sub, filter]).to_string()).await?;
    loop {
        let text = recv_text(read).await?;
        let value = parse_json(&text)?;
        match frame_kind(&value) {
            Some("EOSE") if frame_sub(&value) == Some(sub) => return Ok(()),
            Some("CLOSED") if frame_sub(&value) == Some(sub) => {
                return Err(format!("{sub} subscription closed: {text}"));
            }
            _ => {}
        }
    }
}

async fn expect_stored(url: &str, event: &Value, label: &str) -> Result<String, String> {
    let (accepted, message, event_id) = publish_result(url, event).await?;
    if !accepted {
        return Err(format!(
            "{label} publish rejected: {message} (id {event_id})"
        ));
    }
    println!("{label} stored id={event_id}");
    Ok(event_id)
}

fn tag_value(body: &Value, name: &str) -> Option<String> {
    body.get("tags")?.as_array()?.iter().find_map(|tag| {
        let parts = tag.as_array()?;
        if parts.first()?.as_str()? == name {
            parts.get(1)?.as_str().map(str::to_string)
        } else {
            None
        }
    })
}

async fn expect_received<R, E>(
    read: &mut R,
    sub: &str,
    event_id: &str,
    kind: u64,
    content: &str,
    tag_name: &str,
    tag_expected: &str,
) -> Result<(), String>
where
    R: Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    loop {
        let text = recv_text(read).await?;
        let value = parse_json(&text)?;
        if frame_kind(&value) != Some("EVENT") || frame_sub(&value) != Some(sub) {
            continue;
        }
        let body = value
            .as_array()
            .and_then(|frame| frame.get(2))
            .ok_or("EVENT frame has no body")?;
        let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id != event_id {
            continue;
        }
        let got_kind = body.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
        let got_content = body.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let got_tag = tag_value(body, tag_name);
        if got_kind != kind || got_content != content || got_tag.as_deref() != Some(tag_expected) {
            return Err(format!("received event does not match the publish: {text}"));
        }
        return Ok(());
    }
}

async fn publish_authed(
    url: &str,
    keys: &Keys,
    event: &Value,
) -> Result<(bool, String, String), String> {
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| format!("gate connect: {e}"))?;
    let (mut write, mut read) = ws.split();
    let challenge = loop {
        let text = recv_text(&mut read).await?;
        let value = parse_json(&text)?;
        if frame_kind(&value) == Some("AUTH") {
            break frame_sub(&value)
                .ok_or("AUTH frame has no challenge")?
                .to_string();
        }
    };
    let auth = signed_event(
        keys,
        22242,
        "",
        vec![
            Tag::custom(TagKind::custom("relay"), vec![url.to_string()]),
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
        if !accepted {
            let message = value
                .as_array()
                .and_then(|a| a.get(3))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            return Err(format!("gate AUTH rejected: {message}"));
        }
        break;
    }
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
        return Ok((accepted, message, event_id));
    }
}

/// Create a member and a public channel after the relay is already running.
/// The first kind 9 must be accepted without waiting for the 60s refresh.
async fn expect_fresh_roster_write(strfry: &str) -> Result<(), String> {
    let db = connect_db().await?;
    let keys = Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let member = db
        .create_member(
            "relay-ci-late",
            "Relay CI Late",
            scuffed_db::OrgRole::Member,
        )
        .await
        .map_err(|e| format!("late member: {e}"))?;
    db.update_member_nostr_keys(&member.id, Some(&pubkey), Some("external"), None)
        .await
        .map_err(|e| format!("late pubkey: {e}"))?;
    let team = db
        .create_team("Relay CI Late", "ow2", None, None, None)
        .await
        .map_err(|e| format!("late team: {e}"))?;
    db.add_to_roster(&member.id, &team.id, scuffed_db::TeamRole::Player)
        .await
        .map_err(|e| format!("late roster: {e}"))?;
    db.create_team_channel(&team.id, &team.id, scuffed_db::GroupType::Public, "")
        .await
        .map_err(|e| format!("late channel: {e}"))?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let event = kind9(&keys, &team.id, "late roster write")?;
    let (accepted, message, id) = publish_result(strfry, &event).await?;
    if !accepted {
        return Err(format!(
            "roster reload did not admit the new member ({id}): {message}"
        ));
    }
    println!("new roster member write accepted after live reload id={id}");
    Ok(())
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

    let anon9 = kind9(&keys, &state.group_id, "anonymous kind 9")?;
    expect_reject(&gate, &anon9, "auth-required", "anonymous kind 9").await?;
    let anon_wrap = gift_wrap(&state.pubkey)?;
    expect_reject(&gate, &anon_wrap, "auth-required", "anonymous kind 1059").await?;

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

    let group_id = public_group_id(&state.team_id).await?;
    println!("public channel from get_team_channels group_id={group_id}");

    subscribe_until_eose(
        &mut write,
        &mut read,
        "group",
        json!({"kinds": [9], "#h": [group_id], "limit": 50}),
    )
    .await?;
    println!("subscribed to kind 9 #h={group_id}");

    subscribe_until_eose(
        &mut write,
        &mut read,
        "wraps",
        json!({"kinds": [1059], "#p": [state.pubkey], "limit": 50}),
    )
    .await?;
    println!("subscribed to kind 1059 #p={}", state.pubkey);

    let event = kind9(&keys, &group_id, CONTENT)?;
    let event_id = expect_stored(&strfry, &event, "kind 9").await?;
    expect_received(&mut read, "group", &event_id, 9, CONTENT, "h", &group_id).await?;
    println!("received kind 9 id={event_id} on the #h subscription");

    let other_group = public_group_id(&state.other_team_id).await?;
    println!("other team channel from get_team_channels group_id={other_group}");
    let cross = kind9(&keys, &other_group, "other team")?;
    expect_reject(&strfry, &cross, "not a member", "other team").await?;

    let officer_group = state.officer_group_id.clone();
    let member_officer = kind9(&keys, &officer_group, "member on officer channel")?;
    expect_reject(
        &strfry,
        &member_officer,
        "not a member",
        "plain member officer channel",
    )
    .await?;
    let officer_keys = keys_from_hex(&state.officer_secret_hex)?;
    let officer_event = kind9(&officer_keys, &officer_group, "officer channel")?;
    expect_stored(&strfry, &officer_event, "officer channel").await?;

    // Member bucket is already full when the rate limit is 1. The wrap uses
    // its own gift: bucket, so it still stores.
    let wrap = gift_wrap(&state.pubkey)?;
    let wrap_id = expect_stored(&strfry, &wrap, "kind 1059").await?;
    expect_received(
        &mut read,
        "wraps",
        &wrap_id,
        1059,
        WRAP_CONTENT,
        "p",
        &state.pubkey,
    )
    .await?;
    println!("received kind 1059 id={wrap_id} on the #p subscription");

    let gate_wrap = gift_wrap(&state.pubkey)?;
    let (gate_ok, gate_message, gate_wrap_id) = publish_authed(&gate, &keys, &gate_wrap).await?;
    if !gate_ok {
        return Err(format!(
            "authed member wrap rejected: {gate_message} (id {gate_wrap_id})"
        ));
    }
    println!("authed member wrap stored id={gate_wrap_id}");
    expect_received(
        &mut read,
        "wraps",
        &gate_wrap_id,
        1059,
        WRAP_CONTENT,
        "p",
        &state.pubkey,
    )
    .await?;
    println!("received authed gate wrap id={gate_wrap_id} on the #p subscription");

    expect_fresh_roster_write(&strfry).await?;

    let outsider = Keys::generate();
    let foreign = kind9(&outsider, &group_id, "not a member")?;
    expect_reject(&strfry, &foreign, "allowlist", "non-member").await?;

    let weird = signed_event(&keys, UNRELATED_KIND, "nope", vec![])?;
    expect_reject(&strfry, &weird, "not allowed", "disallowed kind").await?;

    let extra = kind9(&keys, &group_id, "over the limit")?;
    expect_reject(&strfry, &extra, "rate-limited", "rate limit").await?;

    println!("chat flow ok");
    Ok(())
}
