use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::SiteSettings;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbSiteSettings {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    org_name: String,
    site_description: String,
    recruitment_open: bool,
    /// Older rows may omit this — missing means enabled (back-compat).
    #[surreal(default)]
    #[serde(default)]
    strategies_enabled: Option<bool>,
    /// Older rows may omit this — missing means officers cannot edit teams.
    #[surreal(default)]
    #[serde(default)]
    officers_can_edit_teams: Option<bool>,
    recruitment_message: String,
    min_age: u32,
    forum_backend: String,
    /// Older rows may omit this — SurrealValue needs `surreal(default)`, not only serde.
    #[surreal(default)]
    #[serde(default)]
    extra_relay_urls: String,
    #[surreal(default)]
    #[serde(default)]
    home_shell: String,
    #[surreal(default)]
    #[serde(default)]
    home_skin: String,
    #[surreal(default)]
    #[serde(default = "default_layout")]
    public_layout: String,
    #[surreal(default)]
    #[serde(default)]
    homepage_json: String,
    #[surreal(default)]
    #[serde(default)]
    nav_json: String,
    #[surreal(default)]
    #[serde(default)]
    page_bg_color: String,
    #[surreal(default)]
    #[serde(default)]
    page_bg_image_url: String,
    #[surreal(default)]
    #[serde(default)]
    brand_accent_dark: String,
    #[surreal(default)]
    #[serde(default)]
    brand_accent_light: String,
    /// Older rows omit this. Missing reads as "" (Urbanist).
    #[surreal(default)]
    #[serde(default)]
    brand_font_display: String,
    updated_at: SurrealDatetime,
}

fn default_layout() -> String {
    "hub".into()
}

/// Resolve shell/skin for API consumers (read path — no DB write).
/// Empty stored values are inferred from public_layout / homepage / brand.
fn resolve_shell_skin(db: &DbSiteSettings) -> (String, String, String) {
    use scuffed_types::{
        infer_home_shell_from_public_layout, infer_home_skin, HomeShell, HomeSkin, HomepageContent,
    };

    let shell = if db.home_shell.trim().is_empty() {
        infer_home_shell_from_public_layout(&db.public_layout)
    } else {
        HomeShell::from_str_lossy(&db.home_shell)
    };

    let homepage = HomepageContent::from_json(&db.homepage_json);
    let skin = if db.home_skin.trim().is_empty() {
        infer_home_skin(&db.brand_accent_dark, &db.brand_accent_light, &homepage)
    } else {
        HomeSkin::from_str_lossy(&db.home_skin)
    };

    // Dual-write view: public_layout always mirrors shell for consumers that only read layout.
    let public_layout = shell.to_public_layout().as_str().to_string();

    (shell.as_str().into(), skin.as_str().into(), public_layout)
}

fn db_to_settings(db: DbSiteSettings) -> SiteSettings {
    let (home_shell, home_skin, public_layout) = resolve_shell_skin(&db);
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    SiteSettings {
        id,
        org_name: db.org_name,
        site_description: db.site_description,
        recruitment_open: db.recruitment_open,
        strategies_enabled: db.strategies_enabled.unwrap_or(true),
        officers_can_edit_teams: db.officers_can_edit_teams.unwrap_or(false),
        recruitment_message: db.recruitment_message,
        min_age: db.min_age,
        forum_backend: db.forum_backend,
        extra_relay_urls: db.extra_relay_urls,
        home_shell,
        home_skin,
        public_layout,
        homepage_json: db.homepage_json,
        nav_json: db.nav_json,
        page_bg_color: db.page_bg_color,
        page_bg_image_url: db.page_bg_image_url,
        brand_accent_dark: db.brand_accent_dark,
        brand_accent_light: db.brand_accent_light,
        brand_font_display: db.brand_font_display,
        updated_at: db.updated_at.into(),
    }
}

