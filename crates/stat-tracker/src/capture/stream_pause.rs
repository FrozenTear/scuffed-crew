//! Pause the screen stream between grabs.
//!
//! The portal stream runs only while something holds it: a grab in
//! [`super::screencast`], or a Tab key press waiting for its grab. With no
//! hold it is set inactive, so the compositor stops rendering frames the
//! tracker would only throw away. Polls and Tabs do not grab while
//! Overwatch is not running, so a share that stays open then stays paused.
//!
//! A Tab press takes a key hold at the key event, before the session
//! choice and the 300 ms overlay delay, so the compositor has that time to
//! resume. The Tab grab takes over: its grab hold releases every key hold,
//! and the stream pauses again once that grab has its frame or gives up.
//! A poll grab does not release key holds, so a Tab pressed during a poll
//! keeps the stream running after the poll is dropped.
//!
//! The resume itself starts [`tab_resume_at`]: `300 ms - one frame period`
//! after the press (50 ms at 4 fps, at once at 3 fps or less). Mutter, niri
//! and KWin before 6.8 send a frame right after a resume and the next one a
//! frame period later. Resumed at the press, that second frame would land
//! at about 260 ms, inside the overlay delay, and the Tab would wait for the
//! third (about 510 ms). Resumed 50 ms later the second frame lands just
//! after 300 ms. KWin 6.8 paces on a fixed clock grid, so the start does
//! not move its frames.
//!
//! `screencast_pause_between_grabs = false` in config.toml, or
//! `STAT_TRACKER_SCREENCAST_PAUSE=0`, turns this off: the stream then runs
//! the whole time, as in rc.6. The per-stream fail-safe in [`super::pw`]
//! does the same for one stream when a resume brings no frame, and after
//! [`pw::PAUSE_TRIPS_LIMIT`] tripped streams for the rest of the process.

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::framebuf::TAB_OVERLAY_AFTER_PRESS_MS;
use super::pw;

#[derive(Debug, Default)]
struct Ctl {
    enabled: bool,
    grab_holds: usize,
    key_holds: usize,
    /// Bumped when a Tab grab takes over the key holds. A key hold from an
    /// older epoch has already been released.
    key_epoch: u64,
    current: Option<Weak<pw::Handle>>,
    limit_logged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Key,
    Grab,
}

impl Ctl {
    /// The stream should run.
    fn wanted(&self) -> bool {
        !self.enabled || self.grab_holds > 0 || self.key_holds > 0
    }

    fn acquire(&mut self, kind: Kind, takes_over_keys: bool) -> u64 {
        match kind {
            Kind::Key => self.key_holds += 1,
            Kind::Grab => {
                self.grab_holds += 1;
                if takes_over_keys {
                    self.key_holds = 0;
                    self.key_epoch = self.key_epoch.wrapping_add(1);
                }
            }
        }
        self.key_epoch
    }

    fn release(&mut self, kind: Kind, epoch: u64) {
        match kind {
            Kind::Grab => self.grab_holds = self.grab_holds.saturating_sub(1),
            Kind::Key if epoch == self.key_epoch => {
                self.key_holds = self.key_holds.saturating_sub(1);
            }
            Kind::Key => {}
        }
    }

    /// Pass the wanted state to the live stream. Returns the strong
    /// reference so the caller drops it after unlocking: if a retired
    /// stream's last reference ends here, its Drop joins the PipeWire
    /// thread, which must not happen under this lock.
    #[must_use]
    fn apply(&self) -> Option<Arc<pw::Handle>> {
        let handle = self.current.as_ref().and_then(Weak::upgrade)?;
        handle.set_active(self.wanted());
        Some(handle)
    }

    /// `trips` is how many streams have tripped the fail-safe so far.
    fn register(&mut self, handle: &Arc<pw::Handle>, trips: u32) {
        if self.enabled {
            if trips < pw::PAUSE_TRIPS_LIMIT {
                if !handle.enable_pausing() && !self.limit_logged {
                    self.limit_logged = true;
                    tracing::info!(
                        "screencast pause between grabs is off: it needs PipeWire 1.4 or newer"
                    );
                }
            } else if !self.limit_logged {
                self.limit_logged = true;
                tracing::info!(
                    "screencast pause between grabs stays off: the fail-safe tripped on two streams"
                );
            }
        }
        self.current = Some(Arc::downgrade(handle));
        // The caller holds `handle`, so this is never the last reference.
        drop(self.apply());
    }
}

