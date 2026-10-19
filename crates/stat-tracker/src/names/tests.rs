//! Synthetic letters only: no game art, no player names.
use super::pack::{CANVAS, DIM, MARK_CANVAS, NamePack, PackError};
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn resize_lands_on_pillow_bilinear_values() {
    let src: Vec<u8> = (0..35u32).map(|i| (i * 37 % 256) as u8).collect();
    assert_eq!(
        resize::resize_l(&src, 7, 5, 3, 4),
        [33, 112, 191, 36, 115, 194, 40, 119, 198, 43, 122, 201]
    );
    assert_eq!(
        resize::resize_l(&src, 7, 5, 7, 2),
        [3, 40, 77, 114, 151, 188, 225, 9, 46, 83, 120, 157, 194, 231]
    );
    assert_eq!(
        resize::resize_l(&src, 7, 5, 4, 5),
        [
            19, 77, 145, 203, 22, 80, 148, 206, 25, 83, 151, 209, 28, 86, 154, 212, 31, 89, 157,
            215
        ]
    );
    assert_eq!(
        resize::resize_l(&src, 7, 5, 16, 28)[..16],
        [
            0, 6, 22, 38, 54, 71, 87, 103, 119, 135, 151, 168, 184, 200, 216, 222
        ]
    );
    let big: Vec<u8> = (0..1200u32).map(|i| (i * 13 % 256) as u8).collect();
    let out = resize::resize_l(&big, 40, 30, 16, 28);
    let every: Vec<u8> = out.iter().step_by(29).copied().collect();
    assert_eq!(
        every,
        [
            14, 185, 105, 30, 200, 119, 39, 214, 134, 54, 220, 145, 68, 166, 163, 82
        ]
    );
    assert_eq!(
        resize::resize_l(&src, 7, 5, 7, 5),
        src,
        "same size is a copy"
    );
}

/// 3x5 block letters, each one 8-connected part.
fn bitmap(c: char) -> [&'static str; 5] {
    match c {
        'A' => ["010", "101", "111", "101", "101"],
        'B' => ["110", "101", "110", "101", "110"],
        'C' => ["011", "100", "100", "100", "011"],
        _ => panic!("no bitmap for {c}"),
    }
}

/// A 6v6-sized player row (board 1007 px high: cap height 22 px) with `text`
/// in white block letters; a space is a wide gap.
fn row(text: &str) -> image::DynamicImage {
    let (w, h) = (600u32, 75u32);
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([60, 20, 70]));
    let mut x = 40u32;
    for c in text.chars() {
        // a space is a wide gap; '_' a gap that may or may not be one (0.25 to
        // 0.40 cap heights once straightened, before a letter whose top row
        // starts in its first column)
        if c == ' ' || c == '_' {
            x += if c == ' ' { 12 } else { 7 };
            continue;
        }
        for (r, line) in bitmap(c).iter().enumerate() {
            for (k, b) in line.chars().enumerate() {
                if b != '1' {
                    continue;
                }
                let (y0, y1) = (26 + (r as u32 * 22) / 5, 26 + ((r as u32 + 1) * 22) / 5);
                for yy in y0..y1 {
                    for xx in x + k as u32 * 5..x + (k as u32 + 1) * 5 {
                        img.put_pixel(xx, yy, image::Rgb([235, 235, 235]));
                    }
                }
            }
        }
        x += 15 + 4;
    }
    image::DynamicImage::ImageRgb8(img)
}

