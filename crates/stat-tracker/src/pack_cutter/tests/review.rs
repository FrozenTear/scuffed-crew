//! Review-round tests. Helpers live in the parent test module.

use image::{Rgb, RgbImage};

use super::{
    Family, RankSet, assert_posix_ustar, block, cut_queue, cut_rank, draw, member_bytes,
    paint_gate, queue_row, rank_row, role_row, run, save, scratch, write, write_queue, write_rank,
};

fn paint_gate_word(word: &str) -> RgbImage {
    let mut img = RgbImage::from_pixel(2560, 1440, Rgb([40, 44, 60]));
    draw(&mut img, word, 842, 1068, 3, Rgb([250, 250, 250]));
    img
}

fn family_vectors(set: &RankSet, family: Family) -> Vec<(String, Vec<f32>)> {
    let mut rows: Vec<_> = set
        .families
        .get(&family)
        .map(|templates| {
            templates
                .iter()
                .map(|t| (t.class.clone(), t.vector.clone()))
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

#[test]
fn cut_base_does_not_replace_unobserved_numeric_classes() {
    use crate::shadow::rank::synth::{self, Scene};
    let root = scratch();
    let frames = root.path().join("frames");
    let sc = Scene::default();
    save(&frames, "20260101-a/a.png", &synth::paint(&sc));
    let full = root.path().join("full.jsonl");
    let mut value = serde_json::to_value(synth::labels(&sc)).unwrap();
    value["game"] = serde_json::json!("20260101-a");
    value["file"] = serde_json::json!("a.png");
    value["readable"] = serde_json::json!(true);
    write(&full, &format!("{}\n", value));
    let base = root.path().join("base");
    let first = run(&[
        "cut",
        "--kind",
        "rank",
        "--labels",
        full.to_str().unwrap(),
        "--frames",
        frames.to_str().unwrap(),
        "--out",
        base.to_str().unwrap(),
    ]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let pct = root.path().join("pct.jsonl");
    write(
        &pct,
        &format!(
            "{}\n",
            serde_json::json!({
                "game": "20260101-a",
                "file": "a.png",
                "readable": true,
                "progress_pct": sc.pct,
            })
        ),
    );
    let out = root.path().join("out");
    let second = run(&[
        "cut",
        "--kind",
        "rank",
        "--labels",
        pct.to_str().unwrap(),
        "--frames",
        frames.to_str().unwrap(),
        "--base",
        base.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert!(
        second
            .stdout
            .contains("class sr/1: kept from base (pooled)"),
        "{}",
        second.stdout
    );
    let base_set = RankSet::load_dir(&base).unwrap();
    let kept = RankSet::load_dir(&out).unwrap();
    for family in [Family::Sr, Family::Change, Family::Higher, Family::Pill] {
        assert_eq!(
            family_vectors(&kept, family),
            family_vectors(&base_set, family),
            "{family:?}"
        );
    }
}

#[test]
fn eval_queue_swapped_titles_fail_without_a_dropped_class() {
    let root = scratch();
    let files = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "BRAVO QUEUE"),
        ],
        false,
    );
    let swapped = root.path().join("swapped.jsonl");
    write(
        &swapped,
        &format!(
            "{}{}",
            queue_row("s1/a.png", "BRAVO QUEUE"),
            queue_row("s2/b.png", "ALPHA QUEUE")
        ),
    );
    let baseline = root.path().join("baseline");
    let candidate = root.path().join("candidate");
    assert_eq!(cut_queue(&files, &baseline).code, 0);
    let cut = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        swapped.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--out",
        candidate.to_str().unwrap(),
    ]);
    assert_eq!(cut.code, 0, "{}", cut.stderr);
    let got = run(&[
        "eval",
        "--kind",
        "queue-title",
        "--labels",
        files.labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--baseline",
        baseline.to_str().unwrap(),
        "--candidate",
        candidate.to_str().unwrap(),
        "--set-name",
        "swapped",
    ]);
    assert_eq!(got.code, 1, "{}\n{}", got.stderr, got.stdout);
    assert!(!got.stdout.contains("dropped classes:"), "{}", got.stdout);
    assert!(got.stdout.contains("result: fail"), "{}", got.stdout);
    assert!(got.stdout.contains("regressed: 1"), "{}", got.stdout);
}

#[test]
fn eval_queue_new_wrong_on_a_title_missing_from_the_baseline_fails() {
    let root = scratch();
    let baseline_files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let candidate_files = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "BRAVO QUEUE"),
        ],
        false,
    );
    let eval_files = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "CHARLIE QUEUE"),
        ],
        false,
    );
    let baseline = root.path().join("baseline");
    let candidate = root.path().join("candidate");
    assert_eq!(cut_queue(&baseline_files, &baseline).code, 0);
    assert_eq!(cut_queue(&candidate_files, &candidate).code, 0);
    let got = run(&[
        "eval",
        "--kind",
        "queue-title",
        "--labels",
        eval_files.labels.to_str().unwrap(),
        "--frames",
        eval_files.frames.to_str().unwrap(),
        "--baseline",
        baseline.to_str().unwrap(),
        "--candidate",
        candidate.to_str().unwrap(),
        "--set-name",
        "new-title",
        "--allow-unverified",
        "BRAVO QUEUE",
    ]);
    assert_eq!(got.code, 1, "{}\n{}", got.stderr, got.stdout);
    assert!(
        got.stdout
            .contains("class CHARLIE QUEUE (not in the baseline pack)"),
        "{}",
        got.stdout
    );
    let charlie = block(
        &got.stdout,
        "class CHARLIE QUEUE (not in the baseline pack)\n",
    );
    assert!(charlie.contains("regressed: 1"), "{charlie}");
    assert!(got.stdout.contains("result: fail"), "{}", got.stdout);
}

