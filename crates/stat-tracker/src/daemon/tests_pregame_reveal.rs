//! Pre-game saver tests on synthetic frames made in code (no real
//! screenshot): the ban results revealed over several polls, what ends
//! them (the select's title or its phase read), and an `APPLYING UPDATE`
//! band that must not be saved as the loading screen.

use super::pregame_ban_title::*;
use super::pregame_dump::*;
use crate::*;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(1_500);

const GROUND: [u8; 3] = [18, 20, 26];
const TEXT: [u8; 3] = [238, 238, 242];
const ICON: [u8; 3] = [200, 120, 60];

fn fill(img: &mut image::RgbImage, (fx0, fy0, fx1, fy1): (f32, f32, f32, f32), rgb: [u8; 3]) {
    let (w, h) = img.dimensions();
    for y in (fy0 * h as f32) as u32..(fy1 * h as f32) as u32 {
        for x in (fx0 * w as f32) as u32..(fx1 * w as f32) as u32 {
            img.put_pixel(x, y, image::Rgb(rgb));
        }
    }
}

/// A 1280x720 ban results screen: the title's letter blocks in the title
/// region and `icons` of the four ban icons drawn under it.
fn ban_results(icons: usize) -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(1280, 720, image::Rgb(GROUND));
    let [x0, y0, x1, y1] = BAN_RESULTS_TITLE;
    let (rw, rh) = (x1 - x0, y1 - y0);
    for k in 0..14 {
        let x = x0 + rw * (0.15 + 0.05 * k as f32);
        fill(
            &mut img,
            (x, y0 + rh * 0.3, x + rw * 0.046, y0 + rh * 0.7),
            TEXT,
        );
    }
    for k in 0..icons {
        let x = 0.30 + 0.11 * k as f32;
        fill(&mut img, (x, 0.16, x + 0.08, 0.27), ICON);
    }
    img
}

fn shown(rgb: &image::RgbImage) -> Arc<image::DynamicImage> {
    Arc::new(image::DynamicImage::ImageRgb8(rgb.clone()))
}

#[tokio::test]
async fn the_fully_revealed_ban_results_are_the_saved_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let ban = image::RgbImage::from_pixel(64, 36, image::Rgb([40, 20, 20]));
    dump.observe(at(0), Seen::Phase(PregamePhase::HeroBan), Some(shown(&ban)));
    dump.observe(
        at(1_500),
        Seen::Phase(PregamePhase::HeroBan),
        Some(shown(&ban)),
    );
    // No examples installed: the shape rule decides. Each frame passes it
    // (the icons are under the title region), replaces the held one and
    // confirms it, so the held one becomes the frame owed a write.
    let mut handed: Vec<Arc<image::DynamicImage>> = Vec::new();
    for (i, icons) in (1..=4).enumerate() {
        let ms = 3_000 + 1_500 * i as u64;
        let rgb = ban_results(icons);
        let want = dump.want(at(ms));
        assert!(want.ban_title && want.other_frame, "{icons} icons");
        let seen = classify(want, &detect::GamePhase::Unknown, &rgb, &dump.examples());
        assert_eq!(seen, Seen::BanResults, "{icons} icons");
        assert!(keeps_frame(want, seen));
        let frame = shown(&rgb);
        handed.push(Arc::clone(&frame));
        dump.observe(at(ms), seen, Some(frame));
        // Only the latest results frame and the one before it are held;
        // the earlier ones are ours alone again.
        let held: Vec<usize> = handed
            .iter()
            .enumerate()
            .filter(|(_, f)| Arc::strong_count(f) > 1)
            .map(|(k, _)| k)
            .collect();
        let want_held: Vec<usize> = (i.saturating_sub(1)..=i).collect();
        assert_eq!(held, want_held, "{icons} icons");
    }
    // The select's first frame shows its title, then its read.
    let select = select_with_title();
    let want = dump.want(at(9_000));
    let examples = dump.examples();
    let seen = classify_with(
        want,
        &detect::GamePhase::Unknown,
        &select,
        shows_title,
        |f| examples.matches(f),
    );
    assert_eq!(seen, Seen::SelectTitle);
    dump.observe(at(9_000), seen, None);
    dump.observe(
        at(10_500),
        Seen::Phase(PregamePhase::HeroSelect),
        Some(shown(&select)),
    );
    assert!(dump.finish(Duration::from_secs(30)).await);

    let root = pregame_dir(dir.path());
    let games: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(games.len(), 1);
    let saved = image::open(games[0].join(BAN_RESULTS_NAME))
        .unwrap()
        .to_rgb8();
    assert!(saved == ban_results(4), "the four-icon frame is saved");
}

