pub mod calendar;
pub mod extractors;
pub mod notifications;
pub mod routes;
pub mod seed;
pub mod state;
pub mod uploads;

use axum::{
    http::{header, HeaderValue, Method},
    routing::{delete, get, patch, post, put},
    Router,
};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use state::AppState;

/// Build the application router.
pub fn create_router(state: AppState) -> Router {
    let origins: Vec<HeaderValue> = state
        .oauth_config
        .allowed_origins
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::COOKIE])
        .allow_credentials(true);

    Router::new()
        // NIP-05 Nostr identity verification (must be before fallback)
        .route(
            "/.well-known/nostr.json",
            get(routes::nostr::nostr_json),
        )
        // Health check
        .route("/api/health", get(routes::health::health))
        // Dev login (sets session cookie for in-memory dev mode)
        .route("/api/dev/login", get(routes::dev::dev_login))
        // Auth routes
        .route("/api/auth/{provider}/login", get(routes::auth::login))
        .route(
            "/api/auth/{provider}/callback",
            get(routes::auth::callback),
        )
        .route("/api/auth/me", get(routes::auth::me))
        .route("/api/auth/logout", post(routes::auth::logout))
        // Member routes
        .route("/api/members", get(routes::members::list_members))
        .route(
            "/api/members/{id}",
            get(routes::members::get_member).put(routes::members::update_member),
        )
        .route(
            "/api/members/{id}/role",
            patch(routes::members::change_role),
        )
        // Member game accounts
        .route(
            "/api/members/{id}/game-accounts",
            get(routes::members::list_game_accounts)
                .put(routes::members::upsert_game_account),
        )
        .route(
            "/api/members/{member_id}/game-accounts/{id}",
            delete(routes::members::delete_game_account),
        )
        // Member attendance
        .route(
            "/api/members/{id}/attendance",
            get(routes::attendance::member_attendance),
        )
        .route(
            "/api/members/{id}/attendance/stats",
            get(routes::attendance::member_attendance_stats),
        )
        // Game routes
        .route(
            "/api/games",
            get(routes::games::list_games).post(routes::games::create_game),
        )
        .route(
            "/api/games/{id}",
            get(routes::games::get_game).put(routes::games::update_game),
        )
        // Team routes
        .route(
            "/api/teams",
            get(routes::teams::list_teams).post(routes::teams::create_team),
        )
        .route(
            "/api/teams/{id}",
            get(routes::teams::get_team).put(routes::teams::update_team),
        )
        // Roster routes
        .route(
            "/api/teams/{id}/roster",
            get(routes::roster::get_team_roster).post(routes::roster::add_to_roster),
        )
        .route(
            "/api/teams/{id}/roster/{member_id}",
            put(routes::roster::update_roster_role)
                .delete(routes::roster::remove_from_roster),
        )
        // Event routes
        .route(
            "/api/events",
            get(routes::events::list_events).post(routes::events::create_event),
        )
        .route(
            "/api/events/{id}",
            put(routes::events::update_event).delete(routes::events::delete_event),
        )
        // Event RSVPs
        .route(
            "/api/events/{id}/rsvp",
            post(routes::rsvps::rsvp_event),
        )
        .route(
            "/api/events/{id}/rsvps",
            get(routes::rsvps::get_event_rsvps),
        )
        .route(
            "/api/events/{id}/rsvp-summary",
            get(routes::rsvps::get_rsvp_summary),
        )
        // Event attendance
        .route(
            "/api/events/{id}/attendance",
            get(routes::attendance::get_event_attendance)
                .post(routes::attendance::batch_mark_attendance),
        )
        // Application routes
        .route(
            "/api/applications",
            get(routes::applications::list_applications)
                .post(routes::applications::submit_application),
        )
        .route(
            "/api/applications/mine",
            get(routes::applications::my_application),
        )
        .route(
            "/api/applications/expiring",
            get(routes::applications::expiring_trials),
        )
        .route(
            "/api/applications/{id}",
            patch(routes::applications::update_application),
        )
        // Match routes
        .route(
            "/api/teams/{id}/matches",
            get(routes::matches::list_team_matches),
        )
        .route(
            "/api/matches",
            post(routes::matches::record_match),
        )
        .route(
            "/api/matches/{id}",
            put(routes::matches::update_match),
        )
        // Personal stats routes
        .route(
            "/api/stats/upload",
            post(routes::stats::upload_stats),
        )
        .route(
            "/api/stats/me",
            get(routes::stats::my_stats),
        )
        .route(
            "/api/stats/me/matches",
            get(routes::stats::my_matches),
        )
        .route(
            "/api/stats/me/heroes",
            get(routes::stats::my_hero_stats),
        )
        .route(
            "/api/stats/me/maps",
            get(routes::stats::my_map_stats),
        )
        .route(
            "/api/stats/member/{id}",
            get(routes::stats::member_stats),
        )
        .route(
            "/api/stats/member/{id}/heroes",
            get(routes::stats::member_hero_stats),
        )
        .route(
            "/api/stats/member/{id}/maps",
            get(routes::stats::member_map_stats),
        )
        .route(
            "/api/stats/tokens",
            get(routes::stats::list_daemon_tokens).post(routes::stats::create_daemon_token),
        )
        .route(
            "/api/stats/tokens/{id}",
            delete(routes::stats::revoke_daemon_token),
        )
        // Calendar routes
        .route(
            "/api/calendar/all.ics",
            get(routes::calendar::all_events_ics),
        )
        .route(
            "/api/calendar/team/{id}",
            get(routes::calendar::team_events_ics),
        )
        // Audit log
        .route(
            "/api/audit-log",
            get(routes::audit_log::list_audit_log),
        )
        // Moderation routes
        .route(
            "/api/moderation",
            get(routes::moderation::list_moderation)
                .post(routes::moderation::create_moderation_action),
        )
        .route(
            "/api/moderation/{id}/lift",
            patch(routes::moderation::lift_moderation_action),
        )
        .route(
            "/api/members/{id}/moderation",
            get(routes::moderation::member_moderation),
        )
        // Announcement routes
        .route(
            "/api/announcements",
            get(routes::announcements::list_announcements)
                .post(routes::announcements::create_announcement),
        )
        .route(
            "/api/announcements/{id}",
            put(routes::announcements::update_announcement)
                .delete(routes::announcements::delete_announcement),
        )
        // Tournament routes
        .route(
            "/api/tournaments",
            get(routes::tournaments::list_tournaments)
                .post(routes::tournaments::create_tournament),
        )
        .route(
            "/api/tournaments/{id}",
            get(routes::tournaments::get_tournament)
                .put(routes::tournaments::update_tournament),
        )
        .route(
            "/api/tournaments/{id}/status",
            patch(routes::tournaments::transition_status),
        )
        .route(
            "/api/tournaments/{id}/bracket",
            get(routes::tournaments::get_bracket),
        )
        .route(
            "/api/tournaments/{id}/generate-bracket",
            post(routes::tournaments::generate_bracket),
        )
        .route(
            "/api/tournaments/{id}/participants",
            get(routes::tournaments::list_participants)
                .post(routes::tournaments::add_participant),
        )
        .route(
            "/api/tournaments/{id}/participants/{pid}",
            put(routes::tournaments::update_participant)
                .delete(routes::tournaments::remove_participant),
        )
        .route(
            "/api/tournaments/{id}/matches",
            get(routes::tournaments::list_matches),
        )
        .route(
            "/api/tournaments/{id}/matches/{mid}/report",
            patch(routes::tournaments::report_match),
        )
        .route(
            "/api/tournaments/{id}/standings",
            get(routes::tournaments::get_standings),
        )
        .route(
            "/api/tournaments/{id}/next-round",
            post(routes::tournaments::generate_next_round),
        )
        // Scrim routes
        .route(
            "/api/scrims",
            get(routes::scrims::list_scrims).post(routes::scrims::create_scrim),
        )
        .route(
            "/api/scrims/{id}",
            patch(routes::scrims::update_scrim_status),
        )
        // Article routes (blog)
        .route(
            "/api/articles",
            get(routes::articles::list_articles)
                .post(routes::articles::create_article),
        )
        .route(
            "/api/articles/{slug}",
            get(routes::articles::get_article)
                .put(routes::articles::update_article),
        )
        .route(
            "/api/articles/{slug}/publish",
            post(routes::articles::publish_article),
        )
        .route(
            "/api/articles/{slug}/delete",
            delete(routes::articles::delete_article),
        )
        // Wiki routes
        .route(
            "/api/wiki",
            get(routes::wiki::list_wiki_pages)
                .post(routes::wiki::create_wiki_page),
        )
        .route(
            "/api/wiki/{topic}",
            get(routes::wiki::get_wiki_page)
                .put(routes::wiki::update_wiki_page)
                .delete(routes::wiki::delete_wiki_page),
        )
        .route(
            "/api/wiki/{topic}/revisions",
            get(routes::wiki::list_wiki_revisions),
        )
        // Forum routes
        .route(
            "/api/forum/threads",
            get(routes::forum::list_threads)
                .post(routes::forum::create_thread),
        )
        .route(
            "/api/forum/threads/{id}",
            get(routes::forum::get_thread),
        )
        .route(
            "/api/forum/threads/{id}/replies",
            post(routes::forum::create_reply),
        )
        .route(
            "/api/forum/threads/{id}/pin",
            patch(routes::forum::pin_thread),
        )
        .route(
            "/api/forum/threads/{id}/lock",
            patch(routes::forum::lock_thread),
        )
        // Poll routes
        .route(
            "/api/polls",
            get(routes::polls::list_polls).post(routes::polls::create_poll),
        )
        .route(
            "/api/polls/{id}",
            get(routes::polls::get_poll).delete(routes::polls::deactivate_poll),
        )
        .route(
            "/api/polls/{id}/vote",
            post(routes::polls::vote_poll),
        )
        .route(
            "/api/polls/{id}/vote/{option_index}",
            delete(routes::polls::unvote_poll),
        )
        // Nostr identity verification (Phase 1.5)
        .route(
            "/api/nostr/challenge",
            post(routes::nostr::nostr_challenge),
        )
        .route(
            "/api/nostr/verify",
            post(routes::nostr::nostr_verify),
        )
        .route(
            "/api/nostr/identity",
            delete(routes::nostr::nostr_unlink),
        )
        .route(
            "/api/nostr/export-backup",
            post(routes::nostr::nostr_export_backup),
        )
        .route(
            "/api/nostr/import-key",
            post(routes::nostr::nostr_import_key),
        )
        .route(
            "/api/nostr/community",
            post(routes::nostr::nostr_community),
        )
        .route(
            "/api/nostr/react",
            post(routes::nostr::nostr_react),
        )
        .route(
            "/api/nostr/post",
            post(routes::nostr::nostr_post),
        )
        .route(
            "/api/nostr/feed",
            get(routes::nostr::nostr_feed),
        )
        .route(
            "/api/nostr/health",
            get(routes::nostr::nostr_health),
        )
        // Upload routes
        .route(
            "/api/upload/avatar",
            post(routes::uploads::upload_avatar),
        )
        .route(
            "/api/upload/image",
            post(routes::uploads::upload_image),
        )
        // Settings routes
        .route(
            "/api/settings",
            get(routes::settings::get_settings).put(routes::settings::update_settings),
        )
        // Public aggregate
        .route("/api/public/overview", get(routes::public::overview))
        .route(
            "/api/public/members",
            get(routes::public::public_members),
        )
        .route(
            "/api/public/members/{id}",
            get(routes::public::public_member_profile),
        )
        .route(
            "/api/public/teams/{id}",
            get(routes::public::public_team_detail),
        )
        // Serve uploaded files
        .nest_service("/uploads", ServeDir::new(state.upload_dir.clone()))
        // Static files from dist/, falling back to index.html for SPA routing (Dioxus handles all routes)
        .fallback_service(ServeDir::new("dist").fallback(ServeFile::new("dist/index.html")))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
