//! The upload queue: [`SyncClaim`] and the `sync_rev` compare-and-swap in
//! `mark_synced`, upload quarantine and retry, and the reject text.
//!
//! Every payload write elsewhere re-queues its row in the same statement
//! (`synced = false, upload_reject = '', sync_rev = (sync_rev ?? 0) + 1`);
//! the CAS here relies on that.

use super::tombstones::DeletedSession;
use super::{LocalStore, PersonalMatch, rewrite_match_log_session};

/// Identity and payload revision captured when a row was read for upload.
///
/// [`LocalStore::mark_synced`] commits `synced = true` only while `sync_rev`
/// still equals this value. A local write during the upload bumps `sync_rev`
/// in the same statement that changes the payload, so the compare-and-swap
/// misses and the row stays queued.
#[derive(Debug, Clone)]
pub struct SyncClaim {
    pub id: surrealdb_types::RecordId,
    pub sync_rev: u64,
    pub session_id: String,
}

impl SyncClaim {
    /// Claims for rows that have a store id (fresh parses, which have none,
    /// cannot be marked).
    pub fn capture(rows: &[PersonalMatch]) -> Vec<Self> {
        rows.iter()
            .filter_map(|m| {
                m.id.clone().map(|id| SyncClaim {
                    id,
                    sync_rev: m.sync_rev,
                    session_id: m.session_id.clone(),
                })
            })
            .collect()
    }

    /// Empty `session_id` rows are legacy one-offs: each is its own group so a
    /// revision miss on one does not unmark the others.
    fn group_key(&self) -> String {
        if self.session_id.is_empty() {
            format!("id:{:?}", self.id)
        } else {
            self.session_id.clone()
        }
    }

    /// This claim is the uploaded row, or another snapshot of its session.
    /// An empty `session_id` matches only the same store id.
    pub fn covers_row(&self, row: &PersonalMatch) -> bool {
        if row.session_id.is_empty() {
            row.id.as_ref() == Some(&self.id)
        } else {
            self.session_id == row.session_id
        }
    }
}

impl LocalStore {
    /// Rows awaiting upload. Holds back `unknown` outcomes: the server only
    /// stores decided games, and one unstorable row at the head of the queue
    /// used to fail the whole batch forever. An outcome back-fill flips the
    /// row to a decided outcome and `synced = false`, releasing it here.
    ///
    /// A closed session with no result is not sent. [`Self::get_unsynced_unknown`]
    /// loads those rows so the caller can mark them synced with no request.
    /// A GUI `SetOutcome` writes a real outcome and `synced = false`, which
    /// releases the row here.
    ///
    /// Rows with [`PersonalMatch::upload_rejection`] set are quarantined.
    /// They stay local until a retry clears the message.
    pub async fn get_unsynced(
        &self,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        self.pending_upload_rows(false).await
    }

    /// Unsynced rows whose outcome is still `unknown`. The server cannot store
    /// them. Callers mark these synced locally and do not put them in a request.
    pub async fn get_unsynced_unknown(
        &self,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        self.pending_upload_rows(true).await
    }

    async fn pending_upload_rows(
        &self,
        unknown_only: bool,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        let sql = if unknown_only {
            "SELECT * FROM personal_match WHERE synced = false AND outcome = 'unknown' ORDER BY played_at ASC"
        } else {
            "SELECT * FROM personal_match WHERE synced = false AND outcome != 'unknown' ORDER BY played_at ASC"
        };
        let mut result = self.db.query(sql).await?;
        let matches: Vec<PersonalMatch> = result.take(0)?;
        if unknown_only {
            Ok(matches)
        } else {
            Ok(matches
                .into_iter()
                .filter(|row| row.upload_rejection().is_none())
                .collect())
        }
    }

    /// Remember that the server rejected these rows. `synced` stays false
    /// so a later edit or retry can upload them, and [`Self::get_unsynced`]
    /// skips them until then. `code` is the JSON `code` when the body sent
    /// one. The stored text is the game-list line from
    /// [`format_upload_rejection`]. A row whose `sync_rev` changed during
    /// the upload is left alone: the local edit is the payload that should
    /// be retried.
    pub async fn quarantine_upload(
        &self,
        claims: &[SyncClaim],
        code: Option<&str>,
        message: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let message = format_upload_rejection(code, message);
        let mut touched = std::collections::HashSet::new();
        for claim in claims {
            let mut response = self
                .db
                .query(
                    "UPDATE $id SET upload_reject = $msg \
                     WHERE (sync_rev ?? 0) = $rev AND synced = false RETURN AFTER",
                )
                .bind(("id", claim.id.clone()))
                .bind(("msg", message.clone()))
                .bind(("rev", claim.sync_rev))
                .await?;
            let updated: Vec<PersonalMatch> = response.take(0)?;
            if !updated.is_empty() && !claim.session_id.is_empty() {
                touched.insert(claim.session_id.clone());
            }
        }
        for session_id in touched {
            let message = message.clone();
            rewrite_match_log_session(
                &self.data_dir,
                &session_id,
                Some(&move |row| {
                    row.upload_reject = Some(message.clone());
                }),
            );
        }
        Ok(())
    }

