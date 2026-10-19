use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use surrealdb::method::Transaction;
use surrealdb::Connection;

use crate::types::{EventRsvp, RsvpStatus, RsvpSummary};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Deserialize, SurrealValue)]
struct EventActive {
    is_active: bool,
}

async fn event_is_active<C: Connection>(tx: &Transaction<C>, event_id: &str) -> DbResult<bool> {
    let mut result = tx
        .query("SELECT is_active FROM $rid")
        .bind(("rid", RecordId::new("event", event_id)))
        .await?;
    let rows: Vec<EventActive> = result.take(0)?;
    Ok(rows.first().is_some_and(|row| row.is_active))
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbEventRsvp {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    member_id: String,
    event_id: String,
    status: String,
    responded_at: SurrealDatetime,
}

fn parse_rsvp_status(s: &str) -> RsvpStatus {
    match s {
        "yes" => RsvpStatus::Yes,
        "maybe" => RsvpStatus::Maybe,
        "no" => RsvpStatus::No,
        _ => RsvpStatus::No,
    }
}

fn db_to_rsvp(db: DbEventRsvp) -> EventRsvp {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    EventRsvp {
        id,
        member_id: db.member_id,
        event_id: db.event_id,
        status: parse_rsvp_status(&db.status),
        responded_at: db.responded_at.into(),
    }
}

const RSVP_TXN_ATTEMPTS: u32 = 4;

enum RsvpAttempt {
    Missing,
    Conflict(crate::DbError),
    Other(crate::DbError),
}

fn classify_rsvp_error(err: crate::DbError) -> RsvpAttempt {
    if err.to_string().contains("Transaction conflict") {
        RsvpAttempt::Conflict(err)
    } else {
        RsvpAttempt::Other(err)
    }
}

impl Database {
    /// Upsert an RSVP only while the event is active.
    ///
    /// The active check and the write share one transaction. A deactivate that
    /// commits first conflicts with this read of the event row, the attempt is
    /// retried, and the retry returns [`DbError::NotFound`] without writing.
    /// Missing and inactive events use the message `Event not found`.
    pub async fn upsert_rsvp(
        &self,
        event_id: &str,
        member_id: &str,
        status: RsvpStatus,
    ) -> DbResult<EventRsvp> {
        with_timeout(async {
            let mut last_conflict = None;
            for _ in 0..RSVP_TXN_ATTEMPTS {
                match self.upsert_rsvp_once(event_id, member_id, status).await {
                    Ok(rsvp) => return Ok(rsvp),
                    Err(RsvpAttempt::Missing) => {
                        return Err(crate::DbError::NotFound("Event not found".into()));
                    }
                    Err(RsvpAttempt::Conflict(err)) => last_conflict = Some(err),
                    Err(RsvpAttempt::Other(err)) => return Err(err),
                }
            }
            Err(last_conflict.unwrap_or_else(|| {
                crate::DbError::Config("RSVP transaction conflict was not captured".into())
            }))
        })
        .await
    }

    async fn upsert_rsvp_once(
        &self,
        event_id: &str,
        member_id: &str,
        status: RsvpStatus,
    ) -> Result<EventRsvp, RsvpAttempt> {
        let tx = self
            .client
            .clone()
            .begin()
            .await
            .map_err(|err| classify_rsvp_error(err.into()))?;

        let active = match event_is_active(&tx, event_id).await {
            Ok(active) => active,
            Err(err) => {
                let _ = tx.cancel().await;
                return Err(classify_rsvp_error(err));
            }
        };
        if !active {
            if let Err(err) = tx.cancel().await {
                return Err(classify_rsvp_error(err.into()));
            }
            return Err(RsvpAttempt::Missing);
        }

        let now = SurrealDatetime::from(Utc::now());
        let written = match tx
            .query(
                r#"
                LET $existing = (SELECT * FROM event_rsvp WHERE event_id = $eid AND member_id = $mid LIMIT 1);
                IF array::len($existing) > 0 {
                    UPDATE $existing[0].id SET
                        status = $st,
                        responded_at = $rat
                    ;
                } ELSE {
                    CREATE event_rsvp SET
                        event_id = $eid,
                        member_id = $mid,
                        status = $st,
                        responded_at = $rat
                    ;
                };
                "#,
            )
            .bind(("eid", event_id.to_string()))
            .bind(("mid", member_id.to_string()))
            .bind(("st", status.to_string()))
            .bind(("rat", now))
            .await
        {
            Ok(mut result) => match result.take::<Vec<DbEventRsvp>>(1) {
                Ok(rsvps) => rsvps.into_iter().next().map(db_to_rsvp),
                Err(err) => {
                    let _ = tx.cancel().await;
                    return Err(classify_rsvp_error(err.into()));
                }
            },
            Err(err) => {
                let _ = tx.cancel().await;
                return Err(classify_rsvp_error(err.into()));
            }
        };
        let Some(rsvp) = written else {
            let _ = tx.cancel().await;
            return Err(RsvpAttempt::Other(crate::DbError::NotFound(
                "Failed to upsert RSVP".into(),
            )));
        };

        match tx.commit().await {
            Ok(_) => Ok(rsvp),
            Err(err) => Err(classify_rsvp_error(err.into())),
        }
    }

    /// Get all RSVPs for an event.
    pub async fn get_event_rsvps(&self, event_id: &str) -> DbResult<Vec<EventRsvp>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM event_rsvp WHERE event_id = $eid ORDER BY responded_at ASC")
                .bind(("eid", event_id.to_string()))
                .await?;
            let rsvps: Vec<DbEventRsvp> = result.take(0)?;
            Ok(rsvps.into_iter().map(db_to_rsvp).collect())
        })
        .await
    }

    /// Get RSVP summary counts for an event.
    pub async fn get_rsvp_summary(&self, event_id: &str) -> DbResult<RsvpSummary> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u32,
            }

            let mut yes_result = self
                .client
                .query("SELECT count() FROM event_rsvp WHERE event_id = $eid AND status = 'yes' GROUP ALL")
                .bind(("eid", event_id.to_string()))
                .await?;
            let yes: Vec<CountResult> = yes_result.take(0)?;

            let mut maybe_result = self
                .client
                .query("SELECT count() FROM event_rsvp WHERE event_id = $eid AND status = 'maybe' GROUP ALL")
                .bind(("eid", event_id.to_string()))
                .await?;
            let maybe: Vec<CountResult> = maybe_result.take(0)?;

            let mut no_result = self
                .client
                .query("SELECT count() FROM event_rsvp WHERE event_id = $eid AND status = 'no' GROUP ALL")
                .bind(("eid", event_id.to_string()))
                .await?;
            let no: Vec<CountResult> = no_result.take(0)?;

            Ok(RsvpSummary {
                event_id: event_id.to_string(),
                yes_count: yes.first().map(|c| c.count).unwrap_or(0),
                maybe_count: maybe.first().map(|c| c.count).unwrap_or(0),
                no_count: no.first().map(|c| c.count).unwrap_or(0),
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::migrations::run_migrations;
    use crate::DbError;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    async fn make_event(db: &Database, title: &str) -> String {
        db.create_event(title, 1, "19:00", "UTC", 120, true, None, "officer", false)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn rsvp_requires_an_active_event_in_the_write() {
        let db = test_db().await;
        let event_id = make_event(&db, "Scrim").await;

        let yes = db
            .upsert_rsvp(&event_id, "member-1", RsvpStatus::Yes)
            .await
            .unwrap();
        assert_eq!(yes.status, RsvpStatus::Yes);

        let no = db
            .upsert_rsvp(&event_id, "member-1", RsvpStatus::No)
            .await
            .unwrap();
        assert_eq!(no.status, RsvpStatus::No);

        db.deactivate_event(&event_id).await.unwrap();
        let denied = db
            .upsert_rsvp(&event_id, "member-1", RsvpStatus::Maybe)
            .await
            .unwrap_err();
        assert!(
            matches!(denied, DbError::NotFound(ref msg) if msg == "Event not found"),
            "{denied}"
        );
        let rows = db.get_event_rsvps(&event_id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, RsvpStatus::No);

        let missing = db
            .upsert_rsvp("missing-event", "member-1", RsvpStatus::Yes)
            .await
            .unwrap_err();
        assert!(matches!(missing, DbError::NotFound(msg) if msg == "Event not found"));
        assert!(db
            .get_event_rsvps("missing-event")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn rsvp_not_found_does_not_write_when_deactivate_races() {
        let db = Arc::new(test_db().await);
        for n in 0..24 {
            let event_id = make_event(&db, &format!("Race {n}")).await;
            let left = Arc::clone(&db);
            let right = Arc::clone(&db);
            let left_id = event_id.clone();
            let right_id = event_id.clone();
            let (rsvp_res, deact_res) = tokio::join!(
                async move {
                    left.upsert_rsvp(&left_id, "member-1", RsvpStatus::Yes)
                        .await
                },
                async move { right.deactivate_event(&right_id).await },
            );
            deact_res.unwrap();
            let rows = db.get_event_rsvps(&event_id).await.unwrap();
            match rsvp_res {
                Ok(_) => assert_eq!(rows.len(), 1, "committed RSVP must be stored"),
                Err(DbError::NotFound(msg)) => {
                    assert_eq!(msg, "Event not found");
                    assert!(
                        rows.is_empty(),
                        "a rejected RSVP must not be stored, got {rows:?}"
                    );
                }
                Err(err) => panic!("unexpected RSVP error: {err}"),
            }
        }
    }
}
