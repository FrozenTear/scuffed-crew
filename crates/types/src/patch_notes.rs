//! Strategy Patch Notes contract.
//!
//! Site (`GET /strategy/patch-notes`) fetches `GET /api/strategy/patch-notes`
//! and unwraps the standard `{ "data": [...] }` envelope (`ApiSuccess`).
//! Field names here are the payload Site deserializes after unwrap.

use serde::{Deserialize, Serialize};

/// One Overwatch patch card on the Strategy Patch Notes page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchNote {
    pub version: String,
    /// Display date (`YYYY-MM-DD`). Site renders this string as-is.
    pub date: String,
    pub title: Option<String>,
    /// Official / source URL ("View official patch notes").
    pub url: String,
    #[serde(default)]
    pub hero_updates: Vec<PatchHeroUpdate>,
    #[serde(default)]
    pub sections: Vec<PatchSection>,
}

/// Hero-level balance / bugfix block.
///
/// `change_type` values Site colors: `buff`, `nerf`, `adjustment`, `bugfix`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchHeroUpdate {
    pub hero_id: String,
    pub hero_name: String,
    pub change_type: String,
    #[serde(default)]
    pub changes: Vec<PatchChange>,
    pub dev_comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchChange {
    pub ability: Option<String>,
    pub description: String,
    pub change_type: String,
}

/// Non-hero section (Bug Fixes, Maps, Competitive, General, …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchSection {
    pub category: String,
    pub items: Vec<String>,
}

/// Officer POST body. Field names match [`PatchNote`] so Site's JSON stays valid
/// if the same payload is reused; do not rename these keys.
pub type CreatePatchNoteRequest = PatchNote;

/// Officer PUT body — every field optional; `version` stays the path key.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdatePatchNoteRequest {
    pub date: Option<String>,
    pub title: Option<Option<String>>,
    pub url: Option<String>,
    pub hero_updates: Option<Vec<PatchHeroUpdate>>,
    pub sections: Option<Vec<PatchSection>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiSuccess;

    fn sample() -> PatchNote {
        PatchNote {
            version: "1.0.0".into(),
            date: "2026-09-14".into(),
            title: Some("Test".into()),
            url: "https://example.test/notes".into(),
            hero_updates: vec![PatchHeroUpdate {
                hero_id: "ana".into(),
                hero_name: "Ana".into(),
                change_type: "buff".into(),
                changes: vec![PatchChange {
                    ability: Some("Biotic Rifle".into()),
                    description: "Reload slightly faster.".into(),
                    change_type: "buff".into(),
                }],
                dev_comment: None,
            }],
            sections: vec![PatchSection {
                category: "Bug Fixes".into(),
                items: vec!["Fixed a tooltip typo.".into()],
            }],
        }
    }

    #[test]
    fn list_envelope_is_data_array() {
        let body = ApiSuccess {
            data: vec![sample()],
        };
        let value = serde_json::to_value(&body).expect("serialize");
        let data = value.get("data").expect("data key");
        assert!(data.is_array());
        assert_eq!(data[0]["version"], "1.0.0");
        assert_eq!(data[0]["hero_updates"][0]["hero_name"], "Ana");
        assert_eq!(data[0]["sections"][0]["category"], "Bug Fixes");
    }

    #[test]
    fn missing_nested_lists_default_empty() {
        let note: PatchNote = serde_json::from_value(serde_json::json!({
            "version": "x",
            "date": "2026-01-01",
            "url": "https://example.test"
        }))
        .expect("minimal payload");
        assert!(note.hero_updates.is_empty());
        assert!(note.sections.is_empty());
        assert!(note.title.is_none());
    }
}
