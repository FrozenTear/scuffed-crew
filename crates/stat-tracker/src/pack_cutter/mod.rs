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
            code: 2,
            stdout: String::new(),
            stderr: msg.into(),
        }
    }
}

/// Run the pack cutter. Prints the report or the error and returns the
/// process status: 0 pass, 1 gate failure, 2 any other error.
pub fn run<I, S>(args: I) -> i32
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let args: Vec<std::ffi::OsString> = args
        .into_iter()
        .map(|s| s.as_ref().to_os_string())
        .collect();
    let outcome = execute(&args);
    if !outcome.stdout.is_empty() {
        print!("{}", outcome.stdout);
    }
    if !outcome.stderr.is_empty() {
        eprintln!("{}", outcome.stderr);
    }
    outcome.code
}

fn execute(args: &[std::ffi::OsString]) -> Outcome {
    let command = match cli::parse(args) {
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
    if args.kind == Kind::QueueTitle && args.role_labels.is_some() {
        return Err("--role-labels is only used with --kind rank".into());
    }
    require_rank_frames(args.kind, &args.frames)?;
    match args.kind {
        Kind::Rank => rank::cut(args, &out),
        Kind::QueueTitle => queue::cut(args, &out),
    }
}

fn pack(args: &PackArgs) -> Result<String, String> {
    let out = output::check_out(&args.out)?;
    output::check_version(&args.version)?;
    let templates = args.dir.join("templates.json");
    match args.kind {
        Kind::Rank => {
            crate::shadow::rank::TemplateSet::load_dir(&args.dir)
                .map_err(|e| format!("rank pack: {e}"))?;
        }
        Kind::QueueTitle => {
            crate::shadow::queue_title::TemplateSet::load_dir(&args.dir)
                .map_err(|e| format!("queue-title pack: {e}"))?;
        }
    }
    let bytes =
        std::fs::read(&templates).map_err(|e| format!("reading {}: {e}", templates.display()))?;
    let sha = ustar::sha256_hex(&bytes);
    let members = [ustar::Member {
        path: "templates.json".into(),
        bytes,
    }];
    let tar = ustar::archive(&args.name, &args.version, &members)?;
    let mut text = output::install_pack(&out, &args.name, &args.version, &tar)?;
    text.push_str(&format!("templates.json sha256: {sha}\n"));
    Ok(text)
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
