//! Always-on performance log (`perf.log` in the data directory).
//!
//! Each closed [`crate::perf`] window appends one line: the daemon's CPU%
//! and RSS for that window, capture and OCR timing, the X11 grab details
//! (size, full screen or one output, and that no shm is used), the
//! screenshot portal's step times, the PipeWire stream's delivered and
//! copied buffers, and the CPU% of the compositor, the desktop portal and
//! PipeWire over the same window ([`crate::host_probe`]). The
//! first line after start is a header with the setup: session type, kernel,
//! CPU model and count, RAM and the Proton version seen running. Overhead is one small
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
static HOST_CPU: Mutex<crate::host_probe::ProcCpuTracker> =
    Mutex::new(crate::host_probe::ProcCpuTracker::new());

/// Start the log in `data_dir` and write the setup header. Call once.
pub fn init(data_dir: &Path) {
    let _ = DIR.set(data_dir.to_path_buf());
    let header = header_line(&SetupInfo::collect());
    append_to(data_dir, &header, MAX_BYTES);
    *LAST_CPU.lock().unwrap_or_else(|e| e.into_inner()) =
        proc_cpu_ticks().map(|t| (t, std::time::Instant::now()));
    // Baseline scan, so the first window already has desktop CPU.
    HOST_CPU
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .sample_now(clk_tck());
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
    // One /proc scan per window.
    let host = HOST_CPU
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .sample_now(clk_tck());
    let line = sample_line(s, cpu_pct, proc_rss_kb(), &host);
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
    /// `model name` of the first CPU in `/proc/cpuinfo`, cleaned by
    /// [`cpu_model_from_cpuinfo`].
    pub cpu_model: Option<String>,
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
            cpu_model: std::fs::read_to_string("/proc/cpuinfo")
                .ok()
                .and_then(|c| cpu_model_from_cpuinfo(&c)),
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
        "{} setup app={} session={} kernel={} cpus={} ram_mb={} proton={} cpu_model=\"{}\"",
        now_stamp(),
        crate::report::bundle::APP_VERSION,
        s.session,
        s.kernel,
        s.cpus,
        s.ram_mb,
        s.proton.as_deref().unwrap_or("none"),
        s.cpu_model.as_deref().unwrap_or("unknown"),
    )
}

pub fn sample_line(
    s: &Summary,
    cpu_pct: Option<f64>,
    rss_kb: Option<u64>,
    host: &crate::host_probe::HostCpu,
) -> String {
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
    if let Some(p) = &s.portal {
        line.push_str(&format!(
            " portal_shots={} portal_rt_p50_ms={} portal_rt_max_ms={} \
             portal_png_kb_p50={} portal_png_kb_max={} \
             portal_decode_p50_ms={} portal_decode_max_ms={} \
             portal_geom_p50_ms={} portal_geom_max_ms={} \
             portal_crop_p50_ms={} portal_crop_max_ms={}",
            p.round_trip.count,
            p.round_trip.p50_ms,
            p.round_trip.max_ms,
            p.png_kb.p50_ms,
            p.png_kb.max_ms,
            p.decode.p50_ms,
            p.decode.max_ms,
            p.geometry.p50_ms,
            p.geometry.max_ms,
            p.crop.p50_ms,
            p.crop.max_ms,
        ));
    }
    if let Some(st) = &s.stream {
        line.push_str(&format!(
            " stream_delivered={} stream_fps={:.1} stream_copied={} stream_copy_ratio={}",
            st.delivered,
            st.delivered_fps(s.window_secs),
            st.copied,
            st.copy_ratio().map_or("na".into(), |r| format!("{r:.2}")),
        ));
        if let Some(b) = &st.buffer {
            line.push_str(&format!(
                " stream_buf={} stream_buf_bytes={} stream_format={} stream_w={} stream_h={}",
                b.kind, b.bytes, b.format, b.width, b.height
            ));
        }
    }
    line.push(' ');
    line.push_str(&host.fields());
    line
}

pub fn cpu_percent(ticks: u64, hz: u64, secs: f64) -> Option<f64> {
    (secs > 0.0 && hz > 0).then(|| ticks as f64 / hz as f64 / secs * 100.0)
}

/// Longest CPU model kept, in characters.
const CPU_MODEL_MAX_CHARS: usize = 80;

/// The first `model name` in `/proc/cpuinfo` (x86), else the first
/// `Hardware` or `Model` line (ARM), with runs of whitespace collapsed to
/// one space, quotes and control characters dropped, and at most
/// [`CPU_MODEL_MAX_CHARS`] characters. The model stays space separated so
/// the log scrub, which replaces any run of 24 or more token characters,
/// leaves it readable.
pub fn cpu_model_from_cpuinfo(cpuinfo: &str) -> Option<String> {
    let value = |key: &str| {
        cpuinfo.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.to_string())
        })
    };
    let raw = value("model name")
        .or_else(|| value("Hardware"))
        .or_else(|| value("Model"))?;
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(CPU_MODEL_MAX_CHARS)
        .collect();
    let cleaned = cleaned.trim().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
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
/// `GE-Proton9-20` or `Proton 9.0`. Only the game's Proton runtime counts
/// (see [`proton_from_cmdline`]); Proton VPN and Proton Mail never do.
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

