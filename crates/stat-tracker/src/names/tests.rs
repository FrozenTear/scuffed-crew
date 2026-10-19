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
    assert!(invalid(variant("grown", &|d| {
        let p = d.join("whiten.f32");
        let mut b = std::fs::read(&p).unwrap();
        b.extend_from_slice(&[0; 4]);
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

/// Rewrite a test pack's manifest `files` map into pack sync's array form.
fn to_files_array(dir: &std::path::Path, drop: Option<&str>, bad_sha: Option<&str>) {
    let p = dir.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let list: Vec<serde_json::Value> = m["files"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(name, _)| Some(name.as_str()) != drop)
        .map(|(name, v)| {
            let sha = if Some(name.as_str()) == bad_sha {
                "0".repeat(64)
            } else {
                v["sha256"].as_str().unwrap().to_owned()
            };
            serde_json::json!({"path": name, "sha256": sha, "size": v["size"]})
        })
        .collect();
    m["files"] = serde_json::Value::Array(list);
    std::fs::write(&p, serde_json::to_vec(&m).unwrap()).unwrap();
}

#[test]
fn loads_the_pack_sync_files_array_and_the_files_map_alike() {
    let tmp = tempfile::tempdir().unwrap();
    let map = tmp.path().join("map");
    let array = tmp.path().join("array");
    write_pack(&map, &["ABC", "CBA", "BAC"]);
    write_pack(&array, &["ABC", "CBA", "BAC"]);
    to_files_array(&array, None, None);
    let from_map = NamePack::load(&map).expect("map form");
    let from_array = NamePack::load(&array).expect("array form");
    assert_eq!(from_array.templates(), from_map.templates());
    assert_eq!(from_array.params, from_map.params);
    assert_eq!(from_array.z, from_map.z);
    assert_eq!(
        read_row_name(&row("CAB"), 1007, 6, &from_array).map(|r| r.text),
        read_row_name(&row("CAB"), 1007, 6, &from_map).map(|r| r.text)
    );
    let refused = |name: &str, drop: Option<&str>, bad_sha: Option<&str>| {
        let d = tmp.path().join(name);
        write_pack(&d, &["ABC"]);
        to_files_array(&d, drop, bad_sha);
        matches!(NamePack::load(&d).unwrap_err(), PackError::Invalid(_))
    };
    assert!(refused("array_unlisted", Some("whiten.f32"), None));
    assert!(refused("array_bad_sha", None, Some("glyphs.png")));
}

/// Rewrite `glyphs.txt`'s first width to `width` and fix its manifest entry,
/// so only the value itself is wrong.
fn set_first_width(dir: &std::path::Path, width: &str) {
    let p = dir.join("glyphs.txt");
    let text = std::fs::read_to_string(&p).unwrap();
    let (first, rest) = text.split_once('\n').unwrap();
    let letter = first.split('\t').next().unwrap();
    let new = format!("{letter}\t{width}\n{rest}");
    std::fs::write(&p, &new).unwrap();
    let mp = dir.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&mp).unwrap()).unwrap();
    m["files"]["glyphs.txt"] = serde_json::json!({
        "size": new.len(),
        "sha256": format!("{:x}", Sha256::digest(new.as_bytes())),
    });
    std::fs::write(&mp, serde_json::to_vec(&m).unwrap()).unwrap();
}

/// Set (or with `None`, remove) the manifest's `version`.
fn set_version(dir: &std::path::Path, version: Option<&str>) {
    let mp = dir.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&mp).unwrap()).unwrap();
    match version {
        Some(v) => m["version"] = serde_json::json!(v),
        None => {
            m.as_object_mut().unwrap().remove("version");
        }
    }
    std::fs::write(&mp, serde_json::to_vec(&m).unwrap()).unwrap();
}

#[test]
fn a_width_that_is_not_a_positive_number_refuses_the_pack() {
    let tmp = tempfile::tempdir().unwrap();
    let ok = tmp.path().join("ok");
    write_pack(&ok, &["ABC"]);
    set_first_width(&ok, "0.5");
    assert!(NamePack::load(&ok).is_ok(), "the rewrite itself is sound");
    for (i, w) in ["NaN", "nan", "inf", "-inf", "infinity", "0", "-0.5"]
        .iter()
        .enumerate()
    {
        let d = tmp.path().join(format!("w{i}"));
        write_pack(&d, &["ABC"]);
        set_first_width(&d, w);
        assert_eq!(
            NamePack::load(&d).unwrap_err(),
            PackError::Invalid("glyphs.txt width".into()),
            "width {w}"
        );
    }
}

#[test]
fn a_pack_without_a_version_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let named = tmp.path().join("named");
    write_pack(&named, &["ABC"]);
    assert_eq!(NamePack::load(&named).unwrap().version, "test");
    for (name, v) in [("empty", Some("")), ("blank", Some("  ")), ("absent", None)] {
        let d = tmp.path().join(name);
        write_pack(&d, &["ABC"]);
        set_version(&d, v);
        assert_eq!(
            NamePack::load(&d).unwrap_err(),
            PackError::Invalid("no version".into()),
            "{name}"
        );
    }
}
