use serde::{Deserialize, Serialize};
use surrealdb_types::RecordId;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

use scuffed_auth::crypto::{hash_provider_id, EncryptedBlob};
use scuffed_auth::{AuthProvider, User};

use crate::{with_timeout, Database, DbError, DbResult};

/// Internal DB representation of a user (handles encryption fields).
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbUser {
    #[surreal(default)]
    id: Option<RecordId>,
    provider: String,
    username: String,
    avatar_url: Option<String>,
    provider_id: Option<String>,
    provider_id_hash: Option<String>,
    provider_id_encrypted: Option<serde_json::Value>,
    created_at: SurrealDatetime,
}

impl Database {
    /// Upsert a user from OAuth data. Creates if new, updates username/avatar if changed.
    pub async fn upsert_user_from_oauth(
        &self,
        provider: AuthProvider,
        provider_id: String,
        username: String,
        avatar_url: Option<String>,
    ) -> DbResult<User> {
        with_timeout(async {
            if let Some(mut existing) =
                self.get_user_by_provider(provider, &provider_id).await?
            {
                if existing.username != username || existing.avatar_url != avatar_url {
                    existing.username = username;
                    existing.avatar_url = avatar_url;
                    return self.update_user(&existing).await;
                }
                return Ok(existing);
            }

            let user = User::new(provider, provider_id, username, avatar_url);
            self.create_user(&user).await
        })
        .await
    }

    /// Get a user by their internal ID.
    pub async fn get_user(&self, id: &str) -> DbResult<Option<User>> {
        with_timeout(async {
            let db_user: Option<DbUser> = self.client.select(("user", id)).await?;
            match db_user {
                Some(db) => Ok(Some(self.db_user_to_user(db)?)),
                None => Ok(None),
            }
        })
        .await
    }

    /// Look up a user by their OAuth provider and provider-specific ID.
    pub async fn get_user_by_provider(
        &self,
        provider: AuthProvider,
        provider_id: &str,
    ) -> DbResult<Option<User>> {
        with_timeout(async {
            let provider_str = provider.to_string();

            let db_user: Option<DbUser> = if self.crypto.is_some() {
                let id_hash = hash_provider_id(&provider_str, provider_id);
                let mut result = self
                    .client
                    .query(
                        "SELECT * FROM user WHERE provider = $provider AND provider_id_hash = $hash",
                    )
                    .bind(("provider", provider_str.clone()))
                    .bind(("hash", id_hash))
                    .await?;
                let users: Vec<DbUser> = result.take(0)?;
                users.into_iter().next()
            } else {
                let mut result = self
                    .client
                    .query(
                        "SELECT * FROM user WHERE provider = $provider AND provider_id = $pid",
                    )
                    .bind(("provider", provider_str.clone()))
                    .bind(("pid", provider_id.to_string()))
                    .await?;
                let users: Vec<DbUser> = result.take(0)?;
                users.into_iter().next()
            };

            match db_user {
                Some(db) => Ok(Some(self.db_user_to_user(db)?)),
                None => Ok(None),
            }
        })
        .await
    }

    async fn create_user(&self, user: &User) -> DbResult<User> {
        let provider_str = user.provider.to_string();

        let db_user = if let Some(ref crypto) = self.crypto {
            let id_hash = hash_provider_id(&provider_str, &user.provider_id);
            let id_encrypted = crypto.encrypt(&user.provider_id)?;
            DbUser {
                id: None,
                provider: provider_str,
                username: user.username.clone(),
                avatar_url: user.avatar_url.clone(),
                provider_id: None,
                provider_id_hash: Some(id_hash),
                provider_id_encrypted: Some(serde_json::to_value(id_encrypted).map_err(|e| DbError::Config(format!("Failed to serialize encrypted blob: {e}")))?),
                created_at: SurrealDatetime::from(user.created_at),
            }
        } else {
            DbUser {
                id: None,
                provider: provider_str,
                username: user.username.clone(),
                avatar_url: user.avatar_url.clone(),
                provider_id: Some(user.provider_id.clone()),
                provider_id_hash: None,
                provider_id_encrypted: None,
                created_at: SurrealDatetime::from(user.created_at),
            }
        };

        let _: Option<DbUser> = self
            .client
            .create(("user", user.id.as_str()))
            .content(db_user)
            .await?;

        Ok(user.clone())
    }

    async fn update_user(&self, user: &User) -> DbResult<User> {
        let provider_str = user.provider.to_string();

        let db_user = if let Some(ref crypto) = self.crypto {
            let id_hash = hash_provider_id(&provider_str, &user.provider_id);
            let id_encrypted = crypto.encrypt(&user.provider_id)?;
            DbUser {
                id: None,
                provider: provider_str,
                username: user.username.clone(),
                avatar_url: user.avatar_url.clone(),
                provider_id: None,
                provider_id_hash: Some(id_hash),
                provider_id_encrypted: Some(serde_json::to_value(id_encrypted).map_err(|e| DbError::Config(format!("Failed to serialize encrypted blob: {e}")))?),
                created_at: SurrealDatetime::from(user.created_at),
            }
        } else {
            DbUser {
                id: None,
                provider: provider_str,
                username: user.username.clone(),
                avatar_url: user.avatar_url.clone(),
                provider_id: Some(user.provider_id.clone()),
                provider_id_hash: None,
                provider_id_encrypted: None,
                created_at: SurrealDatetime::from(user.created_at),
            }
        };

        let _: Option<DbUser> = self
            .client
            .update(("user", user.id.as_str()))
            .content(db_user)
            .await?;

        Ok(user.clone())
    }

    /// Convert a DB user record to the public User type.
    fn db_user_to_user(&self, db: DbUser) -> DbResult<User> {
        let provider = match db.provider.as_str() {
            "discord" => AuthProvider::Discord,
            "google" => AuthProvider::Google,
            "matrix" => AuthProvider::Matrix,
            other => return Err(DbError::Config(format!("Unknown provider: {other}"))),
        };

        let provider_id = if let Some(ref encrypted_json) = db.provider_id_encrypted {
            let encrypted: EncryptedBlob = serde_json::from_value(encrypted_json.clone())
                .map_err(|e| DbError::Config(format!("Failed to deserialize encrypted blob: {e}")))?;
            if let Some(ref crypto) = self.crypto {
                crypto.decrypt(&encrypted)?
            } else {
                return Err(DbError::Config(
                    "Encrypted data present but no crypto service configured".into(),
                ));
            }
        } else if let Some(plaintext) = db.provider_id {
            plaintext
        } else {
            return Err(DbError::Config("No provider_id found on user record".into()));
        };

        let id = db
            .id
            .map(|r| crate::record_id_key_to_string(r.key))
            .unwrap_or_else(|| "unknown".to_string());

        Ok(User {
            id,
            provider,
            provider_id,
            username: db.username,
            avatar_url: db.avatar_url,
            created_at: db.created_at.into(),
        })
    }
}
