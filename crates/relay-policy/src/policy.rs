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
    /// Whether to require roster membership for events that carry an `h` tag.
    /// Default true. Kind 1059 and other events with no `h` tag skip this check.
    pub enforce_group_membership: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        let mut allowed_kinds = HashSet::new();

        // NIP-01 profile metadata and text notes. site-server publishes kind 0
        // from member profile updates and kind 1 for community posts and forum
        // threads.
        allowed_kinds.insert(0);
        allowed_kinds.insert(1);

        // NIP-25 reactions. site-server routes/nostr.rs publishes these.
        allowed_kinds.insert(7);

        // NIP-29: group chat
        allowed_kinds.insert(9); // group chat message
        allowed_kinds.insert(10); // group chat reply (deprecated but some clients use)
        allowed_kinds.insert(11); // group thread
        allowed_kinds.insert(12); // group thread reply

        // NIP-42: relay auth
        allowed_kinds.insert(22242);

        // NIP-44/NIP-59: encrypted DMs and gift wrap.
        // Kind 1059 is what send_encrypted and POST /api/nostr/dm/send publish.
        // The author is an ephemeral key. evaluate() allowlists the p tag.
        allowed_kinds.insert(13); // seal
        allowed_kinds.insert(14); // rumor (should not appear on relay, but allow for gift-wrapped)
        allowed_kinds.insert(1059); // gift wrap

        // NIP-72 community definition. site-server routes/nostr.rs publishes this.
        allowed_kinds.insert(34550);

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
            enforce_group_membership: true,
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

/// How often the write plugin drops expired rate buckets, including
/// `gift:{sender}:{recipient}` keys. `evaluate` only trims a bucket when
/// that same key is used again.
pub const RATE_BUCKET_SWEEP: std::time::Duration = std::time::Duration::from_secs(60);

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

    /// Replace the group membership map (`group_id` to writer pubkeys).
    ///
    /// `main` loads this from `team_channel` rows and active roster edges.
    pub fn update_group_members(&mut self, groups: HashMap<String, HashSet<String>>) {
        self.group_members = groups;
    }

    /// Returns the number of pubkeys in the current allowlist.
    pub fn allowlist_size(&self) -> usize {
        self.pubkey_allowlist.len()
    }

    /// First `p` tag whose pubkey is on the member allowlist.
    fn allowlisted_recipient(&self, event: &EventInfo) -> Option<String> {
        event.tags.iter().find_map(|tag| {
            if tag.len() >= 2 && tag[0] == "p" {
                let pubkey = tag[1].to_ascii_lowercase();
                if self.pubkey_allowlist.contains(&pubkey) {
                    return Some(pubkey);
                }
            }
            None
        })
    }

    /// Evaluate an event and return an accept/reject decision.
    pub fn evaluate(&mut self, event: &EventInfo) -> Decision {
        // Kind 4 is a plain DM. This relay only stores NIP-59 gift wraps
        // (kind 1059). Reject it even if a config adds kind 4 to the whitelist.
        if event.kind == 4 {
            return Decision::Reject("blocked: kind 4 plain DMs are not accepted".into());
        }

        // Kind 1059 authors are ephemeral (NIP-59). Officer chat and DMs
        // publish these. The member is the `p` tag. The bucket is the sender
        // plus that recipient, not the recipient alone: a per-recipient bucket
        // lets one sender use up the inbox budget for everyone else. The
        // sender is strfry's authenticated pubkey when the connection completed
        // NIP-42. site-server and the read gate do not AUTH to strfry, so the
        // sender there is the event author. Wrap authors are ephemeral, so
        // those publishes do not share a bucket. The gate applies its own
        // `gift:{authed member}:{recipient}` limit before it forwards.
        let rate_key = if event.kind == 1059 {
            if !self.config.allowed_kinds.contains(&event.kind) {
                return Decision::Reject(format!("blocked: event kind {} not allowed", event.kind));
            }
            match self.allowlisted_recipient(event) {
                Some(pubkey) => format!("gift:{}:{pubkey}", gift_sender(event)),
                None => {
                    return Decision::Reject(
                        "blocked: gift wrap recipient not in member allowlist".into(),
                    );
                }
            }
        } else if !self.pubkey_allowlist.contains(&event.pubkey) {
            return Decision::Reject("blocked: pubkey not in member allowlist".into());
        } else if !self.config.allowed_kinds.contains(&event.kind) {
            return Decision::Reject(format!("blocked: event kind {} not allowed", event.kind));
        } else {
            event.pubkey.clone()
        };

        // Roster check for `h` tags, before the rate bucket, so a refused
        // group write does not consume the member's budget. Events with no
        // `h` tag skip this. That includes kind 1059 gift wraps (the member
        // is the `p` tag) and the kinds site-server signs with the member
        // key and no group tag: 0, 7, and 34550.
        if self.config.enforce_group_membership {
            if let Some(group_id) = event.group_id() {
                match self.group_members.get(group_id) {
                    Some(members) if members.contains(&event.pubkey) => {}
                    Some(_) => {
                        return Decision::Reject(format!(
                            "blocked: not a member of group '{group_id}'"
                        ));
                    }
                    None => {
                        return Decision::Reject(format!("blocked: unknown group '{group_id}'"));
                    }
                }
            }
        }

        let now = Instant::now();
        let bucket = self
            .rate_buckets
            .entry(rate_key)
            .or_insert_with(RateBucket::new);
        let count = bucket.record(now, self.config.rate_limit_window_secs);
        if count > self.config.rate_limit_events as usize {
            return Decision::Reject("rate-limited: too many events".into());
        }

        Decision::Accept
    }

    /// Drop stamps outside the rate window, then drop empty buckets.
    ///
    /// Gift-wrap keys are `gift:{sender}:{recipient}`. A pair that never
    /// sends again would otherwise keep that entry. The plugin calls this
    /// on [`RATE_BUCKET_SWEEP`], and also after a writer reload.
    pub fn prune_rate_buckets(&mut self) {
        self.prune_rate_buckets_at(Instant::now());
    }

    fn prune_rate_buckets_at(&mut self, now: Instant) {
        let window = std::time::Duration::from_secs(self.config.rate_limit_window_secs);
        self.rate_buckets.retain(|_, bucket| {
            bucket
                .timestamps
                .retain(|stamp| now.saturating_duration_since(*stamp) < window);
            !bucket.timestamps.is_empty()
        });
    }

    #[cfg(test)]
    fn rate_bucket_count(&self) -> usize {
        self.rate_buckets.len()
    }
}

