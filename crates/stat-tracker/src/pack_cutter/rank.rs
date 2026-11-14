//! Rank cut and leave-one-day-out report. The day is `game[..8]`, the same
//! split as `bench_labelled_rank_screens`. Learning uses
//! [`Learner::add_frame`](crate::shadow::rank::Learner::add_frame). Scoring
//! uses [`Reader::read`](crate::shadow::rank::Reader::read).
//!
//! The pass/fail gate is the candidate pack against the baseline pack.
//! Leave-one-day-out is printed and does not gate, so a class that exists
//! on only one day can still pass when the packs agree.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use super::cli::{CutArgs, EvalArgs};
use super::report::{self, EvalDone, Item, Outcome, ScoreRow};
use super::util::{self, ClassFate, Loaded};
use crate::shadow::rank::layout::Scale;
use crate::shadow::rank::{Family, Labels, Learner, RankRead, Reader, Role, TemplateSet};

pub fn cut(args: &CutArgs, out: &Path) -> Result<String, String> {
    let frames = args.frames.as_deref().ok_or("missing --frames")?;
    let loaded = load(&args.labels, Some(frames), args.role_labels.as_deref())?;
    let mut learner = Learner::new();
    let mut learned_rows = 0usize;
    for sample in &loaded.samples {
        if sample.readable {
            learner.add_frame(&sample.image, &sample.labels);
            learned_rows += 1;
        }
    }
    let learned = learner.build();
    let base = match &args.base {
        Some(dir) => Some(TemplateSet::load_dir(dir).map_err(|e| format!("base pack: {e}"))?),
        None => None,
    };
    let (set, classes) = merge(learned, base)?;
    set.check()
        .map_err(|e| format!("refusing to write an unloadable rank pack: {e}"))?;
    super::output::private_dir(out)?;
    set.save_dir(out)
        .map_err(|e| format!("writing {}: {e}", out.display()))?;
    Ok(util::summary(
        "rank",
        out,
        loaded.rows,
        learned_rows,
        loaded.unreadable,
        loaded.skipped,
        &classes,
    ))
}

pub fn eval(args: &EvalArgs) -> Result<EvalDone, String> {
    let frames = args.frames.as_deref().ok_or("missing --frames")?;
    let baseline =
        TemplateSet::load_dir(&args.baseline).map_err(|e| format!("baseline pack: {e}"))?;
    let candidate =
        TemplateSet::load_dir(&args.candidate).map_err(|e| format!("candidate pack: {e}"))?;
    let dropped = dropped_classes(&baseline, &candidate, &args.allow_drop)?;
    let loaded = load(&args.labels, Some(frames), args.role_labels.as_deref())?;
    if loaded.samples.is_empty() {
        return Err(format!(
            "empty label set ({} rows, {} layout-skipped)",
            loaded.rows, loaded.skipped
        ));
    }
    let base_reader = Reader::new(baseline);
    let pack_reader = Reader::new(candidate);
    let mut days: Vec<String> = loaded.samples.iter().map(|s| s.day.clone()).collect();
    days.sort();
    days.dedup();

    let mut bag = Bag::new();
    for day in &days {
        let mut learner = Learner::new();
        for sample in loaded
            .samples
            .iter()
            .filter(|s| s.readable && s.day != *day)
        {
            learner.add_frame(&sample.image, &sample.labels);
        }
        let fold = Reader::new(learner.build());
        for sample in loaded.samples.iter().filter(|s| s.day == *day) {
            record(&base_reader, &fold, &pack_reader, sample, &mut bag);
        }
    }

    let mut rows = Vec::new();
    for name in Labels::NAMES {
        rows.push(ScoreRow {
            label: format!("field {name}"),
            folds: bag.folds[name].len(),
            items: bag.fields.remove(name).unwrap_or_default(),
            gated: true,
        });
    }
    rows.push(ScoreRow {
        label: "screen gate".into(),
        folds: bag.gate_days.len(),
        items: bag.gate,
        gated: true,
    });
    let mut done = report::finish(
        &args.set_name,
        loaded.rows,
        loaded.samples.len(),
        loaded.skipped,
        &rows,
        &dropped,
    );
    done.report.push_str(&class_fold_report(&loaded.samples));
    Ok(done)
}

