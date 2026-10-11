//! Canonical Overwatch hero list, role lookup, and OCR name matching shared
//! by the stat-tracker daemon and the site (leaderboard / roster hero filters,
//! and the strategy editor).
//!
//! Promoted from `scuffed-stat-tracker::parse` (hero-stats W1 / L1). The
//! daemon re-exports these symbols so existing `parse::…` call sites stay.

use crate::strategy::HeroRole;

use strsim::normalized_levenshtein;

/// All known hero display names (title-case). Single source of truth for
/// OCR matching and UI selectors.
pub const HEROES: &[&str] = &[
    "Ana",
    "Anran",
    "Ashe",
    "Baptiste",
    "Bastion",
    "Brigitte",
    "Cassidy",
    "D.Mon",
    "D.Va",
    "Doctrine",
    "Domina",
    "Doomfist",
    "Echo",
    "Emre",
    "Freja",
    "Genji",
    "Hanzo",
    "Hazard",
    "Illari",
    "Jetpack Cat",
    "Junker Queen",
    "Junkrat",
    "Juno",
    "Kiriko",
    "Lifeweaver",
    "Lucio",
    "Mauga",
    "Mei",
    "Mercy",
    "Mizuki",
    "Moira",
    "Orisa",
    "Pharah",
    "Ramattra",
    "Reaper",
    "Reinhardt",
    "Roadhog",
    "Shion",
    "Sierra",
    "Sigma",
    "Sojourn",
    "Soldier: 76",
    "Sombra",
    "Symmetra",
    "Torbjorn",
    "Tracer",
    "Vendetta",
    "Venture",
    "Widowmaker",
    "Winston",
    "Wrecking Ball",
    "Wuyang",
    "Zarya",
    "Zenyatta",
];

/// Count matches of `needle` in `text` that sit on word boundaries (the
/// neighbouring characters are not letters). Short hero names are substrings
/// of longer words — "ana" ⊂ "havana"/"hanaoka", the same trap class as the
/// fixed "king" ⊂ "wrecking" map bug — so they only count as standalone words.
fn word_boundary_count(text: &str, needle: &str) -> usize {
    text.match_indices(needle)
        .filter(|(i, _)| {
            let before_ok = text[..*i]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphabetic());
            let after_ok = text[i + needle.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphabetic());
            before_ok && after_ok
        })
        .count()
}

/// Match a hero from pre-split OCR lines (scoreboard path).
pub fn find_hero(lines: &[&str]) -> Option<String> {
    let text = lines.join(" ").to_lowercase();

    // --- Pass 1: exact substring matching (original logic) ---
    let mut found: Vec<(&str, usize)> = Vec::new();
    for &hero in HEROES {
        let hero_lower = hero.to_lowercase();
        // Names this short appear inside ordinary words and map labels;
        // longer ones are distinctive enough for plain substring search.
        let count = if hero_lower.len() <= 4 {
            word_boundary_count(&text, &hero_lower)
        } else {
            text.matches(&hero_lower).count()
        };
        if count > 0 {
            found.push((hero, count));
        }
    }

    if found.len() == 1 {
        return Some(found[0].0.to_string());
    }

    if found.len() > 1 {
        // Same short-name rule as the counting pass — without it, a
        // zero-number "HAVANA" map line reads as Ana's career-title line, and
        // "havana accuracy" reads as "ana accuracy".
        fn occurs(haystack: &str, needle: &str, hero_is_short: bool) -> bool {
            if hero_is_short {
                word_boundary_count(haystack, needle) > 0
            } else {
                haystack.contains(needle)
            }
        }
        let panel_keywords = ["accuracy", "critical", "weapon", "kills"];
        for &(hero, _) in &found {
            let hero_lower = hero.to_lowercase();
            let short = hero_lower.len() <= 4;
            for line in lines {
                let line_lower = line.to_lowercase();
                if occurs(&line_lower, &hero_lower, short) {
                    let num_count = line
                        .split(|c: char| !c.is_ascii_digit())
                        .filter(|w| !w.is_empty())
                        .count();
                    if num_count <= 1 {
                        return Some(hero.to_string());
                    }
                }
            }
            for kw in &panel_keywords {
                if occurs(&text, &format!("{hero_lower} {kw}"), short)
                    || text.contains(&format!("{} {}", kw, hero_lower))
                {
                    return Some(hero.to_string());
                }
            }
        }
        // Most-mentioned wins (the player's hero recurs in the career panel
        // and stat lines); alphabetical only as a deterministic last resort.
        // This was sorted ascending for a while — least-mentioned won ties.
        found.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        return Some(found[0].0.to_string());
    }

    // --- Pass 2: fuzzy matching against individual words ---
    fuzzy_match_hero(&text)
}

