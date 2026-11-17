//! The "What's changed" types. Pure (chrono only), so `build.rs` compiles
//! this file too and the generated entries use the same types as the page.

use chrono::NaiveDate;

/// The two changelogs. The URL slug is the source of truth for the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Site,
    Tracker,
}

impl Tab {
    pub const ALL: [Tab; 2] = [Tab::Site, Tab::Tracker];

    pub fn from_slug(slug: &str) -> Option<Tab> {
        match slug {
            "site" => Some(Tab::Site),
            "tracker" => Some(Tab::Tracker),
            _ => None,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Tab::Site => "site",
            Tab::Tracker => "tracker",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Site => "Site",
            Tab::Tracker => "Tracker",
        }
    }

    /// The page label in the document title, before `| <org>`.
    pub fn page_label(self) -> String {
        format!("What's changed, {}", self.label())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKind {
    Added,
    Changed,
    Fixed,
}

impl GroupKind {
    pub fn heading(self) -> &'static str {
        match self {
            GroupKind::Added => "Added",
            GroupKind::Changed => "Changed",
            GroupKind::Fixed => "Fixed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub kind: GroupKind,
    pub items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Tracker version, for example `v0.6.0-alpha.3.1`. Site entries have none.
    pub version: Option<String>,
    pub date: NaiveDate,
    /// The ungrouped list. Empty when the entry only uses groups.
    pub changes: Vec<String>,
    /// Added, Changed, Fixed, in that order, and only the ones the file has.
    pub groups: Vec<Group>,
    /// A public https link. Private releases have none.
    pub link: Option<String>,
}

impl Entry {
    /// `datetime` value for `<time>`.
    pub fn date_iso(&self) -> String {
        self.date.format("%Y-%m-%d").to_string()
    }

    /// Visible date, for example `15 Nov 2026`. Dates are calendar days in
    /// Europe/Oslo with no time, so there is nothing to convert.
    pub fn date_label(&self) -> String {
        self.date.format("%-d %b %Y").to_string()
    }
}