/// Apply dual-write: shell drives public_layout; fill empty shell/skin on write.
fn apply_shell_skin_dual_write(db: &mut DbSiteSettings) {
    use scuffed_types::{
        infer_home_shell_from_public_layout, infer_home_skin, HomeShell, HomeSkin, HomepageContent,
    };

    if db.home_shell.trim().is_empty() {
        db.home_shell = infer_home_shell_from_public_layout(&db.public_layout)
            .as_str()
            .into();
    } else {
        // Normalize lossy
        db.home_shell = HomeShell::from_str_lossy(&db.home_shell).as_str().into();
    }

    if db.home_skin.trim().is_empty() {
        let homepage = HomepageContent::from_json(&db.homepage_json);
        db.home_skin = infer_home_skin(&db.brand_accent_dark, &db.brand_accent_light, &homepage)
            .as_str()
            .into();
    } else {
        db.home_skin = HomeSkin::from_str_lossy(&db.home_skin).as_str().into();
    }

    let shell = HomeShell::from_str_lossy(&db.home_shell);
    db.public_layout = shell.to_public_layout().as_str().into();
}

impl Database {
    /// Get site settings, creating defaults if none exist.
    pub async fn get_settings(&self) -> DbResult<SiteSettings> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM site_settings LIMIT 1")
                .await?;
            let entries: Vec<DbSiteSettings> = result.take(0)?;

            if let Some(settings) = entries.into_iter().next() {
                return Ok(db_to_settings(settings));
            }

