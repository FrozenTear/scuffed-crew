//! Limits of an application: the "Why you want to join" message, the
//! preferred games and roles lists, and the apply and withdraw rate.
//!
//! One place so `POST /api/applications` and the apply page in the app
//! measure a message the same way and cannot drift.

/// Minimum application message length, in Unicode chars after trim.
pub const MIN_APPLICATION_MESSAGE_CHARS: usize = 10;

/// Characters of the message as the rule counts them: trimmed with Unicode
/// whitespace rules (`trim`, not `trim_ascii`), then counted in `char`s:
/// code points, not bytes or graphemes. A message in a non-Latin script is
/// measured the same as one in ASCII.
pub fn application_message_chars(message: &str) -> usize {
    message.trim().chars().count()
}

/// True when the trimmed message has at least
/// [`MIN_APPLICATION_MESSAGE_CHARS`] chars.
pub fn application_message_long_enough(message: &str) -> bool {
    application_message_chars(message) >= MIN_APPLICATION_MESSAGE_CHARS
}

/// Maximum application message length, in Unicode chars after trim.
pub const MAX_APPLICATION_MESSAGE_CHARS: usize = 4000;

/// True when the trimmed message has at most
/// [`MAX_APPLICATION_MESSAGE_CHARS`] chars. Counted like the minimum:
/// [`application_message_chars`].
pub fn application_message_within_max(message: &str) -> bool {
    application_message_chars(message) <= MAX_APPLICATION_MESSAGE_CHARS
}

/// Most entries `preferred_games` or `preferred_roles` may hold.
pub const MAX_APPLICATION_PREFERENCE_ITEMS: usize = 10;

/// Longest entry of `preferred_games` or `preferred_roles`, in Unicode
/// chars after trim.
pub const MAX_APPLICATION_PREFERENCE_ITEM_CHARS: usize = 64;

/// Why a `preferred_games` or `preferred_roles` list is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationPreferenceError {
    /// More than [`MAX_APPLICATION_PREFERENCE_ITEMS`] entries.
    TooMany,
    /// An entry longer than [`MAX_APPLICATION_PREFERENCE_ITEM_CHARS`].
    ItemTooLong,
}

/// Check one preference list (`preferred_games` or `preferred_roles`): at
/// most [`MAX_APPLICATION_PREFERENCE_ITEMS`] entries, each at most
/// [`MAX_APPLICATION_PREFERENCE_ITEM_CHARS`] chars, counted like the message
/// (trim, then `char`s). An empty list passes. The count is checked first.
pub fn check_application_preferences<S: AsRef<str>>(
    items: &[S],
) -> Result<(), ApplicationPreferenceError> {
    if items.len() > MAX_APPLICATION_PREFERENCE_ITEMS {
        return Err(ApplicationPreferenceError::TooMany);
    }
    if items.iter().any(|item| {
        application_message_chars(item.as_ref()) > MAX_APPLICATION_PREFERENCE_ITEM_CHARS
    }) {
        return Err(ApplicationPreferenceError::ItemTooLong);
    }
    Ok(())
}

/// Requests one client IP may send to `POST /api/applications` and
/// `POST /api/applications/mine/withdraw` (one shared bucket) before waiting.
///
/// Tighter than the auth governor (burst 5, one every 2 s): a successful
/// apply writes a row and notifies officers.
pub const APPLICATION_IP_BURST: u32 = 3;

/// Seconds until the apply and withdraw per-IP governor replenishes one
/// request.
pub const APPLICATION_IP_REPLENISH_SECS: u64 = 60;

/// Seconds a user waits after their latest application was closed
/// (withdrawn, rejected or accepted) before a new one is accepted.
pub const APPLICATION_RESUBMIT_COOLDOWN_SECS: i64 = 60 * 60;

