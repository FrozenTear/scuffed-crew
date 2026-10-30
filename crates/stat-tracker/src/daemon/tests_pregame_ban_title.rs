//! Tests for the ban-results title check, next to `pregame_ban_title.rs`.

use super::pregame_ban_title::*;
use super::pregame_dump::*;
use crate::*;

/// Fill a rectangle given in fractions of the title region.
fn fill(img: &mut image::RgbImage, (fx0, fy0, fx1, fy1): (f32, f32, f32, f32), rgb: [u8; 3]) {
    let (w, h) = img.dimensions();
    let [x0, y0, x1, y1] = BAN_RESULTS_TITLE;
    let (rx, ry) = (x0 * w as f32, y0 * h as f32);
    let (rw, rh) = ((x1 - x0) * w as f32, (y1 - y0) * h as f32);
    for y in (ry + rh * fy0) as u32..(ry + rh * fy1) as u32 {
        for x in (rx + rw * fx0) as u32..(rx + rw * fx1) as u32 {
            img.put_pixel(x, y, image::Rgb(rgb));
        }
    }
}

const GROUND: [u8; 3] = [18, 20, 26];
const TEXT: [u8; 3] = [238, 238, 242];

/// A title built to the shape rule: white letter blocks from 15% to 85%
/// of the width, in the middle 40% of the height, on a dark ground.
fn shaped(w: u32, h: u32) -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb(GROUND));
    for k in 0..14 {
        let x = 0.15 + 0.05 * k as f32;
        fill(&mut img, (x, 0.3, x + 0.035, 0.7), TEXT);
    }
    img
}

fn metrics(img: &image::RgbImage) -> ShapeMetrics {
    shape_metrics(img).expect("a title region")
}

/// Which conditions of the rule a frame fails.
fn failing(m: &ShapeMetrics) -> Vec<&'static str> {
    let r = &SHAPE;
    let mut out = Vec::new();
    if m.bright < r.bright_min {
        out.push("bright_min");
    }
    if m.bright > r.bright_max {
        out.push("bright_max");
    }
    if m.dark < r.dark_min {
        out.push("dark_min");
    }
    if !m
        .centroid
        .is_some_and(|c| (r.centroid_min..=r.centroid_max).contains(&c))
    {
        out.push("centroid");
    }
    if m.span < r.span_min {
        out.push("span");
    }
    if m.top_bright > r.edge_bright_max {
        out.push("top");
    }
    if m.bottom_bright > r.edge_bright_max {
        out.push("bottom");
    }
    out
}

#[test]
fn a_title_built_to_the_shape_rule_passes_at_1440p_and_720p() {
    for (w, h) in [(2560, 1440), (1920, 1080), (1280, 720)] {
        let m = metrics(&shaped(w, h));
        assert!(m.passes(), "{w}x{h}: {m:?} fails {:?}", failing(&m));
    }
    assert!(BanTitleExamples::default().matches(&shaped(1280, 720)));
}

#[test]
fn each_shape_condition_alone_turns_a_frame_away() {
    let (w, h) = (1280, 720);
    // Too little bright: thin strokes.
    let mut thin = image::RgbImage::from_pixel(w, h, image::Rgb(GROUND));
    for k in 0..14 {
        let x = 0.15 + 0.05 * k as f32;
        fill(&mut thin, (x, 0.45, x + 0.01, 0.55), TEXT);
    }
    // Too much bright: a solid block.
    let mut solid = image::RgbImage::from_pixel(w, h, image::Rgb(GROUND));
    fill(&mut solid, (0.12, 0.12, 0.88, 0.88), TEXT);
    // Not dark enough: the same title on a mid-grey ground.
    let mut grey = shaped(w, h);
    for p in grey.pixels_mut() {
        if p.0 == GROUND {
            p.0 = [110, 110, 110];
        }
    }
    // Centroid off: dense on the left, one stroke far right for the span.
    let mut left = image::RgbImage::from_pixel(w, h, image::Rgb(GROUND));
    fill(&mut left, (0.05, 0.25, 0.35, 0.75), TEXT);
    fill(&mut left, (0.69, 0.4, 0.71, 0.6), TEXT);
    // Too narrow: a compact word in the middle.
    let mut narrow = image::RgbImage::from_pixel(w, h, image::Rgb(GROUND));
    fill(&mut narrow, (0.3, 0.25, 0.7, 0.75), TEXT);
    // Bright in the top or bottom tenth.
    let mut top = shaped(w, h);
    fill(&mut top, (0.0, 0.0, 1.0, 0.08), TEXT);
    let mut bottom = shaped(w, h);
    fill(&mut bottom, (0.0, 0.93, 1.0, 1.0), TEXT);

    for (img, condition) in [
        (thin, "bright_min"),
        (solid, "bright_max"),
        (grey, "dark_min"),
        (left, "centroid"),
        (narrow, "span"),
        (top, "top"),
        (bottom, "bottom"),
    ] {
        let m = metrics(&img);
        assert_eq!(failing(&m), [condition], "{m:?}");
        assert!(!m.passes());
        assert!(!BanTitleExamples::default().matches(&img));
    }
}

