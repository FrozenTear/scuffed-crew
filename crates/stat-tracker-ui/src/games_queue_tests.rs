use super::*;
use crate::games_detail::{map_chips, queue_value, queue_warning};
use crate::games_rank::RankShown;
use stat_tracker::model::{QUEUE_TITLE_WARNING, QueueTitle, RankText, RankUpdate};
use stat_tracker::storage::PersonalMatch;

/// A stored game the upload sends (a known map and hero).
fn stored() -> PersonalMatch {
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
        crate::fixtures::dt(2026, 11, 27, 20, 0),
    )
}

fn title(value: &str, confidence: f32) -> QueueTitle {
    QueueTitle {
        value: Some(value.into()),
        confidence,
        ..QueueTitle::default()
    }
}

/// A rank screen read: the role line at `confidence`, and a tier.
fn rank(role: Option<&str>, confidence: f32) -> RankUpdate {
    RankUpdate {
        role: role.map(|r| RankText {
            value: r.into(),
            confidence,
        }),
        role_rank: Some(RankText {
            value: "Diamond".into(),
            confidence: 0.9,
        }),
        ..RankUpdate::default()
    }
}

/// The stored game with this title, board size and rank read.
fn stored_with(
    t: Option<QueueTitle>,
    team_size: Option<u8>,
    r: Option<RankUpdate>,
) -> PersonalMatch {
    let mut m = stored();
    m.queue_title = t;
    m.team_size = team_size;
    m.rank_update = r;
    m
}

/// Give a hand-built test game this title, board size and rank read, with
/// the queue fields `Game::from_match` derives from them (the upload's
/// own rule), as the app gets them from the store.
pub(crate) fn set_queue(
    g: &mut Game,
    t: Option<QueueTitle>,
    team_size: Option<u8>,
    r: Option<RankUpdate>,
) {
    let built = Game::from_match(&stored_with(t, team_size, r));
    g.queue_title = built.queue_title;
    g.rank_update = built.rank_update;
    g.rank_slot = built.rank_slot;
    g.team_size = built.team_size;
}

/// What the upload sends as `rank.queue.value` for `m`, read from the
/// real request body, or `None` when it sends no `rank` object.
fn sent_queue(m: &PersonalMatch) -> Option<String> {
    let request = stat_tracker::sync::upload_request(std::slice::from_ref(m), &[]);
    let body = serde_json::to_value(request).unwrap();
    let sent = &body["matches"][0];
    assert!(sent.is_object(), "the row is sent: {body}");
    let rank = sent.get("rank").filter(|r| !r.is_null())?;
    let value = rank["queue"]["value"].as_str();
    Some(value.expect("every rank object has a queue").to_string())
}

/// The words Games must show for a wire value, written out here apart from
/// [`label`] so the two are checked against each other.
fn want_words(wire: Option<&str>, team_size: Option<u8>) -> Option<&'static str> {
    Some(match (wire?, team_size) {
        ("role_5v5", _) => "5V5 ROLE QUEUE",
        ("open_5v5", _) => "5V5 OPEN QUEUE",
        ("role_6v6", _) => "6V6 ROLE QUEUE",
        ("open_6v6", _) => "6V6 OPEN QUEUE",
        ("unknown", Some(5)) => "5V5 queue unknown",
        ("unknown", Some(6)) => "6V6 queue unknown",
        ("unknown", _) => "Queue unknown",
        (other, _) => panic!("wire value {other:?}"),
    })
}

