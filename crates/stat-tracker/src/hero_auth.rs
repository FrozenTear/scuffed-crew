//! CG-4 Lane C: hero-source authority for scoreboard captures.
//!
//! Career-panel title is always authoritative when present. Portrait template
//! matching may **confirm** the current accepted hero, but may **switch** only
//! when career-panel has never succeeded this game **and** ≥2 consecutive
//! identical portrait matches land at confidence ≥ [`PORTRAIT_SWITCH_MIN_CONF`].
//! That blocks the 07-22 field failure (portrait Illari 0.836 mid–Wrecking Ball
//! game while career-panel was unreadable) without blocking real career-panel
//! swaps (e.g. Ramattra→Reinhardt).

use serde::{Deserialize, Serialize};

/// Minimum portrait confidence to count toward a portrait-only switch streak.
pub const PORTRAIT_SWITCH_MIN_CONF: f64 = 0.85;

/// Consecutive identical high-confidence portrait matches required before a
/// portrait-only switch is allowed (and only if career-panel never succeeded).
pub const PORTRAIT_SWITCH_STREAK: u32 = 2;

/// Cross-capture hero authority for one active game.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HeroAuthState {
    /// Career-panel title has been read successfully at least once this game.
    #[serde(default)]
    pub career_ever_ok: bool,
    /// Last hero accepted for snapshot write (canonical display name).
    #[serde(default)]
    pub accepted_hero: Option<String>,
    /// Consecutive portrait-only reads of the same hero at ≥ min conf.
    /// Reset on career success, low-conf portrait, hero change, or missing portrait.
    #[serde(default)]
    pub portrait_pending: Option<(String, u32)>,
}

/// Which input produced the resolved hero (for logs / tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeroSource {
    CareerPanel,
    Portrait,
    /// Held previous accepted hero (portrait switch refused).
    Held,
    OcrText,
}

