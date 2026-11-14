//! Review-round tests. Helpers live in the parent test module.

use image::{Rgb, RgbImage};

use super::{
    Family, QueueSet, RankSet, assert_posix_ustar, block, cut_queue, cut_rank, draw, member_bytes,
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
    use crate::shadow::rank::tests_synth::{self as synth, Scene};
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

fn queue_vector(dir: &std::path::Path, class: &str) -> (usize, Vec<f32>) {
    let set = QueueSet::load_dir(dir).unwrap();
    let class = set.classes.into_iter().find(|c| c.class == class).unwrap();
    (class.n, class.vector)
}

#[test]
fn cut_mixes_queue_templates_by_n_and_replace_keeps_the_new_cut() {
    let root = scratch();
    let base_files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let next_files = write_queue(&[("s2/a.png", 1, "ALPHA QUEUE")], false);
    let base = root.path().join("base");
    let alone = root.path().join("alone");
    let mixed = root.path().join("mixed");
    let replaced = root.path().join("replaced");
    assert_eq!(cut_queue(&base_files, &base).code, 0);
    assert_eq!(cut_queue(&next_files, &alone).code, 0);
    let mix = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        next_files.labels.to_str().unwrap(),
        "--frames",
        next_files.frames.to_str().unwrap(),
        "--base",
        base.to_str().unwrap(),
        "--out",
        mixed.to_str().unwrap(),
    ]);
    assert_eq!(mix.code, 0, "{}", mix.stderr);
    assert!(
        mix.stdout.contains("class ALPHA QUEUE: learned n 1 -> 2"),
        "{}",
        mix.stdout
    );
    let (n_base, v_base) = queue_vector(&base, "ALPHA QUEUE");
    let (n_new, v_new) = queue_vector(&alone, "ALPHA QUEUE");
    let (n_mix, v_mix) = queue_vector(&mixed, "ALPHA QUEUE");
    assert_eq!((n_base, n_new, n_mix), (1, 1, 2));
    let mean: Vec<f32> = v_base
        .iter()
        .zip(&v_new)
        .map(|(b, l)| (b + l) / 2.0)
        .collect();
    assert_eq!(v_mix, mean);
    let with_replace = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        next_files.labels.to_str().unwrap(),
        "--frames",
        next_files.frames.to_str().unwrap(),
        "--base",
        base.to_str().unwrap(),
        "--replace",
        "ALPHA QUEUE",
        "--out",
        replaced.to_str().unwrap(),
    ]);
    assert_eq!(with_replace.code, 0, "{}", with_replace.stderr);
    assert!(
        with_replace
            .stdout
            .contains("class ALPHA QUEUE: replaced n 1 -> 1"),
        "{}",
        with_replace.stdout
    );
    assert_eq!(queue_vector(&replaced, "ALPHA QUEUE"), (1, v_new));
    let missing = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        next_files.labels.to_str().unwrap(),
        "--frames",
        next_files.frames.to_str().unwrap(),
        "--base",
        base.to_str().unwrap(),
        "--replace",
        "BRAVO QUEUE",
        "--out",
        root.path().join("bad").to_str().unwrap(),
    ]);
    assert_eq!(missing.code, 2, "{}", missing.stderr);
    assert!(
        missing.stderr.contains("was not learned"),
        "{}",
        missing.stderr
    );
}

#[test]
fn eval_rank_sr_change_without_hero_sr_labels_is_unverified() {
    use crate::shadow::rank::tests_synth::{self, Scene};
    let root = scratch();
    let frames = root.path().join("frames");
    let sc = Scene::default();
    save(&frames, "20260101-a/a.png", &tests_synth::paint(&sc));
    let full = root.path().join("full.jsonl");
    let mut value = serde_json::to_value(tests_synth::labels(&sc)).unwrap();
    value["game"] = serde_json::json!("20260101-a");
    value["file"] = serde_json::json!("a.png");
    value["readable"] = serde_json::json!(true);
    write(&full, &format!("{value}\n"));
    let baseline = root.path().join("baseline");
    assert_eq!(
        run(&[
            "cut",
            "--kind",
            "rank",
            "--labels",
            full.to_str().unwrap(),
            "--frames",
            frames.to_str().unwrap(),
            "--out",
            baseline.to_str().unwrap(),
        ])
        .code,
        0
    );
    let candidate = root.path().join("candidate");
    std::fs::create_dir_all(&candidate).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut pack: serde_json::Value =
        serde_json::from_slice(&std::fs::read(baseline.join("templates.json")).unwrap()).unwrap();
    let classes = pack["families"]["sr"].as_array_mut().unwrap();
    let name = classes[0]["class"].as_str().unwrap().to_string();
    let first = classes[0]["vector"][0].as_f64().unwrap();
    classes[0]["vector"][0] = serde_json::json!(first + 0.25);
    std::fs::write(
        candidate.join("templates.json"),
        serde_json::to_vec(&pack).unwrap(),
    )
    .unwrap();
    let mut bare = value.clone();
    bare["hero_sr"] = serde_json::Value::Null;
    let labels = root.path().join("bare.jsonl");
    write(&labels, &format!("{bare}\n"));
    let eval = |extra: &[&str]| {
        let mut args = vec![
            "eval",
            "--kind",
            "rank",
            "--labels",
            labels.to_str().unwrap(),
            "--frames",
            frames.to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--candidate",
            candidate.to_str().unwrap(),
            "--set-name",
            "sr-unverified",
        ];
        args.extend(extra);
        run(&args)
    };
    let denied = eval(&[]);
    assert_eq!(denied.code, 1, "{}\n{}", denied.stderr, denied.stdout);
    assert!(
        denied
            .stdout
            .contains(&format!("unverified classes: sr/{name}")),
        "{}",
        denied.stdout
    );
    let allowed = eval(&["--allow-unverified", &format!("sr/{name}")]);
    assert_eq!(allowed.code, 0, "{}\n{}", allowed.stderr, allowed.stdout);
}

