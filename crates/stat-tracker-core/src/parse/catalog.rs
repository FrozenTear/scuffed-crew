//! The map catalog (`MAPS`, mode buckets) and the policy built on it:
//! which maps and modes are tracked or uploaded, and the hero role table.
//! A new map also needs `setup::TRAINING_MAPS` (the Tesseract training text).

/// Modes kept in the local store and never uploaded.
///
/// Deathmatch and Practice were already local. Workshop, Elimination,
/// Capture the Flag, Payload Race, Assault, and Stadium stay local too,
/// until there is a decision on whether those games count.
const UNTRACKED_MODES: &[&str] = &[
    "Deathmatch",
    "Practice",
    "Workshop",
    "Elimination",
    "Capture the Flag",
    "Payload Race",
    "Assault",
    "Stadium",
];

/// True when `mode` is one of [`UNTRACKED_MODES`], ignoring ASCII case.
pub fn mode_is_untracked(mode: &str) -> bool {
    let mode = mode.trim();
    UNTRACKED_MODES
        .iter()
        .any(|name| mode.eq_ignore_ascii_case(name))
}

/// Deathmatch, practice, and the other [`UNTRACKED_MODES`] maps are kept
/// in the local store and never uploaded. Only a trusted map read (top
/// bar, accolade, or an already trusted session) may divert a capture. A
/// fuzzy board read must not.
///
/// Practice Range is not a game. With no table entry the name used to
/// canonicalize to nothing, the row kept an empty map, and that empty map
/// was uploaded. The Practice bucket keeps it out of uploads.
pub fn map_is_untracked(name: &str) -> bool {
    map_mode(name).is_some_and(mode_is_untracked)
}

/// Reason recorded when an untracked map is kept out of uploads.
pub fn untracked_close_reason(name: &str) -> &'static str {
    match map_mode(name) {
        Some("Practice") => "practice: not tracked",
        Some("Workshop") => "workshop: not tracked",
        Some("Elimination") => "elimination: not tracked",
        Some("Capture the Flag") => "capture the flag: not tracked",
        Some("Payload Race") => "payload race: not tracked",
        Some("Assault") => "assault: not tracked",
        Some("Stadium") => "stadium: not tracked",
        _ => "deathmatch: not tracked",
    }
}

/// Mode sent for a row. A known map wins, including a manual map correction.
/// An unknown map keeps the mode stored on the row.
pub fn uploaded_game_mode(map_name: &str, game_mode: &str) -> String {
    let from_map = stored_game_mode(map_name);
    if from_map.is_empty() {
        game_mode.to_string()
    } else {
        from_map
    }
}

/// Display names from [`MAPS`], in table order, duplicates removed.
///
/// The strings are the table's own spellings. `Paraiso` and `Esperanca`
/// stay unaccented. `Watchpoint: Grímsvötn` and `Château Guillard` keep
/// the accents already stored on those rows.
pub fn known_map_names() -> Vec<&'static str> {
    let mut names = Vec::new();
    for &(display, _) in MAPS {
        if !names.contains(&display) {
            names.push(display);
        }
    }
    names
}

/// True when `name` is a [`known_map_names`] display string.
pub fn map_is_known(name: &str) -> bool {
    map_mode(name.trim()).is_some()
}

/// Empty, or the literal `Unknown` in any ASCII case.
///
/// Other hero strings are left as read. This is the OCR miss that must
/// not be uploaded.
pub fn hero_is_unknown_label(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case("unknown")
}

/// Server `suspect_fields` names for one row that is not ready to upload.
///
/// `map` when the map is empty or not in [`known_map_names`]. `mode` when
/// the mode that would be sent is empty. `hero` when
/// [`hero_is_unknown_label`] is true.
///
/// A non-empty list holds the whole match on the machine. The upload
/// leaves that match out. `POST /api/stats/upload` requires `hero` and
/// `map_name` as strings: a null hero and an omitted hero both fail JSON
/// decode, and an empty `map_name` is what got stored on the server.
pub fn review_suspect_fields(map_name: &str, game_mode: &str, hero: &str) -> Vec<&'static str> {
    let map = map_name.trim();
    let mut fields = Vec::new();
    if !map_is_known(map) {
        fields.push("map");
    }
    if uploaded_game_mode(map, game_mode).trim().is_empty() {
        fields.push("mode");
    }
    if hero_is_unknown_label(hero) {
        fields.push("hero");
    }
    fields
}

