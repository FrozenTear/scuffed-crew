use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use surrealdb::engine::any::Any;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb::Surreal;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{NamedRosterEntry, RosterEntry, TeamRole};
use crate::{with_timeout, Database, DbResult};

/// Raw DB result from a RELATE / graph query on plays_on.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbRosterEntry {
    id: Option<String>,
    #[serde(rename = "in")]
    #[surreal(rename = "in")]
    in_id: Option<String>,
    out: Option<String>,
    team_role: String,
    joined_at: SurrealDatetime,
    is_active: bool,
}

fn parse_team_role(s: &str) -> TeamRole {
    match s {
        "captain" => TeamRole::Captain,
        "player" => TeamRole::Player,
        "sub" => TeamRole::Sub,
        "coach" => TeamRole::Coach,
        _ => TeamRole::Player,
    }
}

fn extract_record_id(thing_str: &str) -> String {
    // SurrealDB returns IDs like "member:abc" or "team:xyz" — extract after colon
    thing_str
        .split_once(':')
        .map(|(_, id)| id.to_string())
        .unwrap_or_else(|| thing_str.to_string())
}

/// Raw DB result from the roster-with-names join on plays_on.
/// `member_id` arrives as the full record-string (`member:abc`) via `<string>in`.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbNamedRosterEntry {
    member_id: String,
    team_role: String,
    joined_at: SurrealDatetime,
    member_name: Option<String>,
    avatar_url: Option<String>,
}

fn db_to_named_roster_entry(db: DbNamedRosterEntry) -> NamedRosterEntry {
    NamedRosterEntry {
        member_id: extract_record_id(&db.member_id),
        member_name: db.member_name,
        avatar_url: db.avatar_url,
        team_role: parse_team_role(&db.team_role),
        joined_at: db.joined_at.into(),
    }
}

fn db_to_roster_entry(db: DbRosterEntry) -> RosterEntry {
    let id = db
        .id
        .as_deref()
        .map(extract_record_id)
        .unwrap_or_else(|| "unknown".to_string());
    let member_id = db
        .in_id
        .as_deref()
        .map(extract_record_id)
        .unwrap_or_default();
    let team_id = db.out.as_deref().map(extract_record_id).unwrap_or_default();

    RosterEntry {
        id,
        member_id,
        team_id,
        team_role: parse_team_role(&db.team_role),
        joined_at: db.joined_at.into(),
        is_active: db.is_active,
    }
}

fn member_rid(id: &str) -> RecordId {
    RecordId::new("member", id)
}

fn team_rid(id: &str) -> RecordId {
    RecordId::new("team", id)
}

/// UNIQUE (in, out) on `plays_on`: one roster edge per (member, team) (DB7).
///
/// Not in the schema script: [`migrate_plays_on_pair_index`] defines it only
/// after older duplicate edges are collapsed, since defining it over
/// duplicates fails.
pub(crate) const PLAYS_ON_PAIR_INDEX: &str = "plays_on_member_team_idx";

/// A lost add race retries once, then reuses the edge the winner wrote.
const ADD_ATTEMPTS: u32 = 2;

/// True when `err` is the unique-index rejection for [`PLAYS_ON_PAIR_INDEX`],
/// or a transaction conflict from two adds writing the same pair at once.
/// SurrealDB v3 reports the index case as `IndexExists` / "already contains".
fn is_plays_on_pair_conflict(err: &surrealdb::Error) -> bool {
    is_plays_on_pair_conflict_text(&err.to_string())
}

/// Text check behind [`is_plays_on_pair_conflict`], split out so the known
/// SurrealDB 3.x messages can be tested without staging a real race.
fn is_plays_on_pair_conflict_text(text: &str) -> bool {
    let msg = text.to_ascii_lowercase();
    msg.contains(PLAYS_ON_PAIR_INDEX)
        || msg.contains("already contains")
        || msg.contains("indexexists")
        || msg.contains("can be retried")
        || msg.contains("write conflict")
}

/// The error to report from a failed BEGIN/COMMIT query, if any.
///
/// SurrealDB 3.x rewrites every slot before the failing one to a generic
/// "not executed due to a failed transaction", and puts a commit conflict
/// only on the COMMIT row, so the first error (what `check()` returns) can
/// hide the conflict. Prefer a pair conflict so [`ADD_ATTEMPTS`] still
/// retries, then the first specific error, then the first error.
fn pick_txn_error(errors: HashMap<usize, surrealdb::Error>) -> Option<surrealdb::Error> {
    if errors.is_empty() {
        return None;
    }
    let mut errors: Vec<(usize, surrealdb::Error)> = errors.into_iter().collect();
    errors.sort_by_key(|(slot, _)| *slot);
    let pos = errors
        .iter()
        .position(|(_, e)| is_plays_on_pair_conflict(e))
        .or_else(|| {
            errors
                .iter()
                .position(|(_, e)| !is_txn_not_executed_text(&e.to_string()))
        })
        .unwrap_or(0);
    Some(errors.swap_remove(pos).1)
}

/// True for the placeholder SurrealDB 3.x writes into the slots of a failed
/// or cancelled transaction that did not cause the failure.
fn is_txn_not_executed_text(text: &str) -> bool {
    let msg = text.to_ascii_lowercase();
    msg.contains("not executed due to a failed transaction")
        || msg.contains("not executed due to a cancelled transaction")
        || msg.contains("cannot commit: the transaction was aborted")
}

/// State of [`PLAYS_ON_PAIR_INDEX`] as `INFO FOR TABLE plays_on` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PairIndexState {
    /// No index with that name.
    Missing,
    /// An index with that name exists but is not UNIQUE on (in, out), for
    /// example one created by hand. It does not enforce DB7.
    Wrong,
    /// UNIQUE on (in, out): the dedupe already ran.
    Unique,
}

fn columns_are_in_and_out(names: &[String]) -> bool {
    names.len() == 2
        && names.iter().any(|name| name == "in")
        && names.iter().any(|name| name == "out")
}

