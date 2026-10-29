//! Game deletes and the server-delete tombstones (`deleted_session`).

use serde::{Deserialize, Serialize};
use surrealdb_types::SurrealValue;

use super::{LocalStore, rewrite_match_log_session};

impl LocalStore {
    /// Delete a session and all its capture snapshots (manual cleanup of a
    /// junk/misdetected game). Records a tombstone so the next sync deletes
    /// the game server-side too, and drops the session from the append-only
    /// `matches.jsonl` record.
    ///
    /// The writes (the game's hero timeline record too) run in one
    /// transaction and every statement is checked. Unchecked, a failed
    /// tombstone write still returned Ok after the local rows were gone: the
    /// game vanished from the app but nothing was left to delete it from the
    /// site.
    pub async fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _rev = self.rev_guard();
        // Marked before the delete, so a capture insert cannot land between
        // the delete and the mark. A failed delete (the query or any checked
        // statement) unmarks it, or every later row of that game would be
        // dropped for the rest of the process.
        let newly_marked = self
            .deleted_here
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session_id.to_string());
        let deleted = match self
            .db
            .query(
                "BEGIN TRANSACTION; \
                 DELETE match_session WHERE session_id = $sid; \
                 DELETE personal_match WHERE session_id = $sid; \
                 CREATE deleted_session SET session_id = $sid, deleted_at = time::now(); \
                 DELETE $tid; \
                 COMMIT TRANSACTION;",
            )
            .bind(("sid", session_id.to_string()))
            .bind(("tid", super::timeline::timeline_id(session_id)))
            .await
        {
            Ok(response) => response.check().map(|_| ()),
            Err(e) => Err(e),
        };
        if let Err(e) = deleted {
            if newly_marked {
                self.deleted_here
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(session_id);
            }
            return Err(e.into());
        }
        rewrite_match_log_session(&self.data_dir, session_id, None);
        Ok(())
    }

    /// Queue server deletes for sessions that stay local from now on (an
    /// untracked map or mode after a re-read or an edit). An earlier snapshot
    /// of such a game may already be on the site; the tombstone removes it.
    /// A session that already has a pending tombstone is not queued twice.
    /// A server delete of a session it never stored is a no-op.
    pub async fn queue_tombstones(
        &self,
        session_ids: &[String],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _rev = self.rev_guard();
        let mut sids: Vec<String> = session_ids
            .iter()
            .filter(|sid| !sid.is_empty())
            .cloned()
            .collect();
        sids.sort();
        sids.dedup();
        if sids.is_empty() {
            return Ok(());
        }
        let pending = self.get_pending_tombstones().await?;
        sids.retain(|sid| !pending.contains(sid));
        for sid in sids {
            self.db
                .query("CREATE deleted_session SET session_id = $sid, deleted_at = time::now()")
                .bind(("sid", sid))
                .await?
                .check()?;
        }
        Ok(())
    }

    /// Session ids deleted locally but not yet propagated to the server.
    pub async fn get_pending_tombstones(
        &self,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query("SELECT session_id FROM deleted_session")
            .await?;
        let rows: Vec<DeletedSession> = result.take(0)?;
        Ok(rows.into_iter().map(|r| r.session_id).collect())
    }

    /// Drop tombstones the server has acknowledged.
    pub async fn clear_tombstones(
        &self,
        session_ids: Vec<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _rev = self.rev_guard();
        if session_ids.is_empty() {
            return Ok(());
        }
        self.db
            .query("DELETE deleted_session WHERE session_id IN $sids")
            .bind(("sids", session_ids))
            .await?;
        Ok(())
    }

    /// Re-create a tombstone row (vacuum copies pending ones into the fresh
    /// store so un-acked server-side deletes still propagate).
    pub(super) async fn insert_tombstone(
        &self,
        session_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query("CREATE deleted_session SET session_id = $sid, deleted_at = time::now()")
            .bind(("sid", session_id.to_string()))
            .await?;
        Ok(())
    }
}

/// Local record of a session deleted while (possibly) already synced — the
/// next sync sends these so the server row disappears too.
#[derive(Debug, Serialize, Deserialize, SurrealValue)]
pub(super) struct DeletedSession {
    pub(super) session_id: String,
}
