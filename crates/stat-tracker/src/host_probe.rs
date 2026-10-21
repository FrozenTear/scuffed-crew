//! Read-only probes of the desktop the tracker runs on, for the performance
//! log and problem reports. Files under `/proc` and `/sys` only: no
//! subprocess, no new crate.
//!
//! - [`ProcCpuTracker`]: CPU used by the compositor, Xwayland (and Xorg on
//!   an X11 session), the desktop portal, PipeWire, and the tracker's own
//!   GUI and companion overlay between two `perf.log` samples, from
//!   `utime + stime` in `/proc/<pid>/stat`. `/proc` is scanned once per
//!   sample window.
//! - [`host_line`]: the desktop, the session type and the GPU topology from
//!   `/sys/class/drm` (each card's kernel driver, its connectors and which
//!   card drives a connected output), as one line for `log.txt`.

use std::path::Path;
use std::time::Instant;

/// Compositor process names, as `/proc/<pid>/comm` shows them (the kernel
/// keeps at most 15 bytes of the name).
const COMPOSITORS: &[&str] = &[
    "kwin_wayland",
    "kwin_x11",
    "niri",
    "Hyprland",
    "gnome-shell",
    "sway",
    "labwc",
    "wayfire",
    "river",
];

/// `stat-tracker-gui` as `comm` shows it, cut to 15 bytes. The same
/// binary runs the main window and, with `--companion`, the overlay.
const GUI_COMM: &str = "stat-tracker-gu";

/// The argument that makes a GUI process the companion overlay.
const COMPANION_ARG: &[u8] = b"--companion";

/// `xdg-desktop-portal` and every backend (`-kde`, `-gnome`, `-wlr`,
/// `-hyprland`, `-gtk`) cut to 15 bytes by the kernel.
const PORTAL_COMM_PREFIX: &str = "xdg-desktop-por";

/// What a watched process is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Compositor,
    /// The X server. Counted whoever owns it, because it usually runs as
    /// root, and only on an X11 session: on Wayland an `Xorg` is a display
    /// manager's greeter, not the game's server.
    Xorg,
    /// Our own uid's Xwayland, which a Proton game usually draws through.
    Xwayland,
    Portal,
    Pipewire,
    /// The tracker's main window (`stat-tracker-gui`).
    Gui,
    /// The tracker's overlay (`stat-tracker-gui --companion`). [`classify`]
    /// returns [`Role::Gui`]; [`scan`] tells the two apart by `cmdline`.
    Companion,
}

/// The role of a process from its `comm`, or `None` when it is not watched.
pub fn classify(comm: &str) -> Option<Role> {
    let comm = comm.trim_end_matches('\n');
    if COMPOSITORS.contains(&comm) {
        Some(Role::Compositor)
    } else if comm == "Xorg" {
        Some(Role::Xorg)
    } else if comm == "Xwayland" {
        Some(Role::Xwayland)
    } else if comm == GUI_COMM {
        Some(Role::Gui)
    } else if comm.starts_with(PORTAL_COMM_PREFIX) {
        Some(Role::Portal)
    } else if comm == "pipewire" {
        Some(Role::Pipewire)
    } else {
        None
    }
}

/// One watched process at one scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcSample {
    pub pid: u32,
    /// Field 22 of `stat`: start time in clock ticks after boot. A reused
    /// pid has a different start time, so its ticks are not compared.
    pub start: u64,
    /// `utime + stime` in clock ticks.
    pub ticks: u64,
    pub role: Role,
    pub comm: String,
}

/// `(utime + stime, starttime)` from one `/proc/<pid>/stat` line.
pub fn parse_stat(stat: &str) -> Option<(u64, u64)> {
    // The fields after `(comm)`; comm can hold spaces and parentheses, so
    // split at the last `)`. utime, stime and starttime are fields 14, 15
    // and 22 of the line, so 11, 12 and 19 after `state`'s position 0.
    let rest = stat.get(stat.rfind(')')? + 2..)?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = f.get(11)?.parse().ok()?;
    let stime: u64 = f.get(12)?.parse().ok()?;
    let start: u64 = f.get(19)?.parse().ok()?;
    Some((utime.saturating_add(stime), start))
}

