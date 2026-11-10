//! Which readers a game report says misread ([`misread_labels`]).
//!
//! A game report says what was wrong two ways: its category, and the
//! corrections it carries. The bundle allows correction keys only from
//! mode, result, hero and the six stats, so the map never travels as a
//! correction: Wrong map labels the map reader from the category alone.
//!
//! Nothing calls [`misread_labels`] yet. The reports it is for are stored
//! on the site, from many app versions, and the site server does not
//! depend on this crate; this crate only writes reports, as the current
//! version. A caller is whatever later reads stored training reports to
//! count misreads per reader.
//!
//! Some things the tracker reads have no label, because a report has no
//! correction key and no category for them: role, bans and picks, rank,
//! the queue title and team size. A report can not say one of those was
//! misread today, so they can not be labelled from one.

use std::collections::{BTreeMap, BTreeSet};

use super::GameCategory;
use super::kit::KitRead;

/// A reader a game report says misread. Sorted in [`Self::ALL`] order.
///
/// [`Self::as_str`] is the manifest correction key (and `map`), the same
/// names as `scuffed_types::SUSPECT_FIELD_NAMES`. The digit reader and the
/// vision tooling name the six stats `E`, `A`, `D`, `DMG`, `H`, `MIT`
/// (`crate::shadow::digits::FIELDS`): the same names in upper case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MisreadLabel {
    Map,
    /// The hero reader.
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

    const fn is_stat(self) -> bool {
        matches!(
            self,
            Self::Elims
                | Self::Assists
                | Self::Deaths
                | Self::Damage
                | Self::Healing
                | Self::Mitigation
        )
    }
}

/// The readers a report says misread.
///
/// - `category`: the report's `reason.category`.
/// - `app_version`: the manifest `app_version` of the build that wrote it.
/// - `corrections`: the manifest corrections, by key.
/// - `reads`: the manifest reads, by key (the same shape as [`KitRead`]).
///
/// A correction labels its reader when its value is not blank and differs
/// from the read `value` for that key: names (`hero`, `mode`, `result`)
/// compared trimmed and case-insensitive, stats as numbers (commas and
/// spaces ignored, falling back to the name rule when either side is not a
/// number). A correction for a key with no read value counts as a misread.
/// The category then adds the reader it names: Wrong map adds map, Wrong
/// mode adds mode, Wrong result adds result. Wrong stats adds only the stats
/// corrected, and Missed game and Other add nothing.
///
/// Wrong hero adds hero, except on a report from 0.6.0-alpha.2.3 or older
/// ([`is_role_only_era`]). Those builds offered a Wrong hero report, with
/// training ticked, for an Edit that changed only the role, so a Wrong hero
/// report from them labels hero only through a hero correction. An empty or
/// unparseable `app_version` is treated as one of those builds, as that is
/// the side that adds no label the report did not show.
///
/// Corrections are the member's edits on the game so far, so two reports
/// on one game carry the same corrections and give the same labels. This
/// labels one report; counting labels once per game is up to the caller.
pub fn misread_labels(
    category: GameCategory,
    app_version: &str,
    corrections: &BTreeMap<String, String>,
    reads: &BTreeMap<String, KitRead>,
) -> BTreeSet<MisreadLabel> {
    let mut labels: BTreeSet<MisreadLabel> = corrections
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .filter_map(|(k, v)| {
            let label = MisreadLabel::from_correction_key(k)?;
            let read = reads.get(k.as_str()).and_then(|r| r.value.as_deref());
            (!same_value(label, read, v)).then_some(label)
        })
        .collect();
    let named = match category {
        GameCategory::WrongMap => Some(MisreadLabel::Map),
        GameCategory::WrongHero if is_role_only_era(app_version) => None,
        GameCategory::WrongHero => Some(MisreadLabel::Hero),
        GameCategory::WrongMode => Some(MisreadLabel::Mode),
        GameCategory::WrongResult => Some(MisreadLabel::Result),
        GameCategory::WrongStats | GameCategory::MissedGame | GameCategory::Other => None,
    };
    labels.extend(named);
    labels
}

/// True when `corrected` says the same as the `read` value, so the
/// correction is not a misread. No read value is never the same.
fn same_value(label: MisreadLabel, read: Option<&str>, corrected: &str) -> bool {
    let Some(read) = read else {
        return false;
    };
    let number = |s: &str| {
        s.chars()
            .filter(|c| *c != ',' && !c.is_whitespace())
            .collect::<String>()
            .parse::<u64>()
            .ok()
    };
    if label.is_stat()
        && let (Some(a), Some(b)) = (number(read), number(corrected))
    {
        return a == b;
    }
    read.trim().to_lowercase() == corrected.trim().to_lowercase()
}

/// True when `app_version` is 0.6.0-alpha.2.3 or older by semver order
/// (numeric prerelease parts compare as numbers, a release is newer than
/// its prereleases), or is empty or not a version. Every build up to that
/// one offered a Wrong hero report for a role-only Edit.
pub fn is_role_only_era(app_version: &str) -> bool {
    const CUTOFF: &str = "0.6.0-alpha.2.3";
    match (Version::parse(app_version), Version::parse(CUTOFF)) {
        (Some(v), Some(cutoff)) => v <= cutoff,
        _ => true,
    }
}

/// `MAJOR.MINOR.PATCH[-PRE][+BUILD]`, enough to order tracker versions.
/// The build part is ignored, as semver says.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    core: (u64, u64, u64),
    /// `true` for a release, so it sorts after every prerelease of the
    /// same core.
    release: bool,
    pre: Vec<PreId>,
}

/// A prerelease part. Numbers sort before text (derive order), as in semver.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PreId {
    Num(u64),
    Text(String),
}

impl Version {
    fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.split_once('+').map_or(s, |(v, _)| v);
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (s, None),
        };
        let num = |p: &str| -> Option<u64> {
            if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
                p.parse().ok()
            } else {
                None
            }
        };
        let mut parts = core.split('.');
        let core = (
            num(parts.next()?)?,
            num(parts.next()?)?,
            num(parts.next()?)?,
        );
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => pre
                .split('.')
                .map(|p| {
                    if p.is_empty() || !p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                        None
                    } else {
                        Some(num(p).map_or_else(|| PreId::Text(p.to_string()), PreId::Num))
                    }
                })
                .collect::<Option<Vec<_>>>()?,
        };
        Some(Self {
            core,
            release: pre.is_empty(),
            pre,
        })
    }
}
