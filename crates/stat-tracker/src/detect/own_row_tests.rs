//! Tests for [`super`] (`detect::own_row`). Synthetic boards and made-up
//! glyphs only: no captured pixels and no real name.
use super::*;
use crate::shadow::heroes::test_fixtures::{PURPLE, TEAL, YELLOW, board};
use crate::shadow::heroes::{HeroBoard, HeroFrameRead, HeroPack, HeroRead, PanelRead, RowClass};
use image::Rgb;

/// Made-up 5x7 glyphs (bit 4 is the left column).
const GLYPHS: [[u8; 7]; 10] = [
    [0x1f, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
    [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
    [0x1f, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
    [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
    [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
    [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
    [0x0e, 0x11, 0x10, 0x16, 0x11, 0x11, 0x0e],
    [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
    [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
    [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
];

const NAME_X: f64 = 2.30;
const NAME_Y: f64 = 0.27;
/// Glyph pitch on a row, in row pitches.
const GLYPH_STEP: f64 = 0.25;

/// Where a name is drawn on its row, in row pitches.
#[derive(Clone, Copy)]
struct At {
    x: f64,
    y: f64,
}

const HOME: At = At {
    x: NAME_X,
    y: NAME_Y,
};

/// Top of `row` on a fixture board.
fn row_top(team: usize, pitch: u32, row: usize) -> u32 {
    let hh = (pitch as f32 * 0.39) as u32;
    hh + (row / team) as u32 * (team as u32 * pitch + pitch) + (row % team) as u32 * pitch
}

/// Draw the made-up name `glyphs` on `row` of a fixture board.
fn draw_name(img: &mut RgbImage, team: usize, pitch: u32, row: usize, glyphs: &[usize], at: At) {
    // Whole pixels for the origin and the glyph pitch, so a name is the
    // same raster wherever it is drawn (the game snaps its text too).
    let p = f64::from(pitch);
    let x0 = (f64::from(pitch / 2) + at.x * p).round() as u32;
    let y0 = (f64::from(row_top(team, pitch, row)) + at.y * p).round() as u32;
    let (gw, gh) = ((0.20 * p).round() as u32, (0.34 * p).round() as u32);
    let step = (GLYPH_STEP * p).round() as u32;
    for (i, &g) in glyphs.iter().enumerate() {
        let gx = x0 + i as u32 * step;
        for y in 0..gh {
            let bits = GLYPHS[g][((y * 7) / gh) as usize];
            for x in 0..gw {
                if bits >> (4 - (x * 5) / gw) & 1 == 1 {
                    img.put_pixel(gx + x, y0 + y, Rgb([255; 3]));
                }
            }
        }
    }
}

/// A fixture board with one made-up name per row.
fn named_board(team: usize, pitch: u32, colours: [[u8; 3]; 2], names: &[Vec<usize>]) -> RgbImage {
    let mut img = board(&vec![1; 2 * team], team, pitch, colours);
    for (row, name) in names.iter().enumerate() {
        draw_name(&mut img, team, pitch, row, name, HOME);
    }
    img
}

/// The member's made-up name.
const OWN: [usize; 6] = [0, 3, 6, 1, 8, 5];

/// `n` made-up names of four to seven glyphs, none of them [`OWN`].
fn other_names(n: usize) -> Vec<Vec<usize>> {
    (0..n)
        .map(|i| (0..4 + i % 4).map(|k| (i * 3 + k * 7 + 2) % 10).collect())
        .collect()
}

/// Names for a board of `2 * team` rows with the member's on `own`.
fn roster(team: usize, own: usize) -> Vec<Vec<usize>> {
    let mut names = other_names(2 * team);
    names[own] = OWN.to_vec();
    names
}

fn measure(img: &RgbImage, team: usize) -> NameBoard {
    NameBoard::measure(None, &DynamicImage::ImageRgb8(img.clone()), team).expect("board")
}

fn candidate(img: &RgbImage, team: usize, row: usize) -> Candidate {
    measure(img, team).candidate(row).expect("candidate")
}

/// The game of session `session`: every session is its own game, on its
/// own map.
fn game(session: &str) -> GameTag {
    GameTag {
        session: session.to_string(),
        map: format!("map of {session}"),
        at: 0,
    }
}

/// Offer the member's name from three boards of two games.
fn fill(store: &mut SeedStore, team: usize, pitch: u32) {
    for (i, session) in ["g1", "g1", "g2"].into_iter().enumerate() {
        let img = named_board(team, pitch, [TEAL, PURPLE], &roster(team, i));
        store.offer(candidate(&img, team, i), &game(session));
    }
}

/// A store that trusts the member's name at `pitch`.
fn seeded(team: usize, pitch: u32) -> SeedStore {
    let mut store = SeedStore::in_memory(None);
    fill(&mut store, team, pitch);
    assert_eq!(store.trusted().len(), PROMOTE);
    store
}

#[test]
fn a_seed_finds_its_row_and_no_other_row() {
    let store = seeded(5, 88);
    for own in [0, 3, 7] {
        let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, own));
        let scores = measure(&img, 5).scores(store.trusted());
        assert_eq!(seed_row(&scores), Some(own), "{scores:?}");
        assert!(scores[own] > 0.97, "{scores:?}");
        for (row, &s) in scores.iter().enumerate() {
            assert!(
                row == own || s < MATCH_FLOOR - 0.15,
                "row {row}: {scores:?}"
            );
        }
    }
    // a board without the member: no row
    let img = named_board(5, 88, [TEAL, PURPLE], &other_names(10));
    let scores = measure(&img, 5).scores(store.trusted());
    assert_eq!(seed_row(&scores), None, "{scores:?}");
}

#[test]
fn the_best_row_needs_the_floor_and_a_lead() {
    assert_eq!(seed_row(&[0.5, 0.95, 0.6]), Some(1));
    assert_eq!(seed_row(&[0.5, MATCH_FLOOR, 0.6]), Some(1));
    // under the floor
    assert_eq!(seed_row(&[0.5, MATCH_FLOOR - 0.01, 0.2]), None);
    // two rows too close: a look-alike on the board
    assert_eq!(seed_row(&[0.5, 0.95, 0.95 - MATCH_MARGIN + 0.01]), None);
    assert_eq!(seed_row(&[0.5, 0.95, 0.95 - MATCH_MARGIN - 0.01]), Some(1));
    assert_eq!(seed_row(&[-1.0, -1.0]), None);
    assert_eq!(seed_row(&[]), None);
}

#[test]
fn a_seed_is_trusted_only_after_three_boards_from_two_games_agree() {
    let offer = |store: &mut SeedStore, own: usize, names: Vec<Vec<usize>>, tag: &GameTag| {
        let img = named_board(5, 88, [TEAL, PURPLE], &names);
        store.offer(candidate(&img, 5, own), tag)
    };
    // three boards of one game are not enough
    let mut store = SeedStore::in_memory(None);
    for own in 0..3 {
        assert_eq!(
            offer(&mut store, own, roster(5, own), &game("g1")),
            Offer::Pending
        );
    }
    assert!(store.trusted().is_empty());
    // a board of a second game completes the group
    assert_eq!(
        offer(&mut store, 4, roster(5, 4), &game("g2")),
        Offer::Promoted
    );
    assert_eq!(store.trusted().len(), PROMOTE);
    assert_eq!(store.pending_len(), 0);
    // once trusted, the same name is known and nothing more is kept
    assert_eq!(
        offer(&mut store, 2, roster(5, 2), &game("g3")),
        Offer::Known
    );
    assert_eq!(store.pending_len(), 0);

    // two boards from two games are not enough
    let mut store = SeedStore::in_memory(None);
    assert_eq!(
        offer(&mut store, 0, roster(5, 0), &game("g1")),
        Offer::Pending
    );
    assert_eq!(
        offer(&mut store, 1, roster(5, 1), &game("g2")),
        Offer::Pending
    );
    assert!(store.trusted().is_empty());
    // a third crop that shows another name does not complete it
    assert_eq!(
        offer(&mut store, 2, other_names(10), &game("g3")),
        Offer::Pending
    );
    assert!(store.trusted().is_empty());
    assert_eq!(store.pending_len(), 3);
    // nor does a look-alike: one glyph of the member's name changed
    let mut names = other_names(10);
    names[3] = OWN.to_vec();
    names[3][2] = 9;
    assert_eq!(offer(&mut store, 3, names, &game("g4")), Offer::Pending);
    assert!(store.trusted().is_empty());
}

#[test]
fn the_waiting_list_and_the_trusted_set_stay_small() {
    let name_of = |i: usize| {
        let dense = [0, 1, 5, 6, 8, 9];
        vec![
            dense[i % 6],
            dense[(i / 6) % 6],
            dense[(i / 2 + 1) % 6],
            0,
            1,
        ]
    };
    let crop_of = |i: usize| {
        let mut names = other_names(10);
        names[0] = name_of(i);
        candidate(&named_board(5, 88, [TEAL, PURPLE], &names), 5, 0)
    };
    let mut store = SeedStore::in_memory(None);
    // 20 different names, one board each: none is ever trusted
    for i in 0..20usize {
        store.offer(crop_of(i), &game(&format!("g{i}")));
    }
    assert!(store.trusted().is_empty());
    assert_eq!(store.pending_len(), PENDING_MAX);
    // The oldest waiting crops made room, not the newest: the last name is
    // still waiting (two more crops complete it), the first is gone (two
    // more crops of it are only two).
    assert_eq!(store.offer(crop_of(0), &game("x1")), Offer::Pending);
    assert_eq!(store.offer(crop_of(0), &game("x2")), Offer::Pending);
    assert_eq!(store.offer(crop_of(19), &game("x3")), Offer::Pending);
    assert_eq!(store.offer(crop_of(19), &game("x4")), Offer::Promoted);

    // Trusted seeds: at most TRUSTED_PER_SCALE per scale, the oldest go.
    let mut store = seeded(5, 88);
    let first: Vec<Seed> = store.trusted().to_vec();
    let mut groups: Vec<Vec<Seed>> = Vec::new();
    for (n, name) in [[9, 9, 1, 5, 0], [8, 6, 0, 1, 9]].into_iter().enumerate() {
        let mut group = Vec::new();
        for (i, session) in ["a", "a", "b"].into_iter().enumerate() {
            let mut names = other_names(10);
            names[i] = name.to_vec();
            let crop = candidate(&named_board(5, 88, [TEAL, PURPLE], &names), 5, i);
            group.push(crop.seed().clone());
            store.offer(crop, &game(&format!("{session}{n}")));
        }
        groups.push(group);
        assert!(store.trusted().len() <= TRUSTED_PER_SCALE);
    }
    assert_eq!(TRUSTED_PER_SCALE, 2 * PROMOTE, "two names fit whole");
    assert_eq!(store.trusted().len(), TRUSTED_PER_SCALE);
    // the first name's seeds are the ones that left; the two later groups
    // are whole, oldest first
    assert!(first.iter().all(|seed| !store.trusted().contains(seed)));
    assert_eq!(store.trusted()[..PROMOTE], groups[0][..]);
    assert_eq!(store.trusted()[PROMOTE..], groups[1][..]);
    // a seed of another scale is not counted against this one
    fill(&mut store, 6, 75);
    assert_eq!(store.trusted().len(), TRUSTED_PER_SCALE + PROMOTE);
}

#[test]
fn a_seed_matches_through_team_colours_and_the_highlight() {
    let store = seeded(5, 88);
    // team colours a luma or brightest-channel mask cannot tell from white
    // text, and a light own row
    let palettes: [[[u8; 3]; 2]; 4] = [
        [[255, 255, 40], [230, 30, 200]],
        [[230, 30, 200], [255, 255, 40]],
        [YELLOW, PURPLE],
        [[120, 200, 235], [200, 40, 60]],
    ];
    for colours in palettes {
        for own in [1, 6] {
            let img = named_board(5, 88, colours, &roster(5, own));
            let scores = measure(&img, 5).scores(store.trusted());
            assert_eq!(seed_row(&scores), Some(own), "{colours:?} {scores:?}");
            assert!(scores[own] > 0.95, "{colours:?} {scores:?}");
        }
    }
}

#[test]
fn a_seed_follows_the_name_when_it_moves_on_the_row() {
    // A badge in front of the name moves it right, a title under it moves it
    // up, and a board without ult discs moves it left.
    let store = seeded(5, 88);
    for (x, y) in [(0.55, 0.0), (0.0, 0.07), (-0.60, 0.05), (0.31, -0.04)] {
        let mut names = other_names(10);
        names[2] = Vec::new();
        let mut img = named_board(5, 88, [TEAL, PURPLE], &names);
        let at = At {
            x: NAME_X + x,
            y: NAME_Y + y,
        };
        draw_name(&mut img, 5, 88, 2, &OWN, at);
        let scores = measure(&img, 5).scores(store.trusted());
        assert_eq!(seed_row(&scores), Some(2), "shift {x} {y}: {scores:?}");
        assert!(scores[2] > 0.90, "shift {x} {y}: {scores:?}");
    }
}

#[test]
fn a_look_alike_name_is_not_the_members_row() {
    let store = seeded(5, 88);
    let mut variants: Vec<Vec<usize>> = Vec::new();
    // one glyph changed, at each position
    for i in 0..OWN.len() {
        let mut v = OWN.to_vec();
        v[i] = (v[i] + 1) % 10;
        variants.push(v);
    }
    // the member's name with more glyphs after it, and before it
    variants.push(OWN.iter().copied().chain([2, 4]).collect());
    variants.push([4].into_iter().chain(OWN).collect());
    // the member's name cut short
    variants.push(OWN[..4].to_vec());
    for v in variants {
        let mut names = other_names(10);
        names[3] = v.clone();
        let img = named_board(5, 88, [TEAL, PURPLE], &names);
        let scores = measure(&img, 5).scores(store.trusted());
        assert_eq!(seed_row(&scores), None, "{v:?}: {scores:?}");
        assert!(scores[3] < MATCH_FLOOR, "{v:?}: {scores:?}");
        // and with the member on the board, the member's row still wins
        names[8] = OWN.to_vec();
        let img = named_board(5, 88, [TEAL, PURPLE], &names);
        let scores = measure(&img, 5).scores(store.trusted());
        assert_eq!(seed_row(&scores), Some(8), "{v:?}: {scores:?}");
    }
}

#[test]
fn the_five_and_six_player_tables_each_keep_their_own_seeds() {
    // 1440p and 1080p pitches of both tables, and the table moved sideways
    // (the perk layouts move its left edge).
    for (team, pitch) in [(5usize, 88u32), (5, 66), (6, 75), (6, 57)] {
        let store = seeded(team, pitch);
        for own in [0, team + 1] {
            let img = named_board(team, pitch, [TEAL, PURPLE], &roster(team, own));
            for shift in [0u32, 23, 57] {
                let mut wide =
                    RgbImage::from_pixel(img.width() + 60, img.height(), Rgb([12, 14, 24]));
                image::imageops::replace(&mut wide, &img, i64::from(shift), 0);
                let board = measure(&wide, team);
                assert_eq!(board.len(), 2 * team);
                let scores = board.scores(store.trusted());
                assert_eq!(
                    seed_row(&scores),
                    Some(own),
                    "{team}v{team} @{pitch} +{shift}: {scores:?}"
                );
            }
        }
        // a seed of another scale is never compared
        let other = if team == 5 { (6, 75) } else { (5, 88) };
        let img = named_board(other.0, other.1, [TEAL, PURPLE], &roster(other.0, 1));
        let board = measure(&img, other.0);
        assert!(!board.seeded(store.trusted()));
        assert!(board.scores(store.trusted()).iter().all(|&s| s < 0.0));
    }
}

fn lit(row: usize, lead: f64) -> Option<Highlight> {
    Some(Highlight { row, lead })
}

#[test]
fn combine_is_sure_only_when_seed_and_highlight_agree() {
    let own = |row, trust, source| OwnRow { row, trust, source };
    let (strong, weak) = (SEED_HIGHLIGHT_LEAD, SEED_HIGHLIGHT_LEAD - 0.01);
    for lead in [strong, weak] {
        assert_eq!(
            combine(Some(2), lit(2, lead), true),
            own(Some(2), Trust::Sure, Source::Both)
        );
    }
    // different rows, strong highlight: no row, whichever is which
    assert_eq!(
        combine(Some(2), lit(0, strong), true),
        own(None, Trust::Suspect, Source::Disagree)
    );
    assert_eq!(
        combine(Some(0), lit(2, strong), true),
        own(None, Trust::Suspect, Source::Disagree)
    );
    // different rows, weak highlight: the seed's row, marked (F262-7)
    assert_eq!(
        combine(Some(2), lit(0, weak), true),
        own(Some(2), Trust::Suspect, Source::SeedOverWeakHighlight)
    );
    // one alone is used and marked
    assert_eq!(
        combine(Some(7), None, true),
        own(Some(7), Trust::Suspect, Source::SeedOnly)
    );
    assert_eq!(
        combine(None, lit(1, strong), true),
        own(Some(1), Trust::Suspect, Source::HighlightOnly)
    );
    // before any seed of this scale exists a strong highlight stands
    // unmarked, as it always did; a weak one is marked
    assert_eq!(
        combine(None, lit(1, strong), false),
        own(Some(1), Trust::Unconfirmed, Source::HighlightOnly)
    );
    assert_eq!(
        combine(None, lit(1, weak), false),
        own(Some(1), Trust::Suspect, Source::HighlightOnly)
    );
    assert_eq!(
        combine(None, None, true),
        own(None, Trust::Suspect, Source::Neither)
    );
    assert_eq!(combine(None, None, false).row, None);
    assert!(combine(Some(7), None, true).suspect());
    assert!(combine(None, lit(1, strong), true).suspect());
    assert!(combine(None, lit(1, weak), false).suspect());
    assert!(combine(Some(2), lit(0, weak), true).suspect());
    assert!(!combine(Some(2), lit(2, strong), true).suspect());
    assert!(!combine(None, lit(1, strong), false).suspect());
    assert!(!combine(Some(2), lit(0, strong), true).suspect());
    // the name fallback's row is stored as main stores a name match
    let named = OwnRow::from_name(6);
    assert_eq!(
        named,
        own(Some(6), Trust::Unconfirmed, Source::NameFallback)
    );
    assert!(!named.suspect());
    assert!(
        Highlight {
            row: 0,
            lead: strong
        }
        .strong()
    );
    assert!(!Highlight { row: 0, lead: weak }.strong());
}

fn read(
    img: &RgbImage,
    team: usize,
    highlight: Option<(usize, f64)>,
    seeds: &[Seed],
) -> OwnRowRead {
    read_own_row(
        None,
        &DynamicImage::ImageRgb8(img.clone()),
        team,
        highlight.map(|(row, lead)| Highlight { row, lead }),
        seeds,
    )
}

#[test]
fn read_own_row_combines_the_seed_with_the_highlight() {
    let store = seeded(5, 88);
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 2));
    let sure = read(&img, 5, Some((2, 1.5)), store.trusted());
    assert_eq!((sure.own.row, sure.own.trust), (Some(2), Trust::Sure));
    assert!(sure.best > 0.97 && sure.next < MATCH_FLOOR);
    assert_eq!(sure.verdict, Verdict::Agree(88));
    // a strong highlight on another row than the member's name: held, and
    // counted against the seeds
    let split = read(&img, 5, Some((0, 1.5)), store.trusted());
    assert_eq!((split.own.row, split.own.source), (None, Source::Disagree));
    assert_eq!(split.verdict, Verdict::Disagree(88));
    // a weak highlight on another row: the seed's row, marked
    let over = read(&img, 5, Some((0, 1.1)), store.trusted());
    assert_eq!(
        (over.own.row, over.own.trust, over.own.source),
        (Some(2), Trust::Suspect, Source::SeedOverWeakHighlight)
    );
    assert_eq!(over.verdict, Verdict::Silent);
    // no highlight: the seed alone, marked
    let alone = read(&img, 5, None, store.trusted());
    assert_eq!((alone.own.row, alone.own.trust), (Some(2), Trust::Suspect));
    // the member's name is not on the board: the highlight alone, marked
    let img = named_board(5, 88, [TEAL, PURPLE], &other_names(10));
    let lit = read(&img, 5, Some((1, 1.5)), store.trusted());
    assert_eq!((lit.own.row, lit.own.trust), (Some(1), Trust::Suspect));
    // no seed at all: the highlight as before seeds existed
    let bare = read(&img, 5, Some((1, 1.5)), &[]);
    assert_eq!(
        (bare.own.row, bare.own.trust),
        (Some(1), Trust::Unconfirmed)
    );
    assert_eq!(read(&img, 5, None, &[]).own.row, None);
    // no table to measure: the highlight. With seeds in hand it is marked
    // (the seed could not confirm it); with none it is as it always was.
    let blank = RgbImage::from_pixel(600, 400, Rgb([10, 10, 10]));
    let none = read(&blank, 5, Some((3, 1.5)), store.trusted());
    assert_eq!((none.own.row, none.own.trust), (Some(3), Trust::Suspect));
    assert!(none.candidate.is_none());
    let bare = read(&blank, 5, Some((3, 1.5)), &[]);
    assert_eq!(
        (bare.own.row, bare.own.trust),
        (Some(3), Trust::Unconfirmed)
    );
    assert_eq!(read(&blank, 5, None, store.trusted()).own.row, None);
}

#[test]
fn a_candidate_is_cut_only_from_a_clearly_highlighted_unmatched_row() {
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 2));
    // a clear highlight and no seed: a candidate from that row
    let c = read(&img, 5, Some((2, SEED_HIGHLIGHT_LEAD)), &[])
        .candidate
        .expect("candidate");
    assert_eq!(c.row, 2);
    assert!(c.row_bottom > row_top(5, 88, 2));
    // two rows could be lit (a hover, a party tint): no candidate
    let dim = read(&img, 5, Some((2, SEED_HIGHLIGHT_LEAD - 0.01)), &[]);
    assert!(dim.candidate.is_none());
    assert_eq!(
        (dim.own.row, dim.own.trust),
        (Some(2), Trust::Suspect),
        "the row is still used, and marked"
    );
    assert!(read(&img, 5, None, &[]).candidate.is_none());
    // a seed already matches the row: nothing more to learn
    let store = seeded(5, 88);
    assert!(
        read(&img, 5, Some((2, 1.5)), store.trusted())
            .candidate
            .is_none()
    );
    // seed and highlight disagree: no candidate from either row
    assert!(
        read(&img, 5, Some((0, 1.5)), store.trusted())
            .candidate
            .is_none()
    );
}

#[test]
fn a_row_without_a_clean_name_line_gives_no_candidate() {
    let blank = board(&[1; 10], 5, 88, [TEAL, PURPLE]);
    assert!(measure(&blank, 5).candidate(0).is_none(), "no name");
    // a name that runs into the perk icons
    let mut long = blank.clone();
    draw_name(
        &mut long,
        5,
        88,
        0,
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0],
        HOME,
    );
    assert!(
        measure(&long, 5).candidate(0).is_none(),
        "reaches the perks"
    );
    // something drawn right after the name (an overlay, a second word)
    let mut after = blank.clone();
    draw_name(&mut after, 5, 88, 0, &OWN, HOME);
    let end = 44 + (NAME_X * 88.0).round() as u32 + OWN.len() as u32 * 22;
    let mid = row_top(5, 88, 0) + (NAME_Y * 88.0).round() as u32 + 14;
    for y in mid..mid + 3 {
        for x in end + 4..end + 40 {
            after.put_pixel(x, y, Rgb([255; 3]));
        }
    }
    assert!(
        measure(&after, 5).candidate(0).is_none(),
        "ink after the name"
    );
    // a block over the name, as tall as the row (a menu, a banner)
    let mut covered = blank.clone();
    draw_name(&mut covered, 5, 88, 0, &OWN, HOME);
    let top = row_top(5, 88, 0);
    for y in top..top + 86 {
        for x in 44 + 215..44 + 345 {
            covered.put_pixel(x, y, Rgb([250; 3]));
        }
    }
    assert!(measure(&covered, 5).candidate(0).is_none(), "covered");
    // one glyph is not a name line
    let mut short = blank.clone();
    draw_name(&mut short, 5, 88, 0, &[3], HOME);
    assert!(measure(&short, 5).candidate(0).is_none(), "too short");
    // the plain name is fine
    let mut ok = blank.clone();
    draw_name(&mut ok, 5, 88, 0, &OWN, HOME);
    assert!(measure(&ok, 5).candidate(0).is_some());
    // and so is a name whose last glyph ends in a thin stroke (an L, a T):
    // the crop runs to the end of that stroke
    for last in [7, 4] {
        let mut thin = blank.clone();
        draw_name(&mut thin, 5, 88, 0, &[0, 1, 5, 6, last], HOME);
        let c = measure(&thin, 5).candidate(0).expect("thin tail");
        assert!(
            c.seed().w >= 4 * 22 + 17,
            "glyph {last}: width {}",
            c.seed().w
        );
    }
}

fn hero(row: usize, key: Option<&str>) -> HeroRead {
    HeroRead {
        row,
        class: if key.is_some() {
            RowClass::Hero
        } else {
            RowClass::Unknown
        },
        hero: key.map(str::to_string),
        best: key.unwrap_or("?").to_string(),
        score: 0.99,
        lead: 0.5,
        second: "other".into(),
        suspect: key.is_none(),
    }
}

fn tab(rows: &[Option<&str>], panel: Option<&str>) -> TabHeroes {
    TabHeroes {
        team_size: rows.len() / 2,
        read: Some(HeroFrameRead {
            board: HeroBoard {
                rows: rows.iter().enumerate().map(|(i, k)| hero(i, *k)).collect(),
                elapsed_us: 0,
            },
            pack: HeroPack::V2,
            pad_px: None,
        }),
        panel: panel.map(|key| PanelRead {
            read: hero(0, Some(key)),
            pack: HeroPack::V2,
            box_px: (0, 0, 1, 1),
            rel: (0.0, 0.0, 0.1),
        }),
    }
}

#[test]
fn a_board_seeds_only_when_it_is_the_members_own_live_board() {
    let mut rows = vec![Some("ana"); 10];
    rows[2] = Some("juno");
    let live = tab(&rows, Some("juno"));
    assert!(seed_gate(2, 300, 1007, Chat::None, Some(&live)));
    // chat on screen reaching the row, or not known
    assert!(!seed_gate(2, 300, 1007, Chat::ReachesY(250), Some(&live)));
    assert!(!seed_gate(2, 300, 1007, Chat::Unknown, Some(&live)));
    assert!(seed_gate(2, 300, 1007, Chat::ReachesY(600), Some(&live)));
    // no hero pack, or no career panel: a replay, a spectated board, a
    // post-match table
    assert!(!seed_gate(2, 300, 1007, Chat::None, None));
    let no_panel = tab(&rows, None);
    assert!(!seed_gate(2, 300, 1007, Chat::None, Some(&no_panel)));
    // the panel shows another hero than the highlighted row
    let other = tab(&rows, Some("ana"));
    assert!(!seed_gate(2, 300, 1007, Chat::None, Some(&other)));
    assert!(!seed_gate(1, 300, 1007, Chat::None, Some(&live)));
    // the highlighted row's icon was not read (an ult ring, a skull): the
    // row is not shown to be the member's, so it does not seed (F262-3)
    rows[2] = None;
    let unread = tab(&rows, Some("juno"));
    assert!(!seed_gate(2, 300, 1007, Chat::None, Some(&unread)));
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn seeds_are_kept_private_and_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let mut store = SeedStore::open(data, Some("Made Up Name"));
    // nothing is created before a crop is trusted
    assert!(!data.join("private").exists());
    fill(&mut store, 5, 88);
    assert_eq!(store.trusted().len(), PROMOTE);
    let folder = seed_dir(data);
    assert_eq!(folder, data.join("private").join("own-name"));
    let file = folder.join(SEED_FILE);
    assert!(file.is_file());
    #[cfg(unix)]
    {
        assert_eq!(mode(&data.join("private")), 0o700);
        assert_eq!(mode(&folder), 0o700);
        assert_eq!(mode(&file), 0o600);
    }
    // one file, no temp file left behind, no name in a file name
    let names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![SEED_FILE.to_string()]);
    // the file holds the hash, never the name
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(&name_hash(Some("Made Up Name"))));
    assert!(!text.to_lowercase().contains("made"));
    // the same name after a restart: the trusted seeds are back
    let mut again = SeedStore::open(data, Some(" made up NAME "));
    assert_eq!(again.trusted(), store.trusted());
    // a waiting crop does not survive a restart: it was never written
    let before = std::fs::read(&file).unwrap();
    assert_eq!(again.offer(waiting_crop(), &game("g9")), Offer::Pending);
    assert_eq!(again.pending_len(), 1);
    assert_eq!(std::fs::read(&file).unwrap(), before);
    let restarted = SeedStore::open(data, Some("Made Up Name"));
    assert_eq!(restarted.pending_len(), 0);
    assert_eq!(restarted.trusted(), store.trusted());
}

