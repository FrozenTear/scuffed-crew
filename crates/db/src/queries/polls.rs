use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{Poll, PollOptionResult, PollResults};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbPoll {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    title: String,
    description: Option<String>,
    options: Vec<String>,
    close_at: Option<SurrealDatetime>,
    allow_multiple: bool,
    created_by: String,
    is_active: bool,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbPollVote {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    poll_id: String,
    member_id: String,
    option_index: i64,
    created_at: SurrealDatetime,
}

#[derive(Debug, Clone, Deserialize, SurrealValue)]
struct PollVoteCount {
    option_index: i64,
    count: u64,
}

#[derive(Debug, Clone, Deserialize, SurrealValue)]
struct PollVoteIndex {
    option_index: i64,
}

fn db_to_poll(db: DbPoll) -> Poll {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());

    Poll {
        id,
        title: db.title,
        description: db.description,
        options: db.options,
        close_at: db.close_at.map(Into::into),
        allow_multiple: db.allow_multiple,
        created_by: db.created_by,
        is_active: db.is_active,
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
    }
}

impl Database {
    /// Create a new poll.
    pub async fn create_poll(
        &self,
        title: &str,
        description: Option<&str>,
        options: Vec<String>,
        close_at: Option<DateTime<Utc>>,
        allow_multiple: bool,
        created_by: &str,
    ) -> DbResult<Poll> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_poll = DbPoll {
                id: None,
                title: title.to_string(),
                description: description.map(|s| s.to_string()),
                options,
                close_at: close_at.map(SurrealDatetime::from),
                allow_multiple,
                created_by: created_by.to_string(),
                is_active: true,
                created_at: now.clone(),
                updated_at: now,
            };

            let created: Option<DbPoll> = self.client.create("poll").content(db_poll).await?;
            Ok(db_to_poll(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create poll".into())
            })?))
        })
        .await
    }

    /// List active polls ordered by newest first.
    pub async fn list_polls(&self) -> DbResult<Vec<Poll>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM poll WHERE is_active = true ORDER BY created_at DESC")
                .await?;

            let polls: Vec<DbPoll> = result.take(0)?;
            Ok(polls.into_iter().map(db_to_poll).collect())
        })
        .await
    }

    /// Get a poll by id if it is active.
    pub async fn get_poll(&self, id: &str) -> DbResult<Option<Poll>> {
        with_timeout(async {
            let record: Option<DbPoll> = self.client.select(("poll", id)).await?;
            Ok(record.filter(|poll| poll.is_active).map(db_to_poll))
        })
        .await
    }

    /// Cast a vote for a poll option.
    pub async fn vote_poll(
        &self,
        poll_id: &str,
        member_id: &str,
        option_index: u32,
        allow_multiple: bool,
    ) -> DbResult<()> {
        with_timeout(async {
            if !allow_multiple {
                self.client
                    .query("DELETE poll_vote WHERE poll_id = $pid AND member_id = $mid")
                    .bind(("pid", poll_id.to_string()))
                    .bind(("mid", member_id.to_string()))
                    .await?;
            }

            let mut existing_result = self
                .client
                .query("SELECT * FROM poll_vote WHERE poll_id = $pid AND member_id = $mid AND option_index = $idx LIMIT 1")
                .bind(("pid", poll_id.to_string()))
                .bind(("mid", member_id.to_string()))
                .bind(("idx", option_index as i64))
                .await?;
            let existing: Vec<DbPollVote> = existing_result.take(0)?;

            if existing.is_empty() {
                let vote = DbPollVote {
                    id: None,
                    poll_id: poll_id.to_string(),
                    member_id: member_id.to_string(),
                    option_index: option_index as i64,
                    created_at: SurrealDatetime::from(Utc::now()),
                };
                let _: Option<DbPollVote> = self.client.create("poll_vote").content(vote).await?;
            }

            Ok(())
        })
        .await
    }

    /// Remove one vote for a specific poll option.
    pub async fn unvote_poll(
        &self,
        poll_id: &str,
        member_id: &str,
        option_index: u32,
    ) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("DELETE poll_vote WHERE poll_id = $pid AND member_id = $mid AND option_index = $idx")
                .bind(("pid", poll_id.to_string()))
                .bind(("mid", member_id.to_string()))
                .bind(("idx", option_index as i64))
                .await?;
            Ok(())
        })
        .await
    }

    /// Aggregate poll results (counts + percentages per option).
    pub async fn get_poll_results(&self, poll_id: &str) -> DbResult<PollResults> {
        with_timeout(async {
            let poll = self
                .get_poll(poll_id)
                .await?
                .ok_or_else(|| crate::DbError::NotFound(format!("Poll {poll_id} not found")))?;

            let mut result = self
                .client
                .query("SELECT option_index, count() AS count FROM poll_vote WHERE poll_id = $pid GROUP BY option_index ORDER BY option_index ASC")
                .bind(("pid", poll_id.to_string()))
                .await?;

            let rows: Vec<PollVoteCount> = result.take(0)?;
            let mut counts = vec![0u32; poll.options.len()];
            for row in rows {
                if row.option_index < 0 {
                    continue;
                }
                let idx = row.option_index as usize;
                if idx < counts.len() {
                    counts[idx] = row.count as u32;
                }
            }

            let total_votes: u32 = counts.iter().sum();
            let options = poll
                .options
                .iter()
                .enumerate()
                .map(|(idx, text)| {
                    let vote_count = counts[idx];
                    let percentage = if total_votes > 0 {
                        (vote_count as f64 / total_votes as f64) * 100.0
                    } else {
                        0.0
                    };
                    PollOptionResult {
                        option_index: idx as u32,
                        option_text: text.clone(),
                        vote_count,
                        percentage,
                    }
                })
                .collect();

            Ok(PollResults {
                poll_id: poll.id,
                total_votes,
                options,
            })
        })
        .await
    }

    /// Get option indices voted by a specific member for one poll.
    pub async fn get_member_poll_votes(
        &self,
        poll_id: &str,
        member_id: &str,
    ) -> DbResult<Vec<u32>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT option_index FROM poll_vote WHERE poll_id = $pid AND member_id = $mid ORDER BY option_index ASC")
                .bind(("pid", poll_id.to_string()))
                .bind(("mid", member_id.to_string()))
                .await?;

            let rows: Vec<PollVoteIndex> = result.take(0)?;
            Ok(rows
                .into_iter()
                .filter(|row| row.option_index >= 0)
                .map(|row| row.option_index as u32)
                .collect())
        })
        .await
    }

    /// Soft-delete a poll.
    pub async fn deactivate_poll(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET is_active = false, updated_at = $now")
                .bind(("rid", RecordId::new("poll", id)))
                .bind(("now", SurrealDatetime::from(Utc::now())))
                .await?;
            Ok(())
        })
        .await
    }
}