/// The cases: titles of each queue in the game's own spellings and word
/// orders, titles that name no queue or both, titles of modes that are not
/// Competitive, each read confidently and weakly; every board size the
/// store can hold; and no rank read, role icons, an `Open` role line, and
/// a role line too weak to keep.
fn cases() -> Vec<PersonalMatch> {
    let titles = [
        None,
        Some("ROLE QUEUE"),
        Some("OPEN QUEUE"),
        Some("COMPETITIVE: ROLE QUEUE"),
        Some("COMPETITIVE: 6V6 OPEN QUEUE"),
        Some("COMPETITIVE: 6V6 ROLE QUEUE"),
        Some("ROLE QUEUE 6V6"),
        Some("5V5 ROLE QUEUE"),
        Some("QUEUE"),
        Some("ROLE OPEN QUEUE"),
        Some("ROLE QUEUE: QUICK PLAY"),
        Some("OPEN QUEUE: 6V6 QUICK PLAY"),
        Some("UNRANKED: ROLE QUEUE"),
        Some("ARCADE"),
        Some("STADIUM: OPEN QUEUE"),
        Some("CUSTOM GAME: ROLE QUEUE"),
    ];
    let ranks = [
        None,
        Some(rank(Some("Tank"), 0.9)),
        Some(rank(Some("Support"), 0.9)),
        Some(rank(Some("Open"), 0.9)),
        Some(rank(Some("Tank"), 0.0)),
        Some(rank(None, 0.9)),
        // A rank read whose only field the upload drops (a confidence it
        // does not keep, a tier past 64 characters): no rank screen field
        // is sent, so it is no Competitive evidence by itself.
        Some(RankUpdate {
            role: Some(RankText {
                value: "Tank".into(),
                confidence: 0.0,
            }),
            ..RankUpdate::default()
        }),
        Some(RankUpdate {
            role_rank: Some(RankText {
                value: "D".repeat(65),
                confidence: 0.9,
            }),
            ..RankUpdate::default()
        }),
    ];
    let mut out = Vec::new();
    for t in titles {
        for confidence in [0.95, 0.3] {
            for size in [None, Some(5), Some(6), Some(4)] {
                for r in &ranks {
                    out.push(stored_with(
                        t.map(|t| title(t, confidence)),
                        size,
                        r.clone(),
                    ));
                }
            }
        }
    }
    out
}

/// The Games queue words and the upload's `rank.queue` agree for every
/// case, through the real request body: the chip, the Map cell and the
/// Rank cell's tooltip name the queue the site gets, and a game the upload
/// sends no `rank` for shows no queue anywhere. Every label is reached.
#[test]
fn the_games_queue_agrees_with_the_upload_for_every_case() {
    let mut seen: Vec<Option<&str>> = Vec::new();
    for m in cases() {
        let at = format!(
            "{:?} size {:?} rank {:?}",
            m.queue_title, m.team_size, m.rank_update
        );
        let want = want_words(
            sent_queue(&m).as_deref(),
            m.team_size.filter(|n| *n == 5 || *n == 6),
        );
        let g = Game::from_match(&m);
        assert_eq!(queue_value(&g), want, "{at}");
        let chips = map_chips(&g);
        match want {
            Some(w) => assert_eq!(chips.last().map(String::as_str), Some(w), "{at}"),
            None => assert!(!chips.iter().any(|c| ALL.contains(&c.as_str())), "{at}"),
        }
        if let Some(shown) = RankShown::from_game(&g) {
            assert_eq!(shown.queue.as_deref(), want, "{at}");
            if let Some(w) = want {
                assert!(shown.sentence().contains(w), "{at}: {}", shown.sentence());
            }
        }
        if !seen.contains(&want) {
            seen.push(want);
        }
    }
    for w in ALL.map(Some).into_iter().chain([None]) {
        assert!(seen.contains(&w), "no case shows {w:?}: {seen:?}");
    }
}

