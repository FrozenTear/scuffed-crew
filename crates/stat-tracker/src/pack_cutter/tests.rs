//! Synthetic frames only. No game pixels. Local labelled sets stay on the
//! ignored benches `shadow::rank::tests::bench_labelled_rank_screens` and
//! `shadow::queue_title::tests::bench_labelled_queue_titles`.

use std::path::{Path, PathBuf};

use image::{Rgb, RgbImage};
use sha2::{Digest, Sha256};

use crate::shadow::queue_title::TemplateSet as QueueSet;
use crate::shadow::rank::{Family, GATE_CLASS, TemplateSet as RankSet};

fn run(args: &[&str]) -> super::Outcome {
    let owned: Vec<std::ffi::OsString> = args.iter().map(|s| (*s).into()).collect();
    super::execute(&owned)
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

/// Gate plus a mode banner, at 2560x1440. `word` uses the stand-in font.
fn paint_gate_banner(word: &str) -> RgbImage {
    let mut img = paint_gate(2560, 1440);
    let red = Rgb([208, 59, 97]);
    for y in 120..200 {
        for x in 1952..2540 {
            img.put_pixel(x, y, red);
        }
    }
    draw(&mut img, word, 2068, 150, 3, Rgb([250, 250, 250]));
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
    assert!(got.stdout.contains("unreadable: 0"), "{}", got.stdout);
    assert!(got.stdout.contains("layout-skipped: 1"), "{}", got.stdout);
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
        got.stdout
            .contains("learned: 2\nunreadable: 0\nlayout-skipped: 0"),
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
    assert!(got.stdout.contains("unreadable: 0"), "{}", got.stdout);
    assert!(got.stdout.contains("layout-skipped: 1"), "{}", got.stdout);
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
    assert!(got.stdout.contains("layout-skipped: 0"), "{}", got.stdout);
    assert_eq!(
        queue_classes(&out),
        vec!["ALPHA QUEUE".to_string(), "BRAVO QUEUE".to_string()]
    );
}

#[test]
fn pack_writes_ustar_and_a_second_pack_keeps_the_first() {
    let dir = scratch();
    let rank_files = write_rank(&["20260101"], (2560, 1440), false);
    let queue_files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let one = dir.path().join("one");
    let two = dir.path().join("two");
    assert_eq!(cut_rank(&rank_files, &one).code, 0);
    assert_eq!(cut_queue(&queue_files, &two).code, 0);
    let out = dir.path().join("build");
    let first = run(&[
        "pack",
        "--kind",
        "rank",
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
    assert!(
        first.stdout.contains("templates.json sha256:"),
        "{}",
        first.stdout
    );
    let second = run(&[
        "pack",
        "--kind",
        "queue-title",
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
    let refused = run(&[
        "pack",
        "--kind",
        "rank",
        "--dir",
        one.to_str().unwrap(),
        "--name",
        "heroes-v1.tar",
        "--version",
        "1",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(refused.code, 2, "{}", refused.stderr);
    assert!(
        refused.stderr.contains("does not match --kind"),
        "{}",
        refused.stderr
    );

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
        assert_posix_ustar(&tar_bytes);
        let unpacked =
            crate::packs::read_ustar(&tar_bytes, crate::packs::PackLayout::TemplatesJson)
                .unwrap_or_else(|e| panic!("{name} read_ustar: {e}"));
        let entry = list.iter().find(|e| e["name"] == name).expect(name);
        assert_eq!(entry["version"], version);
        assert_eq!(entry["sha256"], sha256_hex(&tar_bytes));
        assert_eq!(entry["size"], tar_bytes.len() as u64);
        assert_eq!(entry.as_object().unwrap().len(), 4, "list keys");

        let inner = member_bytes(&unpacked, "manifest.json");
        let packed = member_bytes(&unpacked, "templates.json");
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

fn member_bytes(files: &[(String, Vec<u8>)], name: &str) -> Vec<u8> {
    files
        .iter()
        .find(|(path, _)| path == name)
        .unwrap_or_else(|| panic!("missing {name}"))
        .1
        .clone()
}

fn octal(field: &[u8]) -> u64 {
    let digits: Vec<u8> = field
        .iter()
        .copied()
        .take_while(|b| *b != 0 && *b != b' ')
        .collect();
    u64::from_str_radix(std::str::from_utf8(&digits).unwrap(), 8).unwrap()
}

fn assert_posix_ustar(tar: &[u8]) {
    assert!(tar.ends_with(&[0u8; 1024]), "trailer");
    let mut off = 0;
    let mut headers = 0;
    while off + 512 <= tar.len() {
        let h = &tar[off..off + 512];
        if h.iter().all(|b| *b == 0) {
            break;
        }
        let size = octal(&h[124..136]);
        assert_eq!(&h[100..108], b"0000644\0", "mode");
        assert_eq!(&h[108..116], b"0000000\0", "uid");
        assert_eq!(&h[116..124], b"0000000\0", "gid");
        assert_eq!(&h[124..136], format!("{size:011o}\0").as_bytes(), "size");
        assert_eq!(&h[136..148], b"00000000000\0", "mtime");
        assert_eq!(h[156], b'0', "typeflag");
        assert_eq!(&h[257..263], b"ustar\0", "magic");
        assert_eq!(&h[263..265], b"00", "version");
        assert_eq!(h[154], 0, "chksum NUL");
        assert_eq!(h[155], b' ', "chksum space");
        headers += 1;
        off += 512 + (size as usize).div_ceil(512) * 512;
    }
    assert!(headers >= 2, "manifest and templates.json");
}

#[test]
fn cut_and_pack_refuse_an_out_dir_inside_a_git_work_tree() {
    let repo = scratch();
    std::fs::create_dir(repo.path().join(".git")).unwrap();
    let probe = repo.path().join("pack-cutter-probe");
    let files = write_rank(&["20260101"], (2560, 1440), false);
    let cut = cut_rank(&files, &probe);
    assert_eq!(cut.code, 2, "{}", cut.stderr);
    assert!(
        cut.stderr.contains("inside a git work tree"),
        "{}",
        cut.stderr
    );
    assert!(!probe.exists(), "cut created {}", probe.display());

    let pack = run(&[
        "pack",
        "--kind",
        "rank",
        "--dir",
        files.dir.path().to_str().unwrap(),
        "--name",
        "rank-v1.tar",
        "--version",
        "1",
        "--out",
        probe.to_str().unwrap(),
    ]);
    assert_eq!(pack.code, 2, "{}", pack.stderr);
    assert!(
        pack.stderr.contains("inside a git work tree"),
        "{}",
        pack.stderr
    );
    assert!(!probe.exists());
}

#[test]
fn eval_queue_title_dropped_class_exits_nonzero() {
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
    assert_eq!(got.code, 1, "{}", got.stdout);
    assert!(got.stdout.contains("set: queue-held-out"), "{}", got.stdout);
    assert!(got.stdout.contains("frames: 2 scored: 2"), "{}", got.stdout);
    assert!(
        got.stdout.contains("dropped classes: BRAVO QUEUE"),
        "{}",
        got.stdout
    );
    let bravo = block(&got.stdout, "class BRAVO QUEUE\n");
    assert!(
        bravo.contains("baseline: 1 / 0 / 0\ncandidate: 0 / 0 / 1"),
        "{bravo}"
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
fn eval_rank_single_day_pack_match_does_not_fail_loso() {
    // One day: leave-one-day-out has nothing to learn from, so the
    // generalisation line misses the gate. The candidate pack is the
    // baseline pack, so the gate passes. folds: 1 shows the single day.
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
        "rank-one-day",
    ]);
    assert_eq!(got.code, 0, "{}\n{}", got.stderr, got.stdout);
    let gate = block(&got.stdout, "screen gate\n");
    assert!(gate.contains("folds: 1"), "{gate}");
    assert!(
        gate.contains("baseline: 1 / 0 / 0\ncandidate: 1 / 0 / 0"),
        "{gate}"
    );
    assert!(
        gate.contains("generalisation: learned 0 / 1 / 0, baseline 1 / 0 / 0"),
        "{gate}"
    );
    assert!(
        got.stdout
            .contains("template class gate/rank_progress\nfolds: 1"),
        "{}",
        got.stdout
    );
    assert!(got.stdout.contains("result: pass"), "{}", got.stdout);
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
    assert_eq!(got.code, 2, "{}", got.stderr);
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
    assert_eq!(got.code, 2, "{}", got.stderr);
    assert!(got.stderr.contains("baseline pack"), "{}", got.stderr);
}

#[test]
fn help_exits_zero() {
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["help"],
        vec!["cut", "--help"],
    ] {
        let got = run(&args);
        assert_eq!(got.code, 0, "{args:?} {}", got.stderr);
        assert!(
            got.stdout.contains("scuffed-pack-cutter cut"),
            "{args:?} {}",
            got.stdout
        );
    }
}

#[test]
fn cut_prints_unreadable_rows_apart_from_layout_skips() {
    let files = write_rank(&["20260101"], (2560, 1440), true);
    let game = "20260104-blank";
    save(
        &files.frames,
        &format!("{game}/a.png"),
        &RgbImage::from_pixel(2560, 1440, Rgb([40, 44, 60])),
    );
    let mut labels = std::fs::read_to_string(&files.labels).unwrap();
    labels.push_str(&rank_row(game, "a.png", false));
    write(&files.labels, &labels);
    let out = files.dir.path().join("out");
    let got = cut_rank(&files, &out);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(got.stdout.contains("rows: 3\n"), "{}", got.stdout);
    assert!(got.stdout.contains("learned: 1\n"), "{}", got.stdout);
    assert!(got.stdout.contains("unreadable: 1\n"), "{}", got.stdout);
    assert!(got.stdout.contains("layout-skipped: 1\n"), "{}", got.stdout);
}

#[test]
fn cut_keeps_base_classes_the_labels_do_not_produce() {
    let root = scratch();
    let both = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "BRAVO QUEUE"),
        ],
        false,
    );
    let only = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let base = root.path().join("base");
    let out = root.path().join("out");
    assert_eq!(cut_queue(&both, &base).code, 0);
    let got = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        only.labels.to_str().unwrap(),
        "--frames",
        only.frames.to_str().unwrap(),
        "--base",
        base.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert!(
        got.stdout.contains("class ALPHA QUEUE: learned"),
        "{}",
        got.stdout
    );
    assert!(
        got.stdout.contains("class BRAVO QUEUE: kept from base"),
        "{}",
        got.stdout
    );
    assert!(
        got.stdout
            .contains("classes: learned 1, kept from base 1, new 0"),
        "{}",
        got.stdout
    );
    let kept = QueueSet::load_dir(&out).unwrap();
    let base_set = QueueSet::load_dir(&base).unwrap();
    let bravo = |set: &QueueSet| {
        set.classes
            .iter()
            .find(|c| c.class == "BRAVO QUEUE")
            .unwrap()
            .vector
            .clone()
    };
    assert_eq!(bravo(&kept), bravo(&base_set));
}

#[test]
fn cut_queue_title_allows_absolute_paths_without_frames() {
    let files = write_queue(&[("s1/a.png", 0, "ALPHA QUEUE")], false);
    let abs = files.frames.join("s1/a.png");
    let labels = files.dir.path().join("abs.jsonl");
    write(&labels, &queue_row(abs.to_str().unwrap(), "ALPHA QUEUE"));
    let out = files.dir.path().join("out");
    let got = run(&[
        "cut",
        "--kind",
        "queue-title",
        "--labels",
        labels.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert_eq!(queue_classes(&out), vec!["ALPHA QUEUE".to_string()]);
}

#[test]
fn eval_fails_when_the_candidate_drops_a_baseline_class() {
    let root = scratch();
    let both = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "BRAVO QUEUE"),
        ],
        false,
    );
    let only = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s3/c.png", 0, "ALPHA QUEUE"),
        ],
        false,
    );
    let baseline = root.path().join("baseline");
    let candidate = root.path().join("candidate");
    assert_eq!(cut_queue(&both, &baseline).code, 0);
    assert_eq!(cut_queue(&only, &candidate).code, 0);
    let eval = |extra: &[&str]| {
        let mut args = vec![
            "eval",
            "--kind",
            "queue-title",
            "--labels",
            only.labels.to_str().unwrap(),
            "--frames",
            only.frames.to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--candidate",
            candidate.to_str().unwrap(),
            "--set-name",
            "drop-class",
        ];
        args.extend(extra);
        run(&args)
    };
    let denied = eval(&[]);
    assert_ne!(denied.code, 0, "{}", denied.stdout);
    assert!(
        denied.stdout.contains("dropped classes: BRAVO QUEUE"),
        "{}",
        denied.stdout
    );
    let allowed = eval(&["--allow-drop", "BRAVO QUEUE"]);
    assert_eq!(allowed.code, 0, "{}\n{}", allowed.stderr, allowed.stdout);
    assert!(
        allowed.stdout.contains("result: pass"),
        "{}",
        allowed.stdout
    );
}

#[test]
fn eval_single_fold_classes_do_not_fail_when_the_packs_match() {
    let files = write_queue(
        &[
            ("s1/a.png", 0, "ALPHA QUEUE"),
            ("s2/b.png", 1, "BRAVO QUEUE"),
        ],
        false,
    );
    let pack = files.dir.path().join("pack");
    assert_eq!(cut_queue(&files, &pack).code, 0);
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
        "one-session-each",
    ]);
    assert_eq!(got.code, 0, "{}\n{}", got.stderr, got.stdout);
    let alpha = block(&got.stdout, "class ALPHA QUEUE\n");
    assert!(alpha.contains("folds: 1"), "{alpha}");
    assert!(
        alpha.contains("baseline: 1 / 0 / 0\ncandidate: 1 / 0 / 0"),
        "{alpha}"
    );
    assert!(
        alpha.contains("generalisation: learned 0 / 0 / 1, baseline 1 / 0 / 0"),
        "{alpha}"
    );
    assert!(got.stdout.contains("result: pass"), "{}", got.stdout);
}

