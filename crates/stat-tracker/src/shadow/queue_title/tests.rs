//! Unit tests on synthetic frames (no game pixels in the repo), and the
//! ignored bench over local labelled frames.

use image::imageops::{self, FilterType};
use image::{Rgb, RgbImage};

use super::*;

/// Dark red backdrop, like the pregame screens.
const BG: [u8; 3] = [40, 14, 20];
/// Pink voice chat indicator.
const PINK: [u8; 3] = [230, 40, 200];

/// Rectangles `[x, y, w, h]` relative to the title's top left at the
/// 2560x1440 reference, one set per synthetic first word.
fn word(kind: u8) -> Vec<[u32; 4]> {
    match kind {
        // vertical bars
        0 => (0..6).map(|i| [10 + i * 38, 0, 16, 98]).collect(),
        // horizontal bars
        1 => (0..4).map(|i| [6, i * 26, 220, 14]).collect(),
        // a checker
        _ => (0..4)
            .flat_map(|r| (0..4).map(move |c| [6 + c * 56 + (r % 2) * 28, r * 25, 26, 22]))
            .collect(),
    }
}

/// The shared second word: blocks of mixed width.
fn second_word() -> Vec<[u32; 4]> {
    vec![
        [250, 0, 30, 98],
        [290, 20, 60, 20],
        [290, 70, 60, 28],
        [360, 0, 18, 98],
        [390, 40, 50, 18],
        [410, 0, 22, 60],
    ]
}

/// A 2560x1440 frame with title `kind` on `screen`, ink at `level`
/// (255 = full), with the pink indicator over the lower left.
fn frame(kind: u8, screen: Screen, level: u8) -> RgbImage {
    let mut img = RgbImage::from_pixel(2560, 1440, Rgb(BG));
    let (tx, ty) = (46, screen.ref_y() + 8);
    let ink = Rgb([level, level, level.saturating_sub(4)]);
    for [x, y, w, h] in word(kind).into_iter().chain(second_word()) {
        for yy in ty + y..ty + y + h {
            for xx in tx + x..tx + x + w {
                img.put_pixel(xx, yy, ink);
            }
        }
    }
    for yy in ty + 50..ty + 80 {
        for xx in tx..tx + 160 {
            img.put_pixel(xx, yy, Rgb(PINK));
        }
    }
    img
}

const NAMES: [&str; 3] = ["ALPHA QUEUE", "BRAVO QUEUE", "CHECK QUEUE"];

fn pack(kinds: &[u8]) -> TemplateSet {
    let mut l = Learner::new();
    for &k in kinds {
        for s in [Screen::MapVote, Screen::HeroBan] {
            assert!(l.add_frame(&frame(k, s, 230), s, NAMES[k as usize]));
        }
    }
    l.build()
}

#[test]
fn reads_each_known_title() {
    let r = Reader::new(pack(&[0, 1]));
    for k in [0u8, 1] {
        for s in [Screen::MapVote, Screen::HeroSelect, Screen::HeroBan] {
            let f = r.read_field(&frame(k, s, 230), s);
            assert_eq!(f.value.as_deref(), Some(NAMES[k as usize]), "{k} {s:?}");
            assert!(f.confidence >= MIN_SCORE);
            let [x0, y0, x1, y1] = f.bbox.unwrap();
            assert!(x0 <= 48 && x1 >= 500, "{x0} {x1}");
            assert!(y0 <= s.ref_y() + 8 && y1 >= s.ref_y() + 130, "{y0} {y1}");
        }
    }
    let read = r.read(&frame(0, Screen::HeroBan, 230), Screen::HeroBan);
    assert_eq!(read.queue_title_recognizer, RECOGNIZER_ID);
}

#[test]
fn unknown_title_is_none() {
    let r = Reader::new(pack(&[0, 1]));
    let f = r.read_field(&frame(2, Screen::HeroBan, 230), Screen::HeroBan);
    assert_eq!(f, Field::none());
}

#[test]
fn title_left_out_of_a_one_class_pack_is_none() {
    // only the second word is shared: the whole score may pass, the strips
    // over the first word must not
    let r = Reader::new(pack(&[0]));
    let g = plane(&frame(1, Screen::HeroBan, 230), Screen::HeroBan)
        .unwrap()
        .0;
    let ranked = rank(&r.templates, &g);
    assert!(ranked[0].strip < MIN_STRIP, "{ranked:?}");
    assert!(accept(&ranked).is_none());
    assert!(
        r.read_field(&frame(0, Screen::HeroBan, 230), Screen::HeroBan)
            .value
            .is_some()
    );
}