#[test]
fn eval_rank_gate_word_change_is_correct_to_wrong() {
    let root = scratch();
    let base_frames = root.path().join("base-frames");
    let next_frames = root.path().join("next-frames");
    save(&base_frames, "20260101-a/a.png", &paint_gate(2560, 1440));
    save(&next_frames, "20260101-a/a.png", &paint_gate_word("HHHHHH"));
    let labels = root.path().join("labels.jsonl");
    write(&labels, &rank_row("20260101-a", "a.png", true));
    let baseline = root.path().join("baseline");
    let candidate = root.path().join("candidate");
    for (frames, out) in [(&base_frames, &baseline), (&next_frames, &candidate)] {
        let got = run(&[
            "cut",
            "--kind",
            "rank",
            "--labels",
            labels.to_str().unwrap(),
            "--frames",
            frames.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        assert_eq!(got.code, 0, "{}", got.stderr);
    }
    let got = run(&[
        "eval",
        "--kind",
        "rank",
        "--labels",
        labels.to_str().unwrap(),
        "--frames",
        base_frames.to_str().unwrap(),
        "--baseline",
        baseline.to_str().unwrap(),
        "--candidate",
        candidate.to_str().unwrap(),
        "--set-name",
        "gate-word",
    ]);
    assert_eq!(got.code, 1, "{}\n{}", got.stderr, got.stdout);
    assert!(!got.stdout.contains("dropped classes:"), "{}", got.stdout);
    let gate = block(&got.stdout, "screen gate\n");
    assert!(
        gate.contains("baseline: 1 / 0 / 0\ncandidate: 0 / 1 / 0"),
        "{gate}"
    );
    assert!(
        gate.contains("regressed frames: 20260101-a/a.png"),
        "{gate}"
    );
    assert!(got.stdout.contains("result: fail"), "{}", got.stdout);
}

#[test]
fn pack_regressed_truth_table() {
    use super::super::report::{Item, Outcome};
    let outcomes = [Outcome::Correct, Outcome::Wrong, Outcome::Unread];
    for baseline in outcomes {
        for packed in outcomes {
            let item = Item {
                frame: "g/f".into(),
                baseline,
                learned: baseline,
                packed,
            };
            let expect = matches!(
                (baseline, packed),
                (Outcome::Correct, Outcome::Wrong)
                    | (Outcome::Correct, Outcome::Unread)
                    | (Outcome::Unread, Outcome::Wrong)
            );
            assert_eq!(
                Item::pack_regressed(&item),
                expect,
                "{baseline:?} {packed:?}"
            );
        }
    }
}

#[test]
fn offsetting_frame_changes_fail_and_name_the_frames() {
    use super::super::report::{self, Item, Outcome, ScoreRow};
    let row = ScoreRow {
        label: "field role".into(),
        folds: 1,
        labelled: 2,
        items: vec![
            Item {
                frame: "day-a/a.png".into(),
                baseline: Outcome::Correct,
                learned: Outcome::Correct,
                packed: Outcome::Wrong,
            },
            Item {
                frame: "day-b/b.png".into(),
                baseline: Outcome::Wrong,
                learned: Outcome::Wrong,
                packed: Outcome::Correct,
            },
        ],
    };
    let done = report::finish(&report::Inputs {
        set_name: "offset",
        frames: 2,
        scored: 2,
        skipped: 0,
        rows: &[row],
        dropped: &[],
        allowed: &[],
        unverified: &[],
        changed: &[],
        class_folds: &[("gate/rank_progress".into(), 1)],
        candidate_sha: "abc",
    });
    assert!(!done.pass);
    assert!(
        done.report.contains("baseline: 1 / 1 / 0"),
        "{}",
        done.report
    );
    assert!(
        done.report.contains("candidate: 1 / 1 / 0"),
        "{}",
        done.report
    );
    assert!(
        done.report.contains("regressed frames: day-a/a.png"),
        "{}",
        done.report
    );
    let result = done.report.rfind("result:").unwrap();
    let folds = done
        .report
        .find("template class")
        .unwrap_or(done.report.len());
    assert!(folds < result, "class folds must come before result");
    assert!(done.report.trim_end().ends_with("result: fail"));
}

#[test]
fn cut_refuses_a_path_that_walks_out_with_dotdot() {
    let scratch_dir = scratch();
    let repo = scratch_dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir(repo.join(".git")).unwrap();
    let files = write_rank(&["20260101"], (2560, 1440), false);
    let tricky = scratch_dir.path().join("dotdot/missing/../repo/sub");
    let got = cut_rank(&files, &tricky);
    assert_eq!(got.code, 2, "{}", got.stderr);
    assert!(got.stderr.contains(".."), "{}", got.stderr);
    assert!(!repo.join("sub").exists());
}

#[test]
fn cut_refuses_loose_out_same_dir_as_base_and_an_existing_templates_file() {
    let files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let base = files.dir.path().join("base");
    assert_eq!(cut_queue(&files, &base).code, 0);
    let same = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        files.labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--base",
        base.to_str().unwrap(),
        "--out",
        base.to_str().unwrap(),
    ]);
    assert_eq!(same.code, 2, "{}", same.stderr);
    assert!(same.stderr.contains("same folder"), "{}", same.stderr);
    let again = cut_queue(&files, &base);
    assert_eq!(again.code, 2, "{}", again.stderr);
    assert!(
        again.stderr.contains("already holds templates.json"),
        "{}",
        again.stderr
    );
    let loose = files.dir.path().join("loose");
    std::fs::create_dir(&loose).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let got = cut_queue(&files, &loose);
    assert_eq!(got.code, 2, "{}", got.stderr);
    assert!(
        got.stderr.contains("readable by group or other"),
        "{}",
        got.stderr
    );
}

