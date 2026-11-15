//! JSONL and image loading shared by the two kinds.

use std::collections::BTreeSet;
use std::path::{Component, Path};

use image::RgbImage;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde_json::{Map, Value};

pub struct Loaded<T> {
    pub rows: usize,
    /// `readable: false` rows the layout still accepted.
    pub unreadable: usize,
    /// Rows the layout refused (rank scale, or queue-title plane).
    pub skipped: usize,
    pub samples: Vec<T>,
    /// Lines printed above the cut summary or the eval report: one per label
    /// file whose rows carried allowed extra keys, and one for queue rows
    /// skipped because they say `visible: false`. Empty when neither.
    pub notes: String,
}

/// What happened to one template class while cutting.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClassFate {
    /// Labels produced it and it was merged with the base by sample count.
    Learned,
    /// The newly cut template was kept and the base one dropped: by
    /// `--replace` or `--replace-all`, or because an observed rank cut
    /// replaces a base template marked `pooled`.
    Replaced,
    /// The labels did not produce it, so the base copy was kept.
    Kept,
    /// Numeric pooling filled it in this cut, not an observed cut, so the
    /// base copy was kept. The base copy may itself be a pooled fill.
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

/// One JSONL object and its 1-based line in the file.
pub type LabelRow = (usize, Map<String, Value>);

/// Read JSONL objects, each with its 1-based line in the file. Blank lines
/// are skipped. A repeated key in one row is an error. Keys in `known` are
/// kept. Keys in `extra` are the ones the label tools also write: they are
/// dropped and named in the returned note, one line for the file. Any other
/// key is an error, so a typo of a known key is refused, not dropped.
pub fn read_jsonl(
    path: &Path,
    known: &[&str],
    extra: &[&str],
) -> Result<(Vec<LabelRow>, String), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut rows = Vec::new();
    let mut ignored = BTreeSet::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let StrictObject(mut obj) =
            serde_json::from_str(line).map_err(|e| format!("{}:{}: {e}", path.display(), n + 1))?;
        if let Some(key) = obj
            .keys()
            .find(|key| !known.contains(&key.as_str()) && !extra.contains(&key.as_str()))
        {
            return Err(format!("{}:{}: unknown key {key:?}", path.display(), n + 1));
        }
        obj.retain(|key, _| {
            if known.contains(&key.as_str()) {
                true
            } else {
                ignored.insert(key.clone());
                false
            }
        });
        rows.push((n + 1, obj));
    }
    let note = if ignored.is_empty() {
        String::new()
    } else {
        let names: Vec<&str> = ignored.iter().map(String::as_str).collect();
        format!("ignored keys in {}: {}\n", path.display(), names.join(", "))
    };
    Ok((rows, note))
}

struct StrictObject(Map<String, Value>);

impl<'de> serde::Deserialize<'de> for StrictObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> Visitor<'de> for Visit {
            type Value = Map<String, Value>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut map = Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if map.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate key {key:?}")));
                    }
                    map.insert(key, access.next_value()?);
                }
                Ok(map)
            }
        }
        Ok(StrictObject(deserializer.deserialize_map(Visit)?))
    }
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

/// The near-pair check's result. No pairs: nothing to print. Pairs and
/// `allow`: one `allowed near pair:` line each, printed above the summary.
/// Pairs without `allow`: the refusal, naming every pair and the flag.
/// `note` is one more line for the refusal, or empty.
pub fn near_outcome(pairs: Vec<String>, allow: bool, note: &str) -> Result<String, String> {
    if pairs.is_empty() {
        return Ok(String::new());
    }
    if allow {
        return Ok(pairs
            .iter()
            .map(|pair| format!("allowed near pair: {pair}\n"))
            .collect());
    }
    let mut msg = pairs.join("\n");
    msg.push_str(
        "\nrefusing classes the reader may not separate; --allow-near writes them anyway and lists each pair in the summary",
    );
    if !note.is_empty() {
        msg.push('\n');
        msg.push_str(note);
    }
    Err(msg)
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
