//! Temporal stability gate for poll-tick OCR (PR-A, fleet::tracker-fps).
//!
//! Every screen the poller wants to OCR — accolade title, rank screen, map
//! vote, hero ban, hero select — holds still for many seconds, while combat
//! frames never do (camera motion churns every crop tick-to-tick). The
//! brightness pre-gate in `match_end` cannot tell a lit combat frame from a
//! real title (measured: gameplay lights the title crop 15–22% vs a 1%
//! threshold), so before this gate the result-word OCR ran on nearly every
//! poll tick of a live match — the dominant per-tick cost on the 2026-08-11
//! FPS/fuzzy report. Comparing a small grayscale thumbnail of the crop
//! against the previous tick's is theme- and brightness-independent: OCR only
//! runs once the crop has held still across two consecutive ticks.
//!
//! Cost of the trade: detection shifts one poll tick later (the first tick of
//! a new screen has no matching predecessor). The post-match screens stay up
//! ~15–20s (several ticks) and word outcomes already need two agreeing reads
//! inside the confirmation window, so the miss-rate budget is unchanged; the
//! ~3s banner is untouched (color-flood, never OCR-gated).

use std::collections::HashMap;

use image::DynamicImage;

/// Thumbnail size for comparisons. Small enough that downsampling absorbs
/// timers/vote counters and subtle idle animation, large enough that camera
/// motion or a screen change moves many cells.
const THUMB_W: u32 = 32;
const THUMB_H: u32 = 18;

/// Maximum mean absolute per-pixel luma difference (0–255 scale) between
/// consecutive thumbnails still counted as "the same screen". Static screens
/// with countdown digits measure ~0–2; any camera motion in a live match
/// swamps this.
const STABLE_MAX_MEAN_DIFF: f32 = 6.0;

/// Per-region thumbnail history across poll ticks. One instance lives in the
/// poller's session state and travels through the tick's `spawn_blocking`
/// closure; each gated OCR region uses its own key.
#[derive(Debug, Default, Clone)]
pub struct FrameStability {
    prev: HashMap<&'static str, Vec<u8>>,
    /// The value each [`Self::repeats`] key held on the previous tick.
    values: HashMap<&'static str, u8>,
}

impl FrameStability {
    /// Record `crop`'s thumbnail under `key` and report whether it matches the
    /// thumbnail from the previous call with the same key. The first sighting
    /// of a region (or a resolution change) is never stable.
    pub fn check(&mut self, key: &'static str, crop: &DynamicImage) -> bool {
        let thumb = crop.thumbnail_exact(THUMB_W, THUMB_H).to_luma8().into_raw();
        let stable = self.prev.get(key).is_some_and(|prev| {
            prev.len() == thumb.len() && {
                let sum: u64 = prev
                    .iter()
                    .zip(&thumb)
                    .map(|(a, b)| a.abs_diff(*b) as u64)
                    .sum();
                sum as f32 / thumb.len() as f32 <= STABLE_MAX_MEAN_DIFF
            }
        });
        self.prev.insert(key, thumb);
        stable
    }

    /// The same gate for a read that is already made: record this tick's
    /// `value` under `key` and report whether the previous call with the
    /// same key recorded the same value. `None` (this tick read nothing)
    /// clears the key, so two sightings count only on calls in a row. The
    /// caller calls this once per poll tick.
    pub fn repeats(&mut self, key: &'static str, value: Option<u8>) -> bool {
        match value {
            Some(v) => self.values.insert(key, v) == Some(v),
            None => {
                self.values.remove(key);
                false
            }
        }
    }

    /// Drop all history — used when the game process goes away so a stale
    /// thumbnail can't count toward stability across sessions.
    pub fn reset(&mut self) {
        self.prev.clear();
        self.values.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};

    fn solid(r: u8, g: u8, b: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(320, 180, image::Rgb([r, g, b])))
    }

    /// 32px checkerboard, offset by `shift` — an 8px+ shift moves whole
    /// thumbnail cells, modeling camera motion.
    fn checker(shift: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(320, 180, |x, y| {
            if ((x + shift) / 32 + y / 32).is_multiple_of(2) {
                image::Rgb([230, 230, 230])
            } else {
                image::Rgb([20, 20, 20])
            }
        }))
    }

    #[test]
    fn first_sighting_is_never_stable() {
        let mut stab = FrameStability::default();
        assert!(!stab.check("k", &solid(120, 120, 120)));
    }

    #[test]
    fn identical_consecutive_frames_are_stable() {
        let mut stab = FrameStability::default();
        assert!(!stab.check("k", &checker(0)));
        assert!(stab.check("k", &checker(0)));
        assert!(stab.check("k", &checker(0)));
    }

    #[test]
    fn moved_content_is_unstable() {
        let mut stab = FrameStability::default();
        assert!(!stab.check("k", &checker(0)));
        assert!(
            !stab.check("k", &checker(16)),
            "16px shift must read as motion"
        );
        // Holding still again re-establishes stability on the next tick.
        assert!(stab.check("k", &checker(16)));
    }

    #[test]
    fn screen_change_is_unstable() {
        let mut stab = FrameStability::default();
        stab.check("k", &solid(10, 10, 10));
        assert!(!stab.check("k", &solid(240, 240, 240)));
    }

    #[test]
    fn keys_are_independent() {
        let mut stab = FrameStability::default();
        stab.check("a", &solid(10, 10, 10));
        // A first sighting under "b" is unstable even though "a" has history.
        assert!(!stab.check("b", &solid(10, 10, 10)));
        assert!(stab.check("a", &solid(10, 10, 10)));
    }

    #[test]
    fn reset_clears_history() {
        let mut stab = FrameStability::default();
        stab.check("k", &solid(10, 10, 10));
        stab.repeats("v", Some(1));
        stab.reset();
        assert!(!stab.check("k", &solid(10, 10, 10)));
        assert!(!stab.repeats("v", Some(1)));
    }

    /// A value repeats only when the call before it, under the same key,
    /// recorded the same value.
    #[test]
    fn a_value_repeats_on_two_calls_in_a_row() {
        let mut stab = FrameStability::default();
        assert!(!stab.repeats("v", Some(1)), "a first sighting");
        assert!(stab.repeats("v", Some(1)));
        assert!(stab.repeats("v", Some(1)));
        // another value starts over
        assert!(!stab.repeats("v", Some(2)));
        assert!(stab.repeats("v", Some(2)));
        // a tick with no value breaks the run
        assert!(!stab.repeats("v", None));
        assert!(!stab.repeats("v", Some(2)));
        assert!(stab.repeats("v", Some(2)));
        // keys are independent, and so are the thumbnails
        assert!(!stab.repeats("w", Some(2)));
        assert!(!stab.check("v", &solid(10, 10, 10)));
        assert!(stab.repeats("v", Some(2)));
    }
}
