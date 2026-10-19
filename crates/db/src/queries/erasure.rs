//! Member stats erasure and full account erasure.
//!
//! Tracker-token stats deletion is a soft delete: the deadline is stored on
//! the member, reads hide the rows, and [`Database::purge_scheduled_stats`]
//! hard-deletes them once the deadline has passed. A site session hard-deletes
//! immediately. Account erasure removes the user and points shared author
//! fields at the tombstone member ([`crate::DELETED_MEMBER_ID`]).

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use surrealdb::engine::any::Any;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb::Surreal;
use surrealdb_types::{RecordId, SurrealValue};

use scuffed_auth::crypto::hash_provider_id;

use crate::{
    with_timeout, Database, DbError, DbResult, DELETED_MEMBER_ID, DELETED_MEMBER_NAME,
    DELETED_USER_ID,
};

/// How long a tracker soft delete waits before the purge hard-deletes rows.
pub const STATS_DELETE_GRACE: Duration = Duration::days(7);

/// What account erase removed that the caller still needs after commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEraseReport {
    pub personal_match: u64,
    pub stat_report_ids: Vec<String>,
}

/// Rows removed by a stats hard delete, plus tracker tokens that were active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsWipe {
    pub personal_match: u64,
    pub stat_report: u64,
    pub stat_report_ids: Vec<String>,
    pub tokens_revoked: u64,
}

