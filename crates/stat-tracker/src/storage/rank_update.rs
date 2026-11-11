//! The rank update on a game ([`LocalStore::attach_rank_update`]) and the
//! carry that keeps it on every snapshot of the match. Same shape as the
//! queue title (`storage::queue_title`): written once per game.

use crate::model::RankUpdate;

use super::{LocalStore, rewrite_match_log_session};

impl LocalStore {
    /// Store `rank` (`PersonalMatch::rank_update`) on every snapshot of
    /// `session_id`. `false`, nothing written, when the game is unknown,
    /// already has a rank update, or `rank` holds no field: the first one
    /// stored stays.
    ///
    /// The rank update is part of the uploaded `rank`, so the write
    /// re-queues the game for upload in the same statement
    /// (`storage::upload_queue`), as every payload write does.
    pub async fn attach_rank_update(
        &self,
        session_id: &str,
        rank: &RankUpdate,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        if rank.is_empty() {
            return Ok(false);
        }
        let _rev = self.rev_guard();
        let snaps = self.get_session_snapshots(session_id).await?;
        if snaps.is_empty() || snaps.iter().any(|m| m.rank_update.is_some()) {
            return Ok(false);
        }
        self.db
            .query(
                "UPDATE personal_match SET rank_update = $ru, synced = false, \
                 upload_reject = '', sync_rev = (sync_rev ?? 0) + 1 WHERE session_id = $sid",
            )
            .bind(("ru", rank.clone()))
            .bind(("sid", session_id.to_string()))
            .await?
            .check()?;
        let rank = rank.clone();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.rank_update = Some(rank.clone());
                m.upload_reject = None;
            }),
        );
        Ok(true)
    }
}