const FUZZY_HERO_THRESHOLD: f64 = 0.75;

/// Tokenizer [`fuzzy_match_hero`] actually scores. Collision tests must use
/// this, not a second splitter.
fn fuzzy_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split_whitespace()
}

fn fuzzy_match_hero(text: &str) -> Option<String> {
    let words: Vec<&str> = fuzzy_tokens(text).collect();

    let mut best_hero: Option<&str> = None;
    let mut best_score: f64 = 0.0;

    for &hero in HEROES {
        let hero_lower = hero.to_lowercase();
        let hero_parts: Vec<&str> = hero_lower.split_whitespace().collect();

        if hero_parts.len() == 1 {
            for &word in &words {
                let score = normalized_levenshtein(word, &hero_lower);
                if score > best_score && score >= FUZZY_HERO_THRESHOLD {
                    best_score = score;
                    best_hero = Some(hero);
                }
            }
        } else {
            for window in words.windows(hero_parts.len()) {
                let candidate = window.join(" ");
                let score = normalized_levenshtein(&candidate, &hero_lower);
                if score > best_score && score >= FUZZY_HERO_THRESHOLD {
                    best_score = score;
                    best_hero = Some(hero);
                }
            }
        }
    }

    let _ = best_score;
    best_hero.map(|h| h.to_string())
}

/// One row of the hero name table: a pack file, its display name, and its role.
///
/// The new reader and the tracker both use this table. Pack files are the
/// names inside `heroes-v1.tar` (`wrecking-ball.png`). Display names are what
/// a saved game stores (`Wrecking Ball`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeroName {
    pub file: &'static str,
    pub display: &'static str,
    pub role: HeroRole,
}

