use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::Any;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb::Surreal;
use surrealdb_types::{RecordId, SurrealValue};

use scuffed_types::patch_notes::{
    PatchChange, PatchHeroUpdate, PatchNote, PatchSection, UpdatePatchNoteRequest,
};

use crate::{with_timeout, Database, DbError, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbPatchNote {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    version: String,
    date: String,
    title: Option<String>,
    url: String,
    hero_updates: serde_json::Value,
    sections: serde_json::Value,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

fn map_version_unique_violation(e: surrealdb::Error) -> DbError {
    // SurrealDB: "Database index `patch_note_version_idx` already contains [...]"
    if e.to_string().contains("already contains") {
        DbError::Conflict("Patch note version already exists".into())
    } else {
        DbError::Surreal(e)
    }
}

fn json_lists(note: &PatchNote) -> DbResult<(serde_json::Value, serde_json::Value)> {
    let hero_updates = serde_json::to_value(&note.hero_updates)
        .map_err(|e| DbError::Config(format!("Failed to serialize hero_updates: {e}")))?;
    let sections = serde_json::to_value(&note.sections)
        .map_err(|e| DbError::Config(format!("Failed to serialize sections: {e}")))?;
    Ok((hero_updates, sections))
}

fn db_to_patch_note(db: DbPatchNote) -> PatchNote {
    let hero_updates = serde_json::from_value(db.hero_updates).unwrap_or_default();
    let sections = serde_json::from_value(db.sections).unwrap_or_default();
    PatchNote {
        version: db.version,
        date: db.date,
        title: db.title,
        url: db.url,
        hero_updates,
        sections,
    }
}

/// Static catalog shipped in #89. Seeded once when `patch_note` is empty so
/// Contabo is not blank after the in-binary catalog is dropped.
pub fn initial_patch_note_catalog() -> Vec<PatchNote> {
    vec![
        PatchNote {
            version: "2.18.1".into(),
            date: "2026-08-20".into(),
            title: Some("Mid-season balance".into()),
            url: "https://overwatch.blizzard.com/en-us/news/patch-notes/".into(),
            hero_updates: vec![
                PatchHeroUpdate {
                    hero_id: "ana".into(),
                    hero_name: "Ana".into(),
                    change_type: "adjustment".into(),
                    changes: vec![PatchChange {
                        ability: Some("Biotic Grenade".into()),
                        description: "Healing-boost window is a bit shorter.".into(),
                        change_type: "nerf".into(),
                    }],
                    dev_comment: Some(
                        "Keeping her burst sustain in line with other supports.".into(),
                    ),
                },
                PatchHeroUpdate {
                    hero_id: "venture".into(),
                    hero_name: "Venture".into(),
                    change_type: "buff".into(),
                    changes: vec![PatchChange {
                        ability: Some("Drill Dash".into()),
                        description: "Dash distance increased slightly.".into(),
                        change_type: "buff".into(),
                    }],
                    dev_comment: None,
                },
            ],
            sections: vec![
                PatchSection {
                    category: "Competitive".into(),
                    items: vec!["Placement games now show an expected rank band.".into()],
                },
                PatchSection {
                    category: "Bug Fixes".into(),
                    items: vec!["Fixed a rare scoreboard freeze after overtime.".into()],
                },
            ],
        },
        PatchNote {
            version: "2.18.0".into(),
            date: "2026-08-11".into(),
            title: Some("Season 4 launch".into()),
            url: "https://overwatch.blizzard.com/en-us/news/patch-notes/".into(),
            hero_updates: vec![PatchHeroUpdate {
                hero_id: "freja".into(),
                hero_name: "Freja".into(),
                change_type: "bugfix".into(),
                changes: vec![PatchChange {
                    ability: None,
                    description: "Fixed an animation hitch when swapping weapons mid-air.".into(),
                    change_type: "bugfix".into(),
                }],
                dev_comment: None,
            }],
            sections: vec![
                PatchSection {
                    category: "Maps".into(),
                    items: vec![
                        "New Clash rotation includes Aatlis.".into(),
                        "Health pack on New Queen Street mid is easier to contest.".into(),
                    ],
                },
                PatchSection {
                    category: "General".into(),
                    items: vec!["Career profile now lists last season's peak.".into()],
                },
            ],
        },
        PatchNote {
            version: "2.17.3".into(),
            date: "2026-07-22".into(),
            title: Some("Stability hotfix".into()),
            url: "https://overwatch.blizzard.com/en-us/news/patch-notes/".into(),
            hero_updates: vec![],
            sections: vec![
                PatchSection {
                    category: "Bug Fixes".into(),
                    items: vec![
                        "Fixed a disconnect when rejoining a custom game lobby.".into(),
                        "Spectator UI no longer duplicates the ultimate bar.".into(),
                    ],
                },
                PatchSection {
                    category: "Maps".into(),
                    items: vec!["Closed an out-of-bounds perch on Colosseo.".into()],
                },
            ],
        },
    ]
}

/// Insert the #89 catalog when the table is empty. Concurrent migrators lose
/// the UNIQUE version race and are treated as success (already seeded).
pub async fn ensure_patch_note_catalog(client: &Surreal<Any>) -> DbResult<()> {
    let mut count_res = client
        .query("SELECT count() AS total FROM patch_note GROUP ALL")
        .await?;
    #[derive(Deserialize, SurrealValue)]
    struct C {
        total: u64,
    }
    let n: u64 = count_res
        .take::<Option<C>>(0)?
        .map(|c| c.total)
        .unwrap_or(0);
    if n > 0 {
        return Ok(());
    }

    let now = SurrealDatetime::from(Utc::now());
    for note in initial_patch_note_catalog() {
        let (hero_updates, sections) = json_lists(&note)?;
        let row = DbPatchNote {
            id: None,
            version: note.version,
            date: note.date,
            title: note.title,
            url: note.url,
            hero_updates,
            sections,
            created_at: now,
            updated_at: now,
        };
        let created: Result<Option<DbPatchNote>, _> =
            client.create("patch_note").content(row).await;
        match created {
            Ok(_) => {}
            Err(e) if e.to_string().contains("already contains") => {}
            Err(e) => return Err(DbError::Surreal(e)),
        }
    }
    Ok(())
}

impl Database {
    /// Public list, newest display date first (then version).
    pub async fn list_patch_notes(&self) -> DbResult<Vec<PatchNote>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM patch_note ORDER BY date DESC, version DESC")
                .await?;
            let rows: Vec<DbPatchNote> = result.take(0)?;
            Ok(rows.into_iter().map(db_to_patch_note).collect())
        })
        .await
    }

    pub async fn get_patch_note_by_version(&self, version: &str) -> DbResult<PatchNote> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM patch_note WHERE version = $version LIMIT 1")
                .bind(("version", version.to_string()))
                .await?;
            let rows: Vec<DbPatchNote> = result.take(0)?;
            rows.into_iter()
                .next()
                .map(db_to_patch_note)
                .ok_or_else(|| {
                    DbError::NotFound(format!("Patch note version '{version}' not found"))
                })
        })
        .await
    }

    pub async fn create_patch_note(&self, note: &PatchNote) -> DbResult<PatchNote> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let (hero_updates, sections) = json_lists(note)?;
            let row = DbPatchNote {
                id: None,
                version: note.version.clone(),
                date: note.date.clone(),
                title: note.title.clone(),
                url: note.url.clone(),
                hero_updates,
                sections,
                created_at: now,
                updated_at: now,
            };
            let created: Option<DbPatchNote> = self
                .client
                .create("patch_note")
                .content(row)
                .await
                .map_err(map_version_unique_violation)?;
            Ok(db_to_patch_note(created.ok_or_else(|| {
                DbError::NotFound("Failed to create patch note".into())
            })?))
        })
        .await
    }

    pub async fn update_patch_note(
        &self,
        version: &str,
        patch: &UpdatePatchNoteRequest,
    ) -> DbResult<PatchNote> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM patch_note WHERE version = $version LIMIT 1")
                .bind(("version", version.to_string()))
                .await?;
            let mut db = result
                .take::<Vec<DbPatchNote>>(0)?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    DbError::NotFound(format!("Patch note version '{version}' not found"))
                })?;

            if let Some(date) = patch.date.as_deref() {
                db.date = date.to_string();
            }
            if let Some(title) = &patch.title {
                db.title = title.clone();
            }
            if let Some(url) = patch.url.as_deref() {
                db.url = url.to_string();
            }
            if let Some(hero_updates) = &patch.hero_updates {
                db.hero_updates = serde_json::to_value(hero_updates).map_err(|e| {
                    DbError::Config(format!("Failed to serialize hero_updates: {e}"))
                })?;
            }
            if let Some(sections) = &patch.sections {
                db.sections = serde_json::to_value(sections)
                    .map_err(|e| DbError::Config(format!("Failed to serialize sections: {e}")))?;
            }
            db.updated_at = SurrealDatetime::from(Utc::now());

            let id = db
                .id
                .clone()
                .ok_or_else(|| DbError::NotFound(format!("Patch note {version} missing id")))?;
            let updated: Option<DbPatchNote> = self.client.update(id).content(db).await?;
            Ok(db_to_patch_note(updated.ok_or_else(|| {
                DbError::NotFound(format!("Patch note {version} not found after update"))
            })?))
        })
        .await
    }

    pub async fn delete_patch_note(&self, version: &str) -> DbResult<()> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM patch_note WHERE version = $version LIMIT 1")
                .bind(("version", version.to_string()))
                .await?;
            let db = result
                .take::<Vec<DbPatchNote>>(0)?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    DbError::NotFound(format!("Patch note version '{version}' not found"))
                })?;
            let id = db
                .id
                .ok_or_else(|| DbError::NotFound(format!("Patch note {version} missing id")))?;
            let _: Option<DbPatchNote> = self.client.delete(id).await?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        db
    }

    #[test]
    fn catalog_has_fields_site_renders() {
        let notes = initial_patch_note_catalog();
        assert!(
            !notes.is_empty(),
            "empty catalog would show Site empty-state, not cards"
        );
        for note in &notes {
            assert!(!note.version.is_empty());
            assert!(!note.date.is_empty());
            assert!(!note.url.is_empty());
        }
        assert!(
            notes.iter().any(|n| !n.hero_updates.is_empty()),
            "need at least one hero-balance card for the Hero Balance filter"
        );
        assert!(
            notes.iter().any(|n| n
                .sections
                .iter()
                .any(|s| s.category.to_lowercase().contains("bug"))),
            "need a Bug Fixes section for that filter chip"
        );
    }

    #[tokio::test]
    async fn migration_seeds_catalog_once() {
        let db = test_db().await;
        let first = db.list_patch_notes().await.expect("list");
        assert_eq!(first.len(), initial_patch_note_catalog().len());
        ensure_patch_note_catalog(&db.client)
            .await
            .expect("re-seed");
        let second = db.list_patch_notes().await.expect("list after re-seed");
        assert_eq!(second.len(), first.len(), "seed must be one-shot");
    }

    #[tokio::test]
    async fn version_is_unique() {
        let db = test_db().await;
        let note = PatchNote {
            version: "2.18.1".into(),
            date: "2026-09-01".into(),
            title: Some("dup".into()),
            url: "https://example.test/dup".into(),
            hero_updates: vec![],
            sections: vec![],
        };
        let err = db.create_patch_note(&note).await.expect_err("dup version");
        assert!(
            matches!(err, DbError::Conflict(_)),
            "unique version must be Conflict, got {err:?}"
        );
    }

    #[tokio::test]
    async fn create_update_delete_roundtrip() {
        let db = test_db().await;
        let note = PatchNote {
            version: "9.9.9".into(),
            date: "2026-09-14".into(),
            title: Some("Test".into()),
            url: "https://example.test/notes".into(),
            hero_updates: vec![],
            sections: vec![PatchSection {
                category: "General".into(),
                items: vec!["Hello".into()],
            }],
        };
        let created = db.create_patch_note(&note).await.expect("create");
        assert_eq!(created.version, "9.9.9");

        let updated = db
            .update_patch_note(
                "9.9.9",
                &UpdatePatchNoteRequest {
                    title: Some(Some("Renamed".into())),
                    ..Default::default()
                },
            )
            .await
            .expect("update");
        assert_eq!(updated.title.as_deref(), Some("Renamed"));
        assert_eq!(updated.sections[0].category, "General");

        db.delete_patch_note("9.9.9").await.expect("delete");
        let err = db
            .get_patch_note_by_version("9.9.9")
            .await
            .expect_err("gone");
        assert!(matches!(err, DbError::NotFound(_)));
    }
}
