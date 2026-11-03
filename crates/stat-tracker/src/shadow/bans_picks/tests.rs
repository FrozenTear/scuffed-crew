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
fn ban_vote_rows_use_their_own_row_positions() {
    let r = reader();
    let rows = [
        Row::Hero("ashe"),
        Row::Hero("ana"),
        Row::Hero("mei"),
        Row::NoPref,
        Row::Hero("orisa"),
        Row::Hero("juno"),
    ];
    let out = r.read_ban_vote(&row_frame(&HB_Y, rows)).unwrap();
    let got: Vec<_> = out.iter().map(|p| key(p)).collect();
    assert_eq!(
        got,
        [
            Some("ashe"),
            Some("ana"),
            Some("mei"),
            Some(NO_PREFERENCE),
            Some("orisa"),
            Some("juno")
        ]
    );
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