fn pair_define_is_unique(define: &str) -> bool {
    use crate::queries::personal_stats::{define_column_names, sql_token};
    let has_unique = define
        .split_whitespace()
        .any(|word| sql_token(word).eq_ignore_ascii_case("UNIQUE"));
    has_unique && columns_are_in_and_out(&define_column_names(define))
}

/// Same shapes as the personal_match index check: v3 reports an index as
/// its DEFINE statement, or as an object with a `unique` flag and columns.
fn info_pair_index_state(info: &serde_json::Value) -> PairIndexState {
    let Some(defn) = info
        .get("indexes")
        .or_else(|| info.get("indices"))
        .and_then(|idx| idx.get(PLAYS_ON_PAIR_INDEX))
    else {
        return PairIndexState::Missing;
    };
    let unique = match defn {
        serde_json::Value::String(define) => pair_define_is_unique(define),
        serde_json::Value::Object(map) => {
            let columns_ok = map.values().any(|value| {
                value.as_array().is_some_and(|cols| {
                    let names = cols
                        .iter()
                        .filter_map(|col| col.as_str().map(|name| name.to_ascii_lowercase()))
                        .collect::<Vec<_>>();
                    columns_are_in_and_out(&names)
                })
            });
            let unique_flag = matches!(map.get("unique"), Some(serde_json::Value::Bool(true)));
            let unique_text = map
                .values()
                .any(|value| value.as_str().is_some_and(pair_define_is_unique));
            (unique_flag && columns_ok) || unique_text
        }
        _ => false,
    };
    if unique {
        PairIndexState::Unique
    } else {
        PairIndexState::Wrong
    }
}

pub(crate) async fn plays_on_pair_index_state(client: &Surreal<Any>) -> DbResult<PairIndexState> {
    let mut res = client.query("INFO FOR TABLE plays_on").await?.check()?;
    let info: Option<serde_json::Value> = res.take(0)?;
    Ok(info
        .as_ref()
        .map_or(PairIndexState::Missing, info_pair_index_state))
}

