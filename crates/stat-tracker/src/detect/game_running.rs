//! Gate captures on the game process actually running.
//!
//! The Tab listener is a global evdev hook and the poller screenshots whatever
//! is on screen — without this gate, pressing Tab in a browser (or a warm-colored
//! website hitting the banner detector) can record garbage frames. Scanning
//! /proc/<pid>/comm is cheap (~1ms) and the result is cached for a few seconds
//! so the 4s poller doesn't rescan procfs every tick.

use std::time::{Duration, Instant};

const CACHE_TTL: Duration = Duration::from_secs(5);

/// Kernel truncates /proc/<pid>/comm to 15 bytes (TASK_COMM_LEN - 1), so
/// configured names are compared after the same truncation. Exact match (not
/// prefix) so "Overwatch.exe" does not also match "OverwatchLauncher.exe".
const COMM_LEN: usize = 15;

pub struct GameProcessGate {
    /// Lowercased, comm-truncated process names to look for. Empty = gate disabled.
    targets: Vec<String>,
    cached: Option<(Instant, bool)>,
}

impl GameProcessGate {
    pub fn new(names: &[String]) -> Self {
        let targets = names
            .iter()
            .map(|n| n.to_lowercase().chars().take(COMM_LEN).collect())
            .collect();
        Self {
            targets,
            cached: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.targets.is_empty()
    }

    /// Whether the game process is currently running. Always true when the gate
    /// is disabled. Result is cached for a few seconds.
    pub fn is_running(&mut self) -> bool {
        if self.targets.is_empty() {
            return true;
        }
        if let Some((checked_at, running)) = self.cached
            && checked_at.elapsed() < CACHE_TTL
        {
            return running;
        }

        let running = scan_proc(&self.targets);
        let prev = self.cached.map(|(_, r)| r);
        if prev != Some(running) {
            if running {
                tracing::info!("game process detected — captures active");
            } else {
                tracing::info!(
                    looking_for = ?self.targets,
                    "game process not found — captures paused until it starts"
                );
            }
        }
        self.cached = Some((Instant::now(), running));
        running
    }
}

/// Scan /proc for a process whose comm matches one of `targets`.
/// Fails open: if /proc can't be read at all, captures stay enabled.
fn scan_proc(targets: &[String]) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        tracing::warn!("cannot read /proc — game-process gate disabled for this check");
        return true;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str() else { continue };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Ok(comm) = std::fs::read_to_string(entry.path().join("comm"))
            && comm_matches(comm.trim(), targets)
        {
            return true;
        }
    }
    false
}

fn comm_matches(comm: &str, targets: &[String]) -> bool {
    targets.contains(&comm.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_names_disables_gate() {
        let mut gate = GameProcessGate::new(&[]);
        assert!(!gate.is_enabled());
        assert!(gate.is_running());
    }

    #[test]
    fn finds_own_process() {
        let comm = std::fs::read_to_string("/proc/self/comm")
            .expect("read /proc/self/comm")
            .trim()
            .to_string();
        let mut gate = GameProcessGate::new(&[comm]);
        assert!(gate.is_running());
    }

    #[test]
    fn missing_process_is_not_running() {
        let mut gate = GameProcessGate::new(&["no-such-process-zzz.exe".to_string()]);
        assert!(!gate.is_running());
    }

    #[test]
    fn comm_matching_is_case_insensitive_and_truncated() {
        // "OverwatchLauncher.exe" truncates to "overwatchlaunch" and must NOT
        // match a target of "Overwatch.exe".
        let gate = GameProcessGate::new(&["Overwatch.exe".to_string()]);
        assert!(comm_matches("Overwatch.exe", &gate.targets));
        assert!(comm_matches("overwatch.exe", &gate.targets));
        assert!(!comm_matches("OverwatchLaunch", &gate.targets));
    }
}