#[test]
fn eval_queue_changed_class_with_no_frame_is_unverified() {
    let root = scratch();
    let alpha = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let both = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "BRAVO QUEUE"),
        ],
        false,
    );
    let baseline = root.path().join("baseline");
    let candidate = root.path().join("candidate");
    assert_eq!(cut_queue(&alpha, &baseline).code, 0);
    assert_eq!(cut_queue(&both, &candidate).code, 0);
    let got = run(&[
        "eval",
        "--kind",
        "queue-title",
        "--labels",
        alpha.labels.to_str().unwrap(),
        "--frames",
        alpha.frames.to_str().unwrap(),
        "--baseline",
        baseline.to_str().unwrap(),
        "--candidate",
        candidate.to_str().unwrap(),
        "--set-name",
        "unseen-class",
    ]);
    assert_eq!(got.code, 1, "{}\n{}", got.stderr, got.stdout);
    assert!(
        got.stdout.contains("unverified classes: BRAVO QUEUE"),
        "{}",
        got.stdout
    );
}

#[test]
fn pack_rank_refuses_a_queue_cut_and_writes_nothing() {
    let files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let src = files.dir.path().join("src");
    assert_eq!(cut_queue(&files, &src).code, 0);
    let out = files.dir.path().join("build");
    let got = run(&[
        "pack",
        "--kind",
        "rank",
        "--dir",
        src.to_str().unwrap(),
        "--name",
        "rank-v1.tar",
        "--version",
        "1",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(got.code, 2, "{}", got.stderr);
    assert!(got.stderr.contains("rank pack:"), "{}", got.stderr);
    assert!(!out.join("packs").join("rank-v1.tar").exists());
    assert!(!out.join("packs").join("manifest.json").exists());
}

#[test]
fn a_repeated_flag_is_an_error_in_both_forms() {
    for args in [
        vec!["eval", "--baseline", "one", "--baseline", "two"],
        vec!["eval", "--baseline=one", "--baseline=two"],
    ] {
        let got = run(&args);
        assert_eq!(got.code, 2, "{args:?} {}", got.stderr);
        assert!(
            got.stderr.contains("duplicate --baseline"),
            "{args:?} {}",
            got.stderr
        );
    }
}

#[test]
fn an_observed_cut_replaces_a_pooled_base_template() {
    use crate::shadow::rank::tests_synth::{self, Scene};
    let root = scratch();
    let base_frames = root.path().join("base-frames");
    let next_frames = root.path().join("next-frames");
    let base_sc = Scene {
        pct: 47,
        sr_change: Some(12),
        ..Scene::default()
    };
    let next_sc = Scene {
        pct: 10,
        sr_change: Some(7),
        ..Scene::default()
    };
    save(
        &base_frames,
        "20260101-a/a.png",
        &tests_synth::paint(&base_sc),
    );
    save(
        &next_frames,
        "20260102-b/a.png",
        &tests_synth::paint(&next_sc),
    );
    let labelled = |sc: &Scene, game: &str| {
        let mut value = serde_json::to_value(tests_synth::labels(sc)).unwrap();
        value["game"] = serde_json::json!(game);
        value["file"] = serde_json::json!("a.png");
        value["readable"] = serde_json::json!(true);
        format!("{value}\n")
    };
    let base_labels = root.path().join("base.jsonl");
    let next_labels = root.path().join("next.jsonl");
    write(&base_labels, &labelled(&base_sc, "20260101-a"));
    let mut only_change = serde_json::to_value(tests_synth::labels(&next_sc)).unwrap();
    for (key, value) in only_change.as_object_mut().unwrap() {
        if key != "hero_sr_change" {
            *value = serde_json::Value::Null;
        }
    }
    only_change["game"] = serde_json::json!("20260102-b");
    only_change["file"] = serde_json::json!("a.png");
    only_change["readable"] = serde_json::json!(true);
    write(&next_labels, &format!("{only_change}\n"));
    let base = root.path().join("base");
    let alone = root.path().join("alone");
    let out = root.path().join("out");
    for (labels, frames, dest) in [
        (&base_labels, &base_frames, &base),
        (&next_labels, &next_frames, &alone),
    ] {
        let got = run(&[
            "cut",
            "--kind",
            "rank",
            "--labels",
            labels.to_str().unwrap(),
            "--frames",
            frames.to_str().unwrap(),
            "--out",
            dest.to_str().unwrap(),
        ]);
        assert_eq!(got.code, 0, "{}", got.stderr);
    }
    let pooled = |dir: &std::path::Path| {
        let set = RankSet::load_dir(dir).unwrap();
        set.families
            .get(&Family::Change)
            .unwrap()
            .iter()
            .find(|t| t.class == "7")
            .unwrap()
            .clone()
    };
    assert!(pooled(&base).pooled, "change/7 should be a pooled fill");
    let fresh = pooled(&alone);
    assert!(!fresh.pooled);
    let recut = run(&[
        "cut",
        "--kind",
        "rank",
        "--labels",
        next_labels.to_str().unwrap(),
        "--frames",
        next_frames.to_str().unwrap(),
        "--base",
        base.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(recut.code, 0, "{}", recut.stderr);
    assert!(
        recut.stdout.contains("class change/7: replaced n "),
        "{}",
        recut.stdout
    );
    let kept = pooled(&out);
    assert_eq!(kept.n, fresh.n);
    assert_eq!(kept.vector, fresh.vector);
    assert!(!kept.pooled);
}