/// The hero select with its title: the roster, and a mark where the
/// words-v1 read would find `SELECT A PREFERRED HERO` (the read itself
/// needs the mask, which tests do not load).
fn select_with_title() -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(1280, 720, image::Rgb([30, 34, 44]));
    fill(&mut img, (0.41, 0.165, 0.565, 0.2), TEXT);
    img
}

/// Stands in for the words-v1 read of the select title.
fn shows_title(rgb: &image::RgbImage) -> bool {
    let (w, h) = rgb.dimensions();
    rgb.get_pixel(w / 2, h * 18 / 100).0 == TEXT
}

/// How the select comes after the ban results.
enum SelectComes {
    /// A frame showing its title, then its phase read.
    Title,
    /// Its phase read straight away, with no title frame first.
    PhaseRead,
    /// A plain roster frame (no title read), then its phase read.
    Roster,
}

/// A ban, the results revealed over two polls (two icons, then four), then
/// the select as `comes`. Returns the saved ban results and whether the
/// select's first frame was saved.
async fn results_then_select(comes: SelectComes) -> (Option<image::RgbImage>, bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let ban = image::RgbImage::from_pixel(64, 36, image::Rgb([40, 20, 20]));
    dump.observe(at(0), Seen::Phase(PregamePhase::HeroBan), Some(shown(&ban)));
    dump.observe(
        at(1_500),
        Seen::Phase(PregamePhase::HeroBan),
        Some(shown(&ban)),
    );
    let examples = dump.examples();
    let tick = |dump: &mut PregameDump, ms: u64, phase: detect::GamePhase, rgb: image::RgbImage| {
        let want = dump.want(at(ms));
        let seen = classify_with(want, &phase, &rgb, shows_title, |f| examples.matches(f));
        let frame = keeps_frame(want, seen).then(|| shown(&rgb));
        dump.observe(at(ms), seen, frame);
        seen
    };
    let unknown = || detect::GamePhase::Unknown;
    for (ms, icons) in [(3_000, 2), (4_500, 4)] {
        let seen = tick(&mut dump, ms, unknown(), ban_results(icons));
        assert_eq!(seen, Seen::BanResults, "{icons} icons");
    }
    let mut ms = 6_000;
    match comes {
        SelectComes::Title => {
            let seen = tick(&mut dump, ms, unknown(), select_with_title());
            assert_eq!(seen, Seen::SelectTitle);
            ms += 1_500;
        }
        SelectComes::Roster => {
            let roster = image::RgbImage::from_pixel(1280, 720, image::Rgb([30, 34, 44]));
            let seen = tick(&mut dump, ms, unknown(), roster);
            assert_eq!(seen, Seen::Other);
            ms += 1_500;
        }
        SelectComes::PhaseRead => {}
    }
    let seen = tick(
        &mut dump,
        ms,
        detect::GamePhase::HeroSelect,
        select_with_title(),
    );
    assert_eq!(seen, Seen::Phase(PregamePhase::HeroSelect));
    assert!(dump.finish(Duration::from_secs(30)).await);

    let root = pregame_dir(dir.path());
    let games: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(games.len(), 1);
    let saved = image::open(games[0].join(BAN_RESULTS_NAME))
        .ok()
        .map(|i| i.to_rgb8());
    (saved, games[0].join("heroselect_first.png").is_file())
}

