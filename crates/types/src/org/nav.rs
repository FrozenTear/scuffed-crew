use serde::{Deserialize, Serialize};

/// Where a nav item appears in the public chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NavPlacement {
    /// Top-level bar (Members, Tournaments, …).
    #[default]
    Primary,
    /// Overflow “More” menu.
    More,
    /// Not shown in nav (routes still work via URL).
    Hidden,
}

impl NavPlacement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::More => "more",
            Self::Hidden => "hidden",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "more" => Self::More,
            "hidden" => Self::Hidden,
            _ => Self::Primary,
        }
    }
}

/// One configurable public nav entry (catalog id + placement + order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavItemConfig {
    /// Stable id from the built-in catalog (`members`, `forum`, …).
    pub id: String,
    pub placement: NavPlacement,
    /// Sort key within the same placement (lower = first).
    pub order: u32,
}

/// Admin-editable public navigation. Unknown ids are dropped on
/// [`NavConfig::normalize`] (save / public persist), not on GET deserialize —
/// Admin Settings must still *show* every payload row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavConfig {
    pub items: Vec<NavItemConfig>,
}

/// One Admin Settings bucket row (catalog label, or the raw id if unknown).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavEditorRow {
    pub id: String,
    pub label: String,
}

/// Static catalog entry — labels/routes live in the app; ids are shared.
#[derive(Debug, Clone, Copy)]
pub struct NavCatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

/// Built-in pages that can appear in the public nav.
pub const NAV_CATALOG: &[NavCatalogEntry] = &[
    NavCatalogEntry {
        id: "members",
        label: "Members",
        description: "Public roster",
    },
    NavCatalogEntry {
        id: "tournaments",
        label: "Tournaments",
        description: "Competitions",
    },
    NavCatalogEntry {
        id: "news",
        label: "News",
        description: "Announcements",
    },
    NavCatalogEntry {
        id: "forum",
        label: "Forum",
        description: "Discussion boards",
    },
    NavCatalogEntry {
        id: "events",
        label: "Events",
        description: "Schedule / play nights",
    },
    NavCatalogEntry {
        id: "community",
        label: "Community",
        description: "Community hub",
    },
    NavCatalogEntry {
        id: "feed",
        label: "Feed",
        description: "Activity feed",
    },
    NavCatalogEntry {
        id: "polls",
        label: "Polls",
        description: "Member polls",
    },
    NavCatalogEntry {
        id: "blog",
        label: "Blog",
        description: "Long-form posts",
    },
    NavCatalogEntry {
        id: "wiki",
        label: "Wiki",
        description: "Knowledge base",
    },
    NavCatalogEntry {
        id: "stats",
        label: "Stats",
        description: "Player statistics",
    },
    NavCatalogEntry {
        id: "strategy",
        label: "Strategy",
        description: "Strategy browser",
    },
    NavCatalogEntry {
        id: "patch_notes",
        label: "Patch Notes",
        description: "Game patch notes",
    },
    NavCatalogEntry {
        id: "scrims",
        label: "Scrims",
        description: "Scrim finder",
    },
    NavCatalogEntry {
        id: "chat",
        label: "Chat",
        description: "Team chat",
    },
];

impl Default for NavConfig {
    fn default() -> Self {
        // Small-org pack: day-to-day surfaces primary; competitive tools
        // (tournaments / scrims / strategy) sit in More until the org uses them.
        // Admins can re-order in Settings → Navigation without a deploy.
        Self {
            items: vec![
                item("members", NavPlacement::Primary, 0),
                item("forum", NavPlacement::Primary, 1),
                item("events", NavPlacement::Primary, 2),
                item("stats", NavPlacement::Primary, 3),
                item("news", NavPlacement::More, 0),
                item("tournaments", NavPlacement::More, 1),
                item("scrims", NavPlacement::More, 2),
                item("strategy", NavPlacement::More, 3),
                item("patch_notes", NavPlacement::More, 4),
                item("community", NavPlacement::Hidden, 0),
                item("feed", NavPlacement::Hidden, 1),
                item("polls", NavPlacement::Hidden, 2),
                item("blog", NavPlacement::Hidden, 3),
                item("wiki", NavPlacement::Hidden, 4),
                item("chat", NavPlacement::Hidden, 5),
            ],
        }
    }
}

fn item(id: &str, placement: NavPlacement, order: u32) -> NavItemConfig {
    NavItemConfig {
        id: id.into(),
        placement,
        order,
    }
}