/// The cases FrozenTear named, one by one: Games tells 6v6 role queue,
/// 5v5 role queue, open queue, an unknown queue and a 6v6 game whose queue
/// is unknown apart. The bare `COMPETITIVE: ROLE QUEUE` title is the same
/// for 5 and 6 per team; only the board size tells them apart.
#[test]
fn games_tells_each_queue_apart() {
    let comp_role = || Some(title("COMPETITIVE: ROLE QUEUE", 0.95));
    let tank = || Some(rank(Some("Tank"), 0.9));
    let named = |t, size, r| queue_value(&Game::from_match(&stored_with(t, size, r)));
    assert_eq!(named(comp_role(), Some(6), tank()), Some("6V6 ROLE QUEUE"));
    assert_eq!(named(comp_role(), Some(5), tank()), Some("5V5 ROLE QUEUE"));
    let comp_open = Some(title("COMPETITIVE: OPEN QUEUE", 0.95));
    assert_eq!(named(comp_open, Some(5), None), Some("5V5 OPEN QUEUE"));
    let open6 = Some(title("COMPETITIVE: 6V6 OPEN QUEUE", 0.95));
    assert_eq!(named(open6, None, None), Some("6V6 OPEN QUEUE"));
    // No title and 6 per team: a role icon decides role queue 6v6, but an
    // `Open` role line alone decides nothing (FrozenTear's R1).
    assert_eq!(named(None, Some(6), tank()), Some("6V6 ROLE QUEUE"));
    let open_line = || Some(rank(Some("Open"), 0.9));
    assert_eq!(named(None, Some(6), open_line()), Some("6V6 queue unknown"));
    assert_eq!(named(None, Some(5), open_line()), Some("5V5 queue unknown"));
    assert_eq!(named(None, None, open_line()), Some("Queue unknown"));
    // A queue title with no board size: unknown, not guessed.
    assert_eq!(named(comp_role(), None, None), Some("Queue unknown"));
    // The five labels a member must tell apart are all different.
    let mut five = [ROLE_6V6, ROLE_5V5, OPEN_5V5, UNKNOWN, UNKNOWN_6V6];
    five.sort_unstable();
    assert!(five.windows(2).all(|w| w[0] != w[1]), "{five:?}");
}

/// A title naming a mode that is not Competitive shows no queue, even with
/// a rank screen read and a board size, and the Rank cell shows "-" for
/// it as before. The readers off (no title, no rank) show nothing at all:
/// no chip and no warning.
#[test]
fn no_queue_for_other_modes_or_with_the_readers_off() {
    for word in ["QUICK", "UNRANKED", "ARCADE", "STADIUM", "CUSTOM"] {
        for t in [
            format!("ROLE QUEUE: {word} PLAY"),
            format!("OPEN QUEUE: 6V6 {word} PLAY"),
            format!("{word}: ROLE QUEUE"),
        ] {
            for size in [Some(5), Some(6)] {
                let m = stored_with(Some(title(&t, 0.95)), size, Some(rank(Some("Tank"), 0.9)));
                let g = Game::from_match(&m);
                assert_eq!(queue_value(&g), None, "{t} {size:?}");
                assert_eq!(map_chips(&g), ["Control"], "{t} {size:?}");
                assert_eq!(queue_warning(&g), None, "{t} {size:?}");
                assert!(RankShown::from_game(&g).is_none(), "{t} {size:?}");
            }
        }
    }
    // A rank read whose only field the upload drops sends no `rank`: no
    // queue, as the upload, even on a 6-a-side board.
    let dropped = RankUpdate {
        role: Some(RankText {
            value: "Tank".into(),
            confidence: 0.0,
        }),
        ..RankUpdate::default()
    };
    for size in [None, Some(5), Some(6)] {
        let m = stored_with(None, size, Some(dropped.clone()));
        assert_eq!(sent_queue(&m), None, "{size:?}");
        assert_eq!(queue_value(&Game::from_match(&m)), None, "{size:?}");
    }
    for size in [None, Some(5), Some(6)] {
        let g = Game::from_match(&stored_with(None, size, None));
        assert_eq!(queue_value(&g), None, "{size:?}");
        assert_eq!(map_chips(&g), ["Control"], "{size:?}");
        assert_eq!(queue_warning(&g), None, "{size:?}");
    }
}

