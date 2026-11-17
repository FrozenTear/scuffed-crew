//! Daemon tests: a stored stat never falls back below a value the same game
//! already stored when the capture gate holds a jump on that cell (weekly
//! review 2026-11-16, Wrecking Ball games with 30+ elims). A child of
//! `cell_trust.rs`, kept apart so both files stay under the size limit.
//!
//! `Some((v, true))` is a cell the digit reader does not vouch for (held);
//! `sure(v)` is a vouched read.

use super::*;

/// Rialto 13c3fc574a5aca40: rows 25, 29 (sure), 33 held and flagged, 33
/// again, then a final Tab with a vouched 35. The quiet final board makes
/// 29 to 35 a lone +6 jump from the gate's accepted 29 (past `JUMP_MAX`),
/// so the gate holds the sure read. From the stored 33, 35 is a +2 within
/// the rate cap, so the row stores 35, flagged, the same as an unvouched
/// 35 would, and never falls back to the accepted 29.
#[tokio::test]
async fn a_gate_held_sure_jump_stores_the_read_plausible_from_the_stored_one() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let held_33 = [
        Some((33, true)),
        sure(11),
        sure(2),
        sure(8656),
        sure(1263),
        sure(2574),
    ];
    let (_, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([25, 9, 2, 6862, 1048, 2029])),
            (111, all_sure([29, 9, 2, 8446, 1263, 2429])),
            (70, held_33),
            (7, held_33),
            (81, all_sure([35, 12, 2, 8707, 1263, 2674])),
        ],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    let elims: Vec<u32> = rows.iter().map(|r| r.elims).collect();
    assert_eq!(elims, vec![25, 29, 33, 33, 35], "stored elims");
    assert!(
        rows[4].suspect_fields.iter().any(|f| f == "e"),
        "the stored 35 is flagged: {:?}",
        rows[4].suspect_fields
    );
}

/// Lijiang Tower 087e7b495ba0f516 with every elims cell held from 33 on.
/// Busy Tabs store the unsure 33 and 38; the quiet repeat Tabs of the same
/// board keep them. The stored Lijiang series was
/// 22,28,28,33,28,28,38,28,28,28,28,28. This pins the #632 behaviour
/// (held repeat Tabs keep the stored unsure read) and passes before this
/// change too; it guards the real series, it is not a regression test for
/// the gate-held sure read.
#[tokio::test]
async fn held_repeat_tabs_keep_the_stored_unsure_read() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let held_33 = [
        Some((33, true)),
        sure(13),
        sure(9),
        sure(9961),
        sure(1736),
        sure(5632),
    ];
    let held_38 = [
        Some((38, true)),
        sure(17),
        sure(10),
        sure(10468),
        sure(1837),
        sure(5714),
    ];
    let (_, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([22, 8, 9, 8337, 1561, 4954])),
            (36, all_sure([28, 13, 9, 9318, 1611, 5332])),
            (13, all_sure([28, 13, 9, 9318, 1611, 5332])),
            (26, held_33),
            (4, held_33),
            (11, held_33),
            (34, held_38),
            (3, held_38),
            (9, held_38),
            (6, held_38),
            (28, held_38),
            (5, held_38),
        ],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    let elims: Vec<u32> = rows.iter().map(|r| r.elims).collect();
    assert_eq!(elims, vec![22, 28, 28, 33, 33, 33, 38, 38, 38, 38, 38, 38]);
}

/// Lijiang shape with one vouched read on a quiet repeat Tab: 28 accepted,
/// held 33 stored, then a sure 33 on the same quiet board is a lone +5
/// jump from the accepted 28 and the gate holds it. The row keeps 33,
/// flagged, and does not drop to 28.
#[tokio::test]
async fn a_gate_held_sure_repeat_does_not_drop_to_the_accepted_value() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let held_33 = [
        Some((33, true)),
        sure(13),
        sure(9),
        sure(9961),
        sure(1736),
        sure(5632),
    ];
    let (_, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([22, 8, 9, 8337, 1561, 4954])),
            (36, all_sure([28, 13, 9, 9318, 1611, 5332])),
            (26, held_33),
            (4, all_sure([33, 13, 9, 9961, 1736, 5632])),
        ],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    let elims: Vec<u32> = rows.iter().map(|r| r.elims).collect();
    assert_eq!(elims, vec![22, 28, 33, 33], "stored elims");
    assert!(
        rows[3].suspect_fields.iter().any(|f| f == "e"),
        "the kept 33 is flagged: {:?}",
        rows[3].suspect_fields
    );
}

