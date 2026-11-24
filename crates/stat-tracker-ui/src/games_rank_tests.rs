use super::*;
use crate::games_rows::tests::game;
use crate::games_rows::{Col, Grid, Width, columns};
use crate::model::Outcome;
use stat_tracker::model::{QueueTitle, RankNumber, RankText, RankUpdate};

fn word(value: &str) -> Option<RankText> {
    Some(RankText {
        value: value.into(),
        confidence: 0.9,
    })
}

fn num(value: i64) -> Option<RankNumber> {
    Some(RankNumber {
        value,
        confidence: 0.9,
    })
}

fn rank(role: &str, tier: &str, division: i64, progress: i64, change: i64) -> RankUpdate {
    RankUpdate {
        role: word(role),
        role_rank: word(tier),
        division: num(division),
        progress_pct: num(progress),
        progress_change: num(change),
        ..RankUpdate::default()
    }
}

fn with_rank(r: RankUpdate) -> Game {
    let mut g = game(Outcome::Win, "2026-09-02T21:14:00Z", &[]);
    g.rank_update = Some(r);
    g
}

fn queue(value: &str) -> QueueTitle {
    QueueTitle {
        value: Some(value.into()),
        confidence: 0.95,
        ..QueueTitle::default()
    }
}

/// The Rank cell's text, top line first, as [`cell`] draws it: the lines
/// of [`RankShown::from_game`], or "-" when it is None.
fn cell_lines(g: &Game) -> Vec<String> {
    RankShown::from_game(g).map_or_else(|| vec!["-".to_string()], |r| r.lines())
}

fn shown(g: &Game) -> RankShown {
    RankShown::from_game(g).expect("a rank")
}

#[test]
fn rank_text_variants() {
    let full = RankShown {
        tier: Some("Diamond".into()),
        division: Some(2),
        ..RankShown::default()
    };
    assert_eq!(full.rank_text().as_deref(), Some("Diamond 2"));
    let tier = RankShown {
        division: None,
        ..full.clone()
    };
    assert_eq!(tier.rank_text().as_deref(), Some("Diamond"));
    let division = RankShown { tier: None, ..full };
    assert_eq!(division.rank_text().as_deref(), Some("Division 2"));
    assert_eq!(RankShown::default().rank_text(), None);
}

#[test]
fn change_text_signs() {
    assert_eq!(change_text(23), "+23%");
    assert_eq!(change_text(-15), "-15%");
    assert_eq!(change_text(0), "±0%");
}

/// The pack may spell the tier in capitals; the site shows it title cased.
#[test]
fn tier_shows_as_the_site_does() {
    let g = with_rank(rank("Support", "  DIAMOND ", 2, 45, 23));
    assert_eq!(shown(&g).rank_text().as_deref(), Some("Diamond 2"));
    let g = with_rank(rank("Support", "grandmaster", 1, 0, 0));
    assert_eq!(shown(&g).rank_text().as_deref(), Some("Grandmaster 1"));
}

#[test]
fn sentence_with_and_without_queue_and_role() {
    let mut g = with_rank(rank("Support", "Diamond", 2, 45, 23));
    assert_eq!(
        shown(&g).sentence(),
        "Rank: Support, Diamond 2, 45% progress, +23% this game"
    );
    g.queue_title = Some(queue("ROLE QUEUE"));
    assert_eq!(
        shown(&g).sentence(),
        "Rank: Support, ROLE QUEUE, Diamond 2, 45% progress, +23% this game"
    );
    // A weak queue read is not shown, as the map chip and upload skip it.
    g.queue_title = Some(QueueTitle {
        confidence: 0.1,
        ..queue("ROLE QUEUE")
    });
    assert_eq!(
        shown(&g).sentence(),
        "Rank: Support, Diamond 2, 45% progress, +23% this game"
    );

    // No role read: the site's "Role unknown".
    let mut g = with_rank(rank("Support", "Gold", 3, -15, -15));
    g.rank_update.as_mut().unwrap().role = None;
    assert_eq!(
        shown(&g).sentence(),
        "Rank: Role unknown, Gold 3, -15% progress, -15% this game. \
         Negative progress means demotion protection."
    );
    g.queue_title = Some(queue("ROLE QUEUE"));
    assert_eq!(
        shown(&g).sentence(),
        "Rank: Role unknown, ROLE QUEUE, Gold 3, -15% progress, -15% this game. \
         Negative progress means demotion protection."
    );
    // Zero progress is not negative; a negative change alone says nothing
    // about demotion protection.
    let g = with_rank(rank("Support", "Gold", 3, 0, -15));
    assert_eq!(
        shown(&g).sentence(),
        "Rank: Support, Gold 3, 0% progress, -15% this game"
    );
}