/// One pass of the scheduled purge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgeOutcome {
    pub members: u64,
    pub personal_match: u64,
    pub stat_report: u64,
    pub stat_report_ids: Vec<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct CountRow {
    count: i64,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct IdRow {
    id: RecordId,
}

fn record_key(id: RecordId) -> String {
    crate::record_id_key_to_string(id.key)
}

fn count_of(rows: Vec<CountRow>) -> u64 {
    rows.first().map(|row| row.count.max(0) as u64).unwrap_or(0)
}

fn map_erase_error(err: surrealdb::Error) -> DbError {
    let msg = err.to_string();
    if msg.contains("last_admin") {
        DbError::Conflict("last_admin".into())
    } else {
        DbError::Surreal(err)
    }
}

/// Same actionable-admin predicate as `count_actionable_admins`.
const ACTIONABLE_ADMIN: &str = "is_active = true AND org_role = 'admin' \
     AND is_tombstone != true \
     AND meta::id(id) != 'deleted' \
     AND meta::id(id) NOT IN ( \
         SELECT VALUE member_id FROM moderation_action \
         WHERE is_active = true \
         AND action_type IN ['suspension', 'ban'] \
         AND (expires_at IS NONE OR expires_at > time::now()) \
     )";

/// Create the reserved member once. An existing row is forced back to the
/// lowest role and inactive, and its user username is `deleteduser`.
pub async fn ensure_deleted_member(client: &Surreal<Any>) -> DbResult<()> {
    let existing: Option<serde_json::Value> = client
        .select(("member", DELETED_MEMBER_ID))
        .await
        .map_err(DbError::Surreal)?;
    if existing.is_some() {
        client
            .query(
                "UPDATE $member SET org_role = 'recruit', is_active = false, is_tombstone = true; \
                 UPDATE $user SET username = $username, provider = 'local'",
            )
            .bind(("member", RecordId::new("member", DELETED_MEMBER_ID)))
            .bind(("user", RecordId::new("user", DELETED_USER_ID)))
            .bind(("username", DELETED_USER_ID.to_string()))
            .await
            .map_err(DbError::Surreal)?
            .check()
            .map_err(DbError::Surreal)?;
        return Ok(());
    }

    let key = crate::display_name::display_name_key(DELETED_MEMBER_NAME).map_err(|_| {
        DbError::Config("reserved Deleted member name has no comparison key".into())
    })?;
    let pid_hash = hash_provider_id("local", DELETED_USER_ID);

    client
        .query(
            "CREATE $user SET \
                provider = 'local', \
                username = $username, \
                provider_id = $pid, \
                provider_id_hash = $hash, \
                password_hash = NONE, \
                avatar_url = NONE, \
                provider_id_encrypted = NONE, \
                created_at = time::now()",
        )
        .bind(("user", RecordId::new("user", DELETED_USER_ID)))
        .bind(("username", DELETED_USER_ID.to_string()))
        .bind(("pid", DELETED_USER_ID.to_string()))
        .bind(("hash", pid_hash))
        .await
        .map_err(DbError::Surreal)?
        .check()
        .map_err(DbError::Surreal)?;

    client
        .query(
            "CREATE $member SET \
                user_id = $uid, \
                org_role = 'recruit', \
                display_name = $name, \
                display_name_key = $key, \
                is_active = false, \
                is_tombstone = true, \
                joined_at = time::now(), \
                stats_delete_scheduled_at = NONE, \
                bio = NONE, \
                avatar_url = NONE, \
                timezone = NONE, \
                pronouns = NONE, \
                availability_status = NONE, \
                nostr_pubkey = NONE, \
                nostr_key_mode = NONE, \
                nostr_secret_key_encrypted = NONE, \
                main_role = NONE, \
                twitch = NONE, \
                twitter = NONE",
        )
        .bind(("member", RecordId::new("member", DELETED_MEMBER_ID)))
        .bind(("uid", DELETED_USER_ID.to_string()))
        .bind(("name", DELETED_MEMBER_NAME.to_string()))
        .bind(("key", key))
        .await
        .map_err(|err| {
            let msg = err.to_string();
            if msg.to_ascii_lowercase().contains("display_name") {
                DbError::Config(
                    "cannot create the Deleted member tombstone; that display name is taken".into(),
                )
            } else {
                DbError::Surreal(err)
            }
        })?
        .check()
        .map_err(DbError::Surreal)?;
    Ok(())
}

impl Database {
    /// True when removing this member would leave the org with no actionable admin.
    pub async fn member_is_last_actionable_admin(&self, member_id: &str) -> DbResult<bool> {
        if member_id == DELETED_MEMBER_ID {
            return Ok(false);
        }
        let Some(member) = self.get_member(member_id).await? else {
            return Ok(false);
        };
        if member.org_role != crate::OrgRole::Admin || !member.is_active {
            return Ok(false);
        }
        if self.is_member_suspended_or_banned(member_id).await? {
            return Ok(false);
        }
        Ok(self.count_actionable_admins().await? <= 1)
    }

    /// Deadline already stored, or newly set to `now + 7 days` when none is set.
    ///
    /// A second call does not move the deadline. Active tracker tokens are
    /// revoked either way.
    pub async fn schedule_stats_delete(
        &self,
        member_id: &str,
        now: DateTime<Utc>,
    ) -> DbResult<(DateTime<Utc>, u64)> {
        if member_id == DELETED_MEMBER_ID {
            return Err(DbError::Config("tombstone member cannot be erased".into()));
        }
        let when = now + STATS_DELETE_GRACE;
        with_timeout(async {
            let rid = RecordId::new("member", member_id);
            self.client
                .query(
                    "UPDATE $rid SET stats_delete_scheduled_at = $when \
                     WHERE stats_delete_scheduled_at IS NONE",
                )
                .bind(("rid", rid.clone()))
                .bind(("when", SurrealDatetime::from(when)))
                .await?
                .check()?;

            #[derive(Deserialize, SurrealValue)]
            struct Row {
                stats_delete_scheduled_at: Option<SurrealDatetime>,
            }
            let mut read = self
                .client
                .query("SELECT stats_delete_scheduled_at FROM $rid")
                .bind(("rid", rid))
                .await?;
            let rows: Vec<Row> = read.take(0)?;
            let deadline = rows
                .into_iter()
                .next()
                .and_then(|row| row.stats_delete_scheduled_at)
                .map(Into::into)
                .ok_or_else(|| DbError::NotFound(format!("member {member_id} not found")))?;
            let tokens_revoked = revoke_daemon_tokens(&self.client, member_id).await?;
            Ok((deadline, tokens_revoked))
        })
        .await
    }

    /// Clear a pending soft delete. Tokens stay revoked. Returns false when nothing was pending.
    pub async fn cancel_stats_delete(&self, member_id: &str) -> DbResult<bool> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                id: RecordId,
            }
            let mut result = self
                .client
                .query(
                    "UPDATE $rid SET stats_delete_scheduled_at = NONE \
                     WHERE stats_delete_scheduled_at IS NOT NONE",
                )
                .bind(("rid", RecordId::new("member", member_id)))
                .await?;
            let rows: Vec<Row> = result.take(0).unwrap_or_default();
            Ok(!rows.is_empty())
        })
        .await
    }

    /// Hard-delete this member's personal stats and revoke tracker tokens.
    ///
    /// Hero, role, and map figures are aggregates over `personal_match`, so
    /// removing those rows drops them from leaderboards on the next read.
    pub async fn hard_delete_personal_stats(&self, member_id: &str) -> DbResult<StatsWipe> {
        if member_id == DELETED_MEMBER_ID {
            return Err(DbError::Config("tombstone member cannot be erased".into()));
        }
        with_timeout(async {
            let personal_match = count_where(
                &self.client,
                "SELECT count() AS count FROM personal_match WHERE member_id = $mid GROUP ALL",
                member_id,
            )
            .await?;
            let stat_report_ids = string_ids(
                &self.client,
                "SELECT id FROM stat_report WHERE member_id = $mid",
                member_id,
            )
            .await?;
            let tokens_revoked = count_where(
                &self.client,
                "SELECT count() AS count FROM daemon_token \
                 WHERE member_id = $mid AND is_active = true GROUP ALL",
                member_id,
            )
            .await?;

            self.client
                .query(
                    "BEGIN TRANSACTION; \
                     DELETE personal_match WHERE member_id = $mid; \
                     DELETE stat_report WHERE member_id = $mid; \
                     UPDATE daemon_token SET is_active = false WHERE member_id = $mid; \
                     UPDATE $rid SET stats_delete_scheduled_at = NONE; \
                     COMMIT TRANSACTION;",
                )
                .bind(("mid", member_id.to_string()))
                .bind(("rid", RecordId::new("member", member_id)))
                .await
                .map_err(map_erase_error)?
                .check()
                .map_err(map_erase_error)?;

            Ok(StatsWipe {
                personal_match,
                stat_report: stat_report_ids.len() as u64,
                stat_report_ids,
                tokens_revoked,
            })
        })
        .await
    }

    /// Hard-delete stats for every member whose deadline is at or before `now`.
    pub async fn purge_scheduled_stats(&self, now: DateTime<Utc>) -> DbResult<PurgeOutcome> {
        let due = with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                member_id: String,
            }
            let mut result = self
                .client
                .query(
                    "SELECT meta::id(id) AS member_id FROM member \
                     WHERE stats_delete_scheduled_at IS NOT NONE \
                     AND stats_delete_scheduled_at <= $now \
                     AND is_tombstone != true \
                     AND meta::id(id) != $tomb",
                )
                .bind(("now", SurrealDatetime::from(now)))
                .bind(("tomb", DELETED_MEMBER_ID.to_string()))
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            Ok(rows)
        })
        .await?;

        let mut outcome = PurgeOutcome {
            members: 0,
            personal_match: 0,
            stat_report: 0,
            stat_report_ids: Vec::new(),
        };
        for row in due {
            let wipe = self.hard_delete_personal_stats(&row.member_id).await?;
            outcome.members += 1;
            outcome.personal_match += wipe.personal_match;
            outcome.stat_report += wipe.stat_report;
            outcome.stat_report_ids.extend(wipe.stat_report_ids);
        }
        Ok(outcome)
    }

    /// Forum events this member signed that we stored (`nostr_event_id`).
    ///
    /// Gift wraps are signed by ephemeral keys, so DM rows are not included.
    /// Team channel posts that were never written to our database are absent too.
    pub async fn member_stored_nostr_event_ids(&self, member_id: &str) -> DbResult<Vec<String>> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                nostr_event_id: String,
            }
            let mut threads = self
                .client
                .query(
                    "SELECT nostr_event_id FROM forum_thread \
                     WHERE author_member_id = $mid \
                     AND nostr_event_id IS NOT NONE AND nostr_event_id != ''",
                )
                .bind(("mid", member_id.to_string()))
                .await?;
            let mut ids: Vec<String> = Vec::new();
            let thread_rows: Vec<Row> = threads.take(0)?;
            ids.extend(thread_rows.into_iter().map(|row| row.nostr_event_id));
            // Local forum_thread and forum_reply rows stay. Erasure rewrites
            // the author to the tombstone. Replies have no nostr_event_id.
            Ok(ids)
        })
        .await
    }

    /// Delete the account and everything that belongs only to it.
    ///
    /// Shared rows stay. Their member reference becomes the tombstone.
    /// Personal strategies (no team) are deleted. Team strategies keep the
    /// document and the owner becomes the tombstone.
    ///
    /// `signed_deletion_event` is stored when the caller could sign a NIP-09
    /// request. Pass `None` when there is no server-held key or no event id.
    ///
    /// Refuses with [`DbError::Conflict`] `"last_admin"` without deleting when
    /// this member is the last actionable admin. The check runs inside the
    /// same transaction as the deletes.
    pub async fn erase_account(
        &self,
        member_id: &str,
        user_id: &str,
        nostr_pubkey: Option<&str>,
        signed_deletion_event: Option<&str>,
    ) -> DbResult<AccountEraseReport> {
        if member_id == DELETED_MEMBER_ID || user_id == DELETED_USER_ID {
            return Err(DbError::Config("tombstone member cannot be erased".into()));
        }
        let tomb_key =
            crate::display_name::display_name_key(DELETED_MEMBER_NAME).map_err(|_| {
                DbError::Config("reserved Deleted member name has no comparison key".into())
            })?;
        let tomb_hash = hash_provider_id("local", DELETED_USER_ID);
        let stat_report_ids = self.stat_report_ids_for(member_id).await?;
        let personal_match = count_where(
            &self.client,
            "SELECT count() AS count FROM personal_match WHERE member_id = $mid GROUP ALL",
            member_id,
        )
        .await?;
        with_timeout(async {
            let pubkey = nostr_pubkey.unwrap_or("");
            let mut query = self
                .client
                .query(account_erase_sql(signed_deletion_event.is_some()))
                .bind(("mid", member_id.to_string()))
                .bind(("uid", user_id.to_string()))
                .bind(("tomb", DELETED_MEMBER_ID.to_string()))
                .bind(("tomb_member", RecordId::new("member", DELETED_MEMBER_ID)))
                .bind(("tomb_user", RecordId::new("user", DELETED_USER_ID)))
                .bind(("tomb_username", DELETED_USER_ID.to_string()))
                .bind(("tomb_pid", DELETED_USER_ID.to_string()))
                .bind(("tomb_hash", tomb_hash))
                .bind(("tomb_name", DELETED_MEMBER_NAME.to_string()))
                .bind(("tomb_key", tomb_key))
                .bind(("pk", pubkey.to_string()))
                .bind(("member_rid", RecordId::new("member", member_id)))
                .bind(("user_rid", RecordId::new("user", user_id)));
            if let Some(event) = signed_deletion_event {
                query = query
                    .bind(("signed", event.to_string()))
                    .bind(("event_count", event_count_hint(event)));
            }
            let mut response = query.await.map_err(map_erase_error)?;
            let errors = response.take_errors();
            if !errors.is_empty() {
                let text = format!("{errors:?}");
                // A thrown transaction stays on this session until it is cancelled.
                let _ = self.client.query("CANCEL TRANSACTION").await;
                if text.contains("last_admin") {
                    return Err(DbError::Conflict("last_admin".into()));
                }
                let err = errors
                    .into_values()
                    .find(|err| {
                        let msg = err.to_string();
                        !msg.contains("not executed")
                            && !msg.contains("cancelled transaction")
                            && !msg.contains("Cannot COMMIT")
                    })
                    .unwrap_or_else(|| surrealdb::Error::thrown("transaction failed".into()));
                return Err(map_erase_error(err));
            }
            Ok(AccountEraseReport {
                personal_match,
                stat_report_ids,
            })
        })
        .await
    }

    async fn stat_report_ids_for(&self, member_id: &str) -> DbResult<Vec<String>> {
        with_timeout(async {
            string_ids(
                &self.client,
                "SELECT id FROM stat_report WHERE member_id = $mid",
                member_id,
            )
            .await
        })
        .await
    }
}

