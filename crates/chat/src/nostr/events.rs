//! Event construction helpers using the `nostr` crate.
//!
//! Builds properly signed Nostr events for NIP-42 AUTH, NIP-29 group messages,
//! and NIP-29 group management operations.

pub use nostr::key::Keys;
use nostr::{Event, EventBuilder as NostrEventBuilder, Kind, Tag, TagKind};

use scuffed_types::nostr::NostrEvent;

/// Errors from event construction.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("failed to build event: {0}")]
    BuildFailed(String),
    #[error("invalid key: {0}")]
    InvalidKey(String),
    #[error("signing failed: {0}")]
    SigningFailed(String),
    #[error("serialization failed: {0}")]
    SerializationFailed(String),
}

/// Event construction helper for building signed Nostr events.
pub struct EventBuilder;

impl EventBuilder {
    /// Build a NIP-42 AUTH event for relay authentication.
    ///
    /// The event contains:
    /// - kind 22242 (ephemeral)
    /// - content: empty or challenge string
    /// - tags: `["relay", <relay_url>]`, `["challenge", <challenge>]`
    pub fn build_auth_event(
        keys: &Keys,
        relay_url: &str,
        challenge: &str,
    ) -> Result<Event, EventError> {
        let event = NostrEventBuilder::new(Kind::Custom(22242), "")
            .tag(Tag::custom(
                TagKind::custom("relay"),
                vec![relay_url.to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("challenge"),
                vec![challenge.to_string()],
            ))
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))?;