/// Open shows only with an open queue title, and once; a role icon read
/// under an open queue title is a misread the upload drops.
#[test]
fn open_queue_role_follows_the_upload() {
    let mut g = with_rank(rank("Open", "Silver", 1, 10, 0));
    assert_eq!(shown(&g).context(), ROLE_UNKNOWN);
    g.queue_title = Some(queue("OPEN QUEUE"));
    assert_eq!(shown(&g).context(), "OPEN QUEUE");
    // The game's own spelling, 6V6 included, and open queue said once.
    g.queue_title = Some(queue("COMPETITIVE: 6V6 OPEN QUEUE"));
    assert_eq!(shown(&g).context(), "COMPETITIVE: 6V6 OPEN QUEUE");
    assert_eq!(
        shown(&g).sentence(),
        "Rank: COMPETITIVE: 6V6 OPEN QUEUE, Silver 1, 10% progress, ±0% this game"
    );
    // OPEN not next to QUEUE is still an open queue title, said once.
    g.queue_title = Some(queue("OPEN 6V6 QUEUE"));
    assert_eq!(shown(&g).context(), "OPEN 6V6 QUEUE");
    let mut g = with_rank(rank("Tank", "Silver", 1, 10, 0));
    g.queue_title = Some(queue("OPEN QUEUE"));
    assert_eq!(shown(&g).context(), "Role unknown, OPEN QUEUE");
    assert_eq!(context_label(Some("Tank"), None), "Tank");
}

/// The role shows only for the exact `RankRole` spellings the upload
/// sends; other case or surrounding spaces show no role.
#[test]
fn only_the_rank_role_spellings_show_a_role() {
    assert_eq!(role_name("Tank", false), Some("Tank"));
    assert_eq!(role_name("Damage", false), Some("Damage"));
    assert_eq!(role_name("Support", false), Some("Support"));
    assert_eq!(role_name("Open", true), Some(OPEN_ROLE));
    for value in ["TANK", "tank", " Tank", "Tank ", "SUPPORT", "OPEN", " Open"] {
        assert_eq!(role_name(value, false), None, "{value:?}");
        assert_eq!(role_name(value, true), None, "{value:?}");
    }
}

/// The rank tooltip spells the queue as the queue chip and the Map
/// tooltip do (`games_detail::map_chips`, `games_map_fit::fit`).
#[test]
fn both_tooltips_spell_the_queue_the_same_way() {
    for title in ["ROLE QUEUE", "COMPETITIVE: 6V6 OPEN QUEUE"] {
        let mut g = with_rank(rank("Support", "Gold", 3, 10, 5));
        g.queue_title = Some(queue(title));
        let chip = crate::games_detail::map_chips(&g)
            .into_iter()
            .find(|c| c.contains("QUEUE"))
            .unwrap();
        assert_eq!(chip, title);
        assert!(
            shown(&g).sentence().contains(&format!(", {chip}, ")),
            "{title}"
        );
        let parts = crate::games_detail::map_cell_parts(&g, false);
        let mode = crate::games_rows::mode_chip(&g.map_name);
        let fit =
            crate::games_map_fit::fit(1.0, 40.0, &g.map_name, iced::Font::DEFAULT, &parts, mode);
        assert!(fit.tip.unwrap().contains(&chip), "{title}");
    }
}

