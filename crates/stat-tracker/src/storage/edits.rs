//! The back-fill and manual-edit writers: `set_session_*`, the hero
//! timeline, segment resolutions and `edit_match`.

use surrealdb_types::Datetime as SurrealDatetime;

use super::{
    LocalStore, MatchEdit, SEGMENT_CONFIRM, SEGMENT_DISMISS, SegmentResolution, apply_match_edit,
    collect_segment_resolutions, derive_hero_segments, rewrite_match_log_session,
};

impl LocalStore {
    /// MANUAL repair helper: stamp a hero/role onto a session and ALL its
    /// snapshots. Destroys the per-snapshot raw reads by design — explicit
    /// human intent overrides the captured values, exactly like
    /// `set_session_map`. The automatic detection path must NOT use this (it
    /// used to, which recorded a 97%-Ana game as Rein when a late swap won
    /// last-write; it now derives a hero timeline instead — see
    /// [`Self::refresh_session_hero_timeline`]).
    pub async fn set_session_hero(
        &self,
        session_id: &str,
        hero: &str,
        role: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Write through to the capture rows like set_session_map/outcome do:
        // the GUI and the sync path read personal_match, not match_session, so
        // a session-only update is invisible to both (the 07-18 "Tracer" game).
        // The `hero != $hero` guard keeps re-sync churn out of the every-capture
        // refresh path.
        self.db
            .query(
                "UPDATE match_session SET hero = $hero, role = $role WHERE session_id = $sid; \
                 UPDATE personal_match SET hero = $hero, role = $role, synced = false, \
                 upload_reject = '', sync_rev = (sync_rev ?? 0) + 1 \
                 WHERE session_id = $sid AND (hero != $hero OR role != $role)",
            )
            .bind(("hero", hero.to_string()))
            .bind(("role", role.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        // Keep the append-only log in step too (HW-1a jsonl parity). Nothing
        // in the app reads matches.jsonl any more; a manual repair is rare,
        // so the rewrite stays until the log itself is retired.
        let (hero, role) = (hero.to_string(), role.to_string());
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.hero = hero.clone();
                m.role = role.clone();
                m.upload_reject = None;
            }),
        );
        Ok(())
    }

    /// AUTOMATIC hero path (HS-1). Re-derive the session's hero timeline from
    /// its RAW snapshots and stamp the result (`heroes_played`) onto every
    /// snapshot row — WITHOUT touching the per-snapshot `hero`/`role` reads,
    /// unlike [`Self::set_session_hero`]. The derived primary (majority over
    /// the segments) then drives `display_hero`/`display_role`, so a late hero
    /// swap is recorded as its own segment instead of a last-write that
    /// mislabels the whole game (the 97%-Ana-recorded-as-Rein bug).
    ///
    /// Runs after every capture, so it mirrors the churn-avoidance of the
    /// other back-fills: it writes a row only when its timeline actually
    /// changed, and re-queues it for sync (`synced = false`) only when the
    /// row's EFFECTIVE (uploaded) hero/role would change — a stable-hero game
    /// is a no-op for sync after its first snapshot. The rows themselves are
    /// rewritten on every capture: a segment's `snapshots` and `last_seen`
    /// move each time. Only the SurrealKV `personal_match` rows carry the
    /// timeline; `matches.jsonl` (no reader) and `match_session` do not, and
    /// the daemon's `live_snapshot.json` picks it up on its next write.
    pub async fn refresh_session_hero_timeline(
        &self,
        session_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        if snaps.is_empty() {
            return Ok(());
        }
        // Resolutions are stamped uniformly across a session's rows; gather the
        // union so a partially-written row can't drop a decision.
        let resolutions = collect_segment_resolutions(&snaps);
        let segments = derive_hero_segments(&snaps, &resolutions);

        if self.timeline_migrated() {
            return self
                .store_session_timeline(session_id, &snaps, &segments, &resolutions)
                .await;
        }
        for m in &snaps {
            let Some(id) = m.id.clone() else { continue };
            if m.heroes_played == segments {
                continue; // already current — no write, no sync churn
            }
            // Re-queue for sync only when the value the upload sends
            // (`display_hero`/`display_role`) actually changes. Compare via the
            // real display methods so this stays lock-step with the sync path
            // (a `corrected_hero` pins the effective value regardless of the
            // timeline, so such rows never re-queue).
            let old_hero = m.display_hero().to_string();
            let old_role = m.display_role().to_string();
            let mut probe = m.clone();
            probe.heroes_played = segments.clone();
            let requeue = old_hero.as_str() != probe.display_hero()
                || old_role.as_str() != probe.display_role();

            if requeue {
                self.db
                    .query(
                        "UPDATE $id SET heroes_played = $hp, synced = false, \
                         upload_reject = '', sync_rev = (sync_rev ?? 0) + 1",
                    )
                    .bind(("id", id))
                    .bind(("hp", segments.clone()))
                    .await?;
            } else {
                self.db
                    .query("UPDATE $id SET heroes_played = $hp")
                    .bind(("id", id))
                    .bind(("hp", segments.clone()))
                    .await?;
            }
        }
        // matches.jsonl is not stamped: nothing reads it, and this runs after
        // every capture, so a whole-file rewrite here cost a parse of every
        // logged row per capture.
        Ok(())
    }

    /// [`Self::refresh_session_hero_timeline`] on a migrated store: the
    /// timeline is one `hero_timeline` record, written once when it changed.
    /// `snaps` are hydrated, so each row's display is the stored timeline's.
    /// Only rows whose uploaded hero or role changes are written (re-queued
    /// for sync); no row gets a timeline copy.
    async fn store_session_timeline(
        &self,
        session_id: &str,
        snaps: &[super::PersonalMatch],
        segments: &[super::HeroSegment],
        resolutions: &[SegmentResolution],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stored = self.get_timeline(session_id).await?;
        let current = stored
            .as_ref()
            .is_some_and(|t| t.heroes_played == segments && t.segment_resolutions == resolutions);
        // No list stored and nothing to store: leave the rows' own copies
        // (an older game keeps showing what it shows today).
        if current || (stored.is_none() && segments.is_empty() && resolutions.is_empty()) {
            return Ok(());
        }
        self.upsert_timeline(session_id, segments, resolutions)
            .await?;
        for m in snaps {
            let Some(id) = m.id.clone() else { continue };
            let mut probe = m.clone();
            probe.heroes_played = segments.to_vec();
            if m.display_hero() == probe.display_hero() && m.display_role() == probe.display_role()
            {
                continue;
            }
            self.db
                .query(
                    "UPDATE $id SET synced = false, upload_reject = '', \
                     sync_rev = (sync_rev ?? 0) + 1 RETURN NONE",
                )
                .bind(("id", id))
                .await?;
        }
        Ok(())
    }

    /// MANUAL confirm/dismiss of one derived hero segment (HS-1). Records the
    /// decision in `segment_resolutions` (keyed by segment index; a repeat on
    /// the same index replaces it, and an unrecognized action clears it) on
    /// EVERY snapshot row — uniform, like `heroes_played` — then re-derives the
    /// timeline via [`Self::refresh_session_hero_timeline`] so the flags and
    /// the effective primary update (and any resulting hero change re-queues
    /// for sync). Handles the 1-snapshot flap (e.g. Lijiang "Wuyang"/"Mizuki"):
    /// confirm keeps it as a real swap, dismiss drops it from display+majority.
    pub async fn resolve_hero_segment(
        &self,
        session_id: &str,
        segment: u32,
        action: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        if snaps.is_empty() {
            return Ok(());
        }
        let mut resolutions = collect_segment_resolutions(&snaps);
        resolutions.retain(|r| r.segment != segment);
        if action == SEGMENT_CONFIRM || action == SEGMENT_DISMISS {
            resolutions.push(SegmentResolution {
                segment,
                action: action.to_string(),
            });
        }
        resolutions.sort_by_key(|r| r.segment);

        if self.timeline_migrated() {
            // One record holds the decisions; the re-derive below writes the
            // flags and re-queues the rows whose effective hero changed.
            let segments = match self.get_timeline(session_id).await? {
                Some(t) => t.heroes_played,
                None => derive_hero_segments(&snaps, &resolutions),
            };
            self.upsert_timeline(session_id, &segments, &resolutions)
                .await?;
            return self.refresh_session_hero_timeline(session_id).await;
        }
        // Persist the resolution set on all rows. This field is never uploaded
        // (only its effect via the derived primary is), so it does NOT flip
        // `synced` here — the re-derive below re-queues exactly the rows whose
        // effective hero changed.
        for m in &snaps {
            let Some(id) = m.id.clone() else { continue };
            if m.segment_resolutions == resolutions {
                continue;
            }
            self.db
                .query("UPDATE $id SET segment_resolutions = $sr")
                .bind(("id", id))
                .bind(("sr", resolutions.clone()))
                .await?;
        }

        // Re-derive with the updated resolutions and re-stamp heroes_played.
        self.refresh_session_hero_timeline(session_id).await
    }

    /// Set a session's map and stamp it onto ALL its snapshots, re-queuing
    /// them for sync. A game has exactly one map, so snapshots that disagree
    /// (missed or misread the label) are corrected, not preserved.
    pub async fn set_session_map(
        &self,
        session_id: &str,
        map: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mode = crate::parse::stored_game_mode(map);
        self.db
            .query(
                "UPDATE match_session SET map_name = $map WHERE session_id = $sid; \
                 UPDATE personal_match SET map_name = $map, game_mode = $mode, synced = false, \
                 upload_reject = '', sync_rev = (sync_rev ?? 0) + 1 \
                 WHERE session_id = $sid AND (map_name != $map OR game_mode != $mode)",
            )
            .bind(("map", map.to_string()))
            .bind(("mode", mode.clone()))
            .bind(("sid", session_id.to_string()))
            .await?;
        self.clear_session_suspects(session_id, "map_name").await?;
        let map = map.to_string();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.map_name = map.clone();
                m.game_mode = mode.clone();
                m.upload_reject = None;
                crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "map_name");
            }),
        );
        Ok(())
    }

    /// [`Self::set_session_map`] that leaves a member's own map choice alone.
    ///
    /// A row that carries a member's map choice
    /// ([`PersonalMatch::has_member_map_edit`]) keeps the `map_name` read
    /// under the choice and the `game_mode` stored with it. The session and
    /// every other row take `map`, as in [`Self::set_session_map`]: a map
    /// choice is stamped on the rows that exist when it is made, so the rows
    /// captured after it (the newest is the game's row) have none and must
    /// not keep an untrusted or empty map. Returns how many rows were left
    /// alone. With none, this is [`Self::set_session_map`].
    ///
    /// [`PersonalMatch::has_member_map_edit`]: super::PersonalMatch::has_member_map_edit
    pub async fn set_session_map_keeping_member_choices(
        &self,
        session_id: &str,
        map: &str,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        let kept = snaps.iter().filter(|m| m.has_member_map_edit()).count();
        if kept == 0 {
            self.set_session_map(session_id, map).await?;
            return Ok(0);
        }
        let mode = crate::parse::stored_game_mode(map);
        self.db
            .query("UPDATE match_session SET map_name = $map WHERE session_id = $sid")
            .bind(("map", map.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        for m in &snaps {
            let Some(id) = m.id.clone() else { continue };
            if m.has_member_map_edit() {
                continue;
            }
            // the two writes of `set_session_map`, for this row only
            if m.map_name != map || m.game_mode != mode {
                self.db
                    .query(
                        "UPDATE $id SET map_name = $map, game_mode = $mode, synced = false, \
                         upload_reject = '', sync_rev = (sync_rev ?? 0) + 1",
                    )
                    .bind(("id", id.clone()))
                    .bind(("map", map.to_string()))
                    .bind(("mode", mode.clone()))
                    .await?;
            }
            let mut suspects = m.suspect_fields.clone();
            crate::suspect::clear_edited_suspects(&mut suspects, "map_name");
            if suspects != m.suspect_fields {
                self.db
                    .query("UPDATE $id SET suspect_fields = $sf")
                    .bind(("id", id))
                    .bind(("sf", suspects))
                    .await?;
            }
        }
        let map = map.to_string();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                if m.has_member_map_edit() {
                    return;
                }
                m.map_name = map.clone();
                m.game_mode = mode.clone();
                m.upload_reject = None;
                crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "map_name");
            }),
        );
        Ok(kept)
    }

    /// Back-fill a detected outcome onto a session and every capture snapshot it
    /// contains, re-queuing those snapshots for sync so the corrected result
    /// (e.g. a VICTORY read off the accolade screen after the stats were already
    /// captured with `unknown`) propagates to the server.
    pub async fn set_session_outcome(
        &self,
        session_id: &str,
        outcome: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query(
                "UPDATE match_session SET final_outcome = $outcome WHERE session_id = $sid; \
                 UPDATE personal_match SET outcome = $outcome, synced = false, \
                 upload_reject = '', sync_rev = (sync_rev ?? 0) + 1 WHERE session_id = $sid",
            )
            .bind(("outcome", outcome.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        self.clear_session_suspects(session_id, "outcome").await?;
        let outcome = outcome.to_string();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.outcome = outcome.clone();
                m.upload_reject = None;
                crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "outcome");
            }),
        );
        Ok(())
    }

    /// Drop one edited field's suspect names on every snapshot of a session.
    async fn clear_session_suspects(
        &self,
        session_id: &str,
        storage_field: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        for m in snaps {
            let Some(id) = m.id.clone() else { continue };
            let mut fields = m.suspect_fields.clone();
            crate::suspect::clear_edited_suspects(&mut fields, storage_field);
            if fields == m.suspect_fields {
                continue;
            }
            self.db
                .query("UPDATE $id SET suspect_fields = $sf")
                .bind(("id", id))
                .bind(("sf", fields))
                .await?;
        }
        Ok(())
    }

    /// Apply a manual stat correction to a game. Writes the fixed values into
    /// the `corrected_*` overlay on EVERY snapshot of the session (leaving the
    /// immutable OCR fields untouched), records which fields were edited, and
    /// re-queues the rows for sync. Editing all snapshots — not just the final
    /// one — means whichever snapshot the server-side per-session upsert lands
    /// last still carries the correction, and mirrors the write-through the
    /// outcome/map/hero back-fills already use. The `matches.jsonl` record is
    /// rewritten with the same overlay so it stays in step.
    pub async fn edit_match(
        &self,
        session_id: &str,
        edit: &MatchEdit,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = SurrealDatetime::from(chrono::Utc::now());
        let snaps = self.get_session_snapshots(session_id).await?;
        for mut m in snaps {
            let Some(id) = m.id.clone() else { continue };
            if !apply_match_edit(&mut m, edit, now) {
                continue; // empty edit — nothing to persist
            }
            self.db
                .query(
                    "UPDATE $id SET corrected_hero = $ch, corrected_role = $cr, \
                     corrected_map_name = $cmap, corrected_outcome = $co, \
                     corrected_elims = $ce, corrected_deaths = $cd, corrected_assists = $ca, \
                     corrected_damage = $cdmg, corrected_healing = $chl, \
                     corrected_mitigation = $cmit, game_mode = $mode, edited_fields = $ef, \
                     edited_at = $ea, suspect_fields = $sf, \
                     synced = false, upload_reject = '', sync_rev = (sync_rev ?? 0) + 1",
                )
                .bind(("id", id))
                .bind(("ch", m.corrected_hero.clone()))
                .bind(("cr", m.corrected_role.clone()))
                .bind(("cmap", m.corrected_map_name.clone()))
                .bind(("mode", m.game_mode.clone()))
                .bind(("co", m.corrected_outcome.clone()))
                .bind(("ce", m.corrected_elims))
                .bind(("cd", m.corrected_deaths))
                .bind(("ca", m.corrected_assists))
                .bind(("cdmg", m.corrected_damage))
                .bind(("chl", m.corrected_healing))
                .bind(("cmit", m.corrected_mitigation))
                .bind(("ef", m.edited_fields.clone()))
                .bind(("ea", m.edited_at))
                .bind(("sf", m.suspect_fields.clone()))
                .await?;
        }
        let edit = edit.clone();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&move |m| {
                if apply_match_edit(m, &edit, now) {
                    m.upload_reject = None;
                }
            }),
        );
        Ok(())
    }
}