/// Every hero file in the pack, with the name and role a save should store.
pub const HERO_NAMES: &[HeroName] = &[
    HeroName {
        file: "ana.png",
        display: "Ana",
        role: HeroRole::Support,
    },
    HeroName {
        file: "anran.png",
        display: "Anran",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "ashe.png",
        display: "Ashe",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "baptiste.png",
        display: "Baptiste",
        role: HeroRole::Support,
    },
    HeroName {
        file: "bastion.png",
        display: "Bastion",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "brigitte.png",
        display: "Brigitte",
        role: HeroRole::Support,
    },
    HeroName {
        file: "cassidy.png",
        display: "Cassidy",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "dmon.png",
        display: "D.Mon",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "doctrine.png",
        display: "Doctrine",
        role: HeroRole::Support,
    },
    HeroName {
        file: "domina.png",
        display: "Domina",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "doomfist.png",
        display: "Doomfist",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "dva.png",
        display: "D.Va",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "echo.png",
        display: "Echo",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "emre.png",
        display: "Emre",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "freja.png",
        display: "Freja",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "genji.png",
        display: "Genji",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "hanzo.png",
        display: "Hanzo",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "hazard.png",
        display: "Hazard",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "illari.png",
        display: "Illari",
        role: HeroRole::Support,
    },
    HeroName {
        file: "jetpack-cat.png",
        display: "Jetpack Cat",
        role: HeroRole::Support,
    },
    HeroName {
        file: "junker-queen.png",
        display: "Junker Queen",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "junkrat.png",
        display: "Junkrat",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "juno.png",
        display: "Juno",
        role: HeroRole::Support,
    },
    HeroName {
        file: "kiriko.png",
        display: "Kiriko",
        role: HeroRole::Support,
    },
    HeroName {
        file: "lifeweaver.png",
        display: "Lifeweaver",
        role: HeroRole::Support,
    },
    HeroName {
        file: "lucio.png",
        display: "Lúcio",
        role: HeroRole::Support,
    },
    HeroName {
        file: "mauga.png",
        display: "Mauga",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "mei.png",
        display: "Mei",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "mercy.png",
        display: "Mercy",
        role: HeroRole::Support,
    },
    HeroName {
        file: "mizuki.png",
        display: "Mizuki",
        role: HeroRole::Support,
    },
    HeroName {
        file: "moira.png",
        display: "Moira",
        role: HeroRole::Support,
    },
    HeroName {
        file: "orisa.png",
        display: "Orisa",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "pharah.png",
        display: "Pharah",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "ramattra.png",
        display: "Ramattra",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "reaper.png",
        display: "Reaper",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "reinhardt.png",
        display: "Reinhardt",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "roadhog.png",
        display: "Roadhog",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "shion.png",
        display: "Shion",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "sierra.png",
        display: "Sierra",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "sigma.png",
        display: "Sigma",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "sojourn.png",
        display: "Sojourn",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "soldier-76.png",
        display: "Soldier: 76",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "sombra.png",
        display: "Sombra",
        role: HeroRole::Support,
    },
    HeroName {
        file: "symmetra.png",
        display: "Symmetra",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "torbjorn.png",
        display: "Torbjörn",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "tracer.png",
        display: "Tracer",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "vendetta.png",
        display: "Vendetta",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "venture.png",
        display: "Venture",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "widowmaker.png",
        display: "Widowmaker",
        role: HeroRole::Damage,
    },
    HeroName {
        file: "winston.png",
        display: "Winston",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "wrecking-ball.png",
        display: "Wrecking Ball",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "wuyang.png",
        display: "Wuyang",
        role: HeroRole::Support,
    },
    HeroName {
        file: "zarya.png",
        display: "Zarya",
        role: HeroRole::Tank,
    },
    HeroName {
        file: "zenyatta.png",
        display: "Zenyatta",
        role: HeroRole::Support,
    },
];

/// Empty or unknown scoreboard slots in the hero pack. A match against one
/// of these is not a hero. The pack's `special/` directory holds `empty_01`,
/// `placeholder_01` through `placeholder_06`, and `skull_01` through `skull_06`.
pub const HERO_PACK_PLACEHOLDERS: &[&str] = &[
    "special/empty_01.png",
    "special/placeholder_01.png",
    "special/placeholder_02.png",
    "special/placeholder_03.png",
    "special/placeholder_04.png",
    "special/placeholder_05.png",
    "special/placeholder_06.png",
    "special/skull_01.png",
    "special/skull_02.png",
    "special/skull_03.png",
    "special/skull_04.png",
    "special/skull_05.png",
    "special/skull_06.png",
];

/// Every file in the hero pack besides `manifest.json`.
///
/// Hero icons sit next to the manifest. The 13 special icons (`empty_01`,
/// `placeholder_01` through `placeholder_06`, `skull_01` through `skull_06`)
/// sit in `special/` and are not heroes.
pub const HERO_PACK_FILES: &[&str] = &[
    "ana.png",
    "anran.png",
    "ashe.png",
    "baptiste.png",
    "bastion.png",
    "brigitte.png",
    "cassidy.png",
    "dmon.png",
    "doctrine.png",
    "domina.png",
    "doomfist.png",
    "dva.png",
    "echo.png",
    "emre.png",
    "freja.png",
    "genji.png",
    "hanzo.png",
    "hazard.png",
    "illari.png",
    "jetpack-cat.png",
    "junker-queen.png",
    "junkrat.png",
    "juno.png",
    "kiriko.png",
    "lifeweaver.png",
    "lucio.png",
    "mauga.png",
    "mei.png",
    "mercy.png",
    "mizuki.png",
    "moira.png",
    "orisa.png",
    "pharah.png",
    "ramattra.png",
    "reaper.png",
    "reinhardt.png",
    "roadhog.png",
    "shion.png",
    "sierra.png",
    "sigma.png",
    "sojourn.png",
    "soldier-76.png",
    "sombra.png",
    "symmetra.png",
    "torbjorn.png",
    "tracer.png",
    "vendetta.png",
    "venture.png",
    "widowmaker.png",
    "winston.png",
    "wrecking-ball.png",
    "wuyang.png",
    "zarya.png",
    "zenyatta.png",
    "special/empty_01.png",
    "special/placeholder_01.png",
    "special/placeholder_02.png",
    "special/placeholder_03.png",
    "special/placeholder_04.png",
    "special/placeholder_05.png",
    "special/placeholder_06.png",
    "special/skull_01.png",
    "special/skull_02.png",
    "special/skull_03.png",
    "special/skull_04.png",
    "special/skull_05.png",
    "special/skull_06.png",
];

