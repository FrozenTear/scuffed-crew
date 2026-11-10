//! Tests for the cv-v1 bans and picks reader on synthetic frames.
//!
//! No game art and no real screenshot is used. Every hero icon here is a
//! procedural picture (coloured rectangles and ellipses seeded from the hero
//! key), laid into a flat 2560x1440 frame at the positions the reader looks
//! at. The icons carry no text and no name.

use std::sync::OnceLock;

use image::{Rgb, RgbImage, imageops};

use super::*;

// ---- synthetic art ----

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0 >> 8
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }
}

fn seed_of(key: &str) -> u32 {
    key.bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

/// A procedural icon: a flat colour with N random rectangles and ellipses,
/// lightly blurred. It has no text and no face.
fn icon(seed: u32) -> RgbImage {
    let mut r = Lcg(seed);
    // Shapes alternate dark and bright so every one has a strong edge.
    let col = |r: &mut Lcg, bright: bool| {
        let lo = if bright { 160 } else { 0 };
        Rgb([0, 0, 0].map(|_| (lo + r.below(96)) as u8))
    };
    let mut img = RgbImage::from_pixel(TEMPLATE_W, TEMPLATE_H, col(&mut r, false));
    for n in 0..SHAPES {
        let c = col(&mut r, n % 2 == 0);
        let (cx, cy) = (r.below(TEMPLATE_W) as i32, r.below(TEMPLATE_H) as i32);
        let (rw, rh) = (SMIN + r.below(SVAR) as i32, SMIN + r.below(SVAR) as i32);
        let ellipse = r.below(2) == 0;
        for y in (cy - rh).max(0)..(cy + rh).min(TEMPLATE_H as i32) {
            for x in (cx - rw).max(0)..(cx + rw).min(TEMPLATE_W as i32) {
                let inside = if ellipse {
                    let (dx, dy) = ((x - cx) as f32 / rw as f32, (y - cy) as f32 / rh as f32);
                    dx * dx + dy * dy <= 1.0
                } else {
                    true
                };
                if inside {
                    img.put_pixel(x as u32, y as u32, c);
                }
            }
        }
    }
    imageops::blur(&img, BLUR)
}
const SHAPES: u32 = 40;
const SMIN: i32 = 4;
const SVAR: u32 = 16;
const BLUR: f32 = 0.8;

/// The "no preferred hero" stand-in: a dark tile with three pale discs.
fn no_pref_icon() -> RgbImage {
    let mut img = RgbImage::from_pixel(TEMPLATE_W, TEMPLATE_H, Rgb([20, 50, 110]));
    for (cx, cy) in [(50i32, 30i32), (32, 62), (68, 62)] {
        for y in 0..TEMPLATE_H as i32 {
            for x in 0..TEMPLATE_W as i32 {
                if (x - cx).pow(2) + (y - cy).pow(2) <= 14 * 14 {
                    img.put_pixel(x as u32, y as u32, Rgb([190, 205, 235]));
                }
            }
        }
    }
    img
}

fn set() -> &'static TemplateSet {
    static SET: OnceLock<TemplateSet> = OnceLock::new();
    SET.get_or_init(|| {
        let heroes = hero_keys().into_iter().map(|k| {
            let i = icon(seed_of(&k));
            (k, i)
        });
        TemplateSet::from_images(heroes.collect(), Some(no_pref_icon()))
    })
}

fn reader() -> Reader {
    // Clone-free: a Reader owns its set, so rebuild from the cached images.
    let heroes = hero_keys().into_iter().map(|k| {
        let i = icon(seed_of(&k));
        (k, i)
    });
    Reader::new(TemplateSet::from_images(
        heroes.collect(),
        Some(no_pref_icon()),
    ))
}

// ---- scene building ----

const BG: Rgb<u8> = Rgb([28, 32, 44]);
const OWN: Rgb<u8> = Rgb([40, 160, 255]);
const ENEMY: Rgb<u8> = Rgb([235, 50, 60]);

fn blank() -> RgbImage {
    RgbImage::from_pixel(FRAME_W, FRAME_H, BG)
}

/// Lay `icon` at native scale `s`, centred on `(cx, cy)`.
fn put(img: &mut RgbImage, icon: &RgbImage, s: f32, cx: i32, cy: i32) {
    let (w, h) = (
        (icon.width() as f32 * s) as u32,
        (icon.height() as f32 * s) as u32,
    );
    let scaled = imageops::resize(icon, w, h, imageops::FilterType::Lanczos3);
    imageops::replace(
        img,
        &scaled,
        (cx - w as i32 / 2) as i64,
        (cy - h as i32 / 2) as i64,
    );
}

fn fill(img: &mut RgbImage, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb<u8>) {
    for y in y0..y1 {
        for x in x0..x1 {
            img.put_pixel(x as u32, y as u32, c);
        }
    }
}

fn hero_icon(key: &str) -> RgbImage {
    icon(seed_of(key))
}

/// A ban results frame: four heroes, bars coloured per side, optional lobby ban.
fn ban_frame(heroes: [&str; 4], sides: [Rgb<u8>; 4], lobby: Option<&str>) -> RgbImage {
    let mut img = blank();
    let cy = if lobby.is_some() { 850 } else { 645 };
    for (i, &x) in SLOT_X.iter().enumerate() {
        put(&mut img, &hero_icon(heroes[i]), 1.87, x, cy);
        fill(&mut img, x - 60, cy + 254, x - 20, cy + 261, sides[i]);
    }
    if let Some(l) = lobby {
        put(&mut img, &hero_icon(l), 1.6, LOBBY_XY.0, LOBBY_XY.1);
    }
    img
}

/// What a row shows.
#[derive(Clone, Copy)]
enum Row<'a> {
    Hero(&'a str),
    NoPref,
    Empty,
}

fn row_frame(ys: &[i32; 6], rows: [Row; 6]) -> RgbImage {
    let mut img = blank();
    fill(&mut img, 64, 430, 70, 475, OWN);
    for (y, row) in ys.iter().zip(rows) {
        match row {
            Row::Hero(k) => put(&mut img, &hero_icon(k), 0.8, ROW_X, *y),
            Row::NoPref => put(&mut img, &no_pref_icon(), 0.8, ROW_X, *y),
            Row::Empty => {}
        }
    }
    img
}

