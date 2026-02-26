pub mod extractors;
pub mod routes;
pub mod state;

use axum::{
    http::{header, HeaderValue, Method},
    routing::{get, patch, post, put},
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
        // Health check
        .route("/api/health", get(routes::health::health))
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
        // Public aggregate
        .route("/api/public/overview", get(routes::public::overview))
        // Admin SPA (must be before public site fallback)
        .nest_service(
            "/admin",
            ServeDir::new("dist/admin").fallback(ServeFile::new("dist/admin/index.html")),
        )
        // Static files from dist/, falling back to index.html for SPA routing
        .fallback_service(ServeDir::new("dist").fallback(ServeFile::new("dist/index.html")))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
