//! Synthetic frames only. No game pixels. Local labelled sets stay on the
//! ignored benches `shadow::rank::tests::bench_labelled_rank_screens` and
//! `shadow::queue_title::tests::bench_labelled_queue_titles`.

use std::path::{Path, PathBuf};

use image::{Rgb, RgbImage};
use sha2::{Digest, Sha256};

use super::execute;
use crate::shadow::queue_title::TemplateSet as QueueSet;
use crate::shadow::rank::{Family, GATE_CLASS, TemplateSet as RankSet};

fn run(args: &[&str]) -> super::Outcome {
    execute(args.iter().copied())
}

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn save(dir: &Path, rel: &str, img: &RgbImage) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    img.save(&path).unwrap();
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn block(text: &str, header: &str) -> String {
    let start = text
        .find(header)
        .unwrap_or_else(|| panic!("missing {header} in {text}"));
    let rest = &text[start + header.len()..];
    let end = rest
        .find("\nfield ")
        .or_else(|| rest.find("\nclass "))
        .or_else(|| rest.find("\noverall"))
        .unwrap_or(rest.len());
    format!("{header}{}", &rest[..end])
}

// ---- rank gate, the same stand-in word the rank tests draw ----

fn rows_of(c: char) -> [&'static str; 7] {
    match c {
        'T' => [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
        'E' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
        'L' => [
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ],
        'O' => [
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'H' => [
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        _ => [
            "#####", "#####", "#####", "#####", "#####", "#####", "#####",
        ],
    }
}

fn draw(img: &mut RgbImage, text: &str, x: u32, y: u32, s: u32, col: Rgb<u8>) {
    let mut cx = x;
    for c in text.chars() {
        for (r, row) in rows_of(c).iter().enumerate() {
            for (k, b) in row.bytes().enumerate() {
                if b == b'#' {
                    for dy in 0..s {
                        for dx in 0..s {
                            img.put_pixel(cx + k as u32 * s + dx, y + r as u32 * s + dy, col);
                        }
                    }
                }
            }
        }
        cx += 6 * s;
    }
}

/// "TELLOH" where the rank reader looks for the gate, at `w` x `h`.
fn paint_gate(w: u32, h: u32) -> RgbImage {
    let mut img = RgbImage::from_pixel(w, h, Rgb([40, 44, 60]));
    let sy = h as f32 / 1440.0;
    let sx = w as f32 / 2560.0;
    let s = ((3.0 * sy).round() as u32).max(1);
    draw(
        &mut img,
        "TELLOH",
        (842.0 * sx).round() as u32,
        (1068.0 * sy).round() as u32,
        s,
        Rgb([250, 250, 250]),
    );
    img
}

fn rank_row(game: &str, file: &str, readable: bool) -> String {
    format!(
        "{}\n",
        serde_json::json!({"game": game, "file": file, "readable": readable})
    )
}

fn role_row(game: &str) -> String {
    format!("{}\n", serde_json::json!({"game": game, "role": "Open"}))
}

struct RankSetFiles {
    dir: tempfile::TempDir,
    labels: PathBuf,
    frames: PathBuf,
    roles: PathBuf,
}

fn write_rank(days: &[&str], wh: (u32, u32), extra_skip: bool) -> RankSetFiles {
    let dir = scratch();
    let frames = dir.path().join("frames");
    let mut labels = String::new();
    let mut roles = String::new();
    for day in days {
        let game = format!("{day}-game");
        save(&frames, &format!("{game}/a.png"), &paint_gate(wh.0, wh.1));
        labels.push_str(&rank_row(&game, "a.png", true));
        roles.push_str(&role_row(&game));
    }
    if extra_skip {
        let game = "20260103-skip";
        save(
            &frames,
            &format!("{game}/a.png"),
            &RgbImage::from_pixel(800, 600, Rgb([40, 44, 60])),
        );
        labels.push_str(&rank_row(game, "a.png", true));
        roles.push_str(&role_row(game));
    }
    let labels_path = dir.path().join("labels.jsonl");
    let roles_path = dir.path().join("roles.jsonl");
    write(&labels_path, &labels);
    write(&roles_path, &roles);
    RankSetFiles {
        labels: labels_path,
        frames,
        roles: roles_path,
        dir,
    }
}

fn cut_rank(files: &RankSetFiles, out: &Path) -> super::Outcome {
    run(&[
        "cut",
        "--kind",
        "rank",
        "--labels",
        files.labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--role-labels",
        files.roles.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ])
}

// ---- queue title, the same block shapes the queue-title tests draw ----

const QUEUE_BG: [u8; 3] = [40, 14, 20];
const QUEUE_PINK: [u8; 3] = [230, 40, 200];

fn queue_word(kind: u8) -> Vec<[u32; 4]> {
    match kind {
        0 => (0..6).map(|i| [10 + i * 38, 0, 16, 98]).collect(),
        _ => (0..4).map(|i| [6, i * 26, 220, 14]).collect(),
    }
}

fn queue_second() -> Vec<[u32; 4]> {
    vec![
        [250, 0, 30, 98],
        [290, 20, 60, 20],
        [290, 70, 60, 28],
        [360, 0, 18, 98],
        [390, 40, 50, 18],
        [410, 0, 22, 60],
    ]
}

fn paint_title(kind: u8) -> RgbImage {
    let mut img = RgbImage::from_pixel(2560, 1440, Rgb(QUEUE_BG));
    // Hero ban title top, matching `Screen::HeroBan` in the queue tests.
    let (tx, ty) = (46, 87 + 8);
    let ink = Rgb([230, 230, 226]);
    for [x, y, w, h] in queue_word(kind).into_iter().chain(queue_second()) {
        for yy in ty + y..ty + y + h {
            for xx in tx + x..tx + x + w {
                img.put_pixel(xx, yy, ink);
            }
        }
    }
    for yy in ty + 50..ty + 80 {
        for xx in tx..tx + 160 {
            img.put_pixel(xx, yy, Rgb(QUEUE_PINK));
        }
    }
    img
}

fn queue_row(path: &str, title: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "path": path,
            "screen": "heroban",
            "frame": "last",
            "title_text": title,
        })
    )
}

