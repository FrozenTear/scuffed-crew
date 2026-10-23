use super::*;

#[test]
fn short_hero_names_need_word_boundaries() {
    // "ana" ⊂ "havana": a bare map label must not read as the hero Ana.
    assert_eq!(match_hero_in_text("HAVANA"), None);
    assert_eq!(match_hero_in_text("Hanaoka"), None);
    // Standalone the name still matches, with or without punctuation.
    assert_eq!(match_hero_in_text("Ana").as_deref(), Some("Ana"));
    assert_eq!(match_hero_in_text("ana: 14 elims").as_deref(), Some("Ana"));
}

#[test]
fn hero_ties_break_to_most_mentioned() {
    // A support duo on the scoreboard: the player's hero recurs across
    // stat lines — most-mentioned must win (this sorted ascending for a
    // while, so the LEAST-mentioned hero won every multi-match).
    let text = "HAVANA\nana 14 8 2 3400\nmercy 30 2 11 8000\nmercy 1 2 3 4\nmercy 5 6 7 8";
    assert_eq!(match_hero_in_text(text).as_deref(), Some("Mercy"));
}

#[test]
fn map_vote_names_canonicalize_to_display_names() {
    assert_eq!(
        canonical_map("SHAMBALI").as_deref(),
        Some("Shambali Monastery")
    );
    assert_eq!(
        canonical_map("WATCHPOINT").as_deref(),
        None,
        "a bare watchpoint is not Gibraltar"
    );
    assert_eq!(
        canonical_map("GIBRALTAR").as_deref(),
        Some("Watchpoint: Gibraltar")
    );
    assert_eq!(canonical_map("ROUTE 66").as_deref(), Some("Route 66"));
    assert_eq!(
        canonical_map("NEON JUNCTION").as_deref(),
        Some("Neon Junction")
    );
    assert_eq!(canonical_map("garbage read"), None);
}

const GRIMSVOTN: &str = "Watchpoint: Grímsvötn";

#[test]
fn grimsvotn_canonical_name_is_byte_exact() {
    // Same precomposed display string and Escort bucket as
    // `scuffed_types::MapName::WatchpointGrimsvotn`.
    let shared = scuffed_types::MapName::WatchpointGrimsvotn.display_name();
    assert_eq!(shared, GRIMSVOTN);
    assert!(
        !shared
            .chars()
            .any(|c| ('\u{0300}'..='\u{036F}').contains(&c)),
        "display name must be precomposed"
    );
    let name = canonical_map(GRIMSVOTN).expect("canonical name");
    assert_eq!(name.as_bytes(), shared.as_bytes());
    assert_eq!(
        map_mode(&name),
        Some(scuffed_types::MapName::game_mode_label(shared))
    );
    assert_eq!(map_mode(&name), Some("Escort"));
    assert_eq!(map_mode("Watchpoint: Gibraltar"), Some("Escort"));
    for &(display, _) in MAPS {
        assert!(map_mode(display).is_some(), "{display} has no mode bucket");
    }
}

#[test]
fn bare_grimsvotn_resolves_with_or_without_accents() {
    for raw in [
        "grimsvotn",
        "GRIMSVOTN",
        "Grimsvotn",
        "GRIMSVÖTN",
        "GRÍMSVÖTN",
    ] {
        let name = canonical_map(raw).unwrap_or_else(|| panic!("{raw} did not match"));
        assert_eq!(name.as_bytes(), GRIMSVOTN.as_bytes(), "{raw}");
    }
}

#[test]
fn prefixed_grimsvotn_wins_over_watchpoint() {
    // The prefix is also the Gibraltar key. Grímsvötn's own key, or an
    // OCR variant of it, has to win wherever it appears in the text.
    for raw in [
        "WATCHPOINT: GRIMSVOTN",
        "WATCHPOINT GRIMSV0TN",
        "WATCHPOINT: GRÍMSVÖTN",
        "watchpoint grimsvotn",
        "Watchpoint: Grímsvötn",
        "GRLMSVOTN",
        "GR1MSVOTN",
    ] {
        let name = match_map_in_text(raw).unwrap_or_else(|| panic!("{raw} did not match"));
        assert_eq!(name.as_bytes(), GRIMSVOTN.as_bytes(), "{raw}");
    }
}

