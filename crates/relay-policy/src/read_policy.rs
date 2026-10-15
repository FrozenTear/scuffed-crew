//! Who may receive a stored event on a subscription.
//!
//! strfry and strfry29 only run on the write path (stdin JSON, stdout
//! accept/reject). They cannot hide an event that is already stored. This
//! module is the read rule the WebSocket gate applies to every `EVENT` frame
//! before it is forwarded to the client.
//!
//! Direct messages are NIP-59 gift wraps (kind 1059), not kind 4. See
//! `EncryptionService::build_gift_wraps` and `POST /api/nostr/dm/send`. A kind
//! 4 event is therefore not a member inbox and is not delivered. The write
//! plugin rejects kind 4 as well.
//!
//! NIP-29 group metadata (kinds 39000 through 39003) is addressable by a `d`
//! tag, not an `h` tag. `GroupManager::create_group` publishes kind 39000 that
//! way. Those events follow the same team and officer scope as group chat.

use std::collections::{HashMap, HashSet};

/// NIP-59 gift wrap. Outer `p` tag is the recipient. The author pubkey is an
/// ephemeral key, so authorship is not access.
pub const GIFT_WRAP_KIND: u64 = 1059;

/// NIP-04 style plaintext-to-the-relay DM. This codebase does not send these.
pub const PLAIN_DM_KIND: u64 = 4;

/// NIP-29 replaceable group metadata, admins, members, and roles.
/// The group id is the `d` tag.
pub const GROUP_METADATA_KIND_MIN: u64 = 39000;
pub const GROUP_METADATA_KIND_MAX: u64 = 39003;

/// One active member the gate will serve after NIP-42 AUTH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberAcl {
    pub pubkey: String,
    pub member_id: String,
    /// Org role admin or officer (`OrgRole::can_access_officer_channel`).
    pub officer: bool,
    pub teams: HashSet<String>,
}

/// A NIP-29 group id the relay knows about, including deactivated channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupAcl {
    pub team_id: String,
    pub officer_only: bool,
}

/// Snapshot of members and channels. Refresh replaces the whole value.
#[derive(Debug, Clone, Default)]
pub struct ReadDirectory {
    readers: HashMap<String, MemberAcl>,
    groups: HashMap<String, GroupAcl>,
}

impl ReadDirectory {
    pub fn from_parts(
        members: impl IntoIterator<Item = MemberAcl>,
        groups: impl IntoIterator<Item = (String, GroupAcl)>,
    ) -> Self {
        let mut readers = HashMap::new();
        for mut member in members {
            member.pubkey = member.pubkey.trim().to_ascii_lowercase();
            if member.pubkey.len() != 64 || !member.pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            readers.insert(member.pubkey.clone(), member);
        }
        let groups = groups
            .into_iter()
            .map(|(id, rule)| (id.trim().to_string(), rule))
            .filter(|(id, rule)| !id.is_empty() && !rule.team_id.is_empty())
            .collect();
        Self { readers, groups }
    }

    pub fn reader_count(&self) -> usize {
        self.readers.len()
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub fn reader(&self, pubkey: &str) -> Option<&MemberAcl> {
        self.readers
            .get(pubkey.trim().to_ascii_lowercase().as_str())
    }
}

/// Minimal event fields. Content is never inspected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventView {
    pub pubkey: String,
    pub kind: u64,
    pub tags: Vec<Vec<String>>,
}

impl EventView {
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let pubkey = value.get("pubkey")?.as_str()?.trim().to_ascii_lowercase();
        let kind = value.get("kind")?.as_u64()?;
        let tags = value
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|tags| {
                tags.iter()
                    .filter_map(|tag| {
                        let arr = tag.as_array()?;
                        Some(
                            arr.iter()
                                .filter_map(|part| part.as_str().map(str::to_string))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self { pubkey, kind, tags })
    }

    fn tag_values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.tags.iter().filter_map(move |tag| {
            if tag.first().map(String::as_str) == Some(name) {
                tag.get(1).map(String::as_str)
            } else {
                None
            }
        })
    }

    /// First non-empty `h` tag (NIP-29 group chat id).
    pub fn group_id(&self) -> Option<&str> {
        self.first_tag("h")
    }

    /// First non-empty `d` tag (NIP-29 addressable group id).
    pub fn address_id(&self) -> Option<&str> {
        self.first_tag("d")
    }

    fn first_tag<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        self.tag_values(name)
            .map(str::trim)
            .find(|value| !value.is_empty())
    }

