use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Policy decision for an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Reject(String),
}

/// Configuration for the policy engine.
#[derive(Debug, Clone)]
pub struct PolicyConfig {
    /// Maximum events per pubkey within the rate-limit window.
    pub rate_limit_events: u32,
    /// Rate-limit window in seconds.
    pub rate_limit_window_secs: u64,
    /// Allowed event kinds. Events with kinds not in this set are rejected.
    pub allowed_kinds: HashSet<u64>,
    /// Whether to require group membership for NIP-29 group events.
    /// When false, any allowed pubkey can write to any group.
    pub enforce_group_membership: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        let mut allowed_kinds = HashSet::new();

        // NIP-01: basic text notes
        allowed_kinds.insert(1);

        // NIP-29: group chat
        allowed_kinds.insert(9); // group chat message
        allowed_kinds.insert(10); // group chat reply (deprecated but some clients use)
        allowed_kinds.insert(11); // group thread
        allowed_kinds.insert(12); // group thread reply

        // NIP-42: relay auth
        allowed_kinds.insert(22242);

        // NIP-44/NIP-59: encrypted DMs and gift wrap
        allowed_kinds.insert(13); // seal
        allowed_kinds.insert(14); // rumor (should not appear on relay, but allow for gift-wrapped)
        allowed_kinds.insert(1059); // gift wrap

        // NIP-17: private DMs
        allowed_kinds.insert(1111); // NIP-17 DM (wrapped in gift wrap, kind 1059)

        // NIP-09: event deletion
        allowed_kinds.insert(5);

        // NIP-29: group metadata (admin operations)
        for k in 39000..=39009 {
            allowed_kinds.insert(k);
        }

        // NIP-29: group admin events
        allowed_kinds.insert(9000); // add-user
        allowed_kinds.insert(9001); // remove-user
        allowed_kinds.insert(9002); // edit-metadata
        allowed_kinds.insert(9003); // add-permission
        allowed_kinds.insert(9004); // remove-permission
        allowed_kinds.insert(9005); // delete-event
        allowed_kinds.insert(9006); // edit-group-status
        allowed_kinds.insert(9007); // create-group
        allowed_kinds.insert(9008); // delete-group
        allowed_kinds.insert(9021); // join-request
        allowed_kinds.insert(9022); // leave-request

        // Ephemeral events (voice signaling, presence, etc.)
        for k in 20000..=29999 {
            allowed_kinds.insert(k);
        }

        Self {
            rate_limit_events: 30,
            rate_limit_window_secs: 60,
            allowed_kinds,
            enforce_group_membership: false,
        }
    }
}

/// Tracks event timestamps for a single pubkey's rate limiting.
struct RateBucket {
    timestamps: Vec<Instant>,
}

impl RateBucket {
    fn new() -> Self {
        Self {
            timestamps: Vec::new(),
        }
    }

    /// Record an event and return the count within the window.
    fn record(&mut self, now: Instant, window_secs: u64) -> usize {
        let cutoff = now - std::time::Duration::from_secs(window_secs);
        self.timestamps.retain(|t| *t > cutoff);
        self.timestamps.push(now);
        self.timestamps.len()
    }
}