fn key(r: &Read) -> Option<&str> {
    r.value.as_ref().map(Choice::as_str)
}

const FIVE: [&str; 4] = ["ana", "reaper", "wrecking-ball", "soldier-76"];

// ---- tests ----

#[test]
fn switch_is_on_by_default_and_only_off_words_turn_it_off() {
    assert_eq!(BansPicksSetting::default(), BansPicksSetting::CvV1);
    assert!(BansPicksSetting::default().is_on());
    for on in ["cv-v1", " CV-V1 ", "", "on", "1", "true", "ocr-v1"] {
        assert_eq!(BansPicksSetting::parse(on), BansPicksSetting::CvV1, "{on}");
    }
    for off in ["off", " OFF ", "0", "false", "No"] {
        assert_eq!(BansPicksSetting::parse(off), BansPicksSetting::Off, "{off}");
    }
    assert_eq!(
        serde_json::to_string(&BansPicksSetting::CvV1).unwrap(),
        "\"cv-v1\""
    );
    assert_eq!(
        serde_json::to_string(&BansPicksSetting::Off).unwrap(),
        "\"off\""
    );
}

#[test]
fn hero_keys_are_the_54_pack_keys() {
    let keys = hero_keys();
    assert_eq!(keys.len(), 54);
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 54);
    for k in ["wrecking-ball", "soldier-76", "dva", "dmon", "lucio"] {
        assert!(keys.iter().any(|x| x == k), "{k}");
    }
    assert!(!keys.iter().any(|k| k == NO_PREFERENCE_KEY));
}

#[test]
fn plain_layout_reads_four_bans_with_teams_and_no_lobby_ban() {
    let r = reader();
    let img = ban_frame(FIVE, [OWN, OWN, ENEMY, ENEMY], None);
    let out = r
        .read_ban_results(
            &img,
            Some(self_ring_colour(&row_frame(&HS_Y, [Row::Empty; 6]))),
        )
        .unwrap();
    assert_eq!(out.layout, Layout::Plain);
    assert!(out.lobby_ban.is_none());
    let got: Vec<_> = out.bans.iter().map(|b| key(&b.read)).collect();
    assert_eq!(got, FIVE.map(Some));
    let teams: Vec<_> = out.bans.iter().map(|b| b.team).collect();
    assert_eq!(
        teams,
        vec![
            Some(Team::Own),
            Some(Team::Own),
            Some(Team::Enemy),
            Some(Team::Enemy)
        ]
    );
    for b in &out.bans {
        assert!(b.read.confidence > 0.5 && b.read.confidence <= 1.0, "{b:?}");
        assert!(b.read.score >= MIN_SCORE && b.read.margin >= MIN_MARGIN);
    }
}

#[test]
fn lobby_layout_reads_the_lowered_bans_and_the_lobby_ban() {
    let r = reader();
    let img = ban_frame(FIVE, [ENEMY, OWN, OWN, ENEMY], Some("mei"));
    let out = r.read_ban_results(&img, Some([40, 160, 255])).unwrap();
    assert_eq!(out.layout, Layout::Lobby);
    let got: Vec<_> = out.bans.iter().map(|b| key(&b.read)).collect();
    assert_eq!(got, FIVE.map(Some));
    assert_eq!(key(out.lobby_ban.as_ref().unwrap()), Some("mei"));
    let teams: Vec<_> = out.bans.iter().map(|b| b.team).collect();
    assert_eq!(
        teams,
        vec![
            Some(Team::Enemy),
            Some(Team::Own),
            Some(Team::Own),
            Some(Team::Enemy)
        ]
    );
}

#[test]
fn team_is_none_without_the_ring_colour_or_for_a_grey_bar() {
    let r = reader();
    let img = ban_frame(FIVE, [OWN, OWN, ENEMY, Rgb([120, 120, 120])], None);
    let out = r.read_ban_results(&img, None).unwrap();
    assert!(out.bans.iter().all(|b| b.team.is_none()));
    let out = r.read_ban_results(&img, Some([40, 160, 255])).unwrap();
    assert_eq!(out.bans[3].team, None);
    assert_eq!(out.bans[2].team, Some(Team::Enemy));
}

#[test]
fn an_empty_slot_and_an_unknown_icon_read_as_none_never_a_guess() {
    let r = reader();
    let mut img = ban_frame(FIVE, [OWN; 4], None);
    // slot 0 blank, slot 3 an icon that is not any hero in the set; slots 1
    // and 2 stay, so the layout probe still sees the plain layout
    fill(
        &mut img,
        SLOT_X[0] - 110,
        645 - 110,
        SLOT_X[0] + 110,
        645 + 110,
        BG,
    );
    fill(
        &mut img,
        SLOT_X[3] - 110,
        645 - 110,
        SLOT_X[3] + 110,
        645 + 110,
        BG,
    );
    put(&mut img, &icon(0xDEAD_BEEF), 1.87, SLOT_X[3], 645);
    let out = r.read_ban_results(&img, None).unwrap();
    assert_eq!(out.layout, Layout::Plain);
    for i in [0, 3] {
        assert_eq!(out.bans[i].read.value, None, "slot {i}");
        assert_eq!(out.bans[i].read.confidence, 0.0);
    }
    assert_eq!(key(&out.bans[1].read), Some("reaper"));
    assert_eq!(key(&out.bans[2].read), Some("wrecking-ball"));
}

#[test]
fn a_badge_in_the_icon_corner_does_not_change_the_read() {
    let r = reader();
    let mut img = ban_frame(FIVE, [OWN; 4], None);
    // slot 0 spans x 645 +- 93, y 645 +- 90; mark its top right corner
    fill(
        &mut img,
        645 + 40,
        645 - 90,
        645 + 93,
        645 - 90 + 50,
        Rgb([255, 255, 255]),
    );
    let out = r.read_ban_results(&img, None).unwrap();
    assert_eq!(key(&out.bans[0].read), Some("ana"));
}