/// File name without a directory or `.png`. `special/placeholder_01.png` is
/// `placeholder_01`. A display name is left as written.
pub fn pack_file_stem(raw: &str) -> &str {
    let raw = raw.trim();
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    base.strip_suffix(".png")
        .or_else(|| base.strip_suffix(".PNG"))
        .unwrap_or(base)
}

/// The table row for a pack file name or stem (`wrecking-ball.png`,
/// `wrecking-ball`). Display names are not pack keys.
pub fn hero_for_pack_file(raw: &str) -> Option<&'static HeroName> {
    let stem = pack_file_stem(raw);
    HERO_NAMES
        .iter()
        .find(|hero| pack_file_stem(hero.file) == stem)
}

/// True when this read is an empty or unknown slot, not a hero.
///
/// Covers the 13 pack placeholders, a `?placeholder` / `?skull` / `?empty`
/// class, and a stem that starts with `placeholder`, `skull`, or `empty`.
pub fn is_placeholder_hero(raw: &str) -> bool {
    let stem = pack_file_stem(raw);
    if stem.is_empty() {
        return false;
    }
    if HERO_PACK_PLACEHOLDERS
        .iter()
        .any(|path| pack_file_stem(path) == stem)
    {
        return true;
    }
    let bare = stem.trim_start_matches('?');
    let lower = bare.to_ascii_lowercase();
    ["placeholder", "skull", "empty"].iter().any(|prefix| {
        lower == *prefix
            || lower.starts_with(&format!("{prefix}_"))
            || lower.starts_with(&format!("{prefix}-"))
    })
}

/// Map a reader pack key, portrait stem, or alias to the canonical display name.
///
/// Pack files are kebab-case (`wrecking-ball`, `soldier-76`, `dva`). Portrait
/// stems use underscores. Dots and accents fold, so `D.Va`, `Lúcio`,
/// `Torbjörn`, and `Soldier: 76` are the stored names. `None` when the key
/// is not a known hero. A placeholder icon is not a hero.
pub fn hero_key_to_name(raw: &str) -> Option<&'static str> {
    if is_placeholder_hero(raw) {
        return None;
    }
    let key = fold_hero_key(pack_file_stem(raw));
    if key.is_empty() {
        return None;
    }
    HERO_NAMES
        .iter()
        .find(|hero| {
            fold_hero_key(pack_file_stem(hero.file)) == key || fold_hero_key(hero.display) == key
        })
        .map(|hero| hero.display)
}

/// Canonicalize a hero identifier to its display name.
///
/// A pack key (`wrecking-ball`), a portrait stem (`wrecking_ball`), and the
/// display name itself all become `Wrecking Ball`. `Lúcio` and `Torbjörn`
/// keep their accents. An unknown string is returned with `_` and `-` turned
/// into spaces, so it still round-trips.
pub fn canonical_hero(name: &str) -> String {
    if let Some(display) = hero_key_to_name(name) {
        return display.to_string();
    }
    let cleaned = name.replace(['_', '-'], " ");
    match match_hero_in_text(&cleaned) {
        Some(found) => hero_key_to_name(&found)
            .unwrap_or(found.as_str())
            .to_string(),
        None => cleaned,
    }
}

/// Match a hero name from arbitrary OCR text (e.g. the career-panel title).
pub fn match_hero_in_text(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    find_hero(&lines)
}

