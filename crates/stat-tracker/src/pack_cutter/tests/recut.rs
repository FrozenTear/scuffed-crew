//! Rank recut paths, per-class coverage and label-file checks. Helpers live
//! in the parent test module. Synthetic frames only.

use std::path::Path;

use serde_json::Value;

use super::{
    Family, QueueSet, RankSet, block, paint_gate, rank_row, run, save, scratch, write, write_queue,
    write_rank,
};
use crate::shadow::rank::templates::ClassTemplate;
use crate::shadow::rank::tests_synth::{self, Scene};

/// One rank label row for `sc`, game `game`, file `a.png`.
fn synth_row(sc: &Scene, game: &str) -> Value {
    let mut value = serde_json::to_value(tests_synth::labels(sc)).unwrap();
    value["game"] = serde_json::json!(game);
    value["file"] = serde_json::json!("a.png");
    value["readable"] = serde_json::json!(true);
    value
}

fn lines(rows: &[&Value]) -> String {
    rows.iter().map(|row| format!("{row}\n")).collect()
}

fn cut_rank_to(labels: &Path, frames: &Path, out: &Path, extra: &[&str]) -> super::super::Outcome {
    let mut args = vec![
        "cut",
        "--kind",
        "rank",
        "--labels",
        labels.to_str().unwrap(),
        "--frames",
        frames.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ];
    args.extend(extra);
    run(&args)
}

fn eval_rank(
    labels: &Path,
    frames: &Path,
    baseline: &Path,
    candidate: &Path,
    extra: &[&str],
) -> super::super::Outcome {
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
        "recut",
    ];
    args.extend(extra);
    run(&args)
}

/// A private copy of the pack in `from`, with `family/class` nudged.
fn tweaked(from: &Path, to: &Path, family: &str, class: &str) {
    std::fs::create_dir_all(to).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut pack: Value =
        serde_json::from_slice(&std::fs::read(from.join("templates.json")).unwrap()).unwrap();
    let template = pack["families"][family]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["class"] == class)
        .unwrap_or_else(|| panic!("{family}/{class} not in the pack"));
    let first = template["vector"][0].as_f64().unwrap();
    template["vector"][0] = serde_json::json!(first + 0.25);
    std::fs::write(
        to.join("templates.json"),
        serde_json::to_vec(&pack).unwrap(),
    )
    .unwrap();
}

fn template(dir: &Path, family: Family, class: &str) -> ClassTemplate {
    RankSet::load_dir(dir)
        .unwrap()
        .families
        .get(&family)
        .unwrap()
        .iter()
        .find(|t| t.class == class)
        .unwrap_or_else(|| panic!("{family:?}/{class} not in the pack"))
        .clone()
}

/// Each family's templates sorted by class, so build order does not matter.
fn sorted(dir: &Path) -> Vec<(Family, Vec<ClassTemplate>)> {
    RankSet::load_dir(dir)
        .unwrap()
        .families
        .into_iter()
        .map(|(family, mut templates)| {
            templates.sort_by(|a, b| a.class.cmp(&b.class));
            (family, templates)
        })
        .collect()
}

#[test]
fn cut_refuses_a_typo_label_key() {
    let rank = write_rank(&["20260101"], (2560, 1440), false);
    let labels = rank.dir.path().join("typo.jsonl");
    write(
        &labels,
        &format!(
            "{}\n",
            serde_json::json!({
                "game": "20260101-game",
                "file": "a.png",
                "readable": true,
                "hero_sr_chnage": 12,
            })
        ),
    );
    let got = cut_rank_to(
        &labels,
        &rank.frames,
        &rank.dir.path().join("typo-out"),
        &[],
    );
    assert_eq!(got.code, 2, "{}", got.stdout);
    assert!(
        got.stderr
            .contains(&format!("{}:1: unknown key", labels.display())),
        "{}",
        got.stderr
    );
    assert!(got.stderr.contains("hero_sr_chnage"), "{}", got.stderr);

    let queue = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let qlabels = queue.dir.path().join("typo.jsonl");
    write(
        &qlabels,
        &format!(
            "{}\n",
            serde_json::json!({
                "path": "s1/a.png",
                "screen": "heroban",
                "title_text": "ALPHA QUEUE",
                "visibel": false,
            })
        ),
    );
    let got = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        qlabels.to_str().unwrap(),
        "--frames",
        queue.frames.to_str().unwrap(),
        "--out",
        queue.dir.path().join("typo-out").to_str().unwrap(),
    ]);
    assert_eq!(got.code, 2, "{}", got.stdout);
    assert!(
        got.stderr.contains("unknown key \"visibel\""),
        "{}",
        got.stderr
    );
}

