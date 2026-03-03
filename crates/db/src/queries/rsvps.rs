use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{EventRsvp, RsvpStatus, RsvpSummary};
use crate::{with_timeout, Database, DbResult};

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

impl Database {
    /// Upsert an RSVP (create or update by event_id + member_id).
    pub async fn upsert_rsvp(
        &self,
        event_id: &str,
        member_id: &str,
        status: RsvpStatus,
    ) -> DbResult<EventRsvp> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let mut result = self
                .client
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
                .await?;

            let rsvps: Vec<DbEventRsvp> = result.take(1)?;
            rsvps
                .into_iter()
                .next()
                .map(db_to_rsvp)
                .ok_or_else(|| crate::DbError::NotFound("Failed to upsert RSVP".into()))
        })
        .await
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
