use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::sql::Thing;

use crate::types::{Member, OrgRole};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DbMember {
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    id: Option<Thing>,
    user_id: String,
    org_role: String,
    display_name: String,
    bio: Option<String>,
    joined_at: DateTime<Utc>,
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
        .map(|t| t.id.to_raw())
        .unwrap_or_else(|| "unknown".to_string());
    Member {
        id,
        user_id: db.user_id,
        org_role: parse_role(&db.org_role),
        display_name: db.display_name,
        bio: db.bio,
        joined_at: db.joined_at,
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
                joined_at: Utc::now(),
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
