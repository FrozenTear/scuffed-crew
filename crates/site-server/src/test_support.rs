//! Shared helpers for route-handler tests.

use std::path::PathBuf;
use std::sync::Arc;

use scuffed_auth::SessionConfig;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;

use crate::state::{AppState, OAuthConfig};

pub(crate) async fn test_state() -> AppState {
    let db = Database::connect_memory()
        .await
        .expect("in-memory DB connect");
    run_migrations(&db.client).await.expect("migrations");
    AppState {
        db: Arc::new(db),
        session_config: SessionConfig::default(),
        oauth_config: OAuthConfig {
            discord_client_id: String::new(),
            discord_client_secret: String::new(),
            google_client_id: String::new(),
            google_client_secret: String::new(),
            redirect_base_url: "http://localhost:3000".into(),
            allowed_origins: vec!["http://localhost:3000".into()],
        },
        upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
        notifier: None,
        nostr_challenge_key: [0u8; 32],
        consumed_challenges: crate::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: crate::nostr_rate_limit::NostrRateLimiter::new(),
        crypto: None,
        relay_url: None,
        dm_events: None,
    }
}

/// Seed a minimal user row (the fields session/member queries expect).
pub(crate) async fn seed_user(state: &AppState, id: &str, username: &str) {
    // Test-only: `id` is always a fixed alphanumeric literal from a test, so
    // interpolating it into the record id is safe here (site-server has no
    // direct surrealdb dep for a RecordId bind).
    assert!(
        id.chars().all(|c| c.is_ascii_alphanumeric()),
        "test user ids must be alphanumeric"
    );
    state
        .db
        .client
        .query(format!(
            r#"CREATE user:{id} SET
                provider = 'discord',
                username = $username,
                avatar_url = NONE,
                provider_id = $pid,
                provider_id_hash = $pidh,
                provider_id_encrypted = NONE,
                created_at = time::now()"#
        ))
        .bind(("username", username.to_string()))
        .bind(("pid", format!("{id}-pid")))
        .bind(("pidh", format!("{id}-pidh")))
        .await
        .expect("seed user");
}
