//! Canonical Overwatch hero list + OCR name matching shared by the
//! stat-tracker daemon and the site (leaderboard / roster hero filters).
//!
//! Promoted from `scuffed-stat-tracker::parse` (hero-stats W1 / L1). The
//! daemon re-exports these symbols so existing `parse::…` call sites stay.

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
    "D.Va",
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

fn fuzzy_match_hero(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();

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

/// Canonicalize a hero identifier to its display name — e.g. a portrait file
/// stem like "wrecking_ball" or "illari" becomes "Wrecking Ball" / "Illari".
/// Returns the input (underscores spaced) when nothing matches, so unknown
/// names still round-trip.
pub fn canonical_hero(name: &str) -> String {
    let cleaned = name.replace('_', " ");
    match_hero_in_text(&cleaned).unwrap_or(cleaned)
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn heroes_nonempty_unique() {
        assert!(HEROES.len() > 30);
        let mut v: Vec<&str> = HEROES.to_vec();
        v.sort();
        v.dedup();
        assert_eq!(v.len(), HEROES.len());
    }
}