#[test]
fn hero_select_reads_six_rows_with_no_preference_and_unreadable_rows() {
    let r = reader();
    let rows = [
        Row::Hero("kiriko"),
        Row::Hero("genji"),
        Row::NoPref,
        Row::Empty,
        Row::Hero("zarya"),
        Row::NoPref,
    ];
    let hs = r.read_hero_select(&row_frame(&HS_Y, rows)).unwrap();
    let got: Vec<_> = hs.picks.iter().map(|p| key(p)).collect();
    assert_eq!(
        got,
        vec![
            Some("kiriko"),
            Some("genji"),
            Some(NO_PREFERENCE),
            None,
            Some("zarya"),
            Some(NO_PREFERENCE)
        ]
    );
    assert_eq!(hs.picks[2].value, Some(Choice::NoPreference));
    assert_eq!(hs.ring, [40, 160, 255]);
}

#[test]
fn without_a_no_preference_template_that_icon_is_none() {
    let heroes = hero_keys().into_iter().map(|k| {
        let i = icon(seed_of(&k));
        (k, i)
    });
    let r = Reader::new(TemplateSet::from_images(heroes.collect(), None));
    let rows = [
        Row::NoPref,
        Row::Hero("mei"),
        Row::Empty,
        Row::Empty,
        Row::Empty,
        Row::Empty,
    ];
    let hs = r.read_hero_select(&row_frame(&HS_Y, rows)).unwrap();
    assert_eq!(hs.picks[0].value, None);
    assert_eq!(key(&hs.picks[1]), Some("mei"));
}

#[test]
fn a_frame_that_is_not_2560x1440_is_refused() {
    let r = reader();
    let small = RgbImage::from_pixel(1920, 1080, BG);
    assert!(r.read_ban_results(&small, None).is_none());
    assert!(r.read_hero_select(&small).is_none());
    assert!(r.read_ban_vote(&small).is_none());
    assert!(
        r.read_ban_vote(&RgbImage::from_pixel(2560, 1080, BG))
            .is_none()
    );
    assert!(r.read(&small, &small).is_none());
}

#[test]
fn the_combined_read_has_the_documented_json_shape() {
    let r = reader();
    let br = ban_frame(FIVE, [OWN, OWN, ENEMY, ENEMY], Some("mei"));
    let rows = [
        Row::Hero("kiriko"),
        Row::NoPref,
        Row::Empty,
        Row::Hero("zarya"),
        Row::Hero("ana"),
        Row::Hero("juno"),
    ];
    let out = r.read(&br, &row_frame(&HS_Y, rows)).unwrap();
    assert_eq!(out.bans_picks_recognizer, "cv-v1");
    let v = serde_json::to_value(&out).unwrap();
    let obj = v.as_object().unwrap();
    let mut names: Vec<_> = obj.keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(
        names,
        ["bans", "bans_picks_recognizer", "lobby_ban", "picks"]
    );
    assert_eq!(v["bans"][0]["value"], "ana");
    assert_eq!(v["bans"][0]["team"], "own");
    assert_eq!(v["bans"][2]["team"], "enemy");
    assert_eq!(v["lobby_ban"]["value"], "mei");
    assert_eq!(v["picks"][1]["value"], "no_preference");
    assert!(v["picks"][2]["value"].is_null());
    assert_eq!(v["picks"][2]["confidence"], 0.0);
    assert!(v["picks"][0]["confidence"].as_f64().unwrap() > 0.5);
    let back: BansPicks = serde_json::from_value(v).unwrap();
    assert_eq!(back, out);
}

#[test]
fn the_coarse_search_agrees_with_the_exhaustive_search() {
    let fast = reader();
    let exact = reader().exact();
    let img = ban_frame(FIVE, [OWN, OWN, ENEMY, ENEMY], Some("mei"));
    let a = fast.read_ban_results(&img, None).unwrap();
    let b = exact.read_ban_results(&img, None).unwrap();
    for (x, y) in a.bans.iter().zip(&b.bans) {
        assert_eq!(x.read.value, y.read.value);
        assert!((x.read.score - y.read.score).abs() < 0.01);
    }
    assert_eq!(a.lobby_ban.unwrap().value, b.lobby_ban.unwrap().value);
}

// ---- transitions: fade and wipe ----

/// Blend a frame toward black (`1.0` keeps it, `0.0` is black).
fn fade(img: &RgbImage, keep: f32) -> RgbImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for c in &mut p.0 {
            *c = (*c as f32 * keep) as u8;
        }
    }
    out
}

/// Paint the cyan wipe band over a row's icon area.
fn wipe(img: &mut RgbImage, y: i32) {
    fill(img, 77, y - 48, 173, y + 48, Rgb([30, 230, 235]));
}

#[test]
fn a_fade_never_produces_a_wrong_hero_and_black_reads_as_none() {
    let r = reader();
    let rows = [
        Row::Hero("kiriko"),
        Row::Hero("genji"),
        Row::Hero("zarya"),
        Row::NoPref,
        Row::Hero("ana"),
        Row::Hero("juno"),
    ];
    let full = row_frame(&HS_Y, rows);
    let want = [
        Some("kiriko"),
        Some("genji"),
        Some("zarya"),
        Some(NO_PREFERENCE),
        Some("ana"),
        Some("juno"),
    ];
    let hs = r.read_hero_select(&full).unwrap();
    assert_eq!(hs.picks.iter().map(|p| key(p)).collect::<Vec<_>>(), want);
    let mut last_conf_sum = f32::MAX;
    for keep in [1.0f32, 0.8, 0.6, 0.4, 0.25, 0.12, 0.06, 0.03, 0.0] {
        let hs = r.read_hero_select(&fade(&full, keep)).unwrap();
        for (i, p) in hs.picks.iter().enumerate() {
            assert!(
                p.value.is_none() || key(p) == want[i],
                "fade {keep}, row {i}: {p:?}"
            );
        }
        let conf: f32 = hs.picks.iter().map(|p| p.confidence).sum();
        // fading never raises how many values are read
        assert!(conf <= last_conf_sum + 6.0 * 0.15, "fade {keep}");
        last_conf_sum = conf;
        if keep == 0.0 {
            assert!(hs.picks.iter().all(|p| p.value.is_none()));
        }
    }
}