/// The core policy engine that evaluates incoming Nostr events.
pub struct PolicyEngine {
    config: PolicyConfig,
    /// Set of hex pubkeys allowed to write events.
    pubkey_allowlist: HashSet<String>,
    /// Map of NIP-29 group ID -> set of member hex pubkeys.
    group_members: HashMap<String, HashSet<String>>,
    /// Per-pubkey rate limiting state.
    rate_buckets: HashMap<String, RateBucket>,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self {
            config,
            pubkey_allowlist: HashSet::new(),
            group_members: HashMap::new(),
            rate_buckets: HashMap::new(),
        }
    }

    /// Replace the pubkey allowlist with a fresh set from the database.
    pub fn update_allowlist(&mut self, pubkeys: HashSet<String>) {
        self.pubkey_allowlist = pubkeys;
    }

    /// Replace the group membership map.
    pub fn update_group_members(&mut self, groups: HashMap<String, HashSet<String>>) {
        self.group_members = groups;
    }

    /// Returns the number of pubkeys in the current allowlist.
    pub fn allowlist_size(&self) -> usize {
        self.pubkey_allowlist.len()
    }

    /// Evaluate an event and return an accept/reject decision.
    pub fn evaluate(&mut self, event: &EventInfo) -> Decision {
        // 1. Pubkey allowlist check
        if !self.pubkey_allowlist.contains(&event.pubkey) {
            return Decision::Reject("blocked: pubkey not in member allowlist".into());
        }

        // 2. Kind whitelist check
        if !self.config.allowed_kinds.contains(&event.kind) {
            return Decision::Reject(format!("blocked: event kind {} not allowed", event.kind));
        }

        // 3. Rate limiting
        let now = Instant::now();
        let bucket = self
            .rate_buckets
            .entry(event.pubkey.clone())
            .or_insert_with(RateBucket::new);
        let count = bucket.record(now, self.config.rate_limit_window_secs);
        if count > self.config.rate_limit_events as usize {
            return Decision::Reject("rate-limited: too many events".into());
        }

        // 4. NIP-29 group membership (if enforcement is enabled)
        if self.config.enforce_group_membership {
            if let Some(group_id) = event.group_id() {
                if let Some(members) = self.group_members.get(group_id) {
                    if !members.contains(&event.pubkey) {
                        return Decision::Reject(format!(
                            "blocked: not a member of group '{group_id}'"
                        ));
                    }
                } else {
                    // Unknown group — reject
                    return Decision::Reject(format!("blocked: unknown group '{group_id}'"));
                }
            }
        }

        // 5. Encrypted events (NIP-44 gift wrap, seals): metadata-only validation.
        //    We already checked pubkey + kind + rate. Never inspect content.
        //    This is an explicit no-op — the checks above are sufficient.

        Decision::Accept
    }

    /// Prune rate-limit buckets for pubkeys that haven't sent events recently.
    /// Call periodically to prevent unbounded memory growth.
    pub fn prune_rate_buckets(&mut self) {
        let cutoff =
            Instant::now() - std::time::Duration::from_secs(self.config.rate_limit_window_secs * 2);
        self.rate_buckets
            .retain(|_, bucket| bucket.timestamps.last().map_or(false, |t| *t > cutoff));
    }
}

/// Minimal event information extracted from the strfry input.
/// Only fields needed for policy decisions — never the encrypted content.
#[derive(Debug, Clone)]
pub struct EventInfo {
    pub id: String,
    pub pubkey: String,
    pub kind: u64,
    pub tags: Vec<Vec<String>>,
}

impl EventInfo {
    /// Extract the NIP-29 group ID from the event's `h` tag, if present.
    pub fn group_id(&self) -> Option<&str> {
        for tag in &self.tags {
            if tag.len() >= 2 && tag[0] == "h" {
                return Some(&tag[1]);
            }
        }
        None
    }

    /// Check if this is a NIP-29 group event (has an `h` tag).
    pub fn is_group_event(&self) -> bool {
        self.group_id().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> PolicyEngine {
        let config = PolicyConfig::default();
        let mut engine = PolicyEngine::new(config);
        let mut allowlist = HashSet::new();
        allowlist.insert("aabbccdd".repeat(8));
        allowlist.insert("11223344".repeat(8));
        engine.update_allowlist(allowlist);
        engine
    }

    fn make_event(pubkey: &str, kind: u64) -> EventInfo {
        EventInfo {
            id: "deadbeef".repeat(8),
            pubkey: pubkey.to_string(),
            kind,
            tags: vec![],
        }
    }

    fn make_group_event(pubkey: &str, kind: u64, group_id: &str) -> EventInfo {
        EventInfo {
            id: "deadbeef".repeat(8),
            pubkey: pubkey.to_string(),
            kind,
            tags: vec![vec!["h".to_string(), group_id.to_string()]],
        }
    }

    #[test]
    fn accept_known_pubkey_allowed_kind() {
        let mut engine = make_engine();
        let event = make_event(&"aabbccdd".repeat(8), 9);
        assert_eq!(engine.evaluate(&event), Decision::Accept);
    }

    #[test]
    fn reject_unknown_pubkey() {
        let mut engine = make_engine();
        let event = make_event(&"ff".repeat(32), 9);
        assert!(matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("allowlist")));
    }

