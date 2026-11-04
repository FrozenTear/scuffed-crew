use super::*;
use crate::daemon::pregame_dump::PregameShot;

fn select_save(name: &'static str) -> Vec<PregameOp<()>> {
    vec![PregameOp::Save(PregameShot {
        folder: "20261104_200000".into(),
        name,
        frame: (),
    })]
}

fn ok() -> ReaderResult {
    ReaderResult::Status(BansPicksStatus::Ok)
}

#[test]
fn focus_paused_through_hero_select() {
    let mut trail = PregameTrail::default();
    trail.screen(Seen::Phase(PregamePhase::MapVote));
    trail.screen(Seen::Phase(PregamePhase::HeroBan));
    // Tabbed out: no more ticks are read, the pre-game was up.
    trail.focus_paused(true);
    trail.focus_paused(true);
    let reader = ReaderResult::Status(BansPicksStatus::Partial {
        missing: vec![BansPicksMissing::HeroSelect],
    });
    let summary = trail.finish("s1", &reader).expect("first record");
    assert!(!summary.select_seen);
    assert!(!summary.select_frame);
    assert!(!summary.picks_stored);
    assert_eq!(
        summary.reason,
        Some(MissReason::FocusPaused { in_pregame: true })
    );
}

#[test]
fn focus_paused_before_any_pregame_screen() {
    let mut trail = PregameTrail::default();
    trail.focus_paused(false);
    let summary = trail.summary(&ReaderResult::NoFrames("no frames"));
    assert_eq!(
        summary.reason,
        Some(MissReason::FocusPaused { in_pregame: false })
    );
}

#[test]
fn hero_select_never_detected() {
    let mut trail = PregameTrail::default();
    trail.screen(Seen::Phase(PregamePhase::HeroBan));
    trail.screen(Seen::BanResults);
    trail.screen(Seen::Loading);
    let summary = trail.summary(&ReaderResult::Status(BansPicksStatus::Partial {
        missing: vec![BansPicksMissing::HeroSelect],
    }));
    assert!(!summary.select_seen);
    assert!(!summary.picks_stored);
    assert_eq!(summary.reason, Some(MissReason::SelectNeverDetected));
}

#[test]
fn detected_and_stored() {
    let mut trail = PregameTrail::default();
    trail.screen(Seen::Phase(PregamePhase::HeroSelect));
    trail.shots(&select_save(PregamePhase::HeroSelect.first_name()));
    trail.shots(&select_save(PregamePhase::HeroSelect.last_name()));
    let summary = trail.finish("s1", &ok()).expect("first record");
    assert_eq!(
        summary,
        PregameSummary {
            select_seen: true,
            select_frame: true,
            picks_stored: true,
            focus_paused: false,
            reason: None,
        }
    );
}

#[test]
fn a_focus_pause_does_not_hide_stored_picks() {
    let mut trail = PregameTrail::default();
    trail.focus_paused(true);
    trail.screen(Seen::SelectTitle);
    trail.shots(&select_save(PregamePhase::HeroSelect.last_name()));
    let summary = trail.summary(&ok());
    assert_eq!(summary.reason, None);
    assert!(summary.focus_paused);
}

#[test]
fn select_seen_with_no_frame_kept_gives_the_skip_reason() {
    let mut trail = PregameTrail::default();
    trail.screen(Seen::Phase(PregamePhase::HeroSelect));
    trail.select_skipped("no open game folder");
    let summary = trail.summary(&ReaderResult::NoFrames("no frames"));
    assert_eq!(
        summary.reason,
        Some(MissReason::SelectNotKept("no open game folder"))
    );
}

#[test]
fn a_kept_frame_the_reader_skipped_gives_the_reader_reason() {
    let mut trail = PregameTrail::default();
    trail.screen(Seen::Phase(PregamePhase::HeroSelect));
    trail.shots(&select_save(PregamePhase::HeroSelect.last_name()));
    let summary = trail.summary(&ReaderResult::Status(BansPicksStatus::FrameSize {
        width: 1920,
        height: 1080,
    }));
    assert_eq!(
        summary.reason,
        Some(MissReason::ReaderSkipped(
            "wrong resolution 1920x1080".into()
        ))
    );
    let off = trail.summary(&ReaderResult::Off);
    assert_eq!(
        off.reason,
        Some(MissReason::ReaderSkipped("reader off".into()))
    );
}

#[test]
fn one_summary_per_session_and_the_record_starts_over() {
    let mut trail = PregameTrail::default();
    trail.screen(Seen::Phase(PregamePhase::HeroSelect));
    assert!(trail.finish("s1", &ok()).is_some());
    assert!(trail.finish("s1", &ok()).is_none());
    let next = trail.finish("s2", &ReaderResult::Off).expect("next game");
    assert!(!next.select_seen);
    assert_eq!(next.reason, Some(MissReason::SelectNeverDetected));
}

#[test]
fn a_new_game_folder_drops_the_last_attempt() {
    let mut trail = PregameTrail::default();
    trail.focus_paused(true);
    trail.screen(Seen::Phase(PregamePhase::HeroSelect));
    trail.new_game();
    let summary = trail.summary(&ReaderResult::Off);
    assert!(!summary.select_seen);
    assert_eq!(summary.reason, Some(MissReason::SelectNeverDetected));
}

#[test]
fn the_summary_file_keeps_the_last_lines_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(SUMMARY_FILE);
    for i in 0..7 {
        append_capped(&path, &format!("line {i}"), 5).expect("append");
    }
    let text = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines, ["line 2", "line 3", "line 4", "line 5", "line 6"]);
    assert!(!path.with_extension("log.tmp").exists());
}

#[test]
fn the_summary_file_is_capped_at_its_keep() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(SUMMARY_FILE);
    for i in 0..SUMMARY_KEEP + 3 {
        append_capped(&path, &format!("line {i}"), SUMMARY_KEEP).expect("append");
    }
    let text = std::fs::read_to_string(&path).expect("read");
    assert_eq!(text.lines().count(), SUMMARY_KEEP);
    assert_eq!(text.lines().next(), Some("line 3"));
}

#[test]
fn a_summary_write_that_fails_does_not_panic() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A directory where the file should be: the read fails.
    let path = dir.path().join(SUMMARY_FILE);
    std::fs::create_dir(&path).expect("mkdir");
    let summary = PregameTrail::default().summary(&ReaderResult::Off);
    assert!(append_capped(&path, "x", SUMMARY_KEEP).is_err());
    write_summary(&path, "s1", &summary);
}

#[test]
fn the_summary_line_is_one_line() {
    let mut trail = PregameTrail::default();
    trail.focus_paused(true);
    let summary = trail.summary(&ReaderResult::Off);
    let at = chrono::DateTime::parse_from_rfc3339("2026-11-04T20:00:00Z")
        .expect("time")
        .with_timezone(&chrono::Utc);
    assert_eq!(
        summary_line(at, "s1", &summary),
        "2026-11-04T20:00:00Z session=s1 hero_select_seen=false hero_select_frame=false \
         picks_stored=false focus_paused=true reason=focus gate paused the poll during the pre-game"
    );
}

#[test]
fn reasons_read_without_em_dashes() {
    for reason in [
        MissReason::FocusPaused { in_pregame: true },
        MissReason::FocusPaused { in_pregame: false },
        MissReason::SelectNeverDetected,
        MissReason::SelectNotKept("x"),
        MissReason::ReaderSkipped("y".into()),
    ] {
        assert!(!reason.to_string().contains('\u{2014}'));
    }
}
