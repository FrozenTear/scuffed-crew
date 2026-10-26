//! DB4: boot backfill that moves stored `personal_match` heroes onto the
//! canonical display names in [`scuffed_types::HERO_NAMES`].
//!
//! 0.5.0-alpha.1 and alpha.2 trackers uploaded the pack key (`ana`,
//! `wrecking-ball`) as the hero and guessed the role from it. Uploads have
//! been normalized since #222, but older rows still split `ana` from `Ana`
//! in hero stats and leaderboards. This pass applies the same rule to those
//! rows: a hero that [`scuffed_types::hero_key_to_name`] resolves is stored
//! under its display name with the role from the hero table.
//!
//! Rules:
//! - Rows already on a display name are never touched, role included.
//! - Unknown heroes and `Unknown` stay exactly as stored.
//! - Sombra rows are never re-stamped. Neither the hero nor the role of any
//!   row that resolves to Sombra is changed, so old Sombra games keep the
//!   role they were played under.
//! - Paged by record id and idempotent. A second pass writes nothing.
//! - A finished pass records `data_migration:db4_hero_keys`, so later boots
//!   read that one record and never scan `personal_match` again. Uploads
//!   have been normalized since #222, so no new pack-key rows can appear.

use surrealdb::engine::any::Any;
use surrealdb::Surreal;
use surrealdb_types::{RecordId, SurrealValue};

use scuffed_types::{
    hero_key_to_name, is_upload_hero_unknown, role_for_hero_name, HERO_NAMES, UPLOAD_HERO_UNKNOWN,
};

use super::{migration_batch_size, note_data_pass_select, record_key_of};
use crate::{DbError, DbResult};

/// Display name whose stored rows the backfill must never change.
const NEVER_RESTAMP_HERO: &str = "Sombra";

/// Record key in `data_migration` written once the pass has finished.
pub(super) const DB4_MARKER: &str = "db4_hero_keys";

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct HeroRow {
    id: RecordId,
    hero: String,
    #[serde(default)]
    #[surreal(default)]
    role: String,
}

/// New `(hero, role)` for one stored row, or `None` to leave it alone.
///
/// Mirrors the upload normalization in site-server `normalize_hero`, minus
/// the `Unknown` case fold, and with the Sombra guard on top.
pub(super) fn backfill_target(hero: &str, role: &str) -> Option<(&'static str, String)> {
    if is_upload_hero_unknown(hero) {
        return None;
    }
    let display = hero_key_to_name(hero)?;
    if display == NEVER_RESTAMP_HERO || display == hero {
        return None;
    }
    let role = role_for_hero_name(display).map_or_else(|| role.to_string(), |r| r.to_string());
    Some((display, role))
}

/// Every stored hero string that is already canonical. Rows on one of these
/// are skipped by the SELECT itself, so a clean table costs one empty page.
fn canonical_heroes() -> Vec<String> {
    HERO_NAMES
        .iter()
        .map(|h| h.display.to_string())
        .chain(std::iter::once(UPLOAD_HERO_UNKNOWN.to_string()))
        .collect()
}

/// Boot entry point. Skips when the DB4 marker exists, otherwise runs one
/// full pass and then writes the marker. Returns `(scanned, updated)`.
pub(crate) async fn backfill_personal_match_hero_keys(
    client: &Surreal<Any>,
) -> DbResult<(u64, u64)> {
    let mut res = client
        .query("SELECT VALUE id FROM type::record('data_migration', $key)")
        .bind(("key", DB4_MARKER))
        .await?
        .check()?;
    let done: Vec<RecordId> = res.take(0)?;
    if !done.is_empty() {
        tracing::debug!("personal_match hero key backfill already done; skipping");
        return Ok((0, 0));
    }
    let (scanned, updated) = hero_backfill_pass(client).await?;
    client
        .query(
            "UPSERT type::record('data_migration', $key) \
             SET completed_at = time::now(), scanned = $scanned, updated = $updated",
        )
        .bind(("key", DB4_MARKER))
        .bind(("scanned", scanned))
        .bind(("updated", updated))
        .await?
        .check()?;
    Ok((scanned, updated))
}