/// A crop of another made-up name, as a wrong highlight would offer it.
fn waiting_crop() -> Candidate {
    let mut names = other_names(10);
    names[0] = vec![9, 9, 1, 5, 0];
    candidate(&named_board(5, 88, [TEAL, PURPLE], &names), 5, 0)
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// R262-A: a crop that is not yet confirmed as the member's own name may be
/// another player's. It is never on disk, not even between two writes.
#[test]
fn only_confirmed_crops_are_ever_written() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let file = seed_dir(data).join(SEED_FILE);
    let mut store = SeedStore::open(data, None);
    // two crops of the member's name wait: nothing is written at all
    for own in 0..2 {
        let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, own));
        assert_eq!(
            store.offer(candidate(&img, 5, own), &game("g1")),
            Offer::Pending
        );
        assert!(!data.join("private").exists(), "a waiting crop was written");
    }
    // and so does a crop of another name
    let other = waiting_crop();
    assert_eq!(store.offer(other.clone(), &game("g1")), Offer::Pending);
    assert!(!data.join("private").exists(), "a waiting crop was written");
    // the third crop of the member's name, from a second game, is trusted:
    // now the file exists and holds the trusted seeds and nothing else
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 2));
    assert_eq!(
        store.offer(candidate(&img, 5, 2), &game("g2")),
        Offer::Promoted
    );
    assert_eq!(
        store.pending_len(),
        1,
        "the other name still waits in memory"
    );
    let check = |store: &SeedStore| {
        let text = std::fs::read_to_string(&file).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["name_hash", "trusted", "version"]);
        assert_eq!(value["version"], SEED_FILE_VERSION);
        // every crop in the file is one of the trusted seeds
        let written: Vec<&str> = value["trusted"]
            .as_array()
            .unwrap()
            .iter()
            .map(|seed| seed["px"].as_str().unwrap())
            .collect();
        let trusted: Vec<String> = store.trusted().iter().map(|s| hex_of(&s.px)).collect();
        assert_eq!(written, trusted);
        // and the waiting crop is nowhere in the bytes: not its name line,
        // not the name area it was cut from
        assert!(!text.contains(&hex_of(&other.seed.px)));
        assert!(!text.contains(&hex_of(&other.area)));
        assert!(!text.contains("pending") && !text.contains("area"));
        text
    };
    let first = check(&store);
    // another waiting crop does not touch the file
    let img = named_board(6, 75, [TEAL, PURPLE], &roster(6, 0));
    assert_eq!(
        store.offer(candidate(&img, 6, 0), &game("g2")),
        Offer::Pending
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), first);
    // a later write (seeds of another table trusted) still leaves it out
    fill(&mut store, 6, 75);
    assert_eq!(store.trusted().len(), 2 * PROMOTE);
    assert_eq!(store.pending_len(), 1);
    assert_ne!(check(&store), first);
    // a restart: the trusted seeds are back, the waiting crop is not
    let restarted = SeedStore::open(data, None);
    assert_eq!(restarted.trusted(), store.trusted());
    assert_eq!(restarted.pending_len(), 0);
    // a store that cannot write keeps working from memory
    let mut memory = SeedStore::in_memory(None);
    fill(&mut memory, 5, 88);
    assert_eq!(memory.trusted().len(), PROMOTE);
}