/// The Proton build in one process command line (`/proc/<pid>/cmdline`,
/// arguments split by NUL), or `None`.
///
/// Only a Proton install counts: an absolute path argument with a folder whose name
/// holds `proton` followed by the `proton` launcher script, or by the
/// `files` or `dist` folder that holds Proton's Wine. For example
/// `.../steamapps/common/Proton 9.0 (Beta)/proton waitforexitandrun` or
/// `.../compatibilitytools.d/GE-Proton9-20/files/bin/wine64-preloader`.
/// A folder name that mentions `vpn` or `mail` never counts, and a name with
/// no path around it (`proton.vpn.daemon`, `protonvpn-app`) is not a Proton
/// install.
pub fn proton_from_cmdline(cmd: &str) -> Option<String> {
    cmd.split('\0').find_map(proton_from_path)
}

fn proton_from_path(arg: &str) -> Option<String> {
    // An absolute path only: a shell line such as `echo proton/files` is
    // not an install.
    if !arg.starts_with('/') {
        return None;
    }
    let segs: Vec<&str> = arg.split('/').collect();
    segs.windows(2).find_map(|pair| {
        let (dir, next) = (pair[0].trim(), pair[1]);
        let lower = dir.to_ascii_lowercase();
        let runtime_child = next == "proton" || next == "files" || next == "dist";
        let is_proton_dir = lower.contains("proton")
            && lower != "proton"
            && !lower.contains("vpn")
            && !lower.contains("mail");
        (runtime_child && is_proton_dir).then(|| dir.replace(' ', "_"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_probe::{GroupCpu, HostCpu};
    use crate::perf::{PortalStats, StageStats, StreamBuffer, StreamStats, X11Stats};

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
            portal: None,
            stream: None,
        }
    }

    fn stats(count: u32, p50: u64, max: u64) -> StageStats {
        StageStats {
            count,
            p50_ms: p50,
            max_ms: max,
            total_ms: 0,
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
        let line = sample_line(
            &summary(Some(x)),
            Some(3.25),
            Some(81234),
            &HostCpu::default(),
        );
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
        let bare = sample_line(&summary(None), None, None, &HostCpu::default());
        for absent in ["x11_", "portal_shots", "stream_"] {
            assert!(!bare.contains(absent), "{absent} in {bare}");
        }
        // The desktop process fields are always there, at the end.
        assert!(
            bare.ends_with(
                " compositor=none compositor_cpu_pct=none portal_cpu_pct=none pipewire_cpu_pct=none"
            ),
            "{bare}"
        );
    }

    /// The KDE stream case a report needs: our capture CPU, the portal
    /// split, the stream's delivered rate and copy ratio, and KWin's CPU.
    #[test]
    fn portal_stream_and_desktop_fields_are_in_the_sample_line() {
        let mut s = summary(None);
        s.backend = "portal";
        s.capture_cpu_ms = 75;
        s.portal = Some(PortalStats {
            round_trip: stats(3, 180, 240),
            png_kb: stats(3, 5900, 6100),
            decode: stats(3, 95, 120),
            geometry: stats(3, 2, 4),
            crop: stats(3, 0, 1),
        });
        s.stream = Some(StreamStats {
            delivered: 7_200,
            copied: 240,
            buffer: Some(StreamBuffer {
                kind: "memfd",
                bytes: 14_745_600,
                format: "BGRx",
                width: 2560,
                height: 1440,
            }),
        });
        let host = HostCpu {
            compositor: Some("kwin_wayland".into()),
            compositor_cpu: GroupCpu::Pct(38.25),
            xorg_cpu: None,
            portal_cpu: GroupCpu::Pct(0.4),
            pipewire_cpu: GroupCpu::NoBaseline,
        };
        let line = sample_line(&s, Some(6.0), Some(90_000), &host);
        let tail = line.split_once(" sample ").unwrap().1;
        assert_eq!(
            tail,
            "window_s=60 cpu_pct=6.0 rss_kb=90000 backend=portal \
             captures=15 capture_p50_ms=38 capture_max_ms=61 capture_cpu_ms=75 \
             polls=0 poll_p50_ms=0 poll_max_ms=0 \
             ocr_runs=2 ocr_p50_ms=900 ocr_max_ms=950 ocr_cpu_ms=1700 \
             portal_shots=3 portal_rt_p50_ms=180 portal_rt_max_ms=240 \
             portal_png_kb_p50=5900 portal_png_kb_max=6100 \
             portal_decode_p50_ms=95 portal_decode_max_ms=120 \
             portal_geom_p50_ms=2 portal_geom_max_ms=4 \
             portal_crop_p50_ms=0 portal_crop_max_ms=1 \
             stream_delivered=7200 stream_fps=120.0 stream_copied=240 stream_copy_ratio=0.03 \
             stream_buf=memfd stream_buf_bytes=14745600 stream_format=BGRx stream_w=2560 stream_h=1440 \
             compositor=kwin_wayland compositor_cpu_pct=38.2 portal_cpu_pct=0.4 pipewire_cpu_pct=na"
        );
        // A stream with nothing delivered has no ratio, and no buffer line.
        s.portal = None;
        s.stream = Some(StreamStats {
            delivered: 0,
            copied: 2,
            buffer: None,
        });
        let line = sample_line(&s, None, None, &host);
        assert!(line.contains(" stream_copy_ratio=na compositor="), "{line}");
    }

    #[test]
    fn header_names_the_setup() {
        let h = header_line(&SetupInfo {
            session: "x11",
            kernel: "6.8.0-45-generic".into(),
            cpus: 8,
            cpu_model: Some("AMD Ryzen 7 5800X 8-Core Processor".into()),
            ram_mb: 15890,
            proton: Some("GE-Proton9-20".into()),
        });
        for f in [
            "session=x11",
            "kernel=6.8.0-45-generic",
            "cpus=8",
            "ram_mb=15890",
            "proton=GE-Proton9-20",
            "cpu_model=\"AMD Ryzen 7 5800X 8-Core Processor\"",
        ] {
            assert!(h.contains(f), "{f} missing from {h}");
        }
        let unknown = header_line(&SetupInfo {
            session: "wayland",
            kernel: "6.17.1".into(),
            cpus: 4,
            cpu_model: None,
            ram_mb: 8000,
            proton: None,
        });
        assert!(
            unknown.ends_with("proton=none cpu_model=\"unknown\""),
            "{unknown}"
        );
    }

    #[test]
    fn cpu_model_comes_from_the_first_model_name() {
        let x86 = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel\t\t: 33\n\
                   model name\t: AMD Ryzen 7 5800X 8-Core Processor\nprocessor\t: 1\n\
                   model name\t: AMD Ryzen 7 5800X 8-Core Processor\n";
        assert_eq!(
            cpu_model_from_cpuinfo(x86).as_deref(),
            Some("AMD Ryzen 7 5800X 8-Core Processor")
        );
        // `model` (a number) is not `model name`.
        assert_eq!(cpu_model_from_cpuinfo("model\t\t: 33\n"), None);
        let intel = "model name\t:   Intel(R) Core(TM) i5-8250U  CPU @ 1.60GHz \n";
        assert_eq!(
            cpu_model_from_cpuinfo(intel).as_deref(),
            Some("Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz")
        );
        let arm = "processor\t: 0\nBogoMIPS\t: 108.00\nModel\t\t: Raspberry Pi 5 Model B Rev 1.0\n";
        assert_eq!(
            cpu_model_from_cpuinfo(arm).as_deref(),
            Some("Raspberry Pi 5 Model B Rev 1.0")
        );
        let odd = format!("model name\t: Fake \"quoted\"\u{7} {}\n", "x ".repeat(100));
        let got = cpu_model_from_cpuinfo(&odd).unwrap();
        assert!(
            !got.contains('"') && !got.chars().any(char::is_control),
            "{got}"
        );
        assert!(got.chars().count() <= CPU_MODEL_MAX_CHARS, "{got}");
        assert_eq!(cpu_model_from_cpuinfo("model name\t:   \n"), None);
        assert_eq!(cpu_model_from_cpuinfo(""), None);
    }

    #[test]
    fn proton_matches_only_the_games_runtime() {
        let game = [
            (
                "python3\0/home/a/.steam/steam/steamapps/common/Proton 9.0 (Beta)/proton\0waitforexitandrun\0/x/Overwatch.exe",
                "Proton_9.0_(Beta)",
            ),
            (
                "/home/a/.steam/root/compatibilitytools.d/GE-Proton9-20/files/bin/wine64-preloader\0C:\\Overwatch.exe",
                "GE-Proton9-20",
            ),
            (
                "/home/a/.local/share/Steam/steamapps/common/Proton - Experimental/files/bin/wineserver",
                "Proton_-_Experimental",
            ),
            (
                "/opt/proton-cachyos/dist/bin/wine64\0Battle.net.exe",
                "proton-cachyos",
            ),
        ];
        for (cmd, want) in game {
            assert_eq!(proton_from_cmdline(cmd).as_deref(), Some(want), "{cmd:?}");
        }
        let not_game = [
            "/usr/bin/python3\0-m\0proton.vpn.daemon",
            "/usr/bin/python3\0/usr/bin/protonvpn-app",
            "/usr/lib/python3/dist-packages/proton/vpn/daemon/__main__.py",
            "/opt/proton-vpn/proton/bin\0--start-minimized",
            "/opt/Proton Mail/proton\0--hidden",
            "/usr/bin/proton-mail-bridge\0--noninteractive",
            "/home/a/notes/proton-ideas.txt",
            "/usr/bin/bash\0-c\0echo proton/files",
            "/usr/bin/bash\0-c",
        ];
        for cmd in not_game {
            assert_eq!(proton_from_cmdline(cmd), None, "{cmd:?}");
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
    }
}
