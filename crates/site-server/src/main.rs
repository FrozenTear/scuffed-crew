use std::sync::Arc;

use scuffed_auth::SessionConfig;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_site_server::{create_router, state::{AppState, OAuthConfig}};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let oauth_config = OAuthConfig::from_env();

    // Connect to SurrealDB (remote or in-memory fallback)
    let db = match std::env::var("SURREALDB_URL") {
        Ok(url) => {
            let user =
                std::env::var("SURREALDB_USER").unwrap_or_else(|_| "root".to_string());
            let pass =
                std::env::var("SURREALDB_PASSWORD").unwrap_or_else(|_| "root".to_string());
            Database::connect(&url, &user, &pass)
                .await
                .expect("Failed to connect to SurrealDB")
        }
        Err(_) => {
            tracing::info!("No SURREALDB_URL set, using in-memory database");
            Database::connect_memory()
                .await
                .expect("Failed to create in-memory database")
        }
    };

    run_migrations(&db.client)
        .await
        .expect("Failed to run database migrations");

    let db = Arc::new(db);

    let state = AppState {
        db: db.clone(),
        session_config: SessionConfig::default(),
        oauth_config,
    };

    // Spawn hourly session cleanup task
    let cleanup_db = db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Err(e) = cleanup_db.cleanup_expired_sessions().await {
                tracing::error!("Session cleanup failed: {e}");
            }
        }
    });

    let app = create_router(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Scuffed Crew server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
