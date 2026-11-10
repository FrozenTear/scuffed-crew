//! The misread labels a game report may feed the reader benchmark.
//!
//! A training report says what was wrong two ways: its category, and the
//! corrections it carries. The bundle allows correction keys only from
//! mode, result, hero and the six stats, so the map and the role never
//! travel as a correction. Wrong map still labels the map reader from the
//! category. There is no role label: a role fix is not a hero icon misread,
//! and the Edit form offers no report for a role-only fix.

use std::collections::{BTreeMap, BTreeSet};

use super::GameCategory;

/// A reader a training report says misread. Sorted in [`Self::ALL`] order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MisreadLabel {
    Map,
    /// The hero icon reader.
    Hero,
    Mode,
    Result,
    Elims,
    Assists,
    Deaths,
    Damage,
    Healing,
    Mitigation,
}

impl MisreadLabel {
    pub const ALL: [Self; 10] = [
        Self::Map,
        Self::Hero,
        Self::Mode,
        Self::Result,
        Self::Elims,
        Self::Assists,
        Self::Deaths,
        Self::Damage,
        Self::Healing,
        Self::Mitigation,
    ];

    /// Stable name: the correction key, or "map".
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Hero => "hero",
            Self::Mode => "mode",
            Self::Result => "result",
            Self::Elims => "e",
            Self::Assists => "a",
            Self::Deaths => "d",
            Self::Damage => "dmg",
            Self::Healing => "h",
            Self::Mitigation => "mit",
        }
    }

    /// The label for a correction key the bundle allows. "map" is not one.
    pub fn from_correction_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|l| *l != Self::Map && l.as_str() == key)
    }
}

/// The labels a report under `category` with these `corrections` may feed
/// the benchmark: every allowed correction key with a value, plus the
/// reader the category names. Wrong stats adds no stat of its own, only the
/// stats corrected, and Missed game and Other add nothing.
pub fn misread_labels(
    category: GameCategory,
    corrections: &BTreeMap<String, String>,
) -> BTreeSet<MisreadLabel> {
    let mut labels: BTreeSet<MisreadLabel> = corrections
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .filter_map(|(k, _)| MisreadLabel::from_correction_key(k))
        .collect();
    let named = match category {
        GameCategory::WrongMap => Some(MisreadLabel::Map),
        GameCategory::WrongHero => Some(MisreadLabel::Hero),
        GameCategory::WrongMode => Some(MisreadLabel::Mode),
        GameCategory::WrongResult => Some(MisreadLabel::Result),
        GameCategory::WrongStats | GameCategory::MissedGame | GameCategory::Other => None,
    };
    labels.extend(named);
    labels
}