/// R262-A: an earlier draft of the file (version 1) held the waiting crops
/// too. Such a file is deleted on open, trusted seeds and all: nothing of it
/// is loaded and nothing of it stays on disk. No release ever wrote one, so
/// there is nothing worth converting, and deleting cannot leave a waiting
/// crop behind the way a failed rewrite could.
#[test]
fn a_file_that_holds_waiting_crops_is_deleted_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let file = seed_dir(data).join(SEED_FILE);
    let mut store = SeedStore::open(data, None);
    fill(&mut store, 5, 88);
    let good = std::fs::read_to_string(&file).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    let other = waiting_crop();
    let waiting = serde_json::json!([{
        "seed": serde_json::to_value(other.seed()).unwrap(),
        "area_w": other.area_w,
        "area_h": other.area_h,
        "area": hex_of(&other.area),
        "game": "g1",
    }]);
    // the old format: version 1 with a `pending` list
    value["version"] = serde_json::json!(1);
    value["pending"] = waiting.clone();
    std::fs::write(&file, value.to_string()).unwrap();
    let opened = SeedStore::open(data, None);
    assert!(opened.trusted().is_empty() && opened.pending_len() == 0);
    assert!(
        !file.exists(),
        "the old file, waiting crops included, is gone"
    );
    // the current version with a `pending` list smuggled in: not a seed file
    value["version"] = serde_json::json!(SEED_FILE_VERSION);
    std::fs::write(&file, value.to_string()).unwrap();
    let opened = SeedStore::open(data, None);
    assert!(opened.trusted().is_empty() && opened.pending_len() == 0);
    assert!(!file.exists());
    // the file as this version writes it loads
    std::fs::write(&file, &good).unwrap();
    assert_eq!(SeedStore::open(data, None).trusted(), store.trusted());
    assert!(file.is_file());
}

