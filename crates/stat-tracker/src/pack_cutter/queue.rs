//! Queue-title cut and leave-one-session-out report. The session is the
//! parent folder of the frame path, the same split as
//! `bench_labelled_queue_titles`. Learning uses
//! [`Learner::add_frame`](crate::shadow::queue_title::Learner::add_frame).
//! Scoring uses [`Reader::read_field`](crate::shadow::queue_title::Reader::read_field).
//!
//! The pass/fail gate is the candidate pack against the baseline pack.
//! Leave-one-session-out is printed and does not gate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::cli::{CutArgs, EvalArgs};
use super::report::{self, EvalDone, Item, Outcome, ScoreRow};
use super::util::{self, ClassFate, Loaded};
use crate::shadow::queue_title::{Learner, Reader, Screen, TemplateSet, plane};

pub fn cut(args: &CutArgs, out: &Path) -> Result<String, String> {
    let loaded = load(&args.labels, args.frames.as_deref())?;
    let mut learner = Learner::new();
    for sample in &loaded.samples {
        learner.add_frame(&sample.image, sample.screen, &sample.title);
    }
    let learned = learner.build();
    let base = match &args.base {
        Some(dir) => Some(TemplateSet::load_dir(dir).map_err(|e| format!("base pack: {e}"))?),
        None => None,
    };
    let (set, classes) = merge(learned, base)?;
    set.check()
        .map_err(|e| format!("refusing to write an unloadable queue-title pack: {e}"))?;
    super::output::private_dir(out)?;
    set.save_dir(out)
        .map_err(|e| format!("writing {}: {e}", out.display()))?;
    Ok(util::summary(
        "queue-title",
        out,
        loaded.rows,
        loaded.samples.len(),
        loaded.unreadable,
        loaded.skipped,
        &classes,
    ))
}

pub fn eval(args: &EvalArgs) -> Result<EvalDone, String> {
    let baseline =
        TemplateSet::load_dir(&args.baseline).map_err(|e| format!("baseline pack: {e}"))?;
    let candidate =
        TemplateSet::load_dir(&args.candidate).map_err(|e| format!("candidate pack: {e}"))?;
    let dropped = dropped_classes(&baseline, &candidate, &args.allow_drop)?;
    let loaded = load(&args.labels, args.frames.as_deref())?;
    if loaded.samples.is_empty() {
        return Err(format!(
            "empty label set ({} rows, {} layout-skipped)",
            loaded.rows, loaded.skipped
        ));
    }
    let base_reader = Reader::new(baseline);
    let pack_reader = Reader::new(candidate);
    let mut sessions: Vec<&str> = loaded.samples.iter().map(|s| s.session.as_str()).collect();
    sessions.sort_unstable();
    sessions.dedup();

    let titles = titles_of(&loaded.samples);
    let mut items: BTreeMap<String, Vec<Item>> = bag(&titles);
    let mut folds: BTreeMap<String, BTreeSet<&str>> = titles
        .iter()
        .map(|title| (title.clone(), BTreeSet::new()))
        .collect();
    let base_names = class_names_of_reader(&base_reader);
    for session in sessions {
        let mut learner = Learner::new();
        for sample in loaded.samples.iter().filter(|s| s.session != session) {
            learner.add_frame(&sample.image, sample.screen, &sample.title);
        }
        let fold = Reader::new(learner.build());
        for sample in loaded.samples.iter().filter(|s| s.session == session) {
            folds
                .get_mut(&sample.title)
                .expect("title")
                .insert(sample.session.as_str());
            items.get_mut(&sample.title).expect("title").push(Item {
                baseline: outcome(&base_reader, sample),
                learned: outcome(&fold, sample),
                packed: outcome(&pack_reader, sample),
            });
        }
    }

    let rows = titles
        .iter()
        .map(|title| ScoreRow {
            label: if base_names.contains(title) {
                format!("class {title}")
            } else {
                format!("class {title} (not in the baseline pack)")
            },
            folds: folds[title].len(),
            items: items.remove(title).unwrap_or_default(),
            gated: base_names.contains(title),
        })
        .collect::<Vec<_>>();
    Ok(report::finish(
        &args.set_name,
        loaded.rows,
        loaded.samples.len(),
        loaded.skipped,
        &rows,
        &dropped,
    ))
}

fn class_names_of_reader(reader: &Reader) -> BTreeSet<String> {
    reader
        .templates
        .classes
        .iter()
        .map(|class| class.class.clone())
        .collect()
}