static CTL: Mutex<Ctl> = Mutex::new(Ctl {
    enabled: true,
    grab_holds: 0,
    key_holds: 0,
    key_epoch: 0,
    current: None,
    limit_logged: false,
});

fn ctl(slot: &Mutex<Ctl>) -> std::sync::MutexGuard<'_, Ctl> {
    slot.lock().unwrap_or_else(|err| err.into_inner())
}

/// Keeps the stream running while it lives.
#[must_use = "the stream pauses again when the hold is dropped"]
pub struct StreamHold {
    slot: &'static Mutex<Ctl>,
    kind: Kind,
    epoch: u64,
}

impl Drop for StreamHold {
    fn drop(&mut self) {
        let handle = {
            let mut ctl = ctl(self.slot);
            ctl.release(self.kind, self.epoch);
            ctl.apply()
        };
        drop(handle);
    }
}

fn hold_in(slot: &'static Mutex<Ctl>, kind: Kind, takes_over_keys: bool) -> StreamHold {
    let (epoch, handle) = {
        let mut ctl = ctl(slot);
        let epoch = ctl.acquire(kind, takes_over_keys);
        (epoch, ctl.apply())
    };
    drop(handle);
    StreamHold { slot, kind, epoch }
}

/// Config and env switch. Read when a stream is registered.
pub fn set_enabled(enabled: bool) {
    let handle = {
        let mut ctl = ctl(&CTL);
        ctl.enabled = enabled;
        ctl.apply()
    };
    drop(handle);
}

/// When a Tab press resumes a paused stream: one frame period before the
/// 300 ms overlay delay ends, and never before the press.
pub fn tab_resume_at(press: Instant, max_fps: u32) -> Instant {
    let lead = TAB_OVERLAY_AFTER_PRESS_MS.saturating_sub(pw::throttle_gap_ms(max_fps));
    press + Duration::from_millis(lead)
}

/// A key hold that starts at [`tab_resume_at`]. Dropping it before then
/// cancels it. A Tab grab that has already taken over the key holds also
/// cancels it.
#[must_use = "the stream pauses again when the hold is dropped"]
pub struct KeyHold {
    slot: Arc<Mutex<KeySlot>>,
    timer: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Default)]
struct KeySlot {
    dropped: bool,
    hold: Option<StreamHold>,
}

impl Drop for KeyHold {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.abort();
        }
        let hold = {
            let mut slot = self.slot.lock().unwrap_or_else(|err| err.into_inner());
            slot.dropped = true;
            slot.hold.take()
        };
        drop(hold);
    }
}

/// Take a key hold in `epoch`, unless a Tab grab has taken over since.
fn acquire_key(slot: &'static Mutex<Ctl>, epoch: u64) -> Option<StreamHold> {
    let handle = {
        let mut ctl = ctl(slot);
        if ctl.key_epoch != epoch {
            return None;
        }
        ctl.key_holds += 1;
        ctl.apply()
    };
    drop(handle);
    Some(StreamHold {
        slot,
        kind: Kind::Key,
        epoch,
    })
}

fn key_hold_in(slot: &'static Mutex<Ctl>, at: Instant, now: Instant) -> KeyHold {
    let epoch = ctl(slot).key_epoch;
    let key = Arc::new(Mutex::new(KeySlot::default()));
    if at <= now {
        key.lock().unwrap_or_else(|err| err.into_inner()).hold = acquire_key(slot, epoch);
        return KeyHold {
            slot: key,
            timer: None,
        };
    }
    let later = Arc::clone(&key);
    let timer = tokio::spawn(async move {
        tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await;
        let mut key = later.lock().unwrap_or_else(|err| err.into_inner());
        if !key.dropped {
            key.hold = acquire_key(slot, epoch);
        }
    });
    KeyHold {
        slot: key,
        timer: Some(timer),
    }
}

