//! The glyph and word templates the rank-v1 reader matches against, and the
//! learner that builds them from labelled rank screens.
//!
//! Templates are cut from the game's own text, so a pack is never committed.
//! The reader loads it from `<data_dir>/templates/rank-v1/templates.json`
//! ([`TEMPLATE_DIR`], [`PACK_FILE`]); a pack is built locally from labelled
//! frames with [`Learner`] (the ignored bench in `tests.rs` writes one), or
//! arrives in the same folder through the tracker's pack download
//! (`rank-v1.tar`, [`crate::packs`]). Without a pack there is no reader.
//!
//! A template is the mean of the normalised cuts ([`super::ink::vector`]) of
//! one class (a digit, `%`, a tag word, a tier name) in one family (where the
//! text is on screen and so its font and size), plus the mean aspect.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::RgbImage;
use serde::{Deserialize, Serialize};

use super::ink::{Glyph, Plane, aspect, shifted_vectors, unit, vector};
use super::{Labels, layout};

/// The class of the chevrons on a green pill (">>>").
pub const CHEV_RIGHT: &str = "chev_right";
/// The class of the chevrons on a red pill ("<<<", with the minus when it
/// touches them).
pub const CHEV_LEFT: &str = "chev_left";

/// The class of the rank icon left of the hero SR, when its colour is
/// neutral enough to be cut with the digits (a grey disc, for one).
pub const SR_ICON: &str = "sr_icon";

/// Folder under the tracker's data dir.
pub const TEMPLATE_DIR: &str = "templates/rank-v1";
/// The pack file in that folder.
pub const PACK_FILE: &str = "templates.json";

/// Where a cut comes from on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// The "RANK PROGRESS:" label, the rank screen check.
    Gate,
    /// Rank progress percent (cyan, orange when negative).
    Pct,
    /// "Higher ranked than X%" (yellow).
    Higher,
    /// The change pill on the progress bar (white on green or red).
    Pill,
    /// Hero SR digits.
    Sr,
    /// Hero SR change digits (green or red).
    Change,
    /// A change tag word.
    Tag,
    /// The mode banner text.
    Banner,
    /// The tier word under the rank emblem.
    Tier,
    /// The division digit after the tier word.
    Division,
    /// The division digit on a small neighbour emblem at either end of the
    /// progress bar.
    Emblem,
    /// The role icon above the emblem (classes `Tank`, `Damage`, `Support`).
    Role,
}

impl Family {
    pub const NUMERIC: [Family; 5] = [
        Family::Pct,
        Family::Higher,
        Family::Pill,
        Family::Sr,
        Family::Change,
    ];

    /// A single glyph (digit, sign, icon) rather than a word.
    pub fn is_glyph(self) -> bool {
        !matches!(
            self,
            Family::Gate | Family::Tag | Family::Banner | Family::Tier
        )
    }

    /// Vector width, height and the aspect penalty weight.
    pub fn shape(self) -> (usize, usize, f32) {
        match self {
            Family::Gate => (120, 24, 1.0),
            Family::Tag => (160, 24, 1.0),
            Family::Banner => (240, 24, 1.0),
            Family::Tier => (96, 24, 1.0),
            _ => (24, 24, 0.3),
        }
    }
}

/// One class's template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassTemplate {
    pub class: String,
    pub aspect: f32,
    /// How many cuts were averaged.
    pub n: usize,
    pub vector: Vec<f32>,
    /// Filled by numeric pooling, not by a cut of this family. Absent in a
    /// pack written before this field, which loads as `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pooled: bool,
}

/// All templates, by family.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplateSet {
    pub recognizer: String,
    pub families: BTreeMap<Family, Vec<ClassTemplate>>,
}

/// One ranked match.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub class: String,
    pub score: f32,
    /// Lead over the second best allowed class (the score when alone).
    pub margin: f32,
}