#[test]
fn dimmed_frame_reads_the_same() {
    let r = Reader::new(pack(&[0, 1]));
    let f = r.read_field(&frame(1, Screen::HeroSelect, 110), Screen::HeroSelect);
    assert_eq!(f.value.as_deref(), Some(NAMES[1]));
}

#[test]
fn pink_overlay_does_not_count_as_ink() {
    let mut img = RgbImage::from_pixel(2560, 1440, Rgb(BG));
    for y in 100..200 {
        for x in 50..300 {
            img.put_pixel(x, y, Rgb(PINK));
        }
    }
    let (g, _) = plane(&img, Screen::HeroBan).unwrap();
    assert!(g.px.iter().all(|&v| v < 0.2), "pink leaked into the ink");
}

#[test]
fn reads_1080p_with_a_1440p_pack() {
    let r = Reader::new(pack(&[0, 1]));
    for k in [0u8, 1] {
        let img = imageops::resize(
            &frame(k, Screen::HeroBan, 230),
            1920,
            1080,
            FilterType::Lanczos3,
        );
        let f = r.read_field(&img, Screen::HeroBan);
        assert_eq!(f.value.as_deref(), Some(NAMES[k as usize]));
        assert!(f.bbox.unwrap()[2] <= 400);
    }
}

#[test]
fn blank_or_tiny_frame_is_none() {
    let r = Reader::new(pack(&[0, 1]));
    let blank = RgbImage::from_pixel(2560, 1440, Rgb(BG));
    assert_eq!(r.read_field(&blank, Screen::HeroBan), Field::none());
}

#[test]
fn frame_too_narrow_for_the_title_is_none() {
    // A real title frame cut to 400 px wide: the title box (528 px with its
    // margin at 1440p) no longer fits, so plane() refuses it even though
    // the pixels it would read hold a title.
    let r = Reader::new(pack(&[0, 1]));
    let full = frame(0, Screen::HeroBan, 230);
    assert_eq!(
        r.read_field(&full, Screen::HeroBan).value.as_deref(),
        Some(NAMES[0])
    );
    let narrow = imageops::crop_imm(&full, 0, 0, 400, 1440).to_image();
    assert!(plane(&narrow, Screen::HeroBan).is_none());
    assert_eq!(r.read_field(&narrow, Screen::HeroBan), Field::none());
}

fn cs(class: &str, score: f32, strip: f32) -> ClassScore {
    ClassScore {
        class: class.to_string(),
        score,
        strip,
        dx: 0,
        dy: 0,
    }
}

#[test]
fn accept_rules_table() {
    let a = |r: &[ClassScore]| accept(r).map(|c| c.class.clone());
    // all three rules pass
    assert_eq!(
        a(&[cs("A", 0.95, 0.90), cs("B", 0.50, 0.40)]),
        Some("A".into())
    );
    // whole score below MIN_SCORE
    assert_eq!(
        a(&[cs("A", MIN_SCORE - 0.01, 0.90), cs("B", 0.30, 0.30)]),
        None
    );
    // weakest strip below MIN_STRIP
    assert_eq!(
        a(&[cs("A", 0.95, MIN_STRIP - 0.01), cs("B", 0.30, 0.30)]),
        None
    );
    // lead over the runner-up below MIN_MARGIN
    assert_eq!(
        a(&[cs("A", 0.95, 0.90), cs("B", 0.95 - MIN_MARGIN + 0.01, 0.85)]),
        None
    );
    // lead just over MIN_MARGIN passes
    assert_eq!(
        a(&[cs("A", 0.95, 0.90), cs("B", 0.95 - MIN_MARGIN - 0.01, 0.85)]),
        Some("A".into())
    );
    // one class pack: no runner-up, the lead is over 0
    assert_eq!(a(&[cs("A", 0.85, 0.75)]), Some("A".into()));
    // a negative runner-up counts as 0, so it cannot widen the lead
    assert_eq!(
        a(&[cs("A", 0.81, 0.75), cs("B", -0.50, -0.50)]),
        Some("A".into())
    );
    // empty ranking
    assert_eq!(a(&[]), None);
}