#[test]
fn watchpoint_maps_do_not_collide() {
    let gibraltar = [
        "Watchpoint: Gibraltar",
        "WATCHPOINT: GIBRALTAR",
        "GIBRALTAR",
        "gibraltar",
    ];
    let grimsvotn = [
        "grimsvotn",
        "GRIMSVÖTN",
        "Watchpoint: Grímsvötn",
        "WATCHPOINT: GRIMSVOTN",
        "WATCHPOINT GRIMSV0TN",
        "GRLMSVOTN",
        "GR1MSVOTN",
        "GRÍMSVÖTN",
    ];
    for raw in gibraltar {
        assert_eq!(
            match_map_in_text(raw).as_deref(),
            Some("Watchpoint: Gibraltar"),
            "{raw} must not become Grímsvötn"
        );
    }
    for raw in grimsvotn {
        assert_eq!(
            match_map_in_text(raw).as_deref(),
            Some(GRIMSVOTN),
            "{raw} must not become Gibraltar"
        );
    }
    for raw in ["WATCHPOINT", "watchpoint", "WATCHPOINT:"] {
        assert_eq!(
            match_map_in_text(raw),
            None,
            "{raw} must not become Gibraltar"
        );
        assert_eq!(exact_map_in_text(raw), None, "{raw}");
    }
}

#[test]
fn garbled_grimsvotn_after_watchpoint_is_not_gibraltar() {
    // eng LSTM has no í/ö. A misread second word used to hit the bare
    // watchpoint key and become a trusted Gibraltar.
    for raw in [
        "WATCHPOINT: GRIMSV6TN",
        "WATCHPOINT: GRIMSVOT",
        "WATCHPOINT: GRIMSVQTN",
        "WATCHPOINT: GRMSVOTN",
        "WATCHPOINT: GR'IMSVOTN",
        "WATCHPOINT: GRIMS VOTN",
        "WATCHPOINT: GRIMSV6T",
    ] {
        assert_eq!(
            exact_map_in_text(raw).as_deref(),
            Some(GRIMSVOTN),
            "accolade {raw}"
        );
        assert_eq!(
            match_map_in_text(raw).as_deref(),
            Some(GRIMSVOTN),
            "top bar {raw}"
        );
    }
}

#[test]
fn dotted_turkish_i_in_grimsvotn_canonicalizes() {
    // U+0130 lowercases to i + a combining dot. U+0131 is dotless i.
    assert_eq!(canonical_map("GRİMSVÖTN").as_deref(), Some(GRIMSVOTN));
    assert_eq!(
        canonical_map("GR\u{0131}MSVÖTN").as_deref(),
        Some(GRIMSVOTN)
    );
    assert_eq!(
        canonical_map("Gri\u{301}msvo\u{308}tn").as_deref(),
        Some(GRIMSVOTN)
    );
    // Dotless ı survives lowercasing, so the accolade path pins that arm.
    assert_eq!(
        exact_map_in_text("GR\u{0131}MSVÖTN").as_deref(),
        Some(GRIMSVOTN)
    );
    // İ is already i plus a combining dot after lowercasing. Call the
    // fold directly so dropping the İ arm fails this test.
    assert_eq!(normalize_ocr_glyphs("İ"), "i");
    assert_eq!(normalize_ocr_glyphs("ı"), "i");
}

#[test]
fn one_letter_gibraltar_misread_stays_gibraltar() {
    for raw in [
        "WATCHPOINT: GIBRALTAP",
        "WATCHPOINT: GIBRALTA",
        "WATCHPOINT: CIBRALTAR",
    ] {
        assert_eq!(
            exact_map_in_text(raw).as_deref(),
            Some(GIBRALTAR_NAME),
            "accolade {raw}"
        );
        assert_eq!(
            match_map_in_text(raw).as_deref(),
            Some(GIBRALTAR_NAME),
            "top bar {raw}"
        );
    }
}

#[test]
fn liang_tower_is_lijiang() {
    assert_eq!(
        match_map_in_text("LIANG TOWER").as_deref(),
        Some("Lijiang Tower")
    );
}

#[test]
fn lulang_tower_is_lijiang() {
    assert_eq!(
        match_map_in_text("LULANG TOWER").as_deref(),
        Some("Lijiang Tower")
    );
    assert_eq!(
        match_map_in_text("Lulang tower").as_deref(),
        Some("Lijiang Tower")
    );
}

