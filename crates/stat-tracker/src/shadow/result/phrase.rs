//! Phase-screen titles from the words-v1 pack (alpha.7 O7, OCR site P7).
//!
//! words-v1 ships `phrase_<id>.png` masks plus `phrases.json` (`{id: {text,
//! roi, plane, work}}`): the title of a match-start screen, cut from real
//! frames in the centred title band (x 36-64%, y 11-23.5% of the 16:9 game
//! rect, plane `min`: white text), at the result reader's 720p work scale.
//! Under `reader = "new"` the phase detectors in `detect::match_start` read
//! these in place of ocr-v1's P7 OCR, behind the same pixel gates:
//!
//! * `vote_for_a_map`: the map vote ([`Phrase::VoteForAMap`]);
//! * `selecting_map`: the same screen once the vote closed and the map is
//!   drawn ([`Phrase::SelectingMap`], words-v1 version 2; ocr-v1 reads it as
//!   a map vote through the subtitle's "VOTES");
//! * `ban_heroes`: the hero-ban vote ([`Phrase::BanHeroes`]);
//! * `select_a_preferred_hero`: the preferred-hero pick
//!   ([`Phrase::SelectAPreferredHero`]).
//!
//! Other ids in the file (words-v1 also has `vote_result`) are not loaded:
//! ocr-v1 takes no signal from them.
//!
//! The search and its certification are the result word's
//! ([`super::search_fast`], [`super::certify`]): a fast score at the floor
//! is a real window's score, so it is sure; a fast score between
//! [`PRESENT`] and the floor may be an under-read, so the exact check
//! decides whether any window reaches [`PHRASE_FLOOR`]. A sure read here is
//! therefore a sure exhaustive read. Only a sure read is a phase signal.

use std::collections::BTreeMap;
use std::path::Path;

use image::RgbImage;
use serde::{Deserialize, Serialize};

use super::{
    CERT_MACS, Cert, Gray, Level, PRESENT, Phased, Plane, Proj, Stack, WORK, certify, load_gray,
    make_phased, region, roi_is_valid, scaled_stacks, search_fast, search_full,
};

/// The phrase list in the words pack folder.
pub const PHRASES_FILE: &str = "phrases.json";
/// Minimum score for a sure phrase read, the floor of the other words-v1
/// reads. Measured 2026-10-13 on frames none of the masks was cut from: each
/// mask scores 0.79 to 0.98 on the 56 frames of 3 VOD games (1080p) that
/// show its title, and no mask scores above 0.51 on a frame without its
/// title (0.50 to 0.51 for the vote mask on 10 SELECTING MAP frames, at most
/// 0.49 on the rest of the 205 VOD frames, at most 0.41 on 751 other frames).
pub const PHRASE_FLOOR: f32 = 0.70;

/// A phase-screen title the detectors read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Phrase {
    /// "VOTE FOR A MAP" (map vote).
    VoteForAMap,
    /// "SELECTING MAP" (the map vote after the timer: the map is drawn).
    SelectingMap,
    /// "BAN HEROES" (hero-ban vote; the counter after it is not in the mask).
    BanHeroes,
    /// "SELECT A PREFERRED HERO" (preferred-hero pick).
    SelectAPreferredHero,
}

impl Phrase {
    pub const ALL: [Phrase; 4] = [
        Phrase::VoteForAMap,
        Phrase::SelectingMap,
        Phrase::BanHeroes,
        Phrase::SelectAPreferredHero,
    ];

    /// The pack id: `phrases.json` key and `phrase_<id>.png` stem.
    pub fn id(self) -> &'static str {
        match self {
            Phrase::VoteForAMap => "vote_for_a_map",
            Phrase::SelectingMap => "selecting_map",
            Phrase::BanHeroes => "ban_heroes",
            Phrase::SelectAPreferredHero => "select_a_preferred_hero",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }
}

/// What one frame says about one phrase.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum PhraseRead {
    /// A window scores [`PHRASE_FLOOR`] or more: the title is on the frame.
    /// The only read that is a signal.
    Sure(f32),
    /// The best window scores [`PRESENT`] or more but no window reaches the
    /// floor, or the exact check ran out of budget. Not a signal.
    Suspect(f32),
    /// The fast search found no window at [`PRESENT`]. Not a signal.
    Absent(f32),
}