fn record(base: &Reader, learned: &Reader, packed: &Reader, sample: &Sample, bag: &mut Bag) {
    let item = Item {
        baseline: gate_outcome(base, sample),
        learned: gate_outcome(learned, sample),
        packed: gate_outcome(packed, sample),
    };
    bag.gate.push(item);
    bag.gate_days.insert(sample.day.clone());
    if !sample.readable {
        return;
    }
    let base_fields = field_outcomes(base, sample);
    let learned_fields = field_outcomes(learned, sample);
    let packed_fields = field_outcomes(packed, sample);
    for (name, want) in sample.labels.values() {
        if want.is_some() {
            bag.folds
                .get_mut(name)
                .expect("field")
                .insert(sample.day.clone());
        }
        bag.fields.get_mut(name).expect("field").push(Item {
            baseline: base_fields[name],
            learned: learned_fields[name],
            packed: packed_fields[name],
        });
    }
}

fn gate_outcome(reader: &Reader, sample: &Sample) -> Outcome {
    if reader.read(&sample.image).is_ok() == sample.readable {
        Outcome::Correct
    } else {
        Outcome::Wrong
    }
}

fn field_outcomes(reader: &Reader, sample: &Sample) -> BTreeMap<&'static str, Outcome> {
    let read = reader.read(&sample.image).ok();
    let got = read.as_ref().map(RankRead::values).unwrap_or_default();
    let mut out = BTreeMap::new();
    for (name, want) in sample.labels.values() {
        let value = got
            .iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, v)| v.clone());
        let outcome = match value {
            Some(v) if want.as_ref() == Some(&v) => Outcome::Correct,
            Some(_) => Outcome::Wrong,
            None => Outcome::Unread,
        };
        out.insert(name, outcome);
    }
    out
}

struct Sample {
    day: String,
    readable: bool,
    labels: Labels,
    image: image::RgbImage,
}

struct Bag {
    fields: BTreeMap<&'static str, Vec<Item>>,
    folds: BTreeMap<&'static str, BTreeSet<String>>,
    gate: Vec<Item>,
    gate_days: BTreeSet<String>,
}

impl Bag {
    fn new() -> Self {
        let mut fields = BTreeMap::new();
        let mut folds = BTreeMap::new();
        for name in Labels::NAMES {
            fields.insert(name, Vec::new());
            folds.insert(name, BTreeSet::new());
        }
        Self {
            fields,
            folds,
            gate: Vec::new(),
            gate_days: BTreeSet::new(),
        }
    }
}

/// Classes in `base` that `candidate` does not have, minus `--allow-drop`.
fn dropped_classes(
    baseline: &TemplateSet,
    candidate: &TemplateSet,
    allow: &[String],
) -> Result<Vec<String>, String> {
    let base = class_keys(baseline);
    let cand = class_keys(candidate);
    let allow_set: BTreeSet<&str> = allow.iter().map(String::as_str).collect();
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
    let mut dropped: Vec<String> = base
        .difference(&cand)
        .filter(|name| !allow_set.contains(name.as_str()))
        .cloned()
        .collect();
    dropped.sort();
    Ok(dropped)
}

fn class_keys(set: &TemplateSet) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for (family, templates) in &set.families {
        for template in templates {
            keys.insert(class_key(*family, &template.class));
        }
    }
    keys
}

fn class_key(family: Family, class: &str) -> String {
    format!("{}/{}", family_name(family), class)
}

fn family_name(family: Family) -> &'static str {
    match family {
        Family::Gate => "gate",
        Family::Pct => "pct",
        Family::Higher => "higher",
        Family::Pill => "pill",
        Family::Sr => "sr",
        Family::Change => "change",
        Family::Tag => "tag",
        Family::Banner => "banner",
        Family::Tier => "tier",
        Family::Division => "division",
        Family::Emblem => "emblem",
        Family::Role => "role",
    }
}

