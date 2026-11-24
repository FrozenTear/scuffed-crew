//! CG-4 Lane C: hero-source authority for scoreboard captures.
//!
//! Career-panel title is always authoritative when present. Portrait template
//! matching is ignored below [`PORTRAIT_SWITCH_MIN_CONF`] (a live Tab guessed
//! Sierra or Tracer at 0.77). At or above that floor it may **confirm** the
//! current accepted hero, and may **switch** only when career-panel has never
//! succeeded this game **and** ≥2 consecutive identical matches land. That
//! blocks the 07-22 field failure (portrait Illari 0.836 mid-Wrecking Ball
//! game while career-panel was unreadable) without blocking real career-panel
//! swaps (e.g. Ramattra to Reinhardt).
//!
//! Under `reader = "new"` the career-panel title is not read (alpha.7 O8, OCR
//! site T5). [`resolve_hero_new_reader`] puts the heroes-v2 icon reads in its
//! place (the career panel's portrait and the member's own Tab row), with the
//! same authority, and never reads OCR text. `reader = "ocr-v1"` keeps
//! [`resolve_hero`] exactly as it was.

use serde::{Deserialize, Serialize};

/// Minimum portrait confidence to count toward a portrait-only switch streak.
pub const PORTRAIT_SWITCH_MIN_CONF: f64 = 0.85;

/// Consecutive identical high-confidence portrait matches required before a
/// portrait-only switch is allowed (and only if career-panel never succeeded).
pub const PORTRAIT_SWITCH_STREAK: u32 = 2;

/// Cross-capture hero authority for one active game.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HeroAuthState {
    /// The authoritative hero source named a hero at least once this game:
    /// the career-panel title under ocr-v1, the heroes-v2 icon reads under
    /// the new reader. The name is kept for saved active-game state.
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
    /// The heroes-v2 icon reads (`reader = "new"`): the career panel's
    /// portrait and the member's own Tab row. They have the career-panel
    /// title's authority; see [`resolve_hero_new_reader`].
    HeroPack,
    Portrait,
    /// Held previous accepted hero (portrait switch refused).
    Held,
    OcrText,
}

impl HeroSource {
    /// The source with career-panel authority for this reader: it switches
    /// the accepted hero at once and sets [`HeroAuthState::career_ever_ok`].
    pub fn is_authority(self) -> bool {
        matches!(self, Self::CareerPanel | Self::HeroPack)
    }

    /// Log label.
    pub fn label(self) -> &'static str {
        match self {
            Self::CareerPanel => "career_panel",
            Self::HeroPack => "hero_pack",
            Self::Portrait => "portrait",
            Self::Held => "held",
            Self::OcrText => "ocr_text",
        }
    }
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
        let hero = canonicalize(hero);
        next.career_ever_ok = true;
        next.accepted_hero = Some(hero.clone());
        next.portrait_pending = None;
        return (hero, HeroSource::CareerPanel, next);
    }

    if let Some((raw, conf)) = portrait.filter(|(_, conf)| *conf >= PORTRAIT_SWITCH_MIN_CONF) {
        let hero = canonicalize(raw);

        // Confirm: same as currently accepted, and only at the trust floor.
        // A 0.77 guess must not re-assert or override.
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
        let ocr = canonicalize(ocr_text);
        if !ocr.is_empty() && ocr != "Unknown" {
            return (ocr, HeroSource::OcrText, next);
        }
        return ("Unknown".to_string(), HeroSource::Held, next);
    }

    // No career, no portrait this frame.
    next.portrait_pending = None;
    if let Some(prev) = next.accepted_hero.clone() {
        // Prefer held authority over thrashing OCR after career/portrait established.
        let prev = canonicalize(&prev);
        next.accepted_hero = Some(prev.clone());
        return (prev, HeroSource::Held, next);
    }
    (canonicalize(ocr_text), HeroSource::OcrText, next)
}