fn grid(tiles: &[Vec<u8>], (w, h): (usize, usize)) -> Vec<u8> {
    let rows = tiles.len().div_ceil(64).max(1);
    let mut im = image::GrayImage::new(64 * w as u32, (rows * h) as u32);
    for (i, t) in tiles.iter().enumerate() {
        let (r, c) = (i / 64, i % 64);
        for y in 0..h {
            for x in 0..w {
                im.put_pixel(
                    (c * w + x) as u32,
                    (r * h + y) as u32,
                    image::Luma([t[y * w + x]]),
                );
            }
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    im.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A pack cut from synthetic rows, with identity whitening.
fn write_pack(dir: &std::path::Path, training: &[&str]) {
    std::fs::create_dir_all(dir).unwrap();
    let mut tiles = Vec::new();
    let mut lines = String::new();
    for text in training {
        let cut = cut_tiles(&row(text), 1007, 6);
        let letters: Vec<char> = text.chars().filter(|c| !matches!(c, ' ' | '_')).collect();
        assert_eq!(cut.len(), letters.len(), "{text}");
        for ((t, r), c) in cut.into_iter().zip(letters) {
            tiles.push(t);
            lines.push_str(&format!("{c}\t{r}\n"));
        }
    }
    let mut whiten = Vec::with_capacity(DIM * DIM * 4);
    for i in 0..DIM {
        for j in 0..DIM {
            whiten.extend_from_slice(&(if i == j { 1f32 } else { 0f32 }).to_le_bytes());
        }
    }
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("glyphs.png", grid(&tiles, CANVAS)),
        ("glyphs.txt", lines.into_bytes()),
        ("marks.png", grid(&[], MARK_CANVAS)),
        ("marks.txt", Vec::new()),
        ("compose.txt", Vec::new()),
        ("whiten.f32", whiten),
    ];
    let mut meta = serde_json::Map::new();
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).unwrap();
        meta.insert(
            name.to_string(),
            serde_json::json!({"size": bytes.len(), "sha256": format!("{:x}", Sha256::digest(bytes))}),
        );
    }
    let manifest = serde_json::json!({
        "name": "names-v1.tar", "format": 1, "version": "test", "canvas": [16, 28], "mark_canvas": [16, 8],
        "glyphs": tiles.len(), "marks": 0, "files": meta
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}

#[test]
fn reads_synthetic_letters_back_and_abstains_without_a_line() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("names-v1");
    write_pack(&dir, &["ABC", "CBA", "BAC"]);
    let pack = NamePack::load(&dir).expect("pack");
    assert_eq!(pack.templates(), 9);
    assert_eq!(
        pack.params,
        Params::DEFAULT,
        "a pack without params uses the defaults"
    );
    let read = read_row_name(&row("CAB"), 1007, 6, &pack).expect("a read");
    assert_eq!(read.text, "CAB");
    assert!(read.confidence >= pack.params.min_confidence);
    assert_eq!(
        read_row_name(&row("AB C"), 1007, 6, &pack)
            .map(|r| r.text)
            .as_deref(),
        Some("AB C")
    );
    assert_eq!(
        read_row_raw(&row("AC_B"), 1007, 6, &pack).text,
        Err(Abstain::Space)
    );
    let empty = read_row_raw(&row(""), 1007, 6, &pack);
    assert_eq!(empty.text, Err(Abstain::NoLine));
    assert!(
        read_row_name(&row("A"), 1007, 6, &pack).is_none(),
        "one letter is not a name line"
    );
}

#[test]
fn any_problem_with_the_pack_refuses_all_of_it() {
    let tmp = tempfile::tempdir().unwrap();
    let good = tmp.path().join("good");
    write_pack(&good, &["ABC"]);
    assert!(NamePack::load(&good).is_ok());
    assert_eq!(
        NamePack::load(&tmp.path().join("absent")).unwrap_err(),
        PackError::Missing
    );
    assert_eq!(load_installed(tmp.path()).unwrap_err(), PackError::Missing);
    let variant = |name: &str, change: &dyn Fn(&std::path::Path)| {
        let d = tmp.path().join(name);
        write_pack(&d, &["ABC"]);
        change(&d);
        NamePack::load(&d).unwrap_err()
    };
    let invalid = |e: PackError| matches!(e, PackError::Invalid(_));
    std::fs::create_dir_all(tmp.path().join("empty")).unwrap();
    assert!(invalid(
        NamePack::load(&tmp.path().join("empty")).unwrap_err()
    ));
    assert!(invalid(variant("no_manifest", &|d| std::fs::remove_file(
        d.join("manifest.json")
    )
    .unwrap())));
    assert!(invalid(variant("bad_json", &|d| std::fs::write(
        d.join("manifest.json"),
        b"{\"name\": "
    )
    .unwrap())));
    assert!(invalid(variant("missing_file", &|d| std::fs::remove_file(
        d.join("whiten.f32")
    )
    .unwrap())));
    assert!(invalid(variant("truncated", &|d| {
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(d.join("whiten.f32"))
            .unwrap();
        f.set_len(1000).unwrap();
    })));
    assert!(invalid(variant("flipped", &|d| {
        let p = d.join("glyphs.txt");
        let mut b = std::fs::read(&p).unwrap();
        b[0] ^= 1;
        std::fs::write(&p, b).unwrap();
    })));
    assert!(invalid(variant("wrong_name", &|d| {
        let p = d.join("manifest.json");
        let s = std::fs::read_to_string(&p)
            .unwrap()
            .replace("names-v1.tar", "names-v0.tar");
        std::fs::write(&p, s).unwrap();
    })));
}

fn set_min_confidence(dir: &std::path::Path, min_confidence: f32) {
    let path = dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["params"] = serde_json::json!({ "min_confidence": min_confidence });
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

/// A name cell. `confidence` is the thousandths [`read_names`] stores.
fn named(value: &str, confidence: i32) -> crate::ocr::RowOcrResult {
    crate::ocr::RowOcrResult {
        name: Some(crate::ocr::CellOcrResult {
            value: value.into(),
            confidence,
            suspect: false,
        }),
        name_alternates: Vec::new(),
        stats: Vec::new(),
        mean_confidence: 0,
    }
}

fn blank_row() -> crate::ocr::RowOcrResult {
    crate::ocr::RowOcrResult {
        name: None,
        name_alternates: Vec::new(),
        stats: Vec::new(),
        mean_confidence: 0,
    }
}

/// `CAB` here is three synthetic letters, not a member's name.
#[test]
fn the_switch_off_uses_tesseract_and_does_not_read_the_pack() {
    let board = image::DynamicImage::ImageRgb8(image::RgbImage::new(8, 8));
    let switch = NameSwitch::off();
    let off = crate::ocr::gate::scoped(crate::ocr::gate::OcrMode::Off);
    let mut called = false;
    let found = match_player_row(&board, 5, "CAB", &switch, BoardMatch::Cells, || {
        called = true;
        vec![named("CAB", 10)]
    });
    assert!(called);
    assert_eq!(
        found.map(|m| (m.name.row, m.recognizer)),
        Some((0, NameRecognizer::OcrV1))
    );
    assert!(
        off.blocked().is_empty(),
        "the switch off does not call read_names"
    );

    // A pack read sitting on another row is ignored.
    let mut called = false;
    let found = choose_player_row(
        "CAB",
        5,
        false,
        Some(ReaderNames {
            rows: &[named("ZZZ", 900)],
            min_confidence: Some(Params::DEFAULT.min_confidence),
        }),
        BoardMatch::Cells,
        || {
            called = true;
            vec![named("CAB", 10)]
        },
    );
    assert!(called);
    assert_eq!(
        found.map(|m| (m.name.row, m.recognizer)),
        Some((0, NameRecognizer::OcrV1))
    );
}

#[test]
fn the_switch_on_with_no_pack_uses_read_names_and_not_a_second_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let loaded = NameSwitch::load(false, tmp.path());
    assert!(!loaded.enabled);
    assert!(loaded.pack().is_none());
    let missing = NameSwitch::load(true, tmp.path());
    assert!(missing.enabled);
    assert!(missing.pack().is_none());

    let board = image::DynamicImage::ImageRgb8(image::RgbImage::new(8, 8));
    let off = crate::ocr::gate::scoped(crate::ocr::gate::OcrMode::Off);
    let mut called = false;
    let found = match_player_row(&board, 5, "CAB", &missing, BoardMatch::FullBoard, || {
        called = true;
        (0..10).map(|_| named("CAB", 90)).collect()
    });
    assert!(!called, "no pack: read_names is the Tesseract path");
    assert!(found.is_none(), "OCR off leaves that path with no rows");
    let blocked = off.blocked();
    assert_eq!(blocked.len(), 1, "{blocked:?}");
    assert_eq!(
        blocked[0].site.entry,
        "recognize_scoreboard_names_pre_cropped"
    );

    // No pack: the rows read_names already returned are Tesseract, so the
    // upload id is ocr-v1 and tesseract_rows is not called again.
    let found = choose_player_row(
        "CAB",
        1,
        true,
        Some(ReaderNames {
            rows: &[named("CAB", 40), blank_row()],
            min_confidence: None,
        }),
        BoardMatch::Cells,
        || panic!("no pack does not call tesseract_rows"),
    );
    assert_eq!(
        found.map(|m| (m.name.row, m.recognizer)),
        Some((0, NameRecognizer::OcrV1))
    );
}

#[test]
fn a_confident_template_name_is_used_and_a_lower_one_falls_back() {
    let tmp = tempfile::tempdir().unwrap();
    let lo_dir = tmp.path().join("lo");
    let hi_dir = tmp.path().join("hi");
    write_pack(&lo_dir, &["ABC"]);
    write_pack(&hi_dir, &["ABC"]);
    // 0.020 rounds to 20 thousandths. The default 0.016 rounds to 16.
    // A row at 17 passes the default pack and fails this one, so a
    // hardcoded 0.016 would accept both.
    set_min_confidence(&hi_dir, 0.020);
    let lo = NamePack::load(&lo_dir).unwrap();
    let hi = NamePack::load(&hi_dir).unwrap();
    assert_eq!(confidence_millis(lo.params.min_confidence), 16);
    assert_eq!(confidence_millis(hi.params.min_confidence), 20);
    assert_ne!(
        confidence_millis(lo.params.min_confidence),
        confidence_millis(hi.params.min_confidence)
    );

    // Row 0 is the configured letters at 17 thousandths. Row 1 abstains.
    let rows = [named("CAB", 17), blank_row()];
    let pick = |min: f32| {
        let mut called = false;
        let found = choose_player_row(
            "CAB",
            1,
            true,
            Some(ReaderNames {
                rows: &rows,
                min_confidence: Some(min),
            }),
            BoardMatch::FullBoard,
            || {
                called = true;
                vec![named("FALLBACK", 90), named("OTHER", 90)]
            },
        );
        (found.map(|m| (m.name.row, m.recognizer)), called)
    };
    assert_eq!(
        pick(lo.params.min_confidence),
        (Some((0, NameRecognizer::NamesV1)), false)
    );
    assert_eq!(pick(hi.params.min_confidence), (None, true));

    // The pack threshold itself passes. One thousandth under it does not.
    let at = confidence_millis(lo.params.min_confidence);
    let edge = [named("CAB", at), named("OTHER", 90)];
    let under = [named("CAB", at - 1), named("OTHER", 90)];
    let on_edge = choose_player_row(
        "CAB",
        1,
        true,
        Some(ReaderNames {
            rows: &edge,
            min_confidence: Some(lo.params.min_confidence),
        }),
        BoardMatch::FullBoard,
        || panic!("a row at the pack threshold does not need Tesseract"),
    );
    assert_eq!(
        on_edge.map(|m| (m.name.row, m.recognizer)),
        Some((0, NameRecognizer::NamesV1))
    );
    let mut called = false;
    let below = choose_player_row(
        "CAB",
        1,
        true,
        Some(ReaderNames {
            rows: &under,
            min_confidence: Some(lo.params.min_confidence),
        }),
        BoardMatch::FullBoard,
        || {
            called = true;
            vec![named("FALLBACK", 90), named("OTHER", 90)]
        },
    );
    assert!(called);
    assert!(below.is_none());
}

#[test]
fn a_blank_own_row_falls_back_and_a_sure_board_without_the_player_does_not() {
    let min = Params::DEFAULT.min_confidence;
    let rows = [blank_row(), named("OTHER", 90)];
    let mut called = false;
    let found = choose_player_row(
        "CAB",
        1,
        true,
        Some(ReaderNames {
            rows: &rows,
            min_confidence: Some(min),
        }),
        BoardMatch::FullBoard,
        || {
            called = true;
            vec![named("CAB", 40), named("OTHER", 80)]
        },
    );
    assert!(called);
    assert_eq!(
        found.map(|m| (m.name.row, m.recognizer)),
        Some((0, NameRecognizer::OcrV1))
    );

    // A short board still keeps a sure template name on the player's row.
    let short = [named("CAB", 90)];
    let kept = choose_player_row(
        "CAB",
        1,
        true,
        Some(ReaderNames {
            rows: &short,
            min_confidence: Some(min),
        }),
        BoardMatch::Cells,
        || vec![named("FALLBACK", 40)],
    );
    assert_eq!(
        kept.map(|m| (m.name.row, m.recognizer)),
        Some((0, NameRecognizer::NamesV1))
    );

    let sure = [named("OTHER", 90), named("OTHER", 90)];
    let missed = choose_player_row(
        "CAB",
        1,
        true,
        Some(ReaderNames {
            rows: &sure,
            min_confidence: Some(min),
        }),
        BoardMatch::FullBoard,
        || panic!("a full board of sure names does not call Tesseract"),
    );
    assert!(missed.is_none());
}

#[test]
fn a_loaded_pack_that_abstains_fills_the_row_from_tesseract() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("templates").join("names-v1");
    write_pack(&dir, &["ABC"]);
    let switch = NameSwitch::load(true, tmp.path());
    assert!(switch.pack().is_some());
    // Flat colour: no letter line, so every row is blank.
    let board = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        64,
        64,
        image::Rgb([12, 12, 18]),
    ));
    let mut called = false;
    let found = match_player_row(&board, 1, "CAB", &switch, BoardMatch::FullBoard, || {
        called = true;
        vec![named("CAB", 70), named("OTHER", 70)]
    });
    assert!(called);
    assert_eq!(
        found.map(|m| (m.name.row, m.recognizer)),
        Some((0, NameRecognizer::OcrV1))
    );
}