#[test]
fn cut_refuses_spaced_titles_unknown_keys_and_duplicate_roles() {
    let files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let spaced = files.dir.path().join("spaced.jsonl");
    write(&spaced, &queue_row("s1/a.png", "ALPHA QUEUE "));
    let got = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        spaced.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--out",
        files.dir.path().join("spaced-out").to_str().unwrap(),
    ]);
    assert_eq!(got.code, 2, "{}", got.stderr);
    assert!(
        got.stderr.contains("doubled spaces") || got.stderr.contains("trailing"),
        "{}",
        got.stderr
    );

    let rank = write_rank(&["20260101"], (2560, 1440), false);
    let bad = rank.dir.path().join("bad.jsonl");
    write(
        &bad,
        &format!(
            "{}\n",
            serde_json::json!({"game": "20260101-game", "file": "a.png", "readable": true, "hero_sr_typo": 1})
        ),
    );
    let got = run(&[
        "cut",
        "--kind",
        "rank",
        "--labels",
        bad.to_str().unwrap(),
        "--frames",
        rank.frames.to_str().unwrap(),
        "--out",
        rank.dir.path().join("bad-out").to_str().unwrap(),
    ]);
    assert_eq!(got.code, 2, "{}", got.stderr);
    assert!(got.stderr.contains("unknown key"), "{}", got.stderr);

    let roles = rank.dir.path().join("dup-roles.jsonl");
    write(
        &roles,
        &format!("{}{}", role_row("20260101-game"), role_row("20260101-game")),
    );
    let got = run(&[
        "cut",
        "--kind",
        "rank",
        "--labels",
        rank.labels.to_str().unwrap(),
        "--frames",
        rank.frames.to_str().unwrap(),
        "--role-labels",
        roles.to_str().unwrap(),
        "--out",
        rank.dir.path().join("dup-out").to_str().unwrap(),
    ]);
    assert_eq!(got.code, 2, "{}", got.stderr);
    assert!(got.stderr.contains("twice"), "{}", got.stderr);
}