#[test]
fn setting_parse_and_gate() {
    for on in ["on", "ON", " 1 ", "true", "yes", "rank-v1"] {
        assert!(QueueTitleSetting::parse(on).is_on(), "{on}");
    }
    for off in ["", "off", "0", "no", "cv-v1"] {
        assert!(!QueueTitleSetting::parse(off).is_on(), "{off}");
    }
    assert!(!QueueTitleSetting::default().is_on());

    let dir = tempfile::tempdir().unwrap();
    // on without a pack: no reader
    assert!(Reader::for_setting(QueueTitleSetting::On, dir.path()).is_none());
    pack(&[0]).save_dir(&dir.path().join(TEMPLATE_DIR)).unwrap();
    // off with a pack: still no reader
    assert!(Reader::for_setting(QueueTitleSetting::Off, dir.path()).is_none());
    let r = Reader::for_setting(QueueTitleSetting::On, dir.path()).unwrap();
    assert_eq!(r.templates, pack(&[0]));
}

#[test]
fn pack_with_wrong_size_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = pack(&[0]);
    p.width = TW + 1;
    p.save_dir(dir.path()).unwrap();
    assert!(matches!(
        TemplateSet::load_dir(dir.path()),
        Err(TemplateError::Bad(..))
    ));
    assert!(matches!(
        TemplateSet::load_dir(&dir.path().join("nope")),
        Err(TemplateError::Missing(..))
    ));
}

#[test]
fn pick_follows_the_policy() {
    let v = |s: &str| Field {
        value: Some(s.to_string()),
        confidence: 0.9,
        bbox: None,
    };
    let reads = vec![
        (Screen::MapVote, "last", v("M")),
        (Screen::HeroSelect, "last", v("S")),
        (Screen::HeroBan, "first", v("B1")),
        (Screen::HeroBan, "last", Field::none()),
    ];
    assert_eq!(pick(&reads).unwrap().value.as_deref(), Some("B1"));
    assert_eq!(pick(&reads[..2]).unwrap().value.as_deref(), Some("S"));
    assert!(pick(&reads[..1]).is_none());
}

#[test]
fn read_dump_dir_uses_the_policy() {
    let r = Reader::new(pack(&[0, 1]));
    let dir = tempfile::tempdir().unwrap();
    frame(1, Screen::HeroSelect, 230)
        .save(dir.path().join("heroselect_last.png"))
        .unwrap();
    // a mapvote frame alone is not part of the policy
    frame(0, Screen::MapVote, 230)
        .save(dir.path().join("mapvote_last.png"))
        .unwrap();
    let read = r.read_dump_dir(dir.path());
    assert_eq!(read.queue_title.value.as_deref(), Some(NAMES[1]));
    frame(0, Screen::HeroBan, 230)
        .save(dir.path().join("heroban_first.png"))
        .unwrap();
    let read = r.read_dump_dir(dir.path());
    assert_eq!(read.queue_title.value.as_deref(), Some(NAMES[0]));
}