            let mut defaults = DbSiteSettings {
                id: None,
                org_name: "My Clan".to_string(),
                site_description: "Gaming clan".to_string(),
                recruitment_open: true,
                strategies_enabled: Some(true),
                officers_can_edit_teams: Some(false),
                recruitment_message: "Recruitment is closed right now. Check back later."
                    .to_string(),
                min_age: 16,
                forum_backend: "local".to_string(),
                extra_relay_urls: String::new(),
                home_shell: "ops_hub".into(),
                home_skin: "clean".into(),
                public_layout: "hub".into(),
                homepage_json: String::new(),
                nav_json: String::new(),
                page_bg_color: String::new(),
                page_bg_image_url: String::new(),
                brand_accent_dark: String::new(),
                brand_accent_light: String::new(),
                brand_font_display: String::new(),
                updated_at: SurrealDatetime::from(Utc::now()),
            };
            apply_shell_skin_dual_write(&mut defaults);
            let created: Option<DbSiteSettings> = self
                .client
                .create("site_settings")
                .content(defaults)
                .await?;
            Ok(db_to_settings(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create default settings".into())
            })?))
        })
        .await
    }

    /// Update site settings.
    ///
    /// When `home_shell` / `home_skin` are set, `public_layout` is dual-written from shell.
    /// When only `public_layout` is set (legacy client), shell is derived Hub→ops_hub / Landing→recruit_landing.
    pub async fn update_settings(
        &self,
        org_name: Option<&str>,
        site_description: Option<&str>,
        recruitment_open: Option<bool>,
        recruitment_message: Option<&str>,
        min_age: Option<u32>,
        forum_backend: Option<&str>,
        extra_relay_urls: Option<&str>,
        public_layout: Option<&str>,
        homepage_json: Option<&str>,
        nav_json: Option<&str>,
        page_bg_color: Option<&str>,
        page_bg_image_url: Option<&str>,
        brand_accent_dark: Option<&str>,
        brand_accent_light: Option<&str>,
        home_shell: Option<&str>,
        home_skin: Option<&str>,
        strategies_enabled: Option<bool>,
        officers_can_edit_teams: Option<bool>,
        brand_font_display: Option<&str>,
    ) -> DbResult<SiteSettings> {
        with_timeout(async {
            let current = self.get_settings().await?;
            let id = &current.id;

            let existing: Option<DbSiteSettings> =
                self.client.select(("site_settings", id.as_str())).await?;
            let mut db =
                existing.ok_or_else(|| crate::DbError::NotFound("Settings not found".into()))?;

            if let Some(name) = org_name {
                db.org_name = name.to_string();
            }
            if let Some(desc) = site_description {
                db.site_description = desc.to_string();
            }
            if let Some(open) = recruitment_open {
                db.recruitment_open = open;
            }
            if let Some(enabled) = strategies_enabled {
                db.strategies_enabled = Some(enabled);
            } else if db.strategies_enabled.is_none() {
                db.strategies_enabled = Some(true);
            }
            if let Some(allowed) = officers_can_edit_teams {
                db.officers_can_edit_teams = Some(allowed);
            } else if db.officers_can_edit_teams.is_none() {
                db.officers_can_edit_teams = Some(false);
            }
            if let Some(msg) = recruitment_message {
                db.recruitment_message = msg.to_string();
            }
            if let Some(age) = min_age {
                db.min_age = age;
            }
            if let Some(backend) = forum_backend {
                db.forum_backend = backend.to_string();
            }
            if let Some(urls) = extra_relay_urls {
                db.extra_relay_urls = urls.to_string();
            }
            if let Some(json) = homepage_json {
                db.homepage_json = json.to_string();
            }
            if let Some(json) = nav_json {
                db.nav_json = json.to_string();
            }
            if let Some(color) = page_bg_color {
                db.page_bg_color = color.to_string();
            }
            if let Some(url) = page_bg_image_url {
                db.page_bg_image_url = url.to_string();
            }
            if let Some(c) = brand_accent_dark {
                db.brand_accent_dark = c.to_string();
            }
            if let Some(c) = brand_accent_light {
                db.brand_accent_light = c.to_string();
            }
            if let Some(font) = brand_font_display {
                db.brand_font_display = font.to_string();
            }

            // Shell preferred; layout-only updates map to shell for legacy clients.
            if let Some(shell) = home_shell {
                db.home_shell = shell.to_string();
            } else if let Some(layout) = public_layout {
                // Only when shell not explicitly set this request
                use scuffed_types::{HomeShell, PublicLayout};
                let pl = PublicLayout::from_str_lossy(layout);
                db.home_shell = HomeShell::from_public_layout(pl).as_str().into();
            }

            if let Some(skin) = home_skin {
                db.home_skin = skin.to_string();
            }

            // Always normalize shell/skin and dual-write public_layout from shell.
            apply_shell_skin_dual_write(&mut db);
            db.updated_at = SurrealDatetime::from(Utc::now());

            let updated: Option<DbSiteSettings> = self
                .client
                .update(("site_settings", id.as_str()))
                .content(db)
                .await?;
            Ok(db_to_settings(updated.ok_or_else(|| {
                crate::DbError::NotFound("Settings not found after update".into())
            })?))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use surrealdb_types::SurrealValue;

    use crate::migrations::{backfill_officers_can_edit_teams, run_migrations};
    use crate::Database;

    async fn migrated() -> Database {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        db
    }

    #[derive(Debug, Deserialize, SurrealValue)]
    struct FlagRow {
        officers_can_edit_teams: Option<bool>,
    }

    async fn stored_flag(db: &Database) -> Option<bool> {
        let mut q = db
            .client
            .query("SELECT officers_can_edit_teams FROM site_settings")
            .await
            .expect("select flag")
            .check()
            .expect("select flag ok");
        let rows: Vec<FlagRow> = q.take(0).expect("take flag");
        assert_eq!(rows.len(), 1, "one site_settings row");
        rows[0].officers_can_edit_teams
    }

    #[tokio::test]
    async fn officers_can_edit_teams_defaults_false_on_fresh_db() {
        let db = migrated().await;
        let settings = db.get_settings().await.expect("create defaults");
        assert!(!settings.officers_can_edit_teams);
        assert_eq!(stored_flag(&db).await, Some(false));
    }

    #[tokio::test]
    async fn officers_can_edit_teams_backfills_false_and_keeps_explicit_true() {
        let db = migrated().await;
        let created = db.get_settings().await.expect("settings");
        assert!(!created.officers_can_edit_teams);

        // Drop the column so the existing row looks like a pre-L14 record.
        db.client
            .query("REMOVE FIELD IF EXISTS officers_can_edit_teams ON TABLE site_settings")
            .await
            .expect("remove field")
            .check()
            .expect("remove field ok");

        run_migrations(&db.client).await.expect("re-migrate");
        assert_eq!(stored_flag(&db).await, Some(false));
        assert!(
            !db.get_settings()
                .await
                .expect("read")
                .officers_can_edit_teams
        );

        db.update_settings(
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(true),
            None,
        )
        .await
        .expect("admin sets flag");
        backfill_officers_can_edit_teams(&db.client)
            .await
            .expect("backfill again");
        run_migrations(&db.client)
            .await
            .expect("restart migrations");
        assert_eq!(stored_flag(&db).await, Some(true));
    }

    async fn set_font(db: &Database, font: Option<&str>) -> crate::types::SiteSettings {
        db.update_settings(
            None, None, None, None, None, None, None, None, None, None, None, None, None, None,
            None, None, None, None, font,
        )
        .await
        .expect("update brand_font_display")
    }

    /// Raw stored value: `None` when the row has no such field at all.
    async fn stored_font(db: &Database) -> Option<String> {
        #[derive(Debug, Deserialize, SurrealValue)]
        struct FontRow {
            brand_font_display: Option<String>,
        }
        let mut q = db
            .client
            .query("SELECT brand_font_display FROM site_settings")
            .await
            .expect("select font")
            .check()
            .expect("select font ok");
        let rows: Vec<FontRow> = q.take(0).expect("take font");
        assert_eq!(rows.len(), 1, "one site_settings row");
        rows[0].brand_font_display.clone()
    }

    #[tokio::test]
    async fn brand_font_display_defaults_empty_and_round_trips() {
        let db = migrated().await;
        let created = db.get_settings().await.expect("create defaults");
        assert_eq!(created.brand_font_display, "");
        assert_eq!(stored_font(&db).await.as_deref(), Some(""));

        for font in ["ibm-plex-sans", "urbanist", ""] {
            let written = set_font(&db, Some(font)).await;
            assert_eq!(written.brand_font_display, font);
            assert_eq!(stored_font(&db).await.as_deref(), Some(font));
            let read = db.get_settings().await.expect("read");
            assert_eq!(read.brand_font_display, font);
        }
    }

    #[tokio::test]
    async fn brand_font_display_none_keeps_the_stored_value() {
        let db = migrated().await;
        set_font(&db, Some("ibm-plex-sans")).await;

        // An unrelated update passes None for the font.
        let written = set_font(&db, None).await;
        assert_eq!(written.brand_font_display, "ibm-plex-sans");
        assert_eq!(
            db.get_settings().await.expect("read").brand_font_display,
            "ibm-plex-sans"
        );
        // Restarting (re-running migrations) does not reset it either.
        run_migrations(&db.client).await.expect("re-migrate");
        assert_eq!(stored_font(&db).await.as_deref(), Some("ibm-plex-sans"));
    }

    #[tokio::test]
    async fn row_written_before_brand_font_display_reads_as_empty() {
        let db = migrated().await;
        // Schema as it was before this field existed, then a row written under it.
        db.client
            .query("REMOVE FIELD IF EXISTS brand_font_display ON TABLE site_settings")
            .await
            .expect("remove field")
            .check()
            .expect("remove field ok");
        db.client
            .query("CREATE site_settings SET org_name = 'Old Clan', brand_accent_dark = '#38bdf8'")
            .await
            .expect("create old row")
            .check()
            .expect("create old row ok");
        assert_eq!(stored_font(&db).await, None, "old row has no font field");
        let old = db
            .get_settings()
            .await
            .expect("old row reads before migrating");
        assert_eq!(old.org_name, "Old Clan");
        assert_eq!(old.brand_font_display, "");

        // Upgrade: the new field definition lands on the existing row. The boot
        // backfill inside run_migrations is the only write here; its error is
        // only logged there, so the stored "" is the proof that it went through.
        run_migrations(&db.client).await.expect("migrate");
        assert_eq!(
            stored_font(&db).await.as_deref(),
            Some(""),
            "the boot backfill wrote the old row and the font default filled in"
        );
        backfill_officers_can_edit_teams(&db.client)
            .await
            .expect("boot backfill is accepted on a row from before the field");
        assert_eq!(stored_flag(&db).await, Some(false));
        let read = db
            .get_settings()
            .await
            .expect("old row reads after migrating");
        assert_eq!(read.org_name, "Old Clan");
        assert_eq!(read.brand_accent_dark, "#38bdf8");
        assert_eq!(read.brand_font_display, "");

        // An update that does not mention the font keeps it empty, and a later one sets it.
        let kept = set_font(&db, None).await;
        assert_eq!(kept.brand_font_display, "");
        assert_eq!(kept.org_name, "Old Clan");
        let set = set_font(&db, Some("ibm-plex-sans")).await;
        assert_eq!(set.brand_font_display, "ibm-plex-sans");
        assert_eq!(stored_font(&db).await.as_deref(), Some("ibm-plex-sans"));

        // A stored font survives the same partial update on later boots.
        backfill_officers_can_edit_teams(&db.client)
            .await
            .expect("backfill with a stored font");
        run_migrations(&db.client).await.expect("restart");
        assert_eq!(stored_font(&db).await.as_deref(), Some("ibm-plex-sans"));
    }

    /// The shipped definition: the default is the empty string and it is
    /// `DEFAULT ALWAYS` (a plain DEFAULT would refuse the UNSET below).
    #[tokio::test]
    async fn brand_font_display_field_default_is_always_empty() {
        let db = migrated().await;
        db.client
            .query("CREATE site_settings SET org_name = 'Raw Clan'")
            .await
            .expect("raw create")
            .check()
            .expect("raw create ok");
        assert_eq!(stored_font(&db).await.as_deref(), Some(""));

        // UNSET leaves no value in the written row, so only ALWAYS can refill it.
        db.client
            .query("UPDATE site_settings UNSET brand_font_display")
            .await
            .expect("unset")
            .check()
            .expect("unset is accepted and refilled");
        assert_eq!(stored_font(&db).await.as_deref(), Some(""));
    }

    /// `DbSiteSettings` as it is on the merge base (6b098007): the row a
    /// rolled-back build reads and writes. It has no `brand_font_display`.
    #[derive(Debug, Clone, serde::Serialize, Deserialize, SurrealValue)]
    struct OldBuildSiteSettings {
        #[surreal(default)]
        id: Option<surrealdb_types::RecordId>,
        org_name: String,
        site_description: String,
        recruitment_open: bool,
        #[surreal(default)]
        strategies_enabled: Option<bool>,
        #[surreal(default)]
        officers_can_edit_teams: Option<bool>,
        recruitment_message: String,
        min_age: u32,
        forum_backend: String,
        #[surreal(default)]
        extra_relay_urls: String,
        #[surreal(default)]
        home_shell: String,
        #[surreal(default)]
        home_skin: String,
        #[surreal(default)]
        public_layout: String,
        #[surreal(default)]
        homepage_json: String,
        #[surreal(default)]
        nav_json: String,
        #[surreal(default)]
        page_bg_color: String,
        #[surreal(default)]
        page_bg_image_url: String,
        #[surreal(default)]
        brand_accent_dark: String,
        #[surreal(default)]
        brand_accent_light: String,
        updated_at: surrealdb::types::Datetime,
    }

    /// The old build's `update_settings`: select the row, change it, write it
    /// back whole with `.content(..)`.
    async fn old_build_save(db: &Database, org_name: &str) -> Result<String, String> {
        let id = db.get_settings().await.expect("settings id").id;
        let existing: Option<OldBuildSiteSettings> = db
            .client
            .select(("site_settings", id.as_str()))
            .await
            .map_err(|e| e.to_string())?;
        let mut row = existing.expect("row");
        row.org_name = org_name.to_string();
        let updated: Option<OldBuildSiteSettings> = db
            .client
            .update(("site_settings", id.as_str()))
            .content(row)
            .await
            .map_err(|e| e.to_string())?;
        Ok(updated.expect("row after update").org_name)
    }

    /// Rollback A in docs/api/site-settings.md: revert only, the field
    /// definition stays. The old build's settings save must still be accepted.
    #[tokio::test]
    async fn rollback_old_build_saves_settings_with_the_field_definition_in_place() {
        let db = migrated().await;
        db.get_settings().await.expect("create defaults");

        // Stored default.
        assert_eq!(
            old_build_save(&db, "Rolled Back").await.as_deref(),
            Ok("Rolled Back")
        );
        assert_eq!(stored_font(&db).await.as_deref(), Some(""));

        // Stored non-default: the old build's write has no font, so it is
        // accepted and the font goes back to the default.
        set_font(&db, Some("ibm-plex-sans")).await;
        assert_eq!(
            old_build_save(&db, "Rolled Back Twice").await.as_deref(),
            Ok("Rolled Back Twice")
        );
        assert_eq!(stored_font(&db).await.as_deref(), Some(""));

        // The old build's boot backfill is the same statement as today's.
        backfill_officers_can_edit_teams(&db.client)
            .await
            .expect("old build boot backfill");

        // Rolling forward again needs nothing: the row reads and saves.
        run_migrations(&db.client).await.expect("roll forward");
        assert_eq!(set_font(&db, None).await.brand_font_display, "");
        assert_eq!(
            set_font(&db, Some("urbanist")).await.brand_font_display,
            "urbanist"
        );
    }

    /// The one fenced `surql` statement in the doc's rollback section.
    fn documented_rollback_query() -> String {
        let doc = include_str!("../../../../docs/api/site-settings.md");
        let section = doc
            .split("## Rollback")
            .nth(1)
            .expect("doc has a Rollback section");
        let block = section
            .split("```surql\n")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("Rollback section has a surql block");
        block.trim().to_string()
    }

    /// Rollback B: the optional cleanup query, run exactly as the doc gives it.
    #[tokio::test]
    async fn rollback_documented_remove_field_query_works() {
        let query = documented_rollback_query();
        assert_eq!(
            query,
            "REMOVE FIELD IF EXISTS brand_font_display ON TABLE site_settings;"
        );

        let db = migrated().await;
        set_font(&db, Some("ibm-plex-sans")).await;

        // With the definition gone the old build's save drops the stored font.
        db.client
            .query(&query)
            .await
            .expect("doc query")
            .check()
            .expect("doc query ok");
        assert_eq!(
            old_build_save(&db, "Cleaned Up").await.as_deref(),
            Ok("Cleaned Up")
        );
        assert_eq!(stored_font(&db).await, None, "no font left on the row");
        backfill_officers_can_edit_teams(&db.client)
            .await
            .expect("old build boot backfill");

        // Idempotent, and a later roll forward starts from the default.
        db.client
            .query(&query)
            .await
            .expect("doc query twice")
            .check()
            .expect("doc query twice ok");
        run_migrations(&db.client).await.expect("roll forward");
        assert_eq!(
            db.get_settings().await.expect("read").brand_font_display,
            ""
        );
        assert_eq!(stored_font(&db).await.as_deref(), Some(""));
    }
}
