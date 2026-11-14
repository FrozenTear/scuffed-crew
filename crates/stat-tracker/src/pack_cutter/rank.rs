//! Rank cut and leave-one-day-out eval. The day is `game[..8]`, the same
//! split as `bench_labelled_rank_screens`. Learning uses
//! [`Learner::add_frame`](crate::shadow::rank::Learner::add_frame). Scoring
//! uses [`Reader::read`](crate::shadow::rank::Reader::read).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use super::cli::{CutArgs, EvalArgs};
use super::report::{self, Counts, EvalDone, Outcome, ScoreRow};
use super::util::{self, Loaded};
use crate::shadow::rank::layout::Scale;
use crate::shadow::rank::{Labels, Learner, RankRead, Reader, Role, TemplateSet};

pub fn cut(args: &CutArgs, out: &Path) -> Result<String, String> {
    let loaded = load(&args.labels, &args.frames, args.role_labels.as_deref())?;
    let mut learner = Learner::new();
    let mut learned = 0usize;
    for sample in &loaded.samples {
        if sample.readable {
            learner.add_frame(&sample.image, &sample.labels);
            learned += 1;
        }
    }
    let set = learner.build();
    set.check()
        .map_err(|e| format!("refusing to write an unloadable rank pack: {e}"))?;
    super::output::private_dir(out)?;
    set.save_dir(out)
        .map_err(|e| format!("writing {}: {e}", out.display()))?;
    Ok(util::summary(
        "rank",
        out,
        loaded.rows,
        learned,
        loaded.skipped,
    ))
}

pub fn eval(args: &EvalArgs) -> Result<EvalDone, String> {
    let baseline =
        TemplateSet::load_dir(&args.baseline).map_err(|e| format!("baseline pack: {e}"))?;
    let candidate =
        TemplateSet::load_dir(&args.candidate).map_err(|e| format!("candidate pack: {e}"))?;
    let loaded = load(&args.labels, &args.frames, args.role_labels.as_deref())?;
    if loaded.samples.is_empty() {
        return Err(format!(
            "empty label set ({} rows, {} skipped)",
            loaded.rows, loaded.skipped
        ));
    }
    let base_reader = Reader::new(baseline);
    let pack_reader = Reader::new(candidate);
    let mut days: Vec<String> = loaded.samples.iter().map(|s| s.day.clone()).collect();
    days.sort();
    days.dedup();

    let mut base = Bag::new();
    let mut learned = Bag::new();
    let mut packed = Bag::new();
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
            tally(&base_reader, sample, &mut base);
            tally(&fold, sample, &mut learned);
            tally(&pack_reader, sample, &mut packed);
        }
    }

    let mut rows = Vec::new();
    for name in Labels::NAMES {
        rows.push(ScoreRow {
            label: format!("field {name}"),
            baseline: base.fields[name],
            learned: learned.fields[name],
            packed: packed.fields[name],
            gated: true,
        });
    }
    rows.push(ScoreRow {
        label: "screen gate".into(),
        baseline: base.gate,
        learned: learned.gate,
        packed: packed.gate,
        gated: true,
    });
    Ok(report::finish(
        &args.set_name,
        loaded.rows,
        loaded.samples.len(),
        loaded.skipped,
        &rows,
    ))
}

struct Sample {
    day: String,
    readable: bool,
    labels: Labels,
    image: image::RgbImage,
}

struct Bag {
    fields: BTreeMap<&'static str, Counts>,
    gate: Counts,
}

impl Bag {
    fn new() -> Self {
        let mut fields = BTreeMap::new();
        for name in Labels::NAMES {
            fields.insert(name, Counts::default());
        }
        Self {
            fields,
            gate: Counts::default(),
        }
    }
}

fn tally(reader: &Reader, sample: &Sample, bag: &mut Bag) {
    let read = reader.read(&sample.image).ok();
    bag.gate.add(if read.is_some() == sample.readable {
        Outcome::Correct
    } else {
        Outcome::Wrong
    });
    if !sample.readable {
        return;
    }
    let got = read.as_ref().map(RankRead::values).unwrap_or_default();
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
        bag.fields.entry(name).or_default().add(outcome);
    }
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
    frames: &Path,
    role_labels: Option<&Path>,
) -> Result<Loaded<Sample>, String> {
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
        samples.push(Sample {
            day,
            readable: row.readable,
            labels: row.labels.clone(),
            image,
        });
    }
    Ok(Loaded {
        rows: rows.len(),
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
