use chrono::{DateTime, Utc};
use scuffed_auth::crypto::EncryptedBlob;
use serde::{Deserialize, Serialize};

/// Organization role levels, ordered by privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrgRole {
    Admin,
    Officer,
    Member,
    Recruit,
}

impl OrgRole {
    pub fn is_at_least(&self, required: OrgRole) -> bool {
        self.level() >= required.level()
    }

    fn level(&self) -> u8 {
        match self {
            OrgRole::Admin => 3,
            OrgRole::Officer => 2,
            OrgRole::Member => 1,
            OrgRole::Recruit => 0,
        }
    }
}

impl std::fmt::Display for OrgRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrgRole::Admin => write!(f, "admin"),
            OrgRole::Officer => write!(f, "officer"),
            OrgRole::Member => write!(f, "member"),
            OrgRole::Recruit => write!(f, "recruit"),
        }
    }
}

/// How a member's Nostr keypair is managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NostrKeyMode {
    /// Server generated and stored the key — server signs on behalf of the member.
    ServerManaged,
    /// Member linked their own external key — signs client-side (NIP-07).
    External,
}

impl std::fmt::Display for NostrKeyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NostrKeyMode::ServerManaged => write!(f, "server_managed"),
            NostrKeyMode::External => write!(f, "external"),
        }
    }
}

/// An org member (extends a user with org-specific data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub user_id: String,
    pub org_role: OrgRole,
    pub display_name: String,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub timezone: Option<String>,
    pub pronouns: Option<String>,
    pub availability_status: Option<String>,
    pub nostr_pubkey: Option<String>,
    pub nostr_key_mode: Option<NostrKeyMode>,
    /// Encrypted secret key — only populated for `ServerManaged` mode.
    /// Never exposed via API responses; only used server-side.
    #[serde(skip_serializing)]
    pub nostr_secret_key_encrypted: Option<EncryptedBlob>,
    pub joined_at: DateTime<Utc>,
    pub is_active: bool,
}

/// A game/title that teams can play.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub abbreviation: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// A game team/squad.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub game_id: String,
    pub color: Option<String>,
    pub division: Option<String>,
    pub lore_quote: Option<String>,
    pub logo_url: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Role within a specific team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TeamRole {
    Captain,
    Player,
    Sub,
    Coach,
}

impl std::fmt::Display for TeamRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TeamRole::Captain => write!(f, "captain"),
            TeamRole::Player => write!(f, "player"),
            TeamRole::Sub => write!(f, "sub"),
            TeamRole::Coach => write!(f, "coach"),
        }
    }
}

/// A roster entry (member's role on a team).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterEntry {
    pub id: String,
    pub member_id: String,
    pub team_id: String,
    pub team_role: TeamRole,
    pub joined_at: DateTime<Utc>,
    pub is_active: bool,
}

/// A scheduled event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub day_of_week: u8,
    pub time: String,
    pub timezone: String,
    pub duration_minutes: u32,
    pub is_recurring: bool,
    pub team_id: Option<String>,
    pub created_by: String,
    pub is_active: bool,
    /// Hidden from public surfaces when false.
    #[serde(default)]
    pub is_public: bool,
}

/// Application status in the recruitment pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplicationStatus {
    Pending,
    Trial,
    Accepted,
    Rejected,
    Withdrawn,
}

impl std::fmt::Display for ApplicationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplicationStatus::Pending => write!(f, "pending"),
            ApplicationStatus::Trial => write!(f, "trial"),
            ApplicationStatus::Accepted => write!(f, "accepted"),
            ApplicationStatus::Rejected => write!(f, "rejected"),
            ApplicationStatus::Withdrawn => write!(f, "withdrawn"),
        }
    }
}