#[test]
fn cut_rank_keeps_a_banner_the_new_labels_do_not_cut() {
    let root = scratch();
    let base_frames = root.path().join("base-frames");
    let next_frames = root.path().join("next-frames");
    save(
        &base_frames,
        "20260101-a/a.png",
        &paint_gate_banner("HELLO"),
    );
    save(&next_frames, "20260102-b/a.png", &paint_gate(2560, 1440));
    let base_labels = root.path().join("base.jsonl");
    let next_labels = root.path().join("next.jsonl");
    write(
        &base_labels,
        &format!(
            "{}\n",
            serde_json::json!({
                "game": "20260101-a",
                "file": "a.png",
                "readable": true,
                "next_mode_hint": "HELLO",
            })
        ),
    );
    write(&next_labels, &rank_row("20260102-b", "a.png", true));
    let base = root.path().join("base");
    let out = root.path().join("out");
    let first = run(&[
        "cut",
        "--kind",
        "rank",
        "--labels",
        base_labels.to_str().unwrap(),
        "--frames",
        base_frames.to_str().unwrap(),
        "--out",
        base.to_str().unwrap(),
    ]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert!(
        first.stdout.contains("class banner/HELLO: new"),
        "{}",
        first.stdout
    );
    let second = run(&[
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
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert!(
        second.stdout.contains("class banner/HELLO: kept from base"),
        "{}",
        second.stdout
    );
    let kept = RankSet::load_dir(&out).unwrap();
    let base_set = RankSet::load_dir(&base).unwrap();
    let banner = |set: &RankSet| {
        set.families
            .get(&Family::Banner)
            .unwrap()
            .iter()
            .find(|t| t.class == "HELLO")
            .unwrap()
            .vector
            .clone()
    };
    assert_eq!(banner(&kept), banner(&base_set));

    let gate_only = root.path().join("gate-only");
    assert_eq!(
        run(&[
            "cut",
            "--kind",
            "rank",
            "--labels",
            next_labels.to_str().unwrap(),
            "--frames",
            next_frames.to_str().unwrap(),
            "--out",
            gate_only.to_str().unwrap(),
        ])
        .code,
        0
    );
    let denied = run(&[
        "eval",
        "--kind",
        "rank",
        "--labels",
        next_labels.to_str().unwrap(),
        "--frames",
        next_frames.to_str().unwrap(),
        "--baseline",
        base.to_str().unwrap(),
        "--candidate",
        gate_only.to_str().unwrap(),
        "--set-name",
        "rank-drop",
    ]);
    assert_ne!(denied.code, 0, "{}", denied.stdout);
    assert!(
        denied.stdout.contains("dropped classes: banner/HELLO"),
        "{}",
        denied.stdout
    );
    let allowed = run(&[
        "eval",
        "--kind",
        "rank",
        "--labels",
        next_labels.to_str().unwrap(),
        "--frames",
        next_frames.to_str().unwrap(),
        "--baseline",
        base.to_str().unwrap(),
        "--candidate",
        gate_only.to_str().unwrap(),
        "--set-name",
        "rank-drop",
        "--allow-drop",
        "banner/HELLO",
    ]);
    assert_eq!(allowed.code, 0, "{}\n{}", allowed.stderr, allowed.stdout);
}

mod review;
