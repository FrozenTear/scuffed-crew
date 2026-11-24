use super::*;
use crate::games_rows::tests::game;
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
    assert_eq!(change_text(0), "0%");
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
        "Rank: Support, Role Queue, Diamond 2, 45% progress, +23% this game"
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
        "Rank: Role unknown, Gold 3, -15% progress, -15% this game"
    );
    g.queue_title = Some(queue("ROLE QUEUE"));
    assert_eq!(
        shown(&g).sentence(),
        "Rank: Role unknown, Role Queue, Gold 3, -15% progress, -15% this game"
    );
}

/// Open shows only with an open queue title, and once; a role icon read
/// under an open queue title is a misread the upload drops.
#[test]
fn open_queue_role_follows_the_upload() {
    let mut g = with_rank(rank("Open", "Silver", 1, 10, 0));
    assert_eq!(shown(&g).context(), ROLE_UNKNOWN);
    g.queue_title = Some(queue("OPEN QUEUE"));
    assert_eq!(shown(&g).context(), "Open queue");
    let mut g = with_rank(rank("Tank", "Silver", 1, 10, 0));
    g.queue_title = Some(queue("OPEN QUEUE"));
    assert_eq!(shown(&g).context(), "Role unknown, Open Queue");
    assert_eq!(context_label(Some("Tank"), None), "Tank");
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
    assert_eq!(cell_lines(&numbers), ["-15%"]);
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
            vec!["Platinum 4", "80% -15%"],
            vec!["Gold"],
            vec!["-"],
            vec!["-"],
        ]
    );
}