#[cfg(unix)]
#[test]
fn a_loose_private_folder_is_tightened_on_the_next_write() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let folder = seed_dir(data);
    std::fs::create_dir_all(&folder).unwrap();
    for path in [data.join("private"), folder.clone()] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut store = SeedStore::open(data, None);
    fill(&mut store, 5, 88);
    assert_eq!(mode(&data.join("private")), 0o700);
    assert_eq!(mode(&folder), 0o700);
    assert_eq!(mode(&folder.join(SEED_FILE)), 0o600);
}

#[test]
fn another_configured_name_clears_the_seeds_and_an_unknown_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let file = seed_dir(data).join(SEED_FILE);
    let mut store = SeedStore::open(data, Some("First Name"));
    fill(&mut store, 5, 88);
    let seeds = store.trusted().to_vec();
    let bytes = std::fs::read(&file).unwrap();
    // F262-2: the name is not known in this run (not set locally, the fetch
    // failed, blank). That is not a changed name: the seeds stay and are
    // used, for as many runs as it lasts, and a write keeps the binding.
    for unknown in [None, Some(""), Some("   ")] {
        let mut opened = SeedStore::open(data, unknown);
        assert_eq!(opened.trusted(), seeds, "{unknown:?}");
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "{unknown:?}");
        fill(&mut opened, 6, 75);
        assert_eq!(opened.trusted().len(), 2 * PROMOTE);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(&name_hash(Some("First Name"))), "{unknown:?}");
        // back to the first name's file for the next case
        std::fs::write(&file, &bytes).unwrap();
    }
    // the same name again: kept
    assert_eq!(SeedStore::open(data, Some("first name")).trusted(), seeds);
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    // another name: cleared, once
    let other = SeedStore::open(data, Some("Second Name"));
    assert!(other.trusted().is_empty() && other.pending_len() == 0);
    assert!(!file.exists(), "the old seeds are deleted, not kept");
    // ...and the first name does not get them back
    assert!(
        SeedStore::open(data, Some("First Name"))
            .trusted()
            .is_empty()
    );

    // Learned with no name known, then a name is set for the first time:
    // kept, and bound to that name from then on.
    let mut store = SeedStore::open(data, None);
    fill(&mut store, 5, 88);
    let seeds = store.trusted().to_vec();
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains(r#""name_hash":"""#)
    );
    let bound = SeedStore::open(data, Some("First Name"));
    assert_eq!(bound.trusted(), seeds);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(&name_hash(Some("First Name"))));
    // a run with no name keeps that binding; another name then clears
    assert_eq!(SeedStore::open(data, None).trusted(), seeds);
    assert!(
        SeedStore::open(data, Some("Second Name"))
            .trusted()
            .is_empty()
    );
    assert!(!file.exists());
}