/// A recruitment application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub id: String,
    pub user_id: String,
    pub status: ApplicationStatus,
    pub preferred_games: Vec<String>,
    pub preferred_roles: Vec<String>,
    pub message: Option<String>,
    pub reviewed_by: Option<String>,
    pub review_notes: Option<String>,
    pub trial_started_at: Option<DateTime<Utc>>,
    pub trial_ends_at: Option<DateTime<Utc>>,
    pub mentor_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Match type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    #[default]
    Scrim,
    Official,
    Tournament,
}

impl std::fmt::Display for MatchType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchType::Scrim => write!(f, "scrim"),
            MatchType::Official => write!(f, "official"),
            MatchType::Tournament => write!(f, "tournament"),
        }
    }
}

/// A match result record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub id: String,
    pub team_id: String,
    pub opponent: String,
    pub score_us: u32,
    pub score_them: u32,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub match_type: MatchType,
    pub played_at: DateTime<Utc>,
    pub recorded_by: String,
    pub notes: Option<String>,
    #[serde(default)]
    pub is_public: bool,
}

/// Win-loss record for a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRecord {
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

// ─── Tournament Types ───

/// Tournament bracket format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TournamentFormat {
    SingleElim,
    DoubleElim,
    RoundRobin,
    Swiss,
}

impl std::fmt::Display for TournamentFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TournamentFormat::SingleElim => write!(f, "single_elim"),
            TournamentFormat::DoubleElim => write!(f, "double_elim"),
            TournamentFormat::RoundRobin => write!(f, "round_robin"),
            TournamentFormat::Swiss => write!(f, "swiss"),
        }
    }
}

/// Tournament lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TournamentStatus {
    Draft,
    Registration,
    InProgress,
    Completed,
    Archived,
}

impl std::fmt::Display for TournamentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TournamentStatus::Draft => write!(f, "draft"),
            TournamentStatus::Registration => write!(f, "registration"),
            TournamentStatus::InProgress => write!(f, "in_progress"),
            TournamentStatus::Completed => write!(f, "completed"),
            TournamentStatus::Archived => write!(f, "archived"),
        }
    }
}

/// Participant status within a tournament.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantStatus {
    Registered,
    CheckedIn,
    Active,
    Eliminated,
    Withdrawn,
    Disqualified,
}

impl std::fmt::Display for ParticipantStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParticipantStatus::Registered => write!(f, "registered"),
            ParticipantStatus::CheckedIn => write!(f, "checked_in"),
            ParticipantStatus::Active => write!(f, "active"),
            ParticipantStatus::Eliminated => write!(f, "eliminated"),
            ParticipantStatus::Withdrawn => write!(f, "withdrawn"),
            ParticipantStatus::Disqualified => write!(f, "disqualified"),
        }
    }
}

/// Bracket stage for organizing rounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BracketStage {
    Main,
    Winners,
    Losers,
    GrandFinal,
    Group,
}

impl std::fmt::Display for BracketStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BracketStage::Main => write!(f, "main"),
            BracketStage::Winners => write!(f, "winners"),
            BracketStage::Losers => write!(f, "losers"),
            BracketStage::GrandFinal => write!(f, "grand_final"),
            BracketStage::Group => write!(f, "group"),
        }
    }
}

/// Status of an individual tournament match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TournamentMatchStatus {
    Pending,
    Scheduled,
    InProgress,
    Completed,
    Bye,
}

impl std::fmt::Display for TournamentMatchStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TournamentMatchStatus::Pending => write!(f, "pending"),
            TournamentMatchStatus::Scheduled => write!(f, "scheduled"),
            TournamentMatchStatus::InProgress => write!(f, "in_progress"),
            TournamentMatchStatus::Completed => write!(f, "completed"),
            TournamentMatchStatus::Bye => write!(f, "bye"),
        }
    }
}

/// Status of a tournament round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundStatus {
    Pending,
    InProgress,
    Completed,
}

impl std::fmt::Display for RoundStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoundStatus::Pending => write!(f, "pending"),
            RoundStatus::InProgress => write!(f, "in_progress"),
            RoundStatus::Completed => write!(f, "completed"),
        }
    }
}