/// One paged pass over non-canonical heroes. Returns `(scanned, updated)`.
pub(super) async fn hero_backfill_pass(client: &Surreal<Any>) -> DbResult<(u64, u64)> {
    let canonical = canonical_heroes();
    let mut after: Option<RecordId> = None;
    let mut scanned = 0u64;
    let mut updated = 0u64;
    loop {
        note_data_pass_select();
        let page_query = if after.is_some() {
            "SELECT id, hero, role FROM personal_match \
             WHERE hero NOT IN $canonical AND id > $after \
             ORDER BY id LIMIT $lim"
        } else {
            "SELECT id, hero, role FROM personal_match \
             WHERE hero NOT IN $canonical \
             ORDER BY id LIMIT $lim"
        };
        let mut query = client
            .query(page_query)
            .bind(("canonical", canonical.clone()))
            .bind(("lim", migration_batch_size()));
        if let Some(last) = after.clone() {
            query = query.bind(("after", last));
        }
        let rows: Vec<HeroRow> = query.await?.check()?.take(0)?;
        let Some(last) = rows.last() else {
            break;
        };
        let last_id = last.id.clone();
        if let Some(prev) = &after {
            if record_key_of(prev) == record_key_of(&last_id) {
                return Err(DbError::Conflict(
                    "personal_match hero backfill made no progress".into(),
                ));
            }
        }
        for row in rows {
            scanned += 1;
            let Some((hero, role)) = backfill_target(&row.hero, &row.role) else {
                continue;
            };
            client
                .query("UPDATE $rid SET hero = $hero, role = $role")
                .bind(("rid", row.id))
                .bind(("hero", hero.to_string()))
                .bind(("role", role))
                .await?
                .check()?;
            updated += 1;
        }
        after = Some(last_id);
    }
    tracing::info!(
        scanned,
        updated,
        "personal_match hero key backfill complete"
    );
    Ok((scanned, updated))
}

#[cfg(test)]
mod tests {
    use super::{backfill_personal_match_hero_keys, backfill_target, DB4_MARKER};
    use crate::migrations::run_migrations;
    use crate::queries::personal_stats::{
        data_pass_selects, reset_data_pass_selects, set_migration_batch_size,
    };
    use crate::Database;
    use chrono::{TimeZone, Utc};

    #[test]
    fn target_rewrites_pack_keys_case_and_whitespace() {
        assert_eq!(
            backfill_target("ana", "Damage"),
            Some(("Ana", "Support".to_string()))
        );
        assert_eq!(
            backfill_target("wrecking-ball", "Damage"),
            Some(("Wrecking Ball", "Tank".to_string()))
        );
        assert_eq!(
            backfill_target("wrecking_ball", "Damage"),
            Some(("Wrecking Ball", "Tank".to_string()))
        );
        assert_eq!(
            backfill_target("  ANA ", "Support"),
            Some(("Ana", "Support".to_string()))
        );
        assert_eq!(
            backfill_target("dva", "Damage"),
            Some(("D.Va", "Tank".to_string()))
        );
        assert_eq!(
            backfill_target("lucio", "Support"),
            Some(("Lúcio", "Support".to_string()))
        );
        assert_eq!(
            backfill_target("soldier-76", "Damage"),
            Some(("Soldier: 76", "Damage".to_string()))
        );
    }