        Ok(event)
    }

    /// Build a NIP-29 group chat message (kind 9).
    ///
    /// Tags: `["h", <group_id>]`, optionally `["e", <reply_to>]` for threads.
    pub fn build_group_message(
        keys: &Keys,
        group_id: &str,
        content: &str,
        reply_to: Option<&str>,
    ) -> Result<Event, EventError> {
        let mut builder = NostrEventBuilder::new(Kind::Custom(9), content).tag(Tag::custom(
            TagKind::custom("h"),
            vec![group_id.to_string()],
        ));

        if let Some(event_id) = reply_to {
            builder = builder.tag(Tag::custom(
                TagKind::custom("e"),
                vec![event_id.to_string(), String::new(), "reply".to_string()],
            ));
        }

        builder
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Build a NIP-29 group admin event for adding a user to a group (kind 9000).
    pub fn build_add_user(
        keys: &Keys,
        group_id: &str,
        user_pubkey: &str,
    ) -> Result<Event, EventError> {
        NostrEventBuilder::new(Kind::Custom(9000), "")
            .tag(Tag::custom(
                TagKind::custom("h"),
                vec![group_id.to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("p"),
                vec![user_pubkey.to_string()],
            ))
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Build a NIP-29 group admin event for removing a user (kind 9001).
    pub fn build_remove_user(
        keys: &Keys,
        group_id: &str,
        user_pubkey: &str,
    ) -> Result<Event, EventError> {
        NostrEventBuilder::new(Kind::Custom(9001), "")
            .tag(Tag::custom(
                TagKind::custom("h"),
                vec![group_id.to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("p"),
                vec![user_pubkey.to_string()],
            ))
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Build a NIP-29 group metadata event (kind 39000).
    pub fn build_group_metadata(
        keys: &Keys,
        group_id: &str,
        name: &str,
        about: Option<&str>,
        is_public: bool,
        is_open: bool,
    ) -> Result<Event, EventError> {
        let mut builder = NostrEventBuilder::new(Kind::Custom(39000), "")
            .tag(Tag::custom(
                TagKind::custom("d"),
                vec![group_id.to_string()],
            ))
            .tag(Tag::custom(TagKind::custom("name"), vec![name.to_string()]));

        if let Some(about_text) = about {
            builder = builder.tag(Tag::custom(
                TagKind::custom("about"),
                vec![about_text.to_string()],
            ));
        }

        if !is_public {
            builder = builder.tag(Tag::custom(
                TagKind::custom("private"),
                Vec::<String>::new(),
            ));
        }
        if !is_open {
            builder = builder.tag(Tag::custom(TagKind::custom("closed"), Vec::<String>::new()));
        }

        builder
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Build a NIP-09 event deletion request (kind 5).
    pub fn build_delete_event(
        keys: &Keys,
        event_ids: &[&str],
        reason: Option<&str>,
    ) -> Result<Event, EventError> {
        let mut builder = NostrEventBuilder::new(Kind::Custom(5), reason.unwrap_or(""));

        for eid in event_ids {
            builder = builder.tag(Tag::custom(TagKind::custom("e"), vec![eid.to_string()]));
        }

        builder
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Convert a `nostr::Event` to a `scuffed_types::NostrEvent`.
    pub fn to_relay_event(event: &Event) -> NostrEvent {
        NostrEvent {
            id: event.id.to_hex(),
            pubkey: event.pubkey.to_hex(),
            created_at: event.created_at.as_secs(),
            kind: event.kind.as_u16() as u32,
            tags: event
                .tags
                .iter()
                .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
                .collect(),
            content: event.content.to_string(),
            sig: event.sig.to_string(),
        }
    }

    /// Build a NIP-01 kind 0 profile metadata event.
    ///
    /// The content is a JSON string with profile fields:
    /// `{name, about, picture, nip05, banner}`
    ///
    /// Only non-`None` fields are included in the JSON. Kind 0 is a
    /// replaceable event — the relay keeps only the latest per pubkey.
    pub fn build_profile_metadata(
        keys: &Keys,
        name: &str,
        about: Option<&str>,
        picture: Option<&str>,
        nip05: Option<&str>,
        banner: Option<&str>,
    ) -> Result<Event, EventError> {
        let mut profile = serde_json::Map::new();
        profile.insert("name".into(), serde_json::Value::String(name.into()));
        if let Some(about) = about {
            profile.insert("about".into(), serde_json::Value::String(about.into()));
        }
        if let Some(picture) = picture {
            profile.insert("picture".into(), serde_json::Value::String(picture.into()));
        }
        if let Some(nip05) = nip05 {
            profile.insert("nip05".into(), serde_json::Value::String(nip05.into()));
        }
        if let Some(banner) = banner {
            profile.insert("banner".into(), serde_json::Value::String(banner.into()));
        }

        let content = serde_json::to_string(&profile)
            .map_err(|e| EventError::SerializationFailed(e.to_string()))?;

        NostrEventBuilder::new(Kind::Custom(0), &content)
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Build a NIP-72 community definition event (kind 34550).
    ///
    /// Replaceable event — the relay keeps only the latest per (pubkey, d-tag).
    /// Moderator pubkeys are tagged with the `"moderator"` role marker.
    pub fn build_community_definition(
        keys: &Keys,
        community_id: &str,
        name: &str,
        description: Option<&str>,
        rules: Option<&str>,
        image: Option<&str>,
        moderator_pubkeys: &[String],
    ) -> Result<Event, EventError> {
        let mut builder = NostrEventBuilder::new(Kind::Custom(34550), "")
            .tag(Tag::custom(
                TagKind::custom("d"),
                vec![community_id.to_string()],
            ))
            .tag(Tag::custom(TagKind::custom("name"), vec![name.to_string()]));

        if let Some(desc) = description {
            builder = builder.tag(Tag::custom(
                TagKind::custom("description"),
                vec![desc.to_string()],
            ));
        }
        if let Some(rules_text) = rules {
            builder = builder.tag(Tag::custom(
                TagKind::custom("rules"),
                vec![rules_text.to_string()],
            ));
        }
        if let Some(img) = image {
            builder = builder.tag(Tag::custom(TagKind::custom("image"), vec![img.to_string()]));
        }

        for pubkey in moderator_pubkeys {
            builder = builder.tag(Tag::custom(
                TagKind::custom("p"),
                vec![pubkey.clone(), String::new(), "moderator".to_string()],
            ));
        }

        builder
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Build a NIP-25 reaction event (kind 7).
    ///
    /// Content is `"+"` for like, `"-"` for dislike, or an emoji/custom string.
    /// Tags reference the reacted event (`e`) and its author (`p`).
    pub fn build_reaction(
        keys: &Keys,
        event_id: &str,
        event_author_pubkey: &str,
        content: &str,
    ) -> Result<Event, EventError> {
        NostrEventBuilder::new(Kind::Custom(7), content)
            .tag(Tag::custom(
                TagKind::custom("e"),
                vec![event_id.to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("p"),
                vec![event_author_pubkey.to_string()],
            ))
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Build a kind 1 community post event.
    ///
    /// Optional tags:
    /// - `["t", <hashtag>]` for each hashtag
    /// - `["a", <community_id>]` for NIP-72 community context
    /// - `["h", <group_id>]` for NIP-29 group context
    /// - `["e", <root_id>, "", "root"]` and `["e", <reply_id>, "", "reply"]` for NIP-10 threading
    pub fn build_community_post(
        keys: &Keys,
        content: &str,
        hashtags: &[String],
        community_id: Option<&str>,
        group_id: Option<&str>,
        reply_to: Option<&str>,
        root: Option<&str>,
    ) -> Result<Event, EventError> {
        let mut builder = NostrEventBuilder::new(Kind::Custom(1), content);

        for hashtag in hashtags {
            builder = builder.tag(Tag::custom(TagKind::custom("t"), vec![hashtag.to_string()]));
        }

        if let Some(cid) = community_id {
            builder = builder.tag(Tag::custom(TagKind::custom("a"), vec![cid.to_string()]));
        }

        if let Some(gid) = group_id {
            builder = builder.tag(Tag::custom(TagKind::custom("h"), vec![gid.to_string()]));
        }

        if let Some(root_id) = root {
            builder = builder.tag(Tag::custom(
                TagKind::custom("e"),
                vec![root_id.to_string(), String::new(), "root".to_string()],
            ));
        }

        if let Some(reply_id) = reply_to {
            builder = builder.tag(Tag::custom(
                TagKind::custom("e"),
                vec![reply_id.to_string(), String::new(), "reply".to_string()],
            ));
        }

        builder
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Build a custom event with arbitrary kind, content, and tags.
    ///
    /// Used for community features (LFG, match results, announcements).
    pub fn build_custom_event(
        keys: &Keys,
        kind: u16,
        content: &str,
        tags: Vec<Vec<String>>,
    ) -> Result<Event, EventError> {
        let mut builder = NostrEventBuilder::new(Kind::Custom(kind), content);

        for tag_parts in &tags {
            if tag_parts.is_empty() {
                continue;
            }
            let kind = TagKind::custom(&tag_parts[0]);
            let values: Vec<String> = tag_parts[1..].to_vec();
            builder = builder.tag(Tag::custom(kind, values));
        }

        builder
            .sign_with_keys(keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))
    }

    /// Parse a hex secret key into `nostr::Keys`.
    pub fn keys_from_hex(secret_key_hex: &str) -> Result<Keys, EventError> {
        let sk = nostr::SecretKey::from_hex(secret_key_hex)
            .map_err(|e| EventError::InvalidKey(e.to_string()))?;
        Ok(Keys::new(sk))
    }

    /// Generate a new random keypair.
    pub fn generate_keys() -> Keys {
        Keys::generate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keys() -> Keys {
        Keys::generate()
    }

    #[test]
    fn build_auth_event_has_correct_kind() {
        let keys = test_keys();
        let event =
            EventBuilder::build_auth_event(&keys, "wss://relay.example.com", "test-challenge")
                .unwrap();
        assert_eq!(event.kind, Kind::Custom(22242));
    }

    #[test]
    fn build_auth_event_has_relay_tag() {
        let keys = test_keys();
        let event =
            EventBuilder::build_auth_event(&keys, "wss://relay.example.com", "test-challenge")
                .unwrap();
        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(
            relay_event.tag_value("relay"),
            Some("wss://relay.example.com")
        );
    }

    #[test]
    fn build_auth_event_has_challenge_tag() {
        let keys = test_keys();
        let event =
            EventBuilder::build_auth_event(&keys, "wss://relay.example.com", "test-challenge")
                .unwrap();
        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.tag_value("challenge"), Some("test-challenge"));
    }

    #[test]
    fn build_auth_event_verifies() {
        let keys = test_keys();
        let event =
            EventBuilder::build_auth_event(&keys, "wss://relay.example.com", "challenge123")
                .unwrap();
        assert!(event.verify().is_ok());
    }

    #[test]
    fn build_group_message_kind_9() {
        let keys = test_keys();
        let event =
            EventBuilder::build_group_message(&keys, "team-alpha", "Hello team!", None).unwrap();
        assert_eq!(event.kind, Kind::Custom(9));
        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.group_id(), Some("team-alpha"));
        assert_eq!(relay_event.content, "Hello team!");
    }

    #[test]
    fn build_group_message_with_reply() {
        let keys = test_keys();
        let event =
            EventBuilder::build_group_message(&keys, "team-alpha", "replying", Some("abc123def"))
                .unwrap();
        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.tag_value("e"), Some("abc123def"));
    }

    #[test]
    fn build_add_user_event() {
        let keys = test_keys();
        let user_keys = Keys::generate();
        let event =
            EventBuilder::build_add_user(&keys, "team-alpha", &user_keys.public_key().to_hex())
                .unwrap();
        assert_eq!(event.kind, Kind::Custom(9000));
        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.group_id(), Some("team-alpha"));
        assert_eq!(
            relay_event.tag_value("p"),
            Some(user_keys.public_key().to_hex().as_str())
        );
    }

    #[test]
    fn build_remove_user_event() {
        let keys = test_keys();
        let event = EventBuilder::build_remove_user(&keys, "team-alpha", "deadbeef").unwrap();
        assert_eq!(event.kind, Kind::Custom(9001));
    }

    #[test]
    fn build_group_metadata_event() {
        let keys = test_keys();
        let event = EventBuilder::build_group_metadata(
            &keys,
            "team-alpha",
            "Team Alpha",
            Some("The best team"),
            true,
            false,
        )
        .unwrap();
        assert_eq!(event.kind, Kind::Custom(39000));
        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.tag_value("d"), Some("team-alpha"));
        assert_eq!(relay_event.tag_value("name"), Some("Team Alpha"));
        assert_eq!(relay_event.tag_value("about"), Some("The best team"));
        // closed group
        assert!(relay_event
            .tags
            .iter()
            .any(|t| t.first().map(|s| s.as_str()) == Some("closed")));
    }

    #[test]
    fn build_delete_event_has_kind_5() {
        let keys = test_keys();
        let event =
            EventBuilder::build_delete_event(&keys, &["event1", "event2"], Some("spam")).unwrap();
        assert_eq!(event.kind, Kind::Custom(5));
        let relay_event = EventBuilder::to_relay_event(&event);
        let e_tags = relay_event.tag_values("e");
        assert_eq!(e_tags, vec!["event1", "event2"]);
    }

    #[test]
    fn to_relay_event_roundtrip() {
        let keys = test_keys();
        let event = EventBuilder::build_group_message(&keys, "general", "test msg", None).unwrap();
        let relay_event = EventBuilder::to_relay_event(&event);

        assert_eq!(relay_event.id, event.id.to_hex());
        assert_eq!(relay_event.pubkey, event.pubkey.to_hex());
        assert_eq!(relay_event.kind, event.kind.as_u16() as u32);
        assert!(relay_event.is_group_chat());
    }

    #[test]
    fn keys_from_hex_roundtrip() {
        let original = Keys::generate();
        let hex = original.secret_key().to_secret_hex();
        let restored = EventBuilder::keys_from_hex(&hex).unwrap();
        assert_eq!(
            original.public_key().to_hex(),
            restored.public_key().to_hex()
        );
    }

    #[test]
    fn keys_from_hex_invalid() {
        assert!(EventBuilder::keys_from_hex("not_a_valid_hex_key").is_err());
    }

    #[test]
    fn build_community_definition_kind_34550() {
        let keys = test_keys();
        let mod1 = Keys::generate();
        let mod2 = Keys::generate();
        let mods = vec![mod1.public_key().to_hex(), mod2.public_key().to_hex()];

        let event = EventBuilder::build_community_definition(
            &keys,
            "scuffed-crew",
            "Scuffed Crew",
            Some("A gaming community"),
            Some("Be respectful"),
            Some("https://scuffed.gg/logo.png"),
            &mods,
        )
        .unwrap();

        assert_eq!(event.kind, Kind::Custom(34550));
        assert!(event.verify().is_ok());

        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.tag_value("d"), Some("scuffed-crew"));
        assert_eq!(relay_event.tag_value("name"), Some("Scuffed Crew"));
        assert_eq!(
            relay_event.tag_value("description"),
            Some("A gaming community")
        );
        assert_eq!(relay_event.tag_value("rules"), Some("Be respectful"));
        assert_eq!(
            relay_event.tag_value("image"),
            Some("https://scuffed.gg/logo.png")
        );

        let p_tags = relay_event.tag_values("p");
        assert_eq!(p_tags.len(), 2);
    }

    #[test]
    fn build_community_definition_minimal() {
        let keys = test_keys();
        let event = EventBuilder::build_community_definition(
            &keys,
            "my-community",
            "My Community",
            None,
            None,
            None,
            &[],
        )
        .unwrap();

        assert_eq!(event.kind, Kind::Custom(34550));
        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.tag_value("d"), Some("my-community"));
        assert_eq!(relay_event.tag_value("name"), Some("My Community"));
        assert!(relay_event.tag_value("description").is_none());
        assert!(relay_event.tag_value("rules").is_none());
        assert!(relay_event.tag_value("image").is_none());
        assert!(relay_event.tag_values("p").is_empty());
    }

    #[test]
    fn build_reaction_like() {
        let keys = test_keys();
        let event =
            EventBuilder::build_reaction(&keys, "abcdef1234567890", "target_author_pubkey", "+")
                .unwrap();

        assert_eq!(event.kind, Kind::Custom(7));
        assert_eq!(event.content, "+");
        assert!(event.verify().is_ok());

        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.tag_value("e"), Some("abcdef1234567890"));
        assert_eq!(relay_event.tag_value("p"), Some("target_author_pubkey"));
    }

    #[test]
    fn build_reaction_emoji() {
        let keys = test_keys();
        let event =
            EventBuilder::build_reaction(&keys, "event123", "author456", "\u{1f525}").unwrap();

        assert_eq!(event.kind, Kind::Custom(7));
        assert_eq!(event.content, "\u{1f525}");
    }

    #[test]
    fn build_community_post_basic() {
        let keys = test_keys();
        let event = EventBuilder::build_community_post(
            &keys,
            "hello community",
            &[],
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(event.kind, Kind::Custom(1));
        assert_eq!(event.content, "hello community");
        assert!(event.verify().is_ok());

        let relay_event = EventBuilder::to_relay_event(&event);
        assert!(relay_event.tags.is_empty());
    }

    #[test]
    fn build_community_post_with_hashtags() {
        let keys = test_keys();
        let hashtags = vec!["rust".to_string(), "nostr".to_string()];
        let event = EventBuilder::build_community_post(
            &keys,
            "shipping backend work",
            &hashtags,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.tag_values("t"), vec!["rust", "nostr"]);
    }

    #[test]
    fn build_community_post_with_reply_markers() {
        let keys = test_keys();
        let event = EventBuilder::build_community_post(
            &keys,
            "thread reply",
            &[],
            None,
            None,
            Some("reply-event-id"),
            Some("root-event-id"),
        )
        .unwrap();

        let relay_event = EventBuilder::to_relay_event(&event);
        assert!(relay_event.tags.iter().any(|tag| {
            tag.first().map(|s| s.as_str()) == Some("e")
                && tag.get(1).map(|s| s.as_str()) == Some("root-event-id")
                && tag.get(2).map(|s| s.as_str()) == Some("")
                && tag.get(3).map(|s| s.as_str()) == Some("root")
        }));
        assert!(relay_event.tags.iter().any(|tag| {
            tag.first().map(|s| s.as_str()) == Some("e")
                && tag.get(1).map(|s| s.as_str()) == Some("reply-event-id")
                && tag.get(2).map(|s| s.as_str()) == Some("")
                && tag.get(3).map(|s| s.as_str()) == Some("reply")
        }));
    }

    #[test]
    fn build_community_post_with_community_context() {
        let keys = test_keys();
        let event = EventBuilder::build_community_post(
            &keys,
            "post in community feed",
            &[],
            Some("34550:scuffed:crew"),
            Some("team-alpha"),
            None,
            None,
        )
        .unwrap();

        let relay_event = EventBuilder::to_relay_event(&event);
        assert_eq!(relay_event.tag_value("a"), Some("34550:scuffed:crew"));
        assert_eq!(relay_event.tag_value("h"), Some("team-alpha"));
    }

    #[test]
    fn build_profile_metadata_kind_0() {
        let keys = test_keys();
        let event = EventBuilder::build_profile_metadata(
            &keys,
            "TestUser",
            Some("A test user"),
            Some("https://example.com/avatar.png"),
            Some("testuser@scuffed.gg"),
            None,
        )
        .unwrap();
        assert_eq!(event.kind, Kind::Custom(0));
        assert!(event.verify().is_ok());

        let content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert_eq!(content["name"], "TestUser");
        assert_eq!(content["about"], "A test user");
        assert_eq!(content["picture"], "https://example.com/avatar.png");
        assert_eq!(content["nip05"], "testuser@scuffed.gg");
        assert!(content.get("banner").is_none());
    }

    #[test]
    fn build_profile_metadata_minimal() {
        let keys = test_keys();
        let event =
            EventBuilder::build_profile_metadata(&keys, "MinimalUser", None, None, None, None)
                .unwrap();
        assert_eq!(event.kind, Kind::Custom(0));

        let content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert_eq!(content["name"], "MinimalUser");
        // Only name should be present
        assert!(content.get("about").is_none());
        assert!(content.get("picture").is_none());
        assert!(content.get("nip05").is_none());
    }

    #[test]
    fn build_profile_metadata_with_banner() {
        let keys = test_keys();
        let event = EventBuilder::build_profile_metadata(
            &keys,
            "BannerUser",
            None,
            None,
            None,
            Some("https://example.com/banner.jpg"),
        )
        .unwrap();

        let content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert_eq!(content["banner"], "https://example.com/banner.jpg");
    }
}