/// A rejected write that may be a stale allowlist or writer map.
///
/// The LIVE query is the primary reload. This is the backup: an unknown
/// author, an unknown group, or a pubkey that is not in the group's writer
/// set. Callers debounce it so a flood of rejects cannot hammer SurrealDB.
pub fn reload_on_write_miss(reason: &str) -> bool {
    reason.contains("not in member allowlist")
        || reason.contains("unknown group")
        || reason.contains("not a member of group")
}

/// Sender half of a gift-wrap rate key.
///
/// Prefer the NIP-42 pubkey strfry attached to the connection. Fall back to
/// the event author, which is ephemeral for kind 1059.
fn gift_sender(event: &EventInfo) -> String {
    if let Some(pk) = event.authed.as_deref() {
        let pk = pk.trim().to_ascii_lowercase();
        if pk.len() == 64 && pk.chars().all(|c| c.is_ascii_hexdigit()) {
            return pk;
        }
    }
    event.pubkey.to_ascii_lowercase()
}

/// Minimal event information extracted from the strfry input.
/// Only fields needed for policy decisions. Never the encrypted content.
#[derive(Debug, Clone)]
pub struct EventInfo {
    /// Carried for completeness/logging; policy decisions key off the strfry
    /// input's own id, so this field is not read directly.
    #[allow(dead_code)]
    pub id: String,
    pub pubkey: String,
    pub kind: u64,
    pub tags: Vec<Vec<String>>,
    /// NIP-42 pubkey from strfry's `authed` field, when the connection to
    /// strfry itself completed AUTH. Absent for gate-forwarded events and
    /// for site-server's one-shot publishes.
    pub authed: Option<String>,
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
    /// Used by tests; `evaluate` calls `group_id()` directly.
    #[allow(dead_code)]
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
            authed: None,
        }
    }

    fn make_group_event(pubkey: &str, kind: u64, group_id: &str) -> EventInfo {
        EventInfo {
            id: "deadbeef".repeat(8),
            pubkey: pubkey.to_string(),
            kind,
            tags: vec![vec!["h".to_string(), group_id.to_string()]],
            authed: None,
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
        assert!(
            matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("allowlist"))
        );
    }

    #[test]
    fn kind4_plain_dm_rejected_on_write() {
        let mut config = PolicyConfig::default();
        config.allowed_kinds.insert(4);
        let mut engine = PolicyEngine::new(config);
        let pubkey = "aabbccdd".repeat(8);
        let mut allowlist = HashSet::new();
        allowlist.insert(pubkey.clone());
        engine.update_allowlist(allowlist);
        let event = make_event(&pubkey, 4);
        assert!(matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("kind 4")));
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
        assert!(
            matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("rate-limited"))
        );
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
        assert!(
            matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("not a member"))
        );

        // Unknown group gets rejected
        let event = make_group_event(&member_pubkey, 9, "unknown-group");
        assert!(
            matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("unknown group"))
        );
    }

    #[test]
    fn group_membership_skipped_when_disabled() {
        let config = PolicyConfig {
            enforce_group_membership: false,
            ..PolicyConfig::default()
        };
        let mut engine = PolicyEngine::new(config);
        let pubkey = "aabbccdd".repeat(8);
        let mut allowlist = HashSet::new();
        allowlist.insert(pubkey.clone());
        engine.update_allowlist(allowlist);
        let event = make_group_event(&pubkey, 9, "any-group");
        assert_eq!(engine.evaluate(&event), Decision::Accept);
    }

    #[test]
    fn default_enforcement_rejects_unknown_h_group() {
        assert!(PolicyConfig::default().enforce_group_membership);
        let mut engine = make_engine();
        let event = make_group_event(&"aabbccdd".repeat(8), 9, "no-such-channel");
        assert!(
            matches!(engine.evaluate(&event), Decision::Reject(msg) if msg.contains("unknown group"))
        );
    }

    #[test]
    fn gift_wrap_and_server_kinds_skip_group_check() {
        let mut engine = make_engine();
        let member = "aabbccdd".repeat(8);
        let wrap = EventInfo {
            id: "deadbeef".repeat(8),
            pubkey: "ff".repeat(32),
            kind: 1059,
            tags: vec![vec!["p".to_string(), member.clone()]],
            authed: None,
        };
        assert_eq!(engine.evaluate(&wrap), Decision::Accept);
        for kind in [0_u64, 7, 34550] {
            assert_eq!(
                engine.evaluate(&make_event(&member, kind)),
                Decision::Accept,
                "kind {kind} has no h tag"
            );
        }
    }

    #[test]
    fn encrypted_event_accepted_by_metadata() {
        let mut engine = make_engine();
        let member = "aabbccdd".repeat(8);
        // Gift wrap (kind 1059) is signed by an ephemeral key. The member is the p tag.
        let wrap = EventInfo {
            id: "deadbeef".repeat(8),
            pubkey: "ff".repeat(32),
            kind: 1059,
            tags: vec![vec!["p".to_string(), member.to_uppercase()]],
            authed: None,
        };
        assert_eq!(engine.evaluate(&wrap), Decision::Accept);

        // Seal (kind 13) is signed by the member key.
        let event = make_event(&member, 13);
        assert_eq!(engine.evaluate(&event), Decision::Accept);
    }

    #[test]
    fn gift_wrap_without_allowlisted_recipient_is_rejected() {
        let mut engine = make_engine();
        let wrap = EventInfo {
            id: "deadbeef".repeat(8),
            pubkey: "ff".repeat(32),
            kind: 1059,
            tags: vec![vec!["p".to_string(), "ab".repeat(32)]],
            authed: None,
        };
        assert!(
            matches!(engine.evaluate(&wrap), Decision::Reject(msg) if msg.contains("gift wrap"))
        );
    }

    #[test]
    fn gift_wrap_senders_do_not_share_a_recipient_bucket() {
        let config = PolicyConfig {
            rate_limit_events: 1,
            ..PolicyConfig::default()
        };
        let mut engine = PolicyEngine::new(config);
        let member = "aabbccdd".repeat(8);
        let mut allowlist = HashSet::new();
        allowlist.insert(member.clone());
        engine.update_allowlist(allowlist);

        let wrap = |author: &str, authed: Option<&str>| EventInfo {
            id: "deadbeef".repeat(8),
            pubkey: author.to_string(),
            kind: 1059,
            tags: vec![vec!["p".to_string(), member.clone()]],
            authed: authed.map(str::to_string),
        };
        let author_a = "11".repeat(32);
        let author_b = "22".repeat(32);
        assert_eq!(engine.evaluate(&wrap(&author_a, None)), Decision::Accept);
        assert!(matches!(
            engine.evaluate(&wrap(&author_a, None)),
            Decision::Reject(msg) if msg.contains("rate-limited")
        ));
        assert_eq!(
            engine.evaluate(&wrap(&author_b, None)),
            Decision::Accept,
            "a second ephemeral author must not consume the first sender's bucket"
        );

        let authed_a = "33".repeat(32);
        let authed_b = "44".repeat(32);
        assert_eq!(
            engine.evaluate(&wrap(&author_a, Some(&authed_a))),
            Decision::Accept
        );
        assert!(matches!(
            engine.evaluate(&wrap(&author_b, Some(&authed_a))),
            Decision::Reject(msg) if msg.contains("rate-limited")
        ));
        assert_eq!(
            engine.evaluate(&wrap(&author_a, Some(&authed_b))),
            Decision::Accept,
            "a second authenticated sender must not consume the first sender's bucket"
        );
    }

    #[test]
    fn write_miss_reasons_schedule_a_reload() {
        assert!(reload_on_write_miss(
            "blocked: pubkey not in member allowlist"
        ));
        assert!(reload_on_write_miss("blocked: unknown group 'team-z'"));
        assert!(reload_on_write_miss(
            "blocked: not a member of group 'team-z-officers'"
        ));
        assert!(!reload_on_write_miss(
            "blocked: event kind 12345 not allowed"
        ));
        assert!(!reload_on_write_miss("rate-limited: too many events"));
    }

    #[test]
    fn site_published_kinds_accepted() {
        let mut engine = make_engine();
        let pubkey = "aabbccdd".repeat(8);
        // Kinds site-server actually publishes, signed by the member key.
        // Kind 1059 is covered by encrypted_event_accepted_by_metadata.
        for kind in [0_u64, 1, 7, 9, 34550] {
            let event = make_event(&pubkey, kind);
            assert_eq!(
                engine.evaluate(&event),
                Decision::Accept,
                "kind {kind} should be accepted"
            );
        }
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
            assert_eq!(
                engine.evaluate(&event),
                Decision::Accept,
                "kind {kind} should be accepted"
            );
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

    #[test]
    fn sweep_drops_expired_gift_buckets() {
        let config = PolicyConfig {
            rate_limit_events: 1,
            rate_limit_window_secs: 60,
            ..PolicyConfig::default()
        };
        let mut engine = PolicyEngine::new(config);
        let member = "aabbccdd".repeat(8);
        let mut allowlist = HashSet::new();
        allowlist.insert(member.clone());
        engine.update_allowlist(allowlist);
        let wrap = EventInfo {
            id: "deadbeef".repeat(8),
            pubkey: "11".repeat(32),
            kind: 1059,
            tags: vec![vec!["p".to_string(), member]],
            authed: None,
        };
        assert_eq!(engine.evaluate(&wrap), Decision::Accept);
        assert_eq!(engine.rate_bucket_count(), 1);
        assert!(engine
            .rate_buckets
            .keys()
            .any(|key| key.starts_with("gift:")));
        engine.prune_rate_buckets();
        assert_eq!(engine.rate_bucket_count(), 1);
        assert!(matches!(
            engine.evaluate(&wrap),
            Decision::Reject(msg) if msg.contains("rate-limited")
        ));
        engine
            .prune_rate_buckets_at(std::time::Instant::now() + std::time::Duration::from_secs(61));
        assert_eq!(engine.rate_bucket_count(), 0);
        assert_eq!(engine.evaluate(&wrap), Decision::Accept);
    }
}
