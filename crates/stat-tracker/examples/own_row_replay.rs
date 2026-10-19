//! Replay ocr-v1 own-row detection over a folder of Tab PNGs.
//!
//! Usage: `cargo run -p scuffed-stat-tracker --example own_row_replay -- <dir> <player-name>`
//!
//! Each PNG in `<dir>` (not subfolders) is one frame. The printout is
//! counts only. The player name is used to match a row and is not printed.

use std::path::Path;
use std::process::ExitCode;

use stat_tracker::ocr::{format_own_row_counts, tally_own_row_dir};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        return usage();
    };
    let Some(player_name) = args.next() else {
        return usage();
    };
    if args.next().is_some() || player_name.is_empty() {
        return usage();
    }
    let counts = tally_own_row_dir(Path::new(&dir), &player_name);
    print!("{}", format_own_row_counts(&counts));
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    eprintln!("usage: own_row_replay <dir> <player-name>");
    ExitCode::from(2)
}
