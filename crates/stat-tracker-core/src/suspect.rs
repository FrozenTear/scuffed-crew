//! Suspect field names: what a stored row lists in `suspect_fields`, and
//! what of that list goes up to the site.
//!
//! The reader writes row names (`r3.dmg`) or flat names (`dmg`). Uploads
//! carry only the flat names on the server's allowlist
//! ([`SUSPECT_FIELD_NAMES`]), and a manual edit of a field clears its names.
//! The store, the uploader, the report kit and the app all apply these
//! rules, so they live here and not in `reader_apply`, which re-exports
//! them.

use scuffed_types::SUSPECT_FIELD_NAMES;

/// Flat suspect names safe to upload.
///
/// Row names such as `r3.dmg` become `dmg`. Unknown names, duplicates, and
/// anything that is not in the server's allowlist are dropped. Order follows
/// [`SUSPECT_FIELD_NAMES`].
pub fn upload_suspect_fields(stored: &[String]) -> Vec<String> {
    let mut seen = [false; SUSPECT_FIELD_NAMES.len()];
    for name in stored {
        let Some(flat) = flat_suspect_name(name) else {
            continue;
        };
        if let Some(idx) = SUSPECT_FIELD_NAMES
            .iter()
            .position(|allowed| *allowed == flat)
        {
            seen[idx] = true;
        }
    }
    SUSPECT_FIELD_NAMES
        .iter()
        .enumerate()
        .filter(|(idx, _)| seen[*idx])
        .map(|(_, name)| (*name).to_string())
        .collect()
}

/// Drop suspect names cleared by a manual edit.
///
/// `storage_field` is the tracker's edit name (`elims`, `map_name`, ...).
/// Both the flat API name and a leftover `rN.` form of that name are removed.
pub fn clear_edited_suspects(fields: &mut Vec<String>, storage_field: &str) {
    let drop_names = suspect_names_for_edit(storage_field);
    if drop_names.is_empty() {
        return;
    }
    fields.retain(|name| {
        let Some(flat) = flat_suspect_name(name) else {
            return true;
        };
        !drop_names.contains(&flat)
    });
}

/// The flat suspect names a manual edit of `storage_field` (`elims`,
/// `map_name`, ...) settles: `damage` is `dmg`, `map_name` is `map` and
/// `mode`. Empty for a field with no suspect name.
pub fn suspect_names_for_edit(storage_field: &str) -> &'static [&'static str] {
    match storage_field {
        "hero" => &["hero"],
        "map_name" => &["map", "mode"],
        "outcome" => &["result"],
        "elims" => &["e"],
        "assists" => &["a"],
        "deaths" => &["d"],
        "damage" => &["dmg"],
        "healing" => &["h"],
        "mitigation" => &["mit"],
        _ => &[],
    }
}

/// The flat name of a stored suspect name: `dmg` stays `dmg`, a row name
/// such as `r3.dmg` becomes `dmg`. `None` for a name off the allowlist.
pub fn flat_suspect_name(name: &str) -> Option<&str> {
    if SUSPECT_FIELD_NAMES.contains(&name) {
        return Some(name);
    }
    let (row, field) = name.split_once('.')?;
    let digits = row.strip_prefix('r')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    SUSPECT_FIELD_NAMES.contains(&field).then_some(field)
}