#[test]
fn cell_lines_for_each_case() {
    let full = with_rank(rank("Support", "DIAMOND", 2, 45, 23));
    assert_eq!(cell_lines(&full), ["Diamond 2", "45% +23%"]);
    let down = with_rank(rank("Damage", "Platinum", 4, 80, -15));
    assert_eq!(cell_lines(&down), ["Platinum 4", "80% -15%"]);
    let tier = with_rank(RankUpdate {
        role_rank: word("Gold"),
        ..RankUpdate::default()
    });
    assert_eq!(cell_lines(&tier), ["Gold"]);
    assert_eq!(
        shown(&tier).sentence(),
        "Rank: Role unknown, Gold".to_string()
    );
    let numbers = with_rank(RankUpdate {
        progress_pct: num(-15),
        ..RankUpdate::default()
    });
    assert_eq!(cell_lines(&numbers), ["-15% progress"]);
    // One number alone says which it is, in the tooltip's words.
    let mut progress_only = with_rank(rank("Support", "GOLD", 3, 45, 0));
    progress_only.rank_update.as_mut().unwrap().progress_change = None;
    assert_eq!(cell_lines(&progress_only), ["Gold 3", "45% progress"]);
    assert_eq!(
        shown(&progress_only).sentence(),
        "Rank: Support, Gold 3, 45% progress"
    );
    let mut change_only = with_rank(rank("Support", "GOLD", 3, 0, -5));
    change_only.rank_update.as_mut().unwrap().progress_pct = None;
    assert_eq!(cell_lines(&change_only), ["Gold 3", "-5% this game"]);
    assert_eq!(
        shown(&change_only).sentence(),
        "Rank: Support, Gold 3, -5% this game"
    );
    change_only.rank_update.as_mut().unwrap().progress_change = num(23);
    assert_eq!(cell_lines(&change_only), ["Gold 3", "+23% this game"]);
}

#[test]
fn no_rank_shows_a_dash() {
    let g = game(Outcome::Win, "2026-09-02T21:14:00Z", &[]);
    assert_eq!(g.rank_update, None);
    assert_eq!(cell_lines(&g), ["-"]);
    // A rank with nothing the cell can show is no rank either: an empty
    // read, a role alone, or values out of range.
    assert_eq!(cell_lines(&with_rank(RankUpdate::default())), ["-"]);
    let role_only = with_rank(RankUpdate {
        role: word("Support"),
        hero_sr: num(2500),
        ..RankUpdate::default()
    });
    assert_eq!(cell_lines(&role_only), ["-"]);
    assert_eq!(
        cell_lines(&with_rank(rank("Support", "  ", 9, 400, -101))),
        ["-"]
    );
}

#[test]
fn from_match_copies_rank_update() {
    let mut m = crate::fixtures::pm(
        "s",
        "Ana",
        "Ilios",
        "Support",
        "victory",
        1,
        1,
        1,
        1,
        1,
        1,
        crate::fixtures::dt(2026, 9, 2, 20, 0),
    );
    assert_eq!(Game::from_match(&m).rank_update, None);
    let r = rank("Support", "Diamond", 2, 45, 23);
    m.rank_update = Some(r.clone());
    assert_eq!(Game::from_match(&m).rank_update, Some(r));
}

#[test]
fn champion_and_grandmaster_cells() {
    let champion = with_rank(rank("Support", "CHAMPION", 3, 100, 100));
    assert_eq!(cell_lines(&champion), ["Champion 3", "100% +100%"]);
    let grandmaster = with_rank(rank("Tank", "grandmaster", 5, -100, -100));
    assert_eq!(cell_lines(&grandmaster), ["Grandmaster 5", "-100% -100%"]);
    // Division 0 and 6 and values past 100 are not shown, as on the site.
    let edges = with_rank(rank("Tank", "Grandmaster", 6, 101, -101));
    assert_eq!(cell_lines(&edges), ["Grandmaster"]);
    let edges = with_rank(rank("Tank", "Champion", 0, -101, 101));
    assert_eq!(cell_lines(&edges), ["Champion"]);
    let edges = with_rank(rank("Tank", "Champion", 1, 100, -100));
    assert_eq!(cell_lines(&edges), ["Champion 1", "100% -100%"]);
}

