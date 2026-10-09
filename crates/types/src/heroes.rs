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

/// Map a reader pack key, portrait stem, or alias to the canonical display name.
///
/// Pack files are kebab-case (`wrecking-ball`, `soldier-76`, `dva`). Portrait
/// stems use underscores. Dots and accents fold, so `D.Va`, `Lúcio`,
/// `Torbjörn`, and `Soldier: 76` are the stored names. `None` when the key
/// is not a known hero.
pub fn hero_key_to_name(raw: &str) -> Option<&'static str> {
    let key = fold_hero_key(raw);
    if key.is_empty() {
        return None;
    }
    if let Some(hero) = catalog_hero(&key) {
        return Some(hero.display_name());
    }
    match key.as_str() {
        "dmon" => Some("D.Mon"),
        "shion" => Some("Shion"),
        "jetpackcat" => Some("Jetpack Cat"),
        _ => None,
    }
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
/// `Lucio` / `Lúcio`, `Torbjorn` / `Torbjörn`, and `soldier-76` all hit.
/// Names that match [`crate::stats::Hero`] use that variant's role. Three
/// names are on [`HEROES`] and not on that enum yet: D.Mon (Tank), Shion
/// (Damage), Jetpack Cat (Support). An empty or blank name is `None`.
pub fn role_for_hero_name(name: &str) -> Option<HeroRole> {
    let key = fold_hero_key(name);
    if key.is_empty() {
        return None;
    }
    if let Some(hero) = catalog_hero(&key) {
        return Some(hero.role());
    }
    match key.as_str() {
        "dmon" => Some(HeroRole::Tank),
        "shion" => Some(HeroRole::Damage),
        "jetpackcat" => Some(HeroRole::Support),
        _ => None,
    }
}

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