/// The real uid on the `Uid:` line of `/proc/<pid>/status`. The status
/// file, not the owner of `/proc/<pid>`: a process with file capabilities
/// (KWin often has `cap_sys_nice`) is not dumpable, and its `/proc`
/// directory is then owned by root.
pub fn parse_status_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// True when a NUL-separated `cmdline` has the `--companion` argument.
pub fn is_companion_cmdline(cmdline: &[u8]) -> bool {
    cmdline.split(|b| *b == 0).any(|arg| arg == COMPANION_ARG)
}

/// Every watched process under `proc_root` that `uid` owns, plus, when
/// `include_xorg` (an X11 session), the X server whoever owns it.
/// Unreadable entries are skipped.
pub fn scan(proc_root: &Path, uid: u32, include_xorg: bool) -> Vec<ProcSample> {
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let dir = entry.path();
        let Ok(comm) = std::fs::read_to_string(dir.join("comm")) else {
            continue;
        };
        let comm = comm.trim_end().to_string();
        let Some(mut role) = classify(&comm) else {
            continue;
        };
        if role == Role::Xorg && !include_xorg {
            continue;
        }
        if role != Role::Xorg {
            let owner = std::fs::read_to_string(dir.join("status"))
                .ok()
                .and_then(|s| parse_status_uid(&s));
            if owner != Some(uid) {
                continue;
            }
        }
        if role == Role::Gui
            && std::fs::read(dir.join("cmdline")).is_ok_and(|c| is_companion_cmdline(&c))
        {
            role = Role::Companion;
        }
        let Some((ticks, start)) = std::fs::read_to_string(dir.join("stat"))
            .ok()
            .and_then(|s| parse_stat(&s))
        else {
            continue;
        };
        out.push(ProcSample {
            pid,
            start,
            ticks,
            role,
            comm,
        });
    }
    out.sort_by_key(|p| p.pid);
    out
}

/// CPU of one group of processes over one window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GroupCpu {
    /// No such process at this scan. Written `none`.
    NotRunning,
    /// Running, but not at the previous scan (first window, or it
    /// restarted). Written `na`.
    NoBaseline,
    /// Percent of one CPU, summed over the group's processes; above 100
    /// when it used more than one core.
    Pct(f64),
}

impl GroupCpu {
    pub fn label(self) -> String {
        match self {
            Self::NotRunning => "none".into(),
            Self::NoBaseline => "na".into(),
            Self::Pct(p) => format!("{p:.1}"),
        }
    }
}

/// Desktop process CPU for one `perf.log` sample.
#[derive(Debug, Clone, PartialEq)]
pub struct HostCpu {
    /// `comm` of the compositor that used the most CPU, else the first one
    /// found. `None` when no compositor runs under this uid.
    pub compositor: Option<String>,
    pub compositor_cpu: GroupCpu,
    /// The X server. `None` on a session that is not X11: the field is
    /// left out of the line. On X11 it is always there (`none` without an
    /// X server).
    pub xorg_cpu: Option<GroupCpu>,
    pub xwayland_cpu: GroupCpu,
    pub portal_cpu: GroupCpu,
    pub pipewire_cpu: GroupCpu,
    /// The tracker's main window.
    pub gui_cpu: GroupCpu,
    /// The tracker's companion overlay. The line says `companion=none`
    /// when this is [`GroupCpu::NotRunning`], else `companion=running`.
    pub companion_cpu: GroupCpu,
}

impl Default for HostCpu {
    fn default() -> Self {
        Self {
            compositor: None,
            compositor_cpu: GroupCpu::NotRunning,
            xorg_cpu: None,
            xwayland_cpu: GroupCpu::NotRunning,
            portal_cpu: GroupCpu::NotRunning,
            pipewire_cpu: GroupCpu::NotRunning,
            gui_cpu: GroupCpu::NotRunning,
            companion_cpu: GroupCpu::NotRunning,
        }
    }
}

impl HostCpu {
    /// The sample-line fields: `compositor=` `compositor_cpu_pct=`,
    /// `xorg_cpu_pct=` (X11 sessions only), `xwayland_cpu_pct=`,
    /// `portal_cpu_pct=`, `pipewire_cpu_pct=`, `gui_cpu_pct=`,
    /// `companion=running|none` and `companion_cpu_pct=`.
    pub fn fields(&self) -> String {
        let mut out = format!(
            "compositor={} compositor_cpu_pct={}",
            self.compositor.as_deref().unwrap_or("none"),
            self.compositor_cpu.label()
        );
        if let Some(x) = self.xorg_cpu {
            out.push_str(&format!(" xorg_cpu_pct={}", x.label()));
        }
        out.push_str(&format!(
            " xwayland_cpu_pct={} portal_cpu_pct={} pipewire_cpu_pct={} \
             gui_cpu_pct={} companion={} companion_cpu_pct={}",
            self.xwayland_cpu.label(),
            self.portal_cpu.label(),
            self.pipewire_cpu.label(),
            self.gui_cpu.label(),
            if self.companion_cpu == GroupCpu::NotRunning {
                "none"
            } else {
                "running"
            },
            self.companion_cpu.label(),
        ));
        out
    }
}