    fn p_tag_matches(&self, pubkey: &str) -> bool {
        self.tag_values("p")
            .any(|p| p.trim().eq_ignore_ascii_case(pubkey))
    }
}

/// Why an event stays on the relay side of the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadDeny {
    /// No completed NIP-42 AUTH on this connection.
    SignedOut,
    /// AUTH pubkey is not an active member.
    NotMember,
    /// Kind 1059 whose `p` tag is not this member.
    GiftWrapNotAddressed,
    /// Public team channel, and this member is not on that team.
    TeamChat,
    /// Officer channel, and this member is not an officer on that team.
    OfficerChat,
    /// Anything else (kind 1, kind 4, unknown group, no group).
    NotShared,
}

impl std::fmt::Display for ReadDeny {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ReadDeny::SignedOut => "signed out",
            ReadDeny::NotMember => "not an active member",
            ReadDeny::GiftWrapNotAddressed => "gift wrap p-tag is not this member",
            ReadDeny::TeamChat => "team chat is limited to team members",
            ReadDeny::OfficerChat => "officer chat is limited to officers on that team",
            ReadDeny::NotShared => "event is not shared with this member",
        })
    }
}

/// `authed_pubkey` is `None` until NIP-42 AUTH succeeds.
///
/// Rules, in order:
/// 1. No AUTH: no event of any kind.
/// 2. AUTH pubkey must be an active member in the directory.
/// 3. Kind 1059: deliver only when a `p` tag is that member's key.
/// 4. Kinds 39000 through 39003: the `d` tag is the group id. Public groups go
///    to members of that team. Officer groups go to officers on that team.
///    A missing or unknown `d` tag is dropped.
/// 5. An `h` tag for a public team channel: deliver only to members of that team.
/// 6. An `h` tag for an officer channel: deliver only to officers (admin or
///    officer) who are on that team.
/// 7. Anything else, including kind 4, is dropped.
pub fn allow_event(
    authed_pubkey: Option<&str>,
    event: &EventView,
    dir: &ReadDirectory,
) -> Result<(), ReadDeny> {
    let Some(pubkey) = authed_pubkey.map(str::trim).filter(|p| !p.is_empty()) else {
        return Err(ReadDeny::SignedOut);
    };
    let Some(reader) = dir.reader(pubkey) else {
        return Err(ReadDeny::NotMember);
    };

    if event.kind == GIFT_WRAP_KIND {
        return if event.p_tag_matches(&reader.pubkey) {
            Ok(())
        } else {
            Err(ReadDeny::GiftWrapNotAddressed)
        };
    }

    if is_group_metadata(event.kind) {
        let Some(group_id) = event.address_id() else {
            return Err(ReadDeny::NotShared);
        };
        return allow_group(reader, group_id, dir);
    }

    if let Some(group_id) = event.group_id() {
        return allow_group(reader, group_id, dir);
    }

    Err(ReadDeny::NotShared)
}

fn is_group_metadata(kind: u64) -> bool {
    (GROUP_METADATA_KIND_MIN..=GROUP_METADATA_KIND_MAX).contains(&kind)
}

