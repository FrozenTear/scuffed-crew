//! Tests for `snapshot_policy.rs`.

use std::time::{Duration, Instant};

use super::snapshot_policy::*;

#[test]
fn a_change_during_a_write_stays_dirty() {
    let t0 = Instant::now();
    let mut p = SnapshotPolicy::new();
    p.mark(t0, false);
    let seen = p.generation();
    p.mark(t0, false);
    p.wrote(t0, seen);
    assert!(p.dirty());
    assert!(p.due(t0 + DEBOUNCE, true));
}

#[test]
fn nothing_dirty_never_writes() {
    let p = SnapshotPolicy::new();
    assert!(!p.due(Instant::now(), true));
}

#[test]
fn a_running_gui_gets_a_debounced_write() {
    let t0 = Instant::now();
    let mut p = SnapshotPolicy::new();
    p.mark(t0, false);
    assert!(p.due(t0, true), "first write is immediate");
    p.wrote(t0, p.generation());
    p.mark(t0, false);
    assert!(!p.due(t0 + Duration::from_millis(500), true), "debounced");
    assert!(p.due(t0 + DEBOUNCE, true));
}

#[test]
fn no_gui_means_no_write_for_plain_changes() {
    let t0 = Instant::now();
    let mut p = SnapshotPolicy::new();
    for i in 0..300u64 {
        p.mark(t0 + Duration::from_secs(i), false);
        assert!(!p.due(t0 + Duration::from_secs(i), false));
    }
    assert!(p.dirty(), "the change waits for a GUI or an owed write");
    assert!(
        p.due(t0 + Duration::from_secs(300), true),
        "a GUI that opens gets it"
    );
}

#[test]
fn a_finished_game_is_written_without_a_gui_after_it_settles() {
    let t0 = Instant::now();
    let mut p = SnapshotPolicy::new();
    p.mark(t0, true);
    assert!(!p.due(t0 + Duration::from_secs(1), false), "settling");
    p.mark(t0 + Duration::from_secs(2), false);
    assert!(p.due(t0 + MUST_SETTLE, false));
    p.wrote(t0 + MUST_SETTLE, p.generation());
    p.mark(t0 + Duration::from_secs(10), false);
    assert!(
        !p.due(t0 + Duration::from_secs(20), false),
        "the owed write was paid"
    );
}

/// The daemon's snapshot functions as wired in `main.rs`, on their own
/// state (not the process-wide one) and with the GUI probe given.
mod wiring {
    use crate::storage::LocalStore;

    type State = std::sync::Mutex<super::SnapshotPolicy>;

    async fn store() -> (tempfile::TempDir, LocalStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore::open(dir.path()).await.unwrap();
        (dir, store)
    }

    fn file(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("live_snapshot.json")
    }

    #[tokio::test]
    async fn with_no_gui_changes_and_sync_progress_write_nothing() {
        let (dir, store) = store().await;
        let state = State::new(super::SnapshotPolicy::new());
        let no_gui = || false;
        for _ in 0..300 {
            crate::refresh_snapshot_in(&state, &no_gui, &store, dir.path()).await;
            crate::refresh_snapshot_force_in(&state, &no_gui, &store, dir.path()).await;
            crate::flush_snapshot_if_due_in(&state, &no_gui, &store, dir.path()).await;
        }
        assert!(!file(&dir).exists(), "no GUI: no snapshot work");
        // Shutdown writes what is dirty, GUI or not.
        crate::flush_snapshot_if_dirty_in(&state, &store, dir.path()).await;
        assert!(file(&dir).exists());
    }

    #[tokio::test]
    async fn a_running_gui_gets_the_change_and_a_gui_that_opens_later_too() {
        let (dir, store) = store().await;
        let state = State::new(super::SnapshotPolicy::new());
        let gui = std::sync::atomic::AtomicBool::new(false);
        let probe = || gui.load(std::sync::atomic::Ordering::Relaxed);
        crate::refresh_snapshot_in(&state, &probe, &store, dir.path()).await;
        assert!(!file(&dir).exists());
        gui.store(true, std::sync::atomic::Ordering::Relaxed);
        crate::flush_snapshot_if_due_in(&state, &probe, &store, dir.path()).await;
        assert!(
            file(&dir).exists(),
            "the cmd tick writes for a GUI that opened"
        );
        std::fs::remove_file(file(&dir)).unwrap();
        crate::flush_snapshot_if_due_in(&state, &probe, &store, dir.path()).await;
        assert!(!file(&dir).exists(), "nothing dirty: nothing written");
    }

    #[tokio::test]
    async fn a_finished_game_is_written_with_no_gui_once_it_settles() {
        let (dir, store) = store().await;
        let state = State::new(super::SnapshotPolicy::new());
        let no_gui = || false;
        crate::refresh_snapshot_game_finished_in(&state, &no_gui, &store, dir.path()).await;
        assert!(!file(&dir).exists(), "settling");
        tokio::time::sleep(super::MUST_SETTLE + std::time::Duration::from_millis(100)).await;
        crate::flush_snapshot_if_due_in(&state, &no_gui, &store, dir.path()).await;
        assert!(file(&dir).exists(), "owed write");
    }
}