/// Per-group CPU between two scans `secs` apart, at `hz` ticks a second.
/// A process counts only when the same pid with the same start time is in
/// both scans. `x11`: the session is X11, so `xorg_cpu` is reported.
pub fn host_cpu_delta(
    prev: &[ProcSample],
    cur: &[ProcSample],
    secs: f64,
    hz: u64,
    x11: bool,
) -> HostCpu {
    let usable = secs > 0.0 && hz > 0;
    let delta = |p: &ProcSample| -> Option<u64> {
        if !usable {
            return None;
        }
        prev.iter()
            .find(|q| q.pid == p.pid && q.start == p.start)
            .map(|q| p.ticks.saturating_sub(q.ticks))
    };
    let pct = |ticks: u64| ticks as f64 / hz as f64 / secs * 100.0;
    let group = |role: Role| -> GroupCpu {
        let mut any = false;
        let mut sum: Option<u64> = None;
        for p in cur.iter().filter(|p| p.role == role) {
            any = true;
            if let Some(d) = delta(p) {
                sum = Some(sum.unwrap_or(0).saturating_add(d));
            }
        }
        match (any, sum) {
            (false, _) => GroupCpu::NotRunning,
            (true, None) => GroupCpu::NoBaseline,
            (true, Some(t)) => GroupCpu::Pct(pct(t)),
        }
    };
    let compositor = cur
        .iter()
        .filter(|p| p.role == Role::Compositor)
        .max_by_key(|p| (delta(p).unwrap_or(0), std::cmp::Reverse(p.pid)))
        .map(|p| p.comm.clone());
    let compositor_cpu = match &compositor {
        // Only the chosen compositor's processes: a nested or second
        // compositor is not the one drawing the game.
        Some(name) => {
            let deltas: Vec<u64> = cur
                .iter()
                .filter(|p| p.role == Role::Compositor && &p.comm == name)
                .filter_map(&delta)
                .collect();
            if deltas.is_empty() {
                GroupCpu::NoBaseline
            } else {
                GroupCpu::Pct(pct(deltas.iter().sum()))
            }
        }
        None => GroupCpu::NotRunning,
    };
    HostCpu {
        compositor,
        compositor_cpu,
        xorg_cpu: x11.then(|| group(Role::Xorg)),
        xwayland_cpu: group(Role::Xwayland),
        portal_cpu: group(Role::Portal),
        pipewire_cpu: group(Role::Pipewire),
        gui_cpu: group(Role::Gui),
        companion_cpu: group(Role::Companion),
    }
}

/// Keeps the previous scan so each sample reports the CPU used since the
/// last one.
#[derive(Debug, Default)]
pub struct ProcCpuTracker {
    last: Option<(Instant, Vec<ProcSample>)>,
    x11: bool,
}

impl ProcCpuTracker {
    /// `x11`: the session is X11 (see [`host_cpu_delta`]).
    pub const fn new(x11: bool) -> Self {
        Self { last: None, x11 }
    }

    /// Record `cur` (one scan taken at `now`) and return the CPU since the
    /// previous call.
    pub fn sample(&mut self, now: Instant, cur: Vec<ProcSample>, hz: u64) -> HostCpu {
        let out = match &self.last {
            Some((at, prev)) => {
                let secs = now.saturating_duration_since(*at).as_secs_f64();
                host_cpu_delta(prev, &cur, secs, hz, self.x11)
            }
            None => host_cpu_delta(&[], &cur, 0.0, hz, self.x11),
        };
        self.last = Some((now, cur));
        out
    }
}

/// One DRM card under `/sys/class/drm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrmCard {
    /// `card0`, `card1`, ...
    pub name: String,
    /// Kernel driver (`nvidia`, `amdgpu`, `i915`, ...): the name of the
    /// `device/driver` link. `None` when it cannot be read.
    pub driver: Option<String>,
    /// Connector name without the card prefix (`DP-1`) and its `status`
    /// (`connected`, `disconnected`, `unknown`), sorted by name.
    pub connectors: Vec<(String, String)>,
}

