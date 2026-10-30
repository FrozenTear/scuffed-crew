//! Opt-in evidence for the loading check's bounds on real frames: the
//! `ENTERING GAME` text box and the `APPLYING UPDATE` band. No real frame
//! is in the repo (they show the screen as it was), so this reads a local
//! folder named by `SCUFFED_LOADING_FRAMES_DIR` and prints, for each
//! `.png` in it, both loading checks and the measured text box. It asserts
//! nothing about the frames and skips when the variable is unset. Run it
//! with
//!
//! ```text
//! SCUFFED_LOADING_FRAMES_DIR=/path/to/frames \
//!   cargo test -p scuffed-stat-tracker --test integration \
//!   loading_frames -- --ignored --nocapture
//! ```
//!
//! and compare each box with the bounds in `detect/loading_screen.rs`
//! (centre x 0.45 to 0.55, width 0.25 to 0.50, centre y 0.44 to 0.56,
//! height at most 0.10).

use stat_tracker::detect::loading_screen::{is_entering_game, is_loading_screen, text_box};

#[test]
#[ignore = "opt-in: reads real frames from SCUFFED_LOADING_FRAMES_DIR"]
fn report_the_loading_checks_on_local_frames() {
    let Some(dir) = std::env::var_os("SCUFFED_LOADING_FRAMES_DIR").filter(|d| !d.is_empty()) else {
        eprintln!("skip: SCUFFED_LOADING_FRAMES_DIR is not set");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let mut pngs: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .collect();
    pngs.sort();
    if pngs.is_empty() {
        eprintln!("skip: no .png in {}", dir.display());
        return;
    }
    for path in pngs {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let rgb = match image::open(&path) {
            Ok(img) => img.to_rgb8(),
            Err(e) => {
                println!("{name}: not read ({e})");
                continue;
            }
        };
        let (w, h) = rgb.dimensions();
        let text = match text_box(&rgb) {
            Some(t) => format!(
                "width {:.3}, centre x {:.3}, centre y {:.3}, height {:.3}, {} samples",
                t.width, t.centre_x, t.centre_y, t.height, t.samples
            ),
            None => "no text".to_string(),
        };
        println!(
            "{name} ({w}x{h}): loading {}, entering game {}; text: {text}",
            is_loading_screen(&rgb),
            is_entering_game(&rgb)
        );
    }
}