/// Resolve an HTTP `?hero=` query value to a canonical [`HEROES`] display name.
///
/// - `None` / empty / whitespace → `Ok(None)` (no filter)
/// - case-insensitive exact match against [`HEROES`] → `Ok(Some(canonical))`
/// - anything else → `Err(())` (caller should 400)
///
/// Shared by public leaderboards + public members list (HS-DR P4a).
///
/// `Err(())` is intentional: callers only need a boolean unknown-hero signal
/// to map to HTTP 400 (same contract as the pre-hoist route-local helpers).
#[allow(clippy::result_unit_err)]
pub fn resolve_hero_query(raw: Option<&str>) -> Result<Option<&'static str>, ()> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let lower = trimmed.to_lowercase();
    for &hero in HEROES {
        if hero.to_lowercase() == lower {
            return Ok(Some(hero));
        }
    }
    Err(())
}

/// Current-season role for a hero name.
///
/// Matching folds case, spaces, punctuation, and common Latin accents, so
/// `Lucio` / `Lúcio`, `Torbjorn` / `Torbjörn`, and `soldier-76` all hit the
/// same [`HERO_NAMES`] row. An empty, blank, or placeholder name is `None`.
pub fn role_for_hero_name(name: &str) -> Option<HeroRole> {
    let key = fold_hero_key(pack_file_stem(name));
    if key.is_empty() || is_placeholder_hero(name) {
        return None;
    }
    HERO_NAMES
        .iter()
        .find(|hero| {
            fold_hero_key(pack_file_stem(hero.file)) == key || fold_hero_key(hero.display) == key
        })
        .map(|hero| hero.role)
}

#[cfg(test)]
fn catalog_hero(key: &str) -> Option<crate::stats::Hero> {
    crate::stats::Hero::ALL
        .iter()
        .copied()
        .find(|hero| fold_hero_key(hero.display_name()) == key)
}