/// Resolve which hero string to store on this capture.
///
/// * `career` — career-panel title already matched to a known hero (display name).
/// * `portrait` — `(file_stem_or_name, confidence)`; will be passed through
///   `canonical_hero` by the caller before compare, or we canonicalize here via
///   the `canonicalize` callback to avoid a hard parse dependency cycle in tests.
/// * `ocr_text` — scoreboard-text / raw guess already on the parsed row.
/// * `canonicalize` — maps portrait keys (`wrecking_ball`) to display names.
pub fn resolve_hero(
    career: Option<&str>,
    portrait: Option<(&str, f64)>,
    ocr_text: &str,
    state: &HeroAuthState,
    canonicalize: impl Fn(&str) -> String,
) -> (String, HeroSource, HeroAuthState) {
    let mut next = state.clone();

    if let Some(hero) = career {
        let hero = hero.to_string();
        next.career_ever_ok = true;
        next.accepted_hero = Some(hero.clone());
        next.portrait_pending = None;
        return (hero, HeroSource::CareerPanel, next);
    }

    if let Some((raw, conf)) = portrait {
        let hero = canonicalize(raw);

        // Confirm: same as currently accepted — always allow portrait to re-assert.
        if next.accepted_hero.as_deref() == Some(hero.as_str()) {
            next.portrait_pending = None;
            return (hero, HeroSource::Portrait, next);
        }

        // Different from accepted (or no accepted yet) — maybe start/advance a switch streak.
        if conf >= PORTRAIT_SWITCH_MIN_CONF {
            let streak = match &next.portrait_pending {
                Some((h, n)) if h == &hero => n.saturating_add(1),
                _ => 1,
            };
            next.portrait_pending = Some((hero.clone(), streak));

            if !next.career_ever_ok && streak >= PORTRAIT_SWITCH_STREAK {
                next.accepted_hero = Some(hero.clone());
                return (hero, HeroSource::Portrait, next);
            }
        } else {
            next.portrait_pending = None;
        }

        // Switch refused: hold previous accepted hero when we have one.
        if let Some(prev) = next.accepted_hero.clone() {
            return (prev, HeroSource::Held, next);
        }

        // No accepted hero yet and streak not met — do not write a one-off
        // portrait (would open a fake segment). Prefer OCR text if known.
        let ocr = ocr_text.to_string();
        if !ocr.is_empty() && ocr != "Unknown" {
            return (ocr, HeroSource::OcrText, next);
        }
        return ("Unknown".to_string(), HeroSource::Held, next);
    }

    // No career, no portrait this frame.
    next.portrait_pending = None;
    if let Some(prev) = next.accepted_hero.clone() {
        // Prefer held authority over thrashing OCR after career/portrait established.
        return (prev, HeroSource::Held, next);
    }
    (ocr_text.to_string(), HeroSource::OcrText, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> String {
        s.to_string()
    }

    fn canon_map(s: &str) -> String {
        match s {
            "wrecking_ball" | "Wrecking Ball" => "Wrecking Ball".into(),
            "illari" | "Illari" => "Illari".into(),
            "reinhardt" | "Reinhardt" => "Reinhardt".into(),
            "ramattra" | "Ramattra" => "Ramattra".into(),
            other => other.to_string(),
        }
    }

    #[test]
    fn career_panel_always_switches() {
        let st = HeroAuthState {
            career_ever_ok: false,
            accepted_hero: Some("Ramattra".into()),
            portrait_pending: None,
        };
        let (h, src, next) = resolve_hero(Some("Reinhardt"), None, "Unknown", &st, id);
        assert_eq!(h, "Reinhardt");
        assert_eq!(src, HeroSource::CareerPanel);
        assert!(next.career_ever_ok);
        assert_eq!(next.accepted_hero.as_deref(), Some("Reinhardt"));
    }

    #[test]
    fn portrait_confirms_current_hero() {
        let st = HeroAuthState {
            career_ever_ok: true,
            accepted_hero: Some("Wrecking Ball".into()),
            portrait_pending: None,
        };
        let (h, src, next) = resolve_hero(
            None,
            Some(("wrecking_ball", 0.75)),
            "Illari",
            &st,
            canon_map,
        );
        assert_eq!(h, "Wrecking Ball");
        assert_eq!(src, HeroSource::Portrait);
        assert_eq!(next.accepted_hero.as_deref(), Some("Wrecking Ball"));
    }

    #[test]
    fn portrait_cannot_switch_after_career_ok() {
        // 07-22 field: mid-WB game, career miss, portrait Illari 0.836 thrice.
        let mut st = HeroAuthState {
            career_ever_ok: true,
            accepted_hero: Some("Wrecking Ball".into()),
            portrait_pending: None,
        };
        for _ in 0..3 {
            let (h, src, next) =
                resolve_hero(None, Some(("illari", 0.90)), "Illari", &st, canon_map);
            assert_eq!(h, "Wrecking Ball", "must hold WB");
            assert_eq!(src, HeroSource::Held);
            st = next;
        }
    }

    #[test]
    fn portrait_only_switch_needs_two_high_conf() {
        let mut st = HeroAuthState::default();
        // First high-conf portrait: streak=1 — do not open a segment yet.
        let (h1, src1, next1) =
            resolve_hero(None, Some(("illari", 0.90)), "Unknown", &st, canon_map);
        assert_eq!(h1, "Unknown");
        assert_eq!(src1, HeroSource::Held);
        st = next1;
        assert_eq!(st.portrait_pending.as_ref().map(|p| p.1), Some(1));
        assert!(st.accepted_hero.is_none());

        // Second consecutive: switch accepted
        let (h2, src2, next2) =
            resolve_hero(None, Some(("illari", 0.90)), "Unknown", &st, canon_map);
        assert_eq!(h2, "Illari");
        assert_eq!(src2, HeroSource::Portrait);
        assert_eq!(next2.accepted_hero.as_deref(), Some("Illari"));
    }

    #[test]
    fn field_pattern_three_illari_portraits_mid_wb_no_switch() {
        let mut st = HeroAuthState {
            career_ever_ok: true,
            accepted_hero: Some("Wrecking Ball".into()),
            portrait_pending: None,
        };
        for conf in [0.836, 0.836, 0.836] {
            let (h, src, next) =
                resolve_hero(None, Some(("illari", conf)), "Illari", &st, canon_map);
            assert_eq!((h.as_str(), src), ("Wrecking Ball", HeroSource::Held));
            st = next;
        }
        assert_eq!(st.accepted_hero.as_deref(), Some("Wrecking Ball"));
    }

    #[test]
    fn portrait_below_threshold_does_not_start_streak() {
        // Field illari 0.836 < 0.85
        let st = HeroAuthState {
            career_ever_ok: false,
            accepted_hero: Some("Wrecking Ball".into()),
            portrait_pending: None,
        };
        let (h, src, next) = resolve_hero(None, Some(("illari", 0.836)), "Illari", &st, canon_map);
        assert_eq!(h, "Wrecking Ball");
        assert_eq!(src, HeroSource::Held);
        assert!(next.portrait_pending.is_none());
    }

    #[test]
    fn portrait_streak_resets_on_hero_change() {
        let mut st = HeroAuthState::default();
        let (_, _, n1) = resolve_hero(None, Some(("illari", 0.90)), "Unknown", &st, canon_map);
        st = n1;
        let (_, _, n2) = resolve_hero(
            None,
            Some(("wrecking_ball", 0.90)),
            "Unknown",
            &st,
            canon_map,
        );
        assert_eq!(
            n2.portrait_pending.as_ref().map(|p| (p.0.as_str(), p.1)),
            Some(("Wrecking Ball", 1))
        );
    }
}
