//! Fallback chip for hero markers on the strategy canvas (APP4).
//!
//! Hero art under `/assets/heroes/` is not shipped yet, so those requests 404.
//! A portrait only counts as drawn once the image has really loaded; until
//! then the marker is a chip filled with the hero's role colour and labelled
//! with the hero's initials. Real art comes in 0.7.
//!
//! Saved strategies can hold older hero ids (`soldier_76`, `d_va`,
//! `wrecking_ball`). Those fold onto the roster in `scuffed_types::HEROES`
//! for the chip only. The stored id is never rewritten.

use std::cell::RefCell;
use std::collections::HashMap;

use scuffed_types::strategy::Color;

use super::hero_catalog::{CatalogHero, hero_by_id, hero_id};
use crate::theme::tokens::{CANVAS_CHIP_RING_DARK, CANVAS_UNKNOWN_HERO_FILL, CANVAS_WHITE};

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

/// Below this RGB distance the ring and fill read as one colour. Red team
/// (239, 83, 80) and the Damage fill (230, 80, 80) are about 9.5 apart.
const MIN_RING_DISTANCE: f64 = 64.0;

/// Lowercase, accent folded, and only letters and digits, so `soldier_76`,
/// `Soldier: 76`, and `soldier-76` all become `soldier76`.
fn fold_hero_key(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars().flat_map(char::to_lowercase) {
        let mapped = match c {
            'á' | 'à' | 'ã' | 'â' | 'ä' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'õ' | 'ô' | 'ö' | 'ø' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other if other.is_alphanumeric() => other,
            _ => continue,
        };
        out.push(mapped);
    }
    out
}

/// Most alias lookups kept. Saved strategies hold a handful of ids, so this
/// is only a guard against a board full of made up ids.
const ALIAS_CACHE_CAP: usize = 256;

thread_local! {
    /// Alias results by raw id, so an older id does not rescan the roster on
    /// every redraw. The web build is single threaded.
    static ALIAS_CACHE: RefCell<HashMap<String, Option<&'static CatalogHero>>> =
        RefCell::new(HashMap::new());
}

/// Roster entry for a saved hero id. The current kebab id matches directly.
/// An older form (`soldier_76`, `d_va`, `DVA`, `Lúcio`) folds and matches a
/// roster name or id in `scuffed_types::HEROES`, once per raw id.
pub(super) fn roster_hero(raw: &str) -> Option<&'static CatalogHero> {
    if let Some(hero) = hero_by_id(raw) {
        return Some(hero);
    }
    if let Some(cached) = ALIAS_CACHE.with(|cache| cache.borrow().get(raw).copied()) {
        return cached;
    }
    let found = fold_alias(raw);
    ALIAS_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= ALIAS_CACHE_CAP {
            cache.clear();
        }
        cache.insert(raw.to_string(), found);
    });
    found
}

/// The uncached roster scan behind [`roster_hero`].
fn fold_alias(raw: &str) -> Option<&'static CatalogHero> {
    let key = fold_hero_key(raw);
    if key.is_empty() {
        return None;
    }
    scuffed_types::HEROES
        .iter()
        .copied()
        .map(hero_id)
        .find(|id| fold_hero_key(id) == key)
        .and_then(|id| hero_by_id(&id))
}

/// `#rrggbb` to its channels. Anything else is `None`.
fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// Ring colour for a chip. The team colour, unless it is too close to the
/// fill to see, then white on a dark fill or near black on a light one.
pub(super) fn ring_color(team: &Color, fill: &str) -> String {
    let Some((r, g, b)) = parse_hex(fill) else {
        return team.to_css();
    };
    let dr = f64::from(team.r) - f64::from(r);
    let dg = f64::from(team.g) - f64::from(g);
    let db = f64::from(team.b) - f64::from(b);
    if (dr * dr + dg * dg + db * db).sqrt() >= MIN_RING_DISTANCE {
        return team.to_css();
    }
    let luma = (0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)) / 255.0;
    let ring = if luma > 0.6 {
        CANVAS_CHIP_RING_DARK
    } else {
        CANVAS_WHITE
    };
    ring.to_string()
}

/// WCAG relative luminance of a `#rrggbb` colour.
fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// WCAG contrast ratio between two relative luminances.
fn contrast(la: f64, lb: f64) -> f64 {
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Relative luminance of white. Only a fallback for a label token that is not
/// `#rrggbb`, since both label tokens parse.
const WHITE_LUMINANCE: f64 = 1.0;

/// Relative luminance of a label colour token, read from its hex.
fn label_luminance(label: &str) -> f64 {
    parse_hex(label).map_or(WHITE_LUMINANCE, relative_luminance)
}

/// Initials colour for a fill: whichever of white or the dark ring colour
/// contrasts more. Tank, Support and Damage get dark initials, the unknown
/// grey keeps white.
pub(super) fn label_color(fill: &str) -> &'static str {
    let Some(rgb) = parse_hex(fill) else {
        return CANVAS_WHITE;
    };
    let lum = relative_luminance(rgb);
    let dark = contrast(lum, label_luminance(CANVAS_CHIP_RING_DARK));
    if dark > contrast(lum, label_luminance(CANVAS_WHITE)) {
        CANVAS_CHIP_RING_DARK
    } else {
        CANVAS_WHITE
    }
}