/// Withdrawn applications one user may hold. At the cap a new application
/// is refused, and so is a withdraw that would add another.
pub const MAX_WITHDRAWN_APPLICATIONS: u64 = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_minimum_is_ten() {
        assert_eq!(MIN_APPLICATION_MESSAGE_CHARS, 10);
    }

    #[test]
    fn nine_chars_after_trim_is_too_short() {
        assert_eq!(application_message_chars("   123456789   "), 9);
        assert!(!application_message_long_enough("   123456789   "));
        assert!(!application_message_long_enough("\n\t123456789\n"));
    }

    #[test]
    fn padded_ten_chars_is_long_enough() {
        assert_eq!(application_message_chars("  1234567890  "), 10);
        assert!(application_message_long_enough("  1234567890  "));
        // Inner spaces count.
        assert!(application_message_long_enough("Hi, I heal"));
    }

    #[test]
    fn ten_multibyte_chars_is_long_enough() {
        let ten = "æøåæøåæøåæ";
        assert!(ten.len() > 10);
        assert_eq!(application_message_chars(ten), 10);
        assert!(application_message_long_enough(ten));
        assert!(application_message_long_enough("ゲームが好きです。!"));
    }

    #[test]
    fn nine_multibyte_chars_is_too_short() {
        let nine = "🎮🎮🎮🎮🎮🎮🎮🎮🎮";
        assert!(nine.len() > 10);
        assert_eq!(application_message_chars(nine), 9);
        assert!(!application_message_long_enough(nine));
        assert!(!application_message_long_enough("æøåÆØÅéüñ"));
    }

    #[test]
    fn non_ascii_whitespace_padding_is_trimmed() {
        // U+3000 ideographic space and U+00A0 no-break space are Unicode
        // whitespace, so `trim` drops them; `trim_ascii` would not.
        let ideographic = "\u{3000}123456789\u{3000}";
        assert_eq!(application_message_chars(ideographic), 9);
        assert!(!application_message_long_enough(ideographic));
        let no_break = "\u{A0}\u{A0}123456789\u{A0}";
        assert_eq!(application_message_chars(no_break), 9);
        assert!(!application_message_long_enough(no_break));
    }

    #[test]
    fn counts_code_points_not_graphemes() {
        // 5 graphemes, 10 code points: e + combining acute accent.
        let combining = "e\u{301}".repeat(5);
        assert_eq!(application_message_chars(&combining), 10);
        assert!(application_message_long_enough(&combining));
        // 5 graphemes, 10 code points: thumbs up + skin tone modifier.
        let toned = "\u{1F44D}\u{1F3FD}".repeat(5);
        assert_eq!(application_message_chars(&toned), 10);
        assert!(application_message_long_enough(&toned));
    }

    #[test]
    fn empty_is_too_short() {
        assert_eq!(application_message_chars(""), 0);
        assert!(!application_message_long_enough(""));
        assert!(!application_message_long_enough("          "));
    }

    #[test]
    fn the_maximum_is_four_thousand() {
        assert_eq!(MAX_APPLICATION_MESSAGE_CHARS, 4000);
    }

    #[test]
    fn four_thousand_chars_after_trim_is_within_max() {
        let exact = "a".repeat(4000);
        assert!(application_message_within_max(&exact));
        // Padding is trimmed before the count, like the minimum.
        let padded = format!("  \n{exact}\t \u{3000}");
        assert_eq!(application_message_chars(&padded), 4000);
        assert!(application_message_within_max(&padded));
        assert!(application_message_within_max(""));
        assert!(application_message_within_max("Hi, I heal"));
    }

    #[test]
    fn four_thousand_and_one_chars_is_over_max() {
        let over = "a".repeat(4001);
        assert!(!application_message_within_max(&over));
        assert!(!application_message_within_max(&format!("  {over}  ")));
    }

    #[test]
    fn the_maximum_counts_chars_not_bytes() {
        // 4000 chars, 8000 bytes: within. Bytes are not the measure.
        let multibyte = "æ".repeat(4000);
        assert_eq!(multibyte.len(), 8000);
        assert!(application_message_within_max(&multibyte));
        assert!(!application_message_within_max(&"æ".repeat(4001)));
        // 4000 chars, 16000 bytes.
        assert!(application_message_within_max(&"🎮".repeat(4000)));
        assert!(!application_message_within_max(&"🎮".repeat(4001)));
    }

    #[test]
    fn the_preference_caps_are_ten_items_of_sixty_four_chars() {
        assert_eq!(MAX_APPLICATION_PREFERENCE_ITEMS, 10);
        assert_eq!(MAX_APPLICATION_PREFERENCE_ITEM_CHARS, 64);
    }

    #[test]
    fn ten_preferences_pass_and_eleven_are_too_many() {
        let ten = vec!["ow2".to_string(); 10];
        assert_eq!(check_application_preferences(&ten), Ok(()));
        let eleven = vec!["ow2".to_string(); 11];
        assert_eq!(
            check_application_preferences(&eleven),
            Err(ApplicationPreferenceError::TooMany)
        );
        let none: [&str; 0] = [];
        assert_eq!(check_application_preferences(&none), Ok(()));
    }

    #[test]
    fn a_sixty_four_char_preference_passes_and_sixty_five_is_too_long() {
        let exact = "a".repeat(64);
        assert_eq!(check_application_preferences(&[exact.as_str()]), Ok(()));
        let over = "a".repeat(65);
        assert_eq!(
            check_application_preferences(&["ow2", over.as_str()]),
            Err(ApplicationPreferenceError::ItemTooLong)
        );
    }

    #[test]
    fn a_preference_is_trimmed_and_counted_in_chars() {
        // 64 chars after trim, 128+ bytes.
        let padded = format!(" \u{3000}{}\n", "æ".repeat(64));
        assert_eq!(check_application_preferences(&[padded.as_str()]), Ok(()));
        let over = "🎮".repeat(65);
        assert_eq!(
            check_application_preferences(&[over.as_str()]),
            Err(ApplicationPreferenceError::ItemTooLong)
        );
    }

    #[test]
    fn too_many_is_reported_before_a_long_item() {
        let mut items = vec!["a".repeat(65)];
        items.extend(vec!["ow2".to_string(); 10]);
        assert_eq!(
            check_application_preferences(&items),
            Err(ApplicationPreferenceError::TooMany)
        );
    }

    #[test]
    fn the_rate_limits_are_the_documented_ones() {
        assert_eq!(APPLICATION_IP_BURST, 3);
        assert_eq!(APPLICATION_IP_REPLENISH_SECS, 60);
        assert_eq!(APPLICATION_RESUBMIT_COOLDOWN_SECS, 3600);
        assert_eq!(MAX_WITHDRAWN_APPLICATIONS, 5);
    }
}