#[test]
fn a_ban_results_fade_never_produces_a_wrong_hero() {
    let r = reader();
    let full = ban_frame(FIVE, [OWN, OWN, ENEMY, ENEMY], Some("mei"));
    for keep in [1.0f32, 0.5, 0.2, 0.08, 0.03, 0.0] {
        let out = r
            .read_ban_results(&fade(&full, keep), Some([40, 160, 255]))
            .unwrap();
        for (i, b) in out.bans.iter().enumerate() {
            assert!(
                b.read.value.is_none() || key(&b.read) == Some(FIVE[i]),
                "fade {keep} slot {i}"
            );
        }
        let l = out.lobby_ban;
        assert!(
            l.is_none()
                || l.as_ref().unwrap().value.is_none()
                || key(l.as_ref().unwrap()) == Some("mei")
        );
        if keep == 0.0 {
            assert!(out.bans.iter().all(|b| b.read.value.is_none()));
        }
    }
}

#[test]
fn a_row_under_the_wipe_band_is_none_and_its_neighbours_still_read() {
    let r = reader();
    let rows = [
        Row::Hero("kiriko"),
        Row::Hero("genji"),
        Row::Hero("zarya"),
        Row::Hero("ana"),
        Row::Hero("juno"),
        Row::Hero("mei"),
    ];
    let mut img = row_frame(&HS_Y, rows);
    assert!(band_share(&img, HS_Y[2]) < BAND_MAX);
    wipe(&mut img, HS_Y[2]);
    assert!(band_share(&img, HS_Y[2]) > BAND_MAX);
    let hs = r.read_hero_select(&img).unwrap();
    assert_eq!(hs.picks[2].value, None);
    assert_eq!(hs.picks[2].confidence, 0.0);
    for i in [0, 1, 3, 4, 5] {
        assert!(hs.picks[i].value.is_some(), "row {i}");
    }
}

#[test]
fn a_half_covered_row_is_none_or_right_never_wrong() {
    let r = reader();
    let rows = [Row::Hero("kiriko"); 6];
    let full = row_frame(&HS_Y, rows);
    // a band sweeping down the row: covers 0..96 px of its height
    for covered in (0..=96).step_by(8) {
        let mut img = full.clone();
        fill(
            &mut img,
            77,
            HS_Y[2] - 48,
            173,
            HS_Y[2] - 48 + covered,
            Rgb([30, 230, 235]),
        );
        let hs = r.read_hero_select(&img).unwrap();
        let p = &hs.picks[2];
        assert!(
            p.value.is_none() || key(p) == Some("kiriko"),
            "covered {covered}: {p:?}"
        );
    }
}

#[test]
fn band_share_counts_only_wipe_cyan() {
    let mut img = blank();
    assert_eq!(band_share(&img, 452), 0.0);
    fill(
        &mut img,
        77,
        452 - 48,
        77 + 48,
        452 + 48,
        Rgb([30, 230, 235]),
    );
    let s = band_share(&img, 452);
    assert!((s - 0.5).abs() < 0.02, "{s}");
    let mut img = blank();
    fill(&mut img, 77, 404, 173, 500, Rgb([235, 50, 60]));
    assert_eq!(band_share(&img, 452), 0.0);
}

// ---- template folder ----

fn write_set(dir: &std::path::Path, skip: &[&str], no_pref: bool, size: (u32, u32)) {
    for k in hero_keys() {
        if skip.contains(&k.as_str()) {
            continue;
        }
        let mut i = hero_icon(&k);
        if size != (TEMPLATE_W, TEMPLATE_H) {
            i = imageops::resize(&i, size.0, size.1, imageops::FilterType::Lanczos3);
        }
        i.save(dir.join(format!("{k}.png"))).unwrap();
    }
    if no_pref {
        no_pref_icon().save(dir.join("no-preference.png")).unwrap();
    }
}

#[test]
fn templates_load_from_the_data_dir_folder_and_read() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join(TEMPLATE_DIR);
    std::fs::create_dir_all(&folder).unwrap();
    write_set(&folder, &[], true, (TEMPLATE_W, TEMPLATE_H));
    let r = Reader::load(dir.path()).unwrap();
    let img = ban_frame(FIVE, [OWN; 4], None);
    let out = r.read_ban_results(&img, None).unwrap();
    assert_eq!(key(&out.bans[1].read), Some("reaper"));
}

#[test]
fn missing_templates_name_the_missing_heroes_and_keep_the_reader_off() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        Reader::load(dir.path()),
        Err(TemplateError::Missing(k)) if k.len() == 54
    ));
    write_set(dir.path(), &["ana", "mei"], false, (TEMPLATE_W, TEMPLATE_H));
    match TemplateSet::load_dir(dir.path()) {
        Err(TemplateError::Missing(k)) => assert_eq!(k, vec!["ana", "mei"]),
        other => panic!("{:?}", other.err()),
    }
}

#[test]
fn a_template_of_the_wrong_size_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_set(dir.path(), &[], false, (64, 64));
    assert!(matches!(
        TemplateSet::load_dir(dir.path()),
        Err(TemplateError::BadSize(_, 64, 64))
    ));
}

#[test]
fn set_reports_its_classes() {
    let s = set();
    assert_eq!(s.hero_count(), 54);
    assert!(s.has_no_preference());
    assert_eq!(s.key(54), NO_PREFERENCE_KEY);
}

#[test]
fn every_hero_reads_itself_in_a_row_with_the_fast_search() {
    let r = reader();
    let keys = hero_keys();
    for chunk in keys.chunks(6) {
        let mut rows: Vec<Row> = chunk.iter().map(|k| Row::Hero(k.as_str())).collect();
        while rows.len() < 6 {
            rows.push(Row::Empty);
        }
        let rows: [Row; 6] = rows.try_into().ok().unwrap();
        let hs = r.read_hero_select(&row_frame(&HS_Y, rows)).unwrap();
        for (k, p) in chunk.iter().zip(&hs.picks) {
            assert_eq!(key(p), Some(k.as_str()), "{k}: {p:?}");
        }
    }
}

