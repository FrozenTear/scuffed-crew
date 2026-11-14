//! Small flag parser. This crate has no CLI dependency.

use std::collections::BTreeMap;
use std::path::PathBuf;

pub const USAGE: &str = "\
usage:
  scuffed-pack-cutter cut --kind rank|queue-title --labels FILE [--frames DIR] [--role-labels FILE] [--base DIR] --out DIR [--version VER]
  scuffed-pack-cutter pack --dir DIR --name NAME.tar --version VER --out DIR
  scuffed-pack-cutter eval --kind rank|queue-title --labels FILE [--frames DIR] --baseline DIR --candidate DIR --set-name NAME [--role-labels FILE] [--allow-drop CLASS ...]
  scuffed-pack-cutter help

--frames is required for rank, and for queue-title when a label path is relative.
--base on cut keeps classes the labels do not produce.
--allow-drop CLASS (repeatable) lets eval accept a baseline class the candidate does not have.
Rank class names are family/class (role/Tank). Queue-title class names are the title text.";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rank,
    QueueTitle,
}

impl Kind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "rank" => Some(Self::Rank),
            "queue-title" => Some(Self::QueueTitle),
            _ => None,
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
    pub version: Option<String>,
}

pub struct PackArgs {
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
}

pub enum Command {
    Help,
    Cut(CutArgs),
    Pack(PackArgs),
    Eval(EvalArgs),
}

struct Parsed {
    values: BTreeMap<String, String>,
    allow_drop: Vec<String>,
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    if wants_help(args) {
        return Ok(Command::Help);
    }
    let (cmd, rest) = args
        .split_first()
        .ok_or_else(|| "missing command".to_string())?;
    let flags = flags(rest)?;
    match cmd.as_str() {
        "cut" => Ok(Command::Cut(cut_args(flags)?)),
        "pack" => Ok(Command::Pack(pack_args(flags)?)),
        "eval" => Ok(Command::Eval(eval_args(flags)?)),
        other => Err(format!("unknown command {other:?}")),
    }
}

fn wants_help(args: &[String]) -> bool {
    match args.first().map(String::as_str) {
        Some("help" | "--help" | "-h") => true,
        _ => args.iter().any(|arg| arg == "--help" || arg == "-h"),
    }
}

fn cut_args(mut flags: Parsed) -> Result<CutArgs, String> {
    if !flags.allow_drop.is_empty() {
        return Err("--allow-drop is only used with eval".into());
    }
    let args = CutArgs {
        kind: kind(&take(&mut flags.values, "kind")?)?,
        labels: path(take(&mut flags.values, "labels")?),
        frames: take_opt(&mut flags.values, "frames").map(path),
        role_labels: take_opt(&mut flags.values, "role-labels").map(path),
        base: take_opt(&mut flags.values, "base").map(path),
        out: path(take(&mut flags.values, "out")?),
        version: take_opt(&mut flags.values, "version"),
    };
    finish(flags.values)?;
    Ok(args)
}

fn pack_args(mut flags: Parsed) -> Result<PackArgs, String> {
    if !flags.allow_drop.is_empty() {
        return Err("--allow-drop is only used with eval".into());
    }
    let args = PackArgs {
        dir: path(take(&mut flags.values, "dir")?),
        name: take(&mut flags.values, "name")?,
        version: take(&mut flags.values, "version")?,
        out: path(take(&mut flags.values, "out")?),
    };
    finish(flags.values)?;
    Ok(args)
}

fn eval_args(mut flags: Parsed) -> Result<EvalArgs, String> {
    let args = EvalArgs {
        kind: kind(&take(&mut flags.values, "kind")?)?,
        labels: path(take(&mut flags.values, "labels")?),
        frames: take_opt(&mut flags.values, "frames").map(path),
        role_labels: take_opt(&mut flags.values, "role-labels").map(path),
        baseline: path(take(&mut flags.values, "baseline")?),
        candidate: path(take(&mut flags.values, "candidate")?),
        set_name: take(&mut flags.values, "set-name")?,
        allow_drop: flags.allow_drop,
    };
    finish(flags.values)?;
    Ok(args)
}

fn kind(s: &str) -> Result<Kind, String> {
    Kind::parse(s).ok_or_else(|| format!("--kind must be rank or queue-title, got {s:?}"))
}

fn path(s: String) -> PathBuf {
    PathBuf::from(s)
}

fn take(flags: &mut BTreeMap<String, String>, key: &str) -> Result<String, String> {
    flags.remove(key).ok_or_else(|| format!("missing --{key}"))
}

fn take_opt(flags: &mut BTreeMap<String, String>, key: &str) -> Option<String> {
    flags.remove(key)
}

fn finish(flags: BTreeMap<String, String>) -> Result<(), String> {
    if let Some((key, _)) = flags.iter().next() {
        return Err(format!("unknown --{key}"));
    }
    Ok(())
}

fn flags(args: &[String]) -> Result<Parsed, String> {
    let mut values = BTreeMap::new();
    let mut allow_drop = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let Some(body) = arg.strip_prefix("--") else {
            return Err(format!("unexpected argument {arg:?}"));
        };
        if body.is_empty() {
            return Err("unexpected argument --".into());
        }
        let (key, inline) = match body.split_once('=') {
            Some((key, value)) => {
                if key.is_empty() || value.is_empty() {
                    return Err(format!("bad flag {arg}"));
                }
                (key, Some(value.to_string()))
            }
            None => (body, None),
        };
        let value = match inline {
            Some(v) => v,
            None => {
                i += 1;
                let Some(v) = args.get(i) else {
                    return Err(format!("missing value for --{key}"));
                };
                if v.starts_with("--") {
                    return Err(format!("missing value for --{key}"));
                }
                v.clone()
            }
        };
        if key == "allow-drop" {
            if value.is_empty() {
                return Err("--allow-drop needs a class".into());
            }
            allow_drop.push(value);
        } else if values.insert(key.to_string(), value).is_some() {
            return Err(format!("duplicate --{key}"));
        }
        i += 1;
    }
    Ok(Parsed { values, allow_drop })
}
