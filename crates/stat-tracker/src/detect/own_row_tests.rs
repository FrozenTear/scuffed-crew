//! Tests for [`super`] (`detect::own_row`). Synthetic boards and made-up
//! glyphs only: no captured pixels and no real name.
use super::*;
use crate::shadow::heroes::test_fixtures::{PURPLE, TEAL, YELLOW, board};
use crate::shadow::heroes::{
    HeroBoard, HeroFrameRead, HeroPack, HeroRead, PANEL_MIN_SCORE, PanelRead, RowClass,
};
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
    let named = OwnRow::from_name(6, false);
    assert_eq!(
        named,
        own(Some(6), Trust::Unconfirmed, Source::NameFallback)
    );
    assert!(!named.suspect());
    // ...unless a seed for the board exists and matched no row: then the
    // name is one voice where two could have spoken, and is marked (F262-18)
    let named = OwnRow::from_name(6, true);
    assert_eq!(named, own(Some(6), Trust::Suspect, Source::NameFallback));
    assert!(named.suspect());
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
    read_with(img, team, highlight, None, seeds)
}

/// [`read`] on a board whose hero icons were read.
fn read_with(
    img: &RgbImage,
    team: usize,
    highlight: Option<(usize, f64)>,
    heroes: Option<&TabHeroes>,
    seeds: &[Seed],
) -> OwnRowRead {
    read_own_row(
        None,
        &DynamicImage::ImageRgb8(img.clone()),
        team,
        highlight.map(|(row, lead)| Highlight { row, lead }),
        heroes,
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
    // a strong highlight on another row than the member's name: held. With
    // no career hero on the highlighted row the board is no strike against
    // the seeds (F262-17; the proved case is its own test)
    let split = read(&img, 5, Some((0, 1.5)), store.trusted());
    assert_eq!((split.own.row, split.own.source), (None, Source::Disagree));
    assert_eq!(split.verdict, Verdict::Silent);
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
    let live = icons(&TEAM, &ENEMY, Some("juno"));
    // a clear highlight with the career hero on it and no seed: sure, and a
    // candidate from that row
    let c = read_with(&img, 5, Some((2, SEED_HIGHLIGHT_LEAD)), Some(&live), &[])
        .candidate
        .expect("candidate");
    assert_eq!(c.row, 2);
    assert!(c.row_bottom > row_top(5, 88, 2));
    // a clear highlight alone (no career panel): not sure, no candidate
    assert!(
        read(&img, 5, Some((2, SEED_HIGHLIGHT_LEAD)), &[])
            .candidate
            .is_none()
    );
    // two rows could be lit (a hover, a party tint): no candidate with the
    // highlight's proof (the career hero's stricter proof is its own test)
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

// ------------------------------------------------- the career hero (O9b)

/// The member's team and the other team on a made-up live board.
const TEAM: [Option<&str>; 5] = [
    Some("ana"),
    Some("reinhardt"),
    Some("juno"),
    Some("tracer"),
    Some("sojourn"),
];
const ENEMY: [Option<&str>; 5] = [
    Some("mercy"),
    Some("winston"),
    Some("kiriko"),
    Some("genji"),
    Some("ashe"),
];

/// A Tab's icon reads: the member's team, the other team, the career panel.
fn icons(own: &[Option<&str>], enemy: &[Option<&str>], panel: Option<&str>) -> TabHeroes {
    let rows: Vec<Option<&str>> = own.iter().chain(enemy).copied().collect();
    tab(&rows, panel)
}

#[test]
fn the_career_hero_names_the_one_row_of_the_members_team_that_shows_it() {
    assert_eq!(
        career_hero_row(&icons(&TEAM, &ENEMY, Some("juno"))),
        Some(2)
    );
    assert_eq!(career_hero_row(&icons(&TEAM, &ENEMY, Some("ana"))), Some(0));
    assert_eq!(
        career_hero_row(&icons(&TEAM, &ENEMY, Some("sojourn"))),
        Some(4)
    );
    // An enemy on the same hero (a mirror) changes nothing: on a board with
    // a career panel the member is never in the lower table.
    let mut mirror = ENEMY;
    mirror[3] = Some("juno");
    assert_eq!(
        career_hero_row(&icons(&TEAM, &mirror, Some("juno"))),
        Some(2)
    );
    // ...so a hero only the other team plays names no row
    assert_eq!(career_hero_row(&icons(&TEAM, &ENEMY, Some("genji"))), None);
    // two rows of the member's team on that hero (no hero limit, or a
    // misread): no row
    let mut twice = TEAM;
    twice[4] = Some("juno");
    assert_eq!(career_hero_row(&icons(&twice, &ENEMY, Some("juno"))), None);
    // no career panel read: a replay, a post-match table
    assert_eq!(career_hero_row(&icons(&TEAM, &ENEMY, None)), None);
    // the member's own icon is unread (an ult ring, a skull): no other row
    // is taken in its place
    let mut unread = TEAM;
    unread[2] = None;
    assert_eq!(career_hero_row(&icons(&unread, &ENEMY, Some("juno"))), None);
    // a panel read under the panel gate is not a read
    let mut weak = icons(&TEAM, &ENEMY, Some("juno"));
    weak.panel.as_mut().unwrap().read.score = PANEL_MIN_SCORE - 0.01;
    assert_eq!(career_hero_row(&weak), None);
    // nor is a suspect icon read on the row
    let mut suspect = icons(&TEAM, &ENEMY, Some("juno"));
    suspect.read.as_mut().unwrap().board.rows[2].suspect = true;
    assert_eq!(career_hero_row(&suspect), None);
    // no icon read at all (no hero pack)
    let mut none = icons(&TEAM, &ENEMY, Some("juno"));
    none.read = None;
    assert_eq!(career_hero_row(&none), None);
    // 6v6: the member's team is rows 0 to 5
    let six = [
        Some("ana"),
        Some("reinhardt"),
        Some("zarya"),
        Some("tracer"),
        Some("sojourn"),
        Some("juno"),
    ];
    let other = [
        Some("juno"),
        Some("winston"),
        Some("dva"),
        Some("genji"),
        Some("ashe"),
        Some("mercy"),
    ];
    assert_eq!(career_hero_row(&icons(&six, &other, Some("juno"))), Some(5));
    assert_eq!(career_hero_row(&icons(&six, &other, Some("mercy"))), None);
}

#[test]
fn the_career_hero_speaks_only_where_seed_and_highlight_are_silent() {
    let own = |row, trust, source| OwnRow { row, trust, source };
    let (strong, weak) = (SEED_HIGHLIGHT_LEAD, SEED_HIGHLIGHT_LEAD - 0.01);
    // alone it names the row, marked, with or without seeds
    for seeded in [false, true] {
        let hero = combine_with_hero(None, None, Some(3), seeded);
        assert_eq!(hero, own(Some(3), Trust::Suspect, Source::CareerHero));
        assert!(hero.suspect());
    }
    // it confirms a seed that has no highlight beside it
    let sure = combine_with_hero(Some(3), None, Some(3), true);
    assert_eq!(sure, own(Some(3), Trust::Sure, Source::SeedAndHero));
    assert!(!sure.suspect());
    // ...and never overrules one
    assert_eq!(
        combine_with_hero(Some(3), None, Some(1), true),
        own(Some(3), Trust::Suspect, Source::SeedOnly)
    );
    // with a highlight on the board the answer is the one without a career
    // hero, whichever row the hero names
    for seed in [None, Some(0), Some(2)] {
        for lead in [strong, weak] {
            for row in [0, 2] {
                for seeded in [false, true] {
                    for hero in [None, Some(0), Some(2), Some(4)] {
                        let got = combine_with_hero(seed, lit(row, lead), hero, seeded);
                        // except that a strong highlight no seed matched is
                        // sure when the career hero is read on its row
                        if seed.is_none() && lead >= strong && hero == Some(row) {
                            assert_eq!(got, own(Some(row), Trust::Sure, Source::HighlightAndHero));
                            assert!(!got.suspect());
                            continue;
                        }
                        assert_eq!(
                            got,
                            combine(seed, lit(row, lead), seeded),
                            "seed {seed:?} highlight {row} at {lead} hero {hero:?}"
                        );
                    }
                }
            }
        }
    }
    // and with no career hero's row it is `combine`
    for seed in [None, Some(1)] {
        for seeded in [false, true] {
            assert_eq!(
                combine_with_hero(seed, None, None, seeded),
                combine(seed, None, seeded)
            );
        }
    }
    assert_eq!(Source::CareerHero.label(), "career_hero");
    assert_eq!(Source::SeedAndHero.label(), "seed_and_hero");
    assert_eq!(Source::HighlightAndHero.label(), "highlight_and_hero");
}

#[test]
fn a_board_without_a_highlight_takes_the_career_heros_row_and_may_seed_from_it() {
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 2));
    let live = icons(&TEAM, &ENEMY, Some("juno"));
    let weak_lead = SEED_HIGHLIGHT_LEAD - 0.01;
    // no seed, no highlight: the career hero's row, marked, and its name
    // line cut with the weaker proof and the hero it was seen on
    let by_hero = read_with(&img, 5, None, Some(&live), &[]);
    assert_eq!(
        (by_hero.own.row, by_hero.own.trust, by_hero.own.source),
        (Some(2), Trust::Suspect, Source::CareerHero)
    );
    assert_eq!((by_hero.hero_row, by_hero.seeded), (Some(2), false));
    assert_eq!(by_hero.verdict, Verdict::Silent);
    let c = by_hero.candidate.expect("candidate");
    assert_eq!((c.row, c.candidate.proof()), (2, Proof::CareerHero));
    assert_eq!(c.candidate.hero.as_deref(), Some("juno"));
    assert!(c.row_bottom > row_top(5, 88, 2));
    // a strong highlight on the career hero's row: sure, and cut with the
    // highlight's proof
    let strong = read_with(&img, 5, Some((2, SEED_HIGHLIGHT_LEAD)), Some(&live), &[]);
    assert_eq!(
        (strong.own.row, strong.own.trust, strong.own.source),
        (Some(2), Trust::Sure, Source::HighlightAndHero)
    );
    assert_eq!(strong.verdict, Verdict::Silent);
    let lit_c = strong.candidate.expect("candidate").candidate;
    assert_eq!(
        (lit_c.proof(), lit_c.hero.as_deref()),
        (Proof::Highlight, Some("juno"))
    );
    assert!(seed_gate(
        c.row,
        c.row_bottom,
        1007,
        Chat::None,
        Some(&live)
    ));
    assert!(!seed_gate(
        c.row,
        c.row_bottom,
        1007,
        Chat::Unknown,
        Some(&live)
    ));
    // no career panel: nothing names a row, nothing is cut
    let none = read_with(&img, 5, None, Some(&icons(&TEAM, &ENEMY, None)), &[]);
    assert_eq!((none.own.row, none.own.source), (None, Source::Neither));
    assert!(none.candidate.is_none() && none.hero_row.is_none());
    // a weak highlight on that same row: the highlight's row as before
    // (marked), and its crop may seed, with the career hero's proof
    let weak = read_with(&img, 5, Some((2, weak_lead)), Some(&live), &[]);
    assert_eq!(
        (weak.own.row, weak.own.trust, weak.own.source),
        (Some(2), Trust::Suspect, Source::HighlightOnly)
    );
    assert_eq!(
        weak.candidate.expect("crop").candidate.proof(),
        Proof::CareerHero
    );
    // a weak highlight on another row: as before, and no crop from either
    let other = read_with(&img, 5, Some((0, weak_lead)), Some(&live), &[]);
    assert_eq!(
        (other.own.row, other.own.trust, other.own.source),
        (Some(0), Trust::Suspect, Source::HighlightOnly)
    );
    assert!(other.candidate.is_none());
    // a strong highlight with the career hero on another row: marked, no crop
    let elsewhere = read_with(&img, 5, Some((0, SEED_HIGHLIGHT_LEAD)), Some(&live), &[]);
    assert_eq!(
        (elsewhere.own.row, elsewhere.own.trust),
        (Some(0), Trust::Unconfirmed)
    );
    assert!(elsewhere.candidate.is_none());
    // with a trusted seed: seed and career hero on one row is sure
    let store = seeded(5, 88);
    let sure = read_with(&img, 5, None, Some(&live), store.trusted());
    assert_eq!(
        (sure.own.row, sure.own.trust, sure.own.source),
        (Some(2), Trust::Sure, Source::SeedAndHero)
    );
    assert_eq!(sure.verdict, Verdict::Agree(88));
    assert!(sure.candidate.is_none() && sure.seeded);
    // the member's own icon is unread (dead, an ult ring): the career hero
    // names no row, and the seed still finds the member, marked
    let mut unread = TEAM;
    unread[2] = None;
    let dead = icons(&unread, &ENEMY, Some("juno"));
    let alone = read_with(&img, 5, None, Some(&dead), store.trusted());
    assert_eq!(
        (alone.own.row, alone.own.trust, alone.own.source),
        (Some(2), Trust::Suspect, Source::SeedOnly)
    );
    assert_eq!(alone.verdict, Verdict::Silent);
    // seeds exist but match no row (another account name): the career
    // hero's row, marked, and the new name line is cut
    let renamed = named_board(5, 88, [TEAL, PURPLE], &other_names(10));
    let new_name = read_with(&renamed, 5, None, Some(&live), store.trusted());
    assert_eq!(
        (new_name.own.row, new_name.own.source, new_name.seeded),
        (Some(2), Source::CareerHero, true)
    );
    assert_eq!(
        new_name.candidate.expect("crop").candidate.proof(),
        Proof::CareerHero
    );
    // the same account with a strong highlight on the career hero's row:
    // sure, and its name line is cut, so another of the member's accounts
    // is learned as its own group
    let lit_new = read_with(
        &renamed,
        5,
        Some((2, SEED_HIGHLIGHT_LEAD)),
        Some(&live),
        store.trusted(),
    );
    assert_eq!(
        (lit_new.own.row, lit_new.own.trust, lit_new.own.source),
        (Some(2), Trust::Sure, Source::HighlightAndHero)
    );
    assert_eq!(
        lit_new.candidate.expect("crop").candidate.proof(),
        Proof::Highlight
    );
    // a board that cannot be measured: the career hero's row, no crop
    let blank = RgbImage::from_pixel(600, 400, Rgb([10, 10, 10]));
    let unmeasured = read_with(&blank, 5, None, Some(&live), &[]);
    assert_eq!(
        (unmeasured.own.row, unmeasured.own.source),
        (Some(2), Source::CareerHero)
    );
    assert!(unmeasured.candidate.is_none());
    assert_eq!(unmeasured.hero_row, Some(2));
}