    #[test]
    fn target_leaves_canonical_unknown_and_placeholder_rows() {
        // Already canonical: hero and role both untouched, even a stale role.
        assert_eq!(backfill_target("Ana", "Damage"), None);
        assert_eq!(backfill_target("Wrecking Ball", "Damage"), None);
        assert_eq!(backfill_target("Lúcio", "Support"), None);
        // Unknown heroes stay as stored.
        assert_eq!(backfill_target("Unknown", "Damage"), None);
        assert_eq!(backfill_target("unknown", ""), None);
        assert_eq!(backfill_target("Not A Hero", "Tank"), None);
        assert_eq!(backfill_target("", "Tank"), None);
        assert_eq!(backfill_target("placeholder", "Tank"), None);
    }

    #[test]
    fn target_never_restamps_sombra() {
        for hero in ["Sombra", "sombra", " SOMBRA ", "sombra.png"] {
            for role in ["Damage", "Support", "", "Tank"] {
                assert_eq!(backfill_target(hero, role), None, "{hero:?} {role:?}");
            }
        }
    }

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    /// A database from before DB4 has no marker. Fresh test databases get
    /// one on their first boot, so drop it to replay the upgrade.
    async fn forget_marker(db: &Database) {
        db.client
            .query("DELETE type::record('data_migration', $key)")
            .bind(("key", DB4_MARKER))
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    async fn marker_count(db: &Database) -> usize {
        let mut res = db
            .client
            .query("SELECT VALUE id FROM type::record('data_migration', $key)")
            .bind(("key", DB4_MARKER))
            .await
            .unwrap()
            .check()
            .unwrap();
        let ids: Vec<surrealdb_types::RecordId> = res.take(0).unwrap();
        ids.len()
    }

    fn row(session_id: &str, hero: &str, role: &str) -> crate::types::PersonalMatch {
        crate::types::PersonalMatch {
            id: String::new(),
            member_id: "m1".into(),
            session_id: session_id.into(),
            hero: hero.into(),
            map_name: "Oasis".into(),
            game_mode: "control".into(),
            role: role.into(),
            outcome: "victory".into(),
            elims: 10,
            deaths: 2,
            assists: 8,
            damage: 3000,
            healing: 9000,
            mitigation: 0,
            played_at: Utc.with_ymd_and_hms(2026, 7, 1, 20, 0, 0).unwrap(),
            uploaded_at: Utc::now(),
            edited: false,
            recognizer: scuffed_types::RECOGNIZER_OCR_V1.into(),
            name_recognizer: None,
            suspect_fields: Vec::new(),
        }
    }

    fn by_session(rows: &[crate::types::PersonalMatch], sid: &str) -> (String, String) {
        let r = rows
            .iter()
            .find(|r| r.session_id == sid)
            .unwrap_or_else(|| panic!("{sid} missing"));
        (r.hero.clone(), r.role.clone())
    }

    fn pair(hero: &str, role: &str) -> (String, String) {
        (hero.to_string(), role.to_string())
    }

    #[tokio::test]
    async fn boot_backfill_rewrites_pack_keys_and_keeps_the_rest() {
        let db = test_db().await;
        db.upsert_personal_matches(
            "m1",
            &[
                row("s-ana", "ana", "Damage"),
                row("s-ball", "wrecking-ball", "Damage"),
                row("s-dva", "D.VA", "Damage"),
                row("s-canon", "Ana", "Damage"),
                row("s-unknown", "Unknown", "Damage"),
                row("s-mystery", "Mystery Hero", "Tank"),
                row("s-sombra-key", "sombra", "Damage"),
                row("s-sombra-dmg", "Sombra", "Damage"),
                row("s-sombra-sup", "Sombra", "Support"),
                row("s-doctrine", "Doctrine", "Support"),
            ],
        )
        .await
        .unwrap();

        forget_marker(&db).await;
        // Small pages so paging is exercised.
        let _batch = set_migration_batch_size(2);
        let (scanned, updated) = backfill_personal_match_hero_keys(&db.client).await.unwrap();
        assert_eq!(updated, 3, "ana, wrecking-ball and D.VA only");
        // Non-canonical candidates: ana, wrecking-ball, D.VA, Mystery Hero, sombra.
        assert_eq!(scanned, 5);

        let rows = db.list_personal_matches("m1", 50, 0).await.unwrap();
        assert_eq!(rows.len(), 10);
        assert_eq!(by_session(&rows, "s-ana"), pair("Ana", "Support"));
        assert_eq!(by_session(&rows, "s-ball"), pair("Wrecking Ball", "Tank"));
        assert_eq!(by_session(&rows, "s-dva"), pair("D.Va", "Tank"));
        assert_eq!(by_session(&rows, "s-canon"), pair("Ana", "Damage"));
        assert_eq!(by_session(&rows, "s-unknown"), pair("Unknown", "Damage"));
        assert_eq!(by_session(&rows, "s-mystery"), pair("Mystery Hero", "Tank"));
        assert_eq!(by_session(&rows, "s-sombra-key"), pair("sombra", "Damage"));
        assert_eq!(by_session(&rows, "s-sombra-dmg"), pair("Sombra", "Damage"));
        assert_eq!(by_session(&rows, "s-sombra-sup"), pair("Sombra", "Support"));
        assert_eq!(by_session(&rows, "s-doctrine"), pair("Doctrine", "Support"));
        assert_eq!(marker_count(&db).await, 1, "a finished pass is recorded");
    }

    #[tokio::test]
    async fn boot_backfill_is_idempotent_through_run_migrations() {
        let db = test_db().await;
        db.upsert_personal_matches(
            "m1",
            &[
                row("s1", "ana", "Damage"),
                row("s2", "Sombra", "Damage"),
                row("s3", "Ana", "Support"),
            ],
        )
        .await
        .unwrap();

        forget_marker(&db).await;
        run_migrations(&db.client).await.unwrap();
        let first = db.list_personal_matches("m1", 50, 0).await.unwrap();
        assert_eq!(by_session(&first, "s1"), pair("Ana", "Support"));
        assert_eq!(by_session(&first, "s2"), pair("Sombra", "Damage"));

        // A second pass finds nothing to write.
        let (scanned, updated) = backfill_personal_match_hero_keys(&db.client).await.unwrap();
        assert_eq!((scanned, updated), (0, 0));
        run_migrations(&db.client).await.unwrap();
        let second = db.list_personal_matches("m1", 50, 0).await.unwrap();
        assert_eq!(first.len(), second.len());
        for sid in ["s1", "s2", "s3"] {
            assert_eq!(by_session(&first, sid), by_session(&second, sid), "{sid}");
        }
        let stats = db.get_personal_stats("m1").await.unwrap();
        assert_eq!(stats.total_matches, 3);
    }

    #[tokio::test]
    async fn clean_table_costs_one_empty_page() {
        let db = test_db().await;
        db.upsert_personal_matches("m1", &[row("s1", "Ana", "Support")])
            .await
            .unwrap();
        forget_marker(&db).await;
        reset_data_pass_selects();
        let (scanned, updated) = backfill_personal_match_hero_keys(&db.client).await.unwrap();
        assert_eq!((scanned, updated), (0, 0));
        assert_eq!(data_pass_selects(), 1);
    }

    #[tokio::test]
    async fn marker_skips_the_table_on_later_boots() {
        let db = test_db().await;
        assert_eq!(marker_count(&db).await, 1, "first boot records the pass");
        // A pack-key row written behind the server's back after the marker
        // is not picked up: later boots never scan personal_match.
        db.upsert_personal_matches("m1", &[row("s1", "ana", "Damage")])
            .await
            .unwrap();
        reset_data_pass_selects();
        run_migrations(&db.client).await.unwrap();
        assert_eq!(data_pass_selects(), 0);
        let rows = db.list_personal_matches("m1", 10, 0).await.unwrap();
        assert_eq!(by_session(&rows, "s1"), pair("ana", "Damage"));
    }
}
