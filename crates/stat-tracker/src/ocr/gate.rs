//! The OCR on/off gate (`ocr = "on" | "off"` in config.toml).
//!
//! Tesseract has one way in: [`super::ocr_with`], which creates the
//! per-thread `LepTess` instance and runs the read. Every public function of
//! [`crate::ocr`] that can reach it asks this gate first, before it encodes
//! an image, starts the OCR thread pool, or looks for a tessdata language.
//! With the gate off, such a call:
//!
//! * returns [`OcrOff`] (or an empty read, for the functions that return no
//!   `Result`) and does none of that work,
//! * is counted under its call site ([`blocked_calls`]), and logged once per
//!   site.
//!
//! `ocr_with` asks again, so a new entry point that forgets the first check
//! still creates no `LepTess`.
//!
//! The daemon sets the mode once at startup ([`set_mode`]) from
//! [`crate::ocr_switch::decide`]. Tools and tests that never call it run with
//! OCR on, as before. [`scoped`] switches the mode for one thread, for tests
//! and benches that share a process with OCR tests.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

/// Whether Tesseract may run in this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrMode {
    On,
    Off,
}

static MODE: AtomicU8 = AtomicU8::new(0);
static LEPTESS_CREATED: AtomicU64 = AtomicU64::new(0);
static TESSERACT_CALLS: AtomicU64 = AtomicU64::new(0);
static BLOCKED: Mutex<BTreeMap<Site, u64>> = Mutex::new(BTreeMap::new());

thread_local! {
    static SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
    /// Tesseract reads made on this thread ([`tesseract_calls_on_this_thread`]).
    static THREAD_TESSERACT_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

struct Scope {
    mode: OcrMode,
    blocked: BTreeMap<Site, u64>,
}

/// Where a blocked call came from: the `ocr` function that was called, and
/// the source line that called it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Site {
    pub file: &'static str,
    pub line: u32,
    /// The `crate::ocr` function that was called.
    pub entry: &'static str,
}

/// Longest [`Site::key`], in bytes: the report validator's cap for a call
/// site.
pub const SITE_KEY_MAX_BYTES: usize = scuffed_types::stat_report::OCR_BLOCKED_SITE_MAX_BYTES;

/// The tracker's source folder, as it is in a call's file path.
const SOURCE_ROOT: &str = "crates/stat-tracker/src/";

/// The place given for a call made from outside [`SOURCE_ROOT`].
const OTHER_PLACE: &str = "other.rs:1";

impl Site {
    /// `detect/match_end.rs:1133 recognize_prepared`: the file below the
    /// tracker's source folder, the line and the function.
    ///
    /// This is what leaves the process (log, status file, problem report).
    /// Everything up to and including `crates/stat-tracker/src/` is cut
    /// from the file, so a build path can not carry a user name out. A
    /// call from anywhere else (a test under `tests/`, a build with other
    /// paths) is `other.rs:1` and the function, with no path at all.
    /// Every key has the shape the report validator takes
    /// ([`scuffed_types::stat_report::is_ocr_call_site`]).
    pub fn key(&self) -> String {
        use scuffed_types::stat_report::is_ocr_call_site;
        if let Some(at) = self.file.rfind(SOURCE_ROOT) {
            let file = &self.file[at + SOURCE_ROOT.len()..];
            let key = format!("{file}:{} {}", self.line, self.entry);
            if is_ocr_call_site(&key) {
                return key;
            }
        }
        let other = format!("{OTHER_PLACE} {}", self.entry);
        if is_ocr_call_site(&other) {
            other
        } else {
            OTHER_PLACE.to_string()
        }
    }
}

impl std::fmt::Display for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key())
    }
}

/// The result of an OCR call made while OCR is off. Not a read: the caller
/// holds what it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrOff {
    pub site: Site,
}

impl std::fmt::Display for OcrOff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OCR is off (ocr = \"off\"): no read at {}", self.site)
    }
}

impl std::error::Error for OcrOff {}

/// True when `err` is the gate's [`OcrOff`], not a Tesseract failure.
pub fn is_off_error(err: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    err.downcast_ref::<OcrOff>().is_some()
}

/// Set the process-wide mode. The daemon calls this once at startup, from
/// [`crate::ocr_switch::Switch`]. That is the only caller outside tests: a
/// test in `ocr/mod.rs` reads the source tree for any other.
#[doc(hidden)]
pub fn set_mode(mode: OcrMode) {
    MODE.store(mode as u8, Ordering::Relaxed);
}

/// The mode for a call made on this thread. Off when the process-wide mode
/// is off, whatever a [`scoped`] mode says: a scope can turn OCR off for its
/// thread, never back on in a process that runs with OCR off. Otherwise the
/// scoped mode when one is active here, else on.
pub fn mode() -> OcrMode {
    if MODE.load(Ordering::Relaxed) == OcrMode::Off as u8 {
        return OcrMode::Off;
    }
    SCOPE
        .with(|s| s.borrow().as_ref().map(|scope| scope.mode))
        .unwrap_or(OcrMode::On)
}