    /// Put a quarantined session back on the upload queue.
    pub async fn clear_upload_reject(
        &self,
        session_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query("UPDATE personal_match SET upload_reject = '' WHERE session_id = $sid")
            .bind(("sid", session_id.to_string()))
            .await?;
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|row| {
                row.upload_reject = None;
            }),
        );
        Ok(())
    }

    /// Mark exactly these rows as synced — by identity, not queue position, so
    /// rows inserted while an upload was in flight are never marked by mistake.
    ///
    /// Each claim carries the `sync_rev` observed at read time. The update is
    /// a compare-and-swap: `synced = true` lands only when that revision is
    /// unchanged. A session is one server row (the newest snapshot). If any
    /// claimed snapshot in the session changed during the upload, every claim
    /// in that session is left `synced = false` — marking the unchanged
    /// siblings would let the next sync upload the changed older snapshot
    /// alone and last-write the newer stats off the server.
    pub async fn mark_synced(
        &self,
        claims: &[SyncClaim],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if claims.is_empty() {
            return Ok(());
        }
        let mut groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, claim) in claims.iter().enumerate() {
            groups.entry(claim.group_key()).or_default().push(i);
        }
        for idxs in groups.values() {
            let mut marked: Vec<usize> = Vec::new();
            let mut all_ok = true;
            for &i in idxs {
                if self
                    .cas_set_synced(&claims[i].id, claims[i].sync_rev, true)
                    .await?
                {
                    marked.push(i);
                } else {
                    all_ok = false;
                }
            }
            if !all_ok {
                tracing::info!(
                    session_id = %claims[idxs[0]].session_id,
                    "sync left session unsynced: a snapshot changed during upload"
                );
                for i in marked {
                    self.cas_set_synced(&claims[i].id, claims[i].sync_rev, false)
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// Set `synced` only when `sync_rev` is still `rev`. Returns whether this
    /// call applied the write. A missing `sync_rev` (legacy row) compares as 0.
    async fn cas_set_synced(
        &self,
        id: &surrealdb_types::RecordId,
        rev: u64,
        synced: bool,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut response = self
            .db
            .query("UPDATE $id SET synced = $synced WHERE (sync_rev ?? 0) = $rev RETURN AFTER")
            .bind(("id", id.clone()))
            .bind(("synced", synced))
            .bind(("rev", rev))
            .await?;
        let updated: Vec<PersonalMatch> = response.take(0)?;
        Ok(updated.iter().any(|m| m.synced == synced))
    }

    /// Clear every quarantined upload and put those rows back on the queue.
    pub async fn clear_rejected_uploads(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query(
                "SELECT session_id FROM personal_match \
                 WHERE upload_reject != NONE AND upload_reject != ''",
            )
            .await?;
        let rows: Vec<DeletedSession> = result.take(0)?;
        let mut sessions = Vec::new();
        for row in rows {
            if !sessions.iter().any(|id| id == &row.session_id) {
                sessions.push(row.session_id);
            }
        }
        if sessions.is_empty() {
            return Ok(());
        }
        self.db
            .query("UPDATE personal_match SET upload_reject = '' WHERE session_id IN $sids")
            .bind(("sids", sessions.clone()))
            .await?;
        for session_id in &sessions {
            rewrite_match_log_session(
                &self.data_dir,
                session_id,
                Some(&|row| {
                    row.upload_reject = None;
                }),
            );
        }
        Ok(())
    }
}

/// Game-list line for a row the server refused. The desktop list shows this
/// text next to the rejected flag.
pub const UPLOAD_REJECTED_BY_SERVER: &str = "Upload rejected by server";

/// Drop a leading `matches[<n>]: ` from the server's batch error. The index
/// is the position inside one HTTP body, which changes every time the batch
/// is split, so it is not a stable description of the row.
fn strip_match_index_prefix(message: &str) -> &str {
    let Some(rest) = message.strip_prefix("matches[") else {
        return message;
    };
    let Some(end) = rest.find("]:") else {
        return message;
    };
    let index = &rest[..end];
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return message;
    }
    rest[end + 2..].trim_start()
}

/// Plain wording stored on a quarantined row and shown in the game list.
/// A batch index is dropped. A server `code` is kept in front of the message
/// when the body sent one and the message does not already contain it.
pub fn format_upload_rejection(code: Option<&str>, message: &str) -> String {
    let mut detail = strip_match_index_prefix(message.trim()).trim().to_string();
    if let Some(rest) = detail.strip_prefix(UPLOAD_REJECTED_BY_SERVER) {
        detail = rest.trim().trim_start_matches(':').trim().to_string();
    }
    if let Some(code) = code.map(str::trim).filter(|code| !code.is_empty())
        && !detail.contains(code)
    {
        detail = if detail.is_empty() {
            code.to_string()
        } else {
            format!("{code}: {detail}")
        };
    }
    let text = if detail.is_empty() {
        UPLOAD_REJECTED_BY_SERVER.to_string()
    } else {
        format!("{UPLOAD_REJECTED_BY_SERVER}: {detail}")
    };
    clip_chars(&text, 400)
}

fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let end = text
        .char_indices()
        .nth(max)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    format!("{}...", &text[..end])
}
