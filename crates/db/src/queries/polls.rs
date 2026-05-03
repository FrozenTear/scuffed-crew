use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{Poll, PollOptionResult, PollResults, PollVote};
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
    created_at: SurrealDatetime,
    is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbPollVote {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    poll_id: String,
    member_id: String,
    option_index: u32,
    voted_at: SurrealDatetime,
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
        close_at: db.close_at.map(|d| d.into()),
        allow_multiple: db.allow_multiple,
        created_by: db.created_by,
        created_at: db.created_at.into(),
        is_active: db.is_active,
    }
}

fn db_to_vote(db: DbPollVote) -> PollVote {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    PollVote {
        id,
        poll_id: db.poll_id,
        member_id: db.member_id,
        option_index: db.option_index,
        voted_at: db.voted_at.into(),
    }
}

impl Database {
    pub async fn create_poll(
        &self,
        title: &str,
        description: Option<&str>,
        options: &[String],
        close_at: Option<chrono::DateTime<Utc>>,
        allow_multiple: bool,
        created_by: &str,
    ) -> DbResult<Poll> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_poll = DbPoll {
                id: None,
                title: title.to_string(),
                description: description.map(|s| s.to_string()),
                options: options.to_vec(),
                close_at: close_at.map(SurrealDatetime::from),
                allow_multiple,
                created_by: created_by.to_string(),
                created_at: now,
                is_active: true,
            };
            let created: Option<DbPoll> =
                self.client.create("poll").content(db_poll).await?;
            Ok(db_to_poll(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create poll".into())
            })?))
        })
        .await
    }

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

    pub async fn get_poll(&self, id: &str) -> DbResult<Poll> {
        with_timeout(async {
            let poll: Option<DbPoll> = self.client.select(("poll", id)).await?;
            poll.map(db_to_poll)
                .ok_or_else(|| crate::DbError::NotFound(format!("Poll {id} not found")))
        })
        .await
    }

    pub async fn vote_poll(
        &self,
        poll_id: &str,
        member_id: &str,
        option_index: u32,
    ) -> DbResult<PollVote> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let vote = DbPollVote {
                id: None,
                poll_id: poll_id.to_string(),
                member_id: member_id.to_string(),
                option_index,
                voted_at: now,
            };
            let created: Option<DbPollVote> =
                self.client.create("poll_vote").content(vote).await?;
            Ok(db_to_vote(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to record vote".into())
            })?))
        })
        .await
    }

    pub async fn unvote_poll(
        &self,
        poll_id: &str,
        member_id: &str,
        option_index: u32,
    ) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("DELETE FROM poll_vote WHERE poll_id = $pid AND member_id = $mid AND option_index = $oidx")
                .bind(("pid", poll_id.to_string()))
                .bind(("mid", member_id.to_string()))
                .bind(("oidx", option_index as i64))
                .await?;
            Ok(())
        })
        .await
    }

    pub async fn get_poll_results(
        &self,
        poll_id: &str,
        viewer_member_id: Option<&str>,
    ) -> DbResult<PollResults> {
        with_timeout(async {
            let poll = self.get_poll(poll_id).await?;

            let mut result = self
                .client
                .query("SELECT option_index, count() AS count FROM poll_vote WHERE poll_id = $pid GROUP BY option_index")
                .bind(("pid", poll_id.to_string()))
                .await?;

            #[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
            struct VoteCount {
                option_index: u32,
                count: u32,
            }
            let counts: Vec<VoteCount> = result.take(0)?;

            let mut votes: Vec<PollOptionResult> = poll
                .options
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let count = counts
                        .iter()
                        .find(|c| c.option_index == i as u32)
                        .map(|c| c.count)
                        .unwrap_or(0);
                    PollOptionResult {
                        option_index: i as u32,
                        label: label.clone(),
                        count,
                    }
                })
                .collect();
            votes.sort_by(|a, b| b.count.cmp(&a.count));

            let total_votes = votes.iter().map(|v| v.count).sum();

            let my_votes = if let Some(mid) = viewer_member_id {
                let mut r = self
                    .client
                    .query("SELECT option_index FROM poll_vote WHERE poll_id = $pid AND member_id = $mid")
                    .bind(("pid", poll_id.to_string()))
                    .bind(("mid", mid.to_string()))
                    .await?;
                let v: Vec<DbPollVote> = r.take(0)?;
                v.into_iter().map(|v| v.option_index).collect()
            } else {
                vec![]
            };

            Ok(PollResults {
                poll,
                votes,
                total_votes,
                my_votes,
            })
        })
        .await
    }

    pub async fn deactivate_poll(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET is_active = false")
                .bind(("rid", RecordId::new("poll", id)))
                .await?;
            Ok(())
        })
        .await
    }
}