/// A tournament event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tournament {
    pub id: String,
    pub name: String,
    pub game_id: Option<String>,
    pub format: TournamentFormat,
    pub status: TournamentStatus,
    pub max_teams: Option<u32>,
    pub best_of: u32,
    pub swiss_rounds: Option<u32>,
    pub is_external: bool,
    pub is_open: bool,
    pub external_url: Option<String>,
    pub rules: Option<String>,
    pub description: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A participant in a tournament (org team or external).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentParticipant {
    pub id: String,
    pub tournament_id: String,
    pub team_id: Option<String>,
    pub external_name: Option<String>,
    pub seed: Option<u32>,
    pub group_label: Option<String>,
    pub status: ParticipantStatus,
    pub created_at: DateTime<Utc>,
}

impl TournamentParticipant {
    /// Display name: team name lookup should happen at API layer,
    /// but this gives the external_name fallback.
    pub fn display_name(&self) -> &str {
        self.external_name.as_deref().unwrap_or("TBD")
    }
}

/// A round within a tournament.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentRound {
    pub id: String,
    pub tournament_id: String,
    pub round_number: u32,
    pub stage: BracketStage,
    pub status: RoundStatus,
    pub created_at: DateTime<Utc>,
}

/// A match within a tournament bracket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentMatch {
    pub id: String,
    pub tournament_id: String,
    pub round_id: String,
    pub bracket_position: u32,
    pub participant_a_id: Option<String>,
    pub participant_b_id: Option<String>,
    pub score_a: Option<u32>,
    pub score_b: Option<u32>,
    pub winner_id: Option<String>,
    pub status: TournamentMatchStatus,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub match_result_id: Option<String>,
    pub next_match_id: Option<String>,
    pub next_match_slot: Option<String>,
    pub loser_next_match_id: Option<String>,
    pub loser_next_match_slot: Option<String>,
    pub notes: Option<String>,
    pub replay_codes: Vec<String>,
}

/// Swiss standings for a participant (computed, not stored).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwissStanding {
    pub participant_id: String,
    pub participant_name: String,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    pub game_wins: u32,
    pub game_losses: u32,
    pub buchholz: f64,
    pub rank: u32,
}

/// Complete bracket data for rendering (API aggregate response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentBracket {
    pub tournament: Tournament,
    pub participants: Vec<TournamentParticipant>,
    pub rounds: Vec<TournamentRound>,
    pub matches: Vec<TournamentMatch>,
}

/// A personal match record (stat-tracker upload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalMatch {
    pub id: String,
    pub member_id: String,
    /// Client-generated game session id ('' on legacy rows).
    #[serde(default)]
    pub session_id: String,
    pub hero: String,
    pub map_name: String,
    pub game_mode: String,
    pub role: String,
    pub outcome: String,
    pub elims: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
    pub played_at: DateTime<Utc>,
    pub uploaded_at: DateTime<Utc>,
}

/// Aggregated personal stats for a member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalStats {
    pub member_id: String,
    pub total_matches: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

/// Aggregated stats per hero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeroStats {
    pub hero: String,
    pub matches: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    pub avg_elims: f64,
    pub avg_deaths: f64,
    pub avg_damage: f64,
    pub avg_healing: f64,
}

/// Aggregated stats per map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapStats {
    pub map_name: String,
    pub matches: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

/// Per-member daemon/app settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberSettings {
    pub member_id: String,
    pub player_name: Option<String>,
}

