//! The Competitive queue a game was played in (`rank.queue` on the upload).
//!
//! Built from the pre-game title, the board's team size, and, when the
//! title is missing, the rank screen's role line. A failed or incomplete
//! read is [`RankQueue::Unknown`]; it never drops the game.

use scuffed_types::api::{RankQueue, RankRole};

/// 5 or 6 players per team, as the Tab board gave it. Any other count is
/// stored as `None`, so a bad detection cannot look like a real size.
pub fn stored_team_size(team_size: usize) -> Option<u8> {
    matches!(team_size, 5 | 6).then_some(team_size as u8)
}

/// The queue the upload sends for one Competitive game.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecidedQueue {
    pub value: RankQueue,
    /// How sure the input that decided `value` was (the title, or the rank
    /// role line). `None` when the queue is unknown, or that confidence is
    /// not a number the site keeps.
    pub confidence: Option<f32>,
}

impl DecidedQueue {
    fn known(value: RankQueue, confidence: Option<f32>) -> Self {
        Self { value, confidence }
    }

    fn unknown() -> Self {
        Self {
            value: RankQueue::Unknown,
            confidence: None,
        }
    }
}

/// Where one Competitive game's rank belongs on this PC: its queue and,
/// when the rank screen showed one the upload keeps, its role. Ranks are
/// only ever compared or grouped within one slot ([`RankSlot::same_slot`]),
/// so a 6v6 role queue Tank rank never mixes with a 5v5 role queue Tank
/// rank or an open queue rank, and a game whose queue could not be read
/// stays in the `Unknown` queue instead of joining a known one.
///
/// The same queue and role the upload sends as `rank.queue` and
/// `rank.role` for that game (the tracker's `sync::rank_slot` takes both
/// from the upload's own `rank` object). Derived from the stored title,
/// team size and rank screen read whenever a game is built, never stored,
/// so it follows a later Tab that fills in the team size.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RankSlot {
    pub queue: RankQueue,
    /// How sure the input that decided `queue` was ([`DecidedQueue`]).
    /// `None` when the queue is unknown.
    #[serde(default)]
    pub queue_confidence: Option<f32>,
    /// `None` when no role line the upload keeps was read.
    #[serde(default)]
    pub role: Option<RankRole>,
    /// The upload's `rank` object holds a field read on the rank screen
    /// (the tracker's `UploadRank::has_rank_screen_field`), not only the
    /// title's queue. A rank screen field the upload drops (an `Open` role
    /// line with no open queue title, a value the site would null) does not
    /// count, so a game counts as ranked here exactly when the site gets a
    /// rank screen field for it. Plays no part in [`RankSlot::same_slot`].
    #[serde(default)]
    pub rank_read: bool,
}

impl RankSlot {
    /// Same queue and same role: the two games' ranks may be compared.
    /// The confidence plays no part.
    pub fn same_slot(&self, other: &Self) -> bool {
        self.queue == other.queue && self.role == other.role
    }
}

