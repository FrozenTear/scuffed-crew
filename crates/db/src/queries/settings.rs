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
