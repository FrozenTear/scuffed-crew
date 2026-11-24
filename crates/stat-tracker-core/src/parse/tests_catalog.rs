use super::*;

#[test]
fn chateau_guillard_is_deathmatch_and_untracked() {
    for raw in ["GUILLARD", "Château Guillard", "Chateau Guillard"] {
        let name = canonical_map(raw).unwrap_or_else(|| panic!("{raw}"));
        assert_eq!(name, "Château Guillard");
        assert_eq!(map_mode(&name), Some("Deathmatch"));
        assert!(map_is_untracked(&name));
        assert!(!stats_row_is_tracked(&name, "Deathmatch"));
    }
    assert!(stats_row_is_tracked("Busan", "Control"));
    assert!(!stats_row_is_tracked("Busan", "Deathmatch"));
}

#[test]
fn maps_missing_from_the_ocr_table_canonicalize_with_their_mode() {
    // Assault, Elimination, Capture the Flag, Payload Race, Workshop,
    // and Stadium stay local. `tracked` is false for every one of them.
    let cases = [
        ("Hanamura", "Hanamura", "Assault", false),
        ("HANAMURA", "Hanamura", "Assault", false),
        (
            "Horizon Lunar Colony",
            "Horizon Lunar Colony",
            "Assault",
            false,
        ),
        ("HORIZON", "Horizon Lunar Colony", "Assault", false),
        ("LUNAR COLONY", "Horizon Lunar Colony", "Assault", false),
        ("Paris", "Paris", "Assault", false),
        ("PARIS", "Paris", "Assault", false),
        (
            "Volskaya Industries",
            "Volskaya Industries",
            "Assault",
            false,
        ),
        ("VOLSKAYA", "Volskaya Industries", "Assault", false),
        ("Black Forest", "Black Forest", "Elimination", false),
        ("Castillo", "Castillo", "Elimination", false),
        ("Necropolis", "Necropolis", "Elimination", false),
        ("Ayutthaya", "Ayutthaya", "Capture the Flag", false),
        ("Arena Victoriae", "Arena Victoriae", "Stadium", false),
        ("VICTORIAE", "Arena Victoriae", "Stadium", false),
        ("Gogadoro", "Gogadoro", "Stadium", false),
        (
            "Wuxing University - Water College",
            "Wuxing University - Water College",
            "Stadium",
            false,
        ),
        (
            "WUXING",
            "Wuxing University - Water College",
            "Stadium",
            false,
        ),
        (
            "WATER COLLEGE",
            "Wuxing University - Water College",
            "Stadium",
            false,
        ),
        ("Place Lacroix", "Place Lacroix", "Stadium", false),
        ("LACROIX", "Place Lacroix", "Stadium", false),
        ("Redwood Dam", "Redwood Dam", "Stadium", false),
        ("REDWOOD", "Redwood Dam", "Stadium", false),
        ("Serenza", "Serenza", "Stadium", false),
        ("Powder Keg Mine", "Powder Keg Mine", "Payload Race", false),
        ("POWDER KEG MINES", "Powder Keg Mine", "Payload Race", false),
        ("Thames District", "Thames District", "Payload Race", false),
        ("THAMES", "Thames District", "Payload Race", false),
        ("Workshop Chamber", "Workshop Chamber", "Workshop", false),
        ("Workshop Expanse", "Workshop Expanse", "Workshop", false),
        (
            "Workshop Green Screen",
            "Workshop Green Screen",
            "Workshop",
            false,
        ),
        ("Workshop Island", "Workshop Island", "Workshop", false),
        ("Kanezaka", "Kanezaka", "Deathmatch", false),
        ("Malevento", "Malevento", "Deathmatch", false),
        ("Petra", "Petra", "Deathmatch", false),
        ("Practice Range", "Practice Range", "Practice", false),
        ("PRACTICE RANGE", "Practice Range", "Practice", false),
        ("PRACTICE", "Practice Range", "Practice", false),
        ("Mastery Course", "Mastery Course", "Practice", false),
        ("Tutorial", "Tutorial", "Practice", false),
    ];
    for (raw, display, mode, tracked) in cases {
        let name = canonical_map(raw).unwrap_or_else(|| panic!("{raw}"));
        assert_eq!(name, display, "{raw}");
        assert_eq!(map_mode(&name), Some(mode), "{raw}");
        assert_eq!(map_is_untracked(&name), !tracked, "{raw}");
        assert_eq!(stats_row_is_tracked(&name, mode), tracked, "{raw}");
    }
    // Hanaoka is not Hanamura. Paraiso is not Paris.
    assert_eq!(canonical_map("Hanaoka").as_deref(), Some("Hanaoka"));
    assert_eq!(canonical_map("HANAOKA").as_deref(), Some("Hanaoka"));
    assert_eq!(canonical_map("Paraiso").as_deref(), Some("Paraiso"));
    assert_eq!(canonical_map("PARAISO").as_deref(), Some("Paraiso"));
    assert!(!stats_row_is_tracked("Hanamura", "Assault"));
    assert!(!stats_row_is_tracked("Practice Range", "Practice"));
    assert!(!stats_row_is_tracked("Busan", "Practice"));
    assert!(stats_row_is_tracked("Busan", "Control"));
    assert!(stats_row_is_tracked("King's Row", "Hybrid"));
}

#[test]
fn arcade_and_stadium_modes_stay_local() {
    for (map, mode) in [
        ("Hanamura", "Assault"),
        ("Temple of Anubis", "Assault"),
        ("Black Forest", "Elimination"),
        ("Ecopoint: Antarctica", "Elimination"),
        ("Ayutthaya", "Capture the Flag"),
        ("Powder Keg Mine", "Payload Race"),
        ("Thames District", "Payload Race"),
        ("Workshop Chamber", "Workshop"),
        ("Arena Victoriae", "Stadium"),
        ("Place Lacroix", "Stadium"),
        ("Château Guillard", "Deathmatch"),
        ("Practice Range", "Practice"),
    ] {
        assert_eq!(map_mode(map), Some(mode), "{map}");
        assert!(map_is_untracked(map), "{map}");
        assert!(!stats_row_is_tracked(map, mode), "{map}");
        assert!(
            review_suspect_fields(map, mode, "Ana").is_empty(),
            "{map} is a known map with a mode, so the review hold does not keep it"
        );
        assert!(
            !stats_row_is_tracked("Busan", mode),
            "{mode} on another map stays local"
        );
    }
}