impl PhraseRead {
    /// Only a sure read says the title is on the frame.
    pub fn is_signal(self) -> bool {
        matches!(self, PhraseRead::Sure(_))
    }

    fn of(score: f64) -> Self {
        let s = score.max(0.0) as f32;
        if score >= PHRASE_FLOOR as f64 {
            PhraseRead::Sure(s)
        } else if score >= PRESENT as f64 {
            PhraseRead::Suspect(s)
        } else {
            PhraseRead::Absent(s)
        }
    }
}

/// One `phrases.json` entry. Extra fields (`text`, `frames`) are allowed.
#[derive(Debug, Deserialize)]
struct Entry {
    roi: [f64; 4],
    plane: Plane,
    work: f64,
}

#[derive(Debug, Clone)]
struct PhraseTemplate {
    phrase: Phrase,
    roi: [f64; 4],
    plane: Plane,
    stacks: Vec<Stack>,
}

/// The loaded phrase masks.
#[derive(Debug, Clone)]
pub struct PhraseTemplates {
    tpls: Vec<PhraseTemplate>,
}

impl PhraseTemplates {
    /// Load the phrases of `<dir>/phrases.json` that a detector reads, each
    /// from `<dir>/phrase_<id>.png`. `None` without the file or without any
    /// loadable phrase.
    ///
    /// The file comes from a downloaded pack, so a file that does not parse
    /// refuses the folder, and so does any entry (read or not) whose id is
    /// not a plain word id ([`crate::packs::is_word_id`]), whose region is
    /// not inside the game rect, or whose work scale is not the reader's.
    /// A listed phrase whose mask is missing or flat is left out, and so is
    /// one whose mask cannot fit its region ([`fits`]): a mask that can
    /// never match would silence its detector, so it counts as not loaded.
    pub fn load_dir(dir: &Path) -> Option<Self> {
        let text = match std::fs::read_to_string(dir.join(PHRASES_FILE)) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            // Not UTF-8, a directory, no permission: warned like an
            // unreadable layouts.json (F253-14, F260-13).
            Err(e) => {
                tracing::warn!(error = %e, "phrases.json unreadable; phrases refused");
                return None;
            }
        };
        let entries: BTreeMap<String, Entry> = match serde_json::from_str(&text) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "phrases.json does not parse; phrases refused");
                return None;
            }
        };
        let mut tpls = Vec::new();
        for (id, e) in entries {
            if !crate::packs::is_word_id(&id)
                || !roi_is_valid(e.roi)
                || !e.work.is_finite()
                || (e.work - WORK).abs() > 1e-9
            {
                tracing::warn!(
                    dir = %dir.display(),
                    "phrases.json has an unsafe id, region or work scale; phrases refused"
                );
                return None;
            }
            let Some(phrase) = Phrase::from_id(&id) else {
                continue;
            };
            let Some(g) = load_gray(&dir.join(format!("phrase_{id}.png"))) else {
                continue;
            };
            tpls.extend(Self::template(phrase, e.roi, e.plane, &g));
        }
        (!tpls.is_empty()).then_some(Self { tpls })
    }

    fn template(phrase: Phrase, roi: [f64; 4], plane: Plane, t: &Gray) -> Option<PhraseTemplate> {
        let stacks = scaled_stacks(t);
        if !fits(roi, &stacks) {
            static LOGGED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    phrase = phrase.id(),
                    mask_w = t.w,
                    mask_h = t.h,
                    "phrase mask does not fit its region; treated as not loaded"
                );
            }
            return None;
        }
        (!stacks.is_empty()).then_some(PhraseTemplate {
            phrase,
            roi,
            plane,
            stacks,
        })
    }

    /// Build from in-memory masks (tests). `None` when none is usable.
    pub fn from_templates(tpls: Vec<(Phrase, [f64; 4], Plane, Gray)>) -> Option<Self> {
        let tpls: Vec<_> = tpls
            .into_iter()
            .filter_map(|(phrase, roi, plane, g)| Self::template(phrase, roi, plane, &g))
            .collect();
        (!tpls.is_empty()).then_some(Self { tpls })
    }

    /// Whether this phrase has a mask.
    pub fn has(&self, phrase: Phrase) -> bool {
        self.tpls.iter().any(|t| t.phrase == phrase)
    }

    /// Phrase ids with a mask (logged on load).
    pub fn ids(&self) -> Vec<&'static str> {
        self.tpls.iter().map(|t| t.phrase.id()).collect()
    }

    fn get(&self, phrase: Phrase) -> Option<&PhraseTemplate> {
        self.tpls.iter().find(|t| t.phrase == phrase)
    }

    /// Read one phrase off one full frame with the coarse-to-fine search,
    /// certified against the floor when the fast score is between
    /// [`PRESENT`] and the floor. `None` when this phrase has no mask (the
    /// caller keeps its own read).
    pub fn read(&self, phrase: Phrase, rgb: &RgbImage) -> Option<PhraseRead> {
        self.read_with_budget(phrase, rgb, CERT_MACS)
    }

    fn read_with_budget(
        &self,
        phrase: Phrase,
        rgb: &RgbImage,
        budget: usize,
    ) -> Option<PhraseRead> {
        let t = self.get(phrase)?;
        let Some(levels) = region(rgb, t.roi, t.plane) else {
            return Some(PhraseRead::Absent(0.0));
        };
        let fast = t
            .stacks
            .iter()
            .map(|st| search_fast(&levels, st).score)
            .fold(-1.0f64, f64::max);
        Some(settle(&t.stacks, &levels, fast, budget))
    }

    /// [`Self::read`] with every window scored (tests, benches). Slow.
    pub fn read_exhaustive(&self, phrase: Phrase, rgb: &RgbImage) -> Option<PhraseRead> {
        let t = self.get(phrase)?;
        let Some(levels) = region(rgb, t.roi, t.plane) else {
            return Some(PhraseRead::Absent(0.0));
        };
        Some(PhraseRead::of(
            t.stacks
                .iter()
                .map(|st| search_full(&levels, st).score)
                .fold(-1.0f64, f64::max),
        ))
    }
}