fn event_count_hint(event_json: &str) -> i64 {
    serde_json::from_str::<serde_json::Value>(event_json)
        .ok()
        .and_then(|value| value.get("tags").and_then(|tags| tags.as_array()).cloned())
        .map(|tags| {
            tags.iter()
                .filter(|tag| tag.get(0).and_then(|v| v.as_str()) == Some("e"))
                .count() as i64
        })
        .unwrap_or(0)
}

async fn count_where(client: &Surreal<Any>, sql: &str, member_id: &str) -> DbResult<u64> {
    let mut result = client
        .query(sql)
        .bind(("mid", member_id.to_string()))
        .await
        .map_err(DbError::Surreal)?;
    let rows: Vec<CountRow> = result.take(0).map_err(DbError::Surreal)?;
    Ok(count_of(rows))
}

async fn string_ids(client: &Surreal<Any>, sql: &str, member_id: &str) -> DbResult<Vec<String>> {
    let mut result = client
        .query(sql)
        .bind(("mid", member_id.to_string()))
        .await
        .map_err(DbError::Surreal)?;
    let rows: Vec<IdRow> = result.take(0).map_err(DbError::Surreal)?;
    Ok(rows.into_iter().map(|row| record_key(row.id)).collect())
}

async fn revoke_daemon_tokens(client: &Surreal<Any>, member_id: &str) -> DbResult<u64> {
    let tokens_revoked = count_where(
        client,
        "SELECT count() AS count FROM daemon_token \
         WHERE member_id = $mid AND is_active = true GROUP ALL",
        member_id,
    )
    .await?;
    client
        .query(
            "UPDATE daemon_token SET is_active = false WHERE member_id = $mid AND is_active = true",
        )
        .bind(("mid", member_id.to_string()))
        .await
        .map_err(DbError::Surreal)?
        .check()
        .map_err(DbError::Surreal)?;
    Ok(tokens_revoked)
}