/// Words of a title or role line: upper case, each run of letters or
/// digits as one word. `6v6 Role Queue`, `6V6-ROLE-QUEUE` and
/// `COMPETITIVE: 6V6 OPEN QUEUE` all split the same way.
pub fn title_words(text: &str) -> Vec<String> {
    text.to_uppercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueKind {
    Role,
    Open,
}

fn kind_in(words: &[String]) -> Option<QueueKind> {
    let role = words.iter().any(|w| w == "ROLE");
    let open = words.iter().any(|w| w == "OPEN");
    match (role, open) {
        (true, false) => Some(QueueKind::Role),
        (false, true) => Some(QueueKind::Open),
        _ => None,
    }
}

/// Title words of a mode that is not Competitive. A title with any of
/// them is never Competitive evidence, even when it also names ROLE or
/// OPEN (`QUICK PLAY: ROLE QUEUE`), so the upload sends no `rank` object
/// and no `rank.queue` for it (#889: Competitive only).
const NOT_COMPETITIVE: [&str; 5] = ["QUICK", "UNRANKED", "ARCADE", "STADIUM", "CUSTOM"];

fn names_not_competitive(words: &[String]) -> bool {
    words.iter().any(|w| NOT_COMPETITIVE.contains(&w.as_str()))
}

/// The title names a mode that is not Competitive: QUICK, UNRANKED,
/// ARCADE, STADIUM or CUSTOM as a word (any word order, case and
/// punctuation).
pub fn title_rules_out_competitive(text: &str) -> bool {
    names_not_competitive(&title_words(text))
}

/// The title names ROLE or OPEN (any word order, case and punctuation),
/// and not both, and no mode that is not Competitive
/// ([`title_rules_out_competitive`]). Used to decide whether a title-only
/// upload is Competitive evidence.
pub fn title_names_role_or_open(text: &str) -> bool {
    let words = title_words(text);
    kind_in(&words).is_some() && !names_not_competitive(&words)
}

/// The title names OPEN and not ROLE, with the same word reader as
/// [`title_names_role_or_open`].
pub fn title_is_open(text: &str) -> bool {
    kind_in(&title_words(text)) == Some(QueueKind::Open)
}

/// The title names ROLE and not OPEN, with the same word reader as
/// [`title_is_open`].
pub fn title_is_role(text: &str) -> bool {
    kind_in(&title_words(text)) == Some(QueueKind::Role)
}

/// The team size word in the title: `Some(None)` when it has none, and
/// `None` when it names both `5V5` and `6V6`, which contradicts itself.
fn size_in(words: &[String]) -> Option<Option<u8>> {
    let has5 = words.iter().any(|w| w == "5V5");
    let has6 = words.iter().any(|w| w == "6V6");
    match (has5, has6) {
        (true, true) => None,
        (true, false) => Some(Some(5)),
        (false, true) => Some(Some(6)),
        (false, false) => Some(None),
    }
}

fn board_size(team_size: Option<u8>) -> Option<u8> {
    team_size.filter(|&n| n == 5 || n == 6)
}

fn usable_confidence(confidence: Option<f32>) -> Option<f32> {
    confidence.filter(|c| c.is_finite() && *c > 0.0 && *c <= 1.0)
}

/// Parse a rank-screen role line as [`RankRole`]: only the exact serde
/// spellings `Tank`, `Damage`, `Support`, and `Open`.
pub fn parse_rank_role(role: &str) -> Option<RankRole> {
    serde_json::from_value(serde_json::Value::String(role.to_string())).ok()
}

/// The rank screen role line names a role icon (Tank, Damage or Support),
/// parsed the same way the upload maps `rank.role` ([`parse_rank_role`]).
/// `Open` is not one: the reader takes it from an empty role band, which
/// another screen state can show too, so the upload sends it as the role
/// only under an open queue title, and it decides no queue by itself.
fn role_line_is_icon(role: &str) -> bool {
    matches!(
        parse_rank_role(role),
        Some(RankRole::Tank | RankRole::Damage | RankRole::Support)
    )
}

fn from_kind_and_size(kind: QueueKind, size: u8, confidence: Option<f32>) -> Option<DecidedQueue> {
    let value = match (kind, size) {
        (QueueKind::Role, 5) => RankQueue::Role5v5,
        (QueueKind::Role, 6) => RankQueue::Role6v6,
        (QueueKind::Open, 5) => RankQueue::Open5v5,
        (QueueKind::Open, 6) => RankQueue::Open6v6,
        _ => return None,
    };
    Some(DecidedQueue::known(value, usable_confidence(confidence)))
}

/// Decide `rank.queue` from the confident title (any word order, case and
/// punctuation), the board's team size, and, when the title does not name
/// a queue, the rank screen's role line on a 6-player board.
///
/// Rules:
/// - Title names ROLE and the size is 6: `role_6v6`. With 5: `role_5v5`.
/// - Title names OPEN and the size is 6: `open_6v6`. With 5: `open_5v5`.
/// - Size comes from the board when it is 5 or 6, else from a `5V5` or
///   `6V6` word in the title. If those two disagree, or the title names
///   both, the queue is unknown.
/// - No confident title and 6 per team: a sure Tank, Damage or Support
///   role line is `role_6v6`. An `Open` role line decides nothing.
/// - A title that names a mode that is not Competitive
///   ([`title_rules_out_competitive`]) is unknown; the upload sends no
///   queue for it at all.
/// - Anything else is [`RankQueue::Unknown`].
pub fn decide_rank_queue(
    title: Option<&str>,
    title_confidence: Option<f32>,
    team_size: Option<u8>,
    rank_role: Option<&str>,
    rank_role_confidence: Option<f32>,
) -> DecidedQueue {
    let words = title.map(title_words).unwrap_or_default();
    if names_not_competitive(&words) {
        return DecidedQueue::unknown();
    }
    let kind = kind_in(&words);
    let Some(title_size) = size_in(&words) else {
        return DecidedQueue::unknown();
    };
    let board = board_size(team_size);
    let size = match (board, title_size) {
        (Some(a), Some(b)) if a != b => return DecidedQueue::unknown(),
        (Some(n), _) | (None, Some(n)) => Some(n),
        (None, None) => None,
    };

    if let (Some(kind), Some(size)) = (kind, size)
        && let Some(decided) = from_kind_and_size(kind, size, title_confidence)
    {
        return decided;
    }

    // Title named a queue but no size, or named both ROLE and OPEN: stop.
    // The role-line fallback is only for a missing title, not a partial one.
    if kind.is_some() || title.is_some_and(|t| !t.trim().is_empty()) {
        return DecidedQueue::unknown();
    }

    if size == Some(6)
        && usable_confidence(rank_role_confidence).is_some()
        && rank_role.is_some_and(role_line_is_icon)
        && let Some(decided) = from_kind_and_size(QueueKind::Role, 6, rank_role_confidence)
    {
        return decided;
    }

    DecidedQueue::unknown()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(
        title: Option<&str>,
        size: Option<u8>,
        role: Option<&str>,
    ) -> (RankQueue, Option<f32>) {
        let d = decide_rank_queue(title, Some(0.93), size, role, Some(0.8));
        (d.value, d.confidence)
    }

    #[test]
    fn title_and_board_size_map_to_each_queue() {
        let cases = [
            (Some("ROLE QUEUE"), Some(5), RankQueue::Role5v5),
            (Some("ROLE QUEUE"), Some(6), RankQueue::Role6v6),
            (Some("OPEN QUEUE"), Some(5), RankQueue::Open5v5),
            (Some("OPEN QUEUE"), Some(6), RankQueue::Open6v6),
            (Some("6V6 ROLE QUEUE"), Some(6), RankQueue::Role6v6),
            (Some("ROLE QUEUE 6V6"), None, RankQueue::Role6v6),
            (Some("6v6 Role Queue"), None, RankQueue::Role6v6),
            (
                Some("COMPETITIVE: 6V6 OPEN QUEUE"),
                None,
                RankQueue::Open6v6,
            ),
            (Some("Open Queue"), Some(5), RankQueue::Open5v5),
            (Some("COMPETITIVE: OPEN QUEUE"), Some(5), RankQueue::Open5v5),
        ];
        for (title, size, want) in cases {
            let (value, confidence) = decide(title, size, None);
            assert_eq!(value, want, "{title:?} size={size:?}");
            assert_eq!(confidence, Some(0.93), "{title:?} size={size:?}");
        }
    }

    #[test]
    fn a_title_without_a_size_is_unknown() {
        for title in ["ROLE QUEUE", "OPEN QUEUE", "ROLE", "OPEN"] {
            let (value, confidence) = decide(Some(title), None, Some("Tank"));
            assert_eq!(value, RankQueue::Unknown, "{title}");
            assert_eq!(confidence, None, "{title}");
        }
    }

    #[test]
    fn board_and_title_sizes_that_disagree_are_unknown() {
        // Both ways round (board below and above the title's size), for
        // ROLE and OPEN.
        for (title, board) in [
            ("6V6 ROLE QUEUE", 5),
            ("5V5 ROLE QUEUE", 6),
            ("6V6 OPEN QUEUE", 5),
            ("5V5 OPEN QUEUE", 6),
        ] {
            let (value, confidence) = decide(Some(title), Some(board), None);
            assert_eq!(value, RankQueue::Unknown, "{title} board={board}");
            assert_eq!(confidence, None, "{title} board={board}");
        }
    }

    #[test]
    fn a_title_that_names_both_team_sizes_is_unknown() {
        // A title with both size words contradicts itself: the board does
        // not pick one.
        for title in ["5V5 6V6 ROLE QUEUE", "6V6 OPEN QUEUE 5V5"] {
            for board in [Some(5), Some(6), None] {
                let (value, confidence) = decide(Some(title), board, Some("Tank"));
                assert_eq!(value, RankQueue::Unknown, "{title} board={board:?}");
                assert_eq!(confidence, None, "{title} board={board:?}");
            }
        }
    }

    #[test]
    fn no_title_and_six_a_side_uses_the_rank_role_line() {
        for role in ["Tank", "Damage", "Support"] {
            let (value, confidence) = decide(None, Some(6), Some(role));
            assert_eq!(value, RankQueue::Role6v6, "{role}");
            assert_eq!(confidence, Some(0.8), "{role}");
        }
    }

    #[test]
    fn an_open_role_line_with_no_open_title_decides_nothing() {
        // The reader takes Open from an empty role band, which another
        // screen state can show too; the upload does not send it as the
        // role, so it does not decide the queue either.
        for (title, size) in [(None, Some(6)), (Some(""), Some(6)), (None, None)] {
            let d = decide_rank_queue(title, None, size, Some("Open"), Some(0.5));
            assert_eq!(d.value, RankQueue::Unknown, "{title:?} {size:?}");
            assert_eq!(d.confidence, None, "{title:?} {size:?}");
        }
        // With an open queue title, the title decides as before.
        let (value, _) = decide(Some("OPEN QUEUE"), Some(6), Some("Open"));
        assert_eq!(value, RankQueue::Open6v6);
    }

    #[test]
    fn the_title_rule_does_not_depend_on_word_order() {
        // Season 6 may put the size word before or after the queue word.
        // Each word (mode, ROLE or OPEN, size) is read on its own.
        for (titles, want) in [
            (
                ["COMPETITIVE: 6V6 ROLE QUEUE", "COMPETITIVE: ROLE QUEUE 6V6"],
                RankQueue::Role6v6,
            ),
            (
                ["COMPETITIVE: 6V6 OPEN QUEUE", "COMPETITIVE: OPEN QUEUE 6V6"],
                RankQueue::Open6v6,
            ),
            (
                ["COMPETITIVE: 5V5 ROLE QUEUE", "ROLE QUEUE 5V5: COMPETITIVE"],
                RankQueue::Role5v5,
            ),
        ] {
            for title in titles {
                assert!(title_names_role_or_open(title), "{title}");
                for size in [None, Some(want_size(want))] {
                    let (value, confidence) = decide(Some(title), size, None);
                    assert_eq!(value, want, "{title} {size:?}");
                    assert_eq!(confidence, Some(0.93), "{title} {size:?}");
                }
            }
        }
        // The Quick Play mirrors stay not Competitive in either order.
        for title in [
            "ROLE QUEUE: 6V6 QUICK PLAY",
            "ROLE QUEUE 6V6: QUICK PLAY",
            "OPEN QUEUE: 6V6 QUICK PLAY",
            "OPEN QUEUE 6V6: QUICK PLAY",
            "QUICK PLAY: 6V6 ROLE QUEUE",
        ] {
            assert!(title_rules_out_competitive(title), "{title}");
            assert!(!title_names_role_or_open(title), "{title}");
            let (value, confidence) = decide(Some(title), Some(6), Some("Tank"));
            assert_eq!((value, confidence), (RankQueue::Unknown, None), "{title}");
        }
    }

    fn want_size(q: RankQueue) -> u8 {
        match q {
            RankQueue::Role5v5 | RankQueue::Open5v5 => 5,
            _ => 6,
        }
    }

    #[test]
    fn a_title_naming_a_mode_that_is_not_competitive_is_never_evidence() {
        for word in ["QUICK", "UNRANKED", "ARCADE", "STADIUM", "CUSTOM"] {
            let lower = word.to_lowercase();
            // The game's own order puts the mode last ("ROLE QUEUE: QUICK
            // PLAY", "OPEN QUEUE: 6V6 QUICK PLAY"); the mode word first too.
            for title in [
                format!("ROLE QUEUE: {word} PLAY"),
                format!("OPEN QUEUE: 6V6 {word} PLAY"),
                format!("ROLE QUEUE: 6V6 {word} PLAY"),
                format!("ROLE QUEUE 6V6: {word} PLAY"),
                format!("{word}: ROLE QUEUE"),
                format!("{word} PLAY: OPEN QUEUE"),
                format!("6V6 {word} ROLE"),
                format!("open queue - {lower}"),
                word.to_string(),
            ] {
                assert!(title_rules_out_competitive(&title), "{title}");
                assert!(!title_names_role_or_open(&title), "{title}");
                for size in [Some(5), Some(6), None] {
                    let (value, confidence) = decide(Some(&title), size, Some("Tank"));
                    assert_eq!(value, RankQueue::Unknown, "{title} {size:?}");
                    assert_eq!(confidence, None, "{title} {size:?}");
                }
            }
        }
        for title in ["ROLE QUEUE", "OPEN QUEUE", "COMPETITIVE: 6V6 ROLE QUEUE"] {
            assert!(!title_rules_out_competitive(title), "{title}");
            assert!(title_names_role_or_open(title), "{title}");
        }
    }

    #[test]
    fn a_role_line_spelling_rank_role_does_not_take_is_unknown() {
        for role in [
            "tank", "TANK", "damage", "support", "open", "OPEN", " Tank", "Tank ", " Open",
        ] {
            let (value, confidence) = decide(None, Some(6), Some(role));
            assert_eq!(value, RankQueue::Unknown, "{role}");
            assert_eq!(confidence, None, "{role}");
        }
    }

    #[test]
    fn a_role_line_without_a_sure_confidence_is_not_used() {
        for c in [f32::NAN, 0.0, -0.5, 1.5] {
            let d = decide_rank_queue(None, None, Some(6), Some("Tank"), Some(c));
            assert_eq!(d.value, RankQueue::Unknown, "{c}");
            assert_eq!(d.confidence, None, "{c}");
        }
        let d = decide_rank_queue(None, None, Some(6), Some("Tank"), None);
        assert_eq!(d.value, RankQueue::Unknown);
    }

    #[test]
    fn title_word_reader_names_role_or_open_in_any_order() {
        assert!(title_names_role_or_open("OPEN QUEUE 6V6"));
        assert!(title_is_open("OPEN QUEUE 6V6"));
        assert!(title_is_open("COMPETITIVE: 6V6 OPEN QUEUE"));
        assert!(title_names_role_or_open("ROLE QUEUE"));
        assert!(!title_is_open("ROLE QUEUE"));
        assert!(!title_names_role_or_open("QUICK PLAY"));
        assert!(!title_names_role_or_open("6V6 QUICK PLAY"));
        assert!(!title_names_role_or_open("ARCADE"));
        assert!(!title_names_role_or_open("ROLE OPEN QUEUE"));
    }

    #[test]
    fn title_is_role_reads_role_and_not_open_in_any_order() {
        for t in [
            "ROLE QUEUE",
            "6V6 ROLE QUEUE",
            "ROLE QUEUE 6V6",
            "queue-role",
        ] {
            assert!(title_is_role(t), "{t}");
        }
        for t in ["OPEN QUEUE", "ROLE OPEN QUEUE", "6V6", "", "ROLEQUEUE"] {
            assert!(!title_is_role(t), "{t}");
        }
    }

    /// Two slots match only on the same queue and the same role; the
    /// confidence and `rank_read` play no part. A JSON slot from before a
    /// field existed reads that field as `None` (`rank_read` as false).
    #[test]
    fn a_rank_slot_is_one_queue_and_one_role() {
        let slot = |queue, role| RankSlot {
            queue,
            queue_confidence: Some(0.9),
            role,
            rank_read: true,
        };
        let tank6 = slot(RankQueue::Role6v6, Some(RankRole::Tank));
        let other_conf = RankSlot {
            queue_confidence: None,
            rank_read: false,
            ..tank6
        };
        assert!(tank6.same_slot(&other_conf));
        for other in [
            slot(RankQueue::Role5v5, Some(RankRole::Tank)),
            slot(RankQueue::Unknown, Some(RankRole::Tank)),
            slot(RankQueue::Role6v6, Some(RankRole::Support)),
            slot(RankQueue::Role6v6, None),
            slot(RankQueue::Open6v6, Some(RankRole::Open)),
        ] {
            assert!(!tank6.same_slot(&other), "{other:?}");
            assert!(!other.same_slot(&tank6), "{other:?}");
        }
        let read: RankSlot = serde_json::from_str(r#"{"queue":"role_6v6"}"#).unwrap();
        assert_eq!(read.queue, RankQueue::Role6v6);
        assert_eq!((read.queue_confidence, read.role), (None, None));
        assert!(!read.rank_read);
    }

    #[test]
    fn no_title_and_five_a_side_or_no_size_stays_unknown() {
        for size in [Some(5), None] {
            for role in [Some("Tank"), Some("Open"), None] {
                let (value, confidence) = decide(None, size, role);
                assert_eq!(value, RankQueue::Unknown, "{size:?} {role:?}");
                assert_eq!(confidence, None, "{size:?} {role:?}");
            }
        }
    }

    #[test]
    fn a_title_that_names_both_or_neither_is_unknown() {
        for title in ["ROLE OPEN QUEUE", "QUEUE", "ARCADE"] {
            let (value, _) = decide(Some(title), Some(6), Some("Tank"));
            assert_eq!(value, RankQueue::Unknown, "{title:?}");
        }
        // A blank title is the same as no title: the 6-player role line decides.
        let (value, _) = decide(Some("  "), Some(6), Some("Tank"));
        assert_eq!(value, RankQueue::Role6v6);
        let (value, _) = decide(Some(""), Some(6), Some("Tank"));
        assert_eq!(value, RankQueue::Role6v6);
    }

    #[test]
    fn stored_team_size_keeps_only_five_or_six() {
        assert_eq!(stored_team_size(5), Some(5));
        assert_eq!(stored_team_size(6), Some(6));
        for n in [0, 1, 4, 7, 10, 12] {
            assert_eq!(stored_team_size(n), None, "{n}");
        }
    }

    #[test]
    fn a_bad_confidence_is_dropped_and_the_value_is_kept() {
        for c in [f32::NAN, 0.0, -0.5, 1.5] {
            let d = decide_rank_queue(Some("ROLE QUEUE"), Some(c), Some(5), None, None);
            assert_eq!(d.value, RankQueue::Role5v5, "{c}");
            assert_eq!(d.confidence, None, "{c}");
        }
        // The edges the site keeps: above 0, at most 1.
        for c in [f32::MIN_POSITIVE, 1.0] {
            let d = decide_rank_queue(Some("ROLE QUEUE"), Some(c), Some(5), None, None);
            assert_eq!(d.confidence, Some(c), "{c}");
        }
    }
}
