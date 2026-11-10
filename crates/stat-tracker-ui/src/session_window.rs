//! The hint under the Settings Session window field.

/// A Session window of a year or more groups every game of a night, week
/// and month together, so the field says so instead of showing a bare number.
const NO_LIMIT_SECS: u64 = 365 * 24 * 60 * 60;

const DEFAULT_HINT: &str = "Nearby games count as one session.";

/// The hint under the Session window field: "No limit" for a value so big
/// that no gap between games is ever longer (a value too big to parse also
/// reads as no limit, as every digit string that long is past a year).
pub(crate) fn hint(raw: &str) -> &'static str {
    let raw = raw.trim();
    let huge = match raw.parse::<u64>() {
        Ok(secs) => secs >= NO_LIMIT_SECS,
        Err(_) => !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()),
    };
    if huge { "No limit" } else { DEFAULT_HINT }
}

#[cfg(test)]
mod tests {
    use super::hint;

    #[test]
    fn a_huge_session_window_shows_no_limit() {
        for huge in [
            "31536000",
            " 31536000 ",
            "9223372036854775807",
            "18446744073709551615",
            // Past u64: parse_u64 falls back to 1800, but the field still
            // holds this text.
            "18446744073709551616",
            "99999999999999999999999999",
        ] {
            assert_eq!(hint(huge), "No limit", "{huge:?}");
        }
        for normal in ["1800", "0", "31535999", "", "oops", "-5", "1.5"] {
            assert_eq!(hint(normal), DEFAULT, "{normal:?}");
        }
    }

    const DEFAULT: &str = "Nearby games count as one session.";
}
