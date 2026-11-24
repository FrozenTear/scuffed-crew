//! Read-time role fill for the stored per-game role.
//!
//! Each saved game carries the role string the tracker uploaded with it.
//! Older trackers sometimes sent an empty or unrecognized role. Readers fill
//! that gap from the hero list when they build a response, and never write
//! the filled value back:
//!
//! - A recognized stored role (`Tank`, `Damage`, `Support`, any case) is kept,
//!   even when the hero has since moved. An old Sombra game stored as Support
//!   stays Support, and one stored as Damage stays Damage.
//! - An empty or unrecognized stored role takes the current role of the hero
//!   from [`crate::HERO_NAMES`].
//! - A hero that is not on the list (including `Unknown`) keeps an unknown
//!   role, which responses send as `""`.
//!
//! The site History tab applies the same rule on its side (`game_role_label`).

use crate::heroes::role_for_hero_name;
use crate::strategy::HeroRole;

/// `Tank`, `Damage`, or `Support` when `stored` is one of those three,
/// ignoring case and surrounding spaces. Anything else is `None`.
pub fn recognized_stored_role(stored: &str) -> Option<HeroRole> {
    match stored.trim().to_ascii_lowercase().as_str() {
        "tank" => Some(HeroRole::Tank),
        "damage" => Some(HeroRole::Damage),
        "support" => Some(HeroRole::Support),
        _ => None,
    }
}

/// Role for one stored game: the recognized stored role, else the current
/// role of `hero`, else `None` for a hero that is not on the list.
pub fn fill_game_role(stored: &str, hero: &str) -> Option<HeroRole> {
    recognized_stored_role(stored).or_else(|| role_for_hero_name(hero))
}

/// [`fill_game_role`] as the response string: `Tank`, `Damage`, `Support`,
/// or `""` when the role stays unknown.
pub fn filled_role_label(stored: &str, hero: &str) -> String {
    fill_game_role(stored, hero).map_or_else(String::new, |role| role.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognized_stored_role_is_kept() {
        assert_eq!(filled_role_label("Damage", "Ana"), "Damage");
        assert_eq!(filled_role_label("Tank", "Reaper"), "Tank");
        assert_eq!(filled_role_label(" support ", "Reinhardt"), "Support");
        assert_eq!(filled_role_label("DAMAGE", "Unknown"), "Damage");
    }

    #[test]
    fn empty_stored_role_is_filled_from_hero() {
        assert_eq!(filled_role_label("", "Reaper"), "Damage");
        assert_eq!(filled_role_label("   ", "Reinhardt"), "Tank");
        assert_eq!(filled_role_label("", "Ana"), "Support");
        assert_eq!(filled_role_label("", "Lucio"), "Support");
        assert_eq!(filled_role_label("", "wrecking-ball"), "Tank");
    }

    #[test]
    fn garbage_stored_role_is_filled_from_hero() {
        assert_eq!(filled_role_label("dps", "Reaper"), "Damage");
        assert_eq!(filled_role_label("Flex", "Ana"), "Support");
        assert_eq!(filled_role_label("Unknown", "Reinhardt"), "Tank");
        assert_eq!(filled_role_label("t4nk", "Winston"), "Tank");
    }

    #[test]
    fn unknown_hero_keeps_unknown_role() {
        assert_eq!(filled_role_label("", "Unknown"), "");
        assert_eq!(filled_role_label("garbage", "Unknown"), "");
        assert_eq!(filled_role_label("", "NotAHero"), "");
        assert_eq!(filled_role_label("", ""), "");
        assert_eq!(filled_role_label("", "special/skull_01.png"), "");
        assert_eq!(fill_game_role("", "Unknown"), None);
    }

    #[test]
    fn sombra_stored_support_stays_support() {
        // A recognized stored Sombra role is kept whatever the current list
        // says. Old games are never re-stamped.
        assert_eq!(filled_role_label("Support", "Sombra"), "Support");
        assert_eq!(filled_role_label("support", "Sombra"), "Support");
        assert_eq!(filled_role_label("Damage", "Sombra"), "Damage");
        // Only a missing role takes the current list role.
        let current = role_for_hero_name("Sombra").map(|r| r.to_string());
        assert_eq!(Some(filled_role_label("", "Sombra")), current);
    }
}
