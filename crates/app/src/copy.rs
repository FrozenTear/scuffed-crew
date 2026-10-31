//! Render-time dash rule for admin copy and patch notes.
//!
//! One rule, no exceptions:
//! 1. A numeric range (a digit, optional spaces, an en dash or figure dash,
//!    optional spaces, a digit) reads "{a} to {b}". "August 11-31" becomes
//!    "August 11 to 31", "~5-10" becomes "~5 to 10".
//! 2. Any other long dash (em dash, en dash, figure dash, horizontal bar,
//!    two-em and three-em dash) becomes a comma and one space, with the
//!    spaces around it collapsed. No comma is added next to punctuation that
//!    is already there, and a leading or trailing dash is dropped.
//! 3. Hyphen-minus, the true hyphen, the non-breaking hyphen and the minus
//!    sign are left alone.
//!
//! The stored text is never rewritten. This runs only when the page renders.

use scuffed_types::HomepageContent;
use serde_json::Value;

fn is_long_dash(c: char) -> bool {
    matches!(
        c,
        '\u{2014}' | '\u{2013}' | '\u{2012}' | '\u{2015}' | '\u{2E3A}' | '\u{2E3B}'
    )
}

fn is_range_dash(c: char) -> bool {
    matches!(c, '\u{2013}' | '\u{2012}')
}

/// Replace long dashes in `text` by the rule in the module docs.
pub fn plain_copy(text: &str) -> String {
    if !text.chars().any(is_long_dash) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !is_long_dash(c) {
            out.push(c);
            i += 1;
            continue;
        }
        // Where the text after the dash starts, past plain spaces.
        let mut next = i + 1;
        while next < chars.len() && chars[next] == ' ' {
            next += 1;
        }
        let trimmed_len = out.trim_end_matches(' ').len();
        let prev_is_digit = out[..trimmed_len]
            .chars()
            .next_back()
            .is_some_and(|p| p.is_ascii_digit());
        let next_char = chars.get(next).copied();
        if is_range_dash(c) && prev_is_digit && next_char.is_some_and(|n| n.is_ascii_digit()) {
            out.truncate(trimmed_len);
            out.push_str(" to ");
            i = next;
            continue;
        }
        out.truncate(trimmed_len);
        i = next;
        let at_start = out.is_empty() || out.ends_with('\n');
        let at_end = matches!(next_char, None | Some('\n'));
        if at_start || at_end {
            continue;
        }
        let after_punct = out.ends_with([',', ':', ';', '.', '!', '?']);
        let before_punct =
            next_char.is_some_and(|n| matches!(n, ',' | '.' | ';' | ':' | '!' | '?' | ')' | ']'));
        if after_punct {
            out.push(' ');
        } else if !before_punct {
            out.push_str(", ");
        }
    }
    out
}

