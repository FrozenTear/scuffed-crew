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
    extra_relay_urls: String,
    #[serde(default = "default_layout")]
    public_layout: String,
    #[serde(default)]
    homepage_json: String,
    #[serde(default)]
    nav_json: String,
    #[serde(default)]
    page_bg_color: String,
    #[serde(default)]
    page_bg_image_url: String,
    updated_at: SurrealDatetime,
}

fn default_layout() -> String {
    "hub".into()
}

fn db_to_settings(db: DbSiteSettings) -> SiteSettings {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    let public_layout = if db.public_layout == "landing" {
        "landing".into()
    } else {
        "hub".into()
    };
    SiteSettings {
        id,
        org_name: db.org_name,
        site_description: db.site_description,
        recruitment_open: db.recruitment_open,
        recruitment_message: db.recruitment_message,
        min_age: db.min_age,
        forum_backend: db.forum_backend,
        extra_relay_urls: db.extra_relay_urls,
        public_layout,
        homepage_json: db.homepage_json,
        nav_json: db.nav_json,
        page_bg_color: db.page_bg_color,
        page_bg_image_url: db.page_bg_image_url,
        updated_at: db.updated_at.into(),
    }
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

            let defaults = DbSiteSettings {
                id: None,
                org_name: "The Scuffed Crew".to_string(),
                site_description: "EMEA Gaming Organization".to_string(),
                recruitment_open: true,
                recruitment_message: "We are currently recruiting! Apply now to join the crew."
                    .to_string(),
                min_age: 16,
                forum_backend: "local".to_string(),
                extra_relay_urls: String::new(),
                public_layout: "hub".into(),
                homepage_json: String::new(),
                nav_json: String::new(),
                page_bg_color: String::new(),
                page_bg_image_url: String::new(),
                updated_at: SurrealDatetime::from(Utc::now()),
            };
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
            if let Some(layout) = public_layout {
                db.public_layout = if layout == "landing" {
                    "landing".into()
                } else {
                    "hub".into()
                };
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
