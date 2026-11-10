//! Which readers a game report says misread ([`super::training_labels`]).

use std::collections::BTreeMap;

use super::GameCategory;
use super::kit::KitRead;
use super::training_labels::{MisreadLabel, is_role_only_era, misread_labels};

/// A build after the role-only Wrong hero cutoff.
const NEW: &str = "0.6.0-alpha.3";
/// The last build that offered Wrong hero for a role-only Edit.
const OLD: &str = "0.6.0-alpha.2.3";

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn reads(pairs: &[(&str, &str)]) -> BTreeMap<String, KitRead> {
    pairs
        .iter()
        .map(|(k, v)| {
            let read = KitRead {
                value: Some(v.to_string()),
                confidence: None,
                ocr_v1: None,
                suspect: false,
            };
            (k.to_string(), read)
        })
        .collect()
}

fn labels_read(
    category: GameCategory,
    version: &str,
    corrections: &[(&str, &str)],
    read: &[(&str, &str)],
) -> Vec<&'static str> {
    misread_labels(category, version, &map(corrections), &reads(read))
        .into_iter()
        .map(MisreadLabel::as_str)
        .collect()
}

fn names(category: GameCategory, corrections: &[(&str, &str)]) -> Vec<&'static str> {
    labels_read(category, NEW, corrections, &[])
}

/// The label names are `scuffed_types::SUSPECT_FIELD_NAMES`, every
/// correction key is one of them, and the six stats are the digit reader's
/// field names in lower case. A rename on either side breaks this.
#[test]
fn the_label_names_match_the_suspect_names_and_the_digit_fields() {
    let all: Vec<&str> = MisreadLabel::ALL.iter().map(|l| l.as_str()).collect();
    assert_eq!(all, scuffed_types::SUSPECT_FIELD_NAMES);
    for name in scuffed_types::SUSPECT_FIELD_NAMES {
        let label = MisreadLabel::from_correction_key(name);
        if *name == "map" {
            assert_eq!(label, None, "map is not a correction key");
        } else {
            assert_eq!(label.map(MisreadLabel::as_str), Some(*name));
        }
    }
    assert_eq!(MisreadLabel::from_correction_key("role"), None);
    let stats: Vec<String> = MisreadLabel::ALL[4..]
        .iter()
        .map(|l| l.as_str().to_uppercase())
        .collect();
    assert_eq!(stats, crate::shadow::digits::FIELDS);
}

/// Wrong stats labels only the stats the report corrected.
#[test]
fn wrong_stats_labels_only_the_corrected_stats() {
    assert_eq!(
        names(GameCategory::WrongStats, &[("e", "15"), ("dmg", "9000")]),
        ["e", "dmg"]
    );
    assert!(names(GameCategory::WrongStats, &[]).is_empty());
    assert_eq!(
        names(
            GameCategory::WrongStats,
            &[("hero", "Mercy"), ("h", "12000"), ("d", "3")]
        ),
        ["hero", "d", "h"]
    );
}

/// Wrong map labels the map from the category, plus anything corrected.
#[test]
fn wrong_map_labels_map_and_any_corrections() {
    assert_eq!(names(GameCategory::WrongMap, &[]), ["map"]);
    assert_eq!(
        names(GameCategory::WrongMap, &[("hero", "Mercy")]),
        ["map", "hero"]
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

/// Unknown keys, a role or map key, and empty or blank values label
/// nothing.
#[test]
fn unknown_keys_and_blank_values_label_nothing() {
    let got = names(
        GameCategory::Other,
        &[
            ("role", "Tank"),
            ("map", "Havana"),
            ("hero", ""),
            ("e", "   "),
            ("x", "1"),
        ],
    );
    assert!(got.is_empty(), "{got:?}");
}

/// A correction equal to what was read is not a misread: names trimmed and
/// case-insensitive, stats as numbers.
#[test]
fn a_correction_equal_to_the_read_labels_nothing() {
    let read = [
        ("hero", "Mercy"),
        ("result", "Victory"),
        ("dmg", "9000"),
        ("e", "07"),
        ("h", "x"),
    ];
    let corrections = [
        ("hero", " mercy "),
        ("result", "VICTORY"),
        ("dmg", "9,000"),
        ("e", "7"),
        ("h", "X"),
    ];
    assert!(labels_read(GameCategory::Other, NEW, &corrections, &read).is_empty());
    assert_eq!(
        labels_read(
            GameCategory::Other,
            NEW,
            &[("hero", "Ana"), ("dmg", "9001"), ("d", "4")],
            &read
        ),
        ["hero", "d", "dmg"],
        "a different value, or a key with no read, is a misread"
    );
}

/// Wrong hero after the cutoff labels hero even with no hero correction.
#[test]
fn wrong_hero_after_the_cutoff_labels_hero() {
    for version in ["0.6.0-alpha.2.10", NEW, "0.6.0-alpha.10", "0.6.0", "0.7.0"] {
        assert_eq!(names_at(version, &[]), ["hero"], "{version}");
        assert_eq!(
            names_at(version, &[("hero", "Ana"), ("mit", "4000")]),
            ["hero", "mit"],
            "{version}"
        );
    }
}

/// Wrong hero up to 0.6.0-alpha.2.3, or with no readable version, labels
/// hero only through a hero correction: those builds offered Wrong hero
/// for a role-only Edit.
#[test]
fn wrong_hero_up_to_the_cutoff_needs_a_hero_correction() {
    for version in [
        "0.5.0",
        "0.5.12",
        "0.6.0-alpha.2",
        "0.6.0-alpha.2.2",
        OLD,
        "0.6.0-alpha.2.3+abc",
        "",
        "  ",
        "garbage",
        "0.6",
        "0.6.0.1",
        "0.6.0-",
        "0.6.0-alpha..3",
    ] {
        assert!(names_at(version, &[]).is_empty(), "{version:?}");
        assert!(
            names_at(version, &[("hero", "  ")]).is_empty(),
            "{version:?}"
        );
        assert_eq!(names_at(version, &[("mit", "1")]), ["mit"], "{version:?}");
        assert_eq!(
            names_at(version, &[("hero", "Ana")]),
            ["hero"],
            "{version:?}"
        );
    }
}

/// Numeric prerelease parts compare as numbers, and a release is newer than
/// its prereleases.
#[test]
fn the_cutoff_uses_semver_order() {
    for old in [
        "0.5.9",
        "0.6.0-alpha",
        "0.6.0-alpha.1",
        "0.6.0-alpha.2",
        OLD,
    ] {
        assert!(is_role_only_era(old), "{old}");
    }
    for new in [
        "0.6.0-alpha.2.4",
        "0.6.0-alpha.2.10",
        "0.6.0-alpha.3",
        "0.6.0-alpha.10",
        "0.6.0-beta",
        "0.6.0-alpha.2.3.1",
        "0.6.0",
        "0.6.1",
        "0.7.0",
        "1.0.0",
    ] {
        assert!(!is_role_only_era(new), "{new}");
    }
}

fn names_at(version: &str, corrections: &[(&str, &str)]) -> Vec<&'static str> {
    labels_read(GameCategory::WrongHero, version, corrections, &[])
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
