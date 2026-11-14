//! Cut `rank-v1` and `queue-title-v1` packs with the readers' own learners,
//! write a plain ustar plus a list manifest, and hold a candidate to a
//! published pack.
//!
//! `cut` and `pack` refuse to write inside a git work tree. This tool does
//! not publish. Real labelled frames stay on the machine that owns them.
//! The ignored benches `shadow::rank::tests::bench_labelled_rank_screens`
//! and `shadow::queue_title::tests::bench_labelled_queue_titles` are those
//! local runs. Tests here draw synthetic frames only.

mod cli;
mod output;
mod queue;
mod rank;
mod report;
mod ustar;
mod util;

#[cfg(test)]
mod tests;

use cli::{Command, CutArgs, EvalArgs, Kind, PackArgs};

fn require_rank_frames(kind: Kind, frames: &Option<std::path::PathBuf>) -> Result<(), String> {
    if kind == Kind::Rank && frames.is_none() {
        Err("missing --frames".into())
    } else {
        Ok(())
    }
}

/// What `run` prints and the status it returns.
struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Outcome {
    fn ok(stdout: String) -> Self {
        Self {
            code: 0,
            stdout,
            stderr: String::new(),
        }
    }

    fn report(text: String, pass: bool) -> Self {
        Self {
            code: if pass { 0 } else { 1 },
            stdout: text,
            stderr: String::new(),
        }
    }

    fn err(msg: impl Into<String>) -> Self {
        Self {
            code: 1,
            stdout: String::new(),
            stderr: msg.into(),
        }
    }
}

/// Run the pack cutter. Prints the report or the error and returns the
/// process status.
pub fn run<I, S>(args: I) -> i32
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let outcome = execute(args);
    if !outcome.stdout.is_empty() {
        print!("{}", outcome.stdout);
    }
    if !outcome.stderr.is_empty() {
        eprintln!("{}", outcome.stderr);
    }
    outcome.code
}

fn execute<I, S>(args: I) -> Outcome
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    let command = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => return Outcome::err(format!("{e}\n{}", cli::USAGE)),
    };
    match command {
        Command::Help => Outcome::ok(format!("{}\n", cli::USAGE)),
        Command::Cut(args) => match cut(&args) {
            Ok(text) => Outcome::ok(text),
            Err(e) => Outcome::err(e),
        },
        Command::Pack(args) => match pack(&args) {
            Ok(text) => Outcome::ok(text),
            Err(e) => Outcome::err(e),
        },
        Command::Eval(args) => match eval(&args) {
            Ok(done) => Outcome::report(done.report, done.pass),
            Err(e) => Outcome::err(e),
        },
    }
}

fn cut(args: &CutArgs) -> Result<String, String> {
    let out = output::check_out(&args.out)?;
    if let Some(v) = &args.version {
        output::check_version(v)?;
    }
    if args.kind == Kind::QueueTitle && args.role_labels.is_some() {
        return Err("--role-labels is only used with --kind rank".into());
    }
    require_rank_frames(args.kind, &args.frames)?;
    let body = match args.kind {
        Kind::Rank => rank::cut(args, &out)?,
        Kind::QueueTitle => queue::cut(args, &out)?,
    };
    let mut text = body;
    if let Some(v) = &args.version {
        text.push_str(&format!(
            "version: {v} (stored by pack, not written into templates.json)\n"
        ));
    }
    Ok(text)
}

fn pack(args: &PackArgs) -> Result<String, String> {
    let out = output::check_out(&args.out)?;
    output::check_version(&args.version)?;
    output::check_pack_name(&args.name)?;
    let templates = args.dir.join("templates.json");
    let bytes =
        std::fs::read(&templates).map_err(|e| format!("reading {}: {e}", templates.display()))?;
    let members = [ustar::Member {
        path: "templates.json".into(),
        bytes,
    }];
    let tar = ustar::archive(&args.name, &args.version, &members)?;
    output::install_pack(&out, &args.name, &args.version, &tar)
}

fn eval(args: &EvalArgs) -> Result<report::EvalDone, String> {
    if args.set_name.is_empty() || args.set_name.chars().any(char::is_control) {
        return Err("--set-name must be a non-empty line of text".into());
    }
    if args.kind == Kind::QueueTitle && args.role_labels.is_some() {
        return Err("--role-labels is only used with --kind rank".into());
    }
    require_rank_frames(args.kind, &args.frames)?;
    match args.kind {
        Kind::Rank => rank::eval(args),
        Kind::QueueTitle => queue::eval(args),
    }
}