#[test]
fn queue_rows_with_visible_false_are_skipped_and_named() {
    let files = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "BRAVO QUEUE"),
        ],
        false,
    );
    let row = |path: &str, title: &str, visible: Value| {
        format!(
            "{}\n",
            serde_json::json!({
                "path": path,
                "screen": "heroban",
                "title_text": title,
                "visible": visible,
            })
        )
    };
    let labels = files.dir.path().join("visible.jsonl");
    write(
        &labels,
        &format!(
            "{}{}",
            row("s1/a.png", "ALPHA QUEUE", Value::Bool(true)),
            row("s2/b.png", "BRAVO QUEUE", Value::Bool(false))
        ),
    );
    let note = format!(
        "skipped 1 row(s) with visible: false in {}",
        labels.display()
    );
    let out = files.dir.path().join("out");
    let cut = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(cut.code, 0, "{}", cut.stderr);
    assert!(cut.stdout.contains(&note), "{}", cut.stdout);
    assert!(cut.stdout.contains("learned: 1"), "{}", cut.stdout);
    let set = QueueSet::load_dir(&out).unwrap();
    let names: Vec<&str> = set.classes.iter().map(|c| c.class.as_str()).collect();
    assert_eq!(names, vec!["ALPHA QUEUE"]);

    let eval = run(&[
        "eval",
        "--kind",
        "queue-title",
        "--labels",
        labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--baseline",
        out.to_str().unwrap(),
        "--candidate",
        out.to_str().unwrap(),
        "--set-name",
        "visible",
    ]);
    assert_eq!(eval.code, 0, "{}\n{}", eval.stderr, eval.stdout);
    assert!(eval.stdout.contains(&note), "{}", eval.stdout);
    assert!(!eval.stdout.contains("BRAVO"), "{}", eval.stdout);
    // The hidden row is counted as hidden, not as layout-skipped.
    assert!(
        eval.stdout
            .contains("frames: 2 scored: 1 hidden: 1 layout-skipped: 0"),
        "{}",
        eval.stdout
    );

    let bad = files.dir.path().join("bad-visible.jsonl");
    write(
        &bad,
        &row("s1/a.png", "ALPHA QUEUE", serde_json::json!("no")),
    );
    let got = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        bad.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--out",
        files.dir.path().join("bad-out").to_str().unwrap(),
    ]);
    assert_eq!(got.code, 2, "{}", got.stdout);
    assert!(
        got.stderr.contains("visible must be true or false"),
        "{}",
        got.stderr
    );
}