#[test]
fn a_file_that_is_not_this_versions_seed_file_is_deleted_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let folder = seed_dir(data);
    let file = folder.join(SEED_FILE);
    let mut store = SeedStore::open(data, None);
    fill(&mut store, 5, 88);
    let good = std::fs::read_to_string(&file).unwrap();
    let empty = |opened: SeedStore| opened.trusted().is_empty() && opened.pending_len() == 0;
    // another version
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["version"] = serde_json::json!(SEED_FILE_VERSION + 1);
    std::fs::write(&file, value.to_string()).unwrap();
    assert!(empty(SeedStore::open(data, None)));
    assert!(!file.exists());
    // not a seed file
    std::fs::write(&file, b"not json").unwrap();
    assert!(empty(SeedStore::open(data, None)));
    assert!(!file.exists());
    // not text at all (the read itself fails)
    std::fs::write(&file, [0xff, 0xfe, 0x00, 0xff]).unwrap();
    assert!(empty(SeedStore::open(data, None)));
    assert!(!file.exists());
    // larger than any file this module writes (F262-15): not read
    let mut big = good.clone().into_bytes();
    big.resize(MAX_SEED_FILE as usize + 1, b' ');
    std::fs::write(&file, &big).unwrap();
    assert!(empty(SeedStore::open(data, None)));
    assert!(!file.exists());
    // a seed whose sizes do not fit its pixels or a board is not loaded
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["trusted"][0]["w"] = serde_json::json!(1);
    value["trusted"][1]["row_h"] = serde_json::json!(5000);
    std::fs::write(&file, value.to_string()).unwrap();
    assert_eq!(SeedStore::open(data, None).trusted().len(), PROMOTE - 2);
    // more seeds than the cap: the oldest are not loaded
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    let one = value["trusted"][0].clone();
    let many = vec![one; 3 * TRUSTED_PER_SCALE];
    value["trusted"] = serde_json::json!(many);
    std::fs::write(&file, value.to_string()).unwrap();
    assert_eq!(
        SeedStore::open(data, None).trusted().len(),
        TRUSTED_PER_SCALE
    );
    // a temp file left by a crash is removed
    std::fs::write(&file, &good).unwrap();
    std::fs::write(folder.join(SEED_TMP), b"half a file").unwrap();
    assert_eq!(SeedStore::open(data, None).trusted(), store.trusted());
    assert!(!folder.join(SEED_TMP).exists());
    // a link where the file should be is not followed: the link goes, its
    // target stays
    #[cfg(unix)]
    {
        let target = dir.path().join("elsewhere.json");
        std::fs::write(&target, &good).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();
        assert!(empty(SeedStore::open(data, None)));
        assert!(
            std::fs::symlink_metadata(&file).is_err(),
            "the link is gone"
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), good);
    }
}

