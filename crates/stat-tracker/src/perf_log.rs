//! Always-on performance log (`perf.log` in the data directory).
//!
//! Each closed [`crate::perf`] window appends one line: the daemon's CPU%
//! and RSS for that window, capture and OCR timing, and the X11 grab
//! details (size, full screen or one output, and that no shm is used). The
//! first line after start is a header with the setup: session type, kernel,
//! CPU count, RAM and the Proton version seen running. Overhead is one small
//! write a minute. The file rotates at [`MAX_BYTES`] into `perf.log.1`, so
//! at most twice that is kept. A Performance report sends only this log.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::perf::Summary;

pub const FILE: &str = "perf.log";
pub const ROTATED: &str = "perf.log.1";
/// Rotate once `perf.log` reaches this size.
pub const MAX_BYTES: u64 = 256 * 1024;

static DIR: OnceLock<PathBuf> = OnceLock::new();
static LAST_CPU: Mutex<Option<(u64, std::time::Instant)>> = Mutex::new(None);

/// Start the log in `data_dir` and write the setup header. Call once.
pub fn init(data_dir: &Path) {
    let _ = DIR.set(data_dir.to_path_buf());
    let header = header_line(&SetupInfo::collect());
    append_to(data_dir, &header, MAX_BYTES);
    *LAST_CPU.lock().unwrap_or_else(|e| e.into_inner()) =
        proc_cpu_ticks().map(|t| (t, std::time::Instant::now()));
}

/// Append one line for a closed window. No-op before [`init`].
pub fn write_summary(s: &Summary) {
    let Some(dir) = DIR.get() else { return };
    let now = std::time::Instant::now();
    let ticks = proc_cpu_ticks();
    let cpu_pct = {
        let mut last = LAST_CPU.lock().unwrap_or_else(|e| e.into_inner());
        let pct = match (*last, ticks) {
            (Some((t0, i0)), Some(t1)) => {
                let secs = now.saturating_duration_since(i0).as_secs_f64();
                cpu_percent(t1.saturating_sub(t0), clk_tck(), secs)
            }
            _ => None,
        };
        *last = ticks.map(|t| (t, now));
        pct
    };
    let line = sample_line(s, cpu_pct, proc_rss_kb());
    append_to(dir, &line, MAX_BYTES);
}