    #[test]
    fn reject_disallowed_kind() {
        let mut engine = make_engine();
        // Kind 99999 is not in the default allowlist
        let event = make_event(&"aabbccdd".repeat(8), 99999);
        assert!(matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("kind")));
    }

    #[test]
    fn rate_limit_enforced() {
        let config = PolicyConfig {
            rate_limit_events: 3,
            rate_limit_window_secs: 60,
            ..PolicyConfig::default()
        };
        let mut engine = PolicyEngine::new(config);
        let mut allowlist = HashSet::new();
        let pubkey = "aabbccdd".repeat(8);
        allowlist.insert(pubkey.clone());
        engine.update_allowlist(allowlist);

        // First 3 should pass
        for _ in 0..3 {
            let event = make_event(&pubkey, 9);
            assert_eq!(engine.evaluate(&event), Decision::Accept);
        }
        // 4th should be rate-limited
        let event = make_event(&pubkey, 9);
        assert!(matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("rate-limited")));
    }

    #[test]
    fn group_membership_enforced_when_enabled() {
        let config = PolicyConfig {
            enforce_group_membership: true,
            ..PolicyConfig::default()
        };
        let mut engine = PolicyEngine::new(config);

        let member_pubkey = "aabbccdd".repeat(8);
        let nonmember_pubkey = "11223344".repeat(8);

        let mut allowlist = HashSet::new();
        allowlist.insert(member_pubkey.clone());
        allowlist.insert(nonmember_pubkey.clone());
        engine.update_allowlist(allowlist);

        let mut groups = HashMap::new();
        let mut team_members = HashSet::new();
        team_members.insert(member_pubkey.clone());
        groups.insert("team-alpha".to_string(), team_members);
        engine.update_group_members(groups);

        // Member of team-alpha can write group events
        let event = make_group_event(&member_pubkey, 9, "team-alpha");
        assert_eq!(engine.evaluate(&event), Decision::Accept);

        // Non-member of team-alpha gets rejected
        let event = make_group_event(&nonmember_pubkey, 9, "team-alpha");
        assert!(matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("not a member")));

        // Unknown group gets rejected
        let event = make_group_event(&member_pubkey, 9, "unknown-group");
        assert!(matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("unknown group")));
    }

    #[test]
    fn group_membership_not_enforced_by_default() {
        let mut engine = make_engine();
        // Group event from allowed pubkey should pass when enforcement is off
        let event = make_group_event(&"aabbccdd".repeat(8), 9, "any-group");
        assert_eq!(engine.evaluate(&event), Decision::Accept);
    }

    #[test]
    fn encrypted_event_accepted_by_metadata() {
        let mut engine = make_engine();
        // Gift wrap (kind 1059) — should be accepted based on pubkey + kind only
        let event = make_event(&"aabbccdd".repeat(8), 1059);
        assert_eq!(engine.evaluate(&event), Decision::Accept);

        // Seal (kind 13) — same
        let event = make_event(&"aabbccdd".repeat(8), 13);
        assert_eq!(engine.evaluate(&event), Decision::Accept);
    }

    #[test]
    fn ephemeral_event_accepted() {
        let mut engine = make_engine();
        // Voice signaling ephemeral event
        let event = make_event(&"aabbccdd".repeat(8), 25050);
        assert_eq!(engine.evaluate(&event), Decision::Accept);
    }

    #[test]
    fn auth_event_accepted() {
        let mut engine = make_engine();
        let event = make_event(&"aabbccdd".repeat(8), 22242);
        assert_eq!(engine.evaluate(&event), Decision::Accept);
    }

    #[test]
    fn deletion_event_accepted() {
        let mut engine = make_engine();
        let event = make_event(&"aabbccdd".repeat(8), 5);
        assert_eq!(engine.evaluate(&event), Decision::Accept);
    }

    #[test]
    fn group_admin_events_accepted() {
        let mut engine = make_engine();
        let pubkey = "aabbccdd".repeat(8);
        for kind in [9000, 9001, 9002, 9003, 9004, 9005, 9006, 9007, 9008] {
            let event = make_event(&pubkey, kind);
            assert_eq!(engine.evaluate(&event), Decision::Accept, "kind {kind} should be accepted");
        }
    }

    #[test]
    fn event_info_extracts_group_id() {
        let event = make_group_event("abc", 9, "my-group");
        assert_eq!(event.group_id(), Some("my-group"));
        assert!(event.is_group_event());

        let event = make_event("abc", 9);
        assert_eq!(event.group_id(), None);
        assert!(!event.is_group_event());
    }

    #[test]
    fn prune_does_not_panic() {
        let mut engine = make_engine();
        let pubkey = "aabbccdd".repeat(8);
        let event = make_event(&pubkey, 9);
        engine.evaluate(&event);
        engine.prune_rate_buckets();
        // Should still work after prune
        assert_eq!(engine.evaluate(&event), Decision::Accept);
    }
}
