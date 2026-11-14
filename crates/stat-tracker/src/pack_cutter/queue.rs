//! Queue-title cut and leave-one-session-out report. The session is the
//! parent folder of the frame path, the same split as
//! `bench_labelled_queue_titles`. Learning uses
//! [`Learner::add_frame`](crate::shadow::queue_title::Learner::add_frame).
//! Scoring uses one [`Reader::read_field`](crate::shadow::queue_title::Reader::read_field)
//! per reader.
//!
//! The pass/fail gate is the candidate pack against the baseline pack on
//! every row. Leave-one-session-out is printed and does not gate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::cli::{CutArgs, EvalArgs};
use super::report::{self, EvalDone, Item, Outcome, ScoreRow};
use super::util::{self, ClassFate, ClassNote, CountChange, Loaded};
use crate::shadow::queue_title::{Learner, Reader, Screen, TemplateSet, template_ncc, title_crop};

pub fn cut(args: &CutArgs, out: &Path) -> Result<String, String> {
    super::output::refuse_templates(out, args.base.as_deref(), args.force)?;
    let base = match &args.base {
        Some(dir) => Some(TemplateSet::load_dir(dir).map_err(|e| format!("base pack: {e}"))?),
        None => None,
    };
    let loaded = load(&args.labels, args.frames.as_deref())?;
    let mut learner = Learner::new();
    for sample in &loaded.samples {
        let image = util::open_rgb(&sample.path)?;
        learner.add_frame(&image, sample.screen, &sample.title);
    }
    let learned = learner.build();
    let replace: BTreeSet<String> = args.replace.iter().cloned().collect();
    let (set, classes) = merge(learned, base, &replace, args.replace_all)?;
    reject_near(&set, &classes, args.allow_near)?;
    set.check()
        .map_err(|e| format!("refusing to write an unloadable queue-title pack: {e}"))?;
    let dest = super::output::prepare_templates(out, args.base.as_deref(), args.force)?;
    let bytes = serde_json::to_vec(&set).map_err(|e| e.to_string())?;
    super::output::write_private(&dest, &bytes)?;
    let mut text = loaded.ignored;
    text.push_str(&util::summary(
        "queue-title",
        out,
        loaded.rows,
        loaded.samples.len(),
        loaded.unreadable,
        loaded.skipped,
        &classes,
    ));
    Ok(text)
}