/// F262-3: two session ids are not two games. A stat split inside one
/// match opens a new session on the same map a moment later.
#[test]
fn two_sessions_of_one_match_are_one_game() {
    let tag = |session: &str, map: &str, at: u64| GameTag {
        session: session.into(),
        map: map.into(),
        at,
    };
    let a = tag("s1", "Busan", 1000);
    assert!(!a.other_game(&a));
    // the same session is one game whatever else differs
    assert!(!a.other_game(&tag("s1", "Ilios", 1000 + 2 * GAME_GAP_SECS)));
    // a split: another session, the same map, minutes later
    assert!(!a.other_game(&tag("s2", "Busan", 1300)));
    assert!(!a.other_game(&tag("s2", " Busan ", 1000 + GAME_GAP_SECS - 1)));
    // another map: another game
    assert!(a.other_game(&tag("s2", "Ilios", 1300)));
    assert!(tag("s2", "Ilios", 1300).other_game(&a));
    // the same map again after longer than a match runs: another game
    assert!(a.other_game(&tag("s2", "Busan", 1000 + GAME_GAP_SECS)));
    // no map known: only the time can tell
    assert!(!a.other_game(&tag("s2", "", 1300)));
    assert!(!tag("s1", "", 1000).other_game(&tag("s2", "", 1300)));
    assert!(tag("s1", "", 1000).other_game(&tag("s2", "", 1000 + GAME_GAP_SECS)));

    // through the store: three matching crops over a split game wait; a
    // crop from a real second game completes them
    let mut store = SeedStore::in_memory(None);
    let crop = |own: usize| candidate(&named_board(5, 88, [TEAL, PURPLE], &roster(5, own)), 5, own);
    assert_eq!(
        store.offer(crop(0), &tag("s1", "Busan", 1000)),
        Offer::Pending
    );
    assert_eq!(
        store.offer(crop(1), &tag("s2", "Busan", 1200)),
        Offer::Pending
    );
    assert_eq!(
        store.offer(crop(2), &tag("s3", "Busan", 1400)),
        Offer::Pending
    );
    assert!(store.trusted().is_empty());
    assert_eq!(
        store.offer(crop(3), &tag("s4", "Ilios", 1900)),
        Offer::Promoted
    );
}