#[test]
fn lang_tower_line_is_lijiang() {
    assert_eq!(
        match_map_in_text("Q control | Lang Tower Jjjf").as_deref(),
        Some("Lijiang Tower")
    );
}

#[test]
fn llang_tower_line_is_lijiang() {
    assert_eq!(
        match_map_in_text("Q control | LlanG Tower Jjj").as_deref(),
        Some("Lijiang Tower")
    );
}

#[test]
fn luang_tower_line_is_lijiang() {
    assert_eq!(
        match_map_in_text("Q control | Luang tower [jj").as_deref(),
        Some("Lijiang Tower")
    );
}

#[test]
fn lulang_tower_line_is_lijiang() {
    assert_eq!(
        match_map_in_text("Q control | Lulang tower Jj").as_deref(),
        Some("Lijiang Tower")
    );
}

#[test]
fn lijiang_alias_spellings_are_not_a_near_tie() {
    // "luang tower" clears the long-name floor on both Lijiang keys, and
    // the two scores sit inside the margin. They are one canonical map,
    // so the read is not held as a tie between them.
    let text = normalize_ocr_glyphs("luang tower");
    let liang = normalized_levenshtein(&text, &normalize_ocr_glyphs("liang tower"));
    let lulang = normalized_levenshtein(&text, &normalize_ocr_glyphs("lulang tower"));
    assert!(liang >= map_fuzzy_threshold(10), "{liang}");
    assert!(lulang >= map_fuzzy_threshold(11), "{lulang}");
    assert!(
        (liang - lulang).abs() < MAP_FUZZY_MARGIN,
        "liang {liang} lulang {lulang}"
    );
    assert_eq!(
        match_map_in_text("Luang tower").as_deref(),
        Some("Lijiang Tower")
    );
    assert_eq!(
        match_map_in_text("Liang tower").as_deref(),
        Some("Lijiang Tower")
    );
}

#[test]
fn tab_banner_matches_the_map_side_and_ignores_the_timer() {
    // The crop used to include the start of the match timer, so the read
    // was `CONTROL | ILIOS TIM` and the folded bar glued onto the name.
    for raw in [
        "CONTROL | ILIOS TIM",
        "control | ilios tim",
        "Control | Ilios Tim",
        "CONTROL|ILIOS TIM",
        "CONTROL |ILIOS TIM",
        "Q CONTROL | ILIOS TIM",
        "@@ CONTROL | ILIOS TIM",
    ] {
        assert_eq!(match_map_in_text(raw).as_deref(), Some("Ilios"), "{raw}");
    }
    // No bar: junk before the mode word is not a map, even when that
    // junk is itself a map name earlier in the table than Ilios.
    assert_eq!(
        match_map_in_text("BUSAN CONTROL ILIOS TIM").as_deref(),
        Some("Ilios")
    );
    assert_eq!(match_map_in_text("CONTROL | TIM"), None);
    assert_eq!(match_map_in_text("TIM"), None);
}

#[test]
fn ilios_clock_reads_match_ilios() {
    for raw in [
        "ILIOS 05:12",
        "CONTROL | ILIOS 05:12",
        "CONTROL | ILIOS05:12",
    ] {
        assert_eq!(match_map_in_text(raw).as_deref(), Some("Ilios"), "{raw}");
    }
}

#[test]
fn leading_junk_bar_and_mode_word_leave_the_map_text() {
    assert_eq!(tab_banner_map_text("| CONTROL | ILIOS"), "ILIOS");
    assert_eq!(
        match_map_in_text("| CONTROL | ILIOS").as_deref(),
        Some("Ilios")
    );
}

#[test]
fn tab_banner_keeps_anubis_and_antarctica_apart() {
    for (raw, display) in [
        ("ASSAULT | TEMPLE OF ANUBIS", "Temple of Anubis"),
        ("xqz assault | temple of anubis", "Temple of Anubis"),
        ("CLASH | THRONE OF ANUBIS", "Throne of Anubis"),
        ("clash | throne of anubis", "Throne of Anubis"),
        ("CLASH | ANUBIS", "Throne of Anubis"),
        ("ELIMINATION | ECOPOINT: ANTARCTICA", "Ecopoint: Antarctica"),
        ("elimination | ecopoint: antarctica", "Ecopoint: Antarctica"),
        ("ELIMINATION | ANTARCTICA", "Ecopoint: Antarctica"),
        ("CONTROL | ANTARCTIC PENINSULA", "Antarctic Peninsula"),
        ("control | antarctic peninsula", "Antarctic Peninsula"),
        ("CONTROL | ANTARCTIC", "Antarctic Peninsula"),
        (
            "OASIS ELIMINATION ECOPOINT: ANTARCTICA",
            "Ecopoint: Antarctica",
        ),
        ("FLASHPOINT NEW JUNK CITY", "New Junk City"),
    ] {
        assert_eq!(match_map_in_text(raw).as_deref(), Some(display), "{raw}");
    }
}