impl NavConfig {
    pub fn from_json(s: &str) -> Self {
        if s.trim().is_empty() {
            return Self::default();
        }
        match serde_json::from_str::<Self>(s) {
            Ok(mut cfg) => {
                cfg.normalize();
                cfg
            }
            Err(_) => Self::default(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Drop unknown ids, de-dupe, and append any missing catalog entries as Hidden.
    ///
    /// This is the **save / persist** path. Do **not** run it on Admin Settings
    /// load: a GET payload id that the compiled `NAV_CATALOG` does not yet
    /// know (or a stale WASM catalog) is deleted and vanishes from all buckets.
    pub fn normalize(&mut self) {
        let known: Vec<&str> = NAV_CATALOG.iter().map(|e| e.id).collect();
        let mut seen = std::collections::HashSet::new();
        self.items.retain(|it| {
            if !known.contains(&it.id.as_str()) {
                return false;
            }
            seen.insert(it.id.clone())
        });
        self.merge_missing_catalog();
    }

    /// Admin Settings load: keep every GET payload row, de-dupe, add missing
    /// catalog ids as Hidden. Never drops an id the API already returned.
    pub fn prepare_for_editor(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.items.retain(|it| seen.insert(it.id.clone()));
        self.merge_missing_catalog();
    }

    fn merge_missing_catalog(&mut self) {
        let seen: std::collections::HashSet<String> =
            self.items.iter().map(|i| i.id.clone()).collect();
        for entry in NAV_CATALOG {
            if !seen.contains(entry.id) {
                let order = self.next_order(NavPlacement::Hidden);
                self.items.push(item(entry.id, NavPlacement::Hidden, order));
            }
        }
    }

    pub fn items_in(&self, placement: NavPlacement) -> Vec<&NavItemConfig> {
        let mut list: Vec<_> = self
            .items
            .iter()
            .filter(|i| i.placement == placement)
            .collect();
        list.sort_by_key(|i| i.order);
        list
    }

    /// Resolved placement for a catalog id. Missing ids count as Hidden so the
    /// Admin editor can list every `NAV_CATALOG` entry even when stored JSON
    /// (or an un-normalized in-memory value) omitted them.
    pub fn placement_of(&self, id: &str) -> NavPlacement {
        self.items
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.placement)
            .unwrap_or(NavPlacement::Hidden)
    }

    /// Admin Settings buckets: `NAV_CATALOG` ∪ GET payload rows in `placement`.
    /// A payload id is never omitted just because it is missing from the catalog.
    pub fn editor_items(&self, placement: NavPlacement) -> Vec<NavEditorRow> {
        let mut seen = std::collections::HashSet::<&str>::new();
        let mut rows: Vec<(u32, NavEditorRow)> = Vec::new();

        for entry in NAV_CATALOG {
            let (item_placement, order) = self
                .items
                .iter()
                .find(|i| i.id == entry.id)
                .map(|i| (i.placement, i.order))
                .unwrap_or((NavPlacement::Hidden, u32::MAX));
            if item_placement != placement {
                continue;
            }
            seen.insert(entry.id);
            rows.push((
                order,
                NavEditorRow {
                    id: entry.id.to_string(),
                    label: entry.label.to_string(),
                },
            ));
        }
        for item in &self.items {
            if item.placement != placement || seen.contains(item.id.as_str()) {
                continue;
            }
            let label = Self::catalog_label(&item.id)
                .unwrap_or(item.id.as_str())
                .to_string();
            rows.push((
                item.order,
                NavEditorRow {
                    id: item.id.clone(),
                    label,
                },
            ));
        }
        rows.sort_by_key(|(order, row)| (*order, row.id.clone()));
        rows.into_iter().map(|(_, row)| row).collect()
    }

    pub fn catalog_label(id: &str) -> Option<&'static str> {
        NAV_CATALOG.iter().find(|e| e.id == id).map(|e| e.label)
    }

    fn next_order(&self, placement: NavPlacement) -> u32 {
        self.items
            .iter()
            .filter(|i| i.placement == placement)
            .map(|i| i.order)
            .max()
            .map(|o| o.saturating_add(1))
            .unwrap_or(0)
    }

    pub fn set_placement(&mut self, id: &str, placement: NavPlacement) {
        let in_catalog = NAV_CATALOG.iter().any(|e| e.id == id);
        let in_items = self.items.iter().any(|i| i.id == id);
        if !in_catalog && !in_items {
            return;
        }
        let Some(idx) = self.items.iter().position(|i| i.id == id) else {
            let order = self.next_order(placement);
            self.items.push(item(id, placement, order));
            return;
        };
        if self.items[idx].placement == placement {
            return;
        }
        let next_order = self.next_order(placement);
        self.items[idx].placement = placement;
        self.items[idx].order = next_order;
    }

    pub fn move_item(&mut self, id: &str, delta: i32) {
        let Some(idx) = self.items.iter().position(|i| i.id == id) else {
            return;
        };
        let placement = self.items[idx].placement;
        let mut group: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.placement == placement)
            .map(|(i, _)| i)
            .collect();
        group.sort_by_key(|&i| self.items[i].order);
        let Some(pos) = group.iter().position(|&i| i == idx) else {
            return;
        };
        let new_pos = (pos as i32 + delta).clamp(0, group.len() as i32 - 1) as usize;
        if new_pos == pos {
            return;
        }
        group.swap(pos, new_pos);
        for (order, &i) in group.iter().enumerate() {
            self.items[i].order = order as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_lean() {
        let cfg = NavConfig::default();
        let primary: Vec<_> = cfg
            .items_in(NavPlacement::Primary)
            .into_iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(primary, ["members", "forum", "events", "stats"]);
        let more: Vec<_> = cfg
            .items_in(NavPlacement::More)
            .into_iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(
            more,
            ["news", "tournaments", "scrims", "strategy", "patch_notes"]
        );
        assert!(
            cfg.items
                .iter()
                .any(|i| i.id == "tournaments" && i.placement == NavPlacement::More)
        );
        assert!(
            cfg.items
                .iter()
                .any(|i| i.id == "patch_notes" && i.placement == NavPlacement::More),
            "fresh defaults put Patch Notes in More; Admin can move without a redeploy"
        );
        assert_eq!(NavConfig::catalog_label("patch_notes"), Some("Patch Notes"));
        assert!(
            cfg.items
                .iter()
                .any(|i| i.id == "chat" && i.placement == NavPlacement::Hidden),
            "team chat is member-gated; default public nav stays hidden"
        );
    }

    #[test]
    fn empty_json_uses_default() {
        assert_eq!(NavConfig::from_json(""), NavConfig::default());
    }

    #[test]
    fn normalize_adds_missing_catalog() {
        let mut cfg = NavConfig {
            items: vec![item("members", NavPlacement::Primary, 0)],
        };
        cfg.normalize();
        assert_eq!(cfg.items.len(), NAV_CATALOG.len());
    }

    #[test]
    fn normalize_adds_new_catalog_id_as_hidden() {
        // Existing Contabo `site_settings.nav` JSON predates `patch_notes`.
        // Unknown ids are dropped; missing catalog ids appear as Hidden so
        // Admin can place them without a redeploy. Fresh Default stays More.
        let mut stored = NavConfig {
            items: vec![
                item("members", NavPlacement::Primary, 0),
                item("forum", NavPlacement::Primary, 1),
                item("news", NavPlacement::More, 0),
                item("not_a_real_page", NavPlacement::Primary, 9),
            ],
        };
        stored.normalize();
        assert!(
            stored.items.iter().all(|i| i.id != "not_a_real_page"),
            "unknown ids are dropped"
        );
        let added = stored
            .items
            .iter()
            .find(|i| i.id == "patch_notes")
            .expect("new catalog id is merged in");
        assert_eq!(added.placement, NavPlacement::Hidden);
        assert_eq!(stored.items.len(), NAV_CATALOG.len());
    }

    #[test]
    fn default_survives_normalize_with_patch_notes_in_more() {
        let mut cfg = NavConfig::default();
        cfg.normalize();
        assert!(
            cfg.items
                .iter()
                .any(|i| i.id == "patch_notes" && i.placement == NavPlacement::More)
        );
        assert_eq!(cfg.items.len(), NAV_CATALOG.len());
    }

    /// Contabo live `site_settings.nav` shape (2026-09-14): custom Primary/More,
    /// strategy/scrims Hidden, no `patch_notes` row in the stored blob.
    fn contabo_stored_without_patch_notes() -> NavConfig {
        NavConfig {
            items: vec![
                item("members", NavPlacement::Primary, 0),
                item("tournaments", NavPlacement::Primary, 1),
                item("news", NavPlacement::Primary, 2),
                item("forum", NavPlacement::Primary, 3),
                item("events", NavPlacement::More, 0),
                item("stats", NavPlacement::More, 1),
                item("community", NavPlacement::Hidden, 0),
                item("feed", NavPlacement::Hidden, 1),
                item("polls", NavPlacement::Hidden, 2),
                item("blog", NavPlacement::Hidden, 3),
                item("wiki", NavPlacement::Hidden, 4),
                item("strategy", NavPlacement::Hidden, 5),
                item("scrims", NavPlacement::Hidden, 6),
                item("chat", NavPlacement::Hidden, 8),
            ],
        }
    }

    #[test]
    fn editor_items_lists_missing_catalog_id_as_hidden() {
        let stored = contabo_stored_without_patch_notes();
        assert!(
            stored.items.iter().all(|i| i.id != "patch_notes"),
            "fixture is the pre-merge stored blob"
        );
        let hidden: Vec<String> = stored
            .editor_items(NavPlacement::Hidden)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert!(
            hidden.iter().any(|id| id == "patch_notes"),
            "Admin Hidden bucket must show catalog ids omitted from stored JSON"
        );
        assert!(
            !stored
                .editor_items(NavPlacement::Primary)
                .iter()
                .any(|e| e.id == "patch_notes")
        );
        assert!(
            !stored
                .editor_items(NavPlacement::More)
                .iter()
                .any(|e| e.id == "patch_notes")
        );
        let editor_ids: std::collections::HashSet<String> = [
            NavPlacement::Primary,
            NavPlacement::More,
            NavPlacement::Hidden,
        ]
        .into_iter()
        .flat_map(|p| stored.editor_items(p))
        .map(|e| e.id)
        .collect();
        let catalog_ids: std::collections::HashSet<String> =
            NAV_CATALOG.iter().map(|e| e.id.to_string()).collect();
        assert_eq!(
            editor_ids, catalog_ids,
            "every catalog id appears in exactly one Admin bucket"
        );
    }

    /// Live `GET /api/settings` nav on ow.scuffedcrew.no (2026-09-14).
    fn live_contabo_get_nav() -> NavConfig {
        serde_json::from_str(
            r#"{
              "items": [
                {"id":"members","placement":"primary","order":0},
                {"id":"tournaments","placement":"primary","order":1},
                {"id":"news","placement":"primary","order":2},
                {"id":"forum","placement":"primary","order":3},
                {"id":"events","placement":"more","order":0},
                {"id":"stats","placement":"more","order":1},
                {"id":"community","placement":"hidden","order":0},
                {"id":"feed","placement":"hidden","order":1},
                {"id":"polls","placement":"hidden","order":2},
                {"id":"blog","placement":"hidden","order":3},
                {"id":"wiki","placement":"hidden","order":4},
                {"id":"strategy","placement":"hidden","order":5},
                {"id":"scrims","placement":"hidden","order":6},
                {"id":"patch_notes","placement":"hidden","order":7},
                {"id":"chat","placement":"hidden","order":8}
              ]
            }"#,
        )
        .expect("live GET nav JSON")
    }

    #[test]
    fn live_get_payload_patch_notes_renders_in_hidden() {
        let parsed = live_contabo_get_nav();
        assert!(
            parsed
                .items
                .iter()
                .any(|i| i.id == "patch_notes" && i.placement == NavPlacement::Hidden),
            "GET deserialize must keep the payload row (no drop on parse)"
        );
        let mut n = parsed;
        n.prepare_for_editor();
        let hidden: Vec<String> = n
            .editor_items(NavPlacement::Hidden)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert!(
            hidden.iter().any(|id| id == "patch_notes"),
            "Admin Hidden must list the GET payload id"
        );
        assert_eq!(hidden.iter().filter(|id| *id == "patch_notes").count(), 1);
    }

    #[test]
    fn prepare_for_editor_keeps_payload_ids_normalize_would_drop() {
        let mut n = live_contabo_get_nav();
        n.items.push(item("future_page", NavPlacement::Hidden, 99));
        n.prepare_for_editor();
        assert!(
            n.items.iter().any(|i| i.id == "future_page"),
            "load path must not strip GET ids the local catalog does not know"
        );
        assert!(
            n.editor_items(NavPlacement::Hidden)
                .iter()
                .any(|r| r.id == "future_page" && r.label == "future_page")
        );

        let mut dropped = live_contabo_get_nav();
        dropped
            .items
            .push(item("future_page", NavPlacement::Hidden, 99));
        dropped.normalize();
        assert!(
            dropped.items.iter().all(|i| i.id != "future_page"),
            "save-path normalize still drops unknown ids"
        );
        assert!(dropped.items.iter().any(|i| i.id == "patch_notes"));
    }

    #[test]
    fn set_placement_inserts_missing_catalog_id() {
        let mut stored = contabo_stored_without_patch_notes();
        stored.set_placement("patch_notes", NavPlacement::More);
        assert!(
            stored
                .items
                .iter()
                .any(|i| i.id == "patch_notes" && i.placement == NavPlacement::More),
            "moving a catalog id that was never stored must insert it"
        );
        stored.set_placement("not_a_real_page", NavPlacement::Primary);
        assert!(stored.items.iter().all(|i| i.id != "not_a_real_page"));
    }
}