#[derive(Debug)]
pub enum TemplateError {
    /// No pack file.
    Missing(PathBuf),
    /// The file could not be read, is not a pack, or is a pack of another
    /// recognizer or another vector shape ([`TemplateSet::check`]).
    Bad(PathBuf, String),
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateError::Missing(p) => {
                write!(f, "rank-v1 template pack missing: {}", p.display())
            }
            TemplateError::Bad(p, e) => write!(f, "rank-v1 template pack {}: {e}", p.display()),
        }
    }
}

impl std::error::Error for TemplateError {}

impl TemplateSet {
    /// Load `<data_dir>/templates/rank-v1/templates.json`.
    pub fn load(data_dir: &Path) -> Result<Self, TemplateError> {
        Self::load_dir(&data_dir.join(TEMPLATE_DIR))
    }

    /// Load `<dir>/templates.json` and [`check`](Self::check) it.
    pub fn load_dir(dir: &Path) -> Result<Self, TemplateError> {
        let p = dir.join(PACK_FILE);
        let s = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(TemplateError::Missing(p));
            }
            Err(e) => return Err(TemplateError::Bad(p, e.to_string())),
        };
        let set: Self =
            serde_json::from_str(&s).map_err(|e| TemplateError::Bad(p.clone(), e.to_string()))?;
        set.check().map_err(|e| TemplateError::Bad(p, e))?;
        Ok(set)
    }

    /// A pack this reader can use: written for [`super::RECOGNIZER_ID`],
    /// every vector the size its family compares at ([`Family::shape`]),
    /// finite values, a positive aspect, and the gate template. A pack of
    /// another shape would otherwise load and score garbage.
    pub fn check(&self) -> Result<(), String> {
        if self.recognizer != super::RECOGNIZER_ID {
            return Err(format!(
                "recognizer {:?}, want {:?}",
                self.recognizer,
                super::RECOGNIZER_ID
            ));
        }
        for (f, ts) in &self.families {
            let (w, h, _) = f.shape();
            for t in ts {
                if t.vector.len() != w * h {
                    return Err(format!(
                        "{f:?} {:?}: {} values, want {}",
                        t.class,
                        t.vector.len(),
                        w * h
                    ));
                }
                if !(t.aspect.is_finite() && t.aspect > 0.0)
                    || t.vector.iter().any(|v| !v.is_finite())
                {
                    return Err(format!("{f:?} {:?}: not a finite template", t.class));
                }
            }
        }
        if !self.has_class(Family::Gate, super::GATE_CLASS) {
            return Err("no gate template".to_string());
        }
        Ok(())
    }

    pub fn save_dir(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let s = serde_json::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(PACK_FILE), s)
    }

    pub fn has_class(&self, f: Family, class: &str) -> bool {
        self.families
            .get(&f)
            .is_some_and(|v| v.iter().any(|t| t.class == class))
    }

    /// Classes of a family, best first, limited to `allowed` when given.
    pub fn rank(&self, f: Family, ink: &Plane, allowed: Option<&[&str]>) -> Vec<(f32, &str)> {
        let Some(ts) = self.families.get(&f) else {
            return Vec::new();
        };
        let (w, h, k) = f.shape();
        // glyphs may sit a vector pixel off their template: best of the
        // shifts; words are stretched to fill and are compared as they are
        let vs = if f.is_glyph() {
            shifted_vectors(ink, w, h)
        } else {
            vec![vector(ink, w, h)]
        };
        let a = aspect(ink);
        let mut out: Vec<(f32, &str)> = ts
            .iter()
            .filter(|t| allowed.is_none_or(|al| al.contains(&t.class.as_str())))
            .map(|t| {
                let dot = vs
                    .iter()
                    .map(|v| v.iter().zip(&t.vector).map(|(x, y)| x * y).sum::<f32>())
                    .fold(f32::MIN, f32::max);
                (dot - k * (a / t.aspect).ln().abs(), t.class.as_str())
            })
            .collect();
        out.sort_by(|x, y| y.0.total_cmp(&x.0));
        out
    }

    /// The best class with its score and lead.
    pub fn best(&self, f: Family, ink: &Plane, allowed: Option<&[&str]>) -> Option<Match> {
        let r = self.rank(f, ink, allowed);
        let (score, class) = *r.first()?;
        let second = r.get(1).map_or(0.0, |x| x.0);
        Some(Match {
            class: class.to_string(),
            score,
            margin: score - second,
        })
    }
}

