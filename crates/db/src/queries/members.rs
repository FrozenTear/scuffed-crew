use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb_types::RecordId;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

use crate::types::{Member, OrgRole};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbMember {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    user_id: String,
    org_role: String,
    display_name: String,
    bio: Option<String>,
    avatar_url: Option<String>,
    timezone: Option<String>,
    pronouns: Option<String>,
    availability_status: Option<String>,
    joined_at: SurrealDatetime,
    is_active: bool,
}

fn parse_role(s: &str) -> OrgRole {
    match s {
        "admin" => OrgRole::Admin,
        "officer" => OrgRole::Officer,
        "member" => OrgRole::Member,
        _ => OrgRole::Recruit,
    }
}

fn db_to_member(db: DbMember) -> Member {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    Member {
        id,
        user_id: db.user_id,
        org_role: parse_role(&db.org_role),
        display_name: db.display_name,
        bio: db.bio,
        avatar_url: db.avatar_url,
        timezone: db.timezone,
        pronouns: db.pronouns,
        availability_status: db.availability_status,
        joined_at: db.joined_at.into(),
        is_active: db.is_active,
    }
}

impl Database {
    /// Create a new org member linked to a user.
    pub async fn create_member(
        &self,
        user_id: &str,
        display_name: &str,
        role: OrgRole,
    ) -> DbResult<Member> {
        with_timeout(async {
            let db_member = DbMember {
                id: None,
                user_id: user_id.to_string(),
                org_role: role.to_string(),
                display_name: display_name.to_string(),
                bio: None,
                avatar_url: None,
                timezone: None,
                pronouns: None,
                availability_status: None,
                joined_at: SurrealDatetime::from(Utc::now()),
                is_active: true,
            };
            let created: Option<DbMember> =
                self.client.create("member").content(db_member).await?;
            Ok(db_to_member(
                created.ok_or_else(|| crate::DbError::NotFound("Failed to create member".into()))?,
            ))
        })
        .await
    }

    /// List all active members.
    pub async fn list_members(&self) -> DbResult<Vec<Member>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM member WHERE is_active = true ORDER BY display_name ASC")
                .await?;
            let members: Vec<DbMember> = result.take(0)?;
            Ok(members.into_iter().map(db_to_member).collect())
        })
        .await
    }

    /// List active members with cursor-based pagination.
    /// Fetches `limit + 1` rows so the caller can detect a next page.
    pub async fn list_members_paginated(
        &self,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<Member>> {
        with_timeout(async {
            let fetch = limit + 1;
            let mut result = self
                .client
                .query("SELECT * FROM member WHERE is_active = true ORDER BY display_name ASC LIMIT $lim START $off")
                .bind(("lim", fetch))
                .bind(("off", offset))
                .await?;
            let members: Vec<DbMember> = result.take(0)?;
            Ok(members.into_iter().map(db_to_member).collect())
        })
        .await
    }

    /// Get a member by their user_id.
    pub async fn get_member_by_user(&self, user_id: &str) -> DbResult<Option<Member>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM member WHERE user_id = $uid LIMIT 1")
                .bind(("uid", user_id.to_string()))
                .await?;
            let members: Vec<DbMember> = result.take(0)?;
            Ok(members.into_iter().next().map(db_to_member))
        })
        .await
    }

    /// Get a member by their record ID.
    pub async fn get_member(&self, id: &str) -> DbResult<Option<Member>> {
        with_timeout(async {
            let db_member: Option<DbMember> = self.client.select(("member", id)).await?;
            Ok(db_member.map(db_to_member))
        })
        .await
    }

    /// Update a member's profile fields.
    pub async fn update_member(
        &self,
        id: &str,
        display_name: Option<&str>,
        bio: Option<Option<&str>>,
        avatar_url: Option<Option<&str>>,
        timezone: Option<Option<&str>>,
        pronouns: Option<Option<&str>>,
        availability_status: Option<Option<&str>>,
        is_active: Option<bool>,
    ) -> DbResult<Member> {
        with_timeout(async {
            let existing: Option<DbMember> = self.client.select(("member", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Member {id} not found")))?;

            if let Some(name) = display_name {
                db.display_name = name.to_string();
            }
            if let Some(new_bio) = bio {
                db.bio = new_bio.map(|s| s.to_string());
            }
            if let Some(new_avatar) = avatar_url {
                db.avatar_url = new_avatar.map(|s| s.to_string());
            }
            if let Some(new_tz) = timezone {
                db.timezone = new_tz.map(|s| s.to_string());
            }
            if let Some(new_pronouns) = pronouns {
                db.pronouns = new_pronouns.map(|s| s.to_string());
            }
            if let Some(new_status) = availability_status {
                db.availability_status = new_status.map(|s| s.to_string());
            }
            if let Some(active) = is_active {
                db.is_active = active;
            }

            let updated: Option<DbMember> =
                self.client.update(("member", id)).content(db).await?;
            Ok(db_to_member(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Member {id} not found after update"))
            })?))
        })
        .await
    }

    /// Change a member's org role.
    pub async fn change_member_role(&self, id: &str, new_role: OrgRole) -> DbResult<Member> {
        with_timeout(async {
            let existing: Option<DbMember> = self.client.select(("member", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Member {id} not found")))?;

            db.org_role = new_role.to_string();

            let updated: Option<DbMember> =
                self.client.update(("member", id)).content(db).await?;
            Ok(db_to_member(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Member {id} not found after update"))
            })?))
        })
        .await
    }
}