/// Resolve which hero string to store on this capture under `reader = "new"`.
///
/// The career-panel OCR read (T5) is replaced by `pack`: the hero the
/// heroes-v2 icon reads name, the career panel's portrait and the member's
/// own Tab row, each only when it passed its gate (an accepted hero, not
/// suspect, not a placeholder, skull or empty slot), and never when the two
/// name different heroes (`shadow::TabHeroes::authority`). Every other read
/// arrives as `None`, so a suspect or abstaining read never authenticates a
/// hero.
///
/// * `pack` present: the career-panel rule of [`resolve_hero`]. It is stored,
///   becomes the accepted hero, sets `career_ever_ok` and clears a portrait
///   streak, so it confirms an accepted hero it agrees with and vetoes one it
///   does not.
/// * `pack` absent: [`resolve_hero`] with no career read and no OCR text. The
///   portrait matcher may confirm the accepted hero, or switch on its streak
///   before any authoritative read this game; otherwise the accepted hero is
///   held. With nothing held the hero is `Unknown` ([`HeroSource::Held`]),
///   which stays local until the member picks one. No hero is invented.
pub fn resolve_hero_new_reader(
    pack: Option<&str>,
    portrait: Option<(&str, f64)>,
    state: &HeroAuthState,
    canonicalize: impl Fn(&str) -> String,
) -> (String, HeroSource, HeroAuthState) {
    if let Some(hero) = pack.map(&canonicalize).filter(|h| !is_unknown(h)) {
        let mut next = state.clone();
        next.career_ever_ok = true;
        next.accepted_hero = Some(hero.clone());
        next.portrait_pending = None;
        return (hero, HeroSource::HeroPack, next);
    }
    // No OCR text: an empty string names no hero, so the OCR-text branches of
    // `resolve_hero` can only come back empty.
    match resolve_hero(None, portrait, "", state, canonicalize) {
        (_, HeroSource::OcrText, next) => ("Unknown".to_string(), HeroSource::Held, next),
        resolved => resolved,
    }
}

