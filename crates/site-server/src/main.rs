use std::path::PathBuf;
use std::sync::Arc;

use scuffed_auth::SessionConfig;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_site_server::{
    create_router,
    notifications::Notifier,
    state::{AppState, OAuthConfig},
    uploads,
};
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

    // Init-only: root migrations + ensure EDITOR app user, then exit.
    // Use for separate migrate jobs; set SURREALDB_BOOTSTRAP=0 on long-lived app containers.
    if std::env::var("SURREALDB_MIGRATE_ONLY").ok().as_deref() == Some("1") {
        if std::env::var("SURREALDB_URL").is_err() {
            panic!("SURREALDB_MIGRATE_ONLY=1 requires SURREALDB_URL (remote DB)");
        }
        Database::bootstrap_from_env()
            .await
            .expect("SURREALDB_MIGRATE_ONLY: bootstrap failed");
        tracing::info!("SURREALDB_MIGRATE_ONLY complete — exiting");
        return;
    }

    // Connect to SurrealDB (remote or in-memory fallback).
    // Prefer SURREALDB_AUTH_MODE=scoped + non-root user in production.
    // Remote scoped: optional root bootstrap (unless SURREALDB_BOOTSTRAP=0), then EDITOR app user.
    let is_dev = std::env::var("SURREALDB_URL").is_err();
    let db = if is_dev {
        tracing::info!("No SURREALDB_URL set, using in-memory database");
        let db = Database::connect_memory()
            .await
            .expect("Failed to create in-memory database");
        run_migrations(&db.client)
            .await
            .expect("Failed to run database migrations");
        db
    } else {
        Database::connect_from_env()
            .await
            .expect("Failed to connect to SurrealDB")
    };
    if is_dev {
        use scuffed_auth::crypto::hash_session_token;
        let token_hash = hash_session_token(DEV_SESSION_TOKEN);

        db.client
            .query(
                r#"
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
            "#,
            )
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
                    abbreviation = 'OW2',
                    is_active = true,
                    created_at = time::now();

                -- Teams
                CREATE team:alpha SET
                    name = 'Alpha Squad',
                    game_id = 'overwatch2',
                    color = '#e74c3c',
                    division = 'Main',
                    lore_quote = NONE,
                    logo_url = NONE,
                    is_active = true,
                    created_at = time::now();

                CREATE team:bravo SET
                    name = 'Bravo Team',
                    game_id = 'overwatch2',
                    color = '#3498db',
                    division = 'Academy',
                    lore_quote = NONE,
                    logo_url = NONE,
                    is_active = true,
                    created_at = time::now();

                CREATE team:charlie SET
                    name = 'Charlie Company',
                    game_id = 'overwatch2',
                    color = NONE,
                    division = NONE,
                    lore_quote = NONE,
                    logo_url = NONE,
                    is_active = true,
                    created_at = time::now();

                CREATE team:delta SET
                    name = 'Delta Force',
                    game_id = 'overwatch2',
                    color = NONE,
                    division = NONE,
                    lore_quote = NONE,
                    logo_url = NONE,
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
            db.report_tournament_match(
                &s1.id,
                2,
                0,
                winner1,
                Some("Dominant performance"),
                vec!["ABC123".to_string(), "DEF456".to_string()],
            )
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
            db.report_tournament_match(
                &s2.id,
                2,
                1,
                winner2,
                Some("Close series"),
                vec![
                    "GHI789".to_string(),
                    "JKL012".to_string(),
                    "MNO345".to_string(),
                ],
            )
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

        // ── Round Robin tournament seed ──
        db.client
            .query(
                r#"
                CREATE team:echo SET
                    name = 'Echo Company',
                    game_id = 'overwatch2',
                    color = NONE,
                    division = NONE,
                    lore_quote = NONE,
                    logo_url = NONE,
                    is_active = true,
                    created_at = time::now();

                CREATE tournament:rr_demo SET
                    name = 'Scuffed Round Robin',
                    game_id = 'overwatch2',
                    format = 'round_robin',
                    status = 'registration',
                    max_teams = 8,
                    best_of = 3,
                    swiss_rounds = NONE,
                    is_external = false,
                    is_open = false,
                    external_url = NONE,
                    rules = 'Round robin format — everyone plays everyone.',
                    description = 'Round robin showcase with partial results.',
                    starts_at = time::now() + 14d,
                    ends_at = NONE,
                    created_by = 'devadmin',
                    created_at = time::now();

                CREATE tournament_participant:rr1 SET
                    tournament_id = 'rr_demo', team_id = 'alpha', external_name = NONE,
                    seed = 1, group_label = NONE, status = 'registered', created_at = time::now();
                CREATE tournament_participant:rr2 SET
                    tournament_id = 'rr_demo', team_id = 'bravo', external_name = NONE,
                    seed = 2, group_label = NONE, status = 'registered', created_at = time::now();
                CREATE tournament_participant:rr3 SET
                    tournament_id = 'rr_demo', team_id = 'charlie', external_name = NONE,
                    seed = 3, group_label = NONE, status = 'registered', created_at = time::now();
                CREATE tournament_participant:rr4 SET
                    tournament_id = 'rr_demo', team_id = 'echo', external_name = NONE,
                    seed = 4, group_label = NONE, status = 'registered', created_at = time::now();
            "#,
            )
            .await
            .expect("Failed to seed RR tournament");

        db.generate_round_robin_pairings("rr_demo")
            .await
            .expect("Failed to generate RR pairings");

        db.client
            .query("UPDATE tournament:rr_demo SET status = 'in_progress'")
            .await
            .expect("Failed to start RR tournament");

        // Report some RR matches with replay codes
        let rr_matches = db
            .list_tournament_matches("rr_demo")
            .await
            .expect("Failed to list RR matches");

        let rr_pending: Vec<_> = rr_matches
            .iter()
            .filter(|m| {
                m.participant_a_id.is_some()
                    && m.participant_b_id.is_some()
                    && m.status == scuffed_db::types::TournamentMatchStatus::Pending
            })
            .collect();

        // Report first 3 matches of the round robin
        for (i, m) in rr_pending.iter().take(3).enumerate() {
            let winner = if i % 2 == 0 {
                m.participant_a_id.as_ref().unwrap()
            } else {
                m.participant_b_id.as_ref().unwrap()
            };
            let codes = vec![
                format!("RR{}{}", (b'A' + i as u8) as char, "001"),
                format!("RR{}{}", (b'A' + i as u8) as char, "002"),
            ];
            db.report_tournament_match(&m.id, 2, 1, winner, None, codes)
                .await
                .unwrap_or_else(|_| panic!("Failed to report RR match {i}"));
        }

        tracing::info!("Round robin seeded: 4 teams, 3 matches reported");

        // ── Swiss tournament seed ──
        db.client
            .query(
                r#"
                CREATE tournament:swiss_demo SET
                    name = 'Scuffed Swiss Open',
                    game_id = 'overwatch2',
                    format = 'swiss',
                    status = 'registration',
                    max_teams = 8,
                    best_of = 3,
                    swiss_rounds = 3,
                    is_external = false,
                    is_open = false,
                    external_url = NONE,
                    rules = 'Swiss format — 3 rounds, paired by record.',
                    description = 'Swiss format showcase with one completed round.',
                    starts_at = time::now() + 21d,
                    ends_at = NONE,
                    created_by = 'devadmin',
                    created_at = time::now();

                CREATE tournament_participant:sw1 SET
                    tournament_id = 'swiss_demo', team_id = 'alpha', external_name = NONE,
                    seed = 1, group_label = NONE, status = 'registered', created_at = time::now();
                CREATE tournament_participant:sw2 SET
                    tournament_id = 'swiss_demo', team_id = 'bravo', external_name = NONE,
                    seed = 2, group_label = NONE, status = 'registered', created_at = time::now();
                CREATE tournament_participant:sw3 SET
                    tournament_id = 'swiss_demo', team_id = 'charlie', external_name = NONE,
                    seed = 3, group_label = NONE, status = 'registered', created_at = time::now();
                CREATE tournament_participant:sw4 SET
                    tournament_id = 'swiss_demo', team_id = 'delta', external_name = NONE,
                    seed = 4, group_label = NONE, status = 'registered', created_at = time::now();
            "#,
            )
            .await
            .expect("Failed to seed Swiss tournament");

        // Generate and report round 1
        db.client
            .query("UPDATE tournament:swiss_demo SET status = 'in_progress'")
            .await
            .expect("Failed to start Swiss tournament");

        db.generate_swiss_round("swiss_demo")
            .await
            .expect("Failed to generate Swiss round 1");

        let sw_matches = db
            .list_tournament_matches("swiss_demo")
            .await
            .expect("Failed to list Swiss matches");

        let sw_pending: Vec<_> = sw_matches
            .iter()
            .filter(|m| {
                m.participant_a_id.is_some()
                    && m.participant_b_id.is_some()
                    && m.status == scuffed_db::types::TournamentMatchStatus::Pending
            })
            .collect();

        for (i, m) in sw_pending.iter().enumerate() {
            let winner = m.participant_a_id.as_ref().unwrap();
            let codes = vec![
                format!("SW1{}{}", (b'A' + i as u8) as char, "01"),
                format!("SW1{}{}", (b'A' + i as u8) as char, "02"),
            ];
            db.report_tournament_match(&m.id, 2, 0, winner, None, codes)
                .await
                .unwrap_or_else(|_| panic!("Failed to report Swiss match {i}"));
        }

        // Generate round 2 (pairs by standings now)
        db.generate_swiss_round("swiss_demo")
            .await
            .expect("Failed to generate Swiss round 2");

        tracing::info!("Swiss seeded: 4 teams, round 1 complete, round 2 pending");

        tracing::info!("Visit /api/dev/login to set session cookie");
    }

    let db = Arc::new(db);

    let upload_dir =
        PathBuf::from(std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "data/uploads".to_string()));
    uploads::ensure_upload_dir(&upload_dir)
        .await
        .expect("Failed to create upload directory");

    let notifier = Notifier::from_env();
    if notifier.is_none() {
        tracing::info!("Notifications not configured (Matrix/Discord) — running without");
    }

    // Nostr challenge signing key: from env or deterministic dev fallback
    let nostr_challenge_key: [u8; 32] = match std::env::var("NOSTR_CHALLENGE_SECRET") {
        Ok(secret) if !secret.is_empty() => {
            let hash = blake3::hash(secret.as_bytes());
            *hash.as_bytes()
        }
        _ => {
            if is_dev {
                tracing::warn!(
                    "Using deterministic dev key for Nostr challenges — NOT for production"
                );
            }
            let hash = blake3::hash(b"scuffed-crew-dev-nostr-challenge-key");
            *hash.as_bytes()
        }
    };

    // Single shared CryptoService from Database (no second from_env() load).
    let crypto = db.crypto.clone();
    if crypto.is_none() {
        tracing::info!("ENCRYPTION_KEY not set — Nostr key encryption disabled");
    }

    let relay_url = std::env::var("NOSTR_RELAY_URL").ok();
    if let Some(ref url) = relay_url {
        tracing::info!("Nostr relay URL: {url}");
    } else {
        tracing::info!("NOSTR_RELAY_URL not set — kind 0 profile publishing disabled");
    }

    // Start the persistent NIP-44 DM relay subscriber (Phase 5 real-time delivery,
    // [THE-878]). Falls back silently when relay or encryption is not configured —
    // clients keep using `POST /api/nostr/dm/sync` for polling-based delivery.
    let dm_events =
        scuffed_site_server::dm_subscriber::start(db.clone(), crypto.clone(), relay_url.clone());

    let state = AppState {
        db: db.clone(),
        session_config: SessionConfig::default(),
        oauth_config,
        upload_dir,
        notifier,
        nostr_challenge_key,
        crypto,
        relay_url,
        dm_events,
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
    // ConnectInfo is required by the auth rate limiter's peer-IP fallback.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
