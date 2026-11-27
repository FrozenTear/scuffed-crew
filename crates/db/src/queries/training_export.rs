//! Training crop export for Scuffed Vision: read-only export tokens, the
//! export feed and its tombstones.
//!
//! The feed is two tables. `training_crop_export_entry` holds every crop
//! that is in the feed right now, keyed by crop id, with the time it was
//! placed there (`listed_at`). `training_crop_tombstone` holds crops that
//! left it. [`Database::sync_training_export`] brings both in line with what
//! may be exported right now; the export route runs it before every read, so
//! every way a crop stops being eligible (any delete route, consent
//! withdrawal, a member leaving, being deactivated, suspended or banned) is
//! caught in one place and no delete path has to remember to write a
//! tombstone.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use scuffed_auth::crypto::hash_session_token;
use scuffed_types::CropKind;

use crate::queries::hero_icons::stored_kind;
use crate::types::{TrainingExportEvent, TrainingExportPosition, TrainingExportToken};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbExportToken {
    #[surreal(default)]
    #[serde(default)]
    id: Option<RecordId>,
    token_hash: String,
    label: String,
    created_by: String,
    created_at: SurrealDatetime,
    expires_at: SurrealDatetime,
    #[surreal(default)]
    #[serde(default)]
    revoked_at: Option<SurrealDatetime>,
    pseudonym_salt: String,
}

fn db_to_token(db: DbExportToken) -> TrainingExportToken {
    TrainingExportToken {
        id: db
            .id
            .map(|rid| crate::record_id_key_to_string(rid.key))
            .unwrap_or_default(),
        label: db.label,
        created_by: db.created_by,
        created_at: db.created_at.into(),
        expires_at: db.expires_at.into(),
        revoked_at: db.revoked_at.map(Into::into),
        pseudonym_salt: db.pseudonym_salt,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbExportEntry {
    id: RecordId,
    crop_id: String,
    kind: String,
    listed_at: SurrealDatetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbTombstone {
    crop_id: String,
    kind: String,
    reason: String,
    at: SurrealDatetime,
}

/// Why a crop left the feed. Stored for operators; not sent to the puller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TombstoneReason {
    Deleted,
    ConsentWithdrawn,
    MemberInactive,
}

impl TombstoneReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deleted => "deleted",
            Self::ConsentWithdrawn => "consent_withdrawn",
            Self::MemberInactive => "member_inactive",
        }
    }
}

/// What one sync changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrainingExportSync {
    pub listed: usize,
    pub removed: usize,
}

const TOKEN_ACTIVE_SQL: &str = "SELECT * FROM training_export_token WHERE token_hash = $tok \
     AND revoked_at IS NONE AND expires_at > time::now()";

/// The WHERE fragment for one feed table after `after`, as fixed text with
/// `$at` and `$cid` placeholders. `tag` is the table's place in the order
/// (0 items, 1 tombstones).
fn after_clause(col: &str, tag: u8, after: Option<&TrainingExportPosition>) -> String {
    let Some(after) = after else {
        return "true".to_string();
    };
    match after.tag.cmp(&tag) {
        // Same table: strictly after the cursor's (time, crop id).
        std::cmp::Ordering::Equal => {
            format!("({col} > $at OR ({col} = $at AND crop_id > $cid))")
        }
        // The cursor sits before this table at that time: all of that instant.
        std::cmp::Ordering::Less => format!("{col} >= $at"),
        // The cursor is past this table at that time.
        std::cmp::Ordering::Greater => format!("{col} > $at"),
    }
}

impl Database {
    pub async fn create_training_export_token(
        &self,
        created_by: &str,
        raw_token: &str,
        label: &str,
        expires_at: DateTime<Utc>,
        pseudonym_salt: &str,
    ) -> DbResult<TrainingExportToken> {
        let row = DbExportToken {
            id: None,
            token_hash: hash_session_token(raw_token),
            label: label.to_string(),
            created_by: created_by.to_string(),
            created_at: SurrealDatetime::from(Utc::now()),
            expires_at: SurrealDatetime::from(expires_at),
            revoked_at: None,
            pseudonym_salt: pseudonym_salt.to_string(),
        };
        with_timeout(async {
            let created: Option<DbExportToken> = self
                .client
                .create("training_export_token")
                .content(row)
                .await?;
            created
                .map(db_to_token)
                .ok_or_else(|| crate::DbError::NotFound("training export token create".into()))
        })
        .await
    }

