//! Letter templates for the blanker's "not letter-like" glyph gate.
//!
//! Built once by `scripts/gen_blank_letters.py` (18 openly licensed sans
//! fonts, 42 letters, two sub-pixel phases) and bundled, so the gate never
//! depends on the fonts a member has installed. A missing or malformed
//! asset keeps no glyph at all: the gate never switches itself off.

use std::sync::OnceLock;

use super::reader::{CANVAS, CH, CW, normalise};

const LETTERS_1440: &[u8] = include_bytes!("../../../assets/blank-letters/letters_1440.png");
const LETTERS_1080: &[u8] = include_bytes!("../../../assets/blank-letters/letters_1080.png");
/// Templates per set: 18 fonts x 2 phases x 42 letters.
pub(super) const COUNT: usize = 1512;

/// Normalised letter canvases, `CANVAS` floats each.
pub(super) struct Letters {
    pub(super) v: Vec<f32>,
}

impl Letters {
    /// Best correlation of a normalised canvas with any letter.
    pub(super) fn best(&self, canvas: &[f32]) -> f64 {
        self.v
            .as_chunks::<CANVAS>()
            .0
            .iter()
            .map(|t| {
                t.iter()
                    .zip(canvas)
                    .map(|(&a, &b)| f64::from(a) * f64::from(b))
                    .sum::<f64>()
            })
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

fn load(png: &[u8]) -> Option<Letters> {
    let img = image::load_from_memory(png).ok()?.to_luma8();
    if (img.width() as usize, img.height() as usize) != (CW, CH * COUNT) {
        return None;
    }
    let mut v = Vec::with_capacity(CANVAS * COUNT);
    for i in 0..COUNT {
        let mut c = [0f32; CANVAS];
        for y in 0..CH {
            for x in 0..CW {
                c[y * CW + x] =
                    f32::from(img.get_pixel(x as u32, (i * CH + y) as u32).0[0]) / 255.0;
            }
        }
        normalise(&mut c);
        v.extend_from_slice(&c);
    }
    Some(Letters { v })
}

/// The set for the 1440p (`true`) or 1080p digit templates; `None` keeps no
/// glyph.
pub(super) fn letters(set_1440: bool) -> Option<&'static Letters> {
    static L1440: OnceLock<Option<Letters>> = OnceLock::new();
    static L1080: OnceLock<Option<Letters>> = OnceLock::new();
    if set_1440 {
        L1440.get_or_init(|| load(LETTERS_1440)).as_ref()
    } else {
        L1080.get_or_init(|| load(LETTERS_1080)).as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_sets_are_the_generated_ones() {
        // `scripts/gen_blank_letters.py` output; regenerate and update both
        // hashes together, never one alone.
        assert_eq!(
            crate::report::bundle::sha256_hex(LETTERS_1440),
            "3f82659f1e519ee51ae1eabdadc80ba7adb95e6c02044eb43bab77c2d531702c"
        );
        assert_eq!(
            crate::report::bundle::sha256_hex(LETTERS_1080),
            "f091892ae65911a5e4c7ecdc66edaefbd8a5c065f4a170a181a9939bf0522b0f"
        );
        for set in [true, false] {
            let l = letters(set).expect("letter set loads");
            assert_eq!(l.v.len(), CANVAS * COUNT);
        }
    }

    #[test]
    fn a_malformed_asset_loads_no_set() {
        assert!(load(b"not a png").is_none());
        let small = {
            let img = image::GrayImage::new(CW as u32, CH as u32);
            let mut out = Vec::new();
            image::DynamicImage::ImageLuma8(img)
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        assert!(load(&small).is_none());
    }
}
