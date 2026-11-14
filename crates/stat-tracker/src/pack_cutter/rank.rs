//! Rank cut and leave-one-day-out report. The day is `game[..8]`, the same
//! split as `bench_labelled_rank_screens`. Learning uses
//! [`Learner::add_frame`](crate::shadow::rank::Learner::add_frame). Scoring
//! uses one [`Reader::read`](crate::shadow::rank::Reader::read) per reader.
//!
//! The pass/fail gate is the candidate pack against the baseline pack.
//! Leave-one-day-out is printed and does not gate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::cli::{CutArgs, EvalArgs};
use super::report::{self, EvalDone, Item, Outcome, ScoreRow};
use super::util::{self, ClassFate, ClassNote, CountChange, Loaded};
use crate::shadow::rank::ink::unit;
use crate::shadow::rank::layout::Scale;
use crate::shadow::rank::{Family, Labels, Learner, RankRead, Reader, Role, TemplateSet};

pub fn cut(args: &CutArgs, out: &Path) -> Result<String, String> {
    super::output::refuse_templates(out, args.base.as_deref(), args.force)?;
    let base = match &args.base {
        Some(dir) => Some(TemplateSet::load_dir(dir).map_err(|e| format!("base pack: {e}"))?),
        None => None,
    };
    let frames = args.frames.as_deref().ok_or("missing --frames")?;
    let loaded = load(&args.labels, Some(frames), args.role_labels.as_deref())?;
    let mut learner = Learner::new();
    let mut learned_rows = 0usize;
    for sample in &loaded.samples {
        if !sample.readable {
            continue;
        }
        let image = util::open_rgb(&sample.path)?;
        learner.add_frame(&image, &sample.labels);
        learned_rows += 1;
    }
    let observed = learner.observed();
    let learned = learner.build();
    let replace: BTreeSet<String> = args.replace.iter().cloned().collect();
    let (set, classes) = merge(learned, &observed, base, &replace, args.replace_all)?;
    reject_near(&set, &classes, args.allow_near)?;
    set.check()
        .map_err(|e| format!("refusing to write an unloadable rank pack: {e}"))?;
    let dest = super::output::prepare_templates(out, args.base.as_deref(), args.force)?;
    let bytes = serde_json::to_vec(&set).map_err(|e| e.to_string())?;
    super::output::write_private(&dest, &bytes)?;
    let mut text = loaded.ignored;
    text.push_str(&util::summary(
        "rank",
        out,
        loaded.rows,
        learned_rows,
        loaded.unreadable,
        loaded.skipped,
        &classes,
    ));
    Ok(text)
}

pub fn eval(args: &EvalArgs) -> Result<EvalDone, String> {
    let frames = args.frames.as_deref().ok_or("missing --frames")?;
    let baseline =
        TemplateSet::load_dir(&args.baseline).map_err(|e| format!("baseline pack: {e}"))?;
    let candidate =
        TemplateSet::load_dir(&args.candidate).map_err(|e| format!("candidate pack: {e}"))?;
    let sha = file_sha(&args.candidate.join("templates.json"))?;
    let dropped = dropped_classes(&baseline, &candidate, &args.allow_drop)?;
    let loaded = load(&args.labels, Some(frames), args.role_labels.as_deref())?;
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
    let mut days: Vec<String> = loaded.samples.iter().map(|s| s.day.clone()).collect();
    days.sort();
    days.dedup();
    let mut day_learners: BTreeMap<String, Learner> = BTreeMap::new();
    for sample in loaded.samples.iter().filter(|s| s.readable) {
        let image = util::open_rgb(&sample.path)?;
        day_learners
            .entry(sample.day.clone())
            .or_default()
            .add_frame(&image, &sample.labels);
    }

    let mut bag = Bag::new();
    for day in &days {
        let mut learner = Learner::new();
        for (other, part) in &day_learners {
            if other != day {
                learner.merge(part);
            }
        }
        let fold = Reader::new(learner.build());
        for sample in loaded.samples.iter().filter(|s| s.day == *day) {
            let image = util::open_rgb(&sample.path)?;
            record(&base_reader, &fold, &pack_reader, sample, &image, &mut bag);
        }
    }
    let mut rows = Vec::new();
    for name in Labels::NAMES {
        rows.push(ScoreRow {
            label: format!("field {name}"),
            folds: bag.folds[name].len(),
            labelled: bag.labelled[name],
            items: bag.fields.remove(name).unwrap_or_default(),
        });
    }
    rows.push(ScoreRow {
        label: "screen gate".into(),
        folds: bag.gate_days.len(),
        labelled: loaded.samples.len(),
        items: bag.gate,
    });
    let folds = class_folds_from(&day_learners);
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
        class_folds: &folds,
        candidate_sha: &sha,
    });
    done.report.insert_str(0, &loaded.ignored);
    Ok(done)
}

