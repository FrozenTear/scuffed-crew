//! JSONL and image loading shared by the two kinds.

use std::path::{Component, Path};

use image::RgbImage;
use serde::de::DeserializeOwned;

pub struct Loaded<T> {
    pub rows: usize,
    pub skipped: usize,
    pub samples: Vec<T>,
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

pub fn summary(kind: &str, out: &Path, rows: usize, learned: usize, skipped: usize) -> String {
    format!(
        "cut {kind}\nrows: {rows}\nlearned: {learned}\nskipped: {skipped}\nwrote: {}\n",
        out.join("templates.json").display()
    )
}
