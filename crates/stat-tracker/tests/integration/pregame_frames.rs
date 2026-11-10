//! Opt-in checks of the pre-game saves' screen checks on real frames: the
//! ENTERING GAME loading rule (`detect/loading_screen.rs`) and the roster
//! layout check (`detect/roster_screen.rs`). No real frame is in the repo
//! (they show other players), so this reads a local folder named by
//! `SCUFFED_PREGAME_FRAMES_DIR` and skips quietly when the variable is
//! unset or a file is missing. Run it with
//!
//! ```text
//! SCUFFED_PREGAME_FRAMES_DIR=/path/to/frames \
//!   cargo test -p scuffed-stat-tracker --test integration \
//!   pregame_frames -- --nocapture
//! ```
//!
//! Files it looks for, by name (native-resolution captures, `.png`):
//!
//! * `entering_game*.png`: ENTERING GAME frames (on the reference set
//!   `entering_game_1030_223014.png`, `entering_game_1030_g2.png` and
//!   `entering_game_1030_g3.png` are mid-fade, `entering_game_1030_g4.png`
//!   is bright). Each must be the loading slab. One whose text is drawn
//!   (glyphs at the glyph level, a text box measured) must pass the
//!   loading rule; one with no text box yet is a fade-in the rule turns
//!   down on purpose (rule A), and the saver keeps it as
//!   `rejected_loading.png`.
//! * `*applying_update*.png` (on the reference set
//!   `negative_applying_update.png`): must fail the loading rule (rule C,
//!   the word ratio).
//! * `roster*.png` (on the reference set
//!   `roster_5v5_dorado_20261028_2146.png`): must pass the roster check.
//!   Every other file must fail it.
//!
//! Each file's numbers are printed: the text box (A samples, B width, C
//! word ratio, spinner) and the roster part scores.

use stat_tracker::detect::loading_screen::{is_entering_game, is_loading_screen, text_box};
use stat_tracker::detect::match_start::passes_ban_gate;
use stat_tracker::detect::roster_screen::scores;

/// The folder `SCUFFED_PREGAME_FRAMES_DIR` names, if set and not empty.
fn frames_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("SCUFFED_PREGAME_FRAMES_DIR")
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
}

/// Each `.png` in the folder, sorted, with its lowercase name.
fn frames() -> Vec<(String, image::RgbImage)> {
    let Some(dir) = frames_dir() else {
        eprintln!("skip: SCUFFED_PREGAME_FRAMES_DIR is not set");
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skip: {} is not readable", dir.display());
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let rgb = image::open(&path)
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .to_rgb8();
            (name, rgb)
        })
        .collect()
}

/// The saver's loading rule: the ENTERING GAME check, and no hero-ban
/// screen.
fn loading_rule(rgb: &image::RgbImage) -> bool {
    is_entering_game(rgb) && !passes_ban_gate(rgb)
}

fn describe_text(rgb: &image::RgbImage) -> String {
    match text_box(rgb) {
        Some(t) => format!(
            "A samples {}, B width {:.3}, C word ratio {}, words {}, spinner {}, centre x {:.3}, centre y {:.3}, height {:.3}",
            t.samples,
            t.width,
            t.word_ratio
                .map_or("none".to_string(), |r| format!("{r:.3}")),
            t.words,
            t.spinner,
            t.centre_x,
            t.centre_y,
            t.height
        ),
        None => "A: no text at the glyph level".to_string(),
    }
}

#[test]
fn the_loading_rule_on_local_frames() {
    for (name, rgb) in frames() {
        let entering = name.starts_with("entering_game");
        let update = name.contains("applying_update");
        if !entering && !update {
            continue;
        }
        let pass = loading_rule(&rgb);
        println!(
            "{name}: slab {}, loading rule {pass}; {}",
            is_loading_screen(&rgb),
            describe_text(&rgb)
        );
        if update {
            assert!(
                !pass,
                "{name}: an APPLYING UPDATE band passed the loading rule; {}",
                describe_text(&rgb)
            );
            continue;
        }
        assert!(is_loading_screen(&rgb), "{name}: not the loading slab");
        if text_box(&rgb).is_some() {
            assert!(
                pass,
                "{name}: ENTERING GAME text failed the loading rule; {}",
                describe_text(&rgb)
            );
        }
    }
}

#[test]
fn the_roster_check_on_local_frames() {
    for (name, rgb) in frames() {
        let s = scores(&rgb);
        let (w, h) = rgb.dimensions();
        println!("{name} ({w}x{h}): {}", s.describe());
        assert_eq!(
            s.is_roster(),
            name.starts_with("roster"),
            "{name}: {}",
            s.describe()
        );
    }
}