/// A value the upload drops for its confidence (not finite, 0 or less,
/// over 1) or its length (over 64 characters once trimmed) is not shown.
#[test]
fn values_the_upload_drops_are_not_shown() {
    for bad in [0.0, -0.5, 1.01, f32::NAN, f32::INFINITY] {
        let mut r = rank("Support", "Diamond", 2, 45, 23);
        r.role_rank.as_mut().unwrap().confidence = bad;
        r.progress_pct.as_mut().unwrap().confidence = bad;
        assert_eq!(
            cell_lines(&with_rank(r)),
            ["Division 2", "+23% this game"],
            "{bad}"
        );
    }
    let mut r = rank("Support", "Diamond", 2, 45, 23);
    r.role.as_mut().unwrap().confidence = f32::NAN;
    assert_eq!(shown(&with_rank(r)).context(), ROLE_UNKNOWN);
    let at_cap = format!("  {}  ", "d".repeat(64));
    let g = with_rank(rank("Support", &at_cap, 2, 45, 23));
    let title = format!("D{}", "d".repeat(63));
    assert_eq!(shown(&g).tier.as_deref(), Some(title.as_str()));
    let g = with_rank(rank("Support", &"d".repeat(65), 2, 45, 23));
    assert_eq!(cell_lines(&g), ["Division 2", "45% +23%"]);
    let mut g = with_rank(rank("Support", "Diamond", 2, 45, 23));
    g.queue_title = Some(queue(&"Q".repeat(65)));
    assert_eq!(shown(&g).queue, None);
}

/// The sample fixture has every case the Rank cell draws.
#[test]
fn sample_fixture_ranks() {
    let snap = crate::fixtures::snapshot(crate::cli::FixtureKind::Sample);
    let cells: Vec<Vec<String>> = snap
        .matches
        .iter()
        .map(|m| cell_lines(&Game::from_match(m)))
        .collect();
    assert_eq!(
        cells,
        vec![
            vec!["Diamond 2", "45% +23%"],
            vec!["Champion 3", "80% -15%"],
            vec!["Gold"],
            vec!["Grandmaster 5", "-100% -100%"],
            vec!["-"],
        ]
    );
}

/// The site's `RankShown::from_rank` (`crates/app/src/pages/stats/rank.rs`,
/// private to the app crate, which the tracker does not build), copied for
/// this test with the role as the words it shows.
fn site_shown(rank: &scuffed_types::api::MatchRank) -> Option<RankShown> {
    use scuffed_types::api::RankRole;
    let shown = RankShown {
        role: rank.role.value.map(|r| match r {
            RankRole::Tank => "Tank",
            RankRole::Damage => "Damage",
            RankRole::Support => "Support",
            RankRole::Open => "Open queue",
        }),
        tier: rank
            .role_rank
            .value
            .clone()
            .and_then(|t| clean_text(&t))
            .map(|t| title_case(&t.to_lowercase())),
        division: rank.division.value.filter(|d| (1..=5).contains(d)),
        progress: rank.progress_pct.value.filter(|p| (-100..=100).contains(p)),
        change: rank
            .progress_change
            .value
            .filter(|c| (-100..=100).contains(c)),
        queue: rank
            .queue_title
            .value
            .clone()
            .and_then(|q| clean_text(&q))
            .map(|q| q.to_uppercase()),
    };
    (shown != RankShown::default()).then_some(shown)
}

/// A stored game the upload sends (a known map and hero).
fn upload_base() -> stat_tracker::storage::PersonalMatch {
    crate::fixtures::pm(
        "s",
        "Ana",
        "Ilios",
        "Support",
        "victory",
        1,
        1,
        1,
        1,
        1,
        1,
        crate::fixtures::dt(2026, 9, 2, 20, 0),
    )
}