/// A daemon authentication token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonToken {
    pub id: String,
    pub member_id: String,
    pub label: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Audit log action types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    CreatedGame,
    UpdatedGame,
    CreatedTeam,
    UpdatedTeam,
    CreatedEvent,
    UpdatedEvent,
    DeletedEvent,
    AcceptedApplication,
    RejectedApplication,
    WithdrawnApplication,
    StartedTrialApplication,
    ChangedRole,
    UpdatedMember,
    DeactivatedMember,
    AddedToRoster,
    UpdatedRosterRole,
    RemovedFromRoster,
    RecordedMatch,
    UpdatedMatch,
    CreatedAnnouncement,
    UpdatedAnnouncement,
    DeletedAnnouncement,
    CreatedPoll,
    DeletedPoll,
    CreatedModerationAction,
    LiftedModerationAction,
    UpdatedSettings,
    CreatedTournament,
    UpdatedTournament,
    ChangedTournamentStatus,
    GeneratedBracket,
    AddedTournamentParticipant,
    RemovedTournamentParticipant,
    ReportedTournamentMatch,
    PublishedCommunity,
    PublishedPost,
    PublishedReaction,
    CreatedScrim,
    UpdatedScrimStatus,
    CreatedArticle,
    UpdatedArticle,
    PublishedArticle,
    UnpublishedArticle,
    DeletedArticle,
    CreatedWikiPage,
    UpdatedWikiPage,
    DeletedWikiPage,
    PinnedForumThread,
    LockedForumThread,
    DeletedForumThread,
    DeletedForumReply,
    UploadedPersonalStats,
    CreatedDaemonToken,
    RevokedDaemonToken,
    SentDirectMessage,
    SyncedDirectMessages,
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{self:?}"));
        write!(f, "{s}")
    }
}

/// Audit log target types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditTargetType {
    Game,
    Team,
    Member,
    Event,
    Application,
    Roster,
    Match,
    Announcement,
    Poll,
    Moderation,
    Settings,
    Tournament,
    TournamentParticipant,
    TournamentMatch,
    Scrim,
    Article,
    WikiPage,
    ForumThread,
    ForumReply,
    PersonalStats,
    DaemonToken,
    DirectMessage,
}

impl std::fmt::Display for AuditTargetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{self:?}"));
        write!(f, "{s}")
    }
}

/// Type of NIP-29 group channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupType {
    Public,
    Officer,
}

impl std::fmt::Display for GroupType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GroupType::Public => write!(f, "public"),
            GroupType::Officer => write!(f, "officer"),
        }
    }
}

/// A team's auto-provisioned NIP-29 group on the relay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamChannel {
    pub id: String,
    pub team_id: String,
    pub group_id: String,
    pub group_type: GroupType,
    pub relay_url: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub synced_at: Option<DateTime<Utc>>,
}

/// Per-member read cursor for unread badge tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupLastSeen {
    pub id: String,
    pub member_id: String,
    pub group_id: String,
    pub last_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// NIP-29 group role derived from org role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nip29GroupRole {
    GroupAdmin,
    GroupMember,
}

impl OrgRole {
    /// Map org role to NIP-29 group role.
    pub fn to_nip29_role(&self) -> Nip29GroupRole {
        match self {
            OrgRole::Admin | OrgRole::Officer => Nip29GroupRole::GroupAdmin,
            OrgRole::Member | OrgRole::Recruit => Nip29GroupRole::GroupMember,
        }
    }

    /// Whether this role can access officer-only (encrypted) channels.
    pub fn can_access_officer_channel(&self) -> bool {
        matches!(self, OrgRole::Admin | OrgRole::Officer)
    }
}

/// An audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub id: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub details: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Moderation action types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationActionType {
    Note,
    Warning,
    Suspension,
    Ban,
}

impl std::fmt::Display for ModerationActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModerationActionType::Note => write!(f, "note"),
            ModerationActionType::Warning => write!(f, "warning"),
            ModerationActionType::Suspension => write!(f, "suspension"),
            ModerationActionType::Ban => write!(f, "ban"),
        }
    }
}