/// Re-learned classes stay. Classes the labels did not produce are copied
/// from `base` unchanged.
fn merge(
    mut learned: TemplateSet,
    base: Option<TemplateSet>,
) -> Result<(TemplateSet, Vec<(String, ClassFate)>), String> {
    let mut notes: Vec<(String, ClassFate)> = Vec::new();
    let base_keys = base.as_ref().map(class_keys).unwrap_or_default();
    for (family, templates) in &learned.families {
        for template in templates {
            let key = class_key(*family, &template.class);
            let fate = if base_keys.contains(&key) {
                ClassFate::Learned
            } else {
                ClassFate::New
            };
            notes.push((key, fate));
        }
    }
    if let Some(base) = base {
        let have = class_keys(&learned);
        for (family, templates) in base.families {
            for template in templates {
                let key = class_key(family, &template.class);
                if have.contains(&key) {
                    continue;
                }
                learned.families.entry(family).or_default().push(template);
                notes.push((key, ClassFate::Kept));
            }
        }
    }
    notes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((learned, notes))
}

/// Distinct days that produced each template class, from a one-day learn.
/// A class that shows up on a single day is visible here. Leave-one-day-out
/// cannot learn it, which is why that score does not gate.
fn class_fold_report(samples: &[Sample]) -> String {
    let mut by_day: BTreeMap<&str, Vec<&Sample>> = BTreeMap::new();
    for sample in samples.iter().filter(|s| s.readable) {
        by_day.entry(sample.day.as_str()).or_default().push(sample);
    }
    let mut folds: BTreeMap<String, usize> = BTreeMap::new();
    for group in by_day.values() {
        let mut learner = Learner::new();
        for sample in group {
            learner.add_frame(&sample.image, &sample.labels);
        }
        let set = learner.build();
        let mut seen = BTreeSet::new();
        for (family, templates) in &set.families {
            for template in templates {
                seen.insert(class_key(*family, &template.class));
            }
        }
        for key in seen {
            *folds.entry(key).or_default() += 1;
        }
    }
    let mut text = String::new();
    for (key, n) in folds {
        text.push_str(&format!("template class {key}\nfolds: {n}\n"));
    }
    text
}

#[derive(Deserialize)]
struct Row {
    game: String,
    file: String,
    readable: bool,
    #[serde(flatten)]
    labels: Labels,
}

#[derive(Deserialize)]
struct RoleRow {
    game: String,
    role: Option<Role>,
}

fn load(
    labels: &Path,
    frames: Option<&Path>,
    role_labels: Option<&Path>,
) -> Result<Loaded<Sample>, String> {
    let frames = frames.ok_or("missing --frames")?;
    let mut rows: Vec<Row> = util::read_jsonl(labels)?;
    if let Some(path) = role_labels {
        let roles: Vec<RoleRow> = util::read_jsonl(path)?;
        for row in &mut rows {
            if let Some(role) = roles.iter().rev().find(|r| r.game == row.game) {
                row.labels.role = role.role;
            }
        }
    }
    let mut samples = Vec::new();
    let mut skipped = 0usize;
    let mut unreadable = 0usize;
    for row in &rows {
        let day = day_of(&row.game)?;
        reject_name("game", &row.game)?;
        reject_name("file", &row.file)?;
        let rel = Path::new(&row.game).join(&row.file);
        let path = util::under_root(frames, &rel)?;
        let image = util::open_rgb(&path)?;
        if Scale::from_img(&image).is_none() {
            skipped += 1;
            continue;
        }
        if !row.readable {
            unreadable += 1;
        }
        samples.push(Sample {
            day,
            readable: row.readable,
            labels: row.labels.clone(),
            image,
        });
    }
    Ok(Loaded {
        rows: rows.len(),
        unreadable,
        skipped,
        samples,
    })
}

fn day_of(game: &str) -> Result<String, String> {
    if game.is_ascii() && game.len() >= 8 {
        Ok(game[..8].to_string())
    } else {
        Err(format!(
            "rank game id {game:?} needs at least 8 ASCII bytes; the day is the first 8"
        ))
    }
}

fn reject_name(what: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value == "."
        || value == ".."
    {
        Err(format!(
            "rank {what} {value:?} is not a single path component"
        ))
    } else {
        Ok(())
    }
}