#[test]
fn queue_rows_with_visible_false_still_check_their_values() {
    let files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    // A good visible row first, so exit 2 comes from the hidden row's
    // value check, not from a file that has no classes.
    let good = serde_json::json!({
        "path": "s1/a.png",
        "screen": "heroban",
        "title_text": "ALPHA QUEUE",
        "visible": true,
    });
    let cut = |labels: &Path| {
        run(&[
            "cut",
            "--kind",
            "queue-title",
            "--labels",
            labels.to_str().unwrap(),
            "--frames",
            files.frames.to_str().unwrap(),
            "--out",
            files.dir.path().join("hidden-out").to_str().unwrap(),
        ])
    };
    let check = |name: &str, bad: Value, want: &str| {
        let labels = files.dir.path().join(name);
        write(&labels, &format!("{good}\n{bad}\n"));
        let got = cut(&labels);
        assert_eq!(got.code, 2, "{}", got.stderr);
        let line = format!("{}:2: {want}", labels.display());
        assert!(
            got.stderr.contains(&line),
            "want {line:?} in {}",
            got.stderr
        );
    };
    check(
        "hidden-typed.jsonl",
        serde_json::json!({"title_text": 5, "visible": false}),
        "title_text must be a string",
    );
    check(
        "hidden-no-path.jsonl",
        serde_json::json!({
            "screen": "heroban",
            "title_text": "BRAVO QUEUE",
            "visible": false,
        }),
        "missing path",
    );
    check(
        "hidden-spaced.jsonl",
        serde_json::json!({
            "path": "s1/a.png",
            "screen": "heroban",
            "title_text": " BRAVO QUEUE",
            "visible": false,
        }),
        "title_text \" BRAVO QUEUE\" has leading, trailing or doubled spaces",
    );
    check(
        "hidden-bad-frame.jsonl",
        serde_json::json!({
            "path": "s1/a.png",
            "screen": "heroban",
            "title_text": "BRAVO QUEUE",
            "frame": 3,
            "visible": false,
        }),
        "frame must be a string",
    );
    check(
        "hidden-bad-screen.jsonl",
        serde_json::json!({
            "path": "s1/a.png",
            "screen": "nonsense",
            "title_text": "BRAVO QUEUE",
            "visible": false,
        }),
        "queue title frame \"\" has unknown screen \"nonsense\"",
    );
}

#[test]
fn queue_rows_all_hidden_still_print_the_skip_note() {
    let files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let labels = files.dir.path().join("hidden.jsonl");
    write(
        &labels,
        &format!(
            "{}\n",
            serde_json::json!({
                "path": "s1/a.png",
                "screen": "heroban",
                "title_text": "ALPHA QUEUE",
                "visible": false,
            })
        ),
    );
    let note = format!(
        "skipped 1 row(s) with visible: false in {}\n",
        labels.display()
    );
    let cut = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--out",
        files.dir.path().join("out").to_str().unwrap(),
    ]);
    assert_eq!(cut.code, 2, "{}", cut.stdout);
    assert!(cut.stderr.starts_with(&note), "{}", cut.stderr);
    assert!(cut.stderr.contains("no classes"), "{}", cut.stderr);

    let pack = files.dir.path().join("pack");
    assert_eq!(super::cut_queue(&files, &pack).code, 0);
    let eval = run(&[
        "eval",
        "--kind",
        "queue-title",
        "--labels",
        labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--baseline",
        pack.to_str().unwrap(),
        "--candidate",
        pack.to_str().unwrap(),
        "--set-name",
        "hidden",
    ]);
    assert_eq!(eval.code, 2, "{}", eval.stdout);
    assert!(eval.stderr.starts_with(&note), "{}", eval.stderr);
    assert!(eval.stderr.contains("empty label set"), "{}", eval.stderr);
}

#[test]
fn queue_label_errors_name_the_file_line() {
    let files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let cut = |labels: &Path| {
        run(&[
            "cut",
            "--kind",
            "queue-title",
            "--labels",
            labels.to_str().unwrap(),
            "--frames",
            files.frames.to_str().unwrap(),
            "--out",
            files.dir.path().join("line-out").to_str().unwrap(),
        ])
    };
    let good = super::queue_row("s1/a.png", "ALPHA QUEUE");
    let missing = files.dir.path().join("missing.jsonl");
    write(
        &missing,
        &format!(
            "\n{good}\n{}\n",
            serde_json::json!({"path": "s1/a.png", "screen": "heroban"})
        ),
    );
    let got = cut(&missing);
    assert_eq!(got.code, 2, "{}", got.stdout);
    assert!(
        got.stderr
            .contains(&format!("{}:4: missing title_text", missing.display())),
        "{}",
        got.stderr
    );

    let frame = files.dir.path().join("frame.jsonl");
    write(
        &frame,
        &format!(
            "\n{}\n",
            serde_json::json!({
                "path": "s1/a.png",
                "screen": "heroban",
                "frame": 3,
                "title_text": "ALPHA QUEUE",
            })
        ),
    );
    let got = cut(&frame);
    assert_eq!(got.code, 2, "{}", got.stdout);
    assert!(
        got.stderr
            .contains(&format!("{}:2: frame must be a string", frame.display())),
        "{}",
        got.stderr
    );
}