fn record(
    base: &Reader,
    learned: &Reader,
    packed: &Reader,
    sample: &Sample,
    image: &image::RgbImage,
    bag: &mut Bag,
) {
    let b = score(base, image, sample);
    let l = score(learned, image, sample);
    let p = score(packed, image, sample);
    bag.gate.push(Item {
        frame: sample.id.clone(),
        baseline: b.0,
        learned: l.0,
        packed: p.0,
    });
    bag.gate_days.insert(sample.day.clone());
    if !sample.readable {
        return;
    }
    for (name, want) in sample.labels.values() {
        if want.is_some() {
            bag.folds
                .get_mut(name)
                .expect("field")
                .insert(sample.day.clone());
            *bag.labelled.get_mut(name).expect("field") += 1;
        }
        bag.fields.get_mut(name).expect("field").push(Item {
            frame: sample.id.clone(),
            baseline: b.1[name],
            learned: l.1[name],
            packed: p.1[name],
        });
    }
}

fn score(
    reader: &Reader,
    image: &image::RgbImage,
    sample: &Sample,
) -> (Outcome, BTreeMap<&'static str, Outcome>) {
    let read = reader.read(image);
    let gate = if read.is_ok() == sample.readable {
        Outcome::Correct
    } else {
        Outcome::Wrong
    };
    let got = read.ok().as_ref().map(RankRead::values).unwrap_or_default();
    let mut fields = BTreeMap::new();
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
        fields.insert(name, outcome);
    }
    (gate, fields)
}

struct Sample {
    id: String,
    day: String,
    path: PathBuf,
    readable: bool,
    labels: Labels,
}

struct Bag {
    fields: BTreeMap<&'static str, Vec<Item>>,
    folds: BTreeMap<&'static str, BTreeSet<String>>,
    labelled: BTreeMap<&'static str, usize>,
    gate: Vec<Item>,
    gate_days: BTreeSet<String>,
}

impl Bag {
    fn new() -> Self {
        let mut fields = BTreeMap::new();
        let mut folds = BTreeMap::new();
        let mut labelled = BTreeMap::new();
        for name in Labels::NAMES {
            fields.insert(name, Vec::new());
            folds.insert(name, BTreeSet::new());
            labelled.insert(name, 0);
        }
        Self {
            fields,
            folds,
            labelled,
            gate: Vec::new(),
            gate_days: BTreeSet::new(),
        }
    }
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

fn changed_classes(
    baseline: &TemplateSet,
    candidate: &TemplateSet,
    samples: &[Sample],
    allow: &[String],
) -> Result<(Vec<CountChange>, Vec<String>), String> {
    let base = index(baseline);
    let cand = index(candidate);
    let mut changed = Vec::new();
    let mut unverified = Vec::new();
    let allow_set: BTreeSet<&str> = allow.iter().map(String::as_str).collect();
    let mut known = BTreeSet::new();
    for (key, template) in &cand {
        known.insert(class_key(key.0, &key.1));
        let name = class_key(key.0, &key.1);
        let (before, differs) = match base.get(key) {
            Some(old) => (
                old.n,
                old.n != template.n
                    || old.aspect != template.aspect
                    || old.vector != template.vector,
            ),
            None => (0, true),
        };
        if !differs {
            continue;
        }
        changed.push((name.clone(), before, template.n));
        if !exercised(key.0, &key.1, samples) && !allow_set.contains(name.as_str()) {
            unverified.push(name);
        }
    }
    let mut unknown = Vec::new();
    for name in allow {
        if !known.contains(name) && !changed.iter().any(|(n, _, _)| n == name) {
            unknown.push(name.as_str());
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "--allow-unverified is not a candidate class: {}",
            unknown.join(", ")
        ));
    }
    changed.sort();
    unverified.sort();
    Ok((changed, unverified))
}

fn exercised(family: Family, class: &str, samples: &[Sample]) -> bool {
    if family == Family::Gate {
        return !samples.is_empty();
    }
    samples
        .iter()
        .any(|sample| sample.readable && class_shown(family, class, &sample.labels))
}