impl DrmCard {
    pub fn has_connected_output(&self) -> bool {
        self.connectors.iter().any(|(_, s)| s == "connected")
    }
}

fn card_number(name: &str) -> Option<u32> {
    name.strip_prefix("card")?.parse().ok()
}

/// Cards and connectors under `drm_root` (normally `/sys/class/drm`).
/// Render nodes and the `version` file are skipped. Sorted by card number.
pub fn drm_cards(drm_root: &Path) -> Vec<DrmCard> {
    let Ok(entries) = std::fs::read_dir(drm_root) else {
        return Vec::new();
    };
    let names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    let mut cards: Vec<DrmCard> = names
        .iter()
        .filter(|n| card_number(n).is_some())
        .map(|n| DrmCard {
            name: n.clone(),
            driver: std::fs::read_link(drm_root.join(n).join("device").join("driver"))
                .ok()
                .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned())),
            connectors: Vec::new(),
        })
        .collect();
    cards.sort_by_key(|c| card_number(&c.name));
    for card in &mut cards {
        let prefix = format!("{}-", card.name);
        let mut conns: Vec<(String, String)> = names
            .iter()
            .filter_map(|n| {
                let conn = n.strip_prefix(&prefix)?;
                let status = std::fs::read_to_string(drm_root.join(n).join("status"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| "unknown".into());
                Some((conn.to_string(), status))
            })
            .collect();
        conns.sort();
        card.connectors = conns;
    }
    cards
}

/// `Y` or `N` from `nvidia_drm`'s `modeset` parameter file, `None` when the
/// module is not loaded or the file cannot be read.
pub fn nvidia_drm_modeset(param: &Path) -> Option<String> {
    let v = std::fs::read_to_string(param).ok()?;
    let v = clean_token(v.trim(), 8);
    (!v.is_empty()).then_some(v)
}

/// Keep letters, digits and `:_.-`, at most `max` characters, so an odd
/// value cannot break the key=value line.
fn clean_token(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '-'))
        .take(max)
        .collect()
}

/// Longest host line, in bytes.
pub const HOST_LINE_MAX: usize = 2048;

/// The `host` line for `log.txt`:
/// `host desktop=KDE session=wayland drm_output_cards=card0 nvidia_drm_modeset=Y drm=card0:nvidia[DP-1:connected,HDMI-A-1:disconnected];card1:amdgpu[eDP-1:disconnected]`.
/// A card whose driver cannot be read shows `?`. `drm=none` and
/// `drm_output_cards=none` when there is nothing; `nvidia_drm_modeset=na`
/// when the parameter cannot be read.
pub fn host_line(
    desktop: &str,
    session: &str,
    cards: &[DrmCard],
    nvidia_modeset: Option<&str>,
) -> String {
    let desktop = clean_token(desktop.trim(), 48);
    let drm = if cards.is_empty() {
        "none".to_string()
    } else {
        cards
            .iter()
            .map(|c| {
                let conns: Vec<String> = c
                    .connectors
                    .iter()
                    .map(|(n, s)| format!("{}:{}", clean_token(n, 32), clean_token(s, 16)))
                    .collect();
                format!(
                    "{}:{}[{}]",
                    clean_token(&c.name, 16),
                    c.driver
                        .as_deref()
                        .map(|d| clean_token(d, 32))
                        .unwrap_or_else(|| "?".into()),
                    conns.join(",")
                )
            })
            .collect::<Vec<_>>()
            .join(";")
    };
    let outputs: Vec<&str> = cards
        .iter()
        .filter(|c| c.has_connected_output())
        .map(|c| c.name.as_str())
        .collect();
    // `drm` goes last: it is the only field that can be long, so a cut at
    // the size cap never drops the output cards or the modeset flag.
    let mut line = format!(
        "host desktop={} session={} drm_output_cards={} nvidia_drm_modeset={} drm={}",
        if desktop.is_empty() {
            "unknown"
        } else {
            &desktop
        },
        clean_token(session, 16),
        if outputs.is_empty() {
            "none".to_string()
        } else {
            outputs
                .iter()
                .map(|o| clean_token(o, 16))
                .collect::<Vec<_>>()
                .join(",")
        },
        nvidia_modeset.map_or("na".to_string(), |m| clean_token(m, 8)),
        drm,
    );
    if line.len() > HOST_LINE_MAX {
        let mut cut = HOST_LINE_MAX;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        line.truncate(cut);
    }
    line
}

