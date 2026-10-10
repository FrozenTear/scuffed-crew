//! Eval probe: what ocr-v1 (Tesseract) reads for map name and outcome on each frame.
//! Usage: cargo run --example ocrv1_meta -- <img>...   prints TSV: path, map, outcome, outcome_text

use stat_tracker::detect::match_end::{detect_outcome, detect_outcome_text};
use stat_tracker::{ocr, parse};

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(img) = image::open(&path) else {
            println!("{path}\tERR\tERR\tERR");
            continue;
        };
        let raw = ocr::recognize_map_label(&ocr::preprocess::crop_map_name(&img));
        let map = parse::match_map_in_text(&raw).unwrap_or_default();
        let o = detect_outcome(&img);
        let t = detect_outcome_text(&img);
        println!(
            "{path}\t{map}\t{o:?}\t{t:?}\t{}",
            raw.replace(['\n', '\t'], " ")
        );
    }
}