#[test]
fn the_ban_vote_hero_select_and_roster_fail_the_shape() {
    // No bright pixel at all (hero select, roster).
    let roster = image::RgbImage::from_pixel(1280, 720, [30, 34, 44].into());
    assert!(!metrics(&roster).passes());
    assert_eq!(metrics(&roster).centroid, None);
    // A short bright word, about 2.6% bright and 42% wide (the ban vote).
    let mut vote = image::RgbImage::from_pixel(1280, 720, image::Rgb(GROUND));
    for k in 0..6 {
        let x = 0.29 + 0.07 * k as f32;
        fill(&mut vote, (x, 0.45, x + 0.02, 0.6), TEXT);
    }
    let m = metrics(&vote);
    assert!(!m.passes(), "{m:?}");
}

/// A dark frame with a title-like row of bright blocks in the title
/// region; `shift` moves the blocks (a different title).
fn titled_frame(w: u32, h: u32, shift: f32) -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(w, h, [14, 16, 22].into());
    for k in 0..9 {
        let x = (0.06 + 0.1 * k as f32 + shift) % 0.95;
        let width = if k % 3 == 0 { 0.07 } else { 0.04 };
        let height = if k % 2 == 0 { 0.4 } else { 0.25 };
        fill(&mut img, (x, 0.3, x + width, 0.3 + height), [235, 235, 240]);
    }
    img
}

#[test]
fn the_ban_results_title_matches_its_example_at_another_size() {
    let examples = BanTitleExamples::from_frames([titled_frame(2560, 1440, 0.0)]);
    assert_eq!(examples.len(), 1);
    // The same title at 1080p.
    let same = titled_frame(1920, 1080, 0.0);
    assert!(examples.best_score(&same).unwrap() >= BAN_TITLE_MATCH_MIN);
    assert!(examples.matches(&same));
    // Another title (the roster's) does not match, even one that would
    // pass the shape rule: examples replace it.
    let other = titled_frame(1920, 1080, 0.05);
    assert!(
        !examples.matches(&other),
        "{:?}",
        examples.best_score(&other)
    );
    assert!(!examples.matches(&shaped(1920, 1080)));
    // A flat title area matches nothing.
    let flat = image::RgbImage::from_pixel(1920, 1080, [14, 16, 22].into());
    assert_eq!(examples.best_score(&flat), None);
    // No examples: the shape rule decides.
    let none = BanTitleExamples::default();
    assert_eq!(none.best_score(&same), None);
    assert_eq!(
        none.matches(&same),
        shape_metrics(&same).is_some_and(|m| m.passes())
    );
}

#[test]
fn ban_results_examples_are_read_from_the_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let examples = ban_title_dir(dir.path());
    assert_eq!(BanTitleExamples::load_dir(&examples).len(), 0);
    std::fs::create_dir_all(&examples).unwrap();
    titled_frame(640, 360, 0.0)
        .save(examples.join("results.png"))
        .unwrap();
    // Upper-case extension: read too.
    titled_frame(800, 450, 0.0)
        .save_with_format(examples.join("RESULTS2.PNG"), image::ImageFormat::Png)
        .unwrap();
    // Skipped: not png, not an image, flat, a jpg.
    std::fs::write(examples.join("notes.txt"), b"not an image").unwrap();
    std::fs::write(examples.join("broken.png"), b"not a png").unwrap();
    image::RgbImage::from_pixel(640, 360, [0, 0, 0].into())
        .save(examples.join("flat.png"))
        .unwrap();
    titled_frame(640, 360, 0.0)
        .save(examples.join("steam.jpg"))
        .unwrap();
    let loaded = BanTitleExamples::load_dir(&examples);
    assert_eq!(loaded.len(), 2);
    assert!(loaded.matches(&titled_frame(1920, 1080, 0.0)));
}

