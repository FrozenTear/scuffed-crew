//! Parses and checks one "What's changed" TOML file. `build.rs` runs this on
//! the shipped files, so a broken file fails the build (and `dx build` in the
//! Containerfile) instead of reaching visitors. The app compiles it only for
//! tests, so the `toml` parser and the files' comment headers stay out of the
//! wasm.

use chrono::NaiveDate;
use serde::Deserialize;

use super::model::{Entry, Group, GroupKind, Tab};

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

/// Why a changelog file did not parse. Shown in the build or test output,
/// never to visitors.
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

pub fn has_dash(text: &str) -> bool {
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
}