/// A stats row is uploaded only when neither the map nor the mode is in
/// [`UNTRACKED_MODES`].
pub fn stats_row_is_tracked(map_name: &str, game_mode: &str) -> bool {
    !map_is_untracked(map_name) && !mode_is_untracked(game_mode)
}

/// Mode stored on a row, from the canonical map that was actually kept.
pub fn stored_game_mode(canonical: &str) -> String {
    map_mode(canonical).unwrap_or("").to_string()
}

/// A row with a blank map, mode, or hero must not be uploaded.
///
/// Whitespace-only counts as blank. Hero `Unknown` is a real stored value
/// and is not blank.
pub fn upload_identity_blank(map_name: &str, game_mode: &str, hero: &str) -> bool {
    map_name.trim().is_empty() || game_mode.trim().is_empty() || hero.trim().is_empty()
}

pub fn guess_role_public(hero: &str) -> String {
    guess_role(hero)
}

pub(super) fn guess_role(hero: &str) -> String {
    // The role follows the canonical hero, including a pack key such as
    // `wrecking-ball`. An unknown key is not Damage: that used to file every
    // missed name under the Damage column.
    scuffed_types::role_for_hero_name(hero)
        .map(|role| role.to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

pub(super) const MAPS: &[(&str, &str)] = &[
    // NOT plain "king": that substring matches "wrecKING ball" in scoreboard
    // text and fabricated King's Row reads on every Wrecking Ball game. The
    // fuzzy pass still catches OCR variants like "kings row".
    ("King's Row", "king's row"),
    ("Circuit Royal", "circuit royal"),
    ("Dorado", "dorado"),
    ("Havana", "havana"),
    ("Junkertown", "junkertown"),
    ("Rialto", "rialto"),
    ("Route 66", "route 66"),
    ("Shambali Monastery", "shambali"),
    // Grímsvötn is before Gibraltar. A bare "watchpoint" is not a key:
    // `resolve_watchpoint` decides the family, and an undecided prefix
    // matches neither map.
    ("Watchpoint: Grímsvötn", "grimsvotn"),
    ("Watchpoint: Gibraltar", "gibraltar"),
    ("Blizzard World", "blizzard world"),
    ("Eichenwalde", "eichenwalde"),
    // Halloween label for the same Hybrid map. Junkenstein's Revenge uses
    // the name too; the capture path refuses it unless both teams have stats.
    ("Eichenwalde", "adlersbrunn"),
    ("Hollywood", "hollywood"),
    ("Midtown", "midtown"),
    ("Numbani", "numbani"),
    ("Paraiso", "paraiso"),
    ("Paraiso", "paraíso"),
    ("Neon Junction", "neon junction"),
    // "antarctica" starts with "antarctic", so the short control-map key
    // used to store Ecopoint: Antarctica as Antarctic Peninsula. The
    // peninsula's own word is checked first. The full "antarctica" token,
    // and "ecopoint", are the arena map. A bare "antarctic" stays the
    // control map. Both names stay in the table.
    ("Antarctic Peninsula", "peninsula"),
    ("Ecopoint: Antarctica", "ecopoint"),
    ("Ecopoint: Antarctica", "antarctica"),
    ("Antarctic Peninsula", "antarctic"),
    ("Busan", "busan"),
    ("Ilios", "ilios"),
    ("Lijiang Tower", "lijiang"),
    // A koverwatch read drops the J ("LIANG TOWER") or turns IJ into UL
    // ("LULANG TOWER"). Both are this map. Neither string is a substring
    // of another canonical name.
    ("Lijiang Tower", "liang tower"),
    ("Lijiang Tower", "lulang tower"),
    ("Nepal", "nepal"),
    ("Oasis", "oasis"),
    ("Samoa", "samoa"),
    ("Colosseo", "colosseo"),
    ("Esperanca", "esperanca"),
    ("Esperanca", "esperança"),
    ("New Queen Street", "new queen"),
    ("Runasapi", "runasapi"),
    ("New Junk City", "new junk"),
    ("Suravasa", "suravasa"),
    ("Aatlis", "aatlis"),
    ("Hanaoka", "hanaoka"),
    // "anubis" alone is Throne of Anubis. "temple" has to win first, or
    // Temple of Anubis is stored as the clash map. Both names stay.
    ("Temple of Anubis", "temple"),
    ("Throne of Anubis", "throne"),
    ("Throne of Anubis", "anubis"),
    // Assault left Quick Play, and these five are still in custom games
    // and arcade. Temple of Anubis is one of them; the others had no entry.
    ("Hanamura", "hanamura"),
    ("Horizon Lunar Colony", "horizon"),
    ("Horizon Lunar Colony", "lunar colony"),
    ("Paris", "paris"),
    ("Volskaya Industries", "volskaya"),
    // Arena maps. Ecopoint: Antarctica is one of them; the short
    // "antarctic" key above must not claim it.
    ("Black Forest", "black forest"),
    ("Castillo", "castillo"),
    ("Necropolis", "necropolis"),
    // Stadium maps that are not a sub-area of a map already stored here.
    // A name that contains an existing key (Oasis University, Busan
    // Sanctuary) still stores as that map, so those rows do not split.
    ("Arena Victoriae", "victoriae"),
    ("Gogadoro", "gogadoro"),
    ("Wuxing University - Water College", "wuxing"),
    ("Wuxing University - Water College", "water college"),
    ("Place Lacroix", "lacroix"),
    ("Redwood Dam", "redwood"),
    ("Serenza", "serenza"),
    ("Powder Keg Mine", "powder keg"),
    ("Thames District", "thames"),
    ("Ayutthaya", "ayutthaya"),
    // Workshop layouts. The full phrase is the key so "workshop" alone
    // cannot pick one of them.
    ("Workshop Chamber", "workshop chamber"),
    ("Workshop Expanse", "workshop expanse"),
    ("Workshop Green Screen", "workshop green"),
    ("Workshop Island", "workshop island"),
    // Training. Practice Range is not a game: see `map_is_untracked`.
    ("Practice Range", "practice range"),
    ("Practice Range", "practice"),
    ("Mastery Course", "mastery"),
    ("Tutorial", "tutorial"),
    // Deathmatch only. Kept in the local store. Never uploaded.
    // A trusted read is required before a capture is diverted.
    ("Château Guillard", "guillard"),
    ("Kanezaka", "kanezaka"),
    ("Malevento", "malevento"),
    ("Petra", "petra"),
];

/// Mode bucket for a canonical map display name from [`MAPS`].
///
/// The tracker stores the display string and fills `game_mode` from this
/// table. It does not consult `scuffed_types::MapName`. A name that is not
/// in the table has no bucket.
pub fn map_mode(canonical_name: &str) -> Option<&'static str> {
    match canonical_name {
        "Circuit Royal"
        | "Dorado"
        | "Havana"
        | "Junkertown"
        | "Rialto"
        | "Route 66"
        | "Shambali Monastery"
        | "Watchpoint: Gibraltar"
        | "Watchpoint: Grímsvötn" => Some("Escort"),
        "Blizzard World" | "Eichenwalde" | "Hollywood" | "King's Row" | "Midtown"
        | "Neon Junction" | "Numbani" | "Paraiso" => Some("Hybrid"),
        "Antarctic Peninsula"
        | "Busan"
        | "Ilios"
        | "Lijiang Tower"
        | "Nepal"
        | "Oasis"
        | "Samoa" => Some("Control"),
        "Colosseo" | "Esperanca" | "New Queen Street" | "Runasapi" => Some("Push"),
        "Aatlis" | "New Junk City" | "Suravasa" => Some("Flashpoint"),
        "Hanaoka" | "Throne of Anubis" => Some("Clash"),
        "Hanamura"
        | "Horizon Lunar Colony"
        | "Paris"
        | "Temple of Anubis"
        | "Volskaya Industries" => Some("Assault"),
        "Black Forest" | "Castillo" | "Ecopoint: Antarctica" | "Necropolis" => Some("Elimination"),
        "Ayutthaya" => Some("Capture the Flag"),
        "Powder Keg Mine" | "Thames District" => Some("Payload Race"),
        "Arena Victoriae"
        | "Gogadoro"
        | "Place Lacroix"
        | "Redwood Dam"
        | "Serenza"
        | "Wuxing University - Water College" => Some("Stadium"),
        "Workshop Chamber" | "Workshop Expanse" | "Workshop Green Screen" | "Workshop Island" => {
            Some("Workshop")
        }
        "Mastery Course" | "Practice Range" | "Tutorial" => Some("Practice"),
        "Château Guillard" | "Kanezaka" | "Malevento" | "Petra" => Some("Deathmatch"),
        _ => None,
    }
}