/// F262-17: a strike against trusted seeds needs the proof a seed needs,
/// the career hero read on the row the seed contradicts. A strong highlight
/// alone (a replay, a spectated board: no career panel there) is no strike.
#[test]
fn a_strike_against_the_seeds_needs_the_career_hero_on_the_other_row() {
    let store = seeded(5, 88);
    // the member's name on row 2, a strong highlight on row 0
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 2));
    let split = |heroes: Option<&TabHeroes>| {
        let read = read_with(&img, 5, Some((0, 1.5)), heroes, store.trusted());
        assert_eq!((read.own.row, read.own.source), (None, Source::Disagree));
        read.verdict
    };
    // no icon read, or no career panel: held, and no strike
    assert_eq!(split(None), Verdict::Silent);
    assert_eq!(split(Some(&icons(&TEAM, &ENEMY, None))), Verdict::Silent);
    // the career hero is on the highlighted row: a strike
    assert_eq!(
        split(Some(&icons(&TEAM, &ENEMY, Some("ana")))),
        Verdict::Disagree(88)
    );
    // the career hero is on the seed's row (the seed is right), on no row,
    // or the highlighted row's icon is unread: no strike
    assert_eq!(
        split(Some(&icons(&TEAM, &ENEMY, Some("juno")))),
        Verdict::Silent
    );
    assert_eq!(
        split(Some(&icons(&TEAM, &ENEMY, Some("genji")))),
        Verdict::Silent
    );
    let mut unread = TEAM;
    unread[0] = None;
    assert_eq!(
        split(Some(&icons(&unread, &ENEMY, Some("ana")))),
        Verdict::Silent
    );
    // no highlight: the seed's row is used, marked, and a career hero on
    // another row of the team is a strike
    let ana = icons(&TEAM, &ENEMY, Some("ana"));
    let lone = read_with(&img, 5, None, Some(&ana), store.trusted());
    assert_eq!(
        (lone.own.row, lone.own.trust, lone.own.source),
        (Some(2), Trust::Suspect, Source::SeedOnly)
    );
    assert_eq!(lone.verdict, Verdict::Disagree(88));
    // a weak highlight is never a strike, career hero or not
    let over = read_with(&img, 5, Some((0, 1.1)), Some(&ana), store.trusted());
    assert_eq!(over.own.source, Source::SeedOverWeakHighlight);
    assert_eq!(over.verdict, Verdict::Silent);

    // Through the store: strong highlights without that proof never drop
    // the seeds, however many; with it the third board in a row does.
    let mut store = seeded(5, 88);
    for _ in 0..3 * DISAGREE_DROPS {
        let verdict = read_with(&img, 5, Some((0, 1.5)), None, store.trusted()).verdict;
        assert_eq!(store.note(verdict, "s").dropped, 0);
    }
    assert_eq!(store.trusted().len(), PROMOTE);
    for board in 1..=DISAGREE_DROPS {
        let verdict = read_with(&img, 5, Some((0, 1.5)), Some(&ana), store.trusted()).verdict;
        let dropped = store.note(verdict, "s").dropped;
        assert_eq!(dropped, if board == DISAGREE_DROPS { PROMOTE } else { 0 });
    }
    assert!(store.trusted().is_empty());
}

