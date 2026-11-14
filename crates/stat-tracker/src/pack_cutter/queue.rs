//! Queue-title cut and leave-one-session-out eval. The session is the
//! parent folder of the frame path, the same split as
//! `bench_labelled_queue_titles`. Learning uses
//! [`Learner::add_frame`](crate::shadow::queue_title::Learner::add_frame).
//! Scoring uses [`Reader::read_field`](crate::shadow::queue_title::Reader::read_field).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::cli::{CutArgs, EvalArgs};
use super::report::{self, Counts, EvalDone, Outcome, ScoreRow};
use super::util::{self, Loaded};
use crate::shadow::queue_title::{Learner, Reader, Screen, TemplateSet, plane};

pub fn cut(args: &CutArgs, out: &Path) -> Result<String, String> {
    let loaded = load(&args.labels, &args.frames)?;
    let mut learner = Learner::new();
    for sample in &loaded.samples {
        learner.add_frame(&sample.image, sample.screen, &sample.title);
    }
    let set = learner.build();
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
        loaded.skipped,
    ))
}

pub fn eval(args: &EvalArgs) -> Result<EvalDone, String> {
    let baseline =
        TemplateSet::load_dir(&args.baseline).map_err(|e| format!("baseline pack: {e}"))?;
    let candidate =
        TemplateSet::load_dir(&args.candidate).map_err(|e| format!("candidate pack: {e}"))?;
    let known: BTreeSet<String> = baseline.classes.iter().map(|c| c.class.clone()).collect();
    let loaded = load(&args.labels, &args.frames)?;
    if loaded.samples.is_empty() {
        return Err(format!(
            "empty label set ({} rows, {} skipped)",
            loaded.rows, loaded.skipped
        ));
    }
    let base_reader = Reader::new(baseline);
    let pack_reader = Reader::new(candidate);
    let mut sessions: Vec<&str> = loaded.samples.iter().map(|s| s.session.as_str()).collect();
    sessions.sort_unstable();
    sessions.dedup();

    let titles = titles_of(&loaded.samples);
    let mut base = bag(&titles);
    let mut learned = bag(&titles);
    let mut packed = bag(&titles);
    for session in sessions {
        let mut learner = Learner::new();
        for sample in loaded.samples.iter().filter(|s| s.session != session) {
            learner.add_frame(&sample.image, sample.screen, &sample.title);
        }
        let fold = Reader::new(learner.build());
        for sample in loaded.samples.iter().filter(|s| s.session == session) {
            tally(&base_reader, sample, &mut base);
            tally(&fold, sample, &mut learned);
            tally(&pack_reader, sample, &mut packed);
        }
    }

    let rows = titles
        .iter()
        .map(|title| ScoreRow {
            label: if known.contains(title) {
                format!("class {title}")
            } else {
                format!("class {title} (not in the baseline pack)")
            },
            baseline: base[title.as_str()],
            learned: learned[title.as_str()],
            packed: packed[title.as_str()],
            gated: known.contains(title),
        })
        .collect::<Vec<_>>();
    Ok(report::finish(
        &args.set_name,
        loaded.rows,
        loaded.samples.len(),
        loaded.skipped,
        &rows,
    ))
}

struct Sample {
    session: String,
    screen: Screen,
    title: String,
    image: image::RgbImage,
}

fn bag(titles: &[String]) -> BTreeMap<String, Counts> {
    titles
        .iter()
        .map(|title| (title.clone(), Counts::default()))
        .collect()
}

fn titles_of(samples: &[Sample]) -> Vec<String> {
    let mut titles: Vec<String> = samples.iter().map(|s| s.title.clone()).collect();
    titles.sort();
    titles.dedup();
    titles
}

fn tally(reader: &Reader, sample: &Sample, bag: &mut BTreeMap<String, Counts>) {
    let outcome = match reader.read_field(&sample.image, sample.screen).value {
        Some(value) if value == sample.title => Outcome::Correct,
        Some(_) => Outcome::Wrong,
        None => Outcome::Unread,
    };
    bag.get_mut(&sample.title)
        .expect("title inserted before scoring")
        .add(outcome);
}

#[derive(Deserialize)]
struct Row {
    path: String,
    screen: String,
    frame: String,
    title_text: String,
}

fn load(labels: &Path, frames: &Path) -> Result<Loaded<Sample>, String> {
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
        skipped,
        samples,
    })
}

fn resolve_frame(frames: &Path, raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
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