/// F262-3: a trusted seed must not be able to hide the member's real row
/// for good. Seeds that name another row than a strong highlight
/// DISAGREE_DROPS boards in a row are dropped.
#[test]
fn seeds_that_keep_contradicting_a_strong_highlight_are_dropped() {
    assert_eq!(DISAGREE_DROPS, 3);
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let file = seed_dir(data).join(SEED_FILE);
    let mut store = SeedStore::open(data, None);
    fill(&mut store, 5, 88);
    fill(&mut store, 6, 75);
    assert_eq!(store.trusted().len(), 2 * PROMOTE);
    // two in a row, then a board where seed and highlight agree: kept
    assert_eq!(store.note(Verdict::Disagree(88), "s"), Noted::default());
    assert_eq!(store.note(Verdict::Disagree(89), "s"), Noted::default());
    assert_eq!(store.note(Verdict::Agree(88), "s"), Noted::default());
    assert_eq!(store.note(Verdict::Disagree(88), "s"), Noted::default());
    assert_eq!(store.note(Verdict::Disagree(88), "s"), Noted::default());
    // a silent board, or one of the other table, is not "in between"
    assert_eq!(store.note(Verdict::Silent, "s"), Noted::default());
    assert_eq!(store.note(Verdict::Agree(75), "s"), Noted::default());
    assert_eq!(store.trusted().len(), 2 * PROMOTE);
    // the third in a row: the seeds of that scale go, the others stay
    let noted = store.note(Verdict::Disagree(88), "s");
    assert_eq!(noted.dropped, PROMOTE);
    assert_eq!(store.trusted().len(), PROMOTE);
    assert!(store.trusted().iter().all(|seed| seed.row_h == 75));
    // off the disk too, and the count starts again
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(SeedStore::open(data, None).trusted(), store.trusted());
    assert!(!text.is_empty());
    assert_eq!(store.note(Verdict::Disagree(88), "s"), Noted::default());
    // the last scale goes the same way, and then the file itself
    for _ in 0..DISAGREE_DROPS - 1 {
        assert_eq!(store.note(Verdict::Disagree(75), "s").dropped, 0);
    }
    assert_eq!(store.note(Verdict::Disagree(75), "s").dropped, PROMOTE);
    assert!(store.trusted().is_empty());
    assert!(!file.exists());
    // the member's name is learned again from the highlight
    fill(&mut store, 5, 88);
    assert_eq!(store.trusted().len(), PROMOTE);
}

#[test]
fn games_without_a_trusted_seed_are_reported_once() {
    let mut store = SeedStore::in_memory(None);
    let mut notices = 0;
    for game in 0..3 * UNSEEDED_NOTICE_GAMES {
        // several boards per game: a game counts once
        for _ in 0..3 {
            let noted = store.note(Verdict::Silent, &format!("s{game}"));
            if noted.unseeded_notice {
                notices += 1;
                assert_eq!(game + 1, UNSEEDED_NOTICE_GAMES, "on the fifth game");
            }
        }
    }
    assert_eq!(notices, 1);
    // with a trusted seed there is nothing to report
    let mut store = seeded(5, 88);
    for game in 0..3 * UNSEEDED_NOTICE_GAMES {
        assert!(
            !store
                .note(Verdict::Silent, &format!("s{game}"))
                .unseeded_notice
        );
    }
}

