//! Which misread labels a training report may feed the reader benchmark
//! ([`super::training_labels`]).

use std::collections::BTreeMap;

use super::GameCategory;
use super::training_labels::{MisreadLabel, misread_labels};

fn corrections(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn names(category: GameCategory, pairs: &[(&str, &str)]) -> Vec<&'static str> {
    misread_labels(category, &corrections(pairs))
        .into_iter()
        .map(MisreadLabel::as_str)
        .collect()
}

/// The labels' names are stable: the nine correction keys the bundle
/// allows, plus map. There is no role label.
#[test]
fn the_label_names_are_the_correction_keys_plus_map() {
    let all: Vec<&str> = MisreadLabel::ALL.iter().map(|l| l.as_str()).collect();
    assert_eq!(
        all,
        [
            "map", "hero", "mode", "result", "e", "a", "d", "dmg", "h", "mit"
        ]
    );
    assert!(!all.contains(&"role"));
    for label in MisreadLabel::ALL {
        let key = MisreadLabel::from_correction_key(label.as_str());
        if label == MisreadLabel::Map {
            assert_eq!(key, None, "map is not a correction key");
        } else {
            assert_eq!(key, Some(label));
        }
    }
    assert_eq!(MisreadLabel::from_correction_key("role"), None);
}

/// Wrong stats labels only the stats the report corrected.
#[test]
fn wrong_stats_labels_only_the_corrected_stats() {
    assert_eq!(
        names(GameCategory::WrongStats, &[("e", "15"), ("dmg", "9000")]),
        ["e", "dmg"]
    );
    assert!(names(GameCategory::WrongStats, &[]).is_empty());
}

/// A Wrong stats report with a hero fix too labels both.
#[test]
fn wrong_stats_with_a_hero_fix_labels_the_stats_and_hero() {
    assert_eq!(
        names(
            GameCategory::WrongStats,
            &[("hero", "Mercy"), ("h", "12000"), ("d", "3")]
        ),
        ["hero", "d", "h"]
    );
}

/// Wrong map labels the map reader from the category, as map is not a
/// correction key, plus anything corrected.
#[test]
fn wrong_map_labels_map_and_any_corrections() {
    assert_eq!(names(GameCategory::WrongMap, &[]), ["map"]);
    assert_eq!(
        names(GameCategory::WrongMap, &[("hero", "Mercy")]),
        ["map", "hero"]
    );
}

/// Wrong hero labels the hero icon reader even with no hero correction.
#[test]
fn wrong_hero_labels_hero_without_a_correction() {
    assert_eq!(names(GameCategory::WrongHero, &[]), ["hero"]);
    assert_eq!(
        names(GameCategory::WrongHero, &[("hero", "Ana"), ("mit", "4000")]),
        ["hero", "mit"]
    );
}

/// Wrong mode and Wrong result label their reader.
#[test]
fn wrong_mode_and_result_label_their_reader() {
    assert_eq!(names(GameCategory::WrongMode, &[]), ["mode"]);
    assert_eq!(names(GameCategory::WrongResult, &[]), ["result"]);
    assert_eq!(
        names(GameCategory::WrongResult, &[("result", "win"), ("a", "7")]),
        ["result", "a"]
    );
}

/// Missed game and Other label only what the corrections hold.
#[test]
fn missed_game_and_other_label_only_the_corrections() {
    for category in [GameCategory::MissedGame, GameCategory::Other] {
        assert!(names(category, &[]).is_empty(), "{category:?}");
        assert_eq!(
            names(category, &[("mode", "push"), ("e", "20")]),
            ["mode", "e"],
            "{category:?}"
        );
    }
}

/// Unknown keys, a role or map key, and empty values label nothing.
#[test]
fn unknown_keys_and_empty_values_label_nothing() {
    assert!(
        names(
            GameCategory::Other,
            &[
                ("role", "Tank"),
                ("map", "Havana"),
                ("hero", ""),
                ("x", "1")
            ]
        )
        .is_empty()
    );
}

/// Every category with every allowed key labels each key once.
#[test]
fn every_allowed_key_labels_once_under_every_category() {
    let all = [
        ("mode", "push"),
        ("result", "win"),
        ("hero", "Ana"),
        ("e", "1"),
        ("a", "2"),
        ("d", "3"),
        ("dmg", "4"),
        ("h", "5"),
        ("mit", "6"),
    ];
    for category in GameCategory::ALL {
        let got = names(category, &all);
        let want: Vec<&str> = if category == GameCategory::WrongMap {
            MisreadLabel::ALL.iter().map(|l| l.as_str()).collect()
        } else {
            MisreadLabel::ALL[1..].iter().map(|l| l.as_str()).collect()
        };
        assert_eq!(got, want, "{category:?}");
    }
}