/// A moderation action record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationAction {
    pub id: String,
    pub member_id: String,
    pub action_type: ModerationActionType,
    pub reason: String,
    pub issued_by: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Site-wide configurable settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettings {
    pub id: String,
    pub org_name: String,
    pub site_description: String,
    pub recruitment_open: bool,
    pub recruitment_message: String,
    pub min_age: u32,
    pub forum_backend: String,
    pub extra_relay_urls: String,
    /// Canonical homepage shell: ops_hub | recruit_landing | minimal | manifesto
    #[serde(default)]
    pub home_shell: String,
    /// Canonical homepage skin: clean | esports
    #[serde(default)]
    pub home_skin: String,
    /// "hub" | "landing" — dual-write mirror of shell for one release
    #[serde(default = "default_public_layout")]
    pub public_layout: String,
    /// JSON blob of homepage copy (parsed to HomepageContent on the app side).
    #[serde(default)]
    pub homepage_json: String,
    /// JSON blob of public nav config (parsed to NavConfig on the app side).
    #[serde(default)]
    pub nav_json: String,
    /// Optional solid page background color (`#…`). Empty = theme default.
    #[serde(default)]
    pub page_bg_color: String,
    /// Optional full-page background image URL. Empty = none.
    #[serde(default)]
    pub page_bg_image_url: String,
    #[serde(default)]
    pub brand_accent_dark: String,
    #[serde(default)]
    pub brand_accent_light: String,
    pub updated_at: DateTime<Utc>,
}

fn default_public_layout() -> String {
    "hub".to_string()
}

/// An announcement/news post.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announcement {
    pub id: String,
    pub title: String,
    pub content: String,
    pub author_id: String,
    pub pinned: bool,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A linked game account for a member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameAccount {
    pub id: String,
    pub member_id: String,
    pub game_id: String,
    pub account_name: String,
    pub account_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// RSVP status for an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RsvpStatus {
    Yes,
    Maybe,
    No,
}

impl std::fmt::Display for RsvpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RsvpStatus::Yes => write!(f, "yes"),
            RsvpStatus::Maybe => write!(f, "maybe"),
            RsvpStatus::No => write!(f, "no"),
        }
    }
}

/// An RSVP for an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRsvp {
    pub id: String,
    pub member_id: String,
    pub event_id: String,
    pub status: RsvpStatus,
    pub responded_at: DateTime<Utc>,
}

/// RSVP summary counts for an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsvpSummary {
    pub event_id: String,
    pub yes_count: u32,
    pub maybe_count: u32,
    pub no_count: u32,
}

/// Attendance status for an event occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttendanceStatus {
    Attended,
    NoShow,
    Excused,
}

impl std::fmt::Display for AttendanceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttendanceStatus::Attended => write!(f, "attended"),
            AttendanceStatus::NoShow => write!(f, "no_show"),
            AttendanceStatus::Excused => write!(f, "excused"),
        }
    }
}

/// An attendance record for a specific event occurrence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventAttendance {
    pub id: String,
    pub member_id: String,
    pub event_id: String,
    pub occurrence_date: DateTime<Utc>,
    pub status: AttendanceStatus,
    pub marked_by: String,
    pub marked_at: DateTime<Utc>,
}

/// Attendance stats for a member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttendanceStats {
    pub member_id: String,
    pub attended: u32,
    pub no_show: u32,
    pub excused: u32,
    pub total: u32,
}

/// A poll/survey.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Poll {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    pub close_at: Option<DateTime<Utc>>,
    pub allow_multiple: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub is_active: bool,
    pub updated_at: DateTime<Utc>,
}

/// A vote on a poll option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollVote {
    pub id: String,
    pub poll_id: String,
    pub member_id: String,
    pub option_index: u32,
    pub voted_at: DateTime<Utc>,
}

/// Aggregated results for a poll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollResults {
    pub poll: Poll,
    pub votes: Vec<PollOptionResult>,
    pub total_votes: u32,
    pub my_votes: Vec<u32>,
}

/// Vote count for a single poll option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollOptionResult {
    pub option_index: u32,
    pub label: String,
    pub count: u32,
}

