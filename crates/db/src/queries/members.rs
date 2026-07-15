use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use scuffed_auth::crypto::EncryptedBlob;

use crate::types::{Member, NostrKeyMode, OrgRole};
use crate::{with_timeout, Database, DbResult};

/// Columns safe to load for list/API paths — omits `nostr_secret_key_encrypted`
/// so encrypted secrets never enter process memory on bulk reads.
const MEMBER_SAFE_COLS: &str = "id, user_id, org_role, display_name, bio, avatar_url, timezone, \
    pronouns, availability_status, nostr_pubkey, nostr_key_mode, joined_at, is_active";

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
    /// May be omitted from SELECT projections — both serde and SurrealValue defaults required.
    #[serde(default)]
    #[surreal(default)]
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

fn parse_key_mode(s: &str) -> DbResult<Option<NostrKeyMode>> {
    match s {
        "external" => Ok(Some(NostrKeyMode::External)),
        "server_managed" => Ok(Some(NostrKeyMode::ServerManaged)),
        other => Err(crate::DbError::Config(format!(
            "Unknown nostr_key_mode: {other}"
        ))),
    }
}

fn parse_encrypted_secret(
    v: Option<serde_json::Value>,
) -> DbResult<Option<EncryptedBlob>> {
    match v {
        None => Ok(None),
        Some(val) if val.is_null() => Ok(None),
        Some(val) => serde_json::from_value(val).map(Some).map_err(|e| {
            crate::DbError::Config(format!("Corrupt nostr_secret_key_encrypted: {e}"))
        }),
    }
}