/// `perf.log.1` then `perf.log`, as text. Empty when neither exists.
pub fn read(data_dir: &Path) -> String {
    let mut out = String::new();
    for name in [ROTATED, FILE] {
        if let Ok(bytes) = std::fs::read(data_dir.join(name)) {
            out.push_str(&String::from_utf8_lossy(&bytes));
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

/// Append `line` to `dir/perf.log`, rotating first when the file is at or
/// past `max_bytes`.
pub fn append_to(dir: &Path, line: &str, max_bytes: u64) {
    let path = dir.join(FILE);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() >= max_bytes) {
        let _ = std::fs::rename(&path, dir.join(ROTATED));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// The machine the log was written on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupInfo {
    pub session: &'static str,
    pub kernel: String,
    pub cpus: usize,
    pub ram_mb: u64,
    pub proton: Option<String>,
}

impl SetupInfo {
    pub fn collect() -> Self {
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".into());
        let ram_mb = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|m| mem_total_mb(&m))
            .unwrap_or(0);
        Self {
            session: crate::report::SessionType::from_env().as_str(),
            kernel,
            cpus: std::thread::available_parallelism().map_or(1, |n| n.get()),
            ram_mb,
            proton: detect_proton(),
        }
    }
}

fn now_stamp() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn header_line(s: &SetupInfo) -> String {
    format!(
        "{} setup app={} session={} kernel={} cpus={} ram_mb={} proton={}",
        now_stamp(),
        crate::report::bundle::APP_VERSION,
        s.session,
        s.kernel,
        s.cpus,
        s.ram_mb,
        s.proton.as_deref().unwrap_or("none"),
    )
}

pub fn sample_line(s: &Summary, cpu_pct: Option<f64>, rss_kb: Option<u64>) -> String {
    let mut line = format!(
        "{} sample window_s={} cpu_pct={} rss_kb={} backend={} \
         captures={} capture_p50_ms={} capture_max_ms={} capture_cpu_ms={} \
         polls={} poll_p50_ms={} poll_max_ms={} \
         ocr_runs={} ocr_p50_ms={} ocr_max_ms={} ocr_cpu_ms={}",
        now_stamp(),
        s.window_secs,
        cpu_pct.map_or("na".into(), |p| format!("{p:.1}")),
        rss_kb.map_or("na".into(), |k| k.to_string()),
        s.backend,
        s.capture.count,
        s.capture.p50_ms,
        s.capture.max_ms,
        s.capture_cpu_ms,
        s.poll_analyze.count,
        s.poll_analyze.p50_ms,
        s.poll_analyze.max_ms,
        s.tab_analyze.count,
        s.tab_analyze.p50_ms,
        s.tab_analyze.max_ms,
        s.tab_cpu_ms,
    );
    if let Some(x) = &s.x11 {
        line.push_str(&format!(
            " x11_grab_w={} x11_grab_h={} x11_grab_scope={} x11_shm=no x11_grabs={} \
             x11_getimage_p50_ms={} x11_getimage_max_ms={}",
            x.width, x.height, x.scope, x.grabs.count, x.grabs.p50_ms, x.grabs.max_ms
        ));
    }
    line
}

pub fn cpu_percent(ticks: u64, hz: u64, secs: f64) -> Option<f64> {
    (secs > 0.0 && hz > 0).then(|| ticks as f64 / hz as f64 / secs * 100.0)
}

fn mem_total_mb(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

fn clk_tck() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v > 0 { v as u64 } else { 100 }
}

/// utime + stime of this process, in clock ticks.
fn proc_cpu_ticks() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    parse_stat_ticks(&stat)
}

pub fn parse_stat_ticks(stat: &str) -> Option<u64> {
    // Fields after the `(comm)`; utime and stime are fields 14 and 15.
    let rest = &stat[stat.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

fn proc_rss_kb() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf has no preconditions.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    Some(pages * page.max(4096) as u64 / 1024)
}

/// A Proton build name from any running process command line, such as
/// `GE-Proton9-20` or `Proton 9.0`.
fn detect_proton() -> Option<String> {
    let entries = std::fs::read_dir("/proc").ok()?;
    for e in entries.flatten() {
        let name = e.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Ok(cmd) = std::fs::read(e.path().join("cmdline"))
            && let Some(p) = proton_from_cmdline(&String::from_utf8_lossy(&cmd))
        {
            return Some(p);
        }
    }
    None
}

pub fn proton_from_cmdline(cmd: &str) -> Option<String> {
    cmd.split(['\0', '/'])
        .find(|seg| {
            seg.to_ascii_lowercase().contains("proton") && !seg.ends_with(".py") && seg != &"proton"
        })
        .map(|s| s.trim().replace(' ', "_"))
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::{StageStats, X11Stats};

    fn summary(x11: Option<X11Stats>) -> Summary {
        Summary {
            window_secs: 60,
            backend: "x11",
            capture: StageStats {
                count: 15,
                p50_ms: 38,
                max_ms: 61,
                total_ms: 600,
            },
            poll_analyze: StageStats::default(),
            tab_analyze: StageStats {
                count: 2,
                p50_ms: 900,
                max_ms: 950,
                total_ms: 1850,
            },
            capture_cpu_ms: 420,
            tab_cpu_ms: 1700,
            x11,
        }
    }

    #[test]
    fn rotates_into_dot_one_and_keeps_two_files() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..50 {
            append_to(dir.path(), &format!("line {i:03} {}", "x".repeat(40)), 200);
        }
        let cur = std::fs::metadata(dir.path().join(FILE)).unwrap().len();
        let old = std::fs::metadata(dir.path().join(ROTATED)).unwrap().len();
        assert!(cur <= 200 + 60 && old <= 200 + 60, "cur={cur} old={old}");
        assert!(!dir.path().join("perf.log.2").exists());
        let text = read(dir.path());
        assert!(
            text.trim_end()
                .ends_with(&format!("line 049 {}", "x".repeat(40)))
        );
        assert!(!text.contains("line 000"));
    }

    #[test]
    fn x11_timing_fields_are_in_the_sample_line() {
        let x = X11Stats {
            width: 1920,
            height: 1080,
            scope: "full",
            grabs: StageStats {
                count: 15,
                p50_ms: 31,
                max_ms: 55,
                total_ms: 480,
            },
        };
        let line = sample_line(&summary(Some(x)), Some(3.25), Some(81234));
        for f in [
            "cpu_pct=3.2",
            "rss_kb=81234",
            "backend=x11",
            "capture_p50_ms=38",
            "capture_cpu_ms=420",
            "ocr_runs=2",
            "ocr_p50_ms=900",
            "ocr_cpu_ms=1700",
            "x11_grab_w=1920",
            "x11_grab_h=1080",
            "x11_grab_scope=full",
            "x11_shm=no",
            "x11_grabs=15",
            "x11_getimage_p50_ms=31",
            "x11_getimage_max_ms=55",
        ] {
            assert!(line.contains(f), "{f} missing from {line}");
        }
        assert!(!sample_line(&summary(None), None, None).contains("x11_"));
    }

    #[test]
    fn header_names_the_setup() {
        let h = header_line(&SetupInfo {
            session: "x11",
            kernel: "6.8.0-45-generic".into(),
            cpus: 8,
            ram_mb: 15890,
            proton: Some("GE-Proton9-20".into()),
        });
        for f in [
            "session=x11",
            "kernel=6.8.0-45-generic",
            "cpus=8",
            "ram_mb=15890",
            "proton=GE-Proton9-20",
        ] {
            assert!(h.contains(f), "{f} missing from {h}");
        }
    }

    #[test]
    fn parsers() {
        assert_eq!(
            parse_stat_ticks("123 (scuffed tr) S 1 2 3 4 5 6 7 8 9 10 250 50 0"),
            Some(300)
        );
        assert_eq!(
            cpu_percent(300, 100, 60.0).map(|p| (p * 10.0).round()),
            Some(50.0)
        );
        assert_eq!(mem_total_mb("MemTotal:       16271360 kB\n"), Some(15890));
        assert_eq!(
            proton_from_cmdline(
                "/home/a/.steam/compatibilitytools.d/GE-Proton9-20/proton\0waitforexitandrun"
            ),
            Some("GE-Proton9-20".into())
        );
        assert_eq!(proton_from_cmdline("/usr/bin/bash\0-c"), None);
    }
}