/// True when [`PLAYS_ON_PAIR_INDEX`] is defined UNIQUE on (in, out).
pub(crate) async fn plays_on_pair_index_exists(client: &Surreal<Any>) -> DbResult<bool> {
    Ok(plays_on_pair_index_state(client).await? == PairIndexState::Unique)
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DupPair {
    #[serde(rename = "in")]
    #[surreal(rename = "in")]
    member: RecordId,
    out: RecordId,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PairEdge {
    id: RecordId,
    team_role: String,
    is_active: bool,
    joined_at: SurrealDatetime,
}

impl PairEdge {
    /// One log line's worth of the edge, so a deleted edge can be rebuilt
    /// from the boot log without a restore.
    fn describe(&self) -> String {
        let at: chrono::DateTime<chrono::Utc> = self.joined_at.into();
        format!(
            "plays_on:{} team_role={} is_active={} joined_at={}",
            crate::record_id_key_to_string(self.id.key.clone()),
            self.team_role,
            self.is_active,
            at.to_rfc3339()
        )
    }
}

/// Which edge of a duplicate (member, team) group survives the dedupe:
/// an active edge over an inactive one (nobody drops off a roster), then the
/// newest `joined_at` (the latest add, so its role), then the greatest
/// record key so the choice is deterministic.
fn pick_keeper(edges: &[PairEdge]) -> Option<&PairEdge> {
    edges.iter().max_by(|a, b| {
        let a_at: chrono::DateTime<chrono::Utc> = a.joined_at.into();
        let b_at: chrono::DateTime<chrono::Utc> = b.joined_at.into();
        a.is_active
            .cmp(&b.is_active)
            .then(a_at.cmp(&b_at))
            .then_with(|| {
                crate::record_id_key_to_string(a.id.key.clone())
                    .cmp(&crate::record_id_key_to_string(b.id.key.clone()))
            })
    })
}

/// DB7: collapse duplicate `plays_on` edges, then define
/// [`PLAYS_ON_PAIR_INDEX`] UNIQUE on (in, out).
///
/// Steady state: the index exists UNIQUE on (in, out) and this is one
/// `INFO FOR TABLE` read. An index with the same name that is not UNIQUE on
/// (in, out) is removed and re-defined after the dedupe.
///
/// First boot with this code: every (member, team) pair with more than one
/// edge keeps one edge (see [`pick_keeper`]) and the rest are deleted. The
/// keeper's role, `joined_at` and `is_active` are left as stored; nothing is
/// re-activated. Pairs with a single edge are not written. Any failure fails
/// startup, because the add path relies on the index. Each group logs its
/// kept edge and every deleted edge (id, role, flag, `joined_at`) at info,
/// because the deletes are permanent.
///
/// There is no transaction around the whole pass: each group's DELETE
/// commits on its own. If a later step fails, the next boot finds no index,
/// re-runs the dedupe over what is left and picks the same keepers.
///
/// Operator pre-check (read-only):
///
/// ```surql
/// SELECT * FROM (
///     SELECT in, out, count() AS n FROM plays_on GROUP BY in, out
/// ) WHERE n > 1;
/// ```
pub(crate) async fn migrate_plays_on_pair_index(client: &Surreal<Any>) -> DbResult<()> {
    match plays_on_pair_index_state(client).await? {
        PairIndexState::Unique => {
            tracing::info!(
                "{PLAYS_ON_PAIR_INDEX} already UNIQUE on (in, out); skipping plays_on dedupe"
            );
            return Ok(());
        }
        PairIndexState::Wrong => {
            tracing::warn!(
                "{PLAYS_ON_PAIR_INDEX} exists but is not UNIQUE on (in, out); \
                 removing it, then deduping and re-defining it"
            );
            client
                .query(format!(
                    "REMOVE INDEX IF EXISTS {PLAYS_ON_PAIR_INDEX} ON plays_on"
                ))
                .await?
                .check()?;
        }
        PairIndexState::Missing => {}
    }

    let mut res = client
        .query(
            "SELECT in, out FROM ( \
                 SELECT in, out, count() AS n FROM plays_on GROUP BY in, out \
             ) WHERE n > 1",
        )
        .await?
        .check()?;
    let groups: Vec<DupPair> = res.take(0)?;

    let mut deleted = 0usize;
    for group in &groups {
        let mut res = client
            .query(
                "SELECT id, team_role, is_active, joined_at FROM plays_on \
                 WHERE in = $m AND out = $t",
            )
            .bind(("m", group.member.clone()))
            .bind(("t", group.out.clone()))
            .await?
            .check()?;
        let edges: Vec<PairEdge> = res.take(0)?;
        let Some(keeper) = pick_keeper(&edges) else {
            continue;
        };
        let loser_edges: Vec<&PairEdge> = edges.iter().filter(|e| e.id != keeper.id).collect();
        if loser_edges.is_empty() {
            continue;
        }
        let losers: Vec<RecordId> = loser_edges.iter().map(|e| e.id.clone()).collect();
        let deleted_desc: Vec<String> = loser_edges.iter().map(|e| e.describe()).collect();
        tracing::info!(
            kept = %keeper.describe(),
            deleted = ?deleted_desc,
            "plays_on dedupe: deleting duplicate edges for one (member, team)"
        );
        deleted += losers.len();
        client
            .query("DELETE $losers RETURN NONE")
            .bind(("losers", losers))
            .await?
            .check()?;
    }

    client
        .query(format!(
            "DEFINE INDEX IF NOT EXISTS {PLAYS_ON_PAIR_INDEX} ON plays_on COLUMNS in, out UNIQUE"
        ))
        .await?
        .check()?;
    if !plays_on_pair_index_exists(client).await? {
        return Err(crate::DbError::Config(format!(
            "{PLAYS_ON_PAIR_INDEX} is not UNIQUE on (in, out) after DEFINE"
        )));
    }

    tracing::info!(
        duplicate_pairs = groups.len(),
        deleted,
        "plays_on is UNIQUE on (in, out)"
    );
    Ok(())
}

/// Roster rows that are safe to show and to fan out.
///
/// The edge must be active, the member row must be active, and the member must
/// not have an active ban. `moderation_action.member_id` is the bare key, so
/// compare it with `meta::id(in)` (same normalisation as `count_actionable_admins`).
/// A suspension is not a ban: those members stay active and stay on the roster.
macro_rules! roster_select {
    ($select:literal) => {
        concat!(
            $select,
            " AND is_active = true ",
            "AND in.is_active = true ",
            "AND meta::id(in) NOT IN ( ",
            "SELECT VALUE member_id FROM moderation_action ",
            "WHERE is_active = true ",
            "AND action_type = 'ban' ",
            "AND (expires_at IS NONE OR expires_at > time::now()) ",
            ")"
        )
    };
}

impl Database {
    /// Add a member to a team's roster.
    ///
    /// There is at most one `plays_on` edge per (member, team), enforced by
    /// [`PLAYS_ON_PAIR_INDEX`] (DB7). When the pair already has an edge it is
    /// reused instead of relating a second one:
    /// - an inactive edge (removed earlier) is re-activated with the new role
    ///   and `joined_at` restarts, because the member is re-joining;
    /// - an active edge (a double click, or a re-add) keeps its `joined_at`
    ///   and takes the requested role.
    ///
    /// The read and the write run in one transaction. Two concurrent adds can
    /// both see no edge; the unique index or a commit conflict fails the
    /// second, and that call retries once and then finds the edge. Every
    /// success re-reads the edge for the pair, so all callers get its id.
    pub async fn add_to_roster(
        &self,
        member_id: &str,
        team_id: &str,
        role: TeamRole,
    ) -> DbResult<RosterEntry> {
        with_timeout(async {
            let mut attempt = 0;
            loop {
                attempt += 1;
                match self.upsert_roster_edge(member_id, team_id, role).await {
                    Err(crate::DbError::Surreal(e))
                        if attempt < ADD_ATTEMPTS && is_plays_on_pair_conflict(&e) =>
                    {
                        tracing::debug!(
                            member_id,
                            team_id,
                            "roster add lost a race for the (member, team) edge; retrying"
                        );
                    }
                    other => return other,
                }
            }
        })
        .await
    }

    async fn upsert_roster_edge(
        &self,
        member_id: &str,
        team_id: &str,
        role: TeamRole,
    ) -> DbResult<RosterEntry> {
        // SurrealDB 3.x computes every SET right-hand side against the stored
        // row before applying any of them, so the IF reads the stored
        // is_active whatever the assignment order: an active edge keeps its
        // join time, a removed one restarts it. Do not rely on SET order.
        //
        // LIMIT 1 without ORDER BY is safe because PLAYS_ON_PAIR_INDEX
        // (UNIQUE on in, out) allows at most one row for the pair.
        //
        // The transaction makes the read and the write commit together or
        // not at all, so a losing add fails (and retries) instead of
        // reporting an edge that did not survive.
        let mut response = self
            .client
            .query(
                r#"BEGIN TRANSACTION;
                   LET $existing = (SELECT VALUE id FROM plays_on
                       WHERE in = $member_rid AND out = $team_rid LIMIT 1);
                   LET $entry = IF array::len($existing) > 0 {
                       UPDATE $existing[0] SET
                           joined_at = IF is_active THEN joined_at ELSE time::now() END,
                           team_role = $role,
                           is_active = true
                   } ELSE {
                       RELATE $member_rid -> plays_on -> $team_rid
                       SET team_role = $role, joined_at = time::now(), is_active = true
                   };
                   SELECT *, meta::id(id) as id, <string>in as in, <string>out as out
                   FROM $entry;
                   COMMIT TRANSACTION;"#,
            )
            .bind(("member_rid", member_rid(member_id)))
            .bind(("team_rid", team_rid(team_id)))
            .bind(("role", role.to_string()))
            .await?;
        if let Some(e) = pick_txn_error(response.take_errors()) {
            return Err(e.into());
        }
        // Return the edge stored for the pair, not the one this call wrote,
        // so every caller gets the surviving edge id.
        self.get_roster_edge(member_id, team_id)
            .await?
            .ok_or_else(|| crate::DbError::NotFound("Failed to create roster entry".into()))
    }

    /// The single `plays_on` edge for (member, team), active or not.
    async fn get_roster_edge(
        &self,
        member_id: &str,
        team_id: &str,
    ) -> DbResult<Option<RosterEntry>> {
        let mut result = self
            .client
            .query(
                "SELECT *, meta::id(id) as id, <string>in as in, <string>out as out \
                 FROM plays_on WHERE in = $member_rid AND out = $team_rid LIMIT 1",
            )
            .bind(("member_rid", member_rid(member_id)))
            .bind(("team_rid", team_rid(team_id)))
            .await?
            .check()?;
        let entries: Vec<DbRosterEntry> = result.take(0)?;
        Ok(entries.into_iter().next().map(db_to_roster_entry))
    }

    /// Get a team's roster: active edges to active members who are not banned.
    ///
    /// Public roster pages, `roster_count`, officer gift-wrap recipients, and
    /// NIP-29 `sync_team_roster` all read this. A ban leaves the edge in place
    /// and (until lifted) leaves `member.is_active` false; both are filtered,
    /// and a ban row also excludes a member whose `is_active` was set back on.
    pub async fn get_team_roster(&self, team_id: &str) -> DbResult<Vec<RosterEntry>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(roster_select!(
                    r#"SELECT *, meta::id(id) as id, <string>in as in, <string>out as out
                       FROM plays_on
                       WHERE out = $team_rid"#
                ))
                .bind(("team_rid", team_rid(team_id)))
                .await?;
            let entries: Vec<DbRosterEntry> = result.take(0)?;
            Ok(entries.into_iter().map(db_to_roster_entry).collect())
        })
        .await
    }

    /// Get a team's active roster with each member's display name and avatar
    /// joined in from the linked `member` row — one query, no per-member N+1.
    ///
    /// Traverses the `plays_on` edge: `in` is the member record, so `in.display_name`
    /// / `in.avatar_url` dereference the link inline. Only `display_name` and
    /// `avatar_url` are projected — the encrypted nostr secret never loads here.
    /// A dangling edge (member row gone) yields `None` for both, which callers
    /// surface as "Unknown" plus a warning.
    pub async fn get_team_roster_named(&self, team_id: &str) -> DbResult<Vec<NamedRosterEntry>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(roster_select!(
                    r#"SELECT <string>in AS member_id, team_role, joined_at,
                       in.display_name AS member_name, in.avatar_url AS avatar_url
                       FROM plays_on
                       WHERE out = $team_rid"#
                ))
                .bind(("team_rid", team_rid(team_id)))
                .await?;
            let entries: Vec<DbNamedRosterEntry> = result.take(0)?;
            Ok(entries.into_iter().map(db_to_named_roster_entry).collect())
        })
        .await
    }

    /// Active roster edges as `(member_id, team_id)`.
    ///
    /// Same active-member and active-ban filter as [`Self::get_team_roster`].
    /// The relay read gate builds its team map from this instead of one query
    /// per team.
    pub async fn list_active_roster_pairs(&self) -> DbResult<Vec<(String, String)>> {
        #[derive(Debug, Deserialize, SurrealValue)]
        struct Pair {
            member_id: String,
            team_id: String,
        }

        with_timeout(async {
            let mut result = self
                .client
                .query(roster_select!(
                    "SELECT <string>in AS member_id, <string>out AS team_id FROM plays_on WHERE true"
                ))
                .await?;
            let rows: Vec<Pair> = result.take(0)?;
            Ok(rows
                .into_iter()
                .map(|row| {
                    (
                        extract_record_id(&row.member_id),
                        extract_record_id(&row.team_id),
                    )
                })
                .collect())
        })
        .await
    }

    /// Get all teams a member is on.
    pub async fn get_member_teams(&self, member_id: &str) -> DbResult<Vec<RosterEntry>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    r#"SELECT *, meta::id(id) as id, <string>in as in, <string>out as out
                       FROM plays_on
                       WHERE in = $member_rid AND is_active = true"#,
                )
                .bind(("member_rid", member_rid(member_id)))
                .await?;
            let entries: Vec<DbRosterEntry> = result.take(0)?;
            Ok(entries.into_iter().map(db_to_roster_entry).collect())
        })
        .await
    }

    /// True if `member_id` is an active roster entry on `team_id`.
    pub async fn is_on_team_roster(&self, member_id: &str, team_id: &str) -> DbResult<bool> {
        with_timeout(async {
            #[derive(Debug, Deserialize, SurrealValue)]
            struct CountResult {
                count: i64,
            }
            let mut result = self
                .client
                .query(
                    r#"SELECT count() FROM plays_on
                       WHERE in = $member_rid AND out = $team_rid AND is_active = true
                       GROUP ALL"#,
                )
                .bind(("member_rid", member_rid(member_id)))
                .bind(("team_rid", team_rid(team_id)))
                .await?;
            let rows: Vec<CountResult> = result.take(0)?;
            Ok(rows.first().map(|r| r.count > 0).unwrap_or(false))
        })
        .await
    }

    /// Update a member's role on a team. Returns the updated entry, or `None`
    /// when the member has no active roster edge on the team (nothing updated).
    pub async fn update_roster_role(
        &self,
        member_id: &str,
        team_id: &str,
        new_role: TeamRole,
    ) -> DbResult<Option<RosterEntry>> {
        with_timeout(async {
            // UPDATE then SELECT back with string casts for proper deserialization
            let mut result = self
                .client
                .query(
                    r#"UPDATE plays_on SET team_role = $role
                       WHERE in = $member_rid
                       AND out = $team_rid
                       AND is_active = true;
                       SELECT *, meta::id(id) as id, <string>in as in, <string>out as out
                       FROM plays_on
                       WHERE in = $member_rid
                       AND out = $team_rid
                       AND is_active = true"#,
                )
                .bind(("role", new_role.to_string()))
                .bind(("member_rid", member_rid(member_id)))
                .bind(("team_rid", team_rid(team_id)))
                .await?
                .check()?;
            let entries: Vec<DbRosterEntry> = result.take(1)?;
            Ok(entries.into_iter().next().map(db_to_roster_entry))
        })
        .await
    }

    /// Remove a member from a team (soft delete).
    pub async fn remove_from_roster(&self, member_id: &str, team_id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query(
                    r#"UPDATE plays_on SET is_active = false
                       WHERE in = $member_rid
                       AND out = $team_rid"#,
                )
                .bind(("member_rid", member_rid(member_id)))
                .bind(("team_rid", team_rid(team_id)))
                .await?
                .check()?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use crate::Database;

    async fn seeded_db() -> Database {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        db.client
            .query(
                r#"CREATE member:alice SET user_id = 'ua', org_role = 'member',
                       display_name = 'Alice', avatar_url = 'https://cdn/a.png';
                   CREATE member:bob SET user_id = 'ub', org_role = 'member',
                       display_name = 'Bob';
                   CREATE team:t1 SET name = 'T1', game_id = 'g1';"#,
            )
            .await
            .expect("seed rows");
        db
    }

    #[tokio::test]
    async fn named_roster_joins_member_fields() {
        let db = seeded_db().await;
        db.add_to_roster("alice", "t1", TeamRole::Captain)
            .await
            .expect("add alice");
        db.add_to_roster("bob", "t1", TeamRole::Player)
            .await
            .expect("add bob");

        let mut roster = db.get_team_roster_named("t1").await.expect("join");
        roster.sort_by(|a, b| a.member_id.cmp(&b.member_id));
        assert_eq!(roster.len(), 2);

        let alice = &roster[0];
        assert_eq!(alice.member_id, "alice");
        assert_eq!(alice.member_name.as_deref(), Some("Alice"));
        assert_eq!(alice.avatar_url.as_deref(), Some("https://cdn/a.png"));
        assert_eq!(alice.team_role, TeamRole::Captain);

        let bob = &roster[1];
        assert_eq!(bob.member_id, "bob");
        assert_eq!(bob.member_name.as_deref(), Some("Bob"));
        assert_eq!(bob.avatar_url, None);
    }

    #[tokio::test]
    async fn named_roster_excludes_inactive_edges() {
        let db = seeded_db().await;
        db.add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .expect("add alice");
        db.add_to_roster("bob", "t1", TeamRole::Player)
            .await
            .expect("add bob");
        // Soft-delete bob's edge — the is_active = true filter must drop it.
        db.remove_from_roster("bob", "t1")
            .await
            .expect("remove bob");

        let roster = db.get_team_roster_named("t1").await.expect("join");
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].member_id, "alice");
    }

    #[tokio::test]
    async fn update_roster_role_returns_updated_entry() {
        let db = seeded_db().await;
        db.add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .expect("add alice");

        let updated = db
            .update_roster_role("alice", "t1", TeamRole::Coach)
            .await
            .expect("update role")
            .expect("alice is on the roster");
        assert_eq!(updated.member_id, "alice");
        assert_eq!(updated.team_id, "t1");
        assert_eq!(updated.team_role, TeamRole::Coach);

        let roster = db.get_team_roster_named("t1").await.expect("join");
        assert_eq!(roster[0].team_role, TeamRole::Coach);
    }

    #[tokio::test]
    async fn update_roster_role_none_when_not_on_roster() {
        let db = seeded_db().await;
        let updated = db
            .update_roster_role("bob", "t1", TeamRole::Coach)
            .await
            .expect("update runs");
        assert!(updated.is_none(), "no active edge => None");

        // Inactive edge must not be updated either.
        db.add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .expect("add alice");
        db.remove_from_roster("alice", "t1")
            .await
            .expect("soft delete");
        let updated = db
            .update_roster_role("alice", "t1", TeamRole::Coach)
            .await
            .expect("update runs");
        assert!(updated.is_none(), "inactive edge => None");
    }

    #[tokio::test]
    async fn named_roster_omits_dangling_edge() {
        let db = seeded_db().await;
        // An edge whose `in` points at a member id with no backing row. There is
        // no active member to show, so the roster omits it.
        db.add_to_roster("ghost", "t1", TeamRole::Player)
            .await
            .expect("relate ghost edge");

        let named = db.get_team_roster_named("t1").await.expect("join");
        assert!(named.is_empty(), "dangling edge is not a member: {named:?}");
        let plain = db.get_team_roster("t1").await.expect("plain");
        assert!(plain.is_empty(), "dangling edge is not a member: {plain:?}");
    }

    #[tokio::test]
    async fn roster_omits_inactive_and_banned_members() {
        let db = seeded_db().await;
        db.client
            .query(
                r#"CREATE member:gone SET user_id = 'ug', org_role = 'member',
                       display_name = 'Gone', is_active = false;
                   CREATE member:banned SET user_id = 'uban', org_role = 'officer',
                       display_name = 'Banned', is_active = true;
                   CREATE member:suspended SET user_id = 'usus', org_role = 'officer',
                       display_name = 'Suspended', is_active = true;"#,
            )
            .await
            .expect("seed extra members");
        for (id, role) in [
            ("alice", TeamRole::Captain),
            ("gone", TeamRole::Player),
            ("banned", TeamRole::Player),
            ("suspended", TeamRole::Coach),
        ] {
            db.add_to_roster(id, "t1", role).await.expect("add");
        }
        db.create_moderation_action(
            "banned",
            crate::types::ModerationActionType::Ban,
            "roster",
            "alice",
            None,
        )
        .await
        .expect("ban");
        db.create_moderation_action(
            "suspended",
            crate::types::ModerationActionType::Suspension,
            "temp",
            "alice",
            None,
        )
        .await
        .expect("suspend");

        let mut named = db.get_team_roster_named("t1").await.expect("named");
        named.sort_by(|a, b| a.member_id.cmp(&b.member_id));
        let ids: Vec<_> = named.iter().map(|e| e.member_id.as_str()).collect();
        assert_eq!(
            ids,
            ["alice", "suspended"],
            "inactive and banned members stay off the roster; a suspension does not"
        );

        let mut plain = db.get_team_roster("t1").await.expect("plain");
        plain.sort_by(|a, b| a.member_id.cmp(&b.member_id));
        let plain_ids: Vec<_> = plain.iter().map(|e| e.member_id.as_str()).collect();
        assert_eq!(plain_ids, ["alice", "suspended"]);
    }

    // ---- DB7: one plays_on edge per (member, team) ----

    async fn edge_count(db: &Database, member: &str, team: &str) -> i64 {
        #[derive(Debug, Deserialize, SurrealValue)]
        struct N {
            n: i64,
        }
        let mut res = db
            .client
            .query("SELECT count() AS n FROM plays_on WHERE in = $m AND out = $t GROUP ALL")
            .bind(("m", member_rid(member)))
            .bind(("t", team_rid(team)))
            .await
            .expect("count")
            .check()
            .expect("count ok");
        let rows: Vec<N> = res.take(0).expect("take");
        rows.first().map(|r| r.n).unwrap_or(0)
    }

    async fn total_edges(db: &Database) -> i64 {
        #[derive(Debug, Deserialize, SurrealValue)]
        struct N {
            n: i64,
        }
        let mut res = db
            .client
            .query("SELECT count() AS n FROM plays_on GROUP ALL")
            .await
            .expect("count")
            .check()
            .expect("count ok");
        let rows: Vec<N> = res.take(0).expect("take");
        rows.first().map(|r| r.n).unwrap_or(0)
    }

    /// RELATE an edge with a fixed key, role, `joined_at` and flag, as older
    /// builds could write it.
    async fn seed_legacy_edge(
        db: &Database,
        key: &str,
        member: &str,
        team: &str,
        role: &str,
        joined_at: &str,
        active: bool,
    ) {
        db.client
            .query(
                "RELATE $m -> $edge -> $t SET team_role = $role, \
                 joined_at = <datetime>$joined, is_active = $active",
            )
            .bind(("m", member_rid(member)))
            .bind(("edge", RecordId::new("plays_on", key)))
            .bind(("t", team_rid(team)))
            .bind(("role", role.to_string()))
            .bind(("joined", joined_at.to_string()))
            .bind(("active", active))
            .await
            .expect("relate legacy")
            .check()
            .expect("relate legacy ok");
    }

    /// Drop the unique index so a test can seed the duplicates that older
    /// builds could write, then run migrations again.
    async fn drop_unique_index(db: &Database) {
        db.client
            .query(format!(
                "REMOVE INDEX IF EXISTS {PLAYS_ON_PAIR_INDEX} ON plays_on"
            ))
            .await
            .expect("remove index")
            .check()
            .expect("remove index ok");
    }

    async fn edge_keys(db: &Database, member: &str, team: &str) -> Vec<DbRosterEntry> {
        let mut res = db
            .client
            .query(
                "SELECT *, meta::id(id) as id, <string>in as in, <string>out as out \
                 FROM plays_on WHERE in = $m AND out = $t",
            )
            .bind(("m", member_rid(member)))
            .bind(("t", team_rid(team)))
            .await
            .expect("select")
            .check()
            .expect("select ok");
        res.take(0).expect("take")
    }

    #[tokio::test]
    async fn double_add_keeps_one_edge() {
        let db = seeded_db().await;
        let first = db
            .add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .expect("first add");
        let second = db
            .add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .expect("second add reuses the edge");
        assert_eq!(first.id, second.id, "same edge both times");
        assert_eq!(edge_count(&db, "alice", "t1").await, 1);
        assert_eq!(db.get_team_roster("t1").await.unwrap().len(), 1);
        assert_eq!(db.get_member_teams("alice").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn concurrent_double_add_keeps_one_edge() {
        let db = seeded_db().await;
        let (a, b) = tokio::join!(
            db.add_to_roster("alice", "t1", TeamRole::Player),
            db.add_to_roster("alice", "t1", TeamRole::Player),
        );
        let a = a.expect("first click");
        let b = b.expect("second click");
        assert_eq!(a.id, b.id);
        assert_eq!(edge_count(&db, "alice", "t1").await, 1);
        assert_eq!(db.get_team_roster_named("t1").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn re_add_after_remove_reactivates_the_existing_edge() {
        let db = seeded_db().await;
        let first = db
            .add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .expect("add");
        db.remove_from_roster("alice", "t1").await.expect("remove");
        assert!(db.get_team_roster("t1").await.unwrap().is_empty());

        let back = db
            .add_to_roster("alice", "t1", TeamRole::Captain)
            .await
            .expect("re-add");
        assert_eq!(back.id, first.id, "the old edge is reused");
        assert!(back.is_active);
        assert_eq!(back.team_role, TeamRole::Captain);
        assert!(
            back.joined_at >= first.joined_at,
            "joined_at never moves earlier (see re_add_after_remove_moves_joined_at_later)"
        );
        assert_eq!(edge_count(&db, "alice", "t1").await, 1);
        let roster = db.get_team_roster("t1").await.unwrap();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].team_role, TeamRole::Captain);
    }

    #[tokio::test]
    async fn re_add_after_remove_moves_joined_at_later() {
        let db = seeded_db().await;
        seed_legacy_edge(
            &db,
            "a_old",
            "alice",
            "t1",
            "sub",
            "2026-01-01T00:00:00Z",
            false,
        )
        .await;
        let old: chrono::DateTime<chrono::Utc> = "2026-01-01T00:00:00Z".parse().unwrap();

        let back = db
            .add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .expect("re-add");
        assert_eq!(back.id, "a_old", "the removed edge is reused");
        assert!(back.is_active);
        assert!(
            back.joined_at > old,
            "a re-join restarts joined_at: {}",
            back.joined_at
        );
        assert_eq!(edge_count(&db, "alice", "t1").await, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn many_concurrent_adds_on_threads_keep_one_edge() {
        // One race can pass by luck, so run it 20 times on a fresh DB.
        for run in 0..20 {
            let db = std::sync::Arc::new(seeded_db().await);
            let mut tasks = Vec::new();
            for _ in 0..16 {
                let db = db.clone();
                tasks.push(tokio::spawn(async move {
                    db.add_to_roster("alice", "t1", TeamRole::Player).await
                }));
            }
            let mut ok_ids = Vec::new();
            for task in tasks {
                match task.await.expect("join") {
                    Ok(entry) => ok_ids.push(entry.id),
                    // A call that loses twice may surface the conflict; that
                    // is a retryable error, never a second edge.
                    Err(crate::DbError::Surreal(e)) => {
                        assert!(
                            is_plays_on_pair_conflict(&e),
                            "run {run}: unexpected error: {e}"
                        )
                    }
                    Err(other) => panic!("run {run}: unexpected error: {other}"),
                }
            }
            assert!(!ok_ids.is_empty(), "run {run}: at least one add succeeds");
            // Count first: a second edge is a real duplicate, while one edge
            // with mismatched ids means a call returned the wrong edge.
            assert_eq!(
                edge_count(&db, "alice", "t1").await,
                1,
                "run {run}: one edge for the pair"
            );
            let kept = edge_keys(&db, "alice", "t1").await;
            assert!(
                ok_ids
                    .iter()
                    .all(|id| Some(id.as_str()) == kept[0].id.as_deref()),
                "run {run}: every add returns the surviving edge {:?}: {ok_ids:?}",
                kept[0].id
            );
            assert_eq!(db.get_team_roster("t1").await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn re_add_while_active_keeps_joined_at_and_sets_role() {
        let db = seeded_db().await;
        let first = db
            .add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .expect("add");
        let again = db
            .add_to_roster("alice", "t1", TeamRole::Coach)
            .await
            .expect("add again");
        assert_eq!(again.id, first.id);
        assert_eq!(again.joined_at, first.joined_at, "still on the team");
        assert_eq!(again.team_role, TeamRole::Coach);
    }

    #[tokio::test]
    async fn same_member_on_two_teams_is_allowed() {
        let db = seeded_db().await;
        db.client
            .query("CREATE team:t2 SET name = 'T2', game_id = 'g1'")
            .await
            .unwrap()
            .check()
            .unwrap();
        db.add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .unwrap();
        db.add_to_roster("alice", "t2", TeamRole::Player)
            .await
            .unwrap();
        db.add_to_roster("bob", "t1", TeamRole::Player)
            .await
            .unwrap();
        assert_eq!(db.get_member_teams("alice").await.unwrap().len(), 2);
        assert_eq!(total_edges(&db).await, 3);
    }

    #[tokio::test]
    async fn unique_index_rejects_a_raw_duplicate_relate() {
        let db = seeded_db().await;
        db.add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .unwrap();
        let dup = db
            .client
            .query("RELATE member:alice -> plays_on -> team:t1 SET team_role = 'player'")
            .await
            .unwrap()
            .check();
        let err = dup.expect_err("UNIQUE (in, out) must reject a second edge");
        assert!(
            is_plays_on_pair_conflict(&err),
            "the real rejection must take the retry path: {err}"
        );
        assert_eq!(edge_count(&db, "alice", "t1").await, 1);
    }

    #[test]
    fn pair_conflict_text_matches_surreal_messages() {
        // SurrealDB 3.x texts for a losing RELATE (unique key write, then an
        // optimistic transaction conflict).
        assert!(is_plays_on_pair_conflict_text(
            "Database index `plays_on_member_team_idx` already contains \
             [member:alice, team:t1], with record `plays_on:x`"
        ));
        assert!(is_plays_on_pair_conflict_text(
            "Transaction conflict: Write conflict. This transaction can be retried"
        ));
        assert!(!is_plays_on_pair_conflict_text(
            "db7 injected write failure"
        ));
        assert!(!is_plays_on_pair_conflict_text("Operation timed out"));
    }

    #[test]
    fn txn_placeholder_text_is_not_the_cause() {
        assert!(is_txn_not_executed_text(
            "The query was not executed due to a failed transaction"
        ));
        assert!(is_txn_not_executed_text(
            "The query was not executed due to a cancelled transaction"
        ));
        assert!(is_txn_not_executed_text(
            "Cannot COMMIT: the transaction was aborted due to a prior error"
        ));
        assert!(!is_txn_not_executed_text(
            "Cannot COMMIT: Transaction conflict: write conflict. This transaction can be retried"
        ));
        assert!(!is_txn_not_executed_text(
            "An error occurred: db7 injected write failure"
        ));
    }

    #[test]
    fn index_state_requires_unique_on_in_and_out() {
        let info =
            |define: &str| serde_json::json!({ "indexes": { "plays_on_member_team_idx": define } });
        assert_eq!(
            info_pair_index_state(&info(
                "DEFINE INDEX plays_on_member_team_idx ON plays_on FIELDS in, out UNIQUE"
            )),
            PairIndexState::Unique
        );
        assert_eq!(
            info_pair_index_state(&info(
                "DEFINE INDEX plays_on_member_team_idx ON plays_on FIELDS in, out"
            )),
            PairIndexState::Wrong
        );
        assert_eq!(
            info_pair_index_state(&info(
                "DEFINE INDEX plays_on_member_team_idx ON plays_on FIELDS in UNIQUE"
            )),
            PairIndexState::Wrong
        );
        assert_eq!(
            info_pair_index_state(&serde_json::json!({ "indexes": {} })),
            PairIndexState::Missing
        );
    }

    #[tokio::test]
    async fn migration_replaces_a_non_unique_index_with_the_same_name() {
        let db = seeded_db().await;
        drop_unique_index(&db).await;
        db.client
            .query(format!(
                "DEFINE INDEX {PLAYS_ON_PAIR_INDEX} ON plays_on COLUMNS in, out"
            ))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(
            plays_on_pair_index_state(&db.client).await.unwrap(),
            PairIndexState::Wrong
        );
        for key in ["w_1", "w_2"] {
            seed_legacy_edge(
                &db,
                key,
                "alice",
                "t1",
                "player",
                "2026-01-01T00:00:00Z",
                true,
            )
            .await;
        }

        run_migrations(&db.client).await.expect("boot");
        assert_eq!(edge_count(&db, "alice", "t1").await, 1);
        assert_eq!(
            plays_on_pair_index_state(&db.client).await.unwrap(),
            PairIndexState::Unique
        );
    }

    #[tokio::test]
    async fn migration_joined_at_tie_keeps_the_greatest_key() {
        let db = seeded_db().await;
        drop_unique_index(&db).await;
        for key in ["tie_b", "tie_a"] {
            seed_legacy_edge(
                &db,
                key,
                "alice",
                "t1",
                "player",
                "2026-02-01T00:00:00Z",
                true,
            )
            .await;
        }
        run_migrations(&db.client).await.expect("boot");
        let alice = edge_keys(&db, "alice", "t1").await;
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].id.as_deref(), Some("tie_b"));
    }

    #[tokio::test]
    async fn add_to_roster_returns_err_when_the_update_fails() {
        let db = seeded_db().await;
        db.add_to_roster("alice", "t1", TeamRole::Player)
            .await
            .expect("add");
        db.client
            .query(
                "DEFINE EVENT OVERWRITE db7_fail ON plays_on WHEN $event = 'UPDATE' \
                 THEN { THROW 'db7 injected write failure' }",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let res = db.add_to_roster("alice", "t1", TeamRole::Coach).await;
        assert!(
            res.is_err(),
            "a failed UPDATE must not report success: {res:?}"
        );
        db.client
            .query("REMOVE EVENT db7_fail ON plays_on")
            .await
            .unwrap()
            .check()
            .unwrap();
        let roster = db.get_team_roster("t1").await.unwrap();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].team_role, TeamRole::Player, "role unchanged");
    }

    #[tokio::test]
    async fn migration_dedupes_legacy_edges_then_installs_the_index() {
        let db = seeded_db().await;
        drop_unique_index(&db).await;
        db.client
            .query("CREATE member:carol SET user_id = 'uc', org_role = 'member', display_name = 'Carol'")
            .await
            .unwrap()
            .check()
            .unwrap();

        // alice: double click (two active), plus an older removed edge.
        seed_legacy_edge(
            &db,
            "a_old",
            "alice",
            "t1",
            "sub",
            "2026-01-01T00:00:00Z",
            false,
        )
        .await;
        seed_legacy_edge(
            &db,
            "a_one",
            "alice",
            "t1",
            "player",
            "2026-02-01T00:00:00Z",
            true,
        )
        .await;
        seed_legacy_edge(
            &db,
            "a_two",
            "alice",
            "t1",
            "captain",
            "2026-02-01T00:00:01Z",
            true,
        )
        .await;
        // bob: removed, then re-added. The active edge wins over a newer inactive one.
        seed_legacy_edge(
            &db,
            "b_new_off",
            "bob",
            "t1",
            "coach",
            "2026-03-01T00:00:00Z",
            false,
        )
        .await;
        seed_legacy_edge(
            &db,
            "b_on",
            "bob",
            "t1",
            "player",
            "2026-02-15T00:00:00Z",
            true,
        )
        .await;
        // carol: only inactive duplicates. The newest stays, still inactive.
        seed_legacy_edge(
            &db,
            "c_1",
            "carol",
            "t1",
            "player",
            "2026-01-01T00:00:00Z",
            false,
        )
        .await;
        seed_legacy_edge(
            &db,
            "c_2",
            "carol",
            "t1",
            "sub",
            "2026-01-02T00:00:00Z",
            false,
        )
        .await;
        assert_eq!(total_edges(&db).await, 7);

        run_migrations(&db.client).await.expect("dedupe boot");

        let alice = edge_keys(&db, "alice", "t1").await;
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].id.as_deref(), Some("a_two"));
        assert!(alice[0].is_active);
        assert_eq!(alice[0].team_role, "captain");

        let bob = edge_keys(&db, "bob", "t1").await;
        assert_eq!(bob.len(), 1);
        assert_eq!(bob[0].id.as_deref(), Some("b_on"));
        assert!(bob[0].is_active);

        let carol = edge_keys(&db, "carol", "t1").await;
        assert_eq!(carol.len(), 1);
        assert_eq!(carol[0].id.as_deref(), Some("c_2"));
        assert!(!carol[0].is_active, "dedupe never re-activates anyone");

        assert_eq!(total_edges(&db).await, 3);
        assert!(plays_on_pair_index_exists(&db.client).await.unwrap());

        // Second boot is a no-op and the index now blocks a duplicate.
        run_migrations(&db.client).await.expect("second boot");
        assert_eq!(total_edges(&db).await, 3);
        let dup = db
            .client
            .query("RELATE member:alice -> plays_on -> team:t1 SET team_role = 'player'")
            .await
            .unwrap()
            .check();
        assert!(dup.is_err());
    }

    #[tokio::test]
    async fn migration_leaves_unique_pairs_alone() {
        let db = seeded_db().await;
        drop_unique_index(&db).await;
        seed_legacy_edge(
            &db,
            "keep_a",
            "alice",
            "t1",
            "player",
            "2026-01-01T00:00:00Z",
            true,
        )
        .await;
        seed_legacy_edge(
            &db,
            "keep_b",
            "bob",
            "t1",
            "sub",
            "2026-01-01T00:00:00Z",
            false,
        )
        .await;
        run_migrations(&db.client).await.expect("boot");
        assert_eq!(total_edges(&db).await, 2);
        assert_eq!(
            edge_keys(&db, "bob", "t1").await[0].id.as_deref(),
            Some("keep_b")
        );
        assert!(plays_on_pair_index_exists(&db.client).await.unwrap());
    }
}
