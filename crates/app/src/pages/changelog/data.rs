//! The "What's changed" content: one TOML file per tab under
//! `crates/app/content/changelog/`, embedded at build time. This module is
//! pure (parse, order, batch, announce) so it is tested without a browser.
//! The "how to add an entry" note is at the top of each file.

use chrono::NaiveDate;
use serde::Deserialize;

/// Entries on first paint, and how many each "Show older" adds.
pub const PAGE_SIZE: usize = 10;

const SITE_SOURCE: &str = include_str!("../../../content/changelog/site.toml");
const TRACKER_SOURCE: &str = include_str!("../../../content/changelog/tracker.toml");

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

    pub fn source(self) -> &'static str {
        match self {
            Tab::Site => SITE_SOURCE,
            Tab::Tracker => TRACKER_SOURCE,
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
    /// Tracker release tag, for example `v0.6.0-alpha.3.1`. Site entries have none.
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    entry: Vec<RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    version: Option<String>,
    date: String,
    #[serde(default)]
    changes: Vec<String>,
    #[serde(default)]
    added: Vec<String>,
    #[serde(default)]
    changed: Vec<String>,
    #[serde(default)]
    fixed: Vec<String>,
    link: Option<String>,
}

/// Why a changelog file did not load. Shown to the crew in test output and
/// the console, never to visitors (they get the plain error copy).
#[derive(Clone, Debug, PartialEq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn clean_items(field: &str, n: usize, items: Vec<String>) -> Result<Vec<String>, ParseError> {
    items
        .into_iter()
        .map(|item| {
            let item = item.trim().to_string();
            if item.is_empty() {
                Err(ParseError(format!(
                    "entry {n}: an item in {field} is empty"
                )))
            } else {
                Ok(item)
            }
        })
        .collect()
}

fn has_dash(text: &str) -> bool {
    text.contains(['\u{2013}', '\u{2014}'])
}

/// Parse one tab's file and order it newest first. Entries with the same
/// date keep their file order (the file is written newest first).
pub fn parse(tab: Tab, source: &str) -> Result<Vec<Entry>, ParseError> {
    let raw: RawFile =
        toml::from_str(source).map_err(|e| ParseError(format!("{} changelog: {e}", tab.slug())))?;
    let mut entries = Vec::with_capacity(raw.entry.len());
    for (i, raw) in raw.entry.into_iter().enumerate() {
        let n = i + 1;
        let date = NaiveDate::parse_from_str(raw.date.trim(), "%Y-%m-%d").map_err(|_| {
            ParseError(format!(
                "entry {n}: date \"{}\" is not YYYY-MM-DD",
                raw.date
            ))
        })?;
        let version = raw
            .version
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        match (tab, &version) {
            (Tab::Tracker, None) => {
                return Err(ParseError(format!(
                    "entry {n}: Tracker entries need a version"
                )));
            }
            (Tab::Site, Some(_)) => {
                return Err(ParseError(format!(
                    "entry {n}: version is only for Tracker entries"
                )));
            }
            _ => {}
        }
        let changes = clean_items("changes", n, raw.changes)?;
        let mut groups = Vec::new();
        for (kind, items) in [
            (GroupKind::Added, raw.added),
            (GroupKind::Changed, raw.changed),
            (GroupKind::Fixed, raw.fixed),
        ] {
            let items = clean_items(kind.heading(), n, items)?;
            if !items.is_empty() {
                groups.push(Group { kind, items });
            }
        }
        if changes.is_empty() && groups.is_empty() {
            return Err(ParseError(format!("entry {n}: no changes listed")));
        }
        let link = raw
            .link
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty());
        if let Some(link) = &link
            && (!link.starts_with("https://") || link.contains(char::is_whitespace))
        {
            return Err(ParseError(format!(
                "entry {n}: link must be a public https URL"
            )));
        }
        let entry = Entry {
            version,
            date,
            changes,
            groups,
            link,
        };
        let text_has_dash = entry.version.as_deref().is_some_and(has_dash)
            || entry.changes.iter().any(|s| has_dash(s))
            || entry
                .groups
                .iter()
                .any(|g| g.items.iter().any(|s| has_dash(s)));
        if text_has_dash {
            return Err(ParseError(format!(
                "entry {n}: use a comma or a full stop, not an em or en dash"
            )));
        }
        entries.push(entry);
    }
    // Stable, so same-day entries keep the file order.
    entries.sort_by_key(|e| std::cmp::Reverse(e.date));
    Ok(entries)
}

