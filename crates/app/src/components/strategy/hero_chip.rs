//! Fallback chip for hero markers on the strategy canvas (APP4).
//!
//! Hero art under `/assets/heroes/` is not shipped yet, so those requests 404.
//! A portrait only counts as drawn once the image has really loaded; until
//! then the marker is a chip filled with the hero's role colour and labelled
//! with the hero's initials. Real art comes in 0.7.

use super::hero_catalog::hero_by_id;

/// Most characters a chip label holds inside a 20px marker.
const MAX_LABEL_CHARS: usize = 3;

/// True once an image can be painted. A broken image also reports
/// `complete`, but with a natural width of 0, so it stays a chip. The image
/// stays in the cache either way, so a 404 is requested once, not every frame.
pub(super) fn portrait_ready(complete: bool, natural_width: u32) -> bool {
    complete && natural_width > 0
}

/// Short chip label: first letter of each word, keeping a number word whole
/// (`Soldier: 76` is `S76`). A one word name keeps its first two letters
/// (`D.Va` is `DV`, `Ana` is `AN`).
pub(super) fn hero_initials(name: &str) -> String {
    let words: Vec<String> = name
        .split_whitespace()
        .map(|word| word.chars().filter(|c| c.is_alphanumeric()).collect())
        .filter(|word: &String| !word.is_empty())
        .collect();
    let label: String = match words.as_slice() {
        [] => String::new(),
        [only] => only.chars().take(2).collect(),
        many => many
            .iter()
            .map(|word| {
                if word.chars().all(|c| c.is_ascii_digit()) {
                    word.clone()
                } else {
                    word.chars().take(1).collect()
                }
            })
            .collect(),
    };
    label
        .chars()
        .flat_map(char::to_uppercase)
        .take(MAX_LABEL_CHARS)
        .collect()
}

/// What a hero marker shows while its portrait is not drawable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeroChip {
    pub label: String,
    /// Role colour from `HeroRole::color_hex`, the same colours the hero
    /// picker and team panel use. `None` for an id not on the roster, which
    /// keeps the marker's own colour.
    pub fill: Option<&'static str>,
}

pub(super) fn hero_chip(hero_id: &str) -> HeroChip {
    match hero_by_id(hero_id) {
        Some(hero) => HeroChip {
            label: hero_initials(hero.name),
            fill: Some(hero.role.color_hex()),
        },
        None => HeroChip {
            label: hero_initials(&hero_id.replace('-', " ")),
            fill: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_types::HeroRole;

    #[test]
    fn only_a_loaded_image_counts_as_drawn() {
        assert!(portrait_ready(true, 128));
        // Still loading.
        assert!(!portrait_ready(false, 0));
        // A 404 finishes as complete with no pixels.
        assert!(!portrait_ready(true, 0));
        assert!(!portrait_ready(false, 128));
    }

    #[test]
    fn initials_fit_the_chip() {
        let cases = [
            ("Junker Queen", "JQ"),
            ("Wrecking Ball", "WB"),
            ("Soldier: 76", "S76"),
            ("D.Va", "DV"),
            ("Ana", "AN"),
            ("Lúcio", "LÚ"),
            ("Torbjörn", "TO"),
            ("Jetpack Cat", "JC"),
            ("", ""),
            ("  .  ", ""),
        ];
        for (name, label) in cases {
            assert_eq!(hero_initials(name), label, "{name}");
        }
        assert!(hero_initials("One Two Three Four").chars().count() <= MAX_LABEL_CHARS);
    }

    #[test]
    fn every_roster_hero_gets_a_label_and_its_role_colour() {
        for name in scuffed_types::HEROES {
            let id = super::super::hero_catalog::hero_id(name);
            let hero = hero_by_id(&id).unwrap();
            let chip = hero_chip(&id);
            assert!(!chip.label.is_empty(), "{name}");
            assert!(chip.label.chars().count() <= MAX_LABEL_CHARS, "{name}");
            assert_eq!(chip.fill, Some(hero.role.color_hex()), "{name}");
        }
    }

    #[test]
    fn roles_map_to_their_own_colours() {
        let chip = hero_chip("reinhardt");
        assert_eq!(chip.fill, Some(HeroRole::Tank.color_hex()));
        assert_eq!(hero_chip("tracer").fill, Some(HeroRole::Damage.color_hex()));
        assert_eq!(hero_chip("mercy").fill, Some(HeroRole::Support.color_hex()));
        let fills: std::collections::HashSet<_> =
            HeroRole::ALL.iter().map(|r| r.color_hex()).collect();
        assert_eq!(fills.len(), HeroRole::ALL.len());
    }

    #[test]
    fn an_unknown_id_keeps_the_marker_colour() {
        let chip = hero_chip("new-hero");
        assert_eq!(chip.label, "NH");
        assert_eq!(chip.fill, None);
    }
}