fn fold_hero_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for c in c.to_lowercase() {
            let mapped = match c {
                'á' | 'à' | 'ã' | 'â' | 'ä' => 'a',
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'í' | 'ì' | 'î' | 'ï' => 'i',
                'ó' | 'ò' | 'õ' | 'ô' | 'ö' => 'o',
                'ú' | 'ù' | 'û' | 'ü' => 'u',
                'ç' => 'c',
                'ñ' => 'n',
                other if other.is_ascii_alphanumeric() => other,
                _ => continue,
            };
            out.push(mapped);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_hero_query_empty_and_case() {
        assert_eq!(resolve_hero_query(None), Ok(None));
        assert_eq!(resolve_hero_query(Some("")), Ok(None));
        assert_eq!(resolve_hero_query(Some("  ")), Ok(None));
        assert_eq!(resolve_hero_query(Some("ana")), Ok(Some("Ana")));
        assert_eq!(
            resolve_hero_query(Some("Wrecking Ball")),
            Ok(Some("Wrecking Ball"))
        );
        assert_eq!(resolve_hero_query(Some("d.va")), Ok(Some("D.Va")));
        assert!(resolve_hero_query(Some("NotAHero")).is_err());
        // Season 5. Unknown names 400 on ?hero= (leaderboards and public members).
        assert_eq!(resolve_hero_query(Some("doctrine")), Ok(Some("Doctrine")));
        assert_eq!(resolve_hero_query(Some("Doctrine")), Ok(Some("Doctrine")));
        assert_eq!(resolve_hero_query(Some("DOCTRINE")), Ok(Some("Doctrine")));
        assert_eq!(
            resolve_hero_query(Some("  doctrine  ")),
            Ok(Some("Doctrine"))
        );
    }

    #[test]
    fn match_known_and_reject_map_false_positives() {
        assert_eq!(match_hero_in_text("SHION").as_deref(), Some("Shion"));
        assert_eq!(match_hero_in_text("HAVANA"), None);
        assert_eq!(match_hero_in_text("Hanaoka"), None);
        assert_eq!(match_hero_in_text("Ana").as_deref(), Some("Ana"));
        assert_eq!(match_hero_in_text("ana: 14 elims").as_deref(), Some("Ana"));
        assert_eq!(
            match_hero_in_text("Mercy\naccuracy 42%").as_deref(),
            Some("Mercy")
        );
    }

    #[test]
    fn canonical_hero_underscores() {
        assert_eq!(canonical_hero("wrecking_ball"), "Wrecking Ball");
        assert_eq!(canonical_hero("illari"), "Illari");
    }

    /// Every hero template stem in the pack, plus the dotted and accented
    /// display names, stores one canonical name.
    #[test]
    fn every_pack_key_maps_to_the_canonical_display_name() {
        fn pack_stem(display: &str) -> String {
            display
                .to_lowercase()
                .replace('.', "")
                .replace(": ", "-")
                .replace(' ', "-")
                .replace('ö', "o")
                .replace('ú', "u")
        }

        for name in HEROES {
            let display = hero_key_to_name(name).unwrap_or_else(|| panic!("{name} has no display"));
            let stem = pack_stem(display);
            assert_eq!(hero_key_to_name(&stem), Some(display), "{stem}");
            assert_eq!(
                hero_key_to_name(&stem.replace('-', "_")),
                Some(display),
                "{stem} underscore"
            );
            assert_eq!(hero_key_to_name(display), Some(display), "{display}");
            assert_eq!(canonical_hero(&stem), display, "{stem}");
        }

        assert_eq!(hero_key_to_name("dva"), Some("D.Va"));
        assert_eq!(hero_key_to_name("d.va"), Some("D.Va"));
        assert_eq!(hero_key_to_name("lucio"), Some("Lúcio"));
        assert_eq!(hero_key_to_name("Lúcio"), Some("Lúcio"));
        assert_eq!(hero_key_to_name("torbjorn"), Some("Torbjörn"));
        assert_eq!(hero_key_to_name("Torbjörn"), Some("Torbjörn"));
        assert_eq!(hero_key_to_name("soldier-76"), Some("Soldier: 76"));
        assert_eq!(hero_key_to_name("soldier_76"), Some("Soldier: 76"));
        assert_eq!(hero_key_to_name("wrecking-ball"), Some("Wrecking Ball"));
        assert_eq!(canonical_hero("wrecking-ball"), "Wrecking Ball");
        assert_eq!(role_for_hero_name("Wrecking Ball"), Some(HeroRole::Tank));
        assert_eq!(role_for_hero_name("wrecking-ball"), Some(HeroRole::Tank));
    }

    /// The pack file list and the name table are one roster. A file is either
    /// a named hero or one of the 13 special icons.
    #[test]
    fn every_hero_pack_file_is_a_named_hero_or_a_known_placeholder() {
        const SPECIAL: &[&str] = &[
            "empty_01",
            "placeholder_01",
            "placeholder_02",
            "placeholder_03",
            "placeholder_04",
            "placeholder_05",
            "placeholder_06",
            "skull_01",
            "skull_02",
            "skull_03",
            "skull_04",
            "skull_05",
            "skull_06",
        ];
        assert_eq!(HERO_PACK_PLACEHOLDERS.len(), 13);
        assert_eq!(SPECIAL.len(), 13);
        for stem in SPECIAL {
            let path = format!("special/{stem}.png");
            assert!(
                HERO_PACK_PLACEHOLDERS.contains(&path.as_str()),
                "{path} is not a known placeholder"
            );
            assert!(is_placeholder_hero(stem), "{stem}");
            assert!(is_placeholder_hero(&path), "{path}");
            assert!(hero_key_to_name(stem).is_none(), "{stem}");
            assert!(hero_for_pack_file(stem).is_none(), "{stem}");
        }
        let mut named = 0usize;
        let mut placeholders = 0usize;
        for path in HERO_PACK_FILES {
            let in_table = HERO_NAMES.iter().any(|hero| hero.file == *path);
            let placeholder = HERO_PACK_PLACEHOLDERS.contains(path);
            assert!(
                in_table || placeholder,
                "{path} is not in the name table and is not a known placeholder"
            );
            assert!(
                !(in_table && placeholder),
                "{path} is both a hero and a placeholder"
            );
            if in_table {
                named += 1;
                assert!(!is_placeholder_hero(path), "{path}");
                let row = hero_for_pack_file(path).unwrap_or_else(|| panic!("{path}"));
                assert_eq!(row.file, *path);
                assert_eq!(hero_key_to_name(pack_file_stem(path)), Some(row.display));
                assert_eq!(role_for_hero_name(path), Some(row.role));
            } else {
                placeholders += 1;
                assert!(is_placeholder_hero(path), "{path}");
                assert!(hero_key_to_name(path).is_none(), "{path}");
                assert!(hero_for_pack_file(path).is_none(), "{path}");
            }
        }
        assert_eq!(named, HERO_NAMES.len());
        assert_eq!(placeholders, 13);
        for hero in HERO_NAMES {
            assert!(
                HERO_PACK_FILES.contains(&hero.file),
                "{} is missing from the pack file list",
                hero.file
            );
        }
        for path in HERO_PACK_PLACEHOLDERS {
            assert!(
                HERO_PACK_FILES.contains(path),
                "{path} is missing from the pack file list"
            );
        }
    }

    /// D.Mon (added 2026-08-18, WL-5): the career panel prints "D.MON", the
    /// portrait reference stem is "dmon", the query form is "d.mon" — all
    /// three must land on the same canonical name and must not collide with
    /// D.Va, whose dotted form is one character off.
    #[test]
    fn dmon_resolves_and_is_distinct_from_dva() {
        assert_eq!(match_hero_in_text("D.MON").as_deref(), Some("D.Mon"));
        assert_eq!(
            match_hero_in_text("D.Mon: 12 elims").as_deref(),
            Some("D.Mon")
        );
        assert_eq!(canonical_hero("dmon"), "D.Mon");
        assert_eq!(resolve_hero_query(Some("d.mon")), Ok(Some("D.Mon")));
        assert_eq!(match_hero_in_text("D.VA").as_deref(), Some("D.Va"));
        assert_eq!(canonical_hero("dva"), "D.Va");
    }

    #[test]
    fn heroes_nonempty_unique() {
        assert!(HEROES.len() > 30);
        let mut v: Vec<&str> = HEROES.to_vec();
        v.sort();
        v.dedup();
        assert_eq!(v.len(), HEROES.len());
    }

    /// Season 5 Support hero (2026-10-06). Alphabetical in [`HEROES`]. Fuzzy
    /// score against every other hero token, map token, and common scoreboard
    /// word stays under the fuzzy threshold.
    #[test]
    fn doctrine_matches_and_does_not_fuzzy_collide() {
        assert_eq!(match_hero_in_text("DOCTRINE").as_deref(), Some("Doctrine"));
        assert_eq!(match_hero_in_text("Doctrine").as_deref(), Some("Doctrine"));
        assert_eq!(canonical_hero("doctrine"), "Doctrine");
        assert_eq!(resolve_hero_query(Some("doctrine")), Ok(Some("Doctrine")));

        let pos = HEROES
            .iter()
            .position(|h| *h == "Doctrine")
            .expect("Doctrine is listed");
        assert!(pos > 0 && HEROES[pos - 1] < "Doctrine");
        assert!(pos + 1 < HEROES.len() && HEROES[pos + 1] > "Doctrine");

        // One-edit OCR misses still land on Doctrine, not Domina / Doomfist.
        assert_eq!(match_hero_in_text("DOCTRIN").as_deref(), Some("Doctrine"));
        assert_eq!(match_hero_in_text("DOCTRNE").as_deref(), Some("Doctrine"));

        let mut words: Vec<String> = Vec::new();
        for &hero in HEROES {
            if hero.eq_ignore_ascii_case("Doctrine") {
                continue;
            }
            for token in fuzzy_tokens(hero) {
                words.push(token.to_lowercase());
            }
        }
        assert!(!crate::stats::MapName::ALL.is_empty());
        for map in crate::stats::MapName::ALL {
            let name = map.display_name();
            assert_ne!(
                match_hero_in_text(name).as_deref(),
                Some("Doctrine"),
                "{name:?} matched Doctrine"
            );
            for token in fuzzy_tokens(name) {
                words.push(token.to_lowercase());
            }
        }
        for word in [
            "eliminations",
            "assists",
            "deaths",
            "damage",
            "healing",
            "mitigation",
            "victory",
            "defeat",
            "draw",
            "accuracy",
            "critical",
            "weapon",
            "kills",
            "elims",
            "objective",
            "contesting",
            "eliminated",
            "final",
            "blow",
            "card",
            "player",
            "hero",
            "role",
            "score",
            "time",
            "support",
            "tank",
            "payload",
            "overtime",
            "round",
            "attack",
            "defense",
            "escort",
            "hybrid",
            "control",
            "push",
            "flashpoint",
            "clash",
        ] {
            for token in fuzzy_tokens(word) {
                words.push(token.to_lowercase());
            }
        }

        for word in words {
            let score = normalized_levenshtein(&word, "doctrine");
            assert!(
                score < FUZZY_HERO_THRESHOLD,
                "{word:?} scores {score} against doctrine (threshold {FUZZY_HERO_THRESHOLD})"
            );
            assert_ne!(
                match_hero_in_text(&word).as_deref(),
                Some("Doctrine"),
                "{word:?} matched Doctrine"
            );
        }
    }

    #[test]
    fn every_shared_hero_name_has_a_role() {
        for name in HEROES {
            assert!(role_for_hero_name(name).is_some(), "{name} has no role");
        }
        assert_eq!(role_for_hero_name("Domina"), Some(HeroRole::Tank));
        assert_eq!(role_for_hero_name("Mizuki"), Some(HeroRole::Support));
        assert_eq!(role_for_hero_name("Wuyang"), Some(HeroRole::Support));
        assert_eq!(role_for_hero_name("Sombra"), Some(HeroRole::Support));
        assert_eq!(role_for_hero_name("Doctrine"), Some(HeroRole::Support));
        assert_eq!(role_for_hero_name("D.Mon"), Some(HeroRole::Tank));
        assert_eq!(role_for_hero_name("Shion"), Some(HeroRole::Damage));
        assert_eq!(role_for_hero_name("Jetpack Cat"), Some(HeroRole::Support));
        assert_eq!(role_for_hero_name("Lucio"), Some(HeroRole::Support));
        assert_eq!(role_for_hero_name("Lúcio"), Some(HeroRole::Support));
        assert_eq!(role_for_hero_name("NotAHero"), None);
        assert_eq!(role_for_hero_name(""), None);
        assert_eq!(role_for_hero_name("   "), None);
        assert_eq!(role_for_hero_name("SOMBRA"), Some(HeroRole::Support));
        assert_eq!(role_for_hero_name("  sombra "), Some(HeroRole::Support));
        assert_eq!(role_for_hero_name("Torbjörn"), Some(HeroRole::Damage));
        assert_eq!(role_for_hero_name("soldier-76"), Some(HeroRole::Damage));
    }

    /// Pins the roles the tracker stamps on a game. A stored role is whatever
    /// was captured at the time, so this is the current roster, not a rewrite
    /// of older Sombra games.
    #[test]
    fn tracker_stamped_roles() {
        let cases = [
            ("D.Mon", HeroRole::Tank),
            ("d.mon", HeroRole::Tank),
            ("dmon", HeroRole::Tank),
            ("Jetpack Cat", HeroRole::Support),
            ("Doctrine", HeroRole::Support),
            ("Sombra", HeroRole::Support),
        ];
        for (name, role) in cases {
            assert_eq!(role_for_hero_name(name), Some(role), "{name}");
        }
    }

    /// Each enum variant folds to exactly one shared name, and the shared
    /// names with no variant are exactly the three tracker-only heroes.
    #[test]
    fn hero_variants_and_shared_names_cover_each_other() {
        use crate::stats::Hero;
        for hero in Hero::ALL {
            let key = fold_hero_key(hero.display_name());
            let hits: Vec<_> = HEROES
                .iter()
                .copied()
                .filter(|name| fold_hero_key(name) == key)
                .collect();
            assert_eq!(
                hits.len(),
                1,
                "{} folded to {key} and matched {hits:?}",
                hero.display_name()
            );
        }
        let mut unresolved: Vec<&str> = HEROES
            .iter()
            .copied()
            .filter(|name| catalog_hero(&fold_hero_key(name)).is_none())
            .collect();
        unresolved.sort_unstable();
        assert_eq!(unresolved, ["D.Mon", "Jetpack Cat", "Shion"]);
    }
}
