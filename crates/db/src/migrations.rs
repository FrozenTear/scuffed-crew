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
        DEFINE FIELD avatar_url ON member TYPE option<string>;
        DEFINE FIELD timezone ON member TYPE option<string>;
        DEFINE FIELD pronouns ON member TYPE option<string>;
        DEFINE FIELD availability_status ON member TYPE option<string>;
        DEFINE FIELD is_active ON member TYPE bool DEFAULT true;

        DEFINE INDEX member_user_idx ON member COLUMNS user_id UNIQUE;

        -- ================================================
        -- Games (titles that teams can play)
        -- ================================================
        DEFINE TABLE game SCHEMAFULL;
        DEFINE FIELD name ON game TYPE string;
        DEFINE FIELD abbreviation ON game TYPE option<string>;
        DEFINE FIELD is_active ON game TYPE bool DEFAULT true;
        DEFINE FIELD created_at ON game TYPE datetime DEFAULT time::now();

        DEFINE INDEX game_name_idx ON game COLUMNS name UNIQUE;

        -- ================================================
        -- Teams (game squads)
        -- ================================================
        DEFINE TABLE team SCHEMAFULL;
        DEFINE FIELD name ON team TYPE string;
        DEFINE FIELD game_id ON team TYPE string;
        DEFINE FIELD color ON team TYPE option<string>;
        DEFINE FIELD division ON team TYPE option<string>;
        DEFINE FIELD lore_quote ON team TYPE option<string>;
        DEFINE FIELD logo_url ON team TYPE option<string>;
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
        DEFINE FIELD timezone ON event TYPE string DEFAULT 'Europe/Berlin';
        DEFINE FIELD duration_minutes ON event TYPE int DEFAULT 120;
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
        DEFINE FIELD trial_started_at ON application TYPE option<datetime>;
        DEFINE FIELD trial_ends_at ON application TYPE option<datetime>;
        DEFINE FIELD mentor_id ON application TYPE option<string>;
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
        DEFINE FIELD match_type ON match_result TYPE string DEFAULT 'scrim'
            ASSERT $value IN ['scrim', 'official', 'tournament'];
        DEFINE FIELD recorded_by ON match_result TYPE string;
        DEFINE FIELD notes ON match_result TYPE option<string>;

        -- ================================================
        -- Announcements (news posts)
        -- ================================================
        DEFINE TABLE announcement SCHEMAFULL;
        DEFINE FIELD title ON announcement TYPE string;
        DEFINE FIELD content ON announcement TYPE string;
        DEFINE FIELD author_id ON announcement TYPE string;
        DEFINE FIELD pinned ON announcement TYPE bool DEFAULT false;
        DEFINE FIELD is_active ON announcement TYPE bool DEFAULT true;
        DEFINE FIELD created_at ON announcement TYPE datetime DEFAULT time::now();
        DEFINE FIELD updated_at ON announcement TYPE datetime DEFAULT time::now();

        -- ================================================
        -- Audit Log (immutable admin action history)
        -- ================================================
        DEFINE TABLE audit_log SCHEMAFULL;
        DEFINE FIELD actor_id ON audit_log TYPE string;
        DEFINE FIELD action ON audit_log TYPE string;
        DEFINE FIELD target_type ON audit_log TYPE string;
        DEFINE FIELD target_id ON audit_log TYPE string;
        DEFINE FIELD details ON audit_log TYPE option<string>;
        DEFINE FIELD created_at ON audit_log TYPE datetime DEFAULT time::now();

        DEFINE INDEX audit_log_created_idx ON audit_log COLUMNS created_at;
        DEFINE INDEX audit_log_actor_idx ON audit_log COLUMNS actor_id;
        DEFINE INDEX audit_log_target_idx ON audit_log COLUMNS target_type, target_id;

        -- ================================================
        -- Moderation Actions (warnings, suspensions, bans)
        -- ================================================
        DEFINE TABLE moderation_action SCHEMAFULL;
        DEFINE FIELD member_id ON moderation_action TYPE string;
        DEFINE FIELD action_type ON moderation_action TYPE string
            ASSERT $value IN ['note', 'warning', 'suspension', 'ban'];
        DEFINE FIELD reason ON moderation_action TYPE string;
        DEFINE FIELD issued_by ON moderation_action TYPE string;
        DEFINE FIELD expires_at ON moderation_action TYPE option<datetime>;
        DEFINE FIELD is_active ON moderation_action TYPE bool DEFAULT true;
        DEFINE FIELD created_at ON moderation_action TYPE datetime DEFAULT time::now();

        DEFINE INDEX mod_member_idx ON moderation_action COLUMNS member_id;
        DEFINE INDEX mod_active_idx ON moderation_action COLUMNS member_id, action_type, is_active;

        -- ================================================
        -- Site Settings (single-document, admin-configurable)
        -- ================================================
        DEFINE TABLE site_settings SCHEMAFULL;
        DEFINE FIELD org_name ON site_settings TYPE string DEFAULT 'The Scuffed Crew';
        DEFINE FIELD site_description ON site_settings TYPE string DEFAULT 'EMEA Gaming Organization';
        DEFINE FIELD recruitment_open ON site_settings TYPE bool DEFAULT true;
        DEFINE FIELD recruitment_message ON site_settings TYPE string DEFAULT 'We are currently recruiting! Apply now to join the crew.';
        DEFINE FIELD min_age ON site_settings TYPE int DEFAULT 16;
        DEFINE FIELD updated_at ON site_settings TYPE datetime DEFAULT time::now();

        -- ================================================
        -- Game Accounts (linked gaming platform accounts)
        -- ================================================
        DEFINE TABLE game_account SCHEMAFULL;
        DEFINE FIELD member_id ON game_account TYPE string;
        DEFINE FIELD game_id ON game_account TYPE string;
        DEFINE FIELD account_name ON game_account TYPE string;
        DEFINE FIELD account_id ON game_account TYPE option<string>;
        DEFINE FIELD created_at ON game_account TYPE datetime DEFAULT time::now();

        DEFINE INDEX game_account_unique_idx ON game_account
            COLUMNS member_id, game_id UNIQUE;

        -- ================================================
        -- Event RSVPs
        -- ================================================
        DEFINE TABLE event_rsvp SCHEMAFULL;
        DEFINE FIELD member_id ON event_rsvp TYPE string;
        DEFINE FIELD event_id ON event_rsvp TYPE string;
        DEFINE FIELD status ON event_rsvp TYPE string
            ASSERT $value IN ['yes', 'maybe', 'no'];
        DEFINE FIELD responded_at ON event_rsvp TYPE datetime DEFAULT time::now();

        DEFINE INDEX event_rsvp_unique_idx ON event_rsvp
            COLUMNS event_id, member_id UNIQUE;

        -- ================================================
        -- Event Attendance (officer-marked presence tracking)
        -- ================================================
        DEFINE TABLE event_attendance SCHEMAFULL;
        DEFINE FIELD member_id ON event_attendance TYPE string;
        DEFINE FIELD event_id ON event_attendance TYPE string;
        DEFINE FIELD occurrence_date ON event_attendance TYPE datetime;
        DEFINE FIELD status ON event_attendance TYPE string
            ASSERT $value IN ['attended', 'no_show', 'excused'];
        DEFINE FIELD marked_by ON event_attendance TYPE string;
        DEFINE FIELD marked_at ON event_attendance TYPE datetime DEFAULT time::now();

        DEFINE INDEX event_attendance_unique_idx ON event_attendance
            COLUMNS member_id, event_id, occurrence_date UNIQUE;

        -- ================================================
        -- Tournaments
        -- ================================================
        DEFINE TABLE tournament SCHEMAFULL;
        DEFINE FIELD name ON tournament TYPE string;
        DEFINE FIELD game_id ON tournament TYPE option<string>;
        DEFINE FIELD format ON tournament TYPE string
            ASSERT $value IN ['single_elim', 'double_elim', 'round_robin', 'swiss'];
        DEFINE FIELD status ON tournament TYPE string DEFAULT 'draft'
            ASSERT $value IN ['draft', 'registration', 'in_progress', 'completed', 'archived'];
        DEFINE FIELD max_teams ON tournament TYPE option<int>;
        DEFINE FIELD best_of ON tournament TYPE int DEFAULT 1;
        DEFINE FIELD swiss_rounds ON tournament TYPE option<int>;
        DEFINE FIELD is_external ON tournament TYPE bool DEFAULT false;
        DEFINE FIELD is_open ON tournament TYPE bool DEFAULT false;
        DEFINE FIELD external_url ON tournament TYPE option<string>;
        DEFINE FIELD rules ON tournament TYPE option<string>;
        DEFINE FIELD description ON tournament TYPE option<string>;
        DEFINE FIELD starts_at ON tournament TYPE option<datetime>;
        DEFINE FIELD ends_at ON tournament TYPE option<datetime>;
        DEFINE FIELD created_by ON tournament TYPE string;
        DEFINE FIELD created_at ON tournament TYPE datetime DEFAULT time::now();
        DEFINE FIELD updated_at ON tournament TYPE datetime DEFAULT time::now();

        DEFINE INDEX tournament_status_idx ON tournament COLUMNS status;

        -- ================================================
        -- Tournament Participants
        -- ================================================
        DEFINE TABLE tournament_participant SCHEMAFULL;
        DEFINE FIELD tournament_id ON tournament_participant TYPE string;
        DEFINE FIELD team_id ON tournament_participant TYPE option<string>;
        DEFINE FIELD external_name ON tournament_participant TYPE option<string>;
        DEFINE FIELD seed ON tournament_participant TYPE option<int>;
        DEFINE FIELD group_label ON tournament_participant TYPE option<string>;
        DEFINE FIELD status ON tournament_participant TYPE string DEFAULT 'registered'
            ASSERT $value IN ['registered', 'checked_in', 'active', 'eliminated', 'withdrawn', 'disqualified'];
        DEFINE FIELD created_at ON tournament_participant TYPE datetime DEFAULT time::now();

        DEFINE INDEX tp_tournament_idx ON tournament_participant COLUMNS tournament_id;
        DEFINE INDEX tp_tournament_team_idx ON tournament_participant
            COLUMNS tournament_id, team_id UNIQUE;

        -- ================================================
        -- Tournament Rounds
        -- ================================================
        DEFINE TABLE tournament_round SCHEMAFULL;
        DEFINE FIELD tournament_id ON tournament_round TYPE string;
        DEFINE FIELD round_number ON tournament_round TYPE int;
        DEFINE FIELD stage ON tournament_round TYPE string DEFAULT 'main'
            ASSERT $value IN ['main', 'winners', 'losers', 'grand_final', 'group'];
        DEFINE FIELD status ON tournament_round TYPE string DEFAULT 'pending'
            ASSERT $value IN ['pending', 'in_progress', 'completed'];
        DEFINE FIELD created_at ON tournament_round TYPE datetime DEFAULT time::now();

        DEFINE INDEX tr_tournament_idx ON tournament_round COLUMNS tournament_id;

        -- ================================================
        -- Tournament Matches
        -- ================================================
        DEFINE TABLE tournament_match SCHEMAFULL;
        DEFINE FIELD tournament_id ON tournament_match TYPE string;
        DEFINE FIELD round_id ON tournament_match TYPE string;
        DEFINE FIELD bracket_position ON tournament_match TYPE int DEFAULT 0;
        DEFINE FIELD participant_a_id ON tournament_match TYPE option<string>;
        DEFINE FIELD participant_b_id ON tournament_match TYPE option<string>;
        DEFINE FIELD score_a ON tournament_match TYPE option<int>;
        DEFINE FIELD score_b ON tournament_match TYPE option<int>;
        DEFINE FIELD winner_id ON tournament_match TYPE option<string>;
        DEFINE FIELD status ON tournament_match TYPE string DEFAULT 'pending'
            ASSERT $value IN ['pending', 'scheduled', 'in_progress', 'completed', 'bye'];
        DEFINE FIELD scheduled_at ON tournament_match TYPE option<datetime>;
        DEFINE FIELD completed_at ON tournament_match TYPE option<datetime>;
        DEFINE FIELD match_result_id ON tournament_match TYPE option<string>;
        DEFINE FIELD next_match_id ON tournament_match TYPE option<string>;
        DEFINE FIELD next_match_slot ON tournament_match TYPE option<string>;
        DEFINE FIELD loser_next_match_id ON tournament_match TYPE option<string>;
        DEFINE FIELD loser_next_match_slot ON tournament_match TYPE option<string>;
        DEFINE FIELD notes ON tournament_match TYPE option<string>;
        DEFINE FIELD replay_codes ON tournament_match TYPE array DEFAULT [];
        DEFINE FIELD replay_codes.* ON tournament_match TYPE string;

        DEFINE INDEX tm_tournament_idx ON tournament_match COLUMNS tournament_id;
        DEFINE INDEX tm_round_idx ON tournament_match COLUMNS round_id;
    "#,
        )
        .await?
        .check()?;

    tracing::info!("Database migrations complete");
    Ok(())
}