struct QueueFiles {
    dir: tempfile::TempDir,
    labels: PathBuf,
    frames: PathBuf,
}

fn write_queue(rows: &[(&str, u8, &str)], tiny: bool) -> QueueFiles {
    let dir = scratch();
    let frames = dir.path().join("frames");
    let mut labels = String::new();
    for (rel, kind, title) in rows {
        let img = if rel.ends_with("1080.png") {
            image::imageops::resize(
                &paint_title(*kind),
                1920,
                1080,
                image::imageops::FilterType::Nearest,
            )
        } else {
            paint_title(*kind)
        };
        save(&frames, rel, &img);
        labels.push_str(&queue_row(rel, title));
    }
    if tiny {
        save(
            &frames,
            "tiny/a.png",
            &RgbImage::from_pixel(64, 64, Rgb(QUEUE_BG)),
        );
        labels.push_str(&queue_row("tiny/a.png", "ALPHA QUEUE"));
    }
    let labels_path = dir.path().join("labels.jsonl");
    write(&labels_path, &labels);
    QueueFiles {
        labels: labels_path,
        frames,
        dir,
    }
}

fn cut_queue(files: &QueueFiles, out: &Path) -> super::Outcome {
    run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        files.labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ])
}

fn queue_classes(dir: &Path) -> Vec<String> {
    let set = QueueSet::load_dir(dir).expect("queue pack loads");
    let mut names: Vec<String> = set.classes.into_iter().map(|c| c.class).collect();
    names.sort();
    names
}

#[test]
fn cut_rank_frames_write_a_pack_the_reader_accepts() {
    let files = write_rank(&["20260101", "20260102"], (2560, 1440), true);
    let out = files.dir.path().join("out");
    let got = cut_rank(&files, &out);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(got.stdout.contains("learned: 2"), "{}", got.stdout);
    assert!(got.stdout.contains("skipped: 1"), "{}", got.stdout);
    let set = RankSet::load_dir(&out).expect("rank pack loads and checks");
    assert!(set.has_class(Family::Gate, GATE_CLASS));
    assert_eq!(set.recognizer, crate::shadow::rank::RECOGNIZER_ID);
}