fn is_unknown(hero: &str) -> bool {
    let hero = hero.trim();
    hero.is_empty() || hero.eq_ignore_ascii_case("unknown")
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
    fn career_panel_beats_a_confident_portrait() {
        let (h, src, next) = resolve_hero(
            Some("Wrecking Ball"),
            Some(("tracer", 0.95)),
            "Tracer",
            &HeroAuthState::default(),
            canon_map,
        );
        assert_eq!(h, "Wrecking Ball");
        assert_eq!(src, HeroSource::CareerPanel);
        assert!(next.career_ever_ok);
        assert!(next.portrait_pending.is_none());
    }

    #[test]
    fn portrait_confirms_current_hero_at_the_trust_floor() {
        let st = HeroAuthState {
            career_ever_ok: true,
            accepted_hero: Some("Wrecking Ball".into()),
            portrait_pending: None,
        };
        let (h, src, next) = resolve_hero(
            None,
            Some(("wrecking_ball", PORTRAIT_SWITCH_MIN_CONF)),
            "Illari",
            &st,
            canon_map,
        );
        assert_eq!(h, "Wrecking Ball");
        assert_eq!(src, HeroSource::Portrait);
        assert_eq!(next.accepted_hero.as_deref(), Some("Wrecking Ball"));
    }

    #[test]
    fn portrait_below_the_trust_floor_does_not_override() {
        let st = HeroAuthState {
            career_ever_ok: false,
            accepted_hero: Some("Wrecking Ball".into()),
            portrait_pending: Some(("Sierra".into(), 1)),
        };
        let (h, src, next) = resolve_hero(None, Some(("sierra", 0.77)), "Tracer", &st, canon_map);
        assert_eq!(h, "Wrecking Ball");
        assert_eq!(src, HeroSource::Held);
        assert!(next.portrait_pending.is_none());

        let (same, same_src, same_next) = resolve_hero(
            None,
            Some(("wrecking_ball", 0.77)),
            "Tracer",
            &st,
            canon_map,
        );
        assert_eq!(same, "Wrecking Ball");
        assert_eq!(same_src, HeroSource::Held);
        assert!(same_next.portrait_pending.is_none());
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

    // ---- reader = "new" (O8): the heroes-v2 own-row read replaces T5.

    fn wb_accepted() -> HeroAuthState {
        HeroAuthState {
            career_ever_ok: true,
            accepted_hero: Some("Wrecking Ball".into()),
            portrait_pending: None,
        }
    }

    #[test]
    fn new_reader_pack_read_has_career_authority() {
        // Switches an accepted hero at once, like a career-panel read.
        let (h, src, next) = resolve_hero_new_reader(
            Some("reinhardt"),
            Some(("illari", 0.95)),
            &wb_accepted(),
            canon_map,
        );
        assert_eq!((h.as_str(), src), ("Reinhardt", HeroSource::HeroPack));
        assert!(next.career_ever_ok);
        assert_eq!(next.accepted_hero.as_deref(), Some("Reinhardt"));
        assert!(next.portrait_pending.is_none());
        assert!(src.is_authority());

        // A first read of the game authenticates on its own.
        let (h, src, next) = resolve_hero_new_reader(
            Some("wrecking_ball"),
            None,
            &HeroAuthState::default(),
            canon_map,
        );
        assert_eq!((h.as_str(), src), ("Wrecking Ball", HeroSource::HeroPack));
        assert!(next.career_ever_ok);
    }

    #[test]
    fn new_reader_abstain_holds_the_accepted_hero() {
        // No pack read (suspect or abstaining): the accepted hero is held,
        // and a confident portrait cannot switch after an authoritative read.
        let mut st = wb_accepted();
        for _ in 0..3 {
            let (h, src, next) =
                resolve_hero_new_reader(None, Some(("illari", 0.95)), &st, canon_map);
            assert_eq!((h.as_str(), src), ("Wrecking Ball", HeroSource::Held));
            assert!(!src.is_authority());
            st = next;
        }
        let (h, src, next) = resolve_hero_new_reader(None, None, &st, canon_map);
        assert_eq!((h.as_str(), src), ("Wrecking Ball", HeroSource::Held));
        assert_eq!(next.accepted_hero.as_deref(), Some("Wrecking Ball"));
        assert!(next.career_ever_ok);
    }

    #[test]
    fn new_reader_abstain_with_nothing_held_is_unknown_never_ocr_text() {
        // Fresh game, no pack read, no portrait: Unknown, held, not OCR text.
        let (h, src, next) =
            resolve_hero_new_reader(None, None, &HeroAuthState::default(), canon_map);
        assert_eq!((h.as_str(), src), ("Unknown", HeroSource::Held));
        assert!(!next.career_ever_ok);
        assert!(next.accepted_hero.is_none());

        // One confident portrait does not open a hero on its own either.
        let (h, src, next) = resolve_hero_new_reader(
            None,
            Some(("illari", 0.95)),
            &HeroAuthState::default(),
            canon_map,
        );
        assert_eq!((h.as_str(), src), ("Unknown", HeroSource::Held));
        assert!(next.accepted_hero.is_none());

        // A pack read that canonicalizes to nothing is no read.
        for blank in ["", "Unknown"] {
            let (h, src, _) =
                resolve_hero_new_reader(Some(blank), None, &HeroAuthState::default(), canon_map);
            assert_eq!((h.as_str(), src), ("Unknown", HeroSource::Held));
        }
    }

    #[test]
    fn new_reader_portrait_rules_are_unchanged_without_a_pack_read() {
        // Before any authoritative read, two confident portraits still switch,
        // exactly as under ocr-v1.
        let st = HeroAuthState::default();
        let (_, _, st) = resolve_hero_new_reader(None, Some(("illari", 0.90)), &st, canon_map);
        let (h, src, st) = resolve_hero_new_reader(None, Some(("illari", 0.90)), &st, canon_map);
        assert_eq!((h.as_str(), src), ("Illari", HeroSource::Portrait));
        let (v1, v1_src, v1_st) = {
            let s = HeroAuthState::default();
            let (_, _, s) = resolve_hero(None, Some(("illari", 0.90)), "Unknown", &s, canon_map);
            resolve_hero(None, Some(("illari", 0.90)), "Unknown", &s, canon_map)
        };
        assert_eq!((h, src, st), (v1, v1_src, v1_st));
    }

    #[test]
    fn ocr_v1_labels_and_authority_are_unchanged() {
        assert!(HeroSource::CareerPanel.is_authority());
        for s in [HeroSource::Portrait, HeroSource::Held, HeroSource::OcrText] {
            assert!(!s.is_authority());
        }
        assert_eq!(HeroSource::CareerPanel.label(), "career_panel");
        assert_eq!(HeroSource::Portrait.label(), "portrait");
        assert_eq!(HeroSource::Held.label(), "held");
        assert_eq!(HeroSource::OcrText.label(), "ocr_text");
        assert_eq!(HeroSource::HeroPack.label(), "hero_pack");
    }
}
