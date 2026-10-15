//! Run the report blanker on a local picture and write what a report would
//! carry. For checking private boards on your own machine; nothing is sent.
//!
//! ```text
//! cargo run --release -p scuffed-stat-tracker --example blank_board -- \
//!     <board-or-frame.png> [team_size 6] [out_dir .]
//! ```
//!
//! The input is a full frame (2560x1440, 1920x1080, ...) or a saved
//! scoreboard crop (1664x1007 from `debug/accepted/`). Writes
//! `<name>.blanked.png` (the picture a report would attach) and
//! `<name>.boxes.png` (the input with everything blacked out tinted red),
//! and prints why the board or each row kept what it did. A crop has no
//! chat context, so it is blanked as if no chat is on screen.

use stat_tracker::report::blank::{self, Chat, FILL};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(input) = args.next() else {
        eprintln!("usage: blank_board <board-or-frame.png> [team_size 6] [out_dir .]");
        std::process::exit(2);
    };
    let team: usize = args.next().and_then(|t| t.parse().ok()).unwrap_or(6);
    let out_dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let img = image::open(&input).expect("open input").to_rgb8();
    let (w, h) = img.dimensions();
    let is_crop = (f64::from(w) / f64::from(h) - 1664.0 / 1007.0).abs() < 0.02;
    let (src, blanked) = if is_crop {
        (img.clone(), blank::blank_crop(&img, Some(team), Chat::None))
    } else {
        let frame = image::DynamicImage::ImageRgb8(img.clone());
        let (x, y, cw, ch) = stat_tracker::ocr::preprocess::crop_scoreboard_rect(w, h);
        let crop = image::imageops::crop_imm(&img, x, y, cw, ch).to_image();
        (crop, blank::blanked_scoreboard(&frame, Some(team)))
    };
    println!(
        "input {input} {w}x{h} ({})",
        if is_crop { "crop" } else { "frame" }
    );
    println!("board: {:?}", blanked.board);
    for r in &blanked.rows {
        println!(
            "team {} row {}: digits {:?}, portrait {:?}",
            r.team, r.idx, r.digits, r.portrait
        );
    }
    println!(
        "digit rows {}, portraits {}, name-column fallback rows {}, picture kept: {}",
        blanked.digit_rows(),
        blanked.portraits(),
        blanked.name_fill_rows,
        blanked.kept_anything()
    );
    let stem = std::path::Path::new(&input)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "board".into());
    let blanked_path = out_dir.join(format!("{stem}.blanked.png"));
    if blanked.has_picture() {
        blanked.image.save(&blanked_path).expect("write blanked");
    } else {
        println!("no picture saved: the tracker attaches no picture for this board");
    }
    let mut boxes = src.clone();
    for (x, y, p) in boxes.enumerate_pixels_mut() {
        if *blanked.image.get_pixel(x, y) == FILL {
            p.0 = [
                ((u16::from(p.0[0]) + 255) / 2) as u8,
                p.0[1] / 2,
                p.0[2] / 2,
            ];
        }
    }
    let boxes_path = out_dir.join(format!("{stem}.boxes.png"));
    boxes.save(&boxes_path).expect("write boxes");
    println!(
        "wrote {} and {}",
        blanked_path.display(),
        boxes_path.display()
    );
}
