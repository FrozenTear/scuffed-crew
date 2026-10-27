//! The hint under Tracker health when hero names need site sync: no hero
//! pack on this computer and no website to fetch one from.

/// Shown under Tracker health when no hero pack is installed and site sync
/// is off: the hero pack comes from the site, so heroes read as unknown.
pub const HERO_PACK_SYNC_HINT: &str = "Hero names need site sync. Your stats still track normally.";

/// [`HERO_PACK_SYNC_HINT`] when site sync is off and the last pack check
/// found no hero pack. `None` before the first check (and in demo mode,
/// which never checks), and whenever sync is on: with sync on the tracker
/// downloads the pack itself.
pub fn hero_pack_hint(
    sync_on: bool,
    packs: Option<&stat_tracker::ocr_switch::PackReadiness>,
) -> Option<&'static str> {
    match packs {
        Some(p) if !sync_on && !p.heroes => Some(HERO_PACK_SYNC_HINT),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hero_pack_hint_shows_only_without_sync_and_without_a_hero_pack() {
        use stat_tracker::ocr_switch::PackReadiness;
        let none = PackReadiness::default();
        let heroes = PackReadiness {
            heroes: true,
            ..PackReadiness::default()
        };
        assert_eq!(
            hero_pack_hint(false, Some(&none)),
            Some(HERO_PACK_SYNC_HINT)
        );
        assert_eq!(hero_pack_hint(true, Some(&none)), None, "sync on");
        assert_eq!(hero_pack_hint(false, Some(&heroes)), None, "pack there");
        assert_eq!(hero_pack_hint(true, Some(&heroes)), None);
        assert_eq!(hero_pack_hint(false, None), None, "not checked yet");
        assert_eq!(
            HERO_PACK_SYNC_HINT,
            "Hero names need site sync. Your stats still track normally."
        );
        assert!(!HERO_PACK_SYNC_HINT.contains('\u{2014}'));
    }
}
