//! Small flag parser. This crate has no CLI dependency.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub const USAGE: &str = "\
usage:
  scuffed-pack-cutter cut --kind rank|queue-title --labels FILE [--frames DIR] [--role-labels FILE] [--base DIR] [--replace CLASS ...] [--replace-all] [--force] --out DIR
  scuffed-pack-cutter pack --kind rank|queue-title --dir DIR --name NAME.tar --version VER --out DIR
  scuffed-pack-cutter eval --kind rank|queue-title --labels FILE [--frames DIR] --baseline DIR --candidate DIR --set-name NAME [--role-labels FILE] [--allow-drop CLASS ...] [--allow-unverified CLASS ...]
  scuffed-pack-cutter help

--frames is required for rank, and for queue-title when a label path is relative.
--base, --baseline and --candidate are folders that hold templates.json (extract the published tar, or use the tracker's templates/rank-v1 or templates/queue-title-v1 folder).
--name must be rank-v1.tar or queue-title-v1.tar, matching --kind.
--replace CLASS (repeatable) or --replace-all makes a re-learned class replace the base template. The default mixes them by sample count.
--allow-drop CLASS lets eval accept a baseline class the candidate does not have.
--allow-unverified CLASS lets eval accept a changed class the label set does not exercise.
Rank class names are family/class (role/Tank). Queue-title class names are the title text.

Exit status: 0 when eval passes or help is shown, 1 when the eval gate fails, 2 on any other error.";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rank,
    QueueTitle,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rank" => Some(Self::Rank),
            "queue-title" => Some(Self::QueueTitle),
            _ => None,
        }
    }

    pub fn pack_name(self) -> &'static str {
        match self {
            Self::Rank => "rank-v1.tar",
            Self::QueueTitle => "queue-title-v1.tar",
        }
    }
}

pub struct CutArgs {
    pub kind: Kind,
    pub labels: PathBuf,
    pub frames: Option<PathBuf>,
    pub role_labels: Option<PathBuf>,
    pub base: Option<PathBuf>,
    pub out: PathBuf,
    pub replace: Vec<String>,
    pub replace_all: bool,
    pub force: bool,
}

pub struct PackArgs {
    pub kind: Kind,
    pub dir: PathBuf,
    pub name: String,
    pub version: String,
    pub out: PathBuf,
}

pub struct EvalArgs {
    pub kind: Kind,
    pub labels: PathBuf,
    pub frames: Option<PathBuf>,
    pub role_labels: Option<PathBuf>,
    pub baseline: PathBuf,
    pub candidate: PathBuf,
    pub set_name: String,
    pub allow_drop: Vec<String>,
    pub allow_unverified: Vec<String>,
}

pub enum Command {
    Help,
    Cut(CutArgs),
    Pack(PackArgs),
    Eval(EvalArgs),
}

struct Parsed {
    values: BTreeMap<String, OsString>,
    allow_drop: Vec<String>,
    allow_unverified: Vec<String>,
    replace: Vec<String>,
    replace_all: bool,
    force: bool,
}

pub fn parse(args: &[OsString]) -> Result<Command, String> {
    if args
        .first()
        .is_some_and(|arg| is_help(arg) || arg == "help")
    {
        return Ok(Command::Help);
    }
    let (cmd, rest) = args
        .split_first()
        .ok_or_else(|| "missing command".to_string())?;
    let cmd = utf8("command", cmd)?;
    let Some(flags) = flags(rest)? else {
        return Ok(Command::Help);
    };
    match cmd.as_str() {
        "cut" => Ok(Command::Cut(cut_args(flags)?)),
        "pack" => Ok(Command::Pack(pack_args(flags)?)),
        "eval" => Ok(Command::Eval(eval_args(flags)?)),
        other => Err(format!("unknown command {other:?}")),
    }
}

fn is_help(arg: &OsStr) -> bool {
    arg == "--help" || arg == "-h"
}

fn cut_args(mut flags: Parsed) -> Result<CutArgs, String> {
    if !flags.allow_drop.is_empty() || !flags.allow_unverified.is_empty() {
        return Err("--allow-drop and --allow-unverified are only used with eval".into());
    }
    let args = CutArgs {
        kind: kind(&take_str(&mut flags.values, "kind")?)?,
        labels: take_path(&mut flags.values, "labels")?,
        frames: take_path_opt(&mut flags.values, "frames")?,
        role_labels: take_path_opt(&mut flags.values, "role-labels")?,
        base: take_path_opt(&mut flags.values, "base")?,
        out: take_path(&mut flags.values, "out")?,
        replace: flags.replace,
        replace_all: flags.replace_all,
        force: flags.force,
    };
    finish(&flags.values)?;
    Ok(args)
}

