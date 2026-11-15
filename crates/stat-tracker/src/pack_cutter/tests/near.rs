//! The near-pair check and `--allow-near`. Helpers live in the parent test
//! module. Synthetic frames and hand-built templates only.

use std::collections::BTreeMap;

use super::super::util::{ClassFate, ClassNote};
use super::{Family, QueueSet, RankSet, cut_queue, run, write_queue};

fn cut_queue_with(
    files: &super::QueueFiles,
    out: &std::path::Path,
    extra: &[&str],
) -> super::super::Outcome {
    let mut args = vec![
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        files.labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ];
    args.extend(extra);
    run(&args)
}

#[test]
fn a_near_pair_is_refused_then_written_and_listed_with_allow_near() {
    let files = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 0, "BRAVO QUEUE"),
        ],
        false,
    );
    let out = files.dir.path().join("out");
    let refused = cut_queue(&files, &out);
    assert_eq!(refused.code, 2, "{}", refused.stdout);
    assert!(
        refused
            .stderr
            .contains("class ALPHA QUEUE correlates 1.000 with BRAVO QUEUE"),
        "{}",
        refused.stderr
    );
    assert!(
        refused.stderr.contains("--allow-near"),
        "{}",
        refused.stderr
    );
    assert!(!out.join("templates.json").exists());

    let allowed = cut_queue_with(&files, &out, &["--allow-near"]);
    assert_eq!(allowed.code, 0, "{}", allowed.stderr);
    assert!(
        allowed.stdout.contains(
            "allowed near pair: class ALPHA QUEUE correlates 1.000 with BRAVO QUEUE, above 0.850"
        ),
        "{}",
        allowed.stdout
    );
    assert_eq!(QueueSet::load_dir(&out).unwrap().classes.len(), 2);
}

#[test]
fn a_near_pair_of_kept_base_classes_does_not_block_a_recut() {
    let near = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 0, "BRAVO QUEUE"),
        ],
        false,
    );
    let base = near.dir.path().join("base");
    assert_eq!(cut_queue_with(&near, &base, &["--allow-near"]).code, 0);
    let other = write_queue(&[("s3/c.png", 1, "CHECK QUEUE")], false);
    let out = near.dir.path().join("out");
    let got = cut_queue_with(&other, &out, &["--base", base.to_str().unwrap()]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(
        got.stdout.contains("class ALPHA QUEUE: kept from base"),
        "{}",
        got.stdout
    );
    assert!(!got.stdout.contains("allowed near"), "{}", got.stdout);
    assert_eq!(QueueSet::load_dir(&out).unwrap().classes.len(), 3);
}

fn rank_template(class: &str, vector: &[f32]) -> crate::shadow::rank::templates::ClassTemplate {
    crate::shadow::rank::templates::ClassTemplate {
        class: class.into(),
        aspect: 0.6,
        n: 1,
        vector: vector.to_vec(),
        pooled: false,
    }
}

#[test]
fn rank_near_check_skips_classes_the_reader_does_not_offer() {
    let reject = super::super::rank::reject_near;
    let v = [0.6, 0.8];
    let set_of = |other: &str| RankSet {
        recognizer: crate::shadow::rank::RECOGNIZER_ID.into(),
        families: BTreeMap::from([(
            Family::Pct,
            vec![rank_template("7", &v), rank_template(other, &v)],
        )]),
    };
    let notes = |other: &str| -> Vec<ClassNote> {
        vec![
            ("pct/7".into(), ClassFate::New, 0, 1),
            (format!("pct/{other}"), ClassFate::Kept, 1, 1),
        ]
    };
    // the reader never offers sr_icon in pct, so it never ranks it against 7
    let icon = crate::shadow::rank::SR_ICON;
    assert_eq!(
        reject(&set_of(icon), &notes(icon), false),
        Ok(String::new())
    );
    let err = reject(&set_of("%"), &notes("%"), false).unwrap_err();
    assert!(
        err.contains("class pct/7 correlates 1.000 with pct/%"),
        "{err}"
    );
    assert!(err.contains("--allow-near"), "{err}");
    assert!(err.contains("aspect term"), "{err}");
    let listed = reject(&set_of("%"), &notes("%"), true).unwrap();
    assert!(
        listed.starts_with("allowed near pair: class pct/7 correlates 1.000 with pct/%"),
        "{listed}"
    );
}

#[test]
fn rank_near_check_skips_a_pair_of_kept_classes() {
    let reject = super::super::rank::reject_near;
    let v = [0.6, 0.8];
    let set = RankSet {
        recognizer: crate::shadow::rank::RECOGNIZER_ID.into(),
        families: BTreeMap::from([(
            Family::Pct,
            vec![rank_template("7", &v), rank_template("%", &v)],
        )]),
    };
    let notes = |fate: ClassFate| -> Vec<ClassNote> {
        vec![
            ("pct/7".into(), fate, 1, 1),
            ("pct/%".into(), ClassFate::Kept, 1, 1),
        ]
    };
    // neither class is new or re-learned, so the pair is not checked
    assert_eq!(
        reject(&set, &notes(ClassFate::Kept), false),
        Ok(String::new())
    );
    assert_eq!(
        reject(&set, &notes(ClassFate::KeptPooled), false),
        Ok(String::new())
    );
    let err = reject(&set, &notes(ClassFate::Learned), false).unwrap_err();
    assert!(
        err.contains("class pct/7 correlates 1.000 with pct/%"),
        "{err}"
    );
}

#[test]
fn queue_near_check_uses_ncc_not_cosine() {
    use crate::shadow::queue_title::{TH, TW, templates::ClassTemplate};
    let pattern = |i: usize| {
        if (i / TW + i % TW).is_multiple_of(2) {
            0.1
        } else {
            -0.1
        }
    };
    let up: Vec<f32> = (0..TW * TH).map(|i| 1.0 + pattern(i)).collect();
    let down: Vec<f32> = (0..TW * TH).map(|i| 1.0 - pattern(i)).collect();
    let dot: f32 = up.iter().zip(&down).map(|(a, b)| a * b).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let cosine = dot / (norm(&up) * norm(&down));
    let limit = 1.0 - crate::shadow::queue_title::MIN_MARGIN;
    assert!(cosine > limit, "cosine {cosine}");
    let template = |class: &str, vector: Vec<f32>| ClassTemplate {
        class: class.into(),
        n: 1,
        vector,
    };
    let set = QueueSet {
        recognizer: crate::shadow::queue_title::RECOGNIZER_ID.into(),
        width: TW,
        height: TH,
        classes: vec![template("ALPHA QUEUE", up), template("BRAVO QUEUE", down)],
    };
    let notes: Vec<ClassNote> = vec![
        ("ALPHA QUEUE".into(), ClassFate::New, 0, 1),
        ("BRAVO QUEUE".into(), ClassFate::New, 0, 1),
    ];
    assert_eq!(
        super::super::queue::reject_near(&set, &notes, false),
        Ok(String::new())
    );
}
