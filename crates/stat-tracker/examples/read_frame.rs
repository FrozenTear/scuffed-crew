//! Eval probe for the main-reader entry point.
//! Usage: read_frame <pack-root> <tab-frame> [result-frame...]
//! Prints the full field list as JSON, then the suspect field names.

use std::path::Path;

use stat_tracker::shadow::{Reader, ReaderConfig, suspect_field_names};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, tab, rest @ ..] = &args[..] else {
        eprintln!("usage: read_frame <pack-root> <tab-frame> [result-frame...]");
        std::process::exit(2);
    };
    let reader = Reader::load(&ReaderConfig::from_data_dir(Path::new(root)));
    let frame = image::open(tab).expect("tab frame");
    let results: Vec<_> = rest
        .iter()
        .map(|p| image::open(p).expect("result frame"))
        .collect();
    let refs: Vec<&image::DynamicImage> = results.iter().collect();
    let board = reader.read_board_with_results(&frame, &refs);
    println!("{}", serde_json::to_string(&board).unwrap_or_default());
    println!("suspect: {:?}", suspect_field_names(&board));
}