#[test]
fn lijiang_ocr_aliases_do_not_steal_other_maps() {
    let liang = normalize_ocr_glyphs("liang tower");
    let lulang = normalize_ocr_glyphs("lulang tower");
    for &(display, pattern) in MAPS {
        if pattern == "liang tower" || pattern == "lulang tower" {
            continue;
        }
        assert_eq!(
            match_map_in_text(display).as_deref(),
            Some(display),
            "{display} no longer resolves to itself"
        );
        let folded = normalize_ocr_glyphs(&display.to_lowercase());
        assert!(
            !folded.contains(&liang),
            "{display} contains the LIANG TOWER alias"
        );
        assert!(
            !folded.contains(&lulang),
            "{display} contains the LULANG TOWER alias"
        );
    }
}

#[test]
fn adlersbrunn_and_halloween_prefixes_canonicalize() {
    for raw in ["ADLERSBRUNN", "Adlersbrunn", "adiersbrunn", "ADLERSBRUNN "] {
        assert_eq!(
            canonical_map(raw.trim()).as_deref(),
            Some("Eichenwalde"),
            "{raw}"
        );
    }
    assert_eq!(
        match_map_in_text("HALLOWEEN HOLLYWOOD").as_deref(),
        Some("Hollywood")
    );
    assert_eq!(
        match_map_in_text("HALLOWEEN LIJIANG TOWER").as_deref(),
        Some("Lijiang Tower")
    );
    assert_eq!(canonical_map("EICHENWALDE").as_deref(), Some("Eichenwalde"));
    assert_eq!(canonical_map("KING'S ROW").as_deref(), Some("King's Row"));
    assert!(!is_adlersbrunn_alias("Eichenwalde"));
    assert!(is_adlersbrunn_alias("Adlersbrunn"));
    assert!(is_adlersbrunn_alias("adiersbrunn"));
    assert!(is_adlersbrunn_alias("ADLERSBRUNM"));
    assert!(is_adlersbrunn_alias("ADLER5BRUNN"));
    assert!(!is_adlersbrunn_alias("EICHENWALD"));
    assert!(reject_pve_adlersbrunn(true, "Eichenwalde", false));
    assert!(!reject_pve_adlersbrunn(true, "Eichenwalde", true));
    assert!(!reject_pve_adlersbrunn(false, "Eichenwalde", false));
    assert!(!reject_pve_adlersbrunn(true, "Busan", false));
}

