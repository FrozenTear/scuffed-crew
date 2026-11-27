//! The 1920x1080 path of the ban results and ban vote readers (issue #931),
//! on the synthetic frames of [`super`]: a 1440p frame downscaled to 1080p
//! reads as the 1440p frame does, and the region upscale the readers use
//! is the whole-frame upscale wherever a reader looks.

use image::{Rgb, RgbImage, imageops};

use super::super::upscale::{RESULTS_REGIONS, RING_REGION, Region, VOTE_REGIONS, at_read_size};
use super::*;

/// `img` (2560x1440) as a 1920x1080 game would show it.
fn to_1080(img: &RgbImage) -> RgbImage {
    imageops::resize(img, 1920, 1080, imageops::FilterType::Lanczos3)
}

/// What a ban results read says, without the scores.
fn ban_values(b: &BanResults) -> (Layout, Vec<(Option<&str>, Option<Team>)>, Option<&str>) {
    (
        b.layout,
        b.bans.iter().map(|b| (key(&b.read), b.team)).collect(),
        b.lobby_ban.as_ref().and_then(key),
    )
}

/// What a ban vote read says, without the scores.
#[allow(clippy::type_complexity)]
fn vote_values(
    v: &BanVote,
) -> (
    Option<Vec<(Option<SlotState>, Option<&str>)>>,
    Option<Vec<(u8, Option<&str>, Option<u32>, bool)>>,
) {
    (
        v.slots
            .as_ref()
            .map(|s| s.iter().map(|s| (s.state, s.hero.as_deref())).collect()),
        v.score.as_ref().map(|s| {
            s.iter()
                .map(|e| (e.rank, e.hero.as_deref(), e.votes, e.empty))
                .collect()
        }),
    )
}

#[test]
fn a_1080p_ban_results_frame_reads_the_same_as_the_1440p_one() {
    let r = reader();
    let ring = self_ring_colour(&row_frame(&HS_Y, [Row::Empty; 6]));
    for (sides, lobby) in [
        ([OWN, OWN, ENEMY, ENEMY], None),
        ([ENEMY, OWN, OWN, ENEMY], Some("mei")),
    ] {
        let big = ban_frame(FIVE, sides, lobby);
        let at_1440 = r.read_ban_results(&big, Some(ring)).expect("1440p read");
        let at_1080 = r
            .read_ban_results(&to_1080(&big), Some(ring))
            .expect("1080p read");
        assert_eq!(ban_values(&at_1080), ban_values(&at_1440));
        // Not a vacuous match: every ban and the lobby ban are read.
        assert_eq!(at_1080.bans_set(), 4);
        assert_eq!(at_1080.lobby_ban.is_some(), lobby.is_some());
        assert!(at_1080.bans.iter().all(|b| b.team.is_some()));
        for b in &at_1080.bans {
            assert!(b.read.score >= MIN_SCORE && b.read.margin >= MIN_MARGIN);
        }
    }
}

#[test]
fn a_1080p_ban_vote_frame_reads_the_same_as_the_1440p_one() {
    let r = reader();
    for slots in [
        [Slot::Hero("kiriko"), Slot::Skipped, Slot::Empty],
        [
            Slot::Hero("roadhog"),
            Slot::Hero("widowmaker"),
            Slot::Hero("mercy"),
        ],
    ] {
        let big = vote_frame(slots, STRIP3);
        let at_1440 = r.read_ban_vote(&big).expect("1440p read");
        let at_1080 = r.read_ban_vote(&to_1080(&big)).expect("1080p read");
        assert_eq!(vote_values(&at_1080), vote_values(&at_1440));
        // Not a vacuous match: the slots and the strip are read.
        assert!(at_1080.slots.is_some() && at_1080.score.is_some());
        assert!(
            at_1080
                .score
                .as_ref()
                .unwrap()
                .iter()
                .all(|e| e.hero.is_some())
        );
    }
}

#[test]
fn at_frame_size_keeps_1440p_and_upscales_1080p() {
    let big = RgbImage::from_pixel(FRAME_W, FRAME_H, BG);
    assert!(matches!(
        at_frame_size(&big),
        Some(std::borrow::Cow::Borrowed(_))
    ));
    let small = RgbImage::from_pixel(FRAME_1080_W, FRAME_1080_H, BG);
    let up = at_frame_size(&small).expect("1080p is taken");
    assert_eq!(up.dimensions(), (FRAME_W, FRAME_H));
    assert!(matches!(up, std::borrow::Cow::Owned(_)));
}