/// What a hero marker shows while its portrait is not drawable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeroChip {
    pub label: String,
    /// Role colour from `HeroRole::color_hex`, the same colours the hero
    /// picker and team panel use. [`CANVAS_UNKNOWN_HERO_FILL`] for an id not on
    /// the roster.
    pub fill: &'static str,
    /// Team colour, or a contrasting ring when the team colour matches the
    /// fill.
    pub ring: String,
    /// Initials colour picked from the fill's luminance.
    pub label_color: &'static str,
}

pub(super) fn hero_chip(raw: &str, team: &Color) -> HeroChip {
    let (label, fill) = match roster_hero(raw) {
        Some(hero) => (hero_initials(hero.name), hero.role.color_hex()),
        None => (
            hero_initials(&raw.replace(['-', '_'], " ")),
            CANVAS_UNKNOWN_HERO_FILL,
        ),
    };
    HeroChip {
        label,
        fill,
        ring: ring_color(team, fill),
        label_color: label_color(fill),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_types::HeroRole;

    const BLUE: Color = Color::BLUE_TEAM;

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
            let chip = hero_chip(&id, &BLUE);
            assert!(!chip.label.is_empty(), "{name}");
            assert!(chip.label.chars().count() <= MAX_LABEL_CHARS, "{name}");
            assert_eq!(chip.fill, hero.role.color_hex(), "{name}");
        }
    }

    #[test]
    fn roles_map_to_their_own_colours() {
        let chip = hero_chip("reinhardt", &BLUE);
        assert_eq!(chip.fill, HeroRole::Tank.color_hex());
        assert_eq!(
            hero_chip("tracer", &BLUE).fill,
            HeroRole::Damage.color_hex()
        );
        assert_eq!(
            hero_chip("mercy", &BLUE).fill,
            HeroRole::Support.color_hex()
        );
        let fills: std::collections::HashSet<_> =
            HeroRole::ALL.iter().map(|r| r.color_hex()).collect();
        assert_eq!(fills.len(), HeroRole::ALL.len());
    }

    #[test]
    fn legacy_ids_fold_onto_the_roster() {
        let soldier = hero_chip("soldier_76", &BLUE);
        assert_eq!(soldier.label, "S76");
        assert_eq!(soldier.fill, HeroRole::Damage.color_hex());
        let cases = [
            ("soldier_76", "soldier-76"),
            ("Soldier: 76", "soldier-76"),
            ("SOLDIER 76", "soldier-76"),
            ("soldier.76", "soldier-76"),
            ("dva", "dva"),
            ("d_va", "dva"),
            ("D.Va", "dva"),
            ("d-va", "dva"),
            ("d_mon", "dmon"),
            ("wrecking_ball", "wrecking-ball"),
            ("WreckingBall", "wrecking-ball"),
            ("junker_queen", "junker-queen"),
            ("lucio", "lucio"),
            ("Lúcio", "lucio"),
            ("torbjörn", "torbjorn"),
            ("jetpack_cat", "jetpack-cat"),
        ];
        for (saved, id) in cases {
            let hero = roster_hero(saved).unwrap_or_else(|| panic!("{saved} did not resolve"));
            assert_eq!(hero.id, id, "{saved}");
            assert_eq!(hero_chip(saved, &BLUE), hero_chip(id, &BLUE), "{saved}");
        }
        assert_eq!(
            hero_chip("wrecking_ball", &BLUE).fill,
            HeroRole::Tank.color_hex()
        );
        assert_eq!(
            hero_chip("lucio", &BLUE).fill,
            HeroRole::Support.color_hex()
        );
    }

    #[test]
    fn repeated_alias_lookups_hit_the_cache() {
        for raw in ["soldier_76", "d_va", "d_mon", "no_such_hero"] {
            let first = roster_hero(raw).map(|hero| hero.id.as_str());
            let cached = ALIAS_CACHE.with(|cache| cache.borrow().get(raw).copied());
            assert_eq!(
                cached.map(|hit| hit.map(|hero| hero.id.as_str())),
                Some(first)
            );
            assert_eq!(
                roster_hero(raw).map(|hero| hero.id.as_str()),
                first,
                "{raw}"
            );
        }
        assert_eq!(
            roster_hero("d_va").map(|hero| hero.id.as_str()),
            Some("dva")
        );
        assert_eq!(
            roster_hero("d_mon").map(|hero| hero.id.as_str()),
            Some("dmon")
        );
        assert!(roster_hero("no_such_hero").is_none());
        // Exact ids take the fast path and are never cached.
        assert!(roster_hero("tracer").is_some());
        assert!(ALIAS_CACHE.with(|cache| !cache.borrow().contains_key("tracer")));
    }

    #[test]
    fn a_full_alias_cache_clears_and_keeps_working() {
        let cache_len = || ALIAS_CACHE.with(|cache| cache.borrow().len());
        ALIAS_CACHE.with(|cache| cache.borrow_mut().clear());
        assert_eq!(
            roster_hero("soldier_76").map(|hero| hero.id.as_str()),
            Some("soldier-76")
        );
        // Distinct made up ids, none of them exact roster ids, fill the cache.
        for n in 1..ALIAS_CACHE_CAP {
            assert!(roster_hero(&format!("made_up_{n}")).is_none());
        }
        assert_eq!(cache_len(), ALIAS_CACHE_CAP);
        // One more miss clears the full cache before it is stored.
        assert!(roster_hero("made_up_overflow").is_none());
        assert_eq!(cache_len(), 1);
        assert!(ALIAS_CACHE.with(|cache| !cache.borrow().contains_key("soldier_76")));
        // Lookups still resolve and refill the cache.
        assert_eq!(
            roster_hero("soldier_76").map(|hero| hero.id.as_str()),
            Some("soldier-76")
        );
        assert_eq!(
            roster_hero("d_va").map(|hero| hero.id.as_str()),
            Some("dva")
        );
        assert!(roster_hero("made_up_overflow").is_none());
        assert_eq!(cache_len(), 3);
    }

    #[test]
    fn the_white_token_parses_as_white() {
        assert_eq!(parse_hex(CANVAS_WHITE), Some((255, 255, 255)));
        assert!((label_luminance(CANVAS_WHITE) - WHITE_LUMINANCE).abs() < 1e-9);
        assert!(parse_hex(CANVAS_CHIP_RING_DARK).is_some());
        // A label token that does not parse still reads as white.
        assert_eq!(label_luminance("white"), WHITE_LUMINANCE);
    }

    #[test]
    fn initials_contrast_with_every_fill() {
        let fills = HeroRole::ALL
            .iter()
            .map(|role| role.color_hex())
            .chain([CANVAS_UNKNOWN_HERO_FILL]);
        for fill in fills {
            let label = label_color(fill);
            let lum = relative_luminance(parse_hex(fill).unwrap());
            let ratio = contrast(lum, label_luminance(label));
            assert!(ratio >= 4.5, "{fill} on {label} is {ratio:.2}");
        }
        assert_eq!(
            label_color(HeroRole::Tank.color_hex()),
            CANVAS_CHIP_RING_DARK
        );
        assert_eq!(
            label_color(HeroRole::Support.color_hex()),
            CANVAS_CHIP_RING_DARK
        );
        assert_eq!(label_color(CANVAS_UNKNOWN_HERO_FILL), CANVAS_WHITE);
        assert_eq!(
            hero_chip("soldier_76", &BLUE).label_color,
            CANVAS_CHIP_RING_DARK
        );
    }

    #[test]
    fn folded_roster_keys_do_not_collide() {
        let keys: std::collections::HashSet<_> = scuffed_types::HEROES
            .iter()
            .copied()
            .map(fold_hero_key)
            .collect();
        assert_eq!(keys.len(), scuffed_types::HEROES.len());
    }

    #[test]
    fn an_unknown_id_gets_a_grey_chip_with_a_visible_ring() {
        for raw in ["new-hero", "new_hero"] {
            let chip = hero_chip(raw, &BLUE);
            assert_eq!(chip.label, "NH", "{raw}");
            assert_eq!(chip.fill, CANVAS_UNKNOWN_HERO_FILL, "{raw}");
            assert_eq!(chip.ring, BLUE.to_css(), "{raw}");
        }
        assert!(roster_hero("").is_none());
        assert!(roster_hero("_-.").is_none());
        // A grey team colour would hide the ring, so it swaps to white.
        let grey = Color {
            r: 107,
            g: 114,
            b: 128,
            a: 1.0,
        };
        let chip = hero_chip("new-hero", &grey);
        assert_eq!(chip.ring, CANVAS_WHITE);
        assert_ne!(chip.ring, grey.to_css());
    }

    #[test]
    fn the_ring_contrasts_with_a_matching_role_fill() {
        // Red team around a Damage chip is nearly the same red.
        assert_eq!(hero_chip("tracer", &Color::RED_TEAM).ring, CANVAS_WHITE);
        // A light Tank fill gets a dark ring.
        assert_eq!(
            hero_chip("reinhardt", &Color::TANK).ring,
            CANVAS_CHIP_RING_DARK
        );
        // Distinct colours keep the team ring.
        assert_eq!(hero_chip("tracer", &BLUE).ring, BLUE.to_css());
        assert_eq!(ring_color(&BLUE, "not a colour"), BLUE.to_css());
    }
}
