//! Minimum length of the "Why you want to join" application message.
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
}