/// True when an OCR call made here would be blocked. A caller with an OCR
/// fallback asks this first and holds instead, so the fallback is not even
/// attempted.
pub fn is_off() -> bool {
    mode() == OcrMode::Off
}

/// Ask the gate for one call of the `ocr` function `entry`. `Err` when OCR
/// is off; the call is then counted under the caller's source line.
#[track_caller]
pub(super) fn check(entry: &'static str) -> Result<(), OcrOff> {
    if !is_off() {
        return Ok(());
    }
    let caller = std::panic::Location::caller();
    Err(block(Site {
        file: caller.file(),
        line: caller.line(),
        entry,
    }))
}

/// Count a blocked call at `site` and return its [`OcrOff`].
pub(super) fn block(site: Site) -> OcrOff {
    let first = {
        let mut blocked = BLOCKED.lock().unwrap_or_else(|e| e.into_inner());
        let count = blocked.entry(site).or_insert(0);
        *count += 1;
        *count == 1
    };
    SCOPE.with(|s| {
        if let Some(scope) = s.borrow_mut().as_mut() {
            *scope.blocked.entry(site).or_insert(0) += 1;
        }
    });
    if first {
        tracing::warn!(
            site = %site,
            "OCR is off: a Tesseract call was blocked here and its read is held (logged once per call site)"
        );
    }
    OcrOff { site }
}

pub(super) fn note_leptess_created() {
    LEPTESS_CREATED.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn note_tesseract_call() {
    TESSERACT_CALLS.fetch_add(1, Ordering::Relaxed);
    THREAD_TESSERACT_CALLS.with(|calls| calls.set(calls.get() + 1));
}

/// One line of [`blocked_calls`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockedCalls {
    pub site: Site,
    pub count: u64,
}

/// Every call the gate blocked in this process, per call site, in source
/// order.
pub fn blocked_calls() -> Vec<BlockedCalls> {
    BLOCKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(site, count)| BlockedCalls {
            site: *site,
            count: *count,
        })
        .collect()
}

/// Sum of [`blocked_calls`].
pub fn blocked_total() -> u64 {
    BLOCKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .sum()
}

/// `LepTess` instances created in this process (one per OCR thread and
/// language, on first use).
pub fn leptess_created() -> u64 {
    LEPTESS_CREATED.load(Ordering::Relaxed)
}

/// Tesseract reads run in this process.
pub fn tesseract_calls() -> u64 {
    TESSERACT_CALLS.load(Ordering::Relaxed)
}

/// Tesseract reads made on the calling thread. Tests that run next to
/// others use this one: the process-wide count moves under them.
pub fn tesseract_calls_on_this_thread() -> u64 {
    THREAD_TESSERACT_CALLS.with(|calls| calls.get())
}

/// A mode held for the current thread. Dropping it restores what was there.
#[must_use = "the mode ends when this is dropped"]
pub struct ScopedMode {
    previous: Option<Scope>,
}

/// Switch the mode for OCR calls made on this thread until the returned
/// value is dropped. For tests: other threads, the OCR pool's included, keep
/// the process-wide mode, so a test can turn OCR off without touching the
/// tests running next to it. It can only tighten: in a process that runs
/// with OCR off, a scoped `On` changes nothing ([`mode`]). No code outside
/// tests calls it: a test in `ocr/mod.rs` reads the source tree for that.
#[doc(hidden)]
pub fn scoped(mode: OcrMode) -> ScopedMode {
    let previous = SCOPE.with(|s| {
        s.borrow_mut().replace(Scope {
            mode,
            blocked: BTreeMap::new(),
        })
    });
    ScopedMode { previous }
}