#[cfg(test)]
thread_local! {
    /// Fake file content for render tests. Never set outside tests.
    pub static TEST_SOURCE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// The shipped file for `tab`.
pub fn load(tab: Tab) -> Result<Vec<Entry>, ParseError> {
    #[cfg(test)]
    if let Some(src) = TEST_SOURCE.with(|slot| slot.borrow().clone()) {
        return parse(tab, &src);
    }
    parse(tab, tab.source())
}

/// How many entries the first paint shows.
pub fn first_batch(total: usize) -> usize {
    total.min(PAGE_SIZE)
}

/// How many are shown after one more "Show older".
pub fn next_batch(shown: usize, total: usize) -> usize {
    shown.saturating_add(PAGE_SIZE).min(total)
}

/// "Show older" stays while there is anything left to show.
pub fn has_older(shown: usize, total: usize) -> bool {
    shown < total
}

/// What the list region holds right now, for the live region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListState {
    Ready { shown: usize },
    Error,
}

/// The polite live region text. Quiet on a cold load; after a toggle
/// navigation it names the tab and the painted count; a load error always
/// speaks.
pub fn live_text(tab: Tab, state: ListState, after_switch: bool) -> String {
    match state {
        ListState::Error => format!("{} changes could not be loaded. Retry.", tab.label()),
        ListState::Ready { .. } if !after_switch => String::new(),
        ListState::Ready { shown: 0 } => format!("{} changes, no entries yet.", tab.label()),
        ListState::Ready { shown: 1 } => format!("{} changes, 1 entry.", tab.label()),
        ListState::Ready { shown } => format!("{} changes, {shown} entries.", tab.label()),
    }
}

pub fn error_text(tab: Tab) -> String {
    format!("{} changes could not be loaded.", tab.label())
}