fn outcome(reader: &Reader, sample: &Sample) -> Outcome {
    match reader.read_field(&sample.image, sample.screen).value {
        Some(value) if value == sample.title => Outcome::Correct,
        Some(_) => Outcome::Wrong,
        None => Outcome::Unread,
    }
}

fn bag(titles: &[String]) -> BTreeMap<String, Vec<Item>> {
    titles
        .iter()
        .map(|title| (title.clone(), Vec::new()))
        .collect()
}

fn titles_of(samples: &[Sample]) -> Vec<String> {
    let mut titles: Vec<String> = samples.iter().map(|s| s.title.clone()).collect();
    titles.sort();
    titles.dedup();
    titles
}

fn dropped_classes(
    baseline: &TemplateSet,
    candidate: &TemplateSet,
    allow: &[String],
) -> Result<Vec<String>, String> {
    let base: BTreeSet<String> = baseline.classes.iter().map(|c| c.class.clone()).collect();
    let cand: BTreeSet<String> = candidate.classes.iter().map(|c| c.class.clone()).collect();
    let mut unknown = Vec::new();
    for name in allow {
        if !base.contains(name) {
            unknown.push(name.as_str());
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "--allow-drop is not a baseline class: {}",
            unknown.join(", ")
        ));
    }
    let allow_set: BTreeSet<&str> = allow.iter().map(String::as_str).collect();
    let mut dropped: Vec<String> = base
        .difference(&cand)
        .filter(|name| !allow_set.contains(name.as_str()))
        .cloned()
        .collect();
    dropped.sort();
    Ok(dropped)
}

fn merge(
    mut learned: TemplateSet,
    base: Option<TemplateSet>,
) -> Result<(TemplateSet, Vec<(String, ClassFate)>), String> {
    let mut notes = Vec::new();
    let base_names: BTreeSet<String> = base
        .as_ref()
        .map(|set| set.classes.iter().map(|c| c.class.clone()).collect())
        .unwrap_or_default();
    for class in &learned.classes {
        let fate = if base_names.contains(&class.class) {
            ClassFate::Learned
        } else {
            ClassFate::New
        };
        notes.push((class.class.clone(), fate));
    }
    if let Some(base) = base {
        if base.width != learned.width || base.height != learned.height {
            return Err(format!(
                "base pack template size {}x{} does not match this build {}x{}",
                base.width, base.height, learned.width, learned.height
            ));
        }
        let have: BTreeSet<String> = learned.classes.iter().map(|c| c.class.clone()).collect();
        for class in base.classes {
            if have.contains(&class.class) {
                continue;
            }
            notes.push((class.class.clone(), ClassFate::Kept));
            learned.classes.push(class);
        }
    }
    notes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((learned, notes))
}

struct Sample {
    session: String,
    screen: Screen,
    title: String,
    image: image::RgbImage,
}

#[derive(Deserialize)]
struct Row {
    path: String,
    screen: String,
    frame: String,
    title_text: String,
}

fn load(labels: &Path, frames: Option<&Path>) -> Result<Loaded<Sample>, String> {
    let rows: Vec<Row> = util::read_jsonl(labels)?;
    let mut samples = Vec::new();
    let mut skipped = 0usize;
    for row in &rows {
        let screen = Screen::parse(&row.screen).ok_or_else(|| {
            format!(
                "queue title frame {} has unknown screen {:?}",
                row.frame, row.screen
            )
        })?;
        let path = resolve_frame(frames, &row.path)?;
        let image = util::open_rgb(&path).map_err(|e| format!("{e} (frame {})", row.frame))?;
        if plane(&image, screen).is_none() {
            skipped += 1;
            continue;
        }
        let session = session_of(&path)?;
        samples.push(Sample {
            session,
            screen,
            title: row.title_text.clone(),
            image,
        });
    }
    Ok(Loaded {
        rows: rows.len(),
        unreadable: 0,
        skipped,
        samples,
    })
}

fn resolve_frame(frames: Option<&Path>, raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let Some(frames) = frames else {
            return Err(format!(
                "missing --frames ({raw} is relative; queue-title needs --frames unless every path is absolute)"
            ));
        };
        util::under_root(frames, path)
    }
}

fn session_of(path: &Path) -> Result<String, String> {
    let Some(name) = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
    else {
        return Err(format!(
            "{} has no session folder (put the frame in a session directory)",
            path.display()
        ));
    };
    if name.is_empty() || name == "." || name == ".." {
        return Err(format!(
            "{} has no session folder (put the frame in a session directory)",
            path.display()
        ));
    }
    Ok(name.to_string())
}