/// A scrim (practice match) request on the scrim board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scrim {
    pub id: String,
    pub team_id: String,
    pub game_id: String,
    pub requested_by: String,
    pub opponent_name: Option<String>,
    pub scheduled_at: DateTime<Utc>,
    pub duration_minutes: u32,
    pub status: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A blog article.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub content_markdown: String,
    pub summary: Option<String>,
    pub cover_image_url: Option<String>,
    pub author_member_id: String,
    pub published: bool,
    pub nostr_event_id: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A wiki page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPage {
    pub id: String,
    pub topic: String,
    pub title: String,
    pub content_markdown: String,
    pub author_member_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_active: bool,
}

/// A revision of a wiki page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiRevision {
    pub id: String,
    pub page_id: String,
    pub content_markdown: String,
    pub edited_by: String,
    pub edited_at: DateTime<Utc>,
    pub revision_note: Option<String>,
}

/// A top-level forum section (does not hold threads).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumCategory {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub is_active: bool,
}

/// A board or sub-board (threads live here).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumBoard {
    pub id: String,
    pub category_id: String,
    /// `None` = top-level board; `Some` = sub-board of that board.
    pub parent_board_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub is_locked: bool,
    pub min_role: Option<String>,
    pub is_active: bool,
}

/// Board with nested sub-boards for tree responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumBoardNode {
    #[serde(flatten)]
    pub board: ForumBoard,
    pub sub_boards: Vec<ForumBoard>,
    pub thread_count: u64,
}

/// Category with boards for `/api/forum/tree`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumCategoryNode {
    #[serde(flatten)]
    pub category: ForumCategory,
    pub boards: Vec<ForumBoardNode>,
}

/// A forum discussion thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumThread {
    pub id: String,
    pub title: String,
    /// Deprecated string category; kept for migration / display fallback.
    pub category: String,
    /// Board or sub-board id (preferred).
    #[serde(default)]
    pub board_id: Option<String>,
    pub author_member_id: String,
    pub content: String,
    pub pinned: bool,
    pub locked: bool,
    pub nostr_event_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_active: bool,
}

/// A reply within a forum thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumReply {
    pub id: String,
    pub thread_id: String,
    pub author_member_id: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub is_active: bool,
}

// =============================================================================
// Direct Messages (NIP-44 + NIP-59 gift wrap, Phase 5)
// =============================================================================

/// One delivered direct message (gift-wrapped on the relay, decrypted server-side).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmMessage {
    pub id: String,
    /// The kind 1059 gift-wrap event id from the relay (unique).
    pub gift_wrap_id: String,
    pub sender_pubkey: String,
    pub recipient_pubkey: String,
    /// Sorted "lo,hi" pubkey pair — used for fast peer lookups regardless of direction.
    pub conversation_key: String,
    pub content: String,
    pub reply_to_event_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// One conversation summary: the peer + the latest message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmConversation {
    pub peer_pubkey: String,
    pub conversation_key: String,
    pub last_message_preview: String,
    pub last_message_at: DateTime<Utc>,
    pub last_sender_pubkey: String,
    pub unread_count: u32,
}

/// Per-member, per-conversation last-read marker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmReadMarker {
    pub member_id: String,
    pub peer_pubkey: String,
    pub last_read_at: DateTime<Utc>,
}

/// Build the deterministic conversation key from a pubkey pair.
///
/// Sorts the two pubkeys lexicographically and joins with a comma, so
/// `conversation_key(A, B) == conversation_key(B, A)`. Used as a single
/// secondary index for retrieving any DM thread between two pubkeys.
pub fn conversation_key(pubkey_a: &str, pubkey_b: &str) -> String {
    if pubkey_a <= pubkey_b {
        format!("{pubkey_a},{pubkey_b}")
    } else {
        format!("{pubkey_b},{pubkey_a}")
    }
}
