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

/// Longest [`Site::key`], in bytes. The report validator's cap for a call
/// site (`scuffed_types::stat_report::OCR_BLOCKED_SITE_MAX_BYTES`).
pub const SITE_KEY_MAX_BYTES: usize = 96;

impl Site {
    /// `detect/match_end.rs:1133 recognize_prepared`: the file below `src/`
    /// (or from `tests/`), the line and the function.
    ///
    /// This is what leaves the process (log, status file, problem report).
    /// Nothing above the crate's own folders is in it, so a build path can
    /// not carry a user name out, and it holds only ASCII letters, digits,
    /// `_`, `.`, `/`, `:`, `-` and single spaces, at most
    /// [`SITE_KEY_MAX_BYTES`] bytes.
    pub fn key(&self) -> String {
        let file = self.file.replace('\\', "/");
        let file = match (file.rfind("/src/"), file.rfind("/tests/")) {
            (Some(at), _) => &file[at + "/src/".len()..],
            (None, Some(at)) => &file[at + 1..],
            (None, None) => file
                .strip_prefix("src/")
                .unwrap_or(file.rsplit('/').next().unwrap_or("")),
        };
        let mut key: String = format!("{file}:{} {}", self.line, self.entry)
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "_./:- ".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        while key.contains("  ") {
            key = key.replace("  ", " ");
        }
        key.truncate(SITE_KEY_MAX_BYTES);
        key.trim().to_string()
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

/// Set the process-wide mode. The daemon calls this once at startup.
pub fn set_mode(mode: OcrMode) {
    MODE.store(mode as u8, Ordering::Relaxed);
}

/// The mode for a call made on this thread: a [`scoped`] mode when one is
/// active here, else the process-wide one.
pub fn mode() -> OcrMode {
    if let Some(mode) = SCOPE.with(|s| s.borrow().as_ref().map(|scope| scope.mode)) {
        return mode;
    }
    if MODE.load(Ordering::Relaxed) == OcrMode::Off as u8 {
        OcrMode::Off
    } else {
        OcrMode::On
    }
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
pub(crate) fn check(entry: &'static str) -> Result<(), OcrOff> {
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
pub(crate) fn block(site: Site) -> OcrOff {
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

pub(crate) fn note_leptess_created() {
    LEPTESS_CREATED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_tesseract_call() {
    TESSERACT_CALLS.fetch_add(1, Ordering::Relaxed);
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

/// A mode held for the current thread. Dropping it restores what was there.
#[must_use = "the mode ends when this is dropped"]
pub struct ScopedMode {
    previous: Option<Scope>,
}

/// Switch the mode for OCR calls made on this thread until the returned
/// value is dropped. For tests and benches: other threads, the OCR pool's
/// included, keep the process-wide mode, so a test can turn OCR off without
/// touching the tests running next to it.
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
    fn a_site_key_names_the_source_place_and_nothing_above_the_crate() {
        let site = |file: &'static str| Site {
            file,
            line: 1133,
            entry: "recognize_prepared",
        };
        for (file, key) in [
            (
                "crates/stat-tracker/src/detect/match_end.rs",
                "detect/match_end.rs:1133 recognize_prepared",
            ),
            (
                "/home/some user/build/crates/stat-tracker/src/main.rs",
                "main.rs:1133 recognize_prepared",
            ),
            ("src/ocr/mod.rs", "ocr/mod.rs:1133 recognize_prepared"),
            (
                "crates/stat-tracker/tests/ocr_switch.rs",
                "tests/ocr_switch.rs:1133 recognize_prepared",
            ),
            (
                "C:\\Users\\m\u{00fc}ller\\tracker\\src\\main.rs",
                "main.rs:1133 recognize_prepared",
            ),
            (
                "/home/m\u{00fc}ller/odd.rs",
                "odd.rs:1133 recognize_prepared",
            ),
        ] {
            let got = site(file).key();
            assert_eq!(got, key, "{file}");
            assert_eq!(site(file).to_string(), key, "Display is the key");
        }
        // bounded, and only the characters the report validator takes
        let long = Site {
            file: "src/a/very/long/path/that/goes/on/and/on/and/on/and/on/and/on/and/on/and/on/and/on/x.rs",
            line: 4_000_000,
            entry: "recognize_scoreboard_cells_pre_cropped_with_everything",
        }
        .key();
        assert!(long.len() <= SITE_KEY_MAX_BYTES, "{}", long.len());
        let odd = Site {
            file: "src/we\"ird name\u{00e9}.rs",
            line: 2,
            entry: "x",
        }
        .key();
        assert!(
            odd.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_./:- ".contains(c)),
            "{odd}"
        );
        assert!(!odd.contains("  ") && odd == odd.trim(), "{odd:?}");
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
