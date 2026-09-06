//! End-reel / POTG wake fixtures.
//!
//! `tests/fixtures/end-reel/nameplate_potg_synthetic.png` is a committed
//! (non-capture) layout that reproduces the known miss: no letterbox, no
//! cinematic title-band, but PLAY OF THE GAME + an orange nameplate.
//!
//! The real 2560×1440 Overwatch nameplate frame (`known-potg-003708.png`)
//! is a copyrighted capture. Drop it at
//! `test-data/end-reel/known-potg-003708.png` (gitignored). Missing files
//! skip — CI stays green.

use stat_tracker::detect::match_end::detect_end_reel;

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