fn class_shown(family: Family, class: &str, labels: &Labels) -> bool {
    let digits = |v: i64| -> Vec<String> {
        v.unsigned_abs()
            .to_string()
            .chars()
            .map(|c| c.to_string())
            .collect()
    };
    match family {
        Family::Gate => true,
        Family::Pct => labels.progress_pct.is_some_and(|v| {
            let mut chars = Vec::new();
            if v < 0 {
                chars.push("-".to_string());
            }
            chars.extend(digits(i64::from(v)));
            chars.push("%".to_string());
            chars.iter().any(|c| c == class)
        }),
        Family::Higher => labels.higher_than_pct.is_some_and(|v| {
            let mut chars = digits(i64::from(v));
            chars.push("%".to_string());
            chars.iter().any(|c| c == class)
        }),
        Family::Pill => labels.progress_change.is_some_and(|v| {
            if class == crate::shadow::rank::CHEV_LEFT {
                return v < 0;
            }
            if class == crate::shadow::rank::CHEV_RIGHT || class == "+" {
                return v > 0;
            }
            if class == "%" {
                return true;
            }
            digits(i64::from(v)).iter().any(|c| c == class)
        }),
        Family::Sr => {
            if class == crate::shadow::rank::SR_ICON {
                return labels.hero_sr.is_some();
            }
            labels
                .hero_sr
                .is_some_and(|v| digits(i64::from(v)).iter().any(|c| c == class))
        }
        Family::Change => labels
            .hero_sr_change
            .is_some_and(|v| digits(i64::from(v)).iter().any(|c| c == class)),
        Family::Tag => labels
            .change_tags
            .as_ref()
            .is_some_and(|tags| tags.iter().any(|tag| tag == class)),
        Family::Banner => labels.next_mode_hint.as_deref() == Some(class),
        Family::Tier => labels.role_rank.as_deref() == Some(class),
        Family::Role => labels.role.is_some_and(|role| role.as_str() == class),
        Family::Division | Family::Emblem => labels.division.is_some_and(|d| {
            if !(1..=5).contains(&d) {
                return false;
            }
            let (left, right) = crate::shadow::rank::neighbours(d);
            class == d.to_string() || class == left.to_string() || class == right.to_string()
        }),
    }
}

fn index(
    set: &TemplateSet,
) -> BTreeMap<(Family, String), crate::shadow::rank::templates::ClassTemplate> {
    let mut out = BTreeMap::new();
    for (family, templates) in &set.families {
        for template in templates {
            out.insert((*family, template.class.clone()), template.clone());
        }
    }
    out
}