#[test]
fn guillard_ocr_near_misses_still_canonicalize() {
    // koverwatch on a rendered "Chateau Guillard": a line crop is exact.
    // A tall canvas reads Guiltard, and a 43px-tall crop reads Guitlard.
    assert_eq!(
        match_map_in_text("Chateau Guillard").as_deref(),
        Some("Château Guillard")
    );
    assert_eq!(
        match_map_in_text("Chateau Guiltard").as_deref(),
        Some("Château Guillard")
    );
    assert_eq!(
        match_map_in_text("Chateau Guitlard").as_deref(),
        Some("Château Guillard")
    );
    assert_eq!(match_map_in_text("Chateau Guiltarod").as_deref(), None);
    assert_eq!(
        match_map_in_text("AdLErSBRuNN").as_deref(),
        Some("Eichenwalde")
    );
    assert_eq!(
        match_map_in_text("Grimsvotn").as_deref(),
        Some("Watchpoint: Grímsvötn")
    );
}

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
fn anubis_and_antarctica_names_stay_on_their_own_maps() {
    // Stored spellings of the maps that were already in the table do
    // not move. Esperanca and Paraiso stay unaccented.
    assert_eq!(canonical_map("Esperanca").as_deref(), Some("Esperanca"));
    assert_eq!(canonical_map("Esperança").as_deref(), Some("Esperanca"));
    assert_eq!(canonical_map("Paraiso").as_deref(), Some("Paraiso"));
    assert_eq!(canonical_map("Paraíso").as_deref(), Some("Paraiso"));
    assert_eq!(
        canonical_map("Throne of Anubis").as_deref(),
        Some("Throne of Anubis")
    );
    assert_eq!(
        canonical_map("Antarctic Peninsula").as_deref(),
        Some("Antarctic Peninsula")
    );

    for (raw, display, mode) in [
        ("Temple of Anubis", "Temple of Anubis", "Assault"),
        ("TEMPLE OF ANUBIS", "Temple of Anubis", "Assault"),
        ("temple of anubis", "Temple of Anubis", "Assault"),
        ("Throne of Anubis", "Throne of Anubis", "Clash"),
        ("THRONE OF ANUBIS", "Throne of Anubis", "Clash"),
        ("ANUBIS", "Throne of Anubis", "Clash"),
        (
            "Ecopoint: Antarctica",
            "Ecopoint: Antarctica",
            "Elimination",
        ),
        (
            "ECOPOINT: ANTARCTICA",
            "Ecopoint: Antarctica",
            "Elimination",
        ),
        ("ANTARCTICA", "Ecopoint: Antarctica", "Elimination"),
        ("ECOPOINT", "Ecopoint: Antarctica", "Elimination"),
        ("Antarctic Peninsula", "Antarctic Peninsula", "Control"),
        ("ANTARCTIC PENINSULA", "Antarctic Peninsula", "Control"),
        ("ANTARCTIC", "Antarctic Peninsula", "Control"),
    ] {
        let name = canonical_map(raw).unwrap_or_else(|| panic!("{raw}"));
        assert_eq!(name, display, "{raw}");
        assert_eq!(map_mode(&name), Some(mode), "{raw}");
        assert_eq!(
            exact_map_in_text(raw).as_deref(),
            Some(display),
            "accolade {raw}"
        );
    }
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

#[test]
fn short_map_keys_match_whole_words_and_do_not_block_a_real_upload() {
    // "paris" is a letter run inside "comparison".
    assert_eq!(canonical_map("comparison"), None);
    assert_eq!(canonical_map("COMPARISON").as_deref(), None);
    let ilios = canonical_map("Ilios comparison").expect("ilios");
    assert_eq!(ilios, "Ilios");
    assert!(stats_row_is_tracked(&ilios, &stored_game_mode(&ilios)));

    // "petra" inside a longer word must not become the deathmatch map.
    assert_eq!(canonical_map("competra"), None);
    let kings = canonical_map("King's Row competra").expect("kings");
    assert_eq!(kings, "King's Row");
    assert!(stats_row_is_tracked(&kings, "Hybrid"));

    // "practice" inside a longer word must not become Practice Range.
    assert_eq!(canonical_map("inpractice"), None);
    let busan = canonical_map("Busan inpractice").expect("busan");
    assert_eq!(busan, "Busan");
    assert!(stats_row_is_tracked(&busan, "Control"));

    // The keys still match when they are the whole word.
    assert_eq!(canonical_map("PARIS").as_deref(), Some("Paris"));
    assert_eq!(canonical_map("PETRA").as_deref(), Some("Petra"));
    assert_eq!(canonical_map("PRACTICE").as_deref(), Some("Practice Range"));
}

#[test]
fn glyph_mangled_map_names_resolve() {
    // ILIOS is three capital I's — the most OCR-mangled name in the pool.
    // Field 07-17: an Ilios game registered no map on any reader. Each of
    // these fed the old exact `contains` path an empty result.
    // (Mutation check: with FUZZY_MAP_THRESHOLD reverted to 0.85 AND the
    // glyph fold removed, every assertion in this test fails.)
    assert_eq!(match_map_in_text("1LIOS").as_deref(), Some("Ilios"));
    assert_eq!(match_map_in_text("IL10S").as_deref(), Some("Ilios"));
    assert_eq!(match_map_in_text("|LIOS").as_deref(), Some("Ilios"));
    // Oasis with a zero-for-O; Route 66 with a zero-for-O.
    assert_eq!(match_map_in_text("0ASIS").as_deref(), Some("Oasis"));
    assert_eq!(match_map_in_text("R0UTE 66").as_deref(), Some("Route 66"));
    // Busan with a V-for-A (not a glyph fold — needs the looser short-name
    // fuzzy threshold: BUSVN↔BUSAN scores 0.80).
    assert_eq!(match_map_in_text("BUSVN").as_deref(), Some("Busan"));
}

#[test]
fn fuzzy_map_holds_junk_and_close_scores_and_keeps_canonical_names() {
    // Below the floor: not the closest name.
    for raw in ["PARXQ", "BUSXY", "TIM", "zzzzz", "not-a-map", "qqqqq"] {
        assert_eq!(match_map_in_text(raw), None, "{raw}");
        assert_eq!(exact_map_in_text(raw), None, "{raw}");
    }
    // One edit still clears the short-name floor and has no close second.
    assert_eq!(match_map_in_text("PARIZ").as_deref(), Some("Paris"));
    assert_eq!(match_map_in_text("BUSVN").as_deref(), Some("Busan"));

    // Oasis and Paris both score 0.80 on these. Antarctic and Antarctica
    // tie, or lead by less than the margin. Either way the map is unknown.
    for raw in ["oaris", "pasis", "antarcticx", "antarctia"] {
        assert_eq!(match_map_in_text(raw), None, "{raw}");
    }

    // Timer digits stuck on the end of the banner name.
    for raw in [
        "ILIOS 0",
        "ILIOS 7/M",
        "CONTROL | ILIOS 0",
        "CONTROL | ILIOS 7/M",
        "CONTROL|ILIOS 0",
    ] {
        assert_eq!(match_map_in_text(raw).as_deref(), Some("Ilios"), "{raw}");
    }
    // I/L fold, and a single u/i swap that still clears the floor alone.
    assert_eq!(match_map_in_text("ILios").as_deref(), Some("Ilios"));
    assert_eq!(match_map_in_text("iuios").as_deref(), Some("Ilios"));
    // A long name clipped before the last letters. Two missing glyphs
    // clear the floor; a stub does not. The canonical spelling keeps
    // the accents.
    for raw in ["GRiMsVO", "GRiMsVO™", "WATCHPOINT: GRiMsVO™"] {
        assert_eq!(
            match_map_in_text(raw).as_deref(),
            Some("Watchpoint: Grímsvötn"),
            "{raw}"
        );
    }
    assert_eq!(match_map_in_text("GRIMS"), None);
    assert_eq!(match_map_in_text("GRIMSV"), None);

    // #201: a short key does not match inside a longer word.
    for raw in ["comparison", "parisian", "inparis", "COMPARISON"] {
        assert_eq!(canonical_map(raw), None, "{raw}");
    }
    assert_eq!(canonical_map("PARIS").as_deref(), Some("Paris"));

    let grim = canonical_map("GRIMSVOTN").expect("grimsvotn");
    assert_eq!(grim, "Watchpoint: Grímsvötn");
    assert!(grim.contains('í') && grim.contains('ö'), "{grim}");
    let chateau = canonical_map("Chateau Guillard").expect("chateau");
    assert_eq!(chateau, "Château Guillard");
    assert!(chateau.contains('â'), "{chateau}");
}

#[test]
fn player_names_and_hero_names_do_not_false_positive_as_maps() {
    // The map matcher runs over full-board OCR text too, so hero names,
    // gamer tags and stat fragments must not resolve to a map even with the
    // looser short-name threshold.
    assert_eq!(match_map_in_text("REAPER"), None);
    assert_eq!(match_map_in_text("SOMBRA"), None);
    assert_eq!(match_map_in_text("DOCTRINE"), None);
    assert_eq!(match_map_in_text("WIDOWMAKER"), None);
    assert_eq!(match_map_in_text("xXGamerTagXx"), None);
    assert_eq!(match_map_in_text("FR0ZEN"), None);
    assert_eq!(match_map_in_text("31% WEAPON ACCURACY"), None);
}

#[test]
fn wrecking_ball_guard_survives_glyph_fold() {
    // The King's-Row false positive that justified the strict threshold:
    // "wrecKING ball" must still NOT read as King's Row after the glyph
    // fold (which turns "ball"→"baii" but leaves the guard intact).
    assert_eq!(
        match_map_in_text("WRECKING BALL\n31% WEAPON ACCURACY"),
        None
    );
    // And the genuine map still resolves, apostrophe dropped or not.
    assert_eq!(
        match_map_in_text("KING'S ROW").as_deref(),
        Some("King's Row")
    );
    assert_eq!(
        match_map_in_text("KINGS ROW").as_deref(),
        Some("King's Row")
    );
}