/// Whether a mask at every search scale fits inside `roi` at the work scale
/// (F260-5). The region is cut with truncated corners ([`region`]), so it
/// can be one pixel short of `fraction * size` on each axis; a mask must fit
/// that smallest region. A region with no pixels fits nothing.
fn fits(roi: [f64; 4], stacks: &[Stack]) -> bool {
    let (work_w, work_h) = (WORK * 1440.0 * 16.0 / 9.0, WORK * 1440.0);
    let rw = ((roi[2] - roi[0]) * work_w).floor() - 1.0;
    let rh = ((roi[3] - roi[1]) * work_h).floor() - 1.0;
    stacks
        .iter()
        .filter_map(|st| st.levels.first())
        .all(|l| l.w as f64 <= rw && l.h as f64 <= rh)
}

/// The read for this fast score: sure at the floor, absent below
/// [`PRESENT`], and in between sure only when [`certify`] finds a window at
/// the floor (suspect when it finds none or runs out of budget).
fn settle(stacks: &[Stack], levels: &[Level], fast: f64, mut budget: usize) -> PhraseRead {
    let read = PhraseRead::of(fast);
    if !matches!(read, PhraseRead::Suspect(_)) {
        return read;
    }
    let tau = PHRASE_FLOOR as f64;
    let mut phased: Vec<((usize, usize), Phased)> = Vec::new();
    for st in stacks {
        for pr in &st.projs {
            make_phased(&mut phased, 0, levels, pr.f);
        }
        let chain: Vec<(&Proj, &Phased)> = st
            .projs
            .iter()
            .filter_map(|pr| {
                let ph = phased.iter().find(|(k, _)| *k == (0, pr.f))?;
                Some((pr, &ph.1))
            })
            .collect();
        match certify(levels, &chain, st, tau, &mut budget) {
            Cert::Below => {}
            Cert::Above(v) => return PhraseRead::Sure(v as f32),
            Cert::Unknown => return read,
        }
    }
    read
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words-v1 title band.
    const BAND: [f64; 4] = [0.36, 0.11, 0.64, 0.235];

    /// Text-like strokes: bright bars on a dark ground, `w` x `h`.
    fn strokes(seed: u64, w: usize, h: usize) -> Gray {
        let mut g = Gray::new(w, h);
        g.px.iter_mut().for_each(|v| *v = 0.08);
        let mut s = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let mut next = |n: usize| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as usize) % n.max(1)
        };
        for _ in 0..(w / 3) {
            let (bw, bh) = (2 + next(4), 4 + next(h.saturating_sub(4)));
            let (x0, y0) = (next(w.saturating_sub(bw)), next(h.saturating_sub(bh)));
            for y in y0..(y0 + bh).min(h) {
                for x in x0..(x0 + bw).min(w) {
                    g.px[y * w + x] = 0.92;
                }
            }
        }
        g
    }

    /// A 1280x720 frame (work scale 1:1) with dark ground and `t` pasted
    /// as grey at (`dx`, `dy`) inside the title band; only the first `cols`
    /// columns of `t` are pasted.
    fn frame_with(t: &Gray, dx: u32, dy: u32, cols: usize) -> RgbImage {
        let mut img = RgbImage::from_pixel(1280, 720, image::Rgb([14, 20, 30]));
        let (x0, y0) = (
            (0.36f64 * 1280.0) as u32 + dx,
            (0.11f64 * 720.0) as u32 + dy,
        );
        for y in 0..t.h {
            for x in 0..t.w {
                let v = if x < cols { t.px[y * t.w + x] } else { 0.08 };
                let c = (v * 255.0).round() as u8;
                img.put_pixel(x0 + x as u32, y0 + y as u32, image::Rgb([c, c, c]));
            }
        }
        img
    }

    fn masks() -> (Gray, Gray, PhraseTemplates) {
        let vote = strokes(11, 150, 26);
        let ban = strokes(29, 118, 26);
        let p = PhraseTemplates::from_templates(vec![
            (Phrase::VoteForAMap, BAND, Plane::Min, vote.clone()),
            (Phrase::BanHeroes, BAND, Plane::Min, ban.clone()),
        ])
        .unwrap();
        (vote, ban, p)
    }

    #[test]
    fn a_phrase_on_the_frame_reads_sure_and_a_blank_frame_absent() {
        let (vote, _, p) = masks();
        let on = frame_with(&vote, 37, 21, vote.w);
        let r = p.read(Phrase::VoteForAMap, &on).unwrap();
        assert!(matches!(r, PhraseRead::Sure(s) if s > 0.95), "{r:?}");
        assert!(r.is_signal());
        // the other phrase is not on this frame
        assert!(!p.read(Phrase::BanHeroes, &on).unwrap().is_signal());
        let blank = RgbImage::from_pixel(1280, 720, image::Rgb([14, 20, 30]));
        assert!(matches!(
            p.read(Phrase::VoteForAMap, &blank),
            Some(PhraseRead::Absent(_))
        ));
        // no mask, no read: the caller keeps its own
        assert_eq!(p.read(Phrase::SelectAPreferredHero, &on), None);
        assert!(!p.has(Phrase::SelectAPreferredHero));
    }

    #[test]
    fn a_partial_phrase_is_suspect_and_never_a_signal() {
        let (vote, _, p) = masks();
        // Part of the title: half-matches, below the floor.
        let part = frame_with(&vote, 12, 30, vote.w * 3 / 5);
        let ex = p.read_exhaustive(Phrase::VoteForAMap, &part).unwrap();
        assert!(matches!(ex, PhraseRead::Suspect(_)), "{ex:?}");
        let r = p.read(Phrase::VoteForAMap, &part).unwrap();
        assert!(matches!(r, PhraseRead::Suspect(_)), "{r:?}");
        assert!(!r.is_signal());
    }

    #[test]
    fn an_under_read_phrase_is_certified() {
        let (vote, _, p) = masks();
        let t = p.get(Phrase::VoteForAMap).unwrap();
        let on = frame_with(&vote, 37, 21, vote.w);
        let levels = region(&on, t.roi, t.plane).unwrap();
        // A fast search that under-read the title at 0.6: the exact check
        // finds the window at the floor.
        // (The exact check stops at the first window at the floor.)
        assert!(matches!(
            settle(&t.stacks, &levels, 0.6, CERT_MACS),
            PhraseRead::Sure(s) if s >= PHRASE_FLOOR
        ));
        // Out of budget: suspect, not sure.
        assert!(matches!(
            settle(&t.stacks, &levels, 0.6, 0),
            PhraseRead::Suspect(_)
        ));
        // Nothing at the floor anywhere: stays suspect.
        let part = frame_with(&vote, 12, 30, vote.w * 3 / 5);
        let off = region(&part, t.roi, t.plane).unwrap();
        assert!(matches!(
            settle(&t.stacks, &off, 0.6, CERT_MACS),
            PhraseRead::Suspect(_)
        ));
        // At the floor or below PRESENT nothing is certified.
        assert!(matches!(
            settle(&t.stacks, &off, 0.71, 0),
            PhraseRead::Sure(_)
        ));
        assert!(matches!(
            settle(&t.stacks, &levels, 0.49, CERT_MACS),
            PhraseRead::Absent(_)
        ));
    }

    #[test]
    fn the_phrase_floor_is_pinned() {
        assert_eq!(PHRASE_FLOOR, 0.70);
        assert!(matches!(PhraseRead::of(0.70), PhraseRead::Sure(_)));
        assert!(matches!(PhraseRead::of(0.699), PhraseRead::Suspect(_)));
        assert!(matches!(PhraseRead::of(0.50), PhraseRead::Suspect(_)));
        assert!(matches!(PhraseRead::of(0.499), PhraseRead::Absent(_)));
    }

    fn save(dir: &Path, name: &str, g: &Gray) {
        let img = image::GrayImage::from_fn(g.w as u32, g.h as u32, |x, y| {
            image::Luma([(g.px[y as usize * g.w + x as usize] * 255.0).round() as u8])
        });
        img.save(dir.join(name)).unwrap();
    }

    fn entry(roi: &str, work: &str) -> String {
        format!(r#"{{"text": "X", "roi": {roi}, "plane": "min", "work": {work}, "frames": 2}}"#)
    }

    #[test]
    fn phrases_load_from_the_words_pack() {
        let d = tempfile::tempdir().unwrap();
        let band = "[0.36, 0.11, 0.64, 0.235]";
        std::fs::write(
            d.path().join(PHRASES_FILE),
            format!(
                r#"{{"vote_for_a_map": {}, "ban_heroes": {}, "select_a_preferred_hero": {}, "vote_result": {}}}"#,
                entry(band, "0.5"),
                entry(band, "0.5"),
                entry(band, "0.5"),
                entry(band, "0.5"),
            ),
        )
        .unwrap();
        let vote = strokes(11, 150, 26);
        save(d.path(), "phrase_vote_for_a_map.png", &vote);
        save(d.path(), "phrase_vote_result.png", &strokes(5, 150, 30));
        // ban_heroes and select_a_preferred_hero are listed without a mask
        let p = PhraseTemplates::load_dir(d.path()).unwrap();
        assert_eq!(p.ids(), vec!["vote_for_a_map"]);
        assert!(p.has(Phrase::VoteForAMap));
        assert!(!p.has(Phrase::BanHeroes));
        let on = frame_with(&vote, 37, 21, vote.w);
        assert!(p.read(Phrase::VoteForAMap, &on).unwrap().is_signal());
        // no phrases.json: nothing loads
        let empty = tempfile::tempdir().unwrap();
        save(empty.path(), "phrase_vote_for_a_map.png", &vote);
        assert!(PhraseTemplates::load_dir(empty.path()).is_none());
    }

    /// A words folder with result words and the vote phrase.
    fn words_pack(dir: &Path, vote: &Gray) {
        std::fs::create_dir_all(dir).unwrap();
        save(dir, "accolade_victory.png", &strokes(3, 60, 14));
        save(dir, "accolade_defeat.png", &strokes(4, 60, 14));
        std::fs::write(
            dir.join(PHRASES_FILE),
            format!(
                r#"{{"vote_for_a_map": {}}}"#,
                entry("[0.36, 0.11, 0.64, 0.235]", "0.5")
            ),
        )
        .unwrap();
        save(dir, "phrase_vote_for_a_map.png", vote);
    }

    #[test]
    fn the_reader_reads_phrases_from_the_words_folder_only() {
        use crate::shadow::{Reader, ReaderConfig};
        let vote = strokes(11, 150, 26);
        let on = frame_with(&vote, 37, 21, vote.w);
        let d = tempfile::tempdir().unwrap();
        let words = d.path().join("words-v1");
        words_pack(&words, &vote);
        let r = Reader::load(&ReaderConfig {
            words_dir: Some(words.clone()),
            ..Default::default()
        });
        assert!(r.read_phrase(Phrase::VoteForAMap, &on).unwrap().is_signal());
        assert_eq!(r.read_phrase(Phrase::BanHeroes, &on), None);
        // The same files in the old result folder carry no phrases.
        let r = Reader::load(&ReaderConfig {
            result_dir: Some(words),
            ..Default::default()
        });
        assert_eq!(r.read_phrase(Phrase::VoteForAMap, &on), None);
        // No pack at all: no read.
        let r = Reader::load(&ReaderConfig::default());
        assert_eq!(r.read_phrase(Phrase::VoteForAMap, &on), None);
    }

    /// F260-4: a refused `phrases.json` refuses the phrases only. The result
    /// words of the same folder (#253) stay loaded.
    #[test]
    fn a_refused_phrases_json_leaves_the_result_words_loaded() {
        use super::super::{ResultDirs, ResultPack};
        let vote = strokes(11, 150, 26);
        let d = tempfile::tempdir().unwrap();
        let words = d.path().join("words-v1");
        words_pack(&words, &vote);
        let dirs = ResultDirs {
            words: Some(words.clone()),
            legacy: None,
        };
        let good = dirs.load().unwrap();
        assert_eq!(good.pack, ResultPack::WordsV1);
        assert!(good.phrases.is_some_and(|p| p.has(Phrase::VoteForAMap)));
        for body in [
            r#"{"vote_for_a_map": "#.to_string(),
            format!(
                r#"{{"../x": {}}}"#,
                entry("[0.36, 0.11, 0.64, 0.235]", "0.5")
            ),
        ] {
            std::fs::write(words.join(PHRASES_FILE), &body).unwrap();
            let l = dirs.load().expect("the result words still load");
            assert_eq!(l.pack, ResultPack::WordsV1, "{body}");
            assert!(l.phrases.is_none(), "{body}");
        }
    }

    /// F260-13: a `phrases.json` that cannot be read gives no phrases (and a
    /// warning), like a missing one gives none.
    #[test]
    fn an_unreadable_phrases_json_gives_no_phrases() {
        let vote = strokes(11, 150, 26);
        let bad = tempfile::tempdir().unwrap();
        std::fs::write(bad.path().join(PHRASES_FILE), [0xff, 0xfe, b'{', b'}']).unwrap();
        save(bad.path(), "phrase_vote_for_a_map.png", &vote);
        assert!(PhraseTemplates::load_dir(bad.path()).is_none(), "not UTF-8");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(PHRASES_FILE)).unwrap();
        save(dir.path(), "phrase_vote_for_a_map.png", &vote);
        assert!(
            PhraseTemplates::load_dir(dir.path()).is_none(),
            "a directory"
        );
    }

    /// F260-5: a mask that cannot fit its region is not loaded, so its
    /// detector keeps the OCR check instead of reading absent forever.
    #[test]
    fn a_mask_that_cannot_fit_its_region_is_not_loaded() {
        let band = "[0.36, 0.11, 0.64, 0.235]";
        // The band is 358 x 90 at the work scale.
        for (w, h, ok) in [
            (150, 26, true),
            (340, 84, true),
            // fits at 1.00 but not at 1.04
            (350, 26, false),
            (150, 88, false),
            (400, 26, false),
            (150, 120, false),
        ] {
            let d = tempfile::tempdir().unwrap();
            std::fs::write(
                d.path().join(PHRASES_FILE),
                format!(
                    r#"{{"vote_for_a_map": {}, "ban_heroes": {}}}"#,
                    entry(band, "0.5"),
                    entry(band, "0.5")
                ),
            )
            .unwrap();
            save(d.path(), "phrase_vote_for_a_map.png", &strokes(11, w, h));
            save(d.path(), "phrase_ban_heroes.png", &strokes(29, 118, 26));
            let p = PhraseTemplates::load_dir(d.path()).unwrap();
            assert_eq!(p.has(Phrase::VoteForAMap), ok, "{w}x{h}");
            // the other phrase of the pack is not affected
            assert!(p.has(Phrase::BanHeroes));
            let blank = RgbImage::from_pixel(1280, 720, image::Rgb([14, 20, 30]));
            assert_eq!(p.read(Phrase::VoteForAMap, &blank).is_some(), ok);
        }
        // A valid region with no pixels at the work scale fits nothing.
        let thin = [0.5, 0.5, 0.5004, 0.5004];
        assert!(roi_is_valid(thin));
        assert!(
            PhraseTemplates::from_templates(vec![(
                Phrase::VoteForAMap,
                thin,
                Plane::Min,
                strokes(11, 150, 26)
            )])
            .is_none()
        );
    }

    /// words-v1 version 2 adds `selecting_map`; a version 1 folder without
    /// it loads as before.
    #[test]
    fn selecting_map_loads_when_the_pack_has_it() {
        let band = "[0.36, 0.11, 0.64, 0.235]";
        let (vote, sel) = (strokes(11, 150, 26), strokes(47, 140, 26));
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join(PHRASES_FILE),
            format!(r#"{{"vote_for_a_map": {}}}"#, entry(band, "0.5")),
        )
        .unwrap();
        save(d.path(), "phrase_vote_for_a_map.png", &vote);
        let v1 = PhraseTemplates::load_dir(d.path()).unwrap();
        assert_eq!(v1.ids(), vec!["vote_for_a_map"]);
        let on = frame_with(&sel, 37, 21, sel.w);
        assert_eq!(v1.read(Phrase::SelectingMap, &on), None);
        std::fs::write(
            d.path().join(PHRASES_FILE),
            format!(
                r#"{{"vote_for_a_map": {}, "selecting_map": {}}}"#,
                entry(band, "0.5"),
                entry(band, "0.5")
            ),
        )
        .unwrap();
        save(d.path(), "phrase_selecting_map.png", &sel);
        let v2 = PhraseTemplates::load_dir(d.path()).unwrap();
        assert_eq!(v2.ids(), vec!["selecting_map", "vote_for_a_map"]);
        assert!(v2.read(Phrase::SelectingMap, &on).unwrap().is_signal());
        assert!(!v2.read(Phrase::VoteForAMap, &on).unwrap().is_signal());
        assert_eq!(Phrase::from_id("selecting_map"), Some(Phrase::SelectingMap));
    }

    #[test]
    fn a_hostile_phrases_json_refuses_the_folder() {
        let band = "[0.36, 0.11, 0.64, 0.235]";
        let vote = strokes(11, 150, 26);
        for body in [
            format!(
                r#"{{"vote_for_a_map": {}, "../x": {}}}"#,
                entry(band, "0.5"),
                entry(band, "0.5")
            ),
            format!(
                r#"{{"vote_for_a_map": {}, "vote_result": {}}}"#,
                entry(band, "0.5"),
                entry("[0.36, 0.11, 1.64, 0.235]", "0.5")
            ),
            format!(r#"{{"vote_for_a_map": {}}}"#, entry(band, "1.0")),
            r#"{"vote_for_a_map": "#.to_string(),
        ] {
            let d = tempfile::tempdir().unwrap();
            std::fs::write(d.path().join(PHRASES_FILE), &body).unwrap();
            save(d.path(), "phrase_vote_for_a_map.png", &vote);
            assert!(PhraseTemplates::load_dir(d.path()).is_none(), "{body}");
        }
    }
}