/// The member's name line from a board with the member on `own`, its row
/// shown by the career hero `hero` alone.
fn hero_crop(own: usize, hero: &str) -> Candidate {
    candidate(&named_board(5, 88, [TEAL, PURPLE], &roster(5, own)), 5, own)
        .proved(Proof::CareerHero, Some(hero))
}

/// The same from a strongly highlighted row.
fn lit_crop(own: usize, hero: &str) -> Candidate {
    candidate(&named_board(5, 88, [TEAL, PURPLE], &roster(5, own)), 5, own)
        .proved(Proof::Highlight, Some(hero))
}

/// Offer career-hero crops of the member's name, one per `(session, hero)`.
fn offer_hero_crops(boards: &[(&str, &str)]) -> (Vec<Offer>, SeedStore) {
    let mut store = SeedStore::in_memory(None);
    let offers = boards
        .iter()
        .enumerate()
        .map(|(i, (session, hero))| store.offer(hero_crop(i % 5, hero), &game(session)))
        .collect();
    (offers, store)
}

#[test]
fn career_hero_crops_need_four_boards_three_games_and_a_hero_swap() {
    assert_eq!(
        (HERO_SEED_BOARDS, HERO_SEED_GAMES, HERO_SEED_HEROES),
        (4, 3, 2)
    );
    assert_eq!((SEED_BOARDS, SEED_GAMES), (3, 2));
    let waiting = |boards: &[(&str, &str)]| {
        let (offers, store) = offer_hero_crops(boards);
        assert!(offers.iter().all(|o| *o == Offer::Pending), "{offers:?}");
        assert!(store.trusted().is_empty());
        assert_eq!(store.pending_len(), boards.len());
    };
    // what is enough for highlighted rows (three boards, two games) is not
    waiting(&[("g1", "juno"), ("g1", "juno"), ("g2", "ana")]);
    // four boards, three games, one hero: no hero swap
    waiting(&[
        ("g1", "juno"),
        ("g2", "juno"),
        ("g3", "juno"),
        ("g3", "juno"),
    ]);
    // ...however long a one-hero member plays
    waiting(&[
        ("g1", "juno"),
        ("g2", "juno"),
        ("g3", "juno"),
        ("g4", "juno"),
        ("g5", "juno"),
        ("g6", "juno"),
        ("g7", "juno"),
        ("g8", "juno"),
    ]);
    // four boards, two heroes, two games
    waiting(&[("g1", "juno"), ("g1", "ana"), ("g2", "juno"), ("g2", "ana")]);
    // three boards, three games, two heroes
    waiting(&[("g1", "juno"), ("g2", "ana"), ("g3", "juno")]);
    // four boards, three games, two heroes: trusted on the fourth
    let (offers, store) = offer_hero_crops(&[
        ("g1", "juno"),
        ("g2", "juno"),
        ("g3", "juno"),
        ("g3", "ana"),
    ]);
    assert_eq!(
        offers,
        [
            Offer::Pending,
            Offer::Pending,
            Offer::Pending,
            Offer::Promoted
        ]
    );
    assert_eq!(store.trusted().len(), PROMOTE);
    assert_eq!(store.pending_len(), 0);
    // the swap and the third game may come in any order
    let (offers, store) = offer_hero_crops(&[
        ("g1", "ana"),
        ("g1", "juno"),
        ("g2", "juno"),
        ("g3", "juno"),
    ]);
    assert_eq!(offers.last(), Some(&Offer::Promoted));
    assert_eq!(store.trusted().len(), PROMOTE);
    // the trusted seeds find the member on a new board
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 7));
    assert_eq!(seed_row(&measure(&img, 5).scores(store.trusted())), Some(7));
    // a split session is not a third game (the map and the minutes say so)
    let tag = |session: &str, map: &str, at: u64| GameTag {
        session: session.into(),
        map: map.into(),
        at,
    };
    let mut store = SeedStore::in_memory(None);
    for (i, (game, hero)) in [
        (tag("s1", "Busan", 1000), "juno"),
        (tag("s2", "Ilios", 2000), "juno"),
        (tag("s3", "Ilios", 2300), "ana"),
        (tag("s4", "Ilios", 2600), "ana"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(store.offer(hero_crop(i, hero), &game), Offer::Pending);
    }
    assert_eq!(
        store.offer(hero_crop(5, "ana"), &tag("s5", "Oasis", 2900)),
        Offer::Promoted
    );
}

