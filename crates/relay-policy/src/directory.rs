//! Load the read directory and the write-side group map from SurrealDB.
//!
//! Both use the same rows: active members' Nostr pubkeys, active roster
//! edges, and `team_channel` rows. The read gate keeps deactivated channels
//! so officer history stays classified. The write map only includes active
//! channels, matching `POST /api/chat/send-public`.
//!
//! Public channels list every roster pubkey on that team. Officer channels
//! list only roster members whose org role is officer or admin
//! (`OrgRole::can_access_officer_channel`), the same check the read gate uses.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use scuffed_db::{Database, GroupType};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::read_policy::{GroupAcl, MemberAcl, ReadDirectory};

pub async fn load_read_directory(db: &Database) -> scuffed_db::DbResult<ReadDirectory> {
    let members = db.list_nostr_identities().await?;
    let pairs = db.list_active_roster_pairs().await?;
    let channels = db.list_all_team_channels().await?;

    let mut teams_by_member: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for (member_id, team_id) in pairs {
        teams_by_member
            .entry(member_id)
            .or_default()
            .insert(team_id);
    }

    let acls = members.into_iter().filter_map(|member| {
        let pubkey = member.nostr_pubkey?;
        let teams = teams_by_member.remove(&member.id).unwrap_or_default();
        Some(MemberAcl {
            pubkey,
            member_id: member.id,
            officer: member.org_role.can_access_officer_channel(),
            teams,
        })
    });

    let groups = channels.into_iter().map(|channel| {
        (
            channel.group_id,
            GroupAcl {
                team_id: channel.team_id,
                officer_only: channel.group_type == GroupType::Officer,
            },
        )
    });

    Ok(ReadDirectory::from_parts(acls, groups))
}

struct TeamWriters {
    roster: HashSet<String>,
    officers: HashSet<String>,
}

/// `group_id` to the pubkeys that may write an `h` tag for that channel.
///
/// A public channel includes every active roster pubkey on that team. An
/// officer channel includes only those roster members whose org role can
/// open officer chat. Kind 1059 gift wraps are not in this map: they have
/// no `h` tag.
pub async fn load_group_writers(
    db: &Database,
) -> scuffed_db::DbResult<HashMap<String, HashSet<String>>> {
    let members = db.list_nostr_identities().await?;
    let pairs = db.list_active_roster_pairs().await?;
    let channels = db.list_all_team_channels().await?;

    let mut pubkey_by_member: HashMap<String, (String, bool)> = HashMap::new();
    for member in members {
        let Some(pubkey) = member.nostr_pubkey else {
            continue;
        };
        let pubkey = pubkey.to_ascii_lowercase();
        if pubkey.len() == 64 && pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
            pubkey_by_member.insert(
                member.id,
                (pubkey, member.org_role.can_access_officer_channel()),
            );
        }
    }

    let mut writers_by_team: HashMap<String, TeamWriters> = HashMap::new();
    for (member_id, team_id) in pairs {
        let Some((pubkey, officer)) = pubkey_by_member.get(&member_id) else {
            continue;
        };
        let entry = writers_by_team
            .entry(team_id)
            .or_insert_with(|| TeamWriters {
                roster: HashSet::new(),
                officers: HashSet::new(),
            });
        entry.roster.insert(pubkey.clone());
        if *officer {
            entry.officers.insert(pubkey.clone());
        }
    }

    let mut groups = HashMap::new();
    for channel in channels {
        if !channel.is_active {
            continue;
        }
        let Some(team) = writers_by_team.get(&channel.team_id) else {
            groups.insert(channel.group_id, HashSet::new());
            continue;
        };
        let writers = if channel.group_type == GroupType::Officer {
            team.officers.clone()
        } else {
            team.roster.clone()
        };
        groups.insert(channel.group_id, writers);
    }
    Ok(groups)
}