// ---- team side from the relative colours of the ban bars ----

const YELLOW: Rgb<u8> = Rgb([255, 200, 0]);
const PURPLE: Rgb<u8> = Rgb([128, 0, 128]);

fn teams_of(out: &BanResults) -> Vec<Option<Team>> {
    out.bans.iter().map(|b| b.team).collect()
}

/// Outer slots one team, middle slots the other: the shape of a real game.
const OUTER_MIDDLE: [Rgb<u8>; 4] = [YELLOW, PURPLE, PURPLE, YELLOW];

#[test]
fn yellow_and_purple_bars_split_by_the_ring_with_either_side_own() {
    let r = reader();
    let img = ban_frame(FIVE, OUTER_MIDDLE, None);
    let own_purple = r.read_ban_results_rings(&img, &[[128, 0, 128]]).unwrap();
    assert_eq!(
        teams_of(&own_purple),
        vec![
            Some(Team::Enemy),
            Some(Team::Own),
            Some(Team::Own),
            Some(Team::Enemy)
        ]
    );
    let own_yellow = r.read_ban_results_rings(&img, &[[255, 200, 0]]).unwrap();
    assert_eq!(
        teams_of(&own_yellow),
        vec![
            Some(Team::Own),
            Some(Team::Enemy),
            Some(Team::Enemy),
            Some(Team::Own)
        ]
    );
    // the single-ring entry point agrees
    let one = r.read_ban_results(&img, Some([128, 0, 128])).unwrap();
    assert_eq!(teams_of(&one), teams_of(&own_purple));
}

#[test]
fn a_ring_off_the_bar_hue_still_picks_the_nearer_team() {
    let r = reader();
    let img = ban_frame(FIVE, OUTER_MIDDLE, None);
    // violet (hue 138) is 12 from the purple bars and 114 from the yellow
    let violet = r.read_ban_results_rings(&img, &[[150, 0, 255]]).unwrap();
    assert_eq!(violet.bans[1].team, Some(Team::Own));
    assert_eq!(violet.bans[0].team, Some(Team::Enemy));
    // orange (hue 12) is 12 from the yellow bars
    let orange = r.read_ban_results_rings(&img, &[[255, 100, 0]]).unwrap();
    assert_eq!(orange.bans[0].team, Some(Team::Own));
    assert_eq!(orange.bans[2].team, Some(Team::Enemy));
}

#[test]
fn the_default_blue_and_red_still_read_and_the_other_way_round() {
    let r = reader();
    let img = ban_frame(FIVE, [ENEMY, OWN, OWN, ENEMY], None);
    let blue = r.read_ban_results_rings(&img, &[[40, 160, 255]]).unwrap();
    assert_eq!(blue.bans[1].team, Some(Team::Own));
    assert_eq!(blue.bans[0].team, Some(Team::Enemy));
    let red = r.read_ban_results_rings(&img, &[[235, 50, 60]]).unwrap();
    assert_eq!(red.bans[1].team, Some(Team::Enemy));
    assert_eq!(red.bans[0].team, Some(Team::Own));
}

#[test]
fn an_ambiguous_or_far_ring_gives_no_team_never_a_guess() {
    let r = reader();
    let img = ban_frame(FIVE, OUTER_MIDDLE, None);
    // hue 0: 24 from yellow (23.5 before rounding, 24 as read) and 30 from
    // purple, too close to call
    for ring in [[255, 0, 0], [120, 120, 120], [40, 0, 40], [0, 200, 255]] {
        let out = r.read_ban_results_rings(&img, &[ring]).unwrap();
        assert!(out.bans.iter().all(|b| b.team.is_none()), "{ring:?}");
    }
    let none = r.read_ban_results_rings(&img, &[]).unwrap();
    assert!(none.bans.iter().all(|b| b.team.is_none()));
}

#[test]
fn an_unusable_ring_is_skipped_for_the_next_one_and_disagreeing_rings_give_none() {
    let r = reader();
    let img = ban_frame(FIVE, OUTER_MIDDLE, None);
    // a faded select ring, then the ban vote ring
    let out = r
        .read_ban_results_rings(&img, &[[40, 0, 40], [128, 0, 128]])
        .unwrap();
    assert_eq!(out.bans[1].team, Some(Team::Own));
    // a grey one, then the ambiguous red, then a good one
    let out = r
        .read_ban_results_rings(&img, &[[120, 120, 120], [255, 0, 0], [255, 200, 0]])
        .unwrap();
    assert_eq!(out.bans[0].team, Some(Team::Own));
    // two usable rings that name different teams
    let out = r
        .read_ban_results_rings(&img, &[[128, 0, 128], [255, 200, 0]])
        .unwrap();
    assert!(out.bans.iter().all(|b| b.team.is_none()));
    // two usable rings that agree
    let out = r
        .read_ban_results_rings(&img, &[[128, 0, 128], [150, 0, 255]])
        .unwrap();
    assert_eq!(out.bans[1].team, Some(Team::Own));
}

#[test]
fn bars_that_do_not_form_two_groups_give_no_team() {
    let r = reader();
    let ring = [[128, 0, 128]];
    // one colour on all four bars
    let img = ban_frame(FIVE, [PURPLE; 4], None);
    let out = r.read_ban_results_rings(&img, &ring).unwrap();
    assert!(out.bans.iter().all(|b| b.team.is_none()));
    // two colours too near to be two teams (hue 150 and 159)
    let img = ban_frame(
        FIVE,
        [PURPLE, PURPLE, Rgb([128, 0, 90]), Rgb([128, 0, 90])],
        None,
    );
    let out = r.read_ban_results_rings(&img, &ring).unwrap();
    assert!(out.bans.iter().all(|b| b.team.is_none()));
    // three colours (hue 24, 150 and 72)
    let img = ban_frame(FIVE, [YELLOW, PURPLE, PURPLE, Rgb([0, 200, 80])], None);
    let out = r.read_ban_results_rings(&img, &ring).unwrap();
    assert!(out.bans.iter().all(|b| b.team.is_none()));
}

