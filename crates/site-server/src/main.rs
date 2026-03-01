use std::path::PathBuf;
use std::sync::Arc;

use scuffed_auth::SessionConfig;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_site_server::{create_router, notifications::MatrixNotifier, state::{AppState, OAuthConfig}, uploads};
use tracing_subscriber::EnvFilter;

const DEV_SESSION_TOKEN: &str = "dev-session-token-do-not-use-in-production";

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

    // Seed dev data when using in-memory database
    let is_dev = std::env::var("SURREALDB_URL").is_err();
    if is_dev {
        use scuffed_auth::crypto::hash_session_token;
        let token_hash = hash_session_token(DEV_SESSION_TOKEN);

        db.client
            .query(r#"
                CREATE user:devadmin SET
                    provider = 'discord',
                    username = 'DevAdmin',
                    avatar_url = NONE,
                    provider_id = 'dev-user-id',
                    provider_id_hash = NONE,
                    provider_id_encrypted = NONE,
                    created_at = time::now();

                CREATE member:devmember SET
                    user_id = 'devadmin',
                    org_role = 'admin',
                    display_name = 'DevAdmin',
                    bio = NONE,
                    avatar_url = NONE,
                    timezone = NONE,
                    pronouns = NONE,
                    availability_status = NONE,
                    joined_at = time::now(),
                    is_active = true;

                CREATE session:devsession SET
                    user_id = 'devadmin',
                    token = $token_hash,
                    expires_at = time::now() + 365d,
                    created_at = time::now();
            "#)
            .bind(("token_hash", token_hash))
            .await
            .expect("Failed to seed dev data");

        tracing::info!("Dev data seeded: user=devadmin, role=admin");

        // Seed sample tournament data
        db.client
            .query(r#"
                -- Games
                CREATE game:overwatch2 SET
                    name = 'Overwatch 2',
                    slug = 'overwatch-2',
                    icon_url = NONE,
                    created_at = time::now();

                -- Teams
                CREATE team:alpha SET
                    name = 'Alpha Squad',
                    game_id = 'overwatch2',
                    description = 'Main competitive roster',
                    avatar_url = NONE,
                    is_active = true,
                    created_at = time::now();

                CREATE team:bravo SET
                    name = 'Bravo Team',
                    game_id = 'overwatch2',
                    description = 'Secondary roster',
                    avatar_url = NONE,
                    is_active = true,
                    created_at = time::now();

                CREATE team:charlie SET
                    name = 'Charlie Company',
                    game_id = 'overwatch2',
                    description = NONE,
                    avatar_url = NONE,
                    is_active = true,
                    created_at = time::now();

                CREATE team:delta SET
                    name = 'Delta Force',
                    game_id = 'overwatch2',
                    description = NONE,
                    avatar_url = NONE,
                    is_active = true,
                    created_at = time::now();

                -- Sample tournament (registration open, 4 teams)
                CREATE tournament:demo SET
                    name = 'Scuffed Cup #1',
                    game_id = 'overwatch2',
                    format = 'single_elim',
                    status = 'registration',
                    max_teams = 8,
                    best_of = 3,
                    swiss_rounds = NONE,
                    is_external = false,
                    is_open = false,
                    external_url = NONE,
                    rules = 'Standard competitive rules. No hero bans. Map pool: current ranked rotation.',
                    description = 'Internal single elimination tournament for all Scuffed Crew teams.',
                    starts_at = time::now() + 7d,
                    ends_at = NONE,
                    created_by = 'devadmin',
                    created_at = time::now();

                CREATE tournament_participant:p1 SET
                    tournament_id = 'demo',
                    team_id = 'alpha',
                    external_name = NONE,
                    seed = 1,
                    group_label = NONE,
                    status = 'registered',
                    created_at = time::now();

                CREATE tournament_participant:p2 SET
                    tournament_id = 'demo',
                    team_id = 'bravo',
                    external_name = NONE,
                    seed = 2,
                    group_label = NONE,
                    status = 'registered',
                    created_at = time::now();

                CREATE tournament_participant:p3 SET
                    tournament_id = 'demo',
                    team_id = 'charlie',
                    external_name = NONE,
                    seed = 3,
                    group_label = NONE,
                    status = 'registered',
                    created_at = time::now();

                CREATE tournament_participant:p4 SET
                    tournament_id = 'demo',
                    team_id = 'delta',
                    external_name = NONE,
                    seed = 4,
                    group_label = NONE,
                    status = 'registered',
                    created_at = time::now();
            "#)
            .await
            .expect("Failed to seed tournament data");

        tracing::info!("Tournament seed data created: Scuffed Cup #1 (4 teams, registration)");

        // Generate bracket and simulate some results
        db.generate_single_elim_bracket("demo")
            .await
            .expect("Failed to generate bracket");

        // Transition to in_progress
        db.client
            .query("UPDATE tournament:demo SET status = 'in_progress'")
            .await
            .expect("Failed to update tournament status");

        // Get matches to report results on the first round (semi-finals)
        let matches = db
            .list_tournament_matches("demo")
            .await
            .expect("Failed to list matches");

        // Find first-round matches (they have participants assigned)
        let semis: Vec<_> = matches
            .iter()
            .filter(|m| {
                m.participant_a_id.is_some()
                    && m.participant_b_id.is_some()
                    && m.status == scuffed_db::types::TournamentMatchStatus::Pending
            })
            .collect();

        if semis.len() >= 2 {
            // Semi 1: participant A wins (seed 1 beats seed 4)
            let s1 = &semis[0];
            let winner1 = s1.participant_a_id.as_ref().unwrap();
            db.report_tournament_match(&s1.id, 2, 0, winner1, Some("Dominant performance"), vec!["ABC123".to_string(), "DEF456".to_string()])
                .await
                .expect("Failed to report semi 1");
            // Advance winner to final
            if let (Some(next_id), Some(next_slot)) = (&s1.next_match_id, &s1.next_match_slot) {
                db.set_match_participant(next_id, next_slot, winner1)
                    .await
                    .expect("Failed to advance semi 1 winner");
            }

            // Semi 2: participant A wins (seed 2 beats seed 3)
            let s2 = &semis[1];
            let winner2 = s2.participant_a_id.as_ref().unwrap();
            db.report_tournament_match(&s2.id, 2, 1, winner2, Some("Close series"), vec!["GHI789".to_string(), "JKL012".to_string(), "MNO345".to_string()])
                .await
                .expect("Failed to report semi 2");
            // Advance winner to final
            if let (Some(next_id), Some(next_slot)) = (&s2.next_match_id, &s2.next_match_slot) {
                db.set_match_participant(next_id, next_slot, winner2)
                    .await
                    .expect("Failed to advance semi 2 winner");
            }

            tracing::info!("Bracket generated with semi-final results — final pending");
        }

        tracing::info!("Visit /api/dev/login to set session cookie");
    }

    let db = Arc::new(db);

    let upload_dir = PathBuf::from(
        std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "data/uploads".to_string()),
    );
    uploads::ensure_upload_dir(&upload_dir)
        .await
        .expect("Failed to create upload directory");

    let notifier = MatrixNotifier::from_env();
    if notifier.is_none() {
        tracing::info!("Matrix notifications not configured — running without");
    }

    let state = AppState {
        db: db.clone(),
        session_config: SessionConfig::default(),
        oauth_config,
        upload_dir,
        notifier,
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