fn noise_pixel(r: &mut Lcg) -> Rgb<u8> {
    Rgb([0, 0, 0].map(|_| r.below(256) as u8))
}

#[test]
fn the_region_upscale_has_the_whole_frame_pixels_inside_every_region() {
    let mut r = Lcg(7);
    let small = RgbImage::from_fn(FRAME_1080_W, FRAME_1080_H, |_, _| noise_pixel(&mut r));
    let whole = at_frame_size(&small).expect("1080p");
    let regions: Vec<Region> = RESULTS_REGIONS
        .iter()
        .chain(&VOTE_REGIONS)
        .chain([&RING_REGION])
        .copied()
        .collect();
    let part = at_read_size(&small, &regions).expect("1080p");
    assert_eq!(part.dimensions(), (FRAME_W, FRAME_H));
    for &(x0, y0, x1, y1) in &regions {
        for y in y0..y1 {
            for x in x0..x1 {
                assert_eq!(part.get_pixel(x, y), whole.get_pixel(x, y), "({x}, {y})");
            }
        }
    }
    // Outside the regions nothing was upscaled.
    assert_eq!(part.get_pixel(0, 0).0, [0, 0, 0]);
    assert_eq!(part.get_pixel(FRAME_W - 1, FRAME_H - 1).0, [0, 0, 0]);
    // A 1440p frame is not copied.
    let big = RgbImage::from_pixel(FRAME_W, FRAME_H, BG);
    assert!(matches!(
        at_read_size(&big, &regions),
        Some(std::borrow::Cow::Borrowed(_))
    ));
}

/// `img` with its flat background turned to noise: a reader that looked
/// outside its regions would see black in the region upscale and noise in
/// the whole-frame one.
fn noisy(img: &RgbImage) -> RgbImage {
    let mut r = Lcg(99);
    let mut out = img.clone();
    for p in out.pixels_mut() {
        if *p == BG {
            *p = noise_pixel(&mut r);
        }
    }
    out
}

#[test]
fn a_1080p_read_equals_the_read_of_the_whole_upscaled_frame() {
    let r = reader();
    let ring = self_ring_colour(&row_frame(&HS_Y, [Row::Empty; 6]));
    for (sides, lobby) in [
        ([OWN, OWN, ENEMY, ENEMY], None),
        ([ENEMY, OWN, OWN, ENEMY], Some("mei")),
    ] {
        let small = to_1080(&noisy(&ban_frame(FIVE, sides, lobby)));
        let whole = at_frame_size(&small).expect("1080p").into_owned();
        let part = r.read_ban_results(&small, Some(ring)).expect("1080p read");
        assert_eq!(Some(&part), r.read_ban_results(&whole, Some(ring)).as_ref());
        assert_eq!(part.bans_set(), 4, "{part:?}");
    }
    for slots in [
        [Slot::Hero("kiriko"), Slot::Skipped, Slot::Empty],
        [Slot::Hero("roadhog"), Slot::Hero("mercy"), Slot::Skipped],
    ] {
        let small = to_1080(&noisy(&vote_frame(slots, STRIP3)));
        let whole = at_frame_size(&small).expect("1080p").into_owned();
        let part = r.read_ban_vote(&small).expect("1080p read");
        assert_eq!(Some(&part), r.read_ban_vote(&whole).as_ref());
        assert!(part.slots.is_some() && part.score.is_some(), "{part:?}");
    }
}

#[test]
fn the_ban_vote_ring_reads_at_1440p_and_1080p() {
    let big = row_frame(&HS_Y, [Row::Empty; 6]);
    assert_eq!(ban_vote_ring(&big), Some(self_ring_colour(&big)));
    let small = to_1080(&big);
    let whole = at_frame_size(&small).expect("1080p");
    let ring = ban_vote_ring(&small).expect("1080p ring");
    assert_eq!(ring, self_ring_colour(&whole));
    let [h, s, _] = imaging::hsv(ring);
    let [oh, _, _] = imaging::hsv(OWN.0);
    assert!(hue_distance(h, oh) <= 3 && s >= 200, "{ring:?}");
    assert_eq!(ban_vote_ring(&RgbImage::new(1280, 720)), None);
}
