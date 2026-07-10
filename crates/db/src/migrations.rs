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
        DEFINE TABLE IF NOT EXISTS user SCHEMAFULL;
        DEFINE FIELD OVERWRITE provider ON user TYPE string
            ASSERT $value IN ['discord', 'google', 'matrix', 'local'];
        DEFINE FIELD OVERWRITE username ON user TYPE string;
        DEFINE FIELD OVERWRITE avatar_url ON user TYPE option<string>;
        DEFINE FIELD OVERWRITE provider_id ON user TYPE option<string>;
        DEFINE FIELD OVERWRITE provider_id_hash ON user TYPE option<string>;
        DEFINE FIELD OVERWRITE provider_id_encrypted ON user TYPE option<object> FLEXIBLE;
        DEFINE FIELD OVERWRITE password_hash ON user TYPE option<string>;
        DEFINE FIELD OVERWRITE created_at ON user TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS user_provider_idx ON user
            COLUMNS provider, provider_id UNIQUE;
        DEFINE INDEX IF NOT EXISTS user_provider_hash_idx ON user
            COLUMNS provider, provider_id_hash UNIQUE;
        -- Local usernames are unique via create_local_user check (do not UNIQUE
        -- username for all providers — OAuth usernames can collide).

        -- ================================================
        -- Sessions (persistent, replaces DashMap)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS session SCHEMAFULL;
        DEFINE FIELD OVERWRITE user_id ON session TYPE string;
        DEFINE FIELD OVERWRITE token ON session TYPE string;
        DEFINE FIELD OVERWRITE expires_at ON session TYPE datetime;
        DEFINE FIELD OVERWRITE created_at ON session TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS session_token_idx ON session COLUMNS token UNIQUE;

        -- ================================================
        -- Members (org membership, extends user)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS member SCHEMAFULL;
        DEFINE FIELD OVERWRITE user_id ON member TYPE string;
        DEFINE FIELD OVERWRITE org_role ON member TYPE string
            ASSERT $value IN ['admin', 'officer', 'member', 'recruit'];
        DEFINE FIELD OVERWRITE display_name ON member TYPE string;
        DEFINE FIELD OVERWRITE bio ON member TYPE option<string>;
        DEFINE FIELD OVERWRITE joined_at ON member TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE avatar_url ON member TYPE option<string>;
        DEFINE FIELD OVERWRITE timezone ON member TYPE option<string>;
        DEFINE FIELD OVERWRITE pronouns ON member TYPE option<string>;
        DEFINE FIELD OVERWRITE availability_status ON member TYPE option<string>;
        DEFINE FIELD OVERWRITE nostr_pubkey ON member TYPE option<string>;
        DEFINE FIELD OVERWRITE nostr_key_mode ON member TYPE option<string>
            ASSERT $value IN [NONE, 'server_managed', 'external'];
        DEFINE FIELD OVERWRITE nostr_secret_key_encrypted ON member TYPE option<object> FLEXIBLE;
        DEFINE FIELD OVERWRITE is_active ON member TYPE bool DEFAULT true;

        DEFINE INDEX IF NOT EXISTS member_user_idx ON member COLUMNS user_id UNIQUE;
        DEFINE INDEX IF NOT EXISTS member_nostr_pubkey_idx ON member COLUMNS nostr_pubkey UNIQUE;

        -- ================================================
        -- Games (titles that teams can play)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS game SCHEMAFULL;
        DEFINE FIELD OVERWRITE name ON game TYPE string;
        DEFINE FIELD OVERWRITE abbreviation ON game TYPE option<string>;
        DEFINE FIELD OVERWRITE is_active ON game TYPE bool DEFAULT true;
        DEFINE FIELD OVERWRITE created_at ON game TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS game_name_idx ON game COLUMNS name UNIQUE;

        -- ================================================
        -- Teams (game squads)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS team SCHEMAFULL;
        DEFINE FIELD OVERWRITE name ON team TYPE string;
        DEFINE FIELD OVERWRITE game_id ON team TYPE string;
        DEFINE FIELD OVERWRITE color ON team TYPE option<string>;
        DEFINE FIELD OVERWRITE division ON team TYPE option<string>;
        DEFINE FIELD OVERWRITE lore_quote ON team TYPE option<string>;
        DEFINE FIELD OVERWRITE logo_url ON team TYPE option<string>;
        DEFINE FIELD OVERWRITE is_active ON team TYPE bool DEFAULT true;
        DEFINE FIELD OVERWRITE created_at ON team TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS team_name_idx ON team COLUMNS name UNIQUE;

        -- ================================================
        -- Roster (graph relation: member -> plays_on -> team)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS plays_on TYPE RELATION IN member OUT team SCHEMAFULL;
        DEFINE FIELD OVERWRITE team_role ON plays_on TYPE string
            ASSERT $value IN ['captain', 'player', 'sub', 'coach'];
        DEFINE FIELD OVERWRITE joined_at ON plays_on TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE is_active ON plays_on TYPE bool DEFAULT true;

        -- ================================================
        -- Events (schedule)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS event SCHEMAFULL;
        DEFINE FIELD OVERWRITE title ON event TYPE string;
        DEFINE FIELD OVERWRITE day_of_week ON event TYPE int
            ASSERT $value >= 0 AND $value <= 6;
        DEFINE FIELD OVERWRITE time ON event TYPE string;
        DEFINE FIELD OVERWRITE timezone ON event TYPE string DEFAULT 'Europe/Berlin';
        DEFINE FIELD OVERWRITE duration_minutes ON event TYPE int DEFAULT 120;
        DEFINE FIELD OVERWRITE is_recurring ON event TYPE bool DEFAULT true;
        DEFINE FIELD OVERWRITE team_id ON event TYPE option<string>;
        DEFINE FIELD OVERWRITE created_by ON event TYPE string;
        DEFINE FIELD OVERWRITE is_active ON event TYPE bool DEFAULT true;

        -- ================================================
        -- Applications (recruitment pipeline)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS application SCHEMAFULL;
        DEFINE FIELD OVERWRITE user_id ON application TYPE string;
        DEFINE FIELD OVERWRITE status ON application TYPE string DEFAULT 'pending'
            ASSERT $value IN ['pending', 'trial', 'accepted', 'rejected', 'withdrawn'];
        DEFINE FIELD OVERWRITE preferred_games ON application TYPE array DEFAULT [];
        DEFINE FIELD OVERWRITE preferred_games.* ON application TYPE string;
        DEFINE FIELD OVERWRITE preferred_roles ON application TYPE array DEFAULT [];
        DEFINE FIELD OVERWRITE preferred_roles.* ON application TYPE string;
        DEFINE FIELD OVERWRITE message ON application TYPE option<string>;
        DEFINE FIELD OVERWRITE reviewed_by ON application TYPE option<string>;
        DEFINE FIELD OVERWRITE review_notes ON application TYPE option<string>;
        DEFINE FIELD OVERWRITE trial_started_at ON application TYPE option<datetime>;
        DEFINE FIELD OVERWRITE trial_ends_at ON application TYPE option<datetime>;
        DEFINE FIELD OVERWRITE mentor_id ON application TYPE option<string>;
        DEFINE FIELD OVERWRITE created_at ON application TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE updated_at ON application TYPE datetime DEFAULT time::now();

        -- ================================================
        -- Match results
        -- ================================================
        DEFINE TABLE IF NOT EXISTS match_result SCHEMAFULL;
        DEFINE FIELD OVERWRITE team_id ON match_result TYPE string;
        DEFINE FIELD OVERWRITE opponent ON match_result TYPE string;
        DEFINE FIELD OVERWRITE score_us ON match_result TYPE int;
        DEFINE FIELD OVERWRITE score_them ON match_result TYPE int;
        DEFINE FIELD OVERWRITE map_name ON match_result TYPE option<string>;
        DEFINE FIELD OVERWRITE game_mode ON match_result TYPE option<string>;
        DEFINE FIELD OVERWRITE played_at ON match_result TYPE datetime;
        DEFINE FIELD OVERWRITE match_type ON match_result TYPE string DEFAULT 'scrim'
            ASSERT $value IN ['scrim', 'official', 'tournament'];
        DEFINE FIELD OVERWRITE recorded_by ON match_result TYPE string;
        DEFINE FIELD OVERWRITE notes ON match_result TYPE option<string>;

        -- ================================================
        -- Announcements (news posts)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS announcement SCHEMAFULL;
        DEFINE FIELD OVERWRITE title ON announcement TYPE string;
        DEFINE FIELD OVERWRITE content ON announcement TYPE string;
        DEFINE FIELD OVERWRITE author_id ON announcement TYPE string;
        DEFINE FIELD OVERWRITE pinned ON announcement TYPE bool DEFAULT false;
        DEFINE FIELD OVERWRITE is_active ON announcement TYPE bool DEFAULT true;
        DEFINE FIELD OVERWRITE created_at ON announcement TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE updated_at ON announcement TYPE datetime DEFAULT time::now();

        -- ================================================
        -- Polls
        -- ================================================
        DEFINE TABLE IF NOT EXISTS poll SCHEMAFULL;
        DEFINE FIELD OVERWRITE title ON poll TYPE string;
        DEFINE FIELD OVERWRITE description ON poll TYPE option<string>;
        DEFINE FIELD OVERWRITE options ON poll TYPE array;
        DEFINE FIELD OVERWRITE options.* ON poll TYPE string;
        DEFINE FIELD OVERWRITE close_at ON poll TYPE option<datetime>;
        DEFINE FIELD OVERWRITE allow_multiple ON poll TYPE bool DEFAULT false;
        DEFINE FIELD OVERWRITE created_by ON poll TYPE string;
        DEFINE FIELD OVERWRITE is_active ON poll TYPE bool DEFAULT true;
        DEFINE FIELD OVERWRITE created_at ON poll TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE updated_at ON poll TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS poll_active_idx ON poll COLUMNS is_active, created_at;

        -- ================================================
        -- Poll Votes
        -- ================================================
        DEFINE TABLE IF NOT EXISTS poll_vote SCHEMAFULL;
        DEFINE FIELD OVERWRITE poll_id ON poll_vote TYPE string;
        DEFINE FIELD OVERWRITE member_id ON poll_vote TYPE string;
        DEFINE FIELD OVERWRITE option_index ON poll_vote TYPE int
            ASSERT $value >= 0;
        DEFINE FIELD OVERWRITE voted_at ON poll_vote TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS poll_vote_unique_idx ON poll_vote
            COLUMNS poll_id, member_id, option_index UNIQUE;
        DEFINE INDEX IF NOT EXISTS poll_vote_poll_idx ON poll_vote COLUMNS poll_id;

        -- ================================================
        -- Audit Log (immutable admin action history)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS audit_log SCHEMAFULL;
        DEFINE FIELD OVERWRITE actor_id ON audit_log TYPE string;
        DEFINE FIELD OVERWRITE action ON audit_log TYPE string;
        DEFINE FIELD OVERWRITE target_type ON audit_log TYPE string;
        DEFINE FIELD OVERWRITE target_id ON audit_log TYPE string;
        DEFINE FIELD OVERWRITE details ON audit_log TYPE option<string>;
        DEFINE FIELD OVERWRITE created_at ON audit_log TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS audit_log_created_idx ON audit_log COLUMNS created_at;
        DEFINE INDEX IF NOT EXISTS audit_log_actor_idx ON audit_log COLUMNS actor_id;
        DEFINE INDEX IF NOT EXISTS audit_log_target_idx ON audit_log COLUMNS target_type, target_id;

        -- ================================================
        -- Moderation Actions (warnings, suspensions, bans)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS moderation_action SCHEMAFULL;
        DEFINE FIELD OVERWRITE member_id ON moderation_action TYPE string;
        DEFINE FIELD OVERWRITE action_type ON moderation_action TYPE string
            ASSERT $value IN ['note', 'warning', 'suspension', 'ban'];
        DEFINE FIELD OVERWRITE reason ON moderation_action TYPE string;
        DEFINE FIELD OVERWRITE issued_by ON moderation_action TYPE string;
        DEFINE FIELD OVERWRITE expires_at ON moderation_action TYPE option<datetime>;
        DEFINE FIELD OVERWRITE is_active ON moderation_action TYPE bool DEFAULT true;
        DEFINE FIELD OVERWRITE created_at ON moderation_action TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS mod_member_idx ON moderation_action COLUMNS member_id;
        DEFINE INDEX IF NOT EXISTS mod_active_idx ON moderation_action COLUMNS member_id, action_type, is_active;

        -- ================================================
        -- Site Settings (single-document, admin-configurable)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS site_settings SCHEMAFULL;
        DEFINE FIELD OVERWRITE org_name ON site_settings TYPE string DEFAULT 'The Scuffed Crew';
        DEFINE FIELD OVERWRITE site_description ON site_settings TYPE string DEFAULT 'EMEA Gaming Organization';
        DEFINE FIELD OVERWRITE recruitment_open ON site_settings TYPE bool DEFAULT true;
        DEFINE FIELD OVERWRITE recruitment_message ON site_settings TYPE string DEFAULT 'We are currently recruiting! Apply now to join the crew.';
        DEFINE FIELD OVERWRITE min_age ON site_settings TYPE int DEFAULT 16;
        DEFINE FIELD OVERWRITE forum_backend ON site_settings TYPE string DEFAULT 'local'
            ASSERT $value IN ['local', 'nostr'];
        DEFINE FIELD OVERWRITE extra_relay_urls ON site_settings TYPE string DEFAULT '';
        DEFINE FIELD OVERWRITE public_layout ON site_settings TYPE string DEFAULT 'hub'
            ASSERT $value IN ['hub', 'landing'];
        DEFINE FIELD OVERWRITE homepage_json ON site_settings TYPE string DEFAULT '';
        DEFINE FIELD OVERWRITE updated_at ON site_settings TYPE datetime DEFAULT time::now();

        -- ================================================
        -- Game Accounts (linked gaming platform accounts)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS game_account SCHEMAFULL;
        DEFINE FIELD OVERWRITE member_id ON game_account TYPE string;
        DEFINE FIELD OVERWRITE game_id ON game_account TYPE string;
        DEFINE FIELD OVERWRITE account_name ON game_account TYPE string;
        DEFINE FIELD OVERWRITE account_id ON game_account TYPE option<string>;
        DEFINE FIELD OVERWRITE created_at ON game_account TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS game_account_unique_idx ON game_account
            COLUMNS member_id, game_id UNIQUE;

        -- ================================================
        -- Event RSVPs
        -- ================================================
        DEFINE TABLE IF NOT EXISTS event_rsvp SCHEMAFULL;
        DEFINE FIELD OVERWRITE member_id ON event_rsvp TYPE string;
        DEFINE FIELD OVERWRITE event_id ON event_rsvp TYPE string;
        DEFINE FIELD OVERWRITE status ON event_rsvp TYPE string
            ASSERT $value IN ['yes', 'maybe', 'no'];
        DEFINE FIELD OVERWRITE responded_at ON event_rsvp TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS event_rsvp_unique_idx ON event_rsvp
            COLUMNS event_id, member_id UNIQUE;

        -- ================================================
        -- Event Attendance (officer-marked presence tracking)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS event_attendance SCHEMAFULL;
        DEFINE FIELD OVERWRITE member_id ON event_attendance TYPE string;
        DEFINE FIELD OVERWRITE event_id ON event_attendance TYPE string;
        DEFINE FIELD OVERWRITE occurrence_date ON event_attendance TYPE datetime;
        DEFINE FIELD OVERWRITE status ON event_attendance TYPE string
            ASSERT $value IN ['attended', 'no_show', 'excused'];
        DEFINE FIELD OVERWRITE marked_by ON event_attendance TYPE string;
        DEFINE FIELD OVERWRITE marked_at ON event_attendance TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS event_attendance_unique_idx ON event_attendance
            COLUMNS member_id, event_id, occurrence_date UNIQUE;

        -- ================================================
        -- Tournaments
        -- ================================================
        DEFINE TABLE IF NOT EXISTS tournament SCHEMAFULL;
        DEFINE FIELD OVERWRITE name ON tournament TYPE string;
        DEFINE FIELD OVERWRITE game_id ON tournament TYPE option<string>;
        DEFINE FIELD OVERWRITE format ON tournament TYPE string
            ASSERT $value IN ['single_elim', 'double_elim', 'round_robin', 'swiss'];
        DEFINE FIELD OVERWRITE status ON tournament TYPE string DEFAULT 'draft'
            ASSERT $value IN ['draft', 'registration', 'in_progress', 'completed', 'archived'];
        DEFINE FIELD OVERWRITE max_teams ON tournament TYPE option<int>;
        DEFINE FIELD OVERWRITE best_of ON tournament TYPE int DEFAULT 1;
        DEFINE FIELD OVERWRITE swiss_rounds ON tournament TYPE option<int>;
        DEFINE FIELD OVERWRITE is_external ON tournament TYPE bool DEFAULT false;
        DEFINE FIELD OVERWRITE is_open ON tournament TYPE bool DEFAULT false;
        DEFINE FIELD OVERWRITE external_url ON tournament TYPE option<string>;
        DEFINE FIELD OVERWRITE rules ON tournament TYPE option<string>;
        DEFINE FIELD OVERWRITE description ON tournament TYPE option<string>;
        DEFINE FIELD OVERWRITE starts_at ON tournament TYPE option<datetime>;
        DEFINE FIELD OVERWRITE ends_at ON tournament TYPE option<datetime>;
        DEFINE FIELD OVERWRITE created_by ON tournament TYPE string;
        DEFINE FIELD OVERWRITE created_at ON tournament TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE updated_at ON tournament TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS tournament_status_idx ON tournament COLUMNS status;

        -- ================================================
        -- Tournament Participants
        -- ================================================
        DEFINE TABLE IF NOT EXISTS tournament_participant SCHEMAFULL;
        DEFINE FIELD OVERWRITE tournament_id ON tournament_participant TYPE string;
        DEFINE FIELD OVERWRITE team_id ON tournament_participant TYPE option<string>;
        DEFINE FIELD OVERWRITE external_name ON tournament_participant TYPE option<string>;
        DEFINE FIELD OVERWRITE seed ON tournament_participant TYPE option<int>;
        DEFINE FIELD OVERWRITE group_label ON tournament_participant TYPE option<string>;
        DEFINE FIELD OVERWRITE status ON tournament_participant TYPE string DEFAULT 'registered'
            ASSERT $value IN ['registered', 'checked_in', 'active', 'eliminated', 'withdrawn', 'disqualified'];
        DEFINE FIELD OVERWRITE created_at ON tournament_participant TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS tp_tournament_idx ON tournament_participant COLUMNS tournament_id;
        DEFINE INDEX IF NOT EXISTS tp_tournament_team_idx ON tournament_participant
            COLUMNS tournament_id, team_id UNIQUE;

        -- ================================================
        -- Tournament Rounds
        -- ================================================
        DEFINE TABLE IF NOT EXISTS tournament_round SCHEMAFULL;
        DEFINE FIELD OVERWRITE tournament_id ON tournament_round TYPE string;
        DEFINE FIELD OVERWRITE round_number ON tournament_round TYPE int;
        DEFINE FIELD OVERWRITE stage ON tournament_round TYPE string DEFAULT 'main'
            ASSERT $value IN ['main', 'winners', 'losers', 'grand_final', 'group'];
        DEFINE FIELD OVERWRITE status ON tournament_round TYPE string DEFAULT 'pending'
            ASSERT $value IN ['pending', 'in_progress', 'completed'];
        DEFINE FIELD OVERWRITE created_at ON tournament_round TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS tr_tournament_idx ON tournament_round COLUMNS tournament_id;

        -- ================================================
        -- Tournament Matches
        -- ================================================
        DEFINE TABLE IF NOT EXISTS tournament_match SCHEMAFULL;
        DEFINE FIELD OVERWRITE tournament_id ON tournament_match TYPE string;
        DEFINE FIELD OVERWRITE round_id ON tournament_match TYPE string;
        DEFINE FIELD OVERWRITE bracket_position ON tournament_match TYPE int DEFAULT 0;
        DEFINE FIELD OVERWRITE participant_a_id ON tournament_match TYPE option<string>;
        DEFINE FIELD OVERWRITE participant_b_id ON tournament_match TYPE option<string>;
        DEFINE FIELD OVERWRITE score_a ON tournament_match TYPE option<int>;
        DEFINE FIELD OVERWRITE score_b ON tournament_match TYPE option<int>;
        DEFINE FIELD OVERWRITE winner_id ON tournament_match TYPE option<string>;
        DEFINE FIELD OVERWRITE status ON tournament_match TYPE string DEFAULT 'pending'
            ASSERT $value IN ['pending', 'scheduled', 'in_progress', 'completed', 'bye'];
        DEFINE FIELD OVERWRITE scheduled_at ON tournament_match TYPE option<datetime>;
        DEFINE FIELD OVERWRITE completed_at ON tournament_match TYPE option<datetime>;
        DEFINE FIELD OVERWRITE match_result_id ON tournament_match TYPE option<string>;
        DEFINE FIELD OVERWRITE next_match_id ON tournament_match TYPE option<string>;
        DEFINE FIELD OVERWRITE next_match_slot ON tournament_match TYPE option<string>;
        DEFINE FIELD OVERWRITE loser_next_match_id ON tournament_match TYPE option<string>;
        DEFINE FIELD OVERWRITE loser_next_match_slot ON tournament_match TYPE option<string>;
        DEFINE FIELD OVERWRITE notes ON tournament_match TYPE option<string>;
        DEFINE FIELD OVERWRITE replay_codes ON tournament_match TYPE array DEFAULT [];
        DEFINE FIELD OVERWRITE replay_codes.* ON tournament_match TYPE string;

        DEFINE INDEX IF NOT EXISTS tm_tournament_idx ON tournament_match COLUMNS tournament_id;
        DEFINE INDEX IF NOT EXISTS tm_round_idx ON tournament_match COLUMNS round_id;

        -- ================================================
        -- Strategies (strategy planner documents)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS strategy SCHEMAFULL;
        DEFINE FIELD OVERWRITE name ON strategy TYPE string;
        DEFINE FIELD OVERWRITE description ON strategy TYPE option<string>;
        DEFINE FIELD OVERWRITE map_id ON strategy TYPE string;
        DEFINE FIELD OVERWRITE sub_map_id ON strategy TYPE option<string>;
        DEFINE FIELD OVERWRITE game_mode ON strategy TYPE string;
        DEFINE FIELD OVERWRITE owner_id ON strategy TYPE string;
        DEFINE FIELD OVERWRITE team_id ON strategy TYPE option<string>;
        DEFINE FIELD OVERWRITE visibility ON strategy TYPE string DEFAULT 'private'
            ASSERT $value IN ['private', 'unlisted', 'public'];
        DEFINE FIELD OVERWRITE elements ON strategy TYPE array DEFAULT [];
        DEFINE FIELD OVERWRITE elements.* ON strategy TYPE object FLEXIBLE;
        DEFINE FIELD OVERWRITE phases ON strategy TYPE array DEFAULT [];
        DEFINE FIELD OVERWRITE phases.* ON strategy TYPE object FLEXIBLE;
        DEFINE FIELD OVERWRITE coordinate_version ON strategy TYPE string DEFAULT 'v2';
        DEFINE FIELD OVERWRITE created_at ON strategy TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE updated_at ON strategy TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS strategy_owner_idx ON strategy COLUMNS owner_id;
        DEFINE INDEX IF NOT EXISTS strategy_visibility_idx ON strategy COLUMNS visibility;
        DEFINE INDEX IF NOT EXISTS strategy_map_idx ON strategy COLUMNS map_id;

        -- ================================================
        -- Team Channels (NIP-29 group mapping per team)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS team_channel SCHEMAFULL;
        DEFINE FIELD OVERWRITE team_id ON team_channel TYPE string;
        DEFINE FIELD OVERWRITE group_id ON team_channel TYPE string;
        DEFINE FIELD OVERWRITE group_type ON team_channel TYPE string DEFAULT 'public'
            ASSERT $value IN ['public', 'officer'];
        DEFINE FIELD OVERWRITE relay_url ON team_channel TYPE string;
        DEFINE FIELD OVERWRITE is_active ON team_channel TYPE bool DEFAULT true;
        DEFINE FIELD OVERWRITE created_at ON team_channel TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE synced_at ON team_channel TYPE option<datetime>;

        DEFINE INDEX IF NOT EXISTS team_channel_team_idx ON team_channel COLUMNS team_id;
        DEFINE INDEX IF NOT EXISTS team_channel_group_idx ON team_channel COLUMNS group_id UNIQUE;
        DEFINE INDEX IF NOT EXISTS team_channel_team_type_idx ON team_channel
            COLUMNS team_id, group_type UNIQUE;

        -- ================================================
        -- Group Last Seen (per-member read cursor for unread badges)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS group_last_seen SCHEMAFULL;
        DEFINE FIELD OVERWRITE member_id ON group_last_seen TYPE string;
        DEFINE FIELD OVERWRITE group_id ON group_last_seen TYPE string;
        DEFINE FIELD OVERWRITE last_seen_at ON group_last_seen TYPE datetime;
        DEFINE FIELD OVERWRITE updated_at ON group_last_seen TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS gls_member_group_idx ON group_last_seen
            COLUMNS member_id, group_id UNIQUE;
        DEFINE INDEX IF NOT EXISTS gls_member_idx ON group_last_seen COLUMNS member_id;

        -- ================================================
        -- Scrims (scrim board / practice match requests)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS scrim SCHEMAFULL;
        DEFINE FIELD OVERWRITE team_id ON scrim TYPE string;
        DEFINE FIELD OVERWRITE game_id ON scrim TYPE string;
        DEFINE FIELD OVERWRITE requested_by ON scrim TYPE string;
        DEFINE FIELD OVERWRITE opponent_name ON scrim TYPE option<string>;
        DEFINE FIELD OVERWRITE scheduled_at ON scrim TYPE datetime;
        DEFINE FIELD OVERWRITE duration_minutes ON scrim TYPE int DEFAULT 90;
        DEFINE FIELD OVERWRITE status ON scrim TYPE string DEFAULT 'open';
        DEFINE FIELD OVERWRITE notes ON scrim TYPE option<string>;
        DEFINE FIELD OVERWRITE created_at ON scrim TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE updated_at ON scrim TYPE datetime DEFAULT time::now();
        DEFINE INDEX IF NOT EXISTS scrim_team_status_idx ON scrim COLUMNS team_id, status;
        DEFINE INDEX IF NOT EXISTS scrim_scheduled_idx ON scrim COLUMNS scheduled_at;

        -- ================================================
        -- Articles (blog posts)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS article SCHEMAFULL;
        DEFINE FIELD OVERWRITE slug ON article TYPE string;
        DEFINE FIELD OVERWRITE title ON article TYPE string;
        DEFINE FIELD OVERWRITE content_markdown ON article TYPE string;
        DEFINE FIELD OVERWRITE summary ON article TYPE option<string>;
        DEFINE FIELD OVERWRITE cover_image_url ON article TYPE option<string>;
        DEFINE FIELD OVERWRITE author_member_id ON article TYPE string;
        DEFINE FIELD OVERWRITE published ON article TYPE bool DEFAULT false;
        DEFINE FIELD OVERWRITE published_at ON article TYPE option<datetime>;
        DEFINE FIELD OVERWRITE created_at ON article TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE updated_at ON article TYPE datetime DEFAULT time::now();
        DEFINE INDEX IF NOT EXISTS article_slug_idx ON article COLUMNS slug UNIQUE;
        DEFINE INDEX IF NOT EXISTS article_published_idx ON article COLUMNS published, published_at;

        -- ================================================
        -- Wiki Pages (knowledge base)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS wiki_page SCHEMAFULL;
        DEFINE FIELD OVERWRITE topic ON wiki_page TYPE string;
        DEFINE FIELD OVERWRITE title ON wiki_page TYPE string;
        DEFINE FIELD OVERWRITE content_markdown ON wiki_page TYPE string;
        DEFINE FIELD OVERWRITE author_member_id ON wiki_page TYPE string;
        DEFINE FIELD OVERWRITE created_at ON wiki_page TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE updated_at ON wiki_page TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE is_active ON wiki_page TYPE bool DEFAULT true;
        DEFINE INDEX IF NOT EXISTS wiki_page_topic_idx ON wiki_page COLUMNS topic UNIQUE;
        DEFINE INDEX IF NOT EXISTS wiki_page_updated_idx ON wiki_page COLUMNS updated_at;

        -- ================================================
        -- Wiki Revisions (edit history)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS wiki_revision SCHEMAFULL;
        DEFINE FIELD OVERWRITE page_id ON wiki_revision TYPE string;
        DEFINE FIELD OVERWRITE content_markdown ON wiki_revision TYPE string;
        DEFINE FIELD OVERWRITE edited_by ON wiki_revision TYPE string;
        DEFINE FIELD OVERWRITE edited_at ON wiki_revision TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE revision_note ON wiki_revision TYPE option<string>;
        DEFINE INDEX IF NOT EXISTS wiki_revision_page_idx ON wiki_revision COLUMNS page_id, edited_at;

        -- ================================================
        -- Forum hierarchy (category → board → optional sub-board)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS forum_category SCHEMAFULL;
        DEFINE FIELD OVERWRITE name ON forum_category TYPE string;
        DEFINE FIELD OVERWRITE slug ON forum_category TYPE string;
        DEFINE FIELD OVERWRITE description ON forum_category TYPE option<string>;
        DEFINE FIELD OVERWRITE sort_order ON forum_category TYPE int DEFAULT 0;
        DEFINE FIELD OVERWRITE is_active ON forum_category TYPE bool DEFAULT true;
        DEFINE INDEX IF NOT EXISTS forum_category_slug_idx ON forum_category COLUMNS slug UNIQUE;

        DEFINE TABLE IF NOT EXISTS forum_board SCHEMAFULL;
        DEFINE FIELD OVERWRITE category_id ON forum_board TYPE string;
        DEFINE FIELD OVERWRITE parent_board_id ON forum_board TYPE option<string>;
        DEFINE FIELD OVERWRITE name ON forum_board TYPE string;
        DEFINE FIELD OVERWRITE slug ON forum_board TYPE string;
        DEFINE FIELD OVERWRITE description ON forum_board TYPE option<string>;
        DEFINE FIELD OVERWRITE sort_order ON forum_board TYPE int DEFAULT 0;
        DEFINE FIELD OVERWRITE is_locked ON forum_board TYPE bool DEFAULT false;
        DEFINE FIELD OVERWRITE min_role ON forum_board TYPE option<string>;
        DEFINE FIELD OVERWRITE is_active ON forum_board TYPE bool DEFAULT true;
        DEFINE INDEX IF NOT EXISTS forum_board_slug_idx ON forum_board COLUMNS slug UNIQUE;
        DEFINE INDEX IF NOT EXISTS forum_board_cat_idx ON forum_board COLUMNS category_id, sort_order;

        -- ================================================
        -- Forum Threads (discussion board)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS forum_thread SCHEMAFULL;
        DEFINE FIELD OVERWRITE title ON forum_thread TYPE string;
        DEFINE FIELD OVERWRITE category ON forum_thread TYPE string DEFAULT 'general';
        DEFINE FIELD OVERWRITE board_id ON forum_thread TYPE option<string>;
        DEFINE FIELD OVERWRITE author_member_id ON forum_thread TYPE string;
        DEFINE FIELD OVERWRITE content ON forum_thread TYPE string;
        DEFINE FIELD OVERWRITE pinned ON forum_thread TYPE bool DEFAULT false;
        DEFINE FIELD OVERWRITE locked ON forum_thread TYPE bool DEFAULT false;
        DEFINE FIELD OVERWRITE created_at ON forum_thread TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE updated_at ON forum_thread TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE nostr_event_id ON forum_thread TYPE option<string>;
        DEFINE FIELD OVERWRITE is_active ON forum_thread TYPE bool DEFAULT true;
        DEFINE INDEX IF NOT EXISTS forum_thread_cat_idx ON forum_thread COLUMNS category, created_at;
        DEFINE INDEX IF NOT EXISTS forum_thread_board_idx ON forum_thread COLUMNS board_id, created_at;

        -- ================================================
        -- Forum Replies
        -- ================================================
        DEFINE TABLE IF NOT EXISTS forum_reply SCHEMAFULL;
        DEFINE FIELD OVERWRITE thread_id ON forum_reply TYPE string;
        DEFINE FIELD OVERWRITE author_member_id ON forum_reply TYPE string;
        DEFINE FIELD OVERWRITE content ON forum_reply TYPE string;
        DEFINE FIELD OVERWRITE created_at ON forum_reply TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE is_active ON forum_reply TYPE bool DEFAULT true;
        DEFINE INDEX IF NOT EXISTS forum_reply_thread_idx ON forum_reply COLUMNS thread_id, created_at;

        -- ================================================
        -- Personal Matches (individual stat-tracker uploads)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS personal_match SCHEMAFULL;
        DEFINE FIELD OVERWRITE member_id ON personal_match TYPE string;
        DEFINE FIELD OVERWRITE hero ON personal_match TYPE string;
        DEFINE FIELD OVERWRITE map_name ON personal_match TYPE string;
        DEFINE FIELD OVERWRITE game_mode ON personal_match TYPE string;
        DEFINE FIELD OVERWRITE role ON personal_match TYPE string;
        DEFINE FIELD OVERWRITE outcome ON personal_match TYPE string
            ASSERT $value IN ['victory', 'defeat', 'draw'];
        DEFINE FIELD OVERWRITE elims ON personal_match TYPE int DEFAULT 0;
        DEFINE FIELD OVERWRITE deaths ON personal_match TYPE int DEFAULT 0;
        DEFINE FIELD OVERWRITE assists ON personal_match TYPE int DEFAULT 0;
        DEFINE FIELD OVERWRITE damage ON personal_match TYPE int DEFAULT 0;
        DEFINE FIELD OVERWRITE healing ON personal_match TYPE int DEFAULT 0;
        DEFINE FIELD OVERWRITE mitigation ON personal_match TYPE int DEFAULT 0;
        DEFINE FIELD OVERWRITE played_at ON personal_match TYPE datetime;
        DEFINE FIELD OVERWRITE uploaded_at ON personal_match TYPE datetime DEFAULT time::now();
        -- Client-generated game session id. Rows are stored under a
        -- deterministic record id derived from (member_id, session_id), so
        -- uploads are idempotent upserts: capture snapshots of one game
        -- collapse to one row, and outcome/map corrections update in place.
        -- Legacy rows (uploaded before session ids) keep ''.
        DEFINE FIELD OVERWRITE session_id ON personal_match TYPE string DEFAULT '';

        DEFINE INDEX IF NOT EXISTS pm_member_idx ON personal_match COLUMNS member_id, played_at;
        DEFINE INDEX IF NOT EXISTS pm_session_idx ON personal_match COLUMNS member_id, session_id;
        -- Dropped: content-based dedup is obsolete under per-session upserts,
        -- and it wedged the sync queue — the "unique" error-string check that
        -- guarded it never matched SurrealDB v3's IndexExists message, so any
        -- retried or corrected row 500'd the whole upload forever.
        REMOVE INDEX IF EXISTS pm_dedup_idx ON personal_match;

        -- ================================================
        -- Direct Messages (NIP-44 + NIP-59 gift wrap, Phase 5)
        -- ================================================
        -- One row per delivered gift wrap; dedup on `gift_wrap_id`.
        -- Stored content is the decrypted plaintext (server-managed mode only).
        -- `conversation_key` is the sorted "lo,hi" pubkey pair for fast peer
        -- queries regardless of who sent which message.
        DEFINE TABLE IF NOT EXISTS dm_message SCHEMAFULL;
        DEFINE FIELD OVERWRITE gift_wrap_id ON dm_message TYPE string;
        DEFINE FIELD OVERWRITE sender_pubkey ON dm_message TYPE string;
        DEFINE FIELD OVERWRITE recipient_pubkey ON dm_message TYPE string;
        DEFINE FIELD OVERWRITE conversation_key ON dm_message TYPE string;
        DEFINE FIELD OVERWRITE content ON dm_message TYPE string;
        DEFINE FIELD OVERWRITE reply_to_event_id ON dm_message TYPE option<string>;
        DEFINE FIELD OVERWRITE created_at ON dm_message TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE relay_received_at ON dm_message TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS dm_message_gift_wrap_idx ON dm_message COLUMNS gift_wrap_id UNIQUE;
        DEFINE INDEX IF NOT EXISTS dm_message_conv_idx ON dm_message COLUMNS conversation_key, created_at;
        DEFINE INDEX IF NOT EXISTS dm_message_recipient_idx ON dm_message COLUMNS recipient_pubkey, created_at;
        DEFINE INDEX IF NOT EXISTS dm_message_sender_idx ON dm_message COLUMNS sender_pubkey, created_at;

        -- ================================================
        -- DM Read Markers (per-member, per-conversation)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS dm_read_marker SCHEMAFULL;
        DEFINE FIELD OVERWRITE member_id ON dm_read_marker TYPE string;
        DEFINE FIELD OVERWRITE peer_pubkey ON dm_read_marker TYPE string;
        DEFINE FIELD OVERWRITE last_read_at ON dm_read_marker TYPE datetime;
        DEFINE FIELD OVERWRITE updated_at ON dm_read_marker TYPE datetime DEFAULT time::now();

        DEFINE INDEX IF NOT EXISTS dm_read_marker_idx ON dm_read_marker COLUMNS member_id, peer_pubkey UNIQUE;

        -- ================================================
        -- Daemon Tokens (stat-tracker daemon auth)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS daemon_token SCHEMAFULL;
        DEFINE FIELD OVERWRITE member_id ON daemon_token TYPE string;
        DEFINE FIELD OVERWRITE token_hash ON daemon_token TYPE string;
        DEFINE FIELD OVERWRITE label ON daemon_token TYPE string DEFAULT 'default';
        DEFINE FIELD OVERWRITE is_active ON daemon_token TYPE bool DEFAULT true;
        DEFINE FIELD OVERWRITE created_at ON daemon_token TYPE datetime DEFAULT time::now();
        DEFINE FIELD OVERWRITE last_used_at ON daemon_token TYPE option<datetime>;

        DEFINE INDEX IF NOT EXISTS dt_token_hash_idx ON daemon_token COLUMNS token_hash UNIQUE;
        DEFINE INDEX IF NOT EXISTS dt_member_idx ON daemon_token COLUMNS member_id;

        -- ================================================
        -- Member Settings (per-member daemon / app config)
        -- ================================================
        DEFINE TABLE IF NOT EXISTS member_settings SCHEMAFULL;
        DEFINE FIELD OVERWRITE member_id ON member_settings TYPE string;
        DEFINE FIELD OVERWRITE player_name ON member_settings TYPE option<string>;

        DEFINE INDEX IF NOT EXISTS ms_member_idx ON member_settings COLUMNS member_id UNIQUE;
    "#,
        )
        .await?
        .check()?;

    // Seed default category/board tree and migrate legacy thread.category strings.
    if let Err(e) = crate::queries::forum::ensure_forum_hierarchy(client).await {
        tracing::warn!("forum hierarchy seed/migrate: {e}");
    }

    tracing::info!("Database migrations complete");
    Ok(())
}
