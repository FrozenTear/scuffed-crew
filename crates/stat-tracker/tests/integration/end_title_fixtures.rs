//! Real-pixel replay for the in-world end-title overlay (H3).
//!
//! Frames are copyrighted captures and stay gitignored under
//! `test-data/end-title/`. Missing files skip — CI stays green.

use stat_tracker::detect::MatchOutcome;
use stat_tracker::detect::match_end::{detect_outcome, detect_outcome_signal, detect_outcome_text};

fn crate_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

#[test]
fn end_title_victory_magenta_blizzard_if_present() {
    let path = crate_path("test-data/end-title/victory_blizzard_magenta.png");
    let Ok(img) = image::open(&path) else {
        eprintln!("skip (missing): {}", path.display());
        return;
    };
    let signal = detect_outcome_signal(&img);
    assert_eq!(
        signal.map(|(o, _)| o),
        Some(MatchOutcome::Victory),
        "poll path signal={signal:?}"
    );
    assert_eq!(
        signal.map(|(_, s)| format!("{s:?}")).as_deref(),
        Some("EndTitle"),
        "expected EndTitle source, got {signal:?}"
    );
    assert_eq!(detect_outcome(&img), MatchOutcome::Victory);
    assert_eq!(detect_outcome_text(&img), MatchOutcome::Victory);
}