/// Apply [`plain_copy`] to every string inside a JSON value.
pub fn plain_value(value: &mut Value) {
    match value {
        Value::String(s) => {
            let fixed = plain_copy(s);
            if fixed != *s {
                *s = fixed;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(plain_value),
        Value::Object(map) => map.values_mut().for_each(plain_value),
        _ => {}
    }
}

/// A copy of the homepage content with [`plain_copy`] applied to every string
/// field, including the lists. Round-trips through JSON so a field added to
/// `HomepageContent` later is covered without touching this function. If the
/// round trip ever fails the input is returned unchanged.
pub fn plain_homepage(content: &HomepageContent) -> HomepageContent {
    let Ok(mut value) = serde_json::to_value(content) else {
        return content.clone();
    };
    plain_value(&mut value);
    serde_json::from_value(value).unwrap_or_else(|_| content.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EM: char = '\u{2014}';
    const EN: char = '\u{2013}';
    const FIG: char = '\u{2012}';

    fn has_long_dash(s: &str) -> bool {
        s.chars().any(|c| {
            matches!(
                c,
                '\u{2014}' | '\u{2013}' | '\u{2012}' | '\u{2015}' | '\u{2E3A}' | '\u{2E3B}'
            )
        })
    }

    fn samples() -> Vec<String> {
        vec![
            format!("Life first {EM} games second"),
            format!("Life first{EM}games second"),
            format!("August 11{EN}31"),
            format!("August 11 {EN} 31"),
            format!("Squads of ~5{EN}10."),
            format!("3{FIG}4"),
            format!("1{EM}2"),
            format!("Mon{EN}Fri"),
            format!("Hello, {EM} world"),
            format!("Note: {EM} read"),
            format!("word {EM}. Next"),
            format!("a {EM}{EM} b"),
            format!("{EM}Leading"),
            format!("Trailing {EM}"),
            format!("line one {EM}\nline two"),
            "a \u{2015} b \u{2E3A} c \u{2E3B} d".to_string(),
            "well-known 5-10 and -3".to_string(),
            "co\u{2010}op non\u{2011}stop 5 \u{2212} 3".to_string(),
            "Plain text with no dashes.".to_string(),
        ]
    }

    #[test]
    fn em_dash_becomes_comma_and_one_space() {
        assert_eq!(
            plain_copy(&format!("Life first {EM} games second")),
            "Life first, games second"
        );
        assert_eq!(
            plain_copy(&format!("Life first{EM}games second")),
            "Life first, games second"
        );
    }

    #[test]
    fn numeric_range_reads_as_to() {
        assert_eq!(plain_copy(&format!("August 11{EN}31")), "August 11 to 31");
        assert_eq!(plain_copy(&format!("August 11 {EN} 31")), "August 11 to 31");
        assert_eq!(
            plain_copy(&format!("Squads of ~5{EN}10.")),
            "Squads of ~5 to 10."
        );
        assert_eq!(plain_copy(&format!("3{FIG}4")), "3 to 4");
        assert_eq!(plain_copy(&format!("1{EM}2")), "1, 2");
    }

    #[test]
    fn en_dash_between_words_is_a_comma() {
        assert_eq!(plain_copy(&format!("Mon{EN}Fri")), "Mon, Fri");
    }

    #[test]
    fn punctuation_neighbours_get_no_extra_comma() {
        assert_eq!(plain_copy(&format!("Hello, {EM} world")), "Hello, world");
        assert_eq!(plain_copy(&format!("Note: {EM} read")), "Note: read");
        assert_eq!(plain_copy(&format!("word {EM}. Next")), "word. Next");
        assert_eq!(plain_copy(&format!("a {EM}{EM} b")), "a, b");
    }

    #[test]
    fn leading_and_trailing_dash_are_dropped() {
        assert_eq!(plain_copy(&format!("{EM}Leading")), "Leading");
        assert_eq!(plain_copy(&format!("Trailing {EM}")), "Trailing");
        assert_eq!(
            plain_copy(&format!("line one {EM}\nline two")),
            "line one\nline two"
        );
    }

    #[test]
    fn hyphens_and_plain_text_are_untouched() {
        let hyphens = "well-known 5-10 and -3";
        assert_eq!(plain_copy(hyphens), hyphens);
        let other = "co\u{2010}op non\u{2011}stop 5 \u{2212} 3";
        assert_eq!(plain_copy(other), other);
        let plain = "Plain text with no dashes.";
        assert_eq!(plain_copy(plain), plain);
    }

    #[test]
    fn plain_copy_is_idempotent() {
        for x in samples() {
            let once = plain_copy(&x);
            assert_eq!(plain_copy(&once), once, "input {x:?}");
        }
    }

    #[test]
    fn no_long_dash_survives() {
        for x in samples() {
            let out = plain_copy(&x);
            assert!(!has_long_dash(&out), "input {x:?} gave {out:?}");
        }
    }

    #[test]
    fn plain_value_walks_nested_json() {
        let mut value = serde_json::json!({
            "title": format!("Play {EM} together"),
            "tags": [format!("DPS {EM} Tank"), "plain"],
            "count": 3,
        });
        plain_value(&mut value);
        assert_eq!(value["title"], "Play, together");
        assert_eq!(value["tags"][0], "DPS, Tank");
        assert_eq!(value["tags"][1], "plain");
        assert_eq!(value["count"], 3);
    }

    #[test]
    fn plain_homepage_cleans_every_string_field() {
        let content = HomepageContent {
            hero_title: format!("Play {EM} together"),
            hero_sub: format!("Open {EN} always"),
            ethos_rules: vec![format!("Squads of 5{EN}10")],
            seeking_tags: vec![format!("DPS {EM} Tank")],
            footer_note: format!("Bye {EM}"),
            ..Default::default()
        };
        let out = plain_homepage(&content);
        assert_eq!(out.hero_title, "Play, together");
        assert_eq!(out.hero_sub, "Open, always");
        assert_eq!(out.ethos_rules, vec!["Squads of 5 to 10".to_string()]);
        assert_eq!(out.seeking_tags, vec!["DPS, Tank".to_string()]);
        assert_eq!(out.footer_note, "Bye");
        assert!(!has_long_dash(&out.to_json()));
        assert_eq!(
            plain_homepage(&HomepageContent::default()),
            HomepageContent::default()
        );
    }
}