#[test]
fn highlighted_and_career_hero_crops_share_one_waiting_list() {
    // The highlighted rows' rule is what it was, career-hero crops or not:
    // three highlighted crops from two games.
    let mut store = SeedStore::in_memory(None);
    assert_eq!(
        store.offer(hero_crop(0, "juno"), &game("g1")),
        Offer::Pending
    );
    assert_eq!(
        store.offer(lit_crop(1, "juno"), &game("g1")),
        Offer::Pending
    );
    assert_eq!(
        store.offer(lit_crop(2, "juno"), &game("g1")),
        Offer::Pending
    );
    // four crops, two of them highlighted, two games, one hero: neither
    // rule is met (a career-hero crop does not count as a highlighted one)
    assert_eq!(
        store.offer(hero_crop(3, "juno"), &game("g2")),
        Offer::Pending
    );
    assert!(store.trusted().is_empty());
    // the third highlighted crop, in the second game
    assert_eq!(
        store.offer(lit_crop(4, "juno"), &game("g2")),
        Offer::Promoted
    );
    assert_eq!(store.trusted().len(), PROMOTE);

    // A highlighted crop is a career-hero crop with a second proof, so it
    // counts toward the career hero's rule: two and two, three games, two
    // heroes.
    let mut store = SeedStore::in_memory(None);
    assert_eq!(
        store.offer(lit_crop(0, "juno"), &game("g1")),
        Offer::Pending
    );
    assert_eq!(
        store.offer(lit_crop(1, "juno"), &game("g2")),
        Offer::Pending
    );
    assert_eq!(
        store.offer(hero_crop(2, "ana"), &game("g3")),
        Offer::Pending
    );
    assert_eq!(
        store.offer(hero_crop(3, "ana"), &game("g3")),
        Offer::Promoted
    );

    // Another player's name line on the career hero's row (a misread icon)
    // joins no group: the member's three crops keep waiting, and so does it.
    let (_, mut store) = offer_hero_crops(&[("g1", "juno"), ("g2", "juno"), ("g3", "ana")]);
    let other = candidate(&named_board(5, 88, [TEAL, PURPLE], &other_names(10)), 5, 3)
        .proved(Proof::CareerHero, Some("ana"));
    assert_eq!(store.offer(other, &game("g4")), Offer::Pending);
    assert!(store.trusted().is_empty());
    assert_eq!(store.pending_len(), 4);
    // the member's fourth crop completes the member's group, and the other
    // player's crop is not among the seeds
    assert_eq!(
        store.offer(hero_crop(4, "ana"), &game("g4")),
        Offer::Promoted
    );
    assert_eq!(store.pending_len(), 1);
    let strangers = named_board(5, 88, [TEAL, PURPLE], &other_names(10));
    assert_eq!(
        seed_row(&measure(&strangers, 5).scores(store.trusted())),
        None
    );
}

#[test]
fn games_are_crops_that_are_each_another_game_than_each_other() {
    let tag = |session: &str, map: &str, at: u64| GameTag {
        session: session.into(),
        map: map.into(),
        at,
    };
    let a = tag("s1", "Busan", 0);
    let b = tag("s2", "Ilios", 100);
    // another session on the first map minutes later: a split of `a`, but
    // another game than `b`
    let c = tag("s3", "Busan", 200);
    let d = tag("s4", "Oasis", 300);
    assert!(spans_games(&[&a], 1));
    assert!(!spans_games(&[], 1));
    assert!(!spans_games(&[&a], 2));
    assert!(spans_games(&[&a, &b], 2));
    assert!(!spans_games(&[&a, &c], 2));
    // a-b and b-c are two games each; a, b, c are not three
    assert!(spans_games(&[&a, &b, &c], 2));
    assert!(!spans_games(&[&a, &b, &c], 3));
    // with d there are three, in whatever order they came
    assert!(spans_games(&[&a, &b, &c, &d], 3));
    assert!(spans_games(&[&c, &a, &d, &b], 3));
    assert!(!spans_games(&[&a, &b, &c, &d], 4));
}