    /// Every export token, newest first.
    pub async fn list_training_export_tokens(&self) -> DbResult<Vec<TrainingExportToken>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM training_export_token ORDER BY created_at DESC")
                .await?;
            let rows: Vec<DbExportToken> = result.take(0)?;
            Ok(rows.into_iter().map(db_to_token).collect())
        })
        .await
    }

    /// Revoke an unrevoked token. `None` when no unrevoked token has this id.
    pub async fn revoke_training_export_token(
        &self,
        id: &str,
    ) -> DbResult<Option<TrainingExportToken>> {
        let rid = RecordId::new("training_export_token", id);
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "UPDATE $rid SET revoked_at = time::now() WHERE revoked_at IS NONE \
                     RETURN AFTER",
                )
                .bind(("rid", rid))
                .await?;
            let rows: Vec<DbExportToken> = result.take(0)?;
            Ok(rows.into_iter().next().map(db_to_token))
        })
        .await
    }

    /// The token for a presented secret when it is unrevoked and unexpired.
    /// Does not write.
    pub async fn find_training_export_token(
        &self,
        raw_token: &str,
    ) -> DbResult<Option<TrainingExportToken>> {
        let token_hash = hash_session_token(raw_token);
        with_timeout(async {
            let mut result = self
                .client
                .query(TOKEN_ACTIVE_SQL)
                .bind(("tok", token_hash))
                .await?;
            let rows: Vec<DbExportToken> = result.take(0)?;
            Ok(rows.into_iter().next().map(db_to_token))
        })
        .await
    }

    /// Members with an active, unexpired suspension or ban. Same rule as
    /// [`Database::is_member_suspended_or_banned`], for everyone at once.
    pub async fn suspended_or_banned_member_ids(&self) -> DbResult<HashSet<String>> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                member_id: String,
            }
            let mut result = self
                .client
                .query(
                    "SELECT member_id FROM moderation_action WHERE is_active = true \
                     AND action_type IN ['suspension', 'ban'] \
                     AND (expires_at IS NONE OR expires_at > time::now())",
                )
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            Ok(rows.into_iter().map(|row| row.member_id).collect())
        })
        .await
    }

    /// Bring the feed in line with what may be exported now, writing at
    /// `stamp`.
    ///
    /// Eligible: a stored kind, owner active and not suspended or banned,
    /// owner consenting to the kind now. An eligible crop not in the feed is
    /// listed at `stamp` (that includes a crop listed before, removed, and
    /// eligible again: it comes back at a new position). A crop in the feed
    /// that is not eligible leaves it with a tombstone at `stamp`. Both
    /// writes go in one transaction, so a crop never leaves the feed without
    /// its tombstone.
    ///
    /// The caller serialises calls and passes strictly increasing stamps
    /// with microsecond precision.
    pub async fn sync_training_export(&self, stamp: DateTime<Utc>) -> DbResult<TrainingExportSync> {
        #[derive(Deserialize, SurrealValue)]
        struct CropRow {
            id: Option<RecordId>,
            member_id: String,
            #[serde(default)]
            #[surreal(default)]
            kind: Option<String>,
        }
        #[derive(Deserialize, SurrealValue)]
        struct EntryRow {
            crop_id: String,
            kind: String,
        }
        let (crops, entries) = with_timeout(async {
            let mut result = self
                .client
                .query("SELECT id, member_id, kind FROM hero_icon")
                .query("SELECT crop_id, kind FROM training_crop_export_entry")
                .await?;
            let crops: Vec<CropRow> = result.take(0)?;
            let entries: Vec<EntryRow> = result.take(1)?;
            Ok::<_, crate::DbError>((crops, entries))
        })
        .await?;

        let mut owners: Vec<String> = crops.iter().map(|c| c.member_id.clone()).collect();
        owners.sort();
        owners.dedup();
        let active = self.active_member_display_names(&owners).await?;
        let blocked = self.suspended_or_banned_member_ids().await?;
        let consents = self.training_crop_consents().await?;

        // crop id -> (owner usable, consented, kind)
        let mut stored: HashMap<String, (bool, bool, CropKind)> = HashMap::new();
        for crop in crops {
            let (Some(rid), Some(kind)) = (crop.id, stored_kind(crop.kind.as_deref())) else {
                continue;
            };
            if !kind.accepts_uploads() {
                continue;
            }
            let usable = active.contains_key(&crop.member_id) && !blocked.contains(&crop.member_id);
            let consented = consents
                .get(&crop.member_id)
                .is_some_and(|consent| consent.get(kind));
            stored.insert(
                crate::record_id_key_to_string(rid.key),
                (usable, consented, kind),
            );
        }

        let at = SurrealDatetime::from(stamp);
        let listed: HashSet<&str> = entries.iter().map(|e| e.crop_id.as_str()).collect();
        let mut tombstones: Vec<DbTombstone> = Vec::new();
        for entry in &entries {
            let reason = match stored.get(&entry.crop_id) {
                None => TombstoneReason::Deleted,
                Some((false, _, _)) => TombstoneReason::MemberInactive,
                Some((true, false, _)) => TombstoneReason::ConsentWithdrawn,
                Some((true, true, _)) => continue,
            };
            tombstones.push(DbTombstone {
                crop_id: entry.crop_id.clone(),
                kind: entry.kind.clone(),
                reason: reason.as_str().to_string(),
                at,
            });
        }
        let new_entries: Vec<DbExportEntry> = stored
            .iter()
            .filter(|(id, (usable, consented, _))| {
                *usable && *consented && !listed.contains(id.as_str())
            })
            .map(|(id, (_, _, kind))| DbExportEntry {
                id: RecordId::new("training_crop_export_entry", id.as_str()),
                crop_id: id.clone(),
                kind: kind.as_str().to_string(),
                listed_at: at,
            })
            .collect();
        let outcome = TrainingExportSync {
            listed: new_entries.len(),
            removed: tombstones.len(),
        };
        if tombstones.is_empty() && new_entries.is_empty() {
            return Ok(outcome);
        }
        let gone: Vec<RecordId> = tombstones
            .iter()
            .map(|t| RecordId::new("training_crop_export_entry", t.crop_id.as_str()))
            .collect();
        with_timeout(async {
            self.client
                .query(
                    "BEGIN TRANSACTION; \
                     DELETE training_crop_export_entry WHERE id IN $gone; \
                     IF array::len($tombs) > 0 { INSERT INTO training_crop_tombstone $tombs; }; \
                     IF array::len($fresh) > 0 { INSERT INTO training_crop_export_entry $fresh; }; \
                     COMMIT TRANSACTION;",
                )
                .bind(("gone", gone))
                .bind(("tombs", tombstones))
                .bind(("fresh", new_entries))
                .await?
                .check()?;
            Ok(outcome)
        })
        .await
    }

    /// Remove tombstones written before `cutoff`. Returns how many went.
    pub async fn prune_training_tombstones(&self, cutoff: DateTime<Utc>) -> DbResult<usize> {
        with_timeout(async {
            let mut result = self
                .client
                .query("DELETE training_crop_tombstone WHERE at < $cutoff RETURN BEFORE")
                .bind(("cutoff", SurrealDatetime::from(cutoff)))
                .await?;
            let rows: Vec<surrealdb::types::Value> = result.take(0)?;
            Ok(rows.len())
        })
        .await
    }

    /// Up to `limit` items and up to `limit` tombstones after `after`
    /// (oldest first each), optionally of one kind. The caller merges them
    /// by position.
    pub async fn training_export_events_after(
        &self,
        after: Option<&TrainingExportPosition>,
        kind: Option<CropKind>,
        limit: u32,
    ) -> DbResult<Vec<TrainingExportEvent>> {
        #[derive(Deserialize, SurrealValue)]
        struct Row {
            crop_id: String,
            kind: String,
            t: SurrealDatetime,
        }
        let kind_clause = if kind.is_some() {
            " AND kind = $kind"
        } else {
            ""
        };
        let items_sql = format!(
            "SELECT crop_id, kind, listed_at AS t FROM training_crop_export_entry \
             WHERE {}{kind_clause} ORDER BY t ASC, crop_id ASC LIMIT $lim",
            after_clause("listed_at", 0, after)
        );
        let tombs_sql = format!(
            "SELECT crop_id, kind, at AS t FROM training_crop_tombstone \
             WHERE {}{kind_clause} ORDER BY t ASC, crop_id ASC LIMIT $lim",
            after_clause("at", 1, after)
        );
        let (at, cid) = match after {
            Some(pos) => (
                Some(SurrealDatetime::from(pos.at)),
                Some(pos.crop_id.clone()),
            ),
            None => (None, None),
        };
        with_timeout(async {
            let mut result = self
                .client
                .query(items_sql)
                .query(tombs_sql)
                .bind(("at", at))
                .bind(("cid", cid))
                .bind(("kind", kind.map(|k| k.as_str().to_string())))
                .bind(("lim", limit))
                .await?;
            let items: Vec<Row> = result.take(0)?;
            let tombs: Vec<Row> = result.take(1)?;
            let to_event = |row: Row, tag: u8| {
                Some(TrainingExportEvent {
                    kind: CropKind::parse(&row.kind)?,
                    position: TrainingExportPosition {
                        at: row.t.into(),
                        tag,
                        crop_id: row.crop_id,
                    },
                    removed: tag == 1,
                })
            };
            let mut events: Vec<TrainingExportEvent> = items
                .into_iter()
                .filter_map(|row| to_event(row, 0))
                .chain(tombs.into_iter().filter_map(|row| to_event(row, 1)))
                .collect();
            events.sort_by(|a, b| a.position.cmp(&b.position));
            Ok(events)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::migrations::run_migrations;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        db
    }

    #[tokio::test]
    async fn token_lookup_skips_revoked_and_expired() {
        let db = test_db().await;
        let later = Utc::now() + Duration::days(1);
        let live = db
            .create_training_export_token("admin", "live-secret", "live", later, "aa")
            .await
            .unwrap();
        db.create_training_export_token(
            "admin",
            "old-secret",
            "old",
            Utc::now() - Duration::seconds(1),
            "bb",
        )
        .await
        .unwrap();
        let found = db.find_training_export_token("live-secret").await.unwrap();
        assert_eq!(found.map(|t| t.id), Some(live.id.clone()));
        assert!(db
            .find_training_export_token("old-secret")
            .await
            .unwrap()
            .is_none());
        assert!(db
            .find_training_export_token("nope")
            .await
            .unwrap()
            .is_none());
        let revoked = db.revoke_training_export_token(&live.id).await.unwrap();
        assert!(revoked.and_then(|t| t.revoked_at).is_some());
        assert!(db
            .find_training_export_token("live-secret")
            .await
            .unwrap()
            .is_none());
        assert!(db
            .revoke_training_export_token(&live.id)
            .await
            .unwrap()
            .is_none());
        assert_eq!(db.list_training_export_tokens().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn prune_drops_only_old_tombstones() {
        let db = test_db().await;
        db.client
            .query(
                "CREATE training_crop_tombstone SET crop_id = 'a', kind = 'map_name', \
                 reason = 'deleted', at = time::now() - 61d; \
                 CREATE training_crop_tombstone SET crop_id = 'b', kind = 'map_name', \
                 reason = 'deleted', at = time::now() - 59d;",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let pruned = db
            .prune_training_tombstones(Utc::now() - Duration::days(60))
            .await
            .unwrap();
        assert_eq!(pruned, 1);
        let events = db
            .training_export_events_after(None, None, 10)
            .await
            .unwrap();
        let ids: Vec<&str> = events.iter().map(|e| e.position.crop_id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
    }
}
