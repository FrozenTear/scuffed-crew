//! The queue title on a game ([`LocalStore::attach_queue_title`]) and the
//! carry that keeps it on every snapshot of the match. Same shape as the
//! bans and picks result (`storage::bans_picks`): written once per game.

use crate::model::QueueTitle;

use super::{LocalStore, rewrite_match_log_session};

impl LocalStore {
    /// Store `title` (`PersonalMatch::queue_title`) on every snapshot of
    /// `session_id`. `false`, nothing written, when the game is unknown or
    /// already has a queue title: the first one stored stays, so a later
    /// read (say after a restart mid-game, with no frames held) can never
    /// replace a confident title with a warning.
    ///
    /// A title that changes the `rank` object the upload sends for the game
    /// (`sync::wire::queue_title_changes_upload`) re-queues it in the same
    /// statement (`storage::upload_queue`), as every payload write does. One
    /// that changes no uploaded byte leaves the queue, and a set-aside game,
    /// as it is: a warning, and, on a game with no rank screen read, a title
    /// naming a mode that is not Competitive (QUICK, UNRANKED, ARCADE,
    /// STADIUM, CUSTOM) or naming neither ROLE nor OPEN. On a game with a
    /// rank screen read those titles do change the body, so they re-queue
    /// it. Same rule as `attach_rank_update`.
    pub async fn attach_queue_title(
        &self,
        session_id: &str,
        title: &QueueTitle,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let _rev = self.rev_guard();
        let snaps = self.get_session_snapshots(session_id).await?;
        if snaps.is_empty() || snaps.iter().any(|m| m.queue_title.is_some()) {
            return Ok(false);
        }
        // Every snapshot of a game shares its title and rank update, and the
        // team size never decides whether the `rank` object exists, so a
        // title changes every snapshot's body or none: `any` is `all`.
        let uploaded = snaps
            .iter()
            .any(|m| crate::sync::queue_title_changes_upload(m, title));
        let sql = if uploaded {
            "UPDATE personal_match SET queue_title = $qt, synced = false, \
             upload_reject = '', sync_rev = (sync_rev ?? 0) + 1 WHERE session_id = $sid"
        } else {
            "UPDATE personal_match SET queue_title = $qt WHERE session_id = $sid"
        };
        self.db
            .query(sql)
            .bind(("qt", title.clone()))
            .bind(("sid", session_id.to_string()))
            .await?
            .check()?;
        let title = title.clone();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.queue_title = Some(title.clone());
                if uploaded {
                    m.upload_reject = None;
                }
            }),
        );
        Ok(true)
    }

    /// The queue title already stored for this match: a game that has one
    /// is not read again.
    pub async fn session_queue_title(
        &self,
        session_id: &str,
    ) -> Result<Option<QueueTitle>, Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        Ok(snaps.into_iter().rev().find_map(|row| row.queue_title))
    }
}