/// A Tab key went down while Overwatch runs: resume the stream at
/// [`tab_resume_at`]. The Tab grab takes this hold over; dropping it
/// earlier is also fine. Needs a Tokio runtime.
pub fn key_hold(press: Instant) -> KeyHold {
    let at = tab_resume_at(press, super::prefs().max_fps);
    key_hold_in(&CTL, at, Instant::now())
}

/// A grab is about to wait for a frame. A Tab grab also releases the key
/// holds, so the stream pauses when this grab is done.
pub(super) fn grab_hold(tab: bool) -> StreamHold {
    hold_in(&CTL, Kind::Grab, tab)
}

/// The live stream changed. It starts running, and pauses at once unless
/// something holds it.
pub(super) fn register(handle: &Arc<pw::Handle>) {
    ctl(&CTL).register(handle, pw::pause_trips());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leak_ctl(enabled: bool) -> &'static Mutex<Ctl> {
        Box::leak(Box::new(Mutex::new(Ctl {
            enabled,
            ..Ctl::default()
        })))
    }

    fn live(slot: &'static Mutex<Ctl>) -> Arc<pw::Handle> {
        let handle = Arc::new(pw::test_stream(true));
        ctl(slot).register(&handle, 0);
        handle
    }

    #[test]
    fn a_registered_stream_pauses_with_no_hold_and_runs_while_held() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        assert!(!stream.wants_active(), "nothing holds it: paused");
        let poll = hold_in(slot, Kind::Grab, false);
        assert!(stream.wants_active());
        drop(poll);
        assert!(!stream.wants_active(), "the grab is done: paused again");
    }

    #[test]
    fn a_tab_pressed_during_a_poll_grab_keeps_the_stream_running() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        let poll = hold_in(slot, Kind::Grab, false);
        let key = hold_in(slot, Kind::Key, false);
        // The Tab preempts the poll: its grab future is dropped.
        drop(poll);
        assert!(stream.wants_active(), "the key hold still runs it");
        // The Tab grab takes the key hold over.
        let tab = hold_in(slot, Kind::Grab, true);
        assert!(stream.wants_active());
        // The Tab task still owns the key hold, which is already released.
        drop(key);
        assert!(stream.wants_active(), "the grab still holds it");
        drop(tab);
        assert!(
            !stream.wants_active(),
            "after the Tab grab's copy the stream pauses, even though the Tab task runs on"
        );
    }

    #[test]
    fn a_poll_grab_does_not_take_over_a_key_hold() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        let key = hold_in(slot, Kind::Key, false);
        // The end-reel outcome poll can run while a Tab task waits.
        let poll = hold_in(slot, Kind::Grab, false);
        drop(poll);
        assert!(stream.wants_active(), "the Tab grab has not run yet");
        drop(key);
        assert!(!stream.wants_active());
    }

    #[test]
    fn a_grab_that_gives_up_releases_its_hold() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        let key = hold_in(slot, Kind::Key, false);
        let outcome: Result<(), &str> = {
            let _tab = hold_in(slot, Kind::Grab, true);
            Err("Tab grab: no fresh stream frame")
        };
        assert!(outcome.is_err());
        assert!(!stream.wants_active(), "a miss pauses like a frame does");
        drop(key);
        assert!(!stream.wants_active());
    }

    #[test]
    fn a_tab_task_that_never_grabs_releases_its_key_hold_when_it_ends() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        let key = hold_in(slot, Kind::Key, false);
        assert!(stream.wants_active());
        // Share still opening, or the game stopped: no grab ran.
        drop(key);
        assert!(!stream.wants_active());
    }

    #[test]
    fn with_the_game_gone_nothing_holds_and_the_stream_stays_paused() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        {
            let _poll = hold_in(slot, Kind::Grab, false);
        }
        // Polls and Tabs do not grab while Overwatch is not running, so no
        // hold is taken and the open share stays paused.
        assert!(!stream.wants_active());
        assert_eq!(ctl(slot).grab_holds, 0);
        assert_eq!(ctl(slot).key_holds, 0);
    }

    #[test]
    fn switched_off_the_stream_always_runs() {
        let slot = leak_ctl(false);
        let stream = live(slot);
        assert!(stream.wants_active());
        drop(hold_in(slot, Kind::Grab, true));
        assert!(stream.wants_active(), "pausing off: rc.6 behaviour");
    }

    #[test]
    fn a_stream_that_has_not_streamed_is_not_paused() {
        let slot = leak_ctl(true);
        let handle = Arc::new(pw::test_stream(false));
        ctl(slot).register(&handle, 0);
        assert!(handle.wants_active(), "a pause before Streaming is ignored");
    }

    #[test]
    fn a_new_stream_replaces_the_old_one() {
        let slot = leak_ctl(true);
        let old = live(slot);
        let new = live(slot);
        let poll = hold_in(slot, Kind::Grab, false);
        assert!(new.wants_active());
        assert!(!old.wants_active(), "the retired stream is not touched");
        drop(poll);
        assert!(!new.wants_active());
    }

    #[test]
    fn after_two_tripped_streams_a_new_stream_is_not_paused() {
        let slot = leak_ctl(true);
        let once = Arc::new(pw::test_stream(true));
        ctl(slot).register(&once, 1);
        assert!(
            !once.wants_active(),
            "one trip: the next stream tries again"
        );
        let twice = Arc::new(pw::test_stream(true));
        ctl(slot).register(&twice, pw::PAUSE_TRIPS_LIMIT);
        assert!(twice.wants_active(), "two trips: pausing stays off");
        drop(hold_in(slot, Kind::Grab, false));
        assert!(twice.wants_active());
    }

    #[test]
    fn a_tab_resumes_a_frame_period_before_the_overlay_delay() {
        let press = Instant::now();
        assert_eq!(tab_resume_at(press, 4), press + Duration::from_millis(50));
        assert_eq!(tab_resume_at(press, 5), press + Duration::from_millis(100));
        assert_eq!(tab_resume_at(press, 3), press + Duration::from_millis(0));
        assert_eq!(tab_resume_at(press, 2), press);
        assert_eq!(tab_resume_at(press, 1), press);
        // A compositor that sends a frame at once on resume and then one
        // per frame period: resumed at the press, the second frame is inside
        // the 300 ms overlay delay; resumed at tab_resume_at it is not.
        for fps in 1..=5 {
            let gap = pw::throttle_gap_ms(fps);
            let lead = tab_resume_at(press, fps).duration_since(press).as_millis() as u64;
            let first_ready = |start: u64| {
                (0..10)
                    .map(|k| start + 5 + k * gap)
                    .find(|at| *at >= TAB_OVERLAY_AFTER_PRESS_MS)
                    .unwrap()
            };
            assert!(first_ready(lead) <= first_ready(0), "{fps} fps");
            assert!(first_ready(lead) < 300 + gap, "{fps} fps");
        }
        let at_press = 5 + 2 * pw::throttle_gap_ms(4);
        assert_eq!(at_press, 505, "4 fps resumed at the press: the third frame");
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    #[test]
    fn a_key_hold_starts_at_its_resume_time_and_not_before() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        rt().block_on(async {
            let now = Instant::now();
            let key = key_hold_in(slot, now + Duration::from_millis(60), now);
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(!stream.wants_active(), "not yet");
            tokio::time::sleep(Duration::from_millis(120)).await;
            assert!(stream.wants_active(), "resumed at its time");
            drop(key);
            assert!(!stream.wants_active());
            // Already due: at once.
            let key = key_hold_in(slot, now, Instant::now());
            assert!(stream.wants_active());
            drop(key);
            assert!(!stream.wants_active());
        });
    }

    #[test]
    fn a_key_hold_dropped_or_taken_over_before_its_time_never_starts() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        rt().block_on(async {
            let now = Instant::now();
            let key = key_hold_in(slot, now + Duration::from_millis(40), now);
            drop(key);
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(
                !stream.wants_active(),
                "a dropped hold does not start later"
            );

            let now = Instant::now();
            let key = key_hold_in(slot, now + Duration::from_millis(40), now);
            // The Tab grab took over and finished before the key hold's time.
            drop(hold_in(slot, Kind::Grab, true));
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(
                !stream.wants_active(),
                "a key hold the Tab grab already took over does not start later"
            );
            assert_eq!(ctl(slot).key_holds, 0);
            drop(key);
            assert!(!stream.wants_active());
        });
    }
}
