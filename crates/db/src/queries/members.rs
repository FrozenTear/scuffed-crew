use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use scuffed_auth::crypto::EncryptedBlob;

use crate::types::{Member, NostrKeyMode, OrgRole};
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
    nostr_pubkey: Option<String>,
    nostr_key_mode: Option<String>,
    nostr_secret_key_encrypted: Option<serde_json::Value>,
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

fn parse_key_mode(s: &str) -> NostrKeyMode {
    match s {
        "external" => NostrKeyMode::External,
        _ => NostrKeyMode::ServerManaged,
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
        nostr_pubkey: db.nostr_pubkey,
        nostr_key_mode: db.nostr_key_mode.as_deref().map(parse_key_mode),
        nostr_secret_key_encrypted: db
            .nostr_secret_key_encrypted
            .and_then(|v| serde_json::from_value(v).ok()),
        joined_at: db.joined_at.into(),
        is_active: db.is_active,
    }
}

impl Database {
    /// Create a new org member linked to a user.
    ///
    /// Automatically provisions a server-managed Nostr keypair if CryptoService
    /// is configured (ENCRYPTION_KEY set). If not, the member is created without
    /// a keypair — one can be provisioned later.
    pub async fn create_member(
        &self,
        user_id: &str,
        display_name: &str,
        role: OrgRole,
    ) -> DbResult<Member> {
        // Generate keypair if crypto is available
        let keypair = crate::queries::nostr_keys::generate_encrypted_keypair(self).ok();

        with_timeout(async {
            let (nostr_pubkey, nostr_key_mode, nostr_secret_key_encrypted) = match keypair {
                Some(kp) => (
                    Some(kp.pubkey),
                    Some(NostrKeyMode::ServerManaged.to_string()),
                    serde_json::to_value(kp.secret_key_encrypted).ok(),
                ),
                None => (None, None, None),
            };

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
                nostr_pubkey,
                nostr_key_mode,
                nostr_secret_key_encrypted,
                joined_at: SurrealDatetime::from(Utc::now()),
                is_active: true,
            };
            let created: Option<DbMember> = self.client.create("member").content(db_member).await?;
            Ok(db_to_member(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create member".into())
            })?))
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
    pub async fn list_members_paginated(&self, limit: u32, offset: u32) -> DbResult<Vec<Member>> {
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
        nostr_pubkey: Option<Option<&str>>,
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
            if let Some(new_nostr) = nostr_pubkey {
                match new_nostr {
                    Some(pubkey) => {
                        // Member is linking an external pubkey → set external mode,
                        // clear any server-managed encrypted secret key.
                        db.nostr_pubkey = Some(pubkey.to_string());
                        db.nostr_key_mode = Some(NostrKeyMode::External.to_string());
                        db.nostr_secret_key_encrypted = None;
                    }
                    None => {
                        // Member is removing their pubkey — clear everything.
                        db.nostr_pubkey = None;
                        db.nostr_key_mode = None;
                        db.nostr_secret_key_encrypted = None;
                    }
                }
            }
            if let Some(active) = is_active {
                db.is_active = active;
            }

            let updated: Option<DbMember> = self.client.update(("member", id)).content(db).await?;
            Ok(db_to_member(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Member {id} not found after update"))
            })?))
        })
        .await
    }

    /// List all members who have a Nostr pubkey set (for NIP-05 well-known endpoint).
    pub async fn list_nostr_identities(&self) -> DbResult<Vec<Member>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM member WHERE is_active = true AND nostr_pubkey != NONE")
                .await?;
            let members: Vec<DbMember> = result.take(0)?;
            Ok(members.into_iter().map(db_to_member).collect())
        })
        .await
    }

    /// Update a member's Nostr keypair fields (key mode + encrypted secret key).
    ///
    /// Used when provisioning server-managed keys or when a member links an external key.
    pub async fn update_member_nostr_keys(
        &self,
        id: &str,
        pubkey: Option<&str>,
        key_mode: Option<&str>,
        encrypted_secret: Option<&EncryptedBlob>,
    ) -> DbResult<Member> {
        with_timeout(async {
            let existing: Option<DbMember> = self.client.select(("member", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Member {id} not found")))?;

            db.nostr_pubkey = pubkey.map(|s| s.to_string());
            db.nostr_key_mode = key_mode.map(|s| s.to_string());
            db.nostr_secret_key_encrypted =
                encrypted_secret.and_then(|blob| serde_json::to_value(blob).ok());

            let updated: Option<DbMember> = self.client.update(("member", id)).content(db).await?;
            Ok(db_to_member(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Member {id} not found after update"))
            })?))
        })
        .await
    }

    /// Get a member by their Nostr public key.
    pub async fn get_member_by_nostr_pubkey(&self, pubkey: &str) -> DbResult<Option<Member>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM member WHERE nostr_pubkey = $pk AND is_active = true LIMIT 1")
                .bind(("pk", pubkey.to_string()))
                .await?;
            let members: Vec<DbMember> = result.take(0)?;
            Ok(members.into_iter().next().map(db_to_member))
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

            let updated: Option<DbMember> = self.client.update(("member", id)).content(db).await?;
            Ok(db_to_member(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Member {id} not found after update"))
            })?))
        })
        .await
    }

    /// Count currently active admin members (is_active = true, org_role = admin).
    ///
    /// Includes suspended admins (suspension keeps `is_active`). Prefer
    /// [`Self::count_actionable_admins`] for last-admin policy guards.
    pub async fn count_active_admins(&self) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }
            let mut result = self
                .client
                .query(
                    "SELECT count() FROM member WHERE is_active = true AND org_role = 'admin' GROUP ALL",
                )
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count).unwrap_or(0))
        })
        .await
    }

    /// Count admins who can still use admin tools: active, role=admin, and not
    /// currently suspended or banned.
    ///
    /// Use this for last-admin demote/deactivate/suspend/ban guards and setup
    /// recovery (`has_admin_member`). Suspended admins keep `is_active = true`
    /// but must not inflate the count or the org can lock itself out.
    pub async fn count_actionable_admins(&self) -> DbResult<u64> {
        with_timeout(async {
            let mut admins_result = self
                .client
                .query(
                    "SELECT * FROM member WHERE is_active = true AND org_role = 'admin'",
                )
                .await?;
            let admins: Vec<DbMember> = admins_result.take(0)?;
            if admins.is_empty() {
                return Ok(0);
            }

            #[derive(Deserialize, SurrealValue)]
            struct BlockedMid {
                member_id: String,
            }
            let mut blocked_result = self
                .client
                .query(
                    "SELECT member_id FROM moderation_action WHERE is_active = true AND action_type IN ['suspension', 'ban'] AND (expires_at IS NONE OR expires_at > time::now())",
                )
                .await?;
            let blocked: Vec<BlockedMid> = blocked_result.take(0)?;
            let blocked_ids: std::collections::HashSet<String> =
                blocked.into_iter().map(|b| b.member_id).collect();

            let count = admins
                .into_iter()
                .map(db_to_member)
                .filter(|m| !blocked_ids.contains(&m.id))
                .count() as u64;
            Ok(count)
        })
        .await
    }

    /// Fail with [`crate::DbError::Conflict`] if no actionable admin remains.
    ///
    /// Call after demote / deactivate / suspend / ban of an actionable admin to
    /// catch concurrent last-admin races; callers should compensate then surface 409.
    pub async fn assert_has_actionable_admin(&self) -> DbResult<()> {
        let n = self.count_actionable_admins().await?;
        if n == 0 {
            return Err(crate::DbError::Conflict(
                "Would leave org without an actionable admin".into(),
            ));
        }
        Ok(())
    }
}
