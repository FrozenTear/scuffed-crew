//! Without a usable names-v1 pack, `names::read_names` is today's ocr-v1
//! name path: the same reads byte for byte, the same Tesseract calls, and
//! nothing read with OCR off. Boards are synthetic (no game art, no names).
//! The test compares the process-wide Tesseract call count, so it runs in a
//! process of its own.

use crate::common;

use stat_tracker::names::{self, NamePack, NameRecognizer, PackError};
use stat_tracker::ocr::{self, gate};

/// A cropped board: two team bands of rows, each row with blocky white
/// "text" where the name windows are and a few stat-like blobs.
fn synthetic_board(w: u32, h: u32, team: usize, seed: u32) -> image::DynamicImage {
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([20, 20, 30]));
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let rows = team * 2;
    let pitch = h as f32 * if team == 5 { 0.087 } else { 0.074 };
    let top = h as f32 * 0.04;
    for r in 0..rows {
        let y0 = (top + r as f32 * pitch + if r >= team { pitch * 0.4 } else { 0.0 }) as u32;
        let y1 = ((y0 as f32 + pitch * 0.95) as u32).min(h);
        let bg = if r < team {
            image::Rgb([70, 20, 80])
        } else {
            image::Rgb([110, 95, 20])
        };
        for y in y0..y1 {
            for x in 0..(w * 6 / 10) {
                img.put_pixel(x, y, bg);
            }
        }
        // name-like strokes
        let cap = (pitch * 0.29) as u32;
        let base = y0 + (pitch * 0.33) as u32;
        let mut x = w / 16 + next() % (w / 16);
        for _ in 0..(4 + next() % 8) {
            let gw = cap / 2 + next() % (cap / 2 + 1);
            for yy in base..(base + cap).min(h) {
                for xx in x..(x + gw).min(w) {
                    let edge = xx < x + 2 || xx + 2 >= x + gw || yy < base + 2 || next() % 5 == 0;
                    if edge {
                        img.put_pixel(xx, yy, image::Rgb([240, 240, 240]));
                    }
                }
            }
            x += gw + 3 + next() % 4;
        }
        // stat-like blobs
        for k in 0..6u32 {
            let sx = w * 28 / 100 + k * w / 20;
            for yy in base..(base + cap * 2 / 3).min(h) {
                for xx in sx..sx + 3 + next() % 6 {
                    img.put_pixel(xx, yy, image::Rgb([230, 230, 230]));
                }
            }
        }
    }
    image::DynamicImage::ImageRgb8(img)
}

fn boards() -> Vec<(image::DynamicImage, usize)> {
    let mut out = Vec::new();
    for (w, h) in [(1664, 1007), (1248, 756)] {
        for team in [6, 5] {
            for seed in 0..3 {
                out.push((synthetic_board(w, h, team, seed + 7 * team as u32), team));
            }
        }
    }
    out
}

#[test]
fn without_a_pack_names_are_read_exactly_as_ocr_v1_reads_them() {
    if !common::own_process(
        module_path!(),
        "without_a_pack_names_are_read_exactly_as_ocr_v1_reads_them",
    ) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // Every way of having no usable pack.
    let empty = tmp.path().join("templates").join("names-v1");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(matches!(
        names::load_installed(tmp.path()),
        Err(PackError::Invalid(_))
    ));
    assert_eq!(
        NamePack::load(&tmp.path().join("nowhere")).unwrap_err(),
        PackError::Missing
    );
    let pack = names::load_installed(tmp.path()).ok();
    assert!(pack.is_none());

    let mut any_text = false;
    for (board, team) in boards() {
        let calls0 = gate::tesseract_calls();
        let today = ocr::recognize_scoreboard_names_pre_cropped(&board, team);
        let calls1 = gate::tesseract_calls();
        let ours = names::read_names(&board, team, pack.as_ref());
        let calls2 = gate::tesseract_calls();
        assert_eq!(ours.recognizer, NameRecognizer::OcrV1);
        assert_eq!(
            format!("{today:?}"),
            format!("{:?}", ours.rows),
            "{team}v{team}"
        );
        assert_eq!(calls1 - calls0, calls2 - calls1, "same Tesseract calls");
        assert_eq!(today.len(), team * 2);
        any_text |= today
            .iter()
            .any(|r| r.name.as_ref().is_some_and(|n| !n.value.is_empty()));
    }
    assert!(
        any_text,
        "the synthetic boards give Tesseract something to read"
    );

    // OCR off: nothing read, the same refusal counted, no Tesseract call.
    let (board, team) = boards().remove(0);
    let off = gate::scoped(gate::OcrMode::Off);
    let calls = gate::tesseract_calls();
    assert!(ocr::recognize_scoreboard_names_pre_cropped(&board, team).is_empty());
    assert!(names::read_names(&board, team, None).rows.is_empty());
    assert_eq!(gate::tesseract_calls(), calls);
    // Each refusal is counted under the same entry, at the line that asked
    // (read_names passes its caller's line through).
    let blocked = off.blocked();
    assert_eq!(blocked.len(), 2, "{blocked:?}");
    for b in &blocked {
        assert_eq!(b.site.entry, "recognize_scoreboard_names_pre_cropped");
        assert!(
            b.site.file.ends_with("tests/integration/names_fallback.rs"),
            "{}",
            b.site
        );
        assert_eq!(b.count, 1);
    }
}