#[test]
fn cut_refuses_near_duplicate_queue_titles() {
    let files = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 0, "BRAVO QUEUE"),
        ],
        false,
    );
    let out = files.dir.path().join("out");
    let got = cut_queue(&files, &out);
    assert_eq!(got.code, 2, "{}", got.stderr);
    assert!(got.stderr.contains("correlates"), "{}", got.stderr);
}

#[test]
fn dash_h_as_a_flag_value_is_not_help_and_non_utf8_is_an_error() {
    let got = run(&["eval", "--set-name", "-h"]);
    assert_eq!(got.code, 2, "{}", got.stdout);
    assert!(got.stdout.is_empty(), "{}", got.stdout);
    assert!(got.stderr.contains("missing --"), "{}", got.stderr);

    use std::os::unix::ffi::OsStringExt;
    let args = vec![
        std::ffi::OsString::from("pack"),
        std::ffi::OsString::from("--version"),
        std::ffi::OsString::from_vec(vec![0xff, 0xfe]),
    ];
    let got = super::super::execute(&args);
    assert_eq!(got.code, 2, "{}", got.stderr);
    assert!(got.stderr.contains("UTF-8"), "{}", got.stderr);
}

#[test]
fn archive_refuses_duplicates_and_round_trips_through_read_ustar() {
    use super::super::ustar::{self, Member};
    let member = |bytes: &[u8]| Member {
        path: "templates.json".into(),
        bytes: bytes.to_vec(),
    };
    let dup = ustar::archive("rank-v1.tar", "1", &[member(b"{}"), member(b"{}")]).unwrap_err();
    assert!(dup.contains("duplicate"), "{dup}");
    let reserved = ustar::archive(
        "rank-v1.tar",
        "1",
        &[Member {
            path: "manifest.json".into(),
            bytes: b"{}".to_vec(),
        }],
    )
    .unwrap_err();
    assert!(reserved.contains("manifest.json"), "{reserved}");
    let long = ustar::archive(
        "rank-v1.tar",
        "1",
        &[Member {
            path: "a".repeat(101),
            bytes: b"{}".to_vec(),
        }],
    )
    .unwrap_err();
    assert!(long.contains("longer than 100"), "{long}");
    for len in [0usize, 1, 511, 512] {
        let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8 + 1).collect();
        let tar = ustar::archive(
            "rank-v1.tar",
            "1",
            &[Member {
                path: "templates.json".into(),
                bytes: data.clone(),
            }],
        )
        .unwrap();
        assert_posix_ustar(&tar);
        let files = crate::packs::read_ustar(&tar, crate::packs::PackLayout::TemplatesJson)
            .unwrap_or_else(|e| panic!("len {len}: {e}"));
        assert_eq!(member_bytes(&files, "templates.json"), data);
    }
}