#[test]
fn career_hero_crops_wait_in_memory_and_only_trusted_ones_are_written() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let file = seed_dir(data).join(SEED_FILE);
    let mut store = SeedStore::open(data, None);
    for (i, (session, hero)) in [("g1", "juno"), ("g2", "juno"), ("g3", "juno")]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            store.offer(hero_crop(i, hero), &game(session)),
            Offer::Pending
        );
        assert!(
            !data.join(PRIVATE_DIR).exists(),
            "nothing on disk for a waiting crop"
        );
    }
    // a restart has no waiting crop to read back
    assert_eq!(SeedStore::open(data, None).pending_len(), 0);
    assert_eq!(
        store.offer(hero_crop(3, "ana"), &game("g3")),
        Offer::Promoted
    );
    let text = std::fs::read_to_string(&file).unwrap();
    for word in ["pending", "proof", "hero", "juno", "ana", "game"] {
        assert!(!text.contains(word), "{word} is in the seed file");
    }
    let again = SeedStore::open(data, None);
    assert_eq!(again.trusted(), store.trusted());
    assert_eq!(again.trusted().len(), PROMOTE);
}

/// The whole path for a member whose highlight is never found: the career
/// hero names the row (marked), its crops are learned over three games and
/// a hero swap, and from then on seed and career hero make the row sure.
#[test]
fn a_member_who_is_never_highlighted_is_found_learned_and_then_sure() {
    let mut store = SeedStore::in_memory(None);
    let base = [
        Some("mercy"),
        Some("reinhardt"),
        Some("tracer"),
        Some("sojourn"),
        Some("lucio"),
    ];
    let live = |own: usize, hero: &'static str| {
        let mut team = base;
        team[own] = Some(hero);
        icons(&team, &ENEMY, Some(hero))
    };
    for (i, (session, own, hero)) in [
        ("g1", 2, "juno"),
        ("g2", 4, "juno"),
        ("g3", 1, "ana"),
        ("g3", 0, "ana"),
    ]
    .into_iter()
    .enumerate()
    {
        let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, own));
        let heroes = live(own, hero);
        let read = read_with(&img, 5, None, Some(&heroes), store.trusted());
        assert_eq!(
            (read.own.row, read.own.trust, read.own.source),
            (Some(own), Trust::Suspect, Source::CareerHero),
            "board {i}"
        );
        let c = read.candidate.expect("crop");
        assert!(seed_gate(
            c.row,
            c.row_bottom,
            img.height(),
            Chat::None,
            Some(&heroes)
        ));
        let offer = store.offer(c.candidate, &game(session));
        assert_eq!(offer == Offer::Promoted, i == 3, "board {i}: {offer:?}");
    }
    assert_eq!(store.trusted().len(), PROMOTE);
    // the next board: sure, nothing more to learn
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 3));
    let read = read_with(&img, 5, None, Some(&live(3, "kiriko")), store.trusted());
    assert_eq!(
        (read.own.row, read.own.trust, read.own.source),
        (Some(3), Trust::Sure, Source::SeedAndHero)
    );
    assert!(read.candidate.is_none());
    // dead on the board (own icon unread): the career hero cannot name the
    // row, the learned name still does, marked
    let mut team = base;
    team[3] = None;
    let dead = icons(&team, &ENEMY, Some("kiriko"));
    let read = read_with(&img, 5, None, Some(&dead), store.trusted());
    assert_eq!(
        (read.own.row, read.own.trust, read.own.source),
        (Some(3), Trust::Suspect, Source::SeedOnly)
    );
    // and without seeds that same board is held: nothing names a row
    let read = read_with(&img, 5, None, Some(&dead), &[]);
    assert_eq!((read.own.row, read.own.source), (None, Source::Neither));
}

/// 1440p scoreboard crop height (`1440 * 0.70` truncates to 1007) and the
/// 1080p crop (`1080 * 0.70`). The 1007 lives in one constant, shared with
/// the crop.
#[test]
fn the_review_pad_scales_like_the_clipped_header_pad() {
    use crate::ocr::preprocess::{SCOREBOARD_CROP_H_1440, crop_scoreboard_rect};
    use crate::shadow::heroes::CLIPPED_HEADER_PAD_1440;
    assert_eq!(
        crop_scoreboard_rect(2560, 1440).3,
        SCOREBOARD_CROP_H_1440,
        "the shared 1440p height drifted from crop_scoreboard"
    );
    assert_eq!(SCOREBOARD_CROP_H_1440, 1007);
    assert_eq!(
        own_row_review_pad(SCOREBOARD_CROP_H_1440),
        CLIPPED_HEADER_PAD_1440
    );
    assert_eq!(CLIPPED_HEADER_PAD_1440, 120);
    // 1080p scoreboard is 756 px; clipped_header_pad on a 1920x1080 frame is 90.
    assert_eq!(own_row_review_pad(756), 90);
    assert_eq!(own_row_review_pad(0), 0);
}

