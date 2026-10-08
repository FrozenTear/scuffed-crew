//! Real Dorado stat cells at native 2560x1440 scoreboard scale.
//!
//! Live captures call [`ocr::recognize_cell`]. That uses the bright HSV mask
//! and falls through to [`preprocess::dim_zero_glyph`] only when the mask
//! leaves the cell almost empty. Each crop is run as given and with 2, 3,
//! and 4 px trimmed from every side, because column calibration will not
//! match the crop exactly.
//!
//! A dim zero must come back as "0". A white digit must come back as its
//! own value, and the dim-zero check must not claim it.

use image::DynamicImage;
use stat_tracker::ocr::{self, preprocess};

struct Cell {
    file: &'static str,
    expect: &'static str,
    /// This crop is a dim zero, so the geometric check must see the ring.
    dim_zero: bool,
}

const CELLS: &[Cell] = &[
    Cell {
        file: "dorado_purple_assists_dim0.png",
        expect: "0",
        dim_zero: true,
    },
    Cell {
        file: "dorado_purple_healing_dim0.png",
        expect: "0",
        dim_zero: true,
    },
    Cell {
        file: "dorado_purple_healing_dim0_row4.png",
        expect: "0",
        dim_zero: true,
    },
    Cell {
        file: "dorado_purple_mitigation_dim0.png",
        expect: "0",
        dim_zero: true,
    },
    Cell {
        file: "dorado_yellow_assists_dim0.png",
        expect: "0",
        dim_zero: true,
    },
    Cell {
        file: "dorado_yellow_assists_dim0_row5.png",
        expect: "0",
        dim_zero: true,
    },
    Cell {
        file: "dorado_yellow_deaths_dim0.png",
        expect: "0",
        dim_zero: true,
    },
    Cell {
        file: "dorado_yellow_healing_dim0.png",
        expect: "0",
        dim_zero: true,
    },
    Cell {
        file: "dorado_yellow_mitigation_dim0.png",
        expect: "0",
        dim_zero: true,
    },
    Cell {
        file: "dorado_purple_elims_white13.png",
        expect: "13",
        dim_zero: false,
    },
    Cell {
        file: "dorado_purple_elims_white11.png",
        expect: "11",
        dim_zero: false,
    },
    Cell {
        file: "dorado_purple_assists_white1.png",
        expect: "1",
        dim_zero: false,
    },
    Cell {
        file: "dorado_purple_deaths_white4.png",
        expect: "4",
        dim_zero: false,
    },
    Cell {
        file: "dorado_purple_assists_white9.png",
        expect: "9",
        dim_zero: false,
    },
    Cell {
        file: "dorado_yellow_elims_white7.png",
        expect: "7",
        dim_zero: false,
    },
    Cell {
        file: "dorado_yellow_deaths_white5.png",
        expect: "5",
        dim_zero: false,
    },
];

fn trimmed(img: &DynamicImage, px: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    img.crop_imm(px, px, w - px * 2, h - px * 2)
}

#[test]
fn real_dorado_cells_read_their_digits() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data/real-cells");
    let mut lines = Vec::new();
    let mut failures = Vec::new();
    for cell in CELLS {
        let path = dir.join(cell.file);
        let img = image::open(&path).unwrap_or_else(|err| panic!("open {}: {err}", path.display()));
        for px in [0u32, 2, 3, 4] {
            let crop = if px == 0 {
                img.clone()
            } else {
                trimmed(&img, px)
            };
            let binary = preprocess::prepare_cell_binary(&crop);
            let ink = binary.pixels().filter(|p| p.0[0] < 128).count();
            let dim = preprocess::dim_zero_glyph(&crop, 2);
            let read = ocr::recognize_cell(&crop)
                .unwrap_or_else(|err| panic!("{} trim {px}: {err}", cell.file));
            let line = format!(
                "{} trim {px}: value={:?} conf={} suspect={} bright_ink={ink} dim_fallback={} dim_hit={}",
                cell.file,
                read.value,
                read.confidence,
                read.suspect,
                ink < 20,
                dim.is_some()
            );
            eprintln!("{line}");
            lines.push(line.clone());
            if read.value != cell.expect {
                failures.push(format!("want {:?}: {line}", cell.expect));
            }
            if cell.dim_zero && dim.is_none() {
                failures.push(format!("dim check missed the zero: {line}"));
            }
            if !cell.dim_zero && dim.is_some() {
                failures.push(format!("dim check claimed a white digit: {line}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "real cells:\n{}\n\nall:\n{}",
        failures.join("\n"),
        lines.join("\n")
    );
}
