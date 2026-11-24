//! Which Tesseract the ocr-v1 fixture tests run on, and the few fixture
//! cells it reads differently.
//!
//! The fixtures were pinned on the CI runner's Tesseract and on 5.5.0.
//! Tesseract 5.5.3, the engine releases bundle and CachyOS ships, reads a
//! handful of them differently with the same eng.traineddata (tessdata_fast).
//! It does so with Leptonica 1.87, 1.84 and 1.82 alike, and 5.5.0 reads them
//! as pinned with 1.84 and 1.82, so the Tesseract library is what changed.
//! It reads a lone 1 and a white 11 as empty, a template 8 or 5 as 3, and one
//! dim zero itself before the ring check does.
//!
//! Each such read is listed in [`VARIANTS`] by fixture cell, with the exact
//! value that engine returns. A test accepts a listed read only on the
//! engine it is listed for. Any other read, a listed cell read some third
//! way, or a variant on any other engine still fails, so a real regression
//! in the reader does not hide behind this list. A new engine that reads a
//! cell differently fails until its read is checked and added here.

use std::ffi::CStr;

/// One fixture cell that an engine reads differently from the pinned value.
struct Variant {
    /// Tesseract version string, as `TessVersion()` gives it.
    tesseract: &'static str,
    /// The test's own name for the cell.
    cell: &'static str,
    /// What that engine reads there.
    read: &'static str,
}

const fn v553(cell: &'static str, read: &'static str) -> Variant {
    Variant {
        tesseract: "5.5.3",
        cell,
        read,
    }
}

/// Measured 2026-10-27 on Tesseract 5.5.3 with Leptonica 1.87.0, 1.84.1 and
/// 1.82.0, against 5.5.0 with Leptonica 1.84.1 and 1.82.0, all with the
/// same tessdata_fast eng.traineddata.
const VARIANTS: &[Variant] = &[
    // real_dim_zero_cells: a white 11 comes back empty (both 1s lost).
    v553("dorado_purple_elims_white11.png trim 0", ""),
    v553("dorado_purple_elims_white11.png trim 3", ""),
    // vision_baseline_fixtures, 0.75x board: a lone 1 comes back empty.
    v553("s075_tab_board_stats_only.png r8c1", ""),
    // vision_baseline_fixtures, dim zeros: Tesseract reads this 0 itself on
    // the bright path, so the confidence is its own 88, not the ring's 87.
    v553("s075_yellow_healing_dim0.png confidence", "88"),
    // vision_baseline_fixtures, native post-game table. Only the cells 5.5.0
    // reads right and 5.5.3 does not are listed: cells both engines miss
    // stay inside the test's own floor.
    v553("native_postgame_table_names_blanked.png r6c1", "3"),
    v553("native_postgame_table_names_blanked.png r9c1", "3"),
    v553("native_postgame_table_names_blanked.png r11c1", "3"),
];

/// The linked Tesseract's version, e.g. "5.5.0".
pub fn tesseract_version() -> String {
    // SAFETY: TessVersion returns a pointer to a static C string.
    unsafe { CStr::from_ptr(leptess::capi::TessVersion()) }
        .to_string_lossy()
        .into_owned()
}

/// The linked Leptonica's version, e.g. "leptonica-1.84.1".
pub fn leptonica_version() -> String {
    // SAFETY: getLeptonicaVersion returns a new C string the caller frees
    // with lept_free.
    unsafe {
        let ptr = leptess::capi::getLeptonicaVersion();
        let out = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        leptess::capi::lept_free(ptr.cast());
        out
    }
}

/// True when the running engine is listed as reading `cell` as `read`.
pub fn known_variant(cell: &str, read: &str) -> bool {
    let tesseract = tesseract_version();
    VARIANTS
        .iter()
        .any(|v| v.tesseract == tesseract && v.cell == cell && v.read == read)
}

/// Names the engine in the test log, so a fixture failure says which one.
#[test]
fn the_fixture_engine_is_named() {
    let (tesseract, leptonica) = (tesseract_version(), leptonica_version());
    eprintln!("ocr-v1 fixtures run on Tesseract {tesseract} with {leptonica}");
    assert!(
        tesseract.chars().next().is_some_and(|c| c.is_ascii_digit()),
        "unexpected Tesseract version string {tesseract:?}"
    );
}

/// Every listed cell names a fixture file, so a renamed fixture cannot leave
/// a variant that matches nothing.
#[test]
fn every_variant_names_a_fixture() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data");
    for v in VARIANTS {
        let file = v.cell.split(' ').next().unwrap_or_default();
        let found = ["real-cells", "vision-baseline"]
            .iter()
            .any(|dir| root.join(dir).join(file).is_file());
        assert!(found, "variant for {:?} names no fixture", v.cell);
    }
}