/// The saved own-row crop, not the name-line seed. 1440p pitches are 88
/// (5v5) and 75 (6v6); 1080p pitches are 66 and 57.
#[test]
fn the_saved_own_row_crop_includes_the_portrait_and_the_seed_stays_put() {
    for (team, pitch) in [(5usize, 88u32), (6, 75), (5, 66), (6, 57)] {
        let img = named_board(team, pitch, [TEAL, PURPLE], &roster(team, 0));
        let board = image::DynamicImage::ImageRgb8(img.clone());
        let rect = own_row_review_rect(&board, team, 0).expect("review rect");
        let hdr = crate::shadow::heroes::find_header(crate::shadow::heroes::View::rgb(&img))
            .expect("header");
        let bands =
            crate::shadow::heroes::find_rows(crate::shadow::heroes::View::rgb(&img), hdr, team);
        assert_eq!(bands.len(), 2 * team);
        let (px0, py0, px1, py1) = crate::shadow::heroes::portrait_box(hdr, bands[0]);
        assert!(px1 > px0 && py1 > py0, "{team} {pitch}");
        assert!(
            rect.x <= px0 as u32 && rect.y <= py0 as u32,
            "{team}v{team} @{pitch}: crop {rect:?} misses portrait origin {px0},{py0}"
        );
        assert!(
            rect.x + rect.w >= px1 as u32 && rect.y + rect.h >= py1 as u32,
            "{team}v{team} @{pitch}: crop {rect:?} misses portrait end {px1},{py1}"
        );
        assert!(rect.x + rect.w <= img.width());
        assert!(rect.y + rect.h <= img.height());
        // one row: the next row's portrait is not in this crop
        let (_, ny0, _, _) = crate::shadow::heroes::portrait_box(hdr, bands[1]);
        assert!(
            rect.y + rect.h <= ny0 as u32,
            "{team}v{team} @{pitch}: crop reaches the next row"
        );
        let crop = own_row_review_image(&board, team, 0).expect("review image");
        assert_eq!((crop.width(), crop.height()), (rect.w, rect.h));
        let crop = crop.to_rgb8();
        for (sx, sy) in [(px0, py0), (px1 - 1, py1 - 1)] {
            assert_eq!(
                crop.get_pixel(sx as u32 - rect.x, sy as u32 - rect.y),
                img.get_pixel(sx as u32, sy as u32),
                "{team}v{team} @{pitch}: portrait pixel {sx},{sy}"
            );
        }
        // The name-line seed is the glyph run, not this widened row.
        let step = (GLYPH_STEP * f64::from(pitch)).round() as u32;
        let gw = (0.20 * f64::from(pitch)).round() as u32;
        let seed = candidate(&img, team, 0);
        assert_eq!(
            seed.seed().w,
            5 * step + gw,
            "{team}v{team} @{pitch}: seed width changed"
        );
        assert!(
            seed.seed().h < rect.h,
            "{team} {pitch}: seed is a name line"
        );
        assert!(seed.seed().w < rect.w, "{team} {pitch}");
        let row_h = (bands[0].1 - bands[0].0 + 2) as u32;
        let name_x = hdr.0 + px(SEARCH_X0 * f64::from(row_h));
        // The portrait is about one row pitch left of the name. The
        // board-height pad is the margin while it stays wider than that.
        let reach = review_reach(img.height(), row_h);
        assert!(
            own_row_review_pad(img.height()) >= row_h,
            "{team}v{team} @{pitch}: pad should still be the margin"
        );
        assert_eq!(
            rect.x,
            (name_x as u32).saturating_sub(reach),
            "{team}v{team} @{pitch}"
        );
        assert!(
            rect.x > 0,
            "{team} {pitch}: unshifted board should not clamp"
        );
    }
}

/// A name column closer to the image edge than the pad does not read past x = 0.
#[test]
fn the_saved_own_row_crop_clamps_at_the_left_edge() {
    for (team, pitch) in [(5usize, 88u32), (6, 75), (5, 66), (6, 57)] {
        let img = named_board(team, pitch, [TEAL, PURPLE], &roster(team, 0));
        let hdr = crate::shadow::heroes::find_header(crate::shadow::heroes::View::rgb(&img))
            .expect("header");
        let dx = hdr.0 as u32;
        let slid =
            image::imageops::crop_imm(&img, dx, 0, img.width() - dx, img.height()).to_image();
        let board = image::DynamicImage::ImageRgb8(slid.clone());
        let hdr = crate::shadow::heroes::find_header(crate::shadow::heroes::View::rgb(&slid))
            .expect("header at left");
        assert_eq!(hdr.0, 0, "{team} {pitch}");
        let bands =
            crate::shadow::heroes::find_rows(crate::shadow::heroes::View::rgb(&slid), hdr, team);
        let row_h = bands[0].1 - bands[0].0 + 2;
        let name_x = hdr.0 + px(SEARCH_X0 * row_h as f64);
        let pad = own_row_review_pad(slid.height());
        assert!(
            (name_x as u32) < pad,
            "{team}v{team} @{pitch}: name {name_x} pad {pad} does not need a clamp"
        );
        let rect = own_row_review_rect(&board, team, 0).expect("rect");
        assert_eq!(rect.x, 0, "{team} {pitch}");
        assert_eq!(rect.w, slid.width(), "{team} {pitch}");
        assert!(rect.y + rect.h <= slid.height());
        // Sliding the table left by the header margin does not widen the seed.
        let step = (GLYPH_STEP * f64::from(pitch)).round() as u32;
        let gw = (0.20 * f64::from(pitch)).round() as u32;
        let seed = candidate(&slid, team, 0);
        assert_eq!(seed.seed().w, 5 * step + gw, "{team} {pitch}");
    }
}

/// A 2560x1440 frame with a synthetic board whose header bar starts `left`
/// px left of the scoreboard crop (negative: to the right of that edge).
fn frame_1440(team: usize, pitch: u32, left: i64) -> DynamicImage {
    let img = named_board(team, pitch, [TEAL, PURPLE], &roster(team, 0));
    let (cx, cy, _, _) = crate::ocr::preprocess::crop_scoreboard_rect(2560, 1440);
    let mut frame = RgbImage::from_pixel(2560, 1440, Rgb([12, 14, 24]));
    let x = cx as i64 - left - i64::from(pitch / 2);
    image::imageops::overlay(&mut frame, &img, x, i64::from(cy));
    DynamicImage::ImageRgb8(frame)
}

