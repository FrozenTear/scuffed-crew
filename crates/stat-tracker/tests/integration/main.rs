//! The stat tracker's integration tests, in one test binary.
//!
//! Each file in this folder was a test binary of its own. Every debug test
//! binary links the whole tracker and its dependencies (close to 900 MB
//! each), so they are modules of this one binary instead: one link, not one
//! per file. Cargo finds this binary as `tests/integration/main.rs`; run it
//! with `cargo test -p scuffed-stat-tracker --test integration`, and one
//! file's tests with a module filter, e.g. `-- ocr_switch::`.
//!
//! The tests share a process now. A test that needs one to itself (it sets
//! process-wide state or reads a process-wide counter) starts itself again
//! in a process of its own through [`common::own_process`].
//!
//! A new test file goes here with a `mod` line below, not as a new
//! `tests/*.rs`: [`every_test_file_is_a_module_of_this_binary`] checks both.

mod common;

mod accolade_fixtures;
mod dorado_dim_zero_fixtures;
mod end_reel_fixtures;
mod end_title_fixtures;
mod end_title_wiring;
mod final_score_wiring;
mod name_window_geometry;
mod names_fallback;
mod no_game_art;
mod ocr_flags;
mod ocr_replay_benchmark;
mod ocr_switch;
mod ocr_switch_new_reader;
mod omp_thread_limit;
mod outcome_fixtures;
mod pack_keeper_daemon;
mod phase_fixtures;
mod player_row_fixtures;
mod real_dim_zero_cells;
mod result_signal_wiring;
mod run_marker_sigterm;
mod team_size_fixtures;
mod vision_baseline_fixtures;

/// A file in this folder without a `mod` line is never compiled, and a new
/// `tests/*.rs` is a test binary of its own again.
#[test]
fn every_test_file_is_a_module_of_this_binary() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let rs_files = |dir: &std::path::Path| -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .map(|entry| entry.expect("dir entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    assert_eq!(
        rs_files(&tests),
        Vec::<String>::new(),
        "a test file directly in tests/ is a test binary of its own: move it \
         to tests/integration/ and add a `mod` line to main.rs"
    );
    let main = include_str!("main.rs");
    let missing: Vec<String> = rs_files(&tests.join("integration"))
        .into_iter()
        .filter(|module| module != "main")
        .filter(|module| !main.contains(&format!("\nmod {module};\n")))
        .collect();
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "files in tests/integration/ with no `mod` line in main.rs are never compiled"
    );
}