/// One normalised cut and its aspect.
type Example = (Vec<f32>, f32);

/// Builds a [`TemplateSet`] from labelled rank screens: each field's cuts
/// are paired with the label's characters when the counts agree, and
/// skipped when they do not.
#[derive(Debug, Default, Clone)]
pub struct Learner {
    examples: BTreeMap<Family, BTreeMap<String, Vec<Example>>>,
}

impl Learner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Classes a frame actually contributed, before numeric pooling.
    /// [`build`](Self::build) also copies a digit seen in one numeric family
    /// into the others. Those copies are not observed.
    /// Add every example from `other`. Used to combine one learner per day.
    pub fn merge(&mut self, other: &Self) {
        for (family, classes) in &other.examples {
            let dest = self.examples.entry(*family).or_default();
            for (class, list) in classes {
                dest.entry(class.clone())
                    .or_default()
                    .extend(list.iter().cloned());
            }
        }
    }

    pub fn observed(&self) -> std::collections::BTreeSet<(Family, String)> {
        self.examples
            .iter()
            .flat_map(|(family, classes)| classes.keys().map(move |class| (*family, class.clone())))
            .collect()
    }

    pub(super) fn add(&mut self, f: Family, class: &str, ink: &Plane) {
        let (w, h, _) = f.shape();
        self.examples
            .entry(f)
            .or_default()
            .entry(class.to_string())
            .or_default()
            .push((vector(ink, w, h), aspect(ink)));
    }

    /// Learn from one rank screen with its labels. Returns how many cuts
    /// were kept.
    pub fn add_frame(&mut self, img: &RgbImage, labels: &Labels) -> usize {
        if layout::Scale::from_img(img).is_none() {
            return 0;
        }
        let digits = |v: i64| -> Vec<String> {
            v.unsigned_abs()
                .to_string()
                .chars()
                .map(String::from)
                .collect()
        };
        let s = |x: &str| x.to_string();
        let mut n = 0;
        if let Some(g) = layout::gate_word(img) {
            self.add(Family::Gate, super::GATE_CLASS, &g.ink);
            n += 1;
        }
        if let Some(v) = labels.progress_pct {
            let mut want = Vec::new();
            if v < 0 {
                want.push(s("-"));
            }
            want.extend(digits(v.into()));
            want.push(s("%"));
            n += self.pair(Family::Pct, &want, &layout::pct_glyphs(img).0);
        }
        if let Some(v) = labels.higher_than_pct {
            let mut want = digits(v.into());
            want.push(s("%"));
            n += self.pair(Family::Higher, &want, &layout::higher_glyphs(img));
        }
        if let Some(v) = labels.progress_change {
            let gl = layout::pill_glyphs(img).0;
            let ds = digits(v.into());
            let k = gl.len();
            let mut want = Vec::new();
            if v > 0 && (k == ds.len() + 2 || k == ds.len() + 3) {
                want.push(s("+"));
                want.extend(ds);
                want.push(s("%"));
                if k == want.len() + 1 {
                    want.push(s(CHEV_RIGHT));
                }
            } else if v < 0 && (k == ds.len() + 1 || k == ds.len() + 2) {
                if k == ds.len() + 2 {
                    want.push(s(CHEV_LEFT));
                }
                want.extend(ds);
                want.push(s("%"));
            }
            n += self.pair(Family::Pill, &want, &gl);
        }
        let entry = layout::first_entry(img);
        if let (Some(v), Some(e)) = (labels.hero_sr, entry.as_ref()) {
            let ds = digits(v.into());
            let gl = layout::sr_glyphs(img, e);
            // the rank icon (one or two cuts) may sit left of the digits;
            // a single cut there is learned as the icon
            if gl.len() >= ds.len() && gl.len() - ds.len() <= 2 {
                let k = gl.len() - ds.len();
                n += self.pair(Family::Sr, &ds, &gl[k..]);
                if k == 1 {
                    self.add(Family::Sr, SR_ICON, &gl[0].ink);
                    n += 1;
                }
            }
        }
        if let (Some(v), Some(e)) = (labels.hero_sr_change, entry.as_ref()) {
            n += self.pair(
                Family::Change,
                &digits(v.into()),
                &layout::change_glyphs(img, e).0,
            );
        }
        if let Some(tags) = &labels.change_tags {
            n += self.pair(Family::Tag, tags, &layout::tag_words(img));
        }
        if let (Some(b), Some(g)) = (&labels.next_mode_hint, layout::banner_word(img)) {
            self.add(Family::Banner, b, &g.ink);
            n += 1;
        }
        if let (Some(r), layout::RoleLine::Icon(g)) = (labels.role, layout::role_icon(img))
            && r != super::Role::Open
        {
            self.add(Family::Role, r.as_str(), &g.ink);
            n += 1;
        }
        if let Some((word, digit)) = layout::rank_text(img) {
            if let Some(t) = &labels.role_rank {
                self.add(Family::Tier, t, &word.ink);
                n += 1;
            }
            if let Some(d) = labels.division.filter(|d| (1..=5).contains(d)) {
                self.add(Family::Division, &d.to_string(), &digit.ink);
                n += 1;
            }
        }
        if let Some(d) = labels.division.filter(|d| (1..=5).contains(d)) {
            let (left, right) = layout::emblem_digits(img);
            let (l, r) = super::neighbours(d);
            for (g, c) in [(left, l), (right, r)] {
                if let Some(g) = g {
                    self.add(Family::Emblem, &c.to_string(), &g.ink);
                    n += 1;
                }
            }
        }
        n
    }

    /// Add each cut under its label when the counts agree.
    fn pair(&mut self, f: Family, want: &[String], gl: &[Glyph]) -> usize {
        if want.is_empty() || want.len() != gl.len() {
            return 0;
        }
        for (c, g) in want.iter().zip(gl) {
            self.add(f, c, &g.ink);
        }
        want.len()
    }

    /// Mean templates. A numeric family that never saw a class borrows the
    /// class's template pooled over all numeric families (the same digit in
    /// another size), so a digit seen only in one field still reads in the
    /// others.
    pub fn build(self) -> TemplateSet {
        let mean = |lst: &[Example], class: &str| {
            let len = lst[0].0.len();
            let mut v = vec![0.0; len];
            for (x, _) in lst {
                v.iter_mut().zip(x).for_each(|(a, b)| *a += b);
            }
            let la = lst.iter().map(|(_, a)| a.ln()).sum::<f32>() / lst.len() as f32;
            ClassTemplate {
                class: class.to_string(),
                aspect: la.exp(),
                n: lst.len(),
                vector: unit(v),
                pooled: false,
            }
        };
        let mut pool: BTreeMap<String, Vec<Example>> = BTreeMap::new();
        for f in Family::NUMERIC {
            for (c, lst) in self.examples.get(&f).into_iter().flatten() {
                pool.entry(c.clone())
                    .or_default()
                    .extend(lst.iter().cloned());
            }
        }
        let mut families: BTreeMap<Family, Vec<ClassTemplate>> = BTreeMap::new();
        for (f, classes) in &self.examples {
            families.insert(*f, classes.iter().map(|(c, l)| mean(l, c)).collect());
        }
        for f in Family::NUMERIC {
            let have = families.entry(f).or_default();
            for (c, lst) in &pool {
                if !have.iter().any(|t| &t.class == c) {
                    let mut filled = mean(lst, c);
                    filled.pooled = true;
                    have.push(filled);
                }
            }
        }
        TemplateSet {
            recognizer: super::RECOGNIZER_ID.to_string(),
            families,
        }
    }
}