#[test]
fn cut_rank_1080_pack_still_loads() {
    let files = write_rank(&["20260101", "20260102"], (1920, 1080), false);
    let out = files.dir.path().join("out");
    let got = cut_rank(&files, &out);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(
        got.stdout.contains("learned: 2\nskipped: 0"),
        "{}",
        got.stdout
    );
    let set = RankSet::load_dir(&out).expect("1080 rank pack loads");
    assert!(set.has_class(Family::Gate, GATE_CLASS));
}

#[test]
fn cut_queue_title_frames_write_expected_classes() {
    let files = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "BRAVO QUEUE"),
        ],
        true,
    );
    let out = files.dir.path().join("out");
    let got = cut_queue(&files, &out);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(got.stdout.contains("learned: 2"), "{}", got.stdout);
    assert!(got.stdout.contains("skipped: 1"), "{}", got.stdout);
    assert_eq!(
        queue_classes(&out),
        vec!["ALPHA QUEUE".to_string(), "BRAVO QUEUE".to_string()]
    );
    let set = QueueSet::load_dir(&out).unwrap();
    for class in &set.classes {
        let peak = class.vector.iter().copied().fold(0.0_f32, f32::max);
        assert!(peak > 0.5, "{} peak {peak}", class.class);
    }
}

#[test]
fn cut_queue_title_1080_pack_still_loads() {
    let files = write_queue(
        &[
            ("s1/a-1080.png", 0, "ALPHA QUEUE"),
            ("s2/b-1080.png", 1, "BRAVO QUEUE"),
        ],
        false,
    );
    let out = files.dir.path().join("out");
    let got = cut_queue(&files, &out);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(got.stdout.contains("skipped: 0"), "{}", got.stdout);
    assert_eq!(
        queue_classes(&out),
        vec!["ALPHA QUEUE".to_string(), "BRAVO QUEUE".to_string()]
    );
}

