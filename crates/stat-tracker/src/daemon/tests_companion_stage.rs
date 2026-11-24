//! Tests for `daemon/companion_stage.rs`.

use super::companion_stage::{publish, stage_of};
use crate::main_tests::fn_source;
use stat_tracker::companion_stage::{CompanionStage, StageKind};
use stat_tracker::detect::GamePhase;

#[test]
fn each_game_phase_maps_to_its_stage() {
    let raw = vec![
        "HAVANA".to_string(),
        "KING'S ROW".into(),
        "KINGS ROW".into(),
        "ILIOS".into(),
    ];
    let vote = stage_of(&GamePhase::MapVote { maps: raw }, 10).unwrap();
    assert_eq!(
        vote,
        CompanionStage::new(
            StageKind::MapVote,
            vec!["Havana".into(), "King's Row".into(), "Ilios".into()],
            10
        )
    );
    // A vote read with no names is still the vote, with no maps.
    let blank = stage_of(&GamePhase::MapVote { maps: Vec::new() }, 11).unwrap();
    assert_eq!(blank.kind, StageKind::MapVote);
    assert!(blank.maps.is_empty());
    for (phase, kind) in [
        (GamePhase::HeroSelect, StageKind::HeroSelect),
        (GamePhase::HeroBan, StageKind::HeroBan),
        (GamePhase::InGame, StageKind::InGame),
    ] {
        assert_eq!(
            stage_of(&phase, 12),
            Some(CompanionStage::new(kind, Vec::new(), 12))
        );
    }
    assert_eq!(stage_of(&GamePhase::Unknown, 13), None);
}

#[test]
fn vote_maps_are_canonical_display_names_deduped() {
    let watchpoint = stage_of(
        &GamePhase::MapVote {
            maps: vec!["WATCHPOINT".into(), "GIBRALTAR".into()],
        },
        1,
    )
    .unwrap();
    assert_eq!(watchpoint.maps, vec!["Watchpoint: Gibraltar"]);

    // Circuit Royal (not "Royale"), Esperanca (ascii and accented), and
    // a split Throne of Anubis, all from a real detector list.
    let throne = stage_of(
        &GamePhase::MapVote {
            maps: vec![
                "CIRCUIT ROYAL".into(),
                "ESPERANCA".into(),
                "ESPERANÇA".into(),
                "THRONE".into(),
                "ANUBIS".into(),
            ],
        },
        2,
    )
    .unwrap();
    assert_eq!(
        throne.maps,
        vec![
            "Circuit Royal".to_string(),
            "Esperanca".into(),
            "Throne of Anubis".into(),
        ]
    );
}

#[test]
fn an_unknown_phase_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    publish(dir.path(), &GamePhase::Unknown);
    assert!(!stat_tracker::ocr_status::status_path(dir.path()).exists());
}

#[test]
fn publish_stamps_unix_seconds_from_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let before = chrono::Utc::now().timestamp();
    publish(dir.path(), &GamePhase::HeroBan);
    let after = chrono::Utc::now().timestamp();
    let held = stat_tracker::ocr_status::read_status(dir.path())
        .unwrap()
        .companion_stage
        .unwrap();
    assert_eq!(held.kind, StageKind::HeroBan);
    assert!(
        before <= held.since && held.since == held.seen && held.seen <= after,
        "before={before} since={} seen={} after={after}",
        held.since,
        held.seen
    );
}

#[test]
fn run_loop_publishes_the_companion_stage_before_the_boundary_tick() {
    let body = fn_source("run_loop");
    let publish = body
        .find("daemon::companion_stage::publish(data_dir, &phase)")
        .expect("run_loop publishes the companion stage");
    let tick = body
        .find("apply_poll_tick(")
        .expect("run_loop calls apply_poll_tick");
    assert!(
        publish < tick,
        "the companion publish must come before apply_poll_tick"
    );
}
