//! The game record wiring ([`crate::shadow::rank::game`]) and the reader
//! fixes that came with it, on the synthetic frames of `tests.rs`.

use super::*;
use crate::shadow::rank::game::{rank_role, read_for_game, roles_complete, to_rank_update};

fn text(t: &Option<crate::model::RankText>) -> Option<&str> {
    t.as_ref().map(|t| t.value.as_str())
}

fn num(n: &Option<crate::model::RankNumber>) -> Option<i64> {
    n.as_ref().map(|n| n.value)
}

#[test]
fn a_full_read_goes_on_the_game_record_field_by_field() {
    let img = paint(&Scene {
        role: Some(Role::Support),
        ..Scene::default()
    });
    assert!(roles_complete(reader()));
    let u = read_for_game(reader(), &img).expect("a rank update");
    assert_eq!(text(&u.role), Some("Support"));
    assert_eq!(text(&u.role_rank), Some("HELLO"));
    assert_eq!(num(&u.division), Some(5));
    assert_eq!(num(&u.progress_pct), Some(31));
    assert_eq!(num(&u.progress_change), Some(27));
    let tags = u.change_tags.as_ref().map(|t| t.value.clone());
    assert_eq!(tags, Some(vec!["EH".to_string(), "TOT".to_string()]));
    assert_eq!(num(&u.higher_than_pct), Some(96));
    assert_eq!(num(&u.hero_sr), Some(3597));
    assert_eq!(num(&u.hero_sr_change), Some(12));
    assert_eq!(text(&u.next_mode_hint), Some("HELLO 6V6"));
    // each value keeps the reader's own confidence
    let read = reader().read(&img).expect("a rank screen");
    assert_eq!(
        u.hero_sr.as_ref().map(|n| n.confidence),
        Some(read.hero_sr.confidence)
    );
    assert_eq!(
        u.higher_than_pct.as_ref().map(|n| n.confidence),
        Some(read.higher_than_pct.confidence)
    );
    let confidences = [
        u.role.as_ref().map(|t| t.confidence),
        u.division.as_ref().map(|n| n.confidence),
        u.change_tags.as_ref().map(|t| t.confidence),
        u.next_mode_hint.as_ref().map(|t| t.confidence),
    ];
    for c in confidences {
        assert!(c.is_some_and(|c| c > 0.0 && c <= 1.0), "{c:?}");
    }
}

#[test]
fn a_negative_progress_is_kept_as_it_is() {
    let img = paint(&Scene {
        pct: -24,
        ..Scene::default()
    });
    let u = read_for_game(reader(), &img).expect("a rank update");
    assert_eq!(num(&u.progress_pct), Some(-24));
}

#[test]
fn only_a_2560x1440_frame_gives_a_rank() {
    use image::imageops::{self, FilterType};
    let painted = paint(&Scene::default());
    assert!(read_for_game(reader(), &painted).is_some());
    // sizes the reader itself takes, refused by the wiring
    for (w, h) in [(1920, 1080), (2524, 1420)] {
        let img = imageops::resize(&painted, w, h, FilterType::Nearest);
        assert!(layout::Scale::from_img(&img).is_some(), "{w}x{h}");
        assert_eq!(read_for_game(reader(), &img), None, "{w}x{h}");
    }
    let one_080 = imageops::resize(&painted, 1920, 1080, FilterType::Nearest);
    assert!(reader().read(&one_080).is_ok(), "the reader reads 1080p");
    // and sizes nobody takes
    for (w, h) in [(0, 0), (1, 1), (1280, 720), (3840, 2160), (3440, 1440)] {
        let img = RgbImage::from_pixel(w, h, BG);
        assert_eq!(read_for_game(reader(), &img), None, "{w}x{h}");
    }
}

#[test]
fn the_placement_screen_and_other_screens_give_no_rank() {
    // The placement screen shows no "RANK PROGRESS:" label: another word
    // where it goes, the rest of the screen as on a rank update.
    let mut img = paint(&Scene::default());
    fill(&mut img, 830, 1060, 980, 1100, BG);
    draw(&mut img, "OEH 1", 842, 1068, 3, WHITE);
    assert_eq!(reader().read(&img).err(), Some(ReadError::NotRankScreen));
    assert_eq!(read_for_game(reader(), &img), None);
    assert_eq!(read_for_game(reader(), &blank()), None);
}

#[test]
fn a_pack_without_all_three_role_icons_gives_no_role() {
    let mut set = trained().clone();
    if let Some(roles) = set.families.get_mut(&Family::Role) {
        roles.retain(|t| t.class == "Support");
    }
    let one_class = Reader::new(set);
    assert!(!roles_complete(&one_class));
    // A Support icon judged against Support alone reads as Support...
    let img = paint(&Scene {
        role: Some(Role::Support),
        ..Scene::default()
    });
    let raw = one_class.read(&img).expect("a rank screen");
    assert_eq!(raw.role.value, Some(Role::Support));
    // ...and so would any other icon: the game record gets no role, and
    // keeps every other field.
    let u = read_for_game(&one_class, &img).expect("a rank update");
    assert_eq!(u.role, None);
    assert_eq!(num(&u.hero_sr), Some(3597));
    assert_eq!(num(&u.progress_pct), Some(31));
    // an open queue screen (no role line) too
    let open = read_for_game(&one_class, &paint(&Scene::default())).expect("a rank update");
    assert_eq!(open.role, None);
    // the full pack keeps both
    let full = read_for_game(reader(), &paint(&Scene::default())).expect("a rank update");
    assert_eq!(text(&full.role), Some("Open"));
}