fn db_to_member(db: DbMember) -> DbResult<Member> {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    let nostr_key_mode = match db.nostr_key_mode.as_deref() {
        None => None,
        Some(s) => parse_key_mode(s)?,
    };
    Ok(Member {
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
        nostr_key_mode,
        nostr_secret_key_encrypted: parse_encrypted_secret(db.nostr_secret_key_encrypted)?,
        joined_at: db.joined_at.into(),
        is_active: db.is_active,
    })
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
            db_to_member(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create member".into())
            })?)
        })
        .await
    }

    /// Count active members (for public overview — avoids full list).
    pub async fn count_active_members(&self) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }
            let mut result = self
                .client
                .query("SELECT count() FROM member WHERE is_active = true GROUP ALL")
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count).unwrap_or(0))
        })
        .await
    }

    /// List all active members (hard-capped, no secrets). Prefer paginated API.
    pub async fn list_members(&self) -> DbResult<Vec<Member>> {
        self.list_members_paginated(500, 0).await
    }

    /// List active members with cursor-based pagination.
    /// Fetches `limit + 1` rows so the caller can detect a next page.
    /// Omits encrypted Nostr secrets from the SELECT projection.
    pub async fn list_members_paginated(&self, limit: u32, offset: u32) -> DbResult<Vec<Member>> {
        with_timeout(async {
            let fetch = limit + 1;
            let q = format!(
                "SELECT {MEMBER_SAFE_COLS} FROM member WHERE is_active = true \
                 ORDER BY display_name ASC LIMIT $lim START $off"
            );
            let mut result = self
                .client
                .query(&q)
                .bind(("lim", fetch))
                .bind(("off", offset))
                .await?;
            let members: Vec<DbMember> = result.take(0)?;
            members.into_iter().map(db_to_member).collect::<DbResult<Vec<_>>>()
        })
        .await
    }

    /// Get a member by their user_id (full row including encrypted secret for server signing).
    pub async fn get_member_by_user(&self, user_id: &str) -> DbResult<Option<Member>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM member WHERE user_id = $uid LIMIT 1")
                .bind(("uid", user_id.to_string()))
                .await?;
            let members: Vec<DbMember> = result.take(0)?;
            match members.into_iter().next() {
                Some(m) => Ok(Some(db_to_member(m)?)),
                None => Ok(None),
            }
        })
        .await
    }

    /// Get a member by their record ID (full row — secrets included for server-side ops).
    /// Prefer [`Self::get_member_safe`] for HTTP responses.
    pub async fn get_member(&self, id: &str) -> DbResult<Option<Member>> {
        with_timeout(async {
            let db_member: Option<DbMember> = self.client.select(("member", id)).await?;
            match db_member {
                Some(m) => Ok(Some(db_to_member(m)?)),
                None => Ok(None),
            }
        })
        .await
    }

    /// Public/profile path: never loads `nostr_secret_key_encrypted`.
    pub async fn get_member_safe(&self, id: &str) -> DbResult<Option<Member>> {
        with_timeout(async {
            let q = format!(
                "SELECT {MEMBER_SAFE_COLS} FROM $rid LIMIT 1"
            );
            let mut result = self
                .client
                .query(&q)
                .bind(("rid", RecordId::new("member", id)))
                .await?;
            let members: Vec<DbMember> = result.take(0)?;
            match members.into_iter().next() {
                Some(m) => Ok(Some(db_to_member(m)?)),
                None => Ok(None),
            }
        })
        .await
    }

    /// Auth extractor path: safe projection (no encrypted Nostr secret) + suspension check
    /// under a **single** query timeout.
    pub async fn get_member_auth_by_user(
        &self,
        user_id: &str,
    ) -> DbResult<Option<(Member, bool)>> {
        with_timeout(async {
            let q = format!(
                "SELECT {MEMBER_SAFE_COLS} FROM member WHERE user_id = $uid LIMIT 1"
            );
            let mut result = self
                .client
                .query(&q)
                .bind(("uid", user_id.to_string()))
                .await?;
            let members: Vec<DbMember> = result.take(0)?;
            let Some(raw) = members.into_iter().next() else {
                return Ok(None);
            };
            let member = db_to_member(raw)?;

            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }
            let mut sus = self
                .client
                .query(
                    "SELECT count() FROM moderation_action WHERE member_id = $mid \
                     AND is_active = true AND action_type IN ['suspension', 'ban'] \
                     AND (expires_at IS NONE OR expires_at > time::now()) GROUP ALL",
                )
                .bind(("mid", member.id.clone()))
                .await?;
            let counts: Vec<CountResult> = sus.take(0)?;
            let suspended = counts.first().map(|c| c.count > 0).unwrap_or(false);
            Ok(Some((member, suspended)))
        })
        .await
    }

    /// Load server-managed Nostr encrypted secrets for the DM subscriber index.
    /// Only returns rows that have both a pubkey and an encrypted secret.
    pub async fn list_server_managed_nostr_secrets(
        &self,
    ) -> DbResult<Vec<(String, EncryptedBlob)>> {
        with_timeout(async {
            #[derive(Debug, Deserialize, SurrealValue)]
            struct Row {
                nostr_pubkey: Option<String>,
                #[serde(default)]
                #[surreal(default)]
                nostr_secret_key_encrypted: Option<serde_json::Value>,
            }
            let mut result = self
                .client
                .query(
                    "SELECT nostr_pubkey, nostr_secret_key_encrypted FROM member \
                     WHERE is_active = true AND nostr_key_mode = 'server_managed' \
                     AND nostr_pubkey != NONE AND nostr_secret_key_encrypted != NONE",
                )
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            let mut out = Vec::new();
            for row in rows {
                let (Some(pk), Some(val)) = (row.nostr_pubkey, row.nostr_secret_key_encrypted)
                else {
                    continue;
                };
                if let Ok(blob) = serde_json::from_value::<EncryptedBlob>(val) {
                    out.push((pk, blob));
                }
            }
            Ok(out)
        })
        .await
    }

    /// Update a member's profile fields via field-level SET (no full-document RMW).
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
            // Ensure row exists first for clear NotFound errors.
            let existing: Option<DbMember> = self.client.select(("member", id)).await?;
            if existing.is_none() {
                return Err(crate::DbError::NotFound(format!("Member {id} not found")));
            }

            let rid = RecordId::new("member", id);
            let mut sets = Vec::new();
            if display_name.is_some() {
                sets.push("display_name = $display_name");
            }
            if bio.is_some() {
                sets.push("bio = $bio");
            }
            if avatar_url.is_some() {
                sets.push("avatar_url = $avatar_url");
            }
            if timezone.is_some() {
                sets.push("timezone = $timezone");
            }
            if pronouns.is_some() {
                sets.push("pronouns = $pronouns");
            }
            if availability_status.is_some() {
                sets.push("availability_status = $availability_status");
            }
            if is_active.is_some() {
                sets.push("is_active = $is_active");
            }
            if let Some(new_nostr) = nostr_pubkey {
                match new_nostr {
                    Some(_) => {
                        sets.push("nostr_pubkey = $nostr_pubkey");
                        sets.push("nostr_key_mode = $nostr_key_mode");
                        sets.push("nostr_secret_key_encrypted = NONE");
                    }
                    None => {
                        sets.push("nostr_pubkey = NONE");
                        sets.push("nostr_key_mode = NONE");
                        sets.push("nostr_secret_key_encrypted = NONE");
                    }
                }
            }

            if sets.is_empty() {
                return self
                    .get_member(id)
                    .await?
                    .ok_or_else(|| crate::DbError::NotFound(format!("Member {id} not found")));
            }

            let q = format!("UPDATE $rid SET {} RETURN AFTER", sets.join(", "));
            let mut q = self.client.query(&q).bind(("rid", rid));

            if let Some(name) = display_name {
                q = q.bind(("display_name", name.to_string()));
            }
            if let Some(new_bio) = bio {
                q = q.bind(("bio", new_bio.map(|s| s.to_string())));
            }
            if let Some(new_avatar) = avatar_url {
                q = q.bind(("avatar_url", new_avatar.map(|s| s.to_string())));
            }
            if let Some(new_tz) = timezone {
                q = q.bind(("timezone", new_tz.map(|s| s.to_string())));
            }
            if let Some(new_pronouns) = pronouns {
                q = q.bind(("pronouns", new_pronouns.map(|s| s.to_string())));
            }
            if let Some(new_status) = availability_status {
                q = q.bind(("availability_status", new_status.map(|s| s.to_string())));
            }
            if let Some(active) = is_active {
                q = q.bind(("is_active", active));
            }
            if let Some(Some(pubkey)) = nostr_pubkey {
                q = q
                    .bind(("nostr_pubkey", pubkey.to_string()))
                    .bind(("nostr_key_mode", NostrKeyMode::External.to_string()));
            }

            let mut result = q.await?;
            let updated: Option<DbMember> = result.take(0)?;
            db_to_member(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Member {id} not found after update"))
            })?)
        })
        .await
    }

    /// List members who have a Nostr pubkey (NIP-05). Projection omits secrets.
    pub async fn list_nostr_identities(&self) -> DbResult<Vec<Member>> {
        with_timeout(async {
            let q = format!(
                "SELECT {MEMBER_SAFE_COLS} FROM member \
                 WHERE is_active = true AND nostr_pubkey != NONE LIMIT 2000"
            );
            let mut result = self.client.query(&q).await?;
            let members: Vec<DbMember> = result.take(0)?;
            members.into_iter().map(db_to_member).collect::<DbResult<Vec<_>>>()
        })
        .await
    }

    /// Update a member's Nostr keypair fields (key mode + encrypted secret key).
    pub async fn update_member_nostr_keys(
        &self,
        id: &str,
        pubkey: Option<&str>,
        key_mode: Option<&str>,
        encrypted_secret: Option<&EncryptedBlob>,
    ) -> DbResult<Member> {
        with_timeout(async {
            let existing: Option<DbMember> = self.client.select(("member", id)).await?;
            if existing.is_none() {
                return Err(crate::DbError::NotFound(format!("Member {id} not found")));
            }

            let rid = RecordId::new("member", id);
            let enc = encrypted_secret.and_then(|blob| serde_json::to_value(blob).ok());
            let mut result = self
                .client
                .query(
                    "UPDATE $rid SET \
                        nostr_pubkey = $pubkey, \
                        nostr_key_mode = $key_mode, \
                        nostr_secret_key_encrypted = $enc \
                     RETURN AFTER",
                )
                .bind(("rid", rid))
                .bind(("pubkey", pubkey.map(|s| s.to_string())))
                .bind(("key_mode", key_mode.map(|s| s.to_string())))
                .bind(("enc", enc))
                .await?;
            let updated: Option<DbMember> = result.take(0)?;
            db_to_member(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Member {id} not found after update"))
            })?)
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
            match members.into_iter().next() {
                Some(m) => Ok(Some(db_to_member(m)?)),
                None => Ok(None),
            }
        })
        .await
    }

    /// Change a member's org role via field-level SET.
    pub async fn change_member_role(&self, id: &str, new_role: OrgRole) -> DbResult<Member> {
        with_timeout(async {
            let rid = RecordId::new("member", id);
            let mut result = self
                .client
                .query("UPDATE $rid SET org_role = $role RETURN AFTER")
                .bind(("rid", rid))
                .bind(("role", new_role.to_string()))
                .await?;
            let updated: Option<DbMember> = result.take(0)?;
            db_to_member(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Member {id} not found"))
            })?)
        })
        .await
    }

    /// Count currently active admin members (is_active = true, org_role = admin).
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
    /// Loads only admin ids + blocked member_ids (not full member documents).
    pub async fn count_actionable_admins(&self) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct IdOnly {
                id: Option<RecordId>,
            }
            let mut admins_result = self
                .client
                .query(
                    "SELECT id FROM member WHERE is_active = true AND org_role = 'admin'",
                )
                .await?;
            let admins: Vec<IdOnly> = admins_result.take(0)?;
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
                    "SELECT member_id FROM moderation_action WHERE is_active = true \
                     AND action_type IN ['suspension', 'ban'] \
                     AND (expires_at IS NONE OR expires_at > time::now())",
                )
                .await?;
            let blocked: Vec<BlockedMid> = blocked_result.take(0)?;
            let blocked_ids: std::collections::HashSet<String> =
                blocked.into_iter().map(|b| b.member_id).collect();

            let count = admins
                .into_iter()
                .filter_map(|a| a.id.map(|r| crate::record_id_key_to_string(r.key)))
                .filter(|id| !blocked_ids.contains(id))
                .count() as u64;
            Ok(count)
        })
        .await
    }

    /// Fail with [`crate::DbError::Conflict`] if no actionable admin remains.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use scuffed_auth::crypto::CryptoService;
    use std::sync::Arc;

    async fn test_db_with_crypto() -> Database {
        let key = CryptoService::generate_key();
        let crypto = CryptoService::new(&key, 1).unwrap();
        let mut db = Database::connect_memory().await.unwrap();
        db.crypto = Some(Arc::new(crypto));
        run_migrations(&db.client).await.unwrap();
        db
    }

    #[tokio::test]
    async fn list_paginated_decodes_members_with_server_managed_secrets() {
        let db = test_db_with_crypto().await;
        let m = db
            .create_member("user-1", "Alice", OrgRole::Member)
            .await
            .unwrap();
        assert!(m.nostr_secret_key_encrypted.is_some());
        assert!(m.nostr_pubkey.is_some());

        // Projection omits the secret column — must still decode with #[surreal(default)].
        let listed = db.list_members_paginated(10, 0).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].display_name, "Alice");
        assert!(listed[0].nostr_pubkey.is_some());
        assert!(
            listed[0].nostr_secret_key_encrypted.is_none(),
            "list path must not load secrets"
        );

        // Auth path also omits secrets.
        let (auth_m, sus) = db
            .get_member_auth_by_user("user-1")
            .await
            .unwrap()
            .expect("member");
        assert!(!sus);
        assert!(auth_m.nostr_secret_key_encrypted.is_none());

        // Full get still has the secret for signing.
        let full = db.get_member(&m.id).await.unwrap().unwrap();
        assert!(full.nostr_secret_key_encrypted.is_some());
    }
}