/// One transaction. The last-admin `THROW` aborts every statement in the batch.
fn account_erase_sql(store_deletion_event: bool) -> String {
    let store = if store_deletion_event {
        "CREATE nostr_deletion_request SET \
            signed_event = $signed, \
            event_count = $event_count, \
            published = false, \
            created_at = time::now();"
    } else {
        ""
    };
    format!(
        r#"
        BEGIN TRANSACTION;
        LET $self_n = (SELECT count() AS count FROM member
            WHERE meta::id(id) = $mid AND {ACTIONABLE_ADMIN} GROUP ALL);
        LET $other_n = (SELECT count() AS count FROM member
            WHERE meta::id(id) != $mid AND {ACTIONABLE_ADMIN} GROUP ALL);
        LET $self_count = IF array::len($self_n) = 0 THEN 0 ELSE $self_n[0].count END;
        LET $other_count = IF array::len($other_n) = 0 THEN 0 ELSE $other_n[0].count END;
        IF $self_count > 0 AND $other_count = 0 {{
            THROW "last_admin";
        }};
        -- First successful account delete creates the placeholder. A refused
        -- last-admin delete rolls this back with the rest of the transaction.
        LET $tomb_hit = (SELECT id FROM $tomb_member LIMIT 1);
        IF array::len($tomb_hit) = 0 {{
            CREATE $tomb_user SET
                provider = 'local',
                username = $tomb_username,
                provider_id = $tomb_pid,
                provider_id_hash = $tomb_hash,
                password_hash = NONE,
                avatar_url = NONE,
                provider_id_encrypted = NONE,
                created_at = time::now();
            CREATE $tomb_member SET
                user_id = $tomb_username,
                org_role = 'recruit',
                display_name = $tomb_name,
                display_name_key = $tomb_key,
                is_active = false,
                is_tombstone = true,
                joined_at = time::now(),
                stats_delete_scheduled_at = NONE,
                bio = NONE,
                avatar_url = NONE,
                timezone = NONE,
                pronouns = NONE,
                availability_status = NONE,
                nostr_pubkey = NONE,
                nostr_key_mode = NONE,
                nostr_secret_key_encrypted = NONE,
                main_role = NONE,
                twitch = NONE,
                twitter = NONE;
        }};
        -- audit_log is append-only (THROW on UPDATE/DELETE). actor_id stays.
        -- tournament_participant has no member or user column. Left unchanged.
        UPDATE announcement SET author_id = $tomb WHERE author_id = $mid;
        UPDATE article SET author_member_id = $tomb WHERE author_member_id = $mid;
        UPDATE wiki_page SET author_member_id = $tomb WHERE author_member_id = $mid;
        UPDATE wiki_revision SET edited_by = $tomb WHERE edited_by = $mid;
        UPDATE match_result SET recorded_by = $tomb WHERE recorded_by = $mid;
        UPDATE event SET created_by = $tomb WHERE created_by = $mid;
        UPDATE poll SET created_by = $tomb WHERE created_by = $mid;
        UPDATE scrim SET requested_by = $tomb WHERE requested_by = $mid;
        UPDATE tournament SET created_by = $tomb WHERE created_by = $mid;
        UPDATE moderation_action SET member_id = $tomb WHERE member_id = $mid;
        UPDATE moderation_action SET issued_by = $tomb WHERE issued_by = $mid;
        UPDATE event_attendance SET marked_by = $tomb WHERE marked_by = $mid;
        LET $att_rows = (SELECT id, event_id, occurrence_date FROM event_attendance WHERE member_id = $mid);
        IF array::len($att_rows) > 0 {{
            FOR $att IN $att_rows {{
                LET $att_hit = (SELECT id FROM event_attendance
                    WHERE member_id = $tomb
                    AND event_id = $att.event_id
                    AND occurrence_date = $att.occurrence_date
                    LIMIT 1);
                IF array::len($att_hit) > 0 {{
                    DELETE $att.id;
                }} ELSE {{
                    UPDATE $att.id SET member_id = $tomb;
                }};
            }};
        }};
        UPDATE application SET reviewed_by = $tomb WHERE reviewed_by = $mid;
        UPDATE application SET mentor_id = $tomb WHERE mentor_id = $mid;
        UPDATE forum_reply SET author_member_id = $tomb WHERE author_member_id = $mid;
        UPDATE forum_thread SET author_member_id = $tomb WHERE author_member_id = $mid;
        DELETE strategy WHERE owner_id = $mid AND (team_id IS NONE OR team_id = '');
        UPDATE strategy SET owner_id = $tomb
            WHERE owner_id = $mid AND team_id IS NOT NONE AND team_id != '';
        DELETE personal_match WHERE member_id = $mid;
        DELETE stat_report WHERE member_id = $mid;
        DELETE daemon_token WHERE member_id = $mid;
        DELETE member_settings WHERE member_id = $mid;
        DELETE device_link WHERE member_id = $mid;
        DELETE game_account WHERE member_id = $mid;
        DELETE event_rsvp WHERE member_id = $mid;
        DELETE poll_vote WHERE member_id = $mid;
        DELETE application WHERE user_id = $uid;
        DELETE dm_read_marker WHERE member_id = $mid;
        DELETE group_last_seen WHERE member_id = $mid;
        DELETE dm_message WHERE sender_pubkey = $pk AND $pk != '';
        DELETE session WHERE user_id = $uid;
        DELETE plays_on WHERE in = $member_rid;
        {store}
        DELETE $member_rid;
        DELETE $user_rid;
        COMMIT TRANSACTION;
        "#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use crate::types::{AuditAction, AuditTargetType, OrgRole, PersonalMatch};
    use crate::Database;
    use serde::Deserialize;
    use surrealdb_types::{RecordId, SurrealValue};

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    fn sample_match(member_id: &str, session_id: &str) -> PersonalMatch {
        PersonalMatch {
            id: String::new(),
            member_id: member_id.to_string(),
            session_id: session_id.to_string(),
            hero: "Ana".into(),
            map_name: "Busan".into(),
            game_mode: "control".into(),
            role: "Support".into(),
            outcome: "victory".into(),
            elims: 10,
            deaths: 2,
            assists: 8,
            damage: 1000,
            healing: 4000,
            mitigation: 0,
            played_at: Utc::now(),
            uploaded_at: Utc::now(),
            edited: false,
            recognizer: "ocr-v1".into(),
            suspect_fields: Vec::new(),
        }
    }

    #[tokio::test]
    async fn purge_respects_the_injected_clock_and_is_idempotent() {
        let db = test_db().await;
        let member = db
            .create_member("user-purge", "Purge Me", OrgRole::Member)
            .await
            .unwrap();
        let other = db
            .create_member("user-keep", "Keep Me", OrgRole::Member)
            .await
            .unwrap();
        db.upsert_personal_matches(&member.id, &[sample_match(&member.id, "s1")])
            .await
            .unwrap();
        db.upsert_personal_matches(&other.id, &[sample_match(&other.id, "s2")])
            .await
            .unwrap();

        let now = Utc::now();
        let (deadline, _) = db.schedule_stats_delete(&member.id, now).await.unwrap();
        assert!(deadline > now + Duration::days(6));

        let early = db
            .purge_scheduled_stats(deadline - Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(early.members, 0);
        assert_eq!(
            db.get_personal_stats(&member.id)
                .await
                .unwrap()
                .total_matches,
            0,
            "pending soft delete hides reads before the purge"
        );

        let due = db.purge_scheduled_stats(deadline).await.unwrap();
        assert_eq!(due.members, 1);
        assert_eq!(due.personal_match, 1);
        let again = db.purge_scheduled_stats(deadline).await.unwrap();
        assert_eq!(again.members, 0);

        db.cancel_stats_delete(&member.id).await.unwrap();
        assert_eq!(
            db.get_personal_stats(&member.id)
                .await
                .unwrap()
                .total_matches,
            0,
            "purged rows stay gone"
        );
        assert_eq!(
            db.get_personal_stats(&other.id)
                .await
                .unwrap()
                .total_matches,
            1
        );
    }

    #[tokio::test]
    async fn hard_delete_is_scoped_to_one_member_and_revokes_tokens() {
        let db = test_db().await;
        let member = db
            .create_member("user-hard", "Hard Delete", OrgRole::Member)
            .await
            .unwrap();
        let other = db
            .create_member("user-other", "Other", OrgRole::Member)
            .await
            .unwrap();
        db.upsert_personal_matches(
            &member.id,
            &[sample_match(&member.id, "a"), sample_match(&member.id, "b")],
        )
        .await
        .unwrap();
        db.upsert_personal_matches(&other.id, &[sample_match(&other.id, "c")])
            .await
            .unwrap();
        db.create_daemon_token(&member.id, "tok-1", "pc1")
            .await
            .unwrap();
        db.create_daemon_token(&member.id, "tok-2", "pc2")
            .await
            .unwrap();

        let wipe = db.hard_delete_personal_stats(&member.id).await.unwrap();
        assert_eq!(wipe.personal_match, 2);
        assert_eq!(wipe.stat_report, 0);
        assert_eq!(wipe.tokens_revoked, 2);
        assert!(db.validate_daemon_token("tok-1").await.unwrap().is_none());
        assert!(db.validate_daemon_token("tok-2").await.unwrap().is_none());
        assert_eq!(
            db.get_personal_stats(&other.id)
                .await
                .unwrap()
                .total_matches,
            1
        );
    }

    #[tokio::test]
    async fn erase_account_keeps_shared_rows_and_refuses_the_last_admin() {
        let db = test_db().await;
        let admin = db.create_local_user("solo-admin", "hash").await.unwrap();
        let admin_member = db
            .create_member(&admin.id, "Solo Admin", OrgRole::Admin)
            .await
            .unwrap();
        let err = db
            .erase_account(&admin_member.id, &admin.id, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Conflict(ref msg) if msg == "last_admin"));
        assert!(db.get_member(&admin_member.id).await.unwrap().is_some());
        assert!(db.get_user(&admin.id).await.unwrap().is_some());

        let second = db.create_local_user("second-admin", "hash").await.unwrap();
        let _second_member = db
            .create_member(&second.id, "Second Admin", OrgRole::Admin)
            .await
            .unwrap();
        db.create_announcement("Kept", "body", &admin_member.id, false)
            .await
            .unwrap();
        db.erase_account(&admin_member.id, &admin.id, None, None)
            .await
            .unwrap();
        assert!(db.get_member(&admin_member.id).await.unwrap().is_none());
        assert!(db.get_user(&admin.id).await.unwrap().is_none());
        let announcements = db.list_announcements_paginated(10, 0).await.unwrap();
        assert_eq!(announcements.len(), 1);
        assert_eq!(announcements[0].author_id, DELETED_MEMBER_ID);
        let tomb = db.get_member(DELETED_MEMBER_ID).await.unwrap().unwrap();
        assert_eq!(tomb.display_name, DELETED_MEMBER_NAME);
    }

    #[tokio::test]
    async fn erase_account_covers_member_references() {
        let db = test_db().await;
        let keeper = db.create_local_user("keeper", "hash").await.unwrap();
        let _keeper_member = db
            .create_member(&keeper.id, "Keeper", OrgRole::Admin)
            .await
            .unwrap();
        let user = db.create_local_user("gone", "hash").await.unwrap();
        let member = db
            .create_member(&user.id, "Gone User", OrgRole::Member)
            .await
            .unwrap();
        let mid = member.id.clone();
        let uid = user.id.clone();
        let day1 = SurrealDatetime::from(
            DateTime::parse_from_rfc3339("2026-03-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let day2 = SurrealDatetime::from(
            DateTime::parse_from_rfc3339("2026-03-02T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        db.client
            .query(
                r#"
                CREATE game:g1 SET name = 'Overwatch';
                CREATE team:t1 SET name = 'Alpha', game_id = 'g1';
                RELATE $member_rid->plays_on->$team_rid SET team_role = 'player';
                CREATE event:e1 SET title = 'Scrim night', day_of_week = 2, time = '20:00', created_by = $mid;
                CREATE event_attendance:collide SET
                    member_id = $mid, event_id = 'e1', occurrence_date = $day1,
                    status = 'attended', marked_by = $mid;
                CREATE event_attendance:tomb SET
                    member_id = $tomb, event_id = 'e1', occurrence_date = $day1,
                    status = 'excused', marked_by = $tomb;
                CREATE event_attendance:later SET
                    member_id = $mid, event_id = 'e1', occurrence_date = $day2,
                    status = 'no_show', marked_by = $mid;
                CREATE event_rsvp:rsvp1 SET member_id = $mid, event_id = 'e1', status = 'yes';
                CREATE announcement:news SET title = 'News', content = 'body', author_id = $mid;
                CREATE article:post SET slug = 'post', title = 'Post', content_markdown = 'md', author_member_id = $mid;
                CREATE wiki_page:page SET topic = 'page', title = 'Page', content_markdown = 'md', author_member_id = $mid;
                CREATE wiki_revision:rev SET page_id = 'page', content_markdown = 'md', edited_by = $mid;
                CREATE forum_thread:plain SET title = 'Plain', author_member_id = $mid, content = 'hello';
                CREATE forum_thread:signed SET title = 'Signed', author_member_id = $mid, content = 'secret', nostr_event_id = 'evt-thread';
                CREATE forum_reply:reply SET thread_id = 'plain', author_member_id = $mid, content = 'reply';
                CREATE match_result:mr SET team_id = 't1', opponent = 'Other', recorded_by = $mid;
                CREATE poll:poll1 SET title = 'When', options = ['now'], created_by = $mid;
                CREATE poll_vote:vote1 SET poll_id = 'poll1', member_id = $mid, option_index = 0;
                CREATE scrim:scrim1 SET team_id = 't1', game_id = 'g1', requested_by = $mid, scheduled_at = time::now();
                CREATE tournament:cup SET name = 'Cup', format = 'single_elim', created_by = $mid;
                CREATE tournament_participant:p1 SET tournament_id = 'cup', external_name = 'Night Owls', status = 'registered';
                CREATE moderation_action:mod1 SET member_id = $mid, action_type = 'note', reason = 'note', issued_by = $mid;
                CREATE application:own SET user_id = $uid, status = 'pending';
                CREATE application:other SET user_id = $keeper, status = 'trial', reviewed_by = $mid, mentor_id = $mid;
                CREATE strategy:solo SET name = 'Solo', map_id = 'busan', game_mode = 'control', owner_id = $mid;
                CREATE strategy:shared SET name = 'Shared', map_id = 'busan', game_mode = 'control', owner_id = $mid, team_id = 't1';
                CREATE member_settings:ms SET member_id = $mid;
                CREATE device_link:link SET
                    user_code_hash = 'uch', device_code_hash = 'dch', device_label = 'pc',
                    app_version = '0', status = 'pending', member_id = $mid, expires_at = time::now() + 1h;
                CREATE game_account:ga SET member_id = $mid, game_id = 'g1', account_name = 'gone#1';
                CREATE group_last_seen:gls SET member_id = $mid, group_id = 'g', last_seen_at = time::now();
                CREATE dm_read_marker:mark SET member_id = $mid, peer_pubkey = 'pk-other', last_read_at = time::now();
                CREATE dm_message:sent SET
                    gift_wrap_id = 'gw-sent', sender_pubkey = 'pk-gone', recipient_pubkey = 'pk-other',
                    conversation_key = 'pk-gone,pk-other', content = 'hi';
                CREATE dm_message:recv SET
                    gift_wrap_id = 'gw-recv', sender_pubkey = 'pk-other', recipient_pubkey = 'pk-gone',
                    conversation_key = 'pk-gone,pk-other', content = 'yo';
                CREATE session:sess SET user_id = $uid, token = 'tok-gone', expires_at = time::now() + 1d;
                CREATE daemon_token:tok SET member_id = $mid, token_hash = 'hash-gone';
                CREATE stat_report:rpt SET
                    member_id = $mid, created_at = time::now(), reason_category = 'other',
                    reason_text = 'x', app_version = '0', recognizer_matcher = 'm', recognizer_ocr = 'o',
                    training_consent = false, own_name_included = false, glyphs_included = false,
                    size_bytes = 1, zip_sha256 = 'abc';
                "#,
            )
            .bind(("mid", mid.clone()))
            .bind(("uid", uid.clone()))
            .bind(("keeper", keeper.id.clone()))
            .bind(("tomb", DELETED_MEMBER_ID.to_string()))
            .bind(("day1", day1))
            .bind(("day2", day2))
            .bind(("member_rid", RecordId::new("member", mid.as_str())))
            .bind(("team_rid", RecordId::new("team", "t1")))
            .await
            .unwrap()
            .check()
            .unwrap();

        db.upsert_personal_matches(&mid, &[sample_match(&mid, "game-1")])
            .await
            .unwrap();
        db.insert_audit_log(
            &mid,
            AuditAction::CreatedWikiPage,
            AuditTargetType::WikiPage,
            "page",
            None,
        )
        .await
        .unwrap();

        let report = db
            .erase_account(&mid, &uid, Some("pk-gone"), None)
            .await
            .unwrap();
        assert_eq!(report.personal_match, 1);
        assert_eq!(report.stat_report_ids.len(), 1);

        assert!(db.get_member(&mid).await.unwrap().is_none());
        assert!(db.get_user(&uid).await.unwrap().is_none());
        assert_eq!(
            count_sql(
                &db,
                "SELECT count() AS count FROM personal_match WHERE member_id = $mid GROUP ALL",
                &mid
            )
            .await,
            0
        );
        for table in [
            "stat_report",
            "daemon_token",
            "member_settings",
            "device_link",
            "game_account",
            "group_last_seen",
            "dm_read_marker",
            "event_rsvp",
            "poll_vote",
        ] {
            let sql =
                format!("SELECT count() AS count FROM {table} WHERE member_id = $mid GROUP ALL");
            assert_eq!(count_sql(&db, &sql, &mid).await, 0, "{table}");
        }
        assert_eq!(
            count_sql(
                &db,
                "SELECT count() AS count FROM session WHERE user_id = $mid GROUP ALL",
                &uid
            )
            .await,
            0
        );
        assert_eq!(
            count_sql(
                &db,
                "SELECT count() AS count FROM plays_on WHERE in = $member_rid GROUP ALL",
                &mid,
            )
            .await,
            0
        );
        assert_eq!(
            count_sql(
                &db,
                "SELECT count() AS count FROM strategy WHERE name = 'Solo' GROUP ALL",
                &mid
            )
            .await,
            0
        );
        assert_eq!(
            count_sql(
                &db,
                "SELECT count() AS count FROM application WHERE user_id = $mid GROUP ALL",
                &uid
            )
            .await,
            0
        );
        assert_eq!(
            count_sql(
                &db,
                "SELECT count() AS count FROM dm_message WHERE gift_wrap_id = 'gw-sent' GROUP ALL",
                &mid
            )
            .await,
            0
        );
        assert_eq!(
            count_sql(
                &db,
                "SELECT count() AS count FROM dm_message WHERE gift_wrap_id = 'gw-recv' GROUP ALL",
                &mid
            )
            .await,
            1,
            "a message the member only received stays"
        );
        assert_eq!(
            count_sql(
                &db,
                "SELECT count() AS count FROM forum_thread WHERE title = 'Signed' GROUP ALL",
                &mid
            )
            .await,
            1,
            "a forum thread with a nostr event id stays"
        );
        assert_eq!(
            text_sql(
                &db,
                "SELECT author_member_id AS value FROM forum_thread:signed"
            )
            .await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(
                &db,
                "SELECT nostr_event_id AS value FROM forum_thread:signed"
            )
            .await,
            "evt-thread"
        );

        assert_eq!(
            text_sql(&db, "SELECT author_id AS value FROM announcement:news").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT author_member_id AS value FROM article:post").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT author_member_id AS value FROM wiki_page:page").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT edited_by AS value FROM wiki_revision:rev").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(
                &db,
                "SELECT author_member_id AS value FROM forum_thread:plain"
            )
            .await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(
                &db,
                "SELECT author_member_id AS value FROM forum_reply:reply"
            )
            .await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT recorded_by AS value FROM match_result:mr").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT created_by AS value FROM event:e1").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT created_by AS value FROM poll:poll1").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT requested_by AS value FROM scrim:scrim1").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT created_by AS value FROM tournament:cup").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT member_id AS value FROM moderation_action:mod1").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT issued_by AS value FROM moderation_action:mod1").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT reviewed_by AS value FROM application:other").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT mentor_id AS value FROM application:other").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT user_id AS value FROM application:other").await,
            keeper.id
        );
        assert_eq!(
            text_sql(&db, "SELECT owner_id AS value FROM strategy:shared").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT member_id AS value FROM event_attendance:later").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(&db, "SELECT marked_by AS value FROM event_attendance:later").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            count_sql(
                &db,
                "SELECT count() AS count FROM event_attendance:collide GROUP ALL",
                &mid
            )
            .await,
            0,
            "attendance that would collide with the tombstone row is removed"
        );
        assert_eq!(
            text_sql(&db, "SELECT member_id AS value FROM event_attendance:tomb").await,
            DELETED_MEMBER_ID
        );
        assert_eq!(
            text_sql(
                &db,
                "SELECT external_name AS value FROM tournament_participant:p1"
            )
            .await,
            "Night Owls"
        );
        assert_eq!(
            text_sql(
                &db,
                "SELECT actor_id AS value FROM audit_log WHERE target_id = 'page'"
            )
            .await,
            mid
        );

        let entries = db.list_audit_log(10, 0).await.unwrap();
        let row = entries.iter().find(|e| e.actor_id == mid).unwrap();
        assert_eq!(row.actor_name, DELETED_MEMBER_NAME);
    }

    async fn seed_tombstone(db: &Database) {
        ensure_deleted_member(&db.client).await.unwrap();
    }

    #[tokio::test]
    async fn fresh_db_allows_setup_and_lists_omit_the_tombstone() {
        let db = test_db().await;
        assert!(
            !db.has_any_member().await.unwrap(),
            "a fresh database has no tombstone and setup stays open"
        );
        assert!(db.get_member(DELETED_MEMBER_ID).await.unwrap().is_none());
        assert!(db.claim_bootstrap_lock().await.is_ok());

        let admin = db.create_local_user("keeper", "hash").await.unwrap();
        let admin_member = db
            .create_member(&admin.id, "Keeper", OrgRole::Admin)
            .await
            .unwrap();
        let leaver = db.create_local_user("leaver", "hash").await.unwrap();
        let leaver_member = db
            .create_member(&leaver.id, "Leaver", OrgRole::Member)
            .await
            .unwrap();
        db.erase_account(&leaver_member.id, &leaver.id, None, None)
            .await
            .unwrap();

        #[derive(Deserialize, SurrealValue)]
        struct Flag {
            is_tombstone: bool,
        }
        let mut flagged = db
            .client
            .query("SELECT is_tombstone FROM $rid")
            .bind(("rid", RecordId::new("member", DELETED_MEMBER_ID)))
            .await
            .unwrap();
        let flags: Vec<Flag> = flagged.take(0).unwrap();
        assert_eq!(flags.len(), 1);
        assert!(flags[0].is_tombstone);

        let listed = db
            .list_members_paginated_filtered(50, 0, true)
            .await
            .unwrap();
        assert!(
            listed.iter().all(|row| row.id != DELETED_MEMBER_ID),
            "member list: {listed:?}"
        );
        assert!(listed
            .iter()
            .all(|row| row.display_name != DELETED_MEMBER_NAME));
        assert!(listed.iter().any(|row| row.id == admin_member.id));
        assert_eq!(db.count_active_members().await.unwrap(), 1);
        assert_eq!(db.count_actionable_admins().await.unwrap(), 1);
        assert!(db.has_any_member().await.unwrap());

        db.client
            .query("DELETE $member; DELETE $user;")
            .bind(("member", RecordId::new("member", admin_member.id.as_str())))
            .bind(("user", RecordId::new("user", admin.id.as_str())))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert!(
            !db.has_any_member().await.unwrap(),
            "the tombstone alone must not close first-run setup"
        );
        assert_eq!(db.count_active_members().await.unwrap(), 0);
        assert_eq!(db.count_active_admins().await.unwrap(), 0);
        assert_eq!(db.count_actionable_admins().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn oauth_link_cannot_sign_in_as_the_tombstone() {
        let db = test_db().await;
        seed_tombstone(&db).await;
        db.client
            .query(
                "UPDATE $rid SET provider = 'discord', provider_id = 'discord-tomb', \
                 provider_id_hash = NONE",
            )
            .bind(("rid", RecordId::new("user", DELETED_USER_ID)))
            .await
            .unwrap()
            .check()
            .unwrap();
        let err = db
            .upsert_user_from_oauth(
                scuffed_auth::AuthProvider::Discord,
                "discord-tomb".into(),
                "someone".into(),
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, DbError::Conflict(ref code) if code == "tombstone"),
            "{err}"
        );
        let err = db
            .upsert_user_from_oauth(
                scuffed_auth::AuthProvider::Discord,
                "brand-new".into(),
                " DeletedUser ".into(),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::ReservedName), "{err}");
    }

    #[tokio::test]
    async fn device_link_cannot_hand_the_tombstone_a_token() {
        let db = test_db().await;
        db.insert_device_link("USERCODE1", "DEVICECODE1", "pc", "1")
            .await
            .unwrap();
        let err = db
            .approve_device_link("USERCODE1", DELETED_MEMBER_ID, "tid", "secret")
            .await
            .unwrap_err();
        assert!(
            matches!(err, DbError::Conflict(ref code) if code == "tombstone"),
            "{err}"
        );
        let info = db.lookup_device_link("USERCODE1").await.unwrap();
        assert!(info.is_some(), "refused approve leaves the code pending");

        let device_hash = scuffed_auth::crypto::hash_session_token("DEVICECODE2");
        let user_hash = scuffed_auth::crypto::hash_session_token("USERCODE2");
        db.client
            .query(
                "CREATE device_link SET \
                    user_code_hash = $uh, \
                    device_code_hash = $dh, \
                    device_label = 'pc', \
                    app_version = '1', \
                    status = 'approved', \
                    member_id = $mid, \
                    daemon_token_id = 'tid', \
                    handover_token = 'super-secret', \
                    created_at = time::now(), \
                    expires_at = time::now() + 10m, \
                    last_poll_at = NONE",
            )
            .bind(("uh", user_hash))
            .bind(("dh", device_hash))
            .bind(("mid", DELETED_MEMBER_ID.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        let polled = db
            .poll_device_link("DEVICECODE2", Utc::now())
            .await
            .unwrap();
        assert!(
            !matches!(
                polled,
                crate::queries::device_link::DeviceLinkPoll::Approved(_)
            ),
            "a forged approval does not return a daemon secret: {polled:?}"
        );
    }

    #[tokio::test]
    async fn last_admin_count_skips_the_tombstone_even_if_it_is_marked_admin() {
        let db = test_db().await;
        seed_tombstone(&db).await;
        db.client
            .query("UPDATE $rid SET org_role = 'admin', is_active = true")
            .bind(("rid", RecordId::new("member", DELETED_MEMBER_ID)))
            .await
            .unwrap()
            .check()
            .unwrap();
        let admin = db.create_local_user("realadmin", "hash").await.unwrap();
        let admin_member = db
            .create_member(&admin.id, "Real Admin", OrgRole::Admin)
            .await
            .unwrap();
        assert_eq!(db.count_actionable_admins().await.unwrap(), 1);
        assert_eq!(db.count_active_admins().await.unwrap(), 1);
        assert!(db
            .member_is_last_actionable_admin(&admin_member.id)
            .await
            .unwrap());
        assert!(!db
            .member_is_last_actionable_admin(DELETED_MEMBER_ID)
            .await
            .unwrap());
        let err = db
            .erase_account(&admin_member.id, &admin.id, None, None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DbError::Conflict(ref code) if code == "last_admin"),
            "{err}"
        );
        assert!(db.get_member(&admin_member.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn create_member_rejects_the_reserved_display_name() {
        let db = test_db().await;
        for name in ["Deleted member", " deleted member ", "DELETED  MEMBER"] {
            let err = db
                .create_member("user-reg", name, OrgRole::Member)
                .await
                .unwrap_err();
            assert!(matches!(err, DbError::ReservedName), "{name}: {err}");
        }
        let err = db.create_local_user(" Deleted ", "hash").await.unwrap_err();
        assert!(matches!(err, DbError::ReservedName), "{err}");
        let err = db
            .create_local_user("DeletedUser", "hash")
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::ReservedName), "{err}");
    }

    #[tokio::test]
    async fn tombstone_is_hidden_from_roster_leaderboard_and_chat() {
        let db = test_db().await;
        seed_tombstone(&db).await;
        let game = db.create_game("Overwatch", Some("OW")).await.unwrap();
        let team = db
            .create_team("Night", &game.id, None, None, None)
            .await
            .unwrap();
        db.client
            .query(
                "UPDATE $rid SET is_active = true, org_role = 'officer', \
                 nostr_pubkey = 'pk-tomb'",
            )
            .bind(("rid", RecordId::new("member", DELETED_MEMBER_ID)))
            .await
            .unwrap()
            .check()
            .unwrap();
        let add = db
            .add_to_roster(DELETED_MEMBER_ID, &team.id, crate::TeamRole::Player)
            .await
            .unwrap_err();
        assert!(
            matches!(add, DbError::Conflict(ref code) if code == "tombstone"),
            "{add}"
        );
        db.client
            .query(
                "RELATE $member -> plays_on -> $team \
                 SET team_role = 'player', joined_at = time::now(), is_active = true",
            )
            .bind(("member", RecordId::new("member", DELETED_MEMBER_ID)))
            .bind(("team", RecordId::new("team", team.id.as_str())))
            .await
            .unwrap()
            .check()
            .unwrap();
        let roster = db.get_team_roster(&team.id).await.unwrap();
        assert!(
            roster.iter().all(|row| row.member_id != DELETED_MEMBER_ID),
            "roster: {roster:?}"
        );
        let named = db.get_team_roster_named(&team.id).await.unwrap();
        assert!(named.iter().all(|row| row.member_id != DELETED_MEMBER_ID));
        let identities = db.list_nostr_identities().await.unwrap();
        assert!(identities.iter().all(|row| row.id != DELETED_MEMBER_ID));

        let other = db.create_local_user("listed", "hash").await.unwrap();
        let other_member = db
            .create_member(&other.id, "Listed", OrgRole::Member)
            .await
            .unwrap();
        db.upsert_personal_matches(
            DELETED_MEMBER_ID,
            &[sample_match(DELETED_MEMBER_ID, "s-tomb")],
        )
        .await
        .unwrap();
        db.upsert_personal_matches(
            &other_member.id,
            &[sample_match(&other_member.id, "s-live")],
        )
        .await
        .unwrap();
        let rows = db
            .leaderboard_snapshot(None)
            .await
            .unwrap()
            .project("games", "", 25);
        assert!(
            rows.iter().any(|row| row.member_id == other_member.id),
            "a real member still ranks"
        );
        assert!(
            rows.iter().all(|row| row.member_id != DELETED_MEMBER_ID),
            "leaderboard: {rows:?}"
        );

        let listed = db
            .list_members_paginated_filtered(50, 0, true)
            .await
            .unwrap();
        assert!(listed.iter().all(|row| row.id != DELETED_MEMBER_ID));
    }

    async fn count_sql(db: &Database, sql: &str, id: &str) -> i64 {
        #[derive(Deserialize, SurrealValue)]
        struct Row {
            count: i64,
        }
        let mut result = db
            .client
            .query(sql)
            .bind(("mid", id.to_string()))
            .bind(("member_rid", RecordId::new("member", id)))
            .await
            .unwrap();
        let rows: Vec<Row> = result.take(0).unwrap_or_default();
        rows.first().map(|row| row.count).unwrap_or(0)
    }

    async fn text_sql(db: &Database, sql: &str) -> String {
        #[derive(Deserialize, SurrealValue)]
        struct Row {
            value: String,
        }
        let mut result = db.client.query(sql).await.unwrap();
        let rows: Vec<Row> = result.take(0).unwrap();
        rows.into_iter().next().unwrap().value
    }
}