/// Clipped 5v5 at 1440p: the portrait starts left of `crop_scoreboard`, so
/// the saved crop has to come from the hero reader's widened frame and the
/// name column measured there (shifted by the pad).
#[test]
fn the_clipped_5v5_1440p_crop_holds_the_portrait_and_the_name_column() {
    use crate::ocr::preprocess::crop_scoreboard;
    use crate::shadow::heroes::{
        CLIPPED_HEADER_PAD_1440, View, clipped_header_pad, find_header, find_rows, hero_crop,
        portrait_box,
    };
    // Header 90 px left of the crop. The portrait then starts inside the
    // 120 px pad, off the scoreboard image.
    let frame = frame_1440(5, 80, 90);
    let scoreboard = crop_scoreboard(&frame);
    let sb = scoreboard.to_rgb8();
    let hdr_sb = find_header(View::rgb(&sb)).expect("clipped header");
    assert_eq!(hdr_sb.0, 0, "header starts at the scoreboard's left edge");
    let pad = clipped_header_pad(frame.width(), frame.height(), &scoreboard, hdr_sb.0)
        .expect("clipped header pad");
    assert_eq!(pad, CLIPPED_HEADER_PAD_1440);
    let cut = own_row_review_cut(Some(&frame), &scoreboard, 5, 0).expect("review cut");
    assert!(cut.widened);
    assert_eq!(cut.pad, pad);

    let wide = hero_crop(&frame, pad).to_rgb8();
    let hdr = find_header(View::rgb(&wide)).expect("wide header");
    assert!(hdr.0 > 0, "the wide crop holds the real header start");
    let bands = find_rows(View::rgb(&wide), hdr, 5);
    assert_eq!(bands.len(), 10);
    let row_h = (bands[0].1 - bands[0].0 + 2) as u32;
    let name_x = hdr.0 + px(SEARCH_X0 * f64::from(row_h));
    let reach = review_reach(wide.height(), row_h);
    assert_eq!(
        cut.rect.x,
        (name_x as u32).saturating_sub(reach),
        "name column {name_x} does not line up with the crop"
    );
    // The wide crop prepends `pad` px, so this column is the same pixel as
    // scoreboard x = name_x - pad.
    assert!(name_x as u32 >= pad, "name column {name_x} pad {pad}");
    let y = bands[0].0 as u32;
    assert_eq!(
        wide.get_pixel(name_x as u32, y),
        sb.get_pixel(name_x as u32 - pad, y),
        "name column was not shifted by the pad"
    );

    let (px0, py0, px1, py1) = portrait_box(hdr, bands[0]);
    assert!(px1 > px0 && py1 > py0);
    assert!(
        px0 < pad as usize,
        "portrait origin {px0} is already inside the scoreboard crop"
    );
    assert!(
        cut.rect.x <= px0 as u32 && cut.rect.y <= py0 as u32,
        "crop {:?} misses portrait origin {px0},{py0}",
        cut.rect
    );
    assert!(
        cut.rect.x + cut.rect.w >= px1 as u32 && cut.rect.y + cut.rect.h >= py1 as u32,
        "crop {:?} misses portrait end {px1},{py1}",
        cut.rect
    );
    let crop = cut.image.to_rgb8();
    assert_eq!((crop.width(), crop.height()), (cut.rect.w, cut.rect.h));
    for (sx, sy) in [(px0, py0), (px0, py1 - 1), (px1 - 1, py1 - 1)] {
        assert_eq!(
            crop.get_pixel(sx as u32 - cut.rect.x, sy as u32 - cut.rect.y),
            wide.get_pixel(sx as u32, sy as u32),
            "portrait pixel {sx},{sy}"
        );
    }
    // The name-line seed is still the glyph run, measured on the wide crop
    // the same way NameBoard::measure re-cuts a clipped header.
    let step = (GLYPH_STEP * 80.0f64).round() as u32;
    let gw = (0.20f64 * 80.0).round() as u32;
    let seed = NameBoard::measure(Some(&frame), &scoreboard, 5)
        .expect("wide measure")
        .candidate(0)
        .expect("seed");
    assert_eq!(seed.seed().w, 5 * step + gw);
}

/// A 6v6 board whose header sits inside the scoreboard crop keeps the
/// unwidened cut, frame or not.
#[test]
fn the_6v6_review_crop_stays_on_the_scoreboard_when_the_header_is_inside() {
    use crate::ocr::preprocess::crop_scoreboard;
    use crate::shadow::heroes::{View, clipped_header_pad, find_header};
    let frame = frame_1440(6, 75, -40);
    let scoreboard = crop_scoreboard(&frame);
    let sb = scoreboard.to_rgb8();
    let hdr = find_header(View::rgb(&sb)).expect("header");
    assert!(hdr.0 > 0, "header at {}", hdr.0);
    assert_eq!(
        clipped_header_pad(frame.width(), frame.height(), &scoreboard, hdr.0),
        None
    );
    let plain = own_row_review_rect(&scoreboard, 6, 0).expect("scoreboard rect");
    let cut = own_row_review_cut(Some(&frame), &scoreboard, 6, 0).expect("frame cut");
    assert!(!cut.widened);
    assert_eq!(cut.pad, 0);
    assert_eq!(cut.rect, plain);
    let from_frame = cut.image.to_rgb8();
    let from_board = own_row_review_image(&scoreboard, 6, 0)
        .expect("board image")
        .to_rgb8();
    assert_eq!(from_frame.dimensions(), from_board.dimensions());
    assert_eq!(from_frame.as_raw(), from_board.as_raw());
    assert!(plain.x > 0, "6v6 crop clamped");
}

/// The second made-up name of the member: another of their accounts.
const ALT: [usize; 7] = [9, 4, 9, 2, 7, 7, 1];

/// A store that trusts two of the member's names at 88 px, [`OWN`] and
/// [`ALT`], as two groups.
fn seeded_twice() -> SeedStore {
    let mut store = seeded(5, 88);
    for (i, session) in ["g3", "g3", "g4"].into_iter().enumerate() {
        let mut names = other_names(10);
        names[i] = ALT.to_vec();
        let img = named_board(5, 88, [TEAL, PURPLE], &names);
        store.offer(candidate(&img, 5, i), &game(session));
    }
    assert_eq!(store.trusted().len(), 2 * PROMOTE, "two groups trusted");
    store
}