fn class_keys(set: &TemplateSet) -> BTreeSet<String> {
    index(set)
        .keys()
        .map(|(family, class)| class_key(*family, class))
        .collect()
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

fn parse_family(name: &str) -> Result<Family, String> {
    Ok(match name {
        "gate" => Family::Gate,
        "pct" => Family::Pct,
        "higher" => Family::Higher,
        "pill" => Family::Pill,
        "sr" => Family::Sr,
        "change" => Family::Change,
        "tag" => Family::Tag,
        "banner" => Family::Banner,
        "tier" => Family::Tier,
        "division" => Family::Division,
        "emblem" => Family::Emblem,
        "role" => Family::Role,
        other => return Err(format!("unknown rank family {other:?}")),
    })
}

fn merge(
    learned: TemplateSet,
    observed: &BTreeSet<(Family, String)>,
    base: Option<TemplateSet>,
    replace: &BTreeSet<String>,
    replace_all: bool,
) -> Result<(TemplateSet, Vec<ClassNote>), String> {
    for name in replace {
        let (family, class) = split_key(name)?;
        if !observed.contains(&(family, class)) {
            return Err(format!("--replace {name} was not learned from the labels"));
        }
    }
    let Some(base) = base else {
        let mut notes = Vec::new();
        for (family, templates) in &learned.families {
            for template in templates {
                notes.push((
                    class_key(*family, &template.class),
                    ClassFate::New,
                    0,
                    template.n,
                ));
            }
        }
        notes.sort_by(|a, b| a.0.cmp(&b.0));
        return Ok((learned, notes));
    };
    let mut base_map = index(&base);
    let mut learned_map = index(&learned);
    let mut keys: BTreeSet<(Family, String)> = BTreeSet::new();
    keys.extend(base_map.keys().cloned());
    keys.extend(learned_map.keys().cloned());
    let mut families: BTreeMap<Family, Vec<crate::shadow::rank::templates::ClassTemplate>> =
        BTreeMap::new();
    let mut notes = Vec::new();
    for key in keys {
        let name = class_key(key.0, &key.1);
        let obs = observed.contains(&key);
        let do_replace = replace_all || replace.contains(&name);
        let (fate, template, n_base, n_new) = match (
            base_map.remove(&key),
            learned_map.remove(&key),
            obs,
            do_replace,
        ) {
            (Some(base_t), Some(learned_t), true, true) => {
                let n_base = base_t.n;
                let n_new = learned_t.n;
                (ClassFate::Replaced, learned_t, n_base, n_new)
            }
            (Some(base_t), Some(learned_t), true, false) if base_t.pooled => {
                let n_base = base_t.n;
                let n_new = learned_t.n;
                (ClassFate::Replaced, learned_t, n_base, n_new)
            }
            (Some(base_t), Some(learned_t), true, false) => {
                let mixed = mix_rank(&base_t, &learned_t);
                let n_base = base_t.n;
                let n_new = mixed.n;
                (ClassFate::Learned, mixed, n_base, n_new)
            }
            (Some(base_t), Some(_), false, _) => {
                let n = base_t.n;
                (ClassFate::KeptPooled, base_t, n, n)
            }
            (Some(base_t), None, _, _) => {
                let n = base_t.n;
                (ClassFate::Kept, base_t, n, n)
            }
            (None, Some(learned_t), _, _) => {
                let n = learned_t.n;
                let fate = if learned_t.pooled {
                    ClassFate::NewPooled
                } else {
                    ClassFate::New
                };
                (fate, learned_t, 0, n)
            }
            (None, None, _, _) => continue,
        };
        notes.push((name, fate, n_base, n_new));
        families.entry(key.0).or_default().push(template);
    }
    notes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((
        TemplateSet {
            recognizer: learned.recognizer,
            families,
        },
        notes,
    ))
}

fn mix_rank(
    base: &crate::shadow::rank::templates::ClassTemplate,
    learned: &crate::shadow::rank::templates::ClassTemplate,
) -> crate::shadow::rank::templates::ClassTemplate {
    let nb = base.n as f32;
    let nl = learned.n as f32;
    let mut mixed = vec![0.0; base.vector.len()];
    for (slot, (b, l)) in mixed
        .iter_mut()
        .zip(base.vector.iter().zip(&learned.vector))
    {
        *slot = nb * b + nl * l;
    }
    let aspect = ((nb * base.aspect.ln() + nl * learned.aspect.ln()) / (nb + nl)).exp();
    crate::shadow::rank::templates::ClassTemplate {
        class: base.class.clone(),
        aspect,
        n: base.n.saturating_add(learned.n),
        vector: unit(mixed),
        pooled: false,
    }
}

fn split_key(name: &str) -> Result<(Family, String), String> {
    let Some((family, class)) = name.split_once('/') else {
        return Err(format!("rank class {name:?} must be family/class"));
    };
    Ok((parse_family(family)?, class.to_string()))
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
    for (family, templates) in &set.families {
        let limit = if family.is_glyph() {
            1.0 - crate::shadow::rank::MIN_MARGIN
        } else {
            1.0 - crate::shadow::rank::MIN_WORD_MARGIN
        };
        for i in 0..templates.len() {
            for j in (i + 1)..templates.len() {
                let left = class_key(*family, &templates[i].class);
                let right = class_key(*family, &templates[j].class);
                if !hot.contains(left.as_str()) && !hot.contains(right.as_str()) {
                    continue;
                }
                if !competes(*family, &templates[i].class, &templates[j].class) {
                    continue;
                }
                let dot: f32 = templates[i]
                    .vector
                    .iter()
                    .zip(&templates[j].vector)
                    .map(|(a, b)| a * b)
                    .sum();
                if dot > limit {
                    return Err(format!(
                        "class {} correlates {dot:.3} with {}, above {limit:.3}",
                        class_key(*family, &templates[i].class),
                        class_key(*family, &templates[j].class)
                    ));
                }
            }
        }
    }
    Ok(())
}

fn competes(family: Family, a: &str, b: &str) -> bool {
    let digits = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    let division = ["1", "2", "3", "4", "5"];
    let allowed: &[&str] = match family {
        Family::Pct => &[
            "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "%", "-", "+",
        ],
        Family::Higher => &["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "%"],
        Family::Pill => &[
            "0",
            "1",
            "2",
            "3",
            "4",
            "5",
            "6",
            "7",
            "8",
            "9",
            "%",
            "+",
            crate::shadow::rank::CHEV_LEFT,
            crate::shadow::rank::CHEV_RIGHT,
        ],
        Family::Sr => &[
            "0",
            "1",
            "2",
            "3",
            "4",
            "5",
            "6",
            "7",
            "8",
            "9",
            crate::shadow::rank::SR_ICON,
        ],
        Family::Change => &digits,
        Family::Division | Family::Emblem => &division,
        Family::Gate | Family::Tag | Family::Banner | Family::Tier | Family::Role => return true,
    };
    allowed.contains(&a) && allowed.contains(&b)
}

fn class_folds_from(days: &BTreeMap<String, Learner>) -> Vec<(String, usize)> {
    let mut folds: BTreeMap<String, usize> = BTreeMap::new();
    for learner in days.values() {
        let set = learner.clone().build();
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
    folds.into_iter().collect()
}

const RANK_KEYS: &[&str] = &[
    "game",
    "file",
    "readable",
    "role",
    "role_rank",
    "division",
    "progress_pct",
    "progress_change",
    "change_tags",
    "higher_than_pct",
    "hero_sr",
    "hero_sr_change",
    "next_mode_hint",
];

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
    let (values, mut ignored) = read_objects(labels, RANK_KEYS)?;
    let mut rows: Vec<Row> = values
        .iter()
        .map(|value| serde_json::from_value(fill_label_nulls(value)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{}: {e}", labels.display()))?;
    if let Some(path) = role_labels {
        let (role_values, role_ignored) = read_objects(path, &["game", "role"])?;
        ignored.push_str(&role_ignored);
        let roles: Vec<RoleRow> = role_values
            .iter()
            .map(|value| serde_json::from_value(value.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut seen = BTreeSet::new();
        for role in &roles {
            if !seen.insert(role.game.as_str()) {
                return Err(format!("role labels list {} twice", role.game));
            }
        }
        for row in &mut rows {
            if let Some(role) = roles.iter().find(|r| r.game == row.game) {
                row.labels.role = role.role;
            }
        }
    }
    let mut samples = Vec::new();
    let mut skipped = 0usize;
    let mut unreadable = 0usize;
    for row in &rows {
        check_words(&row.labels)?;
        let day = day_of(&row.game)?;
        reject_name("game", &row.game)?;
        reject_name("file", &row.file)?;
        let rel = Path::new(&row.game).join(&row.file);
        let path = util::under_root(frames, &rel)?;
        let (width, height) = image::image_dimensions(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        if Scale::from_dims(width, height).is_none() {
            skipped += 1;
            continue;
        }
        if !row.readable {
            unreadable += 1;
        }
        samples.push(Sample {
            id: format!("{}/{}", row.game, row.file),
            day,
            path,
            readable: row.readable,
            labels: row.labels.clone(),
        });
    }
    Ok(Loaded {
        rows: rows.len(),
        unreadable,
        skipped,
        samples,
        ignored,
    })
}

/// `Labels` options have no `serde(default)`, so a row that omits a field
/// is filled with null before it is decoded. Null and a missing field are
/// the same: the field was not labelled.
fn fill_label_nulls(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(obj) = value.as_object_mut() {
        for key in RANK_KEYS {
            if *key != "game" && *key != "file" && *key != "readable" {
                obj.entry((*key).to_string()).or_insert(Value::Null);
            }
        }
    }
    value
}

fn read_objects(path: &Path, keys: &[&str]) -> Result<(Vec<Value>, String), String> {
    let (rows, ignored) = util::read_jsonl(path, keys)?;
    Ok((rows.into_iter().map(Value::Object).collect(), ignored))
}

fn check_words(labels: &Labels) -> Result<(), String> {
    if let Some(text) = &labels.role_rank {
        bad_word("role_rank", text)?;
    }
    if let Some(text) = &labels.next_mode_hint {
        bad_word("next_mode_hint", text)?;
    }
    if let Some(tags) = &labels.change_tags {
        for tag in tags {
            bad_word("change_tags", tag)?;
        }
    }
    Ok(())
}

fn bad_word(field: &str, text: &str) -> Result<(), String> {
    if text.is_empty() || text != text.trim() || text.contains("  ") {
        Err(format!(
            "{field} {text:?} has leading, trailing or doubled spaces"
        ))
    } else {
        Ok(())
    }
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
