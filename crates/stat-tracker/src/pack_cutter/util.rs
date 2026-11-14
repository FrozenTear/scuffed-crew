//! JSONL and image loading shared by the two kinds.

use std::path::{Component, Path};

use image::RgbImage;
use serde::de::DeserializeOwned;

pub struct Loaded<T> {
    pub rows: usize,
    /// `readable: false` rows the layout still accepted.
    pub unreadable: usize,
    /// Rows the layout refused (rank scale, or queue-title plane).
    pub skipped: usize,
    pub samples: Vec<T>,
}

/// What happened to one template class while cutting.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClassFate {
    /// The labels produced this class, and the base pack had it too.
    Learned,
    /// The labels did not produce this class, so the base copy was kept.
    Kept,
    /// The labels produced this class and the base pack did not have it.
    New,
}

pub fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row =
            serde_json::from_str(line).map_err(|e| format!("{}:{}: {e}", path.display(), n + 1))?;
        out.push(row);
    }
    Ok(out)
}

pub fn open_rgb(path: &Path) -> Result<RgbImage, String> {
    image::open(path)
        .map(|img| img.to_rgb8())
        .map_err(|e| format!("reading {}: {e}", path.display()))
}

/// `rel` joined onto `root`, with no absolute path and no `..`.
pub fn under_root(root: &Path, rel: &Path) -> Result<std::path::PathBuf, String> {
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(c, Component::ParentDir | Component::Prefix(_))
                || matches!(c, Component::Normal(s) if s.to_string_lossy().contains('\\'))
        })
    {
        return Err(format!("{} is outside the frames directory", rel.display()));
    }
    Ok(root.join(rel))
}

pub fn summary(
    kind: &str,
    out: &Path,
    rows: usize,
    learned: usize,
    unreadable: usize,
    skipped: usize,
    classes: &[(String, ClassFate)],
) -> String {
    let mut text = format!(
        "cut {kind}\nrows: {rows}\nlearned: {learned}\nunreadable: {unreadable}\nlayout-skipped: {skipped}\n"
    );
    let mut n_learned = 0usize;
    let mut n_kept = 0usize;
    let mut n_new = 0usize;
    for (name, fate) in classes {
        let word = match fate {
            ClassFate::Learned => {
                n_learned += 1;
                "learned"
            }
            ClassFate::Kept => {
                n_kept += 1;
                "kept from base"
            }
            ClassFate::New => {
                n_new += 1;
                "new"
            }
        };
        text.push_str(&format!("class {name}: {word}\n"));
    }
    text.push_str(&format!(
        "classes: learned {n_learned}, kept from base {n_kept}, new {n_new}\nwrote: {}\n",
        out.join("templates.json").display()
    ));
    text
}
