//! Static Overwatch map catalog for the strategy editor picker.
//!
//! There is no `/api/maps` list on the current contract. IDs are the
//! `snake_case` serde names of [`scuffed_types::MapName`] (`kings_row`).

use scuffed_types::MapName;
use scuffed_types::strategy::GameMode;

/// One selectable map in the editor picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickableMap {
    pub id: &'static str,
    pub name: &'static str,
    pub game_mode: GameMode,
}

/// Competitive maps the picker can assign as `strategy.map_id`.
///
/// Names/modes are literals because [`MapName`] helpers are not `const`. Tests
/// assert they stay aligned with `MapName::display_name` / `game_mode`.
pub const PICKABLE_MAPS: &[PickableMap] = &[
    // Escort
    PickableMap {
        id: map_slug(MapName::CircuitRoyal),
        name: "Circuit Royal",
        game_mode: GameMode::Escort,
    },
    PickableMap {
        id: map_slug(MapName::Dorado),
        name: "Dorado",
        game_mode: GameMode::Escort,
    },
    PickableMap {
        id: map_slug(MapName::Havana),
        name: "Havana",
        game_mode: GameMode::Escort,
    },
    PickableMap {
        id: map_slug(MapName::Junkertown),
        name: "Junkertown",
        game_mode: GameMode::Escort,
    },
    PickableMap {
        id: map_slug(MapName::Rialto),
        name: "Rialto",
        game_mode: GameMode::Escort,
    },
    PickableMap {
        id: map_slug(MapName::Route66),
        name: "Route 66",
        game_mode: GameMode::Escort,
    },
    PickableMap {
        id: map_slug(MapName::ShambaliMonastery),
        name: "Shambali Monastery",
        game_mode: GameMode::Escort,
    },
    PickableMap {
        id: map_slug(MapName::WatchpointGibraltar),
        name: "Watchpoint: Gibraltar",
        game_mode: GameMode::Escort,
    },
    // Hybrid
    PickableMap {
        id: map_slug(MapName::BlizzardWorld),
        name: "Blizzard World",
        game_mode: GameMode::Hybrid,
    },
    PickableMap {
        id: map_slug(MapName::Eichenwalde),
        name: "Eichenwalde",
        game_mode: GameMode::Hybrid,
    },
    PickableMap {
        id: map_slug(MapName::Hollywood),
        name: "Hollywood",
        game_mode: GameMode::Hybrid,
    },
    PickableMap {
        id: map_slug(MapName::KingsRow),
        name: "King's Row",
        game_mode: GameMode::Hybrid,
    },
    PickableMap {
        id: map_slug(MapName::Midtown),
        name: "Midtown",
        game_mode: GameMode::Hybrid,
    },
    PickableMap {
        id: map_slug(MapName::NeonJunction),
        name: "Neon Junction",
        game_mode: GameMode::Hybrid,
    },
    PickableMap {
        id: map_slug(MapName::Numbani),
        name: "Numbani",
        game_mode: GameMode::Hybrid,
    },
    PickableMap {
        id: map_slug(MapName::Paraiso),
        name: "Paraíso",
        game_mode: GameMode::Hybrid,
    },
    // Control
    PickableMap {
        id: map_slug(MapName::AntarcticPeninsula),
        name: "Antarctic Peninsula",
        game_mode: GameMode::Control,
    },
    PickableMap {
        id: map_slug(MapName::Busan),
        name: "Busan",
        game_mode: GameMode::Control,
    },
    PickableMap {
        id: map_slug(MapName::Ilios),
        name: "Ilios",
        game_mode: GameMode::Control,
    },
    PickableMap {
        id: map_slug(MapName::LijangTower),
        name: "Lijiang Tower",
        game_mode: GameMode::Control,
    },
    PickableMap {
        id: map_slug(MapName::Nepal),
        name: "Nepal",
        game_mode: GameMode::Control,
    },
    PickableMap {
        id: map_slug(MapName::Oasis),
        name: "Oasis",
        game_mode: GameMode::Control,
    },
    PickableMap {
        id: map_slug(MapName::Samoa),
        name: "Samoa",
        game_mode: GameMode::Control,
    },
    // Push
    PickableMap {
        id: map_slug(MapName::Colosseo),
        name: "Colosseo",
        game_mode: GameMode::Push,
    },
    PickableMap {
        id: map_slug(MapName::Esperanca),
        name: "Esperança",
        game_mode: GameMode::Push,
    },
    PickableMap {
        id: map_slug(MapName::NewQueenStreet),
        name: "New Queen Street",
        game_mode: GameMode::Push,
    },
    PickableMap {
        id: map_slug(MapName::Runasapi),
        name: "Runasapi",
        game_mode: GameMode::Push,
    },
    // Flashpoint
    PickableMap {
        id: map_slug(MapName::Aatlis),
        name: "Aatlis",
        game_mode: GameMode::Flashpoint,
    },
    PickableMap {
        id: map_slug(MapName::NewJunkCity),
        name: "New Junk City",
        game_mode: GameMode::Flashpoint,
    },
    PickableMap {
        id: map_slug(MapName::Suravasa),
        name: "Suravasa",
        game_mode: GameMode::Flashpoint,
    },
    // Clash
    PickableMap {
        id: map_slug(MapName::Hanaoka),
        name: "Hanaoka",
        game_mode: GameMode::Clash,
    },
    PickableMap {
        id: map_slug(MapName::ThroneOfAnubis),
        name: "Throne of Anubis",
        game_mode: GameMode::Clash,
    },
];