/// Leave-one-session-out bench over local labelled pregame frames.
///
/// `SCUFFED_QUEUE_TITLE_BENCH` is the labels JSONL (path, screen, frame,
/// title_text per line); `SCUFFED_QUEUE_TITLE_OUT` is a folder for
/// `RESULT.md` and a pack learned from every frame (`pack/`). Both stay
/// outside the repo. 1080p is SYNTHETIC: the same frames, Lanczos3 downscaled.
#[test]
#[ignore = "needs local labelled pregame frames"]
fn bench_labelled_queue_titles() {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::Write as _;

    let (Ok(jsonl), Ok(out)) = (
        std::env::var("SCUFFED_QUEUE_TITLE_BENCH"),
        std::env::var("SCUFFED_QUEUE_TITLE_OUT"),
    ) else {
        eprintln!("set SCUFFED_QUEUE_TITLE_BENCH and SCUFFED_QUEUE_TITLE_OUT");
        return;
    };
    let out = std::path::PathBuf::from(out);

    #[derive(serde::Deserialize)]
    struct Row {
        path: String,
        screen: String,
        frame: String,
        title_text: String,
    }
    fn pl(i: &Item, small: bool) -> &Gray {
        if small { &i.p1080 } else { &i.p1440 }
    }
    struct Item {
        session: String,
        screen: Screen,
        frame: String,
        title: String,
        /// Planes at 1440p and at synthetic 1080p.
        p1440: Gray,
        p1080: Gray,
    }

    let mut items = Vec::new();
    for line in std::fs::read_to_string(&jsonl).unwrap().lines() {
        if line.trim().is_empty() {
            continue;
        }
        let r: Row = serde_json::from_str(line).unwrap();
        let screen = Screen::parse(&r.screen).unwrap();
        let p = std::path::Path::new(&r.path);
        let session = p
            .parent()
            .and_then(|d| d.file_name())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let img = image::open(p).unwrap().to_rgb8();
        let small = imageops::resize(&img, 1920, 1080, FilterType::Lanczos3);
        items.push(Item {
            session,
            screen,
            frame: r.frame,
            title: r.title_text,
            p1440: plane(&img, screen).unwrap().0,
            p1080: plane(&small, screen).unwrap().0,
        });
    }
    let sessions: BTreeSet<&str> = items.iter().map(|i| i.session.as_str()).collect();
    let titles: BTreeSet<&str> = items.iter().map(|i| i.title.as_str()).collect();

    #[derive(Clone, Copy, PartialEq)]
    enum Res {
        Bad,
        Ok,
        None,
    }
    fn learn<'a>(it: impl Iterator<Item = (&'a Gray, &'a str)>) -> TemplateSet {
        let mut l = Learner::new();
        for (g, t) in it {
            l.add_plane(g, t);
        }
        l.build()
    }
    fn read(t: &TemplateSet, g: &Gray) -> (Option<String>, Vec<ClassScore>) {
        let ranked = rank(t, g);
        (accept(&ranked).map(|c| c.class.clone()), ranked)
    }

    let mut md = String::new();
    let _ = writeln!(
        md,
        "# Queue title reader bench (queue-title, recognizer `{RECOGNIZER_ID}`)\n"
    );
    let _ = writeln!(
        md,
        "{} frames, {} sessions, titles: {}. Thresholds: MIN_SCORE {MIN_SCORE}, MIN_STRIP {MIN_STRIP}, MIN_MARGIN {MIN_MARGIN}.\n",
        items.len(),
        sessions.len(),
        titles.iter().copied().collect::<Vec<_>>().join(", ")
    );

    // (name, train at 1080?, test at 1080?)
    let configs = [
        ("1440p (train 1440p, test 1440p)", false, false),
        (
            "SYNTHETIC 1080p, realistic (train 1440p, test Lanczos3 1080p)",
            false,
            true,
        ),
        (
            "SYNTHETIC 1080p (train Lanczos3 1080p, test Lanczos3 1080p)",
            true,
            true,
        ),
    ];
    let mut total_wrong = 0;
    for (name, tr, te) in configs {
        let mut res: Vec<Res> = vec![Res::None; items.len()];
        let mut got: Vec<Option<String>> = vec![None; items.len()];
        let (mut min_ok, mut min_ok_strip) = (f32::MAX, f32::MAX);
        let (mut max_other, mut max_other_strip, mut min_lead) = (f32::MIN, f32::MIN, f32::MAX);
        for s in &sessions {
            let t = learn(
                items
                    .iter()
                    .filter(|i| i.session != *s)
                    .map(|i| (pl(i, tr), i.title.as_str())),
            );
            for (k, i) in items.iter().enumerate().filter(|(_, i)| i.session == *s) {
                let (v, ranked) = read(&t, pl(i, te));
                for c in &ranked {
                    if c.class == i.title {
                        min_ok = min_ok.min(c.score);
                        min_ok_strip = min_ok_strip.min(c.strip);
                    } else {
                        max_other = max_other.max(c.score);
                        max_other_strip = max_other_strip.max(c.strip);
                    }
                }
                if ranked.len() > 1 {
                    min_lead = min_lead.min(ranked[0].score - ranked[1].score);
                }
                res[k] = match &v {
                    None => Res::None,
                    Some(c) if *c == i.title => Res::Ok,
                    Some(_) => Res::Bad,
                };
                got[k] = v;
            }
        }
        total_wrong += res.iter().filter(|r| **r == Res::Bad).count();
        let _ = writeln!(md, "## Leave-one-session-out: {name}\n");
        let _ = writeln!(
            md,
            "| group | frames | correct | wrong | None |\n|---|---|---|---|---|"
        );
        let mut groups: BTreeMap<String, [usize; 3]> = BTreeMap::new();
        for (i, r) in items.iter().zip(&res) {
            let slot = match r {
                Res::Ok => 0,
                Res::Bad => 1,
                Res::None => 2,
            };
            for g in [
                format!("screen {}", i.screen.as_str()),
                format!("screen {} {}", i.screen.as_str(), i.frame),
                format!("title {}", i.title),
                "all".to_string(),
            ] {
                groups.entry(g).or_default()[slot] += 1;
            }
        }
        for (g, [ok, bad, none]) in &groups {
            let _ = writeln!(md, "| {g} | {} | {ok} | {bad} | {none} |", ok + bad + none);
        }
        let _ = writeln!(
            md,
            "\nScores: correct class whole >= {min_ok:.3}, strip >= {min_ok_strip:.3}; other class whole <= {max_other:.3}, strip <= {max_other_strip:.3}; lead >= {min_lead:.3}.\n"
        );

        // one value per session by the policy
        let _ = writeln!(
            md,
            "Per-session policy (heroban last, heroban first, heroselect last):\n"
        );
        let _ = writeln!(md, "| session | truth | read | from |\n|---|---|---|---|");
        let (mut p_ok, mut p_bad, mut p_none) = (0, 0, 0);
        for s in &sessions {
            let truth = items
                .iter()
                .find(|i| i.session == *s)
                .unwrap()
                .title
                .clone();
            let mut pickd: Option<(String, String)> = None;
            for (sc, fr) in POLICY {
                if let Some((k, _)) = items.iter().enumerate().find(|(k, i)| {
                    i.session == *s && i.screen == sc && i.frame == fr && got[*k].is_some()
                }) {
                    pickd = Some((got[k].clone().unwrap(), format!("{}_{fr}", sc.as_str())));
                    break;
                }
            }
            let (read, from) = match &pickd {
                Some((v, f)) => {
                    if *v == truth {
                        p_ok += 1
                    } else {
                        p_bad += 1
                    }
                    (v.as_str(), f.as_str())
                }
                None => {
                    p_none += 1;
                    ("None", "-")
                }
            };
            let _ = writeln!(md, "| {s} | {truth} | {read} | {from} |");
        }
        total_wrong += p_bad;
        let _ = writeln!(
            md,
            "\nPolicy: {p_ok} correct, {p_bad} wrong, {p_none} None of {} sessions.\n",
            sessions.len()
        );
    }

    // held out class: the pack never saw it, so every frame must be None
    let _ = writeln!(md, "## Held-out title (pack learned without it)\n");
    let _ = writeln!(
        md,
        "| left out | test size | frames | None | read as other | max whole | max strip |\n|---|---|---|---|---|---|---|"
    );
    for held in &titles {
        let t = learn(
            items
                .iter()
                .filter(|i| i.title != *held)
                .map(|i| (&i.p1440, i.title.as_str())),
        );
        for (label, small) in [("1440p", false), ("SYNTHETIC 1080p", true)] {
            let (mut none, mut other, mut mw, mut ms) = (0, 0, f32::MIN, f32::MIN);
            let held_items: Vec<&Item> = items.iter().filter(|i| i.title == *held).collect();
            for i in &held_items {
                let (v, ranked) = read(&t, pl(i, small));
                mw = mw.max(ranked[0].score);
                ms = ms.max(ranked[0].strip);
                if v.is_none() { none += 1 } else { other += 1 }
            }
            total_wrong += other;
            let _ = writeln!(
                md,
                "| {held} | {label} | {} | {none} | {other} | {mw:.3} | {ms:.3} |",
                held_items.len()
            );
        }
    }

    // a pack from every frame, for local use
    let all = learn(items.iter().map(|i| (&i.p1440, i.title.as_str())));
    all.save_dir(&out.join("pack")).unwrap();
    let _ = writeln!(
        md,
        "\nPack from all {} frames (1440p) written to `pack/{PACK_FILE}`; install under `<data_dir>/{TEMPLATE_DIR}/`.",
        items.len()
    );
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("RESULT.md"), &md).unwrap();
    println!("{md}");
    assert_eq!(total_wrong, 0, "wrong reads in the bench");
}
