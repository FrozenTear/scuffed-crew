//! The queue title on a game ([`LocalStore::attach_queue_title`]) and the
//! carry that keeps it on every snapshot of the match. Same shape as the
//! bans and picks result (`storage::bans_picks`).

use crate::model::QueueTitle;

use super::{LocalStore, rewrite_match_log_session};

impl LocalStore {
    /// Store `title` (`PersonalMatch::queue_title`) on every snapshot of
    /// `session_id`. `false`, nothing written, when the game is unknown or
    /// already has exactly this value. The caller decides what replaces
    /// what (`daemon::queue_title::apply_read`).
    pub async fn attach_queue_title(
        &self,
        session_id: &str,
        title: &QueueTitle,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let _rev = self.rev_guard();
        let snaps = self.get_session_snapshots(session_id).await?;
        if snaps.is_empty() || snaps.iter().all(|m| m.queue_title.as_ref() == Some(title)) {
            return Ok(false);
        }
        self.db
            .query("UPDATE personal_match SET queue_title = $qt WHERE session_id = $sid")
            .bind(("qt", title.clone()))
            .bind(("sid", session_id.to_string()))
            .await?
            .check()?;
        let title = title.clone();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| m.queue_title = Some(title.clone())),
        );
        Ok(true)
    }

    /// The queue title already stored for this match: what a new read is
    /// applied over, and what a later snapshot with none of its own keeps.
    pub async fn session_queue_title(
        &self,
        session_id: &str,
    ) -> Result<Option<QueueTitle>, Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        Ok(snaps.into_iter().rev().find_map(|row| row.queue_title))
    }
}