/// Game-mode section order in the picker (modes with no catalog maps are omitted).
pub const PICKER_MODE_ORDER: &[GameMode] = &[
    GameMode::Escort,
    GameMode::Hybrid,
    GameMode::Control,
    GameMode::Push,
    GameMode::Flashpoint,
    GameMode::Clash,
];

/// Serde `snake_case` slug for [`MapName`]. Keep in lockstep with `rename_all`.
pub const fn map_slug(map: MapName) -> &'static str {
    match map {
        MapName::CircuitRoyal => "circuit_royal",
        MapName::Dorado => "dorado",
        MapName::Havana => "havana",
        MapName::Junkertown => "junkertown",
        MapName::Rialto => "rialto",
        MapName::Route66 => "route_66",
        MapName::ShambaliMonastery => "shambali_monastery",
        MapName::WatchpointGibraltar => "watchpoint_gibraltar",
        MapName::BlizzardWorld => "blizzard_world",
        MapName::Eichenwalde => "eichenwalde",
        MapName::Hollywood => "hollywood",
        MapName::KingsRow => "kings_row",
        MapName::Midtown => "midtown",
        MapName::NeonJunction => "neon_junction",
        MapName::Numbani => "numbani",
        MapName::Paraiso => "paraiso",
        MapName::AntarcticPeninsula => "antarctic_peninsula",
        MapName::Busan => "busan",
        MapName::Ilios => "ilios",
        MapName::LijangTower => "lijang_tower",
        MapName::Nepal => "nepal",
        MapName::Oasis => "oasis",
        MapName::Samoa => "samoa",
        MapName::Colosseo => "colosseo",
        MapName::Esperanca => "esperanca",
        MapName::NewQueenStreet => "new_queen_street",
        MapName::Runasapi => "runasapi",
        MapName::Aatlis => "aatlis",
        MapName::NewJunkCity => "new_junk_city",
        MapName::Suravasa => "suravasa",
        MapName::Hanaoka => "hanaoka",
        MapName::ThroneOfAnubis => "throne_of_anubis",
    }
}

/// Look up a catalog row by stored `map_id` (slug or display name).
pub fn pickable_map_by_id(id: &str) -> Option<&'static PickableMap> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return None;
    }
    PICKABLE_MAPS.iter().find(|m| m.id == trimmed).or_else(|| {
        trimmed
            .parse::<MapName>()
            .ok()
            .and_then(|parsed| PICKABLE_MAPS.iter().find(|m| m.id == map_slug(parsed)))
    })
}

