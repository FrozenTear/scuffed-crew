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
    updated_at: SurrealDatetime,
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

/// Translate a SurrealDB UNIQUE-index violation on `poll_vote` into a
/// [`crate::DbError::Conflict`] (→ 409) so a duplicate same-option vote is a
/// clean conflict rather than a raw 500. Any other error passes through.
fn map_vote_unique_violation(e: surrealdb::Error) -> crate::DbError {
    // SurrealDB reports unique violations as:
    //   "Database index `poll_vote_unique_idx` already contains [...]"
    if e.to_string().contains("already contains") {
        crate::DbError::Conflict("You have already voted for this option".into())
    } else {
        crate::DbError::Surreal(e)
    }
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
        updated_at: db.updated_at.into(),
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
                updated_at: now,
            };
            let created: Option<DbPoll> = self.client.create("poll").content(db_poll).await?;
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

    /// Record a member's vote for `option_index` on a poll, enforcing poll
    /// integrity that the `poll_vote` UNIQUE index alone cannot express.
    ///
    /// Integrity rules (authoritative here, independent of the route):
    /// - Poll must exist (`NotFound` otherwise) and be active; voting on a
    ///   closed poll (`is_active = false`) returns `Conflict`.
    /// - Single-choice polls (`allow_multiple = false`) keep at most one vote
    ///   per member: selecting a different option **replaces** the previous
    ///   vote (radio-button semantics — matches the poll UI, which offers no
    ///   unvote-first step for single choice). We delete the member's votes on
    ///   any *other* option, then create the chosen one; the delete never
    ///   touches the chosen option, so a same-option repeat still trips the
    ///   UNIQUE index with nothing lost.
    /// - Voting the *same* option twice (any poll) hits the UNIQUE index and
    ///   returns `Conflict` (→ 409), never a raw 500.
    ///
    /// `allow_multiple` polls keep the original behavior: a member may hold
    /// one vote per distinct option.
    pub async fn vote_poll(
        &self,
        poll_id: &str,
        member_id: &str,
        option_index: u32,
    ) -> DbResult<PollVote> {
        with_timeout(async {
            // Load the poll so integrity is enforced in the DB layer regardless
            // of what the caller checked. Missing poll → NotFound.
            let poll = self.get_poll(poll_id).await?;
            if !poll.is_active {
                return Err(crate::DbError::Conflict("Poll is closed".into()));
            }

            let now = SurrealDatetime::from(Utc::now());

            // Single-choice polls first clear any existing vote this member has
            // on a DIFFERENT option so the new choice replaces it. The delete
            // intentionally excludes `option_index = $oidx`, so re-voting the
            // same option deletes nothing and the create below still trips the
            // UNIQUE index (→ Conflict) with the prior vote left intact.
            if !poll.allow_multiple {
                self.client
                    .query(
                        "DELETE poll_vote \
                         WHERE poll_id = $pid AND member_id = $mid AND option_index != $oidx",
                    )
                    .bind(("pid", poll_id.to_string()))
                    .bind(("mid", member_id.to_string()))
                    .bind(("oidx", option_index as i64))
                    .await?;
            }

            // Create the chosen vote. The UNIQUE index (poll_id, member_id,
            // option_index) blocks a same-option repeat on either poll type;
            // map that violation to a Conflict instead of a raw DB error so the
            // route answers 409 rather than 500.
            let vote = DbPollVote {
                id: None,
                poll_id: poll_id.to_string(),
                member_id: member_id.to_string(),
                option_index,
                voted_at: now,
            };
            let created: Option<DbPollVote> = self
                .client
                .create("poll_vote")
                .content(vote)
                .await
                .map_err(map_vote_unique_violation)?;
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
            votes.sort_by_key(|b| std::cmp::Reverse(b.count));

            let total_votes = votes.iter().map(|v| v.count).sum();

            let my_votes = if let Some(mid) = viewer_member_id {
                #[derive(Debug, Clone, Deserialize, SurrealValue)]
                struct VoteIdx {
                    option_index: u32,
                }
                let mut r = self
                    .client
                    .query("SELECT option_index FROM poll_vote WHERE poll_id = $pid AND member_id = $mid")
                    .bind(("pid", poll_id.to_string()))
                    .bind(("mid", mid.to_string()))
                    .await?;
                let v: Vec<VoteIdx> = r.take(0)?;
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

#[cfg(test)]
mod tests {
    use crate::migrations::run_migrations;
    use crate::{Database, DbError};

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    async fn make_poll(db: &Database, allow_multiple: bool) -> String {
        db.create_poll(
            "Best map?",
            None,
            &["Oasis".into(), "Ilios".into(), "Nepal".into()],
            None,
            allow_multiple,
            "creator",
        )
        .await
        .unwrap()
        .id
    }

    /// Single-choice: voting a second, different option REPLACES the first,
    /// leaving exactly one counted vote for the member (radio semantics).
    #[tokio::test]
    async fn single_choice_switching_option_replaces_vote() {
        let db = test_db().await;
        let poll = make_poll(&db, false).await;

        db.vote_poll(&poll, "m1", 0).await.unwrap();
        db.vote_poll(&poll, "m1", 1).await.unwrap();

        let r = db.get_poll_results(&poll, Some("m1")).await.unwrap();
        assert_eq!(r.total_votes, 1, "single-choice member must count once");
        assert_eq!(r.my_votes, vec![1], "latest option replaces the prior one");
    }

    /// Single-choice: re-voting the SAME option is a UNIQUE-index conflict,
    /// mapped to `DbError::Conflict` (→ 409) and leaving the vote intact.
    #[tokio::test]
    async fn single_choice_repeat_same_option_conflict() {
        let db = test_db().await;
        let poll = make_poll(&db, false).await;

        db.vote_poll(&poll, "m1", 0).await.unwrap();
        let err = db.vote_poll(&poll, "m1", 0).await.unwrap_err();
        assert!(
            matches!(err, DbError::Conflict(_)),
            "same-option repeat must be Conflict (409), got {err:?}"
        );

        // Transaction rolled back: still exactly one vote on option 0.
        let r = db.get_poll_results(&poll, Some("m1")).await.unwrap();
        assert_eq!(r.total_votes, 1);
        assert_eq!(r.my_votes, vec![0]);
    }

    /// A closed poll rejects new votes with `Conflict` (→ 409).
    #[tokio::test]
    async fn closed_poll_rejects_vote() {
        let db = test_db().await;
        let poll = make_poll(&db, false).await;
        db.deactivate_poll(&poll).await.unwrap();

        let err = db.vote_poll(&poll, "m1", 0).await.unwrap_err();
        assert!(
            matches!(err, DbError::Conflict(_)),
            "closed poll must reject votes, got {err:?}"
        );
        let r = db.get_poll_results(&poll, Some("m1")).await.unwrap();
        assert_eq!(r.total_votes, 0, "no vote recorded on a closed poll");
    }

    /// Multiple-choice: a member may hold votes on several distinct options.
    #[tokio::test]
    async fn allow_multiple_accepts_distinct_options() {
        let db = test_db().await;
        let poll = make_poll(&db, true).await;

        db.vote_poll(&poll, "m1", 0).await.unwrap();
        db.vote_poll(&poll, "m1", 2).await.unwrap();

        let r = db.get_poll_results(&poll, Some("m1")).await.unwrap();
        assert_eq!(r.total_votes, 2, "both distinct options count");
        let mut mine = r.my_votes.clone();
        mine.sort_unstable();
        assert_eq!(mine, vec![0, 2]);
    }

    /// Multiple-choice: re-voting the SAME option is still a `Conflict` (→ 409),
    /// not a raw DB error / 500.
    #[tokio::test]
    async fn allow_multiple_repeat_same_option_conflict() {
        let db = test_db().await;
        let poll = make_poll(&db, true).await;

        db.vote_poll(&poll, "m1", 0).await.unwrap();
        let err = db.vote_poll(&poll, "m1", 0).await.unwrap_err();
        assert!(
            matches!(err, DbError::Conflict(_)),
            "same-option repeat must be Conflict (409), got {err:?}"
        );

        let r = db.get_poll_results(&poll, Some("m1")).await.unwrap();
        assert_eq!(r.total_votes, 1);
    }

    /// Two members voting the same single-choice option are independent.
    #[tokio::test]
    async fn single_choice_is_per_member() {
        let db = test_db().await;
        let poll = make_poll(&db, false).await;

        db.vote_poll(&poll, "m1", 0).await.unwrap();
        db.vote_poll(&poll, "m2", 0).await.unwrap();

        let r = db.get_poll_results(&poll, None).await.unwrap();
        assert_eq!(r.total_votes, 2, "distinct members both count");
    }

    /// Voting on a poll that does not exist is `NotFound` (→ 404), not 500.
    #[tokio::test]
    async fn vote_on_missing_poll_is_not_found() {
        let db = test_db().await;
        let err = db.vote_poll("does_not_exist", "m1", 0).await.unwrap_err();
        assert!(
            matches!(err, DbError::NotFound(_)),
            "missing poll must be NotFound (404), got {err:?}"
        );
    }
}