pub fn eval(args: &EvalArgs) -> Result<EvalDone, String> {
    let baseline =
        TemplateSet::load_dir(&args.baseline).map_err(|e| format!("baseline pack: {e}"))?;
    let candidate =
        TemplateSet::load_dir(&args.candidate).map_err(|e| format!("candidate pack: {e}"))?;
    let sha = file_sha(&args.candidate.join("templates.json"))?;
    let dropped = dropped_classes(&baseline, &candidate, &args.allow_drop)?;
    let loaded = load(&args.labels, args.frames.as_deref())?;
    if loaded.samples.is_empty() {
        return Err(format!(
            "empty label set ({} rows, {} layout-skipped)",
            loaded.rows, loaded.skipped
        ));
    }
    let (changed, unverified) = changed_classes(
        &baseline,
        &candidate,
        &loaded.samples,
        &args.allow_unverified,
    )?;
    let base_reader = Reader::new(baseline);
    let pack_reader = Reader::new(candidate);
    let base_names = names_of(&base_reader);
    let mut sessions: Vec<String> = loaded.samples.iter().map(|s| s.session.clone()).collect();
    sessions.sort();
    sessions.dedup();
    let mut session_learners: BTreeMap<String, Learner> = BTreeMap::new();
    for sample in &loaded.samples {
        let image = util::open_rgb(&sample.path)?;
        session_learners
            .entry(sample.session.clone())
            .or_default()
            .add_frame(&image, sample.screen, &sample.title);
    }
    let titles = titles_of(&loaded.samples);
    let mut items: BTreeMap<String, Vec<Item>> = titles
        .iter()
        .map(|title| (title.clone(), Vec::new()))
        .collect();
    let mut folds: BTreeMap<String, BTreeSet<&str>> = titles
        .iter()
        .map(|title| (title.clone(), BTreeSet::new()))
        .collect();
    for session in &sessions {
        let mut learner = Learner::new();
        for (other, part) in &session_learners {
            if other != session {
                learner.merge(part);
            }
        }
        let fold = Reader::new(learner.build());
        for sample in loaded.samples.iter().filter(|s| s.session == *session) {
            let image = util::open_rgb(&sample.path)?;
            folds
                .get_mut(&sample.title)
                .expect("title")
                .insert(sample.session.as_str());
            items.get_mut(&sample.title).expect("title").push(Item {
                frame: sample.label_path.clone(),
                baseline: outcome(&base_reader, &image, sample),
                learned: outcome(&fold, &image, sample),
                packed: outcome(&pack_reader, &image, sample),
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
            labelled: items[title].len(),
            items: items.remove(title).unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let mut done = report::finish(&report::Inputs {
        set_name: &args.set_name,
        frames: loaded.rows,
        scored: loaded.samples.len(),
        skipped: loaded.skipped,
        rows: &rows,
        dropped: &dropped,
        allowed: &args.allow_drop,
        unverified: &unverified,
        changed: &changed,
        class_folds: &[],
        candidate_sha: &sha,
    });
    done.report.insert_str(0, &loaded.ignored);
    Ok(done)
}

fn outcome(reader: &Reader, image: &image::RgbImage, sample: &Sample) -> Outcome {
    match reader.read_field(image, sample.screen).value {
        Some(value) if value == sample.title => Outcome::Correct,
        Some(_) => Outcome::Wrong,
        None => Outcome::Unread,
    }
}

fn names_of(reader: &Reader) -> BTreeSet<String> {
    reader
        .templates
        .classes
        .iter()
        .map(|c| c.class.clone())
        .collect()
}

fn file_sha(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok(super::ustar::sha256_hex(&bytes))
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
    Ok(base
        .difference(&cand)
        .filter(|name| !allow_set.contains(name.as_str()))
        .cloned()
        .collect())
}

fn changed_classes(
    baseline: &TemplateSet,
    candidate: &TemplateSet,
    samples: &[Sample],
    allow: &[String],
) -> Result<(Vec<CountChange>, Vec<String>), String> {
    let base: BTreeMap<&str, &crate::shadow::queue_title::templates::ClassTemplate> = baseline
        .classes
        .iter()
        .map(|c| (c.class.as_str(), c))
        .collect();
    let mut changed = Vec::new();
    let mut unverified = Vec::new();
    let allow_set: BTreeSet<&str> = allow.iter().map(String::as_str).collect();
    for class in &candidate.classes {
        let (before, differs) = match base.get(class.class.as_str()) {
            Some(old) => (old.n, old.n != class.n || old.vector != class.vector),
            None => (0, true),
        };
        if !differs {
            continue;
        }
        changed.push((class.class.clone(), before, class.n));
        let seen = samples.iter().any(|s| s.title == class.class);
        if !seen && !allow_set.contains(class.class.as_str()) {
            unverified.push(class.class.clone());
        }
    }
    let known: BTreeSet<&str> = candidate.classes.iter().map(|c| c.class.as_str()).collect();
    let mut unknown = Vec::new();
    for name in allow {
        if !known.contains(name.as_str()) {
            unknown.push(name.as_str());
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "--allow-unverified is not a candidate class: {}",
            unknown.join(", ")
        ));
    }
    Ok((changed, unverified))
}

fn merge(
    mut learned: TemplateSet,
    base: Option<TemplateSet>,
    replace: &BTreeSet<String>,
    replace_all: bool,
) -> Result<(TemplateSet, Vec<ClassNote>), String> {
    for name in replace {
        if !learned.classes.iter().any(|c| c.class == *name) {
            return Err(format!("--replace {name} was not learned from the labels"));
        }
    }
    let Some(base) = base else {
        let notes = learned
            .classes
            .iter()
            .map(|c| (c.class.clone(), ClassFate::New, 0, c.n))
            .collect();
        return Ok((learned, notes));
    };
    let mut base_map: BTreeMap<String, crate::shadow::queue_title::templates::ClassTemplate> = base
        .classes
        .into_iter()
        .map(|c| (c.class.clone(), c))
        .collect();
    let learned_classes = std::mem::take(&mut learned.classes);
    let mut notes = Vec::new();
    let mut out = Vec::new();
    for learned_t in learned_classes {
        let name = learned_t.class.clone();
        let do_replace = replace_all || replace.contains(&name);
        if let Some(base_t) = base_map.remove(&name) {
            if do_replace {
                notes.push((name, ClassFate::Replaced, base_t.n, learned_t.n));
                out.push(learned_t);
            } else {
                let mixed = mix_queue(&base_t, &learned_t);
                notes.push((name, ClassFate::Learned, base_t.n, mixed.n));
                out.push(mixed);
            }
        } else {
            notes.push((name, ClassFate::New, 0, learned_t.n));
            out.push(learned_t);
        }
    }
    for (name, base_t) in base_map {
        let n = base_t.n;
        notes.push((name, ClassFate::Kept, n, n));
        out.push(base_t);
    }
    notes.sort_by(|a, b| a.0.cmp(&b.0));
    learned.classes = out;
    Ok((learned, notes))
}

fn mix_queue(
    base: &crate::shadow::queue_title::templates::ClassTemplate,
    learned: &crate::shadow::queue_title::templates::ClassTemplate,
) -> crate::shadow::queue_title::templates::ClassTemplate {
    let nb = base.n as f32;
    let nl = learned.n as f32;
    let vector = base
        .vector
        .iter()
        .zip(&learned.vector)
        .map(|(b, l)| (nb * b + nl * l) / (nb + nl))
        .collect();
    crate::shadow::queue_title::templates::ClassTemplate {
        class: base.class.clone(),
        n: base.n.saturating_add(learned.n),
        vector,
    }
}

fn reject_near(set: &TemplateSet, notes: &[ClassNote], allow: bool) -> Result<(), String> {
    if allow {
        return Ok(());
    }
    let hot: BTreeSet<&str> = notes
        .iter()
        .filter(|(_, fate, _, _)| {
            matches!(
                fate,
                ClassFate::New | ClassFate::NewPooled | ClassFate::Learned | ClassFate::Replaced
            )
        })
        .map(|(name, _, _, _)| name.as_str())
        .collect();
    let limit = 1.0 - crate::shadow::queue_title::MIN_MARGIN;
    for i in 0..set.classes.len() {
        for j in (i + 1)..set.classes.len() {
            if !hot.contains(set.classes[i].class.as_str())
                && !hot.contains(set.classes[j].class.as_str())
            {
                continue;
            }
            let dot = template_ncc(&set.classes[i].vector, &set.classes[j].vector);
            if dot > limit {
                return Err(format!(
                    "class {} correlates {dot:.3} with {}, above {limit:.3}",
                    set.classes[i].class, set.classes[j].class
                ));
            }
        }
    }
    Ok(())
}

struct Sample {
    label_path: String,
    session: String,
    screen: Screen,
    title: String,
    path: PathBuf,
}

fn titles_of(samples: &[Sample]) -> Vec<String> {
    let mut titles: Vec<String> = samples.iter().map(|s| s.title.clone()).collect();
    titles.sort();
    titles.dedup();
    titles
}

fn load(labels: &Path, frames: Option<&Path>) -> Result<Loaded<Sample>, String> {
    let (rows_in, ignored) = util::read_jsonl(labels, &["path", "screen", "frame", "title_text"])?;
    let mut rows = Vec::new();
    for (n, obj) in rows_in.iter().enumerate() {
        let title = match obj.get("title_text") {
            Some(Value::String(text)) => text.clone(),
            Some(_) => {
                return Err(format!(
                    "{}:{}: title_text must be a string",
                    labels.display(),
                    n + 1
                ));
            }
            None => {
                return Err(format!(
                    "{}:{}: missing title_text",
                    labels.display(),
                    n + 1
                ));
            }
        };
        if title.is_empty() || title != title.trim() || title.contains("  ") {
            return Err(format!(
                "{}:{}: title_text {title:?} has leading, trailing or doubled spaces",
                labels.display(),
                n + 1
            ));
        }
        let path = match obj.get("path") {
            Some(Value::String(text)) => text.clone(),
            Some(_) => {
                return Err(format!(
                    "{}:{}: path must be a string",
                    labels.display(),
                    n + 1
                ));
            }
            None => {
                return Err(format!("{}:{}: missing path", labels.display(), n + 1));
            }
        };
        let screen = match obj.get("screen") {
            Some(Value::String(text)) => text.clone(),
            Some(_) => {
                return Err(format!(
                    "{}:{}: screen must be a string",
                    labels.display(),
                    n + 1
                ));
            }
            None => {
                return Err(format!("{}:{}: missing screen", labels.display(), n + 1));
            }
        };
        let frame = obj.get("frame").and_then(Value::as_str).unwrap_or("");
        let screen = Screen::parse(&screen)
            .ok_or_else(|| format!("queue title frame {frame} has unknown screen {screen:?}"))?;
        rows.push((path, screen, title));
    }
    let mut samples = Vec::new();
    let mut skipped = 0usize;
    for (label_path, screen, title) in &rows {
        let path = resolve_frame(frames, label_path)?;
        let (width, height) = image::image_dimensions(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        if title_crop(width, height, *screen).is_none() {
            skipped += 1;
            continue;
        }
        let session = session_of(&path)?;
        samples.push(Sample {
            label_path: label_path.clone(),
            session,
            screen: *screen,
            title: title.clone(),
            path,
        });
    }
    Ok(Loaded {
        rows: rows.len(),
        unreadable: 0,
        skipped,
        samples,
        ignored,
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