#[test]
fn pack_writes_ustar_and_a_second_pack_keeps_the_first() {
    let dir = scratch();
    let one = dir.path().join("one");
    let two = dir.path().join("two");
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    std::fs::write(one.join("templates.json"), br#"{"synthetic":"one"}"#).unwrap();
    std::fs::write(two.join("templates.json"), br#"{"synthetic":"two"}"#).unwrap();
    let out = dir.path().join("build");
    let first = run(&[
        "pack",
        "--dir",
        one.to_str().unwrap(),
        "--name",
        "rank-v1.tar",
        "--version",
        "20261114.1",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let second = run(&[
        "pack",
        "--dir",
        two.to_str().unwrap(),
        "--name",
        "queue-title-v1.tar",
        "--version",
        "20261114.2",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(second.code, 0, "{}", second.stderr);

    let packs = out.join("packs");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&out), 0o700);
        assert_eq!(mode(&packs), 0o700);
        assert_eq!(mode(&packs.join("rank-v1.tar")), 0o600);
        assert_eq!(mode(&packs.join("queue-title-v1.tar")), 0o600);
    }

    let list: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(packs.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(list.len(), 2, "{list:?}");
    for (name, version, templates) in [
        ("rank-v1.tar", "20261114.1", &one),
        ("queue-title-v1.tar", "20261114.2", &two),
    ] {
        let tar_path = packs.join(name);
        let tar_bytes = std::fs::read(&tar_path).unwrap();
        assert!(tar_bytes.ends_with(&[0u8; 1024]), "{name} trailer");
        let header = &tar_bytes[..512];
        assert_eq!(&header[100..107], b"0000644");
        assert_eq!(&header[108..115], b"0000000");
        assert_eq!(&header[116..123], b"0000000");
        assert_eq!(&header[136..147], b"00000000000");
        assert_eq!(header[156], b'0');
        assert_eq!(&header[257..262], b"ustar");
        let entry = list.iter().find(|e| e["name"] == name).expect(name);
        assert_eq!(entry["version"], version);
        assert_eq!(entry["sha256"], sha256_hex(&tar_bytes));
        assert_eq!(entry["size"], tar_bytes.len() as u64);
        assert_eq!(entry.as_object().unwrap().len(), 4, "list keys");

        let inner = tar_member(&tar_path, "manifest.json");
        let packed = tar_member(&tar_path, "templates.json");
        let want = std::fs::read(templates.join("templates.json")).unwrap();
        assert_eq!(packed, want);
        let inner: serde_json::Value = serde_json::from_slice(&inner).unwrap();
        assert_eq!(inner["name"], name);
        assert_eq!(inner["version"], version);
        assert_eq!(inner["source"], super::ustar::SOURCE);
        let file = &inner["files"][0];
        assert_eq!(file["path"], "templates.json");
        assert_eq!(file["sha256"], sha256_hex(&want));
        assert_eq!(file["size"], want.len() as u64);
    }
}

fn tar_member(tar: &Path, name: &str) -> Vec<u8> {
    let out = std::process::Command::new("tar")
        .args(["-xOf", tar.to_str().unwrap(), name])
        .output()
        .expect("tar");
    assert!(
        out.status.success(),
        "tar {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

#[test]
fn cut_and_pack_refuse_an_out_dir_inside_a_git_work_tree() {
    let probe = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("pack-cutter-probe");
    let _ = std::fs::remove_dir_all(&probe);
    let files = write_rank(&["20260101"], (2560, 1440), false);
    let cut = cut_rank(&files, &probe);
    assert_ne!(cut.code, 0);
    assert!(
        cut.stderr.contains("inside a git work tree"),
        "{}",
        cut.stderr
    );
    assert!(!probe.exists(), "cut created {}", probe.display());

    let pack = run(&[
        "pack",
        "--dir",
        files.dir.path().to_str().unwrap(),
        "--name",
        "rank-v1.tar",
        "--version",
        "1",
        "--out",
        probe.to_str().unwrap(),
    ]);
    assert_ne!(pack.code, 0);
    assert!(
        pack.stderr.contains("inside a git work tree"),
        "{}",
        pack.stderr
    );
    assert!(!probe.exists());
}

#[test]
fn eval_queue_title_regression_exits_nonzero() {
    let root = scratch();
    let full = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "BRAVO QUEUE"),
        ],
        false,
    );
    let only_alpha = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let baseline = root.path().join("baseline");
    let candidate = root.path().join("candidate");
    assert_eq!(cut_queue(&full, &baseline).code, 0);
    assert_eq!(cut_queue(&only_alpha, &candidate).code, 0);
    let got = run(&[
        "eval",
        "--kind",
        "queue-title",
        "--labels",
        full.labels.to_str().unwrap(),
        "--frames",
        full.frames.to_str().unwrap(),
        "--baseline",
        baseline.to_str().unwrap(),
        "--candidate",
        candidate.to_str().unwrap(),
        "--set-name",
        "queue-held-out",
    ]);
    assert_ne!(got.code, 0, "{}", got.stdout);
    assert!(got.stdout.contains("set: queue-held-out"), "{}", got.stdout);
    assert!(got.stdout.contains("frames: 2 scored: 2"), "{}", got.stdout);
    let alpha = block(&got.stdout, "class ALPHA QUEUE\n");
    assert!(
        alpha.contains("baseline: 1 / 0 / 0\ncandidate: 0 / 0 / 1"),
        "{alpha}"
    );
    assert!(got.stdout.contains("result: fail"), "{}", got.stdout);
}

#[test]
fn eval_queue_title_match_exits_zero() {
    let files = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 0, "ALPHA QUEUE"),
        ],
        false,
    );
    let pack = files.dir.path().join("pack");
    assert_eq!(cut_queue(&files, &pack).code, 0, "cut");
    let got = run(&[
        "eval",
        "--kind",
        "queue-title",
        "--labels",
        files.labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--baseline",
        pack.to_str().unwrap(),
        "--candidate",
        pack.to_str().unwrap(),
        "--set-name",
        "queue-match",
    ]);
    assert_eq!(got.code, 0, "{}\n{}", got.stderr, got.stdout);
    assert!(got.stdout.contains("set: queue-match"), "{}", got.stdout);
    let class = block(&got.stdout, "class ALPHA QUEUE\n");
    assert!(
        class.contains("baseline: 2 / 0 / 0\ncandidate: 2 / 0 / 0"),
        "{class}"
    );
    assert!(got.stdout.contains("result: pass"), "{}", got.stdout);
}

