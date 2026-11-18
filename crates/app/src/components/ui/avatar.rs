//! Avatar initials, one rule for the account avatar in the top bar, the DM
//! inbox and thread, and the article byline.

/// Up to two initials for a name, upper case, or "?" when the name has no
/// letter or digit.
///
/// - Two or more words: the first letter of the first two ("Player Two" is
///   "PT").
/// - One word with a digit after its first character: the first character
///   and the last digit ("Player02" is "P2", "Officer01" is "O1"), so members
///   named in that style can be told apart, as the frames draw them.
/// - One word with a capital after its first character: the first character
///   and that capital ("TestAdmin" is "TA").
/// - Any other single word: its first two letters or digits ("Frost" is "FR").
pub fn name_initials(name: &str) -> String {
    let mut words = name
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<Vec<_>>()
        })
        .filter(|w| !w.is_empty());
    let first = words.next().unwrap_or_default();
    let picked: Vec<char> = match words.next() {
        Some(second) => first
            .iter()
            .take(1)
            .chain(second.iter().take(1))
            .copied()
            .collect(),
        None => {
            let head = first.first().copied();
            let rest = first.get(1..).unwrap_or_default();
            let second = rest
                .iter()
                .rev()
                .find(|c| c.is_ascii_digit())
                .or_else(|| rest.iter().find(|c| c.is_uppercase()))
                .or_else(|| rest.first());
            head.into_iter().chain(second.copied()).collect()
        }
    };
    let out: String = picked.into_iter().flat_map(char::to_uppercase).collect();
    if out.is_empty() { "?".to_string() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_named_in_one_style_get_different_initials() {
        assert_eq!(name_initials("Player02"), "P2");
        assert_eq!(name_initials("Player03"), "P3");
        assert_eq!(name_initials("Officer01"), "O1");
        assert_eq!(name_initials("Applicant15"), "A5");
        assert_eq!(name_initials("TestAdmin"), "TA");
        assert_eq!(name_initials("Frost"), "FR");
        assert_eq!(name_initials("player two extra"), "PT");
        assert_eq!(name_initials("x"), "X");
        assert_eq!(name_initials("_-"), "?");
        assert_eq!(name_initials("  "), "?");
        assert_eq!(name_initials("npub1\u{2026}abcdef"), "N1");
    }
}