/// The rank cases for [`the_cell_matches_what_the_upload_sends_the_site`]:
/// the edges of each range, Champion and Grandmaster, every role spelling
/// and queue, and confidences the upload keeps and drops.
fn upload_cases() -> Vec<(RankUpdate, Option<QueueTitle>)> {
    let tiers = [
        "GRANDMASTER".to_string(),
        "champion".to_string(),
        "  Diamond  ".to_string(),
        "   ".to_string(),
        "x".repeat(64),
        "x".repeat(65),
    ];
    let roles = [
        "Tank", "Damage", "Support", "Open", "Healer", "TANK", " Tank",
    ];
    let queues = [
        None,
        Some("ROLE QUEUE"),
        Some("OPEN QUEUE"),
        Some("OPEN QUEUE 6V6"),
    ];
    let numbers = [-101, -100, 0, 45, 100, 101, i64::MAX];
    let mut cases = Vec::new();
    let mut i = 0;
    for tier in &tiers {
        for division in [0, 1, 5, 6] {
            for (p, c) in numbers.iter().zip(numbers.iter().rev()) {
                let r = rank(roles[i % roles.len()], tier, division, *p, *c);
                cases.push((r, queues[i % queues.len()].map(queue)));
                i += 1;
            }
        }
    }
    for bad in [
        0.0,
        -0.5,
        1.0,
        1.01,
        f32::NAN,
        f32::INFINITY,
        f32::MIN_POSITIVE,
    ] {
        let mut r = rank("Support", "Grandmaster", 1, 100, -100);
        r.role.as_mut().unwrap().confidence = bad;
        r.role_rank.as_mut().unwrap().confidence = bad;
        r.division.as_mut().unwrap().confidence = bad;
        cases.push((r.clone(), None));
        r.progress_pct.as_mut().unwrap().confidence = bad;
        r.progress_change.as_mut().unwrap().confidence = bad;
        cases.push((r, Some(queue("ROLE QUEUE"))));
    }
    let weak = QueueTitle {
        confidence: 0.1,
        ..queue("OPEN QUEUE")
    };
    cases.push((rank("Open", "Champion", 2, 10, 5), Some(weak)));
    let long_queue = queue(&"Q".repeat(65));
    cases.push((rank("Open", "Champion", 2, 10, 5), Some(long_queue)));
    cases.push((RankUpdate::default(), Some(queue("ROLE QUEUE"))));
    cases.push((RankUpdate::default(), None));
    cases
}

/// Each case goes through the real upload (`stat_tracker::sync::
/// upload_request`, whose `rank` key is `upload_rank`'s object), then the
/// site's `resolve_rank` and its badge rules ([`site_shown`]). The cell
/// shows the same values, or "-" when the site would have no rank text,
/// progress or change to show.
#[test]
fn the_cell_matches_what_the_upload_sends_the_site() {
    let mut shown_count = 0;
    for (r, q) in upload_cases() {
        let mut m = upload_base();
        m.rank_update = Some(r);
        m.queue_title = q;
        let request = stat_tracker::sync::upload_request(std::slice::from_ref(&m), &[]);
        let body = serde_json::to_value(request).unwrap();
        let sent = &body["matches"][0];
        assert!(sent.is_object(), "the row is sent: {body}");
        let site = scuffed_types::api::resolve_rank(sent.get("rank"))
            .and_then(|rank| site_shown(&rank))
            .filter(|s| !s.badge_parts().is_empty());
        let cell = RankShown::from_game(&Game::from_match(&m));
        assert_eq!(cell, site, "{:?} {:?}", m.rank_update, m.queue_title);
        shown_count += usize::from(cell.is_some());
    }
    assert!(shown_count > 100, "most cases show a rank: {shown_count}");
}

/// The tiny-skia renderer's width of `element`, laid out with no width
/// limit, so text that does not wrap comes out at its full width.
fn natural_width(renderer: &iced::Renderer, mut element: Element<'_, Message>) -> f32 {
    use iced::advanced::layout::Limits;
    use iced::advanced::widget::Tree;
    let mut tree = Tree::new(&element);
    let limits = Limits::new(iced::Size::ZERO, iced::Size::new(10_000.0, 400.0));
    element
        .as_widget_mut()
        .layout(&mut tree, renderer, &limits)
        .bounds()
        .width
}