#[test]
fn a_grey_bar_loses_only_its_own_team() {
    let r = reader();
    let grey = Rgb([120, 120, 120]);
    let img = ban_frame(FIVE, [YELLOW, PURPLE, PURPLE, grey], None);
    let out = r.read_ban_results_rings(&img, &[[128, 0, 128]]).unwrap();
    assert_eq!(
        teams_of(&out),
        vec![Some(Team::Enemy), Some(Team::Own), Some(Team::Own), None]
    );
    // the grey bar may even be the first one
    let img = ban_frame(FIVE, [grey, YELLOW, PURPLE, PURPLE], None);
    let out = r.read_ban_results_rings(&img, &[[128, 0, 128]]).unwrap();
    assert_eq!(
        teams_of(&out),
        vec![None, Some(Team::Enemy), Some(Team::Own), Some(Team::Own)]
    );
}

#[test]
fn the_lobby_layout_reads_yellow_and_purple_too() {
    let r = reader();
    let img = ban_frame(FIVE, OUTER_MIDDLE, Some("mei"));
    let out = r.read_ban_results_rings(&img, &[[255, 200, 0]]).unwrap();
    assert_eq!(out.layout, Layout::Lobby);
    assert_eq!(out.bans[0].team, Some(Team::Own));
    assert_eq!(out.bans[1].team, Some(Team::Enemy));
}

#[test]
fn a_three_against_one_split_gives_no_team() {
    let r = reader();
    let ring = [[128, 0, 128]];
    for bars in [
        [YELLOW, PURPLE, PURPLE, PURPLE],
        [PURPLE, YELLOW, YELLOW, YELLOW],
    ] {
        let img = ban_frame(FIVE, bars, None);
        let out = r.read_ban_results_rings(&img, &ring).unwrap();
        assert!(out.bans.iter().all(|b| b.team.is_none()), "{bars:?}");
    }
    // a team has two bars: with one grey the coloured ones still read
    let img = ban_frame(FIVE, [YELLOW, PURPLE, PURPLE, Rgb([120, 120, 120])], None);
    assert!(
        r.read_ban_results_rings(&img, &ring)
            .unwrap()
            .bans
            .iter()
            .any(|b| b.team.is_some())
    );
}

#[test]
fn a_bar_near_both_groups_goes_to_the_nearer_one() {
    // groups at hue 20 and 55 (35 apart); a bar at 40 is within 20 of both
    // (20 and 15) and belongs with 55, not with the bar checked first
    let ring_at_20 = [[255, 170, 0]];
    assert_eq!(hsv(ring_at_20[0])[0], 20);
    let teams = teams_from_hues([Some(20), Some(55), Some(40), Some(20)], &ring_at_20);
    assert_eq!(
        teams,
        [
            Some(Team::Own),
            Some(Team::Enemy),
            Some(Team::Enemy),
            Some(Team::Own)
        ]
    );
    // the bar the other way round: at 35 it is 15 from 20 and 20 from 55
    let teams = teams_from_hues([Some(20), Some(55), Some(35), Some(55)], &ring_at_20);
    assert_eq!(
        teams,
        [
            Some(Team::Own),
            Some(Team::Enemy),
            Some(Team::Own),
            Some(Team::Enemy)
        ]
    );
    // equally near both (18 and 18, groups 36 apart): that slot has no team
    let teams = teams_from_hues([Some(20), Some(56), Some(38), Some(20)], &ring_at_20);
    assert_eq!(
        teams,
        [Some(Team::Own), Some(Team::Enemy), None, Some(Team::Own)]
    );
}

// ---- the ban vote screen ----

/// Where the ban vote screen's roster column shows each player's preferred
/// hero (the rows the old reader wrongly took for votes).
const HB_Y: [i32; 6] = [452, 624, 754, 882, 1012, 1142];

const SLOT_GREY: Rgb<u8> = Rgb([157, 149, 149]);
const VOTE_RED: Rgb<u8> = Rgb([215, 16, 18]);

/// What one of the local player's vote slots shows.
#[derive(Clone, Copy)]
enum Slot<'a> {
    Hero(&'a str),
    NoPref,
    Skipped,
    Empty,
}

/// What one strip box shows: a hero with a count, or nothing yet.
#[derive(Clone, Copy)]
enum StripBox<'a> {
    Hero(&'a str, &'a str),
    Empty,
}

/// A border `BORDER` (3) pixels thick on the inclusive rectangle `r`.
fn frame_box(img: &mut RgbImage, r: (u32, u32, u32, u32), c: Rgb<u8>) {
    let (x0, y0, x1, y1) = (r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32);
    fill(img, x0, y0, x1 + 1, y0 + 3, c);
    fill(img, x0, y1 - 2, x1 + 1, y1 + 1, c);
    fill(img, x0, y0, x0 + 3, y1 + 1, c);
    fill(img, x1 - 2, y0, x1 + 1, y1 + 1, c);
}

fn centre(r: (u32, u32, u32, u32)) -> (i32, i32) {
    (((r.0 + r.2) / 2) as i32, ((r.1 + r.3) / 2) as i32)
}