fn pack_args(mut flags: Parsed) -> Result<PackArgs, String> {
    if !flags.allow_drop.is_empty()
        || !flags.allow_unverified.is_empty()
        || !flags.replace.is_empty()
        || flags.replace_all
        || flags.force
    {
        return Err("pack only takes --kind, --dir, --name, --version and --out".into());
    }
    let kind = kind(&take_str(&mut flags.values, "kind")?)?;
    let name = take_str(&mut flags.values, "name")?;
    if name != kind.pack_name() {
        return Err(format!(
            "--name {name} does not match --kind {} ({})",
            match kind {
                Kind::Rank => "rank",
                Kind::QueueTitle => "queue-title",
            },
            kind.pack_name()
        ));
    }
    let args = PackArgs {
        kind,
        dir: take_path(&mut flags.values, "dir")?,
        name,
        version: take_str(&mut flags.values, "version")?,
        out: take_path(&mut flags.values, "out")?,
    };
    finish(&flags.values)?;
    Ok(args)
}

fn eval_args(mut flags: Parsed) -> Result<EvalArgs, String> {
    if !flags.replace.is_empty() || flags.replace_all || flags.force {
        return Err("--replace, --replace-all and --force are only used with cut".into());
    }
    let args = EvalArgs {
        kind: kind(&take_str(&mut flags.values, "kind")?)?,
        labels: take_path(&mut flags.values, "labels")?,
        frames: take_path_opt(&mut flags.values, "frames")?,
        role_labels: take_path_opt(&mut flags.values, "role-labels")?,
        baseline: take_path(&mut flags.values, "baseline")?,
        candidate: take_path(&mut flags.values, "candidate")?,
        set_name: take_str(&mut flags.values, "set-name")?,
        allow_drop: flags.allow_drop,
        allow_unverified: flags.allow_unverified,
    };
    finish(&flags.values)?;
    Ok(args)
}

fn kind(s: &str) -> Result<Kind, String> {
    Kind::parse(s).ok_or_else(|| format!("--kind must be rank or queue-title, got {s:?}"))
}

fn take_str(flags: &mut BTreeMap<String, OsString>, key: &str) -> Result<String, String> {
    let value = flags
        .remove(key)
        .ok_or_else(|| format!("missing --{key}"))?;
    utf8(key, &value)
}

fn take_path(flags: &mut BTreeMap<String, OsString>, key: &str) -> Result<PathBuf, String> {
    flags
        .remove(key)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing --{key}"))
}

fn take_path_opt(
    flags: &mut BTreeMap<String, OsString>,
    key: &str,
) -> Result<Option<PathBuf>, String> {
    Ok(flags.remove(key).map(PathBuf::from))
}

fn finish(flags: &BTreeMap<String, OsString>) -> Result<(), String> {
    if let Some((key, _)) = flags.iter().next() {
        return Err(format!("unknown --{key}"));
    }
    Ok(())
}

fn utf8(key: &str, value: &OsStr) -> Result<String, String> {
    value
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("--{key} is not valid UTF-8"))
}

/// `Ok(None)` means the user asked for help from a flag-name position.
fn flags(args: &[OsString]) -> Result<Option<Parsed>, String> {
    let mut values = BTreeMap::new();
    let mut allow_drop = Vec::new();
    let mut allow_unverified = Vec::new();
    let mut replace = Vec::new();
    let mut switches = BTreeSet::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if is_help(arg) {
            return Ok(None);
        }
        let Some(text) = arg.to_str() else {
            return Err(format!("argument {} is not valid UTF-8", i + 1));
        };
        let Some(body) = text.strip_prefix("--") else {
            return Err(format!("unexpected argument {text:?}"));
        };
        if body.is_empty() {
            return Err("unexpected argument --".into());
        }
        let (key, inline) = match body.split_once('=') {
            Some((key, value)) => {
                if key.is_empty() || value.is_empty() {
                    return Err(format!("bad flag {text}"));
                }
                (key, Some(value.to_string()))
            }
            None => (body, None),
        };
        if key == "force" || key == "replace-all" {
            if inline.is_some() {
                return Err(format!("--{key} does not take a value"));
            }
            if !switches.insert(key.to_string()) {
                return Err(format!("duplicate --{key}"));
            }
            i += 1;
            continue;
        }
        let value = match inline {
            Some(v) => v,
            None => {
                i += 1;
                let Some(v) = args.get(i) else {
                    return Err(format!("missing value for --{key}"));
                };
                if v.to_str().is_some_and(|s| s.starts_with("--")) {
                    return Err(format!("missing value for --{key}"));
                }
                if matches!(
                    key,
                    "labels"
                        | "frames"
                        | "role-labels"
                        | "base"
                        | "out"
                        | "dir"
                        | "baseline"
                        | "candidate"
                ) {
                    values.insert(key.to_string(), v.clone());
                    i += 1;
                    continue;
                }
                utf8(key, v)?
            }
        };
        match key {
            "allow-drop" => allow_drop.push(value),
            "allow-unverified" => allow_unverified.push(value),
            "replace" => replace.push(value),
            _ => {
                if values
                    .insert(key.to_string(), OsString::from(value))
                    .is_some()
                {
                    return Err(format!("duplicate --{key}"));
                }
            }
        }
        i += 1;
    }
    Ok(Some(Parsed {
        values,
        allow_drop,
        allow_unverified,
        replace,
        replace_all: switches.contains("replace-all"),
        force: switches.contains("force"),
    }))
}