#[test]
fn more_than_eight_change_tags_is_no_tag_list() {
    let mut read = reader()
        .read(&paint(&Scene::default()))
        .expect("a rank screen");
    for (n, kept) in [(8, true), (9, false), (12, false)] {
        read.change_tags.value = Some((0..n).map(|k| format!("T{k}")).collect());
        let u = to_rank_update(&read, true);
        assert_eq!(u.change_tags.is_some(), kept, "{n} tags");
        assert_eq!(num(&u.hero_sr), Some(3597), "{n} tags: other fields stay");
    }
}

#[test]
fn a_value_without_a_usable_confidence_is_not_kept() {
    let mut read = reader()
        .read(&paint(&Scene::default()))
        .expect("a rank screen");
    read.hero_sr.confidence = 0.0;
    read.division.confidence = f32::NAN;
    read.progress_pct.confidence = 1.5;
    let u = to_rank_update(&read, true);
    assert_eq!(u.hero_sr, None);
    assert_eq!(u.division, None);
    assert_eq!(u.progress_pct, None);
    assert_eq!(num(&u.higher_than_pct), Some(96));
}

#[test]
fn roles_are_spelled_as_the_site_role() {
    for r in [Role::Tank, Role::Damage, Role::Support, Role::Open] {
        let site = serde_json::to_value(rank_role(r)).expect("serializes");
        assert_eq!(site.as_str(), Some(r.as_str()), "{r:?}");
    }
}

#[test]
fn one_recognizer_id_for_the_rank_readers() {
    assert_eq!(RECOGNIZER_ID, "rank-v1");
    assert_eq!(RECOGNIZER_ID, crate::model::RANK_RECOGNIZER_ID);
    assert_eq!(RECOGNIZER_ID, crate::model::QUEUE_TITLE_RECOGNIZER_ID);
    assert_eq!(RECOGNIZER_ID, crate::shadow::queue_title::RECOGNIZER_ID);
    assert_eq!(trained().recognizer, RECOGNIZER_ID);
}

#[test]
fn only_the_rank_progress_class_opens_the_gate() {
    let img = paint(&Scene::default());
    assert!(reader().is_rank_screen(&img).is_ok());
    // A pack whose gate family also holds another word: the label read as
    // that word is no rank screen. The real label's template is kept, but
    // as a flat vector it scores below the other word.
    let mut set = trained().clone();
    let gates = set.families.get_mut(&Family::Gate).expect("gate family");
    let mut other = gates[0].clone();
    other.class = "placement".to_string();
    for t in gates.iter_mut() {
        t.vector.iter_mut().for_each(|v| *v = 0.0);
    }
    gates.push(other);
    let r = Reader::new(set);
    assert_eq!(r.is_rank_screen(&img), Err(ReadError::NotRankScreen));
    assert_eq!(r.read(&img).err(), Some(ReadError::NotRankScreen));
}

#[test]
fn an_sr_tall_line_above_the_entry_gives_no_hero_sr() {
    // The top hero's SR line with no name line the shape test takes: the
    // entry found below it is the second hero's.
    let mut img = paint(&Scene::default());
    draw(&mut img, "8888", 2333 - width("8888", 3), 640, 3, WHITE);
    let r = reader().read(&img).expect("a rank screen");
    assert_eq!(r.hero_sr.value, None);
    assert_eq!(r.hero_sr_change.value, None);
    assert_eq!(r.progress_pct.value, Some(31), "the rest still reads");
    assert_eq!(layout::first_entry(&img), None);
    // without that line the same entry reads
    let plain = reader()
        .read(&paint(&Scene::default()))
        .expect("a rank screen");
    assert_eq!(plain.hero_sr.value, Some(3597));
}

#[test]
fn an_sr_with_ink_right_of_its_window_gives_no_hero_sr() {
    // A two word name whose gap ends the name early: the SR window ends
    // after the third digit, which alone reads as a valid 3 digit SR.
    let mut img = paint(&Scene::default());
    fill(&mut img, 2120, 696, 2342, 718, BG);
    draw(&mut img, "HE", 2313 - width("HE", 2), 700, 2, WHITE);
    draw(&mut img, "L", 2329, 700, 2, WHITE);
    let e = layout::first_entry(&img).expect("an entry");
    assert!(e.right < 2318, "the name ends early: {}", e.right);
    assert!(layout::sr_cut_short(&img, &e));
    let r = reader().read(&img).expect("a rank screen");
    assert_eq!(r.hero_sr.value, None);
    assert_eq!(r.hero_sr_change.value, None);
    assert_eq!(r.higher_than_pct.value, Some(96), "the rest still reads");
    // the plain screen's window has no ink right of it
    let plain = paint(&Scene::default());
    let e = layout::first_entry(&plain).expect("an entry");
    assert!(!layout::sr_cut_short(&plain, &e));
}

#[test]
fn a_pack_on_disk_loads_as_the_wirings_frame_read() {
    // the trained pack written where the tracker looks for it
    let dir = tempfile::tempdir().unwrap();
    trained().save_dir(&dir.path().join(TEMPLATE_DIR)).unwrap();
    let read = crate::shadow::rank::game::load_for_game(dir.path()).expect("the pack loads");
    let img = paint(&Scene {
        role: Some(Role::Support),
        ..Scene::default()
    });
    let got = read(&img).expect("a rank update");
    assert_eq!(Some(got), read_for_game(reader(), &img));
    // another size reads nothing; no pack is an error
    assert_eq!(read(&RgbImage::new(1920, 1080)), None);
    assert!(crate::shadow::rank::game::load_for_game(&dir.path().join("none")).is_err());
}
