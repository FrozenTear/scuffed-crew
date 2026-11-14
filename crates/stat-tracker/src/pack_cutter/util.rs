//! JSONL and image loading shared by the two kinds.

use std::path::{Component, Path};

use image::RgbImage;

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
    /// Labels produced it and it was merged with the base by sample count.
    Learned,
    /// `--replace` kept the newly cut template and dropped the base one.
    Replaced,
    /// The labels did not produce it, so the base copy was kept.
    Kept,
    /// Numeric pooling invented it, but the base already had a real cut.
    KeptPooled,
    /// The labels produced it and the base pack did not have it.
    New,
    /// Numeric pooling filled it, and the base pack did not have it.
    NewPooled,
}

/// One cut-summary line: class, what happened to it, base `n`, written `n`.
pub type ClassNote = (String, ClassFate, usize, usize);

/// A class whose template changed: name, baseline `n`, candidate `n`.
pub type CountChange = (String, usize, usize);

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
    classes: &[ClassNote],
) -> String {
    let mut text = format!(
        "cut {kind}\nrows: {rows}\nlearned: {learned}\nunreadable: {unreadable}\nlayout-skipped: {skipped}\n"
    );
    let mut n_learned = 0usize;
    let mut n_kept = 0usize;
    let mut n_new = 0usize;
    for (name, fate, n_base, n_new_n) in classes {
        let word = match fate {
            ClassFate::Learned => {
                n_learned += 1;
                "learned"
            }
            ClassFate::Replaced => {
                n_learned += 1;
                "replaced"
            }
            ClassFate::Kept | ClassFate::KeptPooled => {
                n_kept += 1;
                if *fate == ClassFate::KeptPooled {
                    "kept from base (pooled)"
                } else {
                    "kept from base"
                }
            }
            ClassFate::New => {
                n_new += 1;
                "new"
            }
            ClassFate::NewPooled => {
                n_new += 1;
                "new (pooled)"
            }
        };
        text.push_str(&format!("class {name}: {word} n {n_base} -> {n_new_n}\n"));
    }
    text.push_str(&format!(
        "classes: learned {n_learned}, kept from base {n_kept}, new {n_new}\nwrote: {}\n",
        out.join("templates.json").display()
    ));
    text
}