impl ScopedMode {
    /// The calls blocked on this thread since [`scoped`], per call site.
    pub fn blocked(&self) -> Vec<BlockedCalls> {
        SCOPE.with(|s| {
            s.borrow()
                .as_ref()
                .map(|scope| {
                    scope
                        .blocked
                        .iter()
                        .map(|(site, count)| BlockedCalls {
                            site: *site,
                            count: *count,
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    /// Sum of [`Self::blocked`].
    pub fn blocked_total(&self) -> u64 {
        self.blocked().iter().map(|b| b.count).sum()
    }
}

impl Drop for ScopedMode {
    fn drop(&mut self) {
        let previous = self.previous.take();
        SCOPE.with(|s| *s.borrow_mut() = previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blocked_call_is_counted_under_the_line_that_made_it() {
        let off = scoped(OcrMode::Off);
        let line = line!() + 1;
        let first = check("recognize_region").unwrap_err();
        assert_eq!(first.site.entry, "recognize_region");
        assert_eq!(first.site.line, line);
        assert!(first.site.file.ends_with("ocr/gate.rs"), "{}", first.site);
        for _ in 0..2 {
            // one site, counted three times with the two above it
            let _ = check("recognize_cell");
        }
        let blocked = off.blocked();
        assert_eq!(blocked.len(), 2, "{blocked:?}");
        assert_eq!(off.blocked_total(), 3);
        let cell = blocked
            .iter()
            .find(|b| b.site.entry == "recognize_cell")
            .unwrap();
        assert_eq!(cell.count, 2);
        // the process-wide table has them too
        assert!(
            blocked_calls()
                .iter()
                .any(|b| b.site == first.site && b.count >= 1)
        );
    }

    #[test]
    fn a_site_key_is_the_place_below_the_source_folder_and_never_a_path() {
        use scuffed_types::stat_report::is_ocr_call_site;
        let site = |file: &'static str| Site {
            file,
            line: 1133,
            entry: "recognize_prepared",
        };
        let other = "other.rs:1 recognize_prepared";
        for (file, key) in [
            (
                "crates/stat-tracker/src/detect/match_end.rs",
                "detect/match_end.rs:1133 recognize_prepared",
            ),
            (
                "crates/stat-tracker/src/main.rs",
                "main.rs:1133 recognize_prepared",
            ),
            // a build that gives the compiler absolute paths
            (
                "/home/alice/github/scuffed-crew/crates/stat-tracker/src/ocr/mod.rs",
                "ocr/mod.rs:1133 recognize_prepared",
            ),
            (
                "/home/some user/m\u{00fc}ller/crates/stat-tracker/src/main.rs",
                "main.rs:1133 recognize_prepared",
            ),
            // not below the tracker's source folder: no path at all
            ("crates/stat-tracker/tests/ocr_switch.rs", other),
            ("src/ocr/mod.rs", other),
            ("/home/alice/odd.rs", other),
            (
                "/home/alice/.cargo/registry/src/index/dep-1.0.0/src/lib.rs",
                other,
            ),
            ("C:\\Users\\alice\\tracker\\src\\main.rs", other),
            // below it, but not a file name the report takes
            ("crates/stat-tracker/src/../../../alice/main.rs", other),
            ("crates/stat-tracker/src/we\"ird name.rs", other),
            ("crates/stat-tracker/src/Upper.rs", other),
            ("crates/stat-tracker/src/a-b.rs", other),
            ("crates/stat-tracker/src/notes.txt", other),
        ] {
            let got = site(file).key();
            assert_eq!(got, key, "{file}");
            assert_eq!(site(file).to_string(), key, "Display is the key");
            assert!(is_ocr_call_site(&got), "{got}");
        }
        // a path, a line or a function that does not fit
        let long = Site {
            file: "crates/stat-tracker/src/a/very/long/path/that/goes/on/and/on/and/on/and/on/and/on/and/on/and/on/and/on/and/on/x.rs",
            line: 4,
            entry: "recognize_prepared",
        };
        assert_eq!(long.key(), other, "over the byte cap");
        let far = Site {
            file: "crates/stat-tracker/src/main.rs",
            line: 1_000_000,
            entry: "recognize_prepared",
        };
        assert_eq!(far.key(), other, "a line the report does not take");
        let odd = Site {
            file: "crates/stat-tracker/src/main.rs",
            line: 2,
            entry: "not a function",
        };
        assert_eq!(odd.key(), "other.rs:1");
        // the longest function the gate has, at the deepest file today
        let deep = Site {
            file: "crates/stat-tracker/src/detect/match_start.rs",
            line: 99_999,
            entry: "recognize_scoreboard_cells_pre_cropped",
        };
        assert_eq!(
            deep.key(),
            "detect/match_start.rs:99999 recognize_scoreboard_cells_pre_cropped"
        );
        assert!(deep.key().len() <= SITE_KEY_MAX_BYTES);
    }

    #[test]
    fn on_blocks_nothing_and_a_scope_ends_when_dropped() {
        assert_eq!(mode(), OcrMode::On, "tests run with OCR on");
        assert!(check("recognize_region").is_ok());
        {
            let off = scoped(OcrMode::Off);
            assert!(is_off());
            {
                let on = scoped(OcrMode::On);
                assert!(check("recognize_region").is_ok());
                assert_eq!(on.blocked_total(), 0);
            }
            assert!(is_off(), "the inner scope restored the outer one");
            assert!(check("recognize_region").is_err());
            assert_eq!(off.blocked_total(), 1);
        }
        assert!(!is_off());
        assert!(check("recognize_region").is_ok());
    }

    #[test]
    fn a_scope_is_for_its_own_thread_only() {
        let _off = scoped(OcrMode::Off);
        let other = std::thread::spawn(|| (is_off(), check("recognize_region").is_ok()))
            .join()
            .unwrap();
        assert_eq!(other, (false, true));
    }

    #[test]
    fn the_off_result_is_told_apart_from_a_tesseract_error() {
        let _off = scoped(OcrMode::Off);
        let off: Box<dyn std::error::Error + Send + Sync> = check("recognize").unwrap_err().into();
        assert!(is_off_error(off.as_ref()));
        assert!(off.to_string().contains("OCR is off"), "{off}");
        let other: Box<dyn std::error::Error + Send + Sync> = "tesseract failed".into();
        assert!(!is_off_error(other.as_ref()));
    }
}