/// F262-8: a word at the cell's left edge may be the tail of a name that
/// started further left. It is never cut, so it can never become a seed
/// that matches every name ending the same way.
#[test]
fn a_name_cut_by_the_cells_left_edge_gives_no_candidate() {
    let blank = board(&[1; 10], 5, 88, [TEAL, PURPLE]);
    // (A row with no ult disc, where a name could start far left of the
    // cell, never seeds at all: the gate needs the career panel.)
    for x in [1.95, 2.00, 2.05] {
        let mut img = blank.clone();
        draw_name(&mut img, 5, 88, 0, &OWN, At { x, y: NAME_Y });
        assert!(measure(&img, 5).candidate(0).is_none(), "name at {x}");
    }
    // clear of the edge it is cut whole
    for x in [2.12, 2.20, 2.30] {
        let mut img = blank.clone();
        draw_name(&mut img, 5, 88, 0, &OWN, At { x, y: NAME_Y });
        let c = measure(&img, 5).candidate(0).expect("candidate");
        assert_eq!(c.seed().w, 5 * 22 + 18, "name at {x}");
    }
}

#[test]
fn zncc_is_zero_for_a_flat_block() {
    let ramp: Vec<u8> = (0..64u8).map(|v| v * 4).collect();
    let score = |a: &[u8], b: &[u8]| zncc(a.iter().copied(), b.iter().copied());
    assert!((score(&ramp, &ramp) - 1.0).abs() < 1e-9);
    let inverted: Vec<u8> = ramp.iter().map(|v| 255 - v).collect();
    assert!((score(&ramp, &inverted) + 1.0).abs() < 1e-9);
    // a flat block carries no shape: it matches nothing, itself included
    assert_eq!(score(&ramp, &[0; 64]), 0.0);
    assert_eq!(score(&[255; 64], &ramp), 0.0);
    assert_eq!(score(&[7; 64], &[7; 64]), 0.0);
}

/// The chunks cover the whole crop: a change in its last columns, right of
/// the last regular chunk, is still seen.
#[test]
fn the_weakest_chunk_reaches_the_end_of_the_crop() {
    // 88 px rows: chunks are 26 px wide, 13 px apart. A 46 px crop has
    // regular chunks at 0 and 13 (to column 39) and a tail chunk to 46.
    let (w, h) = (46usize, 12usize);
    let px: Vec<u8> = (0..w * h)
        .map(|i| {
            if (i % w / 3 + i / w / 3) % 2 == 0 {
                255
            } else {
                0
            }
        })
        .collect();
    let seed = Seed {
        row_h: 88,
        w: w as u32,
        h: h as u32,
        px: px.clone(),
        left_blank: false,
    };
    let tpl = Tpl::of(&seed).unwrap();
    let same = Plane::new(w, h, px.clone());
    assert!((weakest_chunk(&tpl, &same, 88, 0, 0) - 1.0).abs() < 1e-9);
    // invert the last 6 columns only
    let mut changed = px;
    for y in 0..h {
        for x in 40..w {
            changed[y * w + x] = 255 - changed[y * w + x];
        }
    }
    let plane = Plane::new(w, h, changed);
    assert!(weakest_chunk(&tpl, &plane, 88, 0, 0) < 0.8);
}

/// Two crops are one name only when each matches on the other's row.
#[test]
fn crops_agree_only_when_each_matches_on_the_others_row() {
    let blank = board(&[1; 10], 5, 88, [TEAL, PURPLE]);
    // a: the member's name with a thin stroke just left of it (a badge
    // edge), so its left side is not blank
    let mut img = blank.clone();
    let at = At {
        x: NAME_X + GLYPH_STEP,
        y: NAME_Y,
    };
    draw_name(&mut img, 5, 88, 0, &OWN, at);
    let x0 = 44 + ((NAME_X + GLYPH_STEP) * 88.0).round() as u32;
    let mid = row_top(5, 88, 0) + (NAME_Y * 88.0).round() as u32 + 14;
    for y in mid..mid + 3 {
        for x in x0 - 9..x0 - 1 {
            img.put_pixel(x, y, Rgb([255; 3]));
        }
    }
    let a = candidate(&img, 5, 0);
    assert!(!a.seed().left_blank);
    // b: the same name with one more glyph in front
    let mut img = blank;
    let longer: Vec<usize> = [0].into_iter().chain(OWN).collect();
    draw_name(&mut img, 5, 88, 0, &longer, HOME);
    let b = candidate(&img, 5, 0);
    // a's crop sits inside b's name line; b's does not sit on a's row
    assert!(seed_on(a.seed(), &b) >= AGREE);
    assert!(seed_on(b.seed(), &a) < AGREE);
    assert!(!agree(&a, &b));
    assert!(!agree(&b, &a));
    assert!(agree(&a, &a) && agree(&b, &b));
}

#[cfg(unix)]
#[test]
fn a_symlinked_private_folder_is_never_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, data.join("private")).unwrap();
    let mut store = SeedStore::open(&data, None);
    fill(&mut store, 5, 88);
    // the seeds work from memory, and nothing landed behind the link
    assert_eq!(store.trusted().len(), PROMOTE);
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn the_name_hash_is_stable_and_never_the_name() {
    let a = name_hash(Some("Made Up Name"));
    assert_eq!(a.len(), 64);
    assert_eq!(a, name_hash(Some("  made up name ")));
    assert_ne!(a, name_hash(Some("Made Up Nam")));
    assert_eq!(name_hash(None), name_hash(Some("  ")));
    assert!(!a.contains("made"));
    assert_eq!(game_key("s1").len(), 16);
    assert_ne!(game_key("s1"), game_key("s2"));
}

#[test]
fn a_seed_never_prints_its_pixels() {
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 0));
    let c = candidate(&img, 5, 0);
    let shown = format!("{c:?} {:?}", c.seed());
    assert!(shown.len() < 80, "{shown}");
    assert!(!shown.contains('['), "{shown}");
}

#[test]
fn seed_pixels_round_trip_through_the_file_format() {
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 0));
    let c = candidate(&img, 5, 0);
    let text = serde_json::to_string(c.seed()).unwrap();
    assert_eq!(&serde_json::from_str::<Seed>(&text).unwrap(), c.seed());
    // pixels are hex text, not a number per pixel
    assert!(text.contains(&hex_of(&c.seed.px)));
    assert!(text.len() < 3 * c.seed.px.len());
    assert!(
        serde_json::from_str::<Seed>(r#"{"row_h":88,"w":1,"h":1,"px":"zz","left_blank":true}"#)
            .is_err()
    );
    assert!(
        serde_json::from_str::<Seed>(r#"{"row_h":88,"w":1,"h":1,"px":"abc","left_blank":true}"#)
            .is_err()
    );
    // a seed whose size does not fit its pixels is never compared
    let mut bad = c.seed().clone();
    bad.w += 1;
    assert!(measure(&img, 5).scores(&[bad]).iter().all(|&s| s < 0.0));
}