#[test]
fn eval_rank_match_exits_zero() {
    let files = write_rank(&["20260101", "20260102"], (2560, 1440), false);
    let pack = files.dir.path().join("pack");
    let cut = cut_rank(&files, &pack);
    assert_eq!(cut.code, 0, "{}", cut.stderr);
    let got = run(&[
        "eval",
        "--kind",
        "rank",
        "--labels",
        files.labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--role-labels",
        files.roles.to_str().unwrap(),
        "--baseline",
        pack.to_str().unwrap(),
        "--candidate",
        pack.to_str().unwrap(),
        "--set-name",
        "rank-match",
    ]);
    assert_eq!(got.code, 0, "{}\n{}", got.stderr, got.stdout);
    assert!(got.stdout.contains("set: rank-match"), "{}", got.stdout);
    let role = block(&got.stdout, "field role\n");
    assert!(
        role.contains("baseline: 2 / 0 / 0\ncandidate: 2 / 0 / 0"),
        "{role}"
    );
    let gate = block(&got.stdout, "screen gate\n");
    assert!(
        gate.contains("baseline: 2 / 0 / 0\ncandidate: 2 / 0 / 0"),
        "{gate}"
    );
    assert!(got.stdout.contains("result: pass"), "{}", got.stdout);
}

#[test]
fn eval_rank_held_out_gate_regression_exits_nonzero() {
    // One day: the held-out learner sees no other day, so it misses the
    // gate the baseline pack (cut from that frame) still reads.
    let files = write_rank(&["20260101"], (2560, 1440), false);
    let pack = files.dir.path().join("pack");
    assert_eq!(cut_rank(&files, &pack).code, 0);
    let got = run(&[
        "eval",
        "--kind",
        "rank",
        "--labels",
        files.labels.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--role-labels",
        files.roles.to_str().unwrap(),
        "--baseline",
        pack.to_str().unwrap(),
        "--candidate",
        pack.to_str().unwrap(),
        "--set-name",
        "rank-held-out",
    ]);
    assert_ne!(got.code, 0, "{}", got.stdout);
    assert!(got.stdout.contains("set: rank-held-out"), "{}", got.stdout);
    let gate = block(&got.stdout, "screen gate\n");
    assert!(
        gate.contains("baseline: 1 / 0 / 0\ncandidate: 0 / 1 / 0"),
        "{gate}"
    );
    let role = block(&got.stdout, "field role\n");
    assert!(
        role.contains("baseline: 1 / 0 / 0\ncandidate: 0 / 0 / 1"),
        "{role}"
    );
    assert!(got.stdout.contains("result: fail"), "{}", got.stdout);
}

#[test]
fn eval_empty_label_set_and_missing_baseline_exit_nonzero() {
    let files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let pack = files.dir.path().join("pack");
    assert_eq!(cut_queue(&files, &pack).code, 0);
    let empty = files.dir.path().join("empty.jsonl");
    write(&empty, "");
    let got = run(&[
        "eval",
        "--kind",
        "queue-title",
        "--labels",
        empty.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--baseline",
        pack.to_str().unwrap(),
        "--candidate",
        pack.to_str().unwrap(),
        "--set-name",
        "empty",
    ]);
    assert_ne!(got.code, 0);
    assert!(got.stderr.contains("empty label set"), "{}", got.stderr);

    let missing = files.dir.path().join("no-such-pack");
    let got = run(&[
        "eval",
        "--kind",
        "rank",
        "--labels",
        empty.to_str().unwrap(),
        "--frames",
        files.frames.to_str().unwrap(),
        "--baseline",
        missing.to_str().unwrap(),
        "--candidate",
        pack.to_str().unwrap(),
        "--set-name",
        "missing",
    ]);
    assert_ne!(got.code, 0);
    assert!(got.stderr.contains("baseline pack"), "{}", got.stderr);
}