/// Control, not a regression test: it passes before this change too.
/// After the stored unsure 33, a Tab whose own row is suspect (it may be
/// another player's) reads a higher sure 36. None of that row's digits
/// land in the stored row, so the gate-held sure read path never takes a
/// digit from a row that may not be the player's.
#[tokio::test]
async fn control_a_suspect_own_row_never_replaces_the_stored_unsure_read() {
    use detect::own_row::combine;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
    let suspect_row = Some(combine(Some(0), None, true));
    let held_33 = [
        Some((33, true)),
        sure(11),
        sure(2),
        sure(8656),
        sure(1263),
        sure(2574),
    ];
    o10_run_rows(
        &ctx,
        vec![
            (0, all_sure([25, 9, 2, 6862, 1048, 2029]), sure_row),
            (111, all_sure([29, 9, 2, 8446, 1263, 2429]), sure_row),
            (70, held_33, sure_row),
            (20, all_sure([36, 12, 2, 8707, 1263, 2674]), suspect_row),
        ],
    )
    .await;
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    let elims: Vec<u32> = rows.iter().map(|r| r.elims).collect();
    assert_eq!(&elims[..3], &[25, 29, 33]);
    assert!(
        !elims.contains(&36),
        "a suspect row's digit was stored: {elims:?}"
    );
}

/// The Rialto shape ending on a quiet board with 35 elims: a vouched 35
/// the gate holds as a lone jump stores the same row as an unvouched 35,
/// flagged both times.
#[tokio::test]
async fn a_vouched_and_an_unvouched_held_read_store_the_same() {
    let held_33 = [
        Some((33, true)),
        sure(11),
        sure(2),
        sure(8656),
        sure(1263),
        sure(2574),
    ];
    let held_35 = [
        Some((35, true)),
        sure(12),
        sure(2),
        sure(8707),
        sure(1263),
        sure(2674),
    ];
    let mut stored = Vec::new();
    for last in [all_sure([35, 12, 2, 8707, 1263, 2674]), held_35] {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (_, seen) = o10_run(
            &ctx,
            vec![
                (0, all_sure([25, 9, 2, 6862, 1048, 2029])),
                (111, all_sure([29, 9, 2, 8446, 1263, 2429])),
                (70, held_33),
                (7, held_33),
                (81, last),
            ],
        )
        .await;
        assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        let last = rows.last().unwrap();
        assert!(
            last.suspect_fields.iter().any(|f| f == "e"),
            "the final elims is flagged: {:?}",
            last.suspect_fields
        );
        stored.push(rows.iter().map(|r| r.elims).collect::<Vec<u32>>());
    }
    assert_eq!(stored[0], vec![25, 29, 33, 33, 35], "vouched 35");
    assert_eq!(stored[1], stored[0], "unvouched 35 stores the same");
}

/// A sure read below the accepted value (a monotonic hold) does not keep a
/// higher stored unsure read: the row falls back to the accepted 29,
/// flagged, rather than keep showing the stale 33.
#[tokio::test]
async fn a_held_lower_sure_read_does_not_keep_a_higher_unsure_read() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let held_33 = [
        Some((33, true)),
        sure(11),
        sure(2),
        sure(8656),
        sure(1263),
        sure(2574),
    ];
    let (_, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([25, 9, 2, 6862, 1048, 2029])),
            (111, all_sure([29, 9, 2, 8446, 1263, 2429])),
            (70, held_33),
            (20, all_sure([27, 11, 2, 8656, 1263, 2574])),
        ],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    let elims: Vec<u32> = rows.iter().map(|r| r.elims).collect();
    assert_eq!(elims, vec![25, 29, 33, 29], "stored elims");
    assert!(
        rows[3].suspect_fields.iter().any(|f| f == "e"),
        "the fallback 29 is flagged: {:?}",
        rows[3].suspect_fields
    );
}