/// The Rank column's width at `grid`.
fn rank_width(grid: Grid) -> f32 {
    columns(grid)
        .into_iter()
        .find_map(|(c, w)| match (c, w) {
            (Col::Rank, Width::Px(px)) => Some(px),
            _ => None,
        })
        .expect("a fixed Rank column")
}

/// Every rank text and second line the cell can draw fits the Rank column
/// at every grid, measured in the bundled Urbanist Medium at the cell's
/// sizes (`SIZE_META` top line, `SIZE_LABEL` second). The cell does not
/// wrap and clips to the column, so a wider line loses the division: at
/// the spec's 64 px "Champion 3" read "Champion", and at 64 and 72 px
/// "Grandmaster 2" read "Grandmaster". The widest cell and the widest
/// second line are taken over every tier, division and number case here
/// (two numbers, progress alone, change alone), not from one named line.
#[test]
fn every_rank_line_fits_the_rank_column() {
    let renderer = crate::games_bans_view::tests::headless();
    let line = |s: &str, size: f32| {
        let t = text(s.to_string()).size(size).wrapping(Wrapping::None);
        natural_width(&renderer, t.into())
    };
    let tiers = [
        "BRONZE",
        "SILVER",
        "GOLD",
        "PLATINUM",
        "DIAMOND",
        "MASTER",
        "GRANDMASTER",
        "CHAMPION",
    ];
    let values = [-100, -99, -10, 0, 10, 99, 100];
    let mut numbers: Vec<(Option<i64>, Option<i64>)> = Vec::new();
    for p in values {
        numbers.push((Some(p), None));
        numbers.push((None, Some(p)));
        for c in values {
            numbers.push((Some(p), Some(c)));
        }
    }
    let mut widest = (0.0_f32, String::new());
    let mut widest_second = (0.0_f32, String::new());
    for tier in tiers {
        for division in 1..=5 {
            for &(p, c) in &numbers {
                let mut r = rank("Support", tier, division, 0, 0);
                r.progress_pct = p.and_then(num);
                r.progress_change = c.and_then(num);
                let g = with_rank(r);
                let lines = cell_lines(&g);
                assert_eq!(lines.len(), 2, "{lines:?}");
                let w = natural_width(&renderer, cell(&g));
                if w > widest.0 {
                    widest = (w, lines.join(" / "));
                }
                let second = line(&lines[1], SIZE_LABEL);
                if second > widest_second.0 {
                    widest_second = (second, lines[1].clone());
                }
            }
        }
    }
    let gm5 = line("Grandmaster 5", SIZE_META);
    let named: Vec<String> = [
        "-100% -100%",
        "-100% +100%",
        "100% +100%",
        "-100% progress",
        "100% progress",
        "-100% this game",
        "+100% this game",
    ]
    .iter()
    .map(|s| format!("{s} {}", line(s, SIZE_LABEL)))
    .collect();
    eprintln!(
        "widest cell {widest:?}; widest second line {widest_second:?}; Grandmaster 5 {gm5}, \
         Grandmaster 1 {}, Champion 5 {}, Champion 1 {}; second lines: {}",
        line("Grandmaster 1", SIZE_META),
        line("Champion 5", SIZE_META),
        line("Champion 1", SIZE_META),
        named.join(", "),
    );
    // The widest line is a Grandmaster one, past the old 72 px.
    assert!(widest.1.starts_with("Grandmaster"), "{widest:?}");
    assert!(gm5 > 72.0, "{gm5}");
    assert!(widest.0 >= gm5 && widest.0 >= widest_second.0, "{widest:?}");
    for grid in [Grid::Wide, Grid::Mid, Grid::Narrow] {
        let rank_w = rank_width(grid);
        assert!(
            widest.0 <= rank_w,
            "{grid:?}: {widest:?} is wider than the {rank_w} px Rank column"
        );
        assert!(
            widest_second.0 <= rank_w,
            "{grid:?}: {widest_second:?} is wider than the {rank_w} px Rank column"
        );
    }
}