#[tokio::test]
async fn results_straight_into_the_select_title_save_the_held_frame() {
    // No plain frame between: the title is read with no lag, so the held
    // replacement was still the results, all four bans shown.
    let (saved, select) = results_then_select(SelectComes::Title).await;
    assert!(
        saved == Some(ban_results(4)),
        "the four-icon frame is saved"
    );
    assert!(select);
}

#[tokio::test]
async fn results_straight_into_a_select_read_save_the_frame_before_the_held_one() {
    // The phase read lags a poll, so the held frame may be the select
    // sliding in: dropped, as before. The frame before it, which it
    // confirmed, is saved instead.
    let (saved, select) = results_then_select(SelectComes::PhaseRead).await;
    assert!(saved == Some(ban_results(2)), "the two-icon frame is saved");
    assert!(select);
}

#[tokio::test]
async fn results_then_a_plain_roster_frame_save_the_frame_before_the_last() {
    // The cost of not trusting a plain frame after a replacement: the
    // last results frame may be the select sliding in, so the roster frame
    // does not confirm it, and the frame before it is saved.
    let (saved, select) = results_then_select(SelectComes::Roster).await;
    assert!(saved == Some(ban_results(2)), "the two-icon frame is saved");
    assert!(select);
}

/// A 640x360 frame: black bars around the loading strip's vertical navy
/// gradient. The same builders as the tests in `detect/loading_screen.rs`.
fn navy_band() -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(640, 360, image::Rgb([0, 0, 0]));
    for y in 136..224 {
        let t = (y - 136) as u8;
        let navy = image::Rgb([20 + t / 4, 28 + t / 5, 42 + t / 3]);
        for x in 0..640 {
            img.put_pixel(x, y, navy);
        }
    }
    img
}

fn glyphs(img: &mut image::RgbImage, xs: std::ops::Range<u32>, ys: std::ops::Range<u32>) {
    for y in ys {
        for x in xs.clone().filter(|x| x % 7 < 3) {
            img.put_pixel(x, y, image::Rgb([240, 240, 240]));
        }
    }
}

/// The `ENTERING GAME` slab: one line of white glyphs centred across the
/// screen, 0.375 of its width, at half height.
fn entering_game_slab() -> image::RgbImage {
    let mut img = navy_band();
    glyphs(&mut img, 200..440, 168..192);
    img
}

/// An `APPLYING UPDATE` band: the same navy band, its text left of centre,
/// with a progress bar under it when `bar` is set, or centred above a bar
/// when `centred` is set.
fn applying_update_band(centred: bool, bar: bool) -> image::RgbImage {
    let mut img = navy_band();
    let (text, track) = if centred {
        (220..420, 120..520)
    } else {
        (48..248, 48..592)
    };
    glyphs(&mut img, text, 160..176);
    if bar {
        for y in 196..200 {
            for x in track.clone() {
                img.put_pixel(x, y, image::Rgb([230, 232, 236]));
            }
        }
    }
    img
}

const LOADING_WANTED: PregameWant = PregameWant {
    loading: true,
    other_frame: true,
    select_title: false,
    ban_title: false,
};

#[test]
fn an_applying_update_band_is_not_saved_as_loading() {
    let unknown = detect::GamePhase::Unknown;
    let no_title = |_: &image::RgbImage| false;
    for (centred, bar) in [(false, true), (false, false), (true, true)] {
        let band = applying_update_band(centred, bar);
        // The plain loading check, which nothing else uses, still passes
        // it; the saver's stricter check does not.
        assert!(detect::loading_screen::is_loading_screen(&band));
        assert_eq!(
            classify_with(LOADING_WANTED, &unknown, &band, no_title, no_title),
            Seen::Other,
            "centred {centred}, bar {bar}"
        );
    }
    assert_eq!(
        classify_with(
            LOADING_WANTED,
            &unknown,
            &entering_game_slab(),
            no_title,
            no_title
        ),
        Seen::Loading
    );
}
