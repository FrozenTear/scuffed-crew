//! One hero timeline per game (`hero_timeline`), read back into rows.
//!
//! Until 0.6.0-alpha.2 every capture row of a game carried the same
//! `heroes_played` and `segment_resolutions`, and every capture rewrote all
//! of the game's rows (the segments' `snapshots` and `last_seen` move each
//! time). Once the store is migrated (`store_meta:schema`, see
//! `migrate_timeline.rs`), the timeline lives in one record per game,
//! `hero_timeline:<session_id>`, and new rows are stored without it. Reads
//! put it back ([`hydrate`]), so every caller still sees
//! `PersonalMatch::heroes_played` as before.
//!
//! Rows written before the migration keep their own copies: an older
//! tracker opening the store shows those games as it always did. A record,
//! when there is one, wins over a row's copy.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use surrealdb_types::{Datetime as SurrealDatetime, RecordId, SurrealValue};

use super::{HeroSegment, LocalStore, PersonalMatch, SegmentResolution};

pub(super) const TIMELINE_TABLE: &str = "hero_timeline";

/// Record ids looked up per query.
const LOOKUP_BATCH: usize = 200;

/// A `hero_timeline` record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "::surrealdb_types")]
pub(super) struct TimelineRecord {
    pub(super) session_id: String,
    #[surreal(default)]
    pub(super) heroes_played: Vec<HeroSegment>,
    #[surreal(default)]
    pub(super) segment_resolutions: Vec<SegmentResolution>,
    #[surreal(default)]
    pub(super) updated_at: Option<SurrealDatetime>,
}

pub(super) fn timeline_id(session_id: &str) -> RecordId {
    RecordId::new(TIMELINE_TABLE, session_id)
}

/// Put each game's stored timeline on its rows. Rows of games without a
/// record (older games, blank session ids, a store not migrated) keep their
/// own copy.
pub(super) fn hydrate(rows: &mut [PersonalMatch], timelines: &HashMap<String, TimelineRecord>) {
    for row in rows {
        if let Some(t) = timelines.get(&row.session_id) {
            row.heroes_played = t.heroes_played.clone();
            row.segment_resolutions = t.segment_resolutions.clone();
        }
    }
}

impl LocalStore {
    /// The stored timelines of these games, by session id.
    pub(super) async fn load_timelines<'a>(
        &self,
        session_ids: impl IntoIterator<Item = &'a str>,
    ) -> Result<HashMap<String, TimelineRecord>, Box<dyn std::error::Error + Send + Sync>> {
        let mut ids: Vec<&str> = session_ids.into_iter().filter(|s| !s.is_empty()).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut out = HashMap::new();
        for chunk in ids.chunks(LOOKUP_BATCH) {
            let rids: Vec<RecordId> = chunk.iter().map(|sid| timeline_id(sid)).collect();
            // Direct record-id lookups: no index, no table scan.
            let records: Vec<TimelineRecord> = self
                .db
                .query("SELECT * FROM $rids")
                .bind(("rids", rids))
                .await?
                .take(0)?;
            for r in records {
                out.insert(r.session_id.clone(), r);
            }
        }
        Ok(out)
    }

    /// [`hydrate`] these rows from the store (migrated stores only).
    pub(super) async fn hydrate_rows(
        &self,
        rows: &mut [PersonalMatch],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Before the marker the rows are the truth: a record left by an
        // interrupted move must not hide a row edit made meanwhile.
        if !self.timeline_migrated() {
            return Ok(());
        }
        let timelines = self
            .load_timelines(rows.iter().map(|r| r.session_id.as_str()))
            .await?;
        hydrate(rows, &timelines);
        Ok(())
    }

    pub(super) async fn get_timeline(
        &self,
        session_id: &str,
    ) -> Result<Option<TimelineRecord>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.load_timelines([session_id]).await?.remove(session_id))
    }

    /// Store one game's timeline. Idempotent: the record id is the session id.
    pub(super) async fn upsert_timeline(
        &self,
        session_id: &str,
        heroes_played: &[HeroSegment],
        segment_resolutions: &[SegmentResolution],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if session_id.is_empty() {
            return Ok(());
        }
        self.db
            .query(
                "UPSERT $rid SET session_id = $sid, heroes_played = $hp, \
                 segment_resolutions = $sr, updated_at = time::now() RETURN NONE",
            )
            .bind(("rid", timeline_id(session_id)))
            .bind(("sid", session_id.to_string()))
            .bind(("hp", heroes_played.to_vec()))
            .bind(("sr", segment_resolutions.to_vec()))
            .await?
            .check()?;
        Ok(())
    }

    /// Every timeline record (vacuum copies them into the fresh store).
    pub(super) async fn all_timelines(
        &self,
    ) -> Result<Vec<TimelineRecord>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .db
            .query("SELECT * FROM hero_timeline")
            .await?
            .take(0)?)
    }

    /// Write a copied record as is (vacuum).
    pub(super) async fn put_timeline(
        &self,
        t: &TimelineRecord,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query(
                "UPSERT $rid SET session_id = $sid, heroes_played = $hp, \
                 segment_resolutions = $sr, updated_at = $at RETURN NONE",
            )
            .bind(("rid", timeline_id(&t.session_id)))
            .bind(("sid", t.session_id.clone()))
            .bind(("hp", t.heroes_played.clone()))
            .bind(("sr", t.segment_resolutions.clone()))
            .bind(("at", t.updated_at))
            .await?
            .check()?;
        Ok(())
    }
}
