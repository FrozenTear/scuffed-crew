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
        DEFINE FIELD provider_id_encrypted ON user TYPE option<object> FLEXIBLE;
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
        DEFINE FIELD nostr_pubkey ON member TYPE option<string>;
        DEFINE FIELD nostr_key_mode ON member TYPE option<string>
            ASSERT $value IN [NONE, 'server_managed', 'external'];
        DEFINE FIELD nostr_secret_key_encrypted ON member TYPE option<object> FLEXIBLE;
        DEFINE FIELD is_active ON member TYPE bool DEFAULT true;

        DEFINE INDEX member_user_idx ON member COLUMNS user_id UNIQUE;
        DEFINE INDEX member_nostr_pubkey_idx ON member COLUMNS nostr_pubkey UNIQUE;

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
        DEFINE FIELD forum_backend ON site_settings TYPE string DEFAULT 'local'
            ASSERT $value IN ['local', 'nostr'];
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

        -- ================================================
        -- Strategies (strategy planner documents)
        -- ================================================
        DEFINE TABLE strategy SCHEMAFULL;
        DEFINE FIELD name ON strategy TYPE string;
        DEFINE FIELD description ON strategy TYPE option<string>;
        DEFINE FIELD map_id ON strategy TYPE string;
        DEFINE FIELD sub_map_id ON strategy TYPE option<string>;
        DEFINE FIELD game_mode ON strategy TYPE string;
        DEFINE FIELD owner_id ON strategy TYPE string;
        DEFINE FIELD team_id ON strategy TYPE option<string>;
        DEFINE FIELD visibility ON strategy TYPE string DEFAULT 'private'
            ASSERT $value IN ['private', 'unlisted', 'public'];
        DEFINE FIELD elements ON strategy TYPE array DEFAULT [];
        DEFINE FIELD elements.* ON strategy TYPE object FLEXIBLE;
        DEFINE FIELD phases ON strategy TYPE array DEFAULT [];
        DEFINE FIELD phases.* ON strategy TYPE object FLEXIBLE;
        DEFINE FIELD coordinate_version ON strategy TYPE string DEFAULT 'v2';
        DEFINE FIELD created_at ON strategy TYPE datetime DEFAULT time::now();
        DEFINE FIELD updated_at ON strategy TYPE datetime DEFAULT time::now();

        DEFINE INDEX strategy_owner_idx ON strategy COLUMNS owner_id;
        DEFINE INDEX strategy_visibility_idx ON strategy COLUMNS visibility;
        DEFINE INDEX strategy_map_idx ON strategy COLUMNS map_id;

        -- ================================================
        -- Team Channels (NIP-29 group mapping per team)
        -- ================================================
        DEFINE TABLE team_channel SCHEMAFULL;
        DEFINE FIELD team_id ON team_channel TYPE string;
        DEFINE FIELD group_id ON team_channel TYPE string;
        DEFINE FIELD group_type ON team_channel TYPE string DEFAULT 'public'
            ASSERT $value IN ['public', 'officer'];
        DEFINE FIELD relay_url ON team_channel TYPE string;
        DEFINE FIELD is_active ON team_channel TYPE bool DEFAULT true;
        DEFINE FIELD created_at ON team_channel TYPE datetime DEFAULT time::now();
        DEFINE FIELD synced_at ON team_channel TYPE option<datetime>;

        DEFINE INDEX team_channel_team_idx ON team_channel COLUMNS team_id;
        DEFINE INDEX team_channel_group_idx ON team_channel COLUMNS group_id UNIQUE;
        DEFINE INDEX team_channel_team_type_idx ON team_channel
            COLUMNS team_id, group_type UNIQUE;

        -- ================================================
        -- Group Last Seen (per-member read cursor for unread badges)
        -- ================================================
        DEFINE TABLE group_last_seen SCHEMAFULL;
        DEFINE FIELD member_id ON group_last_seen TYPE string;
        DEFINE FIELD group_id ON group_last_seen TYPE string;
        DEFINE FIELD last_seen_at ON group_last_seen TYPE datetime;
        DEFINE FIELD updated_at ON group_last_seen TYPE datetime DEFAULT time::now();

        DEFINE INDEX gls_member_group_idx ON group_last_seen
            COLUMNS member_id, group_id UNIQUE;
        DEFINE INDEX gls_member_idx ON group_last_seen COLUMNS member_id;

        -- ================================================
        -- Polls
        -- ================================================
        DEFINE TABLE poll SCHEMAFULL;
        DEFINE FIELD title ON poll TYPE string;
        DEFINE FIELD description ON poll TYPE option<string>;
        DEFINE FIELD options ON poll TYPE array<string>;
        DEFINE FIELD close_at ON poll TYPE option<datetime>;
        DEFINE FIELD allow_multiple ON poll TYPE bool DEFAULT false;
        DEFINE FIELD created_by ON poll TYPE string;
        DEFINE FIELD created_at ON poll TYPE datetime DEFAULT time::now();
        DEFINE FIELD is_active ON poll TYPE bool DEFAULT true;

        DEFINE INDEX poll_active_idx ON poll COLUMNS is_active, created_at;

        -- ================================================
        -- Poll Votes
        -- ================================================
        DEFINE TABLE poll_vote SCHEMAFULL;
        DEFINE FIELD poll_id ON poll_vote TYPE string;
        DEFINE FIELD member_id ON poll_vote TYPE string;
        DEFINE FIELD option_index ON poll_vote TYPE int;
        DEFINE FIELD voted_at ON poll_vote TYPE datetime DEFAULT time::now();

        DEFINE INDEX poll_vote_member_idx ON poll_vote
            COLUMNS poll_id, member_id, option_index UNIQUE;

        -- ================================================
        -- Scrims (scrim board / practice match requests)
        -- ================================================
        DEFINE TABLE scrim SCHEMAFULL;
        DEFINE FIELD team_id ON scrim TYPE string;
        DEFINE FIELD game_id ON scrim TYPE string;
        DEFINE FIELD requested_by ON scrim TYPE string;
        DEFINE FIELD opponent_name ON scrim TYPE option<string>;
        DEFINE FIELD scheduled_at ON scrim TYPE datetime;
        DEFINE FIELD duration_minutes ON scrim TYPE int DEFAULT 90;
        DEFINE FIELD status ON scrim TYPE string DEFAULT 'open';
        DEFINE FIELD notes ON scrim TYPE option<string>;
        DEFINE FIELD created_at ON scrim TYPE datetime DEFAULT time::now();
        DEFINE FIELD updated_at ON scrim TYPE datetime DEFAULT time::now();
        DEFINE INDEX scrim_team_status_idx ON scrim COLUMNS team_id, status;
        DEFINE INDEX scrim_scheduled_idx ON scrim COLUMNS scheduled_at;

        -- ================================================
        -- Articles (blog posts)
        -- ================================================
        DEFINE TABLE article SCHEMAFULL;
        DEFINE FIELD slug ON article TYPE string;
        DEFINE FIELD title ON article TYPE string;
        DEFINE FIELD content_markdown ON article TYPE string;
        DEFINE FIELD summary ON article TYPE option<string>;
        DEFINE FIELD cover_image_url ON article TYPE option<string>;
        DEFINE FIELD author_member_id ON article TYPE string;
        DEFINE FIELD published ON article TYPE bool DEFAULT false;
        DEFINE FIELD published_at ON article TYPE option<datetime>;
        DEFINE FIELD created_at ON article TYPE datetime DEFAULT time::now();
        DEFINE FIELD updated_at ON article TYPE datetime DEFAULT time::now();
        DEFINE INDEX article_slug_idx ON article COLUMNS slug UNIQUE;
        DEFINE INDEX article_published_idx ON article COLUMNS published, published_at;

        -- ================================================
        -- Wiki Pages (knowledge base)
        -- ================================================
        DEFINE TABLE wiki_page SCHEMAFULL;
        DEFINE FIELD topic ON wiki_page TYPE string;
        DEFINE FIELD title ON wiki_page TYPE string;
        DEFINE FIELD content_markdown ON wiki_page TYPE string;
        DEFINE FIELD author_member_id ON wiki_page TYPE string;
        DEFINE FIELD created_at ON wiki_page TYPE datetime DEFAULT time::now();
        DEFINE FIELD updated_at ON wiki_page TYPE datetime DEFAULT time::now();
        DEFINE FIELD is_active ON wiki_page TYPE bool DEFAULT true;
        DEFINE INDEX wiki_page_topic_idx ON wiki_page COLUMNS topic UNIQUE;
        DEFINE INDEX wiki_page_updated_idx ON wiki_page COLUMNS updated_at;

        -- ================================================
        -- Wiki Revisions (edit history)
        -- ================================================
        DEFINE TABLE wiki_revision SCHEMAFULL;
        DEFINE FIELD page_id ON wiki_revision TYPE string;
        DEFINE FIELD content_markdown ON wiki_revision TYPE string;
        DEFINE FIELD edited_by ON wiki_revision TYPE string;
        DEFINE FIELD edited_at ON wiki_revision TYPE datetime DEFAULT time::now();
        DEFINE FIELD revision_note ON wiki_revision TYPE option<string>;
        DEFINE INDEX wiki_revision_page_idx ON wiki_revision COLUMNS page_id, edited_at;

        -- ================================================
        -- Forum Threads (discussion board)
        -- ================================================
        DEFINE TABLE forum_thread SCHEMAFULL;
        DEFINE FIELD title ON forum_thread TYPE string;
        DEFINE FIELD category ON forum_thread TYPE string DEFAULT 'general';
        DEFINE FIELD author_member_id ON forum_thread TYPE string;
        DEFINE FIELD content ON forum_thread TYPE string;
        DEFINE FIELD pinned ON forum_thread TYPE bool DEFAULT false;
        DEFINE FIELD locked ON forum_thread TYPE bool DEFAULT false;
        DEFINE FIELD created_at ON forum_thread TYPE datetime DEFAULT time::now();
        DEFINE FIELD updated_at ON forum_thread TYPE datetime DEFAULT time::now();
        DEFINE FIELD nostr_event_id ON forum_thread TYPE option<string>;
        DEFINE FIELD is_active ON forum_thread TYPE bool DEFAULT true;
        DEFINE INDEX forum_thread_cat_idx ON forum_thread COLUMNS category, created_at;

        -- ================================================
        -- Forum Replies
        -- ================================================
        DEFINE TABLE forum_reply SCHEMAFULL;
        DEFINE FIELD thread_id ON forum_reply TYPE string;
        DEFINE FIELD author_member_id ON forum_reply TYPE string;
        DEFINE FIELD content ON forum_reply TYPE string;
        DEFINE FIELD created_at ON forum_reply TYPE datetime DEFAULT time::now();
        DEFINE FIELD is_active ON forum_reply TYPE bool DEFAULT true;
        DEFINE INDEX forum_reply_thread_idx ON forum_reply COLUMNS thread_id, created_at;
    "#,
        )
        .await?
        .check()?;

    tracing::info!("Database migrations complete");
    Ok(())
}
