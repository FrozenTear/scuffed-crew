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

/// The title names ROLE or OPEN (any word order, case and punctuation),
/// and not both. Used to decide whether a title-only upload is
/// Competitive evidence.
pub fn title_names_role_or_open(text: &str) -> bool {
    kind_in(&title_words(text)).is_some()
}

/// The title names OPEN and not ROLE, with the same word reader as
/// [`title_names_role_or_open`].
pub fn title_is_open(text: &str) -> bool {
    kind_in(&title_words(text)) == Some(QueueKind::Open)
}

fn size_in(words: &[String]) -> Option<u8> {
    let has5 = words.iter().any(|w| w == "5V5");
    let has6 = words.iter().any(|w| w == "6V6");
    match (has5, has6) {
        (true, false) => Some(5),
        (false, true) => Some(6),
        _ => None,
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

/// The rank screen role line as a queue kind, parsed the same way the
/// upload maps `rank.role` ([`parse_rank_role`]).
fn role_line_kind(role: &str) -> Option<QueueKind> {
    match parse_rank_role(role)? {
        RankRole::Tank | RankRole::Damage | RankRole::Support => Some(QueueKind::Role),
        RankRole::Open => Some(QueueKind::Open),
    }
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
///   `6V6` word in the title. If those two disagree, the queue is unknown.
/// - No confident title and 6 per team: a sure Tank, Damage or Support
///   role line is `role_6v6`; Open is `open_6v6`.
/// - Anything else is [`RankQueue::Unknown`].
pub fn decide_rank_queue(
    title: Option<&str>,
    title_confidence: Option<f32>,
    team_size: Option<u8>,
    rank_role: Option<&str>,
    rank_role_confidence: Option<f32>,
) -> DecidedQueue {
    let words = title.map(title_words).unwrap_or_default();
    let kind = kind_in(&words);
    let title_size = size_in(&words);
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
        && let Some(kind) = rank_role.and_then(role_line_kind)
        && let Some(decided) = from_kind_and_size(kind, 6, rank_role_confidence)
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
        let (value, confidence) = decide(Some("6V6 ROLE QUEUE"), Some(5), None);
        assert_eq!(value, RankQueue::Unknown);
        assert_eq!(confidence, None);
    }

    #[test]
    fn no_title_and_six_a_side_uses_the_rank_role_line() {
        for role in ["Tank", "Damage", "Support"] {
            let (value, confidence) = decide(None, Some(6), Some(role));
            assert_eq!(value, RankQueue::Role6v6, "{role}");
            assert_eq!(confidence, Some(0.8), "{role}");
        }
        let (value, confidence) = decide(None, Some(6), Some("Open"));
        assert_eq!(value, RankQueue::Open6v6);
        assert_eq!(confidence, Some(0.8));
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
        let d = decide_rank_queue(None, None, Some(6), Some("Tank"), Some(f32::NAN));
        assert_eq!(d.value, RankQueue::Unknown);
        assert_eq!(d.confidence, None);
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
    fn no_title_and_five_a_side_stays_unknown() {
        for role in [Some("Tank"), Some("Open"), None] {
            let (value, confidence) = decide(None, Some(5), role);
            assert_eq!(value, RankQueue::Unknown, "{role:?}");
            assert_eq!(confidence, None, "{role:?}");
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
        let d = decide_rank_queue(Some("ROLE QUEUE"), Some(f32::NAN), Some(5), None, None);
        assert_eq!(d.value, RankQueue::Role5v5);
        assert_eq!(d.confidence, None);
    }
}
