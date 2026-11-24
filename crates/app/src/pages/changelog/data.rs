//! The "What's changed" content: one TOML file per tab under
//! `crates/app/content/changelog/`. `build.rs` parses and checks both files
//! and writes them out as Rust, so a broken file fails the build, the wasm
//! carries no TOML parser, and nothing is parsed at runtime. This module
//! holds the pure page logic (load, batch, announce), tested without a
//! browser. The "how to add an entry" note is at the top of each file.

use std::rc::Rc;

use super::model::{Entry, Group, GroupKind, Tab};
#[cfg(test)]
use super::parse::parse;
use chrono::NaiveDate;

// `site_entries()` and `tracker_entries()`, newest first, from build.rs.
include!(concat!(env!("OUT_DIR"), "/changelog_entries.rs"));

/// Entries on first paint, and how many each "Show older" adds.
pub const PAGE_SIZE: usize = 10;

thread_local! {
    static SITE: Rc<[Entry]> = site_entries().into();
    static TRACKER: Rc<[Entry]> = tracker_entries().into();
}

#[cfg(test)]
thread_local! {
    /// Fake file content for render tests. Never set outside tests.
    pub static TEST_SOURCE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// The shipped entries for `tab`, newest first. Built once per thread.
pub fn load(tab: Tab) -> Rc<[Entry]> {
    #[cfg(test)]
    if let Some(src) = TEST_SOURCE.with(|slot| slot.borrow().clone()) {
        return parse(tab, &src)
            .unwrap_or_else(|e| panic!("test source: {e}"))
            .into();
    }
    match tab {
        Tab::Site => SITE.with(Rc::clone),
        Tab::Tracker => TRACKER.with(Rc::clone),
    }
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

/// The polite live region text. Quiet on a cold load. A toggle navigation
/// names the tab and its first-paint count; "Show older" clears it again,
/// so it reads the same after a switch as on a fresh load (focus moves to
/// the first new heading, the region says nothing).
pub fn live_text(tab: Tab, announce: bool, first_paint: usize) -> String {
    match (announce, first_paint) {
        (false, _) => String::new(),
        (true, 0) => format!("{} changes, no entries yet.", tab.label()),
        (true, 1) => format!("{} changes, 1 entry.", tab.label()),
        (true, n) => format!("{} changes, {n} entries.", tab.label()),
    }
}

pub fn empty_text(tab: Tab) -> String {
    format!("New {} changes will show up here.", tab.label())
}

#[cfg(test)]
mod tests {
    use super::super::parse::has_dash;
    use super::*;

    const SITE_SOURCE: &str = include_str!("../../../content/changelog/site.toml");
    const TRACKER_SOURCE: &str = include_str!("../../../content/changelog/tracker.toml");

    fn source(tab: Tab) -> &'static str {
        match tab {
            Tab::Site => SITE_SOURCE,
            Tab::Tracker => TRACKER_SOURCE,
        }
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

    /// build.rs output is exactly what the parser makes of the shipped files.
    #[test]
    fn the_build_output_matches_the_shipped_files() {
        assert_eq!(
            &*load(Tab::Site),
            parse(Tab::Site, SITE_SOURCE).unwrap().as_slice()
        );
        assert_eq!(
            &*load(Tab::Tracker),
            parse(Tab::Tracker, TRACKER_SOURCE).unwrap().as_slice()
        );
    }

    #[test]
    fn the_shipped_files_parse_and_hold_the_first_real_entries() {
        for tab in Tab::ALL {
            let entries = load(tab);
            assert!(!entries.is_empty(), "{tab:?}");
            let src = source(tab);
            assert!(src.contains("How to add an entry"), "{tab:?}");
            assert!(!has_dash(src), "{tab:?}: the file has an em or en dash");
            // The placeholders are gone, from the file and from the page.
            assert!(!src.contains("PLACEHOLDER"), "{tab:?}");
            for entry in entries.iter() {
                let version = entry.version.as_deref().unwrap_or("");
                assert!(!version.contains("placeholder"), "{tab:?}: {version}");
                let items = entry
                    .changes
                    .iter()
                    .chain(entry.groups.iter().flat_map(|g| g.items.iter()));
                for item in items {
                    assert!(!item.to_lowercase().contains("placeholder"), "{item}");
                    assert!(!item.contains("Overwatch 2"), "{item}");
                }
            }
        }

        let site = load(Tab::Site);
        assert_eq!(site.len(), 1);
        assert_eq!(site[0].date_iso(), "2026-11-17");
        assert_eq!(site[0].version, None);
        let kinds: Vec<GroupKind> = site[0].groups.iter().map(|g| g.kind).collect();
        assert_eq!(
            kinds,
            [GroupKind::Added, GroupKind::Changed, GroupKind::Fixed]
        );

        let tracker = load(Tab::Tracker);
        assert_eq!(tracker.len(), 1);
        assert_eq!(tracker[0].version.as_deref(), Some("v0.6.0-alpha.3.1"));
        assert_eq!(tracker[0].date_iso(), "2026-11-15");
        assert_eq!(
            tracker[0].link.as_deref(),
            Some(
                "https://github.com/FrozenTear/scuffed-crew/releases/tag/stat-tracker-v0.6.0-alpha.3.1"
            )
        );
    }

    #[test]
    fn the_test_source_overrides_the_shipped_entries() {
        TEST_SOURCE.with(|slot| *slot.borrow_mut() = Some(fake_entries(Tab::Site, 2)));
        let entries = load(Tab::Site);
        TEST_SOURCE.with(|slot| *slot.borrow_mut() = None);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].changes, ["Fake change 1."]);
        assert_eq!(load(Tab::Site).len(), 1, "back to the shipped file");
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
    fn live_region_is_quiet_on_cold_load_and_speaks_after_a_switch() {
        assert_eq!(live_text(Tab::Site, false, 10), "");
        assert_eq!(live_text(Tab::Site, false, 0), "");
        assert_eq!(live_text(Tab::Site, true, 10), "Site changes, 10 entries.");
        assert_eq!(
            live_text(Tab::Tracker, true, 1),
            "Tracker changes, 1 entry."
        );
        assert_eq!(
            live_text(Tab::Tracker, true, 0),
            "Tracker changes, no entries yet."
        );
    }

    #[test]
    fn copy_matches_the_spec() {
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