/// 2026-10-19: two of the member's accounts in one lobby (a friend playing
/// one). Each row matches its own seeds, so the seeds name no row. The
/// strong highlight with the career hero on it decides, and the row is
/// sure: no field is flagged. Before, the board fell back to the highlight
/// alone and every field was marked.
#[test]
fn two_of_the_members_accounts_in_one_lobby_resolve_by_highlight_and_hero() {
    let store = seeded_twice();
    let mut names = other_names(10);
    names[0] = OWN.to_vec();
    names[2] = ALT.to_vec();
    let img = named_board(5, 88, [TEAL, PURPLE], &names);
    let mut team = TEAM;
    team[0] = Some("wrecking_ball");
    team[2] = Some("lucio");
    let live = icons(&team, &ENEMY, Some("wrecking_ball"));
    let strong = SEED_HIGHLIGHT_LEAD;
    // tonight's shape: both rows match, neither leads by the margin
    let scores = measure(&img, 5).scores(store.trusted());
    assert!(
        scores[0] >= MATCH_FLOOR && scores[2] >= MATCH_FLOOR,
        "{scores:?}"
    );
    assert_eq!(seed_row(&scores), None);
    assert_eq!(seed_rows_matched(&scores), Some(vec![0, 2]));
    let sure = read_with(&img, 5, Some((0, strong)), Some(&live), store.trusted());
    assert_eq!(
        (sure.own.row, sure.own.trust, sure.own.source),
        (Some(0), Trust::Sure, Source::HighlightAndHero)
    );
    assert!(!sure.own.suspect(), "no field is flagged");
    assert_eq!(
        sure.verdict,
        Verdict::Silent,
        "no strike against either group"
    );
    // the row's name is already trusted: nothing new to learn
    if let Some(c) = sure.candidate {
        let mut learner = seeded_twice();
        assert_eq!(learner.offer(c.candidate, &game("g9")), Offer::Known);
    }
    // the same lobby with the member on the other account
    let mut swapped = team;
    swapped[0] = Some("lucio");
    swapped[2] = Some("wrecking_ball");
    let other = icons(&swapped, &ENEMY, Some("wrecking_ball"));
    let alt = read_with(&img, 5, Some((2, strong)), Some(&other), store.trusted());
    assert_eq!(
        (alt.own.row, alt.own.trust, alt.own.source),
        (Some(2), Trust::Sure, Source::HighlightAndHero)
    );
    // a weak highlight: marked
    let weak = read_with(
        &img,
        5,
        Some((0, strong - 0.01)),
        Some(&live),
        store.trusted(),
    );
    assert_eq!((weak.own.row, weak.own.trust), (Some(0), Trust::Suspect));
    assert!(weak.candidate.is_none());
    // the career hero on the other matched row: marked
    let disagree = read_with(&img, 5, Some((0, strong)), Some(&other), store.trusted());
    assert_eq!(
        (disagree.own.row, disagree.own.trust),
        (Some(0), Trust::Suspect)
    );
    assert!(disagree.candidate.is_none());
    // no career panel: marked
    let no_panel = read_with(&img, 5, Some((0, strong)), None, store.trusted());
    assert_eq!(no_panel.own.trust, Trust::Suspect);
    // a highlighted row that is neither matched row, career hero on it:
    // the seeds point elsewhere, so it is marked and nothing is cut
    let mut third = TEAM;
    third[3] = Some("wrecking_ball");
    let on_three = icons(&third, &ENEMY, Some("wrecking_ball"));
    let stray = read_with(&img, 5, Some((3, strong)), Some(&on_three), store.trusted());
    assert_eq!(
        (stray.own.row, stray.own.trust, stray.own.source),
        (Some(3), Trust::Suspect, Source::HighlightOnly)
    );
    assert!(stray.candidate.is_none());
}

/// A highlight alone is never cut, so a wrong row is not learned.
#[test]
fn no_seed_is_cut_from_a_marked_row() {
    let img = named_board(5, 88, [TEAL, PURPLE], &roster(5, 2));
    let live = icons(&TEAM, &ENEMY, Some("juno"));
    let weak = SEED_HIGHLIGHT_LEAD - 0.01;
    // a highlight the career hero does not confirm, strong or weak
    for (highlight, heroes) in [
        (Some((0, SEED_HIGHLIGHT_LEAD)), Some(&live)),
        (Some((0, weak)), Some(&live)),
        (Some((2, SEED_HIGHLIGHT_LEAD)), None),
        (Some((2, weak)), None),
    ] {
        let read = read_with(&img, 5, highlight, heroes, &[]);
        assert_ne!(read.own.trust, Trust::Sure, "{highlight:?}");
        assert!(read.candidate.is_none(), "{highlight:?}");
    }
}

/// The career hero alone (no highlight, or a weak one on its row) seeds only
/// on a board where no row matches a trusted group: when another row shows a
/// saved name, the career hero's row may be someone else's, and nothing is
/// learned from that board.
#[test]
fn a_hero_only_board_where_another_row_matches_a_trusted_group_learns_nothing() {
    let store = seeded(5, 88);
    // a saved name on row 0, the career hero (and nothing else) on row 2,
    // which shows a name no group holds
    let mut names = other_names(10);
    names[0] = OWN.to_vec();
    let img = named_board(5, 88, [TEAL, PURPLE], &names);
    let live = icons(&TEAM, &ENEMY, Some("juno"));
    let scores = measure(&img, 5).scores(store.trusted());
    assert!(scores[0] >= MATCH_FLOOR, "{scores:?}");
    for highlight in [None, Some((2, SEED_HIGHLIGHT_LEAD - 0.01))] {
        let read = read_with(&img, 5, highlight, Some(&live), store.trusted());
        assert_eq!(read.hero_row, Some(2), "{highlight:?}");
        assert!(read.candidate.is_none(), "{highlight:?}");
    }
    // the same board with no saved name on it: the career hero's row is cut
    let bare = named_board(5, 88, [TEAL, PURPLE], &other_names(10));
    assert!(
        measure(&bare, 5)
            .scores(store.trusted())
            .iter()
            .all(|s| *s < MATCH_FLOOR)
    );
    for highlight in [None, Some((2, SEED_HIGHLIGHT_LEAD - 0.01))] {
        let read = read_with(&bare, 5, highlight, Some(&live), store.trusted());
        assert_eq!(
            read.candidate.expect("crop").candidate.proof(),
            Proof::CareerHero,
            "{highlight:?}"
        );
    }
}

/// One row leading by the margin stays a normal seed match even when a
/// lookalike name on another row also reaches the floor (0.97 vs 0.84):
/// sure through the seed, and the board still confirms the seeds.
#[test]
fn a_seed_that_leads_by_the_margin_stays_a_seed_match_beside_a_lookalike() {
    let scores = [0.97, 0.20, 0.84, 0.10, 0.15, 0.10, 0.12, 0.10, 0.11, 0.10];
    assert_eq!(seed_row(&scores), Some(0));
    assert_eq!(seed_rows_matched(&scores), Some(vec![0, 2]));
    // the highlight on the seed's row: sure through the seed
    assert_eq!(
        combine_scores(&scores, lit(0, 1.5), Some(0), true),
        OwnRow {
            row: Some(0),
            trust: Trust::Sure,
            source: Source::Both
        }
    );
    // no highlight, the career hero on the seed's row: seed and hero
    assert_eq!(
        combine_scores(&scores, None, Some(0), true).source,
        Source::SeedAndHero
    );
    // a strong highlight with the career hero on the lookalike's row: the
    // seed and the highlight disagree, as they always have
    assert_eq!(
        combine_scores(&scores, lit(2, 1.5), Some(2), true).source,
        Source::Disagree
    );
    // the same scores without the lead are the multi-match path
    let tied = [0.97, 0.20, 0.92, 0.10, 0.15, 0.10, 0.12, 0.10, 0.11, 0.10];
    assert_eq!(seed_row(&tied), None);
    assert_eq!(
        combine_scores(&tied, lit(0, 1.5), Some(0), true).source,
        Source::HighlightAndHero
    );
}
