//! End-reel / POTG wake fixtures.
//!
//! `tests/fixtures/end-reel/nameplate_potg_synthetic.png` is a committed
//! (non-capture) layout that reproduces the known miss: no letterbox, no
//! cinematic title-band, but PLAY OF THE GAME + an orange nameplate.
//!
//! Copyrighted captures stay gitignored under `test-data/end-reel/`:
//! - `known-potg-003708.png` — real nameplate; **must still wake**
//! - `entering_game.png` / `ban_heroes.png` / `scoreboard_tab.png` —
//!   0.4.10 batch false wakes; must **not** wake. Ban Heroes must still
//!   trip [`stat_tracker::detect::match_start::detect_ban_screen`].
//! Missing files skip — CI stays green.

use stat_tracker::detect::match_end::detect_end_reel;
use stat_tracker::detect::match_start::detect_ban_screen;

fn crate_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn open(rel: &str) -> Option<image::DynamicImage> {
    let path = crate_path(rel);
    match image::open(&path) {
        Ok(img) => Some(img),
        Err(_) => {
            eprintln!("skip (missing): {}", path.display());
            None
        }
    }
}

#[test]
fn synthetic_nameplate_potg_wakes_end_reel() {
    let path = "tests/fixtures/end-reel/nameplate_potg_synthetic.png";
    let img = image::open(crate_path(path)).unwrap_or_else(|e| panic!("open {path}: {e}"));
    let rgb = img.to_rgb8();
    assert!(
        detect_end_reel(&img, &rgb),
        "nameplate POTG title card must wake (letterbox + cinematic band miss this layout)"
    );
}

#[test]
fn known_potg_003708_wakes_if_present() {
    let Some(img) = open("test-data/end-reel/known-potg-003708.png") else {
        return;
    };
    let rgb = img.to_rgb8();
    assert!(
        detect_end_reel(&img, &rgb),
        "real nameplate POTG frame known-potg-003708 must wake"
    );
}

#[test]
fn entering_game_does_not_wake_if_present() {
    let Some(img) = open("test-data/end-reel/entering_game.png") else {
        return;
    };
    let rgb = img.to_rgb8();
    assert!(
        !detect_end_reel(&img, &rgb),
        "ENTERING GAME loading must not end-reel-wake"
    );
}

#[test]
fn ban_heroes_does_not_wake_if_present() {
    let Some(img) = open("test-data/end-reel/ban_heroes.png") else {
        return;
    };
    let rgb = img.to_rgb8();
    assert!(
        detect_ban_screen(&img, &rgb),
        "Ban Heroes must remain a first-class hook (future ban OCR)"
    );
    assert!(
        !detect_end_reel(&img, &rgb),
        "Ban Heroes must not set end_reel_wake_until"
    );
}

#[test]
fn scoreboard_tab_does_not_wake_if_present() {
    let Some(img) = open("test-data/end-reel/scoreboard_tab.png") else {
        return;
    };
    let rgb = img.to_rgb8();
    assert!(
        !detect_end_reel(&img, &rgb),
        "Tab scoreboard crop must not nameplate-wake"
    );
}