fn allow_group(reader: &MemberAcl, group_id: &str, dir: &ReadDirectory) -> Result<(), ReadDeny> {
    let Some(group) = dir.groups.get(group_id) else {
        return Err(ReadDeny::NotShared);
    };
    let on_team = reader.teams.contains(&group.team_id);
    if group.officer_only {
        return if reader.officer && on_team {
            Ok(())
        } else {
            Err(ReadDeny::OfficerChat)
        };
    }
    if on_team {
        Ok(())
    } else {
        Err(ReadDeny::TeamChat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pk(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn member(byte: u8, officer: bool, teams: &[&str]) -> MemberAcl {
        MemberAcl {
            pubkey: pk(byte),
            member_id: format!("m{byte}"),
            officer,
            teams: teams.iter().map(|t| (*t).to_string()).collect(),
        }
    }

    fn directory() -> ReadDirectory {
        ReadDirectory::from_parts(
            [
                member(0x11, false, &["team-a"]),
                member(0x22, true, &["team-a"]),
                member(0x33, true, &["team-b"]),
                member(0x44, false, &[]),
            ],
            [
                (
                    "team-a".into(),
                    GroupAcl {
                        team_id: "team-a".into(),
                        officer_only: false,
                    },
                ),
                (
                    "team-a-officers".into(),
                    GroupAcl {
                        team_id: "team-a".into(),
                        officer_only: true,
                    },
                ),
                (
                    "team-b".into(),
                    GroupAcl {
                        team_id: "team-b".into(),
                        officer_only: false,
                    },
                ),
            ],
        )
    }

    fn event(author: &str, kind: u64, tags: Vec<Vec<&str>>) -> EventView {
        EventView {
            pubkey: author.to_string(),
            kind,
            tags: tags
                .into_iter()
                .map(|tag| tag.into_iter().map(str::to_string).collect())
                .collect(),
        }
    }

    fn gift(author: &str, recipient: &str) -> EventView {
        event(author, GIFT_WRAP_KIND, vec![vec!["p", recipient]])
    }

    fn team_chat(group: &str) -> EventView {
        event(&pk(0x99), 9, vec![vec!["h", group]])
    }

    #[test]
    fn signed_out_receives_no_events_of_any_kind() {
        let dir = directory();
        let samples = [
            gift(&pk(0x11), &pk(0x11)),
            team_chat("team-a"),
            team_chat("team-a-officers"),
            event(&pk(0x11), 1, vec![]),
            event(&pk(0x11), PLAIN_DM_KIND, vec![vec!["p", &pk(0x11)]]),
        ];
        for event in samples {
            assert_eq!(
                allow_event(None, &event, &dir),
                Err(ReadDeny::SignedOut),
                "kind {}",
                event.kind
            );
            assert_eq!(
                allow_event(Some(""), &event, &dir),
                Err(ReadDeny::SignedOut)
            );
        }
    }

    #[test]
    fn member_receives_only_gift_wraps_p_tagged_to_self() {
        let dir = directory();
        let me = pk(0x11);
        let other = pk(0x22);
        assert_eq!(allow_event(Some(&me), &gift(&pk(0x99), &me), &dir), Ok(()));
        assert_eq!(
            allow_event(Some(&me), &gift(&pk(0x99), &other), &dir),
            Err(ReadDeny::GiftWrapNotAddressed)
        );
        assert_eq!(
            allow_event(Some(&me), &gift(&me, &other), &dir),
            Err(ReadDeny::GiftWrapNotAddressed),
            "ephemeral author match is not access"
        );
        let untagged = event(&pk(0x99), GIFT_WRAP_KIND, vec![]);
        assert_eq!(
            allow_event(Some(&me), &untagged, &dir),
            Err(ReadDeny::GiftWrapNotAddressed)
        );
    }

    #[test]
    fn gift_wrap_p_tag_match_is_case_insensitive() {
        let dir = directory();
        let me = pk(0x11);
        let upper = me.to_ascii_uppercase();
        assert_eq!(
            allow_event(Some(&upper), &gift(&pk(0x99), &upper), &dir),
            Ok(())
        );
    }

    #[test]
    fn member_without_a_team_still_receives_own_dm_gift_wrap() {
        let dir = directory();
        let me = pk(0x44);
        assert_eq!(allow_event(Some(&me), &gift(&pk(0x99), &me), &dir), Ok(()));
        assert_eq!(
            allow_event(Some(&me), &team_chat("team-a"), &dir),
            Err(ReadDeny::TeamChat)
        );
    }

    #[test]
    fn unknown_authed_pubkey_receives_nothing() {
        let dir = directory();
        let stranger = pk(0x55);
        assert_eq!(
            allow_event(Some(&stranger), &gift(&pk(0x99), &stranger), &dir),
            Err(ReadDeny::NotMember)
        );
        assert_eq!(
            allow_event(Some(&stranger), &team_chat("team-a"), &dir),
            Err(ReadDeny::NotMember)
        );
    }

    #[test]
    fn team_chat_only_for_members_of_that_team() {
        let dir = directory();
        let on_team = pk(0x11);
        let other_team_officer = pk(0x33);
        assert_eq!(
            allow_event(Some(&on_team), &team_chat("team-a"), &dir),
            Ok(())
        );
        assert_eq!(
            allow_event(Some(&other_team_officer), &team_chat("team-a"), &dir),
            Err(ReadDeny::TeamChat)
        );
        assert_eq!(
            allow_event(Some(&on_team), &team_chat("team-b"), &dir),
            Err(ReadDeny::TeamChat)
        );
        let thread = event(&pk(0x99), 11, vec![vec!["h", "team-a"]]);
        assert_eq!(allow_event(Some(&on_team), &thread, &dir), Ok(()));
    }

    #[test]
    fn officer_chat_only_for_officers_on_that_team() {
        let dir = directory();
        let member = pk(0x11);
        let officer = pk(0x22);
        let other_officer = pk(0x33);
        let chat = team_chat("team-a-officers");
        assert_eq!(allow_event(Some(&officer), &chat, &dir), Ok(()));
        assert_eq!(
            allow_event(Some(&member), &chat, &dir),
            Err(ReadDeny::OfficerChat)
        );
        assert_eq!(
            allow_event(Some(&other_officer), &chat, &dir),
            Err(ReadDeny::OfficerChat)
        );
        let metadata = event(
            &pk(0x99),
            39000,
            vec![vec!["h", "team-a-officers"], vec!["d", "team-a-officers"]],
        );
        assert_eq!(
            allow_event(Some(&member), &metadata, &dir),
            Err(ReadDeny::OfficerChat)
        );
        assert_eq!(allow_event(Some(&officer), &metadata, &dir), Ok(()));
    }

    #[test]
    fn kind4_plain_dm_is_not_delivered_to_author_or_recipient() {
        // DMs are built by EncryptionService::build_gift_wraps (kind 1059).
        // A kind 4 event is not that path, so author and recipient do not get a
        // special inbox for it.
        let dir = directory();
        let author = pk(0x11);
        let recipient = pk(0x22);
        let bystander = pk(0x33);
        let plain = event(&author, PLAIN_DM_KIND, vec![vec!["p", &recipient]]);
        assert_eq!(
            allow_event(Some(&author), &plain, &dir),
            Err(ReadDeny::NotShared)
        );
        assert_eq!(
            allow_event(Some(&recipient), &plain, &dir),
            Err(ReadDeny::NotShared)
        );
        assert_eq!(
            allow_event(Some(&bystander), &plain, &dir),
            Err(ReadDeny::NotShared)
        );
    }

    #[test]
    fn group_metadata_d_tag_is_scoped_like_the_group() {
        let dir = directory();
        let member = pk(0x11);
        let officer = pk(0x22);
        let other = pk(0x33);
        for kind in GROUP_METADATA_KIND_MIN..=GROUP_METADATA_KIND_MAX {
            let public = event(
                &pk(0x99),
                kind,
                vec![vec!["d", "team-a"], vec!["name", "Alpha"]],
            );
            assert_eq!(allow_event(Some(&member), &public, &dir), Ok(()), "{kind}");
            assert_eq!(
                allow_event(Some(&other), &public, &dir),
                Err(ReadDeny::TeamChat),
                "{kind}"
            );
            let officers = event(&pk(0x99), kind, vec![vec!["d", "team-a-officers"]]);
            assert_eq!(
                allow_event(Some(&officer), &officers, &dir),
                Ok(()),
                "{kind}"
            );
            assert_eq!(
                allow_event(Some(&member), &officers, &dir),
                Err(ReadDeny::OfficerChat),
                "{kind}"
            );
            let missing = event(&pk(0x99), kind, vec![vec!["h", "team-a"]]);
            assert_eq!(
                allow_event(Some(&member), &missing, &dir),
                Err(ReadDeny::NotShared),
                "metadata without a d tag is not group chat"
            );
            let unknown = event(&pk(0x99), kind, vec![vec!["d", "no-such-group"]]);
            assert_eq!(
                allow_event(Some(&member), &unknown, &dir),
                Err(ReadDeny::NotShared)
            );
        }
    }

    #[test]
    fn kind1_and_unknown_groups_are_not_shared() {
        let dir = directory();
        let me = pk(0x11);
        assert_eq!(
            allow_event(Some(&me), &event(&me, 1, vec![]), &dir),
            Err(ReadDeny::NotShared)
        );
        assert_eq!(
            allow_event(Some(&me), &team_chat("no-such-group"), &dir),
            Err(ReadDeny::NotShared)
        );
    }

    #[test]
    fn short_pubkey_is_left_out_of_the_directory() {
        let dir = ReadDirectory::from_parts(
            [MemberAcl {
                pubkey: "abcd".into(),
                member_id: "m".into(),
                officer: true,
                teams: HashSet::new(),
            }],
            std::iter::empty(),
        );
        assert_eq!(dir.reader_count(), 0);
    }
}
