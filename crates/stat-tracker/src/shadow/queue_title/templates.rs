//! The title templates the queue title reader matches against, and the
//! learner that builds them from labelled pregame frames.
//!
//! Templates are cut from the game's own screens, so a pack is never
//! committed. The reader loads it from
//! `<data_dir>/templates/queue-title-v1/templates.json` ([`TEMPLATE_DIR`],
//! [`PACK_FILE`]); a pack is built locally with [`Learner`], or arrives in
//! the same folder through the tracker's pack download
//! (`queue-title-v1.tar`, [`crate::packs`]). Without a pack there is no
//! reader.
//!
//! A template is the mean of the unpadded ink planes ([`super::centre`]) of
//! one title, `TW x TH` values, row-major.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::RgbImage;
use serde::{Deserialize, Serialize};

use super::super::ncc::Gray;
use super::{RECOGNIZER_ID, Screen, TH, TW, centre, plane};

/// Folder under the tracker's data dir.
pub const TEMPLATE_DIR: &str = "templates/queue-title-v1";
/// The pack file in that folder.
pub const PACK_FILE: &str = "templates.json";

/// One title's template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassTemplate {
    /// The title as written on screen (`OPEN QUEUE`).
    pub class: String,
    /// How many frames were averaged.
    pub n: usize,
    /// `TW x TH` ink values, row-major.
    pub vector: Vec<f32>,
}

/// All title templates.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplateSet {
    pub recognizer: String,
    pub width: usize,
    pub height: usize,
    pub classes: Vec<ClassTemplate>,
}

#[derive(Debug)]
pub enum TemplateError {
    Missing(PathBuf),
    Bad(PathBuf, String),
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateError::Missing(p) => {
                write!(f, "queue title template pack missing: {}", p.display())
            }
            TemplateError::Bad(p, e) => {
                write!(f, "queue title template pack {}: {e}", p.display())
            }
        }
    }
}

impl std::error::Error for TemplateError {}

impl TemplateSet {
    /// Load `<data_dir>/templates/queue-title-v1/templates.json`.
    pub fn load(data_dir: &Path) -> Result<Self, TemplateError> {
        Self::load_dir(&data_dir.join(TEMPLATE_DIR))
    }

    /// Load a pack and check its shape matches this build.
    pub fn load_dir(dir: &Path) -> Result<Self, TemplateError> {
        let p = dir.join(PACK_FILE);
        let s = std::fs::read_to_string(&p).map_err(|_| TemplateError::Missing(p.clone()))?;
        let set: Self =
            serde_json::from_str(&s).map_err(|e| TemplateError::Bad(p.clone(), e.to_string()))?;
        set.check().map_err(|e| TemplateError::Bad(p, e))?;
        Ok(set)
    }

    /// The pack has this build's template size and at least one class.
    pub fn check(&self) -> Result<(), String> {
        if self.width != TW || self.height != TH {
            return Err(format!(
                "template size {}x{}, this build reads {TW}x{TH}",
                self.width, self.height
            ));
        }
        if self.classes.is_empty() {
            return Err("no classes".into());
        }
        if let Some(c) = self.classes.iter().find(|c| c.vector.len() != TW * TH) {
            return Err(format!("class {} has {} values", c.class, c.vector.len()));
        }
        Ok(())
    }

    pub fn save_dir(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let s = serde_json::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(PACK_FILE), s)
    }
}

/// Builds a [`TemplateSet`] from labelled frames.
#[derive(Debug, Default)]
pub struct Learner {
    sums: BTreeMap<String, (usize, Vec<f32>)>,
}

impl Learner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one frame showing `title` on `screen`. False when the frame is
    /// too small to hold the title region.
    pub fn add_frame(&mut self, img: &RgbImage, screen: Screen, title: &str) -> bool {
        let Some((g, _)) = plane(img, screen) else {
            return false;
        };
        self.add_plane(&g, title);
        true
    }

    /// Add one padded ink plane ([`super::plane`]) showing `title`.
    pub fn add_plane(&mut self, g: &Gray, title: &str) {
        let c = centre(g);
        let (n, sum) = self
            .sums
            .entry(title.to_string())
            .or_insert_with(|| (0, vec![0.0; TW * TH]));
        *n += 1;
        for (s, v) in sum.iter_mut().zip(c) {
            *s += v;
        }
    }

    pub fn build(&self) -> TemplateSet {
        TemplateSet {
            recognizer: RECOGNIZER_ID.to_string(),
            width: TW,
            height: TH,
            classes: self
                .sums
                .iter()
                .map(|(class, (n, sum))| ClassTemplate {
                    class: class.clone(),
                    n: *n,
                    vector: sum.iter().map(|s| s / *n as f32).collect(),
                })
                .collect(),
        }
    }
}
