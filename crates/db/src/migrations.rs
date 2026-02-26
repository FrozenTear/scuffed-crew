use surrealdb::engine::any::Any;
use surrealdb::Surreal;

use crate::DbResult;

/// Run all schema migrations. Idempotent — safe to call on every startup.
pub async fn run_migrations(client: &Surreal<Any>) -> DbResult<()> {
    tracing::info!("Running database migrations...");

    client
        .query(
            r#"
        -- ================================================
        -- Users (OAuth accounts, shared with auth crate)
        -- ================================================
        DEFINE TABLE user SCHEMAFULL;
        DEFINE FIELD provider ON user TYPE string
            ASSERT $value IN ['discord', 'google', 'matrix'];
        DEFINE FIELD username ON user TYPE string;
        DEFINE FIELD avatar_url ON user TYPE option<string>;
        DEFINE FIELD provider_id ON user TYPE option<string>;
        DEFINE FIELD provider_id_hash ON user TYPE option<string>;
        DEFINE FIELD provider_id_encrypted ON user FLEXIBLE TYPE option<object>;
        DEFINE FIELD created_at ON user TYPE datetime DEFAULT time::now();

        DEFINE INDEX user_provider_idx ON user
            COLUMNS provider, provider_id UNIQUE;
        DEFINE INDEX user_provider_hash_idx ON user
            COLUMNS provider, provider_id_hash UNIQUE;

        -- ================================================
        -- Sessions (persistent, replaces DashMap)
        -- ================================================
        DEFINE TABLE session SCHEMAFULL;
        DEFINE FIELD user_id ON session TYPE string;
        DEFINE FIELD token ON session TYPE string;
        DEFINE FIELD expires_at ON session TYPE datetime;
        DEFINE FIELD created_at ON session TYPE datetime DEFAULT time::now();

        DEFINE INDEX session_token_idx ON session COLUMNS token UNIQUE;

        -- ================================================
        -- Members (org membership, extends user)
        -- ================================================
        DEFINE TABLE member SCHEMAFULL;
        DEFINE FIELD user_id ON member TYPE string;
        DEFINE FIELD org_role ON member TYPE string
            ASSERT $value IN ['admin', 'officer', 'member', 'recruit'];
        DEFINE FIELD display_name ON member TYPE string;
        DEFINE FIELD bio ON member TYPE option<string>;
        DEFINE FIELD joined_at ON member TYPE datetime DEFAULT time::now();
        DEFINE FIELD is_active ON member TYPE bool DEFAULT true;

        DEFINE INDEX member_user_idx ON member COLUMNS user_id UNIQUE;

        -- ================================================
        -- Teams (game squads)
        -- ================================================
        DEFINE TABLE team SCHEMAFULL;
        DEFINE FIELD name ON team TYPE string;
        DEFINE FIELD game ON team TYPE string;
        DEFINE FIELD color ON team TYPE option<string>;
        DEFINE FIELD division ON team TYPE option<string>;
        DEFINE FIELD lore_quote ON team TYPE option<string>;
        DEFINE FIELD is_active ON team TYPE bool DEFAULT true;
        DEFINE FIELD created_at ON team TYPE datetime DEFAULT time::now();

        DEFINE INDEX team_name_idx ON team COLUMNS name UNIQUE;

        -- ================================================
        -- Roster (graph relation: member -> plays_on -> team)
        -- ================================================
        DEFINE TABLE plays_on TYPE RELATION IN member OUT team SCHEMAFULL;
        DEFINE FIELD team_role ON plays_on TYPE string
            ASSERT $value IN ['captain', 'player', 'sub', 'coach'];
        DEFINE FIELD joined_at ON plays_on TYPE datetime DEFAULT time::now();
        DEFINE FIELD is_active ON plays_on TYPE bool DEFAULT true;

        -- ================================================
        -- Events (schedule)
        -- ================================================
        DEFINE TABLE event SCHEMAFULL;
        DEFINE FIELD title ON event TYPE string;
        DEFINE FIELD day_of_week ON event TYPE int
            ASSERT $value >= 0 AND $value <= 6;
        DEFINE FIELD time ON event TYPE string;
        DEFINE FIELD timezone ON event TYPE string DEFAULT 'CET';
        DEFINE FIELD is_recurring ON event TYPE bool DEFAULT true;
        DEFINE FIELD team_id ON event TYPE option<string>;
        DEFINE FIELD created_by ON event TYPE string;
        DEFINE FIELD is_active ON event TYPE bool DEFAULT true;

        -- ================================================
        -- Applications (recruitment pipeline)
        -- ================================================
        DEFINE TABLE application SCHEMAFULL;
        DEFINE FIELD user_id ON application TYPE string;
        DEFINE FIELD status ON application TYPE string DEFAULT 'pending'
            ASSERT $value IN ['pending', 'trial', 'accepted', 'rejected', 'withdrawn'];
        DEFINE FIELD preferred_games ON application TYPE array DEFAULT [];
        DEFINE FIELD preferred_games.* ON application TYPE string;
        DEFINE FIELD preferred_roles ON application TYPE array DEFAULT [];
        DEFINE FIELD preferred_roles.* ON application TYPE string;
        DEFINE FIELD message ON application TYPE option<string>;
        DEFINE FIELD reviewed_by ON application TYPE option<string>;
        DEFINE FIELD review_notes ON application TYPE option<string>;
        DEFINE FIELD created_at ON application TYPE datetime DEFAULT time::now();
        DEFINE FIELD updated_at ON application TYPE datetime DEFAULT time::now();

        -- ================================================
        -- Match results
        -- ================================================
        DEFINE TABLE match_result SCHEMAFULL;
        DEFINE FIELD team_id ON match_result TYPE string;
        DEFINE FIELD opponent ON match_result TYPE string;
        DEFINE FIELD score_us ON match_result TYPE int;
        DEFINE FIELD score_them ON match_result TYPE int;
        DEFINE FIELD map_name ON match_result TYPE option<string>;
        DEFINE FIELD game_mode ON match_result TYPE option<string>;
        DEFINE FIELD played_at ON match_result TYPE datetime;
        DEFINE FIELD recorded_by ON match_result TYPE string;
        DEFINE FIELD notes ON match_result TYPE option<string>;
    "#,
        )
        .await?
        .check()?;

    tracing::info!("Database migrations complete");
    Ok(())
}