pub fn empty_text(tab: Tab) -> String {
    format!("New {} changes will show up here.", tab.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_SITE: &str = r#"
[[entry]]
date = "2026-10-01"
changes = ["Older fake change."]

[[entry]]
date = "2026-11-15"
added = ["Fake page for Player01."]
fixed = ["Fake fix."]
link = "https://example.com/notes"

[[entry]]
date = "2026-11-15"
changes = ["Second fake change on the same day."]
"#;

    fn site(src: &str) -> Result<Vec<Entry>, ParseError> {
        parse(Tab::Site, src)
    }

    fn fake_entries(tab: Tab, n: usize) -> String {
        let mut out = String::new();
        for i in 0..n {
            let day = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(i as u64);
            if tab == Tab::Tracker {
                out.push_str(&format!("[[entry]]\nversion = \"v0.0.{i}\"\n"));
            } else {
                out.push_str("[[entry]]\n");
            }
            out.push_str(&format!(
                "date = \"{}\"\nchanges = [\"Fake change {i}.\"]\n\n",
                day.format("%Y-%m-%d")
            ));
        }
        out
    }

    #[test]
    fn the_shipped_files_parse_and_carry_one_marked_placeholder() {
        for tab in Tab::ALL {
            let entries = load(tab).unwrap_or_else(|e| panic!("{e}"));
            assert!(!entries.is_empty(), "{tab:?}");
            let src = tab.source();
            assert!(src.contains("How to add an entry"), "{tab:?}");
            assert!(!has_dash(src), "{tab:?}: the file has an em or en dash");
        }
    }

    #[test]
    fn parses_fields_and_orders_newest_first() {
        let entries = site(FAKE_SITE).unwrap();
        let dates: Vec<String> = entries.iter().map(Entry::date_iso).collect();
        assert_eq!(dates, ["2026-11-15", "2026-11-15", "2026-10-01"]);
        // Same day keeps the file order.
        assert_eq!(entries[0].groups.len(), 2);
        assert_eq!(entries[1].changes, ["Second fake change on the same day."]);
        assert_eq!(
            entries[0].link.as_deref(),
            Some("https://example.com/notes")
        );
        assert_eq!(entries[1].link, None);
        assert_eq!(entries[0].date_label(), "15 Nov 2026");
        assert_eq!(entries[2].date_label(), "1 Oct 2026");
    }

    #[test]
    fn groups_appear_only_when_the_source_has_them() {
        let entries = site(FAKE_SITE).unwrap();
        let kinds: Vec<GroupKind> = entries[0].groups.iter().map(|g| g.kind).collect();
        assert_eq!(kinds, [GroupKind::Added, GroupKind::Fixed]);
        assert!(entries[0].changes.is_empty());
        assert!(entries[1].groups.is_empty());

        // An empty group array is the same as no group.
        let e = site("[[entry]]\ndate = \"2026-11-01\"\nchanged = []\nfixed = [\"x\"]\n").unwrap();
        assert_eq!(e[0].groups.len(), 1);
        assert_eq!(e[0].groups[0].kind, GroupKind::Fixed);

        // Groups always render Added, Changed, Fixed whatever the file order.
        let e = site("[[entry]]\ndate = \"2026-11-01\"\nfixed = [\"f\"]\nadded = [\"a\"]\nchanged = [\"c\"]\n")
            .unwrap();
        let kinds: Vec<&str> = e[0].groups.iter().map(|g| g.kind.heading()).collect();
        assert_eq!(kinds, ["Added", "Changed", "Fixed"]);
    }

    #[test]
    fn tracker_entries_need_a_version_and_site_entries_have_none() {
        let t = parse(
            Tab::Tracker,
            "[[entry]]\nversion = \"v0.6.0-alpha.3.1\"\ndate = \"2026-11-02\"\nchanges = [\"x\"]\n",
        )
        .unwrap();
        assert_eq!(t[0].version.as_deref(), Some("v0.6.0-alpha.3.1"));
        assert!(
            parse(
                Tab::Tracker,
                "[[entry]]\ndate = \"2026-11-02\"\nchanges = [\"x\"]\n"
            )
            .is_err()
        );
        assert!(
            site("[[entry]]\nversion = \"v1\"\ndate = \"2026-11-02\"\nchanges = [\"x\"]\n")
                .is_err()
        );
    }

    #[test]
    fn rejects_bad_files() {
        for bad in [
            "[[entry]]\ndate = \"15 Nov 2026\"\nchanges = [\"x\"]\n",
            "[[entry]]\ndate = \"2026-11-15\"\n",
            "[[entry]]\ndate = \"2026-11-15\"\nchanges = [\"  \"]\n",
            "[[entry]]\ndate = \"2026-11-15\"\nchanges = [\"x\"]\nlink = \"http://example.com\"\n",
            "[[entry]]\ndate = \"2026-11-15\"\nchanges = [\"x\"]\nlink = \"javascript:alert(1)\"\n",
            "[[entry]]\ndate = \"2026-11-15\"\nchanges = [\"a \u{2014} b\"]\n",
            "[[entry]]\ndate = \"2026-11-15\"\nfixed = [\"a \u{2013} b\"]\n",
            "[[entry]]\ndate = \"2026-11-15\"\nchanges = [\"x\"]\ntypo = 1\n",
            "[[entry]\n",
        ] {
            assert!(site(bad).is_err(), "should reject: {bad}");
        }
        assert_eq!(site("").unwrap(), Vec::new());
    }

    #[test]
    fn show_older_batches_ten_at_a_time() {
        let entries = parse(Tab::Tracker, &fake_entries(Tab::Tracker, 23)).unwrap();
        assert_eq!(entries.len(), 23);
        assert_eq!(entries[0].version.as_deref(), Some("v0.0.22"));
        let mut shown = first_batch(entries.len());
        assert_eq!(shown, 10);
        assert!(has_older(shown, 23));
        shown = next_batch(shown, 23);
        assert_eq!(shown, 20);
        assert!(has_older(shown, 23));
        shown = next_batch(shown, 23);
        assert_eq!(shown, 23);
        assert!(!has_older(shown, 23));
        assert_eq!(first_batch(4), 4);
        assert!(!has_older(4, 4));
        assert_eq!(first_batch(0), 0);
        assert!(!has_older(0, 0));
        assert_eq!(next_batch(10, 10), 10);
    }

    #[test]
    fn live_region_is_quiet_on_cold_load_and_speaks_after_a_switch_or_error() {
        assert_eq!(
            live_text(Tab::Site, ListState::Ready { shown: 10 }, false),
            ""
        );
        assert_eq!(
            live_text(Tab::Site, ListState::Ready { shown: 0 }, false),
            ""
        );
        assert_eq!(
            live_text(Tab::Site, ListState::Ready { shown: 10 }, true),
            "Site changes, 10 entries."
        );
        assert_eq!(
            live_text(Tab::Tracker, ListState::Ready { shown: 1 }, true),
            "Tracker changes, 1 entry."
        );
        assert_eq!(
            live_text(Tab::Tracker, ListState::Ready { shown: 0 }, true),
            "Tracker changes, no entries yet."
        );
        for after_switch in [false, true] {
            assert_eq!(
                live_text(Tab::Tracker, ListState::Error, after_switch),
                "Tracker changes could not be loaded. Retry."
            );
        }
    }

    #[test]
    fn copy_matches_the_spec() {
        assert_eq!(
            error_text(Tab::Tracker),
            "Tracker changes could not be loaded."
        );
        assert_eq!(empty_text(Tab::Site), "New Site changes will show up here.");
        assert_eq!(Tab::Site.page_label(), "What's changed, Site");
        assert_eq!(Tab::Tracker.page_label(), "What's changed, Tracker");
        for tab in Tab::ALL {
            assert_eq!(Tab::from_slug(tab.slug()), Some(tab));
        }
        assert_eq!(Tab::from_slug("Site"), None);
        assert_eq!(Tab::from_slug("unknown"), None);
    }
}
