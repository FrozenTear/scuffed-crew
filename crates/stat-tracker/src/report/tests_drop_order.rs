//! Section 8 drop order: over 30 files, rank crops leave first, then glyph
//! crops two at a time (the name seen on the latest frame first). The log,
//! scoreboard, outcome crop and own-name crop never leave to fit the count.
//! Also the preview line for every left-out file.

use super::bundle::{MAX_FILES, ReportError};
use super::fit::{CountKind, LeftOut, LeftOutReason, count_drops};

const KEPT: CountKind = CountKind::Kept;
const RANK: CountKind = CountKind::Rank;

const fn glyph(name: u32, last_seen_frame: u64) -> CountKind {
    CountKind::Glyph {
        name,
        last_seen_frame,
    }
}

/// log, scoreboard, outcome, own-name, then the two rank crops, then
/// `names` names of two glyphs each; name `n` was last seen on frame `n`.
fn full_report(names: u32) -> Vec<CountKind> {
    let mut kinds = vec![KEPT, KEPT, KEPT, KEPT, RANK, RANK];
    for n in 0..names {
        kinds.extend([glyph(n, u64::from(n)), glyph(n, u64::from(n))]);
    }
    kinds
}

fn kept_after(kinds: &[CountKind], drops: &[usize]) -> Vec<CountKind> {
    kinds
        .iter()
        .enumerate()
        .filter(|(i, _)| !drops.contains(i))
        .map(|(_, k)| *k)
        .collect()
}

#[test]
fn a_report_that_fits_drops_nothing() {
    assert!(count_drops(&full_report(12), MAX_FILES).unwrap().is_empty());
    assert_eq!(full_report(12).len(), MAX_FILES);
    assert!(count_drops(&[KEPT], MAX_FILES).unwrap().is_empty());
}

#[test]
fn one_over_drops_one_rank_crop_first() {
    // 4 kept + 2 rank + 13 names x 2 glyphs = 32: two over
    let kinds = full_report(13);
    assert_eq!(kinds.len(), 32);
    assert_eq!(count_drops(&kinds, MAX_FILES).unwrap(), vec![4, 5]);
    // 31 files: one over, only rank.png leaves
    let mut kinds = full_report(12);
    kinds.push(KEPT);
    assert_eq!(count_drops(&kinds, MAX_FILES).unwrap(), vec![4]);
}

#[test]
fn past_the_rank_crops_glyphs_leave_in_pairs_latest_frame_first() {
    // 4 kept + 2 rank + 14 names x 2 = 34: four over
    let kinds = full_report(14);
    let drops = count_drops(&kinds, MAX_FILES).unwrap();
    // both rank crops, then the pair of name 13 (seen on frame 13, the latest)
    assert_eq!(drops, vec![4, 5, 32, 33]);
    // 35 files, five over: rank crops, then names 13 and 12, a pair each,
    // so 29 files are left (a pair is never split)
    let mut kinds = full_report(14);
    kinds.push(KEPT);
    let drops = count_drops(&kinds, MAX_FILES).unwrap();
    assert_eq!(drops, vec![4, 5, 32, 33, 30, 31]);
    assert_eq!(kept_after(&kinds, &drops).len(), MAX_FILES - 1);
}

#[test]
fn glyph_order_follows_the_name_frame_not_the_file_order() {
    // Glyphs come shuffled (section 5): name 7 seen last, on frame 90.
    let kinds = vec![
        KEPT,
        glyph(3, 10),
        glyph(7, 90),
        glyph(5, 40),
        glyph(3, 10),
        glyph(5, 40),
        glyph(7, 90),
    ];
    assert_eq!(count_drops(&kinds, 5).unwrap(), vec![2, 6]);
    assert_eq!(count_drops(&kinds, 3).unwrap(), vec![2, 6, 3, 5]);
    // a tie on the frame goes to the lower name
    let tie = vec![KEPT, glyph(2, 5), glyph(1, 5), glyph(2, 5), glyph(1, 5)];
    assert_eq!(count_drops(&tie, 4).unwrap(), vec![2, 4]);
}

#[test]
fn kept_files_never_leave_to_fit_the_count() {
    for names in 13..=40 {
        let kinds = full_report(names);
        let drops = count_drops(&kinds, MAX_FILES).unwrap();
        let left = kept_after(&kinds, &drops);
        assert!(left.len() <= MAX_FILES, "{names}");
        assert_eq!(left.iter().filter(|k| **k == KEPT).count(), 4, "{names}");
        // rank crops always leave before any glyph
        assert!(drops.starts_with(&[4, 5]), "{names}: {drops:?}");
        assert!(drops.iter().all(|&i| kinds[i] != KEPT), "{names}");
    }
    // Fail-safe only: more kept files than the cap can never fit.
    let all_kept = vec![KEPT; MAX_FILES + 1];
    assert!(matches!(
        count_drops(&all_kept, MAX_FILES),
        Err(ReportError::InvalidInput("a report holds at most 30 files"))
    ));
    // Dropping every rank crop and glyph still is not enough: also an error.
    let mut mostly_kept = vec![KEPT; MAX_FILES + 1];
    mostly_kept.extend([RANK, glyph(0, 0), glyph(0, 0)]);
    assert!(count_drops(&mostly_kept, MAX_FILES).is_err());
}

#[test]
fn every_left_out_file_has_one_plain_preview_line() {
    let line = |path: &str, reason| {
        LeftOut {
            path: path.into(),
            reason,
        }
        .preview_line()
    };
    assert_eq!(
        line("crops/rank.png", LeftOutReason::OverSizeLimit),
        "Rank screen left out: over the size limit."
    );
    assert_eq!(
        line("crops/rank_banner.png", LeftOutReason::ReportTooBig),
        "Rank banner left out: the report would be over 10 MB."
    );
    assert_eq!(
        line("crops/rank.png", LeftOutReason::TooManyFiles),
        "Rank screen left out: too many files for one report."
    );
    assert_eq!(
        line("crops/rank_banner.png", LeftOutReason::NotReadable),
        "Rank banner left out: the picture could not be read."
    );
    assert_eq!(
        line("glyphs/3f9a.png", LeftOutReason::TooManyFiles),
        "A name letter left out: too many files for one report."
    );
}