/// Human label for a stored map id; falls back to the raw id.
pub fn display_map_name(id: &str) -> String {
    pickable_map_by_id(id)
        .map(|m| m.name.to_string())
        .or_else(|| {
            id.trim()
                .parse::<MapName>()
                .ok()
                .map(|m| m.display_name().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| id.to_string())
}

/// Create must send a real map id. Never coerce a missing selection to `""`.
pub fn map_id_for_create(current_map: Option<&str>) -> Result<String, &'static str> {
    current_map
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or(CREATE_REQUIRES_MAP)
}

pub const CREATE_REQUIRES_MAP: &str = "Select a map before creating a strategy.";

/// Best-effort game mode from a stored map id. Unknown ids stay Control
/// (same default the create route used to hardcode).
pub fn game_mode_for_map(map_id: &str) -> GameMode {
    pickable_map_by_id(map_id)
        .map(|m| m.game_mode)
        .or_else(|| map_id.trim().parse::<MapName>().ok().map(|m| m.game_mode()))
        .unwrap_or(GameMode::Control)
}

/// Wire format expected by `POST /api/strategy/strategies`.
pub fn game_mode_api_str(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Escort => "escort",
        GameMode::Hybrid => "hybrid",
        GameMode::Control => "control",
        GameMode::Push => "push",
        GameMode::Flashpoint => "flashpoint",
        GameMode::Clash => "clash",
        GameMode::PayloadRace => "payload_race",
        GameMode::Assault => "assault",
    }
}

pub fn maps_for_mode(mode: GameMode) -> impl Iterator<Item = &'static PickableMap> {
    PICKABLE_MAPS.iter().filter(move |m| m.game_mode == mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_id_for_create_refuses_empty() {
        assert_eq!(map_id_for_create(None), Err(CREATE_REQUIRES_MAP));
        assert_eq!(map_id_for_create(Some("")), Err(CREATE_REQUIRES_MAP));
        assert_eq!(map_id_for_create(Some("   ")), Err(CREATE_REQUIRES_MAP));
        assert_eq!(
            map_id_for_create(Some("kings_row")),
            Ok("kings_row".to_string())
        );
        assert_eq!(
            map_id_for_create(Some("  kings_row  ")),
            Ok("kings_row".to_string())
        );
    }

    #[test]
    fn catalog_ids_are_unique_and_parse() {
        let mut seen = std::collections::BTreeSet::new();
        for map in PICKABLE_MAPS {
            assert!(seen.insert(map.id), "duplicate map id {}", map.id);
            let parsed: MapName = map
                .id
                .parse()
                .unwrap_or_else(|_| panic!("catalog id {:?} must parse as MapName", map.id));
            assert_eq!(parsed.display_name(), map.name);
            assert_eq!(parsed.game_mode(), map.game_mode);
            assert_eq!(map_slug(parsed), map.id);
        }
        assert_eq!(PICKABLE_MAPS.len(), 32);
    }

    #[test]
    fn pickable_map_by_id_accepts_slug_and_display() {
        let by_slug = pickable_map_by_id("kings_row").expect("slug");
        assert_eq!(by_slug.name, "King's Row");
        let by_name = pickable_map_by_id("King's Row").expect("display");
        assert_eq!(by_name.id, "kings_row");
        // serde rename of Route66 is "route66"; catalog slug is route_66.
        assert_eq!(
            pickable_map_by_id("route66").map(|m| m.id),
            Some("route_66")
        );
        assert!(pickable_map_by_id("").is_none());
        assert!(pickable_map_by_id("not_a_real_map").is_none());
    }

    #[test]
    fn game_mode_for_known_and_unknown_maps() {
        assert_eq!(game_mode_for_map("kings_row"), GameMode::Hybrid);
        assert_eq!(game_mode_for_map("ilios"), GameMode::Control);
        assert_eq!(game_mode_for_map("unknown_map"), GameMode::Control);
        assert_eq!(game_mode_api_str(GameMode::Hybrid), "hybrid");
    }
}
