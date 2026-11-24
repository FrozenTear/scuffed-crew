//! Real-pixel replay for the accolade / endcards result word (C6, fleet::tracker-wl).
//!
//! Frames are copyrighted captures and stay gitignored under
//! `test-data/accolade/`. Missing files skip — CI stays green. Locally:
//!   defeat_endcards_magenta_2560.png  — USER's 2026-07-15 endcards (large title font)
//!   victory_accolade_1650.png         — 2026-05-30 accolade reference
//!   none_endtitle_is_not_accolade.png — end-title frame: must read via EndTitle,
//!                                       never via the accolade crop

use stat_tracker::detect::MatchOutcome;
use stat_tracker::detect::match_end::{detect_outcome, detect_outcome_signal};

fn open(rel: &str) -> Option<image::DynamicImage> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    match image::open(&path) {
        Ok(img) => Some(img),
        Err(_) => {
            eprintln!("skip (missing): {}", path.display());
            None
        }
    }
}

#[test]
fn accolade_defeat_large_font_if_present() {
    let Some(img) = open("test-data/accolade/defeat_endcards_magenta_2560.png") else {
        return;
    };
    let signal = detect_outcome_signal(&img);
    assert_eq!(
        signal.map(|(o, _)| o),
        Some(MatchOutcome::Defeat),
        "signal={signal:?}"
    );
    assert_eq!(
        signal.map(|(_, s)| format!("{s:?}")).as_deref(),
        Some("ResultWord"),
        "expected accolade ResultWord source, got {signal:?}"
    );
    assert_eq!(detect_outcome(&img), MatchOutcome::Defeat);
}

#[test]
fn accolade_victory_reference_if_present() {
    let Some(img) = open("test-data/accolade/victory_accolade_1650.png") else {
        return;
    };
    let signal = detect_outcome_signal(&img);
    assert_eq!(
        signal.map(|(o, _)| o),
        Some(MatchOutcome::Victory),
        "signal={signal:?}"
    );
    assert_eq!(
        signal.map(|(_, s)| format!("{s:?}")).as_deref(),
        Some("ResultWord"),
        "expected accolade ResultWord source, got {signal:?}"
    );
}

#[test]
fn end_title_frame_does_not_read_via_accolade_crop_if_present() {
    let Some(img) = open("test-data/accolade/none_endtitle_is_not_accolade.png") else {
        return;
    };
    let signal = detect_outcome_signal(&img);
    assert_eq!(
        signal.map(|(o, s)| (o, format!("{s:?}"))),
        Some((MatchOutcome::Victory, "EndTitle".to_string())),
        "signal={signal:?}"
    );
}