/// Watch roster edges, team channels, and member rows. Each change sends on
/// `tx` so the caller can reload the writer list. Member rows are included
/// because officer permission and the Nostr pubkey live on `member`, not on
/// the roster edge.
///
/// If LIVE queries are unavailable the loop logs once and retries. The
/// caller still has the timed refresh and the debounced miss reload.
pub async fn watch_writer_tables(db: Arc<Database>, tx: UnboundedSender<()>) {
    tokio::join!(
        watch_one(Arc::clone(&db), "plays_on", tx.clone()),
        watch_one(Arc::clone(&db), "team_channel", tx.clone()),
        watch_one(db, "member", tx),
    );
}

/// Reload trigger only. The caller discards the notification body.
///
/// SurrealDB 3 accepts a field projection on `LIVE SELECT`. `id` is enough
/// to notice a change. `SELECT *` on `member` would stream
/// `nostr_secret_key_encrypted` into this process. `plays_on` (role and
/// timestamps) and `team_channel` (group id, type, relay URL) have no secret
/// columns. They use the same projection so a later column cannot ride along.
fn live_sql(table: &'static str) -> &'static str {
    match table {
        "plays_on" => "LIVE SELECT id FROM plays_on",
        "team_channel" => "LIVE SELECT id FROM team_channel",
        "member" => "LIVE SELECT id FROM member",
        _ => "LIVE SELECT id FROM member WHERE false",
    }
}

async fn watch_one(db: Arc<Database>, table: &'static str, tx: UnboundedSender<()>) {
    let mut announced = false;
    loop {
        let started = async {
            let mut response = db.client.query(live_sql(table)).await?;
            response.stream::<surrealdb::Notification<surrealdb::types::Value>>(0)
        }
        .await;
        let mut stream = match started {
            Ok(stream) => {
                tracing::info!(table, "live query watching writer changes");
                announced = false;
                stream
            }
            Err(err) => {
                if !announced {
                    tracing::warn!(
                        table,
                        error = %err,
                        "live query did not start; timed refresh and miss reload still run"
                    );
                    announced = true;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        while let Some(item) = stream.next().await {
            match item {
                Ok(_) => {
                    let _ = tx.send(());
                }
                Err(err) => {
                    tracing::warn!(table, error = %err, "live query ended; restarting");
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Coalesce LIVE notices and write misses. Waits 500ms after the latest
/// signal, and at least 2s between reloads, so a burst cannot hammer SurrealDB.
pub async fn coalesce_reloads<F, Fut>(mut rx: UnboundedReceiver<()>, mut reload: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut last = tokio::time::Instant::now();
    loop {
        if rx.recv().await.is_none() {
            break;
        }
        let quiet = tokio::time::sleep(Duration::from_millis(500));
        tokio::pin!(quiet);
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    if msg.is_none() {
                        return;
                    }
                    quiet.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(500));
                }
                _ = &mut quiet => break,
            }
        }
        let min_gap = Duration::from_secs(2);
        if let Some(wait) = min_gap.checked_sub(last.elapsed()) {
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
                while rx.try_recv().is_ok() {}
            }
        }
        reload().await;
        last = tokio::time::Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_db::{GroupType, OrgRole, TeamRole};

    async fn mem(name: &str) -> Database {
        let client = surrealdb::engine::any::connect(format!("mem://{name}"))
            .await
            .expect("mem connect");
        client
            .use_ns("scuffed_crew")
            .use_db("main")
            .await
            .expect("ns");
        Database {
            client,
            crypto: None,
        }
    }

    #[tokio::test]
    async fn officer_channel_writers_exclude_plain_members() {
        let db = mem("relay-policy-writers").await;
        scuffed_db::migrations::run_migrations(&db.client)
            .await
            .expect("migrations");
        let member = db
            .create_member("writer-member", "Member", OrgRole::Member)
            .await
            .unwrap();
        let officer = db
            .create_member("writer-officer", "Officer", OrgRole::Officer)
            .await
            .unwrap();
        let member_pk = "ab".repeat(32);
        let officer_pk = "cd".repeat(32);
        db.update_member_nostr_keys(&member.id, Some(&member_pk), Some("external"), None)
            .await
            .unwrap();
        db.update_member_nostr_keys(&officer.id, Some(&officer_pk), Some("external"), None)
            .await
            .unwrap();
        let team = db
            .create_team("Writers", "ow2", None, None, None)
            .await
            .unwrap();
        db.add_to_roster(&member.id, &team.id, TeamRole::Player)
            .await
            .unwrap();
        db.add_to_roster(&officer.id, &team.id, TeamRole::Coach)
            .await
            .unwrap();
        let public_id = team.id.clone();
        let officer_id = format!("{public_id}-officers");
        db.create_team_channel(&team.id, &public_id, GroupType::Public, "")
            .await
            .unwrap();
        db.create_team_channel(&team.id, &officer_id, GroupType::Officer, "")
            .await
            .unwrap();

        let groups = load_group_writers(&db).await.unwrap();
        let public = groups.get(&public_id).unwrap();
        assert!(public.contains(&member_pk));
        assert!(public.contains(&officer_pk));
        let officers = groups.get(&officer_id).unwrap();
        assert!(!officers.contains(&member_pk));
        assert!(officers.contains(&officer_pk));
    }

    #[tokio::test]
    async fn live_query_signals_when_a_member_row_is_created() {
        let db = Arc::new(mem("relay-policy-live").await);
        db.client
            .query("DEFINE TABLE member SCHEMALESS")
            .await
            .expect("define member");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(watch_writer_tables(Arc::clone(&db), tx));
        tokio::time::sleep(Duration::from_millis(500)).await;
        db.client
            .query("CREATE member:liveprobe SET org_role = 'member', is_active = true")
            .await
            .expect("create");
        let notice = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        assert!(notice.expect("live timeout").is_some());
    }

    #[test]
    fn live_queries_project_the_record_id() {
        for table in ["member", "plays_on", "team_channel"] {
            let sql = live_sql(table);
            assert!(
                sql.starts_with("LIVE SELECT id FROM "),
                "{table} live query must project id only: {sql}"
            );
            assert!(!sql.contains('*'), "{sql}");
        }
    }

    #[tokio::test]
    async fn live_notices_omit_secret_fields() {
        use futures_util::StreamExt;

        let secret = "sekrit-nostr-blob-do-not-leak";
        for table in ["member", "plays_on", "team_channel"] {
            let db = mem(&format!("relay-policy-live-{table}")).await;
            db.client
                .query(format!("DEFINE TABLE {table} SCHEMALESS"))
                .await
                .expect("define");
            let mut response = db
                .client
                .query(live_sql(table))
                .await
                .unwrap_or_else(|err| panic!("{table} live query failed: {err}"));
            let mut stream = response
                .stream::<surrealdb::Notification<surrealdb::types::Value>>(0)
                .unwrap_or_else(|err| panic!("{table} live stream failed: {err}"));
            let record = format!("{table}:secretprobe");
            db.client
                .query(format!(
                    "CREATE {record} SET nostr_secret_key_encrypted = $secret, note = $secret"
                ))
                .bind(("secret", secret))
                .await
                .unwrap_or_else(|err| panic!("{table} create failed: {err}"));
            let stored = db
                .client
                .query(format!("SELECT * FROM {record}"))
                .await
                .expect("select star");
            let stored = format!("{stored:?}");
            assert!(
                stored.contains(secret),
                "{table} fixture must store the secret so a SELECT * live query would fail this test: {stored}"
            );
            let notice = tokio::time::timeout(Duration::from_secs(5), stream.next())
                .await
                .unwrap_or_else(|_| panic!("{table} live timeout"))
                .unwrap_or_else(|| panic!("{table} live stream ended"))
                .unwrap_or_else(|err| panic!("{table} live notice: {err}"));
            let rendered = format!("{:?}", notice.data);
            assert!(
                !rendered.contains(secret),
                "{table} live notice carried a secret field: {rendered}"
            );
            assert!(
                rendered.contains("secretprobe"),
                "{table} live notice should still name the record: {rendered}"
            );
        }
    }
}