/// The warning under the open game's map line ("This game's queue is not
/// known") shows only for a weak title read whose queue the upload does
/// not know. When a 6-a-side role icon still gives a known queue, the
/// chip names it and the warning stays away, so the two never contradict.
#[test]
fn the_queue_warning_never_contradicts_a_known_queue() {
    let weak = || Some(title("ROLE QUEUE", 0.3));
    let g = Game::from_match(&stored_with(
        weak(),
        Some(6),
        Some(rank(Some("Damage"), 0.9)),
    ));
    assert_eq!(queue_value(&g), Some(ROLE_6V6));
    assert_eq!(queue_warning(&g), None);
    // Unknown queue with a weak title: the chip says unknown and the
    // warning says why.
    let g = Game::from_match(&stored_with(weak(), Some(5), Some(rank(Some("Tank"), 0.9))));
    assert_eq!(queue_value(&g), Some(UNKNOWN_5V5));
    assert_eq!(queue_warning(&g), Some(QUEUE_TITLE_WARNING));
    // A weak title and nothing else: no rank is sent, so no chip; the
    // warning still says the queue is not known.
    let g = Game::from_match(&stored_with(weak(), Some(6), None));
    assert_eq!(queue_value(&g), None);
    assert_eq!(queue_warning(&g), Some(QUEUE_TITLE_WARNING));
}

/// The Rank tooltip names the queue in the chip's words, and the Open
/// role with an open queue names open queue once.
#[test]
fn the_rank_tooltip_names_the_queue_like_the_chip() {
    let g = Game::from_match(&stored_with(
        Some(title("COMPETITIVE: ROLE QUEUE", 0.95)),
        Some(6),
        Some(rank(Some("Tank"), 0.9)),
    ));
    let shown = RankShown::from_game(&g).unwrap();
    assert_eq!(shown.sentence(), "Rank: Tank, 6V6 ROLE QUEUE, Diamond");
    let g = Game::from_match(&stored_with(
        Some(title("OPEN QUEUE", 0.95)),
        Some(6),
        Some(rank(Some("Open"), 0.9)),
    ));
    assert_eq!(
        RankShown::from_game(&g).unwrap().context(),
        "6V6 OPEN QUEUE"
    );
    let g = Game::from_match(&stored_with(None, Some(6), Some(rank(Some("Open"), 0.9))));
    assert_eq!(
        RankShown::from_game(&g).unwrap().sentence(),
        "Rank: Role unknown, 6V6 queue unknown, Diamond"
    );
}

/// The label for each wire value is fixed, whatever the board says beside
/// a known queue (the upload already took the size into account).
#[test]
fn a_known_queue_ignores_the_board_size() {
    use scuffed_types::api::RankQueue;
    for size in [None, Some(4), Some(5), Some(6)] {
        assert_eq!(label(RankQueue::Role5v5, size), ROLE_5V5);
        assert_eq!(label(RankQueue::Open5v5, size), OPEN_5V5);
        assert_eq!(label(RankQueue::Role6v6, size), ROLE_6V6);
        assert_eq!(label(RankQueue::Open6v6, size), OPEN_6V6);
    }
    assert_eq!(label(RankQueue::Unknown, Some(4)), UNKNOWN);
}

/// Give a hand-built test game the queue fields that make Games name it
/// `words` (one of [`ALL`]).
pub(crate) fn set_label(g: &mut Game, words: &str) {
    use scuffed_types::api::RankQueue;
    let queues = [
        RankQueue::Role5v5,
        RankQueue::Open5v5,
        RankQueue::Role6v6,
        RankQueue::Open6v6,
        RankQueue::Unknown,
    ];
    let (queue, size) = queues
        .into_iter()
        .flat_map(|q| [None, Some(5), Some(6)].map(|s| (q, s)))
        .find(|&(q, s)| label(q, s) == words)
        .unwrap_or_else(|| panic!("no queue is named {words:?}"));
    g.rank_slot = Some(stat_tracker::rank_queue::RankSlot {
        queue,
        queue_confidence: None,
        role: None,
        rank_read: false,
    });
    g.team_size = size;
}

/// The widest label as a Map cell chip draws it.
pub(crate) fn widest() -> &'static str {
    ALL.into_iter()
        .max_by(|a, b| {
            crate::games_map_fit::chip_width(a).total_cmp(&crate::games_map_fit::chip_width(b))
        })
        .unwrap()
}
