use super::*;

#[test]
fn short_hero_names_need_word_boundaries() {
    // "ana" ⊂ "havana": a bare map label must not read as the hero Ana.
    assert_eq!(match_hero_in_text("HAVANA"), None);
    assert_eq!(match_hero_in_text("Hanaoka"), None);
    // Standalone the name still matches, with or without punctuation.
    assert_eq!(match_hero_in_text("Ana").as_deref(), Some("Ana"));
    assert_eq!(match_hero_in_text("ana: 14 elims").as_deref(), Some("Ana"));
}

#[test]
fn hero_ties_break_to_most_mentioned() {
    // A support duo on the scoreboard: the player's hero recurs across
    // stat lines — most-mentioned must win (this sorted ascending for a
    // while, so the LEAST-mentioned hero won every multi-match).
    let text = "HAVANA\nana 14 8 2 3400\nmercy 30 2 11 8000\nmercy 1 2 3 4\nmercy 5 6 7 8";
    assert_eq!(match_hero_in_text(text).as_deref(), Some("Mercy"));
}