/// [`host_line`] for this machine, read now.
pub fn host_line_now(session: &str) -> String {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let cards = drm_cards(Path::new("/sys/class/drm"));
    let modeset = nvidia_drm_modeset(Path::new("/sys/module/nvidia_drm/parameters/modeset"));
    host_line(&desktop, session, &cards, modeset.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn stat_line(pid: u32, comm: &str, utime: u64, stime: u64, start: u64) -> String {
        // pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt
        // majflt cmajflt utime stime cutime cstime priority nice threads
        // itrealvalue starttime vsize ...
        format!(
            "{pid} ({comm}) S 1 1 1 0 -1 4194560 100 0 0 0 {utime} {stime} 0 0 20 0 12 0 {start} 123456 789"
        )
    }

    fn sample(pid: u32, comm: &str, ticks: u64, start: u64) -> ProcSample {
        ProcSample {
            pid,
            start,
            ticks,
            role: classify(comm).unwrap(),
            comm: comm.into(),
        }
    }

    #[test]
    fn stat_parsing_takes_utime_stime_and_start_after_the_last_paren() {
        assert_eq!(
            parse_stat(&stat_line(812, "kwin_wayland", 900, 100, 4242)),
            Some((1000, 4242))
        );
        // A comm with spaces and a `)` does not shift the fields.
        assert_eq!(
            parse_stat(&stat_line(9, "a) b (c", 5, 6, 77)),
            Some((11, 77))
        );
        assert_eq!(parse_stat("12 (x) S 1 2"), None);
        assert_eq!(parse_stat(""), None);
    }

    #[test]
    fn status_uid_is_the_real_uid() {
        let status = "Name:\tkwin_wayland\nUmask:\t0022\nState:\tS (sleeping)\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(parse_status_uid(status), Some(1000));
        // real, effective, saved, fs: the real uid is the first column
        assert_eq!(parse_status_uid("Uid:\t1000\t0\t0\t0\n"), Some(1000));
        assert_eq!(parse_status_uid("Uid:\t0\t1000\t1000\t1000\n"), Some(0));
        assert_eq!(parse_status_uid("Name:\tx\n"), None);
    }

    #[test]
    fn only_watched_names_are_classified() {
        assert_eq!(classify("kwin_wayland"), Some(Role::Compositor));
        assert_eq!(classify("Hyprland"), Some(Role::Compositor));
        assert_eq!(classify("gnome-shell"), Some(Role::Compositor));
        assert_eq!(classify("Xorg"), Some(Role::Xorg));
        assert_eq!(classify("Xwayland"), Some(Role::Xwayland));
        // stat-tracker-gui, cut to 15 bytes; the overlay is told apart by
        // its cmdline in `scan`
        assert_eq!(classify("stat-tracker-gu"), Some(Role::Gui));
        // the kernel cuts every portal name to the same 15 bytes
        assert_eq!(classify("xdg-desktop-por"), Some(Role::Portal));
        assert_eq!(classify("pipewire"), Some(Role::Pipewire));
        for other in [
            "pipewire-pulse",
            "wireplumber",
            "kwin_wayland_wr",
            "xdg-document-po",
            "hyprland",
            "scuffed-stat-tr",
            "xwayland-satell",
            "bash",
        ] {
            assert_eq!(classify(other), None, "{other}");
        }
    }

    #[test]
    fn cpu_delta_is_per_group_and_skips_reused_pids() {
        let prev = vec![
            sample(100, "kwin_wayland", 1_000, 50),
            sample(200, "xdg-desktop-por", 10, 60),
            sample(201, "xdg-desktop-por", 20, 61),
            sample(300, "pipewire", 500, 70),
        ];
        let cur = vec![
            // 1200 ticks in 60 s at 100 Hz = 20% of one CPU
            sample(100, "kwin_wayland", 2_200, 50),
            sample(200, "xdg-desktop-por", 40, 60),
            sample(201, "xdg-desktop-por", 50, 61),
            // pid 300 was reused: new start time, no baseline
            sample(300, "pipewire", 600, 99),
        ];
        let h = host_cpu_delta(&prev, &cur, 60.0, 100, false);
        assert_eq!(h.compositor.as_deref(), Some("kwin_wayland"));
        assert_eq!(h.compositor_cpu, GroupCpu::Pct(20.0));
        assert_eq!(h.portal_cpu, GroupCpu::Pct(1.0));
        assert_eq!(h.pipewire_cpu, GroupCpu::NoBaseline);
        assert_eq!(h.xorg_cpu, None);
        assert_eq!(
            h.fields(),
            "compositor=kwin_wayland compositor_cpu_pct=20.0 xwayland_cpu_pct=none \
             portal_cpu_pct=1.0 pipewire_cpu_pct=na gui_cpu_pct=none companion=none companion_cpu_pct=none"
        );
    }

    /// The tracker's own windows and Xwayland: the main GUI, the overlay
    /// (same binary with `--companion`) and Xwayland each get a field.
    #[test]
    fn gui_companion_and_xwayland_are_reported() {
        let mut overlay = sample(41, "stat-tracker-gu", 100, 9);
        overlay.role = Role::Companion;
        let mut overlay_now = overlay.clone();
        overlay_now.ticks = 1_300; // 1200 ticks / 60 s / 100 Hz = 20%
        let prev = vec![
            sample(40, "stat-tracker-gu", 10, 8),
            overlay,
            sample(50, "Xwayland", 0, 3),
        ];
        let cur = vec![
            sample(40, "stat-tracker-gu", 70, 8),
            overlay_now,
            sample(50, "Xwayland", 3_000, 3),
        ];
        let h = host_cpu_delta(&prev, &cur, 60.0, 100, false);
        assert_eq!(h.gui_cpu, GroupCpu::Pct(1.0));
        assert_eq!(h.companion_cpu, GroupCpu::Pct(20.0));
        assert_eq!(h.xwayland_cpu, GroupCpu::Pct(50.0));
        let f = h.fields();
        for want in [
            " xwayland_cpu_pct=50.0 ",
            " gui_cpu_pct=1.0 ",
            " companion=running companion_cpu_pct=20.0",
        ] {
            assert!(f.contains(want), "{want} missing from {f}");
        }
        // An overlay that just started has no baseline but is running.
        let fresh = host_cpu_delta(&[], &cur, 60.0, 100, false);
        assert!(
            fresh
                .fields()
                .ends_with(" gui_cpu_pct=na companion=running companion_cpu_pct=na"),
            "{}",
            fresh.fields()
        );
    }

    /// A zero-length or zero-Hz window never divides: every group with a
    /// process is `na`, not inf or NaN.
    #[test]
    fn a_zero_window_has_no_percent() {
        let prev = vec![sample(1, "niri", 0, 1), sample(2, "pipewire", 0, 1)];
        let cur = vec![sample(1, "niri", 50, 1), sample(2, "pipewire", 5, 1)];
        for (secs, hz) in [(0.0, 100), (60.0, 0), (-1.0, 100)] {
            let h = host_cpu_delta(&prev, &cur, secs, hz, false);
            assert_eq!(h.compositor_cpu, GroupCpu::NoBaseline, "{secs} {hz}");
            assert_eq!(h.pipewire_cpu, GroupCpu::NoBaseline, "{secs} {hz}");
        }
    }

    #[test]
    fn xorg_is_reported_only_on_an_x11_session() {
        let prev = vec![sample(5, "Xorg", 10, 1)];
        let cur = vec![sample(5, "Xorg", 70, 1)];
        // Wayland: the field is left out even with an Xorg in the scan.
        let wayland = host_cpu_delta(&prev, &cur, 60.0, 100, false);
        assert_eq!(wayland.xorg_cpu, None);
        assert!(!wayland.fields().contains("xorg"), "{}", wayland.fields());
        // X11: there, and `none` when no X server was found.
        let x11 = host_cpu_delta(&prev, &cur, 60.0, 100, true);
        assert_eq!(x11.xorg_cpu, Some(GroupCpu::Pct(1.0)));
        assert!(x11.fields().contains(" xorg_cpu_pct=1.0 "));
        let gone = host_cpu_delta(&prev, &[], 60.0, 100, true);
        assert!(
            gone.fields().contains(" xorg_cpu_pct=none "),
            "{}",
            gone.fields()
        );
    }

    #[test]
    fn first_scan_has_no_baseline_and_missing_groups_say_none() {
        let mut t = ProcCpuTracker::new(true);
        let t0 = Instant::now();
        let first = t.sample(t0, vec![sample(5, "Xorg", 10, 1)], 100);
        assert_eq!(first.compositor, None);
        assert_eq!(first.xorg_cpu, Some(GroupCpu::NoBaseline));
        assert_eq!(
            first.fields(),
            "compositor=none compositor_cpu_pct=none xorg_cpu_pct=na xwayland_cpu_pct=none \
             portal_cpu_pct=none pipewire_cpu_pct=none gui_cpu_pct=none companion=none companion_cpu_pct=none"
        );
        // 250 ticks over 50 s at 100 Hz: 5%, above 100 would mean several cores
        let next = t.sample(
            t0 + Duration::from_secs(50),
            vec![sample(5, "Xorg", 260, 1)],
            100,
        );
        assert_eq!(next.xorg_cpu, Some(GroupCpu::Pct(5.0)));
    }

    #[test]
    fn the_busiest_compositor_is_named() {
        let prev = vec![sample(10, "sway", 0, 1), sample(11, "niri", 0, 1)];
        let cur = vec![sample(10, "sway", 30, 1), sample(11, "niri", 600, 1)];
        let h = host_cpu_delta(&prev, &cur, 60.0, 100, false);
        assert_eq!(h.compositor.as_deref(), Some("niri"));
        assert_eq!(h.compositor_cpu, GroupCpu::Pct(10.0));
    }

    #[test]
    fn scan_reads_a_fake_proc_tree_and_filters_by_uid() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mk = |pid: u32, comm: &str, uid: u32, ticks: u64| {
            let p = root.join(pid.to_string());
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("comm"), format!("{comm}\n")).unwrap();
            std::fs::write(p.join("cmdline"), format!("/usr/bin/{comm}\0")).unwrap();
            std::fs::write(
                p.join("status"),
                format!("Name:\t{comm}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .unwrap();
            std::fs::write(p.join("stat"), stat_line(pid, comm, ticks, 0, 7)).unwrap();
        };
        mk(10, "kwin_wayland", 1000, 400);
        mk(11, "kwin_wayland", 1001, 999); // another user's session
        mk(12, "Xorg", 0, 50); // root X server counts
        mk(13, "pipewire", 1000, 20);
        mk(14, "firefox", 1000, 5000);
        mk(16, "Xwayland", 1000, 30);
        mk(17, "stat-tracker-gu", 1000, 11);
        mk(18, "stat-tracker-gu", 1000, 12);
        std::fs::write(
            root.join("18/cmdline"),
            b"/usr/bin/stat-tracker-gui\0--companion\0".as_slice(),
        )
        .unwrap();
        mk(19, "stat-tracker-gu", 1001, 13); // another user's tracker
        std::fs::create_dir_all(root.join("self")).unwrap();
        std::fs::create_dir_all(root.join("15")).unwrap(); // no files: skipped
        let got = scan(root, 1000, true);
        let pids: Vec<(u32, Role)> = got.iter().map(|p| (p.pid, p.role)).collect();
        assert_eq!(
            pids,
            vec![
                (10, Role::Compositor),
                (12, Role::Xorg),
                (13, Role::Pipewire),
                (16, Role::Xwayland),
                (17, Role::Gui),
                (18, Role::Companion),
            ]
        );
        assert_eq!(got[0].ticks, 400);
        assert_eq!(got[0].start, 7);
        // A Wayland session never scans Xorg (a greeter's X server).
        let wayland: Vec<u32> = scan(root, 1000, false).iter().map(|p| p.pid).collect();
        assert_eq!(wayland, vec![10, 13, 16, 17, 18]);
        assert!(scan(&root.join("missing"), 1000, true).is_empty());
    }

    #[test]
    fn the_companion_flag_is_a_whole_argument() {
        assert!(is_companion_cmdline(
            b"/usr/bin/stat-tracker-gui\0--companion\0"
        ));
        assert!(is_companion_cmdline(b"stat-tracker-gui\0--companion"));
        assert!(!is_companion_cmdline(b"/usr/bin/stat-tracker-gui\0"));
        assert!(!is_companion_cmdline(b"stat-tracker-gui\0--companion-x\0"));
        assert!(!is_companion_cmdline(
            b"/home/a/--companion/stat-tracker-gui\0"
        ));
    }

    #[test]
    fn drm_topology_from_a_fake_sysfs_tree() {
        let dir = tempfile::tempdir().unwrap();
        let drm = dir.path().join("class/drm");
        let drivers = dir.path().join("bus/pci/drivers");
        std::fs::create_dir_all(&drm).unwrap();
        for d in ["nvidia", "amdgpu"] {
            std::fs::create_dir_all(drivers.join(d)).unwrap();
        }
        let card = |name: &str, driver: Option<&str>| {
            let dev = drm.join(name).join("device");
            std::fs::create_dir_all(&dev).unwrap();
            if let Some(d) = driver {
                std::os::unix::fs::symlink(drivers.join(d), dev.join("driver")).unwrap();
            }
        };
        let conn = |name: &str, status: &str| {
            let p = drm.join(name);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("status"), format!("{status}\n")).unwrap();
        };
        card("card1", Some("nvidia"));
        card("card0", Some("amdgpu"));
        card("card2", None);
        conn("card1-HDMI-A-1", "disconnected");
        conn("card1-DP-1", "connected");
        conn("card0-eDP-1", "disconnected");
        std::fs::create_dir_all(drm.join("renderD128")).unwrap();
        std::fs::write(drm.join("version"), "drm 1.1.0\n").unwrap();

        let cards = drm_cards(&drm);
        assert_eq!(
            cards,
            vec![
                DrmCard {
                    name: "card0".into(),
                    driver: Some("amdgpu".into()),
                    connectors: vec![("eDP-1".into(), "disconnected".into())],
                },
                DrmCard {
                    name: "card1".into(),
                    driver: Some("nvidia".into()),
                    connectors: vec![
                        ("DP-1".into(), "connected".into()),
                        ("HDMI-A-1".into(), "disconnected".into()),
                    ],
                },
                DrmCard {
                    name: "card2".into(),
                    driver: None,
                    connectors: vec![],
                },
            ]
        );

        let param = dir.path().join("module/nvidia_drm/parameters/modeset");
        assert_eq!(nvidia_drm_modeset(&param), None);
        std::fs::create_dir_all(param.parent().unwrap()).unwrap();
        std::fs::write(&param, "Y\n").unwrap();
        let modeset = nvidia_drm_modeset(&param);
        assert_eq!(modeset.as_deref(), Some("Y"));

        let line = host_line("KDE", "wayland", &cards, modeset.as_deref());
        assert_eq!(
            line,
            "host desktop=KDE session=wayland drm_output_cards=card1 nvidia_drm_modeset=Y \
             drm=card0:amdgpu[eDP-1:disconnected];card1:nvidia[DP-1:connected,HDMI-A-1:disconnected];card2:?[]"
        );
        assert!(drm_cards(&dir.path().join("nope")).is_empty());
    }

    #[test]
    fn host_line_without_cards_and_with_odd_values() {
        let line = host_line("ubuntu:GNOME", "x11", &[], None);
        assert_eq!(
            line,
            "host desktop=ubuntu:GNOME session=x11 drm_output_cards=none nvidia_drm_modeset=na drm=none"
        );
        // spaces and `=` in the desktop cannot add a field
        let odd = host_line("  KDE x=1 ", "wayland", &[], None);
        assert!(
            odd.starts_with("host desktop=KDEx1 session=wayland"),
            "{odd}"
        );
        assert!(host_line("", "unknown", &[], None).contains("desktop=unknown"));
    }

    /// A machine with many cards and connectors hits the size cap. The cut
    /// lands inside `drm=`, so the output cards and the modeset flag stay.
    #[test]
    fn truncation_keeps_the_output_cards_and_modeset() {
        let cards: Vec<DrmCard> = (0..40)
            .map(|i| DrmCard {
                name: format!("card{i}"),
                driver: Some("nvidia".into()),
                connectors: (1..=6)
                    .map(|c| {
                        let status = if i == 39 && c == 6 {
                            "connected"
                        } else {
                            "disconnected"
                        };
                        (format!("HDMI-A-{c}"), status.to_string())
                    })
                    .collect(),
            })
            .collect();
        let line = host_line("KDE", "wayland", &cards, Some("Y"));
        assert_eq!(line.len(), HOST_LINE_MAX, "the line is cut");
        assert!(
            line.starts_with(
                "host desktop=KDE session=wayland drm_output_cards=card39 nvidia_drm_modeset=Y drm=card0:nvidia["
            ),
            "{line}"
        );
    }
}