#[cfg(unix)]
#[test]
fn the_example_loader_skips_fifos_symlinks_and_oversized_files() {
    let dir = tempfile::tempdir().unwrap();
    let examples = ban_title_dir(dir.path());
    std::fs::create_dir_all(&examples).unwrap();
    // A FIFO named like an example: opening it would block startup.
    let fifo = examples.join("a_fifo.png");
    let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    // A symlink to a real example outside.
    let outside = dir.path().join("outside.png");
    titled_frame(640, 360, 0.0).save(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, examples.join("b_link.png")).unwrap();
    // Wider than 8K: skipped before decoding.
    image::RgbImage::from_pixel(MAX_EXAMPLE_SIDE + 8, 4, [9, 9, 9].into())
        .save(examples.join("c_wide.png"))
        .unwrap();
    // Larger than the byte cap (sparse, never decoded).
    let big = std::fs::File::create(examples.join("d_big.png")).unwrap();
    big.set_len(MAX_EXAMPLE_BYTES + 1).unwrap();
    // One good example.
    titled_frame(640, 360, 0.0)
        .save(examples.join("e_good.png"))
        .unwrap();
    assert_eq!(BanTitleExamples::load_dir(&examples).len(), 1);
}

#[test]
fn at_most_sixteen_examples_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let examples = ban_title_dir(dir.path());
    std::fs::create_dir_all(&examples).unwrap();
    let img = titled_frame(160, 90, 0.0);
    for i in 0..MAX_EXAMPLES + 3 {
        img.save(examples.join(format!("ex{i:02}.png"))).unwrap();
    }
    assert_eq!(BanTitleExamples::load_dir(&examples).len(), MAX_EXAMPLES);
}

#[test]
fn the_real_classify_uses_the_ban_title_check_for_the_ban_title() {
    let examples = BanTitleExamples::from_frames([titled_frame(1280, 720, 0.0)]);
    let unknown = detect::GamePhase::Unknown;
    let want = PregameWant {
        loading: false,
        other_frame: true,
        select_title: false,
        ban_title: true,
    };
    assert_eq!(
        classify(want, &unknown, &titled_frame(1280, 720, 0.0), &examples),
        Seen::BanResults
    );
    assert_eq!(
        classify(want, &unknown, &titled_frame(1280, 720, 0.05), &examples),
        Seen::Other
    );
    // No examples: the shape rule.
    let none = BanTitleExamples::default();
    assert_eq!(
        classify(want, &unknown, &shaped(1280, 720), &none),
        Seen::BanResults
    );
    let roster = image::RgbImage::from_pixel(1280, 720, [30, 34, 44].into());
    assert_eq!(classify(want, &unknown, &roster, &none), Seen::Other);
    // Outside the wait the title is not looked at.
    let idle = PregameWant {
        ban_title: false,
        ..want
    };
    assert_eq!(
        classify(idle, &unknown, &shaped(1280, 720), &none),
        Seen::Other
    );
}

#[test]
fn lasting_poll_misses_are_a_declined_share_and_paused_screenshots() {
    use stat_tracker::capture::{is_expected_poll_miss, is_lasting_poll_miss};
    let backoff = format!(
        "portal screenshot: {}",
        stat_tracker::capture::portal::SHOT_BACKOFF
    );
    for lasting in [
        "Screen share was declined",
        "Screen share permission timed out: no captures until Overwatch is restarted",
        backoff.as_str(),
    ] {
        assert!(is_lasting_poll_miss(lasting), "{lasting}");
        assert!(is_expected_poll_miss(lasting), "{lasting}");
    }
    for passing in [stat_tracker::capture::STREAM_MISS, "capture failed: EIO"] {
        assert!(!is_lasting_poll_miss(passing), "{passing}");
    }
}