/// Pack sync can land names-v1 after the reader has started. The read that
/// notices the new folder still uses Tesseract. The next read uses the pack.
#[test]
fn a_names_pack_that_lands_after_startup_is_reloaded() {
    let tmp = tempfile::tempdir().unwrap();
    let switch = NameSwitch::load(true, tmp.path());
    assert!(switch.pack().is_none(), "startup has no pack");

    let staging = tmp.path().join("templates").join(".names-v1-staging");
    write_pack(&staging, &["ABC"]);
    crate::packs::swap_dir(&staging, &crate::names::pack::dir_in(tmp.path())).unwrap();

    let board = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        64,
        64,
        image::Rgb([12, 12, 18]),
    ));
    let off = crate::ocr::gate::scoped(crate::ocr::gate::OcrMode::Off);
    let mut called = false;
    let found = match_player_row(&board, 1, "CAB", &switch, BoardMatch::FullBoard, || {
        called = true;
        vec![named("CAB", 70), named("OTHER", 70)]
    });
    assert!(!called, "the in-flight reload is not a second fallback");
    assert!(found.is_none());
    assert_eq!(
        off.blocked().len(),
        1,
        "until the load finishes, read_names is the Tesseract path"
    );
    drop(off);

    switch.wait_reload();
    let loaded = switch.pack().expect("the landed pack is loaded");
    assert_eq!(switch.reload_spawns(), 1, "the late load runs once");
    for _ in 0..20 {
        assert!(std::sync::Arc::ptr_eq(
            &loaded,
            &switch.pack().expect("cached")
        ));
    }
    assert_eq!(switch.reload_spawns(), 1, "later reads do not load again");
    let again = tmp.path().join("templates").join(".names-v1-again");
    write_pack(&again, &["CAB"]);
    crate::packs::swap_dir(&again, &crate::names::pack::dir_in(tmp.path())).unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&loaded, &switch.pack().expect("still cached")),
        "a pack swapped in after the first load waits for the next start"
    );
    assert_eq!(switch.reload_spawns(), 1);
    let mut called = false;
    let found = match_player_row(&board, 1, "CAB", &switch, BoardMatch::FullBoard, || {
        called = true;
        vec![named("CAB", 70), named("OTHER", 70)]
    });
    assert!(
        called,
        "the loaded pack abstains and fills the row from Tesseract"
    );
    assert_eq!(
        found.map(|m| (m.name.row, m.recognizer)),
        Some((0, NameRecognizer::OcrV1))
    );
}

/// A pack that is already installed at startup is loaded then, and a later
/// read does not load it again.
#[test]
fn a_names_pack_loaded_at_startup_is_cached() {
    let tmp = tempfile::tempdir().unwrap();
    write_pack(&crate::names::pack::dir_in(tmp.path()), &["ABC"]);
    let switch = NameSwitch::load(true, tmp.path());
    let loaded = switch.pack().expect("startup load");
    assert_eq!(switch.reload_spawns(), 0, "startup is not a late load");
    for _ in 0..20 {
        assert!(std::sync::Arc::ptr_eq(
            &loaded,
            &switch.pack().expect("cached")
        ));
    }
    assert_eq!(switch.reload_spawns(), 0);
    let staging = tmp.path().join("templates").join(".names-v1-next");
    write_pack(&staging, &["CAB"]);
    crate::packs::swap_dir(&staging, &crate::names::pack::dir_in(tmp.path())).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &loaded,
        &switch.pack().expect("still the startup pack")
    ));
    assert_eq!(switch.reload_spawns(), 0);
}