#[test]
fn a_cut_without_base_prints_pooled_fills_as_new_pooled() {
    let root = scratch();
    let frames = root.path().join("frames");
    let sc = Scene::default();
    save(&frames, "20260101-a/a.png", &tests_synth::paint(&sc));
    let labels = root.path().join("labels.jsonl");
    write(&labels, &lines(&[&synth_row(&sc, "20260101-a")]));
    let out = root.path().join("out");
    let got = cut_rank_to(&labels, &frames, &out, &[]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    // 7 is cut in sr and pill, never in change
    assert!(
        got.stdout.contains("class change/7: new (pooled) n 0 -> "),
        "{}",
        got.stdout
    );
    assert!(template(&out, Family::Change, "7").pooled);
    assert!(
        got.stdout.contains("class change/1: new n 0 -> 1"),
        "{}",
        got.stdout
    );
}

#[test]
fn a_recut_against_an_unmarked_base_warns() {
    let root = scratch();
    let frames = root.path().join("frames");
    let sc = Scene::default();
    save(&frames, "20260101-a/a.png", &tests_synth::paint(&sc));
    let labels = root.path().join("labels.jsonl");
    write(&labels, &lines(&[&synth_row(&sc, "20260101-a")]));
    let marked = root.path().join("marked");
    assert_eq!(cut_rank_to(&labels, &frames, &marked, &[]).code, 0);
    let unmarked = root.path().join("unmarked");
    std::fs::create_dir_all(&unmarked).unwrap();
    let text = std::fs::read_to_string(marked.join("templates.json")).unwrap();
    assert!(
        text.contains(",\"pooled\":true"),
        "no pooled marks to strip"
    );
    std::fs::write(
        unmarked.join("templates.json"),
        text.replace(",\"pooled\":true", ""),
    )
    .unwrap();
    let warning = "base pack has unmarked pooled fills (cut before pooled marks existed, or recut from such a pack): an observed fill is mixed unless replaced, the written pack keeps the rest unmarked, so do not use it as a later --base";
    let from_unmarked = cut_rank_to(
        &labels,
        &frames,
        &root.path().join("out-unmarked"),
        &["--base", unmarked.to_str().unwrap()],
    );
    assert_eq!(from_unmarked.code, 0, "{}", from_unmarked.stderr);
    assert!(
        from_unmarked.stdout.contains(warning),
        "{}",
        from_unmarked.stdout
    );
    // one mark left in place: the other fills are still unmarked, as after a
    // --base recut that brought one new class into the pool
    let partly = root.path().join("partly");
    std::fs::create_dir_all(&partly).unwrap();
    let mark = ",\"pooled\":true";
    let first = text.find(mark).unwrap() + mark.len();
    let kept = format!("{}{}", &text[..first], text[first..].replace(mark, ""));
    assert_eq!(kept.matches(mark).count(), 1);
    std::fs::write(partly.join("templates.json"), kept).unwrap();
    let from_partly = cut_rank_to(
        &labels,
        &frames,
        &root.path().join("out-partly"),
        &["--base", partly.to_str().unwrap()],
    );
    assert_eq!(from_partly.code, 0, "{}", from_partly.stderr);
    assert!(
        from_partly.stdout.contains(warning),
        "{}",
        from_partly.stdout
    );
    let from_marked = cut_rank_to(
        &labels,
        &frames,
        &root.path().join("out-marked"),
        &["--base", marked.to_str().unwrap()],
    );
    assert_eq!(from_marked.code, 0, "{}", from_marked.stderr);
    assert!(
        !from_marked.stdout.contains(warning),
        "{}",
        from_marked.stdout
    );
}

#[test]
fn a_rank_recut_with_base_mixes_by_n() {
    let root = scratch();
    let base_frames = root.path().join("base-frames");
    let next_frames = root.path().join("next-frames");
    // two base frames against one new frame, so a mix that drops or swaps
    // the weights gives a different vector
    save(&base_frames, "20260101-a/a.png", &paint_gate(2560, 1440));
    save(&base_frames, "20260101-b/a.png", &paint_gate(2560, 1440));
    save(&next_frames, "20260102-c/a.png", &paint_gate(1920, 1080));
    let base_labels = root.path().join("base.jsonl");
    let next_labels = root.path().join("next.jsonl");
    write(
        &base_labels,
        &format!(
            "{}{}",
            rank_row("20260101-a", "a.png", true),
            rank_row("20260101-b", "a.png", true)
        ),
    );
    write(&next_labels, &rank_row("20260102-c", "a.png", true));
    let base = root.path().join("base");
    let alone = root.path().join("alone");
    let mixed = root.path().join("mixed");
    assert_eq!(cut_rank_to(&base_labels, &base_frames, &base, &[]).code, 0);
    assert_eq!(cut_rank_to(&next_labels, &next_frames, &alone, &[]).code, 0);
    let got = cut_rank_to(
        &next_labels,
        &next_frames,
        &mixed,
        &["--base", base.to_str().unwrap()],
    );
    assert_eq!(got.code, 0, "{}", got.stderr);
    let gate = crate::shadow::rank::GATE_CLASS;
    assert!(
        got.stdout
            .contains(&format!("class gate/{gate}: learned n 2 -> 3")),
        "{}",
        got.stdout
    );
    let a = template(&base, Family::Gate, gate);
    let b = template(&alone, Family::Gate, gate);
    assert_eq!((a.n, b.n), (2, 1));
    assert_ne!(a.vector, b.vector, "the two cuts should differ");
    let want: Vec<f32> = a
        .vector
        .iter()
        .zip(&b.vector)
        .map(|(x, y)| 2.0 * x + y)
        .collect();
    let got_t = template(&mixed, Family::Gate, gate);
    assert_eq!(got_t.n, 3);
    assert_eq!(got_t.vector, crate::shadow::rank::ink::unit(want));
    let aspect = ((2.0 * a.aspect.ln() + b.aspect.ln()) / 3.0).exp();
    assert_eq!(got_t.aspect, aspect);
}

#[test]
fn mix_rank_takes_the_n_weighted_log_mean_aspect() {
    let base = ClassTemplate {
        class: "rank_progress".into(),
        aspect: 0.5,
        n: 2,
        vector: vec![1.0, 0.0],
        pooled: false,
    };
    let learned = ClassTemplate {
        class: "rank_progress".into(),
        aspect: 2.0,
        n: 1,
        vector: vec![0.0, 1.0],
        pooled: false,
    };
    let mixed = super::super::rank::mix_rank(&base, &learned);
    let want = ((2.0 * 0.5f32.ln() + 2.0f32.ln()) / 3.0).exp();
    assert!(
        (mixed.aspect - want).abs() < 1e-6,
        "aspect {} want {want}",
        mixed.aspect
    );
    // `let aspect = base.aspect;` would give 0.5 here.
    assert!((mixed.aspect - base.aspect).abs() > 0.1, "{}", mixed.aspect);
    assert_eq!(mixed.n, 3);
}

#[test]
fn a_rank_recut_with_replace_keeps_the_new_cut() {
    let root = scratch();
    let base_frames = root.path().join("base-frames");
    let next_frames = root.path().join("next-frames");
    let base_sc = Scene::default();
    let next_sc = tests_synth::training().remove(0);
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
    let base_labels = root.path().join("base.jsonl");
    let next_labels = root.path().join("next.jsonl");
    write(&base_labels, &lines(&[&synth_row(&base_sc, "20260101-a")]));
    write(&next_labels, &lines(&[&synth_row(&next_sc, "20260102-b")]));
    let base = root.path().join("base");
    let alone = root.path().join("alone");
    let out = root.path().join("out");
    assert_eq!(cut_rank_to(&base_labels, &base_frames, &base, &[]).code, 0);
    assert_eq!(cut_rank_to(&next_labels, &next_frames, &alone, &[]).code, 0);
    // 7 is in both SRs (3597 and 6789)
    assert!(!template(&base, Family::Sr, "7").pooled);
    let got = cut_rank_to(
        &next_labels,
        &next_frames,
        &out,
        &["--base", base.to_str().unwrap(), "--replace", "sr/7"],
    );
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(
        got.stdout.contains("class sr/7: replaced n 1 -> 1"),
        "{}",
        got.stdout
    );
    assert!(
        got.stdout.contains("class sr/9: learned n 1 -> 2"),
        "{}",
        got.stdout
    );
    assert_eq!(
        template(&out, Family::Sr, "7"),
        template(&alone, Family::Sr, "7")
    );
    // 0 is cut in pct (10) and change (10), never in sr: sr/0 is a pooled fill
    let got = cut_rank_to(
        &next_labels,
        &next_frames,
        &root.path().join("bad"),
        &["--base", base.to_str().unwrap(), "--replace", "sr/0"],
    );
    assert_eq!(got.code, 2, "{}", got.stdout);
    assert!(
        got.stderr
            .contains("--replace sr/0 was not learned from the labels"),
        "{}",
        got.stderr
    );
}

#[test]
fn replace_all_on_the_frames_that_built_the_base_gives_the_base() {
    let root = scratch();
    let frames = root.path().join("frames");
    let sc = Scene::default();
    save(&frames, "20260101-a/a.png", &tests_synth::paint(&sc));
    let labels = root.path().join("labels.jsonl");
    write(&labels, &lines(&[&synth_row(&sc, "20260101-a")]));
    let base = root.path().join("base");
    let out = root.path().join("out");
    assert_eq!(cut_rank_to(&labels, &frames, &base, &[]).code, 0);
    let got = cut_rank_to(
        &labels,
        &frames,
        &out,
        &["--base", base.to_str().unwrap(), "--replace-all"],
    );
    assert_eq!(got.code, 0, "{}", got.stderr);
    let (mut replaced, mut kept) = (0, 0);
    for (family, templates) in sorted(&base) {
        let name = super::super::rank::family_name(family);
        for t in templates {
            let key = format!("class {name}/{}: ", t.class);
            let line = if t.pooled {
                kept += 1;
                format!("{key}kept from base (pooled) n {} -> {}", t.n, t.n)
            } else {
                replaced += 1;
                format!("{key}replaced n {} -> {}", t.n, t.n)
            };
            assert!(got.stdout.contains(&line), "{line}\n{}", got.stdout);
        }
    }
    assert!(replaced > 0 && kept > 0, "{replaced} {kept}");
    assert_eq!(sorted(&out), sorted(&base));
}

#[test]
fn eval_pooled_classes_are_not_unverified() {
    let root = scratch();
    let frames = root.path().join("frames");
    let one = Scene::default();
    let two = tests_synth::training().remove(0);
    save(&frames, "20260101-a/a.png", &tests_synth::paint(&one));
    save(&frames, "20260102-b/a.png", &tests_synth::paint(&two));
    let one_row = synth_row(&one, "20260101-a");
    let two_row = synth_row(&two, "20260102-b");
    let one_labels = root.path().join("one.jsonl");
    let two_labels = root.path().join("two.jsonl");
    let both = root.path().join("both.jsonl");
    write(&one_labels, &lines(&[&one_row]));
    write(&two_labels, &lines(&[&two_row]));
    write(&both, &lines(&[&one_row, &two_row]));
    let baseline = root.path().join("baseline");
    let candidate = root.path().join("candidate");
    assert_eq!(cut_rank_to(&one_labels, &frames, &baseline, &[]).code, 0);
    assert_eq!(cut_rank_to(&two_labels, &frames, &candidate, &[]).code, 0);
    let got = eval_rank(&both, &frames, &baseline, &candidate, &[]);
    let pooled: Vec<String> = RankSet::load_dir(&candidate)
        .unwrap()
        .families
        .iter()
        .flat_map(|(family, templates)| {
            templates
                .iter()
                .filter(|t| t.pooled)
                .map(move |t| format!("{}/{}", super::super::rank::family_name(*family), t.class))
        })
        .collect();
    assert!(
        pooled
            .iter()
            .any(|name| got.stdout.contains(&format!("changed class {name}: "))),
        "no pooled class changed\n{}",
        got.stdout
    );
    assert!(
        !got.stdout.contains("unverified classes:"),
        "pooled {pooled:?}\n{}",
        got.stdout
    );
}

#[test]
fn eval_skips_coverage_for_a_class_the_family_does_not_offer() {
    let root = scratch();
    let frames = root.path().join("frames");
    let green = Scene::default();
    let red = Scene {
        change: Some(-27),
        ..Scene::default()
    };
    save(&frames, "20260101-a/a.png", &tests_synth::paint(&green));
    save(&frames, "20260102-b/a.png", &tests_synth::paint(&red));
    let green_labels = root.path().join("green.jsonl");
    let red_labels = root.path().join("red.jsonl");
    write(&green_labels, &lines(&[&synth_row(&green, "20260101-a")]));
    write(&red_labels, &lines(&[&synth_row(&red, "20260102-b")]));
    let baseline = root.path().join("baseline");
    assert_eq!(cut_rank_to(&green_labels, &frames, &baseline, &[]).code, 0);
    // + is cut only in the green pill, so higher/+ and pct/+ are pooled
    // fills, and no field of the red frame shows a +
    assert!(template(&baseline, Family::Higher, "+").pooled);
    assert!(template(&baseline, Family::Pct, "+").pooled);

    // higher never offers +, so a changed higher/+ needs no frame
    let higher = root.path().join("higher");
    tweaked(&baseline, &higher, "higher", "+");
    let got = eval_rank(&red_labels, &frames, &baseline, &higher, &[]);
    assert!(
        got.stdout.contains("changed class higher/+: "),
        "{}\n{}",
        got.stderr,
        got.stdout
    );
    assert!(!got.stdout.contains("unverified"), "{}", got.stdout);

    // pct offers +, so a changed pct/+ still needs a frame that shows one
    let pct = root.path().join("pct");
    tweaked(&baseline, &pct, "pct", "+");
    let got = eval_rank(&red_labels, &frames, &baseline, &pct, &[]);
    assert_eq!(got.code, 1, "{}\n{}", got.stderr, got.stdout);
    assert!(
        got.stdout.contains("unverified classes: pct/+\n"),
        "{}",
        got.stdout
    );
}

#[test]
fn division_and_emblem_coverage_use_the_class_the_learner_cuts() {
    let root = scratch();
    let frames = root.path().join("frames");
    let two = Scene {
        division: Some(2),
        ..Scene::default()
    };
    let three = Scene {
        division: Some(3),
        ..Scene::default()
    };
    save(&frames, "20260101-a/a.png", &tests_synth::paint(&two));
    save(&frames, "20260102-b/a.png", &tests_synth::paint(&three));
    let two_row = synth_row(&two, "20260101-a");
    let three_row = synth_row(&three, "20260102-b");
    let both = root.path().join("both.jsonl");
    let two_labels = root.path().join("two.jsonl");
    let three_labels = root.path().join("three.jsonl");
    write(&both, &lines(&[&two_row, &three_row]));
    write(&two_labels, &lines(&[&two_row]));
    write(&three_labels, &lines(&[&three_row]));
    let baseline = root.path().join("baseline");
    assert_eq!(cut_rank_to(&both, &frames, &baseline, &[]).code, 0);

    // a division 2 frame shows emblems 3 and 1, not a big 3
    let division = root.path().join("division");
    tweaked(&baseline, &division, "division", "3");
    let got = eval_rank(&two_labels, &frames, &baseline, &division, &[]);
    assert_eq!(got.code, 1, "{}\n{}", got.stderr, got.stdout);
    assert!(
        got.stdout.contains("unverified classes: division/3\n"),
        "{}",
        got.stdout
    );
    let got = eval_rank(&three_labels, &frames, &baseline, &division, &[]);
    assert!(!got.stdout.contains("unverified"), "{}", got.stdout);

    // a division 3 frame shows emblems 4 and 2, not 3
    let emblem = root.path().join("emblem");
    tweaked(&baseline, &emblem, "emblem", "3");
    let got = eval_rank(&three_labels, &frames, &baseline, &emblem, &[]);
    assert_eq!(got.code, 1, "{}\n{}", got.stderr, got.stdout);
    assert!(
        got.stdout.contains("unverified classes: emblem/3\n"),
        "{}",
        got.stdout
    );
    let got = eval_rank(&two_labels, &frames, &baseline, &emblem, &[]);
    assert!(!got.stdout.contains("unverified"), "{}", got.stdout);
}

#[test]
fn eval_rank_coverage_is_per_class_not_per_family() {
    let root = scratch();
    let frames = root.path().join("frames");
    let shown = Scene::default();
    let other = Scene {
        sr: Some(6789),
        role: Some(crate::shadow::rank::Role::Tank),
        ..Scene::default()
    };
    save(&frames, "20260101-a/a.png", &tests_synth::paint(&shown));
    save(&frames, "20260102-b/a.png", &tests_synth::paint(&other));
    let shown_row = synth_row(&shown, "20260101-a");
    let both = root.path().join("both.jsonl");
    let shown_labels = root.path().join("shown.jsonl");
    write(
        &both,
        &lines(&[&shown_row, &synth_row(&other, "20260102-b")]),
    );
    write(&shown_labels, &lines(&[&shown_row]));
    let baseline = root.path().join("baseline");
    assert_eq!(cut_rank_to(&both, &frames, &baseline, &[]).code, 0);

    // the labelled SR is 3597: hero_sr is labelled, but never with an 8
    assert!(!template(&baseline, Family::Sr, "8").pooled);
    let sr = root.path().join("sr");
    tweaked(&baseline, &sr, "sr", "8");
    let got = eval_rank(&shown_labels, &frames, &baseline, &sr, &[]);
    assert_eq!(got.code, 1, "{}\n{}", got.stderr, got.stdout);
    assert!(
        got.stdout.contains("unverified classes: sr/8\n"),
        "{}",
        got.stdout
    );

    // the labelled role is Open: a role label, but not a Tank one
    assert_eq!(shown_row["role"], "Open");
    let role = root.path().join("role");
    tweaked(&baseline, &role, "role", "Tank");
    let got = eval_rank(&shown_labels, &frames, &baseline, &role, &[]);
    assert_eq!(got.code, 1, "{}\n{}", got.stderr, got.stdout);
    assert!(
        got.stdout.contains("unverified classes: role/Tank\n"),
        "{}",
        got.stdout
    );
}

#[test]
fn eval_rank_two_days_learn_from_the_other_day() {
    let files = write_rank(&["20260101", "20260102"], (2560, 1440), false);
    let out = files.dir.path().join("out");
    assert_eq!(super::cut_rank(&files, &out).code, 0);
    let got = eval_rank(&files.labels, &files.frames, &out, &out, &[]);
    assert_eq!(got.code, 0, "{}\n{}", got.stderr, got.stdout);
    let gate = block(&got.stdout, "screen gate");
    assert!(gate.contains("folds: 2"), "{gate}");
    assert!(
        gate.contains("generalisation: learned 2 / 0 / 0, baseline 2 / 0 / 0"),
        "{gate}"
    );
}
