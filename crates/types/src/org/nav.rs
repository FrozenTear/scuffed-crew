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

/// Admin-editable public navigation. Unknown ids are dropped when resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavConfig {
    pub items: Vec<NavItemConfig>,
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
        id: "scrims",
        label: "Scrims",
        description: "Scrim finder",
    },
];

impl Default for NavConfig {
    fn default() -> Self {
        // Lean public chrome. Extra features stay in the catalog as Hidden
        // so admins can promote them without a deploy.
        Self {
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
    pub fn normalize(&mut self) {
        let known: Vec<&str> = NAV_CATALOG.iter().map(|e| e.id).collect();
        let mut seen = std::collections::HashSet::new();
        self.items.retain(|it| {
            if !known.contains(&it.id.as_str()) {
                return false;
            }
            seen.insert(it.id.clone())
        });
        for entry in NAV_CATALOG {
            if !seen.contains(entry.id) {
                let order = self
                    .items
                    .iter()
                    .filter(|i| i.placement == NavPlacement::Hidden)
                    .map(|i| i.order)
                    .max()
                    .map(|o| o.saturating_add(1))
                    .unwrap_or(0);
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

    pub fn catalog_label(id: &str) -> Option<&'static str> {
        NAV_CATALOG.iter().find(|e| e.id == id).map(|e| e.label)
    }

    pub fn set_placement(&mut self, id: &str, placement: NavPlacement) {
        let Some(idx) = self.items.iter().position(|i| i.id == id) else {
            return;
        };
        if self.items[idx].placement == placement {
            return;
        }
        let next_order = self
            .items
            .iter()
            .filter(|i| i.placement == placement)
            .map(|i| i.order)
            .max()
            .map(|o| o.saturating_add(1))
            .unwrap_or(0);
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
        assert_eq!(primary, ["members", "tournaments", "news", "forum"]);
        assert!(
            cfg.items
                .iter()
                .any(|i| i.id == "strategy" && i.placement == NavPlacement::Hidden)
        );
        assert!(
            cfg.items
                .iter()
                .any(|i| i.id == "scrims" && i.placement == NavPlacement::Hidden)
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
}