/// The skip icon: a white ring of radius 15 and a slash from lower left to
/// upper right, centred in the slot.
fn skip_icon(img: &mut RgbImage, r: (u32, u32, u32, u32)) {
    let (cx, cy) = ((r.0 + r.2) as f32 / 2.0, (r.1 + r.3) as f32 / 2.0);
    for y in r.1 + 4..=r.3 - 4 {
        for x in r.0 + 4..=r.2 - 4 {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            let ring = (13.5..=16.5).contains(&d);
            let slash = d < 14.0 && (dx + dy).abs() <= 1.5;
            if ring || slash {
                img.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
    }
}

/// Draw `text` from the digit grids, one pixel per cell, centred on `cx`,
/// top row at `top` (the strip's counts sit at y 187..203).
fn draw_count(img: &mut RgbImage, text: &str, cx: u32, top: u32) {
    let step = digits::GLYPH_W as u32 + 2;
    let mut x = cx - text.len() as u32 * step / 2;
    for b in text.bytes() {
        let g = digits::glyph((b - b'0') as usize);
        for gy in 0..digits::GLYPH_H {
            for gx in 0..digits::GLYPH_W {
                if g[gy * digits::GLYPH_W + gx] > 0.5 {
                    img.put_pixel(x + gx as u32, top + gy as u32, Rgb([255, 255, 255]));
                }
            }
        }
        x += step;
    }
}

/// The ban vote screen: the local player's three slots, the strip with
/// its title, and (as in the game) a preferred-hero portrait in every
/// roster row, which must never be read as a vote.
fn vote_frame(slots: [Slot; 3], boxes: [StripBox; 3]) -> RgbImage {
    let portraits = [
        Row::Hero("ashe"),
        Row::Hero("ana"),
        Row::Hero("mei"),
        Row::NoPref,
        Row::Hero("orisa"),
        Row::Hero("juno"),
    ];
    let mut img = row_frame(&HB_Y, portraits);
    for (&r, slot) in vote::OWN_SLOTS.iter().zip(slots) {
        let (cx, cy) = centre(r);
        match slot {
            Slot::Hero(k) => {
                put(&mut img, &hero_icon(k), 0.465, cx, cy);
                frame_box(&mut img, r, VOTE_RED);
            }
            Slot::NoPref => {
                put(&mut img, &no_pref_icon(), 0.465, cx, cy);
                frame_box(&mut img, r, VOTE_RED);
            }
            Slot::Skipped => {
                skip_icon(&mut img, r);
                frame_box(&mut img, r, SLOT_GREY);
            }
            Slot::Empty => frame_box(&mut img, r, SLOT_GREY),
        }
    }
    strip(&mut img, boxes);
    img
}

/// The TEAM BAN SCORE strip: a red striped title and three boxes.
fn strip(img: &mut RgbImage, boxes: [StripBox; 3]) {
    let (tx0, ty0, tx1, ty1) = vote::STRIP_TITLE;
    for x in tx0..=tx1 {
        if (x / 3) % 2 == 0 {
            fill(
                img,
                x as i32,
                ty0 as i32,
                x as i32 + 1,
                ty1 as i32 + 1,
                VOTE_RED,
            );
        }
    }
    for (&r, b) in vote::STRIP_BOXES.iter().zip(boxes) {
        let (cx, cy) = centre(r);
        match b {
            StripBox::Hero(k, count) => {
                put(img, &hero_icon(k), 0.70, cx, cy);
                frame_box(img, r, VOTE_RED);
                draw_count(img, count, cx as u32, 188);
            }
            StripBox::Empty => {
                frame_box(img, r, SLOT_GREY);
                fill(img, cx - 3, 197, cx + 3, 200, Rgb([130, 130, 131]));
            }
        }
    }
}

fn slot_heroes(v: &BanVote) -> Vec<Option<&str>> {
    v.slots
        .as_ref()
        .expect("slots read")
        .iter()
        .map(|s| s.hero.as_deref())
        .collect()
}

fn slot_states(v: &BanVote) -> Vec<Option<SlotState>> {
    v.slots
        .as_ref()
        .expect("slots read")
        .iter()
        .map(|s| s.state)
        .collect()
}

const STRIP3: [StripBox<'static>; 3] = [
    StripBox::Hero("sombra", "15"),
    StripBox::Hero("roadhog", "12"),
    StripBox::Hero("junkrat", "7"),
];

#[test]
fn a_voted_hero_a_skip_and_an_empty_slot_read_as_such() {
    let r = reader();
    let img = vote_frame([Slot::Hero("kiriko"), Slot::Skipped, Slot::Empty], STRIP3);
    let v = r.read_ban_vote(&img).unwrap();
    assert_eq!(
        slot_states(&v),
        [
            Some(SlotState::Hero),
            Some(SlotState::Skipped),
            Some(SlotState::Empty)
        ]
    );
    assert_eq!(slot_heroes(&v), [Some("kiriko"), None, None]);
    let slots = v.slots.as_ref().unwrap();
    assert!(slots.iter().all(|s| s.voter == 0));
    assert_eq!(slots.iter().map(|s| s.slot).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(slots[0].confidence > 0.5 && slots[0].confidence <= 1.0);
    assert!(slots[0].score >= vote::VOTE_MIN_SCORE && slots[0].margin >= vote::VOTE_MIN_MARGIN);
    assert_eq!(slots[1].confidence, 0.0);
    // The flat list the game record keeps: the hero, then two nulls.
    let own = v.own_votes();
    assert_eq!(
        own.iter().map(key).collect::<Vec<_>>(),
        [Some("kiriko"), None, None]
    );
}

#[test]
fn three_voted_heroes_read_in_slot_order() {
    let r = reader();
    let img = vote_frame(
        [
            Slot::Hero("roadhog"),
            Slot::Hero("widowmaker"),
            Slot::Hero("mercy"),
        ],
        STRIP3,
    );
    let v = r.read_ban_vote(&img).unwrap();
    assert_eq!(
        slot_heroes(&v),
        [Some("roadhog"), Some("widowmaker"), Some("mercy")]
    );
}

#[test]
fn all_skipped_is_three_skips_and_no_vote_at_all() {
    // Session dfa4ad870eafb270 (Eichenwalde): every vote skipped, while the
    // roster column shows six preferred heroes. None of them is a vote.
    let r = reader();
    let img = vote_frame([Slot::Skipped; 3], STRIP3);
    let v = r.read_ban_vote(&img).unwrap();
    assert_eq!(slot_states(&v), [Some(SlotState::Skipped); 3]);
    assert!(v.own_votes().iter().all(|read| read.value.is_none()));
    assert_eq!(v.own_votes().len(), 3);
}

#[test]
fn the_no_preference_icon_in_a_slot_is_not_a_vote() {
    let r = reader();
    let img = vote_frame([Slot::NoPref, Slot::Empty, Slot::Empty], STRIP3);
    let v = r.read_ban_vote(&img).unwrap();
    let slot = &v.slots.as_ref().unwrap()[0];
    assert_eq!(slot.hero, None);
    assert_ne!(slot.state, Some(SlotState::Skipped));
    assert!(v.own_votes()[0].value.is_none());
}

#[test]
fn the_strip_reads_heroes_counts_and_ranks() {
    let r = reader();
    let img = vote_frame([Slot::Skipped; 3], STRIP3);
    let v = r.read_ban_vote(&img).unwrap();
    let score = v.score.expect("strip read");
    let got: Vec<_> = score
        .iter()
        .map(|e| (e.rank, e.hero.as_deref(), e.votes, e.empty))
        .collect();
    assert_eq!(
        got,
        [
            (1, Some("sombra"), Some(15), false),
            (2, Some("roadhog"), Some(12), false),
            (3, Some("junkrat"), Some(7), false),
        ]
    );
    for e in &score {
        assert!(e.confidence > 0.5 && e.votes_confidence > 0.5, "{e:?}");
        assert!(e.score >= vote::STRIP_MIN_SCORE && e.margin >= vote::STRIP_MIN_MARGIN);
    }
}

#[test]
fn an_empty_strip_box_is_empty_with_no_hero_and_no_count() {
    let r = reader();
    let img = vote_frame(
        [Slot::Empty; 3],
        [
            StripBox::Hero("mauga", "10"),
            StripBox::Empty,
            StripBox::Empty,
        ],
    );
    let v = r.read_ban_vote(&img).unwrap();
    let score = v.score.as_ref().unwrap();
    assert_eq!(score[0].hero.as_deref(), Some("mauga"));
    assert_eq!(score[0].votes, Some(10));
    for e in &score[1..] {
        assert!(e.empty, "{e:?}");
        assert_eq!((e.hero.as_deref(), e.votes), (None, None));
        assert_eq!((e.confidence, e.votes_confidence), (0.0, 0.0));
    }
    assert_eq!(slot_states(&v), [Some(SlotState::Empty); 3]);
}

#[test]
fn the_roster_portraits_are_never_votes() {
    // The old reader read these rows (x 125, the ban vote rows) as votes.
    let r = reader();
    let rows = [
        Row::Hero("ashe"),
        Row::Hero("ana"),
        Row::Hero("mei"),
        Row::NoPref,
        Row::Hero("orisa"),
        Row::Hero("juno"),
    ];
    let img = row_frame(&HB_Y, rows);
    let v = r.read_ban_vote(&img).unwrap();
    assert_eq!(v.slots, None);
    assert_eq!(v.score, None);
    assert!(v.own_votes().is_empty());
    // The same portraits beside real slots: only the slots are read.
    let img = vote_frame([Slot::Skipped, Slot::Empty, Slot::Skipped], STRIP3);
    let v = r.read_ban_vote(&img).unwrap();
    for name in ["ashe", "ana", "mei", "orisa", "juno"] {
        assert!(!slot_heroes(&v).contains(&Some(name)), "{name}");
    }
}

#[test]
fn a_hero_select_frame_has_no_slots_and_no_strip() {
    let r = reader();
    let rows = [
        Row::Hero("kiriko"),
        Row::Hero("genji"),
        Row::NoPref,
        Row::Empty,
        Row::Hero("zarya"),
        Row::NoPref,
    ];
    let v = r.read_ban_vote(&row_frame(&HS_Y, rows)).unwrap();
    assert_eq!((v.slots, v.score), (None, None));
    // nor the ban results screen
    let v = r
        .read_ban_vote(&ban_frame(FIVE, [OWN, OWN, ENEMY, ENEMY], Some("mei")))
        .unwrap();
    assert_eq!((v.slots, v.score), (None, None));
}

#[test]
fn noise_and_flat_frames_have_no_slots_and_no_strip() {
    let r = reader();
    let mut g = Lcg(7);
    let noise = RgbImage::from_fn(FRAME_W, FRAME_H, |_, _| {
        Rgb([g.below(256) as u8, g.below(256) as u8, g.below(256) as u8])
    });
    for img in [
        noise,
        blank(),
        RgbImage::from_pixel(FRAME_W, FRAME_H, SLOT_GREY),
        RgbImage::from_pixel(FRAME_W, FRAME_H, VOTE_RED),
    ] {
        let v = r.read_ban_vote(&img).unwrap();
        assert_eq!(v.slots, None);
        assert_eq!(v.score, None);
    }
}

#[test]
fn a_moved_layout_reads_as_none_not_as_votes() {
    let r = reader();
    let img = vote_frame(
        [Slot::Hero("ana"), Slot::Hero("mei"), Slot::Skipped],
        STRIP3,
    );
    // the whole screen 12 pixels to the right and down
    let mut moved = blank();
    imageops::replace(&mut moved, &img, 12, 12);
    let v = r.read_ban_vote(&moved).unwrap();
    assert_eq!(v.slots, None);
    assert_eq!(v.score, None);
}

#[test]
fn the_ban_vote_json_has_the_documented_shape() {
    let r = reader();
    let img = vote_frame([Slot::Hero("ana"), Slot::Skipped, Slot::Empty], STRIP3);
    let v = r.read_ban_vote(&img).unwrap();
    let json = serde_json::to_value(&v).unwrap();
    assert_eq!(json["slots"][0]["voter"], 0);
    assert_eq!(json["slots"][0]["slot"], 1);
    assert_eq!(json["slots"][0]["state"], "hero");
    assert_eq!(json["slots"][0]["hero"], "ana");
    assert_eq!(json["slots"][1]["state"], "skipped");
    assert!(json["slots"][1]["hero"].is_null());
    assert_eq!(json["slots"][2]["state"], "empty");
    assert_eq!(json["score"][0]["rank"], 1);
    assert_eq!(json["score"][0]["hero"], "sombra");
    assert_eq!(json["score"][0]["votes"], 15);
    assert_eq!(json["score"][0]["empty"], false);
    let back: BanVote = serde_json::from_value(json).unwrap();
    assert_eq!(back, v);
}
